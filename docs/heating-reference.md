# Heating Reference Data

Supporting reference and evidence for heating-control work. The canonical current-state controller behaviour, modes, baseline settings, and constraints live in [`../lat.md/heating-control.md`](../lat.md/heating-control.md), [`../lat.md/infrastructure.md`](../lat.md/infrastructure.md), and [`../lat.md/constraints.md`](../lat.md/constraints.md). For operating policy and narrative reasoning, see [Heating plan](heating-plan.md).

## What this file keeps

This file is for compact extra reference notes that are useful to humans but too detailed or too transient for the canonical `lat.md` summary.

## VRC 700 heat-curve formula reference

Useful as an intuition/initial-guess aid, not as the control law:

```text
flow_temp = setpoint + curve × (setpoint - outside)^1.25
```

Inverse:

```text
curve = (target_flow - setpoint) / (setpoint - outside)^1.25
```

Why this remains only a reference note:

- Vaillant documentation suggests exponent `1.10`, but field fit is closer to `1.25`
- the VRC 700 behaves like a black box with hidden floors and undocumented behaviour
- the live controller closes the loop on `Hc1ActualFlowTempDesired`, not on this formula

## SP=19 / night-mode rationale

The operational rules are in [`../lat.md/heating-control.md`](../lat.md/heating-control.md). The practical reasoning note preserved here is:

- running adaptive control with `Z1OpMode=night` and `SP=19` removes timer transitions and Optimum Start interference
- a low curve is **not** equivalent to off if `Hc1MinFlowTempDesired` is still imposing a floor
- this is why genuine coast uses `Z1OpMode=off`

## Inner-loop tuning notes

Canonical values live in [`../lat.md/heating-control.md#Inner Loop`](../lat.md/heating-control.md#inner-loop).

Field note retained here:

- low-curve operation is where the controller is most likely to hunt, because each `0.01` curve step can move target flow by roughly `0.2°C`

## Writable eBUS register notes

For the official write surface, use [`../lat.md/heating-control.md#Writable eBUS Registers`](../lat.md/heating-control.md#writable-ebus-registers) and [`../lat.md/constraints.md#eBUS Control Flow`](../lat.md/constraints.md#ebus-control-flow).

Extra notes worth preserving here:

- `Z1QuickVetoTemp` exists, but is not part of the normal adaptive-controller loop
- `SetModeOverride` to the HMU remains a future bypass option; direct HMU control is not the current operating model

## System pressure note

`FlowPressure` is the useful water-pressure register.

- Heating: ~2.01 bar
- DHW: ~1.90 bar
- Idle: ~2.05 bar

Interpretation:

- the small DHW dip is a hydraulic-circuit volume effect when the 3-way valve switches circuits
- `WaterPressure` on the 700 returns empty
- `RunDataHighPressure` is refrigerant pressure, not system water pressure

## Deployment and logging notes

Canonical deployment expectations live in [`../lat.md/architecture.md`](../lat.md/architecture.md) and [`../lat.md/infrastructure.md`](../lat.md/infrastructure.md).

Useful operator details retained here:

- binary path on `pi5data`: `/home/jack/adaptive-heating-mvp/target/release/adaptive-heating-mvp`
- state file: `/home/jack/.local/state/adaptive-heating-mvp/state.toml`
- JSONL decision log: `/home/jack/.local/state/adaptive-heating-mvp/actions.jsonl`
- metrics are also written to the `adaptive_heating_mvp` InfluxDB measurement

Typical rebuild on `pi5data`:

```bash
source ~/.cargo/env
cd ~/adaptive-heating-mvp
cargo build --release
```

## Resolved observations worth remembering

- `CurrentCompressorUtil` returns negative values in practice and is not trustworthy for control
- some apparent sawtooth behaviour was just normal mode transitions around DHW charges
- a service hang on 2 Apr was traced to missing reqwest timeouts and fixed with 10s HTTP timeouts
- a long Leather underperformance window on 2 Apr was consistent with the conservatory door being open, not controller instability
