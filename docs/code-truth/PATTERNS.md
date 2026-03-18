<!-- code-truth: db2c2ce -->

# Patterns

## Error Handling

All functions return `anyhow::Result<T>`. Errors are propagated with `?` and contextualised with `.context()` at key boundaries (e.g. DataFrame construction, database opening). No custom error types.

CLI validation uses helper methods on `Cli` (`require_client()`, `require_db()`) that return clear error messages before any work begins.

## Module Organisation

- One file per concern, flat structure (no subdirectories under `src/`)
- Module doc comments at the top of `analysis.rs` and `gaps.rs` serve as the operating model reference
- Constants at the top of each module, public functions below, private helpers at the bottom
- `reference.rs` uses nested `pub mod` for namespacing (`house::`, `arotherm::`, `radiators::`, `gas_era::`)

## Naming

- **Functions:** `snake_case`, verb-first for actions (`fill_gap`, `sync_all`), noun for queries (`summary`, `cop_by_outside_temp`, `degree_days`)
- **Constants:** `SCREAMING_SNAKE_CASE` (`ELEC_RUNNING_W`, `DHW_ENTER_FLOW_RATE`, `HDD_BASE_TEMP`)
- **Types:** `PascalCase` (`HpState`, `TempBinModel`, `BinStats`, `SyncStats`)
- **CLI subcommands:** kebab-case in user-facing names (`cop-by-temp`, `fill-gaps`, `degree-days`), PascalCase in the `Commands` enum

## DataFrame Construction

DataFrames are built manually from `Vec<Option<f64>>` columns in `db::load_dataframe_inner()`:

1. Collect timestamps from SQL into `Vec<i64>`
2. Cast to `Datetime(Milliseconds, UTC)` series
3. For each feed, allocate `vec![None; n]` and fill from SQL results
4. Optionally merge simulated data (fills where real is `None`)
5. Optionally add `is_simulated` boolean column
6. Assemble with `DataFrame::new(columns)`

## Polars Usage

- **Lazy API** for all aggregations: `.lazy()` → `.filter()` → `.group_by()` → `.agg()` → `.collect()`
- **Eager API** only for the state machine (needs row-by-row iteration over raw arrays)
- Results printed directly with `println!("{}", df)` — no custom formatting
- The `enrich()` function mixes eager (state machine) and lazy (COP/DT columns), then `hstack`s the state column
- CSV export via `CsvWriter` with `SerWriter` trait

## State Machine

The `classify_states()` function in `analysis.rs` is a sequential state machine:

- Takes 4 parallel arrays (elec, heat, flow_rate, delta_t)
- Maintains `current: HpState` and `pre_defrost: HpState`
- Returns `Vec<&'static str>` (for direct use as a Polars string column)
- Hysteresis: different thresholds for entering vs exiting DHW (16.0 vs 15.0 l/m)

## Reference Data

`reference.rs` encodes static planning data as compile-time constants:
- House thermal properties (HTC, U-values, design conditions)
- Radiator inventory with output correction factor calculator
- Arotherm manufacturer COP curve
- Gas-era monthly consumption for before/after comparison

The radiator correction factor uses `((flow + return) / 2 - room_temp) / 50) ^ 1.3` with return temperature estimated from a DT regression on real data.

## SQLite Access

- Schema created via `CREATE TABLE IF NOT EXISTS` — no migration tool
- WAL journal mode set on open
- Batch inserts use `unchecked_transaction()` + `prepare_cached()`
- `INSERT OR IGNORE` for idempotent data loading
- `WITHOUT ROWID` tables for high-volume `samples` and `simulated_samples`
- `gaps.rs` accesses SQLite directly (not through `db.rs` functions)
- Sync start date: `DEFAULT_SYNC_START_MS` constant in `db.rs` (2024-10-22)

## Gap-Filling

The `TempBinModel` builds per-°C averages from a 6-way self-join on `samples`. Applied per-minute during gap-fill, then power estimates linearly scaled so integrated energy matches cumulative meter readings (hard constraint). Short gaps (< 10 min) use linear interpolation.

## Date Range Handling

`resolve_time_range()` in `main.rs` supports four modes (in priority order):
1. `--all-data` — from hardcoded epoch (2024-10-22) to now
2. `--from` / `--to` — explicit date range (YYYY-MM-DD)
3. `--days N` — last N days from now (default 7)

## CSV Export

The `export` command uses `polars::prelude::CsvWriter` with the `SerWriter` trait. Outputs all enriched columns (including `cop`, `delta_t`, `state`) to file (`-o`) or stdout.

## Notable Absences

- **No tests** — no `#[cfg(test)]`, no test helpers, no fixtures
- **No logging** — all diagnostic output uses `eprintln!`
- **No configuration file** — all config via CLI args, env vars, and compile-time constants in `reference.rs`
- **No async** — blocking reqwest, single-threaded SQLite
- **No JSON export** — CSV only via `export` command
