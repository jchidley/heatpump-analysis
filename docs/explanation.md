# How the Operating Model Works

This document explains the reasoning behind the operating-state model. For the current canonical thresholds, house facts, and DHW scheduling assumptions, see [`../lat.md/domain.md`](../lat.md/domain.md) and [`../lat.md/constraints.md`](../lat.md/constraints.md).

## What this file is for

Use this page for the high-level reasoning behind the state model, not for current numeric truth.

## Core idea

The Vaillant aroTHERM Plus 5kW has a software-clamped heating flow on the space-heating circuit and a clearly higher flow on the DHW circuit when the diverter valve switches over. That mechanical separation made flow-based classification much more reliable than temperature- or flag-based approaches.

### Flow-rate separation by model

The current project is tuned for the 5kW model.

| Model | Typical heating flow | Why it matters |
|---|---|---|
| 3.5 / 5 kW | ~860 L/h = 14.3 L/min | clean separation from DHW flow band |
| 7 kW | ~1,200 L/h = 20.0 L/min | overlaps the 5kW-style DHW band, so thresholds would need redesign |
| 10 / 12 kW | ~2,000 L/h = 33.3 L/min | very different hydraulic signature |

The enduring lesson is that the classifier depends on the installed unit size. The repo's historical thresholds and reasoning are for the 5kW machine at this house.

## Four-state model

The live thresholds are maintained in [`../lat.md/domain.md#Operating States`](../lat.md/domain.md#operating-states). The model still resolves four practical operating states:

- **Idle** — compressor effectively off
- **Heating** — positive heat output in the heating flow band
- **DHW** — positive heat output in the DHW flow band
- **Defrost** — reverse-cycle heat extraction from the water circuit

## Why flow rate won

Earlier alternatives were weaker:

- **flow temperature** missed DHW ramp-up and late-stage DHW behaviour
- **`DHW_flag` feed** died historically, so it could not classify the full dataset
- **eBUS `StatuscodeNum`** is useful context but unreliable for DHW classification on its own

The key design reason is simple: the diverter valve is binary, so flow rate gives a cleaner state boundary than continuous temperatures do.

## Hysteresis rationale

The diverter valve takes time to move, so transition samples briefly pass through the middle band. Hysteresis prevents chatter when the valve is switching. The exact thresholds are current-state truth in `lat.md`; the reasoning remains that transition noise should not cause rapid mode flapping.

## Defrost reasoning

Defrost is identified by the sign of heat transfer, not just by “the machine looks active”. During defrost the unit is still consuming electricity, but thermal energy briefly flows the wrong way, so negative heat / negative delta-T is the key physical signal.

## Gap-filling reasoning

The monitoring system sometimes loses instantaneous telemetry even while cumulative meters continue. Gap filling therefore uses cumulative meters as the energy anchor and only estimates the minute-level shape. That keeps total energy honest while marking the detailed profile as synthetic unless explicitly requested.

## Monitoring context

The operating model was built around the emonHP-era feed set plus newer eBUS and Multical inputs. The current source roles live in:

- [`../lat.md/domain.md`](../lat.md/domain.md)
- [`../lat.md/infrastructure.md`](../lat.md/infrastructure.md)

The enduring reasoning points are:

- eBUS gives internal heat-pump state and control context
- Multical gives independent DHW comfort truth
- the emon heat meter remains the trusted heat-accounting view

## Scheduling implication

The state model matters because the household uses different evidence for different questions:

- **space heating** asks whether the controller met comfort and preheat goals
- **DHW** asks whether cylinder comfort remained practical for the family pattern
- **history review** needs the model to separate overlap windows rather than smearing them together

That is why joined heating+DHW review exists instead of one flat “heat pump active” concept.

## Validation posture

The model was accepted because it matched the physical system better than the discarded alternatives, not because one pretty chart happened to agree once. When revisiting it, prefer repeated checks across heating, DHW, defrost, and overlap periods.

## Related documents

- [Heating plan](heating-plan.md)
- [DHW plan](dhw-plan.md)
- [Hydraulic analysis](hydraulic-analysis.md)
- [History evidence workflows](history-evidence-workflows.md)
