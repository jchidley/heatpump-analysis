# Heating Plan

This is an explanation document: it records the heating objective, the reasoning behind the current control approach, and the next decisions to test. For canonical current-state controller behaviour, constraints, and infrastructure facts, use `lat.md/`.

**Objective**: Leather 20-21°C during waking hours (07:00-23:00) at minimum cost. Overnight temperature is not a target - it dips and recovers by 07:00.

Supporting reference and field notes: [Heating reference](heating-reference.md)

## Decision constraints

| Constraint | Value | Source |
|---|---|---|
| HP output | 5kW max | Spec |
| House HTC | 261 W/K (model), **~190 W/K (actual overnight)** | Model calibrated; 466 nights of heat meter data show 30% lower actual loss |
| HP deficit below | ~2°C outside | 5kW < heat loss |
| No heating above | 17°C outside | Solar/internal gains sufficient |
| Max flow temp | 45°C | Emitter capacity + COP |
| Leather τ | **50h** (daytime, 53 segments) | Overnight τ unknown — first coast night was confounded (curve 0.10 ≠ off). ~30h estimate unreliable |
| DHW steals HP | 50-100 min/charge | eco ~100, normal ~60 |
| Emitters | 15 radiators, no TRVs, Sterling off | No per-room control |

### HP capacity vs outside temperature

| Outside | HP surplus | Overnight drop (466-night avg) |
|---|---|---|
| ≤0°C | **deficit** | 1.3°C (HP can't maintain) |
| 0-2°C | ~300W | 1.1°C |
| 2-5°C | ~1000W | 1.1°C |
| 5-8°C | ~1900W | 0.9°C |
| 8-12°C | ~2400W | 0.9°C |
| 12-15°C | ~3400W | 0.8°C |

Leather dropped 0.8–1.3°C overnight at curve=0.55 (flow 24–40°C). Lower flow temps have never been tested — the drop at lower flow is unknown.

## Tariff

| Rate | Price | Times |
|---|---|---|
| Cosy | 13.24p | 04:00-07:00, 13:00-16:00, 22:00-00:00 |
| Mid-peak | 26.98p | 00:00-04:00, 07:00-13:00, 19:00-22:00 |
| Peak | 40.48p | 16:00-19:00 |
| Marginal (battery-blended) | 13.9p | Use for scheduling decisions |

Q2 2026 South East inc VAT. All-in effective 16.7p (from 6,908 kWh, ~£1,151, 12 months inc standing 52.76p/day). 95% of import is off-peak via Powerwall. Battery covers overnight at near-Cosy rates.

## Current control approach

Two-loop model-predictive control. `Z1OpMode=night` (SP=19, no Optimum Start). VRC 700 treated as black box - inner loop closes on `Hc1ActualFlowTempDesired`.

| Loop | Interval | Action |
|---|---|---|
| Outer | 15 min | Thermal solver → target flow → initial curve |
| Inner | ~60s | Proportional feedback on `Hc1ActualFlowTempDesired` |

Converges in 1-2 ticks. No runtime learning (EMA ran away - see AGENTS.md). On shutdown: restore `Z1OpMode=auto`, `Hc1HeatCurve=0.55`, `Hc1MinFlowTempDesired=20`.

### Modes

| Mode | Behaviour |
|---|---|
| `occupied` | Full comfort targeting |
| `away` | 15°C frost protection (curve 0.30, ~£0.50/day vs ~£2.50). Week away saves ~£14 |
| `disabled` / `monitor-only` | No eBUS writes |

API: port 3031. `/mode/occupied`, `/mode/away`, `/kill` (baseline restore).

## Overnight strategy

### What 466 nights show

**⚠ All 466 nights ran at curve=0.55, SP=19, MinFlow=20°C.** Flow temps were 24-40°C depending on outside temp - never lower. The data shows what happens at ONE curve, not the minimum viable curve. "Flow temp doesn't correlate with outcome" was wrong - flow was deterministic from outside temp at fixed curve.

Leather dropped 0.8-1.3°C at these flow temps. Lower curves (lower flow, better COP) have never been tested overnight.

### COP by flow temp (from 1067 heating samples)

| Outside | Flow 25-30°C | Flow 30-35°C | Flow 35-40°C |
|---|---|---|---|
| 5-8°C | COP 5.2 | COP 4.5 | COP 3.5 |
| 8-12°C | COP 5.9 | COP 5.4 | - |
| 12-18°C | COP 6.3 | COP 5.8 | - |

Lower flow = better COP, but HP must deliver enough thermal power to offset heat loss.

### Current approach

Heat continuously overnight at model-derived curve. **The real optimisation is finding the minimum overnight curve** where Leather reaches 20°C by 07:00 - not the 0.55 the VRC 700 defaulted to. Lower curve = lower flow = better COP.

At all temperatures, the optimal strategy is unknown — all historical data is from one curve (0.55). The adaptive controller now runs variable curves and every cycle adds data.

### Next: unified model with overnight target optimisation

There is no separate "overnight model." The daytime thermal solver (`bisect_mwt_for_room` → target flow → inner loop) already computes the minimum flow temp to hold Leather at a target. Overnight is the same solver with a different target temperature.

The question is: **what Leather target trajectory from 23:00 to 07:00 delivers ≥20°C at 07:00 at minimum total electricity?**

The only hard constraint is the endpoint: Leather ≥20°C at 07:00. Everything before that is free. A slowly rising ramp (e.g. 18.5°C at 00:00 → 20.5°C at 07:00) keeps flow temp barely above room temp at every moment - best COP throughout. That may beat both "hold flat" and "off then hard preheat" because the HP never needs a high temperature lift.

The solver handles this directly: at each outer cycle, `bisect_mwt_for_room(target(t))` returns the minimum flow temp for that moment's target. If the target rises slowly, flow rises slowly, COP stays high.

Candidate trajectory shapes to simulate:
- **Flat hold** at X°C (19.0, 19.5, 20.0, 20.5) - baseline
- **Slow ramp** from Y°C at 23:00 to 20.5°C at 07:00 - likely best COP
- **Bank + coast** to 22°C in Cosy then let it fall, preheat from T - uses cheap Cosy rate
- **Off + late preheat** - cheapest if HP can recover in time

For each: integrate flow temp → COP → electricity over the night. Pick the cheapest that hits the endpoint.

**Data situation:**
- Historical (466 nights): all at SP=19, curve=0.55, MinFlow=20 — tells us COP and heat loss at flow 24–40°C only. Cannot conclude anything about lower flow temps from this data
- Adaptive controller (running since ~28 Mar): variable curves, MinFlow=19 — every outer cycle adds data across a wider flow temp range
- Going forward: every cycle (day and night) adds to the empirical dataset. There is no day/night distinction in the physics — only the desired temperature changes
- The deployed code still has the separate overnight planner (coast/preheat/maintain with hardcoded τ/K). This needs replacing with the unified model

**To implement:**
1. Remove the separate overnight planner (coast/preheat/maintain logic, hardcoded τ/K)
2. Run `bisect_mwt_for_room` 24/7. Target = 20.5°C during 07:00-23:00, time-varying trajectory overnight
3. Simulate candidate trajectory shapes offline using the thermal solver + COP model. Pick the shape that minimises ∫ electricity while hitting ≥20°C at 07:00
4. Each real night validates: compare predicted vs actual Leather trajectory and total kWh

**Constraint**: morning DHW steals 50-100 min. The target profile must account for DHW timing - see [DHW plan](dhw-plan.md). **On clean crossover nights (T1 ≥45°C at charge end, no overnight draws), morning DHW is unnecessary** — T1 decays to ~43°C by 07:00, well above the 40°C empirical floor. This eliminates the main overnight contention on most nights.

The harder case is when morning DHW is genuinely required. Then the decision is not just whether to keep a morning timer window, but **when the DHW event should happen**. The controller should treat timer windows as fallback envelopes, actively launch `HwcSFMode=load` at the chosen time, and score candidate times by both heating penalty and battery-aware marginal electricity cost. If the battery can bridge to the next Cosy window, an early non-Cosy DHW event is only a small premium over Cosy; if not, it may force expensive import.

### Empirical parameters

| Parameter | Code | Empirical | Notes |
|---|---|---|---|
| Leather τ | 50h | 50h (daytime, 53 segments) | Overnight unknown — no clean data yet |
| Effective HTC | 261 W/K | **~190 W/K** (466 nights at curve=0.55) | Model overpredicts heat loss by ~30%. But this was measured at high flow temps — may differ at lower flow |
| COP vs flow temp | — | Measured from 1067 samples | Lower flow = better COP. Data covers 24–40°C range. Below 24°C untested |

## HP contention with DHW

| Outside | DHW comfort cost |
|---|---|
| <2°C | ~0.5°C drop (unrecoverable) |
| 5°C | ~0.3°C, recovers ~1h |
| 10°C | ~0.2°C, recovers ~30 min |
| 15°C | Negligible |

On cold days schedule DHW at 22:00 to keep preheat window clear. See [DHW plan](dhw-plan.md). **DHW timing is the biggest overnight optimisation lever** - each charge steals 50-100 min of heating capacity.

## Room priorities

- **Leather** (primary): emonth2, τ=50h. Optimise for this when doors closed
- **Aldora** (secondary): second comfort anchor
- **Conservatory**: excluded (30m2 glass, sub-hour τ)

**Door sensors** (2× SNZB-04P, in hand, not fitted):

| Stage | Action | When |
|---|---|---|
| 1. Log | Pair `leather_conservatory_door` + `leather_hall_door`, add to decision log | Now |
| 2. Analyse | Correlate door state with Leather trajectory | 1-2 weeks |
| 3. Integrate | Conservatory open → hold curve. Closed → immediate recalc. Both open → target Aldora | After data |

## How we review this plan

Use historical evidence to judge whether the strategy is working:

```bash
date -u
export INFLUX_TOKEN=$(ak get influxdb)
cargo run --bin heatpump-analysis -- heating-history          # JSON
cargo run --bin heatpump-analysis -- heating-history --human   # readable
```

Success = Leather ≥20°C at 07:00 on clean mornings. Each control change is an experiment. Park DHW and door confounders rather than pretending they did not happen. See `docs/history-evidence-workflows.md` for the full review workflow.

## Physical improvements

| Priority | Action | Cost | Impact |
|---|---|---|---|
| 1 | Close Elvina trickle vents | FREE | Elvina 2-3°C below other bedrooms |
| 2 | Aldora rad upgrade (reuse existing) | FREE | MWT 47→45°C |
| 3 | Jack&Carol bay draught-strip | ~£30 | 60-150W |
| 4 | EWI on SE wall (~30m2) | ~£5k | 19% demand reduction |

## Current position

| Component | Status |
|---|---|
| V2 two-loop control | ✅ Inner loop converges in 1 tick |
| V2 live solver | ✅ Daytime comfort maintained in clean windows |
| V2 overnight | 🟡 466 historical nights all at curve=0.55 (uninformative for optimisation). Separate overnight planner to be replaced with unified model + target trajectory. See § Next. |
| T1-based DHW | 🟡 T1 queried. Min acceptable T1 = 40°C (empirical). Morning timer skip logic exists as an interim step, but the target design is an active battery-aware DHW event scheduler with timer fallback. |
| Door sensors | ⚪ Hardware in hand |

## Next decisions to test

1. **Battery-aware joint heating/DHW scheduler** - treat heating + DHW as the dominant winter load and score candidate DHW event times by battery-aware marginal electricity cost plus heating comfort penalty. Timer windows become fallback rails, not the primary decision maker.
2. **Unify overnight with daytime model** - remove separate overnight planner. Run `bisect_mwt_for_room` 24/7 with a time-varying target trajectory. Simulate candidate shapes (flat hold, slow ramp, bank+coast, off+preheat) offline, pick cheapest that delivers ≥20°C at 07:00. See § Next: unified model.
3. **Need detection for morning DHW** - use predicted T1 and practical capacity to decide whether a DHW event is required at all. The current morning timer skip logic is only an interim implementation of this broader rule. See [DHW plan](dhw-plan.md)
4. **Eco/normal mode integration** - read `hmu HwcMode` directly from eBUS for scheduler inputs, status, and duration expectations.
5. **Fit door sensors** - 2× SNZB-04P (in hand). Stage 1: log only
6. **Effective HTC validation** - 466 nights show ~190 W/K vs model 261 W/K. May partly explain why thermal model τ=15h was wrong - lower real heat loss means slower cooling

### Later

- Event-driven outer loop (DHW→heating transition, door changes)
- Direct flow temp control (`SetModeOverride` to HMU)
- Open-Meteo forecast integration

## Decisions

- **SP=19 night mode**: eliminates Optimum Start, clean separation
- **No runtime learning**: EMA ran away. Static calibration only
- **Overnight = daytime model with target trajectory**: no separate planner needed. Same solver, same inner loop. The optimisation is the target trajectory shape (ramp, flat, bank+coast), not a separate system
- **DHW timing is an event-scheduling problem**: decide whether a charge is required, then choose the best launch time using battery-aware marginal cost and heating contention. Timer windows are fallback envelopes, not the main control logic
- **V1 bang-bang rejected**: 15-min adjustments meaningless against τ=50h

## Key files

| File | Purpose |
|---|---|
| `src/bin/adaptive-heating-mvp.rs` | Controller binary |
| `model/adaptive-heating-mvp.toml` | Config (preheat_hours, waking times) |
| `src/thermal/display.rs` | `bisect_mwt_for_room()` |
| `data/canonical/thermal_geometry.json` | Room geometry |

Deployment on pi5data: see AGENTS.md § Adaptive heating MVP.

## Revert to VRC 700

```bash
echo 'write -c 700 Z1OpMode 1' | nc -w 2 localhost 8888
echo 'write -c 700 Hc1HeatCurve 0.55' | nc -w 2 localhost 8888
echo 'write -c 700 Hc1MinFlowTempDesired 20' | nc -w 2 localhost 8888
```
