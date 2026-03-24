# Room-by-Room Thermal Model

## Purpose

Calibrate a thermal network model of the house using room temperature sensors, then use it to:

1. Calculate actual radiator flow distribution (22mm vs 15mm branches)
2. Determine FRV settings for system rebalancing
3. Identify whether kitchen needs its own radiator
4. Predict the effect of fabric improvements (SWI, glazing, draught-proofing)

## What We Know

### Fabric (from spreadsheet measurements)

U-values and areas for every element of every room — external walls, internal walls, floors, ceilings, windows, roof. These are physical measurements, not estimates. The spreadsheet `Heating needs for the house.xlsx` has the full breakdown.

Key external UA products (W/K — watts lost per degree of temperature difference to outside):

| Room | External UA (W/K) | Dominant element |
|---|---|---|
| Conservatory | 75.2 | Glazed roof (U=2.4, 21m²) |
| Hall | 43.1 | Solid brick (U=2.11, 16.8m²) + loft stairwell |
| Bathroom | 33.5 | Solid brick (U=2.11, 10.9m²) × 2 walls |
| Jack & Carol | 28.5 | Solid brick + bay window |
| Front | 25.8 | Solid brick + bay window |
| Sterling | 23.4 | Solid brick + triple glazed window |
| Office | 23.7 | Solid brick |
| Kitchen | 21.6 | Solid brick + ground floor |
| Elvina | 14.5 | Insulated walls (U=0.15) |
| Aldora | 9.2 | Insulated walls (U=0.15) |
| Shower | 4.9 | Insulated walls (U=0.15) |
| Leather | 0.0 | No external elements |

### Radiators (from physical inventory)

15 Stelrad radiators, exact models and T50 ratings. No TRVs. All valves wide open except Sterling (manually off).

### Pipe Topology

The heating circuit has two distinct pipe configurations:

**22mm primary with short 15mm tails** — most radiators:
- Leather ×2, Conservatory ×2, Front vertical, Bathroom ×2, Sterling, Elvina, Aldora, Shower

**4m of 15mm pipe, two radiators sharing each branch:**
- **Branch 1**: Front horizontal (DP DF 2425W) + Hall (DP DF 2376W)
- **Branch 2**: Jack & Carol (DP DF 1950W) + Office (DP SF 1345W)

15mm pipe has roughly 4× the hydraulic resistance of 22mm per metre. The 15mm branch radiators are flow-starved relative to the 22mm radiators — this is the primary cause of uneven room temperatures.

### Sensors

11 temperature/humidity sensors covering all rooms except Office and Landing. Data flows to InfluxDB at ~5 minute intervals. See AGENTS.md for the full sensor inventory.

### HP State

Total heat output, flow/return temperatures, flow rate, and operating mode (heating/DHW/idle) from emonhp heat meter and eBUS. Outside temperature from the Arotherm OAT sensor (eBUS, every 30s).

## What We Don't Know (and need to fit)

### 1. Thermal Mass per Room (C, kJ/°C)

How much energy it takes to change a room's temperature by 1°C. Determined by the mass of walls, floor, furniture, and air. Solid brick rooms have high thermal mass (slow to heat, slow to cool). Loft rooms with lightweight construction have low thermal mass.

**How to measure**: From the cooldown rate when heating is off. C = Q_loss / (dT/dt). We measure dT/dt directly from the sensors, and Q_loss from the fabric model + measured temperatures.

### 2. Ventilation Rate per Room (ACH, air changes/hour)

Air infiltration through window frames, door gaps, brick porosity, and deliberate ventilation through open doors. This is the biggest unknown in any house model. Typical range: 0.1 ACH (sealed modern room) to 1.0+ ACH (leaky room with open door to outside).

**How to measure**: See "Calibration Rooms" below.

### 3. Inter-Room Air Exchange (through open doors)

Warm air flows through open doorways by convection — warm air out at the top, cold air in at the bottom. The rate depends on the temperature difference and door geometry. Typically 50-100 m³/h through a standard open doorway.

**How to measure**: See "Calibration Rooms" below.

### 4. Effective Radiator Heat Output (and thus flow distribution)

Without FRVs, flow distributes by hydraulic resistance. Radiators on 22mm get more flow than those on 15mm branches. The actual heat output of each radiator depends on the flow it receives.

**How to measure**: From the warmup rate when heating resumes after a DHW cycle.

## The Daily Experiment

Every day, the heating system runs a natural experiment with three distinct phases:

### Phase 1: Evening Setback (target drops to 19°C)

HP reduces output or stops. Rooms cool towards 19°C at rates determined by their losses and thermal mass. Radiators still have some warm water but HP output is reduced.

### Phase 2: Morning DHW Charge (HP diverts to cylinder)

This is the key phase. The diverter valve sends all flow to the DHW cylinder. The radiators contain **standing water** at whatever temperature they were last heated to. This water cools in place — the radiators act as tiny heat stores releasing their remaining warmth into the rooms, then go cold.

During this phase, rooms are free-cooling with zero heating input (except the residual standing water, which dissipates within minutes). This gives the cleanest data:

- **dT/dt per room** = direct measurement of total heat loss rate
- **No radiator output to model** — removes the biggest uncertainty
- **All rooms cooling simultaneously** — same outside temp, same conditions
- **Duration**: typically 30-90 minutes (DHW charge time)

### Phase 3: Morning Warmup (space heating resumes)

HP switches back to space heating. Hot water reaches the radiators. The **rate each room warms up** reveals its actual radiator heat input:

- Rooms on 22mm primary warm first and fastest
- Rooms on 15mm branches warm slower
- The difference directly measures the flow restriction
- Kitchen (no rad) and Sterling (rad off) continue cooling — providing continuous calibration

## Calibration Rooms

Two rooms provide continuous calibration of the model's most uncertain parameters:

### Kitchen — Open Doorway Air Exchange Rate

Kitchen has no radiator and never will (for calibration purposes). It has:
- Open doorway to hall (sensor: 19.9°C)
- Open doorway to conservatory (sensor: 20.4°C)
- Shared wall with leather (sensor: 20.9°C, door closed)
- Bathroom above (sensor: 22.0°C)
- External wall + ground floor losing to outside (sensor: eBUS)

At steady state, heat in = heat out. We know all surfaces, U-values, and temperatures on both sides. The only unknowns are the two open doorway air exchange rates.

This directly solves for the **air exchange rate through a standard open doorway** — the single hardest parameter to estimate in any house model. This rate applies to every other open doorway: bathroom↔stairwell, shower↔stairwell, kitchen↔conservatory, front↔stairwell (partial).

Because kitchen has no radiator, **every data point is a calibration measurement** — not just during setback. Over a week of varying outside temperatures, the fit separates:
- Fabric loss (proportional to ΔT to outside)
- Inter-room transfer through walls (proportional to ΔT to neighbour)
- Inter-room transfer through doorways (proportional to ΔT to neighbour)

### Sterling — Closed Room Background Ventilation Rate

Sterling has its radiator turned off and its door always closed. It has:
- Leather below through carpet/timber/plaster floor (sensor: 20.9°C)
- One external solid brick wall
- One triple-glazed window (flat wall, single sealed unit)
- Internal walls to bathroom and Jack & Carol

At steady state, the only heat input is conduction from leather through the floor and from adjacent rooms through internal walls. The only losses are through the external wall, window, and ventilation.

Since we know the fabric U-values and measure all adjacent temperatures, the **residual is the ventilation rate** — infiltration through the window frame, door gaps, and brick porosity for a sealed, closed room.

This gives the **lower bound on ventilation** for the house. Every other room with a closed door (Elvina, Aldora, Leather) will have a similar or slightly higher rate depending on their construction. Rooms with open doors will have much higher rates (measured from kitchen).

### Together

Kitchen and Sterling bracket the ventilation range:
- **Sterling** = minimum (closed door, sealed window): probably 0.1-0.3 ACH
- **Kitchen** = maximum for an internal room (two open doorways): probably 0.5-1.5 ACH
- **Hall** = maximum for the house (front door, stairwell): probably higher still

Every other room falls between these calibration points based on its door state and construction.

## The Model

For each room i, energy balance:

```
C_i × dT_i/dt = Q_rad_i - Q_fabric_i - Q_vent_i + Σ Q_transfer_ij + Q_gains_i
```

Where:
- `Q_rad_i = T50_i × ((MWT - T_i) / 50)^1.3` — radiator output (W), zero if no rad or rad off
- `Q_fabric_i = Σ (U × A) × (T_i - T_outside)` — external fabric loss (W)
- `Q_vent_i = 0.33 × ACH_i × V_i × (T_i - T_outside)` — ventilation loss (W)
- `Q_transfer_ij` — inter-room heat transfer through walls and doorways (W)
- `Q_gains_i` — internal gains: people (~80W), appliances, solar

13 rooms → 13 coupled differential equations. Solved numerically at 5-minute timesteps against the measured data.

### Fitting Procedure

1. **Use DHW periods** (HP off for space heating) to fit thermal mass C_i per room from cooldown rates
2. **Use kitchen steady-state** to fit open doorway air exchange rate
3. **Use Sterling steady-state** to fit closed room ventilation rate
4. **Assign ventilation rates** to all rooms based on door state (open → kitchen-like, closed → Sterling-like)
5. **Use warmup periods** (after DHW) to fit effective radiator output per room
6. **Validate**: sum of fitted radiator outputs should match HP heat meter total
7. **Cross-validate**: predict room temperatures for a held-out day and compare to measured

### Constraints

- `Σ Q_rad_i = HP heat meter output` (measured)
- Radiators on the same 15mm branch share a fixed total flow (determined by branch resistance)
- Rooms without radiators: Q_rad = 0 (kitchen, landing)
- Rooms with rad off: Q_rad = 0 (Sterling)
- All radiators share the same flow temperature (measured)

## What the Model Tells Us

### Flow Distribution

The warmup rate of each room after DHW reveals its actual radiator heat input. Comparing this to the theoretical T50 output gives the flow fraction:

```
actual_output / theoretical_output = flow_effectiveness
```

A flow_effectiveness of 0.5 means the radiator is getting half the flow it needs to deliver its rated output. This directly maps to FRV settings.

### FRV Recommendations

FRVs go on the **22mm radiators only** — restricting them to their calculated need, which increases pressure available to the 15mm branches. The 15mm branch radiators stay wide open.

The model calculates:
- Current flow per radiator (from measured warmup rates)
- Ideal flow per radiator (from room heat loss requirements)
- FRV setting = ideal flow rate in L/min

### Kitchen Radiator Decision

The model quantifies how much heat kitchen currently receives from neighbours vs how much it loses. If the deficit is small (say <100W in cold weather), rebalancing flow to hall via FRVs might warm kitchen enough indirectly. If the deficit is large (>200W), kitchen needs its own radiator — and the model tells you what size.

### Fabric Improvement Priorities

Re-run the model with modified U-values to predict the effect of:
- Solid wall insulation (U=2.11 → 0.3): which rooms benefit most?
- Conservatory roof replacement (U=2.4 → 0.15): how much does it help?
- Front door draught-proofing: quantified from hall's fitted ventilation rate
- Triple glazing on bay windows: compare with Sterling's performance

## Implementation

Python model in `model/house.py`. Commands:

```bash
# Fetch data from InfluxDB (default 24h)
uv run --with influxdb-client --with numpy --with scipy python model/house.py fetch [hours]

# Show room summary (fabric UA, radiator T50, pipe type)
uv run --with influxdb-client --with numpy --with scipy python model/house.py rooms

# Steady-state energy balance (latest data point)
uv run --with influxdb-client --with numpy --with scipy python model/house.py analyse

# Fit thermal parameters from cooldown periods
uv run --with influxdb-client --with numpy --with scipy python model/house.py fit
```

## Data Requirements

- **Minimum**: one full setback → DHW → warmup cycle (one night)
- **Good**: one week of daily cycles across varying outside temperatures
- **Best**: data spanning a cold snap (0°C or below) for design condition validation

Current sensor coverage: 11 of 13 rooms. Missing: Office, Landing.

## First Overnight Results (23-24 March 2026)

### Temperature Cooldown (00:15→06:30, outside 10.7→9.6°C, HP cycling)

| Room | Drop | °C/h | Key finding |
|---|---|---|---|
| Conservatory | -1.9°C | 0.30 | Fastest. Glazed roof dominates after dark. |
| Shower | -1.6°C | 0.26 | Door open to stairwell — warm air leaks down. |
| Hall | -1.4°C | 0.22 | Dropped DESPITE HP cycling. Flow starvation confirmed. |
| Kitchen | -1.3°C | 0.21 | No rad, tracks hall through open doorway. |
| Elvina | -1.2°C | 0.19 | Faster than expected — sloping roof poorly insulated. |
| Front | -1.1°C | 0.18 | Bay window, partial door. |
| Bathroom | -1.1°C | 0.18 | MVHR extracting, door closed. |
| Jack & Carol | -0.9°C | 0.14 | Door closed, single rad cycling. |
| Aldora | -0.7°C | 0.11 | Occupied (+80W), excellent insulation (flat roof). |
| Sterling | -0.1°C | 0.02 | Rock solid. Leather floor heat maintains it. |

### Humidity Overnight (21:00→06:30)

The humidity data independently validates ventilation patterns:

| Room | RH change | People | Meaning |
|---|---|---|---|
| Aldora | +10% (51→61%) | 1 | Very sealed — moisture accumulates. Mould threshold reached. |
| Elvina | +6% (44→50%) | 1 | Leakier than Aldora despite same spec — sloping roof. |
| Jack & Carol | -0.4% (47→46.6%) | 2 | Bay window infiltration removes moisture faster than 2 people produce it. |
| Hall | -1.4% (49→47.5%) | 0 | Front door infiltration — cold dry air entering. |
| Kitchen | -1.5% (49→47.1%) | 0 | Same — open to hall, drying from infiltration. |
| Sterling | 0.0% (45.7→45.7%) | 0 | Zero ventilation, zero moisture exchange. Perfect sealed box. |
| Bathroom | -31% (83→52%) | 0 | MVHR extracting at 9 L/s. Took 5h through closed door. |

### Key Findings

1. **Elvina cools fast because of trickle vents, not poor insulation.** Despite 2010 spec (U=0.15/0.066), it cools faster than jackcarol (solid brick + bay window). Initially suspected poor insulation but trickle vents being open is the cause — humidity confirms significant air exchange (1 person only +6% RH in 61m³). The vents provide necessary moisture removal but cost heat. Closing them would reduce heat loss but create moisture risk (like Aldora).

2. **Aldora's flat roof works perfectly** but is too well sealed. Humidity reaches 61% overnight with one person — mould threshold. Needs trickle vent or door left ajar.

3. **Hall drops while HP is heating.** The flow-starved 15mm branch rad cannot match the hall's losses even when the HP is running. This is not just a setback issue.

4. **Kitchen and hall are thermally coupled.** Identical cooling rates (0.29°C/h during free-cooling). Kitchen's fate depends entirely on hall.

5. **Sterling is thermally decoupled from the heating system.** Leather's floor heat maintains it regardless of HP state.

6. **MVHR drives whole-house airflow.** Bathroom extracts 9 L/s, pulling air up from ground floor through the stairwell. Closing the bathroom door reduces this draft and saves ~51W across the house.

7. **Shower room loses heat down the stairwell at night.** Despite excellent insulation, the open door allows warm air to sink to cooler floors below. Closing the door at night would preserve temperature.

### Model Fit Assessment

The external-only loss predictions are directionally correct but thermal mass estimates are inflated 3-7× because:
1. Inter-room heat transfer not yet modelled (leather warms hall, kitchen, sterling through walls)
2. Occupancy gains (+80W/person) significant in small rooms (Aldora corrects to plausible with this)
3. Standing water in radiators provides residual heat at start of free-cooling periods
4. 0.1°C sensor resolution limits precision at 0.17°C/h cooling rates (only 1-2 steps per hour)

**Need cold snap data (2°C outside) to resolve.** Double the ΔT gives double the cooling rates, cleaner signal above sensor noise, and better separation between fabric and ventilation losses.

### Ventilation Groups (confirmed by humidity data)

| Group | ACH | Rooms | Evidence |
|---|---|---|---|
| MVHR (measured) | 0.75 (eff. 0.16) | Bathroom | 9 L/s, 78% heat recovery |
| Infiltration (high) | 0.5 | Hall | Front door + stairwell base. RH drops overnight. |
| Open doors + MVHR draft | 0.35 | Kitchen, Conservatory | Open to hall. RH drops overnight. |
| Leaky construction | 0.35 | Elvina, Jack & Carol | Bay window / sloping roof. RH stable or drops with occupants. |
| Partial door | 0.30 | Front | Bay window SE face. |
| Closed, slight leakage | 0.20 | Leather, Shower, Office | Closed doors, some infiltration. |
| Sealed modern | 0.10-0.15 | Aldora, Sterling | Flat roof / triple glazed. RH accumulates. |

## Conservatory Assessment

The conservatory is reported as **OK in winter, mostly** — usable as a dining room. The 2×K3 radiators (T50=5,700W combined, largest in the house) cope with the glazed roof (U=2.4) at typical winter temperatures. Replacing the glazed roof with solid insulated (£10-15k) is **not justified** given the long payback and adequate comfort.

The conservatory does cool fastest overnight (-1.9°C over 6 hours at 10°C outside), driven by the glazed roof. On the upcoming 2°C cold snap night it will drop further — the sensor data will show whether it becomes genuinely uncomfortable or remains acceptable.

## HP Capacity at Design Conditions

From emoncms data, Jan 2-4 2025 (outside avg 1.9°C):
- HP running **95% of time** (only 4.9% idle)
- Average heat output: 5,105W at COP 3.7
- Demand: HTC × ΔT = 261 × 19.1 = 4,993W → supply barely matches demand

At design (-2°C): demand = 261 × 23 = 6,003W. Arotherm capacity ~6,400W at 35°C flow. Only 397W headroom. HP is at its limit — hall goes cold because there is no spare capacity, not just flow starvation.

**Implication for FRVs**: At max capacity, FRVs redistribute a fixed total — hall gains but other rooms cool slightly. Net comfort improves but total demand is unchanged. FRVs alone can't fix the capacity shortfall.

## EWI Opportunity — One Wall, Three Rooms

One accessible SE-facing wall: **10m long × 5m high = 50m²** (ground + first floor).

Rooms on this wall:
- **Ground floor**: Hall external wall + Kitchen external wall (with their windows)
- **First floor**: 75% of Bathroom external wall + Office external wall

With 100mm EWI (U=2.11 → U=0.30):
- **84 W/K saved** = 32% of whole-house HTC from one wall
- At design (-2°C): **1,942W freed** → HP headroom goes from 397W to 2,339W
- HP stops maxing out in winter
- Hall rad (2376W T50) becomes **borderline adequate** at design flow temps (~530W at MWT 34°C vs ~625W demand)
- Without EWI, hall rad is hopeless — losses exceed max output regardless of flow
- Cost: under £5k DIY, one scaffold, one job
- Cascade effect: EWI → bathroom warmer → kitchen warmer (heat through floor) → hall warmer (open doorway)

**FRVs + EWI together**: EWI reduces demand → HP has headroom. FRVs direct the headroom to the right rooms. Both needed, neither sufficient alone.

## Accuracy Expectations

| Parameter | Expected accuracy | Limiting factor |
|---|---|---|
| Thermal mass per room | ±20% | Sensor resolution (0.1°C) on slow cooldowns |
| Open doorway air exchange | ±25% | Kitchen has two doorways (coupled unknowns) |
| Closed room ventilation | ±30% | Sterling has multiple heat inputs to separate |
| Fabric loss per room | ±10% | U-values and areas are measured |
| Flow distribution | ±20% | Requires clean warmup data after DHW |
| FRV settings | ±0.2 L/min | Propagated from flow distribution uncertainty |

The model is most useful for **ranking and decisions** — which rooms are most starved, where FRVs have the biggest impact, whether kitchen needs a radiator — rather than absolute precision.
