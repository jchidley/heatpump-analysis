# heatpump-analysis

Analyse heat pump performance from [emoncms.org](https://emoncms.org) monitoring data. The project downloads monitoring data to a local SQLite database, classifies operating states, runs thermal and DHW analysis, and supports live heating-control experiments.

Built for a **Vaillant Arotherm Plus 5kW** with the emonHP monitoring bundle.

This README is a signpost for human readers:
- use it for quick start, command discovery, and navigation
- use `docs/` for human explanations and task guides
- use `lat.md/` for canonical current-state project truth

## Quick Start

```bash
# Build
cargo build --release

# Download all data (first time — takes a few minutes)
cargo run -- --apikey YOUR_EMONCMS_READ_KEY sync

# Analyse last 30 days
cargo run -- --days 30 summary

# Run everything on all data
cargo run -- --all-data all
```

Set `EMONCMS_APIKEY` environment variable to avoid passing `--apikey` each time. The API key is only needed for `feeds` and `sync` commands — all analysis runs from the local database.

All operating thresholds, feed IDs, house data, and reference data are in `config.toml` — edit that file to change parameters without recompiling.

## CLI reference

| Command | Description |
|---------|-------------|
| `feeds` | List available emoncms feeds (requires API key) |
| `sync` | Download/update data to local SQLite database |
| `db-status` | Show database contents and date range |
| `summary` | Overall stats and breakdown by operating state |
| `cop-by-temp` | COP vs outside temperature bands (heating only) |
| `hourly` | Average profile by hour of day |
| `daily` | Daily energy totals and COP from cumulative meters |
| `degree-days` | HDD analysis — energy normalised by weather, monthly, gas-era comparison |
| `indoor-temp` | Indoor temperature analysis (Leather room sensor) |
| `dhw` | DHW analysis vs design expectations |
| `cop-vs-spec` | Actual COP vs Arotherm manufacturer spec curve |
| `design-comparison` | Full design comparison: radiators, gas era, HDD-normalised savings |
| `gaps` | Report data gaps and their fill status |
| `fill-gaps` | Fill gaps with modelled data (energy-scaled to match meters) |
| `export` | Export enriched data to CSV (`-o file.csv` or stdout) |
| `data` | Show raw enriched data table |
| `octopus` | Octopus Energy data summary (consumption + weather + monthly breakdown) |
| `gas-vs-hp` | Compare gas-era vs heat-pump-era energy use (normalised by degree days) |
| `baseload` | Whole-house electricity minus heat pump electricity |
| `overnight` | Overnight heating strategy optimizer — backtest optimal schedules |
| `all` | Run summary + cop-by-temp + hourly + daily + degree-days |
| `thermal-rooms` | Room summary table (geometry, thermal mass, radiators, pipes) |
| `thermal-connections` | Inter-room connections and doorway exchanges |
| `thermal-analyse` | Live TSDB-backed energy balance (per-room heat flows; PostgreSQL-first) |
| `thermal-equilibrium` | Solve for equilibrium room temperatures at given conditions |
| `thermal-moisture` | Condensation risk + overnight humidity balance |
| `thermal-calibrate` | Calibrate thermal model from house telemetry cooldown data via the shared TSDB seam |
| `thermal-validate` | Validate thermal model on holdout windows |
| `thermal-fit-diagnostics` | Period-by-period cooldown fit diagnostics |
| `thermal-operational` | Operational validation (heating/DHW/off with solar) |
| `thermal-snapshot` | Export/import reproducibility snapshots (human-gated) |
| `thermal-control-table` | Generate MWT control table for adaptive heating |
| `dhw-sessions` | Analyse DHW draw/charge sessions from house telemetry via the shared TSDB seam |
| `heating-history` | Reconstruct fused high-resolution heating-history evidence; defaults to last 7 days ending now |
| `dhw-history` | Reconstruct fused high-resolution DHW-history evidence; defaults to last 7 days ending now |
| `history-review` | Comprehensive high-resolution 7-day-to-now review for `heating`, `dhw`, or `both` |

### Options

| Flag | Default | Description |
|------|---------|-------------|
| `--days N` | 7 | How many days of history to analyse |
| `--all-data` | | Analyse all available data (overrides `--days`) |
| `--from YYYY-MM-DD` | | Start date (overrides `--days`) |
| `--to YYYY-MM-DD` | now | End date |
| `--db PATH` | `heatpump.db` | SQLite database path |
| `--include-simulated` | off | Include gap-filled data in analysis |

### Examples

```bash
# Last winter
cargo run -- --from 2024-12-01 --to 2025-02-28 summary

# Export January data to CSV for spreadsheet analysis
cargo run -- --from 2025-01-01 --to 2025-01-31 export -o january.csv

# Standard historical investigation: rolling 7 days ending now
cargo run --bin heatpump-analysis -- heating-history
cargo run --bin heatpump-analysis -- dhw-history
cargo run --bin heatpump-analysis -- dhw-sessions --days 7 --format json

# One-command comprehensive reviews
cargo run --bin heatpump-analysis -- history-review heating
cargo run --bin heatpump-analysis -- history-review dhw
cargo run --bin heatpump-analysis -- history-review both

# Fixed regression anchor windows when you need a specific known event
cargo run --bin heatpump-analysis -- heating-history \
  --since 2026-04-02T00:00:00Z --until 2026-04-02T09:00:00Z
cargo run --bin heatpump-analysis -- dhw-history \
  --since 2026-03-21T05:00:00Z --until 2026-03-21T08:00:00Z

# All data with gap-filled samples
cargo run -- --all-data --include-simulated all

# Full design comparison
cargo run -- --all-data design-comparison
```

## How It Works

→ [docs/explanation.md](docs/explanation.md)

## Documentation

Start with **[docs/README.md](docs/README.md)** for the human docs map.

### Canonical current-state project truth
- **[lat.md/](lat.md/)** — architecture, domain rules, constraints, infrastructure, controller behaviour, and history-evidence boundaries. Validated by `lat check`.

### Human-oriented plans and deep dives
- **[docs/heating-plan.md](docs/heating-plan.md)** — heating strategy and decision rationale
- **[docs/dhw-plan.md](docs/dhw-plan.md)** — DHW strategy and decision rationale
- **[docs/explanation.md](docs/explanation.md)** — rationale behind the operating-state model
- **[docs/hydraulic-analysis.md](docs/hydraulic-analysis.md)** — y-filter / flow-rate evidence and diagnosis
- **[docs/house-layout.md](docs/house-layout.md)** — detailed room-by-room building notes
- **[docs/room-thermal-model.md](docs/room-thermal-model.md)** — thermal-model methodology and experiment results
- **[docs/heating-reference.md](docs/heating-reference.md)** — supporting heating reference details and evidence
- **[docs/dhw-reference.md](docs/dhw-reference.md)** — supporting DHW reference details and evidence
- **[docs/vrc700-settings-audit.md](docs/vrc700-settings-audit.md)** — full VRC 700 audit trail and timer-encoding investigation
- **[docs/history-evidence-workflows.md](docs/history-evidence-workflows.md)** — step-by-step retrospective analysis workflows

### Operations and implementation maps
- **[lat.md/tsdb-migration.md](lat.md/tsdb-migration.md)** — sole repo-local TSDB migration tracker for heatpump-analysis cutover work
- **`~/github/energy-hub/lat.md/tsdb-migration.md`** — shared platform TSDB migration tracker for schema, ingest, gap-fill, and final decommission
- **[heating-monitoring-setup.md](heating-monitoring-setup.md)** — operational setup/runbook detail beyond the lat summary
- **[docs/emon-installation-runbook.md](docs/emon-installation-runbook.md)** — rebuild/provisioning procedures for emon devices
- **[docs/implementation-maps/](docs/implementation-maps/)** — preserved implementation snapshots for onboarding and file discovery
- **[lat.md/src/](lat.md/src/)** — file-level source pages for implementation discovery
- **[docs/octopus-data-inventory.md](docs/octopus-data-inventory.md)** — Octopus data audit
- **[docs/pico-ebus-plan.md](docs/pico-ebus-plan.md)** — Pico W eBUS adapter build plan

## About This Code

Almost all of this code is AI/LLM-generated. It's best used as a source of
inspiration for your own AI/LLM efforts rather than as a traditional library.

**This is personal alpha software.** All my GitHub projects should be considered
experimental. If you want to use them:

- **Pin to a specific commit** — don't track `main`, it changes without warning
- **Use AI/LLM to adapt** — without AI assistance, these projects are hard to use
- **Treat as inspiration** — build your own version rather than depending on mine

**Suggestions welcome** — If you have ideas for improvements or changes, I'd be
delighted to read them and use them as inspiration for my own efforts.

**Why not a library?** These days it's often quicker to use AI/LLM to build your
own than to integrate traditional libraries. My use of AI/LLM is inspired by
these people and posts:

- [Simon Willison's Weblog](https://simonwillison.net/) — Essential reading on
  LLMs, prompt engineering, and building with AI
- [CLI over MCP](https://lucumr.pocoo.org/2025/8/18/code-mcps/) — Armin Ronacher
  on why command-line tools are better integration points than custom protocols
- [Build It Yourself](https://lucumr.pocoo.org/2025/12/22/a-year-of-vibes/) —
  Armin Ronacher: "With our newfound power from agentic coding tools, you can
  build much of this yourself..."
- [Shipping at Inference Speed](https://steipete.me/posts/2025/shipping-at-inference-speed) —
  Peter Steinberger on the new workflow of building with AI assistance
- [Year in Review 2025](https://mariozechner.at/posts/2025-12-22-year-in-review-2025/) —
  Mario Zechner on AI-assisted development

**What I use:** Currently Anthropic's Claude Opus, evaluating OpenAI's GPT Codex
as an alternative.

## License

This project is dual-licensed under the terms of both the MIT license and the
Apache License (Version 2.0).

See [LICENSE-APACHE](LICENSE-APACHE) and [LICENSE-MIT](LICENSE-MIT) for details.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this project by you, as defined in the Apache-2.0 license,
shall be dual licensed as above, without any additional terms or conditions.
