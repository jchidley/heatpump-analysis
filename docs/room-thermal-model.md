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

13 temperature/humidity sensors covering all rooms. Office and Landing sensors added 24 Mar 2026 — 13/13 room coverage complete. Data flows to InfluxDB at ~5 minute intervals. See AGENTS.md for the full sensor inventory.

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

All thermal commands are now in Rust (`model/house.py` deleted 2026-03-30):

```bash
cargo run --bin heatpump-analysis -- thermal-rooms
cargo run --bin heatpump-analysis -- thermal-connections
cargo run --bin heatpump-analysis -- thermal-analyse --config model/thermal-config.toml
cargo run --bin heatpump-analysis -- thermal-equilibrium --outside 0 --mwt 40
cargo run --bin heatpump-analysis -- thermal-moisture --config model/thermal-config.toml
cargo run --bin heatpump-analysis -- thermal-calibrate --config model/thermal-config.toml
cargo run --bin heatpump-analysis -- thermal-validate --config model/thermal-config.toml
```

## Data Requirements

- **Minimum**: one full setback → DHW → warmup cycle (one night)
- **Good**: one week of daily cycles across varying outside temperatures
- **Best**: data spanning a cold snap (0°C or below) for design condition validation

Current sensor coverage: 13 of 13 rooms (complete since 24 Mar 2026).

## Night 1 Results — Controlled Cooldown (24-25 March 2026)

Heating off via eBUS at 23:00, restored 07:00. Doors in normal state. Bathroom door closed (post-shower). Outside: 10.1→7.5°C, windy.

### Cooling Rates (23:10→03:05, 4h clean period)

| Room | Drop | °C/h | Key finding |
|---|---|---|---|
| Conservatory | -2.0°C | 0.59 | Fastest. Glazed roof (U=2.4) dominates after dark. |
| Office | -1.8°C | 0.51 | Surprise 2nd — open door to landing + tiny volume (12.7m³) |
| Bathroom | -1.8°C | 0.50 | MVHR extracting, door closed post-shower |
| Landing | -1.6°C | 0.48 | No rad, stairwell heat sink — confirmed |
| Kitchen | -1.5°C | 0.43 | No rad, tracks hall through open doorway |
| Front | -1.2°C | 0.35 | Bay window, partial door |
| Hall | -1.1°C | 0.30 | Front door + stairwell. 15mm rad can't compensate |
| Shower | -0.9°C | 0.27 | Open door to stairwell, warm air sinks below |
| Jack & Carol | -0.7°C | 0.19 | Door closed at night — much slower than daytime |
| Leather | -0.7°C | 0.18 | No external walls, all loss through internal paths |
| Aldora | -0.6°C | 0.18 | Occupied (+80W), well sealed |
| Elvina | -0.6°C | 0.18 | Door closed, trickle vents open |
| Sterling | -0.2°C | 0.06 | Slowest — leather floor heat sustains it |

### Moisture Balance (Night 1)

Moisture model independently validates ventilation rates. Outside AH ~6.3 g/m³ (from Open-Meteo actual data).

| Room | ACH (moisture) | ACH (thermal) | Confidence | Finding |
|---|---|---|---|---|
| Bathroom | 0.15 | 0.16 (MVHR eff.) | **HIGH** | MVHR spec validated — near-perfect match |
| Jack & Carol | 1.80 | 0.80 | **Moderate** | 2 people yet AH dropped — bay window is a sieve |
| Elvina | 0.60 | 0.70 | **Moderate** | Trickle vents moving more air than originally estimated |
| Aldora | 0.35 | 0.30 | **Low signal** | AH rose despite ventilation — room too sealed for 1 person |
| Sterling | 0.06 | 0.15 | **Low signal** | Very sealed, minimal moisture exchange |
| Kitchen | 0.19 | 0.35 | **Good** | Infiltration to outside only (doorway exchange is inter-room) |

**Key insight**: Moisture ACH measures infiltration to outside only. Thermal ACH includes inter-room air exchange through doorways. The difference between them IS the doorway exchange rate.

Three rooms updated from moisture data: Aldora 0.10→0.30, Elvina 0.50→0.70, Jack&Carol 0.35→0.80.

**Mould risk**: Aldora at 58.8% RH (surface RH ~71% at ΔT=3°C to cold surface) = **warning level**. Needs trickle vent or door ajar. All other rooms OK.

### Thermal Mass Model (added 25 Mar 2026)

Construction-based thermal mass estimates per room (kJ/K):

| Construction | Floor | Rooms | C range | Key material |
|---|---|---|---|---|
| Brick + concrete slab | Gnd | Kitchen, Conservatory | 4,810–6,308 | Concrete floor + brick internal walls |
| Brick + suspended timber | Gnd | Hall, Front, Leather | 3,761–4,985 | Brick walls, timber floor |
| Brick + timber (1st floor) | 1st | Bathroom, Jack&Carol, Office, Sterling | 2,226–5,202 | Brick internal walls, timber between floors |
| Timber (loft/landing) | Loft/1st | Elvina, Aldora, Shower, Landing | 880–3,778 | Lightweight timber stud, insulated |

**Total house: 48,090 kJ/K.** Implies ~12h to cool 1°C at 4kW loss — matches observed ~2.5°C drop in 8h.

Ground floor: all internal walls are solid single brick. Hall, Front, and Leather have suspended timber floors (not concrete). Kitchen has concrete slab (standard 1930s for service rooms). Conservatory (yr 2000 extension) has concrete slab on London clay.

Office and Landing have **100mm modern insulation** between their floor and the hall ceiling (U≈0.25 vs U≈1.7 for uninsulated timber floor). This thermally decouples them from the hall below — they depend on doorway air exchange, not floor conduction.

Leather has a **spiral cellar** below the suspended timber floor — uncertain ground floor U-value.

### Predicted vs Observed Cooling Rates

With thermal mass + occupancy heat (80W/person), model predicts cooling rates that match 8/13 rooms within 2 ranks (Spearman 0.50):

| Match quality | Rooms | Ratio (pred/obs) | Notes |
|---|---|---|---|
| **Excellent** (0.9–1.1) | Conservatory, Elvina, Hall, Shower, Leather | 0.90–1.18 | Thermal mass + construction correct |
| **Good** (0.5–0.9 or 1.1–1.5) | Bathroom, Jack&Carol, Front, Aldora, Office, Kitchen | 0.52–1.40 | Inter-room coupling effects |
| **Poor** | Landing, Sterling | 0.00, 2.52 | Landing: convective stairwell not modelled. Sterling: leather floor heat sustains it |

### Warmup Analysis (25 Mar, 07:00→09:30)

After Night 1, heating restored at 07:00. HP started at 6.7kW/83% compressor, settled to 4.2kW/57%.

| Group | Rooms | Rise in 2.5h | Finding |
|---|---|---|---|
| **22mm, fast** | Conservatory +1.7, Bathroom +0.8, Elvina +0.6, Aldora +0.5 | 22mm rads delivering well |
| **15mm, slow** | Hall +0.4, Front +0.8 (mixed pipe), Jack&Carol +0.3 (door open) | 15mm branch starvation visible |
| **No rad** | Kitchen +0.6, Landing +0.8 | Neighbour heat through walls/doorways |
| **Rad off** | Sterling -0.2 | Still dropping — leather hadn't warmed enough yet |
| **Heat hub** | Leather +0.5 (doors closed 08:00) | 4752W T50 but only +0.5°C — exports heat to 5 neighbours |
| **Small room** | Office +1.1 | Fast for 15mm — small volume + open door to warm landing |

### Key Findings (updated from Night 1 + warmup)

1. **Conservatory is NOT over-radiatored** despite 5700W T50. It's the coldest ground floor room overnight (15.9°C vs kitchen 17.5°C). Only warm in afternoon from solar gain (+1.1°C above kitchen at peak). Daily swing 3.6°C driven by solar/glazed roof cycle. C-K overnight = -1.6°C confirms heat flows FROM kitchen TO conservatory through open doorway.

2. **Leather is the heat hub.** Two 22mm DP DF rads (4752W T50) but only warms +0.5°C in 2.5h — exports heat to Sterling (floor), Kitchen, Hall, Front, Conservatory. Warms the ground floor more than it warms itself. Doors open in morning for dog, closed during day for work.

3. **Sterling is sustained by leather floor heat** through uninsulated timber floor/ceiling (U≈1.7, 17m², 29 W/K). Occupant prefers cold room, opens windows when home. Floor insulation (mineral wool between joists) would: leather keeps heat, Sterling gets cold room, HP saves ~50-80W. Best single-room intervention after EWI.

4. **Jack&Carol bay window extremely leaky.** Moisture balance: 2 people producing 80g/h yet AH dropped overnight. Moisture-implied ACH ~1.8 (thermal model uses 0.80 as compromise). Door open daytime → 1.6°C drop despite continuous heating. Door closed overnight → only 0.19°/h.

5. **Office + Landing thermally decoupled from hall** by 100mm insulated floor (U≈0.25). Both depend on doorway air exchange, not floor conduction. Both cool fast with doors open (0.51, 0.48°/h) despite office having insulated ceiling too. Door is everything for these rooms.

6. **HP minimum modulation** is 2.2kW (30% of 5kW, same hardware as 3.5kW). Cycling starts when demand < 2.2kW, which occurs above ~11.5°C outside at 20°C setpoint. After EWI (HTC ~177 W/K), cycling threshold drops to ~7-8°C.

### Ventilation Groups (updated from moisture balance)

| Group | ACH | Rooms | Evidence |
|---|---|---|---|
| MVHR (measured) | 0.75 (eff. 0.16) | Bathroom | Validated by moisture balance (0.15 ACH) |
| Very leaky (bay window) | 0.80 | Jack & Carol | Moisture: AH drops with 2 occupants |
| Trickle vents (stack effect) | 0.70 | Elvina | Moisture: barely maintains AH with 1 person |
| Infiltration (high) | 0.50 | Hall | Front door + stairwell base |
| Open doors + draft | 0.30–0.35 | Kitchen, Conservatory, Front | RH drops overnight |
| Sealed but inadequate | 0.30 | Aldora | Moisture: AH rises with 1 person. Mould risk |
| Closed, slight leakage | 0.15–0.20 | Leather, Landing, Office, Shower | Closed doors or interior rooms |
| Sealed modern | 0.10–0.15 | Sterling | Triple glazed, door closed. Nearly zero moisture exchange |

## Conservatory Assessment

Not over-radiatored (contrary to earlier model snapshot during warmup). The 2×K3 radiators (T50=5,700W) are needed — the conservatory is the coldest ground floor room overnight and has the highest heat loss rate (0.59°/h). Solar gain through the glazed roof provides ~1°C of daytime warming above kitchen temperature, but this disappears after dark.

### Kitchen↔conservatory door closer (evaluated, not worth doing)

Equilibrium model shows closing the kitchen↔conservatory door makes **no meaningful difference** to the rest of the house. The doorway exchange is ~170W = 0.07°C spread across the whole house. Closing it actually makes the kitchen colder — the kitchen (no radiator) depends on the warm convective flow from hall→kitchen→conservatory. The conservatory radiator (~1,300W at equilibrium) cannot be turned off either — conservatory drops to ~5°C without it. On cold days the HP is at capacity and delivers the same total watts regardless of door state — heat just redistributes between rooms.

### Leather↔conservatory SG door analysis

UA=21.12 W/K conduction through single-glazed panels (closed). When **open** for the dog (mornings ~07:00–09:30), buoyancy exchange adds **1,500–2,000W** at 10°C ΔT — measured as a **1.4°C dip in leather** over 2.6 hours on 11 coldest mornings (data from emonth2 Nov 2025–Mar 2026). On cold days the HP is at capacity and cannot compensate. The dip hurts leather comfort but doesn’t affect whole-house equilibrium. Minimising open time on cold mornings is the only mitigation. Secondary glazing or heavy curtain would reduce conduction (253W through glass when closed).

Replacing the glazed roof with solid insulated (£10-15k) is **not justified** given adequate comfort. After EWI on the SE wall, the HP will have spare capacity, and the conservatory benefits indirectly from warmer neighbouring rooms.

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

## Controlled Cooldown Experiments (24-26 March 2026)

### Purpose

Previous cooldown data was from overnight setback with HP cycling (status 101/104). This gives noisy data — the HP keeps injecting heat intermittently. Two controlled experiments with heating fully off via eBUS (`write -c 700 Z1OpMode off`) provide clean exponential decays.

### Schedule

Automated via `at` on pi5data. DHW unchanged (auto) throughout.

| Night | Date | Heating off | Heating on | Doors | Outside forecast |
|---|---|---|---|---|---|
| 1 | Mon 24→Tue 25 Mar | 23:00 | 07:00 | Normal (open) | ~5.5°C, windy |
| 2 | Tue 25→Wed 26 Mar | 23:00 | 07:00 | All closed | ~1.4°C, clear, calm |

### Door states — Night 1 (normal)

- **Open**: Bathroom, Office, Shower, Kitchen↔Hall, Kitchen↔Conservatory
- **Open day / closed night**: Jack & Carol (closed by 23:00)
- **Partially closed**: Front
- **Always closed**: Elvina (trickle vents open), Aldora, Sterling, Leather

### Door states — Night 2 (all closed)

Every internal door closed. Elvina: door closed, trickle vents open (occupant won't close them). Each room becomes approximately independent — cooldown dominated by external envelope.

### What we get

| From | Parameter |
|---|---|
| Night 2 (doors closed) | Per-room HLC to outside |
| Night 1 (doors open) | Coupled system cooldown |
| Night 1 − Night 2 | Inter-room air exchange rates through doorways |
| Sum of Night 2 HLCs | Should ≈ 261 W/K (whole-house HTC cross-check) |
| Night 2 cold conditions | Better signal-to-noise (larger ΔT, more °C drop) |

### eBUS control

Tested and confirmed working 24 Mar 2026:
- `write -c 700 Z1OpMode off` → heating stops within 60s, DHW unaffected
- `write -c 700 Z1OpMode auto` → heating restored, HP resumes on next controller cycle
- Status code 101 (standby with pump overrun) confirmed as non-heating state

### Observations from 24 Mar 2026 (pre-experiment, normal heating day)

**Energy balance**: Model predicts 3,313W total loss but HP delivering 2,715W and rooms roughly stable. Model over-estimates losses by ~20%. Experiments should identify which rooms' losses are inflated.

**Jack & Carol**: Dropped 20.8→19.2°C over 20h of continuous heating with door open. Door closed overnight: only 0.08°/h cooling. This proves air exchange through the open door is the dominant loss, not wall fabric or flow starvation.

**Sterling**: 0.2°C swing all day, rad off. Perfect thermal equilibrium with neighbours at ~19.3°C.

**Office (new sensor)**: 19.6°C settled reading. Well insulated ceiling + 50mm insulated floor. Door normally open. Expected to be slowest-cooling room on Night 2 (doors closed).

**Landing (new sensor)**: 19.8°C. No radiator. Key heat sink node — absorbs heat from every room with an open door via stairwell.

**Wind**: Night 1 forecast windy. Will increase infiltration in leaky rooms (Elvina trickle vents, Jack&Carol bay window, Hall front door). Night 2 forecast calm — cleaner data for the critical experiment.

## Recovery from Night 1 Experiment (25 March)

Heating restored 07:00. HP ran continuously at 3.3-6.7kW for 6+ hours. By 13:00, most rooms still 0.7-1.3°C below previous day's temperature at the same time (outside was 3.1°C colder: 9.9 vs 13.0°C).

Key observations:
- **Hall** still 1.3°C below normal after 6h of heating — 15mm branch cannot recover quickly
- **Conservatory** recovered fastest (+4.4°C in 6h) from solar + big 22mm rads
- **Bathroom** was the only room warmer than yesterday (+0.4°C) — shower heat boost
- **Sterling** still slightly below starting temperature after 6h — waiting for leather to warm up enough to push heat through floor
- **Leather** only +0.8°C despite biggest rads — exporting heat to 5 neighbours

The slow recovery demonstrates the house's high thermal mass (48,090 kJ/K). The overnight experiment withdrew roughly 48,000 × 2.5 = 120 MJ of stored heat. At 4kW average HP output, recovering this takes ~8 hours on top of replacing ongoing losses.

## Recommended Improvements (from Night 1 + moisture analysis)

### 1. Jack&Carol bay window draught-proofing
**Problem**: Moisture balance proves extreme leakiness — 2 people producing 80g/h yet AH *drops* overnight. ACH ~0.80-1.80 through closed bay window. Only occupied room where humidity decreases with occupants.

**Fix**: Draught strip frame joints, check sash seals. Target ACH 0.30.

**Saving**: ~60W at design conditions. Room temperature stabilises — less overnight infiltration means it holds heat better.

**Cost**: Low (draught strip materials). **Payback**: Immediate comfort improvement.

### 2. Aldora trickle vent
**Problem**: RH reaches 58.8% overnight with 1 person (surface RH ~71% = mould warning). Room is too well sealed (ACH 0.30) for an occupied bedroom. Required by Part F for a bedroom.

**Fix**: Window trickle vent adding ~0.15 ACH, bringing total to ~0.45. Keeps RH below 55% with 1 person overnight.

**Cost**: ~£30 + fitting. **Eliminates mould risk.**

### 3. Aldora radiator upgrade
**Problem**: Current 376W towel rad is the smallest in the house. With trickle vent adding ~30W of ventilation loss, the room needs more heat.

**Fix**: Replace with 909W DP DF (same as Elvina). More than doubles heat delivery. On 22mm pipe so flow is adequate.

**Cost**: ~£150-200 + fitting.

### Combined effect
Seal Jack&Carol (save 60W) + open Aldora (spend 30W) + bigger Aldora rad (delivers the extra 30W) = **roughly net zero on HP** but much better comfort, no mould risk, and Jack&Carol holds temperature overnight.

### 4. Sterling floor insulation (see above)
Mineral wool between leather/sterling joists. Leather keeps heat, Sterling gets cold room occupant wants, HP saves 50-80W.

### 5. EWI on SE wall (see above)
84 W/K saved = 32% of whole-house HTC. Prerequisite for all other optimisations — without it, HP has no headroom at design conditions.

### Priority order
1. **EWI** — biggest single improvement, enables everything else
2. **Jack&Carol draught-proofing** — cheapest, immediate payback
3. **Aldora trickle vent** — health/compliance requirement
4. **Aldora rad upgrade** — supports trickle vent
5. **Sterling floor insulation** — moderate effort, permanent saving
6. **FRVs** — rebalance flow once EWI reduces total demand

## Model Accuracy (as of 25 March 2026)

### Energy balance
- Model total loss: 4,374W vs HP meter: 3,989W — **10% over-prediction** (good)

### Confidence levels

| Parameter | Confidence | Source |
|---|---|---|
| Fabric U-values | **High** | Measured areas + standard U-values |
| MVHR performance | **High** | Spec validated by moisture balance (0.16 vs 0.17 ACH) |
| Pipe topology / radiator T50 | **High** | Physical survey |
| Thermal mass (brick rooms) | **Medium** | Construction-based estimates, not measured |
| Ventilation (aldora, elvina, jackcarol) | **Medium** | Moisture-validated, some uncertainty in outside AH |
| Ventilation (other rooms) | **Low-Medium** | Estimated, consistent with humidity trends |
| Inter-room doorway exchange | **Medium** | Buoyancy doorway physics (Cd model) with canonical doorway geometry |
| Landing/top-landing convective model | **Medium-Low** | Explicit hall→landing→top_landing→shower chimney links now modelled; top_landing currently virtual (no dedicated temp sensor) |
| Leather ground floor loss | **Low** | Spiral cellar creates uncertain air gap |

### What Night 2 (doors closed, 1.4°C outside) will resolve
- Per-room external HLC measured directly (no inter-room coupling)
- With known C + measured rate → HLC = C × rate / ΔT for each room
- Night 1 − Night 2 difference = doorway exchange rates
- Landing isolated — true external loss revealed
- Sterling isolated — true loss without leather floor heat
- Kitchen isolated — true loss without doorway exchange

The model is most useful for **ranking and decisions** — which rooms are most starved, where FRVs have the biggest impact, whether kitchen needs a radiator — rather than absolute precision.

## Model status update (Mar 2026)

Recent upgrades implemented in canonical geometry + Python/Rust thermal solvers:

- Stair stack path now modelled explicitly with buoyancy links:
  - `hall ↔ landing ↔ top_landing ↔ shower`
- `top_landing` is represented as an explicit model node (virtual temperature proxy for now).
- Internal wall connection UAs were re-derived from plan-derived room internal-wall totals.
- Optional public-wind coupling added to Rust `thermal-calibrate` (Open-Meteo).
  - Current default remains wind disabled in config because it did not improve the two-night objective yet.
