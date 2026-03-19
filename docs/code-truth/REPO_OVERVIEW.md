# Repository Overview

```yaml
commit: 9d6d3e3bc9283a48be4d51fe7c6e0d4f37ca41ca
short_commit: 9d6d3e3
branch: main
commit_date: 2026-03-19 08:36:14 +0000
working_tree: clean
```

## What This System Does

A Rust CLI tool that syncs heat pump monitoring data from emoncms.org to a local SQLite database, then analyses it with Polars. It classifies the heat pump's operating state (heating, DHW, defrost, idle) using a hysteresis state machine driven by flow rate, and produces COP breakdowns, energy analysis, degree-day normalisation, and comparisons against both the manufacturer spec and pre-heat-pump gas consumption.

The system is built for a specific installation: a **Vaillant Arotherm Plus 5kW** air-source heat pump at a residential property in London, monitored via an emonHP bundle feeding emoncms.org.

## Key Technologies

| Technology | Role |
|-----------|------|
| Rust (edition 2021) | All application code |
| Polars 0.46 | DataFrame analysis (lazy evaluation, groupby, aggregation) |
| SQLite (rusqlite, bundled) | Local data storage, WAL mode |
| TOML (serde + toml crate) | External configuration for all domain constants |
| clap 4 | CLI argument parsing (derive mode) |
| reqwest (blocking) | HTTP client for emoncms REST API |
| chrono | Date/time handling |
| anyhow | Error propagation |
| once_cell | Global config singleton |

## Data Flow

```
emoncms.org REST API
        │
        ▼
   emoncms.rs (HTTP client)
        │
        ▼
   db.rs (SQLite: feeds, samples, sync_state tables)
        │
        ├──▶ analysis.rs (Polars DataFrames → state machine → reports)
        │
        ├──▶ gaps.rs (gap detection + synthetic data → simulated_samples table)
        │
        └──▶ octopus.rs (joins with ~/github/octopus/ JSON files)
        
   config.toml ──▶ config.rs (global singleton, read by all modules)
```

## How It's Organised

- **`config.toml`** — Single source of truth for all domain data: emoncms feed IDs, operating thresholds, house thermal properties, radiator inventory, Arotherm manufacturer specs, and gas-era consumption history. Loaded once at startup.
- **`src/`** — Six Rust modules plus `main.rs`. No nested directories, no procedural macros, no build scripts. Each module has one clear responsibility.
- **`docs/`** — Explanation and code-truth documentation. No generated API docs.
- **`heatpump.db`** — SQLite database (gitignored). Created by `sync`, read by all analysis commands.

## 20 Subcommands

The CLI has 20 subcommands grouped by concern:

- **Data acquisition**: `feeds`, `sync`
- **Database inspection**: `db-status`, `data`, `export`
- **Core analysis**: `summary`, `cop-by-temp`, `hourly`, `daily`
- **Weather-normalised**: `degree-days`, `indoor-temp`
- **Comparison**: `dhw`, `cop-vs-spec`, `design-comparison`
- **Gap management**: `gaps`, `fill-gaps`
- **Octopus Energy**: `octopus`, `gas-vs-hp`, `baseload`
- **Batch**: `all` (runs a subset of analyses)

## What Makes This System Unusual

1. **External TOML configuration** — All numeric constants (thresholds, feed IDs, house data, radiator specs, gas history) live in `config.toml`, not in Rust source. Changing a threshold or adding a feed doesn't require recompilation.

2. **State machine from flow rate** — Operating state is classified from flow rate (a mechanical signal from the diverter valve) rather than temperature or flags. This gives a clean bimodal signal that works across the entire dataset.

3. **Gap filling constrained by meters** — Synthetic data during monitoring gaps is scaled so its integrated energy exactly matches the cumulative energy meters, which run continuously even when the logger drops out.

4. **Dual-era comparison** — Gas-era consumption data (Octopus billing) and heat-pump-era data are normalised by heating degree days with temperature from two sources (ERA5-Land for pre-HP, Met Office for HP era) with a measured bias correction.

5. **No tests** — Validated against real data output rather than unit tests. Changes must be verified against the full dataset.
