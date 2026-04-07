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

- **Leather** (primary control room): door normally closed, no external walls, 2×DP DF rads. Exports heat to 5 neighbours before warming itself (+0.3°C in 2.5h despite biggest rads). τ=36h operational (48h doors-closed).
- **Kitchen**: no radiator. Heated by adjacent rooms + bare CH pipes in floor void (~25W each side at MWT=31).
- **Hall/Landing/Top Landing**: one continuous stairwell column, 3 floors. Only 1 radiator (Hall, ground floor). Hall is flow-starved (15mm branch competing with Front). Every internal door opens onto this column, making it the house's central air bus — best AH reference for whole-house moisture baseline. Office (door open) and Front (door partially open) feed drying moisture directly into it.
- **Office**: unoccupied, used for clothes drying most days (major moisture source, 50–100 g/h per load). Overnight: zero moisture generation — useful control room for ventilation analysis.
- **Front**: unoccupied, also used for clothes drying. Same overnight control-room value as Office.
- **Sterling**: rad OFF, door closed. Gets ~19°C from Leather's floor heat alone. Occupant prefers cold.
- **Conservatory**: dining room, cannot be closed off. Largest rads in house but cools fastest overnight (−1.9°C, glazed roof U=2.4).
- **Elvina**: coldest occupied room (16.4–17.5°C at 07:00). Ratio-method moisture analysis shows ventilation ~3× Aldora’s rate (ΔAH 0.77 vs 2.36 g/m³); nearly all the excess UA (32 vs model 24.6 W/K) is ventilation, not fabric. LEVOIT Core 300 HEPA purifier (CADR 187 m³/h, 20W) already runs for child’s allergies. Closing trickle vents would cut UA to ~17 W/K and raise overnight temp by ~3°C while improving allergen control (no outdoor pollen ingress). Requires CO2 monitoring (door ajar or morning purge for Part F fresh air).
- **Aldora**: very well sealed. Humidity reaches 58.8% RH overnight (surface ~71% = mould warning). Needs trickle vent + radiator upgrade.

## Leather Room

The primary control room. Operational τ ≈ 36 h (median of 8 overnight cooling segments: calibration nights, DHW events, coast phases).

The original 50 h figure came from daytime segments where warmer neighbours reduced inter-room loss. Overnight with doors normal and outside ~10°C, Leather cools faster (τ 29–42 h). With all doors closed and outside 1.4°C (Night 2), τ rises to 48 h — purely external envelope loss. The ratio (1.36×) shows ~26% of Leather’s operational cooling is inter-room transfer to cooler neighbours. Overnight occupant: Parson Russell Terrier (~10 g/h moisture, door closed). Feed `503101` / emonth2 provides temperature and humidity.

## DHW Cylinder

Kingspan Albion Ultrasteel Plus Solar 300L (AUXSN300ERP). 221L usable (91% plug flow efficiency). 45°C target. Standing loss 13W. T1 decay 0.23°C/h (P75 of 47 standby segments, no draws).

Twin coil-in-coil heat exchanger — solar (lower) + boiler (upper) both connected in series for HP, doubling surface area. Cold feed via dip pipe from 490mm to bottom.

### Charging

Crossover (HwcStorageTemp ≥ T1 at charge start) = definitive "full" signal. Confirmed 32+ cycles.

| Mode | Avg duration | Timeout rate | Electricity | COP |
|---|---|---|---|---|
| Eco | 102 min | 40% (<5°C) | 1.66 kWh | ~3.3 |
| Normal | 60 min | 2% | 1.19 kWh | ~2.5 |

Eco fails in cold weather (95% timeout below 2°C). Seasonal manual switch Nov–Mar → normal. `hmu HwcMode` is the authoritative eco/normal status signal on eBUS for scheduler inputs and history, but remains read-only from external masters. CylinderChargeHyst=5K (triggers at 40°C).

No-crossover charges are not always failures. Evening charges serving concurrent showers deliver 2–3× more thermal energy than quiet charges — water goes out the taps, not into the cylinder. Crossover failure only matters if it forces a morning DHW charge that steals preheat on a cold night.

### Cylinder Sensors

Three independent temperature measurements at different cylinder heights.

| Sensor | Height | Source | Purpose |
|---|---|---|---|
| **T1** (hot outlet) | 1530mm | `emon/multical/dhw_t1` (2s) | **Authoritative for DHW decisions**. Actual tap temp. |
| T2 (cold inlet) | 490mm | `emon/multical/dhw_t2` (2s) | Mains/WWHR temp (~25°C shower, ~11°C bath) |
| HwcStorageTemp | ~600mm | `ebusd/poll/HwcStorageTemp` (30s) | VRC 700 charging trigger. **Misleading after draws** — reads 13°C with 100L of 45°C above |
| DHW flow | — | `emon/multical/dhw_flow` (2s) | Tap-side, independent of HP circuit |

T1 standby decay: mean 0.21, median 0.22, P75 0.23, P90 0.24 °C/h (47 segments, ≥2h each, 10-min resolution with Multical flow filtering, 18 days). Controller uses P75 (0.23°C/h) for slight pessimism. 22:00 charge at 45°C → ~42.9°C by 07:00 (standby only). Min acceptable T1 = 40°C (empirical: lowest T1 at shower start across 60 days, no complaints).

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
| 04:00–07:00 | Morning Cosy. Matches Cosy tariff window |
| 13:00–16:00 | Afternoon Cosy |
| 22:00–end of day | Evening Cosy. Bank hot water, serve concurrent showers |

`sync_morning_dhw_timer` dynamically enables/disables the 04:00–07:00 window based on predicted T1 at waking time. When T1 is predicted above 40°C, the morning window is removed from the VRC 700 timer to prevent unnecessary charges that would contend with preheat.

⚠ Timer end times use `-:-` (TTM byte `0x90`), never `00:00`. See [[constraints#eBUS Timer Encoding]].

Overnight strategy: charge at 22:00 Cosy, then only schedule a further DHW event if predicted T1 / practical capacity at 07:00 falls below the comfort floor. The controller now scores the active launch slot (22:00 bank, battery-backed overnight pre-emptive launch, 04:00 Cosy launch, later afternoon fallback) and triggers `HwcSFMode=load` when that slot is preferred. Overnight battery-backed launches now use the explicit `energy-hub` topic `emon/tesla/discretionary_headroom_to_next_cosy_kWh`, which represents spare discretionary battery kWh before the next Cosy window. The controller compares that headroom with the expected DHW event kWh for eco vs normal mode. `HwcTimer_<Weekday>` rewrites remain in place as fallback envelopes rather than the primary decision maker. On clean crossover nights, morning DHW is unnecessary (T1 ≈43°C at 07:00 >> 40°C floor).

### HP Contention with Heating

DHW steals 50–100 min of HP capacity per charge. Impact depends on outside temperature.

| Outside | Comfort cost per charge |
|---|---|
| <2°C | ~0.5°C (unrecoverable) |
| 5°C | ~0.3°C, recovers ~1h |
| 10°C | ~0.2°C, recovers ~30 min |
| 15°C | Negligible |

On cold days, schedule DHW during Cosy to avoid stealing HP capacity from heating — the recovery heat after DHW contention requires higher flow, reducing COP and increasing electrical cost.

## Cosy Tariff

Octopus Cosy tariff with three off-peak (Cosy) windows, a peak window, and standard rate otherwise. 95% of import is off-peak via Powerwall.

- **Three Cosy (cheap) windows** (UK local time): 04:00–07:00, 13:00–16:00, 22:00–00:00
- **Peak window**: 16:00–19:00 — a distinct third rate tier, significantly above standard, verified from account API data
- **Rate tier structure**: the Cosy tariff has three rates (cheap/standard/peak). All actual p/kWh values are derived from the Octopus account API at runtime, not stored in repo config. Analysis uses `[[src/octopus_tariff.rs]]`; operators can inspect today's rates with `~/github/energy-hub/scripts/octopus-tariff-windows.sh`.
- **Battery pricing assumption** remains explicit: `config.toml` stores only `tariff.battery_coverage = 0.95`, meaning 95% of non-lowest-rate demand is treated as battery-backed energy charged at the agreement's cheapest import rate.
- **All-in household rate** is an external accounting metric, not controller truth; recompute it from Octopus account data when needed rather than hardcoding a snapshot here.

For scheduling decisions use account-derived marginal import rates, not a hardcoded tariff table or an all-in annualised rate that mixes standing charge amortisation into every kWh. The important operational distinction is battery state: battery-backed non-Cosy kWh are only a small premium over the agreement's cheapest rate, while grid-exposed non-Cosy kWh are much more expensive (peak hours 16–19 most so). Heating + DHW is the dominant controllable winter load, so battery adequacy before the next Cosy window is a key input to scheduling. `energy-hub` now publishes that as `emon/tesla/discretionary_headroom_to_next_cosy_kWh`, derived from Powerwall SoC and projected nondiscretionary load to the next Cosy window; the heating controller consumes the headroom signal instead of recomputing adequacy from raw telemetry.

**During a Cosy window, battery state must never gate heating or DHW** — grid electricity is at its cheapest, so it's always the best time to run either. The headroom signal only matters for non-Cosy gaps. Note: the headroom value is unreliable during Cosy because it projects base-load drain from current SoC without accounting for active grid charging.

Operationally, battery scarcity is not symmetric across the three gaps. The overnight gap (00:00–04:00) is the primary depletion risk window. Morning (07:00–13:00) is usually comfortable because the battery charged during the preceding Cosy. Afternoon/evening (16:00–22:00) can be battery-sensitive and expensive if misjudged. The controller treats the published headroom signal as the primary battery-cost input, with time-of-day heuristics only as secondary guards.

## Feeds

Key emoncms feed IDs and their meanings. All defined in `config.toml` `[[emoncms.feeds]]`.

| Feed ID | Name | Notes |
|---------|------|-------|
| 503093 | outside_temp | Met Office hourly. For real-time prefer `ebusd/poll/OutsideTemp` (30s) |
| 503101 | indoor_temp | emonth2 in **Leather only**, not whole-house |
| 512889 | DHW_flag | Dead since Dec 2024 — do not use |

`emon/heatpump/heatmeter_FlowRate` reads ~1 L/min constantly — this is the DHW circuit meter, useless for state classification. Use `BuildingCircuitFlow` from eBUS instead.
