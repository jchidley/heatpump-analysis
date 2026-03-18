<!-- code-truth: 07b9b7f -->

# Architecture

## Module Dependencies

```
main.rs
  ├── emoncms.rs      (feeds, sync only)
  ├── db.rs           (all commands except feeds)
  │     └── emoncms.rs  (sync_feeds, sync_all call Client)
  ├── analysis.rs     (data, summary, cop-by-temp, hourly, all)
  └── gaps.rs         (gaps, fill-gaps)
        └── db.rs tables directly via rusqlite (no db.rs API)
```

**Key boundary:** `analysis.rs` has no dependency on `emoncms.rs` or `db.rs`. It receives a Polars `DataFrame` and returns nothing — all output is `println!`. This makes it a pure analysis layer.

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
SQLite: samples table
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

- **Changing operating state thresholds:** Modify constants at top of `analysis.rs` (`DHW_ENTER_FLOW_RATE`, `DEFROST_DT_THRESHOLD`, etc.). The state machine in `classify_states()` uses these. Also update the module doc comment. The gap-filling model in `gaps.rs` uses a hardcoded `flow_rate >= 16.0` for DHW classification — this would need updating too.

- **Changing the database schema:** Modify `db::open()` for core tables, `gaps::ensure_schema()` for simulation tables. Existing databases will get new tables via `CREATE TABLE IF NOT EXISTS` but won't migrate existing tables.

- **Adding a new analysis command:** Add variant to `Commands` enum in `main.rs`, add a `pub fn` to `analysis.rs`, route in `main()`.

- **Adding a new CLI flag:** Add to `Cli` struct in `main.rs`. If it affects data loading, thread it through `load_dataframe()`.

## External Boundaries

| System | Protocol | Where |
|--------|----------|-------|
| emoncms.org | HTTPS REST API, read-only via API key | `emoncms.rs` |
| SQLite | File-based, WAL mode | `db.rs`, `gaps.rs` |
| stdout | Polars formatted tables | `analysis.rs` |

## Concurrency

None. Everything is single-threaded, blocking. `reqwest` is used in blocking mode. SQLite is accessed single-connection.
