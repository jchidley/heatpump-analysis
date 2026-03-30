<!-- code-truth: dfdffb4 -->

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
  ├── octopus.rs     (reads config for thresholds, gas_era)
  │     └── config.rs
  ├── overnight.rs   (reads config for thresholds; uses analysis::enrich)
  │     └── analysis.rs, config.rs
  └── thermal.rs     (thin facade — re-exports from 14 submodules)
        └── thermal/
              ├── config.rs      (ThermalConfig from model/thermal-config.toml)
              ├── geometry.rs    (rooms/connections/doorways from thermal_geometry.json)
              ├── physics.rs     (constants, thermal mass, energy balance)
              ├── solar.rs       (solar position + irradiance)
              ├── wind.rs        (Open-Meteo wind + ventilation multiplier)
              ├── calibration.rs (grid search + shared helpers)
              ├── validation.rs  (metrics, residuals, holdout validation)
              ├── diagnostics.rs (cooldown detection + fit diagnostics)
              ├── operational.rs (BCF-based state, operational validation)
              ├── artifact.rs    (JSON artifact build/write)
              ├── snapshot.rs    (export/import with human signoff)
              ├── error.rs       (ThermalError enum)
              ├── influx.rs      (InfluxDB Flux queries)
              └── report.rs      (table formatting)

model/house.py       (separate executable — shares data/canonical/thermal_geometry.json with Rust)
  └── InfluxDB on pi5data (reads room temps, outside temp, HP state, eBUS data)
```

Key constraints:
- **analysis.rs has no dependency on db.rs or emoncms.rs** — operates purely on Polars DataFrames
- **thermal.rs has no dependency on config.rs** — uses its own `ThermalConfig` from `model/thermal-config.toml`
- **model/house.py and the Rust tool are separate executables** — share InfluxDB data and `thermal_geometry.json` but don't interact at runtime
- **gaps.rs bypasses db.rs** — writes directly to `simulated_samples` and `gap_log` tables via own SQL

## Data Flow

### Sync path (online)

```
emoncms.org API → emoncms.rs::Client → db.rs::sync_all() → SQLite (samples table)
```

`sync_all()` iterates `config().emoncms.feeds`, fetches 7-day chunks at 60s resolution. Each feed tracks its last-synced timestamp independently.

### Analysis path (offline)

```
SQLite → db.rs::load_dataframe() → Polars DataFrame
                                          │
                              analysis.rs::enrich()
                                          │
                              ┌────────────┼────────────┐
                              ▼            ▼            ▼
                          summary()    degree_days()  cop_vs_spec()
```

`enrich()` adds `cop`, `delta_t`, `state` columns. All downstream analysis functions require enriched DataFrames.

### Gap-fill path (writes back to SQLite)

```
db.rs::find_gaps() → gaps.rs::TempBinModel::from_db() → gaps.rs::fill_gap()
                                                                │
                                                     simulated_samples table
```

Gap filling bypasses `db.rs` — manages its own schema (`simulated_samples`, `gap_log`).

### Octopus path (external CSV + JSON + SQLite)

```
~/github/octopus/data/usage_merged.csv → octopus.rs::load_consumption()
~/github/octopus/data/weather.json     → octopus.rs::load_weather()
~/github/octopus/data/config.json      → gas m³→kWh conversion factors
```

Semi-independent — only optionally touches SQLite for temperature data.

### Thermal model path (InfluxDB)

```
InfluxDB (pi5data:8086, bucket "energy")
  ├── room temps (zigbee2mqtt/*_temp_humid, emon/emonth2_23/temperature)
  ├── outside temp (ebusd/poll/OutsideTemp)
  ├── HP state (ebusd/poll/BuildingCircuitFlow, StatuscodeNum)
  ├── MWT (ebusd/poll/FlowTemp, ReturnTemp)
  ├── PV power (emon/EmonPi2/P3)
  │
  └──→ thermal/influx.rs → thermal.rs (calibrate / validate / operational)
                                 │
                          artifacts/thermal/*.json (structured output)
```

### Thermal regression path

```
thermal commands → artifacts/thermal/*.json
                         │
thermal-regression-check binary → compare against baselines/*.json
                                        │
                               regression-thresholds.toml (pass/fail gates)
```

### Python thermal model path (completely separate)

```
InfluxDB → model/house.py fetch → model/data/*.csv
                                        │
                   ┌────────────────────┼──────────────────┐
                   ▼                    ▼                   ▼
            analyse/fit          equilibrium            moisture
```

## Configuration Architecture

### config.toml (main CLI)

Loaded once at startup by `main.rs`, stored in `once_cell::OnceCell`. All modules access via `config::config()`.

| Section | Consumers |
|---------|-----------|
| `emoncms` (feeds, base_url, sync start) | db.rs, emoncms.rs, gaps.rs |
| `thresholds` (elec_running_w, dhw flow rates, defrost DT, HDD base) | analysis.rs, gaps.rs, octopus.rs |
| `house` (HTC, floor area, design temps, construction) | analysis.rs |
| `arotherm` (manufacturer spec curve) | analysis.rs |
| `radiators` (15 radiators with room, dimensions, T50 watts) | analysis.rs |
| `gas_era` (boiler efficiency, annual totals, monthly data) | analysis.rs, octopus.rs |

### model/thermal-config.toml (thermal commands)

Loaded by `thermal.rs` independently. Not connected to `config.rs` singleton.

| Section | Purpose |
|---------|---------|
| `influx` | InfluxDB connection (url, org, bucket, token_env) |
| `test_nights` | Night 1/Night 2 start/end timestamps |
| `objective` | Excluded rooms, prior weight |
| `priors` | Landing ACH, doorway Cd priors |
| `bounds` | Grid search ranges and steps |
| `wind` | Optional wind multiplier (disabled by default) |
| `validation` | Holdout windows and pass/fail thresholds |
| `fit_diagnostics` | Period detection params, HP off codes |

### data/canonical/thermal_geometry.json

Single source of truth for room geometry. Consumed by both Rust (`thermal.rs::build_rooms/connections/doorways`) and Python (`house.py`). Generated by `model/extract_house_inventory.py`, audited by `model/audit_model_dimensions.py`.

## State Machine Design

The operating state classifier in `analysis.rs::classify_states()` is a **deterministic hysteresis state machine** processing rows in time order.

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

Hysteresis zone (14.7–15.0 L/min) prevents rapid switching during diverter valve transition.

**Separate eBUS state classification** exists in `thermal.rs::classify_hp_state_from_flow()` using `BuildingCircuitFlow` (L/h): >900=DHW, 780–900=heating, <100=off. These two classifiers serve different data sources (emoncms vs InfluxDB) and are not connected.

## Implicit Contracts

| Contract | Where | Risk |
|----------|-------|------|
| DataFrame column names must match `config.toml` feed `column` fields | analysis.rs, octopus.rs | Wrong column name → silent null results |
| `fill_gap_interpolate()` uses hardcoded feed IDs, not config lookup | gaps.rs | Config feed ID change breaks interpolation |
| `resolve_time_range()` hardcodes `1_729_555_200` for `--all-data` | main.rs | Duplicates `config.toml`'s `default_sync_start_ms` |
| `ERA5_BIAS_CORRECTION_C` is a Rust constant in octopus.rs | octopus.rs | Not in config.toml — two sources for temperature correction |
| Octopus data path hardcoded to `~/github/octopus/data/` | octopus.rs | Moving octopus project breaks analysis |
| `daily_hp_by_state()` assumes 1-minute sample interval | octopus.rs | `SAMPLE_HOURS = 1/60` — different interval → wrong energy |
| DHW tracking (161L capacity, boost logic) lives in z2m-hub | `~/github/z2m-hub/` | Changing usable volume requires updating `DHW_FULL_LITRES` |
| gaps.rs DHW classification uses `dhw_enter_flow_rate` from config | gaps.rs | Must stay consistent with analysis.rs thresholds |
| Radiator T50 values duplicated in `config.toml` and `thermal_geometry.json` | `config.toml` (analysis.rs), `thermal_geometry.json` (thermal.rs + house.py) | Out-of-sync → inconsistent radiator output between analysis and thermal commands |
| Room geometry shared via `data/canonical/thermal_geometry.json` | `model/house.py`, `src/thermal.rs` | Both consume same file; `model/audit_model_dimensions.py` verifies wiring |
| `model/house.py` InfluxDB token from env var or `ak get influxdb` | `INFLUX_TOKEN` env var | Token rotation requires `ak set influxdb` |
| Thermal regression baselines must be refreshed after intentional model changes | `artifacts/thermal/baselines/` | Stale baselines → false regression failures |
| `thermal-operational` assumes specific InfluxDB measurement names | `thermal/influx.rs` | Telegraf config changes on pi5data break queries |
| `thermal_geometry.json` room names must match InfluxDB sensor names (with `_temp_humid` suffix stripped) | `thermal.rs`, `thermal/influx.rs` | Sensor rename breaks room→data mapping |

## External Boundaries

| System | Connection | Prerequisite |
|--------|-----------|--------------|
| emoncms.org | REST API (read key) | API key via `--apikey` or `EMONCMS_APIKEY` |
| `~/github/octopus/` | File read (CSV + JSON) | Must exist with `data/usage_merged.csv`, `weather.json`, `config.json` |
| InfluxDB on pi5data (10.0.1.230:8086) | HTTP API for Rust thermal + Python model | Docker running, `INFLUX_TOKEN` env var set |
| Open-Meteo API | HTTP for solar irradiance (DNI/DHI) + outside humidity | Internet access (solar falls back gracefully; humidity assumes 75% RH) |
| pi5data (10.0.1.230) | SSH/systemd for ebusd-poll.sh; Docker stack | Docker + systemd running |
| emonpi (10.0.1.117) | Z2M Docker + Mosquitto (MQTT bridge to pi5data) | Running |
| emondhw (10.0.1.46) | Multical data source (bridged via MQTT to pi5data) | Raspberry Pi on network |
| emonhp (10.0.1.169) | Data source (MBUS + SDM120 → emoncms.org) | Must be running for data sync |

## Change Propagation

| If you change... | You must also... |
|-----------------|------------------|
| A feed ID in `config.toml` | Nothing for code — but existing SQLite data uses old IDs |
| A threshold in `config.toml` | Re-run analysis. Consider `DELETE FROM simulated_samples` and re-running `fill-gaps` |
| DHW flow thresholds | Check gaps.rs TempBinModel. Update `docs/explanation.md` |
| Room geometry | Edit `data/canonical/thermal_geometry.json`. Run `model/audit_model_dimensions.py` to verify. Re-run thermal calibration |
| Radiator T50 in config.toml | Also update `data/canonical/thermal_geometry.json` (thermal.rs + house.py read from there) |
| Room ventilation ACH in model | Joint calibration — changing one room may require adjusting others |
| Doorway Cd or landing ACH | Jointly calibrated — verify against Night 1/Night 2 data |
| Thermal model physics | Run `scripts/thermal-regression-ci.sh`. Update baselines if intentional |
| Monitoring infrastructure | Update `heating-monitoring-setup.md` |
| DHW boost/tracking or Z2M automations | Edit z2m-hub (`~/github/z2m-hub/`), cross-compile, deploy to pi5data |
| DHW usable volume (161L) | Update `DHW_FULL_LITRES` in z2m-hub, update docs |

## SQLite Schema

Core tables (created by `db.rs::open()`):
```sql
feeds (id TEXT PK, name, tag, unit)
samples (feed_id TEXT, timestamp INTEGER, value REAL) WITHOUT ROWID  -- PK: (feed_id, timestamp)
sync_state (feed_id TEXT PK, last_timestamp INTEGER)
```

Gap-fill tables (created by `gaps.rs::ensure_schema()`):
```sql
simulated_samples (feed_id TEXT, timestamp INTEGER, value REAL, gap_start_ts INTEGER) WITHOUT ROWID
gap_log (start_ts INTEGER PK, end_ts, duration_min, elec_kwh, heat_kwh, method, samples_generated)
```

WAL mode enabled. Schema uses `CREATE TABLE IF NOT EXISTS` — no migration system.
