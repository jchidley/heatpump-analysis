<!-- code-truth: db2c2ce -->

# Architecture

## Module Dependencies

```
main.rs
  ├── emoncms.rs      (feeds, sync only)
  ├── db.rs           (all commands except feeds)
  │     └── emoncms.rs  (sync_feeds, sync_all call Client)
  ├── analysis.rs     (data, summary, cop-by-temp, hourly, degree-days,
  │     └── reference.rs  (design_comparison, cop_vs_spec, dhw_analysis)
  │                        indoor-temp, dhw, cop-vs-spec, design-comparison)
  └── gaps.rs         (gaps, fill-gaps)
        └── db.rs tables directly via rusqlite (no db.rs API)
```

**Key boundary:** `analysis.rs` has no dependency on `emoncms.rs` or `db.rs`. It receives Polars DataFrames and pre-loaded data arrays, and outputs to stdout. Pure analysis layer.

**`reference.rs`** is a data-only module (constants and simple functions). Used by `analysis.rs` for design comparison, COP vs spec, DHW comparison, and radiator output calculations.

**`gaps.rs` bypasses `db.rs`:** The gap-filling module queries and writes to SQLite tables directly using raw SQL, rather than going through `db.rs` functions. It accesses both the `samples` table (reads) and `simulated_samples` table (writes).

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

### Analysis (SQLite → Polars → stdout)

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

## How Changes Propagate

- **Adding a new feed to sync:** Add to `db::FEED_IDS`. If it should appear in analysis DataFrames, also add to the `feed_cols` array in `db::load_dataframe_inner()`.

- **Changing operating state thresholds:** Modify constants at top of `analysis.rs` (`DHW_ENTER_FLOW_RATE`, `DEFROST_DT_THRESHOLD`, etc.). Also update `gaps.rs` which has hardcoded `flow_rate >= 16.0` for DHW classification. Update the module doc comment in `analysis.rs`.

- **Changing degree day base temperature:** Modify `HDD_BASE_TEMP` in `analysis.rs`. Note `reference::house::BASE_TEMP_GAS_ERA` (17°C) is a separate value used for gas-era comparison.

- **Updating reference data (house, radiators, gas):** Edit `reference.rs`. These are compile-time constants, not loaded from files.

- **Changing the database schema:** Modify `db::open()` for core tables, `gaps::ensure_schema()` for simulation tables. Existing databases get new tables via `CREATE TABLE IF NOT EXISTS` but won't migrate existing tables.

- **Adding a new analysis command:** Add variant to `Commands` enum in `main.rs`, add a `pub fn` to `analysis.rs`, route in `main()`.

- **Adding a new CLI flag:** Add to `Cli` struct in `main.rs`. If it affects date range, update `resolve_time_range()`. If it affects data loading, thread through `load_dataframe()`.

## External Boundaries

| System | Protocol | Where |
|--------|----------|-------|
| emoncms.org | HTTPS REST API, read-only via API key | `emoncms.rs` |
| SQLite | File-based, WAL mode | `db.rs`, `gaps.rs` |
| stdout | Polars formatted tables, manual println | `analysis.rs` |

## Concurrency

None. Everything is single-threaded, blocking. `reqwest` is used in blocking mode. SQLite is accessed single-connection.
