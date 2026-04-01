# Adaptive Heating V2 — Model-Predictive Control

Last updated: 1 April 2026

## Objective

Leather room 20–21°C during waking hours (07:00–23:00) at minimum electricity cost, with reliable DHW.

Constraints: HP maxes out below 6°C outside (accept 19.5–20°C). DHW steals HP for ~1h. Tariff optimisation not worth the complexity. Overnight temp is a free variable.

## Architecture

### VRC 700 control

On startup: `Z1OpMode=night` (value 3). VRC 700 uses `Z1NightTemp` (19°C) permanently. Disables CcTimer, Optimum Start, day/night transitions. Setpoint values are never modified.

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
        curve += 0.10 × error      (max step 0.20, clamp 0.10–4.00)
        write Hc1HeatCurve
```

The outer loop uses the calibrated thermal physics model. The inner loop treats the VRC 700 as a black box — nudge curve until output matches target. The formula is only the initial guess; the inner loop converges in 2–3 minutes.

**Why not open-loop formula?** The VRC 700's internal computation is opaque — hidden Optimum Start, `Hc1MinFlowTempDesired`=20°C floor, undocumented offsets. See "Pilot data findings" below.

### Error correction

**Inner loop** replaces the old `flow_offset` EMA — closes directly on `Hc1ActualFlowTempDesired`.

**`room_offset`** (outer loop): after 2h+ at stable flow, compare equilibrium model prediction vs actual Leather. EMA α=0.2, clamped ±3°C. Applied: `bisect_mwt_for_room("leather", target - room_offset, ...)`.

### DHW

Cosy windows (04:00–07:00, 13:00–16:00, 22:00–23:59) + cylinder < 40°C → `HwcSFMode=load`.

### Safety

- Baseline restore on shutdown: `Z1OpMode=auto`, `Hc1HeatCurve=0.55`
- Solver fails → hold last target_flow, inner loop maintains it
- eBUS fails → hold, don't write
- Inner loop: deadband 0.5°C, max step 0.20, curve 0.10–4.00
- Warn if curve > 1.50
- `room_offset` clamped ±3°C

## Implementation plan

### Phase 1a: Inner feedback loop + fixed setpoint 🔴 NEXT

Fix the deployed Phase 1 code. Replace open-loop formula with closed-loop feedback.

**Config** (`model/adaptive-heating-mvp.toml`):
- `model.setpoint_c` = **19.0** (was 21.0)
- Add: `inner_loop_gain = 0.10`, `inner_loop_deadband_c = 0.5`, `inner_loop_max_step = 0.20`

**Code** (`src/bin/adaptive-heating-mvp.rs`):

1. **Startup**: add `ebusd_write(config, "700", "Z1OpMode", "night")` before control loop.

2. **Add `target_flow_c: Option<f64>`** to shared state between loops.

3. **Split control loop**:
   - Outer (every 900s): `calculate_required_curve()` sets `target_flow_c` instead of writing curve directly. Provides initial curve guess via formula.
   - Inner (every 60s): reads `Hc1ActualFlowTempDesired`, adjusts curve proportionally toward `target_flow_c`. Same guards (not DHW, not defrost, not missing sensors).

4. **Remove `flow_offset`** from `RuntimeState`, `calculate_required_curve()`, logging.

5. **Fix `curve_stable_since`**: init to `Some(Utc::now())` on startup.

6. **Fix `last_leather_prediction_c`**: store `target_leather_c - room_offset` (was storing MWT).

7. **Add curve >1.50 warning**.

8. **Update `restore_baseline()`**: write `Z1OpMode=auto`, `Hc1HeatCurve=0.55`. Remove `Z1DayTemp`, `Z1NightTemp`, `HwcTempDesired` writes.

9. **Update `StatusResponse`**: replace `flow_offset` with `target_flow_c`.

**Deploy**:
```bash
scp src/bin/adaptive-heating-mvp.rs pi5data:~/adaptive-heating-mvp/src/main.rs
scp model/adaptive-heating-mvp.toml pi5data:~/adaptive-heating-mvp/model/
ssh pi5data "source ~/.cargo/env && cd ~/adaptive-heating-mvp && cargo build --release"
ssh pi5data "sudo systemctl restart adaptive-heating-mvp"
curl -s http://pi5data:3031/status | python3 -m json.tool
```

**Validate**: `Z1OpMode` reads `night`. Inner loop converges within 3 cycles. No `flow_offset` in logs. Kill restores `Z1OpMode=auto`.

### Phase 1b: Library crate + live solver

Eliminate control table. Call equilibrium solver directly from binary.

1. Create `src/lib.rs`, move `pub mod thermal` there. Widen `pub(crate)` → `pub` on: `geometry::{build_rooms, build_connections, build_doorways}`, `physics::{full_room_energy_balance_components, radiator_output, compute_thermal_masses}`, `display::{solve_equilibrium_temps, bisect_mwt_for_room}`, `config::*`, `error::*`.

2. Replace `ControlTable` with `bisect_mwt_for_room("leather", target - room_offset, outside, solar, 0.0)`. Remove `control_table_path` config.

3. Deploy `data/canonical/thermal_geometry.json` + `model/thermal-config.toml` to pi5data. Benchmark solver on ARM (<1s).

4. Add event-driven outer loop: trigger on DHW→heating transition, leather deviation >0.5°C for >15 min.

5. Add HP capacity clamp: if `CurrentCompressorUtil` > 95% for >30 min, stop raising curve.

### Phase 2: Overnight planner (requires 1b)

Forward simulation: `overnight_start_time(current_leather, forecast[], dhw_expected, target, target_time)`. Simulate cooling with thermal mass + hourly forecast, heating with radiator output at calculated MWT. Binary search on latest start time for Leather ≥ 20°C by 07:00. Account for 1h DHW interruption. Replaces fixed `preheat_hours`.

### Phase 3: Predictive DHW compensation

15 min before predicted DHW charge, boost target_flow to pre-raise Leather by ~0.3°C. After DHW finishes, immediate outer-loop recalc.

## Pilot data findings

70 data points from V1 pilot (31 Mar – 1 Apr 2026), curves 0.10–1.00, outside 10.9–16.4°C.

**Exponent**: Best fit 1.25–1.27 (RMSE 0.63°C deduplicated daytime). Vaillant manual says 1.10 — underpredicts by 2.5–3.1°C at curves ≥0.50. Correcting for actual VRC 700 setpoint per hour doesn't change the result.

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
| `Hc1HeatCurve` | RW | 0.10–4.00 |
| `Hc1ActualFlowTempDesired` | R | **Inner loop feedback** |
| `DisplayedOutsideTemp` | R | Filtered outside temp |
| `HwcStorageTemp` | R | Cylinder NTC |
| `HwcSFMode` | RW | auto / load |
| `RunDataStatuscode` | R (hmu) | HP state (Heating/Warm_Water/Standby/etc) |
| `RunDataFlowTemp` | R (hmu) | Actual flow |
| `RunDataReturnTemp` | R (hmu) | Actual return |
| `CurrentCompressorUtil` | R (hmu) | HP load % |
| `Hc1MinFlowTempDesired` | RW | Currently 20°C (VRC 700 floor) |
