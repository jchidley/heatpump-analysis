# Roadmap

Planned enhancements, roughly ordered by value and readiness. Completed items are in git history.

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

## Rust Thermal Model — Remaining Ports

See [rust-migration-plan.md](rust-migration-plan.md) for the full plan.

**Infrastructure done** (2026-03-29): `thermal.rs` split into 15 submodules + DRY cleanup. `geometry.rs` and `physics.rs` are now cleanly importable for new ports.

**Deleted** (2026-03-30):
- ~~`model/calibrate.py`~~ — superseded by Rust `thermal-calibrate`
- ~~`model/overnight.py`~~ — superseded by Rust `overnight`

**Three Python commands remain** (in `model/house.py`):

1. ~~`thermal-rooms`~~ — ✅ Ported 2026-03-30
2. ~~`thermal-connections`~~ — ✅ Ported 2026-03-30
3. `thermal-analyse` — live energy balance from InfluxDB (medium complexity)
4. `thermal-equilibrium` — steady-state solver (high complexity, high value)
5. `thermal-moisture` — humidity analysis (medium complexity, lower priority)

After all ported, mark `model/house.py` as legacy.

## Other Potential Enhancements

- **Cost analysis subcommand** — tariff data and cost calculations as a proper Rust subcommand
- **Defrost analysis** — dedicated report on defrost frequency, duration, energy cost vs outside temp/humidity
- **Multi-period comparison** — "this January vs last January" with degree-day normalisation
- **Alerting** — detect COP degradation, unusual cycling, sensor drift
- **Weather forecast correlation** — predict next-day heating demand from Met Office forecast
