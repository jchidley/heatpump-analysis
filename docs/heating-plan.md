# Heating Plan

Adaptive space heating control for 6 Rhodes Avenue. Vaillant Arotherm Plus 5kW, VRC 700 controller, 13 room sensors, calibrated thermal model.

## Objective

**Leather room 20–21°C during waking hours (07:00–23:00) at minimum electricity cost.**

Nobody cares what temperature the house reaches at 3am. The overnight temperature is constrained by HP reheat capacity, not a target.

## Constraints

| Constraint | Value | Source |
|---|---|---|
| HP thermal output | 5kW max | Arotherm Plus 5kW spec |
| House HTC | 261 W/K | Calibrated thermal model |
| HP deficit below | ~2°C outside (5kW < 5.9kW loss at -2°C) | Measured — accept 19.5–20°C |
| No heating needed above | 17°C outside | Empirical — solar/internal gains sufficient |
| Max useful flow temp | 45°C | Emitter capacity + COP limit |
| House time constant | 26h (Leather ~15h) | Calibrated from 27k DHW minutes |
| Cooling rate (k) | 0.039/hr per °C ΔT | Calibrated from 27k DHW minutes (genuine no-heating). **Not** the idle-cycle rate (k=0.014) which is 3× too slow because surrounding rooms stay warm from recent heating |
| Thermal capacity | 6,723 Wh/°C (τ = 25.8h) | Derived from k and HTC |
| DHW steals HP for | 50–100 min per charge | eco ~100 min, normal ~60 min |
| Emitters | 15 radiators (no TRVs), Sterling off | No per-room flow control |
| Sensors | 13 rooms (12× SNZB-02P + 1 emonth2) | ~5 min update rate |

### HP capacity vs outside temperature

| Outside | Heat loss | HP surplus | Overnight drop (8h no heat) | Min floor (3h reheat to 20.5°C) |
|---|---|---|---|---|
| -2°C | 5872W | **deficit** | Not recoverable | Must heat continuously |
| 0°C | 5350W | **deficit** | Not recoverable | Must heat continuously |
| 2°C | 4828W | 172W | 7.7°C | 20.4°C |
| 5°C | 4046W | 954W | 6.6°C | 19.7°C |
| 8°C | 3262W | 1738W | 5.4°C | 19.0°C |
| 10°C | 2740W | 2260W | 4.5°C | 18.6°C |
| 14°C | 1696W | 3304W | 2.9°C | 17.7°C |

Below ~2°C the HP runs flat out and can barely maintain 20°C. Scheduling is irrelevant — the HP never stops.

## Tariff

Octopus Cosy, three windows:

| Rate | Price | Times |
|---|---|---|
| **Cosy** | 14.05p/kWh | 04:00–07:00, 13:00–16:00, 22:00–00:00 |
| **Mid-peak** | 28.65p/kWh | 00:00–04:00, 07:00–13:00, 19:00–22:00 |
| **Peak** | 42.97p/kWh | 16:00–19:00 |
| **Battery effective** | 14.63p/kWh | Powerwall covers ~95% of non-Cosy usage |

The battery already captures most tariff arbitrage. From 512 days of data: effective cost £912 vs £1596 naive (no battery). Total scheduling optimisation yields £15–40/year. Cost difference between Cosy and battery-effective is 0.58p/kWh — negligible for heating. The real value of Cosy alignment is **protecting the battery for peak hours on cold days** when the HP runs flat out.

## Control surface

### VRC 700 heat curve formula

```
flow_temp = setpoint + curve × (setpoint - outside)^1.27
```

Exponent 1.27 fitted from Vaillant manual + 70 pilot data points (RMSE 0.63°C). Vaillant says 1.10 — underpredicts by 2.5–3.1°C at curves ≥0.50.

Inverse: `curve = (target_flow - setpoint) / (setpoint - outside)^1.27`

### Primary levers

| Register | Role | Notes |
|---|---|---|
| `Hc1HeatCurve` | Flow temp gradient (0.10–4.00) | Primary control. 0.01 step ≈ 0.20°C flow change |
| `Z1OpMode` | Operating mode | Set to 3 (night) on startup → permanent SP=19 |

### Why SP=19 (permanent night mode)

Three setpoints analysed. SP=19 chosen because:
- Curve 0.10 = genuinely zero rad output (no formula leakage)
- Any overnight heating is a deliberate curve raise, not accidental
- Curves stay under 1.50 warning up to 15°C outside
- No heating runs above 17°C anyway

**Why not Z1OpMode=auto?** The VRC 700 has undocumented Optimum Start: at 03:00 (3h before 06:00 day timer), `Hc1ActualFlowTempDesired` jumped from 21.0°C to 22.3°C with curve at 0.10. No register to disable it. Night mode eliminates Optimum Start, CcTimer transitions, and day/night setpoint switches — giving the controller full authority.

On shutdown/kill: **baseline restore** writes `Z1OpMode=auto`, `Hc1HeatCurve=0.55`. VRC 700 resumes autonomous timer control. Crash without restore: house sits at 19°C with last curve — safe.

### Other writable registers

| Register | Purpose |
|---|---|
| `Z1DayTemp` / `Z1NightTemp` | Room setpoint (shifts curve up/down) |
| `Hc1MaxFlowTempDesired` / `Hc1MinFlowTempDesired` | Flow temp bounds |
| `HwcSFMode` | DHW boost trigger (auto / load) |
| `HwcTempDesired` | DHW target temp |
| `Z1QuickVetoTemp` | Temporary override |

Future option: `SetModeOverride` directly to HMU bypasses VRC 700 entirely. Message format decoded (D1C encoding). Not yet used.

### VRC 700 is opaque

Back-solving pilot data gives effective setpoint ~20°C (neither `Z1NightTemp`=19 nor `Z1DayTemp`=21). Hidden `Hc1MinFlowTempDesired`=20°C floor, undocumented Optimum Start ramp. **Do not model the VRC 700 formula. Treat as black box. Inner loop closes on measured `Hc1ActualFlowTempDesired`.**

## Control approach

### Two-loop model-predictive control

```
Outer loop (every 15 min):
    thermal model: (forecast outside, solar) → required MWT for Leather 20.5°C
    target_flow = MWT + ΔT/2
    initial curve = (target_flow - 19) / (19 - outside)^1.25

Inner loop (every ~60s):
    error = target_flow - Hc1ActualFlowTempDesired
    if |error| > deadband:
        curve += gain × error      (max step 0.20, clamp 0.10–4.00)
        write Hc1HeatCurve
```

**Inner loop tuning**: gain=0.05, deadband=0.5°C, max_step=0.20, curve clamped to 0.10–4.00 (trust the VRC 700's accepted range — no extra software limits). Below curve 0.25: gain halved to 0.025, deadband doubled to 1.0°C. Converges in 1–2 ticks.

**ΔT stabilisation**: uses live flow-return ΔT only when `RunDataStatuscode` contains "Heating" + "Compressor". Otherwise `default_delta_t_c` = 4.0°C.

**No runtime learning**: `room_offset` EMA was tried and removed — it ran away to +2.18°C overnight, learning the cooling trend as "model error" and suppressing preheat by ~8°C (target_flow 23.5°C when 31.2°C was needed). If systematic model bias appears, apply a static calibration offset.

### Comfort guard and COP optimisation

The controller has layered priorities:

1. **Comfort guard** (hard constraints): any heated room < 18°C → raise curve. `CurrentCompressorUtil` > 95% for >30 min → hold (HP at capacity). DHW active → don't adjust
2. **COP optimisation**: gradient-follow — step toward better COP, stop when rooms cool or COP plateaus
3. **Context**: tariff (bank during Cosy, coast during expensive), door states, occupancy, forecast

### Operating modes

| Mode | Behaviour |
|---|---|
| `occupied` | Full comfort targeting, all layers active |
| `short-absence` | Mild setback, cost bias |
| `away` | 15°C frost protection, warm-up ramp before return |
| `disabled` | No writes, monitoring only |
| `monitor-only` | Read sensors + log, no eBUS writes |

HTTP API on port 3031: `POST /mode/occupied`, `/mode/away`, `/mode/disabled`, `/mode/monitor-only`, `/kill`. Kill switch triggers immediate baseline restore and stops all eBUS writes.

### Logging

Every decision logged to:
- **InfluxDB** (`adaptive_heating` measurement): curve, target_flow, leather_temp, outside_temp, action, mode
- **Local JSONL** on pi5data: full decision context for debugging

### Room priorities

- **Leather** (primary): emonth2, 15h time constant. Optimise for this when doors closed
- **Aldora** (secondary): second comfort anchor
- **Conservatory**: excluded — 30m² glass, sub-hour time constant, follows outdoor + solar
- **Other rooms**: constraints and context, not targets

**Leather door sensors** (2× SONOFF SNZB-04P, in hand, not fitted). One on conservatory door, one on hall door. Three-stage plan:

**Stage 1: Fit + pair + log.** Pair to Z2M on emonpi. Name: `leather_conservatory_door`, `leather_hall_door`. Telegraf picks up MQTT → InfluxDB automatically. Add topics to `adaptive-heating-mvp.toml` and query in outer loop (same pattern as T1 query). Log `door_conservatory` and `door_hall` (true/false) in every decision log entry. **No control changes yet** — just collect data.

**Stage 2: Analyse (1–2 weeks of data).** Correlate door state with leather temp trajectory. Quantify: how much does leather drop per hour with conservatory door open at various outside temps? How quickly does it recover after closing? Does the model MWT need adjustment for door-open conditions, or is the current MWT correct and leather just can't reach target with the door open?

**Stage 3: Control integration.** Based on Stage 2 data:
- Conservatory door open: suppress target_flow increase (don't chase unreachable target). Hold current curve or reduce slightly. Log "door open, holding" as action.
- Conservatory door closed after being open: immediate outer loop recalc (event-driven trigger) to resume normal targeting.
- Hall door: likely smaller effect, may not need special handling. Data will tell.
- Both doors open: leather is a corridor, not a room — switch primary target to aldora.

## Overnight strategy

The controller calculates the latest heating start time that achieves Leather ≥ 20°C by 07:00.

### Algorithm

1. Simulate cooling: exponential decay with τ=15h toward equilibrium (outside + 2.5°C internal gains)
2. At each 30-min step backward from 07:00: can HP reheat from here to 20.5°C in time?
3. Reheat rate: 7500W per °C/h from pilot data (HP surplus / 7500 = °C/h rise)
4. Latest safe start with 30-min safety margin
5. Below 2°C outside: maintain 19.5°C continuously (HP can't recover from any drop)

### Heating recovery by outside temperature

From emoncms data (heating state, indoor_t rising):

| Outside | Heat output | Electricity | COP | MWT |
|---|---|---|---|---|
| -2–0°C | 5700W | 1849W | 3.08 | 30.5°C |
| 2–4°C | 5180W | 1420W | 3.65 | 31.3°C |
| 6–8°C | 4045W | 841W | 4.81 | 30.2°C |
| 10–12°C | 2913W | 481W | 6.06 | 28.3°C |

Reheat rate for overnight planner: empirical 7500W per °C/h (HP_surplus / 7500 = °C/h rise). Calibrated from 2 data points — needs more overnight runs.

### Three overnight actions

| Action | Curve | When |
|---|---|---|
| `overnight_coast` | 0.10 (zero output) | House warm enough, free cooling |
| `overnight_preheat` | Model curve + inner loop | Calculated start time reached |
| `overnight_maintain` | Continuous heating at 19.5°C | Below 2°C outside |

### Known limitations

- Reheat rate calibrated from 2 data points (needs more overnight runs)
- Solar gain not included in reheat estimate (conservative — will overshoot on sunny mornings)
- Uses average overnight outside temp (should use hourly forecast from Open-Meteo)

## Away mode

1. **Trigger**: API endpoint `/api/heating/away` or config
2. **Setpoint**: 15°C, curve 0.30 — frost protection only. Costs ~£0.50/day vs ~£2.50 at 21°C
3. **Warm-up ramp**: thermal model computes lead time from current temp + forecast. At 7°C outside: 15→21°C takes ~20h at full power. Ramp in two stages: 15→18°C (curve 0.45), then 18→21°C (curve 0.55)
4. **Forecast adjustment**: cold snap → start earlier. Mild → start later
5. **A week away saves ~£14**

## HP contention with DHW

Each DHW charge blocks heating for 50–100 minutes. Known issue: on 1–2 Apr, DHW stole 1.5h of preheat (cylinder drifted to 39.5°C, barely below 40°C trigger). Leather dropped from 20.1→19.9°C, below comfort by 07:15. Phase 2 T1-based scheduling will fix this by sequencing DHW and preheat explicitly.

Impact depends on outside temperature:

| Outside | HP surplus for heating | DHW cost in comfort |
|---|---|---|
| -2°C | Deficit | ~0.5°C Leather drop per charge (unrecoverable) |
| 5°C | 954W | ~0.3°C drop, recovers in ~1h |
| 10°C | 2260W | ~0.2°C drop, recovers in ~30 min |
| 15°C | 3826W | Negligible |

On cold days (<5°C), every DHW charge matters. Scheduling DHW in the 22:00–00:00 Cosy window frees the 04:00–07:00 window for uninterrupted preheat. On mild days, it doesn't matter. See [DHW plan](dhw-plan.md).

## Physical improvements

| Priority | Action | Cost | Impact |
|---|---|---|---|
| 1 | Close Elvina trickle vents | FREE | Removes system bottleneck — MWT 49→47°C at -3°C |
| 2 | Aldora rad upgrade (reuse existing 909W DP DF) | FREE | MWT 47→45°C |
| 3 | Jack&Carol bay window draught-strip | ~£30 | 60–150W saving |
| 4 | EWI on SE wall (~30m²) | ~£5k DIY | 19% heat demand reduction. MWT 49→43°C at -3°C |
| 5 | Sterling floor insulation | ~£200 | Leather keeps heat, Sterling gets cold room |

FRVs deprioritised — HP at capacity on cold days, FRVs redistribute insufficient output.

## Decisions and rationale

- **V1 bang-bang rejected**: curve oscillated 0.55→0.10→1.00 in one overnight cycle. 15-minute adjustments are meaningless against Leather's 15-hour time constant. The controller needed a model of the house.
- **SP=19 night mode**: zero rad leakage at curve 0.10, clean separation between "heating" and "not heating"
- **Exponent 1.27**: best fit from 70 pilot data points (Vaillant says 1.10 — underpredicts by 2.5–3.1°C)
- **Inner loop only, no EMA**: runtime learning (room_offset) ran away. Static calibration if needed
- **Thermal model drives initial guess**: inner loop converges regardless, but model makes it 1-tick convergence

## Current state

| Component | Status |
|---|---|
| V1 MVP (bang-bang) | Proved eBUS writes work. Oscillated badly. Retired |
| V2 Phase 1a (two-loop) | ✅ Deployed. Inner loop converges in 1 tick |
| V2 Phase 1b (live solver) | ✅ Deployed. `bisect_mwt_for_room` on ARM <1ms |
| V2 Phase 2 (overnight planner) | ✅ Deployed, awaiting more overnight data |
| V2 Phase 2b (T1-based DHW) | 🟡 T1 query added. Scheduling logic not yet implemented |
| Open-Meteo forecast | 🟡 Designed, not implemented |
| Door sensors | ⚪ Waiting on hardware |
| Away mode API | ✅ Endpoint exists |

## Next steps

### Immediate (this week)

1. **Fit leather door sensors** — 2× SONOFF SNZB-04P (in hand). Pair to Z2M, add to controller logging. No control changes — Stage 1 only (see door sensor plan above).
2. **Review tonight's overnight planner run** — first real test. Did it coast the right amount? Preheat start on time? Leather ≥20°C by 07:00?
3. **More overnight data** — reheat rate calibrated from 2 points. Tonight gives a third. Need 10+ nights across 0–15°C range.

### Needs evidence first (1–2 weeks of data collection)

4. **Outer/inner loop sawtooth** — observed 2 Apr but data is contaminated by conservatory door open all morning. The inner loop pushing curve up was *compensating* for the door (correct behaviour). Need a clean doors-closed day to confirm whether the sawtooth is a real problem or an artefact. Do not fix until confirmed on clean data.
5. **T1-based DHW decisions** — T1 now logged every cycle. Building evidence base: 2 Apr 12:14, VRC 700 triggered DHW at HwcStorageTemp=34°C while T1=43.9°C in Cosy window. Not necessarily wrong (bottom zone cold, afternoon demand, cheap rate). Need pattern across many charge events before changing logic. Blocked on: household shower experiment for minimum acceptable T1.
6. **Leather response with doors closed** — 2 Apr leather stuck at 19.7°C was fully explained by conservatory door open (~1,500W cold air load). Not a model or controller bug. Door sensors will detect this in future.

### Later (after evidence is in)

7. **Event-driven outer loop** — trigger on DHW→heating transition, door state change, Leather deviation >0.5°C for >15 min
8. **HP capacity clamp** — ignore `CurrentCompressorUtil` (reads negative values, unreliable). Use `RunDataElectricPowerConsumption` > 1500W for >30 min instead.
9. **Eco/normal mode detection** — plan DHW duration from detected mode (max flow temp ≥50°C = normal)
10. **Pre-DHW banking** — 15 min before predicted DHW charge, boost target_flow to pre-raise Leather ~0.3°C
11. **Direct flow temp control** — `SetModeOverride` to HMU, bypassing VRC 700 entirely
12. **Defrost analysis** — eBUS provides definitive defrost status (code 516) vs current inference from negative DT/heat

### Observations (2 Apr 2026 daytime — conservatory door open all morning)

- Leather stuck at 19.6–19.9°C for 6h. **Fully explained by conservatory door open** (~1,500W cold air load halves HP surplus). Model MWT≈28.3°C is correct for door-closed conditions.
- Outer/inner loop sawtooth: outer resets curve every 15 min (model guess 0.51–0.57), inner overrides to 0.59–0.68. With door open, the inner loop was *correctly compensating* for the extra heat loss. **Do not fix until confirmed on clean doors-closed data.**
- `CurrentCompressorUtil` reads negative values (-29, -55, -89, -102). Unreliable register — do not use for control decisions.
- DHW triggered by VRC 700 at HwcStorageTemp=34°C while T1=43.9°C in 13:00 Cosy window. Data input for DHW plan — not necessarily wrong (bottom zone cold, afternoon demand, cheap rate).

## Key files

| File | Purpose |
|---|---|
| `src/bin/adaptive-heating-mvp.rs` | Controller binary |
| `model/adaptive-heating-mvp.toml` | Config |
| `src/lib.rs` | Library crate exposing thermal solver |
| `deploy/adaptive-heating-mvp.service` | systemd unit for pi5data |
| `src/thermal/display.rs` | `bisect_mwt_for_room()`, `solve_equilibrium_temps()` |
| `data/canonical/thermal_geometry.json` | Room geometry for solver |

## eBUS quick reference

Writes to circuit `700`. TCP `localhost:8888` on pi5data.

| Register | R/W | Notes |
|---|---|---|
| `Hc1HeatCurve` | RW | 0.10–4.00. Primary control lever |
| `Z1OpMode` | RW | 0=off, 1=auto, 2=day, **3=night** |
| `Hc1ActualFlowTempDesired` | R | Inner loop feedback target |
| `DisplayedOutsideTemp` | R | Filtered outside temp |
| `RunDataStatuscode` | R (hmu) | HP state |
| `RunDataFlowTemp` / `ReturnTemp` | R (hmu) | Actual flow/return |
| `CurrentCompressorUtil` | R (hmu) | HP load % |

Derived: instantaneous COP = `CurrentYieldPower × 1000 / RunDataElectricPowerConsumption`.
| `HwcSFMode` | RW | auto / load (DHW boost trigger) |

## Revert to autonomous VRC 700

```bash
# Restore baseline (adaptive-heating-mvp does this on shutdown/kill):
echo 'write -c 700 Z1OpMode 1' | nc -w 2 localhost 8888    # auto
echo 'write -c 700 Hc1HeatCurve 0.55' | nc -w 2 localhost 8888
```

VRC 700 resumes timer-based day/night control with `Z1DayTemp`=21, `Z1NightTemp`=19, day mode from 04:00.
