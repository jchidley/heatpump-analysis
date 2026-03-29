# Repository Overview

```yaml
commit: 3af9fd0dc90cbba4031ed9df585acd5a118a9c71
short_commit: 3af9fd0
branch: main
commit_date: 2026-03-26
working_tree: clean
```

## What This System Does

A Rust CLI tool that syncs heat pump monitoring data from emoncms.org to a local SQLite database, then analyses it with Polars. It classifies the heat pump's operating state (heating, DHW, defrost, idle) using a hysteresis state machine driven by flow rate, and produces COP breakdowns, energy analysis, degree-day normalisation, and comparisons against both the manufacturer spec and pre-heat-pump gas consumption.

The system is built for a specific installation: a **Vaillant Arotherm Plus 5kW** air-source heat pump at a residential property in London (6 Rhodes Avenue, N22), monitored via an emonHP bundle feeding emoncms.org.

Beyond the Rust analysis tool, the project includes:
- Shell-based **monitoring script** deployed to pi5data (`scripts/ebusd-poll.sh` — systemd service for eBUS data collection)
- A **Python thermal model** (`model/house.py`) for room-by-room thermal network analysis using Zigbee temperature sensors and InfluxDB data
- Extensive **domain documentation** on the hydraulic system, DHW cylinder analysis, monitoring infrastructure, house layout, and room thermal model.
- A separate **z2m-hub** project (`~/github/z2m-hub/`) handles Zigbee devices, automations, DHW tracking/boost, and mobile dashboard.

## Key Technologies

| Technology | Role |
|-----------|------|
| Rust (edition 2021) | All analysis application code |
| Polars 0.46 | DataFrame analysis (lazy evaluation, groupby, aggregation) |
| SQLite (rusqlite 0.33, bundled) | Local data storage, WAL mode |
| TOML (serde + toml crate) | External configuration for all domain constants |
| clap 4 | CLI argument parsing (derive mode) |
| reqwest 0.12 (blocking) | HTTP client for emoncms REST API |
| chrono | Date/time handling |
| Python (via uv) | Room thermal model (`model/house.py`) |
| influxdb-client (Python) | Fetching room sensor data from InfluxDB |
| NumPy / SciPy (Python) | Equilibrium solver (fsolve), thermal parameter fitting |

## What Changed Since Last Code-Truth (1900ca7)

### No Rust code changes

`src/`, `config.toml`, and `Cargo.toml` are identical. All changes are in the Python model, documentation, and AGENTS.md.

### Major: Room Thermal Model Refactored and Calibrated (`model/house.py`)

Grew from ~750 lines to **1397 lines**. Now a full lumped-parameter thermal network with zero free parameters in the fabric model. Key additions since 1900ca7:

- **13 rooms** (up from 11): Office and Landing sensors added 24 Mar 2026
- **Symmetric internal connections** (`InternalConnection`): wall/floor/ceiling conduction between rooms, defined once per connection (was ad-hoc before)
- **Buoyancy-driven doorway exchange** (`Doorway`): Cd=0.20 calibrated from Night 1 vs Night 2 (24-26 Mar 2026). Stairwell modelled as chimney ACH on landing, not pairwise doorways
- **Solar gain model** (`SolarGlazing`): per-room glazing with orientation (SW/NE), tilt, g-value, shading factor. Calibrated from PV (EmonPi2 P3)
- **Thermal mass estimation** (`estimate_thermal_mass()`): construction-based, no free parameters
- **Equilibrium solver** (`cmd_equilibrium()`): scipy fsolve for steady-state room temps at given outside temp and MWT
- **Moisture analysis** (`moisture_analysis()`): absolute humidity tracking, surface RH via physics-based surface temp calculation, ACH cross-validation between moisture and thermal models
- **Night 1/Night 2 calibration** (24-26 Mar 2026): doors-normal vs all-doors-closed. Joint calibration of doorway Cd=0.20 and landing chimney ACH=1.30. RMSE=0.41°C cooldown, 1.16°C warmup

### Updated: AGENTS.md

Extensive additions (277 lines changed) documenting:
- Night 1/Night 2 calibration results (measured heat loss, humidity-derived ACH per room)
- Doorway effects analysis (kitchen exports heat, landing chimney dominates)
- Intervention analysis table (elvina vents, aldora rad, J&C draught-strip, EWI SE wall)
- System costs: HP £3,624 + controller £198 + cylinder £1,483 + 8 new Stelrad rads ~£2,744 = ~£8,048 (DIY, no grant)
- 14-year payback at £565/yr saving
- Overnight strategy revised 29 Mar 2026: 19°C setback 00:00–04:00, DHW at 05:30 (after 1.5h house heating), Cosy-aligned windows. See `docs/overnight-strategy-analysis.md`.
- Solar gain model details and P3 CT scaling issue
- Bottleneck sequence (elvina→aldora→bathroom)

### New: Documentation

- `docs/room-thermal-model.md` (541 lines) — full methodology, calibration, results
- `docs/house-layout.md` (171 lines added) — room connectivity, vertical stacking, sensors
- `docs/roadmap.md` updated with calibration completion status

## Repository Size

| Category | Count |
|----------|-------|
| Rust source files | 6 (3,591 lines) |
| Python model files | 1 (1,397 lines) |
| Domain docs | 10 |
| Code-truth docs | 5 |
| Shell scripts | 3 |
| Git submodules | 6 |
