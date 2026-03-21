# Roadmap

Planned enhancements, roughly ordered by value and readiness.

## eBUS Integration

**Status:** ✅ Hardware connected and publishing data. Not yet integrated into the Rust analysis tool.

The eBUS adapter (ESP32 Shield v1.24, firmware 20260317) connects to ebusd 26.1 running as Docker on pi5data (port 8888). A polling script (`ebusd-poll.sh`, systemd on pi5data) publishes 25+ values every 30 seconds to MQTT, stored in InfluxDB with Grafana dashboards.

### What eBUS now provides (via MQTT/InfluxDB)

| Signal | Available | Status |
|--------|-----------|--------|
| **Operating mode (StatuscodeNum)** | ✅ Every 30s | 104=heating, 134=DHW, 100=standby, 516=defrost |
| **Compressor speed / utilisation** | ✅ Every 30s | Percentage values |
| **Target flow temperature** | ✅ Every 30s | Weather compensation output |
| **Outside air temperature** | ✅ Every 30s | From the outdoor unit sensor |
| **Cylinder temperature (HwcStorageTemp)** | ✅ Every 5 min | Replaces dead DHW_flag feed |
| **COP values (HC/HWC/monthly)** | ✅ Every 5 min | HP's own COP calculation |
| **Compressor inlet/outlet temps** | ✅ Every 30s | Refrigerant circuit data |
| **High pressure, EEV position, fan speed** | ✅ Every 30s | Diagnostic data |
| **Error codes / status** | ✅ Every 30s | Via Statuscode text |

### What's NOT yet done

- **Rust analysis integration** — the flow-rate state machine in `analysis.rs` could be validated or replaced using `StatuscodeNum`. Not yet investigated.
- **eBUS OAT for temperature analysis** — could replace Met Office data for better resolution, but needs calibration comparison first.
- **Defrost analysis** — eBUS provides definitive defrost status (516) vs the current inference from negative DT/heat.
- **emoncms import** — eBUS data is only in InfluxDB (via pi5data). Could be added as new emoncms feeds for the existing sync pipeline.

See [../heating-monitoring-setup.md](../heating-monitoring-setup.md) for the full eBUS data dictionary and MQTT topic list.

## Octopus Energy Integration

**Status:** ✅ Complete. Integrated into heatpump-analysis via `octopus.rs`.

Full data audit: [octopus-data-inventory.md](octopus-data-inventory.md)

### What it gives us

| Analysis | Status |
|----------|--------|
| Gas vs HP comparison (heating-only, DHW separated) | ✅ `gas-vs-hp` subcommand |
| Baseload (whole-house − HP) | ✅ `baseload` subcommand |
| Monthly breakdown with HDD | ✅ `octopus` subcommand |
| Cost per kWh of heat | ✅ Calculated (gas 6.29p, HP heating 3.60p) |
| Annual saving | ✅ £565/yr (46%) at current tariffs |
| Tariff history | ✅ Documented in CLAUDE.md |

### Data pipeline

```
~/github/octopus/ project:
  Octopus REST API → usage CSVs → merge → usage_merged.csv
  Legacy parquet (OctopusEnergyMonitor) → legacy_usage.csv → merged in
  Open-Meteo ERA5-Land → weather.json

heatpump-analysis:
  octopus.rs reads usage_merged.csv + weather.json + config.json
  + heatpump.db for HP state machine (heating/DHW/defrost classification)
  + emoncms feed 503093 for accurate HP-era temperatures
```

### Refresh
```bash
cd ~/github/octopus && npm run cli -- refresh
```

### Key findings

- **Heating heat/HDD**: gas era 9.2, HP era 8.8 (4% drop = insulation improvement)
- **Heating COP**: 4.74 (state machine, heating-only days with HDD > 0.5)
- **DHW COP**: 3.46 (state machine)
- **Cost/kWh heat**: gas 6.29p (5.66p ÷ 90%), HP 3.60p (17.07p ÷ COP 4.74)
- **Break-even gas price**: 2.92p/kWh (below current and recent rates)
- **Remaining data gap**: 102 days Dec 2023 → Mar 2024 (unfillable — not in REST API)

### Temperature data quality

ERA5-Land (used for gas era) reads ~1.0°C colder than the emoncms Met Office sensor.
`octopus.rs` applies a +1.0°C bias correction to ERA5 for gas-era dates, and uses emoncms
directly for HP-era dates. Without this correction, ERA5 overstates HDD by ~14%.

## Degree Day Analysis

**Status:** ✅ Done — implemented with Met Office data. Monthly aggregation and gas-era comparison included.

### What it would give us

| Analysis | Description |
|----------|-------------|
| **Heating degree days (HDD)** | Standardised measure of heating demand per day/week/month |
| **Energy per degree day** | Normalised efficiency metric — removes weather variation |
| **Seasonal performance trends** | Compare same-HDD periods across months to detect degradation |
| **Base temperature estimation** | At what outside temp does the house stop needing heating? |

### Implementation notes

- HDD = max(0, base_temp − mean_daily_OAT) — typically base 15.5°C in UK
- With eBUS OAT: calculate from actual readings (more accurate than Met Office for the specific location)
- With Met Office: adequate for weekly/monthly aggregates
- Combine with Octopus data for £/HDD (cost normalised for weather)

## Excel Planning Data Import

**Status:** ✅ Done — key reference data encoded in `reference.rs`.

### What it would give us

| Data | Why it matters |
|------|---------------|
| **Design heat loss** | Compare actual consumption against design — is the system oversized/undersized? |
| **Radiator/UFH emitter data** | Expected flow temps at design conditions |
| **Cylinder specification** | Coil area, volume — affects DHW COP expectations |
| **Installation parameters** | Glycol concentration, pipe runs, etc. |

### Implementation notes

- Could be a one-time import of reference values into the SQLite database (a `planning` table)
- Or a separate config file (TOML/YAML) loaded at analysis time
- Key use: overlay design expectations on actual performance charts
- Need to understand the workbook structure before designing the import

## Solar PV + Battery Integration

**Status:** System installed and commissioned 19/04/2024. Not yet integrated into analysis.

### Installed system

| Component | Details |
|-----------|---------|
| **Panels** | 7× Trina Vertex S+ 440W (TSM-440NEG9RC.27), N-Type Mono, dual glass |
| **Array** | 3.08 kWp, single string, pitched roof above-roof mount (ValkPitched) |
| **Inverter** | Fox ESS F3600, 3.6kW single phase, dual MPPT (one input used) |
| **Battery** | Tesla Powerwall 2, 13.5 kWh, with Gateway |
| **Generation meter** | Emlite M24 (serial: Eml2405204845) |
| **Estimated annual** | ~2,900 kWh (pro-rata from MCS 3,711 for 9 panels) |
| **Installer** | Nanopro-Tech Ltd (MCS NAP66233) |

### What it would give us

| Data | Why it matters |
|------|---------------|
| **Generation profile** | When is solar available vs when does the HP run? |
| **Self-consumption** | How much HP electricity comes from solar vs grid? |
| **Battery state** | When is the Powerwall charging/discharging vs HP running? |
| **Effective COP** | Solar kWh at £0 changes the economics — cost-weighted COP |
| **DHW scheduling** | Should DHW shift to solar-peak hours (midday) instead of 05:05? |

### Implementation notes

- Tesla Powerwall Gateway provides local API for battery state, solar generation, grid import/export
- Fox ESS inverter may have its own monitoring portal/API
- Generation meter reading provides ground truth for total solar production
- Key analysis: overlay HP consumption on solar generation + battery state timeline
- With Octopus data: marginal cost per kWh consumed (grid vs solar vs battery)

## Other Data Sources

Files in `C:\Users\jackc\OneDrive\Documents\House\` that could be imported:

| File | Potential Use |
|------|--------------|
| `EGWU_HDD_17C.csv` | Historical degree days for longer comparison |
| `ILONDONL9_HDD_18C-DegreeDaysData.csv` | Alternative weather station |
| `Regressions_EGWU_*.csv` | Pre-computed regression analysis |
| `Utility - Gas Electric v2.xlsx` | Detailed utility bills |
| `agile_rates_2019.xlsx` | Historical Octopus Agile rates |
| `weekly.xlsx` | Weekly gas consumption data |
| `Cost_data_summary.xlsx` | Cost analysis |
| `DHDG sizing spreadsheet @ 19&21 v1.1 finish.xlsx` | DHW sizing calculations |

## Other Potential Enhancements

- **Home Assistant integration** — pull myVAILLANT data (room temp setpoints, schedules) if rate limiting allows
- **Weather forecast correlation** — predict next-day heating demand from Met Office forecast
- **Defrost analysis** — dedicated report on defrost frequency, duration, energy cost vs outside temp/humidity
- **Multi-period comparison** — "this January vs last January" with degree-day normalisation
- **Alerting** — detect COP degradation, unusual cycling, sensor drift
