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
- `thermal-operational` (implemented — full operational validation with heating/DHW/off, solar gain, BCF-based state classification)
- `thermal-rooms` (planned)
- `thermal-analyse` (planned)
- `thermal-connections` (planned)
- `thermal-equilibrium` (planned)
- `thermal-moisture` (planned / lower priority)
- `thermal-report` (planned)
- `thermal-snapshot export|import` (implemented, explicit + signed-off)

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
- [x] Match/verify all relevant formulas vs `model/house.py`
- [x] Document intentional deltas
  - See `docs/rust-thermal-formula-parity.md`

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
- [x] Regression checks in CI (where feasible)
- [x] Baseline comparison tooling (artifact-to-artifact diff + thresholds)
  - `src/bin/thermal-regression-check.rs`
  - `artifacts/thermal/regression-thresholds.toml`
  - `scripts/thermal-regression-ci.sh`
  - `.github/workflows/thermal-regression.yml`
- [x] Baseline coverage for all implemented thermal commands
  - [x] `artifacts/thermal/baselines/thermal-calibrate-baseline.json`
  - [x] `artifacts/thermal/baselines/thermal-validate-baseline.json`
  - [x] `artifacts/thermal/baselines/thermal-fit-diagnostics-baseline.json`

### Phase 3 operating procedure (baseline lifecycle)

1. Generate fresh artifacts locally for implemented thermal commands.
   - `cargo run --bin heatpump-analysis -- thermal-calibrate --config model/thermal-config.toml`
   - `cargo run --bin heatpump-analysis -- thermal-validate --config model/thermal-config.toml`
   - `cargo run --bin heatpump-analysis -- thermal-fit-diagnostics --config model/thermal-config.toml`
   - Ensure `INFLUX_TOKEN` is exported before running (from a secure source).2. Copy the intended baseline artifacts into `artifacts/thermal/baselines/` using fixed names (or use `scripts/refresh-thermal-baselines.sh`):
   - `thermal-calibrate-baseline.json`
   - `thermal-validate-baseline.json`
   - `thermal-fit-diagnostics-baseline.json`
3. Run `bash scripts/thermal-regression-ci.sh` and confirm all gates pass.
   - This script is now strict: missing baselines are a hard failure (no skip path).
4. If changes are intentional and reviewed, update baseline JSON(s) in the same PR as code/config changes.
   - Baseline refresh is automated by `scripts/refresh-thermal-baselines.sh`.
5. Keep threshold edits in `artifacts/thermal/regression-thresholds.toml` tightly justified in PR notes.
6. Never relax thresholds and change model logic in the same unreviewed commit.

### Phase 3 exit criteria

Phase 3 is complete when all are true:

1. Baselines exist for all implemented thermal commands.
2. `scripts/thermal-regression-ci.sh` passes locally and in CI.
3. Threshold file changes are exceptional and explicitly justified.
4. Artifact schema remains backward-compatible or is version-bumped with migration notes.

Deliverable: reproducible, auditable calibration history.

## Phase 4 — Controlled snapshot workflow (optional)

Snapshots are optional and must never be implicit.

- [x] Add explicit snapshot command
  - `thermal-snapshot export`
  - `thermal-snapshot import`
- [x] Require human confirmation (CLI flag + metadata)
  - `--approved-by-human` (required)
- [x] Record signoff reason in snapshot manifest
  - `--signoff-reason` required on export/import; persisted in `manifest.json`

Deliverable: snapshots available for audit reproducibility without replacing direct-source operation.

## Phase 5 — House-input revalidation (after Rust conversion)

This phase is intentionally deferred until Rust conversion is complete.

- [x] Revisit original XLSX source measurements (areas, U-values, radiator inventory, geometry)
- [x] Ingest/compare new physical house-plan photos for dimensional validation (via canonical scan transcription + inventory pipeline)
- [x] Reconcile measured inputs against Rust/TOML model inputs
- [x] Remove/replace any remaining fudge factors with measured or physically-derived values (remaining assumptions/calibration terms are explicit + documented)
- [x] Re-run calibration + holdout validation with the corrected inputs

See execution report: `docs/phase5-house-input-revalidation-2026-03-28.md`

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

### Remaining Python→Rust ports (5 commands)

1. **`thermal-rooms`** — Room summary table (area, volume, thermal mass, T50, ext UA, ACH, pipe branch). Pure geometry, no InfluxDB. Low complexity.
2. **`thermal-connections`** — Internal wall UA + doorway table. Pure geometry. Low complexity.
3. **`thermal-analyse`** — Live energy balance snapshot: query latest room temps + HP state from InfluxDB, compute per-room heat flows at current instant. Medium complexity. High daily-use value. Most of the code exists in `thermal-operational`.
4. **`thermal-equilibrium`** — Steady-state solver: given T_out + MWT, find room temps where all energy balances = 0. Python uses `scipy.fsolve` (Newton). Rust options: Gauss-Seidel relaxation (well-conditioned system), or `argmin`/`nalgebra` crate. High complexity, high strategic value (intervention planning).
5. **`thermal-moisture`** — Overnight humidity analysis: AH tracking, surface RH, ACH cross-validation. Needs humidity queries + Magnus formula + Open-Meteo. Medium complexity. Lower priority — used occasionally for mould risk.

After all 5 are ported, mark `model/house.py` as legacy and retire Python from the thermal model.

### Infrastructure and quality

6. Complete thermal module split: extract `model.rs`, `calibration.rs`, `validation.rs`, `diagnostics.rs` from `src/thermal.rs` (~3000 lines).
7. Enforce lint gates in workflow (`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo check`) and clear current project-wide clippy failures.
8. Maintain and review thermal regression baselines (`artifacts/thermal/baselines/*`) when intentional model changes are made.
   - Use `scripts/refresh-thermal-baselines.sh` after generating fresh artifacts.
9. Add `thermal-operational` baseline to regression CI once operational output stabilises.

### Model improvements (identified from operational validation, 28 Mar 2026)

10. **Kitchen**: positive bias +0.245 — bare CH pipes in floor void provide unmodelled heat (~25W each side). Add pipe heat term to kitchen + bathroom energy balance.
11. **Bathroom/shower room: shower event modelling** — positive bias +0.250 in bathroom. Shower draws dump significant heat AND moisture into the room: hot water warms the air directly, and evaporation from wet surfaces continues for 30+ min after. Detectable from emondhw Multical data already in InfluxDB: `emon/multical/dhw_flow` (2s, sub-litre), `emon/multical/dhw_t1` (outlet temp), `emon/multical/dhw_t2` (return), `emon/multical/dhw_power` (instantaneous heat). A sustained draw > 5 L/min is a shower; heat dumped = power × duration. The model should: (a) add transient heat input to bathroom (or shower room when back in use) during/after a detected draw, (b) add moisture load for the humidity/moisture model (evaporation from wet surfaces, steam). The MVHR in bathroom (Vent-Axia Tempra, 78% HR) recovers some heat but exhausts the moisture. Door state matters: open door spreads heat+moisture to landing/hall; closed door concentrates it.
12. **Office**: RMSE 1.020 — small room coupled to landing chimney. Will improve when landing chimney model is fixed.
13. **Landing chimney**: current ACH-to-outside model is structurally wrong for heated operation (wrong sign 9/14 periods). Needs bidirectional inter-floor air exchange model replacing outdoor ventilation assumption.
14. **Conservatory solar**: Open-Meteo NE irradiance is approximate. Future SE solar array (perpendicular to ground, pointing SE) will provide direct measurement. Architecture ready in `solar_gain_full`.
15. **Wind sensitivity**: conservatory ACH varies 2-8× with wind. Optional wind model exists (`[wind]` config) but disabled. Consider per-room wind coupling for leaky rooms.
