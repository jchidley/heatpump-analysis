# TSDB Migration

This file tracks only the outstanding repo-local work to finish the PostgreSQL migration for heatpump-analysis.

Durable transport, deployment, secret-boundary, and test-contract truth now lives in the normal thematic `lat.md/` files such as [[architecture]], [[infrastructure]], [[constraints]], and [[tests]]. Shared platform, schema, live-ingest, gap-fill, and final decommission truth still live in `~/github/energy-hub/lat.md/tsdb-migration.md`.

## Migration Snapshot

This snapshot is the shortest current-state handoff for the remaining migration work.

- **Now:** shared typed readers are mostly staged onto PostgreSQL; `display.rs`, `history.rs`, and `dhw_sessions.rs` use PostgreSQL on their migrated paths when `[postgres]` is configured; and `adaptive-heating-mvp` repo code/config now requires PostgreSQL for latest-value reads while writing decision rows to `adaptive_heating_mvp` plus local JSONL.
- **Current proof:** ignored real-PG controller insert tests pass; the read-only `pi5data` predeploy rehearsal confirms PostgreSQL-only status reads work; a transient verification window wrote a fresh `adaptive_heating_mvp` row at `2026-04-13 08:11:31+00`; the permanent `pi5data` service now runs with `TIMESCALEDB_CONNINFO` in `/etc/adaptive-heating-mvp.env`, no `LoadCredential=influx_token`, `/status` responds again after restart, and a fresh live-service row appeared at `2026-04-13 08:16:19+00`; a fixed-window controller parity rerun over `2026-04-13T05:33:00Z..05:34:00Z` now passes; a post-Multical-recovery spot-check over `2026-04-23T09:25Z..10:13Z` showed PostgreSQL and legacy Influx both current for `ebusd_poll` (`OutsideTemp`, `HwcStorageTemp`) plus Multical-driven DHW telemetry (`emon/multical/dhw_t1`), while `adaptive_heating_mvp` remained PostgreSQL-only by design after the live cutover removed Influx decision mirroring; and the follow-up Zigbee-route fix on 23 Apr made `heating-history` and `history-review both` run again on PostgreSQL for `2026-04-22T16:15:00Z..2026-04-23T10:15:00Z` by falling back to Flux for Zigbee temperature/humidity topics when `pi5data` lacks those `zigbee` columns.
- **Still open:** broader representative DHW/history/controller parity is still not recorded. The room-temperature route blocker is now cleared, but DHW output shape still drifts versus Flux, controller evidence is intentionally no longer mirrored into Influx so representative `heating-history`/`history-review` JSON still diverges on controller events, and rollback confidence is rehearsed but not yet fully proven.
- **Shared tracker status:** `~/github/energy-hub/lat.md/tsdb-migration.md` now reflects that this repo's live controller cutover and `history.rs` helper cleanup are done, leaving only the final parity/integration/rollback evidence pack as the shared Phase 5 blocker.
- **Recent cleanup:** the `history.rs` `--profile-queries` Flux profiler tail has now been deleted from the CLI, helper code, and tests, so representative operator history reads are PostgreSQL-first without a special profiler exception.
- **Shared-platform state:** `energy-hub` shared-platform phases are green, `z2m-hub` has closed its repo-local migration, `ebusd_poll_text` is live on `pi5data`, and repo-local readers now use `Statuscode` from that table while reading `HwcSFMode` from its real `ebusd`/`700` source.
- **Do not re-decide:** use sibling `ebusd_poll_text` rather than mixing numeric/text values in one `ebusd_poll` table.

## Remaining direct migration items

This table lists the highest-value remaining items so agents do not need to rediscover them by grep before every session.

| Remaining item | File / owner | Functions / scope | Why still open | Primary proof |
|---|---|---|---|---|
| Live controller TSDB parity + deploy | `src/bin/adaptive-heating-mvp.rs` | PostgreSQL-only latest-value reads, decision-log mirror, rollback rehearsal | live rollout is now complete on `pi5data`, but rollback confidence and cross-repo completion signalling still keep this as the highest-risk remaining component | controller contract tests + fixed-window decision parity + live deploy rehearsal |
| Real parity/integration verification | repo-local verification task | representative history, DHW, and controller windows | code paths are staged, and the 23 Apr Zigbee fallback fix restored PostgreSQL execution for representative heating/history windows, but JSON parity still differs because DHW output is richer on PostgreSQL and controller events are now PostgreSQL-only after live cutover | parity fixtures/windows + integration checks |

## Completion-critical next actions

This ordered list is the repo-local plan to finish the PostgreSQL cutover without mixing in unrelated controller or domain work.

1. Reconcile the remaining DHW parity drift (`remaining_litres`, sample counts, and timestamp precision) on the 23 Apr representative window or record why the richer PostgreSQL shape is the accepted replacement contract.
2. Reframe representative controller parity around PostgreSQL-only decision evidence now that live Influx mirroring is intentionally gone, then record the accepted proof window here alongside the existing fixed-window controller evidence.
3. Record a clean rollback rehearsal for the now-live PostgreSQL-only controller path on `pi5data` so restore confidence is explicit rather than inferred.
4. Update the shared migration tracker again to mark this repo complete once the remaining representative parity and rollback evidence is recorded.

## 2026-04-23 Representative parity attempt

This section records the first post-recovery exact-window parity run so later sessions can continue from facts rather than repeat discovery work.

- **How it was run:** local `cargo run --bin heatpump-analysis -- ...` with `INFLUX_TOKEN` from `ak get influxdb`, PostgreSQL reached over an SSH tunnel to `pi5data:5432`, and a temporary copy of `model/thermal-config.toml` with `[postgres]` removed for the Flux-only comparison path.
- **Heating/history window:** `heating-history --since 2026-04-22T16:15:00Z --until 2026-04-23T10:15:00Z` and `history-review both --since 2026-04-22T16:15:00Z --until 2026-04-23T10:15:00Z --no-sessions` now both run on PostgreSQL again after `history.rs` falls back to Flux for Zigbee environmental topics when `pi5data` lacks `zigbee.temperature` / `zigbee.humidity`.
- **Direct SQL blocker and fix:** querying `pi5data` directly showed `zigbee` currently has columns `time, device, power, energy, voltage, current, state` and no `temperature` column. The local fix was not to invent a fake PG source, but to keep PostgreSQL-first reads and fall back per-topic to Flux for those unsupported Zigbee environmental columns.
- **Heating/review parity status:** runtime failure is fixed, but exact JSON parity is still not achieved. The PostgreSQL path now includes controller events from `adaptive_heating_mvp`, while the Flux-only comparison path has no fresh controller rows because live Influx decision mirroring was intentionally removed at cutover.
- **DHW window:** `dhw-history --since 2026-04-23T10:10:00Z --until 2026-04-23T10:27:00Z` succeeds on both backends, but the outputs are not yet parity-equivalent: PostgreSQL returns `remaining_litres`/`dhw_capacity` evidence that Flux leaves null, and sample counts plus timestamps differ (`t1_c.samples` 102 on PostgreSQL vs 510 on Flux).
- **Controller evidence:** the old fixed-window controller parity proof remains valid, but a fresh representative-window proof now needs a PostgreSQL-first framing rather than a Flux-vs-PostgreSQL comparison because live Influx no longer receives controller decision rows.

## History profiler tail cleanup

`src/thermal/history.rs` no longer exposes the old `--profile-queries` Flux-profiler path.

The project chose deletion rather than replacement: representative history commands stay PostgreSQL-first, and any future PostgreSQL-native profiling should be added only for a proven operator need rather than to preserve transport symmetry for its own sake.

## Exact live Influx dependency removal plan

This section turns the remaining controller cutover into a concrete file-by-file work pack so the last live Influx dependency can be deleted rather than just discussed.

### Controller code changes

These source changes remove the live service's read/write/token dependency on Influx.

1. **Make PostgreSQL mandatory for the live controller path** in `src/bin/adaptive-heating-mvp.rs`.
   - Stop treating `[postgres]` as optional for live reads.
   - Fail fast at startup when `conninfo_env` is missing instead of silently falling back to Flux for controller-critical reads.
2. **Delete controller Influx auth plumbing** in `src/bin/adaptive-heating-mvp.rs`.
   - Remove `influx_url`, `influx_org`, `influx_bucket`, `influx_token_env`, and `influx_token_credential` from `Config` once no live controller path uses them.
   - Remove `INFLUX_TOKEN_CACHE`, `influx_token()`, `resolve_influx_token()`, and `read_token_file()`.
3. **Delete controller Flux fallback reads** in `src/bin/adaptive-heating-mvp.rs`.
   - Remove `parse_flux_single_value()` and `query_single_value()`.
   - Remove the Influx branch from `query_latest_topic_value()` and `query_latest_measurement_value()`.
   - Expand the PostgreSQL routing in `query_latest_topic_value_pg()` / `query_latest_measurement_value_pg()` until every live controller topic/measurement succeeds without the `unsupported PostgreSQL topic route` escape hatch for production inputs.
4. **Delete controller Influx decision writes** in `src/bin/adaptive-heating-mvp.rs`.
   - Remove `write_influx_decision()` once PostgreSQL plus JSONL is the accepted durable logging path.
   - Simplify the write path so decision logging is PostgreSQL-first with JSONL side-by-side, not PostgreSQL-plus-Influx mirroring.

### Live config and service changes

These config/deploy changes remove the operational secret and startup dependency on Influx.

1. **Cut over the live config** in `model/adaptive-heating-mvp.toml`.
   - Enable `[postgres] conninfo_env = "TIMESCALEDB_CONNINFO"` in the real service config.
   - Remove the live-service `influx_*` keys once the controller binary no longer reads them.
2. **Remove the systemd Influx credential** in `deploy/adaptive-heating-mvp.service`.
   - Delete `LoadCredential=influx_token:/etc/adaptive-heating-mvp/influx.token`.
   - Keep `/etc/adaptive-heating-mvp.env` only for non-Influx values unless PostgreSQL conninfo is later moved to a credential too.
3. **Update the transient verifier** in `scripts/run-controller-tsdb-verify-window.sh`.
   - Remove the transient unit's `LoadCredential=influx_token:...` property.
   - Keep only the PostgreSQL conninfo environment for the staged runtime window.
4. **Retire the legacy secret runbook** in `deploy/SECRETS.md`.
   - Reframe it around the production PostgreSQL conninfo/credential path.
   - Keep Influx notes only as historical migration context until the secret is actually deleted from the host.

### Controller validation sequence

Use this exact order so the live cutover has explicit proof and rollback checkpoints.

1. Run the existing read-only predeploy check with PostgreSQL enabled.
2. Run the transient verification window without any Influx credential loaded.
3. Confirm the transient run still serves `/status`, writes fresh `adaptive_heating_mvp` rows, and shows no Flux/token fallback in logs.
4. Edit the permanent service config/unit on `pi5data`, restart systemd, and immediately verify journal/API/TimescaleDB evidence.
5. Only after that delete `/etc/adaptive-heating-mvp/influx.token` from the host and remove the remaining legacy references from repo docs/scripts.

### After the live cutover

Once the controller no longer needs Influx, the remaining repo-local migration tail becomes non-live cleanup rather than a service blocker.

- `src/thermal/history.rs`: the old Flux profiler tail is now deleted; avoid reintroducing transport-symmetry abstractions unless a real PostgreSQL-native profiling need appears.
- `model/thermal-config.toml` and `src/thermal/config.rs`: keep the Influx block only while non-controller thermal commands still depend on it.
- `src/thermal/influx.rs` and related thermal modules: remove Flux/CSV parser paths command-by-command as PostgreSQL equivalents become authoritative.
- Retire the migration-tail test sections listed below as their code paths disappear.

## Migration-tail test coverage to retire after cutover

This audit lists the `lat.md/tests` sections that exist only because legacy Flux/CSV or line-protocol compatibility still survives in code.

Retire these specs once the matching migration-tail code paths are deleted, and move any remaining open work back into this tracker rather than leaving stale compatibility truth in `tests.md`.

- **Remove with raw Flux/CSV parser removal:** [[tests#InfluxDB wire-format parsing]]
- **Retired with `history.rs` profiler-tail removal:** `tests#History evidence helpers#profiled_flux wraps query with profiler import`
- **Remove with remaining `history.rs` Flux compatibility/parity tail removal:** the Flux-specific compatibility framing in [[tests#History filter variant routing]] plus any Flux-vs-PostgreSQL parity checks such as `Controller rows match between Flux and PostgreSQL on a representative window`
- **Remove with final LP write removal for DHW session mirroring:** LP-only helpers under [[tests#DHW write contracts]] such as `dhw_inflection LP line contains all required fields`, `LP tag spaces replaced with underscores`, and `dhw_capacity LP line maps to TimescaleDB columns`

Keep the transport-agnostic PostgreSQL-facing contracts in `tests.md`: query return shapes, PostgreSQL routing, timestamp behaviour that still affects PG reads, and real PostgreSQL insert/value/type checks.

## Outstanding completion gate

This checklist keeps only the migration-completion gates that are still open.

- [x] Delete the obsolete `history.rs` Flux profiler tail.
- [ ] Verify one-shot readers against PostgreSQL on representative history windows.
- [ ] Verify one-shot writers for row-equivalent SQL output.
- [x] Verify `adaptive-heating-mvp` read/write behaviour against PostgreSQL in live-service conditions.
- [x] Remove the live service's legacy Influx-only config and credential dependency.
- [ ] Remove the migration-tail test sections identified above once their code paths are deleted.
- [x] Record the accepted shared prerequisite status (`energy-hub` green, `z2m-hub` closed) as satisfied for final cutover.
- [x] Verify the live controller deploy on `pi5data`.
- [ ] Test or fully rehearse the rollback path.
- [x] Update shared `energy-hub` migration status to note this repo's current closeout state.
