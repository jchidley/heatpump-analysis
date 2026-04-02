# DHW Cylinder Analysis

Analysis of the 300L Kingspan Albion cylinder, Multical/eBUS sensors, and heat pump charging. Based on sensor data from March–April 2026: 32+ charge cycles, 55+ draw events, 6 T1 inflection measurements at 2-second resolution.

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
| Cold feed | Dip pipe from 490mm connection to bottom (~0mm) |
| Coils | Twin coil (solar model), **both connected in series for HP** |
| Internal expansion | Air bubble at top (no external expansion vessel) |

### Twin coil configuration

The cylinder has two coils designed for solar (lower) + boiler (upper). In this installation, **both coils are connected in series** for the heat pump, giving double the heat exchange surface area.

### Measured connection heights

Measured from the outside bottom of the cylinder (actual internal positions are ~50mm lower, accounting for bottom insulation):

| Outside height (mm) | Internal height (mm) | Connection |
|---------------------|---------------------|------------|
| 420 | 370 | Bottom coil top (entry/exit at same height — U-shaped loop hangs down) |
| 540 | 490 | T2 sensor + cold water inlet (dip pipe runs to bottom ~0mm) |
| 1020 | 970 | Top coil (entry/exit) |
| 1580 | 1530 | T1 sensor + hot water draw-off |

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
| G1 | Dry stat pocket (solar — **VR10 NTC installed here**, ~600mm internal) | 465 |
| G2 | Dry stat pocket (high limit) | 1584 |

Reference datasheets in `docs/datasheets/`.

### Volume budget

| Zone | Height | Volume | Notes |
|---|---|---|---|
| Total water | 0→1907mm | **303L** | Entire cylinder contents (at 45°C) |
| Below draw-off | 0→1530mm | **243L** | Maximum drawable (geometric) |
| Above draw-off | 1530→1907mm | 60L | Trapped above outlet (hot but inaccessible) |
| Top coil → T1 | 970→1530mm | 89L | Main hot zone |
| HwcStorage → top coil | 600→970mm | 59L | Coil zone (upper) |
| T2 → HwcStorage | 490→600mm | 17L | Between sensors |
| Bottom coil → T2 | 370→490mm | 19L | Below T2 |
| Below bottom coil | 0→370mm | 59L | Coil loops down into this zone |

### Internal expansion (air bubble)

The Ultrasteel uses a floating baffle and captive air pocket for thermal expansion, eliminating the external expansion vessel. As water heats from 10°C to 45°C, it expands ~1% (3L), compressing the air space from ~46mm to ~25mm. The air/water interface at the top acts as a near-perfect insulator.

## Sensor infrastructure

### Sensor locations and data

| Sensor | Height | MQTT topic | Sample rate | Resolution |
|---|---|---|---|---|
| T1 (hot outlet) | 1530mm | `emon/multical/dhw_t1` | ~2s | 0.01°C |
| T2 (cold inlet) | 490mm | `emon/multical/dhw_t2` | ~2s | 0.01°C |
| HwcStorageTemp (VR10 NTC) | ~600mm | `ebusd/poll/HwcStorageTemp` | ~30s | 0.5°C |
| DHW flow rate | — | `emon/multical/dhw_flow` | ~2s | 1 L/h |
| DHW volume register | — | `emon/multical/dhw_volume_V1` | ~2s | 10L steps |
| Building circuit flow | — | `ebusd/poll/BuildingCircuitFlow` | ~30s | — |

InfluxDB 2 on pi5data (10.0.1.230:8086), bucket `energy`, org `home`.

### Cold inlet: dip pipe to the bottom

The cold water inlet connects at 490mm (T2 height) but is a **dip pipe** — it delivers all water to the bottom of the cylinder (~0mm), regardless of temperature. WWHR does not change the insertion point; it is the same cold feed after passing through the drain heat exchanger, connected via 6m of 15mm pipe. The ~3 minute delay at the start of showers before T2 rises is the transit time through this pipe plus WWHR warm-up, not a buoyancy effect.

### WWHR characterisation (19 March 2026 shower)

| Phase | T2 (post-WWHR) | Lift from 15.8°C mains |
|-------|----------------|----------------------|
| Start (drain cold) | 15.5°C | −0.3°C |
| 1 minute | 17.1°C | +1.3°C |
| 2 minutes | 19.7°C | +3.9°C |
| 3 minutes | 22.1°C | +6.3°C |
| **3.5 min (steady state)** | **24.8°C** | **+9.0°C** |

WWHR effectiveness: **41%**. Mains cold: 15.8°C (London, March). WWHR steady state: 25°C. Ramp-up: 3–3.5 minutes. Benefits: 32% energy saving, improved stratification (lower ΔT at inlet), reduced hot fraction at mixer (77% vs 84%).

## Bottom coil homogenises the entire lower cylinder

The bottom coil enters and exits at 370mm (same height — a U-shaped loop hanging downward). HwcStorageTemp sits at 600mm, just 230mm above. Evidence from the 31 March morning draw (70L shower):

| Time | HwcS | T2 | Event |
|---|---|---|---|
| 06:15 | 41.5° | 28.2° | Pre-draw: bottom zone uniformly warm |
| 06:20 | 40.0° | 25.3° | 4 min / ~30L: cold front approaching |
| 06:25 | 29.0° | 25.0° | 9 min / ~66L: **step-function crash** |
| 06:30 | 26.5° | 24.7° | Settling toward T2 |
| 06:44 | 26.5° | 24.5° | HwcS ≈ T2: entire bottom zone cold |

The crash is a **step function**, not gradual. This proves the zone below HwcS was at a **uniform temperature** before the draw — convective mixing from the U-shaped coil. During a charge, HwcStorageTemp represents the temperature of the entire 0→600mm zone (~95L), not just a point reading.

**There is no permanent dead zone.** After a charge, the entire bottom ~95L is at HwcStorageTemp. After a draw flushes the bottom with cold water, the zone becomes cold — but it's heated again by the next charge.

## Charge cycle physics

### Two-phase charging

Every charge has two distinct phases, visible in minute-by-minute data across 28 cycles:

1. **Below-T1 heating** — coils heat cold water in the lower cylinder. T1 is dead flat. HwcStorageTemp rises steadily toward T1_pre.

2. **Uniform heating** — begins the instant HwcStorageTemp crosses T1_pre (**crossover**). The entire cylinder is now at or above T1_pre. T1 starts rising immediately at ~0.1°C/min.

**The cylinder can only be called full once crossover is achieved.**

### Crossover evidence (28 charge cycles)

| Crossover? | Count | T1 behaviour | Cylinder state |
|---|---|---|---|
| Yes (HwcS ≥ T1_pre) | 16 | T1 rose +0.3° to +6.6° after crossover | **Full** |
| No (charge ended first) | 12 | T1 moved <0.2° total | **Not full** |

Representative traces:

```
23 Mar full charge (125 min):
  05:06–06:22  HwcS: 25→41.5°  T1: 42.0→42.0°  (76 min, T1 dead flat)
  06:24         HwcS crosses T1_pre=42.0°          ◀ CROSSOVER
  06:24–07:06  HwcS: 42→45°    T1: 42.4→45.5°  (42 min, T1 rising 0.1°/min)

22 Mar partial charge (120 min):
  05:10–07:10  HwcS: 24→41.5°  T1: 42.3→42.2°  (NEVER crossed, T1 unchanged)

28 Mar cold morning (119 min):
  05:10–06:06  HwcS: 24→33.5°  T1: 33.0→33.8°  (T1 actually DROPPED initially)
  06:06         HwcS crosses T1_pre=33.0°          ◀ CROSSOVER
  06:06–07:08  HwcS: 33.5→40°  T1: 33.8→40.4°  (uniform heating, but only to 40°C)
```

### Coil-driven destratification

During below-T1 heating, T1 can drop slightly (28 Mar: 33.0→30.9° before recovering). The coils create convective cells pulling cold water through the mid-section when the primary flow temperature is below T1. T1 only starts rising once the primary exceeds existing T1.

### Morning charge data: 21 March 2026, 05:10–07:05 UTC

| Time | HP FlowT | HP ReturnT | HP ΔT | Heat kW | Elec W | T1 | T2 |
|------|----------|-----------|-------|---------|--------|-----|-----|
| 05:10 (start) | 31°C | 30°C | 1°C | 2.0 | 780 | 42.0 | 23.3 |
| 05:30 | 39°C | 37°C | 2°C | 3.1 | 921 | 42.3 | 24.3 |
| 06:00 | 43°C | 41°C | 2°C | 3.0 | 993 | 42.6 | 26.6 |
| 06:30 | 46°C | 44°C | 2°C | 3.0 | 1039 | 43.4 | 29.7 |
| 07:00 | 48°C | 46°C | 2°C | 2.9 | 1069 | 44.9 | 32.2 |
| 07:05 (end) | 48°C | 46°C | 2°C | 2.9 | 1072 | 45.2 | 32.4 |

Duration: 115 min. Flow rate: 1.3 m³/h. Total heat: ~5.75 kWh. Electricity: ~1.92 kWh. **COP: 3.0**. Eco mode: constant ~2°C primary ΔT, front-loads high-COP operation.

### Heat exchanger approach temperature

| Time | HP Flow | T1 | Approach |
|------|---------|-----|----------|
| Start | 37.5°C | 42.2°C | −4.7°C (HP cooler than cylinder top) |
| Mid | 42.7°C | 42.1°C | +0.6°C (crossing over) |
| End | 48.2°C | 45.0°C | +3.2°C (excellent for indirect coil-in-coil) |

## No-crossover charges: thermocline physics

When the thermocline is **above the top coil** (>76L drawn from full), the coils heat water below it but **cannot push it down**. Heated water (~35–42°C) rises by buoyancy but is blocked by the hotter zone above.

```
  ┌──────────────────────────────────────┐  1530mm  T1 = 43.5°C
  │  HOT ZONE (unchanged by charge)     │         ← remaining = pre-charge value
  ├──────────── THERMOCLINE ────────────┤  ~1371mm (after 140L drawn)
  │  WARM ZONE (created by charge)      │         ← at HwcS_end temperature
  ├──────────────────────────────────────┤  970mm   Top coil
  │  COIL ZONE → HwcStorage → bottom    │         ← uniformly heated
  └──────────────────────────────────────┘  0mm
```

### The gap determines thermocline sharpness

| Gap (T1 − HwcS_end) | Thermocline state | Effect on remaining | Evidence |
|---|---|---|---|
| > 3°C | Sharp. Buoyancy barrier Δρ > 1.1 kg/m³ | **Unchanged** | 23 Mar: gap 4.0°, 50L draw crashed T1 by 2.5° |
| < 1.5°C | Dissolved. Mixing dominates | **Restored to full** at lower temp | 22 Mar: gap 0.6°, 70L draw, T1 stable |
| 1.5–3°C | Intermediate | **Depends on standby time** | 29 Mar: gap 2.7°, 6h standby → T1 stable for 80L |

Thermal diffusion blurs the thermocline: diffusion length = √(κ × t) where κ = 0.15 mm²/s. After 6h: ~57mm (matches thermocline thickness). After 8h: ~66mm (fully diffused). Modelled as `effective_gap = gap × exp(-hours/8)`.

## Usable volume

### Measured inflection volumes (2-second resolution)

The `dhw-sessions` CLI (`cargo run --bin heatpump-analysis -- dhw-sessions`) finds the exact volume at which T1 begins dropping, using rolling dT1/dV at native 2-second Multical resolution. It also classifies draws by type (bath/shower/tap) based on peak flow rate and tracks HwcStorageTemp during draws.

| Date | Cumulative (L) | T1 (°C) | T2 (°C) | Flow (L/h) | Context |
|---|---|---|---|---|---|
| 21 Mar | **177** | 44.3 | 25.8 | 464 | Full charge, WWHR showers |
| 23 Mar | **155** | 44.1 | 25.6 | 527 | Full charge, shower during charge |
| 27 Mar | **173** | 43.5 | 25.2 | 530 | Full charge, back-to-back showers |
| 29 Mar | **119** | 41.2 | 24.8 | 529 | Low T1 (no proper charge), weak stratification |
| 31 Mar | **198** | 43.7 | 25.6 | 529 | Full charge, shower during charge |
| 01 Apr | **174** | 43.5 | 25.0 | 534 | Full charge, shower during charge |

**From full charge at 45°C: 177–198L usable** (geometric maximum: 243L, plug flow efficiency ~81%). The recommended capacity for z2m-hub is autoloaded from InfluxDB: currently **198L**.

The mixing loss is caused by the two coil sets (370mm and 970mm) disrupting plug flow. The cold front broadens into a temperature gradient as it passes through 600mm of coil structures.

### What affects usable volume

- **Cylinder temperature**: lower T1 → weaker density contrast → earlier inflection (119L at 41°C vs 198L at 45°C)
- **Gap between draws**: longer gap → thermocline diffusion → slightly earlier inflection
- **Flow rate**: higher flow → more turbulence through coils → more mixing
- **WWHR vs raw mains**: different inlet temperatures change density contrast

### Draw type classification

| Type | Peak flow | Volume | Typical |
|---|---|---|---|
| Bath | ≥650 L/h | 100–150L | Son's bath, taps wide open |
| Shower | 350–650 L/h | 30–100L | 30L (Jack), 70L (girls), 100L (long) |
| Tap | <350 L/h | 10–20L | Kitchen/bathroom sink |

### Household usage (14-day data, everyone home)

| Metric | Value |
|---|---|
| Daily average | 171L (0.9 tanks) |
| Busiest days | 260–270L (1.3–1.4 tanks) |
| Showers per day (avg) | 2.2 |
| Typical pattern | 1 bath, 18 showers, 12 taps per week |

The 198L tank handles 2 normal showers (70+70=140L) or 1 long + 1 short (100+30=130L) between charges comfortably. The 13:00–16:00 Cosy window is long enough for 2 full recharges in eco mode, allowing draws during the window at Cosy rate.

### Draws during HP charging

`dhw_flow` is measured by the tap-side Multical meter, independent of the HP charging circuit. Draws during charging are real usage that depletes the cylinder and must be tracked. The `dhw-sessions` CLI marks these with `*` (during_charge flag). z2m-hub v0.2.1+ tracks them in real time.

### Draw rates and practical capacity

| Draw type | Total flow | Cold side | Hot fraction | Cylinder draw rate | WWHR? |
|-----------|-----------|----------|-------------|-------------------|-------|
| Shower | 7 L/min | 25°C (WWHR) | 77% | 5.4 L/min | Yes |
| Bath fill | ~12 L/min | 15.8°C (mains) | 84% | ~10 L/min | No |
| Sink | ~3 L/min | 15.8°C (mains) | 84% | ~2.5 L/min | No |

### Sink draws: context-dependent

After a charge (HwcS > 30°C): bottom zone is hot → all draws consume usable volume equally. After a shower has flushed the bottom (HwcS ≈ T2): sink draws recirculate cold water with no thermocline impact — only showers (>300 L/h) matter.

## Standby heat loss

### Measured: 19–20 March 2026 overnight (4h undisturbed)

| | 01:00 | 05:00 | Change |
|---|---|---|---|
| T1 (upper) | 43.4°C | 42.4°C | −1.0°C |
| T2 (mid) | 21.5°C | 22.0°C | +0.5°C (heat migrating down) |
| Room temp | 20.9°C | 20.9°C | — |

| Metric | Value |
|--------|-------|
| T1 drop rate | **0.25°C/hour** (σ=0.02, from 20 observations) |
| Average power loss | **13 W** |
| Projected daily loss | 0.3 kWh/day |
| Implied UA | 1.1 W/°C |
| Annual cost at COP 3.9 | ~£5/year |

The measured loss is 13W vs rated 93W because: only the top ~150L is hot (stratification), the air bubble insulates the top, and the surface-averaged temperature is much lower than the rated uniform 45°C test.

## Energy accounting

### Morning charge vs shower draws (21 March)

| | Value |
|---|---|
| HP thermal input (morning charge) | 5.75 kWh |
| Energy stored in usable hot zone (149L, 45−25°C) | 3.5 kWh |
| Energy stored in warm zone (154L, 28−15°C) | 2.3 kWh |
| Energy removed by showers (180L, 44.5−25°C) | 4.1 kWh |

The showers removed 117% of the usable hot energy — this is why the cylinder was fully depleted.

## Remaining-litres model (z2m-hub)

Implemented in z2m-hub on pi5data. Config in `/etc/z2m-hub.toml`. `full_litres` autoloaded from InfluxDB (`dhw_capacity` measurement, written by `dhw-sessions` CLI).

### State variables

```rust
struct DhwState {
    remaining: f64,
    volume_at_reset: f64,
    
    // Crossover tracking
    t1_at_charge_start: f64,
    crossover_achieved: bool,
    
    // Post-charge state
    t1_at_charge: f64,
    hwc_at_charge: f64,
    charge_complete_time: Instant,
    effective_temp: f64,
    
    // Thermocline tracking during draws
    hwc_pre_draw: f64,
    hwc_crash_detected: bool,
    t1_pre_draw: f64,
    
    was_charging: bool,
    boost_initiated: bool,
}
```

### Algorithm summary

**During charge** (bc_flow > 900): Watch for HwcStorage ≥ T1_at_charge_start. On crossover: `remaining = full_litres` (autoloaded from InfluxDB, currently 198L).

**After charge** (was_charging → !charging):
- Crossover achieved → `remaining = full_litres`, `effective_temp = T1`
- No crossover, gap < 1.5°C → thermocline dissolved, `remaining = full_litres` at lower temp
- No crossover, gap > 3.5°C → sharp thermocline, remaining unchanged from pre-charge
- Gap 1.5–3.5°C → interpolate; apply diffusion model during standby

**During draws** (tracked regardless of charging state — Multical is tap-side): Subtract volume drawn. Temperature corrections override if worse:
- HwcStorage crash >5°C → thermocline at 600mm, cap remaining at 148L minus further draws
- T1 drop >0.5° during draw → thermocline at T1, remaining ≤ 20L
- T1 drop >1.5° → remaining = 0
- When HwcS > 30°C: all draws cost equally. When HwcS ≈ T2: only showers cost.

**During standby**: `effective_T1 = T1_at_charge - 0.25 × hours`. Below 38°C → remaining = 0. 38–42°C → capacity scales linearly. Apply gap diffusion model.

**Boost**: same crossover logic as scheduled charges.

## Partial-charge volume estimation (future work)

With V_full validated at 198L (±10L, 6 measurements), partial charges can be modelled as two zones:

```
  ┌─────────────────────────────┐  1530mm
  │  HOT ZONE at T1             │  ← V_hot = V_remaining (pre-charge)
  ├──────── thermocline ────────┤
  │  WARM ZONE at HwcS_end      │  ← V_warm = V_full - V_hot
  └─────────────────────────────┘  0mm
```

Each zone's contribution to shower water:
```
shower_equivalent = V × (T_zone - T_cold) / (T_shower - T_cold)
```

Example (23 March after no-crossover, gap 4°C):
- Hot: 21L × (43.5 - 25) / (40 - 25) = 26L shower-equivalent
- Warm: 150L × (39.5 - 25) / (40 - 25) = 145L shower-equivalent
- Total: **171L shower-equivalent**

The complication: a sharp thermocline (gap >3°C) means the user experiences a temperature dip when the hot zone is exhausted and T1 crashes before the warm zone takes over. With a diffuse thermocline (gap <1.5°C or after standby), the transition is smooth.

Pending: (a) thermocline mixing profile from more crash events, (b) gap-diffusion model validated.

## Validation needs and data collection

### Parameters

| Parameter | Value | Source | Confidence |
|---|---|---|---|
| Full capacity | 177–198L | 2-second inflection, 6 events | **High** |
| Crossover condition | HwcS ≥ T1_pre | 32+ charge cycles, 100% | **Very high** |
| Volume above HwcStorage | 148L | Geometry | High |
| Standby T1 decay | 0.25°C/h | 20 observations, σ=0.02 | High |

### Highest-value next steps

1. **Summer repeat** — mains at ~18°C vs current ~15°C. Will affect WWHR effectiveness and density contrast.
2. **Continuous large draw test** — bath tap wide open after full charge, no WWHR, no gap. Expected 200–220L.
3. **Run `dhw-sessions` periodically** — `cargo run --bin heatpump-analysis -- dhw-sessions --days 14`. Writes to InfluxDB automatically; z2m-hub autoloads on restart.

### InfluxDB logging

z2m-hub should write to `dhw` measurement:
```
dhw remaining_litres=X,model_version=2,t1=Y,hwc_storage=Z,
    volume_drawn=W,charge_state="full|partial|standby",
    bottom_zone_hot=B,effective_t1=E,gap=G
```

Inflection detector writes to `dhw_inflection`:
```
dhw_inflection,category=capacity,crossover=true,draw_type=shower
    cumulative_volume=X,draw_volume=Y,gap_hours=Z,
    t1_start=A,t1_at_inflection=B,mains_temp=C,flow_rate=D,
    hwc_pre=E,hwc_min=F,hwc_drop=G
```

## SPA display improvements

Current: "Empty / Low / OK / Full" based on litres alone.

Improved:
- **Full** (>150L, T1≥44°C): green
- **OK** (40–150L, T1≥40°C): green, shows litres
- **Low** (<40L OR T1<42°C with draws): amber
- **Empty** (T1 dropped >1°C during draw): red
- **Stale** (>8h since charge): show litres with "~" prefix
- During charge: "Heating below" / "Heating uniformly" based on crossover
- After no-crossover: "Partially charged"
- Boost button: estimated time to crossover

## PHE + secondary return analysis (evaluated, not implemented)

A plate heat exchanger on the primary side with secondary return pump was evaluated and rejected:
- COP doesn't change (same Q, ṁ, Cp)
- T1 dip during below-T1 heating is only 0.3°C (negligible)
- PHE can only run for ~60 of 115 min (primary < T1 for the first 48 min)
- Maximum COP benefit: ~3–4%, saving ~£7–8/year — not worth the complexity
- The coil-in-coil is already 90–95% efficient

## DHW target temperature analysis

Cost per shower is nearly constant across 42–50°C: higher temp → worse COP but fewer litres per shower. The effects cancel to within 0.4p (5%). Current 45°C is optimal — ~1°C above the practical minimum for the household's hottest shower preference + bath margin. Standing losses at 45°C are near-minimum (13W).

## Historical context

From 181 cycles over 90 days of emoncms data (pre-InfluxDB):
- Start return temp: 37.3°C avg (31–44°C range)
- Typical cycle: 30–45 min (eco mode)
- Max flow temp: 53–55°C
- Schedule: ~05:15 + ~13:15

## Reference datasheets

| File | Contents |
|------|----------|
| `albion-ultrasteel-installation-manual.pdf` | Full installation & maintenance manual |
| `kingspan-ultrasteel-product-guide.pdf` | Product guide with dimensions for all models |
| `albion-ultrasteel-gotogasdocs.pdf` | Alternative installation instructions |
| `ultrasteel-plus-data-fiche.pdf` | ErP data fiche with AUXSN300ERP specs |
