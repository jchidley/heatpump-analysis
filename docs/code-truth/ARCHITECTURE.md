<!-- code-truth: 9d6d3e3 -->

# Architecture

## Module Dependency Graph

```
main.rs
  ├── config.rs      (load at startup, global singleton)
  ├── emoncms.rs     (reads config for base_url)
  ├── db.rs          (reads config for feed IDs, sync start)
  │     └── emoncms.rs (sync only)
  ├── analysis.rs    (reads config for thresholds, house, gas_era, arotherm)
  │     └── config.rs
  ├── gaps.rs        (reads config for feed IDs, thresholds)
  │     └── config.rs
  └── octopus.rs     (reads config for thresholds, gas_era)
        └── config.rs
```

Key constraint: **analysis.rs has no dependency on db.rs or emoncms.rs**. It operates purely on Polars DataFrames passed in by main.rs. This separation means analysis functions can be called with any DataFrame that has the right columns.

## Data Flow Through the System

### Sync path (online)

```
emoncms.org API → emoncms.rs::Client → db.rs::sync_all() → SQLite (samples table)
```

`sync_all()` iterates `config().emoncms.feeds`, fetches 7-day chunks at 60s resolution, and stores non-null values. Each feed tracks its last-synced timestamp independently.

### Analysis path (offline)

```
SQLite → db.rs::load_dataframe() → Polars DataFrame
                                          │
                              analysis.rs::enrich()
                                          │
                              ┌────────────┼────────────┐
                              ▼            ▼            ▼
                          summary()    degree_days()  cop_vs_spec()
                                       (+ daily temps  (+ config
                                        + daily energy)  arotherm)
```

`enrich()` is the central transformation — it adds `cop`, `delta_t`, and `state` columns. All downstream analysis functions require an enriched DataFrame.

### Gap-fill path (writes back to SQLite)

```
db.rs::find_gaps() → gaps.rs::TempBinModel::from_db() → gaps.rs::fill_gap()
                                                                │
                                                     simulated_samples table
```

Gap filling bypasses `db.rs` — it writes directly to `simulated_samples` and `gap_log` tables via its own SQL. This is a deliberate design choice (gaps.rs manages its own schema) but means the two modules must agree on feed ID naming.

### Octopus path (external JSON + SQLite)

```
~/github/octopus/dist/data/consumption.json → octopus.rs::load_consumption()
~/github/octopus/dist/data/weather.json     → octopus.rs::load_weather()
                                                  │
                                          (optionally joins with
                                           db.rs outside_temp for
                                           emoncms temperatures)
```

The Octopus module is semi-independent — it reads its own JSON files and only optionally touches the SQLite database for temperature data.

## Configuration Architecture

`config.toml` is loaded once at startup by `main.rs` and stored in a `once_cell::OnceCell` static. All modules access it via `config::config()`, which returns `&'static Config`.

The config has six sections:

| Section | Consumers |
|---------|-----------|
| `emoncms` (feeds, base_url, sync start) | db.rs, emoncms.rs, gaps.rs |
| `thresholds` (elec_running_w, dhw flow rates, defrost DT, HDD base) | analysis.rs, gaps.rs, octopus.rs |
| `house` (HTC, floor area, design temps, construction) | analysis.rs |
| `arotherm` (manufacturer spec curve) | analysis.rs |
| `radiators` (15 radiators with room, dimensions, T50 watts) | analysis.rs |
| `gas_era` (boiler efficiency, annual totals, monthly data) | analysis.rs, octopus.rs |

Feed definitions include an optional `column` field that maps feed IDs to DataFrame column names. Feeds without a `column` (e.g. humidity, battery, dhw_flag) are synced to SQLite but not loaded into analysis DataFrames.

## State Machine Design

The operating state classifier in `analysis.rs::classify_states()` is a **deterministic hysteresis state machine** that processes rows strictly in time order.

```
          ┌───────────────────────────────────┐
          │              IDLE                  │
          │         elec ≤ 50W                 │
          └──────┬────────────────┬────────────┘
                 │ elec > 50W     │ elec > 50W
                 │ h > 0, dt > 0  │ h ≤ 0 or dt < -0.5
                 ▼                ▼
          ┌──────────┐    ┌──────────┐
          │ HEATING  │◄──►│ DEFROST  │
          │ fr < 16  │    │ any fr   │
          └────┬─────┘    └──────────┘
               │ fr ≥ 16         ▲
               ▼                 │
          ┌──────────┐           │
          │   DHW    │───────────┘
          │ fr ≥ 15  │  h ≤ 0 or dt < -0.5
          └──────────┘
```

The hysteresis zone (15.0–16.0 L/min) prevents rapid switching during the ~3-second diverter valve transition. The machine remembers the pre-defrost state to return correctly after defrost events that can happen at any flow rate.

## SQLite Schema

Three core tables (created by `db.rs::open()`):

```sql
feeds (id TEXT PK, name, tag, unit)
samples (feed_id TEXT, timestamp INTEGER, value REAL) WITHOUT ROWID  -- PK: (feed_id, timestamp)
sync_state (feed_id TEXT PK, last_timestamp INTEGER)
```

Two optional tables (created by `gaps.rs::ensure_schema()`):

```sql
simulated_samples (feed_id TEXT, timestamp INTEGER, value REAL, gap_start_ts INTEGER) WITHOUT ROWID
gap_log (start_ts INTEGER PK, end_ts, duration_min, elec_kwh, heat_kwh, method, samples_generated)
```

WAL mode is enabled for concurrent read performance. Schema uses `CREATE TABLE IF NOT EXISTS` — no migration system.

## Change Propagation

| If you change... | You must also... |
|-----------------|------------------|
| A feed ID in `config.toml` | Nothing — all modules resolve by name. But existing SQLite data uses old IDs. |
| A threshold in `config.toml` | Re-run analysis. Existing simulated samples used old thresholds — consider `DELETE FROM simulated_samples` and re-running `fill-gaps`. |
| A new feed in `config.toml` | Add `column` if it should appear in analysis DataFrames. Run `sync` to fetch data. |
| DataFrame column names | Update `config.toml` feed `column` fields. Check analysis.rs and octopus.rs column references. |
| Add a new analysis function | Add to `analysis.rs`, add `Commands` variant in `main.rs`, wire up in `match`. |
| Arotherm model size | All thresholds are model-specific (especially flow rates). The 7kW heating rate overlaps the 5kW DHW rate. |
