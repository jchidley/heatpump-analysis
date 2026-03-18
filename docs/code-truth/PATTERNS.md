<!-- code-truth: 07b9b7f -->

# Patterns

## Error Handling

All functions return `anyhow::Result<T>`. Errors are propagated with `?` and contextualised with `.context()` at key boundaries (e.g. DataFrame construction, database opening). No custom error types.

CLI errors surface as `Error: <message>` on stderr and exit code 1.

## Module Organisation

- One file per concern, flat structure (no subdirectories under `src/`)
- Module doc comments at the top of `analysis.rs` and `gaps.rs` serve as the operating model reference
- Constants at the top of each module, public functions below, private helpers at the bottom

## Naming

- **Functions:** `snake_case`, verb-first for actions (`fetch_dataframe`, `sync_all`, `fill_gap`), noun for queries (`summary`, `cop_by_outside_temp`)
- **Constants:** `SCREAMING_SNAKE_CASE` (`ELEC_RUNNING_W`, `DHW_ENTER_FLOW_RATE`)
- **Types:** `PascalCase` (`HpState`, `TempBinModel`, `BinStats`, `SyncStats`)
- **CLI subcommands:** kebab-case in user-facing names (`cop-by-temp`, `fill-gaps`), PascalCase in the `Commands` enum

## DataFrame Construction

DataFrames are built manually from `Vec<Option<f64>>` columns, not from CSV or Parquet. The pattern in `db::load_dataframe_inner()` and the removed `fetch_dataframe`:

1. Collect timestamps into a `Vec<i64>`
2. Cast to `Datetime(Milliseconds, UTC)` series
3. For each feed, allocate `vec![None; n]` and fill from SQL results
4. Assemble with `DataFrame::new(columns)`

## Polars Usage

- **Lazy API** for all aggregations: `.lazy()` → `.filter()` → `.group_by()` → `.agg()` → `.collect()`
- **Eager API** only for the state machine (needs row-by-row iteration over raw arrays)
- Results printed directly with `println!("{}", df)` — no custom formatting
- The `enrich()` function mixes eager (state machine) and lazy (COP/DT columns), then `hstack`s the state column onto the lazy result

## State Machine

The `classify_states()` function in `analysis.rs` is a sequential state machine that processes rows in time order:

- Takes 4 parallel arrays (elec, heat, flow_rate, delta_t)
- Maintains `current: HpState` and `pre_defrost: HpState`
- Returns `Vec<&'static str>` (not enum — for direct use as a Polars string column)
- Hysteresis: different thresholds for entering vs exiting DHW state (16.0 vs 15.0 l/m)

## SQLite Access

- Schema created via `CREATE TABLE IF NOT EXISTS` in `db::open()` and `gaps::ensure_schema()` — no migration tool
- WAL journal mode set on open
- Batch inserts use `unchecked_transaction()` + `prepare_cached()`
- `INSERT OR IGNORE` for idempotent data loading
- `WITHOUT ROWID` tables for the high-volume `samples` and `simulated_samples` tables
- `gaps.rs` accesses SQLite directly (not through `db.rs` functions)

## Gap-Filling

The `TempBinModel` builds per-°C averages from a 6-way self-join on the `samples` table. The model is applied per-minute during gap-fill, then power estimates are linearly scaled so integrated energy matches the cumulative meter readings (hard constraint from `elec_energy` and `heat_energy` feeds).

Short gaps (< 10 min) use linear interpolation instead.

## CLI Structure

clap derive macros with `#[command]` and `#[arg]` attributes. Subcommands in a `Commands` enum. Helper methods on `Cli` struct (`require_client()`, `require_db()`) for validation.

## CSV Export

The `export` command uses `polars::prelude::CsvWriter` with the `SerWriter` trait. Outputs all enriched columns (including `cop`, `delta_t`, `state`) to a file or stdout.

## Notable Absences

- **No tests** — no `#[cfg(test)]`, no test helpers, no fixtures
- **No logging** — all diagnostic output uses `eprintln!`
- **No configuration file** — all config via CLI args and env vars
- **No async** — blocking reqwest, single-threaded SQLite
- **No JSON export** — CSV only via `export` command
