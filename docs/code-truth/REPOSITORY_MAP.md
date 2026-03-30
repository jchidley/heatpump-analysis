<!-- code-truth: e67fc92 -->

# Repository Map

## Top-Level Files

| File | Concern |
|------|---------|
| `config.toml` | All domain constants, thresholds, feed IDs, house data, radiator inventory, Arotherm specs, gas-era history |
| `Cargo.toml` | Dependencies and build configuration (two binaries: `heatpump-analysis`, `thermal-regression-check`) |
| `AGENTS.md` | LLM agent context (canonical project documentation) |
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

### `src/overnight.rs` — Overnight strategy optimizer (1,479 lines)

Backtest model for overnight heating strategies. Calibrated cooling model (k=0.039/hr from DHW events), three-rate Cosy tariff, battery coverage. Evaluates 30 strategies × 324 winter nights.

### `src/thermal.rs` — Thin facade (23 lines)

Re-exports 8 public entry points from 15 submodules. All implementation is in `src/thermal/`.

### `src/thermal/` — Thermal model submodules (4,247 lines total)

Room-level thermal network: 13 rooms, fabric U×A, radiators, ventilation, doorway exchange, solar gain. Split from monolithic `thermal.rs` (3,506 lines) on 2026-03-29.

| Module | Lines | Responsibility |
|--------|-------|---------------|
| `config.rs` | 213 | TOML config structs for thermal model |
| `geometry.rs` | 257 | Room/connection/doorway types + JSON loading from `thermal_geometry.json` |
| `physics.rs` | 399 | Constants, thermal mass computation, energy balance equations |
| `solar.rs` | 180 | Solar position + irradiance (PV + Open-Meteo DNI/DHI) |
| `wind.rs` | 75 | Open-Meteo wind speed + ventilation multiplier |
| `calibration.rs` | 626 | Grid search calibration, shared helpers (`calibrate_model`, `avg_series_in_window`, `avg_room_temps_in_window`) |
| `validation.rs` | 435 | Metrics, residuals, holdout window validation |
| `diagnostics.rs` | 444 | Cooldown detection + period-by-period fit diagnostics |
| `operational.rs` | 551 | HP state classification (BCF-based), segmentation, operational validation with solar/radiators |
| `artifact.rs` | 224 | Artifact types, git metadata, build/write JSON artifacts |
| `snapshot.rs` | 233 | Export/import manifests with human signoff |
| `error.rs` | 99 | `ThermalError` enum with `thiserror` derive (20+ variants) |
| `influx.rs` | 352 | Flux query builders for room temps, outside temp, HP status, PV, BCF, MWT |
| `display.rs` | 78 | `print_rooms()` and `print_connections()` CLI output |
| `report.rs` | 44 | Table printer and RMSE calculator |

Public entry points (re-exported via `src/thermal.rs`):

| Function | Command | What it does |
|----------|---------|-------------|
| `calibrate()` | `thermal-calibrate` | Grid search over Cd + landing ACH against Night 1/Night 2 cooldown rates |
| `validate()` | `thermal-validate` | Run calibrated model on holdout windows, check pass/fail thresholds |
| `fit_diagnostics()` | `thermal-fit-diagnostics` | Period-by-period cooldown diagnostics from HP status codes |
| `operational_validate()` | `thermal-operational` | Full operational validation with heating/DHW/off, solar gain, BCF-based state |
| `print_rooms()` | `thermal-rooms` | Room summary table (geometry, thermal mass, radiators, pipes) |
| `print_connections()` | `thermal-connections` | Internal wall/floor connections + doorway exchanges |
| `snapshot_export()` / `snapshot_import()` | `thermal-snapshot` | Human-gated reproducibility workflow |

## Standalone Binaries

### `src/bin/thermal-regression-check.rs` (607 lines)

Compares fresh thermal artifacts against baseline JSON files. Supports 4 artifact types: `thermal-calibrate`, `thermal-validate`, `thermal-fit-diagnostics`, `thermal-operational`. Checks per-metric drift against thresholds in `artifacts/thermal/regression-thresholds.toml`.

### `src/bin/cosy-scheduler.rs` (162 lines)

**Retired.** Source kept for reference. Binary removed from pi5data. Was deployed to read outside temp from eBUS and log DHW recommendations. Conflicts with timer-only VRC 700 operation.

## Python Model

### `model/house.py` (1,250 lines)

Lumped-parameter thermal network. 13 rooms with fabric, radiators, ventilation, doorways, solar. Commands: `fetch`, `rooms`, `connections`, `analyse`, `fit`, `equilibrium`, `moisture`. InfluxDB token loaded from `INFLUX_TOKEN` env var or `ak get influxdb`.

**Deleted** (fully superseded by Rust):
- ~~`model/calibrate.py`~~ — replaced by `thermal-calibrate`
- ~~`model/overnight.py`~~ — replaced by `overnight` command

### `model/extract_house_inventory.py` (1,531 lines)

One-off extraction script. Produces `model/data/inventory/` artifacts and `data/canonical/thermal_geometry.json`.

### `model/audit_model_dimensions.py` (123 lines)

One-off audit. Verifies Python and Rust wiring to canonical geometry (509 checks).

## Configuration

### `config.toml` — Domain constants

Six sections: `emoncms` (feeds, sync), `thresholds` (state machine, HDD), `house` (HTC, floor area), `arotherm` (spec curves), `radiators` (15 entries), `gas_era` (monthly gas data).

### `model/thermal-config.toml` — Thermal model config

InfluxDB connection, test night windows, objective function config (excluded rooms, prior weight), calibration bounds/steps, validation windows, fit diagnostics config, wind model (disabled).

### `artifacts/thermal/regression-thresholds.toml` — Regression gates

Thresholds for 4 artifact types: calibrate, validate, fit-diagnostics, operational. Used by `thermal-regression-check` binary and `scripts/thermal-regression-ci.sh`.

### `data/canonical/thermal_geometry.json` — Room geometry

Single source of truth for room dimensions, external fabric, internal connections, doorways, solar glazing. Consumed by both Python (`model/house.py`) and Rust (`src/thermal/geometry.rs`). Provenance tracked.

## Scripts

| Script | Location | Deployment | Purpose |
|--------|----------|-----------|---------|
| `scripts/ebusd-poll.sh` | `ebusd-poll` systemd on pi5data | `scp` + `systemctl restart` | Reads 25+ eBUS values every 30s via `nc`, publishes to MQTT |
| `scripts/ebusd-poll.service` | `/etc/systemd/system/` on pi5data | Part of ebusd-poll deploy | Systemd unit |
| `scripts/backup-sdcard.sh` | Run on imaging host (pi5nvme) | Manual | dd → PiShrink → xz backup pipeline |
| `scripts/thermal-regression-ci.sh` | Local/CI | `bash scripts/thermal-regression-ci.sh` | Lint gates (fmt + clippy) then runs 4 regression checks against baselines |
| `scripts/refresh-thermal-baselines.sh` | Local | After intentional model changes | Generates fresh baseline artifacts |

## Artifacts

```
artifacts/thermal/
  regression-thresholds.toml           # Thresholds for 4 artifact types
  baselines/
    thermal-calibrate-baseline.json    # Reference calibration output
    thermal-validate-baseline.json     # Reference validation output
    thermal-fit-diagnostics-baseline.json
    thermal-operational-baseline.json  # Reference operational output
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
| Radiator data | `config.toml` `[radiators]` AND `data/canonical/thermal_geometry.json` (keep in sync) |
| Room geometry / fabric | `data/canonical/thermal_geometry.json` (consumed by Rust `geometry.rs` and Python `house.py`) |
| Thermal calibration bounds | `model/thermal-config.toml` `[bounds]` |
| Thermal physics / energy balance | `src/thermal/physics.rs` (cooldown) and `src/thermal/operational.rs` (full) |
| Solar gain model | `src/thermal/solar.rs` |
| InfluxDB queries (thermal) | `src/thermal/influx.rs` |
| Artifact schema / output | `src/thermal/artifact.rs` |
| Regression CI gates | `src/bin/thermal-regression-check.rs` + `artifacts/thermal/regression-thresholds.toml` |
| eBUS polling | `scripts/ebusd-poll.sh` on pi5data |
| Octopus data refresh | `~/github/octopus/` project — `npm run cli -- refresh` |
| DHW tracking/boost | `~/github/z2m-hub/` project |
| Zigbee automations | `~/github/z2m-hub/` project |
| VRC 700 settings | `docs/vrc700-settings-audit.md` (reference only, no code) |
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
