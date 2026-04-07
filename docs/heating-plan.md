# Heating Plan

This page is a human-readable entry point to the heating strategy. It is a plan/explanation document, not the canonical source of current system truth. The current operational facts for the topics referenced here live in [`lat.md/`](../lat.md/).

## Where the current truth lives

Use these `lat.md` sections for the current operational truth behind this plan:

- [`lat.md/heating-control.md#Heating Control`](../lat.md/heating-control.md#heating-control) — controller objective, live behaviour, overnight strategy, DHW interaction
- [`lat.md/domain.md#Domain Model`](../lat.md/domain.md#domain-model) — tariff, house/domain assumptions, DHW contention facts
- [`lat.md/architecture.md#Live Control Path`](../lat.md/architecture.md#live-control-path) — live control path and data flow
- [`lat.md/constraints.md#Constraints`](../lat.md/constraints.md#constraints) — operational boundaries and gotchas

## Current status (7 Apr 2026, 10:34 BST)

V2 model-predictive controller is **live on pi5data** (`adaptive-heating-mvp` systemd service).

**What's working:**
- Coast-then-hold overnight logic: flat comfort-floor target (20.0°C), coast with `Z1OpMode=off` while Leather above floor, thermal solver for equilibrium hold. Replaced the linear ramp which back-loaded temperature rise and missed by 0.3°C. Deployed 7 Apr.
- Daytime model-predictive control: Open-Meteo forecast → thermal solver → target flow → curve.
- Inner loop (60s): proportional feedback on `Hc1ActualFlowTempDesired`, standby guard for `fd < 1.0`
- Powerwall telemetry readable (SoC, power flows) for observability
- DHW scheduling: T1 prediction, `HwcSFMode=load` trigger, timer fallback rails
- `energy-hub` headroom signal confirmed working since 5 Apr.
- Model runs every tick including during DHW — no more blind ticks (deployed 7 Apr).
- Build workflow: dev on laptop (`cargo check`), release build natively on pi5data (`cargo build --release`), sync via `scripts/sync-to-pi5data.sh`. Cross-compile from WSL2 fails (glibc mismatch).

**Recently fixed and deployed (7 Apr 2026):**
- **Overnight ramp → coast-then-hold** — flat 20.0°C target replaces linear ramp. Saves ~34% electrical overnight. Motivated by 6–7 Apr night where ramp caused 0.3°C miss.
- **Forecast nulls during DHW** — model calculation now runs every tick regardless of HP mode; action logged as `dhw_active` with full model fields. Was blind for up to 12 ticks per night.
- **DHW timer dedup bug** — `sync_morning_dhw_timer` now checks for `ERR:` in eBUS response and clears dedup state on failure; startup also clears dedup state.
- **τ revised 50→36h** — operational overnight cooling segments (8 independent observations). T1 decay revised 0.25→0.23 °C/h (P75 of 47 segments).

**Open items:**
- **Energy-hub headroom unreliable during Cosy windows**: doesn't account for active grid charging. No impact on control but misleading for observability.
- **Overnight data growing**: 5+ trajectory nights logged. Still need cold (<5°C) night.
- **Wind compensation and PV-aware curve**: modelled but not tuned. Low urgency.
- **DHW charge decision is T1-only, no draw prediction**: volume-aware demand budgeting per Cosy slot is the next DHW improvement.
- **Elvina overnight comfort**: trickle vents are the problem (ACH ≈1.0). Close vents + HEPA purifier is the proposed fix. No controller changes.

## What this page is for

Use this page when you want a quick human overview of where to read next.

The detailed current controller behaviour is maintained in `lat.md` because it changes with code and live operation. This page should stay short and should not duplicate controller rules, parameter tables, or decision logic.

## Supporting human-readable docs

These documents add explanation and field notes around the canonical plan:

- [Heating reference](heating-reference.md) — supporting notes and measurements
- [Room thermal model](room-thermal-model.md) — model background and methodology
- [History evidence workflows](history-evidence-workflows.md) — how to review what happened historically
- [VRC700 settings audit](vrc700-settings-audit.md) — controller and timer audit context

## When to use which source

- **Need the current truth?** Read [`lat.md/heating-control.md`](../lat.md/heating-control.md) first.
- **Need background or rationale?** Use the human docs linked above.
- **Need to change code or control behaviour?** Update `lat.md` alongside the code.
