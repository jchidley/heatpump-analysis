<!-- code-truth: dfdffb4 -->

# Patterns

## Configuration: TOML + Global Singleton (main CLI)

All domain constants live in `config.toml` and are accessed via `config::config()`:

```rust
use crate::config::config;
let thresholds = &config().thresholds;
let feed_id = config().emoncms.feed_id("elec_power");
```

Singleton initialised once in `main()` via `config::load(&path)`. Feed IDs always looked up by name, never by literal string.

**Cost to break**: Every module depends on `config()`. Replacing with dependency injection would touch every function.

**Exception**: `fill_gap_interpolate()` in gaps.rs still uses hardcoded feed ID strings. `ERA5_BIAS_CORRECTION_C` in octopus.rs is a Rust constant. These are known inconsistencies.

## Configuration: Separate TOML (thermal module)

`thermal.rs` loads its own `ThermalConfig` from `model/thermal-config.toml`. Not connected to the `config.rs` singleton. This keeps the thermal module independent of the emoncms analysis pipeline.

**Cost to break**: Merging into `config.toml` would couple thermal commands to the emoncms config structure.

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

- Input: reference to enriched DataFrame (already has `cop`, `delta_t`, `state` columns)
- Functions never touch database or API
- Output goes directly to stdout via `println!`
- Return `Result<()>` — no structured return values

**Cost to break**: Moving to structured output (JSON, CSV) would require changing every analysis function.

## Thermal Functions: Typed Errors + JSON Artifacts

Thermal commands use `ThermalResult<T>` with `ThermalError` enum (20+ variants via `thiserror`). No `anyhow` inside thermal module — typed errors from domain through infrastructure.

Thermal commands produce JSON artifacts to `artifacts/thermal/`:
```rust
fn write_artifact(prefix: &str, artifact: &CalibrationArtifact) -> ThermalResult<PathBuf>
```

Artifacts contain: git SHA, config hash, window definitions, fitted params, per-room residuals. Used for regression checking.

**Cost to break**: Changing artifact schema requires updating `thermal-regression-check` binary and baseline JSONs.

## Polars Usage Style

- Always **lazy** evaluation: `.clone().lazy()...collect()?`
- Column names as string literals: `col("elec_w")`, `col("state")`
- Temperature banding via floor division: `(col("outside_t") / lit(2.0)).floor()`
- DataFrames printed with `println!("{}", df)` — Polars Display handles formatting

**Cost to break**: Polars pinned to 0.46. Upgrading would require auditing all lazy queries — frequent breaking API changes between minor versions.

## Error Handling: Split by Module

| Module | Style | Pattern |
|--------|-------|---------|
| Main CLI boundary | `anyhow::Result<()>` | `.context("message")` chains |
| analysis.rs, db.rs, gaps.rs, octopus.rs | `anyhow::Result<T>` | `anyhow::ensure!()` for preconditions |
| thermal.rs + submodules | `ThermalResult<T>` = `Result<T, ThermalError>` | Typed errors via `thiserror` |

State machine (`classify_states`) never fails — uses `.unwrap_or(0.0)` for missing values.

**Cost to break**: Full typed-error migration for non-thermal modules would require defining error types and updating every `?` chain.

## Data Loading: Real vs Simulated

Two separate loading paths keep simulated data strictly opt-in:

```rust
pub fn load_dataframe(conn, start, end) -> DataFrame        // real only
pub fn load_dataframe_with_simulated(conn, start, end) -> DataFrame  // real + gap-filled
```

Simulated samples never overwrite real data. `is_simulated` boolean column added when included.

**Cost to break**: Mixing silently would contaminate COP and energy analysis. Core integrity constraint.

## Thermal Model: Physics in Code, Tunables in TOML/JSON

Physical equations (`radiator_output`, `external_loss`, `ventilation_loss`, `wall_conduction`, `doorway_exchange`, `solar_gain_full`) are implemented as pure functions in Rust source code.

Room definitions, geometry, and connections are loaded from `data/canonical/thermal_geometry.json` at runtime. Calibration bounds and windows are in `model/thermal-config.toml`.

**Cost to break**: Moving physics equations to TOML would lose type safety. Moving geometry back into code would break the Python↔Rust sharing path and the audit pipeline.

## Thermal Model: Symmetric Connections

All wall/floor/ceiling conduction and doorway exchange defined **once** per pair (as `InternalConnection` or `Doorway`), applied to both rooms in `room_energy_balance()`.

**Cost to break**: Defining in both rooms would double-count transfers. Adding a room requires defining all its connections in the geometry file.

## Python Model Patterns

### Dataclass-based domain model
Room definitions use `@dataclass`. No inheritance. Physical properties only.

### Pure physics functions
Core calculations are pure functions taking physical parameters, returning watts. No global state.

### Shared geometry via JSON
Room geometry loaded from `data/canonical/thermal_geometry.json` — same file consumed by Rust.

### Constants at module level
Physical constants (`U_INTERNAL_WALL`, `DOORWAY_CD`, `AIR_DENSITY`) are Python constants. Not externalised to config.

### CSV-based data pipeline
Data fetched from InfluxDB, written to CSV, loaded from CSV. Provides explicit cache and offline capability.

## Shell Script Pattern (monitoring on pi5data)

`ebusd-poll.sh`:
- All tunables as shell variables at top
- `mosquitto_pub` for MQTT, `nc | head -1` for eBUS TCP
- `log()` helper with timestamp prefix
- `cleanup()` trap for SIGTERM/SIGINT
- systemd service (`Restart=always`)
- Deploy: `scp` to pi5data + `sudo systemctl restart`

**Cost to break**: Runs independently on pi5data. No CI/CD.

## Regression Testing Pattern

```
model change → run thermal commands → compare artifacts against baselines
                                              │
                                    thermal-regression-check binary
                                              │
                                    regression-thresholds.toml (per-room gates)
```

- `scripts/thermal-regression-ci.sh` orchestrates the full check
- `scripts/refresh-thermal-baselines.sh` updates baselines after intentional changes
- Missing baselines = hard failure (no skip path)
- Never relax thresholds and change model logic in the same commit

## Notable Absences

| What's missing | Why it matters |
|---------------|---------------|
| Tests | No unit, integration, or property tests. Validation by running against real data + regression baselines. |
| CI/CD | No automated pipeline. GitHub workflow file exists for thermal regression but requires InfluxDB access. |
| Logging framework | `println!`/`eprintln!` in Rust. `print()` in Python. |
| Structured output | Terminal-formatted text (except `export` command and thermal JSON artifacts). |
| Migration system | SQLite `CREATE TABLE IF NOT EXISTS`. No versioning. |
| Async | All I/O blocking (intentional — no benefit for sequential CLI). |
