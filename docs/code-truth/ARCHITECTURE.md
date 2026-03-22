<!-- code-truth: 08e43eb -->

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

### Octopus path (external CSV + JSON + SQLite)

```
~/github/octopus/data/usage_merged.csv → octopus.rs::load_consumption()
~/github/octopus/data/weather.json     → octopus.rs::load_weather()
~/github/octopus/data/config.json      → gas m³→kWh conversion factors
                                                  │
                                          (optionally joins with
                                           db.rs outside_temp for
                                           emoncms temperatures)
```

The Octopus module is semi-independent — it reads its own CSV/JSON files and only optionally touches the SQLite database for temperature data.

### z2m-hub (separate Rust server on pi5data)

All Zigbee automations, DHW tracking/boost, and mobile dashboard are handled by z2m-hub (`~/github/z2m-hub/`). It connects to Z2M via WebSocket and ebusd via TCP. Writes `dhw.remaining_litres` to InfluxDB. Not connected to the Rust analysis tool in this repo.

Previously these were shell scripts in this repo (`dhw-auto-trigger.sh`, `z2m-automations.sh`) and an InfluxDB Flux task — all replaced Mar 2026.

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
          │ fr < 15  │    │ any fr   │
          └────┬─────┘    └──────────┘
               │ fr ≥ 15         ▲
               ▼                 │
          ┌──────────┐           │
          │   DHW    │───────────┘
          │ fr ≥ 14.7│  h ≤ 0 or dt < -0.5
          └──────────┘
```

The hysteresis zone (14.7–15.0 L/min) prevents rapid switching during the ~3-second diverter valve transition. The machine remembers the pre-defrost state to return correctly after defrost events that can happen at any flow rate.

**Threshold history**: Originally 16.0/15.0 L/min entry/exit. Tightened to 15.0/14.7 in March 2026 because DHW flow dropped from 21.0 to 16.8 L/min due to y-filter sludge buildup. After filter cleaning (19 March 2026), DHW flow recovered to 21.3 L/min, but the tighter thresholds are retained as they're still safe (heating clamped at 14.3 L/min).

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

## Implicit Contracts

| Contract | Where | Risk |
|----------|-------|------|
| DataFrame column names must match `config.toml` feed `column` fields | analysis.rs, octopus.rs | Wrong column name → silent null results |
| `fill_gap_interpolate()` uses hardcoded feed IDs, not config lookup | gaps.rs line ~580 | Config feed ID change breaks interpolation |
| `resolve_time_range()` hardcodes `1_729_555_200` for `--all-data` | main.rs | Duplicates `config.toml`'s `default_sync_start_ms` — can drift |
| `ERA5_BIAS_CORRECTION_C` is a Rust constant in octopus.rs | octopus.rs | Not in config.toml — two sources for temperature correction |
| Octopus data path hardcoded to `~/github/octopus/data/` | octopus.rs `default_data_dir()` | Moving octopus project breaks analysis |
| `daily_hp_by_state()` assumes 1-minute sample interval | octopus.rs `SAMPLE_HOURS = 1/60` | Different sample interval → wrong energy |
| DHW tracking (161L capacity, boost logic) lives in z2m-hub, not this repo | `~/github/z2m-hub/` | Changing usable volume requires updating z2m-hub `DHW_FULL_LITRES` constant |
| gaps.rs DHW classification uses `dhw_enter_flow_rate` from config | gaps.rs TempBinModel | Threshold changes must be consistent between analysis.rs and gaps.rs |

## External Boundaries

| System | Connection | Prerequisite |
|--------|-----------|--------------|
| emoncms.org | REST API (read key) | API key via `--apikey` or `EMONCMS_APIKEY` |
| `~/github/octopus/` | File read (CSV + JSON) | Must exist with `data/usage_merged.csv`, `weather.json`, `config.json` |
| pi5data (10.0.1.230) | SSH/systemd for ebusd-poll.sh; Docker stack (InfluxDB, Grafana, ebusd, etc.) | Docker + systemd running |
| emonpi (10.0.1.117) | Z2M Docker + Mosquitto (MQTT bridge to pi5data) | Running |
| emondhw (10.0.1.46) | Multical data source (bridged via MQTT to pi5data) | Raspberry Pi on network, emonhub + Mosquitto running |
| emonhp (10.0.1.169) | Data source (MBUS + SDM120 → emoncms.org) | Must be running for data sync |

## Change Propagation

| If you change... | You must also... |
|-----------------|------------------|
| A feed ID in `config.toml` | Nothing — all modules resolve by name. But existing SQLite data uses old IDs. |
| A threshold in `config.toml` | Re-run analysis. Existing simulated samples used old thresholds — consider `DELETE FROM simulated_samples` and re-running `fill-gaps`. |
| DHW flow thresholds | Check that `gaps.rs` TempBinModel still classifies correctly (it uses `dhw_enter_flow_rate` from config). Update `docs/explanation.md` thresholds table. |
| A new feed in `config.toml` | Add `column` if it should appear in analysis DataFrames. Run `sync` to fetch data. |
| DataFrame column names | Update `config.toml` feed `column` fields. Check analysis.rs and octopus.rs column references. |
| Add a new analysis function | Add to `analysis.rs`, add `Commands` variant in `main.rs`, wire up in `match`. |
| Arotherm model size | All thresholds are model-specific (especially flow rates). The 7kW heating rate overlaps the 5kW DHW rate. |
| DHW boost/tracking or Z2M automations | Edit z2m-hub (`~/github/z2m-hub/`), cross-compile, deploy to pi5data |
| DHW usable volume (161L) | Update `DHW_FULL_LITRES` in z2m-hub, update `docs/dhw-cylinder-analysis.md` and AGENTS.md |
| Monitoring infrastructure | Update `heating-monitoring-setup.md`. |
