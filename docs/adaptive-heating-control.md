# Adaptive Heating Control

Date: 31 March 2026

## Core idea

The VRC 700 controls the heat pump using a single straight line (weather compensation curve) and no room feedback. We have 20+ sensors it can't see, a thermal model it doesn't have, and eBUS read/write access to override its settings. Every adjustment we make is an experiment — the house responds, we measure the response, and the empirical dataset grows.

The house is a physical system that's difficult to model precisely. But we don't need a perfect model — we need a controller that makes small adjustments, observes the result, and learns which direction is better. The thermal model provides the starting point and safety bounds. The real data provides the truth.

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
| Time of day / tariff period | Clock | — |
| DHW schedule | Known (05:30, 13:00, 22:00) | — |

### HP state (what the heat pump is doing)

| Metric | Source | What it tells us |
|---|---|---|
| `CurrentCompressorUtil` | eBUS | How hard the HP is working (0–100%) |
| `RunDataCompressorSpeed` | eBUS | Compressor RPM |
| `RunDataFlowTemp` | eBUS | Actual water temperature out |
| `RunDataReturnTemp` | eBUS | Water temperature back |
| `CurrentYieldPower` | eBUS | Heat being delivered (kW) |
| `RunDataElectricPowerConsumption` | eBUS | Electricity being consumed (W) |
| `RunDataStatuscode` | eBUS | HP state (heating/DHW/defrost/standby) |
| `BuildingCircuitFlow` | eBUS | Flow rate (L/h) — distinguishes heating/DHW |
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

One lever: the VRC 700 heat curve (`Hc1HeatCurve`). This changes the flow temperature the VRC 700 requests for a given outside temp. The HP, circulation pump, and radiators respond automatically.

The setpoint (`Z1DayTemp`) could also be adjusted but this changes *what temperature the VRC 700 is aiming for*, not *how efficiently it gets there*. Keep it at 21°C. The curve is the efficiency lever.

## The house as a laboratory

Every time we change the curve, the house runs an experiment:

1. **Stimulus:** Curve changes from 0.55 to 0.50
2. **HP response:** Flow temp drops ~2°C, compressor slows, COP changes
3. **House response:** Room temps drift over the next 1–2 hours (τ = 26h)
4. **Measurement:** 13 room temps, COP, compressor state, cycling frequency

Because the house has a 26-hour time constant, each experiment takes 1–2 hours to show its effect in room temperatures. We can run ~10 experiments per day. Over a heating season, that's ~2,000 data points mapping `(outside_temp, curve, door_states, occupancy) → (room_temps, COP, cycling)`.

### What each sensor adds

**Door sensors** (future: Zigbee contact sensors on internal doors):
- Open door = two rooms thermally coupled via doorway exchange
- Closed door = rooms are independent thermal zones
- The thermal model already has doorway exchange physics (buoyancy-driven, calibrated Cd=0.20)
- Real-time door state tells us which rooms to treat as a single zone
- Example: hall door open → hall coupled to front room → hall is warmer than the model predicts if door were closed

**Occupancy sensors** (2 existing Aqara motion on hall + landing, more possible):
- Empty room = no body heat (70–100W per person), don't optimise for it
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

## Control architecture

```
┌─────────────────────────────────────────────────┐
│              hp-curve-controller                  │
│                                                   │
│  Inputs:                                          │
│    eBUS (TCP :8888) → outside temp, HP state      │
│    MQTT (Mosquitto) → room temps, door, motion    │
│                                                   │
│  Logic:                                           │
│    Layer 1: Comfort guard (hard constraints)       │
│      - Any heated room < 18°C → raise curve       │
│      - Compressor > 90% → hold (HP at capacity)   │
│      - DHW active or setback → don't adjust        │
│                                                   │
│    Layer 2: COP optimisation (gradient-following)  │
│      - Track COP as curve changes                  │
│      - Step toward better COP                      │
│      - Stop when rooms cool or COP plateaus        │
│                                                   │
│    Layer 3: Context (future)                       │
│      - Door states → adjust room coupling model    │
│      - Occupancy → weight room priorities          │
│      - Weather forecast → anticipate, don't react  │
│                                                   │
│  Output:                                          │
│    eBUS write → Hc1HeatCurve (when value changes) │
│    InfluxDB → log every decision for analysis      │
│                                                   │
│  Cadence: every 15 min                            │
│  Rate limit: max 0.05 change per cycle            │
│  Bounds: curve 0.35–0.60                          │
└─────────────────────────────────────────────────┘
```

## What we build first

**Phase 1: Observer only** (no writes, just logging)
- Read all inputs every 15 min
- Compute what the controller *would* do
- Log to InfluxDB: proposed curve, reason, current COP, room temps
- Run for 2 weeks, validate logic against actual conditions
- Zero risk — doesn't touch the VRC 700

**Phase 2: Basic curve control**
- Layer 1 (comfort guard) + Layer 2 (COP gradient-following)
- Write `Hc1HeatCurve` when adjustment needed
- Log every change with before/after COP, room temps, outside temp
- Each write is an experiment — analyse results to tune thresholds

**Phase 3: Context-aware**
- Add door sensors, use occupancy data
- Weight room priorities by occupancy
- Add weather forecast (don't raise curve on a cold morning if warming by noon)
- Pre-DHW banking experiments

## Relationship to thermal model

The thermal model provides:
- **Starting point:** Equilibrium solver gives the MWT where rooms are comfortable. This initialises the curve target.
- **Safety bounds:** The model knows which rooms go cold first at each MWT. The comfort guard uses this.
- **Experiment design:** The model predicts what *should* happen when we change the curve. The actual measurements tell us what *did* happen. The difference is the model error — which shrinks with each experiment.

The controller doesn't need the model to run — it can gradient-follow purely from observations. But the model makes it converge faster (better initial guess) and safer (knows when to stop before a room gets cold).

## What this doesn't fix

- **Hall/bathroom/office at Tout < 5°C:** Fabric losses exceed rad capacity. Only EWI fixes this.
- **Elvina trickle vents:** Need to be physically closed. Not a control problem.
- **DHW efficiency:** Determined by cylinder and HP DHW mode (eco/normal). Not affected by space heating curve.
- **Defrost:** Controlled autonomously by the HMU. We can predict it, not prevent it.

The adaptive controller optimises within the current physical constraints. EWI changes the constraints. Both are worth doing — the controller first (free, immediate, works better after EWI too).
