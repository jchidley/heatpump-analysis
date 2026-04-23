<!-- code-truth: 9c24a09 -->

# Repository Map

## Top-Level Files

| File | Concern |
|------|---------|
| `config.toml` | All domain constants, thresholds, feed IDs, house data, radiator inventory, Arotherm specs, gas-era history |
| `Cargo.toml` | Dependencies and build configuration (three binaries: `heatpump-analysis`, `thermal-regression-check`, `adaptive-heating-mvp`) |
| `AGENTS.md` | Agent workflow and doc-routing rules; points to `lat.md/` for current project truth |
| `README.md` | Human-facing quick start, command reference, and documentation signpost |
| `heatpump.db` | SQLite database (gitignored, created by `sync`) |
| `heating-monitoring-setup.md` | Operational deep-dive and runbook detail beyond the `lat.md/` infrastructure summary |
| `.gitmodules` | Six submodules: ebusd, avrdb_firmware, EmonScripts, emonhub, emoncms, emonPiLCD |

## Source Modules

### `src/main.rs` — CLI entry point (~1,417 lines)

Defines 28+ CLI subcommands via clap derive. Loads `config.toml` at startup. Routes to analysis functions, DB operations, thermal commands.

### `src/config.rs` — Configuration (227 lines)

Deserialises `config.toml` into typed structs. Global singleton via `once_cell::OnceCell`. All modules access via `config::config()`.

### `src/emoncms.rs` — API client (82 lines)

Minimal HTTP client for emoncms.org REST API. Used only by `sync` command. Blocking reqwest with 100ms politeness delay.

### `src/db.rs` — SQLite storage (508 lines)

Three tables: `feeds`, `samples` (WITHOUT ROWID), `sync_state`. WAL mode. `load_dataframe()` and `load_dataframe_with_simulated()` are the two DataFrame loading paths.

### `src/analysis.rs` — State machine + Polars queries (1,060 lines)

Core HP analysis. `classify_states()` implements the hysteresis state machine. All analysis functions take enriched DataFrames and print to stdout.

### `src/gaps.rs` — Gap detection + synthetic data (655 lines)

`TempBinModel` builds temperature-bin power profiles. Bypasses `db.rs` — manages own schema.

### `src/octopus.rs` — Octopus Energy consumption + weather integration (818 lines)

Reads from `~/github/octopus/data/`. Gas-vs-HP comparison, baseload analysis, consumption/weather summaries.

### `src/octopus_tariff.rs` — Re-export of shared `octopus-tariff` crate (11 lines)

Thin re-export of the shared `octopus-tariff` crate (`~/github/octopus-tariff`). The 627-line local implementation was replaced with an 11-line module that re-exports the crate's public API.

### `src/overnight.rs` — Overnight strategy optimizer (1,578 lines)

Backtest model for overnight heating strategies, priced with account-derived historical tariff rates via `src/octopus_tariff.rs`.

### `src/lib.rs` — Library crate entry (3 lines)

Exposes `pub mod thermal` so standalone binaries (`adaptive-heating-mvp`) can call solver functions via `heatpump_analysis::thermal::bisect_mwt_for_room()`.

### `src/thermal.rs` — Thin facade (34 lines)

Re-exports public entry points from 17 submodules. Solver re-exports: `bisect_mwt_for_room`, `solve_equilibrium_temps`, `ThermalError`, `ThermalResult`.

### `src/thermal/` — Thermal model submodules (~5,500 lines total)

Room-level thermal network: 13 rooms, fabric U×A, radiators, ventilation, doorway exchange, solar gain.

| Module | Lines | Responsibility |
|--------|-------|---------------|
| `config.rs` | 213 | TOML config structs for thermal model |
| `geometry.rs` | 269 | Room/connection/doorway types + JSON loading |
| `physics.rs` | 400 | Constants, thermal mass, energy balance |
| `solar.rs` | 180 | Solar position + irradiance |
| `wind.rs` | 75 | Open-Meteo wind + ventilation multiplier |
| `calibration.rs` | 626 | Grid search calibration, shared helpers |
| `validation.rs` | 435 | Metrics, residuals, holdout validation |
| `diagnostics.rs` | 444 | Cooldown detection + fit diagnostics |
| `operational.rs` | 551 | HP state classification, operational validation |
| `artifact.rs` | 224 | Artifact types, git metadata, build/write |
| `snapshot.rs` | 233 | Export/import manifests with human signoff |
| `error.rs` | 99 | `ThermalError` enum |
| `influx.rs` | 352 | Flux query builders |
| `display.rs` | 1014 | CLI output, **equilibrium solver**, **MWT bisection**, **control table generation** |
| `report.rs` | 44 | Table printer and RMSE |
| `history.rs` | ~2259 | Heating/DHW history reconstruction from the shared TSDB seam, with PostgreSQL-first reads plus a remaining legacy Flux compatibility tail. Comfort miss detection (clipped to 07:00–23:00 waking hours), DHW overlap detection, controller event extraction |
| `dhw_sessions.rs` | ~1169 | DHW draw/charge session analysis: inflection detection, draw type classification (bath/shower/tap), HWC tracking, during-charge draw detection |

## Standalone Binaries

### `src/bin/adaptive-heating-mvp.rs` (~2,053 lines)

Live V2 adaptive heating controller. Two-loop architecture:

**Outer loop** (every 900s): Open-Meteo forecast → live thermal solver (`bisect_mwt_for_room`) → target flow temp → initial curve guess via heat curve formula.

**Inner loop** (every 60s): proportional feedback on `Hc1ActualFlowTempDesired` toward target flow. Gain 0.05, deadband 0.5°C, max step 0.20. Floor guard: halve gain + double deadband when curve < 0.25.

Key components:
- `calculate_required_curve()` — forecast + live solver → MWT → flow → curve. Uses default ΔT when compressor not actively heating (ΔT stabilisation fix).
- `run_outer_cycle()` — full sensor sweep, mode-specific control logic, writes initial curve
- `run_inner_cycle()` — light eBUS reads, proportional curve adjustment
- `restore_baseline()` — write `Hc1HeatCurve=0.55` + `Z1OpMode=auto` + `Hc1MinFlowTempDesired=20` on shutdown
- HTTP API: `/status`, `/mode/{mode}`, `/kill` (toggle restore/resume)
- Modes: `Occupied`, `ShortAbsence`, `AwayUntil`, `Disabled`, `MonitorOnly`

On startup: `Z1OpMode=night` (SP=19) + `Hc1MinFlowTempDesired=19` when persisted mode is active. If persisted mode is `Disabled` or `MonitorOnly`, startup skips eBUS initialisation. This eliminates Optimum Start, day/night transitions, and the hidden 20°C flow floor that prevented genuine coast while preserving a true disabled baseline.

### `src/bin/thermal-regression-check.rs` (607 lines)

Compares fresh thermal artifacts against baseline JSON files. 4 artifact types.

## Configuration

| File | Used by | Concern |
|------|---------|---------|
| `config.toml` | `src/main.rs`, `src/config.rs`, most modules | Domain constants, thresholds, feed IDs, battery-coverage assumption for tariff blending |
| `model/thermal-config.toml` | `src/thermal/config.rs` | PostgreSQL + legacy Flux/Influx tail settings, test nights, calibration bounds |
| `model/adaptive-heating-mvp.toml` | `src/bin/adaptive-heating-mvp.rs` | eBUS host, PostgreSQL conninfo, Cosy windows, baseline values, room topics, inner loop tuning |
| `model/control-table.json` | **No longer loaded** | Legacy MWT lookup table (replaced by live solver in Phase 1b) |
| `artifacts/thermal/regression-thresholds.toml` | `src/bin/thermal-regression-check.rs` | Regression gates |
| `data/canonical/thermal_geometry.json` | `src/thermal/geometry.rs` | Room geometry single source of truth |

## Domain Docs

| Document | Concern |
|----------|---------|
| `docs/heating-plan.md` | Human entry point to heating strategy; defers current-state truth to `lat.md/heating-control.md` |
| `docs/dhw-plan.md` | Human entry point to DHW strategy; defers current-state truth to `lat.md/domain.md` + `lat.md/heating-control.md` |
| `docs/dhw-reference.md` | DHW reference and measurement evidence beyond the condensed `lat.md` summary |
| `docs/heating-reference.md` | Heating reference and field evidence beyond the condensed `lat.md` summary |
| `docs/pico-ebus-plan.md` | Pico W eBUS adapter build plan |
| `docs/vrc700-settings-audit.md` | VRC 700 settings, timer encoding, eBUS commands |
| `docs/dhw-plan.md` | (see above) |
| `docs/house-layout.md` | 13 rooms, radiators, ventilation, sensor locations |
| `docs/room-thermal-model.md` | Thermal model methodology |
| `docs/hydraulic-analysis.md` | Flow rate thresholds, sludge filter |

| `docs/explanation.md` | Domain background (HTC, COP, degree days) |
| `docs/emon-installation-runbook.md` | emonPi2/emonhp rebuild procedures |
| `docs/octopus-data-inventory.md` | Octopus consumption data fields |

## lat.md/ (Agent-Facing Knowledge Graph)

| File | Concern |
|------|--------|
| `lat.md/lat.md` | Root index |
| `lat.md/domain.md` | Domain model: operating states, DHW cylinder, household usage |
| `lat.md/constraints.md` | Hard constraints: eBUS, VRC 700, tariff, sensors |
| `lat.md/architecture.md` | Data flow, binary structure, library crate |
| `lat.md/heating-control.md` | V2 controller: two-loop, overnight, modes, pilot history |
| `lat.md/thermal-model.md` | Calibration, validation, solver, parameters |
| `lat.md/history-evidence.md` | Review-window defaults, joined evidence workflows, promotion boundaries |
| `lat.md/infrastructure.md` | Devices, MQTT, eBUS, VRC 700 settings |
| `lat.md/plan.md` | Live operational plan and newest status snapshot |
| `lat.md/reviews.md` | Archived dated review snapshots that no longer belong in `plan.md` |
| `lat.md/tests.md` | Executable specs for safety- and migration-sensitive behaviour |

Validated by `lat check`. Cross-linked with `[[wiki refs]]` to source code.

## Concern Mapping

| Change | Look in |
|--------|---------|
| Operating state thresholds | `config.toml` `[thresholds]`, then `analysis.rs::classify_states()` |
| Feed IDs or column names | `config.toml` `[emoncms.feeds]`, then `db.rs` and `analysis.rs` |
| Radiator data | `config.toml` `[radiators]` AND `data/canonical/thermal_geometry.json` (keep in sync) |
| Room geometry / fabric | `data/canonical/thermal_geometry.json` → `src/thermal/geometry.rs` |
| Thermal calibration bounds | `model/thermal-config.toml` `[bounds]` |
| Thermal physics / energy balance | `src/thermal/physics.rs` and `src/thermal/operational.rs` |
| Equilibrium solver / MWT bisection | `src/thermal/display.rs::solve_equilibrium_temps()`, `bisect_mwt_for_room()` |
| Solar gain model | `src/thermal/solar.rs` |
| Legacy Flux/Influx queries (thermal compatibility tail) | `src/thermal/influx.rs` |
| Artifact schema / output | `src/thermal/artifact.rs` |
| Regression CI gates | `src/bin/thermal-regression-check.rs` + `artifacts/thermal/regression-thresholds.toml` |
| **V2 control logic (outer loop)** | `src/bin/adaptive-heating-mvp.rs::run_outer_cycle()` |
| **V2 control logic (inner loop)** | `src/bin/adaptive-heating-mvp.rs::run_inner_cycle()` |
| **V2 forecast + model → curve** | `src/bin/adaptive-heating-mvp.rs::calculate_required_curve()` → `heatpump_analysis::thermal::bisect_mwt_for_room()` |
| **Adaptive heating config** | `model/adaptive-heating-mvp.toml` |
| **Adaptive heating modes/API** | `src/bin/adaptive-heating-mvp.rs` (HTTP handlers + Mode enum) |
| **Adaptive heating baseline** | `model/adaptive-heating-mvp.toml` `[baseline]` + `restore_baseline()` |
| **Heating strategy + constraints** | `lat.md/heating-control.md` + `lat.md/constraints.md`, then `docs/heating-plan.md` / `docs/heating-reference.md` for human context |
| **DHW scheduling + duration model** | `lat.md/domain.md` + `lat.md/heating-control.md`, then `docs/dhw-plan.md` / `docs/dhw-reference.md` for human context |
| **DHW session analysis** | `src/thermal/dhw_sessions.rs` |
| **Mobile dashboard** | `~/github/z2m-hub/src/main.rs` (HOME_PAGE + proxy routes) |
| eBUS polling | `scripts/ebusd-poll.sh` on pi5data |
| Octopus data refresh | `~/github/octopus/` — `npm run cli -- refresh` |
| Octopus tariff account truth | `src/octopus_tariff.rs` + `~/github/octopus/.envrc` |
| Controller deploy to pi5data | `scripts/sync-to-pi5data.sh`, then native `cargo build --release` on `pi5data` |
| VRC 700 settings | `docs/vrc700-settings-audit.md` |
| Monitoring infrastructure | `heating-monitoring-setup.md`, `docs/emon-installation-runbook.md` |
