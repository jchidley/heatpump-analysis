# Heating Reference Data

Domain reference for the control surface, tuning constants, and VRC 700 behaviour. Loaded by agents when reasoning about control changes. For operating policy and decisions, see [Heating plan](heating-plan.md).

## VRC 700 heat curve formula

```
flow_temp = setpoint + curve × (setpoint - outside)^1.25
```

Exponent 1.25 is best-fit from pilot data (Vaillant says 1.10 — underpredicts by 2.5–3.1°C at curves ≥0.50).

Inverse: `curve = (target_flow - setpoint) / (setpoint - outside)^1.25`

## SP=19 rationale

- Curve 0.10 = genuinely zero rad output (no formula leakage)
- Any overnight heating is a deliberate curve raise
- Curves stay under 1.50 warning up to 15°C outside

**VRC 700 is opaque.** Back-solving gives effective setpoint ~20°C (not 19 or 21). Hidden `Hc1MinFlowTempDesired`=20°C floor. Undocumented Optimum Start ramp (~3h before day timer). Night mode eliminates all of this. **Do not model the formula. Inner loop closes on `Hc1ActualFlowTempDesired`.**

## Inner loop tuning

| Parameter | Value |
|---|---|
| Gain | 0.05 (halved to 0.025 below curve 0.25) |
| Deadband | 0.5°C (doubled to 1.0°C below curve 0.25) |
| Max step | 0.20 |
| Curve clamp | 0.10–4.00 |
| Convergence | 1–2 ticks |

**ΔT stabilisation**: live flow-return ΔT only when `RunDataStatuscode` contains "Heating" + "Compressor". Otherwise `default_delta_t_c` = 4.0°C.

**No runtime learning**: `room_offset` EMA ran away to +2.18°C overnight (learned cooling trend as "model error", suppressed preheat by ~8°C). Static calibration only.

## Comfort guard layers

1. **Hard constraints**: any heated room <18°C → raise curve. DHW active → don't adjust
2. **COP optimisation**: gradient-follow toward better COP, stop when rooms cool
3. **Context**: tariff, door states, occupancy, forecast

## Writable eBUS registers

| Register | Purpose |
|---|---|
| `Hc1HeatCurve` | Primary control lever (0.10–4.00) |
| `Z1OpMode` | 0=off, 1=auto, 2=day, **3=night** |
| `Z1DayTemp` / `Z1NightTemp` | Room setpoint |
| `Hc1MaxFlowTempDesired` / `Hc1MinFlowTempDesired` | Flow temp bounds |
| `HwcSFMode` | DHW boost (auto / load) |
| `HwcTempDesired` | DHW target temp |
| `Z1QuickVetoTemp` | Temporary override |

Future: `SetModeOverride` to HMU bypasses VRC 700. Message format decoded (D1C encoding).

## System pressure

`FlowPressure` (HMU): 2.01 bar heating, 1.90 bar DHW (hydraulic circuit volume effect — 3-way valve), 2.05 bar idle. Rock steady 1.98–2.03 bar daily mean over 30 days. `WaterPressure` (700) returns empty. `RunDataHighPressure` (HMU) is refrigerant, not water.

## Deployment (pi5data)

| Component | Location |
|---|---|
| Binary | `/home/jack/adaptive-heating-mvp/target/release/adaptive-heating-mvp` |
| Config | `/home/jack/adaptive-heating-mvp/model/adaptive-heating-mvp.toml` |
| Thermal geometry | `/home/jack/adaptive-heating-mvp/data/canonical/thermal_geometry.json` |
| Systemd unit | `/etc/systemd/system/adaptive-heating-mvp.service` |
| Secrets | `/etc/adaptive-heating-mvp.env` (root:root 0600) |
| State | `/home/jack/.local/state/adaptive-heating-mvp/state.toml` |
| Decision log | `/home/jack/.local/state/adaptive-heating-mvp/actions.jsonl` |

Build: `source ~/.cargo/env && cd ~/adaptive-heating-mvp && cargo build --release`

Deploy source: `scp src/bin/adaptive-heating-mvp.rs pi5data:~/adaptive-heating-mvp/src/main.rs`

## Resolved observations

- **`CurrentCompressorUtil`**: reads negative values (−29, −55, −89, −102). Unreliable — do not use for control
- **Sawtooth flag**: `daytime_model` ↔ `hold` alternations during DHW charges. Not real oscillation
- **Service hang** (2 Apr ~12:46): reqwest had no timeout. Fixed: 10s timeout on all HTTP
- **2 Apr door-open**: Leather stuck 19.6–19.9°C for 6h — conservatory door open (~1,500W cold air). Inner loop correctly compensated
