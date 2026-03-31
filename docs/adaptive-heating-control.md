# Adaptive Heating Control

Date: 31 March 2026

## Core idea

The VRC 700 controls the heat pump using a single straight line (weather compensation curve) and no room feedback. We have 20+ sensors it can't see, a thermal model it doesn't have, and eBUS read/write access to override its settings. Every adjustment we make is an experiment - the house responds, we measure the response, and the empirical dataset grows.

The house is a physical system that's difficult to model precisely. But we don't need a perfect model - we need a controller that makes small adjustments, observes the result, and learns which direction is better. The thermal model provides the starting point and safety bounds. The real data provides the truth.

## What we can observe

### Inputs (things that drive the system)

| Input | Source | Update rate |
|---|---|---|
| Outside temperature | eBUS `Broadcast/Outsidetemp` | 30s |
| Outside humidity | Open-Meteo API | 1h |
| Weather forecast (next 12h) | Open-Meteo API | 1h |
| Room temperatures (13 rooms) | Zigbee SNZB-02P + emonth2 | ~5 min |
| Room humidity (12 rooms) | Zigbee SNZB-02P | ~5 min |
| Door states | Zigbee door sensors (future) | instant |
| Occupancy | Zigbee motion sensors (2 existing + future) | instant |
| Time of day / tariff period | Clock | - |
| DHW schedule | Known (05:30, 13:00, 22:00) | - |

### HP state (what the heat pump is doing)

| Metric | Source | What it tells us |
|---|---|---|
| `CurrentCompressorUtil` | eBUS | How hard the HP is working (0-100%) |
| `RunDataCompressorSpeed` | eBUS | Compressor RPM |
| `RunDataFlowTemp` | eBUS | Actual water temperature out |
| `RunDataReturnTemp` | eBUS | Water temperature back |
| `CurrentYieldPower` | eBUS | Heat being delivered (kW) |
| `RunDataElectricPowerConsumption` | eBUS | Electricity being consumed (W) |
| `RunDataStatuscode` | eBUS | HP state (heating/DHW/defrost/standby) |
| `BuildingCircuitFlow` | eBUS | Flow rate (L/h) - distinguishes heating/DHW |
| `Hc1ActualFlowTempDesired` | eBUS | What the VRC 700 is requesting |

### Derived (calculated from observations)

| Metric | Formula | What it tells us |
|---|---|---|
| Instantaneous COP | `YieldPower × 1000 / ElecConsumption` | Current efficiency |
| Heat per elec watt | Same as COP | The thing to maximise |
| Room cooling rate | `ΔT_room / Δt` when HP off | Per-room thermal loss |
| Room heating rate | `ΔT_room / Δt` when HP on | Per-room heat delivery |
| Cycling frequency | Compressor on/off transitions per hour | Indicates oversupply |
| Flow-return DT | `FlowTemp - ReturnTemp` | Heat being extracted by rads |
| Desired vs actual flow gap | `FlowTempDesired - FlowTemp` | Whether HP can deliver what's asked |

## What we can control

Two levers, both writable via eBUS:

| Lever | Register | Effect |
|---|---|---|
| `Hc1HeatCurve` | Weather comp gradient | Changes flow temp for given outside temp - the efficiency lever |
| `Z1DayTemp` | Room setpoint | Changes what temperature the VRC 700 targets - the demand lever |

The VRC 700 day/night timer and fixed setpoints become unnecessary. The controller writes both every 15 minutes based on current conditions, replacing the fixed straight-line with a continuously-adapted operating point.

`Z1NightTemp` and the VRC 700 timers stay as a safety net in case the controller stops. Set to 19°C setback, current timer schedule. The controller overrides `Z1DayTemp` (which the VRC 700 uses whenever it's in "day" mode) to effectively control 24/7.

## The house as a laboratory

Every time we change the curve, the house runs an experiment:

1. **Stimulus:** Curve changes from 0.55 to 0.50
2. **HP response:** Flow temp drops ~2°C, compressor slows, COP changes
3. **House response:** Room temps drift over the next 1-2 hours (τ = 26h)
4. **Measurement:** 13 room temps, COP, compressor state, cycling frequency

Because the house has a 26-hour time constant, each experiment takes 1-2 hours to show its effect in room temperatures. We can run ~10 experiments per day. Over a heating season, that's ~2,000 data points mapping `(outside_temp, curve, door_states, occupancy) → (room_temps, COP, cycling)`.

### What each sensor adds

**Door sensors** (future: Zigbee contact sensors on internal doors):
- Open door = two rooms thermally coupled via doorway exchange
- Closed door = rooms are independent thermal zones
- The thermal model already has doorway exchange physics (buoyancy-driven, calibrated Cd=0.20)
- Real-time door state tells us which rooms to treat as a single zone
- Example: hall door open → hall coupled to front room → hall is warmer than the model predicts if door were closed

**Occupancy sensors** (2 existing Aqara motion on hall + landing, more possible):
- Empty room = no body heat (70-100W per person), don't optimise for it
- Occupied room = comfort matters, prioritise in control decisions
- Example: nobody in elvina → let it drift to 17°C → don't raise curve to warm it → save COP

**Room humidity** (already have from SNZB-02P):
- High humidity in occupied room = condensation risk → don't let room go too cold
- Rising humidity overnight = ventilation rate cross-check
- Already analysed in `thermal-moisture` command

### Example experiments

**Experiment 1: Mild day curve sweep**
- Conditions: Tout 10°C, all rooms > 20°C, compressor cycling
- Action: Drop curve from 0.55 to 0.50, wait 1h, measure COP and room temps
- If COP improved and rooms still > 19°C: drop to 0.45, wait 1h, measure again
- Keep going until COP stops improving or a room hits 18°C
- Result: optimal curve for Tout 10°C with current door states

**Experiment 2: Door effect on hall**
- Conditions: Hall at 18°C, front room at 20°C, door state unknown
- Observe: If door sensor shows hall→front open, hall should be warmer than model predicts
- If hall is cold despite open door: the model's doorway exchange coefficient may be wrong, or there's a draught source
- Result: refined doorway Cd for hall→front

**Experiment 3: Pre-DHW banking**
- Conditions: Tout 5°C, compressor at 60%, DHW scheduled in 30 min
- Action: Raise curve by 0.05 for 30 min to bank 0.5°C
- Measure: Room temp at DHW start vs typical, room temp at DHW end vs typical
- Result: Is pre-heating measurably better than catch-up recovery?

**Experiment 4: Occupancy-driven setback**
- Conditions: Motion sensors show no activity upstairs for 2h
- Action: (Future) If per-room TRV control existed, reduce upstairs setpoint
- Without TRVs: Can only observe the effect, not act on it per-room
- But: If entire house is unoccupied (no motion anywhere for 1h), could drop curve
- Result: Energy saved vs comfort penalty when occupants return

**Experiment 5: Tariff-aware overnight**
- Conditions: 01:00, Tout 10°C, house at 20.5°C, Cosy window ended at 00:00
- Action: Drop curve to 0.35 and setpoint to 19°C - HP idles at minimum mod
- At 04:00 (Cosy starts): raise setpoint to 21°C, curve to 0.45 - cheap recovery
- Result: Minimal electricity during expensive 00:00-04:00, fast recovery at Cosy rate

**Experiment 6: Away mode with known return**
- Conditions: House empty, return in 48h, Tout forecast 5-10°C
- Action: Drop setpoint to 15°C, curve to 0.30 - frost protection only
- At return minus 6h: raise to 19°C/0.45. At return minus 2h: raise to 21°C/0.55
- The thermal model predicts warm-up time: at Tout 7°C, house gains ~0.3°C/hr from 15°C
  with HP at full output - needs ~20h from 15°C to 21°C. So 6h lead time reaches ~17°C,
  then 2h at higher curve gets to 19°C. Not 21°C on arrival, but not cold.
- With forecast: if cold snap during absence, start warm-up earlier
- Result: Days of savings (~£2-4/day in winter) with acceptable arrival comfort

## Control architecture

```
┌───────────────────────────────────────────────────────────────┐
│              hp-curve-controller                             │
│                                                              │
│  Inputs:                                                     │
│    eBUS (TCP :8888) → outside temp, HP state, compressor     │
│    MQTT (Mosquitto) → room temps, door sensors, motion       │
│    Open-Meteo      → weather forecast, humidity              │
│    Config/API      → away schedule, tariff periods           │
│                                                              │
│  Layer 0: Mode selection                                     │
│    - Away mode (house empty, known return time)              │
│      → setpoint 15°C, curve 0.30 until warm-up ramp          │
│    - Normal mode → layers 1-3                                │
│                                                              │
│  Layer 1: Comfort guard (hard constraints)                   │
│    - Any heated room < 18°C → raise curve                    │
│    - Compressor > 90% → hold (HP at capacity)                │
│    - DHW active → don't adjust                               │
│                                                              │
│  Layer 2: COP optimisation (gradient-following)              │
│    - Track COP as curve/setpoint change                      │
│    - Step toward better COP                                  │
│    - Stop when rooms cool or COP plateaus                    │
│                                                              │
│  Layer 3: Context                                            │
│    - Tariff → bank heat during Cosy, coast during expensive  │
│    - Door states → adjust room coupling expectations         │
│    - Occupancy → weight room priorities                      │
│    - Forecast → anticipate, don't react                      │
│                                                              │
│  Outputs:                                                    │
│    eBUS write → Hc1HeatCurve + Z1DayTemp (when changed)      │
│    InfluxDB  → log every decision + before/after metrics     │
│                                                              │
│  Cadence: every 15 min (reads every 1 min for averaging)     │
│  Rate limit: max 0.05 curve / 1°C setpoint per cycle         │
│  Bounds: curve 0.30-0.60, setpoint 15-22°C                   │
│  Safety net: VRC 700 timers + Z1NightTemp 19°C unchanged     │
└───────────────────────────────────────────────────────────────┘
```

## What we build

**Phase 1: Curve + setpoint control (build now)**
- Layers 0–2: away mode, comfort guard, COP gradient-following
- Writes `Hc1HeatCurve` and `Z1DayTemp` every 15 min when adjustment needed
- Tariff-aware: bank heat during Cosy windows, coast during expensive periods
- Away mode: API endpoint or config to set return time → automatic warm-up ramp
- Log every decision with before/after COP, room temps, outside temp to InfluxDB
- Every write is an experiment — the dataset grows with each cycle

**Phase 2: Context (add when sensors arrive)**
- Door sensors → know which rooms are coupled
- Occupancy from motion sensors → prioritise occupied rooms
- Weather forecast → anticipate conditions, don’t just react
- Pre-DHW banking when compressor has headroom

## 24-hour operation

The VRC 700 day/night timer and fixed setback become redundant. The controller owns the setpoint 24/7:

| Period | Current (VRC 700 timers) | Adaptive controller |
|---|---|---|
| 00:00–04:00 | Fixed 19°C setback, curve 0.55 | Setpoint 19–21°C based on room temps. Curve lowered to minimise cost during expensive tariff. HP idles at min mod instead of cycling. |
| 04:00–07:00 (Cosy) | Fixed 21°C, curve 0.55 | Setpoint 21°C, curve raised if rooms need recovery. Cheap electricity — bank heat if possible. |
| 07:00–13:00 | Fixed 21°C, curve 0.55 | COP optimise — lower curve while rooms hold. Expensive electricity — coast on thermal mass where possible. |
| 13:00–16:00 (Cosy) | Fixed 21°C, curve 0.55 | Cheap electricity — if rooms drifted during 07–13, recover now cheaply. |
| 16:00–19:00 (Peak) | Fixed 21°C, curve 0.55 | Most expensive period. Lower curve as far as comfort allows. Coast on afternoon Cosy banking. |
| 19:00–22:00 | Fixed 21°C, curve 0.55 | Evening — occupied rooms matter. Comfort guard active. |
| 22:00–00:00 (Cosy) | Fixed 21°C, curve 0.55 | Cheap electricity. Bank heat before overnight — raise setpoint to 21.5°C or curve to 0.55 to enter the night warm. |

The VRC 700 timer stays programmed as a safety net (`Z1NightTemp = 19°C`, day mode from 04:00). If the controller dies, the VRC 700 falls back to the current behaviour. The controller overrides by writing `Z1DayTemp` — which takes effect whenever the VRC 700 is in day mode (04:00–00:00 per the timer).

For the 00:00–04:00 setback period, the controller writes `Z1NightTemp` instead. This is the only time the VRC 700 is in night mode.

## Away mode

When the house is empty for an extended period:

1. **Trigger:** API endpoint on z2m-hub (`/api/away?return=2026-04-05T18:00`) or manual config
2. **Immediate:** Drop setpoint to 15°C, curve to 0.30. Frost protection only. HP barely runs.
3. **Warm-up ramp:** Based on thermal model prediction of warm-up time:
   - House gains ~0.3°C/hr from 15°C with HP at full output (Tout dependent)
   - At Tout 7°C: 15→21°C takes ~20 hours at full power
   - Controller starts ramp `warm_up_hours` before return time
   - Ramp: 15→18°C (curve 0.45), then 18→21°C (curve 0.55)
4. **Forecast adjustment:** If cold snap during absence, start ramp earlier. If mild, later.
5. **Cost during absence:** At Tout 7°C, maintaining 15°C costs ~£0.50/day vs ~£2.50/day at 21°C. A week away saves ~£14.

The warm-up ramp timing comes from the calibrated cooling model (k=0.039/hr, capacity 6,723 Wh/°C). The controller doesn't guess — it computes the required lead time from the current house temperature and forecast outside temp.

## Relationship to thermal model

The thermal model provides:
- **Starting point:** Equilibrium solver gives the MWT where rooms are comfortable. This initialises the curve target.
- **Safety bounds:** The model knows which rooms go cold first at each MWT. The comfort guard uses this.
- **Experiment design:** The model predicts what *should* happen when we change the curve. The actual measurements tell us what *did* happen. The difference is the model error - which shrinks with each experiment.

The controller doesn't need the model to run - it can gradient-follow purely from observations. But the model makes it converge faster (better initial guess) and safer (knows when to stop before a room gets cold).

## What this doesn't fix

- **Hall/bathroom/office at Tout < 5°C:** Fabric losses exceed rad capacity. Only EWI fixes this.
- **Elvina trickle vents:** Need to be physically closed. Not a control problem.
- **DHW efficiency:** Determined by cylinder and HP DHW mode (eco/normal). Not affected by space heating curve.
- **Defrost:** Controlled autonomously by the HMU. We can predict it, not prevent it.

The adaptive controller optimises within the current physical constraints. EWI changes the constraints. Both are worth doing - the controller first (free, immediate, works better after EWI too).
