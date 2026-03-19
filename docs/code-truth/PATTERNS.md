<!-- code-truth: 9d6d3e3 -->

# Patterns

## Configuration: TOML + Global Singleton

All domain constants live in `config.toml` and are accessed via `config::config()`:

```rust
// In any module:
use crate::config::config;

let thresholds = &config().thresholds;
let feed_id = config().emoncms.feed_id("elec_power");
```

The singleton is initialised once in `main()` via `config::load(&path)`. Attempting to access it before loading panics with a clear message. This pattern replaced hardcoded constants scattered across 6 source files.

Feed IDs are always looked up by name, never by literal string:
```rust
// Good:
config().emoncms.feed_id("outside_temp")

// Bad (was everywhere before config.toml migration):
"503093"
```

## Analysis Functions: DataFrame In, Stdout Out

Every analysis function follows the same shape:

```rust
pub fn some_analysis(df: &DataFrame) -> Result<()> {
    let result = df.clone().lazy()
        .filter(...)
        .group_by(...)
        .agg(...)
        .collect()?;
    println!("{}", result);
    Ok(())
}
```

Key conventions:
- Input is always a reference to an enriched DataFrame (already has `cop`, `delta_t`, `state` columns)
- Functions never touch the database or API — they receive all data via parameters
- Output goes directly to stdout via `println!`
- No return values for display data — functions return `Result<()>`
- Some functions take additional data (e.g. `degree_days` takes daily temp and energy vectors alongside the DataFrame)

## Polars Usage Style

- Always **lazy** evaluation: `.clone().lazy()...collect()?`
- Column names are string literals: `col("elec_w")`, `col("state")`
- Aggregations use inline expressions: `col("cop").mean().alias("avg_cop")`
- Temperature banding via floor division: `(col("outside_t") / lit(2.0)).floor().cast(DataType::Int32) * lit(2)`
- DataFrames are printed directly with `println!("{}", df)` — Polars' Display impl handles formatting

## Error Handling

- All public functions return `anyhow::Result<()>` or `anyhow::Result<T>`
- `.context("message")` on fallible operations for chain
- `anyhow::ensure!()` for preconditions (e.g. API key present, database exists)
- SQLite queries use `.unwrap_or(default)` for optional values (e.g. missing temp readings default to 0.0 or 10.0)
- The state machine never fails — it uses `.unwrap_or(0.0)` for missing values

## SQL: Parameterised vs Format Strings

Two patterns coexist:

**Parameterised** (for user/runtime values):
```rust
conn.prepare("SELECT ... WHERE feed_id = ?1 AND timestamp >= ?2")?
stmt.query_map(params![feed_id, start_ms], ...)?
```

**Format strings** (for config-derived feed IDs in multi-join queries):
```rust
conn.prepare(&format!(
    "SELECT ... FROM samples s_elec
     JOIN samples s_heat ON s_heat.feed_id = '{}' ...",
    feeds.feed_id("heat_power"),
))?
```

The format string pattern is used in `gaps.rs` where queries join 6+ feeds — building the join clauses dynamically from config. Feed IDs come from the trusted config file, not user input, so SQL injection is not a concern.

**Exception**: `fill_gap_interpolate()` in gaps.rs still uses hardcoded feed ID strings (`"503094"`, `"503096"`, etc.). This is a known inconsistency from the config migration.

## CLI Structure

Commands use clap's derive macro with subcommands:

```rust
#[derive(Subcommand)]
enum Commands {
    Summary,
    CopByTemp,
    // ...
}
```

Global flags (`--days`, `--all-data`, `--from`/`--to`, `--db`, `--include-simulated`) are on the parent `Cli` struct. Time range resolution happens once in `resolve_time_range()`, returning `(start_unix_s, end_unix_s)`.

The `require_client()` and `require_db()` helpers enforce preconditions — `require_client()` fails if no API key, `require_db()` fails if the database file doesn't exist.

## Data Loading: Real vs Simulated

Two separate loading paths keep simulated data strictly opt-in:

```rust
pub fn load_dataframe(conn, start, end) -> DataFrame        // real only
pub fn load_dataframe_with_simulated(conn, start, end) -> DataFrame  // real + gap-filled
```

When simulated data is included, an `is_simulated` boolean column is added. Simulated samples never overwrite real data — they only fill timestamps where no real sample exists.

## Sync: Chunked with Polite Throttling

Data download uses 7-day chunks with a 100ms sleep between API calls:
```rust
let chunk_ms: i64 = 7 * 86_400 * 1000;
std::thread::sleep(std::time::Duration::from_millis(100));
```

Values are `INSERT OR IGNORE` — re-syncing the same period is idempotent and only adds new timestamps.

## Module Naming

Modules match their concern directly:
- `config` — configuration
- `emoncms` — emoncms API
- `db` — database
- `analysis` — analysis
- `gaps` — gap handling
- `octopus` — Octopus Energy

No prefix/suffix conventions (`_service`, `_module`, etc.), no trait abstractions, no generic types. Each module is a flat collection of functions and structs.
