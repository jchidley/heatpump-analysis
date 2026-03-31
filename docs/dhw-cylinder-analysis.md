# DHW Cylinder Heat Exchange Analysis

## Overview

With the emondhw Multical meter (secondary/DHW side) and emonhp MBUS heat meter (primary/HP side) both feeding into InfluxDB on pi5data, we can see both sides of the cylinder heat exchanger simultaneously. This document captures analyses from 19–21 March 2026, including a validated 1D stratification model.

**⚠ Corrections (31 March 2026):** Several findings in this document have been revised based on 12 days of additional sensor data. See `docs/dhw-improved-model.md` for the current model. Key corrections:
- **Cold inlet is a dip pipe to the bottom (~0mm)**, not insertion at T2 height. WWHR preheats the same cold feed; the 3-min delay is pipe transit (6m of 15mm pipe) + WWHR warm-up, not buoyancy insertion.
- **The 161L usable figure was an underestimate.** Re-analysis at 2-second resolution gives 177–183L from a full charge. The 165L geometric T2→T1 volume was a coincidence; the real geometric maximum (bottom to draw-off) is 243L.
- **The "dead zone" below the bottom coil is heated during charges.** The U-shaped coil loops down into it. HwcStorageTemp crash data confirms the entire 0→600mm zone is uniformly heated.
- **The crossover (HwcStorage ≥ T1_pre) is the definitive "full" signal**, confirmed across 28 charge cycles.

## Cylinder specification

**Kingspan Albion Ultrasteel Plus Solar Indirect 300L (AUXSN300ERP)**

| Spec | Value |
|------|-------|
| Capacity | 300 litres |
| Overall height | 2032 mm |
| Overall diameter | 550 mm |
| Insulation | 50mm PU foam |
| Internal height | ~1932 mm |
| Internal diameter | ~450 mm |
| Cross-section | 0.159 m² |
| Volume per mm height | 0.159 L/mm |
| Rated standing heat loss | 93 W (BS EN 12897, uniform 45°C, 20°C ambient) |
| Measured standing heat loss | 13 W (stratified, eco mode) |
| Energy rating | C |
| Heat exchanger | Coil-in-coil (90–95% efficient vs 75–80% for conventional coil) |
| Cold feed diffuser | Yes — dip pipe to bottom reduces turbulence |
| Coils | Twin coil (solar model), **both connected in series for HP** |
| Internal expansion | Air bubble at top (no external expansion vessel) |

### Twin coil configuration

The cylinder has two coils designed for solar (lower) + boiler (upper). In this installation, **both coils are connected in series** for the heat pump, giving double the heat exchange surface area.

### Measured connection heights

Measured from the outside bottom of the cylinder (actual internal positions are ~50mm lower, accounting for bottom insulation):

| Outside height (mm) | Internal height (mm) | Connection |
|---------------------|---------------------|------------|
| 420 | 370 | Bottom coil top (entry/exit at same height) |
| 540 | 490 | T2 sensor + cold water inlet (dip pipe to bottom) |
| 1020 | 970 | Top coil (entry/exit) |
| 1580 | 1530 | T1 sensor + hot water draw-off |

Note: the Kingspan technical data (AUSI 300) gives slightly different heights (A=365, B=420, C=979, D=1034, G=465). The measured heights above are from the physical cylinder.

### Factory connection positions (from Kingspan product guide)

For the 300L Solar Indirect model (AUXSN300ERP), measured from outside bottom:

| Letter | Connection | Height (mm) |
|--------|-----------|-------------|
| A | 22mm Cold feed (dip pipe to diffuser at bottom) | 530 |
| B | 22mm Hot water outlet | 1584 |
| C | Immersion heater (3kW, 1¾" thread) | 1060 |
| D1 | 22mm Boiler coil connections (upper coil) | 1005 |
| D2 | 22mm Solar coil connections (lower coil) | 410 |
| E | ½" BSP T&P relief valve (10 bar / 90°C) | 1552 |
| F | 22mm Secondary return (≥210L models only) | 1519 |
| G1 | Dry stat pocket (solar) | 465 |
| G2 | Dry stat pocket (high limit) | 1584 |

The secondary return (F) at 1519mm is 65mm below the hot outlet (B) at 1584mm — both near the top of the cylinder.

Reference datasheets in `docs/datasheets/`.

### Cylinder profile and zone volumes

At operating temperature (45°C), water expands ~1%, compressing the internal air bubble to ~25mm. The water surface sits at ~1907mm internal height.

```
         ┌─────────────────────┐  2032mm outside (1932mm internal)
         │▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒│
         │▒▒ AIR + DOME ▒▒▒▒▒▒│  air bubble + cylinder dome (not usable water)
         │▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒│
  1580mm │── T1 + draw-off ────│  draw-off and T1 at same height
         │                     │
         │  89L                │  30% — heated by convection from top coil
         │                     │
  1020mm │── top coil ─────────│  50% height, 154L below
         │                     │
         │  76L between coils  │  25% — partially heated zone
         │                     │
   540mm │── T2 + cold inlet ──│  25% height, 78L below
         │  19L                │   6%
   420mm │── bottom coil top ──│  19% height, 59L below
         │                     │
         │  59L dead zone      │  20% — dip pipe delivers cold here
         └─────────────────────┘  0mm (outside base)
```

| Zone | Height (mm) | Volume (L) | % of 300L | Typical temp |
|------|-------------|-----------|-----------|-------------|
| Air bubble (expansion) | 25 | 4 | — | — |
| Above T1/draw-off (air + dome) | 377 | — | — | air bubble |
| Top coil → T1 | 560 | 89 | 30% | ~45°C |
| T2 → top coil | 480 | 76 | 25% | 25–32°C |
| Bottom coil → T2 | 120 | 19 | 6% | 22–32°C |
| Below bottom coil (DEAD) | 370 | 59 | 20% | 13–22°C |

The **usable hot water** (above the top coil, heated by convection) is approximately **149L** — about half the nominal 300L. However, the draw-off point is at the same height as T1 (1580mm), so the 60L above T1 is just the air bubble and the domed top of the cylinder — not accessible water. The effective usable volume before tap temperature drops is **161L** — the T1 inflection point measured by flow integration at 2-second resolution. This is 97.6% of the 165L geometric volume between T2 and T1; the 4L difference represents the thermocline thickness (~25mm).

### Internal expansion (air bubble)

The Ultrasteel uses a floating baffle and captive air pocket for thermal expansion, eliminating the external expansion vessel. As water heats from 10°C to 45°C, it expands ~1% (3L), compressing the air space from ~46mm to ~25mm. The air/water interface at the top acts as a near-perfect insulator — no conductive heat loss upward through metal — contributing to the very low measured standby losses.

## Measurement infrastructure

### Data access

InfluxDB 2 on pi5data (10.0.1.230:8086), bucket `energy`, org `home`.

```bash
# Get the API token:
ssh jack@pi5data "grep token ~/monitoring/telegraf/telegraf.conf"

# Query example:
INFLUX_TOKEN="..."
curl -s "http://10.0.1.230:8086/api/v2/query?org=home" \
  -H "Authorization: Token $INFLUX_TOKEN" \
  -H "Content-Type: application/vnd.flux" \
  -d 'from(bucket:"energy")
    |> range(start: -2h)
    |> filter(fn: (r) => r._measurement == "emon" and r.source == "multical")
    |> filter(fn: (r) => r.field == "dhw_t1" or r.field == "dhw_t2" or r.field == "dhw_flow")
    |> aggregateWindow(every: 1m, fn: mean, createEmpty: false)'
```

### MQTT topics

| Topic | Source | Measurement |
|-------|--------|-------------|
| `emon/heatpump/heatmeter_FlowT` | emonhp MBUS | Primary flow temp (°C) |
| `emon/heatpump/heatmeter_ReturnT` | emonhp MBUS | Primary return temp (°C) |
| `emon/heatpump/heatmeter_FlowRate` | emonhp MBUS | Primary flow rate (m³/h) |
| `emon/heatpump/heatmeter_Power` | emonhp MBUS | Primary thermal power (W) |
| `emon/heatpump/electric_Power` | emonhp SDM120 | HP electrical consumption (W) |
| `emon/multical/dhw_t1` | emondhw Multical | T1 Cylinder Top — hot water outlet, 1580mm (°C) |
| `emon/multical/dhw_t2` | emondhw Multical | T2 Mains Inlet — cold water in post-WWHR, 540mm (°C) |
| `ebusd/poll/HwcStorageTemp` | VR 10 NTC via VWZ AI SP1 | Cylinder Temp — dry stat pocket, just above bottom coil, ~600mm. VRC 700 charging trigger (°C) |
| `emon/multical/dhw_flow` | emondhw Multical | DHW draw flow rate (L/h) |
| `emon/multical/dhw_power` | emondhw Multical | DHW thermal power (kW) |
| `emon/multical/dhw_volume_V1` | emondhw Multical | Cumulative DHW volume (L) |

**Note:** `dhw_P1` and `dhw_mass_m1` return 4294967296 (0xFFFFFFFF) — register read errors. Those Modbus registers are not valid for this Multical model.

## Stratification model (validated)

### The draw-off model — 97% accuracy

Validated against two shower events on 21 March 2026 (70L + 110L = 180L total from Multical volume register).

**Key insight:** WWHR-preheated water enters at ~25°C, which is buoyancy-neutral with the existing warm zone at the same temperature. It doesn't push from the very bottom — it effectively inserts at the ~25°C isotherm (approximately T2 height, 490mm internal). From there, the warm/hot water column above gets pushed upward as plug flow. The sharp thermocline between the warm zone (~30°C) and the hot zone (~45°C) rises toward T1.

| Parameter | Value | Source |
|-----------|-------|--------|
| Volume from T2 (490mm) to T1 (1530mm) | 165 L | Geometry |
| Volume drawn when T1 step change started | **~161 L** | Flow integration at 2s resolution |
| Multical volume register resolution | 10 L | Register increments in 10L steps |
| **Model accuracy** | **97.6%** | 161L measured vs 165L geometric prediction |

The step change in T1 (from 44.2°C dropping at 0.01°C/min to 0.15°C/min) confirms a thin, sharp thermocline — consistent with the research literature finding of 50–100mm thermocline thickness in well-designed vertical cylinders. Thermal diffusion during the 55-minute gap between showers was negligible (~1mm diffusion length).

### How the model works

During a shower draw:
1. Hot water exits the top of the cylinder (draw-off at 1580mm)
2. Cold water enters at 540mm, travels via dip pipe to the bottom
3. With WWHR, incoming water is ~25°C — buoyant relative to the dead zone below but neutral with the warm zone above the lower coil
4. The effective insertion point is at the 25°C isotherm (~490mm, T2 level)
5. Above this point, water moves upward as plug flow
6. The hot/warm thermocline (originally at ~970mm, top coil height) rises at:
   - Rate = draw_rate / cross_section = 420 L/h ÷ 0.159 m² = 44 mm/min
7. T1 at 1530mm sees the thermocline after (1530-490) / 44 = ~24 min of continuous drawing

### The WWHR paradox

Counter-intuitively, WWHR **reduces** the number of shower-minutes before T1 drops:

| | With WWHR | Without WWHR |
|---|---|---|
| Cold side temperature | 25°C | 15.8°C |
| Hot fraction at shower mixer | 77% | 84% |
| Hot draw rate from cylinder | 5.4 L/min | 5.9 L/min |
| Volume to T1 step change | 165 L | 243 L |
| **Minutes until tap temp drops** | **31 min** | **41 min** |

But WWHR halves the **energy** removed per session (5.1 kWh vs 10.1 kWh). The effect is:
- Without WWHR, 15.8°C water sinks below everything and pushes from 0mm — 243L must be displaced before T1 sees it
- With WWHR, 25°C water inserts at 490mm — only 165L must be displaced
- But without WWHR the shower mixer draws 10% more hot water per minute (colder cold side)

The practical difference is marginal (42 vs 51 min — both far exceed typical usage). **WWHR's real value is energy saving, not capacity extension.** It also dramatically improves stratification quality by reducing the inlet-to-tank temperature differential from 29°C to 6°C, increasing the Richardson number and preserving the sharp thermocline.

## DHW charge cycle analysis

### Morning charge: 21 March 2026, 05:10–07:05 UTC

Eco mode charge, cylinder warm from previous day.

| Time | HP FlowT | HP ReturnT | HP ΔT | Heat kW | Elec W | T1 | T2 |
|------|----------|-----------|-------|---------|--------|-----|-----|
| 05:10 (start) | 31°C | 30°C | 1°C | 2.0 | 780 | 42.0 | 23.3 |
| 05:30 | 39°C | 37°C | 2°C | 3.1 | 921 | 42.3 | 24.3 |
| 06:00 | 43°C | 41°C | 2°C | 3.0 | 993 | 42.6 | 26.6 |
| 06:30 | 46°C | 44°C | 2°C | 3.0 | 1039 | 43.4 | 29.7 |
| 07:00 | 48°C | 46°C | 2°C | 2.9 | 1069 | 44.9 | 32.2 |
| 07:05 (end) | 48°C | 46°C | 2°C | 2.9 | 1072 | 45.2 | 32.4 |

- Duration: 115 minutes
- Flow rate: 1.3 m³/h (21.3 L/min) — DHW pump speed
- Total heat: ~5.75 kWh, total electricity: ~1.92 kWh, COP: 3.0
- T1 rose 3.2°C (42.0→45.2), T2 rose 9.1°C (23.3→32.4)

The primary ΔT stayed constant at ~2°C throughout — eco mode runs at low compressor speed with the large twin-coil surface area keeping ΔT small. The return temp (bottom of coil pair exit) rose from 30°C to 46°C, tracking the water temperature around the coils.

### Afternoon charge: 21 March 2026, 13:12–14:55+ UTC

Eco mode charge after two showers depleted the cylinder.

| Time | HP FlowT | HP ReturnT | T1 | T2 |
|------|----------|-----------|-----|-----|
| 13:12 (start) | 33°C | 30°C | 43.2 | 21.3 |
| 13:30 | 37°C | 34°C | 43.1 | 22.1 |
| 14:00 | 42°C | 39°C | 43.0 | 26.6 |
| 14:20 | 45°C | 42°C | 42.8 | 30.9 |
| 14:25 | 46°C | 43°C | 42.2 | 31.6 |

**Key finding: T1 drops during early charging.** T1 fell from 43.2°C to 42.2°C in the first 75 minutes despite 3+ kW being pumped in. This is "coil-driven destratification" — the coils create convective cells that partially mix the stratified layers. The primary (33–42°C) is cooler than T1 (43°C) early in the cycle, and the circulation pulls cold water from the depleted bottom zone (T2=21°C) upward through the coil region, slightly cooling the zone above. T1 only starts rising once the primary flow temp exceeds the existing T1 temperature (~14:20 when FlowT hits 45°C).

### What the primary return temp reveals

The HP return temp is the temperature at the bottom of the coil pair (exit point) — the coldest water the coils encounter.

| Condition | ReturnT | T2 | Gap | Interpretation |
|-----------|---------|-----|-----|----------------|
| Morning charge start | 30°C | 23°C | 7°C | Coil zone warmer than below-coil zone |
| Morning charge end | 46°C | 32°C | 14°C | Coil zone fully heated; below-coil still warm |
| Afternoon charge start | 30°C | 21°C | 9°C | Sharp thermal boundary at coil base |
| During heating mode | 30→28°C | 26→21°C | 4–7°C | Passive proxy for mid-cylinder temp |

The ReturnT ≠ T2 because the coil sits above T2 and heats its immediate surroundings. The gap between ReturnT and T2 shows the sharp thermal boundary at the bottom of the coil zone — water around the coil is warmer than water just below it.

## Shower events: 21 March 2026

### Back-to-back showers with 55-minute gap

| | Shower 1 | Shower 2 |
|---|---|---|
| Time | 10:19–10:29 | 11:24–11:40 |
| Duration | 10 min | 16 min |
| Flow rate | ~410 L/h | ~420 L/h |
| Volume from cylinder (Multical) | 70 L | 110 L |
| T1 before | 44.6°C | 44.3°C |
| T1 after | 44.5°C | 43.5°C |
| T1 behaviour | Barely moved (−0.1°C) | Flat 13 min, then **step change** |
| T2 during draw | 26→21→26°C (WWHR ramp) | 25→22→26°C (WWHR ramp) |
| T2 post-draw | 26°C | 25→13°C (sink at 11:46) |

The T1 step change marks the thermocline reaching the T1 sensor position. The Multical volume register showed this at "~160L" but the register only increments in 10L steps, too coarse to pinpoint. Flow rate integration at **2-second resolution** (trapezoidal, from `dhw_flow` in L/h) pinpoints the inflection at **~161L**.

#### Multical volume register vs flow integration

The Multical `dhw_volume_V1` register is cumulative but updates in **10-litre increments**. This means:
- It's reliable for total-volume accounting across charge cycles (error < 10L)
- It's too coarse to pinpoint thermocline transitions (which need ~1L precision)
- For the Flux task tracking remaining hot water, flow integration provides 1L precision between register steps, while the register anchors through any data gaps

Flow rate integration (`dhw_flow` at ~2s sample interval, trapezoidal rule) cross-checks well: 189.4L integrated vs 190L from the register for the same draw period. At full resolution, the T1 inflection is clear:

| Integrated volume | T1 | dT1/dL (°C per litre) | Phase |
|---|---|---|---|
| 0–153L | 44.58→44.22°C | −0.002 | Stable (background standby decay) |
| 153–161L | 44.22→44.20°C | −0.004 | First hint |
| **161–168L** | **44.20→44.15°C** | **−0.010** | **Thermocline arriving at T1** |
| 168–176L | 44.15→44.03°C | −0.023 | Thermocline passing T1 |
| 176–183L | 44.03→43.89°C | −0.027 | Accelerating decline |
| 183L+ | 43.89→43.54°C | −0.035+ | Full decline |

The inflection at 161L where dT1/dL jumps from background noise (−0.002) to −0.010 was initially masked by 1-minute aggregation (which pointed to ~165L). The 2-second resolution data shows the true transition starts at **161L** — 97.6% of the 165L geometric prediction. The 4L difference is likely the thermocline thickness (~25mm at 0.159 L/mm).

### Sink draw: 11:46 (post-showers)

Brief sink use at 104 L/h caused T2 to crash from 25°C to 13.4°C — raw mains temperature without WWHR. This confirmed the mains temperature and showed the cylinder bottom was fully depleted after the two showers.

## Practical usage scenarios

The usable capacity is 161L from the cylinder before the tap temperature starts dropping. Different draw types consume this budget at different rates because baths don't benefit from WWHR (waste goes to the drain, not through the shower heat exchanger).

### Draw rates from the cylinder

| Draw type | Total flow | Cold side temp | Hot fraction | Cylinder draw rate | WWHR? |
|-----------|-----------|---------------|-------------|-------------------|-------|
| Shower | 7 L/min | 25°C (WWHR) | 77% | 5.4 L/min | Yes |
| Bath fill | ~12 L/min | 15.8°C (mains) | 84% | ~10 L/min | No |
| Sink | ~3 L/min | 15.8°C (mains) | 84% | ~2.5 L/min | No |

### How far does 161L go?

| Scenario | Cylinder draw | Remaining | Status |
|----------|--------------|-----------|--------|
| One 7-min shower | ~38L | 127L | ✅ Plenty left |
| Two 10-min showers | ~108L | 57L | ✅ OK |
| One 16-min shower | ~86L | 79L | ✅ OK |
| **Bath (100L at tap)** | **~84L** | **81L** | ✅ OK alone |
| **Bath + 8-min shower** | **~127L** | **38L** | ⚠️ Tight — works if no prior draws |
| Bath + 10-min shower | ~138L | 27L | ⚠️ Just within limit |
| Earlier shower + bath + shower | ~38+84+43 = **165L** | **−4L** | ❌ T1 step change mid-shower |
| Two baths | ~168L | **−3L** | ❌ Exceeds capacity |

This matches real-world experience: a bath followed by a shower works if nobody else has showered that day. If someone showered earlier, the bath + shower combination pushes past the 161L limit and the second person notices the temperature dropping.

### Why the dhw-auto-trigger matters for this scenario

The bath + shower combination is exactly the use case the trigger is designed for. A bath takes ~8 minutes to fill — sustained flow above 200 L/h for long enough to trigger. The HP starts charging during or just after the bath fill, feeding heat back into the cylinder before the shower starts. Without the trigger, the next person waits for the scheduled off-peak charge.

## Energy accounting

### Morning charge vs shower draws

| | Value |
|---|---|
| HP thermal input (morning charge) | 5.75 kWh |
| Energy stored in usable hot zone (149L, 45−25°C) | 3.5 kWh |
| Energy stored in warm zone (154L, 28−15°C) | 2.3 kWh |
| Energy removed by showers (180L, 44.5−25°C) | 4.1 kWh |
| Deficit (removed > usable hot) | −0.6 kWh |

The showers removed 117% of the usable hot energy (above the warm zone baseline). This is why the cylinder was depleted — the remaining 2.3 kWh in the warm zone was at too low a temperature for shower delivery.

## Standby heat loss: 19–20 March 2026 overnight

### Clean measurement period

After a shower draw settled at ~23:04, the cylinder sat undisturbed from 01:00 to 05:00 (4 hours, no draws, T2 stable).

| | 01:00 | 05:00 | Change |
|---|---|---|---|
| T1 (upper) | 43.4°C | 42.4°C | −1.0°C |
| T2 (mid) | 21.5°C | 22.0°C | +0.5°C (heat migrating down) |
| Room temp | 20.9°C | 20.9°C | — |

T2 **rose** during standby — heat migrating internally from the hot upper cylinder to the cooler mid-section. This is internal redistribution, not loss to the room.

### Measured loss

| Metric | Value |
|--------|-------|
| T1 drop rate | 0.25°C/hour |
| **Net energy lost to surroundings** | **0.051 kWh in 4 hours** |
| **Average power loss** | **13 W** |
| **Projected daily loss** | **0.3 kWh/day** |
| Implied UA | 1.1 W/°C |

### Comparison with rated specification

| | Rated (BS EN 12897) | Measured |
|---|---|---|
| Condition | Uniform 45°C, 20°C ambient | Stratified: T1 43°C, T2 22°C |
| Standing loss | **93 W** | **13 W** |

The measured loss is far below the rated spec because:

1. **Stratification** — only the top ~150L is hot; the bottom ~150L is near room temperature and barely loses heat
2. **Air bubble** — the air/water interface at the top insulates better than metal-to-air conduction
3. **Lower mean surface temperature** — the surface-averaged cylinder temperature is ~32°C (11°C above room), not 45°C (25°C above room)

At 0.3 kWh/day and COP 3.9, standby costs about **£5/year** in electricity.

## Shower event analysis: 19 March 2026, 22:45–23:03 UTC

### Sink use (22:45–22:50) — mains temperature baseline

Brief, low-flow draw (170 L/h). No WWHR benefit — sink waste doesn't flow through the shower drain heat exchanger. T2 dropped to **15.8°C**, which is the actual mains cold water temperature (London mains ~10–12°C at street, warmed to ~16°C running through house pipework in March). T1 was completely unaffected — the cold feed dip pipe delivered water to the bottom without disturbing stratification.

### Shower (22:57:30–23:02:30) — WWHR characterisation

| Phase | T2 (post-WWHR) | Lift from 15.8°C mains |
|-------|----------------|----------------------|
| Drain cold (start) | 15.5°C | −0.3°C |
| 1 minute | 17.1°C | +1.3°C |
| 2 minutes | 19.7°C | +3.9°C |
| 3 minutes | 22.1°C | +6.3°C |
| **3.5 min (steady state)** | **24.8°C** | **+9.0°C** |

| WWHR metric | Value |
|---|---|
| Mains cold (measured) | 15.8°C |
| Post-WWHR steady state | 25°C |
| WWHR temperature lift | 9.2°C |
| Estimated drain water temp | ~38°C |
| **WWHR effectiveness** | **41%** |
| Ramp-up time | 3–3.5 minutes |

T1 dropped only **0.1°C** during the entire shower — stratification held perfectly.

### WWHR compound benefits

1. **Energy saving** — cold feed at 25°C instead of 15.8°C means 32% less reheat energy per litre
2. **Stratification preservation** — inlet-to-tank ΔT reduced from 29°C to 6°C, dramatically improving the Richardson number. This is why our thermocline is so sharp.
3. **Shower mixer effect** — at 40°C shower temp, WWHR reduces the hot fraction from 84% to 77%, drawing less hot water per minute

### Limitations

- Sink and bath draws get no WWHR benefit — waste water doesn't flow through the shower drain heat exchanger
- Baths are the worst case: no WWHR means 84% hot fraction vs 77% for showers, depleting the cylinder faster
- 3.5-minute ramp-up is overhead; longer showers get proportionally more benefit
- Mains temperature varies seasonally (~8°C January, ~18°C August)

## DHW reheat cycle: 20 March 2026, 05:27–07:06 UTC

### Temperature profile

| Time | HP Flow | HP Return | HP ΔT | T1 (upper) | T2 (mid) |
|------|---------|-----------|-------|------------|----------|
| 05:27 (start) | 37.5°C | 35.3°C | 2.2°C | 42.2°C | 22.9°C |
| 06:00 | 42.7°C | 40.6°C | 2.1°C | 42.1°C | 26.2°C |
| 06:30 | 45.9°C | 43.8°C | 2.1°C | 42.7°C | 27.2°C |
| 07:00 | 47.9°C | 45.8°C | 2.1°C | 44.6°C | 28.4°C |
| 07:05 (end) | 48.2°C | 46.1°C | 2.1°C | 45.0°C | 28.7°C |

### Heat exchanger performance — approach temperature

| Time | HP Flow | T1 | Approach |
|------|---------|-----|----------|
| Start | 37.5°C | 42.2°C | **−4.7°C** (HP cooler than upper cylinder) |
| Mid | 42.7°C | 42.1°C | **+0.6°C** (crossing over) |
| End | 48.2°C | 45.0°C | **+3.2°C** |

At the start, the HP primary is cooler than the upper cylinder — it's heating the cold lower water without disturbing the hot top. The 3.2°C approach at end of cycle is excellent for an indirect coil-in-coil design.

### COP across the cycle

- Start: 37.5°C flow → COP ~4.5
- End: 48.2°C flow → COP ~3.3
- Weighted average: ~3.9

Eco mode front-loads the high-COP operation — most energy is delivered in the first half when flow temps are lower.

## emonhp vs eBUS — complementary data sources

- **emonhp** (MBUS heat meter + SDM120) = independent auditor. MID-certified, legal "truth" for energy accounting. Used by the state machine in `analysis.rs`.
- **eBUS** (via ebusd on pi5data) = inside view. Operating modes, compressor speed, refrigerant circuit. **Note:** `StatuscodeNum` is unreliable for DHW detection — code 134 appears during both off/frost standby AND active DHW. Use `BuildingCircuitFlow` (> 900 L/h = DHW, 780–900 = heating, < 100 = off) for state classification.
- **Multical** (on emondhw) = demand side. Actual DHW delivery to taps.

Together: HP electricity → HP heat output → cylinder → useful heat at taps.

## Building a 1D model

The data supports a multi-node 1D stratified tank model with these validated components:

**Inputs:**
- Cylinder geometry: 300L, 450mm Ø, 1932mm internal height, 25mm air gap
- 10–20 horizontal nodes, each ~100mm = 15.9L
- Coil positions: lower 370mm, upper 970mm (both in series)
- T1 sensor: 1530mm, T2 sensor: 490mm

**Physics per node:**
- Conduction between adjacent nodes: k_water × A / Δx
- Convection from coil: buoyant plume rising off coil surface (empirical from charge data)
- Draw-off: plug flow, cold water inserted at buoyancy-neutral height (not always bottom)
- WWHR: incoming water at 25°C inserts at ~490mm (T2 level), not at 0mm
- Standby loss: UA = 1.1 W/°C (measured)
- Coil destratification: mixing during charge when primary < T1 (observed in afternoon cycle)

**Validation datasets (all from 19–21 March 2026):**

| Dataset | What it validates |
|---------|------------------|
| Morning charge (T1, T2, FlowT, ReturnT, 115 min) | Charge model, COP curve |
| Afternoon charge (depleted cylinder) | Coil destratification effect |
| Shower 1 (70L) + Shower 2 (110L) with 55 min gap | Draw-off plug flow, thermocline speed |
| T1 step change at 161L (flow integration at 2s) vs 165L model | **97.6% accuracy** on thermocline position |
| Overnight standby (4 hours undisturbed) | UA calibration, internal heat migration |
| Sink draw (no WWHR) | Mains temperature baseline |
| Shower WWHR ramp-up (15-second resolution) | WWHR effectiveness curve |
| T1 drop during afternoon charge | Coil-driven mixing coefficient |

## Historical DHW cycle observations (from emoncms.org)

From 181 cycles over 90 days of emoncms data (pre-dating the InfluxDB setup):

| Metric | Value |
|--------|-------|
| Start return temp (avg) | 37.3°C (min 31, max 44) |
| Typical cycle duration | 30–45 minutes (eco mode) |
| Max flow temp | 53–55°C |
| Schedule | ~05:15 (morning) + ~13:15 (afternoon) |

## What we can now monitor

1. **Heat exchanger degradation** — approach temperature widening over time (limescale from hard London mains)
2. **WWHR effectiveness** — track T2 steady-state during showers across seasons
3. **Stratification quality** — T1 step change sharpness during draws; should remain at <100mm thermocline thickness
4. **Standby losses** — T1 decay rate overnight (baseline 0.25°C/hour)
5. **Mains temperature** — T2 during sink draws (no WWHR), track seasonally
6. **Coil destratification** — T1 drop during early charge cycles; indicates how much mixing the coils cause
7. **Thermocline model validation** — draw volume at T1 step change should consistently match 161L (±10%)
8. **DHW remaining** — live InfluxDB metric (see below)

## DHW remaining litres — live tracking

**Update (March 2026):** The original InfluxDB Flux task has been **disabled** — replaced by DHW tracking in z2m-hub (`~/github/z2m-hub/`), which polls ebusd directly via TCP, detects charge completion (scheduled → 161L, manual boost → +50%), and tracks usage via Multical volume register. z2m-hub writes `dhw.remaining_litres` to InfluxDB. The approach described below is the historical Flux task implementation for reference.

### How it works

1. Finds the last DHW charge event via eBUS (originally used `StatuscodeNum == 134`, but z2m-hub now uses `BuildingCircuitFlow` > 900 L/h as the definitive DHW indicator)
2. Gets the Multical cumulative volume (`dhw_volume_V1`) at charge end and now → `drawn_reg` (ground truth, 10L steps)
3. Finds when the register last changed value (last reading with a different V1)
4. Integrates `dhw_flow` since that last register step → `frac` (0–9.9L interpolation within current 10L window)
5. Computes: `remaining = 161 - (drawn_reg + frac)`
6. Writes `dhw.remaining_litres` (smooth, 1L precision) and `dhw.remaining_register` (ground truth, 10L steps) to the `energy` bucket

### Volume register as ground truth, flow for interpolation

The Multical `dhw_volume_V1` register is the **ground truth** for total draw volume:
- Captures every drop regardless of sampling gaps
- Monotonically increasing — differences are always reliable
- No integration error accumulation
- Cross-checks with flow integration to <1% (189.4L integrated vs 190L register for a 3-hour draw period)

However, the register only updates in **10L increments**, producing a staircase in Grafana. Flow rate (`dhw_flow`) provides smooth 1L interpolation between register steps:

```
drawn = register_delta + clamp(flow_since_last_register_step, 0, 9.9)
```

This ensures:
- **Register anchors every 10L step** — flow can never drift beyond one step
- **Flow fills in between steps** — smooth curves during draws
- **Gaps are handled** — if flow data is missing, the register still provides the correct 10L-resolution value
- **No cumulative drift** — flow accumulator resets at every register step

### Sink vs shower draws

Not all litres are equal in their impact on T1 (draw-off temperature):

- **Shower draws**: WWHR preheats inlet water to ~25°C, which enters at buoyancy-neutral height (~T2 level, 490mm). Pushes the thermocline upward as plug flow. Each litre directly reduces the hot water column above T1.
- **Sink draws**: Cold mains (~14°C) enters at the bottom via the dip pipe. Fills the dead zone below the coils first. Has minimal impact on T1 until the entire bottom zone is displaced.

On 21 March, ~10L of intermittent sink use (16:22–17:15) caused zero change in T1 (held at 45.2°C) while T2 crashed to 13.6°C (raw mains). This confirms sink draws consume from the bottom dead zone (59L below the lower coil).

**For the remaining-litres metric**, we track total cumulative volume regardless of draw type. This is conservative for sink-heavy use (actual usable capacity slightly higher) but accurate for shower-heavy use (the common case). The 161L figure was validated under shower conditions with WWHR at 2-second resolution.

### Grafana query

```flux
from(bucket: "energy")
  |> range(start: v.timeRangeStart, stop: v.timeRangeStop)
  |> filter(fn: (r) => r._measurement == "dhw" and r._field == "remaining_litres")
```

### DHW charge detection via eBUS

The eBUS `StatuscodeNum` values relevant to DHW:

| Code | Status | Meaning |
|------|--------|---------|
| 100 | Standby | Idle |
| 101 | Heating_Prerun | Pump starting for heating |
| 104 | Heating_Compressor_active | Space heating |
| 107 | Heating_Overrun | Post-heating circulation |
| **134** | **Off/frost standby** | **⚠ Also appears during DHW — unreliable for DHW detection. Use BuildingCircuitFlow instead.** |
| 516 | Deicing_active | Defrost cycle |

These are available in InfluxDB as `ebusd_poll.StatuscodeNum` (numeric, from ebusd-poll.sh) and `ebusd.RunDataStatuscode` (string, from ebusd's own MQTT publishing).

## PHE + secondary return analysis (evaluated, not implemented)

### Concept

A plate heat exchanger (PHE) on the primary HP side with a secondary pump circulating DHW water back into the cylinder via the secondary return (F, 1519mm). Primary path: HP outlet → PHE → top coil → bottom coil → HP return. The goal was to improve efficiency by reducing the approach temperature between flow temp and cylinder temp.

### Why it doesn't help

1. **COP doesn't change.** The HP targets 45°C flow and outputs ~4kW regardless. The PHE changes *where* the heat goes (top vs coils) but not the total heat or the HP's operating point. Return temp = Flow - Q/(ṁ×Cp) — unchanged because Q, ṁ, and Cp are the same.

2. **The T1 dip is only 0.3°C.** T1 drops from 42.3°C to 42.0°C in the first hour due to coil-driven destratification. Nobody notices 0.3°C at the tap, and the charge runs at 05:15 when nobody is drawing water.

3. **The end state is identical.** After 115 minutes, T1 = 45.3°C with or without the PHE.

4. **The PHE can only run for ~60 min of a 115-min charge.** For the first 48 minutes, the primary flow temp (27→42°C) is *below* T1 (42°C). Running the PHE pump during this period would inject cooler-than-T1 water at the top, destroying stratification.

5. **Maximum COP benefit: ~3-4%** from reducing the approach temperature by 2°C. Saves ~£7-8/year. The PHE, pump, plumbing, control logic, and fouling risk from hard London mains water cost more than this.

### Conclusion

The coil-in-coil heat exchanger is already 90-95% efficient. The PHE adds complexity for negligible benefit. The equipment may be useful for another application.

## DHW target temperature analysis

### Cost per shower is nearly constant across temperature range

Analysis of the trade-off between tank temperature, COP, and hot water draw rate:

| Tank °C | COP | Litres per shower | Showers per charge | Cost per shower |
|---------|-----|-------------------|-------------------|-----------------|
| 42 | 3.67 | 62 | 2.6 | 7.7p |
| 43 | 3.60 | 58 | 2.8 | 7.7p |
| 44 | 3.53 | 55 | 2.9 | 7.7p |
| **45** | **3.46** | **52** | **3.1** | **7.7p** |
| 46 | 3.39 | 50 | 3.2 | 7.8p |
| 48 | 3.25 | 46 | 3.5 | 7.9p |
| 50 | 3.11 | 42 | 3.8 | 8.0p |

Assumptions: 10-min shower at 40°C, 7 L/min, WWHR cold side 25°C, Cosy off-peak 14.63p/kWh. Cost per shower varies by only 0.4p (5%) across the entire 40-51°C range.

**Why:** higher temp → worse COP → more electricity per charge, BUT higher temp → less hot water per shower (more cold mixed in) → more showers per charge. The two effects almost perfectly cancel. People always mix to their comfortable temperature regardless of tank setting.

### Minimum temperature constraint

The tank must be hot enough that hot water arriving at the mixer (after ~1.5°C pipe loss) exceeds the desired shower temperature:

- Person who likes 38°C showers → tank ≥ 40°C
- Person who likes 40°C showers → tank ≥ 42°C
- Person who likes 42°C showers → tank ≥ 44°C

The bath + shower scenario (100L bath at 40°C without WWHR, then a 10-min shower) requires ≥ 43°C to have enough volume remaining.

### Increasing temperature doesn't help either

If capacity is already sufficient (161L is more than enough for one person), raising the temperature just means:
- More showers per charge — but the HP already skips charges when the cylinder is hot enough
- Higher standing losses (13W → 14W at 48°C, pure waste)
- Longer charge cycles at worse COP toward the end

### Current 45°C is optimal

The 45°C target (`HwcTempDesired`) is ~1°C above the practical minimum (44°C for the hottest-shower person + bath margin). It provides:
- Comfortable showers for everyone (up to 42°C at the showerhead after pipe losses)
- Bath + shower capacity with margin
- Near-minimum standing losses
- The HP already skips unnecessary charges when water isn't used

The biggest efficiency wins are already banked: WWHR (41% saving), Cosy off-peak charging, and low standing losses (13W vs 93W rated).

## Reference datasheets

Kingspan Albion cylinder documentation in `docs/datasheets/`:

| File | Contents |
|------|----------|
| `albion-ultrasteel-installation-manual.pdf` | Full installation & maintenance manual |
| `kingspan-ultrasteel-product-guide.pdf` | Product guide with dimensions for all models |
| `albion-ultrasteel-gotogasdocs.pdf` | Alternative installation instructions |
| `ultrasteel-plus-data-fiche.pdf` | ErP data fiche with AUXSN300ERP specs |
