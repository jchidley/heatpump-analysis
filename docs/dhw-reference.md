# DHW Reference Data

Domain reference for the 300L cylinder, sensors, charging behaviour, and measured usage. Loaded by agents when reasoning about DHW capacity, charging, or cylinder physics. For operating policy and decisions, see [DHW plan](dhw-plan.md).

## Cylinder specification

**Kingspan Albion Ultrasteel Plus Solar Indirect 300L (AUXSN300ERP)**

| Spec | Value |
|---|---|
| Capacity | 300L total, 243L geometric max drawable (below draw-off at 1530mm) |
| Internal dimensions | ~450mm diameter, ~1932mm internal height |
| Insulation | 50mm PU foam |
| Heat exchanger | Twin coil-in-coil — solar (lower) + boiler (upper) **both connected in series for HP**, doubling surface |
| Cold feed | **Dip pipe** from 490mm connection to bottom (~0mm) — all cold water enters at bottom regardless of WWHR |
| Internal expansion | Air bubble at top (floating baffle, no external vessel). ~46mm→25mm as water heats 10→45°C |
| Standing heat loss | 13W measured (vs 93W rated — stratification + air bubble insulates top) |
| T1 decay rate | 0.25°C/h (σ=0.02, 20 observations). Measured: T1 43.4→42.4°C over 4h |
| Annual standby cost | ~£5/year at COP 3.9 |

### Connection heights (outside mm / internal mm)

| Outside | Internal | Connection |
|---|---|---|
| 420 | 370 | Bottom coil top (U-shaped loop hangs down into 0–370mm zone) |
| 465 | 415 | Dry stat pocket (VR10 NTC — `HwcStorageTemp`) |
| 540 | 490 | T2 sensor + cold water inlet (dip pipe to bottom ~0mm) |
| 1020 | 970 | Top coil (entry/exit) |
| 1580 | 1530 | T1 sensor + hot water draw-off |

### Sensor detail

| Sensor | Rate | Resolution | Notes |
|---|---|---|---|
| T1 (`emon/multical/dhw_t1`) | ~2s | 0.01°C | **Authoritative**. Kamstrup calls it "Inlet" (from meter perspective) — counterintuitive but correct |
| T2 (`emon/multical/dhw_t2`) | ~2s | 0.01°C | Kamstrup "Outlet". Reads ~25°C (WWHR) during showers, ~11°C (mains) during baths |
| HwcStorageTemp (`ebusd/poll/HwcStorageTemp`) | ~30s | 0.5°C | VRC 700 trigger (5K hysteresis = 40°C). **Misleading after draws**: reads 13°C with 100L of 45°C above |
| DHW flow (`emon/multical/dhw_flow`) | ~2s | 1 L/h | Tap-side, independent of HP |
| DHW volume (`emon/multical/dhw_volume_V1`) | ~2s | 10L steps | Cumulative |

### Bottom coil behaviour

Enters and exits at 370mm (U-shaped loop). During charges, convective mixing makes entire 0–600mm zone (~95L) uniform. Evidence: HwcStorageTemp crashes as step function during large draws (41.5→29°C in <5 min), not gradually.

## WWHR

Waste Water Heat Recovery on shower drain (baths bypass). Effectiveness: **41%**.

| Phase | T2 (post-WWHR) | Lift from 15.8°C mains |
|---|---|---|
| Start (drain cold) | 15.5°C | −0.3°C |
| 1 min | 17.1°C | +1.3°C |
| 2 min | 19.7°C | +3.9°C |
| 3 min | 22.1°C | +6.3°C |
| **Steady state (3.5 min)** | **24.8°C** | **+9.0°C** |

~3 min delay = transit time through 6m of 15mm pipe + WWHR warm-up.

### Draw rates and hot fractions

| Draw type | Total flow | Cold side | Hot fraction | Cylinder draw rate |
|---|---|---|---|---|
| Shower (WWHR) | 7 L/min | 25°C | 77% | 5.4 L/min |
| Bath fill (mains) | ~12 L/min | 15.8°C | 84% | ~10 L/min |
| Sink (mains) | ~3 L/min | 15.8°C | 84% | ~2.5 L/min |

## Capacity measurements (inflection analysis)

| Date | Usable (L) | T1 (°C) | T2 (°C) | Flow (L/h) | Context |
|---|---|---|---|---|---|
| 21 Mar | 177 | 44.3 | 25.8 | 464 | Full charge, WWHR showers |
| 23 Mar | 155 | 44.1 | 25.6 | 527 | Shower during charge |
| 27 Mar | 173 | 43.5 | 25.2 | 530 | Back-to-back showers |
| 29 Mar | 119 | 41.2 | 24.8 | 529 | Low T1 (41°C), weak stratification |
| 31 Mar | 198 | 43.7 | 25.6 | 529 | Shower during charge |
| 01 Apr | 174 | 43.5 | 25.0 | 534 | Shower during charge |
| 03 Apr | 146 | 42.9 | 25.6 | 523 | Back-to-back showers |
| 03 Apr | **221** | 41.0 | 25.1 | 231 | Tap after back-to-back showers |
| 03 Apr | 170 | 42.3 | 25.0 | 529 | Shower during charge |

Capacity depends on T1: lower T1 → weaker density contrast → earlier inflection.

## Charging behaviour

### Two-phase cycle

1. **Below-T1 heating**: coils heat cold water in lower cylinder. T1 dead flat. HwcStorageTemp rises toward T1
2. **Crossover** (HwcStorageTemp ≥ T1 at charge start): entire cylinder now heated. T1 rises ~0.1°C/min

Heat exchanger approach: starts −4.7°C (HP cooler than T1), mid +0.6°C, end +3.2°C.

### Eco mode by outside temperature

| Outside | Avg duration | Hit 120-min timeout | Assessment |
|---|---|---|---|
| <2°C | 118 min | 95% | Nearly all incomplete |
| 2–5°C | 119 min | 89% | Mostly incomplete |
| 5–8°C | 111 min | 53% | Borderline |
| 8–12°C | 101 min | 23% | Usually completes |
| 12°C+ | 86 min | 13% | Fine |

### No-crossover thermocline

| Gap (T1 − HwcS at end) | Thermocline | Capacity effect |
|---|---|---|
| >3°C | Sharp (buoyancy barrier) | Unchanged from pre-charge |
| <1.5°C | Dissolved (mixing) | Restored to full at lower temp |
| 1.5–3°C | Intermediate | Interpolated. Diffuses over ~8h |

Thermal diffusion: κ = 0.15 mm²/s. After 6h: ~57mm. After 8h: ~66mm (fully diffused). z2m-hub: `effective_gap = gap × exp(-hours/8)`.

## Evening concurrent draws (observed)

| Night | HwcS start | Draws during charge | HwcS end | Crossover | Thermal energy |
|---|---|---|---|---|---|
| 1 Apr 21:05 | 15.5°C | 60L shower | 41.5°C | ✗ | ~10.2 kWh |
| 2 Apr 21:03 | 36.0°C | None | 45.0°C | ✓ | ~3.1 kWh |
| 3 Apr 21:04 | 26.0°C | 270L showers + tap | 39.5°C | ✗ | ~10.2 kWh |

"Failed" charges delivered 3× more energy. Mild nights: morning top-up fits before preheat. Cold nights: may steal preheat window.

## Household usage profile

| Person | Draw type | Typical volume | Peak flow |
|---|---|---|---|
| Jack | Shower | 30L | ~525 L/h |
| 3 girls | Shower | 70L each (100L occasionally) | ~530 L/h |
| Son | Bath + short shower | 110L + 30L | ~730 L/h (bath) |
| Everyone | Taps | ~15L/day | <350 L/h |

Quiet days: 40–120L. Not everyone showers daily; busiest days have 3–4 showers.

## Reference charge trace (21 Mar 2026, eco, 05:10–07:05)

| Time | HP Flow | HP Return | Heat kW | Elec W | T1 | T2 |
|---|---|---|---|---|---|---|
| 05:10 (start) | 31°C | 30°C | 2.0 | 780 | 42.0 | 23.3 |
| 05:30 | 39°C | 37°C | 3.1 | 921 | 42.3 | 24.3 |
| 06:00 | 43°C | 41°C | 3.0 | 993 | 42.6 | 26.6 |
| 06:30 | 46°C | 44°C | 3.0 | 1039 | 43.4 | 29.7 |
| 07:00 | 48°C | 46°C | 2.9 | 1069 | 44.9 | 32.2 |
| 07:05 (end) | 48°C | 46°C | 2.9 | 1072 | 45.2 | 32.4 |

115 min, 5.75 kWh thermal, 1.92 kWh electrical. **COP 3.0**.

### Energy accounting (21 Mar)

| Item | Value |
|---|---|
| HP thermal input | 5.75 kWh |
| Usable hot zone (149L, 45−25°C) | 3.5 kWh |
| Warm zone (154L, 28−15°C) | 2.3 kWh |
| Showers removed (180L, 44.5−25°C) | 4.1 kWh (117% of usable — why cylinder fully depleted) |

## z2m-hub remaining-litres algorithm

- **During charge** (bc_flow > 900): watch for HwcStorage ≥ T1 at charge start. On crossover: `remaining = full_litres`
- **After charge (crossover)**: `remaining = full_litres`, `effective_temp = T1`
- **After charge (no crossover)**: gap < 1.5°C → full at lower temp. Gap > 3.5°C → unchanged. 1.5–3.5°C → interpolate + diffusion
- **During draws**: subtract Multical volume. HwcStorage crash >5°C → cap at 148L. T1 drop >0.5°C → ≤20L. T1 drop >1.5°C → 0
- **Standby**: `effective_T1 = T1_at_charge - 0.25 × hours`. Below 38°C → 0. 38–42°C → linear scale
