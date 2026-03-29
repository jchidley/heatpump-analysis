<!-- code-truth: f9694e2 -->

# Repository Map

## Top-Level Files

| File | Concern |
|------|---------|
| `config.toml` | All domain constants, thresholds, feed IDs, house data, radiator inventory, Arotherm specs, gas-era history |
| `Cargo.toml` | Dependencies and build configuration (two binaries: `heatpump-analysis`, `thermal-regression-check`) |
| `AGENTS.md` | LLM agent context (canonical project documentation, ~440 lines) |
| `CLAUDE.md` | Points to AGENTS.md |
| `README.md` | Human-facing quick start, command reference, project philosophy |
| `heatpump.db` | SQLite database (gitignored, created by `sync`) |
| `heating-monitoring-setup.md` | Full monitoring infrastructure documentation (devices, MQTT topics, eBUS data, credentials) |
| `.gitmodules` | Six submodules: ebusd, avrdb_firmware, EmonScripts, emonhub, emoncms, emonPiLCD |

## Source Modules

### `src/main.rs` — CLI entry point (599 lines)

Defines 25 CLI subcommands via clap derive. Loads `config.toml` at startup. Routes to analysis functions, DB operations, thermal commands.

### `src/config.rs` — Configuration (213 lines)

Deserialises `config.toml` into typed structs. Global singleton via `once_cell::OnceCell`. All modules access via `config::config()`.

### `src/emoncms.rs` — API client (82 lines)

Minimal HTTP client for emoncms.org REST API. Used only by `sync` command. Blocking reqwest with 100ms politeness delay.

### `src/db.rs` — SQLite storage (507 lines)

Three tables: `feeds`, `samples` (WITHOUT ROWID), `sync_state`. WAL mode. `load_dataframe()` and `load_dataframe_with_simulated()` are the two DataFrame loading paths. Also provides `load_daily_outside_temp()` and `load_daily_energy()` for degree-day analysis.

### `src/analysis.rs` — State machine + Polars queries (986 lines)

Core HP analysis. `classify_states()` implements the hysteresis state machine (flow rate → heating/DHW/defrost/idle). `enrich()` adds `cop`, `delta_t`, `state` columns. All analysis functions take enriched DataFrames and print to stdout.

### `src/gaps.rs` — Gap detection + synthetic data (648 lines)

`TempBinModel` builds temperature-bin power profiles from real data. `fill_gap()` generates synthetic samples scaled to match cumulative meter readings. Writes to separate `simulated_samples` table. Bypasses `db.rs` — manages own schema.

### `src/octopus.rs` — Octopus Energy integration (814 lines)

Reads from `~/github/octopus/data/` (CSV + JSON). Gas-vs-HP comparison, baseload analysis, monthly breakdown with HDD. ERA5 bias correction (+1.0°C) as Rust constant.

### `src/overnight.rs` — Overnight strategy optimizer (1,442 lines)

Backtest model for overnight heating strategies. Calibrated cooling model (k=0.039/hr from DHW events), three-rate Cosy tariff, battery coverage. Evaluates 30 strategies × 324 winter nights.

### `src/thermal.rs` — Thermal model (3,506 lines)

The largest module. Room-level thermal network: 13 rooms, fabric U×A, radiators, ventilation, doorway exchange, solar gain. Five public entry points:

| Function | Command | What it does |
|----------|---------|-------------|
| `calibrate()` | `thermal-calibrate` | Grid search over Cd + landing ACH against Night 1/Night 2 cooldown rates |
| `validate()` | `thermal-validate` | Run calibrated model on holdout windows, check pass/fail thresholds |
| `fit_diagnostics()` | `thermal-fit-diagnostics` | Period-by-period cooldown diagnostics from HP status codes |
| `operational_validate()` | `thermal-operational` | Full operational validation with heating/DHW/off, solar gain, BCF-based state |
| `snapshot_export()` / `snapshot_import()` | `thermal-snapshot` | Human-gated reproducibility workflow |

Key internal functions:
- `room_energy_balance()` — cooldown-only balance (radiators=0, solar=0)
- `full_room_energy_balance()` — operational balance with MWT, radiators, solar, body heat
- `solar_gain_full()` — per-room solar from PV (SW) + Open-Meteo DNI/DHI (NE)
- `classify_hp_state_from_flow()` — BCF-based state classification (>900=DHW, 780-900=heating, <100=off)
- `build_rooms()` / `build_connections()` / `build_doorways()` — house definition from `thermal_geometry.json`

### `src/thermal/error.rs` — Typed domain errors (99 lines)

`ThermalError` enum with `thiserror` derive. 20+ variants covering config, InfluxDB, parsing, calibration, and snapshot errors.

### `src/thermal/influx.rs` — InfluxDB queries (352 lines)

Flux query builders for room temps, outside temp, HP status codes, PV power, building circuit flow, MWT. Parses annotated CSV responses.

### `src/thermal/report.rs` — Output formatting (44 lines)

Table printer and RMSE calculator shared across thermal commands.

## Standalone Binaries

### `src/bin/thermal-regression-check.rs` (525 lines)

Compares fresh thermal artifacts against baseline JSON files. Checks per-room RMSE/bias drift against thresholds in `artifacts/thermal/regression-thresholds.toml`.

### `src/bin/cosy-scheduler.rs` (163 lines)

Deployed to pi5data. Reads outside temp from eBUS, logs DHW recommendations. Zero dependencies (pure std). Currently unused — VRC 700 timer handles scheduling.

### `cosy-scheduler/` (separate Cargo workspace member)

Cross-compiled for aarch64-unknown-linux-musl. Same code as `src/bin/cosy-scheduler.rs` but with its own `Cargo.toml` for cross-compilation.

## Python Model

### `model/house.py` (1,239 lines)

Lumped-parameter thermal network. 13 rooms with fabric, radiators, ventilation, doorways, solar. Commands: `fetch`, `rooms`, `connections`, `analyse`, `fit`, `equilibrium`, `moisture`.

### `model/calibrate.py` (340 lines)

Fitting thermal parameters from controlled cooldown data. Outputs calibrated Cd and landing ACH.

### `model/overnight.py` (692 lines)

Initial Python overnight model. Superseded by `src/overnight.rs`.

### `model/extract_house_inventory.py` (1,531 lines)

Extracts dimensional data from XLSX scans and building plans. Produces `model/data/inventory/` artifacts and `data/canonical/thermal_geometry.json`.

### `model/audit_model_dimensions.py` (123 lines)

Verifies Python and Rust wiring to canonical geometry. Checks 509 geometry/provenance fields.

## Configuration

### `config.toml` — Domain constants

Six sections: `emoncms` (feeds, sync), `thresholds` (state machine, HDD), `house` (HTC, floor area), `arotherm` (spec curves), `radiators` (15 entries), `gas_era` (monthly gas data).

### `model/thermal-config.toml` — Thermal model config

InfluxDB connection, test night windows, objective function config (excluded rooms, prior weight), calibration bounds/steps, validation windows, fit diagnostics config, wind model (disabled).

### `artifacts/thermal/regression-thresholds.toml` — Regression gates

Per-room absolute RMSE thresholds and delta thresholds for regression checks.

### `data/canonical/thermal_geometry.json` — Room geometry

Single source of truth for room dimensions, external fabric, internal connections, doorways, solar glazing. Consumed by both Python (`model/house.py`) and Rust (`src/thermal.rs`). Provenance tracked.

## Scripts

| Script | Location | Deployment | Purpose |
|--------|----------|-----------|---------|
| `scripts/ebusd-poll.sh` | `ebusd-poll` systemd on pi5data | `scp` + `systemctl restart` | Reads 25+ eBUS values every 30s via `nc`, publishes to MQTT |
| `scripts/ebusd-poll.service` | `/etc/systemd/system/` on pi5data | Part of ebusd-poll deploy | Systemd unit |
| `scripts/backup-sdcard.sh` | Run on imaging host (pi5nvme) | Manual | dd → PiShrink → xz backup pipeline |
| `scripts/thermal-regression-ci.sh` | Local/CI | `bash scripts/thermal-regression-ci.sh` | Runs thermal commands + regression check against baselines |
| `scripts/refresh-thermal-baselines.sh` | Local | After intentional model changes | Generates fresh baseline artifacts |

## Artifacts

```
artifacts/thermal/
  regression-thresholds.toml           # Per-room RMSE thresholds
  baselines/
    thermal-calibrate-baseline.json    # Reference calibration output
    thermal-validate-baseline.json     # Reference validation output
    thermal-fit-diagnostics-baseline.json
    README.md
  snapshots/                           # Human-signed reproducibility snapshots
    thermal-snapshot-*/
      manifest.json
      files/                           # Config + thresholds at snapshot time
```

## Concern Mapping

| Change | Look in |
|--------|---------|
| Operating state thresholds | `config.toml` `[thresholds]`, then `analysis.rs::classify_states()` |
| Feed IDs or column names | `config.toml` `[emoncms.feeds]`, then `db.rs` and `analysis.rs` |
| Radiator data | `config.toml` `[radiators]` AND `model/house.py` `build_rooms()` AND `data/canonical/thermal_geometry.json` |
| Room geometry / fabric | `data/canonical/thermal_geometry.json` (consumed by both Rust and Python) |
| Thermal calibration bounds | `model/thermal-config.toml` `[bounds]` |
| eBUS polling | `scripts/ebusd-poll.sh` on pi5data |
| Octopus data refresh | `~/github/octopus/` project — `npm run cli -- refresh` |
| DHW tracking/boost | `~/github/z2m-hub/` project |
| Zigbee automations | `~/github/z2m-hub/` project |
| Monitoring infrastructure | `heating-monitoring-setup.md`, `docs/emon-installation-runbook.md` |

## Git Submodules (upstream reference only)

| Submodule | Purpose |
|-----------|---------|
| `avrdb_firmware/` | AVR-DB firmware hex files for flashing EmonPi2/EmonTx |
| `EmonScripts/` | emonSD install/update scripts |
| `emonhub/` | Data multiplexer (serial/MBUS/MQTT interfacers) |
| `ebusd/` | eBUS daemon CSV config files |
| `emoncms/` | Web framework (reference, not deployed on most devices) |
| `emonPiLCD/` | OLED/LCD display + button handler for emonpi |
