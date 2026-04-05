# Architecture

Three Rust binaries sharing a thermal solver library. Data flows from emoncms/InfluxDB through analysis and modelling to live VRC 700 control.

## Binaries

The project produces three binaries from a single Cargo workspace with a library crate.

- **`heatpump-analysis`** (default binary): CLI for data sync, state classification, COP analysis, thermal modelling, DHW session analysis, and history review. ~1,411 lines in `main.rs` + modules.
- **`adaptive-heating-mvp`**: long-lived service on pi5data. Model-predictive heating controller with HTTP API. ~2,053 lines. Depends on the thermal solver via `heatpump_analysis::thermal::bisect_mwt_for_room()`.
- **`thermal-regression-check`**: CI tool comparing thermal artifacts against baselines. 607 lines.

`src/lib.rs` exposes `pub mod thermal` so the adaptive controller can call solver functions as a library dependency. `thermal_geometry.json` must be deployed alongside the binary.

## Module Dependencies

Analysis and thermal modules are decoupled. They share no config and use different data sources.

```
main.rs (CLI)
  ├── config.rs      (global singleton from config.toml)
  ├── emoncms.rs     (sync only)
  ├── db.rs          (SQLite: feeds, samples, sync_state)
  ├── analysis.rs    (state machine + Polars, no db.rs dependency)
  ├── gaps.rs        (bypasses db.rs, own SQLite tables)
  ├── octopus.rs     (reads ~/github/octopus/data/)
  ├── overnight.rs   (backtest model)
  └── thermal.rs     (facade → 17 submodules, own ThermalConfig)

adaptive-heating-mvp.rs (separate binary)
  ├── heatpump_analysis::thermal (library crate)
  ├── model/adaptive-heating-mvp.toml (own config)
  ├── ebusd TCP :8888 (VRC 700 reads/writes)
  ├── InfluxDB HTTP :8086 (room temps, decision logs)
  ├── Open-Meteo HTTP (hourly forecast)
  └── Axum HTTP :3031 (status/mode/kill API)
```

Key isolation: `analysis.rs` has no dependency on `db.rs` — operates purely on Polars DataFrames. `thermal.rs` has no dependency on `config.rs` — uses its own `ThermalConfig` from `model/thermal-config.toml`. `gaps.rs` bypasses `db.rs` entirely.

## Data Flow

Four distinct data paths serve different purposes.

### Sync Path

emoncms.org REST API → `emoncms.rs` → [[src/db.rs#sync_all]] → SQLite (samples table). Online, blocking, 100ms politeness delay.

### Analysis Path

SQLite → [[src/db.rs#load_dataframe]] → [[src/analysis.rs#enrich]] → analysis functions → stdout. Offline, Polars lazy evaluation.

### Thermal Model Path

InfluxDB (pi5data, bucket "energy") → `thermal/influx.rs` → calibration/validation/operational → `artifacts/thermal/*.json`. Room temps from Zigbee, outside from eBUS, HP state from BuildingCircuitFlow, MWT from FlowTemp/ReturnTemp, PV from P3 CT.

### Live Control Path

Open-Meteo forecast → outer loop → [[src/thermal/display.rs#bisect_mwt_for_room]] → target flow → inner loop → eBUS `Hc1HeatCurve` write. Decision logs to InfluxDB + local JSONL.

Mobile controls: phone → z2m-hub (:3030) `/api/heating/*` → HTTP proxy → adaptive-heating-mvp (:3031) `/mode/*`, `/status`, `/kill`.

## Configuration

Three independent TOML configs for three independent concerns.

| File | Used by | Concern |
|------|---------|---------|
| `config.toml` | CLI analysis modules | Domain constants, thresholds, feed IDs, radiators |
| `model/thermal-config.toml` | Thermal model | InfluxDB, test nights, calibration bounds |
| `model/adaptive-heating-mvp.toml` | Adaptive controller | eBUS host, InfluxDB, Cosy windows, baseline, inner loop tuning |

`data/canonical/thermal_geometry.json` is the single source of truth for room geometry, consumed by both the thermal solver and the adaptive controller. `model/control-table.json` is legacy — no longer loaded (replaced by live solver in Phase 1b).

## Implicit Contracts

Assumptions that are not enforced by the type system but will break the system if violated.

### eBUS Availability

ebusd must be running on localhost:8888. If down: reads error → `missing_core = true` → control skipped. No retry beyond cycle interval.

### InfluxDB Topic Naming

Room temperature topics must match between Telegraf MQTT→InfluxDB config, `adaptive-heating-mvp.toml` `[topics]`, and `thermal-config.toml` sensor_topics. The emonth2 uses `_field = "value"` while Zigbee sensors use `_field = "temperature"`.

### VRC 700 Baseline Safety

Startup and shutdown writes ensure the VRC 700 can resume autonomous operation if the controller crashes.

Startup: `Z1OpMode=night` + `Hc1MinFlowTempDesired=19`. Shutdown: `Z1OpMode=auto` + `Hc1HeatCurve=0.55` + `Hc1MinFlowTempDesired=20`. Normal crash: house at 19°C with last curve — safe. Coast crash (`Z1OpMode=off`): heating is off — `heating_off` flag tracks this, restore points re-enable on next waking/preheat.

### Thermal Solver Compilation

Since Phase 1b, `adaptive-heating-mvp` calls `bisect_mwt_for_room()` directly from the library crate. `thermal_geometry.json` must be deployed alongside the binary on pi5data. Solver runs in <1ms on ARM.
