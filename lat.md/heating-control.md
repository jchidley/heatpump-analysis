# Heating Control

V2 model-predictive controller for the Vaillant aroTHERM Plus. The live controller now runs the thermal solver across day and night with an active DHW launch scheduler plus timer fallback rails.

## Control Objective

Minimise total electrical input while keeping Leather in the 20–21°C comfort band during waking hours (07:00–23:00).

Electrical kWh is the only cost function. COP is a derived intermediate, not a target — if electrical input is minimised, COP is necessarily good. Thermal energy is an output of the physics, not a goal. This principle drives every control decision: overnight trajectory shape, coast/heat switching, flow-temp selection, and DHW scheduling.

## Two-Loop Architecture

VRC 700 treated as a black box. `Z1OpMode=night` (SP=19) on startup eliminates Optimum Start and timer interference. Inner loop closes on `Hc1ActualFlowTempDesired` readback.

### Outer Loop

Runs every 900s ([[src/bin/adaptive-heating-mvp.rs#run_outer_cycle]]). Open-Meteo forecast + live thermal solver ([[src/thermal/display.rs#bisect_mwt_for_room]]) → target flow temp → initial curve guess via [[src/bin/adaptive-heating-mvp.rs#calculate_required_curve_for_target]].

Uses forecast temperature, solar irradiance, and humidity. `ForecastCache` refreshes every 3600s. When compressor is not actively heating, falls back to `default_delta_t_c` (4.0°C) instead of live flow-return ΔT — prevents target oscillation when flow ≈ return. Model calculation runs every tick even during DHW charges — only eBUS writes are suppressed. Action is logged as `dhw_active` with full model fields so the controller is never blind.

Space-heating demand is generated from a Leather trajectory. During waking hours the target is the midband comfort setpoint (20.5°C). Overnight the target steps down to the comfort-band floor (20.0°C) — a flat hold, not a ramp. Coast is allowed when Leather is above the floor and outside temperature is not in the cold-deficit region. This minimises electrical input: coast is free, and holding the floor at equilibrium flow uses the lowest possible flow temp. See [[heating-control#Overnight Strategy#Trajectory Logic]].

Before and alongside space-heating decisions, the same loop predicts cylinder-top T1 at waking time and calls [[src/bin/adaptive-heating-mvp.rs#sync_morning_dhw_timer]] so VRC 700 timer windows remain fallback rails. It reads `hmu HwcMode`, raw Powerwall telemetry for observability, and the explicit `energy-hub` headroom topic `emon/tesla/discretionary_headroom_to_next_cosy_kWh`. Overnight non-Cosy launches are now judged against that headroom signal rather than re-deriving battery adequacy locally from SoC and power flows. When the chosen slot is active and predicted T1 at 07:00 falls below the comfort floor, it actively launches DHW via `HwcSFMode=load`.

The contract is now: `energy-hub` publishes discretionary battery headroom to the next Cosy window, while the heating controller decides whether that headroom is worth spending on DHW timing. The morning/afternoon/evening asymmetry remains an operational sense check, not the primary control law: the actual decision should remain model-based and telemetry-driven, with heuristics used only to sanity-check outputs and highlight suspicious conclusions.

### Inner Loop

Runs every ~60s ([[src/bin/adaptive-heating-mvp.rs#run_inner_cycle]]). Proportional feedback: `error = target_flow - Hc1ActualFlowTempDesired`, `curve += gain × error`. Converges in 1–2 ticks.

| Parameter | Value |
|---|---|
| Gain | 0.05 (halved to 0.025 below curve 0.25) |
| Deadband | 0.5°C (doubled to 1.0°C below curve 0.25) |
| Max step | 0.20 |
| Curve clamp | 0.10–4.00 |

Floor guard: halved gain + doubled deadband when `Hc1HeatCurve < 0.25` prevents hunting where 0.01 curve ≈ 0.20°C flow change.

Standby guard: when `Hc1ActualFlowTempDesired < 1.0` the inner loop skips entirely. During HP standby this register reads 0.0, which without the guard causes `error ≈ 29°C` and ramps the curve to 3+ before the next outer tick resets it.

When `target_flow_c` is `None` (overnight coast), the inner loop does nothing.

## Startup and Shutdown

Startup sequence establishes clean control state. Shutdown restores VRC 700 to autonomous operation.

**Startup**: `Z1OpMode=night` (value 3) + `Hc1MinFlowTempDesired=19`. Night mode uses `Z1NightTemp` (19°C) permanently — flat setpoint, no timer transitions. MinFlow lowered from 20→19 to remove the hidden floor that prevented genuine coast (curve 0.10 at MinFlow=20 still produced 20°C+ flow).

**Shutdown** ([[src/bin/adaptive-heating-mvp.rs#restore_baseline]]): `Z1OpMode=auto` + `Hc1HeatCurve=0.55` + `Hc1MinFlowTempDesired=20` + `HwcSFMode=auto`, and all `HwcTimer_<Weekday>` registers are restored to the three-window Cosy baseline. VRC 700 resumes timer control with factory defaults.

## Modes

Each mode has its own branch in the outer loop. Mode persisted as TOML in state file, changeable via HTTP API.

| Mode | Behaviour |
|---|---|
| `Occupied` | Full comfort targeting (two-loop control) |
| `ShortAbsence` | Reduced target |
| `AwayUntil` | 15°C frost protection (curve 0.30, ~£0.50/day vs ~£2.50). Week away saves ~£14 |
| `Disabled` | No eBUS writes |
| `MonitorOnly` | Read-only, log decisions without writing |

API on port 3031: `/status`, `/mode/{mode}`, `/kill` (baseline restore). Mobile controls proxied via z2m-hub (:3030).

## Overnight Strategy

Overnight heating now follows the same live thermal solver as daytime control, but against a time-varying room target and a coast heuristic.

### Coast Mechanism

Coast turns heating **off** via `Z1OpMode=off` — not a low curve. This was changed after discovering that curve 0.10 at SP=19 with `Hc1MinFlowTempDesired=20` still produced 20°C+ flow temp (the hidden floor prevented genuine coasting).

`RuntimeState.heating_off` tracks when `Z1OpMode=off`. The outer loop restores `Z1OpMode=night` before any active heating write, and leaves the system in `off` only while Leather is at least 0.15°C above the overnight comfort floor, outside temperature is ≥2°C, and waking time is not imminent. Coasting is free (zero electrical input) so the margin is kept tight (just above sensor resolution) to maximise coast duration.

### Trajectory Logic

`[[src/bin/adaptive-heating-mvp.rs#overnight_target_leather]]` returns a flat overnight target at the comfort-band floor (20.0°C), not a ramp.

**Physics rationale** (lumped-capacitance optimal control): total electrical cost = ∫ Q_hp/COP(T_flow) dt. Heat-pump COP degrades with flow temperature, so higher thermal output → higher flow → worse COP. A linear ramp back-loaded the hardest temperature rise into the final hours, demanding high flow when the room’s exponential approach (τ ≈ 36 h operational) was slowest — the controller could never catch up, arriving 0.3°C below target at 07:00. Simulation (outside 9.5°C): linear ramp used 2.82 kWh; coast-then-hold-floor uses 1.86 kWh (−34%).

- **Target shape**: flat at `target_leather_c − 0.5` (comfort-band floor, 20.0°C). During waking hours, steps to midband target (20.5°C).
- **Coast phase**: [[src/bin/adaptive-heating-mvp.rs#should_coast_overnight]] allows `Z1OpMode=off` while Leather > floor + 0.15°C (sensor-noise deadband). Coasting is free — every minute at Q=0 is minimum electrical cost.
- **Hold phase**: once Leather reaches the floor, the thermal solver finds equilibrium flow — the lowest possible flow temp. This IS the electrical minimum: deeper coasting saves thermal energy but the reheat needs T_ss far above target (with τ=36h, reheating 0.5°C in 2.8h requires aiming at T_ss well above target), so the COP penalty from higher reheat flow always exceeds the thermal saving. At outside 9.5°C: hold at 20.0 costs 1.86 kWh; coast to 19.5 then reheat costs 1.96 kWh (+5%); coast to 19.3 costs 2.14 kWh (+15%). The result is even stronger at τ=36h than the original 50h analysis — faster cooling means less coast time and even higher reheat flow.
- **Waking transition**: at 07:00 the target steps from 20.0 to 20.5°C. The daytime solver handles the 0.5°C lift with the same minimum-flow logic.
- **Cold night behaviour**: below 2°C outside, the coast gate stays closed and the model keeps heating active.

### Empirical Parameters

Hardcoded constants in `adaptive-heating-mvp.rs` that drive overnight decisions.

| Parameter | Code value | Empirical | Status |
|---|---|---|---|
| Leather τ | 36h | 36h median (8 segments: cal nights + DHW + coast) | Revised from 50h — operational overnight cooling is faster than daytime |
| Comfort floor offset | 0.5°C below target | — | Band floor = 20.0°C when target = 20.5 |
| Coast margin | 0.15°C | sensor resolution 0.1°C | Avoids hunting without wasting coast |
| Effective HTC | 261 W/K (model) | ~190 W/K (466 nights) | Model overpredicts loss by ~30% |

### 466-Night Analysis

All 466 historical nights (Oct 2024 – Apr 2026) ran at curve=0.55, SP=19, MinFlow=20. Flow temps were 24–40°C depending on outside temp — never lower. Leather dropped 0.8–1.3°C. Lower flow temps have never been tested overnight.

COP improves significantly at lower flow temps (from 1,067 heating samples):

| Outside | Flow 25–30°C | Flow 30–35°C | Flow 35–40°C |
|---|---|---|---|
| 5–8°C | COP 5.2 | COP 4.5 | COP 3.5 |
| 8–12°C | COP 5.9 | COP 5.4 | — |
| 12–18°C | COP 6.3 | COP 5.8 | — |

The real optimisation is minimising total electrical input while keeping Leather ≥20.0°C at 07:00. Lower flow → better COP → less electricity; the coast-then-hold strategy achieves this by letting the room cool for free then holding the comfort floor at equilibrium flow.

### Per-Room Comfort Implications

The controller targets Leather but all rooms are on the same circuit. Empirical τ varies 22–57h across rooms (see [[thermal-model#Empirical Room Time Constants]]), so overnight cooling rates differ.

| Room | τ (h) | Actual morning range | Occupied overnight | Status |
|---|---|---|---|---|
| aldora | 41 | 20.0–21.3°C | Yes (child) | ✅ Fine — sealed room + body heat holds temp. Mould risk is the problem, not cold. |
| jackcarol | 57 | not yet tracked | Yes (2 adults) | ✅ Likely fine — slowest-cooling bedroom, 2 occupants |
| elvina | 34 | **16.4–19.4°C** | Yes (child, allergies) | ⚠️ **Actionable** — regularly 17°C at 07:00. Nearly all excess loss is ventilation (~3× Aldora rate). HEPA purifier already runs. Closing trickle vents + door ajar would cut UA 32→17 W/K, gaining ~3°C. Validate with CO2 monitor. |
| leather | 36 | 20.0–20.5°C | Dog (PRT, door closed) | ✅ Primary control room, held at comfort floor |
| office | 22 | not yet tracked | No | Fastest cooling but unoccupied overnight |
| bathroom | 25 | not yet tracked | No | MVHR ventilation loss, unoccupied overnight |
| front | 28 | not yet tracked | No | Unoccupied overnight |
| hall | 29 | not yet tracked | No | Stairwell, transit only |
| kitchen | 27 | not yet tracked | No | No radiator, unoccupied |

Actual morning temps from 7 days of sensor data (31 Mar–7 Apr). Elvina is the only occupied bedroom with a comfort problem: it cools from ~18–20°C at bedtime to 16–17°C by morning. Full overnight moisture analysis (13 sensors as proxy network) shows nearly all excess UA is ventilation (ACH ≈1.0 vs model 0.51), not fabric. Closing trickle vents would cut UA 32→17 W/K (+3°C overnight) while the HEPA purifier provides better allergen control. Leather emonth2 humidity confirms low ventilation (dog + closed door → ΔAH 0.39 g/m³, consistent with ACH ~0.6). Aldora is the opposite — sealed and warm overnight but with a mould risk from inadequate ventilation.

### Active DHW Scheduling

Morning DHW contention is largely eliminated on clean crossover nights, so most nights remain a pure heating problem and only depleted evenings need another charge.

During any Cosy window, DHW launches unconditionally if the cylinder needs charging — battery state is irrelevant because grid electricity is at its cheapest. Outside Cosy windows, the controller checks the `energy-hub` headroom signal `emon/tesla/discretionary_headroom_to_next_cosy_kWh` before launching. Currently only the overnight battery slot (00:00–04:00) is actively scheduled; the other non-Cosy gaps (07:00–13:00, 16:00–22:00) have no software DHW scheduling and rely on VRC 700 timer fallback rails. That signal is `available_above_reserve − projected_base_load_to_next_cosy`; positive means spare capacity, negative means deficit. The gate is `headroom >= dhw_event_kwh` (eco 1.9 kWh, normal 2.4 kWh). The controller also predicts T1 at 07:00 and reads `hmu HwcMode` for the eco/normal kWh estimate. Timer windows are still maintained as fallback rails by [[src/bin/adaptive-heating-mvp.rs#sync_morning_dhw_timer]]. Raw Powerwall SoC / power topics remain useful for observability but the scheduling decision uses the headroom signal, not raw telemetry.

The headroom signal went live on 5 Apr 2026 (deployed to emonpi ~22:20 BST, verified end-to-end: MQTT → Telegraf → InfluxDB → controller). Publishing every 10s with no gaps since deployment. Note: the energy-hub headroom value is unreliable during Cosy windows because it projects drain from current SoC without accounting for active grid charging — this doesn't matter because the controller ignores it during Cosy slots.

## Pilot History

Key findings from V1 and V2 deployment that shaped the current design.

- **V1 bang-bang rejected** (31 Mar 2026): ±0.10 curve every 15 min ping-ponged 0.55→0.10→1.00. Leather τ=36h means 15-min adjustments are noise.
- **EMA learning rejected**: `room_offset` EMA ran away to +2.18°C overnight (learned cooling trend as "model error", suppressed preheat by ~8°C). Static calibration only.
- **Curve 0.10 ≠ off** (4 Apr 2026): first coast night was confounded — HP still cycling at curve 0.10 due to MinFlowTemp=20 floor. Led to Z1OpMode=off for genuine coast.
- **τ correction** (4 Apr 2026): `LEATHER_TAU_H` changed from 15→50, later revised to 36 (operational overnight τ from 8 cooling segments). Missing `break` in planner meant coast=0 always won.
- **Sawtooth flag false alarm**: `daytime_model` ↔ `hold` alternations during DHW charges, not real curve oscillation.
- **Inner loop standby runaway** (5 Apr 2026): `Hc1ActualFlowTempDesired=0.0` during HP standby caused `error≈29°C`, ramping curve to 3.3+ before the next outer tick. Fixed with `fd < 1.0` guard. Also discovered: reqwest needs `rustls-tls` for aarch64 cross-compilation.
- **First trajectory overnight success** (5 Apr 2026): Leather 21.9→20.4°C coast (5h compressor off), preheat from 05:00 Cosy, Leather 20.5°C at 07:00 — model predicted correctly. Outside 9–12°C.
- **Headroom signal deployed** (5 Apr 2026): `derive_discretionary_headroom()` committed but not deployed to emonpi — controller saw `null` for 22 ticks (16:51–22:14 BST) until manual deploy + restart of energy-hub-tesla.service. First non-null value −9.3 kWh at 22:30 BST. Signal confirmed continuous (every 10s) from 22:25 BST onward — no further gaps. Lesson: verify MQTT topic is actually arriving after any energy-hub change.
- **Headroom unreliable during Cosy** (5 Apr 2026): headroom showed −9.3 kWh inside the 22:00 Cosy window because derivation projects drain from current SoC without accounting for active grid charging. Doesn't affect control (controller ignores headroom during Cosy slots) but the signal is misleading for observability. Energy-hub fix needed: either return null or project forward through remaining Cosy charging.
- **Forecast nulls during DHW** (5 Apr 2026): when HP is in DHW mode (`Warm_Water_Compressor_active`), `forecast_outside_c` and all `model_required_*` fields were null, action was "no rule fired". Observed for 4 consecutive outer ticks (21:14–22:02 BST). Fixed 6 Apr — see below.
- **Second trajectory overnight — slight undershoot** (6 Apr 2026): Leather 21.3→20.2°C coast (23:05→03:12 BST, 4h compressor off), preheat from 03:12, outside 7–9°C. Leather only 20.1°C at 07:00 vs 20.5°C target. Coast and model performed well, but DHW charge at 04:16–04:35 BST interrupted preheat with forecast nulls for ~1h, likely causing the 0.4°C miss.
- **DHW timer dedup bug caused preheat contention** (6 Apr 2026): `sync_morning_dhw_timer` correctly decided to skip the 04:00 morning window (T1 41.5°C predicted) but the eBUS write failed. Dedup state was set anyway, suppressing retries. `restore_baseline` then re-enabled all windows without clearing dedup state. VRC 700 fired DHW at 04:00 (HwcStorageTemp 37.5°C < 45°C threshold), interrupting preheat. Fixed: dedup state now cleared on write failure and on startup.
- **Forecast nulls during DHW fixed** (6 Apr 2026): root cause was `!is_dhw` guard skipping the entire model calculation block, not just eBUS writes. Fix: model (forecast + thermal solver) now runs every tick regardless of HP mode. During DHW, writes suppressed but `target_flow_c` populated and action logged as `dhw_active` with full model fields. Inner loop can resume immediately when DHW finishes.
- **T1 standby decay recalibrated** (6 Apr 2026): 47 flow-filtered standby segments (≥2h each, 10-min resolution, 18 days) measured: mean 0.21, median 0.22, P75 0.23, P90 0.24 °C/h. Constant set to P75 (0.23°C/h). Previous value 0.25 was at P90, directionally correct. Initial naive analysis from hourly averages had wrongly suggested 0.12 — the hourly windows spanned charge events and were unreliable.

## Writable eBUS Registers

The controller writes to a small set of VRC 700 registers via ebusd TCP.

| Register | Purpose |
|---|---|
| `Hc1HeatCurve` | Primary control lever (0.10–4.00, IEEE 754 float) |
| `Z1OpMode` | 0=off, 1=auto, 2=day, 3=night |
| `Hc1MinFlowTempDesired` | Flow temp floor (19 during operation, 20 on restore) |
| `HwcSFMode` | Active DHW launch lever (`auto` / `load`) |
| `HwcTimer_<Weekday>` | Fallback rails; skip or keep the morning Cosy window based on predicted T1 |

Future: `SetModeOverride` to HMU bypasses VRC 700. Message format decoded (D1C encoding). Requires outpacing the 700's 30-second writes.

## Key Files

Source, config, and deployment files for the adaptive heating controller.

| File | Purpose |
|---|---|
| `src/bin/adaptive-heating-mvp.rs` | Controller binary (~2,053 lines) |
| `model/adaptive-heating-mvp.toml` | Config (eBUS, InfluxDB, Cosy windows, baseline, tuning) |
| `src/thermal/display.rs` | [[src/thermal/display.rs#bisect_mwt_for_room]], [[src/thermal/display.rs#solve_equilibrium_temps]] |
| `data/canonical/thermal_geometry.json` | Room geometry for solver |
| `deploy/adaptive-heating-mvp.service` | systemd unit for pi5data |
