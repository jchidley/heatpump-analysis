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

**In scope**: `model/house.py`, `model/calibrate.py`

**Deleted** (fully superseded by Rust, removed 2026-03-30):
- ~~`model/calibrate.py`~~ — replaced by `thermal-calibrate` command
- ~~`model/overnight.py`~~ — replaced by `overnight` command in `src/overnight.rs`

**Utility scripts** (one-off, not part of migration):
- `model/audit_model_dimensions.py` (123L) — one-off audit, keep for reference
- `model/extract_house_inventory.py` (1531L) — one-off extraction, keep for reference

**Out of scope**: Python in git submodules (emonhub, emoncms, EmonScripts, emonPiLCD)

## Current state

### Implemented in Rust

| Command | Status | Notes |
|---------|--------|-------|
| `thermal-calibrate` | ✅ | Grid search, Night 1/Night 2, JSON artifacts |
| `thermal-validate` | ✅ | Holdout windows, pass/fail thresholds |
| `thermal-fit-diagnostics` | ✅ | Period-by-period cooldown QA |
| `thermal-operational` | ✅ | Heating/DHW/off, solar gain, BCF-based state |
| `thermal-snapshot` | ✅ | Export/import with human signoff |

All produce structured JSON artifacts to `artifacts/thermal/`. Regression baselines in `artifacts/thermal/baselines/`. Formula parity with Python verified (audit completed 2026-03-28, 509 checks, 0 mismatches).

### Remaining Python-only commands

1. **`thermal-rooms`** — room summary table. Pure geometry, low complexity.
2. **`thermal-connections`** — internal wall/doorway table. Pure geometry.
3. **`thermal-analyse`** — live energy balance from InfluxDB. Medium complexity.
4. **`thermal-equilibrium`** — steady-state solver (`scipy.fsolve` → Gauss-Seidel or `nalgebra`). High complexity.
5. **`thermal-moisture`** — humidity analysis. Medium complexity, lower priority.

After all ported, mark `model/house.py` as legacy.

### Infrastructure completed

- ✅ **Thermal module split** (2026-03-29): `src/thermal.rs` (3,506 lines) → 15 focused submodules (4,155 lines total). Thin facade re-exports 6 public entry points.
- ✅ **DRY cleanup** (2026-03-29): Extracted 5 shared helpers (`calibrate_model`, `resolve_influx_token`, `compute_thermal_masses`, `avg_series_in_window`, `avg_room_temps_in_window`). ~90 lines of duplication removed.
- ✅ **Regression baselines refreshed** against current config.

Module layout:
  - `config.rs` — TOML config structs
  - `geometry.rs` — room/connection/doorway types + JSON loading
  - `physics.rs` — constants + thermal mass + energy balance
  - `solar.rs` — solar position + irradiance
  - `wind.rs` — Open-Meteo wind + multiplier
  - `calibration.rs` — grid search + setup + predict/measured rates + time-series helpers
  - `validation.rs` — metrics + residuals + validate()
  - `diagnostics.rs` — cooldown detection + fit_diagnostics()
  - `operational.rs` — HP state + segmentation + operational_validate()
  - `artifact.rs` — artifact types + git meta + build/write
  - `snapshot.rs` — export/import manifests
  - Existing: `error.rs`, `influx.rs`, `report.rs`

### Infrastructure remaining

- ~~Add `thermal-operational` to regression CI~~ ✔️ Done 2026-03-30: `[operational]` thresholds + comparison logic + baseline
- ~~Enforce lint gates in CI workflow~~ ✔️ Done 2026-03-30: fmt + clippy gates in `scripts/thermal-regression-ci.sh`
- ~~Remove hardcoded `INFLUX_TOKEN` from `model/house.py`~~ ✔️ Done 2026-03-30: reads `INFLUX_TOKEN` env var or `ak get influxdb`

### Cleanup remaining

- ~~Remove `cosy-scheduler` binary from pi5data~~ ✔️ Done 2026-03-30: binary deleted, source kept for reference
- ~~Regenerate `docs/code-truth/`~~ ✔️ Done 2026-03-30: REPOSITORY_MAP, ARCHITECTURE, REPO_OVERVIEW updated for thermal split + deleted files

## Quality gates

Every change must pass:
1. `cargo fmt --check`
2. `cargo clippy --all-targets -- -D warnings`
3. `cargo check`
4. `bash scripts/thermal-regression-ci.sh` (thermal commands against baselines)

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
