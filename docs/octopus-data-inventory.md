# Octopus Energy Data Inventory

Audited: 2026-03-18

## Sources

### 1. `~/github/OctopusEnergyMonitor/` (legacy parquet)

| File | Rows | Coverage | Granularity | Total kWh | Non-zero % | Notes |
|------|------|----------|-------------|-----------|------------|-------|
| `e_consumption.parquet` | 64,129 | 2020-04-10 → 2023-12-09 | Half-hourly | 19,115 | 99.99% | Primary electricity source |
| `g_consumption.parquet` | 61,868 | 2020-04-10 → 2023-12-09 | Half-hourly | 6,112 (m³) | 58% | Gas in m³, zeros are summer/no-heating |
| `electricity_consumption.parquet` | 25,000 | 2021-06-11 → 2022-11-13 | Half-hourly | — | — | Subset of `e_consumption`, redundant |
| `gas_consumption.parquet` | 25,000 | 2021-06-09 → 2022-11-13 | Half-hourly | — | — | Subset of `g_consumption`, redundant |
| `agile_tariff_rates.parquet` | 100 | 2022-11-13 → 2022-11-15 | Half-hourly | — | — | 2-day sample, not useful |

**Parquet schema (e/g_consumption):**
- Index: `interval_start` (datetime64[ns, UTC])
- Columns: `consumption` (float64), `interval_end` (datetime64[ns, UTC])

**Gas unit note:** Values are in m³. Convert to kWh: `m³ × volume_correction (1.02264) × calorific_value (39.2) ÷ 3.6`. This gives ~1 kWh per 0.092 m³. The `heat_context.example.toml` in `~/github/octopus/data/` documents these conversion factors.

**Other files in repo:**
- `app.py`, `main.py` — legacy FastAPI + Plotly dashboard (not maintained)
- `config.ini` — contains account number reference
- `octopus.ipynb`, `arrow-octopus.ipynb` — Jupyter exploration notebooks
- `file.json`, `consumption_example.json`, `iris.json` — test/example data

### 2. `~/github/octopus/` (active REST scripts)

| File | Rows | Coverage | Granularity | Notes |
|------|------|----------|-------------|-------|
| `electricity_365d.csv` | 365 | 2025-01-31 → 2026-01-29 | **Daily** | Source: "Amphio", not REST API |

**CSV schema (`electricity_365d.csv`):**
```
startAt,endAt,readAt,value,unit,source,durationInSeconds
2025-01-31T09:15:58-06:00,2025-02-01T00:00:00-06:00,...,16.047000,kwh,Amphio,53042
```

**Key commands (single TypeScript codebase):**
- `npm run cli -- fetch` — fetches half-hourly elec+gas via REST API, paginated (25k/page)
  - Output CSV schema: `fuel,interval_start,interval_end,consumption_kwh`
  - Supports `--electricity`, `--gas`, `--both`, `--from`, `--to`, `--csv`
- `npm run cli -- sync-env` — auto-discovers MPAN/MPRN/serial numbers from account, writes `.envrc`
- `npm run cli -- merge` — merges multiple CSVs with deduplication
- `npm run cli -- refresh` — one-command fetch + merge + weather update (primary refresh method)

**SPA dashboard:** `dist/` contains a TypeScript SPA that imports CSVs and caches in IndexedDB.

## Coverage Timeline

```
2020-04 ─────── gas+elec half-hourly (parquet) ─────── 2023-12
                                                                [GAP: ~13 months]
                                                                            2024-10 ── HP emoncms ── present
                                                                      2025-01 ── daily elec (CSV) ── 2026-01
```

### The Gap: Jan 2024 → Jan 2025

- No Octopus consumption data exists locally for this period
- The Octopus REST API retains data for at least 2 years — this gap is almost certainly fillable
- This period includes the critical **gas→heat pump transition** (Oct–Nov 2024)
- Filling it would give: gas consumption in final gas-heating months, electricity during HP commissioning

## Environment Variables

| Variable | Purpose | Source |
|----------|---------|--------|
| `OCTOPUS_API_KEY` | REST API authentication | `ak get octopus` (GPG-encrypted) |
| `OCTOPUS_ACCOUNT_NUMBER` | Account lookup | Manual |
| `OCTOPUS_MPAN` | Electricity meter point | Auto-discovered by `npm run cli -- sync-env` |
| `OCTOPUS_MARKET_SUPPLY_POINT_ID` | Same as MPAN | Auto-discovered |
| `OCTOPUS_E_SERIAL` | Electricity meter serial | Auto-discovered |
| `OCTOPUS_MPRN` | Gas meter point | Auto-discovered |
| `OCTOPUS_G_SERIAL` | Gas meter serial | Auto-discovered |

## Status: ✅ Complete

API key stored in `ak` (GPG-encrypted). All steps done 2026-03-18.

### What was done

1. **`ak set octopus`** — API key stored ✅
2. **`sync-env`** — meter IDs discovered into `.envrc` ✅
3. **Full REST history fetched** — `usage_full.csv` (Mar 2024 → Mar 2026, 40,313 rows) ✅
4. **Legacy parquet converted** — `data/legacy_usage.csv` (Apr 2020 → Dec 2023, 125,997 rows) ✅
5. **Merged** — `data/usage_merged.csv` (166,310 rows, Apr 2020 → Mar 2026) ✅
6. **Integrated** — `octopus.rs` in heatpump-analysis reads `usage_merged.csv` + `weather.json` + `config.json` directly from `~/github/octopus/data/` ✅

### Remaining gap

102 days with no data for either fuel: **Dec 2023 → Mar 2024**. The REST API doesn't hold it — likely a meter/comms outage.

### To refresh data

```bash
cd ~/github/octopus && npm run cli -- refresh
```

## Integration with heatpump-analysis

Implemented in `octopus.rs` with three subcommands:

| Subcommand | What it does |
|------------|-------------|
| `octopus` | Monthly breakdown: elec, gas, HDD, elec/HDD (all data) |
| `gas-vs-hp` | Gas era vs HP era comparison with heating/DHW separated |
| `baseload` | Daily whole-house electricity minus HP electricity |

### Key results

| Metric | Gas era | HP era |
|--------|---------|--------|
| Heating heat/HDD | 9.2 kWh | 8.8 kWh |
| Heating COP | 0.9 (boiler) | 4.74 |
| DHW heat/day | 11.8 kWh (est.) | 11.0 kWh (measured) |
| DHW COP | 0.9 (boiler) | 3.46 |
| Cost/kWh heat | 6.29p | 3.60p |
| Annual cost | £1,239 | £674 |
| **Annual saving** | | **£565 (46%)** |

### Temperature sources

- HP era: emoncms feed 503093 (Met Office hourly) — most accurate
- Gas era: ERA5-Land + 1.0°C bias correction (from 507-day overlap)
- Without bias correction, ERA5 overstates HDD by ~14%

### Alignment notes

- HP state machine classifies every 1-min sample into heating/DHW/defrost
- `gas-vs-hp` uses state machine for HP era, estimated 11.82 kWh/day DHW for gas era
- Octopus timestamps are UTC; emoncms feeds are also UTC — no TZ conversion needed
- The cutover date 2024-10-22 separates gas and HP eras
