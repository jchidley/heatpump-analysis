# Overnight heating strategy analysis

Date: 28 March 2026

## Context

Vaillant Arotherm Plus 5kW heat pump, Octopus Cosy tariff, 13 rooms with Zigbee sensors, eBUS monitoring. House is 1930s solid brick, 180m², HTC ~261 W/K.

This analysis uses measured emonhp data (510 winter nights with 4°C setback, 2 nights no setback, 2 nights heating off) plus calibrated thermal model parameters to find the optimal overnight heating strategy.

## Tariff structure (Octopus Cosy)

| Period | Rate | Times |
|---|---|---|
| Off-peak | 14.05p/kWh | 00:00–04:00, **04:00–07:00**, 13:00–16:00 |
| Mid-peak | 28.65p/kWh | 07:00–13:00, 19:00–00:00 |
| Peak | 42.97p/kWh | 16:00–19:00 |
| Blended (with battery) | ~17p/kWh | Effective rate outside Cosy windows |

The **04:00–07:00 morning Cosy window** is the key heating opportunity.

## DHW timing (measured)

From BuildingCircuitFlow data (9 morning DHW cycles, Mar 2026):

- DHW starts: **05:05–05:10** consistently (eco mode, VRC 700 schedule)
- DHW ends: **06:50–07:12**
- Duration: **104–125 min** (eco mode, ~2h)
- Normal mode: ~1h (higher MWT, same energy)

Of the 180-min Cosy morning window (04:00–07:00):
- **65 min** (36%) available for space heating before DHW starts
- **115 min** (64%) consumed by DHW cycle
- Recovery after DHW ends at ~07:10 falls in **mid-peak** (28.65p)

DHW can be rescheduled anywhere within the Cosy window. Normal mode completes in ~1h vs 2h eco.

## Measured parameters (calibrated from data)

### Room cooling rate (from controlled experiment nights, heating off)

| Outside temp | Midnight–03:00 | 03:00–07:00 | Note |
|---|---|---|---|
| 8.5°C | 0.28°C/hr | 0.27°C/hr | Night 1, calm |
| 5.0°C | 0.26°C/hr | 0.30°C/hr | Night 2, calm |

**Rate scales with indoor-outdoor ΔT: 0.023°C/hr per °C of ΔT.**

Barely any difference midnight vs pre-dawn — wind calms overnight (land effect).

Starting at 21°C, 8h off, outside avg 7°C: rooms reach **~18.6°C** (2.4°C drop).

### Outside temperature profile (Dec–Mar average, hourly)

| Hour | Avg °C | vs midnight |
|---|---|---|
| 23:00 | 8.1 | +0.5 |
| 00:00 | 7.6 | 0 |
| 02:00 | 7.2 | -0.4 |
| 04:00 | 6.8 | -0.8 |
| 06:00 | 6.5 | -1.1 |
| 07:00 | 6.5 | -1.1 |

Outside temp falls ~1°C from midnight to dawn. DHW earlier in Cosy window = warmer outside = better COP.

### HP performance (measured from emonhp)

| Mode | Heat output | Electricity | COP | MWT |
|---|---|---|---|---|
| Recovery (08–10, post-setback) | 3.5 kWh/hr | 0.73 kWh/hr | 4.8 | ~34°C |
| Steady state (14–18) | 2.2 kWh/hr | 0.43 kWh/hr | 5.3 | ~31°C |
| DHW eco (2h) | 6.0 kWh total | 1.9 kWh total | 3.1 | ~45°C |

Measured net room recovery rate: **1.0°C/hr** (first 2°C of deficit).

### Overnight energy consumption (measured, 23:00–07:00, ~8°C outside)

| Regime | Nights | Heat kWh | Elec kWh | COP |
|---|---|---|---|---|
| 4°C setback | 100 (at 7–9.5°C) | 19.1 | 4.1 | 4.61 |
| No setback | 2 | 29.0 | 6.5 | 4.43 |
| Heating off | 2 | 6.0 | 1.8 | 3.4 |

### Full cycle energy (measured, 23:00–12:00, ~8°C outside)

| Regime | Overnight kWh elec | Recovery kWh elec | Total | COP |
|---|---|---|---|---|
| 4°C setback | 4.1 | 3.6 | 7.7 | 4.51 |
| No setback | 6.5 | 1.7 | 8.2 | 4.42 |
| Heating off | 1.8 | 5.1 | 6.9 | 4.06 |

## Key findings

### 1. Setback saves energy at flat rate, costs more on Cosy

At flat 17.07p/kWh: 4°C setback saves £86/yr vs no setback (7.7 vs 8.2 kWh, less total energy).

**On Cosy tariff: 4°C setback costs £77/yr MORE than no setback.** The setback saves 2.4 kWh overnight at off-peak (14.05p = 34p saved) but adds 1.9 kWh recovery at mid-peak (28.65p = 54p extra). The 2:1 rate differential penalises shifting energy from overnight to morning.

### 2. DHW steals the Cosy morning window

The current eco DHW schedule (05:05–07:10) consumes 64% of the Cosy morning window. Post-DHW recovery at 07:10+ is at mid-peak (28.65p). Any strategy that requires morning recovery is penalised.

### 3. Optimal strategy: OFF overnight, DHW first in Cosy, recover at Cosy rate

The model (calibrated from measured data) recommends:

```
22:00  Heating OFF
       (rooms cool 0.3°C/hr, wind calm, zero electricity)
04:00  DHW normal mode (Cosy starts, outside still warmest = best DHW COP)
05:00  DHW complete, heating ON at 21°C (Cosy rate)
05:30  Rooms at 19.6°C, recovering at 1.0°C/hr
06:30  Rooms at 20.6°C
07:00  Rooms at 20.9°C — Cosy window ends
07:00+ Steady state at mid-peak (minimal, rooms already at target)
```

Room temperature trace: **21.0 → 19.2°C (04:00) → 19.1°C (05:00, post-DHW) → 20.9°C (07:00)**

### 4. Cost comparison (modelled at avg winter outside temps)

| Strategy | Elec/night | Cost/night | Annual | vs current |
|---|---|---|---|---|
| No setback + eco DHW 05:00 (current) | 6.4 kWh | 118p | £212 | baseline |
| **OFF 22–04, DHW 04–05, heat 05–07** | **4.6 kWh** | **84p** | **£151** | **save £61** |
| 4°C setback + eco DHW 05:00 (historical) | 7.7 kWh | 161p | £289 | costs £77 more |

### 5. Why turning off beats setback on Cosy

- Setback (HP cycling at 17°C target, MWT ~25°C): uses 4.1 kWh overnight at 17p blended, then 3.6 kWh recovery at 28.65p mid-peak
- Turning off: uses 0 kWh overnight, then all recovery within Cosy window at 14.05p
- The 6h off period only drops rooms 1.8°C (thermal mass), recoverable in 2h at Cosy rate

### 6. Diminishing returns beyond 4°C setback (flat rate only)

At flat rate, each degree of setback 0→4°C saves £20/yr. Beyond 4°C (towards heating off), returns drop to £11/yr per degree. The break is at 4°C because recovery COP degrades (4.06 vs 4.51) and thermal mass recharge costs more from deeper deficits.

**This is irrelevant on Cosy** because the tariff structure dominates the physics.

## What needs to be trialled

The recommended strategy is modelled, not measured. The key parameters are calibrated from measured data, but the specific combination needs validation:

1. **DHW in normal mode at 04:00** — verify it completes by 05:00 and the VRC 700 transitions cleanly to space heating
2. **Recovery rate 05:00–07:00** — verify rooms recover 1.8°C in 2h at Cosy rate
3. **Room temps at 07:00** — verify ≥19.5°C across scored rooms

### eBUS commands for trial

```bash
# On pi5data, schedule via at/cron:

# 22:00: heating off
echo 'write -c 700 Z1OpMode off' | nc -w 2 localhost 8888

# 04:00: heating on + DHW trigger
echo 'write -c 700 Z1OpMode auto' | nc -w 2 localhost 8888
# (DHW scheduling via VRC 700 timer — set HwcTimer to 04:00-05:00 window)

# Revert to normal:
echo 'write -c 700 Z1NightTemp 17' | nc -w 2 localhost 8888
echo 'write -c 700 Z1OpMode auto' | nc -w 2 localhost 8888
```

### Measurement plan

Run for 3+ nights at similar outside temps. Compare against:
- Measured 4°C setback baseline (508 nights)
- Measured no-setback (2 nights)
- Model prediction

Collect:
- `emon/heatpump/heatmeter_Energy` + `electric_Energy` (cumulative, hourly)
- `ebusd/poll/BuildingCircuitFlow` (state classification)
- `ebusd/poll/FlowTemp` + `ReturnTemp` (MWT)
- Room temps from Zigbee sensors (all 13)
- Outside temp from `ebusd/poll/OutsideTemp`

## Assumptions and limitations

- Cooling rate calibrated from 2 experiment nights in March — may differ in deep winter (colder outside, higher ΔT, faster cooling)
- Recovery rate measured from post-setback mornings — recovery from 6h off may be slightly different
- Battery roundtrip losses absorbed into blended 17p rate — actual varies
- DHW normal mode duration assumed 1h — verify with VRC 700
- Model uses average winter outside temp profile — individual nights vary
- Wind effects not modelled — windy nights will cool faster
- Occupancy patterns (door opening, showers) not modelled

## eBUS status code notes

`StatuscodeNum` is **unreliable for DHW detection** on the Arotherm. Code 134 ("off/frost") appears during the entire DHW cycle when the diverter switches flow to the cylinder. Use `BuildingCircuitFlow` instead: >900 L/h = DHW, 780–900 = heating, <100 = off.

## Related documents

- `AGENTS.md` — setback history, eBUS state classification, operational model accuracy
- `docs/rust-migration-plan.md` — thermal model development roadmap
- `model/thermal-config.toml` — current model configuration
