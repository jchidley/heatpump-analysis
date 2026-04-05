# Heating Control

V2 model-predictive controller for the Vaillant aroTHERM Plus. The live controller now runs the thermal solver across day and night with an active DHW launch scheduler plus timer fallback rails.

## Control Objective

Leather 20–21°C during waking hours (07:00–23:00) at minimum electricity cost. Overnight temperature is not a target — it dips and recovers by 07:00.

## Two-Loop Architecture

VRC 700 treated as a black box. `Z1OpMode=night` (SP=19) on startup eliminates Optimum Start and timer interference. Inner loop closes on `Hc1ActualFlowTempDesired` readback.

### Outer Loop

Runs every 900s ([[src/bin/adaptive-heating-mvp.rs#run_outer_cycle]]). Open-Meteo forecast + live thermal solver ([[src/thermal/display.rs#bisect_mwt_for_room]]) → target flow temp → initial curve guess via [[src/bin/adaptive-heating-mvp.rs#calculate_required_curve_for_target]].

Uses forecast temperature, solar irradiance, and humidity. `ForecastCache` refreshes every 3600s. When compressor is not actively heating, falls back to `default_delta_t_c` (4.0°C) instead of live flow-return ΔT — prevents target oscillation when flow ≈ return.

Space-heating demand is now generated from a continuous Leather trajectory instead of a separate overnight planner. During waking hours the trajectory target is the normal comfort setpoint. Overnight it ramps from roughly target−1°C at 23:00 back to the waking target by 07:00, with explicit coast allowed when actual Leather is already above the trajectory and outside temperature is not in the cold-deficit region.

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

`RuntimeState.heating_off` tracks when `Z1OpMode=off`. The outer loop restores `Z1OpMode=night` before any active heating write, and leaves the system in `off` only while Leather is at least ~0.3°C above the overnight trajectory target, outside temperature is ≥2°C, and waking time is not imminent.

### Trajectory Logic

`[[src/bin/adaptive-heating-mvp.rs#overnight_target_leather]]` defines the overnight room target as a continuous trajectory rather than a binary preheat schedule.

- **Target shape**: Leather target ramps from roughly target−1°C after 23:00 back to the waking target by 07:00
- **Continuous solve**: each outer-loop tick calls [[src/bin/adaptive-heating-mvp.rs#calculate_required_curve_for_target]] with the trajectory target, so the same solver path serves day and night
- **Coast gate**: [[src/bin/adaptive-heating-mvp.rs#should_coast_overnight]] allows `Z1OpMode=off` only when Leather is above target and outside temperature is not in the <2°C deficit zone
- **Cold night behaviour**: below 2°C outside, the coast gate stays closed and the model keeps heating active

### Empirical Parameters

Hardcoded constants in `adaptive-heating-mvp.rs` that drive overnight decisions.

| Parameter | Code value | Empirical | Status |
|---|---|---|---|
| Leather τ | 50h | 50h (53 daytime segments) | Validated. Overnight τ unknown |
| Reheat K | 7,500 | ~20,600 (27 segments) | Code conservative, each coast validates |
| Effective HTC | 261 W/K (model) | ~190 W/K (466 nights) | Model overpredicts loss by ~30% |

### 466-Night Analysis

All 466 historical nights (Oct 2024 – Apr 2026) ran at curve=0.55, SP=19, MinFlow=20. Flow temps were 24–40°C depending on outside temp — never lower. Leather dropped 0.8–1.3°C. Lower flow temps have never been tested overnight.

COP improves significantly at lower flow temps (from 1,067 heating samples):

| Outside | Flow 25–30°C | Flow 30–35°C | Flow 35–40°C |
|---|---|---|---|
| 5–8°C | COP 5.2 | COP 4.5 | COP 3.5 |
| 8–12°C | COP 5.9 | COP 5.4 | — |
| 12–18°C | COP 6.3 | COP 5.8 | — |

The real optimisation is finding the minimum overnight curve where Leather reaches 20°C by 07:00.

### Active DHW Scheduling

Morning DHW contention is largely eliminated on clean crossover nights, so most nights remain a pure heating problem and only depleted evenings need another charge.

The live controller now predicts T1 at 07:00, reads `hmu HwcMode`, and scores overnight non-Cosy launches against the explicit `energy-hub` headroom signal `emon/tesla/discretionary_headroom_to_next_cosy_kWh`. That signal represents spare discretionary battery kWh before the next Cosy window; the controller compares it with the expected DHW event kWh for eco vs normal mode. Timer windows are still maintained as fallback rails by [[src/bin/adaptive-heating-mvp.rs#sync_morning_dhw_timer]], so the VRC 700 can still catch a missed software launch. Raw Powerwall SoC / power topics remain useful for observability and operator review, but the scheduling decision no longer re-derives adequacy locally from them. Any period-specific heuristics should remain secondary sense checks on top of the model, not the decision source itself. Review at least one live cycle / overnight window before treating the new headroom signal as fully trusted operational input.

## Pilot History

Key findings from V1 and V2 deployment that shaped the current design.

- **V1 bang-bang rejected** (31 Mar 2026): ±0.10 curve every 15 min ping-ponged 0.55→0.10→1.00. Leather τ=50h means 15-min adjustments are noise.
- **EMA learning rejected**: `room_offset` EMA ran away to +2.18°C overnight (learned cooling trend as "model error", suppressed preheat by ~8°C). Static calibration only.
- **Curve 0.10 ≠ off** (4 Apr 2026): first coast night was confounded — HP still cycling at curve 0.10 due to MinFlowTemp=20 floor. Led to Z1OpMode=off for genuine coast.
- **τ correction** (4 Apr 2026): `LEATHER_TAU_H` changed from 15→50. Missing `break` in planner meant coast=0 always won.
- **Sawtooth flag false alarm**: `daytime_model` ↔ `hold` alternations during DHW charges, not real curve oscillation.
- **Inner loop standby runaway** (5 Apr 2026): `Hc1ActualFlowTempDesired=0.0` during HP standby caused `error≈29°C`, ramping curve to 3.3+ before the next outer tick. Fixed with `fd < 1.0` guard. Also discovered: reqwest needs `rustls-tls` for aarch64 cross-compilation.

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
