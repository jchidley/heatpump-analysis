# Before starting work

- Run `lat search` to find sections relevant to your task. Read them to understand the design intent before writing code.
- Run `lat expand` on user prompts to expand any `[[refs]]` — this resolves section names to file locations and provides context.

# Post-task checklist (REQUIRED — do not skip)

After EVERY task, before responding to the user:

- [ ] Update `lat.md/` if you added or changed any functionality, architecture, tests, or behavior
- [ ] Run `lat check` — all wiki links and code refs must pass
- [ ] Do not skip these steps. Do not consider your task done until both are complete.

---

# What is lat.md?

This project uses [lat.md](https://www.npmjs.com/package/lat.md) to maintain a structured knowledge graph of its architecture, design decisions, and test specs in the `lat.md/` directory. It is a set of cross-linked markdown files that describe **what** this project does and **why** — the domain concepts, key design decisions, business logic, and test specifications. Use it to ground your work in the actual architecture rather than guessing.

# Commands

```bash
lat locate "Section Name"      # find a section by name (exact, fuzzy)
lat refs "file#Section"        # find what references a section
lat search "natural language"  # semantic search across all sections
lat expand "user prompt text"  # expand [[refs]] to resolved locations
lat check                      # validate all links and code refs
```

Run `lat --help` when in doubt about available commands or options.

If `lat search` fails because no API key is configured, explain to the user that semantic search requires a key provided via `LAT_LLM_KEY` (direct value), `LAT_LLM_KEY_FILE` (path to key file), or `LAT_LLM_KEY_HELPER` (command that prints the key). Supported key prefixes: `sk-...` (OpenAI) or `vck_...` (Vercel). If the user doesn't want to set it up, use `lat locate` for direct lookups instead.

# Syntax primer

- **Section ids**: `lat.md/path/to/file#Heading#SubHeading` — full form uses project-root-relative path (e.g. `lat.md/tests/search#RAG Replay Tests`). Short form uses bare file name when unique (e.g. `search#RAG Replay Tests`, `cli#search#Indexing`).
- **Wiki links**: `[[target]]` or `[[target|alias]]` — cross-references between sections. Can also reference source code: `[[src/foo.ts#myFunction]]`.
- **Source code links**: Wiki links in `lat.md/` files can reference functions, classes, constants, and methods in TypeScript/JavaScript/Python/Rust/Go/C files. Use the full path: `[[src/config.ts#getConfigDir]]`, `[[src/server.ts#App#listen]]` (class method), `[[lib/utils.py#parse_args]]`, `[[src/lib.rs#Greeter#greet]]` (Rust impl method), `[[src/app.go#Greeter#Greet]]` (Go method), `[[src/app.h#Greeter]]` (C struct). `lat check` validates these exist.
- **Code refs**: `// @lat: [[section-id]]` (JS/TS/Rust/Go/C) or `# @lat: [[section-id]]` (Python) — ties source code to concepts

# Test specs

Key tests can be described as sections in `lat.md/` files (e.g. `tests.md`). Add frontmatter to require that every leaf section is referenced by a `// @lat:` or `# @lat:` comment in test code:

```markdown
---
lat:
  require-code-mention: true
---
# Tests

Authentication and authorization test specifications.

## User login

Verify credential validation and error handling for the login endpoint.

### Rejects expired tokens
Tokens past their expiry timestamp are rejected with 401, even if otherwise valid.

### Handles missing password
Login request without a password field returns 400 with a descriptive error.
```

Every section MUST have a description — at least one sentence explaining what the test verifies and why. Empty sections with just a heading are not acceptable. (This is a specific case of the general leading paragraph rule below.)

Each test in code should reference its spec with exactly one comment placed next to the relevant test — not at the top of the file:

```python
# @lat: [[tests#User login#Rejects expired tokens]]
def test_rejects_expired_tokens():
    ...

# @lat: [[tests#User login#Handles missing password]]
def test_handles_missing_password():
    ...
```

Do not duplicate refs. One `@lat:` comment per spec section, placed at the test that covers it. `lat check` will flag any spec section not covered by a code reference, and any code reference pointing to a nonexistent section.

# Section structure

Every section in `lat.md/` **must** have a leading paragraph — at least one sentence immediately after the heading, before any child headings or other block content. The first paragraph must be ≤250 characters (excluding `[[wiki link]]` content). This paragraph serves as the section's overview and is used in search results, command output, and RAG context — keeping it concise guarantees the section's essence is always captured.

```markdown
# Good Section

Brief overview of what this section documents and why it matters.

More detail can go in subsequent paragraphs, code blocks, or lists.

## Child heading

Details about this child topic.
```

```markdown
# Bad Section

## Child heading

Details about this child topic.
```

The second example is invalid because `Bad Section` has no leading paragraph. `lat check` validates this rule and reports errors for missing or overly long leading paragraphs.
# AGENTS.md

## What This Is

Rust CLI + Python thermal model for heat pump analysis. Vaillant Arotherm Plus 5kW at 6 Rhodes Avenue, London N22 7UT.

- emoncms dashboard: `https://emoncms.org/app/view?name=MyHeatpump&readkey=1b00410c57d5df343ede7c09e6aab34f`
- Read API key (read-only): `1b00410c57d5df343ede7c09e6aab34f`

`AGENTS.md` is now intentionally compact. The canonical current-state architecture, domain rules, infrastructure inventory, controller behaviour, and gotchas live in `lat.md/`.

## Commands

| Task | Command |
|------|---------|
| Build | `cargo build` |
| Sync data | `cargo run -- --apikey KEY sync` |
| Analyse (7 days) | `cargo run -- --days 7 summary` |
| Analyse (all data) | `cargo run -- --all-data all` |
| Octopus summary | `cargo run -- octopus` |
| Gas vs HP | `cargo run -- --all-data gas-vs-hp` |
| Overnight optimizer | `cargo run -- --all-data overnight` |
| Thermal rooms | `cargo run --bin heatpump-analysis -- thermal-rooms` |
| Thermal connections | `cargo run --bin heatpump-analysis -- thermal-connections` |
| Thermal analyse | `cargo run --bin heatpump-analysis -- thermal-analyse --config model/thermal-config.toml` |
| Thermal equilibrium | `cargo run --bin heatpump-analysis -- thermal-equilibrium --outside 0 --mwt 40` |
| Thermal moisture | `cargo run --bin heatpump-analysis -- thermal-moisture --config model/thermal-config.toml` |
| Thermal calibrate | `cargo run --bin heatpump-analysis -- thermal-calibrate --config model/thermal-config.toml` |
| Thermal validate | `cargo run --bin heatpump-analysis -- thermal-validate --config model/thermal-config.toml` |
| Thermal fit diagnostics | `cargo run --bin heatpump-analysis -- thermal-fit-diagnostics --config model/thermal-config.toml` |
| Thermal operational | `cargo run --bin heatpump-analysis -- thermal-operational --config model/thermal-config.toml` |
| Thermal snapshot | `cargo run --bin heatpump-analysis -- thermal-snapshot export --config model/thermal-config.toml --signoff-reason "reason" --approved-by-human` |
| Thermal control table | `cargo run --bin heatpump-analysis -- thermal-control-table --config model/thermal-config.toml` |
| Regression check | `bash scripts/thermal-regression-ci.sh` |
| Adaptive heating MVP | `cargo run --bin adaptive-heating-mvp -- --config model/adaptive-heating-mvp.toml run` |
| Adaptive heating status | `cargo run --bin adaptive-heating-mvp -- --config model/adaptive-heating-mvp.toml status` |
| Adaptive heating restore | `cargo run --bin adaptive-heating-mvp -- --config model/adaptive-heating-mvp.toml restore-baseline` |
| DHW sessions | `cargo run --bin heatpump-analysis -- dhw-sessions --days 7` |
| DHW sessions (verbose) | `cargo run --bin heatpump-analysis -- dhw-sessions --days 12 --no-write` |
| DHW sessions (JSON) | `cargo run --bin heatpump-analysis -- dhw-sessions --days 7 --format json` |
| Sync sources to pi5data | `bash scripts/sync-to-pi5data.sh` |
| Build on pi5data | `ssh pi5data 'cd ~/adaptive-heating-mvp && . ~/.cargo/env && cargo build --release'` |
| Deploy (restart service) | `ssh pi5data 'sudo systemctl restart adaptive-heating-mvp'` |
| Controller logs | `ssh pi5data 'journalctl -u adaptive-heating-mvp --since "1 hour ago" --no-pager'` |


`--apikey` only needed for `feeds` and `sync`. `overnight` additionally needs Octopus account access via `OCTOPUS_API_KEY` + `OCTOPUS_ACCOUNT_NUMBER` or `~/github/octopus/.envrc`, because tariff rates are now fetched from the account API at runtime. Two binaries: use `cargo run --bin heatpump-analysis` for thermal commands. Three binaries total: `adaptive-heating-mvp` is the live pilot controller. Dev on laptop (`cargo check`), release build natively on pi5data (cross-compile fails due to glibc mismatch).

## Where To Read Next

Use `lat.md/` for current project truth and `docs/code-truth/` for code location maps.

- `lat.md/constraints.md` — boundaries, gotchas, eBUS timer rules, duplicated values
- `lat.md/domain.md` — operating states, house model, DHW cylinder, tariff, feeds
- `lat.md/heating-control.md` — adaptive controller, overnight logic, modes, pilot history
- `lat.md/infrastructure.md` — hosts, MQTT, eBUS stack, room sensors, baseline VRC 700 settings
- `lat.md/architecture.md` — binaries, data flow, config split, implicit contracts
- `lat.md/history-evidence.md` — default review window and history-review boundaries
- `docs/code-truth/` — file map, architecture, patterns, decisions

## Fast reminders

- Thermal/history commands: `cargo run --bin heatpump-analysis -- ...`
- Adaptive controller API: `http://pi5data:3031` (phone proxy `http://pi5data:3030`)
- Infrastructure rebuild/recovery: `heating-monitoring-setup.md`, `docs/emon-installation-runbook.md`
- Secrets and tokens: `deploy/SECRETS.md`
- **InfluxDB queries from dev machine**: `INFLUX_TOKEN=$(ak get influxdb)` then `curl` to `http://pi5data:8086/api/v2/query?org=home` with Flux. See `lat.md/infrastructure.md#Ad-hoc InfluxDB Queries from Dev Machine`.
- **InfluxDB-first analysis**: push all filtering, aggregation, pivoting, and arithmetic into Flux queries. Client-side code (Python/shell) is for final formatting only. See `lat.md/constraints.md#InfluxDB-First Analysis`.

For operational facts, gotchas, and hard boundaries, prefer `lat.md/` instead of duplicating them here.
