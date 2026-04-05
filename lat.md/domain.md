# Domain Model

Core domain concepts for the Vaillant aroTHERM Plus 5kW heat pump analysis at 6 Rhodes Avenue, London N22 7UT.

## Operating States

The HP state machine ([[src/analysis.rs#classify_states]]) classifies each timestep by flow rate and power. Thresholds live in `config.toml` ([[src/config.rs#Thresholds]]).

The 5kW model has a software-clamped pump at 14.3 L/min (860 L/h) on the heating circuit. A diverter valve switches flow to the cylinder coil for DHW at a higher rate. This separation enables reliable state detection from flow rate alone.

- **Heating**: flow_rate 14.0–14.5 L/min, DT > 0, heat > 0
- **DHW**: flow_rate ≥ 15.0 (enter) / < 14.7 (exit), DT > 0, heat > 0
- **Defrost**: heat ≤ 0 OR DT < −0.5
- **Idle**: elec ≤ 50W

Thresholds tightened from 16.0/15.0 to 15.0/14.7 in Mar 2026 when y-filter sludge reduced DHW flow from 21 to ~16.8 L/min. Y-filter cleaned 19 Mar 2026 — DHW flow recovered to 21.3 L/min, but thresholds kept conservative (safe while heating clamped at 14.3). Monitor idle flow rate (<10 L/min = degraded, clean filter). See `docs/hydraulic-analysis.md`.

### eBUS State Classification

The Rust thermal model uses `BuildingCircuitFlow` (L/h) from eBUS for state detection, independent of the emoncms flow meter.

- \> 900 L/h = DHW
- 780–900 L/h = heating
- < 100 L/h = off

⚠ `StatuscodeNum` is unreliable for DHW detection. Code 134 appears during both off/frost standby AND active DHW. Never mean-aggregate status codes — use `last()`.

## House

1930s solid-brick semi-detached, 180 m², 2010 loft extension. Total thermal mass 48,090 kJ/K.

- **HTC**: 261 W/K (model calibrated), ~190 W/K (actual overnight from 466 nights of heat meter data — model overpredicts heat loss by ~30%)
- **13 rooms**, all sensored: 12× SNZB-02P (v2.2.0) + 1 emonth2 (Leather). Office + Landing added 24 Mar 2026 for 13/13 coverage.
- **15 radiators**, no TRVs (all valves wide open). Kitchen and Landing have no radiator. Sterling rad OFF.
- **Conservatory** excluded from thermal scoring (30 m² glass, sub-hour time constant)
- **Landing** excluded from thermal scoring (chimney model wrong for heating)
- HP maxes out at ~2°C outside — below this the house is in heat deficit (5000W − 261 × ΔT)
- No heating needed above 17°C outside — solar/internal gains sufficient

Room geometry is defined in `data/canonical/thermal_geometry.json` (single source of truth, loaded via [[src/thermal/geometry.rs#thermal_geometry_path]], consumed by Rust thermal solver + adaptive controller). Radiator T50 values duplicated in `config.toml` — keep both in sync. See `docs/house-layout.md` for full room connectivity, pipe topology, construction details, and sensor placement.

### Key Thermal Relationships

Ground-floor brick rooms (4,000–6,300 kJ/K) cool much slower than loft timber rooms (880–3,800 kJ/K). Thermal mass dominates cooling behaviour more than fabric U-values.

- **Leather** (primary control room): door normally closed, no external walls, 2×DP DF rads. Exports heat to 5 neighbours before warming itself (+0.3°C in 2.5h despite biggest rads). τ=50h empirical.
- **Kitchen**: no radiator. Heated by adjacent rooms + bare CH pipes in floor void (~25W each side at MWT=31).
- **Hall/Landing/Top Landing**: one continuous stairwell column, 3 floors. Only 1 radiator (Hall, ground floor). Hall is flow-starved (15mm branch competing with Front).
- **Sterling**: rad OFF, door closed. Gets ~19°C from Leather's floor heat alone. Occupant prefers cold.
- **Conservatory**: dining room, cannot be closed off. Largest rads in house but cools fastest overnight (−1.9°C, glazed roof U=2.4).
- **Elvina**: trickle vents open = faster cooling than expected from 2010 insulation spec.
- **Aldora**: very well sealed. Humidity reaches 58.8% RH overnight (surface ~71% = mould warning). Needs trickle vent + radiator upgrade.

## Leather Room

The primary control room. Its thermal mass dominates control strategy with τ=50 h (empirical, from 53 cooling segments).

The thermal model initially estimated τ=15 h — wrong by 3.3×. The empirical value comes from 18 calibration-night + 35 DHW mini-experiment cooling segments, all agreeing on ~50 h. Overnight planner uses the empirical value. Feed `503101` (indoor_temp) is the emonth2 in Leather only, not whole-house.

Overnight τ is unknown — first coast night was confounded (curve 0.10 ≠ off, HP still cycling). K=7500 in code, empirical K≈20,600 from post-DHW segments.

## DHW Cylinder

Kingspan Albion Ultrasteel Plus Solar 300L (AUXSN300ERP). 221L usable (91% plug flow efficiency). 45°C target. Standing loss 13W. T1 decay 0.25°C/h.

Twin coil-in-coil heat exchanger — solar (lower) + boiler (upper) both connected in series for HP, doubling surface area. Cold feed via dip pipe from 490mm to bottom.

### Charging

Crossover (HwcStorageTemp ≥ T1 at charge start) = definitive "full" signal. Confirmed 32+ cycles.

| Mode | Avg duration | Timeout rate | Electricity | COP |
|---|---|---|---|---|
| Eco | 102 min | 40% (<5°C) | 1.66 kWh | ~3.3 |
| Normal | 60 min | 2% | 1.19 kWh | ~2.5 |

Eco fails in cold weather (95% timeout below 2°C). Seasonal manual switch Nov–Mar → normal. CylinderChargeHyst=5K (triggers at 40°C).

No-crossover charges are not always failures. Evening charges serving concurrent showers deliver 2–3× more thermal energy than quiet charges — water goes out the taps, not into the cylinder. Crossover failure only matters if it forces a morning DHW charge that steals preheat on a cold night.

### Cylinder Sensors

Three independent temperature measurements at different cylinder heights.

| Sensor | Height | Source | Purpose |
|---|---|---|---|
| **T1** (hot outlet) | 1530mm | `emon/multical/dhw_t1` (2s) | **Authoritative for DHW decisions**. Actual tap temp. |
| T2 (cold inlet) | 490mm | `emon/multical/dhw_t2` (2s) | Mains/WWHR temp (~25°C shower, ~11°C bath) |
| HwcStorageTemp | ~600mm | `ebusd/poll/HwcStorageTemp` (30s) | VRC 700 charging trigger. **Misleading after draws** — reads 13°C with 100L of 45°C above |
| DHW flow | — | `emon/multical/dhw_flow` (2s) | Tap-side, independent of HP circuit |

T1 decays 0.19–0.25°C/h. 22:00 charge at 45°C → ~43°C by 07:00. Min acceptable T1 = 40°C (empirical: lowest T1 at shower start across 60 days, no complaints).

### Household Usage

5 people, ~171 L/day avg (0.9 tanks), busiest days 260–270 L (1.3 tanks). Weekly pattern: ~1 bath, ~18 showers, ~12 taps. Draw classification: bath ≥650 L/h, shower 350–650 L/h, tap <350 L/h.

WWHR on shower drain: 41% effectiveness, +9°C steady-state lift. ~3 min delay. Baths bypass (taps, not drain).

### DHW Session Analysis

The [[src/thermal/dhw_sessions.rs]] CLI analyses draws at 2s Multical resolution with HWC state tracking.

Classifies draws by type (bath/shower/tap), detects draws during HP charging. Writes `dhw_inflection` + `dhw_capacity` to InfluxDB. z2m-hub autoloads `recommended_full_litres` on startup. The Multical `dhw_flow` is tap-side (independent of HP circuit) — draws must be tracked regardless of charging state.

### DHW Scheduling

VRC 700 timer windows aligned to Cosy tariff. DHW timing difference is <0.3p/shower — only matters on cold days when battery depletes.

| Window | Rationale |
|---|---|
| 05:30–07:00 | Morning Cosy. HP heats house 04:00–05:30 first |
| 13:00–15:00 | Afternoon Cosy. Shortened from 16:00 to prevent peak spills |
| 22:00–end of day | Evening Cosy. Bank hot water, serve concurrent showers |

⚠ Timer end times use `-:-` (TTM byte `0x90`), never `00:00`. See [[constraints#eBUS Timer Encoding]].

Overnight strategy: charge at 22:00 Cosy, monitor T1, top up at 04:00 Cosy only when predicted T1 at 07:00 < 40°C. On clean crossover nights, morning DHW is unnecessary (T1 ≈43°C at 07:00 >> 40°C floor).

### HP Contention with Heating

DHW steals 50–100 min of HP capacity per charge. Impact depends on outside temperature.

| Outside | Comfort cost per charge |
|---|---|
| <2°C | ~0.5°C (unrecoverable) |
| 5°C | ~0.3°C, recovers ~1h |
| 10°C | ~0.2°C, recovers ~30 min |
| 15°C | Negligible |

On cold days, schedule DHW at 22:00 to keep preheat window clear.

## Cosy Tariff

Octopus Cosy tariff with three off-peak windows and a Powerwall battery. 95% of import is off-peak via Powerwall.

- **Windows**: 04:00–07:00, 13:00–16:00, 22:00–00:00
- **Q2 2026 rates** (inc VAT): off-peak 13.24p, day 26.98p, peak 40.48p, standing 52.76p/day
- **All-in effective rate**: 16.7p/kWh (6,908 kWh, ~£1,151, 12 months inc standing + VAT)
- **Marginal battery-blended rate**: 13.9p/kWh (95% battery coverage)

For scheduling decisions use marginal rate (13.9p), not all-in (16.7p which includes 2.8p/kWh standing charge amortisation). Annual saving: £565 (46%) vs gas combi at current tariff. Octopus data in `~/github/octopus/`, refresh via `cd ~/github/octopus && npm run cli -- refresh`.

## Feeds

Key emoncms feed IDs and their meanings. All defined in `config.toml` `[[emoncms.feeds]]`.

| Feed ID | Name | Notes |
|---------|------|-------|
| 503093 | outside_temp | Met Office hourly. For real-time prefer `ebusd/poll/OutsideTemp` (30s) |
| 503101 | indoor_temp | emonth2 in **Leather only**, not whole-house |
| 512889 | DHW_flag | Dead since Dec 2024 — do not use |

`emon/heatpump/heatmeter_FlowRate` reads ~1 L/min constantly — this is the DHW circuit meter, useless for state classification. Use `BuildingCircuitFlow` from eBUS instead.
