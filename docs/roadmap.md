# Roadmap

Planned enhancements, roughly ordered by value and readiness.

## eBUS Integration

**Status:** Hardware connected but not configured.

An eBUS adapter is plugged into the Vaillant Arotherm. eBUS is the serial bus protocol Vaillant uses for internal communication between the outdoor unit, indoor controller (SensoCOMFORT), and cylinder sensor.

### What it would give us

| Signal | Why it matters |
|--------|---------------|
| **Outside air temperature (OAT)** | Real-time at the unit, ~10s resolution vs Met Office hourly. Critical for accurate COP-vs-temperature analysis, defrost timing, and degree-day calculations |
| **Compressor speed / frequency** | Actual modulation level — currently we infer load from electrical power |
| **Target flow temperature** | The weather compensation curve output — lets us compare actual vs target |
| **Defrost status** | Binary flag rather than inferring from negative DT/heat |
| **DHW target/actual cylinder temp** | Currently no cylinder temperature in emoncms (the DHW_flag feed died Dec 2024) |
| **Error codes / status** | Diagnostic data |

### Implementation notes

- eBUS adapters typically expose data via MQTT or REST (e.g. ebusd)
- Data could either be logged to emoncms (new feeds → existing sync picks them up) or stored directly in the SQLite database
- The OAT sensor would replace Met Office data for all temperature-dependent analysis, giving much better resolution for gap-filling model
- Need to research: ebusd setup for Arotherm Plus, which eBUS message IDs carry the signals above

## Octopus Energy Integration

**Status:** Account exists, API access available.

### What it would give us

| Data | Why it matters |
|------|---------------|
| **Half-hourly electricity consumption** | Independent cross-check against the SDM120 meter |
| **Half-hourly electricity cost** | Cost per kWh of heat, cost-weighted COP |
| **Tariff rates by time slot** | Agile/Cosy/Go rate structure for optimisation analysis |
| **Gas consumption** (if any backup) | Total heating cost including any gas top-up |

### Implementation notes

- Octopus API: `https://api.octopus.energy/v1/` — well-documented REST API
- Need: account number, API key, MPAN/MPRN, meter serial
- Data is half-hourly, so joins with minute-resolution HP data need careful alignment
- Key analysis: "what did each kWh of heat actually cost?" broken down by tariff period, operating state (heating vs DHW), and time of day
- Could identify optimal DHW scheduling (shift to cheapest slots)

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
| **Panels** | 9× Trina Vertex S+ 440W (TSM-440NEG9RC.27), N-Type Mono, dual glass |
| **Array** | 3.96 kWp, 2 strings (5 + 4), pitched roof above-roof mount (ValkPitched) |
| **Inverter** | Fox ESS F3600, 3.6kW single phase, dual MPPT |
| **Battery** | Tesla Powerwall 2, 13.5 kWh, with Gateway |
| **Generation meter** | Emlite M24 (serial: Eml2405204845) |
| **Estimated annual** | 3,711 kWh |
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
