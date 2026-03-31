<!-- code-truth: 7b6bfed -->

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

### `src/main.rs` — CLI entry point (~611 lines)

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

### `src/thermal.rs` — Thin facade (23 lines)

Re-exports 8 public entry points from 15 submodules. All implementation is in `src/thermal/`.

### `src/thermal/` — Thermal model submodules (4,222 lines total)

Room-level thermal network: 13 rooms, fabric U×A, radiators, ventilation, doorway exchange, solar gain.

| Module | Lines | Responsibility |
|--------|-------|---------------|
| `config.rs` | 213 | TOML config structs for thermal model |
| `geometry.rs` | 269 | Room/connection/doorway types + JSON loading |
| `physics.rs` | 399 | Constants, thermal mass, energy balance |
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
| `display.rs` | 78 | CLI output for rooms/connections |
| `report.rs` | 44 | Table printer and RMSE |

## Standalone Binaries

### `src/bin/adaptive-heating-mvp.rs` (900 lines)

**NEW.** Live adaptive heating controller. Axum HTTP API + background control loop. Reads eBUS via TCP, InfluxDB for room temps. Writes VRC 700 registers. Logs to InfluxDB + JSONL. Deployed as systemd service on `pi5data` (port 3031). Config: `model/adaptive-heating-mvp.toml`. Service: `deploy/adaptive-heating-mvp.service`.

Key components:
- `run_control_cycle()` — 15-minute decision loop: read state, evaluate mode-specific rules, write levers, log
- `restore_baseline()` — write known-good values to VRC 700 (called on kill/stop)
- `classify_tariff_period()` — Cosy/peak/standard from clock
- HTTP API: `/status`, `/mode/{mode}`, `/kill`
- Modes: `Occupied`, `ShortAbsence`, `AwayUntil`, `Disabled`, `MonitorOnly`

### `src/bin/thermal-regression-check.rs` (607 lines)

Compares fresh thermal artifacts against baseline JSON files. 4 artifact types.

### `src/bin/cosy-scheduler.rs` (162 lines)

**Retired.** Source kept for reference. Binary removed from pi5data.

## Configuration

| File | Used by | Concern |
|------|---------|---------|
| `config.toml` | `src/main.rs`, `src/config.rs`, most modules | Domain constants, thresholds, feed IDs |
| `model/thermal-config.toml` | `src/thermal/config.rs` | InfluxDB, test nights, calibration bounds |
| `model/adaptive-heating-mvp.toml` | `src/bin/adaptive-heating-mvp.rs` | eBUS host, InfluxDB, Cosy windows, baseline values, room topics |
| `artifacts/thermal/regression-thresholds.toml` | `src/bin/thermal-regression-check.rs` | Regression gates |
| `data/canonical/thermal_geometry.json` | `src/thermal/geometry.rs` | Room geometry single source of truth |

## Deploy

| File | Target | Purpose |
|------|--------|---------|
| `deploy/adaptive-heating-mvp.service` | pi5data `/etc/systemd/system/` | systemd unit for adaptive heating MVP |

## Domain Docs

| Document | Concern |
|----------|---------|
| `docs/adaptive-heating-control.md` | Strategy, philosophy, room targeting, control theory |
| `docs/adaptive-heating-mvp.md` | Frozen MVP spec, implementation status, outstanding work |
| `docs/roadmap.md` | Planned enhancements with status |
| `docs/pico-ebus-plan.md` | Pico W eBUS adapter build plan |
| `docs/vrc700-settings-audit.md` | VRC 700 settings, timer encoding, eBUS commands |
| `docs/dhw-fixes.md` | DHW sensor labelling, Grafana, cylinder analysis follow-ups |
| `docs/dhw-cylinder-analysis.md` | 300L Kingspan Albion cylinder analysis |
| `docs/house-layout.md` | 13 rooms, radiators, ventilation, sensor locations |
| `docs/room-thermal-model.md` | Thermal model methodology |
| `docs/hydraulic-analysis.md` | Flow rate thresholds, sludge filter |
| `docs/overnight-strategy-analysis.md` | Overnight heating strategy backtest |
| `docs/rust-migration-plan.md` | Python→Rust migration (complete) |
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
| Solar gain model | `src/thermal/solar.rs` |
| InfluxDB queries (thermal) | `src/thermal/influx.rs` |
| Artifact schema / output | `src/thermal/artifact.rs` |
| Regression CI gates | `src/bin/thermal-regression-check.rs` + `artifacts/thermal/regression-thresholds.toml` |
| **Adaptive heating control logic** | `src/bin/adaptive-heating-mvp.rs::run_control_cycle()` |
| **Adaptive heating config** | `model/adaptive-heating-mvp.toml` |
| **Adaptive heating modes/API** | `src/bin/adaptive-heating-mvp.rs` (HTTP handlers + Mode enum) |
| **Adaptive heating baseline** | `model/adaptive-heating-mvp.toml` `[baseline]` + `restore_baseline()` |
| **Mobile dashboard** | `~/github/z2m-hub/src/main.rs` (HOME_PAGE + proxy routes) |
| eBUS polling | `scripts/ebusd-poll.sh` on pi5data |
| Octopus data refresh | `~/github/octopus/` — `npm run cli -- refresh` |
| VRC 700 settings | `docs/vrc700-settings-audit.md` |
| Monitoring infrastructure | `heating-monitoring-setup.md`, `docs/emon-installation-runbook.md` |
