# Adaptive Heating V2 — Model-Predictive Control

Date: 1 April 2026

## Objective

**Leather room at 20–21°C during waking hours (07:00–23:00) at minimum electricity cost, with reliable DHW.**

Everything else — overnight temperature, curve value, setpoint, heating start time — is a means to that end, not a goal in itself.

### Constraints

- On cold days (<6°C), the 5kW HP cannot reach 21°C. Leather stabilises at 19.5–20°C. The controller must accept this, not fight it.
- The HP can't heat and charge DHW simultaneously. DHW steals the HP for ~1h.
- The Tesla Powerwall covers ~95% of non-Cosy usage at effective 14.63p/kWh. Tariff scheduling yields only £15–40/year — not worth adding complexity for.
- Overnight temperature is irrelevant. Only the morning arrival temperature matters.

## Inputs

### Read every cycle (~1 min sampling, decisions when triggered)

| Input | Source | What it's for |
|---|---|---|
| Leather temp | InfluxDB (emonth2) | Primary comfort target |
| Aldora temp | InfluxDB (Zigbee) | Secondary reference / validation |
| Outside temp | eBUS `DisplayedOutsideTemp` | Equilibrium + curve calculation |
| Cylinder temp | eBUS `HwcStorageTemp` | DHW decisions |
| HP status | eBUS `RunDataStatuscode` | DHW/defrost/heating detection |
| Flow temp actual | eBUS `RunDataFlowTemp` | Verify curve produced right flow |
| Return temp actual | eBUS `RunDataReturnTemp` | Track ΔT for MWT calculation |
| Flow temp desired | eBUS `Hc1ActualFlowTempDesired` | Verify VRC 700 responded to write |
| Current curve | eBUS `Hc1HeatCurve` | Know state before writing |
| Power consumption | eBUS `RunDataElectricPowerConsumption` | Logging / COP |
| Yield power | eBUS `CurrentYieldPower` | Logging / COP |
| Compressor util | eBUS `CurrentCompressorUtil` | Detect HP at capacity |

### Fetched periodically (cache hourly)

| Input | Source | What it's for |
|---|---|---|
| 24h temp forecast | Open-Meteo API | Overnight planning, daytime curve trajectory |
| 24h solar radiation forecast | Open-Meteo API | Solar gain prediction — the equilibrium solver already takes irradiance per orientation and calculates gain through each room's glazing |
| 24h humidity forecast | Open-Meteo API | Future: predict defrost frequency (high humidity + low temp = more defrosts = less HP capacity) |

Open-Meteo URL (free, no API key):
```
https://api.open-meteo.com/v1/forecast?latitude=51.611&longitude=-0.108&hourly=temperature_2m,relative_humidity_2m,direct_radiation&forecast_hours=24&timezone=Europe/London
```

## Outputs

### Written to VRC 700

| Output | Register | When |
|---|---|---|
| Heat curve | `Hc1HeatCurve` | When model calculation produces a different value (>0.05 change) |
| Day setpoint | `Z1DayTemp` | Normally fixed at 21. Changed for away mode. |
| Night setpoint | `Z1NightTemp` | Normally fixed at 19 (safety net). Overnight planner may lower. |
| DHW boost | `HwcSFMode=load` | Cosy window + cylinder below 40°C |

### Logged (InfluxDB + JSONL)

Every decision logs: timestamp, mode, all input values, forecast used, model calculation (target MWT, target flow, required curve), action taken, write results, reason.

## The model

Two formulas connect the real objective to the VRC 700 register:

### 1. Thermal equilibrium: target Leather → required MWT

The calibrated thermal model (`thermal-equilibrium`) solves:

```
given (outside_temp, target_leather_temp) → required MWT
```

| Outside °C | Leather target | Required MWT |
|---|---|---|
| 0 | 20.5 | ~33 |
| 5 | 20.5 | ~30 |
| 10 | 20.5 | ~27 |
| 13 | 20.5 | ~26 |
| 15 | 20.5 | ~25 |

Implemented as bisection on the full 13-room equilibrium solver.

### 2. Heat curve formula: required flow → required curve

Reverse-engineered from Vaillant installation manual (p15) + 13 empirical pilot data points (RMSE 0.74°C):

```
flow_temp = setpoint + curve × (setpoint - outside)^1.27
curve = (target_flow - setpoint) / (setpoint - outside)^1.27
```

Two sources, two exponents:
- **1.10** — from digitising the Vaillant heat curve chart (VRC 700 installation manual p15). Chart is drawn at setpoint 20°C across the full range (-20°C to +15°C outside).
- **1.27** — from fitting 13 actual `Hc1ActualFlowTempDesired` readbacks during the V1 pilot. Setpoint 21°C, outside 11–16°C. RMSE 0.74°C.

The discrepancy is likely a combination of: the VRC 700’s setpoint shift mechanism (curve translates along a 45° axis when setpoint ≠ 20°C, shown in the bottom chart on p15), and fitting to a narrow outside temp range. Use **1.27** — it’s what our VRC 700 actually produces at our setpoint in our operating range. Re-validate if operating at outside temps significantly below 10°C.

The flow temp relates to MWT via the system ΔT: `flow = MWT + ΔT/2` where ΔT is tracked from live readings (typically 3–5°C).

### End-to-end

```
target_leather (20.5°C)
    → required MWT (equilibrium solver, using forecast outside temp)
    → required flow (MWT + ΔT/2)
    → required curve (heat curve formula)
    → write Hc1HeatCurve
```

One calculation, not trial and error. At 13°C outside this gives curve ≈ 0.55 — which is the baseline we started with.

## Control strategy

### Daytime (07:00–23:00): maintain comfort

The forecast gives the full daytime outside temp trajectory. The controller uses it to plan the curve profile for the day, not just react to the current temperature.

1. Each hour (or when forecast updates): calculate the required curve for each of the next few hours using the forecast outside temp **and solar radiation** at that hour
2. Set the curve for the **current hour's forecast conditions** (temp + solar), not the current measured outside temp (unless forecast is unavailable). On a sunny afternoon the equilibrium solver will calculate a lower MWT because solar gain is doing some of the heating.
3. As the day progresses, the curve naturally follows the forecast trajectory:
   - Morning: higher curve (cold, house recovering)
   - Midday: curve reduces as outside warms + solar gain
   - Evening: curve rises again as outside cools
4. Recalculate immediately (don't wait for the next hour) when:
   - DHW charge has just finished (HP available again, Leather has dipped)
   - Leather has drifted >0.5°C from model prediction (unexpected event — door opened, etc.)
5. Each calculation produces an **absolute curve value** from the model, not a delta
6. Only write if the required curve differs from current by >0.05 (avoid pointless eBUS writes)

### Overnight (23:00–07:00): minimise cost, hit target by morning

1. At 23:00, fetch forecast for overnight outside temps (hourly to 07:00)
2. Using the thermal model, simulate forward:
   - House starts at current Leather temp
   - Cooling rate depends on forecast outside temp trajectory
   - DHW charge at 05:30 steals HP for ~1h (if cylinder needs it)
   - Heating rate depends on the curve value and forecast outside temp at that hour
3. Find the **latest heating start time** that achieves Leather ≥ 20°C by 07:00
4. Until that time: let the house cool freely (curve at minimum / heating off)
5. At the calculated start time: set the curve for morning recovery using the model (forecast outside temp at that hour → required MWT → required curve)
6. If HP can't reach 20°C by 07:00 even starting at 23:00 (very cold night): accept the physics, start as early as makes sense, don't waste energy trying to hold an impossible floor

### DHW: simple and independent

1. Is it a Cosy window? (05:30–07:00, 13:00–15:00, 22:00–00:00)
2. Is the cylinder below 40°C?
3. If both yes: `HwcSFMode=load`
4. If no: do nothing

### Predictive planning

The controller doesn't just react to current conditions — it uses the forecast to plan ahead:

| Event | What the controller does |
|---|---|
| **DHW charge approaching** | Cylinder below threshold + Cosy window in <30 min → pre-raise curve so Leather enters the charge 0.3°C higher than needed, exits in band |
| **Outside temp falling** | Forecast shows 5°C drop over next 3h → adjust curve now for the future temp, not current |
| **Outside temp rising** | Forecast shows warming → reduce curve ahead of overshoot |
| **Morning DHW interruption** | Factor 1h HP loss into the overnight heating start time calculation |
| **Solar gain (afternoon)** | Forecast shows high direct radiation → reduce curve before overshoot |
| **Evening cooling** | Forecast shows overnight drop starting → plan curve to leave Leather at 21°C by 23:00 |

### Unexpected events

When Leather deviates >0.5°C from prediction for >30 minutes:
- Recalculate from the model using **current measured state**, not the prediction
- Don't bump the curve by a fixed amount — compute the new absolute target
- Log the deviation for model calibration

## What V1 taught us

The V1 bang-bang controller (±0.10 every 15 min) proved:
- eBUS writes work reliably (curve + setpoint accepted by VRC 700)
- The VRC 700 responds: `Hc1ActualFlowTempDesired` changes within seconds
- Baseline restore works on shutdown
- DHW hold logic is correct

But it also showed:
- Curve ping-ponged 0.55→0.10→1.00 in one overnight cycle
- 15-minute decisions against a 15-hour time constant = noise
- No model = no ability to predict or plan
- Leather rose 1.5°C above band from thermal lag despite aggressive coasting

## Implementation plan

### Phase 1: Core model integration

1. **`src/thermal/control.rs`** — new module:
   - `target_mwt_for_leather(outside_temp, target_leather, rooms, connections, doorways) → f64`
   - Bisection on the equilibrium solver (already exists in `display.rs`, extract the solver)
   - `curve_for_flow(target_flow, setpoint, outside_temp) → f64` (heat curve formula inverse)
   - `flow_for_curve(curve, setpoint, outside_temp) → f64` (heat curve formula forward)

2. **`src/bin/adaptive-heating-mvp.rs`** — replace V1 occupied-mode logic:
   - Import thermal model (rooms, connections, doorways from `thermal_geometry.json`)
   - On each decision: calculate target curve from model instead of ±0.10 bumps
   - Track `last_outside_temp_at_calc` and `last_leather_prediction` for recalculation triggers
   - Keep V1 safety logic: DHW hold, defrost hold, missing sensor hold, baseline restore

3. **Weather forecast client** — add to the binary:
   - Fetch Open-Meteo hourly forecast, cache for 1 hour
   - Use forecast outside temp (not current) for curve calculation when the difference is >1°C
   - Log forecast alongside decisions for later validation

### Phase 2: Overnight planner

4. **Forward simulation** — new function in `control.rs`:
   - `overnight_start_time(current_leather, forecast_temps, dhw_expected, target_leather, target_time) → DateTime`
   - Simulate cooling using `C × dT/dt = -HLC × (T_room - T_outside)` with hourly forecast temps
   - Simulate heating using radiator output at the calculated MWT
   - Account for DHW interruption (1h gap in heating)
   - Binary search on start time

5. **Overnight mode in the binary**:
   - At 23:00: run the planner, log the calculated start time
   - Until start time: set curve to minimum (or Z1OpMode off)
   - At start time: set curve for recovery

### Phase 3: Predictive DHW compensation

6. **Pre-DHW curve raise**:
   - 15 min before a predicted DHW charge, raise curve by enough to add ~0.3°C to Leather
   - After DHW finishes, recalculate and set the normal curve

### What's NOT in V2

- Direct SetModeOverride to HMU (bypass VRC 700) — future V2b
- Leather door sensor integration — waiting on hardware
- Aldora as fallback when Leather unavailable — needs proxy band data
- Per-room occupancy-driven control — needs TRVs
- Legionella risk monitoring — future

## Safety

- VRC 700 timers remain as safety net (19°C night, 21°C day from 04:00)
- Baseline restore on shutdown/kill (0.55 curve, 21°C day, 19°C night)
- If forecast fetch fails: use current outside temp (degrades to V1-like but with model)
- If equilibrium solver fails: fall back to V1 bang-bang logic
- If eBUS reads fail: hold, don't write
- Curve bounds: trust VRC 700 accepted range (0.10–4.00), but log a warning if model requests >1.50 (unusual)
