<!-- code-truth: 33e263a -->

# Architecture

## Module Dependencies

```
main.rs
  ├── emoncms.rs        (feeds, sync only)
  ├── db.rs             (all commands except feeds)
  │     └── emoncms.rs    (sync_feeds, sync_all call Client)
  ├── analysis.rs       (data, summary, cop-by-temp, hourly, degree-days,
  │     └── reference.rs    indoor-temp, dhw, cop-vs-spec, design-comparison)
  ├── octopus.rs        (octopus, gas-vs-hp, baseload)
  │     ├── reads JSON from ~/github/octopus/dist/data/
  │     ├── reads emoncms outside_temp via rusqlite (optional db conn)
  │     └── receives enriched DataFrames from analysis::enrich() (via main.rs)
  └── gaps.rs           (gaps, fill-gaps)
        └── db.rs tables directly via rusqlite (no db.rs API)
```

**Key boundary:** `analysis.rs` has no dependency on `emoncms.rs`, `db.rs`, or `octopus.rs`. It receives Polars DataFrames and pre-loaded data arrays, and outputs to stdout. Pure analysis layer.

**`octopus.rs` bridges two data sources:** It loads Octopus JSON independently, but for `gas-vs-hp` it also receives an enriched DataFrame (with state machine classifications) from `analysis.rs` via `main.rs`. It reads emoncms outside temps directly from SQLite for hybrid temperature.

**`reference.rs`** is a data-only module (constants and simple functions). Used by `analysis.rs` for design comparison, COP vs spec, and DHW comparison. Gas-era DHW estimate (11.82 kWh/day) is also used by `octopus.rs`.

**`gaps.rs` bypasses `db.rs`:** The gap-filling module queries and writes to SQLite tables directly using raw SQL, rather than going through `db.rs` functions.

## Data Flow

### Sync (API → SQLite)

```
emoncms.org API
    ↓ (reqwest, 7-day chunks, 60s interval)
emoncms::Client::feed_data()
    ↓
db::sync_all()
    ↓ (INSERT OR IGNORE)
SQLite: samples table (7.4M rows, 12 feeds)
```

### Core Analysis (SQLite → Polars → stdout)

```
SQLite: samples (+ optional simulated_samples)
    ↓
db::load_dataframe_inner()   — builds DataFrame from SQL queries
    ↓
analysis::enrich()           — adds cop, delta_t, state columns
    ↓                          (state machine runs over raw arrays)
analysis::summary() etc.     — Polars lazy queries, prints results
    ↓
stdout
```

### Octopus Comparison (JSON + SQLite → stdout)

```
~/github/octopus/dist/data/consumption.json     (half-hourly elec+gas, Apr 2020+)
~/github/octopus/dist/data/weather.json          (daily ERA5 temps+HDD)
    ↓
octopus::load_consumption()   — JSON → Polars DataFrame
octopus::load_weather()       — hybrid: emoncms feed 503093 for HP era,
    ↓                           ERA5+1.0°C bias correction for gas era
    ↓
For gas-vs-hp:
    SQLite → db::load_dataframe() → analysis::enrich()
        ↓
    octopus::daily_hp_by_state()  — 1-min power × state → daily kWh by state
        ↓
    octopus::print_gas_vs_hp()    — gas era: total gas − DHW estimate (11.82 kWh/day)
                                    HP era: measured heating + DHW from state machine
    ↓
stdout
```

### Degree Days / Design Comparison (SQLite → arrays → stdout)

```
SQLite: samples
    ↓
db::load_daily_outside_temp()  — daily min/mean/max outside temp
db::load_daily_energy()        — daily cumulative kWh (last value per day)
    ↓
analysis::degree_days()        — HDD, energy/HDD, monthly, gas-era comparison
analysis::design_comparison()  — radiator output, HTC, gas vs HP savings
    ↓
stdout (+ reference::house, reference::gas_era, reference::radiators)
```

### Gap-Filling (SQLite → model → SQLite)

```
SQLite: samples table
    ↓
gaps::find_gaps()            — window function over elec_power feed
    ↓
gaps::TempBinModel::from_db() — 6-way JOIN, bins by outside temp
    ↓
gaps::fill_gap()             — per-minute estimates, energy-scaled
    ↓
SQLite: simulated_samples table + gap_log table
```

## Temperature Hierarchy

Two sources of outside temperature with different coverage and accuracy:

| Source | Feed/File | Coverage | Resolution | Accuracy |
|--------|-----------|----------|------------|----------|
| emoncms 503093 | Met Office hourly | Oct 2024 → present | ~hourly | Best — local station |
| ERA5-Land | weather.json | Apr 2020 → present | Daily mean | +1.0°C cold bias vs emoncms |

`octopus.rs::load_weather()` builds a unified daily temperature series:
- HP-era dates: uses emoncms (513 days)
- Gas-era dates: uses ERA5 + 1.0°C correction (1,656 days)
- Bias derived from 507-day overlap: mean +1.00°C, stdev 0.57°C
- Without correction, ERA5 overstates HDD by ~14%

## How Changes Propagate

- **Adding a new feed to sync:** Add to `db::FEED_IDS`. If it should appear in analysis DataFrames, also add to the `feed_cols` array in `db::load_dataframe_inner()`.

- **Changing operating state thresholds:** Modify constants at top of `analysis.rs` (`DHW_ENTER_FLOW_RATE`, `DEFROST_DT_THRESHOLD`, etc.). Also update `gaps.rs` which has hardcoded `flow_rate >= 16.0` for DHW classification. Update the module doc comment in `analysis.rs`. This will change `daily_hp_by_state()` output in `octopus.rs`.

- **Changing degree day base temperature:** Modify `HDD_BASE_TEMP` in `analysis.rs` AND `HDD_BASE_C` in `octopus.rs` — they must match. Note `reference::house::BASE_TEMP_GAS_ERA` (17°C) is a separate value.

- **Updating reference data (house, radiators, gas):** Edit `reference.rs`. These are compile-time constants. `octopus.rs` also has `GAS_DHW_KWH_PER_DAY` (11.82) and `BOILER_EFFICIENCY` (0.9) which should stay in sync with `reference.rs` values.

- **Changing the database schema:** Modify `db::open()` for core tables, `gaps::ensure_schema()` for simulation tables. Existing databases get new tables via `CREATE TABLE IF NOT EXISTS` but won't migrate existing tables.

- **Adding a new analysis command:** Add variant to `Commands` enum in `main.rs`, add a `pub fn` to `analysis.rs` or `octopus.rs`, route in `main()`.

- **Updating Octopus data:** Run `cd ~/github/octopus && bash scripts/run_dashboard.sh`. No code changes needed — `octopus.rs` reads the JSON at runtime.

- **Changing the ERA5 bias correction:** Modify `ERA5_BIAS_CORRECTION_C` in `octopus.rs`. Current value (+1.0°C) derived from 507-day overlap. Actual bias varies +0.6 to +1.8°C by month.

- **Adding a new CLI flag:** Add to `Cli` struct in `main.rs`. If it affects date range, update `resolve_time_range()`. If it affects data loading, thread through `load_dataframe()`.

## External Boundaries

| System | Protocol | Where |
|--------|----------|-------|
| emoncms.org | HTTPS REST API, read-only via API key | `emoncms.rs` |
| SQLite | File-based, WAL mode | `db.rs`, `gaps.rs`, `octopus.rs` (read-only) |
| Octopus JSON files | Local filesystem (~/github/octopus/dist/data/) | `octopus.rs` |
| stdout | Polars formatted tables, manual println | `analysis.rs`, `octopus.rs` |

## Concurrency

None. Everything is single-threaded, blocking. `reqwest` is used in blocking mode. SQLite is accessed single-connection.
