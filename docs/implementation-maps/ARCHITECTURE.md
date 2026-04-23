<!-- code-truth: 9c24a09 -->

# Architecture

## Module Dependency Graph

```
main.rs (CLI)
  ├── config.rs      (load at startup, global singleton)
  ├── emoncms.rs     (reads config for base_url)
  ├── db.rs          (reads config for feed IDs, sync start)
  │     └── emoncms.rs (sync only)
  ├── analysis.rs    (reads config for thresholds, house, gas_era, arotherm)
  │     └── config.rs
  ├── gaps.rs        (reads config for feed IDs, thresholds)
  │     └── config.rs
  ├── octopus.rs     (reads config for thresholds, gas_era)
  │     └── config.rs
  ├── octopus_tariff.rs (re-exports shared octopus-tariff crate)
  ├── overnight.rs   (reads config.tariff; uses analysis::enrich + octopus_tariff)
  │     └── analysis.rs, config.rs, octopus_tariff.rs
  └── thermal.rs     (thin facade — re-exports from 17 submodules)
        └── thermal/  (own config from model/thermal-config.toml)
              └── display.rs  (equilibrium solver, MWT bisection, control table)
              └── dhw_sessions.rs  (DHW draw/charge analysis)

lib.rs (library crate)
  └── thermal.rs (re-exports solver: bisect_mwt_for_room, solve_equilibrium_temps)

adaptive-heating-mvp.rs (separate binary, depends on heatpump_analysis lib)
  ├── heatpump_analysis::thermal::bisect_mwt_for_room() (live solver)
  ├── model/adaptive-heating-mvp.toml (own TOML config)
  ├── data/canonical/thermal_geometry.json (room geometry for solver)
  ├── ebusd TCP (localhost:8888)
  ├── PostgreSQL / TimescaleDB
  ├── legacy Flux/Influx compatibility tail (still present in some code paths)
  ├── Open-Meteo HTTP (forecast API)
  ├── Axum HTTP API (:3031)
  └── JSONL + PostgreSQL logging
```

Key constraints:
- **analysis.rs has no dependency on db.rs or emoncms.rs** — operates purely on Polars DataFrames
- **thermal.rs has no dependency on config.rs** — uses its own `ThermalConfig`
- **adaptive-heating-mvp depends on the thermal solver** via `heatpump_analysis::thermal::bisect_mwt_for_room()` (since Phase 1b). Uses its own config, PostgreSQL-backed latest-value reads, its own eBUS access, its own forecast client, and still carries a legacy Influx compatibility tail in some code. The thermal module is compiled into the binary as a library dependency.
- **gaps.rs bypasses db.rs** — writes directly to `simulated_samples` and `gap_log` tables

## Data Flow

### Sync path (online)

```
emoncms.org API → emoncms.rs::Client → db.rs::sync_all() → SQLite (samples table)
```

### Analysis path (offline)

```
SQLite → db.rs::load_dataframe() → analysis.rs::enrich() → analysis functions → stdout
```

### Thermal model path (live PostgreSQL-first, with legacy compatibility tail)

```
PostgreSQL / TimescaleDB (pi5data:5432, db "energy")
  ├── room temps (zigbee2mqtt/*_temp_humid, emon/emonth2_23/temperature)
  ├── outside temp (ebusd/poll/OutsideTemp)
  ├── HP state (ebusd/poll/BuildingCircuitFlow, StatuscodeNum)
  ├── MWT (ebusd/poll/FlowTemp, ReturnTemp)
  ├── PV power (emon/EmonPi2/P3)
  │
  └──→ PostgreSQL-first readers + legacy `thermal/influx.rs` compatibility helpers
                                 │
                                 └──→ thermal.rs (calibrate / validate / operational)
                                               │
                                               └──→ artifacts/thermal/*.json
```

### Adaptive heating V2 control path (live on pi5data)

```
Open-Meteo forecast API (hourly: temp, solar, humidity)
  └──→ ForecastCache (refreshed every 3600s)

eBUS (ebusd TCP :8888)
  ├── reads: RunDataStatuscode, DisplayedOutsideTemp, Hc1ActualFlowTempDesired,
  │          Hc1HeatCurve, HwcStorageTemp, CurrentCompressorUtil,
  │          RunDataElectricPowerConsumption, CurrentYieldPower,
  │          RunDataFlowTemp, RunDataReturnTemp
  ├── writes: Hc1HeatCurve (inner loop), Z1OpMode (startup/shutdown),
  │           HwcSFMode (DHW boost)
  │
PostgreSQL / TimescaleDB
  ├── reads: Leather temp, Aldora temp, Tesla, Multical, controller history
  ├── writes: adaptive_heating_mvp table (decision logs)

  └──→ Outer loop (900s): forecast → live thermal solver (bisect_mwt_for_room) → target_flow + initial curve
         │
         └──→ Inner loop (60s): error = target_flow - Hc1ActualFlowTempDesired
                │                  curve += gain × error (proportional feedback)
                │
                ├──→ eBUS write: Hc1HeatCurve
                ├──→ PostgreSQL (decision metrics)
                └──→ JSONL file (full decision audit log)
```

### Mobile control path

```
Phone browser
  → z2m-hub (pi5data:3030) /api/heating/*
    → HTTP proxy to adaptive-heating-mvp (pi5data:3031) /mode/*, /status, /kill
```

## Implicit Contracts

### eBUS availability

The adaptive-heating-mvp assumes ebusd is running on localhost:8888 and responsive. If ebusd is down:
- reads return errors → `missing_core = true` → control logic skipped
- writes fail → logged but control cycle continues
- no retry/reconnect logic beyond the cycle interval

### Telemetry topic naming

Room temperature topics must match between:
- the shared ingest/routing layer (what gets written)
- `model/adaptive-heating-mvp.toml` `[topics]` (what gets queried)
- `model/thermal-config.toml` sensor_topics (what thermal model uses)

The emonth2 uses `value` while Zigbee sensors use `temperature`. PostgreSQL routing preserves that distinction, and the remaining legacy `src/thermal/influx.rs` helpers mirror the same reader semantics.

### VRC 700 baseline safety net

On startup: `Z1OpMode=night` (value 3) + `Hc1MinFlowTempDesired=19`, but only when persisted mode is active. If persisted mode is `Disabled` or `MonitorOnly`, startup skips eBUS initialisation so the baseline stays untouched. On shutdown/kill: `restore_baseline()` writes `Z1OpMode=auto` + `Hc1HeatCurve=0.55` + `Hc1MinFlowTempDesired=20`. `/kill` is now a toggle: disabled → restore baseline, disabled state; disabled + `/kill` again → reinitialise eBUS and resume `Occupied`.

### Coast mechanism

Coast uses `Z1OpMode=off` (not a low curve). `RuntimeState.heating_off` tracks this state. Two restore points write `Z1OpMode=night` to re-enable: (1) entering waking/preheat hours, (2) during overnight when maintain becomes true or preheat is ≤15 min away. Previous approach (curve 0.10) failed because `Hc1MinFlowTempDesired=20` created a hidden floor.

### ΔT stabilisation contract

The outer loop only uses live flow-return ΔT when `RunDataStatuscode` contains both "Heating" and "Compressor" AND ΔT > 1.0°C. Otherwise falls back to `default_delta_t_c` (4.0°C). This prevents target_flow oscillation when compressor cycles off and flow ≈ return.

### Inner loop floor guard contract

When `Hc1HeatCurve < 0.25`, the inner loop halves its gain and doubles its deadband. This prevents hunting near the curve floor where each 0.01 curve ≈ 0.20°C flow change (verified by measurement).

### Tariff truth bridge

`src/octopus_tariff.rs` is the bridge from heatpump-analysis to Octopus account truth. It loads import tariff agreements from the account API, fetches half-hourly unit rates for each overlapping agreement, derives each agreement's cheapest import rate, and lets analysis price demand without hardcoded tariff snapshots.

`config.toml` now keeps only `tariff.battery_coverage`; current and historical unit rates are not stored in repo config.

### Config duplication

Radiator T50 values exist in both `config.toml` (used by `analysis.rs`) and `data/canonical/thermal_geometry.json` (used by `thermal/geometry.rs`). These must be kept in sync manually.

The adaptive-heating-mvp has its own baseline values in `model/adaptive-heating-mvp.toml` `[baseline]`. These must match the known-good VRC 700 settings documented in `docs/vrc700-settings-audit.md`.

### Thermal solver is compiled into the binary

Since Phase 1b, `adaptive-heating-mvp` calls `heatpump_analysis::thermal::bisect_mwt_for_room()` directly. This means `data/canonical/thermal_geometry.json` must be deployed alongside the binary on pi5data. The solver runs in <1ms on ARM. `model/control-table.json` is legacy and no longer loaded.

### Deployment workflow

Current deployment is: edit on laptop, sync controller sources with `scripts/sync-to-pi5data.sh`, then build natively on pi5data and restart the systemd service. Cross-compiling from WSL2 to pi5data's glibc is no longer the primary path.
