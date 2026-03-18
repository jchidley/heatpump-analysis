# CLAUDE.md

## What This Is

CLI tool that syncs heat pump data from emoncms.org to local SQLite, then analyses it with Polars. Vaillant Arotherm Plus 5kW.

## Data Access

- emoncms dashboard: `https://emoncms.org/app/view?name=MyHeatpump&readkey=1b00410c57d5df343ede7c09e6aab34f`
- Read API key (read-only, safe to share): `1b00410c57d5df343ede7c09e6aab34f`

## Commands

| Task | Command |
|------|---------|
| Build | `cargo build` |
| Run | `cargo run -- <subcommand>` |
| Sync data | `cargo run -- --apikey KEY sync` |
| Analyse (7 days) | `cargo run -- --days 7 summary` |
| Analyse (date range) | `cargo run -- --from 2025-01-01 --to 2025-01-31 summary` |
| Analyse (all data) | `cargo run -- --days 500 all` |
| With simulated | `cargo run -- --days 500 --include-simulated summary` |
| Export CSV | `cargo run -- --days 30 export -o output.csv` |
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
- Sync start date hardcoded as `DEFAULT_SYNC_START_MS` in `db.rs` (2024-10-22)
- Polars pinned to 0.46 (0.53 available) — untested on newer versions
- Outside temp feed (Met Office) is lower resolution (~hourly) than HP feeds (~10s)
- Thresholds are 5kW-specific — 7kW model would need different values (its heating rate = 20 L/min overlaps 5kW DHW rate)

## Feed Notes

- `503101` (indoor_temp) = emonth2 sensor in **Leather room only**, not whole-house
- `503093` (outside_temp) = Met Office hourly, not Arotherm OAT sensor
- `512889` (DHW_flag) = dead since Dec 2024
- Solar PV + battery system installed (not yet integrated):
    - 7× Trina 440W panels (TSM-440NEG9RC.27), 3.08 kWp, single string
    - Fox ESS F3600 inverter (3.6kW, dual MPPT — one MPPT input used)
    - Tesla Powerwall 2 (13.5 kWh) + Gateway
    - Commissioned: 19/04/2024, Emlite M24 generation meter

## Planned Enhancements

See [docs/roadmap.md](docs/roadmap.md) for full details:
- **eBUS** — adapter is physically connected but not configured. Would give real-time OAT, compressor speed, defrost status, cylinder temp
- **Octopus Energy** — API integration for electricity cost analysis, cost-weighted COP
- **Solar PV + battery** — system installed, details to be provided. Self-consumption analysis, DHW scheduling to solar peak
- Other data in `C:\Users\jackc\OneDrive\Documents\House\`: degree day CSVs (EGWU), utility bills, Octopus Agile rates, weekly consumption

## Boundaries

- Don't change operating state thresholds without re-validating against the full dataset
- Don't mix simulated and real data by default
- Don't commit `heatpump.db` or API keys
