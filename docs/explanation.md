# How the Operating Model Works

This document explains how the tool classifies heat pump operating states and why the approach was chosen.

## The Vaillant Arotherm 5kW

The Arotherm Plus 5kW has a **fixed internal pump speed** of ~860 L/h (14.3 L/min). Unlike some heat pumps that modulate flow rate, the Arotherm keeps it constant. The only thing that changes the measured flow rate is the **diverter valve** switching between two circuits:

- **Heating circuit** (radiators/underfloor): flow rate ~14.3 L/min
- **DHW cylinder coil**: lower flow resistance → flow rate jumps to ~20.7 L/min

This produces a clear bimodal distribution in the flow rate data, with a near-empty gap between 14.5 and 16.0 L/min.

## Four Operating States

| State | How it's detected | What's happening |
|-------|-------------------|------------------|
| **Idle** | Electrical power ≤ 50W | Compressor off, system standby |
| **Heating** | Flow rate 14.0–14.5 L/min, positive heat output | Space heating via radiators/underfloor |
| **DHW** | Flow rate ≥ 16.0 L/min, positive heat output | Charging the hot water cylinder |
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

- **Enter DHW** when flow rate rises above **16.0** L/min
- **Exit DHW** when flow rate drops below **15.0** L/min

Analysis of the transition zone (1,241 samples across 17 months) shows 67% are DHW→heating ramp-downs. Only 3% are heating→DHW ramp-ups (the valve moves faster in that direction).

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

## Validation

The operating model was validated against the full dataset: 448,000 running samples from October 2024 to March 2026.

| State | Samples | % | Avg COP | Avg Flow Rate |
|-------|---------|---|---------|---------------|
| Heating | 358,565 | 81.4% | 5.32 | 14.4 L/min |
| DHW | 74,145 | 16.8% | 3.89 | 20.4 L/min |
| Defrost | 7,561 | 1.7% | −5.60 | 14.7 L/min |
