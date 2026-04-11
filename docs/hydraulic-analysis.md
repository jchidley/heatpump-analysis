# Hydraulic Analysis: Pump, Flow Rates, and System Resistance

This document is a self-contained investigation report covering flow-rate degradation, root-cause diagnosis, and remediation. The canonical current operating-state thresholds live in [`../lat.md/domain.md#Operating States`](../lat.md/domain.md#operating-states); flow-rate monitoring context lives in [`../lat.md/infrastructure.md`](../lat.md/infrastructure.md).

## How the Arotherm 5kW circulates water

The Vaillant Arotherm Plus 5kW (VWL 55/6) has a single internal circulation pump that serves both heating and DHW circuits. A diverter valve switches flow between the radiator circuit and the cylinder coil. The pump runs on a fixed-speed curve — it does not vary speed to maintain a flow target.

However, the Arotherm **software clamps** heating flow to a maximum of 860 L/h (14.3 L/min). If the pump could deliver more at the system's resistance, the controller reduces pump speed until flow equals the target. This is configured via the installer menu setting `Conf. Heat. Build. Pump` (settable 50–100% or Auto; Auto targets 860 L/h for the 5kW).

For DHW, the pump runs at its configured DHW speed (`Conf. DHW. Build. Pump`), typically higher or uncapped, allowing maximum flow through the lower-resistance cylinder circuit.

**Source:** [energy-stats.uk — DT5 and Mass Flow Rate](https://energy-stats.uk/mass-flow-rate/), Vaillant installer quick guide pump curve.

## Pump curve (VWL 55/6)

From the Vaillant technical documentation (curve 1, lower line in `Pump_curve.png`):

```
Remaining feed pressure (kPa) vs Volume flow (L/h)

  ~78 kPa ─┐
            │▁▁▁▁▁
            │      ╲
            │       ╲
  ~56 kPa ──│────────● 860 L/h (14.3 L/min) — heating spec point
            │         ╲
            │          ╲
            │           ╲
  ~20 kPa ──│────────────● ~1000 L/h (16.7 L/min) — DHW now
            │             ╲
   ~5 kPa ──│──────────────● ~1260 L/h (21.0 L/min) — DHW at commissioning
            │               ╲
     0 kPa ─┘────────────────── → Flow
            0   400  800  1200  1600  L/h
```

"Remaining feed pressure" is the head available to push water through the **external** system (pipes, valves, cylinder coil, radiators) after internal heat exchanger losses.

**Key spec (from Vaillant technical data sheet):**

| Parameter | VWL 55/6 |
|-----------|----------|
| Volume flow, minimum | 400 L/h (6.7 L/min) |
| Volume flow, maximum | 860 L/h (14.3 L/min) |
| Remaining feed pressure at max flow | 56.0 kPa (560 mbar) |

## Three flow regimes

The system operates at three distinct flow rates, each revealing different information:

### Heating: 14.3 L/min (860 L/h) — software-clamped

The pump is throttled to deliver exactly 860 L/h regardless of system resistance. At this flow, the pump has **56 kPa of remaining head** — massive margin. The software will maintain 14.3 L/min until the system is so clogged that the pump at maximum speed physically cannot deliver 860 L/h.

**Heating flow is useless as a diagnostic.** It will read 14.3 right up to catastrophic failure.

### DHW: 16–21 L/min (960–1260 L/h) — pump-limited

When the diverter switches to the cylinder, the pump runs unrestricted (or at a higher configured speed). The cylinder coil has lower resistance than the radiator circuit, so flow increases. The actual flow rate is determined by the intersection of the pump curve and the system resistance curve.

At commissioning, DHW flow was **21.0 L/min** — near the bottom of the pump curve with only ~5 kPa of remaining head. There was almost no margin from day one.

**DHW flow is the first casualty of any resistance increase** because the pump curve is steepest here. A small increase in system resistance causes a large drop in flow.

### Idle: 8–11 L/min (510–660 L/h) — uncontrolled

When the system is circulating without active heating or DHW demand (e.g., pump overrun, standby), flow settles at whatever the pump delivers against the system's base resistance. No software clamping, no high-flow demand.

**Idle flow is the best early warning indicator.** It responds honestly to resistance changes without software masking.

## Pipework layout (DHW circuit)

The DHW circuit shares the primary pipework with heating up to the diverter valve, then branches to the cylinder:

```
Arotherm → [y-filter] → 35mm Cu (~6m, several 90° and 45° bends)
         → [diverter valve]
         → 28mm Cu (short run)
         → 22mm Cu (cylinder coil)
         → 28mm Cu → 35mm Cu → return to Arotherm
```

The 35mm primary then branches to three 22mm heating circuits (each serving a zone of radiators). A y-mesh filter sits on the 35mm primary, upstream of the diverter — every circuit passes through it.

### Theoretical pressure drops at commissioning flow (1259 L/h)

| Section | Velocity | Pressure drop |
|---------|----------|--------------|
| 35mm Cu, 12m (flow+return) + bends | 0.43 m/s | ~1.5 kPa |
| 28mm Cu, ~4m + bends | 0.66 m/s | ~1.2 kPa |
| 22mm Cu coil, ~3m + bends | 1.11 m/s | ~4.1 kPa |
| Diverter valve (estimate) | — | ~2.0 kPa |
| **Total theoretical** | | **~8.8 kPa** |
| **Pump curve reading** | | **~5 kPa** |

The theoretical and measured values are in the same ballpark (the pump curve is hard to read precisely at the steep end). The 22mm cylinder coil dominates the resistance at DHW flow rates.

## Flow rate degradation timeline

Data from emoncms feed 503100 (flow rate, L/min):

| Period | Idle | Heating | DHW | Notes |
|--------|------|---------|-----|-------|
| Oct–Nov 2024 (commissioning) | 10.9 | 14.3 | 21.0 | Baseline |
| Dec 2024 – Oct 2025 | 11.3–11.6 | 14.4 | 20.5 | Stable, slight improvement as system beds in |
| Nov–Dec 2025 | 11.2–11.4 | 14.3 | 20.2–20.4 | Barely perceptible drift |
| Early Jan 2026 | — | 14.3 | 20.0 | Heavy defrost period (0°C, 60–116 defrost samples/day) |
| **30 Jan – 5 Feb** | 10.5→10.1 | 14.3 | **20.0→19.2** | **Step drop 1** |
| **6–8 Feb** | 10.1→9.2 | 14.3 | **19.2→18.2** | **Step drop 2** |
| Feb–early Mar | 9.0→8.5 | 14.3 | 18.0→17.0 | Continued decline |
| Mar 2026 | **8.5** | **14.3** | **16.8** | Stabilised but degraded |

### Correlation with defrost activity

The system ran heavy defrost cycles 4–10 January 2026 (outside temps 0–2°C). Flow rates were stable during this period. The first drop came **three weeks later** on 30 January. The hydraulic disturbance of repeated defrost cycling (flow reversal, on/off cycling) likely mobilised magnetite that had been settled in low-flow areas, which then migrated and partially blocked the y-filter mesh over the following weeks.

## Root cause: magnetite sludge in y-filter

On inspection (March 2026), the y-mesh filter on the 35mm primary had **fine black magnetite sludge** coating the mesh, plus larger particulates caught in the mesh.

This is consistent with:
- The system uses deionised water (no limescale), ruling out scale
- Magnetite is generated by corrosion of steel radiators and iron components
- The filter sits in the shared primary — blocking it affects all circuits equally
- Two step-changes rather than gradual fouling: debris mobilised by defrost, lodged in filter mesh in stages

### Resistance increase

Using the pump curve to extract system resistance:

| | Commissioning | Now | Change |
|---|---|---|---|
| DHW flow | 1259 L/h | 1006 L/h | −20% |
| Remaining head (from curve) | ~5 kPa | ~20 kPa | +15 kPa |
| Resistance coefficient (ΔP/Q²) | 3.2×10⁻³ | 2.0×10⁻² | **~6× increase** |

## State machine threshold adjustment

The DHW/heating state machine classifies operating mode by flow rate. As DHW flow dropped towards the original 16.0 L/min entry threshold, thresholds were tightened (March 2026):

| Threshold | Original | Updated | Rationale |
|-----------|----------|---------|-----------|
| DHW entry | 16.0 L/min | **15.0 L/min** | Heating clamped at 14.3, no false-trigger risk |
| DHW exit | 15.0 L/min | **14.7 L/min** | 0.3 L/min hysteresis sufficient with clamped heating |

This is a **monitoring workaround**, not a fix. The thresholds should be reviewed after the system is flushed and flow rates recover.

## Monitoring guidance

**Watch the idle flow rate** in emoncms (feed 503100, samples where 5 < value < 13).

| Idle flow (L/min) | System state |
|-------------------|-------------|
| > 12.0 | Healthy — clean filter baseline (post-clean Mar 2026) |
| 10.5–12.0 | Normal — commissioning-level resistance |
| 9.0–10.5 | Degraded — filter needs checking |
| 8.0–9.0 | Poor — flush or filter clean needed |
| < 8.0 | Critical — approaching minimum flow fault (400 L/h = 6.7 L/min) |

Also available via eBUS: `BuildingCircuitFlow` (L/h). Grafana alert recommended at < 600 L/h (10 L/min) — see [heating-monitoring-setup.md](../heating-monitoring-setup.md) TODO.

Post-clean baseline (19 March 2026): idle **12.6 L/min**, DHW **21.3 L/min**.

Cross-validated: eBUS `BuildingCircuitFlow` reports 760 L/h (12.7 L/min) at idle, matching the MBUS heat meter at 759 L/h (12.6 L/min). Both sources confirm the recovery.

## Post-clean results (19 March 2026)

After cleaning the y-filter mesh (removing fine black magnetite sludge and particulates):

| | Commissioning | Before clean | After clean |
|---|---|---|---|
| Idle | 10.9 L/min | 8.6 L/min | **12.6 L/min** |
| Heating | 14.3 L/min | 14.3 L/min | **14.4 L/min** |
| DHW | 21.0 L/min | 16.8 L/min | **21.3 L/min** |

All flow rates recovered fully — in fact the idle flow is **higher than commissioning** (12.6 vs 10.9), suggesting the filter was already partially loaded with installation debris when the system was first commissioned.

This confirms:
- The restriction was **entirely in the y-filter**, not the cylinder coil or pipework
- The cylinder coil is clean (no scale — deionised water, closed system)
- The pump and diverter valve are functioning correctly

## Remediation

1. ✅ **Clean the y-filter** — done 19 March 2026, full flow recovery confirmed
2. **Fit a magnetic filter** (e.g., MagnaClean) on the 35mm primary, upstream of the y-filter — catches fine magnetite that the y-mesh cannot, preventing recurrence
3. **Monitor ongoing** — watch idle and DHW flow rates for early warning of re-fouling, especially after heavy defrost periods in winter
