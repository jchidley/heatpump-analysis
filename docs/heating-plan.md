# Heating Plan

**Objective**: Leather 20–21°C during waking hours (07:00–23:00) at minimum cost. Overnight temperature is not a target — it dips and recovers by 07:00.

Reference data (VRC 700, tuning constants, eBUS registers, deployment): [Heating reference](heating-reference.md)

## Constraints

| Constraint | Value | Source |
|---|---|---|
| HP output | 5kW max | Spec |
| House HTC | 261 W/K | Calibrated thermal model |
| HP deficit below | ~2°C outside | 5kW < heat loss |
| No heating above | 17°C outside | Solar/internal gains sufficient |
| Max flow temp | 45°C | Emitter capacity + COP |
| Leather τ | **50h** | 53 empirical cooling segments (18 calibration-night + 35 DHW) |
| Leather cooling rate | 0.020/hr per °C ΔT | k = 1/τ |
| DHW steals HP | 50–100 min/charge | eco ~100, normal ~60 |
| Emitters | 15 radiators, no TRVs, Sterling off | No per-room control |

### HP capacity vs outside temperature

| Outside | HP surplus | Overnight drop (8h, τ=50h) |
|---|---|---|
| ≤0°C | **deficit** | Must heat continuously |
| 2°C | 172W | 2.4°C |
| 5°C | 954W | 2.0°C |
| 8°C | 1738W | 1.6°C |
| 10°C | 2260W | 1.3°C |
| 14°C | 3304W | 0.7°C |

## Tariff

| Rate | Price | Times |
|---|---|---|
| Cosy | 13.24p | 04:00–07:00, 13:00–16:00, 22:00–00:00 |
| Mid-peak | 26.98p | 00:00–04:00, 07:00–13:00, 19:00–22:00 |
| Peak | 40.48p | 16:00–19:00 |
| Marginal (battery-blended) | 13.9p | Use this for scheduling decisions |

Q2 2026 South East inc VAT. All-in effective 16.7p (from 6,908 kWh, ~£1,151, 12 months inc standing 52.76p/day). 95% of import is off-peak via Powerwall. Real value of Cosy alignment: **protecting battery for peak hours on cold days**.

## Control approach

Two-loop model-predictive control. `Z1OpMode=night` (SP=19, no Optimum Start). VRC 700 treated as black box — inner loop closes on `Hc1ActualFlowTempDesired`.

| Loop | Interval | Action |
|---|---|---|
| Outer | 15 min | Thermal solver → target flow → initial curve |
| Inner | ~60s | Proportional feedback on `Hc1ActualFlowTempDesired` |

Converges in 1–2 ticks. No runtime learning (EMA ran away — see AGENTS.md). On shutdown: restore `Z1OpMode=auto`, `Hc1HeatCurve=0.55`.

### Modes

| Mode | Behaviour |
|---|---|
| `occupied` | Full comfort targeting |
| `away` | 15°C frost protection (curve 0.30, ~£0.50/day vs ~£2.50). Week away saves ~£14. Warm-up ramp before return |
| `disabled` / `monitor-only` | No eBUS writes |

API: port 3031. `/mode/occupied`, `/mode/away`, `/kill` (baseline restore).

## Overnight strategy

Coast after 23:00 (curve 0.10), then preheat at latest safe time for Leather ≥20°C by 07:00.

### Algorithm

1. Simulate cooling: τ=50h toward equilibrium (outside + 2.5°C internal gains)
2. Scan 30-min steps backward from 07:00: can HP reheat to 20.5°C in remaining time?
3. Pick **latest** start with 30-min safety margin
4. Below 2°C: maintain 19.5°C continuously

### Empirical model parameters

| Parameter | Code value | Empirical | n | Status |
|---|---|---|---|---|
| τ (cooling) | **50h** | 50h median | 53 segments | ✅ Updated |
| K (reheat: surplus W per °C/h) | 7,500 | ~20,600 median | 27 segments | ⚠ Not yet updated — each coast night validates |

Two independent sources agree on τ≈50h: calibration nights (median 51h, n=18) and DHW mini-experiments (median 50h, n=35). Best single overnight (Night 2, 3.9h continuous no-heating): τ=65.8h. Every DHW charge is a cooling experiment; every heating restart is a reheat experiment.

### Heating recovery by outside temperature

| Outside | Heat output | COP | MWT |
|---|---|---|---|
| -2–0°C | 5700W | 3.08 | 30.5°C |
| 2–4°C | 5180W | 3.65 | 31.3°C |
| 6–8°C | 4045W | 4.81 | 30.2°C |
| 10–12°C | 2913W | 6.06 | 28.3°C |

### Known limitations

- K=7500 likely wrong (empirical K≈20,600) — each coast night validates
- No solar gain in reheat estimate (conservative)
- Uses average overnight outside temp (should use Open-Meteo hourly forecast)

## HP contention with DHW

| Outside | DHW comfort cost |
|---|---|
| <2°C | ~0.5°C drop (unrecoverable) |
| 5°C | ~0.3°C, recovers ~1h |
| 10°C | ~0.2°C, recovers ~30 min |
| 15°C | Negligible |

On cold days schedule DHW at 22:00 to keep preheat window clear. See [DHW plan](dhw-plan.md).

## Room priorities

- **Leather** (primary): emonth2, τ=50h. Optimise for this when doors closed
- **Aldora** (secondary): second comfort anchor
- **Conservatory**: excluded (30m² glass, sub-hour τ)

**Door sensors** (2× SNZB-04P, in hand, not fitted):

| Stage | Action | When |
|---|---|---|
| 1. Log | Pair `leather_conservatory_door` + `leather_hall_door`, add to decision log | Now |
| 2. Analyse | Correlate door state with Leather trajectory | 1–2 weeks |
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
| 1 | Close Elvina trickle vents | FREE | Elvina 2–3°C below other bedrooms |
| 2 | Aldora rad upgrade (reuse existing) | FREE | MWT 47→45°C |
| 3 | Jack&Carol bay draught-strip | ~£30 | 60–150W |
| 4 | EWI on SE wall (~30m²) | ~£5k | 19% demand reduction |

## Current state

| Component | Status |
|---|---|
| V2 two-loop control | ✅ Inner loop converges in 1 tick |
| V2 live solver | ✅ Daytime comfort maintained in clean windows |
| V2 overnight planner | 🟡 τ=50h + break fix deployed 4 Apr. First real coast night pending |
| T1-based DHW | 🟡 T1 queried. Scheduling logic not implemented |
| Open-Meteo forecast | 🟡 Designed, not implemented |
| Door sensors | ⚪ Hardware in hand |

## Next steps

1. **Deploy and validate first coast nights** — each night is an experiment: predicted vs actual coast duration, Leather trajectory, and preheat timing
2. **Validate K (reheat rate)** — if Leather doesn't reach 20.5°C by 07:00 after calculated preheat start, increase K. Every DHW charge also provides reheat data
3. **Fit door sensors** — Stage 1 (log only)
4. **Converge τ and K** — need 10+ coast nights across 0–15°C
5. **Morning DHW coordination** — with later preheat starts, DHW contention picture changes

### Later

- Event-driven outer loop (DHW→heating transition, door changes)
- Direct flow temp control (`SetModeOverride` to HMU)
- Open-Meteo forecast integration

## Decisions

- **SP=19 night mode**: zero rad leakage at curve 0.10. Eliminates Optimum Start
- **No runtime learning**: EMA ran away. Static calibration only
- **Thermal model drives initial guess**: inner loop converges regardless
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
```
