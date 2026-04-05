# Heating Plan

This page is a human-readable entry point to the heating strategy. It is a plan/explanation document, not the canonical source of current system truth. The current operational facts for the topics referenced here live in [`lat.md/`](../lat.md/).

## Where the current truth lives

Use these `lat.md` sections for the current operational truth behind this plan:

- [`lat.md/heating-control.md#Heating Control`](../lat.md/heating-control.md#heating-control) — controller objective, live behaviour, overnight strategy, DHW interaction
- [`lat.md/domain.md#Domain Model`](../lat.md/domain.md#domain-model) — tariff, house/domain assumptions, DHW contention facts
- [`lat.md/architecture.md#Live Control Path`](../lat.md/architecture.md#live-control-path) — live control path and data flow
- [`lat.md/constraints.md#Constraints`](../lat.md/constraints.md#constraints) — operational boundaries and gotchas

## Current status (5 Apr 2026)

V2 model-predictive controller is **live on pi5data** (`adaptive-heating-mvp` systemd service).

**What's working:**
- Trajectory-based overnight logic: continuous Leather target ramp from 23:00–07:00, coast gate with `Z1OpMode=off`, thermal solver for preheat
- Daytime model-predictive control: Open-Meteo forecast → thermal solver → target flow → curve
- Inner loop (60s): proportional feedback on `Hc1ActualFlowTempDesired`, standby guard for `fd < 1.0`
- Powerwall telemetry readable (SoC, power flows) for observability
- DHW scheduling: T1 prediction, `HwcSFMode=load` trigger, timer fallback rails
- Cross-compiled for aarch64-musl, `rustls-tls` (no OpenSSL dependency)

**Open items:**
- `energy-hub` service not yet built — `discretionary_headroom_to_next_cosy_kWh` topic not published. Controller falls back gracefully (no active overnight DHW launch without headroom signal)
- First trajectory overnight will run tonight (5 Apr) — review results tomorrow
- Wind compensation and PV-aware curve adjustment are modelled but not yet tuned against real data

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
