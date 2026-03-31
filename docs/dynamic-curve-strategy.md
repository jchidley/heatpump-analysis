# Dynamic Weather Compensation Curve Strategy

Date: 31 March 2026

## Background

The VRC 700 weather compensation is a single straight line:

```
Flow temp = Setpoint + Curve × (Setpoint − Outside temp)
```

Currently fixed at `Hc1HeatCurve = 0.55`, `Z1DayTemp = 21°C`. This is a compromise — too high on mild days (wastes COP), can't help on cold days (HP at capacity). The VRC 700 runs weather comp only, no room temp feedback.

We have what Vaillant doesn't:
- 13 room temperatures (Zigbee + emonth2), updating every 5 minutes
- Calibrated thermal model: HTC 261 W/K, k=0.039/hr, τ=25.8 hours
- Per-room equilibrium solver (knows the minimum MWT for each room to reach target)
- Real-time compressor utilisation, flow/return temps, HP state (all via eBUS)

## What we found

### The HP performance envelope

At lower flow temp, the Arotherm delivers **more** heat at **better** COP:

| Flow temp | Heat output | COP | Elec input |
|---|---|---|---|
| 55°C | 5,800W | 3.06 | 1,895W |
| 45°C | 6,100W | 3.77 | 1,618W |
| 35°C | 6,800W | 4.48 | 1,518W |

(Spec data at −3°C outside. The compressor does less work per cycle at lower pressure ratio, so it cycles refrigerant faster.)

The constraint isn't the HP — it's the radiators. They deliver less heat at lower MWT. The curve sets the flow temp, which determines how much heat the rads can distribute.

### Two operating regimes

**Capacity-limited (Tout < 7°C):** HP at or near max. Compressor utilisation >70%. Raising the curve asks for higher flow temp that the HP can't deliver. The compressor is the bottleneck. Nothing to optimise — run at current curve 0.55.

**COP-limited (Tout > 7°C):** HP has spare capacity. Compressor often below 40%. HP cycles on/off at minimum modulation (2.2kW). Lowering the curve reduces flow temp, improves COP, and prevents cycling.

### Why cycling is bad

At minimum modulation the HP can't go below 2.2kW. When demand drops below that, it cycles: run → overshoot → stop → cool → restart. Each start wastes 2–3 minutes re-establishing refrigerant pressure and heating cold heat exchangers. At 6 cycles/hour that's a 10–15% COP penalty plus compressor wear.

Lowering the flow temp reduces rad output, so the HP runs continuously at minimum mod instead of cycling. Same heat delivered, better COP, less wear.

### Thermal model validation

Equilibrium solver results — minimum MWT where all heated rooms stay ≥18°C:

| Outside temp | Min MWT | COP at that MWT | Constraint room |
|---|---|---|---|
| 0°C | >32°C | 3.7 | elvina (14.8°C even at MWT 32) |
| 3°C | >32°C | 4.2 | elvina |
| 5°C | >32°C | 4.5 | elvina |
| 7°C | 32°C | 4.8 | elvina (18.2°C) |
| 10°C | 28°C | 5.8 | hall (18.3°C) |

Below 7°C, even MWT 32°C can't get elvina to 18°C (trickle vents — priority 1 fix on roadmap). Above 10°C, MWT 28°C keeps every room comfortable.

### Annual impact

- ~120 days above 7°C in the heating season where the dynamic curve helps
- ~5–10% COP improvement on those days
- ~£12–16/year saving at 17p/kWh
- Reduced compressor cycling wear (unquantified but real)

## Proposed control logic

### Decision inputs (read-only, every cycle)

| Source | Data | Method |
|---|---|---|
| eBUS | `Broadcast/Outsidetemp` | `ebusd read` via TCP |
| eBUS | `hmu/CurrentCompressorUtil` | `ebusd read` via TCP |
| eBUS | `hmu/RunDataFlowTemp` | `ebusd read` via TCP |
| eBUS | `700/Hc1ActualFlowTempDesired` | `ebusd read` via TCP |
| Zigbee/emonth2 | Room temperatures | MQTT subscription or InfluxDB query |

### Decision outputs (writes, only when value changes)

| Target | Data | Method |
|---|---|---|
| VRC 700 | `Hc1HeatCurve` | `ebusd write -c 700 Hc1HeatCurve <value>` via TCP |

Single numeric write. No TTM timer encoding. The VRC 700 recalculates flow temp from the new curve within 10 seconds.

### Control loop

```
Every 15 minutes:
  1. Read outside temp, compressor util, room temps
  2. IF compressor_util < 40% AND coldest_heated_room > 19°C:
       → Lower curve toward 0.40 (step by 0.05 per cycle, floor 0.35)
  3. IF compressor_util > 85% OR any heated room < 18°C:
       → Raise curve toward 0.55 (step by 0.05 per cycle, cap 0.60)
  4. IF curve changed: write to VRC 700
```

### Why 15 minutes

- Outside temp changes ~0.5°C/hour — 15 min resolution is more than enough
- Room temps update every 5 min (Zigbee) — need at least 3 readings to see a trend
- The house time constant is 26 hours — nothing happens fast
- eBUS writes should be infrequent to avoid confusing the VRC 700
- Polling eBUS reads are cheap (ebusd caches most values from the 10-second broadcast cycle)

### Guardrails

- **Curve floor:** 0.35 (below this, rads can't distribute enough even on mild days)
- **Curve ceiling:** 0.60 (above this, HP is at capacity and higher curve just raises the request it can't meet)
- **Rate limit:** max 0.05 change per 15-min cycle (no sudden jumps)
- **Night mode:** During setback (00:00–04:00), don't adjust — let the VRC 700 run at setback settings
- **DHW lockout:** During DHW (BCF > 900), don't adjust — the diverter valve is on DHW circuit
- **Minimum hold time:** Don't change curve if last change was <30 min ago

### Monitoring

Write every curve change to InfluxDB for analysis:
```
hp_control curve=0.45,outside_temp=9.2,compressor_util=35,coldest_room=18.8,reason="cycling_prevention"
```

## Implementation

### Architecture

Rust binary on pi5data. Runs as a systemd service alongside z2m-hub and ebusd-poll.

```
ebusd (TCP :8888) ← reads/writes ← hp-curve-controller
MQTT (Mosquitto)  ← room temps   ← hp-curve-controller
InfluxDB          ← logging       ← hp-curve-controller
```

### Why Rust

- Already cross-compiling for aarch64 (pi5data) from this project
- z2m-hub is the same pattern (Rust binary, systemd service, eBUS + MQTT)
- No runtime dependencies (no Python, no uv, no npm)

### Deployment

Same as z2m-hub: cross-compile, scp, systemctl restart.

```bash
cargo build --release --target aarch64-unknown-linux-gnu
scp target/aarch64-unknown-linux-gnu/release/hp-curve-controller pi5data:/tmp/
ssh pi5data "sudo mv /tmp/hp-curve-controller /usr/local/bin/ && sudo systemctl restart hp-curve-controller"
```

### Future extensions

1. **Weather forecast:** Read Met Office or Open-Meteo forecast. If warming trend, don't raise curve on a cold morning.
2. **Equilibrium solver integration:** Instead of fixed curve thresholds, compute the optimal MWT from the thermal model for current conditions.
3. **Pre-DHW boost:** When DHW window is approaching and compressor has headroom, bump curve briefly to bank 0.5°C.
4. **Defrost prediction:** Monitor air inlet temp + humidity. Pre-heat slightly when defrost is likely.
5. **Seasonal learning:** Track actual room temps vs model predictions, adjust curve thresholds over time.

Items 2–5 require the thermal model crate as a dependency. Item 1 is a simple HTTP call. Start with the basic curve adjustment, add complexity only if the data shows it helps.

## Relationship to EWI

The dynamic curve optimises the margin within the current fabric constraints. EWI changes the constraints:

| | Without EWI | With EWI |
|---|---|---|
| Cold days (< 3°C) | HP at capacity, curve can't help | HP has 1,442W headroom, curve can optimise |
| Transition (3–7°C) | Comfort vs COP tradeoff | Both achievable |
| Mild days (> 7°C) | 5–10% COP gain from lower curve | Same gain, but from a higher baseline COP |

The dynamic curve is worth building now — it's free, helps on mild days immediately, and the same code works better after EWI. It's not a substitute for EWI.
