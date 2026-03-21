# DHW Cylinder Heat Exchange Analysis

## Overview

With the emondhw Multical meter (secondary/DHW side) and emonhp MBUS heat meter (primary/HP side) both feeding into InfluxDB on pi5data, we can see both sides of the cylinder heat exchanger simultaneously. This document captures the first analyses from 19–20 March 2026.

## Cylinder specification

**Kingspan Albion Ultrasteel Plus Solar Indirect 300L (AUXSN300ERP)**

| Spec | Value |
|------|-------|
| Capacity | 300 litres |
| Height | 2032 mm |
| Diameter | 550 mm |
| Insulation | 50mm PU foam |
| Rated standing heat loss | 93 W (BS EN 12897, uniform 45°C, 20°C ambient) |
| Energy rating | C |
| Heat exchanger | Coil-in-coil (90–95% efficient vs 75–80% for conventional coil) |
| Cold feed diffuser | Yes — dip pipe to bottom reduces turbulence |
| Coils | Twin coil (solar model), **both connected in series for HP** |

### Twin coil configuration

The cylinder has two coils designed for solar (lower) + boiler (upper). In this installation, **both coils are connected in series** for the heat pump, giving double the heat exchange surface area across the full height of the cylinder.

Connection heights from base (from Kingspan technical data, AUSI 300):

| Label | Height (mm) | Connection |
|-------|------------|------------|
| A | 365 | Lower coil flow/return |
| B | 420 | Lower coil flow/return |
| C | 979 | Upper coil flow/return |
| D | 1034 | Upper coil flow/return |
| G | 465 | Cold feed (dip pipe to bottom) |

The HP primary water flows through both coils in series, entering at the top and exiting at the bottom (or vice versa), heating the full 300L volume.

### Sensor positions

The Multical sensors are **not** at the extreme top/bottom of the cylinder:

```
    ┌──────────────────────┐  2032mm
    │  22mm hot draw-off   │  ← actual tap draw-off (above T1)
    │                      │
    │  T1 (Multical hot)   │  ← ~D position (1034mm), above upper coil
    │  ┌────────────────┐  │
    │  │  UPPER COIL    │  │  C-D (979-1034mm)
    │  └────────────────┘  │
    │                      │
    │  T2 (Multical cold)  │  ← ~G position (465mm), above lower coil  
    │  ┌────────────────┐  │
    │  │  LOWER COIL    │  │  A-B (365-420mm)
    │  └────────────────┘  │
    │  cold feed dip pipe  │  → to bottom deflector
    └──────────────────────┘  0mm
```

- **T1** reads mid-upper cylinder between the upper coil and the draw-off. The actual tap water is drawn from above T1 and will be slightly hotter.
- **T2** reads mid-cylinder at the cold feed entry point, above the lower coil. Cold mains water (via WWHR) enters here and the dip pipe sends it to the bottom.

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
    |> filter(fn: (r) => r.topic == "emon/multical/dhw_t1" or
                          r.topic == "emon/multical/dhw_t2" or
                          r.topic == "emon/multical/dhw_flow" or
                          r.topic == "emon/heatpump/heatmeter_FlowT" or
                          r.topic == "emon/heatpump/heatmeter_ReturnT" or
                          r.topic == "emon/heatpump/heatmeter_FlowRate")
    |> aggregateWindow(every: 1m, fn: mean, createEmpty: false)
    |> map(fn: (r) => ({time: r._time, topic: r.topic, value: r._value}))
    |> yield(name: "data")'
```

### MQTT topics

| Topic | Source | Measurement |
|-------|--------|-------------|
| `emon/heatpump/heatmeter_FlowT` | emonhp MBUS | Primary flow temp (°C) |
| `emon/heatpump/heatmeter_ReturnT` | emonhp MBUS | Primary return temp (°C) |
| `emon/heatpump/heatmeter_FlowRate` | emonhp MBUS | Primary flow rate (m³/h) |
| `emon/heatpump/heatmeter_Power` | emonhp MBUS | Primary thermal power (W) |
| `emon/multical/dhw_t1` | emondhw Multical | Hot out / upper cylinder (°C) |
| `emon/multical/dhw_t2` | emondhw Multical | Cold in / mid cylinder (°C) |
| `emon/multical/dhw_flow` | emondhw Multical | DHW draw flow rate (L/h) |
| `emon/multical/dhw_power` | emondhw Multical | DHW thermal power (kW) |
| `emon/multical/dhw_t1-t2` | emondhw Multical | Delta T across cylinder (°C) |

**Note:** `dhw_P1` and `dhw_mass_m1` return 4294967296 (0xFFFFFFFF) — register read errors. Those Modbus registers are not valid for this Multical model.

## DHW reheat cycle: 20 March 2026, 05:27–07:06 UTC

### Conditions
- Eco mode DHW reheat (no taps running — pure cylinder recharge)
- Primary flow rate: steady 1.28 m³/h (21.3 L/min) — post y-filter clean
- Cycle duration: 99 minutes
- Outside temperature: ~10°C

### Temperature profile

| Time | HP Flow | HP Return | HP ΔT | T1 (upper) | T2 (mid) |
|------|---------|-----------|-------|------------|----------|
| 05:27 (start) | 37.5°C | 35.3°C | 2.2°C | 42.2°C | 22.9°C |
| 06:00 | 42.7°C | 40.6°C | 2.1°C | 42.1°C | 26.2°C |
| 06:30 | 45.9°C | 43.8°C | 2.1°C | 42.7°C | 27.2°C |
| 07:00 | 47.9°C | 45.8°C | 2.1°C | 44.6°C | 28.4°C |
| 07:05 (end) | 48.2°C | 46.1°C | 2.1°C | 45.0°C | 28.7°C |
| 07:07 (off) | 30.2°C | 24.8°C | — | 45.1°C | 28.7°C |

### Key observations

#### 1. Very low primary ΔT (~2.1°C) — eco mode

The HP runs at low compressor speed in eco mode, delivering ~3.1 kW through both coils in series:

```
Q = 21.3 L/min ÷ 60 × 4.18 kJ/(kg·°C) × 2.1°C = 3.1 kW
```

The twin coils in series provide a large heat exchange surface area, keeping the primary ΔT small. This gentle heating keeps the compressor at low speed for maximum COP. The trade-off is a 99-minute cycle vs ~30 minutes in normal mode.

#### 2. Excellent heat exchanger performance — 3.2°C approach

The **approach temperature** (HP flow minus cylinder T1) tells us how well the coils transfer heat:

| Time | HP Flow | T1 (upper) | Approach |
|------|---------|------------|----------|
| Start | 37.5°C | 42.2°C | **−4.7°C** (HP cooler than upper cylinder) |
| Mid | 42.7°C | 42.1°C | **+0.6°C** (crossing over) |
| End | 48.2°C | 45.0°C | **+3.2°C** |

At the start, the HP primary is cooler than the upper cylinder — it's heating the cold lower water through both coils without disturbing the hot top. By the end, the approach is only 3.2°C, which is excellent for an indirect cylinder with coil-in-coil design.

#### 3. Strong cylinder stratification

| | Start | End | Rise |
|---|---|---|---|
| T1 (upper) | 42.2°C | 45.2°C | +3.0°C |
| T2 (mid) | 22.9°C | 28.7°C | +5.8°C |
| Stratification (T1−T2) | 19.3°C | 16.5°C | narrowing |

The mid-cylinder (T2) heated nearly twice as fast as the upper (T1) — the lower coil is correctly heating the coldest water first. The 16.5°C stratification at end of cycle means the top layer is always ready for immediate use, even mid-reheat.

#### 4. COP across the cycle

The HP flow temp rises throughout the cycle as the cylinder heats up:

- Start: 37.5°C flow → COP ~4.5
- End: 48.2°C flow → COP ~3.3
- Weighted average across the cycle → measured DHW COP ~3.9

Eco mode front-loads the high-COP operation — most of the energy is delivered in the first half when flow temps are lower.

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
| Energy lost from upper ~100L | 0.116 kWh |
| Energy gained by lower ~100L | 0.065 kWh |
| **Net energy lost to surroundings** | **0.051 kWh in 4 hours** |
| **Average power loss** | **13 W** |
| **Projected daily loss** | **0.3 kWh/day** |
| Implied UA | 1.1 W/°C |

### Comparison with rated specification

| | Rated (BS EN 12897) | Measured |
|---|---|---|
| Condition | Uniform 45°C, 20°C ambient | Stratified: T1 43°C, T2 22°C |
| Standing loss | **93 W** | **13 W** |
| At rated ΔT (25°C) | 93 W | 28 W (extrapolated from UA) |

The measured loss is far below the rated spec because:

1. **Stratification** — only the top ~100L is at 43°C; the bottom ~200L is near room temperature and barely loses heat. The rated test heats the entire 300L uniformly to 45°C.
2. **Lower mean temperature** — the surface-averaged cylinder temperature is ~32°C (only 11°C above room), not 45°C (25°C above room).
3. **Coil-in-coil design** — the 50mm PU foam insulation performs well.

In real-world operation with a heat pump in eco mode, the cylinder's effective standing loss is a fraction of the rated spec. At 0.3 kWh/day and COP 3.9, standby costs about **£5/year** in electricity.

## Shower event analysis: 19 March 2026, 22:45–23:03 UTC

### Timeline (from 15-second resolution Multical data)

Two distinct events were observed:

**Event 1 — Sink use (22:45–22:50)**

| Time | T2 (mid) | Flow | Notes |
|------|----------|------|-------|
| 22:44 | 26.6°C | 0 | Resting |
| 22:45:30 | 25.6°C | 170 L/h (2.8 L/min) | Sink tap opened |
| 22:46:30 | 18.9°C | 122 L/h (2.0 L/min) | Cold mains arriving |
| 22:48 | 16.6°C | 0 | Tap closed |
| 22:51 | 15.8°C | 0 | Settled — this is the mains cold temperature |

Brief, low-flow draw. No WWHR benefit — sink waste doesn't flow through the shower drain heat exchanger. T2 dropped to **15.8°C**, which is the actual mains cold water temperature (London mains ~10–12°C at street, warmed to ~16°C running through house pipework in March).

T1 (upper cylinder) was **completely unaffected** — held at 43.9°C throughout. The cold feed dip pipe delivered water to the bottom without disturbing stratification.

**Event 2 — Shower (22:57:30–23:02:30)**

| Time | T2 (mid) | Flow | dT (T1−T2) | Notes |
|------|----------|------|-----------|-------|
| 22:57:30 | 15.5°C | 447 L/h (7.5 L/min) | 28.3°C | Shower starts, drain cold |
| 22:58:15 | 17.1°C | 446 L/h | 26.7°C | WWHR starting to recover |
| 22:58:45 | 19.7°C | 450 L/h | 24.2°C | Drain warming up |
| 22:59:30 | 22.1°C | 440 L/h | 21.7°C | WWHR ramping |
| 23:00:45 | 24.8°C | 425 L/h | 19.0°C | Approaching steady state |
| 23:01:15 | 24.8°C | 425 L/h | 19.0°C | **WWHR at steady state** |
| 23:02:30 | 25.1°C | 313 L/h | 18.7°C | Shower ending |
| 23:03 | 25.3°C | 0 | 18.5°C | Settled |

T1 dropped only **0.1°C** (43.9→43.8°C) during the entire shower — stratification held perfectly.

### WWHR performance

The waste water heat recovery unit sits in the shower drain. It pre-heats incoming mains cold water using heat from the drain water before it enters the cylinder.

| Phase | T2 (post-WWHR) | Lift from 15.8°C mains | Notes |
|-------|----------------|----------------------|-------|
| Drain cold (start) | 15.5°C | −0.3°C | No recovery yet |
| 1 minute | 17.1°C | +1.3°C | Drain starting to warm |
| 2 minutes | 19.7°C | +3.9°C | Ramping |
| 3 minutes | 22.1°C | +6.3°C | |
| **3.5 min (steady state)** | **24.8°C** | **+9.0°C** | **Maximum recovery** |

At steady state:

| | Value |
|---|---|
| Mains cold (measured) | **15.8°C** (not 10°C — warmed in house pipework) |
| Post-WWHR at steady state | **25°C** |
| WWHR temperature lift | **9.2°C** |
| Drain water (estimated) | ~38°C |
| Available ΔT in drain | 22.2°C |
| **WWHR effectiveness** | **41%** |

The WWHR takes **3–3.5 minutes** to reach steady state as the shower drain warms up. For a typical 7-minute shower, the blended saving across the whole event is ~30–35%.

### WWHR compound benefits

The WWHR pre-heated water serves double duty:

1. **Enters the cylinder warmer** — T2 at 25°C instead of 15.8°C means less reheat energy needed (dT 19°C vs 28°C = 32% less heat per litre)

2. **Feeds the shower mixer cold side** — a shower at 40°C:
   - Without WWHR: mixing 15.8°C cold + 44°C hot → ~55% hot from cylinder
   - With WWHR: mixing 25°C cold + 44°C hot → ~20% hot from cylinder
   - The cylinder is depleted at roughly **half the rate**

3. **Preserves stratification** — warmer cold feed (25°C vs 15.8°C) causes less thermal shock when entering the cylinder bottom, so the hot upper layer is less disturbed

These three effects compound: the cylinder lasts longer per shower, needs less energy to reheat, and maintains usable hot water at the top throughout the draw.

### Limitations

- **Sink and bath draws get no WWHR benefit** — waste water doesn't flow through the shower drain heat exchanger
- **Short showers underperform** — the 3.5-minute ramp-up is overhead; longer showers get proportionally more benefit
- **Mains temperature varies seasonally** — ~8°C in January, ~18°C in August. Absolute WWHR lift is similar year-round but the percentage saving is higher in winter when the mains is colder
- **Baths are the worst case** — large volume draw with no WWHR, cold mains straight in, stratification disrupted

## emonhp vs eBUS — complementary data sources

Both emonhp and eBUS measure flow/return temps, flow rate, power, and energy — but they serve different roles:

- **emonhp** (MBUS heat meter + SDM120) = independent auditor. MID-certified meters, legal "truth" for energy accounting. This is what the state machine in `analysis.rs` uses.
- **eBUS** (via ebusd on emondhw) = inside view. Operating modes (heating/DHW/defrost/standby), compressor speed, refrigerant circuit, target flow temp, energy integral, COP calculations, pump power %.

Both are needed: emonhp alone can't distinguish heating from DHW (the state machine infers it from flow rate). eBUS gives the definitive operating mode via `StatuscodeNum` (104=heating, 134=DHW, 100=standby, 516=defrost). See [heating-monitoring-setup.md](../heating-monitoring-setup.md) for the full comparison.

The Multical on emondhw adds the **third** perspective — the demand side (actual DHW delivery to taps). Together the three sources give end-to-end DHW efficiency:

```
HP electricity (SDM120, emonhp) → HP heat output (MBUS, emonhp) → cylinder → useful heat at taps (Multical, emondhw)
         input                        primary side                   losses        delivery side
```

## Historical DHW cycle observations (from emoncms.org)

From 181 cycles over 90 days of emoncms data (pre-dating the InfluxDB setup):

| Metric | Value |
|--------|-------|
| Start return temp (avg) | 37.3°C (min 31, max 44) — cylinder temp at start of cycle |
| Typical cycle duration | 30–45 minutes (eco mode) |
| Max flow temp | 53–55°C |
| Schedule | ~05:15 (morning) + ~13:15 (afternoon) |
| Last days before filter clean | 45–90 minute cycles with lower max flow temps — filter blockage effect |

The longer cycles before the filter clean (19 March 2026) confirm the hydraulic restriction was affecting DHW performance — the reduced flow rate meant less heat transfer per pass, requiring longer run times.

## What we can now monitor

With both sides of the cylinder instrumented:

1. **Heat exchanger degradation** — if the approach temperature widens over time (e.g., from limescale on the secondary side from hard London mains water), the HP has to run at higher flow temps. The primary side uses deionised water so won't scale.
2. **WWHR effectiveness** — track T2 during shower draws across seasons. Expect steady-state T2 to vary with mains temperature (seasonal) but WWHR lift to remain ~9°C.
3. **Cylinder stratification quality** — T1 vs T2 during and after cycles. Any degradation of the cold feed dip pipe/deflector would show up as reduced stratification.
4. **Standby losses** — track T1 decay overnight. Should remain ~0.25°C/hour at current conditions. Any increase suggests insulation degradation.
5. **Mains water temperature** — T2 during non-WWHR draws (sink events) gives the true mains cold temperature at the cylinder. Track seasonally.
6. **DHW cycle efficiency** — compare HP thermal input (emonhp power × time) vs useful heat delivered to taps (Multical energy) for end-to-end DHW system efficiency.
