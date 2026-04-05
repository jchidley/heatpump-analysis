# DHW Plan

This is an explanation document: it records the DHW objective, the reasoning behind the current charging strategy, and the next decisions to test. For canonical current-state cylinder facts, scheduling rules, and infrastructure details, use `lat.md/`.

**Objective**: Reliable hot water for 5 people. Priority: (1) DHW adequacy, (2) heating compatibility, (3) cost.

Supporting reference and measurements: [DHW reference](dhw-reference.md)

## Cylinder

**Kingspan Albion 300L** (AUXSN300ERP). 221L usable (91% plug flow efficiency). 45°C target. Standing loss 13W. T1 decay 0.25°C/h.

### Sensors

| Sensor | Height | Source | What it tells you |
|---|---|---|---|
| **T1** (hot outlet) | 1530mm | `emon/multical/dhw_t1` (2s) | Actual tap temp. **Authoritative for DHW decisions** |
| T2 (cold inlet) | 490mm | `emon/multical/dhw_t2` (2s) | Mains/WWHR temp (~25°C shower, ~11°C bath) |
| HwcStorageTemp | ~600mm | `ebusd/poll/HwcStorageTemp` (30s) | Lower cylinder. **Misleading after draws** — reads 13°C with 100L of 45°C water above |
| DHW flow | — | `emon/multical/dhw_flow` (2s) | Tap-side, independent of HP circuit |

### Key heights (internal mm)

370 bottom coil top → 415 dry stat → 490 T2+cold inlet → 970 top coil → 1530 T1+draw-off

### Volume budget

| Zone | Volume | Notes |
|---|---|---|
| Above draw-off (1530–1907mm) | 60L | Trapped |
| Top coil → T1 (970–1530mm) | 89L | Main hot zone |
| HwcStorage → top coil (600–970mm) | 59L | Upper coil zone |
| Below HwcStorage (0–600mm) | 95L | Homogenised by bottom U-coil during charges |

WWHR: 41% effectiveness, +9°C steady-state lift on showers. Baths bypass (taps, not drain).

## Capacity: 221L usable

From 12 inflection measurements at 2s Multical resolution. Depends on T1: at 41°C only 119L; at 44–45°C consistently 170–221L. z2m-hub autoloads from InfluxDB.

### Between charges

| Scenario | Volume | Remaining |
|---|---|---|
| 2 normal showers (70L each) | 140L | 81L ✓ |
| 1 long + 1 normal | 170L | 51L ✓ |
| 3 normal showers | 210L | 11L (tight) |
| Bath + normal + short | 210L | 11L (tight) |

## Charging

**Crossover = full.** When HwcStorageTemp ≥ T1 at charge start, entire cylinder is heated. Confirmed 32+ cycles.

From 402 AM charges + 436 cycles ≥30 min:

| Mode | Avg duration | Timeout rate | Electricity | COP |
|---|---|---|---|---|
| Eco | 102 min | 40% (<5°C) | 1.66 kWh | ~3.3 |
| Normal | 60 min | 2% | 1.19 kWh | ~2.5 |

Eco fails in cold weather. Seasonal manual switch (Nov–Mar → normal). `hmu HwcMode` is readable via eBUS for detection/status, but read-only for control — mode changes still have to be made on the aroTHERM controller.

### No-crossover charges

**Not always a failure.** Evening charges serving concurrent showers deliver 2–3× more thermal energy than quiet charges — water goes out the taps, not into the cylinder. Crossover failure only matters if it forces a morning DHW charge that steals preheat on a cold night.

When charge ends without crossover: gap >3°C = sharp thermocline (capacity unchanged); gap <1.5°C = dissolved (capacity restored at lower temp). z2m-hub models diffusion: `effective_gap = gap × exp(-hours/8)`.

## Household usage

| Metric | Value |
|---|---|
| Daily average | 171L (0.9 tanks) |
| Busiest days | 260–270L |
| Showers/day avg | 2.2 |
| Weekly pattern | ~1 bath, ~18 showers, ~12 taps |

### Draw classification

| Type | Peak flow | Typical volume |
|---|---|---|
| Bath | ≥650 L/h | 100–150L |
| Shower | 350–650 L/h | 20–100L |
| Tap | <350 L/h | 10–20L |

Draws during HP charging are tracked (Multical is tap-side, independent of HP circuit).

## Current scheduling strategy

### Present VRC 700 timer windows

| Window | Rationale |
|---|---|
| 05:30–07:00 | Morning Cosy. HP heats house 04:00–05:30 first |
| 13:00–15:00 | Afternoon Cosy. Shortened from 16:00 to prevent peak spills |
| 22:00–00:00 | Evening Cosy. Bank hot water, serve concurrent showers |

Charge triggers at HwcStorageTemp < 40°C (5K hysteresis, 45°C target).

Tariff: see [Heating plan § Tariff](heating-plan.md#tariff). DHW timing difference is <0.3p/shower — **only matters on cold days** when battery depletes.

### Midday window (13:00–16:00)

Best heavy-draw window. Schedule bath + showers here. **Simple rule: if everyone needs a shower/bath, one person goes at ~14:30.**

### Overnight strategy

- **22:00–00:00** (Cosy): bank hot water to reduce/eliminate morning DHW
- **Pre-emptive launch inside the allowed envelope**: if morning DHW is still needed, the controller should choose the best launch time rather than blindly wait for the timer window
- **Morning timer window becomes fallback**: leave a timer envelope so the VRC 700 still recovers if automation misses, crashes, or loses data

Key decision: **morning shower budget** — enough practical hot water for expected normal morning showers. Inputs: T1, remaining litres, crossover state, standby decay, expected morning usage, predicted charge duration, next heating need, and battery adequacy before the next Cosy window.

T1 decays 0.19–0.25°C/h (measured). 22:00 charge at 45°C → ~43°C by 07:00. **Min acceptable T1 = 40°C** (empirical: lowest T1 at shower start across 60 days of data, no complaints). On clean crossover nights, morning DHW charge is unnecessary (T1 ≈ 43°C at 07:00 >> 40°C floor). Morning charge is only needed when evening draws deplete the tank below the decay trajectory.

The real control question is not just whether the morning window is enabled. It is **when the unavoidable DHW event should happen**. The preferred sequence is:
1. decide whether DHW is genuinely required before morning use,
2. score candidate launch times (22:00 bank, battery-backed pre-emptive overnight launch, 04:00 Cosy launch, later fallback inside Cosy),
3. actively trigger `HwcSFMode=load` at the chosen time,
4. keep the timer window as a safety net rather than the primary decision maker.

This matters because non-Cosy energy is not one price in practice. If the battery can cover the gap to the next Cosy window, an early non-Cosy launch costs only a small premium over Cosy. If the battery is low, the same launch may force expensive import. Heating + DHW is the biggest controllable winter load, so battery-aware timing here is the main house-level optimisation lever.

### Historical morning charge data (491 sessions)

| Percentile | Duration |
|---|---|
| Median | 78 min |
| p75 | 105 min |
| p90 | 121 min |
| Max | 123 min |

61% fit in 90 min, 88% in 120 min, 100% in 150 min. If morning DHW is needed before preheat, back off start time by predicted duration.

Historical pattern: 418 days with morning charge vs 42 days with evening charge. When evening charge occurred, 43% still needed a morning follow-up.

### VRC 700 sequencing

Need to express **active DHW launch with timer fallback, then heat at variable start time** day by day. The likely architecture is:
- timer windows define allowed/fallback envelopes,
- controller sends pre-emptive `HwcSFMode=load` when the chosen launch time arrives,
- heating model then uses the remaining cheap/battery-backed window as efficiently as possible.

This is now a scheduling problem, not just a static timer problem. The sequencing decision should account for predicted charge duration, morning comfort risk, outside temperature, and battery adequacy before the next Cosy window.

## HP contention with heating

| Outside | Comfort cost per charge |
|---|---|
| <2°C | ~0.5°C (unrecoverable) |
| 5°C | ~0.3°C, recovers ~1h |
| 10°C | ~0.2°C, recovers ~30 min |
| 15°C | Negligible |

On cold days, schedule DHW at 22:00 to keep preheat window clear. DHW-active periods parked from heating scoring but reused for cooldown analysis.

## Decisions

- **45°C target**: cost per shower flat across 42–50°C (COP vs volume cancel). Standing loss near-minimum at 13W
- **T1 authoritative**: HwcStorageTemp reads 13°C with 100L of 45°C above. T1 at the outlet is the real signal
- **Evening crossover failures are usually fine**: concurrent-draw charges deliver 3× more thermal energy than quiet ones. Only matters if it forces cold-night morning DHW
- **PHE rejected**: max 3–4% COP benefit (~£7/year). Not worth complexity with coil-in-coil at 90–95% efficiency

### Evidence anchor: T1 vs HwcStorageTemp divergence

2 Apr 05:00–08:00: after morning top-up (crossover=true), T1 stayed 45°C while HwcStorageTemp fell to 27°C. z2m-hub: 118L remaining. Confirms T1 authority, 221L capacity, crossover rule.

## How we review this plan

```bash
date -u
export INFLUX_TOKEN=$(ak get influxdb)
cargo run --bin heatpump-analysis -- dhw-history          # JSON
cargo run --bin heatpump-analysis -- dhw-history --human   # readable
cargo run --bin heatpump-analysis -- dhw-sessions --days 7 # capacity analysis
```

Success = reliable hot-water readiness and correct interpretation of evening concurrent draws. See `docs/history-evidence-workflows.md` for the full workflow. Evidence layers: T1 (comfort), HwcStorageTemp (control), z2m-hub remaining litres (practical), crossover (completion).

## Supporting tooling

### z2m-hub (pi5data:3030)

Polls 10s. Tracks remaining litres, draws during charging, crossover, thermocline diffusion, standby decay. Autoloads `recommended_full_litres` from InfluxDB.

API: `GET /api/hot-water`, `GET /api/dhw/status`, `POST /api/dhw/boost`

### InfluxDB measurements

| Measurement | Written by | Key fields |
|---|---|---|
| `dhw` | z2m-hub | remaining_litres, t1, hwc_storage, charge_state |
| `dhw_inflection` | dhw-sessions CLI | cumulative_volume, t1_start, flow_rate |
| `dhw_capacity` | dhw-sessions CLI | recommended_full_litres |

### Live status

```bash
cargo run --bin heatpump-analysis -- dhw-live-status
curl -s http://pi5data:3030/api/dhw/status
```

## Hygiene

Monitor, don't over-engineer. Cylinder turns over 171L/day. Track time since last >55°C cycle; trigger hygiene cycle only on stagnation risk.

## Next decisions to test

1. **Battery-aware DHW event scheduler** — for each required charge, score candidate launch times (22:00 bank, battery-backed overnight pre-emptive launch, 04:00 Cosy launch, later fallback inside Cosy). Cost = marginal electricity cost + heating comfort penalty, with battery adequacy before the next Cosy window determining whether non-Cosy kWh are cheap or expensive.
2. **Need detection from T1 + practical capacity** — decide whether a DHW event is required at all before scheduling it. Use predicted T1, remaining usable litres, crossover state, standby decay, and expected morning demand.
3. **Eco/normal mode integration** — read `hmu HwcMode` directly from eBUS for scheduler inputs, status, history, and duration expectations. Do not infer from max flow temp unless the eBUS register is unavailable.
4. **Timer windows as fallback envelopes** — keep a broad morning DHW window for resilience, but make active `HwcSFMode=load` launches the primary control path.
5. **Joint overnight optimisation with heating** — the unified overnight model (see [Heating plan § Next: unified model](heating-plan.md)) must account for DHW launch timing. DHW steals 50–100 min of HP capacity. The cheapest path to Leather ≥20°C at 07:00 depends on whether a morning DHW charge is needed, and when it runs.
6. **Summer mains temp repeat** — capacity may shift as mains warms from ~11°C to ~18°C.

### Later

- Legionella monitor (turnover + temperature history)
- SPA display improvements (colour-coded status)
- Predictive DHW compensation (pre-raise Leather ~0.3°C before charge, cold days)

## Key files

| File | Purpose |
|---|---|
| `src/thermal/dhw_sessions.rs` | Session analysis CLI |
| `~/github/z2m-hub/` | Live tracking + dashboard |

## Revert to VRC 700

```bash
for day in Monday Tuesday Wednesday Thursday Friday Saturday Sunday; do
  echo "write -c 700 HwcTimer_${day} 05:30;07:00;13:00;15:00;22:00;-:-" | nc -w 2 localhost 8888
done
echo 'write -c 700 HwcSFMode auto' | nc -w 2 localhost 8888
```
