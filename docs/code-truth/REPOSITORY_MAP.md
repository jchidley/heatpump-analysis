<!-- code-truth: 33e263a -->

# Repository Map

```
Cargo.toml                # Dependencies: polars, rusqlite, reqwest, clap, chrono, anyhow, serde, serde_json
Cargo.lock                # Pinned dependency versions
.gitignore                # Excludes target/ and heatpump.db
LICENSE-MIT               # Dual license
LICENSE-APACHE            # Dual license
CLAUDE.md                 # LLM agent context — commands, architecture, measured performance, gotchas
README.md                 # Human-facing: quick start, command reference, links to docs

src/
  main.rs                 # CLI definition (clap) + command routing. 20 subcommands.
                          # Date range parsing (--from/--to/--all-data/--days).
  emoncms.rs              # Emoncms REST API client: list_feeds, feed_data. ~81 lines.
  db.rs                   # SQLite schema, sync, DataFrame loading, daily energy/temp queries.
                          # Single source of truth for feed IDs (FEED_IDS constant).
  analysis.rs             # State machine (classify_states) + all Polars analysis queries.
                          # Module doc comment is the Arotherm operating model reference.
                          # Degree days, COP vs spec, design comparison, indoor temp, DHW.
  octopus.rs              # Octopus Energy integration:
                          #   load_consumption()    — JSON → Polars DataFrame (half-hourly)
                          #   load_weather()         — hybrid temp: emoncms + bias-corrected ERA5
                          #   daily_totals()         — aggregate to daily elec/gas kWh
                          #   daily_hp_by_state()    — aggregate enriched HP data by state
                          #   print_summary()        — monthly breakdown with HDD
                          #   print_gas_vs_hp()      — gas era vs HP era, heating/DHW separated
                          #   print_baseload()       — whole-house minus HP electricity
  reference.rs            # Static data from planning workbook:
                          #   house:: — HTC, U-values, design heat loss, construction notes
                          #   arotherm:: — manufacturer COP curve at -3°C
                          #   radiators:: — 15 Stelrad units with T50 ratings, output calculator
                          #   gas_era:: — monthly Octopus data, boiler efficiency, hot water
  gaps.rs                 # Gap detection, TempBinModel, gap-filling with energy scaling.
                          # Accesses SQLite directly (not through db.rs).

docs/
  explanation.md          # Diátaxis explanation: operating model, Arotherm flow rates,
                          #   hysteresis, defrost, gap-filling, monitoring setup, DHW scheduling
  roadmap.md              # Status: eBUS (planned), Octopus (done), solar PV+battery (planned)
  octopus-data-inventory.md # Full audit of Octopus data sources, coverage, tariff history
  code-truth/             # This directory — derived-from-code documentation

heatpump.db               # SQLite database (gitignored). Created by 'sync' command.
                          # Tables: feeds, samples, sync_state, simulated_samples, gap_log
```

## External Data (not in this repo)

```
~/github/octopus/dist/data/
  consumption.json        # Half-hourly elec + gas, Apr 2020 → present. Gas in kWh (converted).
  weather.json            # Daily mean temp + HDD (ERA5-Land). Some recent nulls.
```

Produced by the `~/github/octopus/` project's preload pipeline. Refresh with `cd ~/github/octopus && bash scripts/run_dashboard.sh`.

## What Lives Where

| To change... | Look at... |
|-------------|-----------|
| CLI commands or flags | `main.rs` — `Commands` enum, `Cli` struct, `resolve_time_range()` |
| Operating state thresholds (DHW/defrost) | `analysis.rs` — constants at top of file |
| Which feeds are synced | `db.rs` — `FEED_IDS` constant |
| How data is loaded into Polars | `db.rs` — `load_dataframe_inner()` |
| Analysis queries (summary, COP, hourly, etc.) | `analysis.rs` — individual `pub fn` functions |
| Degree day calculations | `analysis.rs` — `degree_days()`, `HDD_BASE_TEMP` |
| Design comparison / COP vs spec | `analysis.rs` — `cop_vs_spec()`, `design_comparison()` |
| House thermal properties | `reference.rs` — `house::` module |
| Radiator data | `reference.rs` — `radiators::` module |
| Manufacturer spec curve | `reference.rs` — `arotherm::` module |
| Gas-era consumption data | `reference.rs` — `gas_era::` module |
| Gap-filling model or strategy | `gaps.rs` — `TempBinModel` and `fill_gap()` |
| Emoncms API interaction | `emoncms.rs` — `Client` struct |
| Database schema | `db.rs` — `open()` function |
| Simulated data schema | `gaps.rs` — `ensure_schema()` |
| Octopus data loading / path | `octopus.rs` — `default_data_dir()`, `load_consumption()`, `load_weather()` |
| Temperature bias correction | `octopus.rs` — `ERA5_BIAS_CORRECTION_C` constant |
| Gas-vs-HP comparison logic | `octopus.rs` — `print_gas_vs_hp()`, `daily_hp_by_state()` |
| Gas-era DHW estimate | `octopus.rs` — `GAS_DHW_KWH_PER_DAY` constant (from `reference.rs`) |
| Boiler efficiency assumption | `octopus.rs` — `BOILER_EFFICIENCY` constant |
| Baseload analysis | `octopus.rs` — `print_baseload()` |
| Octopus data refresh | `~/github/octopus/scripts/run_dashboard.sh` (external) |

## Feed ID Mapping

Feed IDs are hardcoded in two files:

| Location | Feed IDs | Purpose |
|----------|----------|---------|
| `db.rs: FEED_IDS` | All 12 feeds | Which feeds to download during sync |
| `db.rs: load_dataframe_inner()` | 7 analysis feeds | Which feeds to load into DataFrames |
| `db.rs: load_daily_outside_temp()` | Feed `503093` | Daily outside temp for degree days |
| `db.rs: load_daily_energy()` | Feeds `503095`, `503097` | Cumulative energy for daily/degree day |
| `gaps.rs: TempBinModel::from_db()` | 6 feeds (hardcoded in SQL) | Model building |
| `gaps.rs: find_gaps()` | Feed `503094` | Gap detection reference feed |
| `gaps.rs: fill_gap()` | 5 feeds + `503093` | Gap-filling writes + outside temp lookups |
| `octopus.rs: load_weather()` | Feed `503093` (via db.rs) | Emoncms outside temp for hybrid weather |

## Tests

**None.** There are no `#[cfg(test)]` modules, no test files, no integration tests.

## Generated / External

- `heatpump.db` — generated by `sync`, not checked in
- `~/github/octopus/dist/data/*.json` — generated by octopus project's preload pipeline, not in this repo
- `LICENSE-*` and base `README.md` template — generated by GitHub skill
- `Cargo.lock` — auto-generated, checked in
