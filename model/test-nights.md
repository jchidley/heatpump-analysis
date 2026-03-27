# Controlled Test Nights (Calibration Windows)

Use only these windows for cooldown calibration. These are the two deliberate test nights with space heating off.

## Night 1 (doors normal)
- **Window:** `2026-03-24 23:10` → `2026-03-25 05:05` (UTC)
- **Context:** Normal door states
- **Outside temp:** ~8.0°C average

## Night 2 (all doors closed)
- **Window:** `2026-03-25 23:10` → `2026-03-26 05:05` (UTC)
- **Context:** All internal doors closed (test condition)
- **Outside temp:** ~5.2°C average

## Why these exact start/end times
- Space-heating run-down finishes at ~23:10 (before this there is shutdown transient)
- DHW starts at ~05:10 (status 134 and high power)
- So calibration uses **23:10→05:05** to avoid both transitions

## Exclusions
- Do **not** include DHW cycle (from ~05:10 onward)
- Do **not** include daytime or random cooldown periods
- Night 3 (26-27 Mar, no-setback trial) is for validation only, not calibration

## Data signals used to define windows
- `hp_status.csv` (status codes)
- `hp_state.csv` (`electric_Power`, `heatmeter_Power`)

During the calibration windows:
- Electrical power is mostly ~4–15W
- Heat power is mostly ~0W with occasional small transient spikes
