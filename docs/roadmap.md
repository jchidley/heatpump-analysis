# Roadmap

Planned enhancements, roughly ordered by value and readiness. Completed items are in git history.

## Adaptive Heating Control

**Status:** V1 pilot complete (31 Mar – 1 Apr 2026). V2 design written. V1 running on `pi5data` while V2 is implemented.

See [`adaptive-heating-control.md`](adaptive-heating-control.md) for the strategy, [`adaptive-heating-mvp.md`](adaptive-heating-mvp.md) for V1 spec + pilot results, [`adaptive-heating-v2-design.md`](adaptive-heating-v2-design.md) for V2 model-predictive design.

| Priority | Item | Status |
|----------|------|--------|
| ✅ | VRC 700 writable control-surface discovery (~25 registers confirmed) | Done |
| ✅ | V1 MVP deployed, pilot run, bugs fixed | Done |
| ✅ | VRC 700 heat curve formula: `flow = setpoint + curve × (setpoint - outside)^1.27` | Done |
| ✅ | V1 pilot findings: bang-bang fails, thermal model needed, curve ping-pong | Done |
| ✅ | V2 design: model-predictive control with equilibrium solver + heat curve formula | Done |
| 🟡 | V2 implementation: `target_mwt_for_leather()` + curve lookup in controller | Next |
| 🟡 | Overnight optimisation: calculated start time replacing fixed 19°C setback | Next |
| 🟡 | Predictive DHW planning: pre-compensate before known DHW charges | Next |
| 🟡 | Open-Meteo forecast (24h temp + solar + humidity): drives daytime curve trajectory and overnight planner | V2 core |
| 🔵 | Direct flow temp control via SetModeOverride (bypass VRC 700 curve) | Future |
| 🔵 | Leather door sensors → disqualify Leather when open | Waiting on hardware |
| 🔵 | Derive Aldora proxy band from historical data | Future |
| 🔵 | DHW hygiene / legionella risk monitoring | Future |

Real objective: Leather at 20–21°C during waking hours at minimum cost, with reliable DHW during Cosy windows. Overnight temperature is a free variable, not a target.

## Pico W eBUS Adapter

**Status:** Design complete, waiting for xyzroe eBus-TTL adapter boards to arrive (~few weeks).

See [`pico-ebus-plan.md`](pico-ebus-plan.md) for the full build plan.

Replaces the closed-source ESP32 firmware + ebusd stack with our own Rust/Embassy firmware on a Pi Pico W. Passive bus listener first, then active command sending. Directly publishes decoded eBUS telegrams to MQTT.

Relationship to adaptive heating:
- currently the MVP talks to ebusd via TCP on `pi5data`
- once the Pico W adapter is working, it could replace ebusd as the eBUS interface
- the Pico W will also enable direct `SetMode` observation, which the adaptive controller can use for richer feedback
- the two projects are independent but complementary

## Solar PV + Battery Integration

**Status:** System installed and commissioned 19/04/2024. Not yet integrated into analysis.

| Component | Details |
|-----------|---------|
| **Panels** | 7× Trina Vertex S+ 440W (TSM-440NEG9RC.27), 3.08 kWp |
| **Inverter** | Fox ESS F3600, 3.6kW single phase |
| **Battery** | Tesla Powerwall 2, 13.5 kWh, with Gateway |

### What it would give us

- **Self-consumption** — how much HP electricity comes from solar vs grid?
- **Effective COP** — solar kWh at £0 changes the economics
- **DHW scheduling** — shift more DHW to afternoon Cosy when solar available?

### Implementation notes

- Tesla Powerwall Gateway provides local API for battery state, solar generation, grid import/export
- Key analysis: overlay HP consumption on solar generation + battery state timeline
- With Octopus data: marginal cost per kWh consumed (grid vs solar vs battery)

## Physical Improvements (from thermal model)

See [room-thermal-model.md](room-thermal-model.md) for full methodology and data.

| Priority | Action | Cost | Impact |
|----------|--------|------|--------|
| 1 | Close Elvina trickle vents | FREE | Removes system bottleneck — MWT 49→47°C at -3°C |
| 2 | Aldora rad upgrade (reuse existing 909W DP DF) | FREE | MWT 47→45°C. Unblocks trickle vent for mould risk. |
| 3 | Jack&Carol bay window draught-strip | ~£30 | Moisture-proven ACH 1.00–1.89. Saves ~60–150W |
| 4 | EWI on SE wall (~30m²) | ~£5k DIY | 19% heat demand reduction. MWT 49→43°C at -3°C |
| 5 | Sterling floor insulation | ~£200 | Leather keeps heat, Sterling gets cold room, HP saves energy |

FRVs deprioritised — HP at capacity on cold days, FRVs redistribute insufficient output.

## Remaining eBUS Opportunities

- **Defrost analysis** — eBUS provides definitive defrost status (516) vs current inference from negative DT/heat
- **emoncms import** — eBUS data only in InfluxDB; could be added as emoncms feeds for sync pipeline

## Rust Thermal Model

**Status:** Python → Rust migration complete (2026-03-30). All thermal commands ported. `model/house.py` and `model/calibrate.py` deleted.

See [rust-migration-plan.md](rust-migration-plan.md) for the full plan.

## Other Potential Enhancements

- **Cost analysis subcommand** — tariff data and cost calculations as a proper Rust subcommand
- **Defrost analysis** — dedicated report on defrost frequency, duration, energy cost vs outside temp/humidity
- **Multi-period comparison** — "this January vs last January" with degree-day normalisation
- **Alerting** — detect COP degradation, unusual cycling, sensor drift
- **Weather forecast correlation** — predict next-day heating demand from forecast (largely addressed by V2 Open-Meteo integration)
- **Leather door sensors** — Zigbee contact sensors on Leather room doors, feed into adaptive controller
- **Aldora radiator upgrade** — reuse existing 909W DP DF, currently deferred (see Physical Improvements above)
