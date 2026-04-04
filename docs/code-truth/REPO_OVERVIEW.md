# Repository Overview

> Scope: implementation overview derived from source. For operating policy and deployment truth, see `../heating-plan.md`, `../dhw-plan.md`, `../../deploy/SECRETS.md`, and `../../AGENTS.md`.

```yaml
commit: 1c2a44a
branch: main
commit_date: 2026-04-04
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
| reqwest 0.12 | HTTP client for emoncms REST API, InfluxDB queries, Open-Meteo forecast | `Cargo.toml` |
| thiserror 2 | Typed domain errors in thermal module | `src/thermal/error.rs` |
| sha2 | Config/artifact hashing for reproducibility | `Cargo.toml` |
| chrono | Date/time handling | `Cargo.toml` |
| tracing + tracing-subscriber | Structured logging for adaptive heating MVP | `Cargo.toml` |

## What Changed Since Last Code-Truth (7b6bfed, 2026-03-31)

42 commits covering:

### Adaptive heating V2 (2026-03-31 → 2026-04-02)

Complete rewrite from V1 bang-bang to V2 model-predictive control:
- **Two-loop architecture**: outer loop (900s) uses Open-Meteo forecast + live thermal solver (`bisect_mwt_for_room`) → target flow temp + initial curve guess. Inner loop (60s) proportional feedback on `Hc1ActualFlowTempDesired`. Phase 2 overnight planner with cooling simulation and adaptive preheat timing.
- **Z1OpMode=night on startup** (SP=19): eliminates VRC 700 Optimum Start, day/night transitions, and timer interference. Clean restore on shutdown (`Z1OpMode=auto`, `Hc1HeatCurve=0.55`).
- **Removed flow_offset and room_offset EMAs** — inner loop replaces both. room_offset ran away to +2.18°C overnight.
- **Phase 1b bug fixes deployed**: inner loop floor guard (halve gain below curve 0.25), ΔT stabilisation (use default ΔT when compressor not actively heating).
- **Phase 1b live solver deployed**: replaced `ControlTable` (104-point JSON bilinear interpolation) with direct `bisect_mwt_for_room()` calls from the calibrated thermal physics model. Created `src/lib.rs` exposing thermal module as library crate. Solver matches old table exactly (29.1°C at 5°C/0W).
- Source grew from 900 to ~1350 lines (net reduction from removing ControlTable).

### VRC 700 curve resolution discovery (2026-04-02)

`Hc1HeatCurve` is IEEE 754 float (verified via hex read). 0.01 step = ~0.20°C flow change at SP=19. No quantization to 0.05 steps. Documented across all relevant files.

### DHW strategy rework (2026-04-02)

Analysis of 402 AM DHW charges from emoncms data (Oct 2024 – Mar 2026):
- Eco mode avg 102 min, 40% hit 120-min timeout, 95% incomplete below 2°C
- Normal mode avg 60 min, 2% timeout, works at all temperatures
- T1 (Multical, 0.01°C/2s at cylinder top) is far better than HwcStorageTemp (VR10 NTC, 0.5°C/30s at 600mm) for DHW decisions
- Standing loss: 0.25°C/h T1 drop (not 0.26°C total as previously back-calculated)
- Preferred strategy: charge at 22:00 Cosy window, monitor T1, top up at 04:00 if needed
- Cosy windows preferred to reduce battery pressure on cold days, but overnight timing flexible

### Thermal submodule: dhw_sessions.rs

DHW draw/charge session analysis. Raw 10s data for event detection, HwcStorageTemp tracking during draws, draw type classification (bath/shower/tap by peak flow rate), draws during HP charging detected via tap-side Multical meter. Writes `dhw_inflection` + `dhw_capacity` to InfluxDB; z2m-hub autoloads recommended capacity on startup.

### Thermal display.rs expanded (78 → 993 lines)

Added `solve_equilibrium_temps()`, `bisect_mwt_for_room()`, `generate_control_table()` — solver functions now called directly by the adaptive controller (Phase 1b complete).

### src/lib.rs created (2 lines)

Exposes `pub mod thermal` as library crate. Enables `adaptive-heating-mvp` binary to call thermal solver functions via `heatpump_analysis::thermal::bisect_mwt_for_room()`.

### Heat curve exponent updated

Best fit 1.25 (was 1.27) from expanded 17-point pilot data. VRC 700 formula: `flow = setpoint + curve × (setpoint - outside)^1.25`.

### Overnight planner fixes + empirical τ (2026-04-04)

- **Break bug**: planner scanned coast times from max→0 but never broke on first match — always chose coast=0 (heat immediately). Fixed with `break`.
- **τ updated**: `LEATHER_TAU_H` changed from 15.0 to 50.0 (empirical, from 53 cooling segments: 18 calibration-night + 35 DHW). Two independent sources agree on median ~50h.
- **K=7500 flagged**: empirical K≈20,600 from 27 reheat segments. Not yet updated in code — each coast night validates.
- **Comfort miss clipping**: `clip_period_to_waking_hours()` replaces `period_intersects_waking_hours()` filter. Comfort misses now trimmed to 07:00–23:00 (overnight cooling is expected, not a miss).

### Plan docs restructured (2026-04-04)

`docs/heating-plan.md` and `docs/dhw-plan.md` rewritten as LLM working memory (tables + commands). Domain reference data extracted to `docs/heating-reference.md` and `docs/dhw-reference.md`.

## Repository Size

| Category | Count |
|----------|-------|
| Rust source files (`src/`) | 10 core + 16 thermal submodules (~14,011 lines) |
| Standalone Rust binaries (`src/bin/`) | 3 (adaptive-heating-mvp, thermal-regression-check, cosy-scheduler [retired]) |
| Python utility scripts | 1 (scripts/dhw-auto-trigger.py [legacy, do not deploy]) |
| Shell scripts (`scripts/`) | 3 |
| Domain docs (`docs/`) | 16 |
| Code-truth docs (`docs/code-truth/`) | 5 |
| Config files | 4 (config.toml, thermal-config.toml, adaptive-heating-mvp.toml, regression-thresholds.toml) |
| Deploy files | 1 (adaptive-heating-mvp.service) |
| Canonical data | 1 (thermal_geometry.json) — control-table.json is legacy, no longer loaded |
| Git submodules | 6 |
