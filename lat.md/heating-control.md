# Heating Control

V2 model-predictive controller for the Vaillant aroTHERM Plus. Two-loop architecture with overnight planner.

## Control Objective

Leather 20–21°C during waking hours (07:00–23:00) at minimum electricity cost. Overnight temperature is not a target — it dips and recovers by 07:00.

## Two-Loop Architecture

VRC 700 treated as a black box. `Z1OpMode=night` (SP=19) on startup eliminates Optimum Start and timer interference. Inner loop closes on `Hc1ActualFlowTempDesired` readback.

### Outer Loop

Runs every 900s ([[src/bin/adaptive-heating-mvp.rs#run_outer_cycle]]). Open-Meteo forecast + live thermal solver ([[src/thermal/display.rs#bisect_mwt_for_room]]) → target flow temp → initial curve guess via [[src/bin/adaptive-heating-mvp.rs#calculate_required_curve]].

Uses forecast temperature, solar irradiance, and humidity. `ForecastCache` refreshes every 3600s. When compressor is not actively heating, falls back to `default_delta_t_c` (4.0°C) instead of live flow-return ΔT — prevents target oscillation when flow ≈ return.

### Inner Loop

Runs every ~60s ([[src/bin/adaptive-heating-mvp.rs#run_inner_cycle]]). Proportional feedback: `error = target_flow - Hc1ActualFlowTempDesired`, `curve += gain × error`. Converges in 1–2 ticks.

| Parameter | Value |
|---|---|
| Gain | 0.05 (halved to 0.025 below curve 0.25) |
| Deadband | 0.5°C (doubled to 1.0°C below curve 0.25) |
| Max step | 0.20 |
| Curve clamp | 0.10–4.00 |

Floor guard: halved gain + doubled deadband when `Hc1HeatCurve < 0.25` prevents hunting where 0.01 curve ≈ 0.20°C flow change.

When `target_flow_c` is `None` (overnight coast), the inner loop does nothing.

## Startup and Shutdown

Startup sequence establishes clean control state. Shutdown restores VRC 700 to autonomous operation.

**Startup**: `Z1OpMode=night` (value 3) + `Hc1MinFlowTempDesired=19`. Night mode uses `Z1NightTemp` (19°C) permanently — flat setpoint, no timer transitions. MinFlow lowered from 20→19 to remove the hidden floor that prevented genuine coast (curve 0.10 at MinFlow=20 still produced 20°C+ flow).

**Shutdown** ([[src/bin/adaptive-heating-mvp.rs#restore_baseline]]): `Z1OpMode=auto` + `Hc1HeatCurve=0.55` + `Hc1MinFlowTempDesired=20`. VRC 700 resumes timer control with factory defaults.

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

Overnight planner decides between coast, maintain, or preheat based on cooling simulation and reheat estimation.

### Coast Mechanism

Coast turns heating **off** via `Z1OpMode=off` — not a low curve. This was changed after discovering that curve 0.10 at SP=19 with `Hc1MinFlowTempDesired=20` still produced 20°C+ flow temp (the hidden floor prevented genuine coasting).

`RuntimeState.heating_off` tracks when `Z1OpMode=off`. Two restore points write `Z1OpMode=night` to re-enable heating:
1. Entering waking hours or preheat period
2. During overnight when `maintain_heating` becomes true or preheat is ≤15 min away

### Planner Logic

Binary search ([[src/bin/adaptive-heating-mvp.rs#plan_overnight]]) for latest safe preheat start that delivers Leather ≥20°C by 07:00.

- **Cooling simulation** ([[src/bin/adaptive-heating-mvp.rs#simulate_cooling]]): `projected = current − (current − outside) × (1 − exp(−hours/τ))` with τ=50h ([[src/bin/adaptive-heating-mvp.rs#LEATHER_TAU_H]], empirical, 53 segments)
- **Reheat estimation** ([[src/bin/adaptive-heating-mvp.rs#estimate_reheat_hours]]): `hours = (target − projected) × thermal_mass / (K × 3600)` with K=7500 (empirical K≈20,600 from 27 segments — each coast night validates)
- **Cold night override**: below 2°C outside, maintain heating at model-derived curve (HP in deficit, can't recover)

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

### Next: Unified Model

Replace the separate overnight planner with `bisect_mwt_for_room` running 24/7 on a time-varying target trajectory.

The question: what Leather trajectory from 23:00 to 07:00 delivers ≥20°C at 07:00 at minimum electricity? Candidate shapes: flat hold, slow ramp, bank+coast, off+preheat.

## Pilot History

Key findings from V1 and V2 deployment that shaped the current design.

- **V1 bang-bang rejected** (31 Mar 2026): ±0.10 curve every 15 min ping-ponged 0.55→0.10→1.00. Leather τ=50h means 15-min adjustments are noise.
- **EMA learning rejected**: `room_offset` EMA ran away to +2.18°C overnight (learned cooling trend as "model error", suppressed preheat by ~8°C). Static calibration only.
- **Curve 0.10 ≠ off** (4 Apr 2026): first coast night was confounded — HP still cycling at curve 0.10 due to MinFlowTemp=20 floor. Led to Z1OpMode=off for genuine coast.
- **τ correction** (4 Apr 2026): `LEATHER_TAU_H` changed from 15→50. Missing `break` in planner meant coast=0 always won.
- **Sawtooth flag false alarm**: `daytime_model` ↔ `hold` alternations during DHW charges, not real curve oscillation.

## Writable eBUS Registers

The controller writes to a small set of VRC 700 registers via ebusd TCP.

| Register | Purpose |
|---|---|
| `Hc1HeatCurve` | Primary control lever (0.10–4.00, IEEE 754 float) |
| `Z1OpMode` | 0=off, 1=auto, 2=day, 3=night |
| `Hc1MinFlowTempDesired` | Flow temp floor (19 during operation, 20 on restore) |
| `HwcSFMode` | DHW boost (auto / load) |

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
