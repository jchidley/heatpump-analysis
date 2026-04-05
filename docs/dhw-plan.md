# DHW Plan

This page is a human-readable entry point to the DHW strategy. It is a plan/explanation document, not the canonical source of current system truth. The current operational facts for the topics referenced here live in [`lat.md/`](../lat.md/).

## Where the current truth lives

Use these `lat.md` sections for the current operational truth behind this plan:

- [`lat.md/domain.md#DHW Cylinder`](../lat.md/domain.md#dhw-cylinder) — cylinder facts, charging behaviour, scheduling, contention with heating
- [`lat.md/domain.md#Cosy Tariff`](../lat.md/domain.md#cosy-tariff) — tariff structure and battery/headroom context
- [`lat.md/heating-control.md#Active DHW Scheduling`](../lat.md/heating-control.md#active-dhw-scheduling) — how the live controller currently makes DHW timing decisions
- [`lat.md/architecture.md#Live Control Path`](../lat.md/architecture.md#live-control-path) — how telemetry and control decisions flow through the system

## Current status (5 Apr 2026)

DHW scheduling is **operational** within the adaptive heating controller.

**What's working:**
- Evening Cosy charge (22:00 window): reliable, T1 reaches 45°C, decays to ~43°C by 07:00 (well above 40°C floor)
- T1 prediction for morning decision: controller predicts cylinder-top temperature at 07:00
- `HwcSFMode=load` active trigger: fires when predicted T1 is below comfort floor and slot conditions met
- `HwcTimer_<Weekday>` fallback rails: maintained by controller as safety net for missed software launches
- DHW session analysis: `dhw-sessions` CLI writes `dhw_inflection` + `dhw_capacity` to InfluxDB
- `hmu HwcMode` (eco/normal) read for scheduling input

**Open items:**
- `energy-hub` headroom signal not yet published — overnight battery-backed DHW launches disabled until `emon/tesla/discretionary_headroom_to_next_cosy_kWh` is available. Impact: on cold depleted evenings, DHW may wait until morning Cosy rather than launching overnight on battery. Low risk — most nights T1 stays above 40°C.
- Seasonal eco→normal switch still manual (Nov–Mar). `hmu HwcMode` is read-only from eBUS.

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
