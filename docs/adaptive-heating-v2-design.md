# Adaptive Heating V2 — Model-Informed Control

Date: 1 April 2026

## The real control objective

The goal is not simply "keep Leather at 20.5°C." The real objective is:

1. **Day target: 21°C** — Z1DayTemp is set to 21. On cold days (<6°C outside), the 5kW HP cannot actually reach 21°C. Leather stabilises at 19.5–20°C regardless of strategy. The HP runs flat out and the house temperature is limited by HP sizing, not scheduling (see `docs/overnight-strategy-analysis.md`).

2. **Comfort band: 20.0–21.0°C** — below 20 feels cold, above 21 feels hot. The band is asymmetric in consequence: undershooting by 1°C is uncomfortable, overshooting by 1°C wastes energy and makes the room stuffy.

3. **Cosy tariff windows** — three cheap windows (04:00–07:00, 13:00–16:00, 22:00–00:00) at 14.05p/kWh vs 28.65p mid-peak and 42.97p peak. The Tesla Powerwall covers ~95% of non-Cosy usage, making the effective blended rate 14.63p/kWh. This means **tariff scheduling yields only £15–40/year** — the battery has already captured most of the arbitrage.

4. **DHW competes with heating** — the HP can't do both simultaneously. The diverter valve sends all flow to either the heating circuit or the DHW cylinder. A DHW charge takes 58 min (Normal) to 108 min (Eco). During that time, the house gets zero heating input and cools at ~0.2–0.5°C. DHW windows are aligned to Cosy periods (05:30–07:00, 13:00–15:00, 22:00–00:00).

5. **Night strategy: hit the target by morning, not maintain a floor overnight** — The current 19°C setback (00:00–04:00) is a V1 heuristic, not the real objective. Nobody cares what temperature the house reaches at 3am. What matters is Leather at 20–21°C by 07:00 at minimum cost. The 19°C setback wastes energy holding a floor nobody needs — on a mild night the house stays above 19°C anyway, and on a cold night the energy spent maintaining 19°C could be better spent on a targeted morning recovery. The real optimisation is a **calculated heating start time**: let the house cool freely, then start heating at the latest moment that still achieves 20°C in Leather by 07:00. The thermal model + outside temp forecast can calculate this directly.

6. **Cold day reality** — on <0°C nights, Leather averages 19.1°C at 08:00 and never reaches 21°C. On 0–3°C nights, Leather reaches 21°C at 15:00 on average. On 6–9°C nights, by noon. The controller must accept this reality rather than fighting it.

### What this means for V2

The controller should:
- **Target 20.5°C** (band midpoint) on mild days, accepting that 21°C is the ceiling not the floor
- **Accept 19.5–20°C** on cold days as the physical limit of the 5kW HP
- **Not waste energy overshooting** — every degree above 21°C is wasted
- **Protect DHW reliability** — DHW service must never be compromised by heating optimisation
- **Not chase Cosy windows** — the battery already handles tariff arbitrage; scheduling complexity yields minimal savings
- **Calculate overnight strategy dynamically** — use the thermal model + forecast to determine the optimal heating start time each night, rather than a fixed setback temperature
- **Plan around DHW charges** — know that a DHW charge is coming, don't panic when Leather dips during one

## What V1 taught us

The V1 bang-bang controller (add/subtract 0.10 to curve every 15 minutes) produced:
- Curve driven from 0.55→0.10 in 2 hours while Leather was still above band (stored heat dissipating)
- Curve driven from 0.10→1.00 in 2 hours while Leather was stuck at 19.7°C (thermal lag)
- Pointless repeated writes when the curve was already at floor
- A null-read bug that reset the curve and triggered unnecessary heating

The fundamental problem: **the controller had no model of the house.** It didn't know that Leather has a 15-hour thermal time constant, that MWT 28°C is sufficient at 13°C outside, or that a curve change takes hours to register as a room temperature change.

## What we already have

The thermal model (`thermal-equilibrium`) can already answer the key question:

> Given outside temperature X, what MWT produces Leather temperature Y at equilibrium?

From the model at 13°C outside:
- MWT 25°C → Leather 19.7°C
- MWT 28°C → Leather 21.1°C
- MWT 30°C → Leather 22.1°C

This is approximately linear in this range: **~0.5°C Leather per 1°C MWT.**

We also know:
- Leather thermal mass: 4,907 kJ/K
- Leather thermal time constant: ~15 hours
- Leather cooling rate (heating off): 0.18°C/h at ΔT=10°C
- Radiator output: `T50 × ((MWT - T_room) / 50)^1.3` where T50 = 4,752W
- Per-room fabric losses, ventilation rates, inter-room transfer coefficients
- All calibrated against two controlled cooldown experiments

## The V2 control concept

### Core idea

Instead of adjusting the curve blindly, the controller should:

1. **Calculate the target MWT** from current outside temperature + desired Leather temperature
2. **Set the flow temperature** (via curve/setpoint, or eventually direct SetMode)
3. **Wait** for the house to respond — not 15 minutes, but an appropriate fraction of the thermal time constant
4. **Re-evaluate** only when enough time has passed for the change to register

### Target MWT calculation

Build a lookup table (or use the equilibrium solver directly) that maps:

```
(outside_temp, target_leather_temp) → required_MWT
```

For the Leather comfort band of 20.0–21.0°C, target the midpoint: **20.5°C**.

Pre-computed examples:

| Outside °C | Target Leather | Required MWT | Notes |
|---|---|---|---|
| 0 | 20.5 | ~33 | Cold day, HP working hard |
| 5 | 20.5 | ~30 | Typical winter |
| 10 | 20.5 | ~27 | Mild |
| 13 | 20.5 | ~26 | Spring/autumn |
| 15 | 20.5 | ~25 | Near summer cutoff |

These come from running the equilibrium solver at each outside temp and interpolating the MWT that gives Leather = 20.5°C.

### Converting MWT to flow temperature

The VRC 700 demands a **flow temperature**, not an MWT. The relationship depends on the system ΔT:

```
flow_temp = MWT + ΔT/2
```

At our typical conditions, ΔT is 3–5°C (we see flow 28°C, return 24°C → ΔT=4, MWT=26). So:

```
target_flow = target_MWT + 2  (approximately)
```

This is observable — we read both flow and return from the HMU. The controller can track the actual ΔT and adjust.

### Setting the flow temperature

**V2a (keep VRC 700):** Reverse-engineer the curve/setpoint combination that produces the desired `Hc1ActualFlowTempDesired`. We can observe this: write curve X + setpoint Y, read back the flow demand, adjust. Or build an empirical lookup from the data we've already collected.

**V2b (bypass VRC 700):** Send SetModeOverride directly to the HMU with the desired flow temperature. This is cleaner but requires either disabling the 700 or outpacing its 30-second writes.

**V2c (VWZ AI standalone):** Remove the VRC 700, let the VWZ AI operate standalone, and write to it directly. The VWZ AI has its own heat curve and setpoint registers.

V2a is the safe first step. V2b/c are future options.

### Wait time — respecting thermal lag

Leather's time constant is ~15 hours. The 1-sigma response time (63% of the way to equilibrium) is 15 hours. But we don't need to wait for full equilibrium — we need to wait long enough to see the **direction** of change.

Practical wait time: **1–2 hours minimum** between MWT adjustments.

The controller should:
- Set the target MWT
- Log the expected equilibrium Leather temp from the model
- Wait at least 60 minutes
- Re-read Leather
- If Leather is moving in the right direction (towards target), hold
- If Leather is moving the wrong way or not moving, check whether the MWT is being achieved (flow temp correct?) before adjusting MWT further

### Edge cases

**Rapid disturbance (door opened, solar gain, occupancy change):**
The model predicts equilibrium. Short-term disturbances shouldn't trigger MWT changes. The controller should filter out transients — e.g., require Leather to be outside the band for 30+ minutes before acting.

**DHW interruption:**
When the HP switches to DHW, the radiators cool. Leather will drift down during a long DHW charge. The controller should not react to this — it knows DHW is active and the heating will resume. The model can predict how much Leather will drop during a typical DHW charge.

**Defrost:**
Similar to DHW — temporary interruption, don't react.

**Outside temperature changing:**
Recalculate target MWT when outside temp changes significantly (>1°C). The model handles this naturally.

## Implementation sketch

### New module: `src/thermal/control.rs`

```rust
/// Calculate the MWT that produces target_leather_temp at the given outside temp.
/// Uses bisection on the equilibrium solver.
pub fn target_mwt_for_leather(
    outside_temp: f64,
    target_leather: f64,
    rooms: &HashMap<String, Room>,
    connections: &[Connection],
    doorways: &[Doorway],
) -> f64 {
    // Bisection: find MWT where equilibrium leather temp = target
    let mut lo_mwt = 15.0;
    let mut hi_mwt = 55.0;
    for _ in 0..50 {
        let mid = (lo_mwt + hi_mwt) / 2.0;
        let leather_eq = solve_equilibrium_leather(mid, outside_temp, rooms, connections, doorways);
        if leather_eq < target_leather {
            lo_mwt = mid;
        } else {
            hi_mwt = mid;
        }
    }
    (lo_mwt + hi_mwt) / 2.0
}
```

### Changes to `adaptive-heating-mvp.rs`

Replace the current occupied-mode logic:

```
// V1 (current):
if leather < 20.0 { curve += 0.10 }
if leather > 21.0 { curve -= 0.10 }

// V2 (model-informed):
let target_mwt = target_mwt_for_leather(outside_temp, 20.5, &model);
let target_flow = target_mwt + actual_dt / 2.0;
let required_curve = flow_to_curve(target_flow, outside_temp);  // reverse lookup
if (required_curve - current_curve).abs() > 0.05 {
    write_curve(required_curve);
    last_adjustment = now;
}
// Don't re-evaluate for at least 60 minutes
```

### Data we need to collect first

Before implementing V2, we should build the **curve→flow temp lookup** from the data the pilot has already generated:

| Curve | Outside | Resulting flow demand |
|---|---|---|
| 0.10 | 12°C | 21.0°C |
| 0.20 | 12°C | 22.2°C |
| 0.30 | 12°C | 24.1°C |
| 0.40 | 12°C | 26.4°C |
| 0.50 | 12°C | 28.8°C |
| 0.55 | 16°C | 26.8°C |
| ... | ... | ... |

We already have this from the JSONL logs! Every decision logged `curve_before`, `outside_temp`, and `flow_desired`. We can fit the VRC 700's algorithm empirically.

## VRC 700 heat curve formula

From the Vaillant installation manual (p15) + empirical pilot data:

```
flow_temp = setpoint + curve × (setpoint - outside)^1.27
```

Fitted against 13 pilot data points with RMSE 0.74°C. This allows us to convert between "what flow temp do we want" and "what curve value to write."

Inverse (what curve for a target flow):
```
curve = (target_flow - setpoint) / (setpoint - outside)^1.27
```

Examples at setpoint 21°C:
| Outside | Target flow | Required curve |
|---|---|---|
| 13°C | 29°C | 0.57 |
| 5°C | 32°C | 0.33 |
| 0°C | 35°C | 0.29 |

Note: at 13°C outside, the baseline curve of 0.55 was already nearly optimal. The V1 controller drove it to 0.10 and 1.00 unnecessarily.

## DHW in V2

DHW policy remains as specified in V1 (`docs/adaptive-heating-mvp.md`), but V2 must actively account for DHW in its planning:

1. **Predict DHW timing** — the controller knows the DHW timer windows (05:30–07:00, 13:00–15:00, 22:00–00:00) and can anticipate when DHW will fire based on cylinder temperature vs trigger threshold (40°C).

2. **Don't react to DHW-induced cooling** — when DHW is active, Leather will cool at ~0.2–0.5°C. The controller should not adjust the heating curve in response. It should log "DHW active, holding" and wait.

3. **Pre-compensate if needed** — if the model predicts that a DHW charge will push Leather below 20°C, the controller could pre-raise the curve slightly before DHW starts. This is a V2+ optimisation, not V2 launch.

4. **DHW boost remains via HwcSFMode** — the V1 mechanism (trigger boost during Cosy windows when cylinder is below threshold) is correct and should be kept.

5. **DHW mode (Eco vs Normal) is not controllable via eBUS** — hmu HwcMode is read-only. The seasonal manual switch remains.

## What this gives us

**V1**: curve oscillates 0.10↔1.00, constant writes, 15-minute thrashing
**V2**: controller calculates MWT 27°C, sets curve once, waits 2 hours, confirms Leather is at 20.5°C, done

The model turns a feedback control problem (measure→react→overshoot→correct) into a feedforward control problem (calculate→set→verify). The thermal model is the feedforward path. Room temperature is the feedback for long-term trim only.

## Still learning

The V2 approach assumes the equilibrium model is accurate enough to be useful. The pilot data will validate this — if the model says MWT 27°C gives Leather 20.5°C, and reality shows 19.5°C, we calibrate the model.

The curve→flow temp mapping may also drift with VRC 700 firmware or `AdaptHeatCurve` behaviour. The controller should always verify `Hc1ActualFlowTempDesired` matches expectations after a write.

Eventually (V2b/V2c), we eliminate the curve entirely and set flow temp directly. The thermal model remains the same — only the actuator changes.
