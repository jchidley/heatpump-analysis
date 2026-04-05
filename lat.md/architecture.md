# Architecture

Three Rust binaries sharing a thermal solver library. Data flows from emoncms/InfluxDB through analysis and modelling to live VRC 700 control.

## Binaries

The project produces three binaries from a single Cargo workspace with a library crate.

- **`heatpump-analysis`** (default binary): CLI for data sync, state classification, COP analysis, thermal modelling, DHW session analysis, and history review. ~1,411 lines in `main.rs` + modules.
- **`adaptive-heating-mvp`**: long-lived service on pi5data. Model-predictive heating controller with HTTP API. ~2,053 lines. Depends on the thermal solver via `heatpump_analysis::thermal::bisect_mwt_for_room()`.
- **`thermal-regression-check`**: CI tool comparing thermal artifacts against baselines. 607 lines.

`src/lib.rs` exposes `pub mod thermal` so the adaptive controller can call solver functions as a library dependency. `thermal_geometry.json` must be deployed alongside the binary.

- **`ebus-core`** (library crate, `ebus-core/`): `no_std` eBUS protocol primitives — CRC-8, address classification, byte stuffing, telegram parsing, SYN-delimited framing. Ported from yuhu-ebus. 22 tests. Not yet integrated into the main workspace — standalone crate for future Pico W firmware.

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

Five distinct data paths serve different purposes.

### Sync Path

emoncms.org REST API → `emoncms.rs` → [[src/db.rs#sync_all]] → SQLite (samples table). Online, blocking, 100ms politeness delay.

### Analysis Path

SQLite → [[src/db.rs#load_dataframe]] → [[src/analysis.rs#enrich]] → analysis functions → stdout. Offline, Polars lazy evaluation.

### Thermal Model Path

InfluxDB (pi5data, bucket "energy") → `thermal/influx.rs` → calibration/validation/operational → `artifacts/thermal/*.json`. Room temps from Zigbee, outside from eBUS, HP state from BuildingCircuitFlow, MWT from FlowTemp/ReturnTemp, PV from P3 CT.

### Live Control Path

Open-Meteo forecast drives the outer loop, which solves a trajectory-aware target flow and may launch DHW based on T1 plus Powerwall telemetry.

The path is: Open-Meteo forecast → outer loop → trajectory-aware [[src/thermal/display.rs#bisect_mwt_for_room]] solve → target flow → inner loop → eBUS `Hc1HeatCurve` write. The same outer loop queries Influx for DHW T1 plus the `energy-hub` headroom topic `emon/tesla/discretionary_headroom_to_next_cosy_kWh` (alongside raw Powerwall telemetry for observability) and logs decisions to InfluxDB + local JSONL.

Mobile controls: phone → z2m-hub (:3030) `/api/heating/*` → HTTP proxy → adaptive-heating-mvp (:3031) `/mode/*`, `/status`, `/kill`.

### History Evidence Path

InfluxDB compact queries → [[src/thermal/history.rs#heating_history_summary]] / [[src/thermal/history.rs#dhw_history_summary]] → fused evidence summaries. `history-review` ([[src/main.rs#run_history_review]]) adds heuristic verdicts and optional day-rounded `dhw_sessions` context.

## Configuration

Four active config artifacts define four separate concerns.

| File | Used by | Concern |
|------|---------|---------|
| `config.toml` | CLI analysis modules | Domain constants, thresholds, feed IDs, radiators |
| `model/thermal-config.toml` | Thermal model + history commands | InfluxDB, test nights, calibration bounds |
| `model/adaptive-heating-mvp.toml` | Adaptive controller | eBUS host, InfluxDB, Cosy windows, baseline, inner loop tuning |
| `artifacts/thermal/regression-thresholds.toml` | `thermal-regression-check` | Artifact regression gates |

`data/canonical/thermal_geometry.json` is the single source of truth for room geometry, consumed by both the thermal solver and the adaptive controller. `model/control-table.json` is legacy — no longer loaded (replaced by live solver in Phase 1b).

## Documentation Topology

Markdown in this repo has distinct roles to avoid conflicting truths.

- `lat.md/` holds current structured truth for architecture, domain rules, constraints, infrastructure, and controller behaviour
- `docs/` and top-level markdown provide human explanations, runbooks, reference evidence, and historical audits; they should point back to `lat.md/` for live facts
- when first-party docs are condensed, durable operator/reference detail should stay in the active docs and old wording can be recovered from git history rather than a permanent mirror
- `docs/code-truth/` maps implementation structure from source and is useful for file discovery, not for live operating policy
- vendored submodule docs remain upstream references and are not reconciled against project `lat.md/`

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

### History Review Session Scope

`history-review` only includes `dhw_sessions` on rolling day windows. Exact `since`/`until` windows omit that summary because `dhw_sessions` is currently day-rounded and could include out-of-window evidence.
