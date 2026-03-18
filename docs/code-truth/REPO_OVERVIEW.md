# Repository Overview

```yaml
commit: db2c2ce1fd999ebbf4244cf0adf39119d286d0f6
short_commit: db2c2ce
branch: main
commit_date: 2026-03-18T12:14:00+00:00
working_tree: clean
```

## Purpose

A CLI tool that downloads heat pump monitoring data from [emoncms.org](https://emoncms.org) into a local SQLite database, then analyses it using Polars DataFrames. Built for a **Vaillant Arotherm Plus 5kW** installation with an emonHP monitoring bundle at 6 Rhodes Avenue, London N22 7UT.

The tool classifies each data sample into operating states (heating, DHW, defrost, idle) using a hysteresis state machine driven by flow rate and delta-T, then provides COP breakdowns, temperature correlations, hourly/daily/weekly/monthly profiles, degree day analysis, and comparisons against manufacturer spec and pre-HP gas consumption.

## Key Technologies

| Technology | Version | Role |
|-----------|---------|------|
| Rust | 2021 edition | Language |
| Polars | 0.46 | DataFrame analysis (lazy eval, group_by, CSV export) |
| rusqlite | 0.33 (bundled) | Local data storage — 7.4M samples at 1-min resolution |
| reqwest | 0.12 (blocking) | HTTP client for emoncms API |
| clap | 4 | CLI argument parsing with subcommands |
| chrono | 0.4 | Timestamp and date range handling |
| anyhow | 1 | Error handling |

## High-Level Organisation

| Module | Lines | Responsibility |
|--------|-------|---------------|
| `analysis.rs` | 964 | Operating state classification (state machine), all Polars queries, degree days, COP vs spec, design comparison, indoor temp, DHW analysis |
| `gaps.rs` | 614 | Gap detection, temperature-bin modelling, synthetic data generation |
| `db.rs` | 514 | SQLite storage, incremental sync, loading DataFrames and daily data |
| `main.rs` | 429 | CLI definition (clap), command routing, date range parsing |
| `reference.rs` | 204 | Static reference data from planning workbook (house, radiators, Arotherm spec, gas-era data) |
| `emoncms.rs` | 81 | API client — list feeds, fetch time-series data. Used only by `sync` |

## Entrypoint

`src/main.rs` — defines CLI with `clap::Parser`, routes 17 subcommands.

## Suggested Reading Order

1. `src/main.rs` — understand the CLI commands and data flow
2. `src/analysis.rs` — the operating model documentation (module doc comment) and state machine
3. `src/reference.rs` — house design data, radiator inventory, manufacturer spec
4. `src/db.rs` — how data is stored and loaded
5. `src/emoncms.rs` — the API client (simple, 81 lines)
6. `src/gaps.rs` — gap-filling strategy (largest file, most complex)
