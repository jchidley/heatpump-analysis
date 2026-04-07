# Heating Plan

This page is a human-readable entry point to the heating strategy. It is a plan/explanation document, not the canonical source of current system truth. The current operational facts for the topics referenced here live in [`lat.md/`](../lat.md/).

## Where the current truth lives

Use these `lat.md` sections for the current operational truth behind this plan:

- [`lat.md/heating-control.md#Heating Control`](../lat.md/heating-control.md#heating-control) — controller objective, live behaviour, overnight strategy, DHW interaction
- [`lat.md/domain.md#Domain Model`](../lat.md/domain.md#domain-model) — tariff, house/domain assumptions, DHW contention facts
- [`lat.md/architecture.md#Live Control Path`](../lat.md/architecture.md#live-control-path) — live control path and data flow
- [`lat.md/constraints.md#Constraints`](../lat.md/constraints.md#constraints) — operational boundaries and gotchas

## Current status (6 Apr 2026, 08:30 BST)

V2 model-predictive controller is **live on pi5data** (`adaptive-heating-mvp` systemd service).

**What's working:**
- Trajectory-based overnight logic: continuous Leather target ramp from 23:00–07:00, coast gate with `Z1OpMode=off`, thermal solver for preheat. Two successful trajectory overnights: 5 Apr (Leather 20.5°C at 07:00, outside 9–12°C) and 6 Apr (Leather 20.1°C at 07:00, outside 7–9°C — slight undershoot due to DHW contention during preheat).
- Daytime model-predictive control: Open-Meteo forecast → thermal solver → target flow → curve. Today: curve ranged 0.60–0.90, correctly responding to solar gain and evening cooling.
- Inner loop (60s): proportional feedback on `Hc1ActualFlowTempDesired`, standby guard for `fd < 1.0`
- Powerwall telemetry readable (SoC, power flows) for observability
- DHW scheduling: T1 prediction, `HwcSFMode=load` trigger, timer fallback rails
- `energy-hub` headroom signal **confirmed working** — deployed ~22:20 BST 5 Apr, publishing every 10s since, controller receiving non-null values every tick since 22:30 BST. Was null for 22 ticks (16:51–22:14) before the energy-hub was deployed to emonpi.
- Cross-compiled for aarch64-musl, `rustls-tls` (no OpenSSL dependency)

**Recently fixed (6 Apr 2026):**
- **Forecast nulls during DHW** — model calculation now runs every tick regardless of HP mode; action logged as `dhw_active` with full model fields. Was blind for up to 12 ticks per night.
- **DHW timer dedup bug** — `sync_morning_dhw_timer` now checks for `ERR:` in eBUS response and clears dedup state on failure; startup also clears dedup state. Was leaving morning timer window enabled after failed skip writes.
- **T1 standby decay rate** — recalibrated from 47 flow-filtered standby segments: P75 0.23°C/h (was 0.25, directionally correct but now properly measured).

**Open items:**
- **Energy-hub headroom unreliable during Cosy windows**: doesn't account for active grid charging — shows -9.3 kWh at SoC 33%, then +4.6 at SoC 51%. No impact on control (controller ignores headroom during Cosy) but misleading for observability. Energy-hub fix needed: return null or project through Cosy charging.
- **Overnight data growing**: 3 trajectory nights (4 Apr confounded, 5 Apr success 9–12°C, 6 Apr success 7–9°C with DHW contention). Still need cold (<5°C) and warm (>12°C) nights. No code changes — observe and record.
- **Wind compensation and PV-aware curve**: modelled but not tuned against real data. Low urgency until weather provides test cases.
- **DHW charge decision is T1-only, no draw prediction**: controller predicts standby T1 at 07:00 but has no model of overnight draws. On nights with late showers (47% of nights, avg 62L), the prediction can be 5°C+ optimistic. Volume-aware demand budgeting per Cosy slot is the next DHW improvement.

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
