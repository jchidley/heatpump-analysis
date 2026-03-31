# About the Python → Rust Migration

Policy and execution plan for migrating all first-party Python programs to Rust.

## Engineering standard

- Correctness before convenience
- Typed errors (`thiserror`) in domain code; `anyhow` at CLI boundary only
- Configuration in TOML, physics in code, geometry in `thermal_geometry.json`
- Direct InfluxDB queries as default data source
- Snapshots require explicit human signoff (`--approved-by-human`)
- Warning-free builds (`cargo clippy -- -D warnings`)
- Measured parameters over fudge factors

## Scope

**Complete**: `model/house.py` deleted 2026-03-30 — all commands ported to Rust.

**Deleted** (fully superseded by Rust, removed 2026-03-30):
- `model/calibrate.py` — replaced by `thermal-calibrate`
- `model/overnight.py` — replaced by `overnight` command

**Utility scripts** (one-off, not part of migration):
- `model/audit_model_dimensions.py` (123L) — one-off audit, keep for reference
- `model/extract_house_inventory.py` (1531L) — one-off extraction, keep for reference

**Out of scope**: Python in git submodules (emonhub, emoncms, EmonScripts, emonPiLCD)

## Current state

### Implemented in Rust

| Command | Notes |
|---------|-------|
| `thermal-rooms` | Room summary table (geometry, thermal mass, radiators, pipes) |
| `thermal-connections` | Internal connections + doorway exchanges |
| `thermal-calibrate` | Grid search, Night 1/Night 2, JSON artifacts |
| `thermal-validate` | Holdout windows, pass/fail thresholds |
| `thermal-fit-diagnostics` | Period-by-period cooldown QA |
| `thermal-operational` | Heating/DHW/off, solar gain, BCF-based state |
| `thermal-snapshot` | Export/import with human signoff |
| `thermal-analyse` | Live energy balance from InfluxDB (per-room heat flows) |
| `thermal-equilibrium` | Steady-state room temperature solver (Gauss-Seidel + bisection) |
| `thermal-moisture` | Condensation risk + overnight humidity balance |

All calibration/validation/operational commands produce structured JSON artifacts to `artifacts/thermal/`. Regression baselines in `artifacts/thermal/baselines/`. Formula parity with Python verified (audit completed 2026-03-28, 509 checks, 0 mismatches).

### Remaining Python-only commands

| # | Command | Complexity | Notes |
|---|---------|-----------|-------|
| 1 | ~~`thermal-analyse`~~ | ✅ | Ported 2026-03-30. |
| 2 | ~~`thermal-equilibrium`~~ | ✅ | Ported 2026-03-30. **Fixes Python fsolve convergence failure** (ier=5). |
| 3 | ~~`thermal-moisture`~~ | ✅ | Ported 2026-03-30. |

All commands ported. `model/house.py` deleted.

### Module layout (`src/thermal/`, 15 submodules)

| Module | Responsibility |
|--------|---------------|
| `config.rs` | TOML config structs |
| `geometry.rs` | Room/connection/doorway types + JSON loading |
| `physics.rs` | Constants, thermal mass, energy balance equations |
| `solar.rs` | Solar position + irradiance |
| `wind.rs` | Open-Meteo wind + ventilation multiplier |
| `calibration.rs` | Grid search + shared helpers |
| `validation.rs` | Metrics, residuals, holdout validation |
| `diagnostics.rs` | Cooldown detection + fit diagnostics |
| `operational.rs` | HP state classification, operational validation |
| `artifact.rs` | JSON artifact build/write |
| `snapshot.rs` | Export/import manifests |
| `display.rs` | `print_rooms()` and `print_connections()` CLI output |
| `error.rs` | `ThermalError` enum |
| `influx.rs` | InfluxDB Flux query builders |
| `report.rs` | Table formatting + RMSE |

### Infrastructure — all complete ✅

All infrastructure tasks completed 2026-03-29 to 2026-03-30:
- Thermal module split (monolith → 15 submodules) + DRY cleanup
- Regression CI: 4 artifact types (calibrate, validate, fit-diagnostics, operational) + lint gates (fmt + clippy)
- `INFLUX_TOKEN` removed from source — env var + `ak` fallback
- `influxdb` + `grafana` credentials in `ak` GPG keystore
- `cosy-scheduler` binary removed from pi5data
- `docs/code-truth/` regenerated
- Grafana DHW chart: 3 sensors with correct labels
- z2m-hub dashboard: descriptive temperature labels

## Quality gates

Every change must pass:
1. `cargo fmt --check`
2. `cargo clippy --all-targets -- -D warnings`
3. `cargo check`
4. `bash scripts/thermal-regression-ci.sh` (4 artifact types against baselines)

Baseline lifecycle: generate artifacts → copy to `baselines/` (or use `scripts/refresh-thermal-baselines.sh`) → verify regression CI passes. Never relax thresholds and change model logic in the same commit.

## Operating policy

- Rust outputs are authoritative when command exists
- Python for exploratory comparisons only
- Parameter changes in TOML first, consumed by Rust
- Never rely on stale CSV extracts for canonical conclusions

## Definition of done

1. No production workflow depends on Python
2. All outputs available from Rust CLI
3. Config-driven via TOML
4. Default data source is direct InfluxDB
5. Python files removed or marked `legacy/`

## Model improvements (identified from operational validation)

| Room | Issue | Fix |
|------|-------|-----|
| Kitchen | +0.245 bias — bare CH pipes in floor void (~25W each side) | Add pipe heat term |
| Bathroom | +0.250 bias — shower events dump heat + moisture | Detect draws from Multical, add transient heat |
| Office | RMSE 1.020 — coupled to landing chimney | Improves when landing model is fixed |
| Landing | Wrong sign 9/14 heating periods | Replace ACH-to-outside with bidirectional inter-floor exchange |
| Conservatory | RMSE 0.686 — 30m² glass, massive solar/wind sensitivity | Use as measured boundary condition instead of predicting |
