# Repository Overview

```yaml
commit: f9694e21351a4e159063082c1faaec487cecef3d
short_commit: f9694e2
branch: main
commit_date: 2026-03-29
working_tree: modified
```

## What This System Does

A Rust CLI tool that syncs heat pump monitoring data from emoncms.org to a local SQLite database, then analyses it with Polars. It classifies the heat pump's operating state (heating, DHW, defrost, idle) using a hysteresis state machine driven by flow rate, and produces COP breakdowns, energy analysis, degree-day normalisation, and comparisons against both the manufacturer spec and pre-heat-pump gas consumption.

The system is built for a specific installation: a **Vaillant Arotherm Plus 5kW** air-source heat pump at a residential property in London (6 Rhodes Avenue, N22), monitored via an emonHP bundle feeding emoncms.org.

Beyond the Rust analysis tool, the project includes:
- A **Rust thermal model** (`src/thermal.rs` + `src/thermal/`) for room-level calibration, validation, and operational analysis using Zigbee temperature sensors and InfluxDB data
- A **Python thermal model** (`model/house.py`) for equilibrium solving, moisture analysis, and exploratory analysis — shares canonical geometry with Rust via `data/canonical/thermal_geometry.json`
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
| Python (via uv) | Room thermal model (`model/house.py`) | `model/house.py` |
| influxdb-client (Python) | Fetching room sensor data from InfluxDB | `model/house.py` |
| NumPy / SciPy (Python) | Equilibrium solver (fsolve), thermal parameter fitting | `model/house.py` |

## What Changed Since Last Code-Truth (3af9fd0, 2026-03-26)

### Major: Rust thermal model tripled in size

`src/thermal.rs` grew from ~1,500 lines to **3,506 lines**. New thermal submodules added. Total Rust: **14 files, ~10,000 lines** (was 6 files, 3,591 lines).

Key additions:
- **`thermal-operational`** command — full operational validation with heating/DHW/off state classification using `BuildingCircuitFlow` (eBUS), solar gain model (PV + Open-Meteo), per-room scoring
- **Solar geometry** — Spencer (1971) solar position, isotropic sky model for oriented surface irradiance (DNI + DHI decomposition)
- **`thermal-snapshot`** export/import — human-gated reproducibility workflow with manifest and SHA-256 verification
- **Regression infrastructure** — `thermal-regression-check` binary, baseline artifacts, `thermal-regression-ci.sh` script, thresholds TOML

### Major: Overnight strategy analysis (`src/overnight.rs`)

New 1,442-line module implementing a Rust backtest model for overnight heating optimisation. Calibrated cooling/heating/DHW models from 512 days of data. Evaluated 30 strategies × 324 nights. Conclusion: battery makes scheduling nearly irrelevant (£15–40/yr total opportunity).

### Documentation overhaul (this session)

15 documentation files updated to fix contradictions (StatuscodeNum 134, EWI area, Zigbee counts, DHW schedule), remove stale references (dhw-auto-trigger, ebusd-poll.py), and add archival headers.

## Repository Size

| Category | Count |
|----------|-------|
| Rust source files (`src/`) | 14 (~10,000 lines) |
| Standalone Rust binaries (`src/bin/`) | 2 (cosy-scheduler, thermal-regression-check) |
| Python model files (`model/`) | 5 (~3,925 lines) |
| Shell scripts (`scripts/`) | 3 + 2 services |
| Domain docs (`docs/`) | 14 |
| Code-truth docs (`docs/code-truth/`) | 5 |
| Config files | 3 (config.toml, thermal-config.toml, regression-thresholds.toml) |
| Canonical data | 1 (thermal_geometry.json) |
| Git submodules | 6 |
