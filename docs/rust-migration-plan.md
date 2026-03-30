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

### Infrastructure remaining

- ~~Complete thermal module split~~ ✅ Done 2026-03-29: `src/thermal.rs` (3,506 lines) → 15 focused submodules (4,192 lines total)
  - `config.rs` (207L) — TOML config structs
  - `geometry.rs` (257L) — room/connection/doorway types + JSON loading
  - `physics.rs` (388L) — constants + thermal mass + energy balance
  - `solar.rs` (180L) — solar position + irradiance
  - `wind.rs` (75L) — Open-Meteo wind + multiplier
  - `calibration.rs` (532L) — grid search + setup + predict/measured rates
  - `validation.rs` (469L) — metrics + residuals + validate()
  - `diagnostics.rs` (492L) — cooldown detection + fit_diagnostics()
  - `operational.rs` (617L) — HP state + segmentation + operational_validate()
  - `artifact.rs` (224L) — artifact types + git meta + build/write
  - `snapshot.rs` (233L) — export/import manifests
  - `thermal.rs` (23L) — thin facade with re-exports
  - Existing: `error.rs`, `influx.rs`, `report.rs` (unchanged)
- Add `thermal-operational` baseline to regression CI
- Enforce lint gates in CI workflow

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
