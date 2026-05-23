# TSDB Migration

This file keeps only the current migration state, the actions still required to complete migration, and the repo-local backlog that remains after migration is complete.

Shared platform shutdown work lives in `~/github/energy-hub/lat.md/tsdb-migration.md`.

## Current state

The repo-local PostgreSQL cutover is complete, and the shared platform shutdown is complete too.

`heatpump-analysis` now runs PostgreSQL-first on its migrated paths, the live `adaptive-heating-mvp` path reads and writes through TimescaleDB/PostgreSQL, and the shared Phase 5 shutdown on `pi5data` was completed on 2026-04-23 by `energy-hub`.

Current accepted PostgreSQL behaviour:
- controller decision rows keep whole-second precision; the shared rationale lives in `~/github/energy-hub/lat.md/infrastructure.md#Timestamp semantics and required precision`
- history and DHW outputs use PostgreSQL/TimescaleDB bucket and table semantics directly
- `ebusd_poll_text` remains the correct sibling table for text values rather than folding text back into `ebusd_poll`

## Actions required to complete migration

No migration-critical actions remain for this repo.

The shared Phase 5 shutdown is complete; use `~/github/energy-hub/lat.md/tsdb-migration.md` and `~/github/energy-hub/docs/timescaledb-cutover-runbook.md` only as the completion record.

## Post-migration backlog

These items remain real work, but they are not TSDB migration work.

1. **Finish the Pico eBUS active-sending gap**
   - `docs/pico-ebus-plan.md` still stops at passive observation; finish arbitration / active-send so the Pico path can replace the current write-capable eBUS stack.
2. **Keep the timestamp-precision policy explicit**
   - Whole seconds are the honest default for these cadences unless a series has a proven sub-second event-time contract. Keep that rationale aligned with the shared `energy-hub` note rather than reintroducing migration-era ambiguity.
