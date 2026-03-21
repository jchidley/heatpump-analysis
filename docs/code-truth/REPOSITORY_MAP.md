> **STALE**: References to dhw-auto-trigger.py on emondhw and ebusd-poll.py in Docker are outdated. Both replaced by shell scripts on pi5data (March 2026). See AGENTS.md for current architecture.

<!-- code-truth: 4cc0d3d -->

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
| `heating-monitoring-setup.md` | Full monitoring infrastructure documentation (devices, MQTT topics, eBUS data, credentials) |
| `.gitmodules` | ebusd submodule reference |

## Source Modules

### `src/main.rs` — CLI entry point (498 lines)

- Loads `config.toml` at startup (tries next to executable, falls back to cwd)
- Defines `Cli` struct with clap derive macros (20 subcommands)
- Routes each subcommand to the appropriate module functions
- Owns time-range resolution logic (`--days`, `--from`/`--to`, `--all-data`)
- Helper: `load_dataframe()` abstracts real vs simulated data loading
- `resolve_time_range()` — hardcodes `1_729_555_200` for `--all-data`, duplicating `config.toml`'s `default_sync_start_ms`

### `src/config.rs` — Configuration layer (211 lines)

- Deserializes `config.toml` into typed structs (`Config`, `Emoncms`, `Thresholds`, `House`, `Arotherm`, `Radiator`, `GasEra`)
- Global singleton via `once_cell::OnceCell` — loaded once by `main()`, accessed via `config::config()` from any module
- Contains computed helpers:
  - `Arotherm::expected_cop_at_flow_temp()` — linear interpolation on manufacturer COP curve
  - `radiator_correction_factor()` — ΔT50 correction using `(actual_dt / 50)^1.3`
  - `total_radiator_output_at_flow_temp()` — sum across all radiators with correction
- `Emoncms::feed_id(&self, name)` — look up feed ID by name (replaces hardcoded string literals)

### `src/emoncms.rs` — API client (83 lines)

- `Client` struct wrapping `reqwest::blocking::Client` with an API key
- Two methods: `list_feeds()` and `feed_data(id, start, end, interval)`
- Base URL read from `config().emoncms.base_url`
- Returns `Vec<DataPoint>` where `DataPoint = (i64, Option<f64>)` (timestamp_ms, value)

### `src/db.rs` — SQLite storage and DataFrame loading (503 lines)

- **Schema**: three tables (`feeds`, `samples`, `sync_state`) + optional `simulated_samples` and `gap_log` (created by gaps.rs)
- **Sync**: `sync_all()` iterates config feed definitions, fetches in 7-day chunks at 60s interval, stores non-null values. Tracks last sync timestamp per feed.
- **DataFrame loading**: `load_dataframe()` / `load_dataframe_with_simulated()` builds a Polars DataFrame by:
  1. Collecting all distinct timestamps in range
  2. Creating a column per feed (using the `column` field from config feed definitions)
  3. Optionally merging simulated samples (gap-filled data never overwrites real data)
- **Daily helpers**: `load_daily_energy()` (cumulative meter deltas), `load_daily_outside_temp()` (daily avg/min/max)

### `src/analysis.rs` — State machine and Polars analysis (950 lines)

This is the largest module. Two concerns:

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

### `src/gaps.rs` — Gap detection and synthetic data (638 lines)

- `find_gaps()` — finds monitoring gaps > N minutes using windowed SQL on the elec_power feed
- `TempBinModel` — builds per-°C-bin averages for heating and DHW from real data, plus DHW fraction by hour
- `fill_gap()` — generates per-minute synthetic samples, scales power so integrated energy matches cumulative meters
- `fill_gap_interpolate()` — linear interpolation for gaps < 10 minutes
- `print_gap_report()` — lists all gaps with duration, energy, and fill status
- **Known issue**: `fill_gap_interpolate()` still uses hardcoded feed IDs (`"503094"`, `"503096"`, `"503098"`, `"503099"`, `"503100"`) — not migrated to config lookup

### `src/octopus.rs` — Octopus Energy integration (708 lines)

- Loads data from `~/github/octopus/data/` (usage_merged.csv for consumption, weather.json for temps, config.json for gas unit conversion)
- **Temperature hierarchy**: emoncms (Met Office, Oct 2024+) preferred over ERA5-Land (bias-corrected +1.0°C)
- `load_consumption()` → half-hourly Polars DataFrame with gas m³→kWh conversion
- `load_weather()` → daily DataFrame with date, tmean, hdd, source (hybrid emoncms + ERA5)
- `daily_totals()` — aggregates to daily elec/gas kWh
- `daily_hp_by_state()` — converts enriched DataFrame to daily heating/DHW energy split
- `print_gas_vs_hp()` — dual-era comparison with state machine split
- `print_baseload()` — whole-house minus HP electricity
- **Note**: `ERA5_BIAS_CORRECTION_C` (1.0°C) is a Rust constant, not in config.toml

## Scripts

| File | Deployed to | Purpose |
|------|------------|---------|
| `scripts/dhw-auto-trigger.py` | emondhw `/usr/local/bin/` | Watches Multical DHW flow via MQTT, triggers eBUS DHW charge on prolonged draws. Blocks during Cosy peak (16–19). |
| `scripts/dhw-auto-trigger.service` | emondhw `/etc/systemd/system/` | Systemd unit for auto-trigger |

## Documentation

| Path | Content | Type (Diátaxis) |
|------|---------|-----------------|
| `README.md` | Quick start, command reference, project philosophy | Signpost |
| `docs/explanation.md` | How the operating model works (state machine, flow rates, gap filling) | Explanation |
| `docs/hydraulic-analysis.md` | Pump curves, flow degradation, y-filter diagnosis, post-clean results | Explanation |
| `docs/dhw-auto-trigger.md` | Emergency DHW recharge automation: trigger logic, eBUS commands, deployment | How-to + Explanation |
| `docs/dhw-cylinder-analysis.md` | Cylinder heat exchange: reheat cycles, standby loss, WWHR, stratification | Explanation |
| `docs/octopus-data-inventory.md` | Audit of Octopus data sources, coverage, integration status | Reference |
| `docs/roadmap.md` | Planned enhancements (eBUS ✅, Octopus ✅, solar PV, degree days ✅) | Reference |
| `docs/code-truth/` | This documentation set (derived from code) | — |
| `heating-monitoring-setup.md` | Full monitoring infrastructure: devices, MQTT, eBUS, InfluxDB, Grafana, credentials | Reference |

## External Dependencies

| Path | What it provides |
|------|-----------------|
| `~/github/octopus/data/usage_merged.csv` | Half-hourly Octopus consumption (electricity + gas, Apr 2020 → present) |
| `~/github/octopus/data/weather.json` | ERA5-Land daily temps + HDD |
| `~/github/octopus/data/config.json` | Gas m³→kWh conversion factors (calorific value, correction factor) |
| `ebusd/` (submodule) | ebusd source (reference only, not built from this project) |

## Change Guide

| To change... | Look in... |
|--------------|-----------|
| Operating thresholds (flow rates, defrost DT, HDD base) | `config.toml` `[thresholds]` |
| Feed IDs or add new feeds | `config.toml` `[[emoncms.feeds]]` |
| House thermal properties | `config.toml` `[house]` |
| Radiator inventory | `config.toml` `[[radiators]]` |
| Gas-era reference data | `config.toml` `[gas_era]` |
| State machine logic | `src/analysis.rs` `classify_states()` |
| New analysis subcommand | `src/analysis.rs` (function) + `src/main.rs` (Commands enum + match) |
| Gap-fill model or strategy | `src/gaps.rs` |
| Octopus data loading or comparison | `src/octopus.rs` |
| DHW auto-trigger behaviour | `scripts/dhw-auto-trigger.py` (constants at top) |
| Monitoring infrastructure | `heating-monitoring-setup.md` |
