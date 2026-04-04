# DHW Plan

Domestic hot water management for 6 Rhodes Avenue. 300L Kingspan Albion cylinder, Vaillant Arotherm Plus 5kW, Multical 403 metering, z2m-hub real-time tracking.

## Scope and related docs

This document is the canonical reference for **DHW operating policy, cylinder behaviour, comfort/capacity model, and rationale**.

Use other docs for adjacent needs:
- **Historical evidence workflows / how-to:** `docs/history-evidence-workflows.md`
- **Code locations / module structure in this repo:** `docs/code-truth/README.md`, `docs/code-truth/REPOSITORY_MAP.md`, `docs/code-truth/ARCHITECTURE.md`
- **Secrets / InfluxDB token handling:** `deploy/SECRETS.md`
- **Agent-facing project memory / gotchas:** `AGENTS.md`
- **Broader documentation guide:** `docs/README.md`

`z2m-hub` is a separate repo/service; this document describes its role in the live DHW system, but not its full source layout.

## Objective

**Reliable hot water for 5 people, with DHW adequacy first, heating compatibility second, and cost minimisation third.**

In practical terms, the system must preserve enough usable hot-water capacity for the household's expected showers and baths. Once that requirement is satisfied, it should minimise disruption to heating. Tariff optimisation is a third-priority tie-breaker, not the governing objective. Hygiene is monitored, not over-engineered.

## Cylinder

### Specification

**Kingspan Albion Ultrasteel Plus Solar Indirect 300L (AUXSN300ERP)**

| Spec | Value |
|---|---|---|
| Capacity | 300L total, **221L usable** from full charge (91% plug flow efficiency) |
| Geometric max drawable | 243L (below draw-off at 1530mm) |
| Internal dimensions | ~450mm diameter, ~1932mm internal height |
| Insulation | 50mm PU foam |
| Heat exchanger | Twin coil-in-coil — solar (lower) + boiler (upper) coils **both connected in series for HP**, doubling heat exchange surface |
| Cold feed | **Dip pipe** from 490mm connection to bottom (~0mm) — all cold water enters at the bottom regardless of WWHR |
| Internal expansion | Air bubble at top (floating baffle, no external vessel). ~46mm→25mm as water heats 10→45°C |
| Standing heat loss | 13W measured (vs 93W rated — stratification + air bubble insulates top) |
| T1 decay rate | 0.25°C/h (σ=0.02, 20 observations). Measured: T1 43.4→42.4°C over 4h, T2 21.5→22.0°C (heat migrating down), room 20.9°C unchanged |
| Annual standby cost | ~£5/year at COP 3.9 |

### Connection heights

Measured from outside bottom (internal = outside − 50mm for insulation):

| Outside (mm) | Internal (mm) | Connection |
|---|---|---|
| 420 | 370 | Bottom coil top (U-shaped loop hangs down into 0–370mm zone) |
| 465 | 415 | Dry stat pocket (VR10 NTC — `HwcStorageTemp`) |
| 540 | 490 | T2 sensor + cold water inlet (dip pipe runs to bottom ~0mm) |
| 1020 | 970 | Top coil (entry/exit) |
| 1580 | 1530 | T1 sensor + hot water draw-off |

### Sensor positions

| Sensor | Height | Source | Rate | Resolution | What it tells you |
|---|---|---|---|---|---|
| T1 (hot outlet) | 1530mm | `emon/multical/dhw_t1` | ~2s | 0.01°C | Actual tap water temperature. Best sensor for DHW decisions |
| T2 (cold inlet) | 490mm | `emon/multical/dhw_t2` | ~2s | 0.01°C | Mains/WWHR inlet temp. Reads ~25°C (WWHR) during showers, ~11°C (mains) during baths |
| HwcStorageTemp (VR10 NTC) | ~600mm | `ebusd/poll/HwcStorageTemp` | ~30s | 0.5°C | Lower cylinder zone. VRC 700 uses this for charge trigger (5K hysteresis = triggers at 40°C). Crashes to mains temp after large draws even with 60L+ usable hot water above |
| DHW flow | — | `emon/multical/dhw_flow` | ~2s | 1 L/h | Tap-side meter. Independent of HP circuit. Peak rate identifies draw type |
| DHW volume | — | `emon/multical/dhw_volume_V1` | ~2s | 10L steps | Cumulative register for volume tracking |

**Kamstrup naming**: Multical registers call T1 "Inlet" and T2 "Outlet" from the meter's energy measurement perspective. In this installation T1=hot (cylinder top), T2=cold (mains inlet). Counterintuitive but correct.

**T1 is authoritative.** HwcStorageTemp reads below the thermocline and gives misleading readings after draws (13.5°C with 100L of 45°C water above it). DHW decisions should use T1.

### Volume budget

| Zone | Height range | Volume | Notes |
|---|---|---|---|
| Above draw-off | 1530→1907mm | 60L | Trapped (hot but inaccessible) |
| Top coil → T1 | 970→1530mm | 89L | Main hot zone |
| HwcStorage → top coil | 600→970mm | 59L | Upper coil zone |
| Below HwcStorage | 0→600mm | 95L | Uniformly heated by bottom U-coil during charges |

### WWHR

Waste Water Heat Recovery on shower drain. Reduces shower energy by 32%, improves stratification. Bath fills bypass WWHR (taps, not drain).

| Phase | T2 (post-WWHR) | Lift from 15.8°C mains |
|---|---|---|
| Start (drain cold) | 15.5°C | −0.3°C |
| 1 minute | 17.1°C | +1.3°C |
| 2 minutes | 19.7°C | +3.9°C |
| 3 minutes | 22.1°C | +6.3°C |
| **Steady state (3.5 min)** | **24.8°C** | **+9.0°C** |

Effectiveness: **41%**. The ~3 min T2 delay at shower start is transit time through 6m of 15mm pipe + WWHR warm-up, not a buoyancy effect (cold feed is a dip pipe to the bottom regardless).

### Bottom coil homogenises the lower cylinder

The bottom coil enters and exits at 370mm (same height — U-shaped loop hanging downward). During a charge, convective mixing from this loop makes the entire 0–600mm zone (~95L) reach a **uniform temperature**. Evidence: HwcStorageTemp crashes as a step function during large draws (e.g. 41.5°C → 29°C in <5 minutes), not gradually. This proves the zone was uniformly hot before the draw.

### Draw rates and hot fractions

| Draw type | Total flow | Cold side | Hot fraction | Cylinder draw rate |
|---|---|---|---|---|
| Shower (WWHR) | 7 L/min | 25°C | 77% | 5.4 L/min |
| Bath fill (mains) | ~12 L/min | 15.8°C | 84% | ~10 L/min |
| Sink (mains) | ~3 L/min | 15.8°C | 84% | ~2.5 L/min |

## Capacity

### Measured: 221L usable (12 inflection measurements)

The `dhw-sessions` CLI analyses draws at 2-second Multical resolution, finding the exact volume where T1 begins dropping.

| Date | Usable (L) | T1 (°C) | T2 (°C) | Flow (L/h) | Context |
|---|---|---|---|---|---|
| 21 Mar | 177 | 44.3 | 25.8 | 464 | Full charge, WWHR showers |
| 23 Mar | 155 | 44.1 | 25.6 | 527 | Full charge, shower during charge |
| 27 Mar | 173 | 43.5 | 25.2 | 530 | Full charge, back-to-back showers |
| 29 Mar | 119 | 41.2 | 24.8 | 529 | Low T1 (41°C), weak stratification |
| 31 Mar | 198 | 43.7 | 25.6 | 529 | Full charge, shower during charge |
| 01 Apr | 174 | 43.5 | 25.0 | 534 | Full charge, shower during charge |
| 03 Apr | 146 | 42.9 | 25.6 | 523 | Back-to-back showers |
| 03 Apr | **221** | 41.0 | 25.1 | 231 | Tap after back-to-back showers |
| 03 Apr | 170 | 42.3 | 25.0 | 529 | Shower during charge |

Capacity depends on T1 (lower T1 → weaker density contrast → earlier inflection). At 41°C only 119L; at 44–45°C consistently 170–221L. Geometric max 243L, plug flow efficiency **91%** (was 81%).

z2m-hub autoloads the recommended capacity from InfluxDB on startup. Currently **221L**.

### Between charges: 2–3 showers comfortably

| Scenario | Volume | Remaining (from 221L) |
|---|---|---|
| 2 normal girl showers | 70 + 70 = 140L | 81L ✓ |
| 1 long + 1 short | 100 + 30 = 130L | 91L ✓ |
| 1 long + 1 normal | 100 + 70 = 170L | 51L ✓ |
| Bath + short shower | 110 + 30 = 140L | 81L ✓ |
| 3 normal showers | 70 + 70 + 70 = 210L | 11L (tight but possible) |
| Bath + normal + short | 110 + 70 + 30 = 210L | 11L (tight) |

## Charging

### Two-phase cycle

Every charge has two phases, visible in sensor data:

1. **Below-T1 heating**: coils heat cold water in lower cylinder. T1 is dead flat. HwcStorageTemp rises toward T1
2. **Uniform heating** (crossover): begins when HwcStorageTemp ≥ T1 at charge start. Entire cylinder is now at/above T1. T1 starts rising ~0.1°C/min

**Crossover = cylinder is full.** Confirmed across 32+ charge cycles (100%).

Heat exchanger approach temperature: starts negative (HP flow cooler than T1), crosses over mid-charge, ends at +3.2°C (excellent for indirect coil-in-coil). Typical morning charge: HP flow rises 31→48°C over 115 min, delivering ~5.75 kWh thermal at COP ~3.0 in eco mode.

### Charge duration and cost

From 402 AM charges (emoncms, Oct 2024 – Mar 2026) + 436 cycles ≥30 min:

| Mode | Avg duration | 120-min timeout rate | Electricity | COP | Max flow temp | House temp drop |
|---|---|---|---|---|---|---|
| **Eco** | 102 min | 40% (nearly all below 5°C) | 1.66 kWh | ~3.3 | <50°C | 0.5°C (cold), 0.2°C (mild) |
| **Normal** | 60 min | 2% | 1.19 kWh | ~2.5 | ≥50°C | 0.2°C (cold), 0.1°C (mild) |

Eco is cheaper per kWh (COP ~3.3 vs ~2.5) but takes longer and fails in cold weather. **Seasonal switch**: change to normal when mornings feel cold (typically Nov–Mar), back to eco when the house is warm through the morning. Cannot be automated — `hmu HwcMode` is read-only via eBUS. Investigation ongoing: VWZ AI B512/B513 registers may offer a writable path.

### Eco mode by outside temperature

| Outside | Avg duration | Hit timeout | Assessment |
|---|---|---|---|
| <2°C | 118 min | 95% | **Nearly all incomplete** |
| 2–5°C | 119 min | 89% | Mostly incomplete |
| 5–8°C | 111 min | 53% | Borderline |
| 8–12°C | 101 min | 23% | Usually completes |
| 12°C+ | 86 min | 13% | Fine |

### No-crossover charges (thermocline physics)

**⚠ "No crossover" does not mean "failed charge".** Many evening charges end without crossover because hot water was being actively drawn during charging (see § Evening charges with concurrent draws below). The HP was simultaneously heating the cylinder AND serving showers — delivering 2–3× more useful thermal energy than a quiet charge that reaches crossover. Crossover failure only matters operationally if it forces a morning DHW charge that steals from heating preheat on a cold night.

When a charge ends without crossover (HwcStorageTemp never reached T1), the cylinder has two zones:

| Gap (T1 − HwcS at end) | Thermocline | Remaining |
|---|---|---|
| >3°C | Sharp (buoyancy barrier) | Unchanged from pre-charge |
| <1.5°C | Dissolved (mixing) | Restored to full at lower temp |
| 1.5–3°C | Intermediate | Interpolated. Diffuses over ~8h (√κt) |

Thermal diffusion blurs the thermocline: diffusion length = √(κ × t) where κ = 0.15 mm²/s. After 6h: ~57mm. After 8h: ~66mm (fully diffused). z2m-hub models this as `effective_gap = gap × exp(-hours/8)`.

## Household usage

### Profile (14 days, everyone home)

| Person | Draw type | Typical volume | Peak flow |
|---|---|---|---|
| Jack | Shower | 30L | ~525 L/h |
| 3 girls | Shower | 70L each (100L occasionally) | ~530 L/h |
| Son | Bath + short shower | 110L + 30L | ~730 L/h (bath) |
| Everyone | Taps | ~15L/day | <350 L/h |

### Draw type classification

| Type | Peak flow | Volume | Identification |
|---|---|---|---|
| **Bath** | ≥650 L/h | 100–150L | Taps wide open |
| **Shower** | 350–650 L/h | 20–100L | Mixer valve |
| **Tap** | <350 L/h | 10–20L | Kitchen/bathroom sink |

### Daily usage

| Metric | Value |
|---|---|
| Daily average | 171L (0.9 tanks) |
| Busiest days | 260–270L (1.3–1.4 tanks) |
| Quiet days | 40–120L (0.2–0.6 tanks) |
| Showers per day (avg) | 2.2 |
| Weekly pattern | ~1 bath, ~18 showers, ~12 taps |

Not everyone showers every day (or on the same day). Busiest days have 3–4 showers, not 5.

### Draws during HP charging

`dhw_flow` is the tap-side Multical meter — completely independent of the HP charging circuit. Draws during charging are real usage that depletes the cylinder. Both `dhw-sessions` CLI and z2m-hub track these (marked with `*` / `[during charge]` in output).

### Evening charges with concurrent draws

Evening charges (22:00 Cosy window) frequently coincide with household showers. These charges typically don't reach crossover, but this is **not a failure** — the HP is delivering hot water in real time at the Cosy rate.

Observed examples from the first week of adaptive control (28 Mar – 4 Apr 2026):

| Night | HwcS start | Draws during charge | HwcS end | Crossover | Thermal energy |
|---|---|---|---|---|---|
| 1 Apr 21:05 | 15.5°C | 60L shower | 41.5°C | ✗ | ~10.2 kWh (heat 300L from 15→42°C + serve 60L) |
| 2 Apr 21:03 | 36.0°C | None | 45.0°C | ✓ | ~3.1 kWh (quiet top-up) |
| 3 Apr 21:04 | 26.0°C | 140L + 120L showers + tap | 39.5°C | ✗ | ~10.2 kWh (heat cylinder + serve 270L) |

**The "failed" charges delivered 3× more useful energy than the "successful" one.** They just didn't reach crossover because the water was going out the taps, not staying in the cylinder. T1 stayed above 42°C throughout — everyone got hot showers.

The morning consequence depends on outside temperature:
- **Mild nights (≥8°C)**: morning top-up of 62 min fits before preheat, no comfort miss (e.g. 4 Apr: morning top-up 03:08–04:10, Leather minimum 20.7°C)
- **Cold nights (<5°C)**: morning top-up steals from preheat window and may cause comfort miss (e.g. 2 Apr: 82 min DHW overlap during preheat, Leather only 19.6°C by 09:00 — though also confounded by door open)

**Implication for scheduling**: on cold nights, ensure the evening charge completes before the overnight preheat window, or switch to Normal mode for faster charging (60 min vs 120 min eco). On mild nights, the current pattern works fine.

## Scheduling

### VRC 700 DHW timer windows (current)

| Window | Cosy period | Rationale |
|---|---|---|
| **05:30–07:00** | Morning Cosy | Delayed from 05:00 to 05:30 — the latest start where 100% of Normal cycles finish within Cosy (worst case 06:58). HP heats the house for 1.5h first (04:00–05:30). Eco spills ~30 min past 07:00 but costs 40p/year |
| **13:00–15:00** | Afternoon Cosy | Shortened from 16:00 to prevent peak (40.48p) spills. Data showed 18 historical spills under old 16:00 end |
| **22:00–00:00** | Evening Cosy | Top-up after evening showers. Now standard with adaptive heating (charges every evening). Preferred for cold-night DHW to free morning for preheat. Often serves concurrent showers — see § Evening charges with concurrent draws |

Charge triggers when HwcStorageTemp drops below 40°C (CylinderChargeHyst=5K, target 45°C).

### Tariff

See [Heating plan § Tariff](heating-plan.md#tariff) for rates. Key point for DHW: Cosy-vs-battery cost difference is only 0.7p/kWh — a shower reheat costs <0.3p more off-Cosy. **Timing only matters on cold days** when the battery depletes and you hit real grid peak at 40.48p.

### The midday Cosy window (13:00–16:00)

3 hours. In eco mode: 2 full charges comfortably (but tight for a third — the last few degrees from 43→45°C take disproportionately long). In normal mode: 3+ charges.

**The ideal heavy-draw window.** Schedule the bath and 1–2 showers here when everyone's home:

| Time | What |
|---|---|
| 13:00 | Cosy starts, HP charges (~50–100 min) |
| ~14:00 | Full. Bath + short shower (140L) |
| ~14:20 | HP recharges |
| ~15:10 | Full again. Girl's shower (70L) |
| ~15:20 | HP recharging |
| 16:00 | Cosy ends, cylinder nearly full |

**Simple rule: if everyone needs a shower/bath, at least one person goes at ~14:30.**

### Overnight DHW

T1 decays 0.25°C/h. A 22:00 charge to 45.5°C delivers ~43.3°C by 07:00 (9h × 0.25°C = 2.25°C drop). Verified: 1 Apr charge peaked at 45.5°C at 14:00, decayed to 43.6°C by 22:00 (−1.9°C in 8h, no draws). This is marginal — minimum acceptable T1 for morning showers is TBD (household experiment needed: 45°C definitely fine, 43°C might be too cool).

Preferred overnight strategy:
- **22:00–00:00** (Cosy): bank hot water when practical, because this can reduce or eliminate the next morning's DHW requirement
- **Before the heating-critical preheat window**: if morning DHW is still required, schedule it to **finish before heating must start**, even if that means running before the ideal Cosy slot or earlier than the historic timer pattern
- **04:00–07:00** (Cosy): use only when a morning recharge is genuinely required and an earlier completion window is not available or not enough
- **00:00–04:00** (mid-peak): acceptable when needed to satisfy DHW adequacy before heating, because DHW need outranks tariff optimisation

The key overnight decision is **morning shower budget** — enough practical hot water for the expected normal morning showers, not a bare T1 threshold. Inputs: T1, remaining litres, crossover state, standby decay, expected demand, next preheat start, predicted charge duration. Don't reserve for rare worst-case draws.

### Historical emoncms evidence: could morning DHW have been scheduled earlier?

Historic emoncms data says **yes, often**.

Across **491 morning DHW sessions** in the synced historical dataset:
- average duration = **75.6 min**
- median duration = **78.0 min**
- p75 = **104.5 min**
- p90 = **121.0 min**
- p95 = **122.0 min**
- max = **123.0 min**

This means:
- **61.1%** of historical morning charges would fit inside a **90-minute** pre-heating window
- **88.2%** would fit inside a **120-minute** pre-heating window
- **100%** would fit inside a **150-minute** pre-heating window in this dataset

So if the house is heating-constrained and a morning recharge is still required, an evidence-backed alternative to letting DHW consume the heating window is to start DHW **earlier**, by approximately the predicted charge duration, so it finishes before the required heating start.

The same historical data also shows why 22:00 banking is helpful but not sufficient on its own:
- evening charges were historically **rare** (**42 days** with an evening charge vs **418 days** with a morning charge)
- and when an evening charge did occur, a next-morning charge still followed on **18/42 days** (**42.9%**)

So the historical evidence supports this priority order:
1. preserve enough DHW capacity for expected household use
2. if more DHW is needed before morning use, try to complete it **before** the heating-critical window
3. only then optimise for Cosy timing

### VRC 700 sequencing investigation (required)

The earlier-before-heating strategy needs VRC 700 to express **DHW first, then heating at a variable start time** — day by day. Options: timer rewrites, mode changes, repeated boosts, or direct eBUS writes. Not yet investigated.

## Decisions and rationale

- **45°C target**: cost per shower is nearly constant across 42–50°C (higher temp → worse COP but fewer litres → effects cancel within 0.4p/5%). 45°C is ~1°C above the practical minimum for the household's hottest shower preference + bath margin. Standing losses at 45°C are near-minimum (13W)
- **PHE + secondary return evaluated and rejected**: COP doesn't change (same Q, ṁ, Cp). T1 dip during below-T1 heating is only 0.3°C. PHE can only run for ~60 of 115 min (primary < T1 for first 48 min). Max benefit: 3–4% COP, saving ~£7–8/year. Not worth the complexity with coil-in-coil already at 90–95% efficient
- **DHW timer windows**: 05:30 (not 05:00) for morning — gives HP 1.5h of heating at Cosy rate first. 15:00 (not 16:00) for afternoon — prevents peak-rate spills. 22:00 for evening — preferred for cold nights to free morning for preheat
- **T1 is authoritative for DHW decisions**: HwcStorageTemp reads 13°C with 100L of 45°C water above it after large draws. Multical T1 at the actual outlet is the reliable signal. Phase 2 will use T1 for charge triggering instead of VRC 700 hysteresis
- **Partial-charge model**: when a charge ends without crossover, z2m-hub uses gap-based interpolation. Future: shower-equivalent calculation (`V × (T_zone − T_cold) / (T_shower − T_cold)`) would give more accurate remaining estimate for two-zone cylinders
- **"Failed" evening crossover is usually fine**: evening charges that serve concurrent showers deliver 2–3× more thermal energy than quiet charges (see § Evening charges with concurrent draws). Crossover rate is a misleading quality metric — what matters is whether enough hot water remains for the next morning without forcing a preheat-window DHW charge on a cold night

### Evidence anchor: T1 vs HwcStorageTemp divergence (2 Apr 05:00–08:00)

After a morning top-up (05:01–05:37, crossover=true), T1 stayed at 45°C while HwcStorageTemp fell to 27°C by 08:00. z2m-hub estimated 118L remaining. Confirms: T1 is authoritative, cold lower-cylinder ≠ empty, 45°C target and 221L capacity are correct. A charge can be successful as a DHW event while harmful to heating if it occupies the preheat window.

## HP contention with heating

Each DHW charge blocks space heating. Impact by outside temperature:

| Outside | Comfort cost per DHW charge |
|---|---|
| <2°C | ~0.5°C Leather drop (unrecoverable for hours) |
| 5°C | ~0.3°C drop, recovers in ~1h |
| 10°C | ~0.2°C drop, recovers in ~30 min |
| 15°C | Negligible |

On cold days, schedule DHW at 22:00 (before overnight) to keep preheat window clear. See [Heating plan](heating-plan.md). DHW-active periods are parked from heating-effectiveness scoring but reused for cooldown/building-response analysis.

## Hygiene (legionella)

**Monitor, don't over-engineer.** The cylinder turns over 171L/day on average — nearly a full tank daily. Legionella risk is low in normal occupied use.

Strategy:
- Monitor cylinder temperature history + turnover from Multical volume data
- Track time since last sufficiently hot cycle (>55°C throughout)
- Trigger a targeted hygiene cycle **only when** low turnover / stagnation raises risk
- Do not hold the cylinder at a permanent high temperature

## Tooling

### z2m-hub (real-time, deployed on pi5data:3030)

- Polls every 10 seconds
- Tracks remaining litres (volume subtraction + temperature overrides)
- Detects draws during charging (Multical flow is tap-side, independent of HP)
- HWC crash detection (>5°C drop → cap remaining at volume above sensor)
- T1 drop detection (>1.5°C → remaining = 0)
- Crossover tracking for charge completeness
- Gap-based thermocline model for partial charges
- Standby decay modelling (0.25°C/h)
- Autoloads `recommended_full_litres` from InfluxDB on startup
- API: `GET /api/hot-water`, `GET /api/dhw/status`, `POST /api/dhw/boost`

**Remaining-litres algorithm:**

- **During charge** (bc_flow > 900): watch for HwcStorage ≥ T1_at_charge_start. On crossover: `remaining = full_litres`
- **After charge (crossover)**: `remaining = full_litres`, `effective_temp = T1`
- **After charge (no crossover)**: gap < 1.5°C → thermocline dissolved, `remaining = full_litres` at lower temp. Gap > 3.5°C → sharp thermocline, remaining unchanged. Gap 1.5–3.5°C → interpolate + diffusion model
- **During draws**: subtract Multical volume. Overrides: HwcStorage crash >5°C → cap at 148L minus further draws. T1 drop >0.5°C → remaining ≤ 20L. T1 drop >1.5°C → remaining = 0
- **Standby**: `effective_T1 = T1_at_charge - 0.25 × hours`. Below 38°C → remaining = 0. 38–42°C → linear scale

### Live DHW checks

Use these when the question is **what hot-water state looks like right now**, not whether the recent DHW plan worked.

```bash
cargo run --bin heatpump-analysis -- dhw-live-status
cargo run --bin heatpump-analysis -- dhw-live-status --human
curl -s http://pi5data:3030/api/hot-water
curl -s http://pi5data:3030/api/dhw/status
```

Use the structured default `dhw-live-status` output for LLM/tool consumption.
Use `--human` only for operator readability.
For practical availability, prefer `T1`, remaining litres, and charge state over `HwcStorageTemp` on its own.

### Historical evidence commands

#### `dhw-history` (fused window reconstruction)

```bash
date -u
export INFLUX_TOKEN=$(ak get influxdb)
cargo run --bin heatpump-analysis -- dhw-history          # JSON (default)
cargo run --bin heatpump-analysis -- dhw-history --human   # readable
```

Success criteria: reliable hot-water readiness, acceptable crossover rate, evening concurrent draws classified correctly. Each schedule/trigger change is an experiment. See `docs/history-evidence-workflows.md` for review workflow and evidence interpretation.

Evidence layers: T1 (comfort truth), HwcStorageTemp (control truth), z2m-hub remaining litres (practical state), charge crossover (completion). For joined DHW + heating review, classify each charge as DHW event, heating interaction, and thermal-response evidence.

#### `dhw-sessions` CLI (capacity + inflection analysis)

```bash
date -u
export INFLUX_TOKEN=$(ak get influxdb)
cargo run --bin heatpump-analysis -- dhw-sessions --days 7               # verbose (default investigation window)
cargo run --bin heatpump-analysis -- dhw-sessions --days 7 --format json
cargo run --bin heatpump-analysis -- dhw-sessions --days 7 --no-write    # don't update InfluxDB
```

- Queries InfluxDB at 10s resolution (event detection) + 2s resolution (inflection analysis)
- Classifies draws: bath (≥650 L/h), shower (350–650), tap (<350)
- Tracks HwcStorageTemp during draws (pre/min/drop)
- Detects draws during HP charging
- Writes `dhw_inflection` measurements + `dhw_capacity` recommended value to InfluxDB
- Run periodically to keep capacity number fresh as seasonal mains temp changes

Use `dhw-history` when you want a fused explanation for a specific charge window. Use `dhw-drilldown --since ... --until ...` when you want bounded native-cadence detail for one chosen DHW event/window. Use `dhw-sessions` when you want the deeper capacity / inflection evidence behind this plan. For historical workflow and interpretation, use `docs/history-evidence-workflows.md`.

### InfluxDB measurements

| Measurement | Written by | Fields |
|---|---|---|
| `dhw` | z2m-hub (every 10s) | remaining_litres, t1, hwc_storage, effective_t1, charge_state |
| `dhw_inflection` | dhw-sessions CLI | cumulative_volume, draw_volume, t1_start, mains_temp, flow_rate, hwc_pre/min/drop |
| `dhw_capacity` | dhw-sessions CLI | recommended_full_litres, method |

### SPA display (z2m-hub phone dashboard)

Currently shows litres + simple status. Planned improvements:
- **Full** (>150L, T1≥44°C): green
- **OK** (40–150L, T1≥40°C): green with litres
- **Low** (<40L OR T1<42°C with draws): amber
- **Empty** (T1 dropped >1°C during draw): red
- During charge: "Heating below" / "Heating uniformly" (crossover)
- Boost button: estimated time to crossover

## Key files and operational dependencies

| File / system | Purpose |
|---|---|
| `docs/dhw-plan.md` | DHW control and capacity strategy |
| `deploy/SECRETS.md` | Secrets management: InfluxDB token setup, dev vs prod |
| `src/thermal/dhw_sessions.rs` | Historical DHW session analysis CLI |
| `~/github/z2m-hub/` | Live DHW tracking, dashboard, and boost endpoint on pi5data |

## Deployment notes

DHW operations span this repo and the separately deployed `z2m-hub` service on pi5data.

| Component | Location |
|---|---|
| `dhw-sessions` CLI source | `src/thermal/dhw_sessions.rs` |
| `z2m-hub` runtime | pi5data (`http://pi5data:3030`) |
| InfluxDB | pi5data Docker (`influxdb`) |
| Secrets (InfluxDB token) | `/etc/adaptive-heating-mvp.env` (root:root 0600) |

The InfluxDB token is the same one Telegraf uses. See `deploy/SECRETS.md` for fresh-install setup, token sourcing, and dev-vs-prod rules.

Development fallback: when `INFLUX_TOKEN` is not set locally, use `ak get influxdb`. Do not hardcode tokens in repo-tracked config.

Run historical analysis from this repo using the standard rolling 7-day window:

```bash
date -u
export INFLUX_TOKEN=$(ak get influxdb)
cargo run --bin heatpump-analysis -- dhw-sessions --days 7
```

## Reference data

### Morning charge trace (21 March 2026, eco mode, 05:10–07:05 UTC)

| Time | HP FlowT | HP ReturnT | HP ΔT | Heat kW | Elec W | T1 | T2 |
|------|----------|-----------|-------|---------|--------|-----|-----|
| 05:10 (start) | 31°C | 30°C | 1°C | 2.0 | 780 | 42.0 | 23.3 |
| 05:30 | 39°C | 37°C | 2°C | 3.1 | 921 | 42.3 | 24.3 |
| 06:00 | 43°C | 41°C | 2°C | 3.0 | 993 | 42.6 | 26.6 |
| 06:30 | 46°C | 44°C | 2°C | 3.0 | 1039 | 43.4 | 29.7 |
| 07:00 | 48°C | 46°C | 2°C | 2.9 | 1069 | 44.9 | 32.2 |
| 07:05 (end) | 48°C | 46°C | 2°C | 2.9 | 1072 | 45.2 | 32.4 |

115 min, 1.3 m³/h, 5.75 kWh thermal, 1.92 kWh electrical. **COP 3.0**. Constant ~2°C primary ΔT throughout (eco mode).

Heat exchanger approach: start −4.7°C (HP cooler than T1), mid +0.6°C (crossing over), end +3.2°C. Excellent for indirect coil-in-coil.

### Energy accounting (21 March)

| | Value |
|---|---|
| HP thermal input (morning charge) | 5.75 kWh |
| Energy stored in usable hot zone (149L, 45−25°C) | 3.5 kWh |
| Energy stored in warm zone (154L, 28−15°C) | 2.3 kWh |
| Energy removed by showers (180L, 44.5−25°C) | 4.1 kWh |

Showers removed 117% of usable hot energy — this is why the cylinder fully depleted.

## Next steps

1. **Morning shower-capacity trigger validation** — use the improved data collection over the coming week to validate a practical overnight top-up rule based on whether the cylinder can support the expected number of **normal morning showers** at acceptable comfort. T1 remains authoritative for outlet temperature, but the trigger should be practical-capacity-led rather than a bare T1 cutoff. Review first with rolling 7-day-to-now `dhw-history`, then inspect representative anchors such as `2026-04-02T05:00:00Z` → `2026-04-02T08:00:00Z` where `T1` stayed ~45°C while `HwcStorageTemp` fell to 27°C with ~118 L still remaining.
2. **VRC 700 sequencing + earlier-morning scheduling** — investigate how to express "DHW first, then heat" on the real controller (timer rewrites, boosts, mode changes, or direct writes). When morning DHW is needed, back off the start time by predicted charge duration so it finishes before preheat. Historical evidence: 61% of charges fit in 90 min, 88% in 120 min, 100% in 150 min.
3. **T1-led overnight top-up logic** — once the morning shower-capacity rule is validated, trigger DHW via `HwcSFMode=load` only when predicted morning capacity is insufficient, not when `HwcStorageTemp` alone looks low. Monitor completion from T1, crossover, and remaining practical capacity.
4. **DHW/heating interaction accounting** — when reviewing charges, explicitly classify whether each DHW-active interval was:
   - operationally justified for hot-water readiness
   - strategically harmful to the heating objective
   - useful as cooldown / building-response evidence for the heating plan
5. **Summer mains temp repeat** — mains warms from ~11°C to ~18°C, WWHR effectiveness changes, capacity number may shift. Run `dhw-sessions --days 7` as the default rolling review, then inspect representative charge windows with `dhw-history`
6. **Legionella monitor** — track turnover + temperature history, alert on stagnation risk
7. **SPA display improvements** — richer status on phone dashboard
8. **Eco/normal mode detection** — detect from max flow temp (≥50°C = normal), plan charge duration accordingly. Investigate if writable via eBUS (VWZ AI B512/B513 registers?) and validate against `dhw-history`
9. **Predictive DHW compensation** — 15 min before predicted charge, boost heating target_flow to pre-raise Leather ~0.3°C (cold days only). Correlate `dhw-history` with `heating-history`

## Revert to autonomous VRC 700 DHW

```bash
# Current DHW timers (already set):
for day in Monday Tuesday Wednesday Thursday Friday Saturday Sunday; do
  echo "write -c 700 HwcTimer_${day} 05:30;07:00;13:00;15:00;22:00;-:-" | nc -w 2 localhost 8888
done

# Reset HwcSFMode if stuck on 'load' after a boost:
echo 'write -c 700 HwcSFMode auto' | nc -w 2 localhost 8888
```

45°C target. CylinderChargeHyst=5K. VRC 700 triggers charge when HwcStorageTemp < 40°C within a timer window.
