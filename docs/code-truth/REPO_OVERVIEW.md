# Repository Overview

```yaml
commit: 07b9b7f56631d4273263ef5adc6917885f74f0c8
short_commit: 07b9b7f
branch: main
commit_date: 2026-03-18T11:08:56+00:00
working_tree: clean
```

## Purpose

A CLI tool that downloads heat pump monitoring data from [emoncms.org](https://emoncms.org) into a local SQLite database, then analyses it using Polars DataFrames. Built for a **Vaillant Arotherm Plus 5kW** installation with an emonHP monitoring bundle.

The tool classifies each data sample into operating states (heating, DHW, defrost, idle) using a hysteresis state machine driven by flow rate and delta-T, then provides COP breakdowns, temperature correlations, hourly profiles, and daily energy summaries.

## Key Technologies

| Technology | Version | Role |
|-----------|---------|------|
| Rust | 2021 edition | Language |
| Polars | 0.46 | DataFrame analysis (lazy eval, group_by, pivot) |
| rusqlite | 0.33 (bundled) | Local data storage — 7.4M samples at 1-min resolution |
| reqwest | 0.12 (blocking) | HTTP client for emoncms API |
| clap | 4 | CLI argument parsing with subcommands |
| chrono | 0.4 | Timestamp handling |
| anyhow | 1 | Error handling |

## High-Level Organisation

| Module | Responsibility |
|--------|---------------|
| `emoncms.rs` | API client — list feeds, fetch time-series data. Used only by `sync` |
| `db.rs` | SQLite storage, incremental sync, loading DataFrames from local data |
| `analysis.rs` | Operating state classification (state machine), Polars queries for all analyses |
| `gaps.rs` | Gap detection, temperature-bin modelling, synthetic data generation |
| `main.rs` | CLI definition (clap) and command routing |

## Entrypoint

`src/main.rs` — defines CLI with `clap::Parser`, routes to subcommands.

## Suggested Reading Order

1. `src/main.rs` — understand the CLI commands and data flow
2. `src/analysis.rs` — the operating model documentation (module doc comment) and state machine
3. `src/db.rs` — how data is stored and loaded
4. `src/emoncms.rs` — the API client (simple, ~80 lines)
5. `src/gaps.rs` — gap-filling strategy (largest file, most complex)
