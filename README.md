# heatpump-analysis

Analyse heat pump performance from [emoncms.org](https://emoncms.org) monitoring data. Downloads data to a local SQLite database, classifies operating states (heating, DHW, defrost), and produces COP breakdowns, temperature correlations, and energy summaries.

Built for a **Vaillant Arotherm Plus 5kW** with emonHP monitoring bundle.

## Quick Start

```bash
# Build
cargo build --release

# Download all data (first time — takes a few minutes)
cargo run -- --apikey YOUR_EMONCMS_READ_KEY sync

# Analyse last 30 days
cargo run -- --days 30 summary

# Run everything
cargo run -- --days 365 all
```

Set `EMONCMS_APIKEY` environment variable to avoid passing `--apikey` each time. The API key is only needed for `feeds` and `sync` commands — all analysis runs from the local database.

## Commands

| Command | Description |
|---------|-------------|
| `feeds` | List available emoncms feeds (requires API key) |
| `sync` | Download/update data to local SQLite database |
| `db-status` | Show database contents and date range |
| `summary` | Overall stats and breakdown by operating state |
| `cop-by-temp` | COP vs outside temperature bands (heating only) |
| `hourly` | Average profile by hour of day |
| `daily` | Daily energy totals and COP from cumulative meters |
| `gaps` | Report data gaps and their fill status |
| `fill-gaps` | Fill gaps with modelled data (energy-scaled to match meters) |
| `data` | Show raw enriched data table |
| `all` | Run summary + cop-by-temp + hourly + daily |

### Options

| Flag | Default | Description |
|------|---------|-------------|
| `--days N` | 7 | How many days of history to analyse |
| `--db PATH` | `heatpump.db` | SQLite database path |
| `--include-simulated` | off | Include gap-filled data in analysis |

## How It Works

→ [docs/explanation.md](docs/explanation.md)

## Documentation

- **[docs/code-truth/](docs/code-truth/)** — Derived-from-code documentation (architecture, patterns, decisions)
- **[docs/explanation.md](docs/explanation.md)** — How the operating model works and why

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
