# Architecture

Three Rust binaries sharing a thermal solver library. Data flows from emoncms and TSDB-backed house telemetry through analysis and modelling to live VRC 700 control.

## Binaries

The project produces three binaries from a single Cargo workspace with a library crate.

- **`heatpump-analysis`** (default binary): CLI for data sync, state classification, COP analysis, thermal modelling, DHW session analysis, and history review. ~1,411 lines in `main.rs` + modules.
- **`adaptive-heating-mvp`**: long-lived service on pi5data. Model-predictive heating controller with HTTP API. ~2,574 lines. Depends on the thermal solver via `heatpump_analysis::thermal::bisect_mwt_for_room()`.
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
  ├── octopus.rs     (reads config.toml octopus.data_dir)
  ├── overnight.rs   (backtest model)
  └── thermal.rs     (facade → 17 submodules, own ThermalConfig)

adaptive-heating-mvp.rs (separate binary)
  ├── heatpump_analysis::thermal (library crate)
  ├── model/adaptive-heating-mvp.toml (own config)
  ├── ebusd TCP :8888 (VRC 700 reads/writes)
  ├── TimescaleDB / PostgreSQL (target latest-value reads + decision-log mirror)
  ├── InfluxDB HTTP :8086 (legacy coexistence / migration tail)
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

The thermal model reads through `thermal/influx.rs`, a shared TSDB seam that preserves typed contracts while PostgreSQL replaces the old Influx-first paths.

On pi5data the path is TSDB store → `thermal/influx.rs` → calibration/validation/operational → `artifacts/thermal/*.json`. Room temps come from Zigbee, outside from eBUS, HP state from BuildingCircuitFlow, MWT from FlowTemp/ReturnTemp, and PV from P3 CT.

### Live Control Path

Open-Meteo forecast drives the outer loop, which solves for minimum-electrical-input flow temp via the thermal model and may launch DHW based on T1 plus Powerwall telemetry.

The path is: Open-Meteo forecast → outer loop → trajectory-aware [[src/thermal/display.rs#bisect_mwt_for_room]] solve → target flow → inner loop → eBUS `Hc1HeatCurve` write. The controller now requires PostgreSQL for latest-value reads and mirrors each decision row into TimescaleDB, with local JSONL logging side-by-side.

Mobile controls: phone → z2m-hub (:3030) `/api/heating/*` → HTTP proxy → adaptive-heating-mvp (:3031) `/mode/*`, `/status`, `/kill`.

### History Evidence Path

`thermal/history.rs` now uses the shared TSDB seam, with PostgreSQL as the intended path for representative history reads when `[postgres]` is configured.

`history-review` ([[src/main.rs#run_history_review]]) adds heuristic verdicts and optional day-rounded `dhw_sessions` context. History evidence remains PostgreSQL-first; any remaining Flux compatibility or parity tail work stays tracked in [[tsdb-migration]] rather than the default operator path.

## Configuration

Four active config artifacts define four separate concerns.

| File | Used by | Concern |
|------|---------|---------|
| `config.toml` | CLI analysis modules | Domain constants, thresholds, feed IDs, radiators, battery coverage assumption, Octopus data path |
| `model/thermal-config.toml` | Thermal model + history commands | Influx connection plus PostgreSQL conninfo for the TSDB seam, test nights, calibration bounds |
| `model/adaptive-heating-mvp.toml` | Adaptive controller | eBUS host, legacy Influx compatibility settings plus PostgreSQL conninfo, fallback Cosy windows, baseline, inner loop tuning |
| `artifacts/thermal/regression-thresholds.toml` | `thermal-regression-check` | Artifact regression gates |

`data/canonical/thermal_geometry.json` is the single source of truth for room geometry, consumed by both the thermal solver and the adaptive controller. `model/control-table.json` is legacy — no longer loaded (replaced by live solver in Phase 1b).

Tariff rate lookup and window discovery use the shared `octopus-tariff` crate (`~/github/octopus-tariff`), declared as a path dependency in `Cargo.toml`. [[src/octopus_tariff.rs]] is a thin re-export module that re-exports `TariffBook`, `CachedTariffWindows`, `TariffTimeWindow`, `format_windows`, `naive_time_to_night_offset`, `OctopusCredentials`, `RateInterval`, and `AgreementMinRate` from the shared crate, keeping `crate::octopus_tariff::*` paths stable inside heatpump-analysis. Unit rates AND tariff window times (Cosy + peak) come from the Octopus account API at runtime. The live controller caches the window structure as JSON at `tariff_cache_path` (default `~/.local/state/adaptive-heating-mvp/tariff-windows.json`); the cache is refreshed when older than 12 hours. `model/adaptive-heating-mvp.toml` `[[dhw.cosy_windows]]` entries are TOML fallbacks only — used when the API is unreachable at startup. `config.toml` retains only the battery-coverage assumption for pricing battery-backed non-lowest-rate demand.

## Documentation Topology

Markdown in this repo has distinct roles to avoid conflicting truths.

- `lat.md/` holds current structured truth for architecture, domain rules, constraints, infrastructure, and controller behaviour
- `plan.md` carries the newest operational truth for active items, and any operational fact that remains relevant should eventually be reconciled back into the appropriate thematic `lat.md/` file
- `docs/` and top-level markdown provide human explanations, runbooks, and other non-canonical narrative material; they should point back to `lat.md/` for live facts
- when first-party docs are condensed, durable operator/reference detail should stay in the active docs and old wording can be recovered from git history rather than a permanent mirror
- `docs/implementation-maps/` preserves the retired code-truth snapshots for human onboarding and file discovery, while `lat.md/src/` holds any file-level source pages that the project keeps alongside the thematic graph; otherwise use the source tree directly for implementation discovery
- vendored submodule docs remain upstream references and are not reconciled against project `lat.md/`

## Implicit Contracts

Assumptions that are not enforced by the type system but will break the system if violated.

### eBUS Availability

ebusd must be running on localhost:8888. If down: reads error → `missing_core = true` → control skipped. No retry beyond cycle interval.

### TSDB Topic and Field Naming

Room temperature topics must match between Telegraf/MQTT ingest, `adaptive-heating-mvp.toml` `[topics]`, and `thermal-config.toml` sensor_topics.

The emonth2 uses the `value` field while Zigbee sensors use `temperature`; PostgreSQL routing preserves that distinction from the legacy Influx schema.

### VRC 700 Baseline Safety

Startup and shutdown writes ensure the VRC 700 can resume autonomous operation if the controller crashes.

Startup: `Z1OpMode=night` + `Hc1MinFlowTempDesired=19`. Shutdown: `Z1OpMode=auto` + `Hc1HeatCurve=0.55` + `Hc1MinFlowTempDesired=20`. Normal crash: house at 19°C with last curve — safe. Coast crash (`Z1OpMode=off`): heating is off — `heating_off` flag tracks this, restore points re-enable on next waking/preheat.

### Thermal Solver Compilation

Since Phase 1b, `adaptive-heating-mvp` calls `bisect_mwt_for_room()` directly from the library crate. `thermal_geometry.json` must be deployed alongside the binary on pi5data. Solver runs in <1ms on ARM.

### Deployment Workflow

Dev on laptop (fast `cargo check`), release build natively on pi5data (correct glibc). Cross-compile from WSL2 fails due to glibc version mismatch (host 2.39 vs pi5data bookworm 2.36).

`scripts/sync-to-pi5data.sh` now syncs the controller-specific sources directly onto the matching remote paths: `src/bin/adaptive-heating-mvp.rs`, thermal modules, `lib.rs`, `thermal_geometry.json`, `model/control-table.json`, config, `src/octopus_tariff.rs`, and the full `~/github/octopus-tariff/` path dependency to `~/github/octopus-tariff/` on pi5data. The remote build now runs `cargo build --release --bin adaptive-heating-mvp`, so the produced artifact already lands at `target/release/adaptive-heating-mvp` and systemd executes the exact freshly built path without an extra copy step. Service restart: `sudo systemctl restart adaptive-heating-mvp`.

For TSDB migration rehearsals that must not touch the live service, `scripts/stage-controller-tsdb-verify.sh` layers on top of that workflow: it syncs, preserves the previous release binary on `pi5data`, builds the current controller release, writes a separate `model/adaptive-heating-mvp.postgres-verify.toml`, and runs a read-only PostgreSQL predeploy check against that same freshly built `target/release/adaptive-heating-mvp` artifact without restarting systemd.

When a real runtime window is required, `scripts/run-controller-tsdb-verify-window.sh` performs the next-safe step: it temporarily stops the main service, launches the already-fresh `target/release/adaptive-heating-mvp` path in a transient systemd unit with PostgreSQL conninfo only, then restores baseline and restarts the main service so the verification path is exercised without changing the permanent systemd unit or checked-in config.

### History Review Session Scope

`history-review` only includes `dhw_sessions` on rolling day windows. Exact `since`/`until` windows omit that summary because `dhw_sessions` is currently day-rounded and could include out-of-window evidence.
