<!-- code-truth: 1c2a44a -->

# Repository Map

## Top-Level Files

| File | Concern |
|------|---------|
| `config.toml` | All domain constants, thresholds, feed IDs, house data, radiator inventory, Arotherm specs, gas-era history |
| `Cargo.toml` | Dependencies and build configuration (three binaries: `heatpump-analysis`, `thermal-regression-check`, `adaptive-heating-mvp`) |
| `AGENTS.md` | LLM agent context (canonical project documentation) |
| `README.md` | Human-facing quick start, command reference, project philosophy |
| `heatpump.db` | SQLite database (gitignored, created by `sync`) |
| `heating-monitoring-setup.md` | Full monitoring infrastructure documentation |
| `.gitmodules` | Six submodules: ebusd, avrdb_firmware, EmonScripts, emonhub, emoncms, emonPiLCD |

## Source Modules

### `src/main.rs` — CLI entry point (~704 lines)

Defines 28+ CLI subcommands via clap derive. Loads `config.toml` at startup. Routes to analysis functions, DB operations, thermal commands.

### `src/config.rs` — Configuration (213 lines)

Deserialises `config.toml` into typed structs. Global singleton via `once_cell::OnceCell`. All modules access via `config::config()`.

### `src/emoncms.rs` — API client (82 lines)

Minimal HTTP client for emoncms.org REST API. Used only by `sync` command. Blocking reqwest with 100ms politeness delay.

### `src/db.rs` — SQLite storage (507 lines)

Three tables: `feeds`, `samples` (WITHOUT ROWID), `sync_state`. WAL mode. `load_dataframe()` and `load_dataframe_with_simulated()` are the two DataFrame loading paths.

### `src/analysis.rs` — State machine + Polars queries (986 lines)

Core HP analysis. `classify_states()` implements the hysteresis state machine. All analysis functions take enriched DataFrames and print to stdout.

### `src/gaps.rs` — Gap detection + synthetic data (648 lines)

`TempBinModel` builds temperature-bin power profiles. Bypasses `db.rs` — manages own schema.

### `src/octopus.rs` — Octopus Energy integration (814 lines)

Reads from `~/github/octopus/data/`. Gas-vs-HP comparison, baseload analysis.

### `src/overnight.rs` — Overnight strategy optimizer (1,479 lines)

Backtest model for overnight heating strategies.

### `src/lib.rs` — Library crate entry (2 lines)

Exposes `pub mod thermal` so standalone binaries (`adaptive-heating-mvp`) can call solver functions via `heatpump_analysis::thermal::bisect_mwt_for_room()`.

### `src/thermal.rs` — Thin facade (34 lines)

Re-exports public entry points from 16 submodules. Solver re-exports: `bisect_mwt_for_room`, `solve_equilibrium_temps`, `ThermalError`, `ThermalResult`.

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
| `display.rs` | 993 | CLI output, **equilibrium solver**, **MWT bisection**, **control table generation** |
| `report.rs` | 44 | Table printer and RMSE |
| `history.rs` | ~2230 | Heating/DHW history reconstruction from InfluxDB. Comfort miss detection (clipped to 07:00–23:00 waking hours), DHW overlap detection, controller event extraction |
| `dhw_sessions.rs` | ~1050 | DHW draw/charge session analysis: inflection detection, draw type classification (bath/shower/tap), HWC tracking, during-charge draw detection |

## Standalone Binaries

### `src/bin/adaptive-heating-mvp.rs` (~2,030 lines)

Live V2 adaptive heating controller. Two-loop architecture:

**Outer loop** (every 900s): Open-Meteo forecast → live thermal solver (`bisect_mwt_for_room`) → target flow temp → initial curve guess via heat curve formula.

**Inner loop** (every 60s): proportional feedback on `Hc1ActualFlowTempDesired` toward target flow. Gain 0.05, deadband 0.5°C, max step 0.20. Floor guard: halve gain + double deadband when curve < 0.25.

Key components:
- `calculate_required_curve()` — forecast + live solver → MWT → flow → curve. Uses default ΔT when compressor not actively heating (ΔT stabilisation fix).
- `run_outer_cycle()` — full sensor sweep, mode-specific control logic, writes initial curve
- `run_inner_cycle()` — light eBUS reads, proportional curve adjustment
- `restore_baseline()` — write `Hc1HeatCurve=0.55` + `Z1OpMode=auto` on shutdown
- HTTP API: `/status`, `/mode/{mode}`, `/kill`
- Modes: `Occupied`, `ShortAbsence`, `AwayUntil`, `Disabled`, `MonitorOnly`

On startup: `Z1OpMode=night` (SP=19). This eliminates VRC 700 Optimum Start and day/night transitions.

### `src/bin/thermal-regression-check.rs` (607 lines)

Compares fresh thermal artifacts against baseline JSON files. 4 artifact types.

### `src/bin/cosy-scheduler.rs` (162 lines)

**Retired.** Source kept for reference. Binary removed from pi5data.

## Configuration

| File | Used by | Concern |
|------|---------|---------|
| `config.toml` | `src/main.rs`, `src/config.rs`, most modules | Domain constants, thresholds, feed IDs |
| `model/thermal-config.toml` | `src/thermal/config.rs` | InfluxDB, test nights, calibration bounds |
| `model/adaptive-heating-mvp.toml` | `src/bin/adaptive-heating-mvp.rs` | eBUS host, InfluxDB, Cosy windows, baseline values, room topics, inner loop tuning |
| `model/control-table.json` | **No longer loaded** | Legacy MWT lookup table (replaced by live solver in Phase 1b) |
| `artifacts/thermal/regression-thresholds.toml` | `src/bin/thermal-regression-check.rs` | Regression gates |
| `data/canonical/thermal_geometry.json` | `src/thermal/geometry.rs` | Room geometry single source of truth |

## Domain Docs

| Document | Concern |
|----------|---------|
| `docs/heating-plan.md` | Heating strategy, constraints, parameters, next steps (LLM working memory) |
| `docs/dhw-plan.md` | DHW: strategy, scheduling, capacity, decisions (LLM working memory) |
| `docs/dhw-reference.md` | DHW domain reference: cylinder spec, WWHR, charge traces, usage, z2m-hub algorithm |
| `docs/heating-reference.md` | Heating domain reference: VRC 700, tuning, eBUS registers, deployment |
| `docs/pico-ebus-plan.md` | Pico W eBUS adapter build plan |
| `docs/vrc700-settings-audit.md` | VRC 700 settings, timer encoding, eBUS commands |
| `docs/dhw-plan.md` | (see above) |
| `docs/house-layout.md` | 13 rooms, radiators, ventilation, sensor locations |
| `docs/room-thermal-model.md` | Thermal model methodology |
| `docs/hydraulic-analysis.md` | Flow rate thresholds, sludge filter |

| `docs/explanation.md` | Domain background (HTC, COP, degree days) |
| `docs/emon-installation-runbook.md` | emonPi2/emonhp rebuild procedures |
| `docs/octopus-data-inventory.md` | Octopus consumption data fields |

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
| InfluxDB queries (thermal) | `src/thermal/influx.rs` |
| Artifact schema / output | `src/thermal/artifact.rs` |
| Regression CI gates | `src/bin/thermal-regression-check.rs` + `artifacts/thermal/regression-thresholds.toml` |
| **V2 control logic (outer loop)** | `src/bin/adaptive-heating-mvp.rs::run_outer_cycle()` |
| **V2 control logic (inner loop)** | `src/bin/adaptive-heating-mvp.rs::run_inner_cycle()` |
| **V2 forecast + model → curve** | `src/bin/adaptive-heating-mvp.rs::calculate_required_curve()` → `heatpump_analysis::thermal::bisect_mwt_for_room()` |
| **Adaptive heating config** | `model/adaptive-heating-mvp.toml` |
| **Adaptive heating modes/API** | `src/bin/adaptive-heating-mvp.rs` (HTTP handlers + Mode enum) |
| **Adaptive heating baseline** | `model/adaptive-heating-mvp.toml` `[baseline]` + `restore_baseline()` |
| **Heating strategy + constraints** | `docs/heating-plan.md` (plan) + `docs/heating-reference.md` (reference) |
| **DHW scheduling + duration model** | `docs/dhw-plan.md` (plan) + `docs/dhw-reference.md` (reference) |
| **DHW session analysis** | `src/thermal/dhw_sessions.rs` |
| **Mobile dashboard** | `~/github/z2m-hub/src/main.rs` (HOME_PAGE + proxy routes) |
| eBUS polling | `scripts/ebusd-poll.sh` on pi5data |
| Octopus data refresh | `~/github/octopus/` — `npm run cli -- refresh` |
| VRC 700 settings | `docs/vrc700-settings-audit.md` |
| Monitoring infrastructure | `heating-monitoring-setup.md`, `docs/emon-installation-runbook.md` |
