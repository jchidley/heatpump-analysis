# Repository Overview

```yaml
commit: 08e43eb
branch: main
updated: 2026-03-22
```

## What This System Does

A Rust CLI tool that syncs heat pump monitoring data from emoncms.org to a local SQLite database, then analyses it with Polars. It classifies the heat pump's operating state (heating, DHW, defrost, idle) using a hysteresis state machine driven by flow rate, and produces COP breakdowns, energy analysis, degree-day normalisation, and comparisons against both the manufacturer spec and pre-heat-pump gas consumption.

The system is built for a specific installation: a **Vaillant Arotherm Plus 5kW** air-source heat pump at a residential property in London, monitored via an emonHP bundle feeding emoncms.org.

Beyond the Rust analysis tool, the project includes:
- Shell-based **monitoring script** deployed to pi5data (`scripts/ebusd-poll.sh` — systemd service for eBUS data collection)
- Extensive **domain documentation** on the hydraulic system, DHW cylinder analysis, and monitoring infrastructure.
- A separate **z2m-hub** project (`~/github/z2m-hub/`) handles Zigbee devices, automations, DHW tracking/boost, and mobile dashboard. Previously shell scripts here (z2m-automations.sh, dhw-auto-trigger.sh) — both removed Mar 2026.

## Key Technologies

| Technology | Role |
|-----------|------|
| Rust (edition 2021) | All analysis application code |
| Polars 0.46 | DataFrame analysis (lazy evaluation, groupby, aggregation) |
| SQLite (rusqlite 0.33, bundled) | Local data storage, WAL mode |
| TOML (serde + toml crate) | External configuration for all domain constants |
| clap 4 | CLI argument parsing (derive mode) |
| reqwest 0.12 (blocking) | HTTP client for emoncms REST API |
| chrono | Date/time handling |
| anyhow | Error propagation |
| once_cell | Global config singleton |
| Shell (bash) | Monitoring scripts deployed to pi5data (DHW trigger, eBUS polling, Z2M automations) |
| ebusd 26.1 | eBUS protocol decoder for Vaillant heat pump communication |

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
        └──▶ octopus.rs (joins with ~/github/octopus/data/ CSV+JSON files)

   config.toml ──▶ config.rs (global singleton, read by all modules)

   [Separately on pi5data:]
   ebusd-poll.sh → ebusd (eBUS → HP) → MQTT → Telegraf → InfluxDB
   z2m-hub (Rust, ~/github/z2m-hub/) → Z2M WebSocket + ebusd TCP
```

## How It's Organised

- **`config.toml`** — Single source of truth for all domain data: emoncms feed IDs, operating thresholds, house thermal properties, radiator inventory, Arotherm manufacturer specs, and gas-era consumption history. Loaded once at startup.
- **`src/`** — Six Rust modules plus `main.rs` (3,591 lines total). No nested directories, no procedural macros, no build scripts. Each module has one clear responsibility.
- **`scripts/`** — Shell script deployed to pi5data as systemd service (`ebusd-poll.sh`). Previously also had `dhw-auto-trigger.sh` and `z2m-automations.sh` — both removed Mar 2026, replaced by z2m-hub.
- **`ebusd/`** — Git submodule (https://github.com/john30/ebusd), reference for eBUS protocol.
- **`.gitmodules`** — Six submodules: ebusd, avrdb_firmware, EmonScripts, emonhub, emoncms, emonPiLCD.
- **`docs/`** — Human-facing documentation (Diátaxis style) plus code-truth.
- **`heatpump.db`** — SQLite database (gitignored). Created by `sync`, read by all analysis commands.

## 20 CLI Subcommands

Grouped by concern:

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

7. **Three-layer monitoring** — emonhp (MID-certified meters, ground truth), eBUS (internal HP operating data), and Multical (DHW delivery side) give end-to-end visibility from electricity input through to hot water at taps.
