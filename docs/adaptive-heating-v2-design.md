# Adaptive Heating V2 - Model-Predictive Control

Last updated: 2 April 2026

## Objective

Leather room 20-21°C during waking hours (07:00-23:00) at minimum electricity cost, with reliable DHW.

Constraints:
- HP maxes out below ~2°C outside (5kW vs 5.9kW heat loss at -2°C). Accept 19.5-20°C.
- No heating above 17°C outside - empirically, solar/internal gains are sufficient.
- 45°C max flow on heating - emitter capacity and COP limit. Above this, diminishing returns.
- DHW steals HP for ~1-2h. A bath draw can empty the cylinder and require a full recharge.
- Battery is fully charged overnight - tariff window alignment is irrelevant for heating cost. DHW should still target Cosy windows.
- Overnight temp is **not** a free variable - constrained by HP reheat capacity. At -2°C the HP is in deficit and can barely drop 0.5°C. At 10°C, minimum floor is ~18.6°C for 3h reheat to 20.5°C by 07:00. See Phase 2.

## Architecture

### VRC 700 control

On startup: `Z1OpMode=night` (value 3). VRC 700 uses `Z1NightTemp` (19°C) permanently. Disables CcTimer, Optimum Start, day/night transitions. Setpoint values are never modified.

**Why SP=19 (night mode) not SP=21 (day mode)?** Three setpoints were analysed:
- SP=21: curves 0.33-0.74 across -5 to 17°C (most headroom), but rads leak 56-155W overnight - unwanted heating when the overnight planner wants a clean "off".
- SP=20: curves 0.37-1.32, zero overnight leakage, but no clear advantage over 19.
- SP=19: curves 0.40-1.24 up to 15°C (under 1.50 warning), **zero overnight rad output** at curve=0.10, cleanest separation between "heating" and "not heating". Curves go to 1.67/2.61 at 16-17°C but no heating runs above 17°C anyway. Inner loop converges regardless of initial guess quality.

SP=19 chosen because overnight planner (Phase 2) needs a setpoint where curve=0.10 means genuinely zero output, and any overnight heating is deliberate (raised curve), not formula leakage. At cold temps (<5°C) the HP must run overnight anyway - the planner will raise the curve, not rely on the setpoint.

On shutdown: `Z1OpMode=auto`, `Hc1HeatCurve=0.55`. VRC 700 resumes timer control.

Crash without restore: house at 19°C with last curve. Safe.

### Two-loop control

```
Outer loop (every 15 min):
    thermal model: (forecast outside, solar) → required MWT → target flow temp
    initial curve guess via formula: curve = (flow - 19) / (19 - outside)^1.25

Inner loop (every ~1 min):
    error = target_flow - Hc1ActualFlowTempDesired
    if |error| > 0.5°C:
        curve += 0.05 × error      (max step 0.20, clamp 0.10–4.00)
        write Hc1HeatCurve
```

The outer loop uses the calibrated thermal physics model via control table (Phase 1) or live solver (Phase 1b). The inner loop treats the VRC 700 as a black box - nudge curve until output matches target. The formula is only the initial guess; the inner loop converges in 1 tick (~60s).

**Why not open-loop formula?** The VRC 700's internal computation is opaque - hidden Optimum Start, `Hc1MinFlowTempDesired`=20°C floor, undocumented offsets. See "Pilot data findings" below.

### Error correction

**Inner loop** is the only feedback mechanism. It closes directly on `Hc1ActualFlowTempDesired`. No EMA, no offset accumulation.

**`room_offset` was removed** (2 Apr 2026). The EMA ran away to +2.18°C overnight - it learned the overnight cooling as model error, then suppressed preheat target_flow by ~8°C (23.5 vs 31.2°C needed). The inner loop is sufficient: it converges in 1 tick regardless of model accuracy. If Phase 1b reveals systematic model bias, a static calibration offset would be better than a runtime EMA.

### DHW

Cosy windows (04:00-07:00, 13:00-16:00, 22:00-23:59) + cylinder < 40°C → `HwcSFMode=load`.

**Caution**: `HwcStorageTemp` reads the NTC in a dry pocket above the bottom coil. After a large draw (e.g. bath), it reads mains cold (~13°C) even with 60-70L of usable hot water above the thermocline. Do not trigger emergency DHW charges based solely on a low NTC reading - the stratification holds and the scheduled Cosy window charge is usually sufficient.

### Safety

- Baseline restore on shutdown: `Z1OpMode=auto`, `Hc1HeatCurve=0.55`
- Solver fails → hold last target_flow, inner loop maintains it
- eBUS fails → hold, don't write
- Inner loop: deadband 0.5°C, max step 0.20, curve 0.10-4.00
- Warn if curve > 1.50

## Implementation plan

### Phase 1a: Inner feedback loop ✅ DONE (1 Apr 2026)

Replaced V1 bang-bang (±0.10 every 15 min, oscillating 0.10↔1.00) with two-loop architecture.

**Changes made:**
- `model.setpoint_c` = **19.0**, `Z1OpMode=night` on startup
- Added `inner_loop_gain=0.05`, `inner_loop_deadband_c=0.5`, `inner_loop_max_step=0.20`
- Outer loop (900s): forecast → model → `target_flow_c` + initial curve guess
- Inner loop (60s): proportional feedback on `Hc1ActualFlowTempDesired` toward `target_flow_c`
- Removed `flow_offset` EMA (inner loop replaces it)
- Removed `room_offset` EMA (ran away overnight, inner loop is sufficient)
- `restore_baseline()` writes only `Hc1HeatCurve=0.55` + `Z1OpMode=auto`
- `preheat_hours` = 2.0 (05:00 start). Battery handles cost; no need to align with Cosy window.

**Validation results** (1-2 Apr 2026):
- Inner loop converges in **1 tick** - flow_desired within 0.3°C of target after single adjustment
- Daytime (13-14°C outside): leather stable at 21.4-21.6°C, COP ~7.1, curve 0.76-1.06
- Shutdown restores correctly: 2 writes only (Hc1HeatCurve, Z1OpMode)

**Known issues to address in Phase 1b:**

1. ~~**Inner loop hunts near curve floor.**~~ **FIXED** (2 Apr): reduced `inner_loop_gain` from 0.10 to 0.05. Hunting was caused by gain too high at low curves — each 0.01 curve ≈ 0.18°C flow, so gain=0.10 with 0.9°C error made 1.6°C overshoots. At gain=0.05 the loop converges in 2 ticks. The 20°C floor (`Hc1MinFlowTempDesired`) was not the cause — it doesn’t bind at SP=19 with outside < 17°C.

2. **Outer/inner ΔT fight.** When compressor shuts down, live ΔT collapses (flow≈return), `target_flow = MWT + ΔT/2` drops, outer loop writes a lower curve, then inner loop adjusts. Wasted eBUS writes, not harmful. Fix: use `default_delta_t_c` when compressor is off.

3. **DHW steals preheat.** Night of 1-2 Apr: cylinder drifted to 39.5°C (barely below 40°C trigger), DHW charged for 1.5h during preheat, leather dropped from 20.1→19.9°C. By 07:15 leather was 19.9°C - below comfort band. **Not fixing in Phase 1b** — 40°C trigger is already marginal for morning hot water, and the Phase 2 overnight planner will schedule DHW and preheat sequentially.

### Phase 1b: Bug fixes + live solver 🔴 NEXT

Fix the two known issues from Phase 1a, then replace the control table with the live solver.

**Bug fixes:**

1. **Inner loop floor guard**: when `curve_before < 0.25`, halve the gain (0.05 instead of 0.10) and double the deadband (1.0°C instead of 0.5°C). Prevents hunting near MinFlowTempDesired floor.

2. **ΔT stabilisation**: in `calculate_required_curve()`, if `RunDataStatuscode` is not `Heating_Compressor_active`, use `default_delta_t_c` instead of live ΔT. Prevents outer loop target_flow oscillation on compressor cycling.

**Live solver:**

1. Create `src/lib.rs`, move `pub mod thermal` there. Widen visibility on: `geometry::*`, `physics::*`, `display::{solve_equilibrium_temps, bisect_mwt_for_room}`, `config::*`, `error::*`.

2. Replace `ControlTable` with `bisect_mwt_for_room("leather", target_leather, outside, solar, wind)`. Remove `control_table_path` config.

3. Deploy `data/canonical/thermal_geometry.json` + `model/thermal-config.toml` to pi5data. Benchmark solver on ARM (<1s).

4. Add event-driven outer loop: trigger on DHW→heating transition, leather deviation >0.5°C for >15 min.

5. Add HP capacity clamp: if `CurrentCompressorUtil` > 95% for >30 min, stop raising curve.

### Phase 2: Overnight planner (requires 1b)

Replace fixed `overnight_curve=0.10` and `preheat_hours=2.0` with temperature-dependent overnight strategy.

**Key constraint**: reheat capacity. HP surplus = 5000W - 261×(20.5-outside). Analysis:

| Outside | Heat loss | HP surplus | 8h no-heat drop | Min floor (3h reheat) | Max free cooling |
|---------|-----------|------------|------------------|-----------------------|------------------|
| -2°C | 5872W | **-872W** (deficit) | 21→11.5°C | 20.5°C (can't cool) | 0.5°C |
| 0°C | 5350W | **-350W** (deficit) | 21→12.3°C | 20.5°C (can't cool) | 0.5°C |
| 2°C | 4828W | 172W | 21→13.1°C | 20.4°C | 0.6°C |
| 5°C | 4046W | 954W | 21→14.4°C | 19.7°C | 1.3°C |
| 8°C | 3262W | 1738W | 21→15.6°C | 19.0°C | 2.0°C |
| 10°C | 2740W | 2260W | 21→16.5°C | 18.6°C | 2.4°C |
| 12°C | 2218W | 2782W | 21→17.3°C | 18.1°C | 2.9°C |
| 14°C | 1696W | 3304W | 21→18.1°C | 17.7°C | 3.3°C |

**First overnight data** (1-2 Apr 2026, outside 10-12°C): leather dropped 20.6→20.2°C in 5h (23:00-04:00) with zero heating. Very mild. Then DHW stole 1.5h and leather hit 19.9°C by 07:15. The house barely cools at these temps - the problem was DHW timing, not insufficient preheat.

**Implication**: below ~2°C outside, the HP must run nearly continuously overnight - the overnight curve must be *raised*, not held at 0.10. SP=19 with curve=0.10 gives zero rad output, so any overnight heating is a deliberate curve raise by the planner.

Forward simulation: `overnight_plan(current_leather, forecast[], dhw_expected, target, target_time)`:
1. Simulate cooling with thermal mass (τ=15h for leather) + hourly forecast
2. At each hour, check: can the HP reheat from here to 20.5°C by 07:00?
3. Binary search on latest start time for Leather ≥ 20°C by 07:00
4. If outside < 2°C: maintain minimum curve to prevent unrecoverable drop
5. Schedule DHW in the optimal Cosy window (see DHW duration model below)
6. Battery handles cost, so preheat timing is purely thermal, not tariff-aligned

Outputs: overnight curve profile (may vary hourly) + preheat start time + DHW window.

#### DHW duration model (prerequisite)

From 402 AM DHW charges (Oct 2024 - Mar 2026 emoncms data), two populations:

| Mode | n | Avg duration | Hit 120-min timeout | Avg max flow |
|------|---|-------------|--------------------|--------------|
| **Eco** (flow <50°C) | 280 | 102 min | 111 (40%) | 47.8°C |
| **Normal** (flow ≥50°C) | 122 | 60 min | 2 (2%) | 52.6°C |

Eco mode by outside temp:

| Outside | n | Avg min | Hit timeout | Notes |
|---------|---|---------|-------------|-------|
| <2°C | 19 | 118 min | 95% | **Nearly all incomplete** |
| 2-5°C | 38 | 119 min | 89% | Mostly incomplete |
| 5-8°C | 55 | 111 min | 53% | Borderline |
| 8-12°C | 88 | 101 min | 23% | Usually completes |
| 12°C+ | 80 | 86 min | 13% | Fine |

Normal mode by outside temp:

| Outside | n | Avg min | Hit timeout | Notes |
|---------|---|---------|-------------|-------|
| <2°C | 8 | 81 min | 1 | Works but slow |
| 2-5°C | 24 | 59 min | 1 | Good |
| 5-8°C | 34 | 48 min | 0 | Fast |
| 8-12°C | 31 | 57 min | 0 | Good |
| 12°C+ | 25 | 73 min | 0 | Fine |

**Eco mode is cheaper** (lower flow = better COP) but below ~8°C it takes so long that the heating steal becomes costly: 261 W/K × ΔT × extra_minutes of lost heating. At some crossover temperature, the total system cost (DHW energy + heating recovery) favours normal mode.

**DHW scheduling decision** for the overnight planner:
- Mild nights (>8°C): eco mode, charge in 04:00-07:00 Cosy window before preheat (~90 min budget)
- Cold nights (2-8°C): eco mode if DHW can fit in 22:00-00:00 window the night before, freeing the morning for preheat. Or normal mode in the morning (~60 min).
- Coldest nights (<2°C): normal mode mandatory (eco hits timeout and is incomplete). Schedule in 22:00-00:00 if cylinder needs it, to protect morning preheat. HP is at capacity — every minute matters.

**HwcMode (eco/normal) control**: Currently read-only via `hmu HwcMode` — must be changed on the aroTHERM controller physically. Investigation needed: the VWZ AI (0x76) has extensive undecoded B512/B513 register traffic and its own control panel (manual p22). There may be a writable register on the VWZ AI or an undiscovered VRC 700 register that controls eco/normal. The SetMode message from VRC 700 → VWZ AI includes DHW mode bits — if we can decode these, we might be able to set HwcMode indirectly. Until then, the planner must detect the active mode from max flow temp (≥50°C = normal, <50°C = eco) and plan accordingly.

### Phase 3: Predictive DHW compensation

15 min before predicted DHW charge, boost target_flow to pre-raise Leather by ~0.3°C. After DHW finishes, immediate outer-loop recalc.

## Pilot data findings

70 data points from V1 pilot (31 Mar - 1 Apr 2026), curves 0.10-1.00, outside 10.9-16.4°C.

**V1 bang-bang failure mode** (31 Mar overnight): Leather 21.3°C at 23:00, `heating_coast` ratcheted curve 0.55→0.05 over 2h. House cooled to 19.8°C by 07:00. `heating_recovery` then ratcheted 0.10→1.00 over 2.5h (one step per 15 min). Took 2h22m to recover 19.8→20.0°C. Recovery massively overshot - curve 1.00 (flow ~33°C) when model only needs ~27°C. Phase 1a's inner loop eliminates this.

**Phase 1a overnight** (1-2 Apr): Overnight coast worked well (20.6→20.2°C in 5h). But `room_offset` ran away to +2.18°C, suppressing preheat target_flow to 23.5°C when 31.2°C was needed. Inner loop hunting near curve floor (5 oscillations in 5 min at 0.11↔0.19). DHW at 05:09 stole 1.5h of preheat. Leather reached 19.9°C at 07:15 - below comfort. After removing room_offset and restarting, target_flow immediately corrected to 31.2°C.

**Exponent**: Best fit 1.25-1.27 (RMSE 0.63°C deduplicated daytime). Vaillant manual says 1.10 - underpredicts by 2.5-3.1°C at curves ≥0.50. Correcting for actual VRC 700 setpoint per hour doesn't change the result.

**Optimum Start**: At 03:00 (3h before 06:00 day timer), `Hc1ActualFlowTempDesired` jumped 21.0→22.3°C with curve at 0.10. VRC 700 silently ramps effective setpoint. No register to disable. `Z1OpMode=night` eliminates it.

**VRC 700 is opaque**: Back-solving night data gives effective setpoint ~20°C (neither `Z1NightTemp`=19 nor `Z1DayTemp`=21). Hidden `Hc1MinFlowTempDesired`=20°C floor, undocumented offsets. **Do not model the VRC 700's formula. Treat as black box. Validate thermal model on actual measured flow/return temps only.**

## Key files

| File | Purpose |
|---|---|
| `src/bin/adaptive-heating-mvp.rs` | Controller source (deployed as `src/main.rs` on pi5data) |
| `model/adaptive-heating-mvp.toml` | Config |
| `model/control-table.json` | MWT lookup (Phase 1 only, removed in 1b) |
| `src/thermal/display.rs` | `solve_equilibrium_temps()`, `bisect_mwt_for_room()` |
| `src/thermal/physics.rs` | Energy balance, radiator output, thermal mass |
| `data/canonical/thermal_geometry.json` | Room geometry (needed by solver in Phase 1b) |
| `deploy/adaptive-heating-mvp.service` | systemd unit |

## eBUS quick reference

Writes to circuit `700`. TCP `localhost:8888` on pi5data.

| Register | R/W | Notes |
|---|---|---|
| `Z1OpMode` | RW | 0=off, 1=auto, 2=day, **3=night** |
| `Hc1HeatCurve` | RW | 0.10-4.00 |
| `Hc1ActualFlowTempDesired` | R | **Inner loop feedback** |
| `DisplayedOutsideTemp` | R | Filtered outside temp |
| `HwcStorageTemp` | R | Cylinder NTC (reads cold after draws - see DHW section) |
| `HwcSFMode` | RW | auto / load |
| `RunDataStatuscode` | R (hmu) | HP state (Heating/Warm_Water/Standby/etc) |
| `RunDataFlowTemp` | R (hmu) | Actual flow |
| `RunDataReturnTemp` | R (hmu) | Actual return |
| `CurrentCompressorUtil` | R (hmu) | HP load % |
| `Hc1MinFlowTempDesired` | RW | Currently 20°C (VRC 700 floor). Never binds with SP=19 since formula always outputs ≥19°C. Rads produce 0W at flow ≤22°C with room at 20°C. |
