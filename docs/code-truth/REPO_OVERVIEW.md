# Repository Overview

```yaml
commit: 7b6bfed
branch: main
commit_date: 2026-03-31
working_tree: clean
```

## What This System Does

A Rust CLI tool and live control system for a **Vaillant Arotherm Plus 5kW** air-source heat pump at 6 Rhodes Avenue, London N22 7UT.

Three main functions:

1. **Analysis CLI** — syncs monitoring data from emoncms.org to local SQLite, classifies HP operating states (heating/DHW/defrost/idle), produces COP breakdowns, energy analysis, degree-day normalisation, and gas-era comparisons using Polars.

2. **Thermal model** — 13-room thermal network calibrated from Zigbee temperature sensors and InfluxDB data. Calibration, validation, operational analysis, and reproducibility snapshots.

3. **Adaptive heating MVP** — live pilot controller on `pi5data` that reads room sensors, outside temp, HP state, and cylinder state every 15 minutes, makes bounded control decisions by writing to the VRC 700 via eBUS, and logs everything to InfluxDB and JSONL. Mobile controls via z2m-hub.

Beyond this repo:
- **z2m-hub** (`~/github/z2m-hub/`) — Zigbee automations, DHW tracking/boost, mobile dashboard, and heating mode control proxy
- **Monitoring infrastructure** — emonpi, emonhp, emondhw, pi5data (see `heating-monitoring-setup.md`)

## Key Technologies

| Technology | Role | Evidence |
|-----------|------|---------|
| Rust (edition 2021) | All analysis + thermal model + adaptive controller | `Cargo.toml` |
| Polars 0.46 | DataFrame analysis (lazy evaluation, groupby, aggregation) | `Cargo.toml` |
| SQLite (rusqlite 0.33, bundled) | Local data storage, WAL mode | `Cargo.toml` |
| Axum 0.8 + Tokio | HTTP API for adaptive heating MVP | `Cargo.toml` |
| TOML (serde + toml) | External configuration for domain constants | `config.toml`, `model/thermal-config.toml`, `model/adaptive-heating-mvp.toml` |
| clap 4 | CLI argument parsing (derive mode) | `Cargo.toml` |
| reqwest 0.12 | HTTP client for emoncms REST API, InfluxDB queries, and eBUS reads | `Cargo.toml` |
| thiserror 2 | Typed domain errors in thermal module | `src/thermal/error.rs` |
| sha2 | Config/artifact hashing for reproducibility | `Cargo.toml` |
| chrono | Date/time handling | `Cargo.toml` |
| tracing + tracing-subscriber | Structured logging for adaptive heating MVP | `Cargo.toml` |

## What Changed Since Last Code-Truth (dfdffb4, 2026-03-30)

### Adaptive heating MVP (2026-03-31)

New binary `src/bin/adaptive-heating-mvp.rs` (900 lines). Live pilot controller deployed as systemd service on `pi5data`. Reads eBUS (via TCP to ebusd) and InfluxDB (room temps), writes VRC 700 registers, logs to InfluxDB and JSONL. HTTP API on port 3031 for mode control. Config in `model/adaptive-heating-mvp.toml`. systemd unit in `deploy/adaptive-heating-mvp.service`.

New dependencies added to `Cargo.toml`: axum, tokio, tracing, tracing-subscriber.

### VRC 700 control surface discovery (2026-03-31)

~25 writable VRC 700 registers confirmed by live eBUS write + readback. `Hc1HeatCurve` proven to change `Hc1ActualFlowTempDesired` on the live system. Documented in `docs/adaptive-heating-mvp.md`.

### z2m-hub patched (2026-03-31)

Added heating mode proxy routes and mobile dashboard section to `~/github/z2m-hub/src/main.rs`. Proxies to adaptive-heating-mvp on localhost:3031.

### Documentation added/updated (2026-03-31)

- `docs/adaptive-heating-control.md` — strategy, philosophy, room targeting, control theory
- `docs/adaptive-heating-mvp.md` — frozen MVP spec, implementation status, outstanding work
- `docs/roadmap.md` — adaptive heating and Pico eBUS sections added
- `docs/dhw-fixes.md` — hygiene monitoring item added
- `docs/pico-ebus-plan.md` — cross-reference to adaptive-heating-mvp
- `docs/dynamic-curve-strategy.md` — deleted (superseded)
- `AGENTS.md` — new binary, service, ports

## Repository Size

| Category | Count |
|----------|-------|
| Rust source files (`src/`) | 10 core + 15 thermal submodules (~10,356 lines) |
| Standalone Rust binaries (`src/bin/`) | 3 (adaptive-heating-mvp, thermal-regression-check, cosy-scheduler [retired]) |
| Python utility scripts (`model/`) | 2 (~1,654 lines, one-off only) |
| Shell scripts (`scripts/`) | 3 + 2 services |
| Domain docs (`docs/`) | 15 |
| Code-truth docs (`docs/code-truth/`) | 5 |
| Config files | 4 (config.toml, thermal-config.toml, adaptive-heating-mvp.toml, regression-thresholds.toml) |
| Deploy files | 1 (adaptive-heating-mvp.service) |
| Canonical data | 1 (thermal_geometry.json) |
| Git submodules | 6 |
