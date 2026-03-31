# Repository Overview

```yaml
commit: dfdffb4
branch: main
commit_date: 2026-03-30
working_tree: modified
```

## What This System Does

A Rust CLI tool that syncs heat pump monitoring data from emoncms.org to a local SQLite database, then analyses it with Polars. It classifies the heat pump's operating state (heating, DHW, defrost, idle) using a hysteresis state machine driven by flow rate, and produces COP breakdowns, energy analysis, degree-day normalisation, and comparisons against both the manufacturer spec and pre-heat-pump gas consumption.

The system is built for a specific installation: a **Vaillant Arotherm Plus 5kW** air-source heat pump at a residential property in London (6 Rhodes Avenue, N22), monitored via an emonHP bundle feeding emoncms.org.

Beyond the Rust analysis tool, the project includes:
- A **Rust thermal model** (`src/thermal/` — 15 submodules, 4,222 lines) for room-level calibration, validation, and operational analysis using Zigbee temperature sensors and InfluxDB data
- ~~Python thermal model (`model/house.py`)~~ — deleted 2026-03-30, all commands ported to Rust
- Shell-based **monitoring scripts** deployed to pi5data (`scripts/ebusd-poll.sh`)
- Extensive **domain documentation** on the hydraulic system, DHW cylinder, monitoring infrastructure, house layout, and room thermal model
- A separate **z2m-hub** project (`~/github/z2m-hub/`) handles Zigbee devices, automations, DHW tracking/boost, and mobile dashboard

## Key Technologies

| Technology | Role | Evidence |
|-----------|------|---------|
| Rust (edition 2021) | All analysis + thermal model code | `Cargo.toml` |
| Polars 0.46 | DataFrame analysis (lazy evaluation, groupby, aggregation) | `Cargo.toml` |
| SQLite (rusqlite 0.33, bundled) | Local data storage, WAL mode | `Cargo.toml` |
| TOML (serde + toml) | External configuration for domain constants | `config.toml`, `model/thermal-config.toml` |
| clap 4 | CLI argument parsing (derive mode) | `Cargo.toml` |
| reqwest 0.12 (blocking) | HTTP client for emoncms REST API + InfluxDB queries | `Cargo.toml` |
| thiserror 2 | Typed domain errors in thermal module | `src/thermal/error.rs` |
| sha2 | Config/artifact hashing for reproducibility | `Cargo.toml` |
| chrono | Date/time handling | `Cargo.toml` |


## What Changed Since Last Code-Truth (f9694e2, 2026-03-29)

### Thermal module split (2026-03-29)

Monolithic `src/thermal.rs` (3,506 lines) split into 15 submodules under `src/thermal/` (4,222 lines total). `src/thermal.rs` is now a thin facade re-exporting 8 public entry points. DRY cleanup extracted 5 shared helpers.

### Regression CI expanded (2026-03-30)

`thermal-operational` added as 4th artifact type in regression CI. Lint gates (fmt + clippy) added to `scripts/thermal-regression-ci.sh`.

### Infrastructure cleanup (2026-03-30)

- `model/calibrate.py` and `model/overnight.py` deleted (fully superseded by Rust)
- `model/house.py` deleted — all 5 commands ported to Rust (rooms, connections, analyse, equilibrium, moisture)
- `cosy-scheduler` binary removed from pi5data (source kept for reference)
- `influxdb` and `grafana` credentials added to `ak` GPG keystore
- Grafana DHW dashboard updated: 3 cylinder sensors (T1 Hot Out, T2 Cold In, Cylinder Temp)
- z2m-hub dashboard updated with descriptive temperature labels

## Repository Size

| Category | Count |
|----------|-------|
| Rust source files (`src/`) | 10 core + 15 thermal submodules (~10,356 lines) |
| Standalone Rust binaries (`src/bin/`) | 2 (cosy-scheduler [retired], thermal-regression-check) |
| Python utility scripts (`model/`) | 2 (~1,654 lines, one-off only) |
| Shell scripts (`scripts/`) | 3 + 2 services |
| Domain docs (`docs/`) | 14 |
| Code-truth docs (`docs/code-truth/`) | 5 |
| Config files | 3 (config.toml, thermal-config.toml, regression-thresholds.toml) |
| Canonical data | 1 (thermal_geometry.json) |
| Git submodules | 6 |
