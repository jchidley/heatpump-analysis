<!-- code-truth: 33e263a -->

# Patterns

## Error Handling

All functions return `anyhow::Result<T>`. Errors are propagated with `?` and contextualised with `.context()` at key boundaries (e.g. DataFrame construction, database opening, JSON parsing). No custom error types.

CLI validation uses helper methods on `Cli` (`require_client()`, `require_db()`) that return clear error messages before any work begins. For optional resources, `.ok()` converts `Result` to `Option` (e.g. `cli.require_db().ok()` in octopus commands where DB is needed but not fatal if missing).

## Module Organisation

- One file per concern, flat structure (no subdirectories under `src/`)
- Module doc comments at the top of `analysis.rs` and `gaps.rs` serve as the operating model reference
- Constants at the top of each module, public functions below, private helpers at the bottom
- `reference.rs` uses nested `pub mod` for namespacing (`house::`, `arotherm::`, `radiators::`, `gas_era::`)
- `octopus.rs` groups JSON schema structs (private), public loading functions, then analysis helpers

## Naming

- **Functions:** `snake_case`, verb-first for actions (`fill_gap`, `sync_all`), noun for queries (`summary`, `cop_by_outside_temp`, `degree_days`). Print functions prefixed `print_` (`print_summary`, `print_gas_vs_hp`, `print_baseload`).
- **Constants:** `SCREAMING_SNAKE_CASE` (`ELEC_RUNNING_W`, `DHW_ENTER_FLOW_RATE`, `HDD_BASE_TEMP`, `ERA5_BIAS_CORRECTION_C`)
- **Types:** `PascalCase` (`HpState`, `TempBinModel`, `BinStats`, `SyncStats`)
- **CLI subcommands:** kebab-case in user-facing names (`cop-by-temp`, `fill-gaps`, `gas-vs-hp`), PascalCase in the `Commands` enum
- **JSON structs:** `camelCase` serde rename for Octopus JSON (`ConsumptionRecord`, `WeatherRecord`)

## DataFrame Construction

DataFrames are built manually from `Vec<Option<f64>>` columns in `db::load_dataframe_inner()`:

1. Collect timestamps from SQL into `Vec<i64>`
2. Cast to `Datetime(Milliseconds, UTC)` series
3. For each feed, allocate `vec![None; n]` and fill from SQL results
4. Optionally merge simulated data (fills where real is `None`)
5. Optionally add `is_simulated` boolean column
6. Assemble with `DataFrame::new(columns)`

In `octopus.rs`, DataFrames are built from JSON via serde deserialization into `Vec<T>`, then assembled column-by-column with `Series::new()`.

## Polars Usage

- **Lazy API** for all aggregations: `.lazy()` → `.filter()` → `.group_by()` → `.agg()` → `.collect()`
- **Eager API** only for the state machine (needs row-by-row iteration over raw arrays)
- **String operations** via `.str().head()` for date-to-month extraction (requires `strings` feature)
- **strftime** for timestamp → date string: `.dt().strftime("%Y-%m-%d")`
- Results printed directly with `println!("{}", df)` — no custom formatting for core analysis
- `octopus.rs` prints with manual `println!` formatting (not Polars table display)
- The `enrich()` function mixes eager (state machine) and lazy (COP/DT columns), then `hstack`s the state column
- CSV export via `CsvWriter` with `SerWriter` trait

## State Machine

The `classify_states()` function in `analysis.rs` is a sequential state machine:

- Takes 4 parallel arrays (elec, heat, flow_rate, delta_t)
- Maintains `current: HpState` and `pre_defrost: HpState`
- Returns `Vec<&'static str>` (for direct use as a Polars string column)
- Hysteresis: different thresholds for entering vs exiting DHW (16.0 vs 15.0 l/m)

## Octopus Data Integration Pattern

`octopus.rs` follows a pattern of loading external data, optionally enriching with DB data, then printing:

1. `load_consumption(None)` — reads JSON, returns DataFrame
2. `load_weather(None, conn.as_ref())` — reads JSON + optionally emoncms DB, returns hybrid DataFrame
3. For `gas-vs-hp`: `main.rs` loads the full HP DataFrame via `db::load_dataframe()`, enriches it with `analysis::enrich()`, then passes it to `octopus::daily_hp_by_state()` which aggregates by state
4. `print_gas_vs_hp()` receives Octopus consumption, hybrid weather, and HP state data — joins and compares

The `daily_hp_by_state()` function converts 1-minute power samples to energy using a fixed `SAMPLE_HOURS = 1.0/60.0` constant, then filters by state column (`"heating"`, `"dhw"`) using `when/then/otherwise`.

## Reference Data

`reference.rs` encodes static planning data as compile-time constants:
- House thermal properties (HTC, U-values, design conditions)
- Radiator inventory with output correction factor calculator
- Arotherm manufacturer COP curve
- Gas-era monthly consumption for before/after comparison

Some reference values are duplicated in `octopus.rs` as local constants: `GAS_DHW_KWH_PER_DAY` (11.82), `BOILER_EFFICIENCY` (0.9). These should track the values in `reference.rs`.

## SQLite Access

- Schema created via `CREATE TABLE IF NOT EXISTS` — no migration tool
- WAL journal mode set on open
- Batch inserts use `unchecked_transaction()` + `prepare_cached()`
- `INSERT OR IGNORE` for idempotent data loading
- `WITHOUT ROWID` tables for high-volume `samples` and `simulated_samples`
- `gaps.rs` accesses SQLite directly (not through `db.rs` functions)
- `octopus.rs` reads from SQLite via an optional `&Connection` parameter for emoncms temps
- Sync start date: `DEFAULT_SYNC_START_MS` constant in `db.rs` (2024-10-22)

## Gap-Filling

The `TempBinModel` builds per-°C averages from a 6-way self-join on `samples`. Applied per-minute during gap-fill, then power estimates linearly scaled so integrated energy matches cumulative meter readings (hard constraint). Short gaps (< 10 min) use linear interpolation.

## Date Range Handling

`resolve_time_range()` in `main.rs` supports four modes (in priority order):
1. `--all-data` — from hardcoded epoch (2024-10-22) to now
2. `--from` / `--to` — explicit date range (YYYY-MM-DD)
3. `--days N` — last N days from now (default 7)

Octopus commands (`octopus`, `gas-vs-hp`, `baseload`) load their own date ranges from the JSON data. The `--all-data` flag is relevant for `gas-vs-hp` and `baseload` which also load HP data from the DB using the resolved range.

## Notable Absences

- **No tests** — no `#[cfg(test)]`, no test helpers, no fixtures
- **No logging** — all diagnostic output uses `eprintln!`
- **No configuration file** — all config via CLI args, env vars, and compile-time constants
- **No async** — blocking reqwest, single-threaded SQLite
- **No JSON export** — CSV only via `export` command
- **No tariff rate storage** — Octopus tariff rates are not stored; cost analysis done via ad-hoc Python scripts
