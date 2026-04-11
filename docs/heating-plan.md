# Heating Plan

This page is a human-readable entry point to the heating strategy. It is a plan/explanation document, not the canonical source of current system truth. The current operational facts for the topics referenced here live in [`lat.md/`](../lat.md/).

## Where the current truth lives

Use these `lat.md` sections for the current operational truth behind this plan:

- [`lat.md/heating-control.md#Heating Control`](../lat.md/heating-control.md#heating-control) — controller objective, live behaviour, overnight strategy, DHW interaction
- [`lat.md/domain.md#Domain Model`](../lat.md/domain.md#domain-model) — tariff, house/domain assumptions, DHW contention facts
- [`lat.md/architecture.md#Live Control Path`](../lat.md/architecture.md#live-control-path) — live control path and data flow
- [`lat.md/constraints.md#Constraints`](../lat.md/constraints.md#constraints) — operational boundaries and gotchas

## Current status

For live operational status, open items, and review history, see [`lat.md/plan.md`](../lat.md/plan.md). For current controller behaviour, see [`lat.md/heating-control.md`](../lat.md/heating-control.md).

**Summary as of 11 Apr 2026:** V2 model-predictive controller is live on pi5data. Coast-then-hold overnight logic, daytime forecast-driven MPC, inner-loop proportional feedback, and DHW scheduling are all operational. See `lat.md/plan.md` for the current open items list.

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
