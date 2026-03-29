# How the Operating Model Works

This document explains how the tool classifies heat pump operating states and why the approach was chosen.

## The Vaillant Arotherm Plus

The Arotherm Plus has a **fixed internal pump speed** per unit size. Unlike some heat pumps that modulate flow rate, the Arotherm keeps it constant. The only thing that changes the measured flow rate is the **diverter valve** switching between the heating circuit and the DHW cylinder coil (which has lower flow resistance).

### Flow rates by model

| Model | Heating Flow Rate | Source |
|-------|------------------|--------|
| 3.5 / 5 kW | ~860 L/h = **14.3 L/min** | [Energy Stats UK](https://energy-stats.uk/mass-flow-rate/) |
| 7 kW | ~1,200 L/h = **20.0 L/min** | |
| 10 / 12 kW | ~2,000 L/h = **33.3 L/min** | |

This tool is configured for the **5kW model**. The thresholds in the code would need adjusting for other sizes — in particular, the 7kW model's heating flow rate (20.0 L/min) overlaps with the 5kW's DHW flow rate, so a different classification strategy would be needed.

For the 5kW, the diverter valve switching produces a clear bimodal distribution: heating at 14.3–14.4 L/min, DHW peaking at ~20.7 L/min, with a near-empty gap between 14.5 and 16.0 L/min.

## Four Operating States

| State | How it's detected | What's happening |
|-------|-------------------|------------------|
| **Idle** | Electrical power ≤ 50W | Compressor off, system standby |
| **Heating** | Flow rate 14.0–14.5 L/min, positive heat output | Space heating via radiators |
| **DHW** | Flow rate ≥ 15.0 L/min (enter) / < 14.7 (exit), positive heat output | Charging the hot water cylinder |
| **Defrost** | Negative heat output or negative delta-T | Reverse cycle melting ice off the outdoor unit |

### Why not use flow temperature?

The initial approach classified DHW as flow temperature > 38°C. This missed:
- DHW ramp-up periods (flow temp starts low and climbs)
- Late-stage DHW where the cylinder is nearly charged
- DHW at mild outside temperatures

Flow rate gives a clean binary signal because it's controlled by a mechanical valve, not a continuous variable.

### Why not use the DHW_flag feed?

The emoncms installation has a `DHW_flag` feed (512889) from the emonTxV5, but it only has data until December 2024. Flow rate works for the entire dataset.

## Hysteresis

The diverter valve takes a few seconds to move. During this transition, flow rate passes through the 14.5–16.0 L/min zone. To avoid rapid state-switching:

- **Enter DHW** when flow rate rises above **15.0** L/min
- **Exit DHW** when flow rate drops below **14.7** L/min

Analysis of the transition zone (1,241 samples across 17 months) shows 67% are DHW→heating ramp-downs. Only 3% are heating→DHW ramp-ups (the valve moves faster in that direction).

**Note**: These thresholds were tightened in March 2026 (originally 16.0/15.0) because DHW flow dropped from 21.0 to 16.8 L/min due to y-filter sludge buildup. The tighter thresholds are safe because heating is software-clamped at 14.3 L/min. See [hydraulic-analysis.md](hydraulic-analysis.md) for the full flow rate degradation timeline and diagnosis.

## Defrost

During defrost, the heat pump reverses its refrigerant cycle to melt ice on the outdoor coil. It extracts heat **from** the water in the heating/DHW circuit, so:
- Return temperature becomes higher than flow temperature (negative delta-T)
- Heat meter reads negative (heat flowing backwards)
- The compressor is still running (electrical power > 50W)

Defrost can happen at **any** flow rate — it depends on which position the diverter valve is in when defrost triggers. About 85% of defrost events occur at the heating flow rate (14.3 L/min), 10% at DHW flow rates.

After defrost ends (heat output returns positive, delta-T returns positive), the system returns to whichever productive state it was in before.

## Gap Filling

The monitoring logger occasionally drops out, creating gaps in the instantaneous data (power, temperatures, flow rate). However, the **cumulative energy meters** (kWh) run continuously.

Gap filling works by:
1. Building a model of typical power/temperature values for each 1°C outside temperature bin
2. Generating per-minute estimates during each gap
3. **Scaling** the power estimates so their time-integral matches the cumulative meter readings

This means the total energy during gaps is exact (from the meters), but the minute-by-minute profile is approximate. All synthetic data is stored in a separate database table and only included in analysis when explicitly requested with `--include-simulated`.

## Monitoring Setup

The emoncms feeds come from an **emonHP** monitoring bundle:

| Feed | Source | Notes |
|------|--------|-------|
| `electric_Power/Energy` | SDM120 MID meter (Modbus) | Inline on AC circuit |
| `heatmeter_*` | M-Bus heat meter | Flow, return, flow rate, cumulative kWh |
| `outside_temperature` | Met Office feed | Updates less frequently than HP feeds (~hourly vs ~10s) |
| `temperature/humidity` | emonth2 (node 23) | Wireless sensor in **Leather room only** — not whole-house. Battery at 2.4V (feed 503103) |
| `DHW_flag` | emonTxV5 | Only has data until Dec 2024 — not used for classification |

The outside temperature feed's lower resolution matters for gap-filling: the temperature-bin model can only be as granular as the Met Office updates.

### DHW scheduling

DHW timer windows are set via eBUS on the VRC 700: **05:30–07:00, 13:00–15:00, 22:00–00:00** (aligned to Octopus Cosy tariff periods). Morning DHW starts at 05:30 to give the HP 1.5h of house heating first (04:00–05:30) at Cosy rate. DHW mode is eco (mild season) or normal (cold season, switched manually on the Arotherm controller). See `docs/overnight-strategy-analysis.md` for the full analysis behind these timings.

Previously (before 29 Mar 2026), DHW triggered at ~05:05 and ~13:05 daily under the old VRC 700 schedule.

Previously, an emergency DHW auto-trigger script on pi5data forced a cylinder recharge via eBUS when prolonged draws were detected. This was **removed March 2026** — replaced by manual boost via z2m-hub dashboard (`~/github/z2m-hub/`).

### eBUS and Multical metering (added March 2026)

In addition to the emonHP bundle, the system now has:
- **eBUS adapter** — decodes internal HP communication (operating mode, compressor speed, target flow temp, cylinder temp, COP calculations). eBUS provides real-time HP state, but **`StatuscodeNum` is unreliable for DHW detection** — code 134 appears during both off/frost standby AND active DHW charging. The Rust thermal model uses `BuildingCircuitFlow` (L/h) instead: > 900 = DHW, 780–900 = heating, < 100 = off.
- **Multical DHW meter** on emondhw — measures the secondary (tap water) side of the cylinder, giving T1 (hot out), T2 (cold in), flow rate, and thermal power. This enables end-to-end DHW efficiency tracking.

Both feed into InfluxDB on pi5data via MQTT bridges. DHW remaining litres tracking is handled by z2m-hub (`~/github/z2m-hub/`), which polls ebusd directly via TCP, detects charge completion, and tracks usage via Multical volume register — writing `dhw.remaining_litres` to InfluxDB. See [dhw-cylinder-analysis.md](dhw-cylinder-analysis.md) for full details including cylinder specification, stratification model, temperature optimisation, and live monitoring setup.

## Validation

The operating model was validated against the full dataset: 448,000 running samples from October 2024 to March 2026.

| State | Samples | % | Avg COP | Avg Flow Rate |
|-------|---------|---|---------|---------------|
| Heating | 358,565 | 81.4% | 5.32 | 14.4 L/min |
| DHW | 74,145 | 16.8% | 3.89 | 20.4 L/min |
| Defrost | 7,561 | 1.7% | −5.60 | 14.7 L/min |
