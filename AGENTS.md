# AGENTS.md

## What This Is

CLI tool that syncs heat pump data from emoncms.org to local SQLite, then analyses it with Polars. Vaillant Arotherm Plus 5kW at 6 Rhodes Avenue, London N22 7UT.

## Data Access

- emoncms dashboard: `https://emoncms.org/app/view?name=MyHeatpump&readkey=1b00410c57d5df343ede7c09e6aab34f`
- Read API key (read-only, safe to share): `1b00410c57d5df343ede7c09e6aab34f`

## Commands

| Task | Command |
|------|---------|
| Build | `cargo build` |
| Run | `cargo run -- <subcommand>` |
| Sync data | `cargo run -- --apikey KEY sync` |
| Analyse (7 days) | `cargo run -- --days 7 summary` |
| Analyse (date range) | `cargo run -- --from 2025-01-01 --to 2025-01-31 summary` |
| Analyse (all data) | `cargo run -- --all-data all` |
| With simulated | `cargo run -- --all-data --include-simulated summary` |
| Export CSV | `cargo run -- --days 30 export -o output.csv` |
| Degree days | `cargo run -- --all-data degree-days` |
| Indoor temp | `cargo run -- --all-data indoor-temp` |
| DHW analysis | `cargo run -- --all-data dhw` |
| COP vs spec | `cargo run -- --all-data cop-vs-spec` |
| Design comparison | `cargo run -- --all-data design-comparison` |
| Gap report | `cargo run -- gaps` |
| Fill gaps | `cargo run -- fill-gaps` |
| Octopus summary | `cargo run -- octopus` |
| Gas vs HP comparison | `cargo run -- --all-data gas-vs-hp` |
| Baseload analysis | `cargo run -- --all-data baseload` |

`--apikey` only needed for `feeds` and `sync`. Analysis reads from `heatpump.db`.
Octopus commands read from `~/github/octopus/dist/data/` (consumption.json, weather.json).
`gas-vs-hp` and `baseload` also need `heatpump.db` for HP state machine data.

## Architecture

```
config.toml   → All domain constants, thresholds, feed IDs, reference data (TOML)
config.rs     → Deserializes config.toml into typed structs (global singleton)
emoncms.rs    → API client (used only by sync)
db.rs         → SQLite storage + DataFrame loading
analysis.rs   → State machine + all Polars queries (no DB/API dependency)
gaps.rs       → Gap detection + synthetic data (accesses SQLite directly)
octopus.rs    → Octopus Energy integration + gas-vs-HP comparison
main.rs       → CLI routing (20 subcommands)
```

## Octopus Energy Integration

Data flows from the `~/github/octopus/` project into heatpump-analysis:

```
Octopus REST API → usage CSVs → merge → preload CLI → consumption.json + weather.json
                                                              ↓
                                                    octopus.rs loads JSON
                                                    + emoncms DB for HP state machine
```

### Data sources and coverage
- **Electricity**: Apr 2020 → present (half-hourly, 99k+ records)
- **Gas**: Apr 2020 → Jul 2024 (half-hourly, gas supply ended at HP install)
- **Gap**: 102 days Dec 2023 → Mar 2024 (meter/comms outage, unfillable)
- **Weather**: ERA5-Land daily temps + HDD, bias-corrected (see below)

### Temperature hierarchy
1. **emoncms feed 503093** (Met Office hourly) — used for HP era (Oct 2024+), most accurate
2. **ERA5-Land** (weather.json) — used for gas era, bias-corrected by +1.0°C
   - Derived from 507-day overlap: emoncms reads 1.0°C warmer on average (range +0.6 to +1.8°C by month)
   - ERA5 overstates HDD by ~14% without correction
   - HDD base: 15.5°C (UK standard)

### Refreshing Octopus data
```bash
cd ~/github/octopus && bash scripts/run_dashboard.sh
```
This fetches latest REST data, merges with legacy parquet, and regenerates the JSON files.

### Tariff history
| Period | Electricity | Gas |
|--------|------------|-----|
| Apr 2020 → Dec 2020 | Fix 15.03p | Fix 2.56p |
| Dec 2020 → Dec 2022 | Agile (avg 26.35p) | Variable 2.73→3.83p |
| Dec 2022 → Apr 2023 | Flex 51.38p (crisis) | Variable 10.31p |
| Apr 2023 → Apr 2024 | Flex ~30p | Variable 7–12p |
| Apr 2024 → Nov 2024 | Agile (avg 17.98p) | Variable 5–6p |
| Nov 2024 → Oct 2025 | **Cosy** (off 14.63, mid 29.82, peak 44.74p) | — |
| Oct 2025 → present | **Cosy Fix** (off 14.05, mid 28.65, peak 42.97p) | — |

Cosy time slots: off-peak 04–07, 13–16, 22–00 (9h); mid 00–04, 07–13, 19–22 (12h); peak 16–19 (3h).
82.6% of HP-era electricity lands in off-peak slots.

## Key Measured Performance

All from actual data (state machine + Octopus + emoncms):

| Metric | Value | Source |
|--------|-------|--------|
| Heating COP | 4.74 | State machine, heating days with HDD > 0.5 |
| DHW COP | 3.46 | State machine |
| Overall COP (instantaneous) | 5.09 | analysis.rs summary |
| Heating heat/HDD | 8.8 kWh | HP era, emoncms temps |
| Gas-era heating heat/HDD | 9.2 kWh | Gas × 90% − DHW est., ERA5+1.0°C |
| DHW demand | 11.0 kWh/day | HP heat meter, state machine |
| Annual HDD (5-yr avg) | 1,503 | Bias-corrected ERA5, base 15.5°C |
| House baseload | ~9 kWh/day | Octopus whole-house − HP SDM120 |
| Blended elec rate (HP era) | 17.07p/kWh | Octopus Cosy, consumption-weighted |
| Weighted gas rate (all gas era) | 5.66p/kWh | Octopus tariffs, consumption-weighted |

### Annual cost comparison (current tariffs, typical weather)

| | Gas combi | Heat pump |
|---|---|---|
| Gas consumed | 19,157 kWh | — |
| Electricity consumed | — | 3,951 kWh |
| Fuel cost | £1,125 | £674 |
| Gas standing charge | £115 | £0 |
| **Total** | **£1,239** | **£674** |
| **Annual saving** | | **£565 (46%)** |

Cost per kWh of heat: gas 6.29p (5.87p ÷ 90%), HP heating 3.60p (17.07p ÷ COP 4.74).
Break-even gas price: 2.92p/kWh — gas hasn't been that cheap since early 2021.
Insulation improvements between eras reduced heat/HDD by ~4% (9.2 → 8.8).

## Key Domain Model

Operating states classified by flow rate (Arotherm 5kW fixed pump = 14.3 L/min):
- **Heating**: flow_rate 14.0–14.5, DT > 0, heat > 0
- **DHW**: flow_rate ≥ 16.0 (enter) / < 15.0 (exit), DT > 0, heat > 0
- **Defrost**: heat ≤ 0 OR DT < −0.5 (any flow rate)
- **Idle**: elec ≤ 50W

Thresholds: `analysis.rs` top-of-file constants. Also hardcoded in `gaps.rs` (`flow_rate >= 16.0`).

## Feed Notes

- `503101` (indoor_temp) = emonth2 sensor in **Leather room only**, not whole-house
- `503093` (outside_temp) = Met Office hourly, not Arotherm OAT sensor. Reads ~1.0°C warmer than ERA5-Land (507-day overlap). Ground truth for HP era; ERA5 bias-corrected +1.0°C for gas era.
- `512889` (DHW_flag) = dead since Dec 2024
- Solar PV + battery system installed (not yet integrated):
    - 7× Trina 440W panels (TSM-440NEG9RC.27), 3.08 kWp, single string
    - Fox ESS F3600 inverter (3.6kW, dual MPPT — one MPPT input used)
    - Tesla Powerwall 2 (13.5 kWh) + Gateway
    - Commissioned: 19/04/2024, Emlite M24 generation meter

## Reference Data (reference.rs)

- House: HTC 261 W/°C, floor area 180m², solid brick + 2010-standard top floor
- Radiators: 15× Stelrad, total T50 = 25,133W, output calculator with correction factor
- Arotherm spec: COP curve at -3°C (35°C→4.48, 55°C→3.06)
- Gas era: 18,702 kWh/yr gas, 90% boiler, 11.82 kWh/day hot water
- Insulation improved between gas and HP eras (heat/HDD dropped ~4%)
- Solid wall insulation planned but not yet done

## Gotchas

- All domain constants, feed IDs, thresholds, and reference data live in `config.toml` — edit there, not in code
- `config.toml` must be next to the executable or in the current working directory
- `gaps.rs` bypasses `db.rs` — writes to SQLite tables directly
- No tests — validate changes against real data output
- Simulated data in separate table (`simulated_samples`) — never mixed unless `--include-simulated`
- DB schema is `CREATE TABLE IF NOT EXISTS` — no migrations
- Polars pinned to 0.46 (0.53 available) — untested on newer versions. `strings` feature added for octopus.rs.
- Outside temp feed (Met Office) is lower resolution (~hourly) than HP feeds (~10s)
- Thresholds are 5kW-specific — 7kW model would need different values (its heating rate = 20 L/min overlaps 5kW DHW rate)
- Two different HDD base temps: 15.5°C (UK standard in thresholds) vs 17°C (gas-era regression in house config)
- `octopus.rs` reads JSON files from `~/github/octopus/dist/data/` — path hardcoded in `default_data_dir()`
- `gas-vs-hp` uses `daily_hp_by_state()` which converts 1-min power samples to energy assuming exactly 1/60 hour per sample — accurate for 1-min data but would overcount if sample interval changes
- Gas-era DHW estimated at 11.82 kWh/day (from config) — not measured. HP-era DHW is measured by state machine.
- ERA5 bias correction (+1.0°C) is a single constant in octopus.rs — actual bias varies +0.6 to +1.8°C by month. Monthly correction would be more accurate but the constant is adequate for seasonal/annual analysis.

## Planned Enhancements

See [docs/roadmap.md](docs/roadmap.md) for full details:
- **eBUS** — adapter is physically connected but not configured. Would give real-time OAT, compressor speed, defrost status, cylinder temp
- **Octopus Energy** — ✅ integrated. See "Octopus Energy Integration" above.
- **Solar PV + battery** — system installed, details above. Self-consumption analysis, DHW scheduling to solar peak
- **Cost analysis subcommand** — the tariff data and cost calculations are currently ad-hoc Python scripts; could be a proper Rust subcommand
- Other data in `C:\Users\jackc\OneDrive\Documents\House\`: degree day CSVs (EGWU), utility bills, Octopus Agile rates, weekly consumption

## Boundaries

- Don't change operating state thresholds without re-validating against the full dataset
- Don't mix simulated and real data by default
- Don't commit `heatpump.db` or API keys
- Don't modify `~/github/octopus/` from this project — refresh via `run_dashboard.sh`
- Keep `HDD_BASE_C` in `octopus.rs` in sync with `HDD_BASE_TEMP` in `analysis.rs`
- Keep `GAS_DHW_KWH_PER_DAY` and `BOILER_EFFICIENCY` in `octopus.rs` in sync with `reference.rs`
- Human-facing docs: `docs/` (Diátaxis style) — see `docs/code-truth/` for derived-from-code docs
- This file (`AGENTS.md`) is the single LLM context source. `docs/code-truth/` is for human comprehension.
