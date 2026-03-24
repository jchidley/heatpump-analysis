# Repository Overview

```yaml
commit: 1900ca7+
branch: main
updated: 2026-03-24
```

## What This System Does

A Rust CLI tool that syncs heat pump monitoring data from emoncms.org to a local SQLite database, then analyses it with Polars. It classifies the heat pump's operating state (heating, DHW, defrost, idle) using a hysteresis state machine driven by flow rate, and produces COP breakdowns, energy analysis, degree-day normalisation, and comparisons against both the manufacturer spec and pre-heat-pump gas consumption.

The system is built for a specific installation: a **Vaillant Arotherm Plus 5kW** air-source heat pump at a residential property in London (6 Rhodes Avenue, N22), monitored via an emonHP bundle feeding emoncms.org.

Beyond the Rust analysis tool, the project includes:
- Shell-based **monitoring script** deployed to pi5data (`scripts/ebusd-poll.sh` — systemd service for eBUS data collection)
- A **Python thermal model** (`model/house.py`) for room-by-room heat loss analysis using Zigbee temperature sensors and InfluxDB data
- Extensive **domain documentation** on the hydraulic system, DHW cylinder analysis, monitoring infrastructure, and room thermal model.
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
| NumPy / SciPy (Python) | Thermal parameter fitting |

## What Changed Since Last Code-Truth (08e43eb)

### New: Room Thermal Model (`model/house.py`)

Python-based lumped-parameter thermal network model. Uses 11 Zigbee room temperature sensors (SONOFF SNZB-02P) + eBUS outside temperature + HP heat meter data from InfluxDB. Models fabric losses, ventilation (including bathroom MVHR), inter-room heat transfer, and radiator output for each room.

Key capabilities:
- `fetch`: pulls sensor data from InfluxDB on pi5data
- `rooms`: displays room parameters (fabric UA, radiator T50, ventilation ACH, pipe topology)
- `analyse`: steady-state energy balance per room at current conditions
- `fit`: identifies free-cooling periods from eBUS status codes and calculates cooldown rates

See `docs/room-thermal-model.md` for full documentation.

### Updated: AGENTS.md

Extensive additions documenting:
- Complete house layout with room connectivity map and vertical stacking
- All 15 radiators (exact models, T50 ratings, pipe topology — 22mm primary vs 15mm branches)
- 11 Zigbee room sensors + emonth2
- Door states and their thermal implications
- Bathroom MVHR (Vent-Axia Tempra LP, 9 L/s, 78% heat recovery)
- Kitchen hob hood extract (intermittent)
- Elvina trickle vents
- Key thermal relationships (leather as heat hub, Sterling calibration, kitchen no-rad)
- SNZB-02P firmware v2.1.0 bug (stuck readings) and v2.2.0 fix
- Outside temperature hierarchy (eBUS primary, Met Office control)
- HP capacity limit evidence (95% runtime at 2°C outside, Jan 2025)

### New: Spreadsheet Data

Two xlsx files added (not committed to git, referenced in AGENTS.md):
- `Heating needs for the house.xlsx`: room-by-room U-values, radiator specs, HDD regression
- `Utility - Gas Electric-Jack_Laptop.xlsx`: gas/electric history 2007-2024, PV models, hot water meter data, degree days

## Repository Size

| Category | Count |
|----------|-------|
| Rust source files | 6 |
| Python model files | 1 |
| Domain docs | 10 |
| Code-truth docs | 5 |
| Shell scripts | 3 |
| Git submodules | 6 |
