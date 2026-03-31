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
| Door states | Zigbee door sensors (future; Leather first) | instant |
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

Beyond these two main levers, the VRC 700 exposes other writable inputs that may be useful experimentally: `HwcSFMode`, `HwcTempDesired`, `HwcOpMode`, `Hc1MaxFlowTempDesired`, `Hc1MinFlowTempDesired`, `Z1OpMode`, timers, holiday periods, and selected installer-policy settings. Their intended Vaillant purpose is irrelevant. What matters is whether writing them causes the VRC 700 to emit different downstream commands and whether those commands improve house-level outcomes.

## Outcomes first, registers second

The control problem is not "which registers should we write?" It is "what outcomes do we want, and which VRC 700 inputs let us get them?"

### Top-level objectives

1. **When occupied: optimise for comfort**
   - Occupied rooms comfortable
   - DHW available when expected
   - No critical room below safety floor
   - Efficiency matters, but only after comfort is secure

2. **When unoccupied: optimise for cost**
   - Minimise electricity cost
   - Maintain protection against frost / damp / silly deep cooldown
   - Respect known return time and warm-up requirement
   - Avoid strategies that create unnecessary cycling or equipment stress

This turns occupancy into the top-level mode selector. The same outside temperature can imply very different control actions depending on whether the house is occupied, empty for two hours, or empty until tomorrow evening.

### Primary comfort targets in this house

The controller should not treat all rooms equally.

- **Primary target room: Leather room** — measured by the emonth2. This is the most important comfort reference and should be treated as the main room-level target when the room is thermally independent.
- **Secondary target room: Aldora** — this should act as the second comfort anchor, especially when Leather is satisfied but the rest of the occupied house is not.
- **All other rooms** modify the control decision rather than define it. They matter as constraints, context, and evidence of distribution problems.
- **Conservatory** should be treated as a **heat sink / boundary room**, not a room to optimise for directly. It largely follows outdoor conditions, strongly modified by solar gain, and can distort whole-house optimisation if treated as a normal target room.

This suggests a weighted room strategy rather than a single whole-house average: Leather highest weight, Aldora next, the rest as guardrails and context.

### Door-state dependency for Leather

Leather should only be used as a primary optimisation target when its doors are closed enough for the room to behave as an independent zone.

- If Leather doors are **closed**, optimise normally for Leather comfort.
- If Leather doors are **open**, do **not** over-optimise the whole heating strategy for Leather alone, because its temperature is then being strongly influenced by adjacent spaces.
- Planned Zigbee door sensors on the Leather room should feed directly into the room-weighting logic.

Until those door sensors are installed, any Leather-led optimisation should be treated cautiously and cross-checked against Aldora and nearby-room behaviour.

### DHW hygiene is a monitored constraint, not a constant setpoint

Legionella control should not mean "always hold the cylinder at a high temperature". The better strategy is:

- monitor DHW turnover from the Multical volume data
- monitor cylinder temperature history (`HwcStorageTemp`, T1/T2)
- track time since last sufficiently hot hygiene cycle
- treat legionella as a **risk signal** that is usually low in normal occupied use
- trigger an explicit hygiene cycle only when low turnover / stagnation makes it necessary

In practice this means DHW control has two separate goals:
- **service**: enough hot water at the right times
- **hygiene**: occasional targeted anti-legionella intervention when risk rises

The adaptive controller should optimise service and cost most of the time, while continuously monitoring hygiene risk in the background.

## The house as a laboratory

Every time we change the curve, the house runs an experiment:

1. **Stimulus:** Curve changes from 0.55 to 0.50
2. **HP response:** Flow temp drops ~2°C, compressor slows, COP changes
3. **House response:** Room temps drift over the next 1-2 hours (τ = 26h)
4. **Measurement:** 13 room temps, COP, compressor state, cycling frequency

Because the house has a 26-hour time constant, each experiment takes 1-2 hours to show its effect in room temperatures. We can run ~10 experiments per day. Over a heating season, that's ~2,000 data points mapping `(outside_temp, curve, door_states, occupancy) → (room_temps, COP, cycling)`.

### What each sensor adds

**Door sensors** (future: Zigbee contact sensors on internal doors, Leather first):
- Open door = two rooms thermally coupled via doorway exchange
- Closed door = rooms are independent thermal zones
- The thermal model already has doorway exchange physics (buoyancy-driven, calibrated Cd=0.20)
- Real-time door state tells us which rooms to treat as a single zone
- Immediate practical use: avoid over-optimising for Leather when its doors are open
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
│    DHW history      → turnover, hygiene-risk monitor         │
│                                                              │
│  Layer 0: Mode selection                                     │
│    - Occupied → comfort-first targets                        │
│    - Short absence → mild setback / cost bias                │
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
│    - DHW hygiene risk → schedule targeted hot cycle if due   │
│                                                              │
│  Outputs:                                                    │
│    eBUS write → Hc1HeatCurve + Z1DayTemp (when changed)      │
│    InfluxDB  → log every decision + before/after metrics     │
│                                                              │
│  Cadence: every 15 min (reads every 1 min for averaging)     │
│  Rate limit: min 0.10 curve step per cycle                   │
│  Bounds: trust 700 accepted range (no extra software limits) │
│  Safety net: VRC 700 timers + baseline restore on stop/kill  │
└───────────────────────────────────────────────────────────────┘
```

## What we build

**Phase 1: Effect map of the VRC 700 as a steerable state machine**
- Treat every potentially useful writable register as fair game
- For each register: confirm writeability, readback, effect on VRC 700 state, effect on downstream eBUS messages, effect on plant behaviour
- Focus first on `Hc1HeatCurve`, `Z1DayTemp`, `Z1NightTemp`, `Z1QuickVetoTemp`, `Z1OpMode`, `HwcSFMode`, `HwcTempDesired`, `HwcOpMode`, `Hc1MaxFlowTempDesired`, `Hc1MinFlowTempDesired`
- Build an empirical table: `register → downstream SetMode change → HP/house effect`
- Use small reversible writes and restore after each experiment
- **Status:** largely completed enough to begin the live pilot. Many key writable levers have now been confirmed by direct write + readback on the real VRC 700, and `Hc1HeatCurve` has been shown to change `Hc1ActualFlowTempDesired` on the live system.

**Phase 2: Curve + setpoint control**
- Layers 0–2: occupancy mode, away mode, comfort guard, COP gradient-following
- Writes `Hc1HeatCurve` and `Z1DayTemp`/`Z1NightTemp` every 15 min when adjustment needed
- Tariff-aware: bank heat during Cosy windows, coast during expensive periods
- Away mode: API endpoint or config to set return time → automatic warm-up ramp
- Log every decision with before/after COP, room temps, outside temp to InfluxDB
- Every write is an experiment — the dataset grows with each cycle
- **Status:** MVP implemented in Rust, installed on `pi5data`, and running as a systemd service. See `docs/adaptive-heating-mvp.md` for the frozen MVP spec and current deployment details.

**Phase 3: Context and DHW policy**
- Door sensors → know which rooms are coupled
- Leather door sensors first → gate whether Leather is allowed to dominate optimisation
- Occupancy from motion sensors → prioritise occupied rooms
- Weather forecast → anticipate conditions, don’t just react
- Pre-DHW banking when compressor has headroom
- Add DHW turnover / hygiene-risk monitoring with targeted hygiene cycles only when due
- **Status:** still outstanding. This remains the next major refinement layer after the initial pilot has produced useful live data.

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
5. **DHW policy during absence:** minimise DHW maintenance, but continue monitoring turnover and temperature history. If low use / stagnation means hygiene risk rises, schedule a targeted high-temperature hygiene cycle rather than holding DHW hot continuously.
6. **Cost during absence:** At Tout 7°C, maintaining 15°C costs ~£0.50/day vs ~£2.50/day at 21°C. A week away saves ~£14.

The warm-up ramp timing comes from the calibrated cooling model (k=0.039/hr, capacity 6,723 Wh/°C). The controller doesn't guess — it computes the required lead time from the current house temperature and forecast outside temp.

## Experimental method: write, observe, classify

For each candidate VRC 700 register, the controller test harness should classify it in four stages:

1. **Write accepted** — returns `done`/`empty`
2. **Readback changed** — the register stores the new value
3. **VRC 700 state changed** — derived values or outbound control messages change
4. **Plant changed** — VWZ AI / HMU behaviour changes in a measurable way

The key observation is not just readback. It is what the VRC 700 then sends downstream to the indoor unit and heat pump. The VRC 700 behaves as a steerable state machine: we change its inputs, it recomputes its policy, and it emits repeating downstream control messages (especially `SetMode`) roughly every 10 seconds.

So the real effect map is:

`700 register write → VRC 700 internal state → downstream eBUS messages → VWZ AI / HMU response → house response`

This is what turns the register list into a practical control surface.

## Current project status

The project is no longer at the "should we build this?" stage.

It is now at the **live MVP pilot** stage:
- the adaptive-control strategy is documented
- the MVP scope has been frozen in `docs/adaptive-heating-mvp.md`
- a Rust service has been built and installed on `pi5data`
- baseline restore is implemented and verified
- mode control API is live
- local JSONL + InfluxDB logging are enabled

The remaining work is primarily:
- integrate mobile controls into `z2m-hub`
- verify and inspect live logs
- derive Aldora fallback behaviour from historical data
- add the next context layers only after observing real pilot results

## Relationship to thermal model

The thermal model provides:
- **Starting point:** Equilibrium solver gives the MWT where rooms are comfortable. This initialises the curve target.
- **Safety bounds:** The model knows which rooms go cold first at each MWT. The comfort guard uses this.
- **Experiment design:** The model predicts what *should* happen when we change the curve. The actual measurements tell us what *did* happen. The difference is the model error - which shrinks with each experiment.

The controller doesn't need the model to run - it can gradient-follow purely from observations. But the model makes it converge faster (better initial guess) and safer (knows when to stop before a room gets cold).

## What this doesn't fix

- **Hall/bathroom/office at Tout < 5°C:** Fabric losses exceed rad capacity. Only EWI fixes this.
- **Elvina trickle vents:** Need to be physically closed. Not a control problem.
- **Conservatory:** Too coupled to outdoor conditions and solar gain to be a normal comfort target. Treat it as a boundary / sink, not a room to optimise for directly.
- **DHW efficiency:** Determined by cylinder physics, DHW timing, target temperature, and HP operating mode. The adaptive controller can improve timing and targeting, but it cannot change the physical cylinder or refrigerant-side DHW limits.
- **Defrost:** Controlled autonomously by the HMU. We can predict it, not prevent it.

The adaptive controller optimises within the current physical constraints. EWI changes the constraints. Both are worth doing - the controller first (free, immediate, works better after EWI too).
