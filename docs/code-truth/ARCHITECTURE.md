<!-- code-truth: 0b91843 -->

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
  ├── overnight.rs   (reads config for thresholds; uses analysis::enrich)
  │     └── analysis.rs, config.rs
  └── thermal.rs     (thin facade — re-exports from 16 submodules)
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
  ├── InfluxDB HTTP (localhost:8086)
  ├── Open-Meteo HTTP (forecast API)
  ├── Axum HTTP API (:3031)
  └── JSONL + InfluxDB logging
```

Key constraints:
- **analysis.rs has no dependency on db.rs or emoncms.rs** — operates purely on Polars DataFrames
- **thermal.rs has no dependency on config.rs** — uses its own `ThermalConfig`
- **adaptive-heating-mvp depends on the thermal solver** via `heatpump_analysis::thermal::bisect_mwt_for_room()` (since Phase 1b). Uses its own config, own InfluxDB queries, own eBUS access, own forecast client. The thermal module is compiled into the binary as a library dependency.
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

### Thermal model path (InfluxDB)

```
InfluxDB (pi5data:8086, bucket "energy")
  ├── room temps (zigbee2mqtt/*_temp_humid, emon/emonth2_23/temperature)
  ├── outside temp (ebusd/poll/OutsideTemp)
  ├── HP state (ebusd/poll/BuildingCircuitFlow, StatuscodeNum)
  ├── MWT (ebusd/poll/FlowTemp, ReturnTemp)
  ├── PV power (emon/EmonPi2/P3)
  │
  └──→ thermal/influx.rs → thermal.rs (calibrate / validate / operational)
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
InfluxDB (localhost:8086)
  ├── reads: Leather temp, Aldora temp
  ├── writes: adaptive_heating_mvp measurement (decision logs)

  └──→ Outer loop (900s): forecast → live thermal solver (bisect_mwt_for_room) → target_flow + initial curve
         │
         └──→ Inner loop (60s): error = target_flow - Hc1ActualFlowTempDesired
                │                  curve += gain × error (proportional feedback)
                │
                ├──→ eBUS write: Hc1HeatCurve
                ├──→ InfluxDB (decision metrics)
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

### InfluxDB topic naming

Room temperature topics must match between:
- Telegraf MQTT→InfluxDB config (what gets written)
- `model/adaptive-heating-mvp.toml` `[topics]` (what gets queried)
- `model/thermal-config.toml` sensor_topics (what thermal model uses)

The emonth2 uses `_field = "value"` while Zigbee sensors use `_field = "temperature"`. This is handled in both `src/thermal/influx.rs` and `src/bin/adaptive-heating-mvp.rs::query_latest_room_temp()`.

### VRC 700 baseline safety net

On startup: `Z1OpMode=night` (value 3) + `Hc1MinFlowTempDesired=19`. VRC 700 uses `Z1NightTemp` (19°C) permanently. On shutdown: `Z1OpMode=auto` + `Hc1HeatCurve=0.55` + `Hc1MinFlowTempDesired=20`. VRC 700 resumes timer control. Crash without restore: house at 19°C with last curve. Safe.

### Coast mechanism

Coast uses `Z1OpMode=off` (not a low curve). `RuntimeState.heating_off` tracks this state. Two restore points write `Z1OpMode=night` to re-enable: (1) entering waking/preheat hours, (2) during overnight when maintain becomes true or preheat is ≤15 min away. Previous approach (curve 0.10) failed because `Hc1MinFlowTempDesired=20` created a hidden floor.

### ΔT stabilisation contract

The outer loop only uses live flow-return ΔT when `RunDataStatuscode` contains both "Heating" and "Compressor" AND ΔT > 1.0°C. Otherwise falls back to `default_delta_t_c` (4.0°C). This prevents target_flow oscillation when compressor cycles off and flow ≈ return.

### Inner loop floor guard contract

When `Hc1HeatCurve < 0.25`, the inner loop halves its gain and doubles its deadband. This prevents hunting near the curve floor where each 0.01 curve ≈ 0.20°C flow change (verified by measurement).

### Overnight planner empirical constants

`LEATHER_TAU_H = 50.0` and `REHEAT_RATE = 7500` in `adaptive-heating-mvp.rs` are hardcoded constants that drive the overnight coast/preheat decision. τ=50h is empirically validated (53 segments). K=7500 is conservative — empirical K≈20,600 from 27 segments suggests the code overpredicts reheat time. Each coast-then-preheat night validates these.

### Config duplication

Radiator T50 values exist in both `config.toml` (used by `analysis.rs`) and `data/canonical/thermal_geometry.json` (used by `thermal/geometry.rs`). These must be kept in sync manually.

The adaptive-heating-mvp has its own baseline values in `model/adaptive-heating-mvp.toml` `[baseline]`. These must match the known-good VRC 700 settings documented in `docs/vrc700-settings-audit.md`.

### Thermal solver is compiled into the binary

Since Phase 1b, `adaptive-heating-mvp` calls `heatpump_analysis::thermal::bisect_mwt_for_room()` directly. This means `data/canonical/thermal_geometry.json` must be deployed alongside the binary on pi5data. The solver runs in <1ms on ARM. `model/control-table.json` is legacy and no longer loaded.
