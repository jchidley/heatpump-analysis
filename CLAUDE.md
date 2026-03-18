# CLAUDE.md

## What This Is

CLI tool that syncs heat pump data from emoncms.org to local SQLite, then analyses it with Polars. Vaillant Arotherm Plus 5kW.

## Commands

| Task | Command |
|------|---------|
| Build | `cargo build` |
| Run | `cargo run -- <subcommand>` |
| Sync data | `cargo run -- --apikey KEY sync` |
| Analyse (7 days) | `cargo run -- --days 7 summary` |
| Analyse (all data) | `cargo run -- --days 500 all` |
| With simulated | `cargo run -- --days 500 --include-simulated summary` |
| Gap report | `cargo run -- gaps` |
| Fill gaps | `cargo run -- fill-gaps` |

`--apikey` only needed for `feeds` and `sync`. Analysis reads from `heatpump.db`.

## Architecture

```
emoncms.rs  → API client (used only by sync)
db.rs       → SQLite storage + DataFrame loading (source of truth for feed IDs)
analysis.rs → State machine + Polars queries (no DB/API dependency)
gaps.rs     → Gap detection + synthetic data (accesses SQLite directly)
main.rs     → CLI routing
```

## Key Domain Model

Operating states classified by flow rate (Arotherm 5kW fixed pump = 14.3 L/min):
- **Heating**: flow_rate 14.0–14.5, DT > 0, heat > 0
- **DHW**: flow_rate ≥ 16.0 (enter) / < 15.0 (exit), DT > 0, heat > 0
- **Defrost**: heat ≤ 0 OR DT < −0.5 (any flow rate)
- **Idle**: elec ≤ 50W

Thresholds: `analysis.rs` top-of-file constants. Also hardcoded in `gaps.rs` (`flow_rate >= 16.0`).

## Gotchas

- Feed IDs hardcoded in both `db.rs` and `gaps.rs` — keep in sync
- `gaps.rs` bypasses `db.rs` — writes to SQLite tables directly
- No tests — validate changes against real data output
- Simulated data in separate table (`simulated_samples`) — never mixed unless `--include-simulated`
- DB schema is `CREATE TABLE IF NOT EXISTS` — no migrations

## Boundaries

- Don't change operating state thresholds without re-validating against the full dataset
- Don't mix simulated and real data by default
- Don't commit `heatpump.db` or API keys
