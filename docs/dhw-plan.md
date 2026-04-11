# DHW Plan

This page is a human-readable entry point to the DHW strategy. It is a plan/explanation document, not the canonical source of current system truth. The current operational facts for the topics referenced here live in [`lat.md/`](../lat.md/).

## Where the current truth lives

Use these `lat.md` sections for the current operational truth behind this plan:

- [`lat.md/domain.md#DHW Cylinder`](../lat.md/domain.md#dhw-cylinder) — cylinder facts, charging behaviour, scheduling, contention with heating
- [`lat.md/domain.md#Cosy Tariff`](../lat.md/domain.md#cosy-tariff) — tariff structure and battery/headroom context
- [`lat.md/heating-control.md#Active DHW Scheduling`](../lat.md/heating-control.md#active-dhw-scheduling) — how the live controller currently makes DHW timing decisions
- [`lat.md/architecture.md#Live Control Path`](../lat.md/architecture.md#live-control-path) — how telemetry and control decisions flow through the system

## Current status

For live operational status and open items, see [`lat.md/plan.md`](../lat.md/plan.md). For current DHW scheduling behaviour, see [`lat.md/domain.md#DHW Cylinder`](../lat.md/domain.md#dhw-cylinder) and [`lat.md/heating-control.md#Active DHW Scheduling`](../lat.md/heating-control.md#active-dhw-scheduling).

**Summary as of 11 Apr 2026:** DHW scheduling is operational within the adaptive heating controller. Evening Cosy charge, T1 prediction for morning decisions, and `HwcSFMode=load` triggering all work. The main open software item remains draw prediction (volume-aware demand budgeting). See `lat.md/plan.md` for the full open items list.

## What this page is for

Use this page when you want a quick human overview of where to read next.

The detailed current DHW behaviour is maintained in `lat.md` because it changes with controller logic, battery inputs, and operating rules. This page should stay short and should not duplicate scheduling rules, thresholds, or control logic.

## Supporting human-readable docs

These documents add explanation and measurement context around the canonical plan:

- [DHW reference](dhw-reference.md) — supporting notes and field measurements
- [History evidence workflows](history-evidence-workflows.md) — how to inspect what happened in practice
- [Heating plan](heating-plan.md) — companion human entry point for the space-heating side

## When to use which source

- **Need the current truth?** Read [`lat.md/domain.md`](../lat.md/domain.md) and [`lat.md/heating-control.md`](../lat.md/heating-control.md) first.
- **Need background or rationale?** Use the human docs linked above.
- **Need to change code or scheduling behaviour?** Update `lat.md` alongside the code.
