<!-- code-truth: 7b6bfed -->

# Architecture

## Module Dependency Graph

```
main.rs (CLI)
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
  └── thermal.rs     (thin facade — re-exports from 15 submodules)
        └── thermal/  (own config from model/thermal-config.toml)

adaptive-heating-mvp.rs (separate binary, own dependency tree)
  ├── model/adaptive-heating-mvp.toml (own TOML config)
  ├── ebusd TCP (localhost:8888)
  ├── InfluxDB HTTP (localhost:8086)
  ├── Axum HTTP API (:3031)
  └── JSONL + InfluxDB logging
```

Key constraints:
- **analysis.rs has no dependency on db.rs or emoncms.rs** — operates purely on Polars DataFrames
- **thermal.rs has no dependency on config.rs** — uses its own `ThermalConfig`
- **adaptive-heating-mvp is fully independent** — shares no code with the analysis CLI or thermal module. Uses its own config, own InfluxDB queries, own eBUS access pattern.
- **gaps.rs bypasses db.rs** — writes directly to `simulated_samples` and `gap_log` tables

## Data Flow

### Sync path (online)

```
emoncms.org API → emoncms.rs::Client → db.rs::sync_all() → SQLite (samples table)
```

### Analysis path (offline)

```
SQLite → db.rs::load_dataframe() → analysis.rs::enrich() → analysis functions → stdout
```

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
                                 └──→ artifacts/thermal/*.json
```

### Adaptive heating control path (live on pi5data)

```
eBUS (ebusd TCP :8888)
  ├── reads: RunDataStatuscode, DisplayedOutsideTemp, Hc1ActualFlowTempDesired,
  │          Hc1HeatCurve, Z1DayTemp, Z1NightTemp, HwcTempDesired,
  │          HwcStorageTemp, CurrentCompressorUtil, RunDataElectricPowerConsumption,
  │          CurrentYieldPower, RunDataFlowTemp, RunDataReturnTemp
  ├── writes: Hc1HeatCurve, Z1DayTemp, Z1NightTemp, HwcSFMode (+ others confirmed writable)
  │
InfluxDB (localhost:8086)
  ├── reads: Leather temp (emon/emonth2_23/temperature), Aldora temp (zigbee2mqtt/aldora_temp_humid)
  ├── writes: adaptive_heating_mvp measurement (decision logs)
  │
  └──→ adaptive-heating-mvp::run_control_cycle()
         │
         ├── mode-specific control logic (occupied/absence/away/disabled)
         ├── DHW Cosy-window boost logic
         │
         ├──→ InfluxDB (decision metrics)
         └──→ JSONL file (full decision audit log)
```

### Mobile control path

```
Phone browser
  → z2m-hub (pi5data:3030) /api/heating/*
    → HTTP proxy to adaptive-heating-mvp (pi5data:3031) /mode/*, /status, /kill
```

## Implicit Contracts

### eBUS availability

The adaptive-heating-mvp assumes ebusd is running on localhost:8888 and responsive. If ebusd is down:
- reads return errors → `missing_core = true` → control logic skipped
- writes fail → logged but control cycle continues
- no retry/reconnect logic beyond the 15-minute cycle

### InfluxDB topic naming

Room temperature topics must match between:
- Telegraf MQTT→InfluxDB config (what gets written)
- `model/adaptive-heating-mvp.toml` `[topics]` (what gets queried)
- `model/thermal-config.toml` sensor_topics (what thermal model uses)

The emonth2 uses `_field = "value"` while Zigbee sensors use `_field = "temperature"`. This is handled in both `src/thermal/influx.rs` and `src/bin/adaptive-heating-mvp.rs::query_latest_room_temp()`.

### VRC 700 baseline safety net

The VRC 700 timers (`Z1Timer_*`, `HwcTimer_*`) remain programmed as fallback. If the adaptive controller stops writing, the VRC 700 continues operating on its own timer/curve schedule. The kill switch explicitly restores known-good register values.

### Config duplication

Radiator T50 values exist in both `config.toml` (used by `analysis.rs`) and `data/canonical/thermal_geometry.json` (used by `thermal/geometry.rs`). These must be kept in sync manually.

The adaptive-heating-mvp has its own baseline values in `model/adaptive-heating-mvp.toml` `[baseline]`. These must match the known-good VRC 700 settings documented in `docs/vrc700-settings-audit.md`.
