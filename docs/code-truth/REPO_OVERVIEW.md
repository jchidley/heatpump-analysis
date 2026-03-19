# Repository Overview

```yaml
commit: 33e263ae250612cb0b4287815a462b4e93e88161
short_commit: 33e263a
branch: main
commit_date: 2026-03-18T20:25:01+00:00
working_tree: modified_with_untracked
```

## Purpose

A CLI tool that downloads heat pump monitoring data from [emoncms.org](https://emoncms.org) into a local SQLite database, then analyses it using Polars DataFrames. Also integrates Octopus Energy consumption and tariff data for gas-vs-HP cost comparison. Built for a **Vaillant Arotherm Plus 5kW** installation with an emonHP monitoring bundle at 6 Rhodes Avenue, London N22 7UT.

The tool classifies each data sample into operating states (heating, DHW, defrost, idle) using a hysteresis state machine driven by flow rate and delta-T, then provides COP breakdowns, temperature correlations, degree day analysis, gas-era comparisons (with DHW separated from space heating), baseload analysis, and comparisons against manufacturer spec and pre-HP gas consumption.

## Key Technologies

| Technology | Version | Role |
|-----------|---------|------|
| Rust | 2021 edition | Language |
| Polars | 0.46 | DataFrame analysis (lazy eval, group_by, string ops, CSV export) |
| rusqlite | 0.33 (bundled) | Local data storage — 7.4M samples at 1-min resolution |
| reqwest | 0.12 (blocking) | HTTP client for emoncms API |
| clap | 4 | CLI argument parsing with subcommands |
| chrono | 0.4 | Timestamp and date range handling |
| serde / serde_json | 1 | JSON deserialization (Octopus consumption + weather data) |
| anyhow | 1 | Error handling |

Polars features: `lazy`, `pivot`, `dtype-datetime`, `fmt`, `round_series`, `temporal`, `csv`, `strings`.

## High-Level Organisation

| Module | Lines | Responsibility |
|--------|-------|---------------|
| `analysis.rs` | 964 | Operating state classification (state machine), all Polars queries, degree days, COP vs spec, design comparison, indoor temp, DHW analysis |
| `octopus.rs` | 649 | Octopus Energy integration: loads consumption + weather JSON, hybrid temperature (emoncms + bias-corrected ERA5), gas-vs-HP comparison with heating/DHW split, baseload analysis |
| `gaps.rs` | 614 | Gap detection, temperature-bin modelling, synthetic data generation |
| `db.rs` | 514 | SQLite storage, incremental sync, loading DataFrames and daily data |
| `main.rs` | 490 | CLI definition (clap), command routing, date range parsing |
| `reference.rs` | 204 | Static reference data from planning workbook (house, radiators, Arotherm spec, gas-era data) |
| `emoncms.rs` | 81 | API client — list feeds, fetch time-series data. Used only by `sync` |

## Two Data Sources

The tool combines data from two independent systems:

1. **emoncms** (HP monitoring) — 10-second/1-minute resolution power, temperature, flow data from Oct 2024. Stored in local SQLite (`heatpump.db`).

2. **Octopus Energy** (billing data) — half-hourly electricity and gas consumption from Apr 2020. Pre-processed JSON at `~/github/octopus/dist/data/`. Gas era ends Jul 2024, electricity continues to present.

The overlap period (Oct 2024 → present) enables: whole-house vs HP-only electricity comparison (baseload), and cross-validation of energy totals.

## Entrypoint

`src/main.rs` — defines CLI with `clap::Parser`, routes 20 subcommands.

## Suggested Reading Order

1. `src/main.rs` — understand the CLI commands and data flow
2. `src/analysis.rs` — the operating model documentation (module doc comment) and state machine
3. `src/octopus.rs` — Octopus Energy integration, temperature hierarchy, gas-vs-HP comparison
4. `src/reference.rs` — house design data, radiator inventory, manufacturer spec
5. `src/db.rs` — how data is stored and loaded
6. `src/emoncms.rs` — the API client (simple, 81 lines)
7. `src/gaps.rs` — gap-filling strategy (largest file, most complex)
