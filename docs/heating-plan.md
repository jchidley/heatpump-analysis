# Heating Plan

**Objective**: Leather 20-21°C during waking hours (07:00-23:00) at minimum cost. Overnight temperature is not a target - it dips and recovers by 07:00.

Reference data (VRC 700, tuning constants, eBUS registers, deployment): [Heating reference](heating-reference.md)

## Constraints

| Constraint | Value | Source |
|---|---|---|
| HP output | 5kW max | Spec |
| House HTC | 261 W/K (model), **~190 W/K (actual overnight)** | Model calibrated; 466 nights of heat meter data show 30% lower actual loss |
| HP deficit below | ~2°C outside | 5kW < heat loss |
| No heating above | 17°C outside | Solar/internal gains sufficient |
| Max flow temp | 45°C | Emitter capacity + COP |
| Leather τ | **50h** (daytime), ~30h (overnight) | 53 DHW segments (daytime); overnight lower due to reduced internal gains |
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

Leather drops 0.8-1.3°C overnight **regardless of flow temp or strategy**. HP is power-limited - it slows the decline but can't prevent it below ~12°C.

## Tariff

| Rate | Price | Times |
|---|---|---|
| Cosy | 13.24p | 04:00-07:00, 13:00-16:00, 22:00-00:00 |
| Mid-peak | 26.98p | 00:00-04:00, 07:00-13:00, 19:00-22:00 |
| Peak | 40.48p | 16:00-19:00 |
| Marginal (battery-blended) | 13.9p | Use for scheduling decisions |

Q2 2026 South East inc VAT. All-in effective 16.7p (from 6,908 kWh, ~£1,151, 12 months inc standing 52.76p/day). 95% of import is off-peak via Powerwall. Battery covers overnight at near-Cosy rates.

## Control approach

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

| Outside | Strategy | Why |
|---|---|---|
| ≤5°C | Heat continuously, accept drop | HP at capacity |
| 5-12°C | Minimum flow for best COP | Modest surplus |
| ≥12°C | Bank heat in 22:00 Cosy, then reduce | Surplus > loss |

### Next: empirical overnight optimiser

The 466 nights all ran at curve=0.55 (flow 24-40°C). We don't know what happens at lower flow temps. The overnight planner should find the **minimum-cost strategy to deliver Leather ≥20°C at 07:00** from empirical data, not hardcoded τ/K constants.

We already have the empirical data to build this:

| Data source | What it tells us | Where |
|---|---|---|
| 53 cooling segments (calibration + DHW) | Leather cooling rate vs outside temp, no heating | Calibration nights + every DHW charge |
| 1067 heating samples | COP vs flow temp vs outside temp | emoncms SQLite |
| Every DHW→heating transition | Leather reheat rate vs outside temp vs flow temp | emoncms SQLite |

**Build an overnight optimiser that:**

1. Fits empirical equations for cooling rate (from cooling segments) and reheat rate (from DHW→heating transitions) as functions of outside temp and flow temp
2. Fits COP = f(flow_temp, outside_temp) from heating samples
3. For a given night (forecast outside temp profile), simulates candidate strategies:
   - Overheat to X°C in 22:00-00:00 Cosy, then off until preheat at time T
   - Continuous at flow F°C all night
   - Off until preheat at time T
   - Combinations: heat to 22:00, reduce, preheat from T
4. For each strategy, computes: predicted Leather at 07:00, total kWh electricity, cost at marginal rate
5. Picks the cheapest strategy where Leather ≥20°C at 07:00 (with safety margin)
6. Each real night validates the prediction - compare predicted vs actual Leather trajectory and kWh

Use equations fitted to empirical data (not lookup tables - equations generalise across the parameter space). The calibration data, DHW cooling segments, and heating samples provide enough points to fit simple functional forms:
- Cooling: exponential decay with empirically-fitted τ(outside_temp) and equilibrium(outside_temp)
- Reheat: rate = f(surplus_W) where surplus = HP_output(flow_temp) - heat_loss(inside_temp, outside_temp)
- COP: polynomial or Carnot-fraction fit to (flow_temp, outside_temp)

The overnight planner then becomes: for each candidate strategy, integrate the Leather trajectory using these equations, compute electricity cost, pick the minimum.

**Constraint**: morning DHW may steal 50-100 min of heating. The optimiser must account for DHW charge timing - either by scheduling DHW first (see [DHW plan](dhw-plan.md)) or by reserving enough headroom that a morning DHW charge doesn't cause a comfort miss.

### Empirical parameters

| Parameter | Code | Empirical | Notes |
|---|---|---|---|
| Leather τ | 50h | 50h daytime, ~30h overnight | Lower overnight due to reduced internal gains |
| Effective HTC | 261 W/K | **~190 W/K** (466 nights) | Model overpredicts heat loss by ~30% |

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

## Review

```bash
date -u
export INFLUX_TOKEN=$(ak get influxdb)
cargo run --bin heatpump-analysis -- heating-history          # JSON
cargo run --bin heatpump-analysis -- heating-history --human   # readable
```

Success = Leather ≥20°C at 07:00 on clean mornings. Each control change is an experiment. Park DHW/door confounders, don't discard them. See `docs/history-evidence-workflows.md` for full workflow.

## Physical improvements

| Priority | Action | Cost | Impact |
|---|---|---|---|
| 1 | Close Elvina trickle vents | FREE | Elvina 2-3°C below other bedrooms |
| 2 | Aldora rad upgrade (reuse existing) | FREE | MWT 47→45°C |
| 3 | Jack&Carol bay draught-strip | ~£30 | 60-150W |
| 4 | EWI on SE wall (~30m2) | ~£5k | 19% demand reduction |

## Current state

| Component | Status |
|---|---|
| V2 two-loop control | ✅ Inner loop converges in 1 tick |
| V2 live solver | ✅ Daytime comfort maintained in clean windows |
| V2 overnight | 🟡 466-night analysis: coasting saves pennies at mild temps, impossible below ~8°C. Pivot to optimal continuous flow temp + evening banking + DHW coordination |
| T1-based DHW | 🟡 T1 queried. Scheduling logic not implemented |
| Door sensors | ⚪ Hardware in hand |

## Next steps

1. **Build empirical overnight optimiser** - see § Next: empirical overnight optimiser above. Fit cooling, reheat, and COP equations from existing data. Simulate candidate strategies. Pick minimum-cost path to Leather ≥20°C at 07:00.
2. **Morning DHW/heating coordination** - coupled with overnight strategy. DHW steals 50-100 min. Joint optimisation required. See [DHW plan](dhw-plan.md)
3. **Fit door sensors** - 2× SNZB-04P (in hand). Stage 1: log only
4. **Effective HTC validation** - 466 nights show ~190 W/K vs model 261 W/K. May partly explain why thermal model τ=15h was wrong - lower real heat loss means slower cooling

### Later

- Event-driven outer loop (DHW→heating transition, door changes)
- Direct flow temp control (`SetModeOverride` to HMU)
- Open-Meteo forecast integration

## Decisions

- **SP=19 night mode**: eliminates Optimum Start, clean separation
- **No runtime learning**: EMA ran away. Static calibration only
- **Overnight strategy is an optimisation problem**: 466 nights at curve=0.55 don’t tell us minimum viable flow. Build empirical optimiser to find cheapest path to 20°C at 07:00 from cooling/reheat/COP data
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
