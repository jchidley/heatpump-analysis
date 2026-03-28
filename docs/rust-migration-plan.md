# About the Python → Rust Migration

This document defines how this project will migrate all first-party Python programs to Rust.

It is a policy + execution plan, not a speculative roadmap.

## Engineering standard (project commitment)

This migration is held to a **high programming standard**:

- correctness before convenience
- explicit design over implicit behavior
- typed errors over generic catch-alls in domain code
- reproducibility over ad-hoc experimentation
- measured physics over fitted fudge factors
- warning-free builds (warnings treated as failures)
- small, reviewable, testable changes

Any change that violates these principles is out of scope for this migration, even if it appears faster in the short term.

## Non-negotiable ground rules

1. **Models belong in code**
   - Thermal/physics/state models are implemented in Rust source, versioned and reviewed.
   - No model constants should be hidden in ad-hoc notebooks or one-off scripts.

2. **Configuration belongs in TOML**
   - Tunable values (bounds, priors, windows, room overrides, feature flags, API endpoints) must live in TOML files.
   - Changing behavior should usually mean editing TOML, not editing Rust source.

3. **Data belongs in the source of truth**
   - Default data source is **direct InfluxDB query**.
   - No implicit local extracts as normal operation.

4. **Snapshots require human signoff**
   - Local snapshots are allowed only for explicit reproducibility/audit workflows.
   - A human must approve creating and using a snapshot.
   - Snapshot usage must be explicit in command flags and reported in outputs.

5. **Use typed errors in domain code**
   - Domain and infrastructure modules use explicit error enums (`thiserror`) with meaningful variants.
   - `anyhow` is allowed only at command boundaries / top-level orchestration.
   - No generic `anyhow` propagation inside core model/calibration logic.

6. **Strong linting is mandatory**
   - Rust code must pass strict lint gates before merge.
   - Warnings are treated as failures for project code.
   - Clippy and formatting are part of the default workflow, not optional cleanup.

7. **Prefer measured parameters and physical models over fudge factors**
   - First preference: direct measurements (instrumented data, surveyed geometry, datasheets, validated plans).
   - Second preference: physically-derived estimates with explicit assumptions.
   - Last resort: fitted correction factors / ad-hoc offsets.
   - Any fitted factor must be documented with: why it exists, valid range, sensitivity, and removal plan.

## Scope

### In scope (first-party Python in this repo)

- `model/house.py`
- `model/calibrate.py`

### Out of scope (for this migration)

Python files inside git submodules (`emonhub/`, `emoncms/`, `EmonScripts/`, `emonPiLCD/`, etc.) are external/upstream concerns and are not part of this project’s migration target.

## Current state

- Rust CLI is the canonical analytics tool for emoncms/SQLite workflows.
- Thermal modelling/calibration originated in Python.
- Rust thermal commands now implemented:
  - `thermal-calibrate`
  - `thermal-validate` (holdout windows + pass/fail thresholds)
  - `thermal-fit-diagnostics` (period-by-period cooldown diagnostics parity)
- Thermal runs now produce structured JSON artifacts under `artifacts/thermal/` including params, residuals, config hash, and git metadata.
- Thermal code now has typed domain errors (`thermal::error::ThermalError`) and partial module split (`error`, `influx`, `report`) under `src/thermal/`.
- Canonical geometry is now consumed by Rust/Python with explicit stair-stack links (`hall→landing→top_landing→shower`) and plan-constrained internal wall UAs.
- Optional public wind input (Open-Meteo) is available in Rust calibration config (`[wind]`); currently default-disabled because baseline 2-night fit was better without wind multiplier.
- Remaining migration work is focused on command parity for `rooms`/`analyse`/`connections`/`equilibrium`/`moisture`, module decomposition, and lint/CI hardening.

## Target architecture

### Rust modules

- `src/thermal/` (new module tree over time)
  - `model.rs` — room definitions, physics equations, energy balance
  - `influx.rs` — direct Influx queries and typed parsing
  - `calibration.rs` — objective functions, constrained search, priors
  - `validation.rs` — holdout/night validation and residual analysis
  - `report.rs` — tabular/JSON outputs

(Short-term implementation may remain in `src/thermal.rs`; target is modular split.)

### Config files

- `model/thermal-config.toml` (current)
  - Influx connection
  - test windows
  - objective exclusions
  - priors
  - bounds/steps

- Future split if needed:
  - `config/thermal/rooms.toml`
  - `config/thermal/windows.toml`
  - `config/thermal/calibration.toml`

### Commands (target)

- `thermal-calibrate` (implemented)
- `thermal-validate` (implemented)
- `thermal-fit-diagnostics` (implemented)
- `thermal-rooms` (planned)
- `thermal-analyse` (planned)
- `thermal-connections` (planned)
- `thermal-equilibrium` (planned)
- `thermal-moisture` (planned / lower priority)
- `thermal-report` (planned)
- `thermal-snapshot export|import` (planned, explicit + signed-off)

## Quality gates (required)

Every migration step must satisfy:

1. `cargo fmt --check`
2. `cargo clippy --all-targets -- -D warnings`
3. `cargo check`
4. Thermal commands run successfully against configured test nights

Additional linting policy:

- Add crate/module-level lints for migrated modules (example):
  - `#![forbid(unsafe_code)]`
  - `#![deny(clippy::unwrap_used)]`
  - `#![deny(clippy::expect_used)]`
  - `#![deny(clippy::todo)]`
  - `#![deny(clippy::dbg_macro)]`
- Prefer explicit parsing/validation errors over panic paths.
- If a warning must be suppressed, include a narrow `#[allow(...)]` with reason.

## Error handling policy

- **Domain / core modules:** typed errors via `thiserror`
- **CLI boundary (`main.rs`)**: convert typed errors to top-level reporting (`anyhow` acceptable here)
- Error variants should preserve actionable context (window names, room name, query scope, config key).

Target shape:

- `thermal::error::ThermalError` (and sub-error enums if needed)
- `Result<T, ThermalError>` through model/influx/calibration/report layers
- single conversion at command dispatch boundary

## Migration phases

## Phase 1 — Freeze Python, establish Rust parity (mostly complete)

- [x] Add Rust `thermal-calibrate`
- [x] Move calibration controls to TOML
- [x] Use direct InfluxDB reads by default
- [x] Introduce typed thermal domain errors (`thiserror`) and remove internal `anyhow` use in thermal modules
- [x] Begin module split (`error`, `influx`, `report`)
- [x] Add Rust `thermal-validate` for holdout nights
- [x] Add Rust `thermal-fit-diagnostics` for period-by-period cooldown QA
- [ ] Match/verify all relevant formulas vs `model/house.py`
- [ ] Document intentional deltas

Deliverable: Rust commands produce equivalent calibration + validation + cooldown diagnostics with typed, auditable error handling.

## Phase 2 — Port model capabilities from `house.py`

Port, in this order:

1. Core physics primitives
   - radiator output
   - fabric loss
   - ventilation loss
   - wall conduction
   - doorway exchange

2. Full room energy balance
   - occupancy heat
   - DHW parasitic heat terms

3. Analysis features
   - steady-state analysis table
   - cooldown fit diagnostics
   - equilibrium solver (or equivalent constrained solver)
   - moisture analysis (if retained as requirement)

Deliverable: all required outputs previously produced by Python are available via Rust commands.

## Phase 3 — Validation hardening (in progress)

- [x] Golden test windows encoded in config
- [x] Deterministic run artifacts (JSON) with:
  - git SHA
  - config hash
  - window definitions
  - fitted params
  - residual metrics by room
- [ ] Regression checks in CI (where feasible)
- [ ] Add baseline comparison tooling (artifact-to-artifact diff and thresholds)

Deliverable: reproducible, auditable calibration history.

## Phase 4 — Controlled snapshot workflow (optional)

Snapshots are optional and must never be implicit.

- [ ] Add explicit snapshot command
- [ ] Require human confirmation (CLI flag + metadata)
- [ ] Record signoff reason in snapshot manifest

Deliverable: snapshots available for audit reproducibility without replacing direct-source operation.

## Phase 5 — House-input revalidation (after Rust conversion)

This phase is intentionally deferred until Rust conversion is complete.

- [ ] Revisit original XLSX source measurements (areas, U-values, radiator inventory, geometry)
- [ ] Ingest/compare new physical house-plan photos for dimensional validation
- [ ] Reconcile measured inputs against Rust/TOML model inputs
- [ ] Remove/replace any remaining fudge factors with measured or physically-derived values
- [ ] Re-run calibration + holdout validation with the corrected inputs

Deliverable: model inputs are measurement-backed and traceable to source artifacts.

## Definition of done for “Python fully migrated”

Migration is complete when all are true:

1. No production/analysis workflow depends on Python.
2. Equivalent or better outputs are available from Rust CLI.
3. Runtime behavior is config-driven via TOML.
4. Default data source is direct InfluxDB.
5. Snapshot path is explicit and gated by human signoff.
6. Python files are either removed or clearly marked `legacy/` and unused.

## Operating policy during migration

Until full migration is complete:

- Rust outputs are authoritative when command exists.
- Python may be used only for exploratory comparisons.
- Any parameter change must be made in TOML first, then consumed by Rust.
- Never rely on stale CSV extracts for canonical conclusions.

## Risks and mitigations

1. **Model drift between Python and Rust**
   - Mitigation: side-by-side comparison on fixed nights and room-level residuals.

2. **Overfitting calibration windows**
   - Mitigation: keep separate validation nights and report holdout error.

3. **Silent config sprawl**
   - Mitigation: keep a small number of TOML files with strict schema.

4. **Snapshot misuse**
   - Mitigation: explicit command + signoff requirement + manifest metadata.

## Immediate next actions

1. Port `rooms` summary to Rust (`thermal-rooms`) for day-to-day model sanity checks.
2. Port `analyse` summary to Rust (`thermal-analyse`) for daily energy-balance QA.
3. Port `connections` view to Rust (`thermal-connections`) for topology debugging parity.
4. Port equilibrium solver to Rust (`thermal-equilibrium`) to preserve intervention planning workflows.
5. Decide moisture migration timing: either keep Python temporarily or port to `thermal-moisture` for full retirement.
6. Complete thermal module split: extract `model.rs`, `calibration.rs`, `validation.rs`, `diagnostics.rs` from `src/thermal.rs`.
7. Enforce lint gates in workflow (`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo check`) and clear current project-wide clippy failures.
8. Add CI regression checks using saved thermal artifacts (fail on metric drift beyond threshold).
9. Mark `model/house.py` and `model/calibrate.py` as legacy once command parity is confirmed.
10. After Rust parity, execute Phase 5 input revalidation (XLSX + house-plan photos) before declaring model stable.
