# PostgreSQL Migration

This file tracks the repo-local migration from Flux/InfluxDB to PostgreSQL/TimescaleDB for heatpump-analysis.

Shared platform, schema, live-ingest, gap-fill, and final decommission truth live in `~/github/energy-hub/lat.md/timescaledb-migration.md`. This file covers only the code, tests, deployment order, and rollback for this repo.

## Scope

This migration touches the shared Flux client plus every binary or one-shot that still reads from or writes to InfluxDB.

### Primary modules (direct Flux/LP usage)

Modules that directly call InfluxDB HTTP APIs or build Flux queries.

| Module | Current role | R/W | Migration work |
|---|---|---|---|
| `src/thermal/influx.rs` | Shared Flux read client (7 query functions + CSV parser) | Read | Replace Flux/HTTP with PostgreSQL queries; preserve return contracts |
| `src/bin/adaptive-heating-mvp.rs` | Live controller daemon | R+W | Replace Flux reads, LP writes, and inline CSV parser; preserve HTTP API and control decisions |
| `src/thermal/dhw_sessions.rs` | One-shot DHW analysis + writer | R+W | Replace Flux reads (10s/2s resolution) and LP writes for `dhw_inflection` / `dhw_capacity` |
| `src/thermal/history.rs` | One-shot history evidence queries (15 call sites) | Read | Replace Flux queries with PostgreSQL; preserve review outputs and `HashMap<String,String>` intermediate format |

### Consumer modules (indirect — call influx.rs functions)

These modules call `query_room_temps`, `query_outside_temp`, `query_mwt`, etc. They are affected only if the influx.rs public API changes shape.

| Module | Functions called | Resolution |
|---|---|---|
| `src/thermal/operational.rs` | room_temps, outside_temp, bcf, mwt, flux_csv_pub | 5m/1m aggregate |
| `src/thermal/validation.rs` | room_temps, outside_temp | 5m aggregate |
| `src/thermal/diagnostics.rs` | room_temps, outside_temp, status_codes | 1m–5m aggregate |
| `src/thermal/calibration.rs` | room_temps, outside_temp | 5m aggregate |
| `src/thermal/display.rs` | room_temps, outside_temp, mwt, flux_csv_pub | 5m aggregate |

### Config module

`src/thermal/config.rs` defines `InfluxCfg` (url, org, bucket, token_env). The migration replaces this with a PostgreSQL connection config (conninfo or host/port/dbname/user/password).

## Shared dependency on the energy-hub migration

This repo can only cut over after the shared TimescaleDB platform phases provide the expected data shape and continuity.

Required shared gates before repo cutover:

1. TimescaleDB schema exists on `pi5data` and matches the shared migration document.
2. Historical import is complete so every local query can be tested against full history.
3. Live ingest is running for MQTT-sourced measurements used by this repo.
4. Gap-fill is complete and verified so no controller or evidence query spans a blind boundary.
5. Final v2 decommission remains blocked until this repo's local migration gate is green.

**Hard dependency:** this repo does not cut over before shared platform Phases 3 and 3b are complete and verified.

**Recommended order:** cut over this repo **after** `energy-hub` and `z2m-hub`. The reason is operational risk: `adaptive-heating-mvp` directly controls heating, so this repo should be the final consumer migration and should still deploy its live daemon last inside the repo.

## Transport rewrite plan

The rewrite should replace transport and row-shaping first, while leaving control policy and evidence logic intact.

### Shared read path

`src/thermal/influx.rs` should become the single PostgreSQL read seam for code that currently depends on Flux.

Preferred shape:
- one small connection/config layer for PostgreSQL
- query helpers that return already-decoded Rust structures, not raw rows
- measurement-specific helpers for wide tables such as `ct_monitor`, `tesla`, `heatpump`, `multical`, `zigbee`, and `adaptive_heating_mvp`
- no controller policy, tariff policy, or evidence heuristics in the transport layer

Preserve existing return contracts to avoid cascading changes across 5 consumer modules:
- `query_room_temps` → `Vec<(DateTime<FixedOffset>, String, f64)>` sorted by time
- `query_outside_temp` / `query_pv_power` / `query_building_circuit_flow` → `Vec<(DateTime<FixedOffset>, f64)>` sorted by time
- `query_status_codes` → `Vec<(DateTime<FixedOffset>, i32)>` sorted by time (float→round→int)
- `query_mwt` → `Vec<(DateTime<FixedOffset>, f64)>` (average of FlowTemp + ReturnTemp)
- `query_flux_csv_pub` → `Vec<HashMap<String, String>>` — consider preserving this intermediate format for history.rs compatibility, via a thin `pg_rows_to_hashmaps` adapter

### Topic-to-table routing

InfluxDB stores all data in one measurement with a `topic` tag. TimescaleDB uses separate wide tables. Every query function must route to the correct table and column.

| InfluxDB topic prefix | TimescaleDB table | Source/device column | Field routing |
|---|---|---|---|
| `zigbee2mqtt/*` | `zigbee` | `device` | `_field = "temperature"` → `temperature` column |
| `emon/EmonPi2/*` | `ct_monitor` | `source = 'EmonPi2'` | `P3` → `"P3"` column (quoted) |
| `emon/emonpi2_cu/*` | `ct_monitor` | `source = 'emonpi2_cu'` | same as above |
| `emon/emontx5_cu/*` | `ct_monitor` | `source = 'emontx5_cu'` | same as above |
| `emon/tesla/*` | `tesla` | — | field name → column name |
| `emon/heatpump/*` | `heatpump` | — | field name → column name |
| `emon/multical/*` | `multical` | — | field name → column name |
| `emon/emonth2_23/*` | `emonth` | — | `_field = "value"` → column name |
| `emon/sensors/*` | `sensors` | — | field name → column name |
| `emon/metoffice/*` | `metoffice` | — | `outside_temperature` column |
| `ebusd/poll/*` | `ebusd_poll` | `field` column | `_field = "value"` → `value` column |

The `_field` distinction is critical: Zigbee sensors use `_field = "temperature"`, while emonth2_23 and ebusd/poll sensors use `_field = "value"`. The `query_room_temps` function encodes this logic. In PostgreSQL, this distinction disappears — each table has named columns.

### Resolution contracts

Different consumers query at different resolutions. The SQL migration must match these.

| Consumer | Flux aggregation | SQL equivalent |
|---|---|---|
| Thermal model (operational, calibration, validation, display) | `aggregateWindow(every: 5m, fn: mean)` | `time_bucket('5 minutes', time)` + `avg()` |
| Status codes (diagnostics) | `aggregateWindow(every: 1m, fn: last)` | `time_bucket('1 minute', time)` + `DISTINCT ON` or `last()` |
| Building circuit flow | `aggregateWindow(every: 1m, fn: mean)` | `time_bucket('1 minute', time)` + `avg()` |
| DHW event detection (dhw_sessions) | `aggregateWindow(every: 10s, fn: max/last)` | `time_bucket('10 seconds', time)` + `max()`/`last()` |
| DHW inflection (dhw_sessions) | Raw 2s resolution (no aggregation) | `SELECT ... WHERE time BETWEEN` at raw resolution |
| Live control (adaptive-heating-mvp) | `last()` over `-2h` lookback | `ORDER BY time DESC LIMIT 1` with `WHERE time > now() - interval '2 hours'` |

Note: Flux `aggregateWindow(createEmpty: false)` skips time buckets with no data. The SQL equivalent must also omit empty buckets (standard `GROUP BY` + `HAVING` or inner join naturally does this).

### Direct writers

This repo has direct write paths that must move from line protocol to SQL `INSERT` statements.

Writers to migrate:
- `adaptive-heating-mvp` → `adaptive_heating_mvp`
- `dhw_sessions.rs` → `dhw_inflection`, `dhw_capacity`

The migration should preserve the existing semantic row shape and only change transport. If `ON CONFLICT` handling is needed for replay safety or repeated runs, keep it in the write helper rather than spreading SQL policy into the analysis logic.

### Controller deploy order

`adaptive-heating-mvp` deploys last because it is the only migrated component here that directly influences live heating behaviour.

Recommended order:
1. migrate and test `src/thermal/influx.rs`
2. migrate and test one-shot readers (`history.rs`)
3. migrate and test one-shot writers (`dhw_sessions.rs`)
4. migrate and test `adaptive-heating-mvp` in dry/status/read-only paths first
5. deploy the live controller only after parity and rollback checks are ready

## Regression gates

This repo is not migrated just because the code compiles against `tokio-postgres`.

### Existing migration regression tests (43 tests)

Pre-migration contract tests already in the codebase, covering the output shapes and field mappings that must survive the rewrite:

| Test group | File | Count | What they pin |
|---|---|---|---|
| CSV parser contracts | `influx.rs` | 6 | `parse_influx_annotated_csv` output shape: annotations skipped, multi-table, duplicate headers, empty keys |
| Timestamp parsing | `influx.rs` | 2 | `parse_dt` RFC3339 acceptance and rejection boundaries |
| Query return contracts | `influx.rs` | 8 | Typed output of each query function: room temps triples, outside/MWT/PV/BCF pairs, status code rounding, single-value/empty-result, multi-topic conditions, wide-row NULLs |
| Topic→table routing | `influx.rs` | 3 | Topic prefix→table mapping, field name routing (value vs temperature), PV column decomposition |
| Timestamp migration | `influx.rs` | 2 | Microsecond truncation safety, TIMESTAMPTZ format incompatibility flag |
| DHW write contracts | `dhw_sessions.rs` | 7 | LP field coverage, naive PG timestamps, 10s resolution, tag escaping, measurement→table routing, triple-field filter |
| Controller write contracts | `adaptive-heating-mvp.rs` | 8 | LP field coverage (all 24 fields), None omission, boolean encoding, inline CSV parser, empty result, tag escaping, field routing, DHW T1 field |
| History filter variants | `history.rs` | 3 | Topic vs measurement vs plain-measurement filter patterns and their PG table routing |
| Display migration | `display.rs` | 2 | Humidity topic skipping, humidity field name contract |
| Existing influx_field test | `adaptive-heating-mvp.rs` | 1 | influx_field Some/None formatting |
| Existing history helpers | `history.rs` | 6 | `summaries_from_batch_rows`, `numeric_values_from_batch_rows`, `string_values_from_batch_rows` — survive if `HashMap<String,String>` intermediate preserved |

See [[tests#InfluxDB wire-format parsing]], [[tests#Query return contracts]], [[tests#Topic to table routing]], [[tests#Timestamp migration contracts]], [[tests#DHW write contracts]], [[tests#Adaptive heating write contracts]], [[tests#History filter variant routing]], [[tests#Display migration contracts]] for full specs.

### Required parity layers (still needed during migration)

These layers require real PostgreSQL fixtures and cannot be fully tested until the migration is underway.

1. **Query parity for shared readers** — old Flux path vs PostgreSQL path on the same frozen historical windows, compared on decoded domain results rather than raw SQL/CSV formatting.
2. **History evidence parity** — `history.rs` and any review helpers must preserve the same user-visible summary fields, counts, and verdict inputs for representative historical windows.
3. **DHW sessions parity** — `dhw_sessions.rs` must preserve session boundaries, inflection/capacity calculations, and emitted rows for fixed fixtures.
4. **Controller decision parity** — for fixed input windows or captured fixtures, `adaptive-heating-mvp` must preserve the same mode decisions, overnight/DHW gating outcomes, and key logged fields within documented tolerances.
5. **Direct-writer equivalence** — SQL rows written to `adaptive_heating_mvp`, `dhw_inflection`, and `dhw_capacity` must be semantically equivalent to the prior line-protocol writes.
6. **Real PostgreSQL integration** — at least one test layer must exercise actual PostgreSQL queries/inserts against small seeded fixtures so quoted identifiers, timestamps, and conflict behaviour are not only mock-tested.

## Known migration hazards

Concrete issues discovered during the regression test audit that will break the migration if not addressed.

### Timestamp format incompatibility

`parse_dt` requires RFC3339; PostgreSQL TIMESTAMPTZ uses `2026-01-15 10:30:00+00` by default. Must adapt.

Options: configure the PG client to return RFC3339, or replace `parse_dt` with native `chrono::DateTime` extraction from `tokio-postgres` Row types. `parse_ts_val` in `dhw_sessions.rs` has a NaiveDateTime fallback but only with `T` separator.

### Inline CSV parser in adaptive-heating-mvp

`query_single_value` in the live controller has its own CSV parser (line-splitting, not the `csv` crate). This is separate from `influx.rs::parse_influx_annotated_csv`. The migration must replace both parsers.

### Boolean field encoding

`battery_adequate_to_next_cosy` is encoded as integer `1`/`0` in LP. The TimescaleDB schema defines this column as `FLOAT8`, not `BOOLEAN`. The PostgreSQL INSERT must write `1.0`/`0.0` or the schema should be changed to `BOOLEAN`.

### Tag-to-column conversion

LP tags (`mode`, `action`, `tariff`) become `TEXT` columns in the wide table.

LP tag values must not contain spaces — currently enforced by `.replace(' ', "_")`. SQL INSERT has no such restriction, but existing InfluxDB data has underscored values. The PG consumer should match or translate.

### Wide-row NULLs

`ct_monitor` has P7–P12 NULL for 6-channel devices. PV queries reading P3 must handle NULL siblings.

In Flux this was invisible (narrow model). In the wide PostgreSQL table, `SELECT "P3", "P7"` returns NULL for P7.

### Missing tag columns in shared schema

The energy-hub TimescaleDB schema is missing columns for LP tags that this repo writes.

`dhw_inflection` writes `category` (capacity/partial/lower_bound), `crossover` (bool), and `draw_type` (bath/shower/tap) as LP tags. The shared schema has no corresponding columns — these are silently lost on INSERT. The schema needs `category TEXT`, `crossover BOOLEAN`, `draw_type TEXT` columns added, or the writer must drop these values.

`adaptive_heating_mvp` writes `mode`, `action`, `tariff` as LP tags. The shared schema has no corresponding columns. The schema needs `mode TEXT`, `action TEXT`, `tariff TEXT` columns added, or the writer must drop these values. These carry operational meaning for review and debugging.

This must be resolved in the shared `energy-hub/lat.md/timescaledb-migration.md` schema before this repo's writer migration proceeds.

### MWT query complexity

`query_mwt` builds a complex Flux: two queries, union, pivot, map to average. SQL is simpler but must match.

In PostgreSQL: `SELECT time, avg(value) FROM ebusd_poll WHERE field IN ('FlowTemp','ReturnTemp') GROUP BY time` or a self-join. Verify equivalence on overlapping windows.

## Deployment and rollback

Deployment must keep the house safe even if the PostgreSQL rewrite is wrong.

Deployment rule:
- build and test on dev first
- build the release binary on `pi5data`
- deploy one-shot tools before the live controller
- deploy `adaptive-heating-mvp` last and monitor journal + API status immediately after restart

Rollback rule:
- keep the prior controller binary/config available on `pi5data`
- if PostgreSQL-backed behaviour is wrong, revert this repo to the last known InfluxDB-backed release while the shared platform remains side-by-side
- do not treat a repo-local rollback as a platform rollback; shared TimescaleDB ingest and data import can remain in place

## Done gate

This repo is only migration-complete when its local rewrite and verification are finished, independent of the other repos.

Checklist:
- [ ] shared schema updated with missing tag columns (dhw_inflection: category/crossover/draw_type; adaptive_heating_mvp: mode/action/tariff)
- [ ] `src/thermal/influx.rs` rewritten for PostgreSQL
- [ ] one-shot readers verified against PostgreSQL on representative history windows
- [ ] one-shot writers verified for row-equivalent SQL output
- [ ] `adaptive-heating-mvp` read/write path verified against PostgreSQL
- [ ] local regression/parity suite green
- [ ] `energy-hub` and `z2m-hub` migrations verified before this repo's final cutover
- [ ] live controller deploy verified on `pi5data`
- [ ] rollback path tested or rehearsed
- [ ] shared `energy-hub` migration status updated to note this repo's state
