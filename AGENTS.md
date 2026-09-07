# AGENTS.md

## What This Is

Rust CLI + Python thermal model for heat pump analysis. Vaillant Arotherm Plus 5kW at 6 Rhodes Avenue, London N22 7UT.

- EmonCMS read access: retrieve the key at runtime with `ak get emoncms-read`.

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


`--apikey` only needed for `feeds` and `sync`. `overnight` additionally needs Octopus account access via `OCTOPUS_API_KEY` + `OCTOPUS_ACCOUNT_NUMBER` or `~/git/octopus/.envrc`, because tariff rates are fetched from the account API at runtime via the shared `octopus-tariff` crate (`~/git/octopus-tariff`). Two binaries: use `cargo run --bin heatpump-analysis` for thermal commands. Three binaries total: `adaptive-heating-mvp` is the live pilot controller. Dev on laptop (`cargo check`), release build natively on pi5data (cross-compile fails due to glibc mismatch).

## Where To Read Next

Use `lat.md/` for current project truth and `lat.md/src/` plus the source tree for code location maps.

- `lat.md/constraints.md` — boundaries, gotchas, eBUS timer rules, duplicated values
- `lat.md/domain.md` — operating states, house model, DHW cylinder, tariff, feeds
- `lat.md/heating-control.md` — adaptive controller, overnight logic, modes, pilot history
- `lat.md/infrastructure.md` — hosts, MQTT, eBUS stack, room sensors, baseline VRC 700 settings
- `lat.md/architecture.md` — binaries, data flow, config split, implicit contracts
- `lat.md/history-evidence.md` — default review window and history-review boundaries
- `lat.md/tsdb-migration.md` — sole repo-local TSDB migration tracker; shared platform truth stays in `~/git/energy-hub/lat.md/tsdb-migration.md`
- `lat.md/src/` — file-level source pages when a source file has dedicated documentation

## Fast reminders

- Thermal/history commands: `cargo run --bin heatpump-analysis -- ...`
- Adaptive controller API: `http://pi5data:3031` (phone proxy `http://pi5data:3030`)
- Infrastructure rebuild/recovery: `heating-monitoring-setup.md`, `docs/emon-installation-runbook.md`
- Secrets and tokens: `deploy/SECRETS.md`
- **PostgreSQL queries from dev machine**: use `TIMESCALEDB_CONNINFO` with `psql`. See `lat.md/infrastructure.md#Ad-hoc PostgreSQL Queries from Dev Machine`.
- **PostgreSQL-first analysis**: push all filtering, aggregation, windowing, and arithmetic into SQL/TimescaleDB queries. Client-side code (Python/shell) is for final formatting only. See `lat.md/constraints.md#PostgreSQL-First Analysis`.
- **TSDB verification**: use PostgreSQL/TimescaleDB for thermal/history/controller verification.

For operational facts, gotchas, and hard boundaries, prefer `lat.md/` instead of duplicating them here. Read the relevant sections when documented intent matters; update them when an in-scope change alters the documented architecture, behaviour, constraints, or tests, then run `lat check`.

Sync, deploy, service-restart, and settings-changing commands affect remote systems and require explicit approval for the concrete operation. Read-only status, logs, and data retrieval do not authorize a later write.
