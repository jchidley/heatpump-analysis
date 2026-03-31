# DHW Improved Remaining-Litres Model

Date: 31 March 2026  
Based on: 12 days of sensor data (19–31 March 2026), 28 charge cycles, 206 draw events.

## The key physical insight

A DHW charge has **two distinct phases**, visible in every charge cycle:

1. **Below-T1 heating** — the coils heat cold water in the lower cylinder. T1 is dead flat. HwcStorageTemp rises steadily from ~25°C toward T1_pre. The hot zone volume isn't changing; only the cold zone is warming up.

2. **Uniform heating** — begins the instant HwcStorageTemp crosses T1_pre. Now the entire cylinder is at or above T1_pre. T1 starts rising immediately (~0.1°C/min). All 161L are being heated uniformly.

**The cylinder can only be called full once HwcStorageTemp has passed T1_pre.** Before that, the coils are still reheating the depleted lower zone — the thermocline between hot (T1 level) and cold (below coils) hasn't been eliminated yet.

### Why no crossover means no thermocline movement

The coils heat water between 370mm and 970mm (top coil). During a draw, the thermocline (sharp hot→cold boundary) rises from T2 height (490mm) at 0.159 L/mm:

```
  After   0L drawn: thermocline at  490mm (at T2)         ← full
  After  76L drawn: thermocline at  970mm (at top coil)   ← critical boundary
  After 110L drawn: thermocline at 1182mm
  After 140L drawn: thermocline at 1371mm
  After 161L drawn: thermocline at 1503mm (at T1)         ← empty
```

When the thermocline is **above the top coil** (>76L drawn), the coils heat water below the thermocline. The heated water (~35–42°C) rises by buoyancy but **cannot penetrate the thermocline** because the hot water above (~43–45°C) is less dense. The warm water pools between the coil zone and the thermocline:

```
  ┌──────────────────────────────────────┐  1530mm  T1 = 43.5°C
  │  HOT ZONE (unchanged by charge)     │         ← This volume = remaining
  │  T ≈ T1_pre                         │
  ├──────────── THERMOCLINE ────────────┤  ~1371mm (after 140L drawn)
  │  WARM ZONE (created by charge)      │
  │  T ≈ HwcS_end (39.5°C)             │         ← Coil output pools here
  ├──────────────────────────────────────┤  970mm   Top coil
  │  COIL ZONE (active heating)         │
  ├──────────────────────────────────────┤  600mm   HwcStorageTemp
  │  T2 zone                            │
  ├──────────────────────────────────────┤  490mm   T2
  │  Dead zone                          │
  └──────────────────────────────────────┘  0mm
```

**Result: remaining capacity after a no-crossover charge = remaining capacity before the charge.** The charge warmed the cold zone (benefiting the next charge) but did not create more usable hot water.

### But the gap matters: sharp vs diffuse thermocline

The temperature gap between HwcS_end and T1 determines whether the thermocline is a real barrier or just a gentle gradient:

| Gap (T1 − HwcS_end) | Thermocline state | Effect on remaining | Evidence |
|---|---|---|---|
| > 3°C | Sharp. Buoyancy barrier Δρ > 1.1 kg/m³ | **Unchanged** — charge can't penetrate | 23 Mar: gap 4.0°, 50L draw crashed T1 by 2.5° |
| < 1.5°C | Dissolved. Δρ < 0.6 kg/m³, mixing dominates | **Restored to ~161L** at the lower temp | 22 Mar: gap 0.6°, 70L draw, T1 stable |
| 1.5–3°C | Intermediate. Blurs over time via diffusion | **Depends on standby duration** | 29 Mar: gap 2.7°, but 6h standby → T1 stable for 80L |

Thermal diffusion blurs the thermocline over time. Diffusion length = √(κ × t) where κ = 0.15 mm²/s for water:
- After 1h: ~23mm (thinner than the thermocline)
- After 6h: ~57mm (comparable to thermocline thickness of 25–50mm)
- After 8h: ~66mm (thermocline fully diffused)

The 29 Mar case proves this: 2.7°C gap should have been a barrier, but after 6 hours of standby the thermocline had diffused enough that 80L of draw passed through with T1 changing only 0.1°C.

### Evidence from 28 charge cycles

| Crossover? | Count | T1 behaviour | Cylinder state |
|---|---|---|---|
| Yes (HwcS ≥ T1_pre) | 16 | T1 rose +0.3° to +6.6° after crossover | **Full** — all 161L usable at T1_end |
| No (charge ended first) | 12 | T1 moved <0.2° total | **Not full** — hot zone at original T1, cold zone partially reheated |

Minute-by-minute traces confirm this is a sharp transition, not gradual:

```
23 Mar full charge:
  05:06–06:22  HwcS: 25→41.5°  T1: 42.0→42.0°  (76 min, T1 dead flat)
  06:24         HwcS crosses T1_pre=42.0°          ◀ CROSSOVER
  06:24–07:06  HwcS: 42→45°    T1: 42.4→45.5°  (42 min, T1 rising 0.1°/min)

22 Mar partial charge:
  05:10–07:10  HwcS: 24→41.5°  T1: 42.3→42.2°  (120 min, NEVER crossed)
  → model says 161L, reality is pre-existing hot zone unchanged

28 Mar cold morning:
  05:10–06:06  HwcS: 24→33.5°  T1: 33.0→33.8°  (56 min, T1 actually DROPPED)
  06:06         HwcS crosses T1_pre=33.0°          ◀ CROSSOVER
  06:06–07:08  HwcS: 33.5→40°  T1: 33.8→40.4°  (62 min, uniform heating)
  → Full volume (161L) but only at 40°C — marginally usable
```

The T1 dip during below-T1 heating (28 Mar: T1 dropped from 33.0° to 30.9° before recovering) is the coil-driven destratification documented in `dhw-cylinder-analysis.md` — convection from the coils pulls cold water through the cylinder mid-section.

## The bottom coil homogenises the entire lower cylinder

The bottom coil enters and exits at 370mm (same height — a U-shaped loop hanging downward into the "dead zone"). HwcStorageTemp sits at 600mm, just 230mm above the coil connections.

**Evidence from the 31 March morning draw (70L shower):**

| Time | HwcS | T2 | Gap | Event |
|---|---|---|---|---|
| 06:15 | 41.5° | 28.2° | +13.3° | Pre-draw: bottom zone uniformly warm |
| 06:20 | 40.0° | 25.3° | +14.7° | 4 min / ~30L: cold front approaching |
| 06:25 | 29.0° | 25.0° | +4.0° | 9 min / ~66L: **step-function crash** |
| 06:30 | 26.5° | 24.7° | +1.8° | Settling toward T2 |
| 06:44 | 26.5° | 24.5° | +2.0° | HwcS ≈ T2: entire bottom zone cold |

The crash is a **step function**, not a gradual decline. If there were a temperature gradient in the bottom zone (e.g., 41° at 600mm grading to 25° at 0mm), HwcS would decline smoothly as progressively cooler water was displaced past the sensor. Instead, HwcS held at 40–41° for 4 minutes, then crashed 12° in 5 minutes. This proves the zone below HwcS was at a **uniform temperature** — exactly what convective mixing from a U-shaped coil predicts.

**Implication:** During a charge, HwcStorageTemp represents the temperature of the entire 0→600mm zone (~95L), not just a point reading. The coil heats the "dead zone" directly because it loops down into it. There is no permanently cold dead zone — after a charge, the entire bottom ~95L is at HwcStorageTemp.

**Revised sink draw model:** The original assumption that 59L below the coil is a "dead zone" where sink draws are free was wrong. After a charge, the bottom zone is hot — every litre drawn (sink or shower) displaces hot water and moves the thermocline. The "free sink" effect only applies when the bottom zone is already cold (HwcS ≈ T2, i.e., after a prior shower has flushed the bottom).

## Revised cylinder geometry

### Cold inlet: dip pipe to the bottom

The cold water inlet connects at 490mm (T2 height) but is a **dip pipe** — it delivers water to the bottom of the cylinder (~0mm), regardless of temperature. WWHR does not change the insertion point; it is the same cold feed after passing through the drain heat exchanger, connected via 6m of 15mm pipe. The ~3 minute delay at the start of showers before T2 rises is the transit time through this pipe plus WWHR warm-up, not a buoyancy effect.

The original `dhw-cylinder-analysis.md` incorrectly described WWHR-heated water as "inserting at the 25°C isotherm (~490mm)". In reality, all cold water enters at the bottom and pushes upward as plug flow.

### Volume budget

| Zone | Height | Volume | Notes |
|---|---|---|---|
| Total water | 0→1907mm | **303L** | Entire cylinder contents |
| Below draw-off | 0→1530mm | **243L** | Maximum drawable (geometric) |
| Above draw-off | 1530→1907mm | 60L | Trapped above outlet (hot but inaccessible) |
| Coil zone | 370→970mm | 95L | Two coil sets — causes mixing during draws |
| Bottom zone | 0→600mm | 95L | Uniformly heated by bottom coil (no dead zone) |

### Usable volume: 177–183L, not 161L

The original 161L was measured at 2-second resolution from a specific pair of WWHR showers (21 March). Re-analysis of the same event and three additional T1 crash events at 2-second resolution gives:

- **From full charge (T1≥44°C): 177–183L** (two measurements)
- **From cooler cylinder (T1=41°C): 129L** (one measurement)
- **After no-crossover charge: 30L** (confirms model, not capacity)
- **Geometric maximum: 243L** — plug flow efficiency ~75%

The 82L mixing loss (243L geometric − ~180L actual) is caused by the two coil sets (370mm and 970mm) disrupting plug flow. The cold front broadens into a temperature gradient as it passes through 600mm of coil structures, reaching T1 earlier than a sharp plug front would.

Usable volume depends on conditions — see the inflection detector results for correlations with T1, T2, flow rate, and gap between draws.

## What's wrong with the current model

The z2m-hub (`~/github/z2m-hub/src/main.rs`) uses pure volume subtraction:

```
After scheduled charge:  remaining = 161
After boost:             remaining += 80.5 (50% of 161)
During draws:            remaining -= (volume_now - volume_at_reset)
```

**Failure modes observed in the data:**

| Failure | Example | Impact |
|---------|---------|--------|
| No crossover detection | 22 Mar 06:02 ended at HwcS=36° (never crossed T1_pre=42.3°) — model claims 161L | Overestimates: hot zone was never restored |
| Partial charges not detected | 12 of 28 charges ended without crossover — model claims 161L each time | Systematic overestimate |
| Split charges (charge interrupted, resumed) | 22 Mar 06:02 + 07:10 — two charges, model resets to 161L twice | Masks partial state |
| Boost adds fixed 50% regardless of cylinder state | A nearly-full boost adds 80L to an already 130L cylinder | Overestimates |
| No standby decay | After 10h standby, T1 drops 2.5°C (from 44.4° to 41.9°) | Over-promises after long standby |
| Sink draws counted same as showers | After a charge, bottom zone is hot — all draws cost equally | **Revised** (see bottom-zone analysis) |
| Cold morning not detected | 28 Mar charge reached crossover but T1 only hit 40.4° — model shows 161L at barely usable temp | Overestimates effective capacity |

## What the data actually shows

### T1 is binary: stable or crashing

From 26 draws >30L:
- **22 draws**: T1 moved **<0.3°C** regardless of volume drawn (up to 140L)
- **4 draws**: T1 moved **>0.3°C** — all occurred when cumulative draw since charge exceeded ~150L

T1 doesn't gradually decline. It's a **step function** — rock-solid until the thermocline reaches 1530mm, then crashes at ~1°C per 10L drawn. This is consistent with the 2-second-resolution validation in `dhw-cylinder-analysis.md`.

**T1 alone cannot predict remaining capacity** until it's nearly gone. It's the "fuel gauge empty light" — useful but too late.

### HwcStorageTemp is the key predictor

HwcStorageTemp (600mm, VR10 NTC) sits 930mm below T1. The thermocline reaches it ~148L before reaching T1.

| Draw pattern | HwcStorage behaviour | Interpretation |
|---|---|---|
| Small shower (40L) | Drops 0–3°C | Thermocline still below sensor |
| Medium shower (70L) | Drops 7–14°C | **Thermocline passing sensor** |
| Heavy use (140L+) | Crashes to mains temp | Sensor fully in cold zone |
| During charge | Rises steadily to 44–45°C | Coil zone heating up around sensor |

**HwcStorage crash pattern during draws:**
- When HwcStorage drops >5°C in <5min → thermocline is at sensor height (600mm)
- At that point: remaining usable = (1530 - 600) × 0.159 = **148L minus volume already drawn past that point**

### Post-charge state varies enormously

| Charge type | T1 | HwcStorage | Effective capacity |
|---|---|---|---|
| Full scheduled (typical) | 45.0–45.6°C | 44–45°C | 161L ✓ |
| Partial scheduled (cold morning) | 40.4–42.5°C | 32–42°C | ~100–140L |
| Split charge (interrupted) | 42.1°C + 42.2°C | 36° + 41.5° | ~140L after both |
| Short boost | 45.4°C | 43°C | ~161L (nearly full) |

**The current model's assumption of 161L after any scheduled charge is wrong 25% of the time.**

### Standby decay is consistent

From 20 standby periods ≥2h:
- **T1 decay: 0.22–0.25°C/h** (remarkably consistent)
- **HwcStorage decay: variable** (0.3–1.8°C/h depending on stratification state)

After 10h standby: T1 drops ~2.5°C. At 45°C start, this means 42.5°C — still usable.  
After 24h standby: T1 drops ~6°C. At 45°C start, this means 39°C — marginally usable.

### Sink draws don't consume usable capacity

From the draw event data: every sink draw (flow <300 L/h) showed T1 change of **exactly 0.0°C** and HwcStorage change of **0.0°C**. However, these sink draws were all sub-10L (below the Multical register resolution). The zero-change observation was true but didn't distinguish hot-bottom from cold-bottom state. See the bottom-zone analysis above for the revised understanding: the "dead zone" is heated by the bottom coil and is not permanently cold.

## The improved model

### State variables

```rust
struct DhwState {
    // Core tracking
    remaining: f64,           // usable litres estimate
    volume_at_reset: f64,     // Multical register at last charge completion
    
    // Crossover tracking (the key insight)
    t1_at_charge_start: f64,  // T1 when charge BEGAN (the crossover target)
    crossover_achieved: bool, // True once HwcStorage ≥ t1_at_charge_start
    crossover_time: Option<Instant>,
    
    // Post-charge state
    t1_at_charge: f64,        // T1 when charge completed
    hwc_at_charge: f64,       // HwcStorageTemp when charge completed  
    charge_complete_time: Instant,
    effective_temp: f64,      // Post-crossover T1 (determines quality)
    
    // Thermocline tracking during draws
    hwc_pre_draw: f64,        // HwcStorage before current draw started
    hwc_crash_detected: bool, // True if HwcStorage dropped >5°C during draw
    t1_pre_draw: f64,         // T1 before current draw started
    
    // Existing fields
    was_charging: bool,
    boost_initiated: bool,
}
```

### Algorithm

#### 1. During charge (every 10s poll while `bc_flow > 900`)

```rust
// Track the crossover event in real time
if !crossover_achieved {
    let hwc_now = read_hwc();
    if hwc_now >= t1_at_charge_start {
        crossover_achieved = true;
        crossover_time = now;
        // From this moment, the entire cylinder is ≥ T1_pre.
        // Volume is full (161L), temperature is still rising.
        remaining = 161.0;
        volume_at_reset = volume_register;
    }
}
```

**Note:** The crossover is not purely binary — when HwcStorage gets close to T1 but doesn't quite reach it, the thermocline may be too diffuse to matter. See the gap-based model in section 2 below.

#### 2. After charge completion (`was_charging → !charging`)

```rust
let t1 = read_t1();
let hwc = read_hwc();

if crossover_achieved {
    // Crossover happened during this charge — cylinder is full (161L)
    // but effective temperature determines quality
    remaining = 161.0;
    // Temperature-adjusted capacity: at 40°C, a 40°C shower 
    // needs nearly 100% hot water — capacity is full but fragile
    effective_temp = t1;
} else {
    // Charge ended before crossover — NOT full
    // The hot zone (above thermocline) is unchanged from before the charge.
    // What changed: the cold zone below the thermocline got warmer.
    // Remaining stays at whatever it was before the charge started,
    // UNLESS we can estimate that the warmed lower zone pushed the
    // thermocline down (giving more usable volume).
    //
    // The thermocline position depends on the gap between HwcS_end and T1.
    // Large gap = sharp thermocline = remaining unchanged.
    // Small gap = dissolved thermocline = remaining restored to full.
    let gap = t1 - hwc;
    let hours_standby = 0.0; // just ended, no standby yet
    // Diffusion reduces effective gap over time (8h time constant)
    let effective_gap = gap * (-hours_standby / 8.0f64).exp();
    
    if effective_gap < 1.5 {
        // Thermocline dissolved — warm zone merged with hot zone
        remaining = 161.0;
        effective_temp = hwc; // at the lower temperature
    } else if effective_gap > 3.5 {
        // Sharp thermocline — remaining unchanged
        // remaining stays at pre-charge value
        effective_temp = t1;
    } else {
        // Intermediate — interpolate
        let frac = (effective_gap - 1.5) / 2.0;
        remaining = 161.0 + frac * (remaining - 161.0); // lerp toward pre-charge
        effective_temp = hwc + frac * (t1 - hwc);
    }
    
    volume_at_reset = volume_register;
}

// Save state for standby decay
t1_at_charge = t1;
hwc_at_charge = hwc;
charge_complete_time = now;
crossover_achieved = false;  // reset for next charge
```

#### 2. During draws (every 10s poll)

```rust
let volume_drawn = volume_register - volume_at_reset;
let flow_rate = read_flow(); // L/h

// All draws consume usable volume when bottom zone is hot.
// The "dead zone" is only dead when already flushed cold.
if flow_rate > 50.0 {
    if hwc_now > 30.0 {
        // Bottom zone is warm/hot — every litre drawn displaces hot water
        remaining = (remaining - delta_volume).max(0.0);
    } else {
        // Bottom zone already cold (HwcS ≈ T2)
        // Sink draws recirculate cold water, no impact on thermocline
        // Shower draws still push the thermocline (cold enters at bottom,
        // hot exits at top) but the bottom zone doesn't contribute
        if flow_rate > 300.0 {
            remaining = (remaining - delta_volume).max(0.0);
        }
        // Sink draws (<300 L/h) when bottom is cold: no effect
    }
}

// Temperature-based corrections (override volume model if worse)
let hwc_now = read_hwc();
let t1_now = read_t1();

// HwcStorage crash detection
if hwc_pre_draw - hwc_now > 5.0 {
    // Thermocline has reached 600mm
    // Maximum remaining = 148L minus volume drawn since crash
    let max_from_hwc = 148.0 - volume_since_hwc_crash;
    remaining = remaining.min(max_from_hwc);
    hwc_crash_detected = true;
}

// T1 crash detection (endgame)
if t1_pre_draw - t1_now > 0.5 {
    // Thermocline at T1 height — very little left
    remaining = remaining.min(20.0);
}
if t1_pre_draw - t1_now > 1.5 {
    remaining = 0.0;
}
```

#### 3. During standby (every 10s, no draw, no charge)

```rust
let hours = charge_complete_time.elapsed().as_secs_f64() / 3600.0;

// T1 decays at 0.25°C/h — reduce effective capacity
let effective_t1 = t1_at_charge - 0.25 * hours;

// Below 38°C, water isn't useful for showers
if effective_t1 < 38.0 {
    remaining = 0.0;
} else if effective_t1 < 42.0 {
    // Marginal zone: 38-42°C. Capacity scales with temperature.
    // At 42°C: full remaining. At 38°C: 0.
    let temp_factor = (effective_t1 - 38.0) / 4.0;
    remaining = remaining * temp_factor;
}

// Thermocline diffusion: if a previous no-crossover charge left a
// warm zone with a moderate gap (1.5-3.5°C), thermal diffusion during
// standby will gradually dissolve it. Recalculate remaining using
// the gap-based model with updated standby hours.
// This means a cylinder that was "not full" after a partial charge
// may become effectively full after ~8h of standby (if gap was <3.5°C).
let effective_gap = gap_at_charge_end * (-hours / 8.0f64).exp();
if effective_gap < 1.5 && !crossover_achieved {
    // Thermocline has diffused away during standby
    remaining = remaining.max(161.0);
    effective_temp = (t1_at_charge + hwc_at_charge) / 2.0 - 0.25 * hours;
}

// Don't increase remaining during standby beyond what diffusion allows
remaining = remaining.min(previous_remaining);
```

#### 4. Boost handling

```rust
// Boosts use the same crossover logic as scheduled charges.
// When the user presses boost:
//   1. Record t1_at_charge_start = current T1
//   2. Set crossover_achieved = false
//   3. During the charge, watch for HwcStorage ≥ T1_pre
//   4. If crossover achieved → remaining = 161
//   5. If charge ends without crossover → remaining unchanged
//
// This replaces the arbitrary "+50%" with physics.
if boost_initiated {
    // Same crossover tracking as scheduled charges
    // (handled by the "during charge" poll above)
}
if boost_initiated && was_charging && !charging {
    boost_initiated = false;
    // crossover_achieved already handled remaining
}
```

## What we need to validate

The model has 6 parameters, all derived from data or geometry:

| Parameter | Value | Source | Uncertainty |
|---|---|---|---|
| Full capacity | 177–183L | 2-second flow integration from 4 events (161L was underestimate) | Medium — need more data |
| Crossover condition | HwcStorage ≥ T1_pre | 28 charge cycles, 100% consistent | **Very low** |
| HwcStorage crash threshold | 5°C drop during draw | Observed in 15 shower events | Medium |
| Usable volume above HwcStorage | 148L | Geometry: (1530-600) × 0.159 | Low |
| Standby T1 decay | 0.25°C/h | 20 observations, σ=0.02 | Low |
| ~~Sink dead zone~~ | ~~59L~~ | **Removed** — bottom coil heats "dead zone" directly | See bottom-zone analysis |

The crossover condition is the highest-confidence parameter: in **all 28 observed charges**, T1 rose if and only if HwcStorage crossed T1_pre. Zero false positives, zero false negatives.

**To validate further**, we need:
1. More partial charge events (cold mornings in winter) — especially ones where crossover happens late with T1 only reaching 40–42°C
2. Continuous large draw test (bath tap wide open after charge — no gap, no WWHR)
3. A large (>30L) sink-only draw after a fresh charge (to verify whether hot-bottom sink draws consume usable volume as the model predicts)
4. A boost triggered mid-draw (to verify crossover logic works when cylinder is partially depleted)
5. A period with no draws and no charges for >24h (to validate long standby)

## T1 inflection detector

The script `scripts/dhw-inflection-detector.py` processes every draw ≥40L at 2-second Multical resolution, computing the exact T1 inflection volume via rolling dT1/dV. Run with `--write` to log results to InfluxDB (`dhw_inflection` measurement) for Grafana tracking.

Four inflection measurements from 12 days of data:

| Date | Cumulative | Gap | T1 | Context |
|---|---|---|---|---|
| 21 Mar | **177L** | 0.9h | 44.3° | Full charge, WWHR showers |
| 27 Mar | **183L** | 2.0h | 43.5° | Full charge, 2h gap |
| 29 Mar | **129L** | 1.2h | 41.2° | Low T1 (weak stratification) |
| 23 Mar | 30L | 0.0h | 43.0° | After no-crossover (not true capacity) |

Geometric maximum (dip pipe to draw-off): **243L**. Plug flow efficiency: ~75% at full charge.

## Partial-charge volume estimation (future work)

Once we have an accurate full-charge usable volume V_full (currently 177–183L, target ±10L), we can model partial charges using the physics we've established.

### The energy-balance approach

After a crossover charge, the cylinder is uniform at T1_end. All V_full litres are usable. But what about a no-crossover charge where HwcS_end < T1?

The cylinder has three zones after a no-crossover charge:

```
  ┌─────────────────────────────────────┐  1530mm
  │  HOT ZONE at T1 (≥ T1_pre)        │  ← V_hot = V_remaining (from pre-charge state)
  ├──────────── thermocline ───────────┤  height H_thermo
  │  WARM ZONE at HwcS_end            │  ← V_warm (heated by charge, below T1)
  ├─────────────────────────────────────┤  0mm
  │  (no cold zone — bottom coil      │
  │   heated everything to HwcS_end)  │
  └─────────────────────────────────────┘  0mm
```

The key insight: the warm zone IS usable water if its temperature is high enough for showering. A shower mixer blends hot+cold to reach target temperature. Water at 39°C is less useful per litre than water at 45°C (more hot needed per litre of shower), but it's not zero.

### Equivalent usable litres

For a target shower temperature T_shower (e.g., 40°C) with cold side at T_cold:

Each litre of cylinder water at temperature T produces shower water:
```
  shower_litres_per_cylinder_litre = (T - T_cold) / (T_shower - T_cold)
```

After a partial charge with a hot zone (V_hot at T1) and warm zone (V_warm at T_warm):
```
  equivalent_usable = V_hot × (T1 - T_cold) / (T_shower - T_cold)
                    + V_warm × (T_warm - T_cold) / (T_shower - T_cold)
```

But this only works if the draw reaches the warm zone before the mixer runs cold. The draw order matters: hot zone is drawn first (it's on top), then the warm zone.

The complication: when the thermocline between the hot and warm zones passes T1, the outlet temperature drops from T1 to somewhere between T1 and T_warm. The mixer adjusts, drawing more hot (now warm) water per litre of shower. The transition is gradual, not a step — the coil-broadened thermocline spans many litres.

### What we need to compute this

1. **V_full** (accurate): the full-charge usable volume. Currently 177–183L from two measurements, need ±10L precision from more data.

2. **V_remaining before the charge**: from the volume-subtraction model (litres drawn since last crossover). This is V_hot.

3. **T1** and **HwcS_end** after the charge: T1 gives the hot zone temperature, HwcS_end gives the warm zone temperature.

4. **The thermocline mixing profile**: how quickly does T1 drop when the thermocline passes it? From our crash events, the transition spans ~30–40L (T1 goes from stable to fully crashed over that range). This is the coil mixing penalty.

5. **Gap-dependent thermocline diffusion**: the gap model (effective_gap = gap × exp(-hours/8)) determines whether the hot/warm boundary is sharp or diffuse. After enough standby time, the zones merge and the entire volume becomes usable at an intermediate temperature.

### Example calculation

After a no-crossover charge on 23 March 21:20:
- V_hot = 21L at T1 = 43.5°C (from 140L drawn out of 161L full capacity)
- V_warm = ~150L at HwcS_end = 39.5°C (charge heated the lower zone)
- T_cold = 25°C (WWHR), T_shower = 40°C

```
  Hot zone:  21L × (43.5 - 25) / (40 - 25) = 21 × 1.23 = 26L shower-equivalent
  Warm zone: 150L × (39.5 - 25) / (40 - 25) = 150 × 0.97 = 145L shower-equivalent
  Total: 171L shower-equivalent
```

But the 4°C gap means a sharp thermocline — the 150L warm zone only becomes available after the 21L hot zone is exhausted and T1 crashes. The mixer must cope with the transition. In practice, the user notices a brief temperature dip as the shower adjusts.

With V_full refined to 180L and using the energy-weighted model, the z2m-hub could show:
- "161L at 43°C" (hot zone only — conservative, current behaviour)
- "171L shower-equivalent" (including warm zone — optimistic but more accurate)
- "Tap temperature may drop briefly after ~21L" (user warning)

This is future work pending: (a) V_full validated to ±10L, (b) thermocline mixing profile characterised from more crash events, (c) gap-diffusion model validated against the 29 Mar case.

## Logging for forward validation

The z2m-hub should write additional InfluxDB fields to `dhw` measurement:

```
dhw remaining_litres=X,model_version=2,t1=Y,hwc_storage=Z,
    volume_drawn=W,charge_state="full|partial|standby",
    bottom_zone_hot=B,effective_t1=E,gap=G
```

This lets us backtest model v2 against v1 and identify any remaining systematic errors.

## Impact on the SPA display

The current SPA shows "Empty / Low / OK / Full" based on litres alone. With the improved model:

- **Full** (>150L, T1≥44°C): green, "Full"
- **OK** (40–150L, T1≥40°C): green, shows litres
- **Low** (<40L OR T1<42°C with draws): amber, "Low — X litres"
- **Empty** (<5L OR T1 dropped >1°C during draw): red, "Empty"
- **Stale** (>8h since charge, no draw feedback): show litres but add "~" prefix ("~130L")

Additionally:
- The boost button should show estimated time to crossover: `(T1_pre - HwcStorage_now) / HwcStorage_rise_rate`. At ~0.2°C/min observed climb rate, a 15°C gap = ~75 minutes to crossover.
- During a charge, show "Heating below" / "Heating uniformly" based on crossover state.
- After a charge that didn't cross over, show "Partially charged" instead of "Full".
