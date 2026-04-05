# Repository Overview

> Scope: implementation overview derived from source. For current operating truth, start in `../../lat.md/`; use `../heating-plan.md`, `../dhw-plan.md`, `../../deploy/SECRETS.md`, and `../../AGENTS.md` as complements.

```yaml
commit: 0b91843
branch: main
commit_date: 2026-04-05
working_tree: clean
```

## What This System Does

A Rust CLI tool and live control system for a **Vaillant Arotherm Plus 5kW** air-source heat pump at 6 Rhodes Avenue, London N22 7UT.

Three main functions:

1. **Analysis CLI** — syncs monitoring data from emoncms.org to local SQLite, classifies HP operating states (heating/DHW/defrost/idle), produces COP breakdowns, energy analysis, degree-day normalisation, and gas-era comparisons using Polars.

2. **Thermal model** — 13-room thermal network calibrated from Zigbee temperature sensors and InfluxDB data. Includes equilibrium solver, MWT bisection for control, and DHW session analysis. Calibration, validation, operational analysis, and reproducibility snapshots.

3. **Adaptive heating V2** — live model-predictive controller on `pi5data`. Two-loop architecture: outer loop (15 min) uses forecast + **live thermal solver** (`bisect_mwt_for_room`) → target flow temp; inner loop (60s) nudges VRC 700 heat curve until `Hc1ActualFlowTempDesired` matches target. Reads eBUS + InfluxDB, writes to VRC 700 via eBUS, logs to InfluxDB and JSONL. Mobile controls via z2m-hub.

Beyond this repo:
- **z2m-hub** (`~/github/z2m-hub/`) — Zigbee automations, DHW tracking/boost, mobile dashboard, and heating mode control proxy
- **Monitoring infrastructure** — emonpi, emonhp, emondhw, pi5data (current summary in `../../lat.md/infrastructure.md`, deeper runbooks in `../../heating-monitoring-setup.md`)

## Key Technologies

| Technology | Role | Evidence |
|-----------|------|---------|
| Rust (edition 2021) | All analysis + thermal model + adaptive controller | `Cargo.toml` |
| Polars 0.46 | DataFrame analysis (lazy evaluation, groupby, aggregation) | `Cargo.toml` |
| SQLite (rusqlite 0.33, bundled) | Local data storage, WAL mode | `Cargo.toml` |
| Axum 0.8 + Tokio | HTTP API for adaptive heating MVP | `Cargo.toml` |
| TOML (serde + toml) | External configuration for domain constants | `config.toml`, `model/thermal-config.toml`, `model/adaptive-heating-mvp.toml` |
| clap 4 | CLI argument parsing (derive mode) | `Cargo.toml` |
| reqwest 0.12 | HTTP client for emoncms REST API, InfluxDB queries, Open-Meteo forecast | `Cargo.toml` |
| thiserror 2 | Typed domain errors in thermal module | `src/thermal/error.rs` |
| sha2 | Config/artifact hashing for reproducibility | `Cargo.toml` |
| chrono | Date/time handling | `Cargo.toml` |
| tracing + tracing-subscriber | Structured logging for adaptive heating MVP | `Cargo.toml` |

## What Changed Since Last Code-Truth (1c2a44a, 2026-04-04)

1 source commit covering:

### Genuine coast mechanism (2026-04-04)

Coast changed from writing a low curve (0.10) to turning heating **off** via `Z1OpMode=off`. Discovery: curve 0.10 at SP=19 with `Hc1MinFlowTempDesired=20` still produced 20°C+ flow temp — the hidden MinFlow floor prevented genuine coasting.

- `RuntimeState.heating_off` field added to track when `Z1OpMode=off`
- Two restore points write `Z1OpMode=night` to re-enable heating: (1) entering waking/preheat hours, (2) during overnight when maintain or preheat ≤15 min away
- Startup now sets `Hc1MinFlowTempDesired=19` (matches SP=19, removes hidden floor)
- `restore_baseline()` now restores `Hc1MinFlowTempDesired=20` alongside curve and OpMode

### lat.md knowledge graph added (2026-04-05)

6 structured documentation files in `lat.md/`: domain, constraints, architecture, heating-control, thermal-model, infrastructure. Cross-linked with `[[wiki refs]]` to source code. Validated by `lat check`.

## Repository Size

| Category | Count |
|----------|-------|
| Rust source files (`src/`) | 10 core + 17 thermal submodules (~14,050 lines) |
| Standalone Rust binaries (`src/bin/`) | 3 (adaptive-heating-mvp, thermal-regression-check, cosy-scheduler [retired]) |
| Python utility scripts | 1 (scripts/dhw-auto-trigger.py [legacy, do not deploy]) |
| Shell scripts (`scripts/`) | 3 |
| Domain docs (`docs/`) | 16 |
| Code-truth docs (`docs/code-truth/`) | 5 + README |
| lat.md/ | 6 structured knowledge-graph files (agent-facing, validated by `lat check`) |
| Config files | 4 (config.toml, thermal-config.toml, adaptive-heating-mvp.toml, regression-thresholds.toml) |
| Deploy files | 1 (adaptive-heating-mvp.service) |
| Canonical data | 1 (thermal_geometry.json) — control-table.json is legacy, no longer loaded |
| Git submodules | 6 |
