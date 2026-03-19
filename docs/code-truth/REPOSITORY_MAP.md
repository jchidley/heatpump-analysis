<!-- code-truth: 9d6d3e3 -->

# Repository Map

## Top-Level Files

| File | Concern |
|------|---------|
| `config.toml` | All domain constants, thresholds, feed IDs, house data, radiator inventory, Arotherm specs, gas-era history |
| `Cargo.toml` | Dependencies and build configuration |
| `AGENTS.md` | LLM agent context (canonical project documentation) |
| `CLAUDE.md` | Points to AGENTS.md |
| `README.md` | Human-facing quick start, command reference, project philosophy |
| `heatpump.db` | SQLite database (gitignored, created by `sync`) |

## Source Modules

### `src/main.rs` — CLI entry point

- Loads `config.toml` at startup (tries next to executable, falls back to cwd)
- Defines `Cli` struct with clap derive macros (20 subcommands)
- Routes each subcommand to the appropriate module functions
- Owns time-range resolution logic (`--days`, `--from`/`--to`, `--all-data`)
- Helper: `load_dataframe()` abstracts real vs simulated data loading

### `src/config.rs` — Configuration layer

- Deserializes `config.toml` into typed structs (`Config`, `Emoncms`, `Thresholds`, `House`, `Arotherm`, `Radiator`, `GasEra`)
- Global singleton via `once_cell::OnceCell` — loaded once by `main()`, accessed via `config::config()` from any module
- Contains computed helpers that moved from the deleted `reference.rs`:
  - `Arotherm::expected_cop_at_flow_temp()` — linear interpolation on manufacturer COP curve
  - `radiator_correction_factor()` — ΔT50 correction using `(actual_dt / 50)^1.3`
  - `total_radiator_output_at_flow_temp()` — sum across all radiators with correction
- `Emoncms::feed_id(&self, name)` — look up feed ID by name (replaces hardcoded string literals)

### `src/emoncms.rs` — API client

- `Client` struct wrapping `reqwest::blocking::Client` with an API key
- Two methods: `list_feeds()` and `feed_data(id, start, end, interval)`
- Base URL read from `config().emoncms.base_url`
- Returns `Vec<DataPoint>` where `DataPoint = (i64, Option<f64>)` (timestamp_ms, value)

### `src/db.rs` — SQLite storage and DataFrame loading

- **Schema**: three tables (`feeds`, `samples`, `sync_state`) + optional `simulated_samples` and `gap_log` (created by gaps.rs)
- **Sync**: `sync_all()` iterates config feed definitions, fetches in 7-day chunks at 60s interval, stores non-null values. Tracks last sync timestamp per feed.
- **DataFrame loading**: `load_dataframe()` / `load_dataframe_with_simulated()` builds a Polars DataFrame by:
  1. Collecting all distinct timestamps in range
  2. Creating a column per feed (using the `column` field from config feed definitions)
  3. Optionally merging simulated samples (gap-filled data never overwrites real data)
- **Daily helpers**: `load_daily_energy()` (cumulative meter deltas), `load_daily_outside_temp()` (daily avg/min/max)

### `src/analysis.rs` — State machine and Polars analysis

This is the largest module (~950 lines). Two concerns:

**1. State classification** (`classify_states()`, `enrich()`)
- Hysteresis state machine processing rows in time order
- Uses `config().thresholds` for all transition boundaries
- `enrich()` adds `cop`, `delta_t`, and `state` columns to any DataFrame

**2. Analysis functions** (each prints directly to stdout)
- `summary()` — overall stats + breakdown by state
- `cop_by_outside_temp()` — COP in 2°C outside temp bands, heating only
- `hourly_profile()` — averages by hour of day, heating only
- `daily_energy()` — daily totals from cumulative meter deltas
- `degree_days()` — HDD analysis with weekly/monthly/period summaries + gas-era comparison
- `indoor_temp()` — Leather room sensor stats, hourly profile, comfort vs outside temp
- `dhw_analysis()` — DHW energy vs gas-era design estimate
- `cop_vs_spec()` — actual COP vs Arotherm manufacturer curve at each spec flow temp
- `design_comparison()` — house properties, radiator output table, gas vs HP comparison

### `src/gaps.rs` — Gap detection and synthetic data

- `find_gaps()` — finds monitoring gaps > N minutes using windowed SQL on the elec_power feed
- `TempBinModel` — builds per-°C-bin averages for heating and DHW from real data, plus DHW fraction by hour
- `fill_gap()` — generates per-minute synthetic samples, scales power so integrated energy matches cumulative meters
- `fill_gap_interpolate()` — linear interpolation for gaps < 10 minutes
- `print_gap_report()` — lists all gaps with duration, energy, and fill status
- **Note**: `fill_gap_interpolate()` still uses hardcoded feed IDs (`"503094"`, etc.) — this is a known inconsistency

### `src/octopus.rs` — Octopus Energy integration

- Loads pre-computed JSON files from `~/github/octopus/dist/data/` (consumption.json, weather.json)
- **Temperature hierarchy**: emoncms (Met Office, Oct 2024+) preferred over ERA5-Land (bias-corrected +1.0°C)
- `load_consumption()` → half-hourly Polars DataFrame
- `load_weather()` → daily DataFrame with date, tmean, hdd, source
- `daily_totals()` — aggregates to daily elec/gas kWh
- `daily_hp_by_state()` — converts enriched DataFrame to daily heating/DHW energy split
- `print_gas_vs_hp()` — dual-era comparison with state machine split
- `print_baseload()` — whole-house minus HP electricity
- **Note**: `ERA5_BIAS_CORRECTION_C` (1.0°C) remains a Rust constant — it's derived from data overlap analysis, not a domain parameter

## Documentation

| Path | Content |
|------|---------|
| `docs/explanation.md` | How the operating model works (Diátaxis explanation) |
| `docs/roadmap.md` | Planned enhancements (eBUS, Octopus expansion, solar PV) |
| `docs/octopus-data-inventory.md` | Audit of Octopus data across repos |
| `docs/code-truth/` | This documentation set |
