# Heating Plan

Adaptive space heating control for 6 Rhodes Avenue. Vaillant Arotherm Plus 5kW, VRC 700 controller, 13 room sensors, calibrated thermal model.

## Scope and related docs

This document is the canonical reference for **space-heating strategy, control policy, measured constraints, and rationale**.

Use other docs for adjacent needs:
- **Historical evidence workflows / how-to:** `docs/history-evidence-workflows.md`
- **Code locations / module structure:** `docs/code-truth/README.md`, `docs/code-truth/REPOSITORY_MAP.md`, `docs/code-truth/ARCHITECTURE.md`
- **Secrets / deployment environment:** `deploy/SECRETS.md`
- **Agent-facing project memory / gotchas:** `AGENTS.md`
- **Broader documentation guide:** `docs/README.md`

This doc is **not** intended to be the full repository map or the only operational runbook.

## Objective

**Leather room 20-21°C during waking hours (07:00-23:00) at minimum electricity cost.**

Nobody cares what temperature the house reaches at 3am. The overnight temperature is constrained by HP reheat capacity, not a target.

## Constraints

| Constraint | Value | Source |
|---|---|---|
| HP thermal output | 5kW max | Arotherm Plus 5kW spec |
| House HTC | 261 W/K | Calibrated thermal model |
| HP deficit below | ~2°C outside (5kW < 5.9kW loss at -2°C) | Measured - accept 19.5-20°C |
| No heating needed above | 17°C outside | Empirical - solar/internal gains sufficient |
| Max useful flow temp | 45°C | Emitter capacity + COP limit |
| Leather time constant (τ) | **50h** (empirical) | From 53 cooling segments: 18 calibration-night + 35 DHW mini-experiments. Was modelled as 15h - wrong by 3.3× (see § Empirical vs model parameters) |
| Cooling rate (k) | 0.020/hr per °C ΔT | k = 1/τ = 1/50. Was 0.039 (from τ=15h model) |
| Leather thermal capacity | ~13,000 Wh/°C | Derived from k and HTC. Higher than expected because Leather's response includes coupling to adjacent warm rooms |
| DHW steals HP for | 50-100 min per charge | eco ~100 min, normal ~60 min |
| Emitters | 15 radiators (no TRVs), Sterling off | No per-room flow control |
| Sensors | 13 rooms (12× SNZB-02P + 1 emonth2) | ~5 min update rate |

### HP capacity vs outside temperature

| Outside | Heat loss | HP surplus | Overnight drop (8h no heat) | Min floor (3h reheat to 20.5°C) |
|---|---|---|---|---|
| -2°C | 5872W | **deficit** | Not recoverable | Must heat continuously |
| 0°C | 5350W | **deficit** | Not recoverable | Must heat continuously |
| 2°C | 4828W | 172W | 2.4°C | 20.4°C |
| 5°C | 4046W | 954W | 2.0°C | 19.7°C |
| 8°C | 3262W | 1738W | 1.6°C | 19.0°C |
| 10°C | 2740W | 2260W | 1.3°C | 18.6°C |
| 14°C | 1696W | 3304W | 0.7°C | 17.7°C |

Below ~2°C the HP runs flat out and can barely maintain 20°C. Scheduling is irrelevant - the HP never stops.

## Tariff

Octopus Cosy, three windows:

| Rate | Price | Times |
|---|---|---|
| **Cosy** | 13.24p/kWh | 04:00-07:00, 13:00-16:00, 22:00-00:00 |
| **Mid-peak** | 26.98p/kWh | 00:00-04:00, 07:00-13:00, 19:00-22:00 |
| **Peak** | 40.48p/kWh | 16:00-19:00 |
| **Effective (all-in)** | 16.7p/kWh | Total bill ÷ total kWh (last 12 months, inc standing + VAT) |
| **Marginal (battery-blended)** | 13.9p/kWh | 95% battery coverage × Cosy + 5% grid mid-peak |

Rates are Q2 2026 South East inc VAT. All-in effective rate from `~/github/octopus` half-hourly data (6,908 kWh, ~£1,151, 12 months). The 16.7p includes standing charge (52.76p/day = 2.8p/kWh) - for scheduling decisions use the **marginal battery-blended rate** (13.9p) not the all-in rate. 95% of import falls in off-peak. Battery captures most tariff arbitrage: marginal Cosy-vs-non-Cosy difference is only 0.7p/kWh (mid) to 1.4p/kWh (peak). Total scheduling optimisation yields ~£5/year. The real value of Cosy alignment is **protecting the battery for peak hours on cold days** when the HP runs flat out.

## Control surface

### VRC 700 heat curve formula

```
flow_temp = setpoint + curve × (setpoint - outside)^1.25
```

Exponent 1.25 is the current best-fit working value from pilot data expansion. Vaillant says 1.10 - underpredicts by 2.5-3.1°C at curves ≥0.50.

Inverse: `curve = (target_flow - setpoint) / (setpoint - outside)^1.25`

### Primary levers

| Register | Role | Notes |
|---|---|---|
| `Hc1HeatCurve` | Flow temp gradient (0.10-4.00) | Primary control. 0.01 step ≈ 0.20°C flow change |
| `Z1OpMode` | Operating mode | Set to 3 (night) on startup → permanent SP=19 |

### Why SP=19 (permanent night mode)

Three setpoints analysed. SP=19 chosen because:
- Curve 0.10 = genuinely zero rad output (no formula leakage)
- Any overnight heating is a deliberate curve raise, not accidental
- Curves stay under 1.50 warning up to 15°C outside
- No heating runs above 17°C anyway

**Why not Z1OpMode=auto?** The VRC 700 has undocumented Optimum Start: at 03:00 (3h before 06:00 day timer), `Hc1ActualFlowTempDesired` jumped from 21.0°C to 22.3°C with curve at 0.10. No register to disable it. Night mode eliminates Optimum Start, CcTimer transitions, and day/night setpoint switches - giving the controller full authority.

On shutdown/kill: **baseline restore** writes `Z1OpMode=auto`, `Hc1HeatCurve=0.55`. VRC 700 resumes autonomous timer control. Crash without restore: house sits at 19°C with last curve - safe.

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
        curve += gain × error      (max step 0.20, clamp 0.10-4.00)
        write Hc1HeatCurve
```

**Inner loop tuning**: gain=0.05, deadband=0.5°C, max_step=0.20, curve clamped to 0.10-4.00 (trust the VRC 700's accepted range - no extra software limits). Below curve 0.25: gain halved to 0.025, deadband doubled to 1.0°C. Converges in 1-2 ticks.

**ΔT stabilisation**: uses live flow-return ΔT only when `RunDataStatuscode` contains "Heating" + "Compressor". Otherwise `default_delta_t_c` = 4.0°C.

**No runtime learning**: `room_offset` EMA was tried and removed - it ran away to +2.18°C overnight, learning the cooling trend as "model error" and suppressing preheat by ~8°C (target_flow 23.5°C when 31.2°C was needed). If systematic model bias appears, apply a static calibration offset.

### Comfort guard and COP optimisation

The controller has layered priorities:

1. **Comfort guard** (hard constraints): any heated room < 18°C → raise curve. `CurrentCompressorUtil` > 95% for >30 min → hold (HP at capacity). DHW active → don't adjust
2. **COP optimisation**: gradient-follow - step toward better COP, stop when rooms cool or COP plateaus
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
- **InfluxDB** (`adaptive_heating_mvp` measurement): `target_flow_c`, `curve_after`, `flow_desired_c`, room temps, outside temp, action, mode, tariff, and model outputs
- **Local JSONL** on pi5data: full decision context for debugging and controller-intent audit trail

### Live heating checks

Use these when the question is **what is happening now**, not whether the recent plan worked.

```bash
cargo run --bin adaptive-heating-mvp -- --config model/adaptive-heating-mvp.toml status
cargo run --bin adaptive-heating-mvp -- --config model/adaptive-heating-mvp.toml status --human
echo 'read -c 700 Hc1HeatCurve' | nc -w 2 localhost 8888
echo 'read -c 700 Hc1ActualFlowTempDesired' | nc -w 2 localhost 8888
```

Use the structured default `status` output for LLM/tool consumption.
Use `--human` only for operator readability.
A live snapshot does **not** prove whether the overnight planner worked; use the historical commands below for that.

### Historical evidence commands

```bash
date -u
export INFLUX_TOKEN=$(ak get influxdb)
cargo run --bin heatpump-analysis -- heating-history          # JSON (default)
cargo run --bin heatpump-analysis -- heating-history --human   # readable
```

Success criteria: Leather ≥20°C at 07:00 on clean mornings, waking-hours comfort in clean windows. Each control change is an experiment. See `docs/history-evidence-workflows.md` for review workflow, confounder handling, and evidence interpretation.

**Confounders**: park DHW-active, door-open, and missing-data periods from primary scoring. Reuse DHW windows for cooldown/building-response analysis. DHW timing interactions scored under `docs/dhw-plan.md`.

### Room priorities

- **Leather** (primary): emonth2, **50h time constant** (empirical). Optimise for this when doors closed
- **Aldora** (secondary): second comfort anchor
- **Conservatory**: excluded - 30m2 glass, sub-hour time constant, follows outdoor + solar
- **Other rooms**: constraints and context, not targets

**Leather door sensors** (2× SONOFF SNZB-04P, in hand, not fitted): `leather_conservatory_door`, `leather_hall_door`.

| Stage | Action | Duration |
|---|---|---|
| 1. Log | Pair to Z2M, add to controller decision log. No control changes. | Now |
| 2. Analyse | Correlate door state with Leather trajectory at various outside temps. | 1-2 weeks |
| 3. Integrate | Conservatory open: hold curve (don't chase). Closed: immediate recalc. Both open: target Aldora. | After data |

## Overnight strategy

The controller calculates the latest heating start time that achieves Leather ≥ 20°C by 07:00.

### Algorithm

1. Simulate cooling: exponential decay with **τ=50h** (empirical) toward equilibrium (outside + 2.5°C internal gains)
2. At each 30-min step backward from 07:00: can HP reheat from here to 20.5°C in time?
3. Reheat rate: K=7500 W per °C/h (surplus / K = °C/h rise). **⚠ Empirical K≈20,600 from 27 segments - current value likely overpredicts reheat speed. Each overnight run validates this.**
4. Find the **latest** safe preheat start (maximum coast time) with 30-min safety margin
5. Below 2°C outside: maintain 19.5°C continuously (HP can't recover from any drop)

### Heating recovery by outside temperature

From emoncms data (heating state, indoor_t rising):

| Outside | Heat output | Electricity | COP | MWT |
|---|---|---|---|---|
| -2-0°C | 5700W | 1849W | 3.08 | 30.5°C |
| 2-4°C | 5180W | 1420W | 3.65 | 31.3°C |
| 6-8°C | 4045W | 841W | 4.81 | 30.2°C |
| 10-12°C | 2913W | 481W | 6.06 | 28.3°C |

Reheat rate: see § Empirical vs model parameters.

### Controller actions

From 7 days of observed data (28 Mar - 4 Apr 2026, 10,080 1-minute samples):

| Action | Occurrences | Curve | When |
|---|---|---|---|
| `hold` | 197 | Unchanged | DHW active, or stabilising between outer cycles. Most common action |
| `daytime_model` | 117 | Model-derived | Waking hours: thermal solver → target flow → curve |
| `overnight_preheat` | 35 | Model curve | Overnight: calculated preheat start reached |
| `heating_coast` | 21 | ~0.45 | Pre-adaptive period: coasting on residual heat |
| `preheat_model` | 12 | Model curve | Morning: live solver driving preheat toward 20.5°C |
| `heating_recovery` | 9 | Ramping | Pre-adaptive period: recovering from setback |
| `dhw_boost` | 2 | Held | Controller triggered DHW boost (`HwcSFMode=load`) |
| `overnight_coast` | 1 | 0.10 (zero output) | House warm enough, free cooling |
| `overnight_maintain` | 0 (not yet triggered) | Continuous at 19.5°C | Below 2°C outside |

### Known limitations

- K=7500 reheat rate likely wrong (empirical K≈20,600) - each coast night validates
- Solar gain not included in reheat estimate (conservative - will overshoot on sunny mornings)
- Uses average overnight outside temp (should use hourly forecast from Open-Meteo)

### Empirical vs model parameters

From 35 DHW-cooling and 27 post-DHW reheat segments (15-min resolution, 16 days of ebusd data):

| Parameter | Model | Empirical | Ratio | Effect on planner |
|---|---|---|---|---|
| τ (cooling time constant) | 15.0h | ~50h (median, from both sources) | 3.3× slower | Model overpredicts overnight cooling |
| K (reheat: surplus W per °C/h) | 7,500 | ~20,600 (median) | 2.7× slower | Model overpredicts reheat speed |

**Net effect**: the model thinks the house cools fast AND reheats fast, so it panics and starts heating immediately. The real house barely cools overnight. Result: zero coasting, wasting 3-5h of heating per mild night.

**Two independent data sources agree on τ ≈ 50h:**

| Source | τ mean | τ median | n |
|---|---|---|---|
| Calibration nights (no heating, 4 nights, 24-28 Mar) | 48h | 51h | 18 segments |
| DHW mini-experiments (90 days of charges) | 44h | 50h | 35 segments |
| Model assumption | 15h | - | - |

Best single overnight observation: Night 2, 23:07→03:02 (3.9h continuous, no heating), Leather dropped 20.82→20.12°C = 0.18°C/h, giving τ = **65.8h**.

The model τ=15h may correspond to the whole-house response (house τ=25.8h from thermal model), not Leather specifically. But Leather is the controlled variable for overnight planning.

Every DHW charge is a mini cooling experiment. Every heating restart is a mini reheat experiment. These accumulate over months and provide far more empirical data than the 2-point calibration the planner was originally built on.

**Recommended approach**: update τ and K toward empirical values. Each overnight is now an experiment: record predicted vs actual cooling/reheat, compare to Met Office forecast, and refine. The `break` fix (commit e11cbd6) was the immediate blocker - the planner now actually coasts, producing real overnight data to validate against.

## Away mode

1. **Trigger**: API endpoint `/api/heating/away` or config
2. **Setpoint**: 15°C, curve 0.30 - frost protection only. Costs ~£0.50/day vs ~£2.50 at 21°C
3. **Warm-up ramp**: thermal model computes lead time from current temp + forecast. At 7°C outside: 15→21°C takes ~20h at full power. Ramp in two stages: 15→18°C (curve 0.45), then 18→21°C (curve 0.55)
4. **Forecast adjustment**: cold snap → start earlier. Mild → start later
5. **A week away saves ~£14**

## HP contention with DHW

Each DHW charge blocks heating for 50-100 minutes. Known issue: on 1-2 Apr, DHW stole 1.5h of preheat (cylinder drifted to 39.5°C, barely below 40°C trigger). Leather dropped from 20.1→19.9°C, below comfort by 07:15. Phase 2 T1-based scheduling will fix this by sequencing DHW and preheat explicitly.

Impact depends on outside temperature:

| Outside | HP surplus for heating | DHW cost in comfort |
|---|---|---|
| -2°C | Deficit | ~0.5°C Leather drop per charge (unrecoverable) |
| 5°C | 954W | ~0.3°C drop, recovers in ~1h |
| 10°C | 2260W | ~0.2°C drop, recovers in ~30 min |
| 15°C | 3826W | Negligible |

On cold days (<5°C), every DHW charge matters. Scheduling DHW in the 22:00-00:00 Cosy window frees the 04:00-07:00 window for uninterrupted preheat. On mild days, it doesn't matter. See [DHW plan](dhw-plan.md).

## Physical improvements

| Priority | Action | Cost | Impact |
|---|---|---|---|
| 1 | Close Elvina trickle vents | FREE | Removes system bottleneck - MWT 49→47°C at -3°C. Elvina reads 17-18.8°C consistently (3-4 Apr), 2-3°C below all other bedrooms |
| 2 | Aldora rad upgrade (reuse existing 909W DP DF) | FREE | MWT 47→45°C |
| 3 | Jack&Carol bay window draught-strip | ~£30 | 60-150W saving |
| 4 | EWI on SE wall (~30m2) | ~£5k DIY | 19% heat demand reduction. MWT 49→43°C at -3°C |
| 5 | Sterling floor insulation | ~£200 | Leather keeps heat, Sterling gets cold room |

FRVs deprioritised - HP at capacity on cold days, FRVs redistribute insufficient output.

## Decisions and rationale

- **V1 bang-bang rejected**: curve oscillated 0.55→0.10→1.00 in one overnight cycle. 15-minute adjustments are meaningless against Leather's ~50-hour time constant. The controller needed a model of the house.
- **SP=19 night mode**: zero rad leakage at curve 0.10, clean separation between "heating" and "not heating"
- **Exponent 1.25**: current best-fit working value for initial curve guess (Vaillant says 1.10 - underpredicts by 2.5-3.1°C)
- **Inner loop only, no EMA**: runtime learning (room_offset) ran away. Static calibration if needed
- **Thermal model drives initial guess**: inner loop converges regardless, but model makes it 1-tick convergence

## Current state

| Component | Status |
|---|---|
| V1 MVP (bang-bang) | Proved eBUS writes work. Oscillated badly. Retired |
| V2 Phase 1a (two-loop) | ✅ Deployed. Inner loop converges in 1 tick |
| V2 Phase 1b (live solver) | ✅ Deployed. `bisect_mwt_for_room` on ARM <1ms. Clean afternoon/evening windows show maintained comfort under model-driven control |
| V2 Phase 2 (overnight planner) | 🟡 Deployed. Planner bug (missing `break`) and τ error (15→50h) fixed 4 Apr. First real coast night pending. Pre-fix "validation" was continuous heating, not true coast-then-preheat |
| V2 Phase 2b (T1-based DHW) | 🟡 T1 query added. Scheduling logic not yet implemented |
| Open-Meteo forecast | 🟡 Designed, not implemented |
| Door sensors | ⚪ Waiting on hardware |
| Away mode API | ✅ Endpoint exists |

## Evidence-based status of current heating changes

Current extracted evidence supports different conclusions for different parts of the controller:

- **Daytime model-driven control:** supported by clean-window evidence. In clean afternoon/evening windows on **2026-04-01 13:29:30-21:05:00** and **2026-04-02 13:17:30-21:00:00**, the controller had **no detected comfort-miss period**. In the 2026-04-02 clean window, Leather rose from **20.2°C to 21.5°C**. On **2026-04-04**, Leather held 20.7-22.0°C all day with smooth curve adjustments (0.60-1.28 range depending on outside temp).
- **Overnight planner:** 🟡 **Pre-fix runs were continuous heating (not real coasting)** due to two stacked bugs: (1) missing `break` in binary search meant coast=0 always won, (2) τ=15h overpredicted cooling by 3.3×. Fixed 4 Apr. Both nights (2-4 Apr) had Leather ≥20.7°C because the HP heated all night - this validated the inner loop and curve tracking, but not the coasting strategy. **First real coast-then-preheat night pending.**
- **Sawtooth flag:** reclassified as **not a real control problem**. The 54 alternations flagged over 7 days are `daytime_model` ↔ `hold` transitions during DHW charges - the controller correctly holds during charging then resumes. Overnight traces show smooth operation with curve held steady for hours. The earlier 2 Apr sawtooth was inner-loop compensation for conservatory door open (correct behaviour).
- **DHW-active windows:** excluded from primary scoring of heating-control effectiveness, but retained as useful cooldown/building-response evidence.
- **Door-open windows:** handled under the existing door plan and excluded from baseline heating-effectiveness scoring.

The first observed `preheat_model` morning (**2026-04-02 03:06:07Z**) had **82.5 minutes of DHW overlap** and still entered a comfort-miss period - this was the DHW-confounded baseline. Subsequent nights had Leather ≥20.7°C, but this was because the planner heated continuously (not coasting). The inner loop and daytime model are validated; the overnight coasting strategy is not yet tested with corrected parameters.

## Next steps

### Immediate (this week)

1. **Deploy corrected planner and validate first coast nights** - the τ=50h and `break` fix must run for real. Each night is an experiment: record predicted coast duration and preheat start, compare predicted Leather trajectory against actual, note outside temp. The planner should now coast 2-6h on mild nights (8-14°C) vs zero coasting before. Success = Leather ≥20°C at 07:00 after genuine coasting.
2. **Validate K (reheat rate)** - K=7500 may overpredicts reheat speed (empirical K≈20,600 from 27 post-DHW segments). If the planner starts preheat at the calculated time but Leather doesn't reach 20.5°C by 07:00, K needs increasing. Every DHW charge also provides a mini-reheat experiment when heating resumes - keep accumulating these.
3. **Fit leather door sensors** - 2× SONOFF SNZB-04P (in hand). Pair to Z2M, add to controller logging. No control changes - Stage 1 only (see door sensor plan above).
4. **Continue rolling 7-day reviews** - use `heating-history` as default evidence sweep. Compare predicted vs actual for each overnight run.

### Needs evidence first (1-2 weeks of coast data)

5. **Converge τ and K from overnight runs** - need 10+ coast-then-preheat nights across 0-15°C range. Compare predicted Leather dip and reheat time against actual. If the planner is consistently too conservative (Leather well above 20°C at 07:00), coast longer. If comfort misses appear, tighten safety margin or adjust K.
6. **Morning DHW/heating coordination rule** - with the corrected planner, preheat now starts later (02:30-05:00 depending on outside temp). This changes the DHW contention picture - morning DHW charges are less likely to overlap preheat on mild nights.
7. **Leather response with doors closed** - 2 Apr leather stuck at 19.7°C was fully explained by conservatory door open (~1,500W cold air load). Door sensors will detect this in future.

### Later (after evidence is in)

7. **Event-driven outer loop** - trigger on DHW→heating transition, door state change, Leather deviation >0.5°C for >15 min
8. **HP capacity clamp** - ignore `CurrentCompressorUtil` (reads negative values, unreliable). Use `RunDataElectricPowerConsumption` > 1500W for >30 min instead.
9. **Eco/normal mode detection** - plan DHW duration from detected mode (max flow temp ≥50°C = normal)
10. **Pre-DHW banking** - 15 min before predicted DHW charge, boost target_flow to pre-raise Leather ~0.3°C
11. **Direct flow temp control** - `SetModeOverride` to HMU, bypassing VRC 700 entirely
12. **Defrost analysis** - eBUS provides definitive defrost status (code 516) vs current inference from negative DT/heat

### Resolved observations

- **2 Apr door-open**: Leather stuck at 19.6–19.9°C for 6h — conservatory door open (~1,500W cold air load). Inner loop correctly compensated. Not a control bug.
- **Sawtooth flag**: `daytime_model` ↔ `hold` alternations during DHW charges. Not real oscillation. Resolved.
- **Service hang** (2 Apr ~12:46): reqwest had no timeout. Fixed: 10s timeout on all HTTP.
- **`CurrentCompressorUtil`**: reads negative values. Unreliable — do not use.
- **3–4 Apr "overnight successes"**: pre-fix, planner heated continuously (coast=0 due to bugs). Validates inner loop + curve tracking, not coasting. With τ=50h fix, these nights should coast ~6h.

## Key files

| File | Purpose |
|---|---|
| `src/bin/adaptive-heating-mvp.rs` | Controller binary |
| `model/adaptive-heating-mvp.toml` | Config |
| `src/lib.rs` | Library crate exposing thermal solver |
| `deploy/adaptive-heating-mvp.service` | systemd unit for pi5data |
| `deploy/SECRETS.md` | Secrets management: InfluxDB token setup, dev vs prod |
| `src/thermal/display.rs` | `bisect_mwt_for_room()`, `solve_equilibrium_temps()` |
| `data/canonical/thermal_geometry.json` | Room geometry for solver |

## eBUS quick reference

Writes to circuit `700`. TCP `localhost:8888` on pi5data.

| Register | R/W | Notes |
|---|---|---|
| `Hc1HeatCurve` | RW | 0.10-4.00. Primary control lever |
| `Z1OpMode` | RW | 0=off, 1=auto, 2=day, **3=night** |
| `Hc1ActualFlowTempDesired` | R | Inner loop feedback target |
| `DisplayedOutsideTemp` | R | Filtered outside temp |
| `RunDataStatuscode` | R (hmu) | HP state |
| `RunDataFlowTemp` / `ReturnTemp` | R (hmu) | Actual flow/return |
| `CurrentCompressorUtil` | R (hmu) | HP load % |

Derived: instantaneous COP = `CurrentYieldPower × 1000 / RunDataElectricPowerConsumption`.
| `FlowPressure` | R (hmu) | System water pressure (bar). See below |
| `HwcSFMode` | RW | auto / load (DHW boost trigger) |

System pressure: `FlowPressure` (HMU) reads 2.01 bar heating, 1.90 bar DHW, 2.05 bar idle. Rock steady. See AGENTS.md for full analysis.

## Deployment (pi5data)

The controller runs as a systemd service on pi5data (10.0.1.230).

| Component | Location |
|---|---|
| Binary | `/home/jack/adaptive-heating-mvp/target/release/adaptive-heating-mvp` |
| Config | `/home/jack/adaptive-heating-mvp/model/adaptive-heating-mvp.toml` |
| Thermal geometry | `/home/jack/adaptive-heating-mvp/data/canonical/thermal_geometry.json` |
| Systemd unit | `/etc/systemd/system/adaptive-heating-mvp.service` |
| Secrets (InfluxDB token) | `/etc/adaptive-heating-mvp.env` (root:root 0600) |
| Runtime state | `/home/jack/.local/state/adaptive-heating-mvp/state.toml` |
| Decision log (JSONL) | `/home/jack/.local/state/adaptive-heating-mvp/actions.jsonl` |

The InfluxDB token is the same one Telegraf uses (both read from the local InfluxDB Docker container). See `deploy/SECRETS.md` for setup on fresh install.

Build on pi5data: `source ~/.cargo/env && cd ~/adaptive-heating-mvp && cargo build --release`

Deploy source from dev machine: `scp src/bin/adaptive-heating-mvp.rs pi5data:~/adaptive-heating-mvp/src/main.rs`

## Revert to autonomous VRC 700

```bash
# Restore baseline (adaptive-heating-mvp does this on shutdown/kill):
echo 'write -c 700 Z1OpMode 1' | nc -w 2 localhost 8888    # auto
echo 'write -c 700 Hc1HeatCurve 0.55' | nc -w 2 localhost 8888
```

VRC 700 resumes timer-based day/night control with `Z1DayTemp`=21, `Z1NightTemp`=19, day mode from 04:00.
