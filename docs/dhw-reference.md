# DHW Reference Data

Supporting reference and evidence for DHW reasoning. The canonical current-state cylinder rules, sensors, scheduling, and household assumptions live in [`../lat.md/domain.md`](../lat.md/domain.md) and [`../lat.md/infrastructure.md`](../lat.md/infrastructure.md). For operating policy and decisions, see [DHW plan](dhw-plan.md).

## What this file keeps

This file keeps detailed measurement notes and reference evidence that are useful for operator reasoning but too detailed for the condensed `lat.md` model.

## Cylinder specification (reference)

**Kingspan Albion Ultrasteel Plus Solar Indirect 300L (AUXSN300ERP)**

| Spec | Value |
|---|---|
| Capacity | 300L total, 243L geometric max drawable below draw-off |
| Internal dimensions | ~450mm diameter, ~1932mm internal height |
| Insulation | 50mm PU foam |
| Heat exchanger | Twin coil-in-coil; both coils in series for HP use |
| Cold feed | Dip pipe from 490mm connection to bottom |
| Standing heat loss | ~13W measured |
| T1 decay rate | ~0.23°C/h (P75 of 47 standby segments; mean 0.21) |

### Connection heights (outside / internal mm)

| Outside | Internal | Connection |
|---|---|---|
| 420 | 370 | Bottom coil top |
| 465 | 415 | Dry stat pocket (VR10 / `HwcStorageTemp`) |
| 540 | 490 | T2 sensor + cold inlet |
| 1020 | 970 | Top coil |
| 1580 | 1530 | T1 sensor + hot draw-off |

## Sensor detail

Canonical sensor roles are summarised in [`../lat.md/domain.md#Cylinder Sensors`](../lat.md/domain.md#cylinder-sensors). Detailed reference table kept here:

| Sensor | Rate | Resolution | Notes |
|---|---|---|---|
| T1 (`emon/multical/dhw_t1`) | ~2s | 0.01°C | authoritative top-of-cylinder / outlet truth |
| T2 (`emon/multical/dhw_t2`) | ~2s | 0.01°C | cold-side reference |
| `HwcStorageTemp` | ~30s | 0.5°C | lower-cylinder control sensor used by VRC 700 |
| `dhw_flow` | ~2s | 1 L/h | tap-side flow, independent of HP circuit |
| `dhw_volume_V1` | ~2s | 10L steps | cumulative draw volume |

## WWHR measurements

Waste Water Heat Recovery on the shower drain improves shower cold-side temperature materially.

| Phase | T2 | Lift from 15.8°C mains |
|---|---|---|
| Start | 15.5°C | −0.3°C |
| 1 min | 17.1°C | +1.3°C |
| 2 min | 19.7°C | +3.9°C |
| 3 min | 22.1°C | +6.3°C |
| Steady state | 24.8°C | +9.0°C |

Practical note: showers benefit; baths bypass the WWHR path.

## Draw-rate reference

| Draw type | Total flow | Cold side | Hot fraction | Cylinder draw rate |
|---|---|---|---|---|
| Shower (WWHR) | ~7 L/min | ~25°C | 77% | ~5.4 L/min |
| Bath fill | ~12 L/min | ~15.8°C | 84% | ~10 L/min |
| Sink/tap | ~3 L/min | ~15.8°C | 84% | ~2.5 L/min |

## Capacity observations

Capacity is strongly T1-dependent. Lower T1 means weaker stratification and earlier inflection.

| Date | Usable L | T1 | T2 | Flow L/h | Context |
|---|---|---|---|---|---|
| 21 Mar | 177 | 44.3°C | 25.8°C | 464 | Full charge, WWHR showers |
| 23 Mar | 155 | 44.1°C | 25.6°C | 527 | Shower during charge |
| 27 Mar | 173 | 43.5°C | 25.2°C | 530 | Back-to-back showers |
| 29 Mar | 119 | 41.2°C | 24.8°C | 529 | Lower T1, weaker stratification |
| 31 Mar | 198 | 43.7°C | 25.6°C | 529 | Shower during charge |
| 01 Apr | 174 | 43.5°C | 25.0°C | 534 | Shower during charge |
| 03 Apr | 146 | 42.9°C | 25.6°C | 523 | Back-to-back showers |
| 03 Apr | 221 | 41.0°C | 25.1°C | 231 | Tap after back-to-back showers |
| 03 Apr | 170 | 42.3°C | 25.0°C | 529 | Shower during charge |

## Charging behaviour notes

### Two-phase cycle

1. lower cylinder heats while T1 stays flat
2. once `HwcStorageTemp` reaches the initial T1, crossover occurs and T1 rises

Reference note:

- heat-exchanger approach typically moves from negative at start to positive by end of charge

### Eco-mode behaviour by outside temperature

| Outside | Avg duration | Timeout rate | Interpretation |
|---|---|---|---|
| <2°C | 118 min | 95% | eco effectively fails |
| 2–5°C | 119 min | 89% | mostly incomplete |
| 5–8°C | 111 min | 53% | borderline |
| 8–12°C | 101 min | 23% | usually completes |
| 12°C+ | 86 min | 13% | generally fine |

### No-crossover interpretation

No-crossover charges are not automatically useless. The practical distinction is whether the event still serves the household and whether it steals a future heating/preheat window.

## Evening concurrent-draw examples

| Night | HwcS start | Draws during charge | HwcS end | Crossover | Thermal energy |
|---|---|---|---|---|---|
| 1 Apr 21:05 | 15.5°C | 60L shower | 41.5°C | no | ~10.2 kWh |
| 2 Apr 21:03 | 36.0°C | none | 45.0°C | yes | ~3.1 kWh |
| 3 Apr 21:04 | 26.0°C | 270L showers + tap | 39.5°C | no | ~10.2 kWh |

The “failed” charges above delivered more energy because people were using the hot water while the HP was charging.

## Household usage profile

| Person/group | Typical use | Approx volume |
|---|---|---|
| Jack | shower | ~30L |
| 3 girls | shower | ~70L each, sometimes ~100L |
| Son | bath + short shower | ~110L + ~30L |
| Everyone | taps | ~15L/day total |

Quiet days are materially lower than busy family days, so operational comfort margins should assume bursty demand rather than a flat average.

## Reference charge trace

Example: 21 Mar 2026, eco, 05:10–07:05.

| Time | HP Flow | HP Return | Heat kW | Elec W | T1 | T2 |
|---|---|---|---|---|---|---|
| 05:10 | 31°C | 30°C | 2.0 | 780 | 42.0 | 23.3 |
| 05:30 | 39°C | 37°C | 3.1 | 921 | 42.3 | 24.3 |
| 06:00 | 43°C | 41°C | 3.0 | 993 | 42.6 | 26.6 |
| 06:30 | 46°C | 44°C | 3.0 | 1039 | 43.4 | 29.7 |
| 07:00 | 48°C | 46°C | 2.9 | 1069 | 44.9 | 32.2 |
| 07:05 | 48°C | 46°C | 2.9 | 1072 | 45.2 | 32.4 |

Summary: 115 min, ~5.75 kWh thermal, ~1.92 kWh electrical, COP ~3.0.

## z2m-hub remaining-litres notes

The canonical operational summary now lives in [`../lat.md/domain.md`](../lat.md/domain.md). Extra algorithm notes preserved here:

- during charge, crossover promotes estimated remaining litres to full
- after a clean crossover, `effective_temp` follows T1
- after no-crossover, the T1/HwcStorage gap is used to interpolate likely practical capacity
- during standby, T1 decay is approximated linearly over time
- sharp HwcStorage crashes during draws act as lower-cylinder depletion signals, but T1 remains the practical comfort truth
