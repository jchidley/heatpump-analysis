# TSDB Migration

This file keeps only the current migration state, the actions still required to complete migration, and the repo-local backlog that remains after migration is complete.

Shared platform shutdown work lives in `~/github/energy-hub/lat.md/tsdb-migration.md`.

## Current state

The repo-local PostgreSQL cutover is complete, and the shared platform shutdown is complete too.

`heatpump-analysis` now runs PostgreSQL-first on its migrated paths, the live `adaptive-heating-mvp` path no longer needs InfluxDB to read or write, and the shared Phase 5 InfluxDB shutdown on `pi5data` was completed on 2026-04-23 by `energy-hub`.

Current accepted PostgreSQL-first behaviour that is not a migration blocker:
- controller decision rows keep whole-second precision; the shared rationale lives in `~/github/energy-hub/lat.md/infrastructure.md#Timestamp semantics and required precision`
- history and DHW outputs may differ from old Flux-era results where PostgreSQL now exposes richer or differently bucketed current-state data
- `/status.updated_at` is stale, but that is a controller observability bug rather than a migration blocker
- `ebusd_poll_text` remains the correct sibling table for text values rather than folding text back into `ebusd_poll`

## Actions required to complete migration

No migration-critical actions remain for this repo.

The shared Phase 5 shutdown is complete; use `~/github/energy-hub/lat.md/tsdb-migration.md` and `~/github/energy-hub/docs/timescaledb-cutover-runbook.md` only as the completion record.

## New work backlog once migration is done

These items remain real work, but none of them block migration completion.

1. **Fix controller `/status.updated_at` semantics**
   - Expose a genuinely current controller-state timestamp, or split it into explicit `mode_updated_at` and `last_cycle_at` fields.
2. **Remove the remaining Flux/CSV compatibility tail**
   - Delete surviving Flux-era parser/query/config paths such as `src/thermal/influx.rs`, the remaining Influx block in `model/thermal-config.toml` / `src/thermal/config.rs`, and any parity helpers that survive only for migration symmetry.
3. **Retire migration-tail test specs when the code disappears**
   - Remove with raw Flux/CSV parser removal: [[tests#InfluxDB wire-format parsing]]
   - Remove with remaining `history.rs` Flux compatibility/parity tail removal: [[tests#History filter variant routing]] and any Flux-vs-PostgreSQL parity checks such as `Controller rows match between Flux and PostgreSQL on a representative window`
   - Remove with final LP write removal for DHW session mirroring: LP-only helpers under [[tests#DHW write contracts]] such as `dhw_inflection LP line contains all required fields`, `LP tag spaces replaced with underscores`, and `dhw_capacity LP line maps to TimescaleDB columns`
4. **Finish the Pico eBUS active-sending gap**
   - `docs/pico-ebus-plan.md` still stops at passive observation; finish arbitration / active-send so the Pico path can replace the current write-capable eBUS stack.
5. **Keep the timestamp-precision policy explicit**
   - Whole seconds are the honest default for these cadences unless a series has a proven sub-second event-time contract. Keep that rationale aligned with the shared `energy-hub` note rather than reintroducing migration-era ambiguity.
