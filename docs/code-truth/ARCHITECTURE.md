<!-- code-truth: 3af9fd0 -->

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

model/house.py       (completely independent — no Rust connection)
  └── InfluxDB on pi5data (reads room temps, outside temp, HP state, eBUS status)
```

Key constraint: **analysis.rs has no dependency on db.rs or emoncms.rs**. It operates purely on Polars DataFrames passed in by main.rs. This separation means analysis functions can be called with any DataFrame that has the right columns.

Key constraint: **model/house.py has no connection to the Rust tool**. They share the same InfluxDB and emoncms data sources but don't interact. The Rust tool analyses HP performance; the Python model analyses room-level heat distribution.

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

### Room thermal model path (completely separate)

```
InfluxDB (pi5data)
  ├── room temps (zigbee2mqtt/*_temp_humid, emon/emonth2_23/temperature)
  ├── room humidity (zigbee2mqtt/*_temp_humid)
  ├── outside temp (ebusd/poll/OutsideTemp)
  ├── HP state (emon/heatpump/heatmeter_*, ebusd/poll/StatuscodeNum)
  │
  └──→ model/house.py fetch → model/data/*.csv
                                      │
                    ┌─────────────────┼──────────────────┐
                    ▼                 ▼                   ▼
              analyse/fit       equilibrium          moisture
           (energy balance,   (scipy fsolve for    (AH tracking,
            cooldown rates)    steady-state temps)  ACH validation)
```

### z2m-hub (separate Rust server on pi5data)

All Zigbee automations, DHW tracking/boost, and mobile dashboard are handled by z2m-hub (`~/github/z2m-hub/`). It connects to Z2M via WebSocket and ebusd via TCP. Writes `dhw.remaining_litres` to InfluxDB. Not connected to the Rust analysis tool in this repo.

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

**Note**: The Python thermal model (`model/house.py`) has its own room/radiator definitions — not connected to `config.toml`. Radiator T50 values appear in both places and must be kept in sync manually.

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

## Room Thermal Model Architecture

`model/house.py` implements a lumped-parameter thermal network with clear separation of concerns:

**Layer 1: Data types** (dataclasses) — pure physical properties
- `RoomDef`, `RadiatorDef`, `ExternalElement`, `InternalConnection`, `Doorway`, `SolarGlazing`

**Layer 2: Physics** (pure functions) — standard building physics equations
- `radiator_output()`, `external_loss()`, `ventilation_loss()`, `wall_conduction()`, `doorway_exchange()`, `solar_gain()`, `estimate_thermal_mass()`

**Layer 3: Integration** — combines physics into a complete room balance
- `room_energy_balance()` — returns dict of all heat flow components for one room

**Layer 4: House definition** — physical constants
- `build_rooms()` → 13 `RoomDef` instances
- `build_connections()` → ~25 `InternalConnection` instances
- `build_doorways()` → ~9 `Doorway` instances

**Layer 5: Analysis commands** — data loading + physics + display
- `analyse()`, `fit()`, `cmd_equilibrium()`, `moisture_analysis()`, `cmd_rooms()`, `cmd_connections()`

**Key design principle**: All inter-room connections are **symmetric** — defined once and applied to both rooms. This avoids double-counting and makes the connection list authoritative.

**Calibration approach**: Night 1 (24-25 Mar, doors normal, T_out 8.5°C) vs Night 2 (25-26 Mar, all doors closed, T_out 5.0°C). Joint optimisation of two parameters (Cd and landing ACH) against measured cooling rates for all 13 rooms.

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
| DHW tracking (161L capacity, boost logic) lives in z2m-hub | `~/github/z2m-hub/` | Changing usable volume requires updating z2m-hub `DHW_FULL_LITRES` |
| gaps.rs DHW classification uses `dhw_enter_flow_rate` from config | gaps.rs TempBinModel | Threshold changes must be consistent between analysis.rs and gaps.rs |
| Radiator T50 values duplicated between `config.toml` and `model/house.py` | Both files | Out-of-sync values → inconsistent radiator output calculations |
| `model/house.py` InfluxDB token hardcoded | `INFLUX_TOKEN` constant | Token rotation requires code change |
| `model/house.py` ACH values are per-room constants, not connected to any config | `build_rooms()` | Each room's ACH was individually calibrated — changing one room without understanding the joint calibration can break the whole model |

## External Boundaries

| System | Connection | Prerequisite |
|--------|-----------|--------------|
| emoncms.org | REST API (read key) | API key via `--apikey` or `EMONCMS_APIKEY` |
| `~/github/octopus/` | File read (CSV + JSON) | Must exist with `data/usage_merged.csv`, `weather.json`, `config.json` |
| pi5data (10.0.1.230) | SSH/systemd for ebusd-poll.sh; Docker stack (InfluxDB, Grafana, ebusd, etc.) | Docker + systemd running |
| InfluxDB on pi5data | HTTP API (port 8086) for Python thermal model | pi5data Docker running |
| Open-Meteo API | HTTP for outside humidity in moisture analysis | Internet access (falls back to 75% RH) |
| emonpi (10.0.1.117) | Z2M Docker + Mosquitto (MQTT bridge to pi5data) | Running |
| emondhw (10.0.1.46) | Multical data source (bridged via MQTT to pi5data) | Raspberry Pi on network |
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
| Radiator T50 in config.toml | Also update `model/house.py` `build_rooms()` for the same radiator |
| Room ventilation ACH in model | Verify against Night 1/Night 2 calibration data. Joint calibration means changing one room's ACH may require adjusting others. |
| Doorway Cd or landing ACH | These are jointly calibrated — changing one affects the other |
| Monitoring infrastructure | Update `heating-monitoring-setup.md` |
