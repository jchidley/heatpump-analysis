# History Query Architecture Plan

Plan for making historical DHW and heating review commands fast, correct, and operationally useful.

## Goal

Historical review commands should:
- default to **last 7 days ending now**
- answer the real operator question: **are the recent heating and DHW changes working, and what should change next?**
- return output that is **comprehensive enough to assess plan performance**
- use **native device cadence** where relevant
- push the real work into **InfluxDB / Flux**, not Rust post-processing
- return **small, shaped result sets**, not large CSV dumps for local reconstruction

The primary output should be **decision-first**, not fact-first.
That means the top of `history-review` should eventually read more like:
- heating: working / mixed / failing
- DHW: working / mixed / failing
- strongest supporting evidence
- main caveats / missing evidence
- recommended next changes

The lower-level compact facts still matter, but they support the decision instead of being the decision.

## Experiment-driven review model

The main use of history is now **experiment evaluation**, not generic reporting.

Each review should help evaluate a recent change against the heating or DHW plan:
1. what changed?
2. what outcome was the change trying to improve?
3. did the outcome improve?
4. what evidence supports that?
5. what confounders reduce confidence?
6. what should change next?

That means `history-review` should evolve into an **experiment scorecard** over the authoritative compact facts from `heating-history` and `dhw-history`.

## InfluxDB anti-pattern guardrail

We should always remember that this is **InfluxDB-backed historical analysis**, not a generic in-memory analytics pipeline.

Do **not** regress into these anti-patterns:
- pulling broad raw time series into Rust and reconstructing events client-side by default
- issuing repeated per-event round trips when one batched compact query can return the same facts
- using heavy Flux operators early (`pivot`, `join`, `union`, `map`, `reduce`) before storage pushdowns have narrowed the data
- inventing fake resolution with tiny windows instead of using true source cadence
- expanding output schemas with facts that do not help judge the experiment or choose the next change
- treating `history-review` as a generic dashboard dump instead of a decision layer

If a proposed addition does not improve **correctness**, **decision usefulness**, or **recommended next action**, it should be treated as YAGNI until proven otherwise.

## YAGNI cut list

The following work should be cut or explicitly deprioritised unless a concrete need appears:
- extra raw fields that do not change the experiment verdict
- generic reporting abstractions in `history-review`
- richer developer-facing profiler output
- query micro-optimisation beyond current routine-use performance
- broad provenance/schema expansion before verdicts are stable
- new commands that duplicate existing history commands without improving decisions

Keep only work that clearly improves one of:
- correctness
- experiment judgement
- confidence / confounder handling
- recommended next action

## Core rule

**The database should do the database work — but specifically in the most effective way InfluxDB recommends.**

Within that rule:
- choose the **authoritative source first** for each field
- then use that source at its **native cadence** where relevant
- structure Flux so **pushdown-capable operations happen first**
- avoid heavy in-memory Flux operators until the input has already been narrowed
- batch related summaries into **fewer query requests** where one request can return multiple shaped result tables

Do not prefer a faster but less authoritative signal over a slower authoritative one.

That means:
- event detection in Flux where it can still be expressed efficiently
- period construction in Flux where possible
- min/max/first/last/count in Flux
- compact per-event summaries from Flux
- Rust used mainly for:
  - command wiring
  - query batching / orchestration
  - small final labels / interpretation
  - human / JSON presentation

It does **not** mean:
- pulling 7 days of detailed samples into Rust by default
- reconstructing charge periods client-side from big CSV responses
- repeated per-event round trips when one compact summary query can do the job
- assuming "Flux-first" automatically means "efficient" without checking pushdowns, heavy operators, and actual query profiles

## Resolution policy

"Maximum resolution" means:
- use the **native cadence of the directly connected device/source**
- do **not** invent extra detail by choosing a smaller aggregate bucket
- do **not** silently downsample below native cadence unless there is a documented reason

### Current observed native cadences

Observed from recent raw timestamp checks:

| Signal | Native cadence observed |
|---|---:|
| `emon/dhw_t1` | ~2s |
| `ebusd_poll/HwcStorageTemp` | ~30s |
| `ebusd_poll/BuildingCircuitFlow` | ~30s |
| `dhw/remaining_litres` | ~10s |
| Leather (`emonth2`) | ~56s |
| `ebusd_poll/OutsideTemp` | ~30s |
| Zigbee room sensors | sparse / event-driven; treat as native irregular cadence |

These cadences should guide query design.

## Current executable baseline

The command surface needed by this plan already exists and is runnable today:
- `cargo run --bin heatpump-analysis -- heating-history [--human]`
- `cargo run --bin heatpump-analysis -- dhw-history [--human]`
- `cargo run --bin heatpump-analysis -- dhw-drilldown --since ... --until ... [--human]`
- `cargo run --bin heatpump-analysis -- history-review heating|dhw|both [--no-sessions]`

Current implementation status:
- `dhw-history` already follows the compact Flux-first direction reasonably well
- `dhw-drilldown` now exists as an explicit bounded native-cadence command for one chosen DHW window
- `heating-history` now uses compact pushdown-first / Flux-shaped summaries for its main numeric fields and Flux/state-change period construction for comfort misses and DHW overlap
- `heating-history` now reports recent cadence estimates as part of the default compact review output
- compact numeric summary queries in `heating-history` and `dhw-history` are now batched into fewer `/api/v2/query` requests
- `heating-history` and `dhw-history` now support `--profile-queries`, which emits raw Flux profiler output to stderr for key query blocks
- `history-review` now reuses the lower-level compact summaries directly for structured output
- structured default output is already in place for `heating-history`, `dhw-history`, and `history-review`
- `history-review --human` preserves the previous operator-facing wrapper presentation

So this plan is executable as an incremental refactor plan, not a greenfield design.

## InfluxDB efficiency policy

The project standard is now **pushdown-first**, not just "Flux-first".

For InfluxDB 2 / Flux, official InfluxData guidance says to:
- start queries with pushdown-capable functions such as `range()` and static `filter()`
- remember that once a non-pushdown function runs, subsequent work may execute in memory rather than in the storage layer
- treat `map()`, `reduce()`, `join()`, `union()`, and `pivot()` as relatively heavy operators and use them sparingly
- prefer `set()` over `map()` when assigning a static value
- avoid unnecessarily short windows
- use Flux profilers to measure actual query/operator cost instead of guessing

For this repo, that translates into the following practical rules:
1. **Push down early**: `range()` + static `filter()` + selector/aggregate as early as possible.
2. **Transform late**: `map()` / `pivot()` / `union()` only after the candidate rows are already small.
3. **Batch related summaries**: prefer one query request returning multiple yielded result tables over many small round trips.
4. **Prefer compact outputs**: return shaped result tables, not large raw dumps.
5. **Profile important queries**: especially those using `pivot()`, `map()`, `union()`, or state-change logic.
6. **Respect schema realities**: tags are indexed, fields are not; repeated filter patterns should influence schema decisions where feasible.

## Design pattern

### Default 7-day review

Default history investigation should start with a **rolling 7-day window ending now**.

This default review is the standard experiment-evaluation pass.
It should be good enough to decide whether the latest heating and DHW changes look:
- successful
- mixed
- unsuccessful
- or inconclusive due to confounders

Default review commands should return:
- top-level verdicts for the current heating and DHW plans
- compact whole-window summaries
- compact event lists
- compact per-event summaries
- compact recent cadence estimates
- warnings where evidence is missing, stale, or inferred
- explicit "what should change next" recommendations where the evidence is strong enough

This matches the wider repo workflow:
1. rolling 7-day review first
2. fixed named anchor windows second when reproducing a documented case
3. explicit event-scoped drill-down only after an event worth zooming into has been identified

### Named anchor-window replay

Named anchor windows are for:
- regression checking
- reproducing examples cited in plan docs
- comparing future behaviour against a known historical case

They should reuse the same compact summary architecture as the rolling 7-day review, just with fixed `--since` / `--until` bounds.

### Native-resolution drill-down

Use native-resolution detail only for:
- one nominated charge / draw / overlap / comfort-miss event
- one named anchor window where compact summary is not enough
- event refinement where Flux cannot compactly express the logic

Even then, the drill-down should be explicit and bounded.

## Heating experiment scorecard

A heating experiment review should eventually expose:
- `status`: working | mixed | failing | inconclusive
- `change_under_review`
- `success_criteria_checked`
- `supporting_evidence`
- `confounders`
- `recommended_next_change`

Typical heating success criteria:
- Leather reaches or exceeds 20°C by 07:00 on relevant mornings
- waking-hours comfort misses are reduced or acceptably rare
- DHW overlap is not materially causing comfort failure
- suspected sawtooth behaviour is reduced, absent, or clearly confounded

Typical heating confounders:
- open doors / disturbance-heavy day
- DHW overlap during preheat or recovery
- very cold weather where HP capacity is the limiting factor
- missing controller intent rows or missing room evidence

## DHW experiment scorecard

A DHW experiment review should eventually expose:
- `status`: working | mixed | failing | inconclusive
- `change_under_review`
- `success_criteria_checked`
- `supporting_evidence`
- `confounders`
- `recommended_next_change`

Typical DHW success criteria:
- charge timing keeps the household reliably supplied
- full-charge fraction is acceptable
- partial evening charges are rare or explained
- top-up behaviour matches real demand rather than controller artefacts
- practical readiness after charges remains adequate

Typical DHW confounders:
- embedded session analysis not exact-window bounded
- sparse `remaining_litres` boundary evidence
- high-demand or unusual household-use day
- showers during charge
- eco/normal mode mismatch or unknown mode

## DHW architecture

### What the default DHW review must answer

A default `dhw-history` / `history-review dhw` should answer:
- is the recent DHW plan **working, mixed, or failing**?
- how many charges happened?
- how many were full vs partial?
- when did they occur?
- what did `T1` / `HwcStorageTemp` do during those charges?
- what is the current practical DHW state?
- does recent behaviour support the current DHW plan?
- what should the **next DHW change** be, if any?

Concretely, the operator-facing review should be able to say things like:
- the new DHW timing is working / not working
- evening charges are still timing out or ending partial
- 04:00 top-ups look justified / unnecessary
- current recommended next change is to keep timing, switch mode, or change trigger logic

### DHW authority and summary semantics

For DHW review:
- `T1` must come from the Multical DHW meter
- `HwcStorageTemp` must come from eBUS
- remaining litres is a derived practical-state signal from z2m-hub, not raw physical truth
- charge completion is an operational interpretation, not a raw register

Compact per-charge summaries should use explicit event semantics, not ambiguous in-window first/last values.
At minimum, the implementation should distinguish between:
- event boundary timestamps: `charge_start`, `charge_end`
- event-boundary temperatures: `t1_start`, `t1_peak`, `t1_end`, `hwc_start`, `hwc_peak`, `hwc_end`
- completion interpretation: `crossover = true|false`
- practical-state fields: `remaining_litres_pre_charge`, `remaining_litres_post_charge`

If a field is not a true event-boundary value and is instead the first/last value found inside the outer review window, that should be labelled clearly or excluded.

### Current status

`dhw-history` has now been moved toward a **Flux-first compact summary** approach:
- whole-window summaries from compact Flux queries
- charge periods from compact Flux state-change queries on `BuildingCircuitFlow`
- per-charge compact summaries from Flux
- divergence summary from Flux
- recent cadence estimates from short recent-window checks

This is a substantial improvement over large client-side reconstruction.

### Remaining DHW issues

1. **Charge summary semantics are improved but not finished**
   - event-boundary lookups are now used for charge start/end fields in `dhw-history`
   - `remaining_litres` boundary attribution still needs validation and may still be absent when the source stream has no boundary-adjacent sample
   - compact summaries should keep preferring true pre/post operational state over accidental period-local values

2. **Charge boundary stability must remain a test target**
   - event boundaries should be invariant to the chosen outer review window
   - no drifting starts/ends when the surrounding query window changes

3. **DHW drill-down now exists; validate and iterate**
   - `dhw-drilldown` keeps native 2s `T1` review separate from the default compact 7-day path
   - next work is on usability, boundary validation, and deciding whether event selection should later be by explicit event ID as well as by `--since` / `--until`

## Heating architecture

### What the default heating review must answer

A default `heating-history` / `history-review heating` should answer:
- is the recent heating plan **working, mixed, or failing**?
- was comfort met?
- when were comfort misses?
- when did DHW overlap with heating?
- what did the controller intend?
- what did the actuator actually do?
- does the recent window support or weaken the current heating plan?
- what should the **next heating change** be, if any?

Concretely, the operator-facing review should be able to say things like:
- overnight preheat is working / late / too aggressive
- DHW overlap is or is not materially harming morning comfort
- sawtooth risk looks real / not yet proven
- current recommended next change is to hold course, tweak overnight logic, or investigate a confounder first

### Heating authority rules

For heating review:
- comfort truth must come from room temperatures
- controller intent must come from adaptive-heating runtime state and/or JSONL decision logs where available
- actuator output truth must come from eBUS (`Hc1HeatCurve`, `Hc1ActualFlowTempDesired`, actual flow/return, outside temp)
- DHW overlap must be derived from fused heating + DHW evidence, not inferred from one unreliable categorical register

The default heating review must support the operational questions already used elsewhere in the docs:
- overnight planner effectiveness
- comfort by 07:00 / waking-hours comfort
- DHW interference with preheat or recovery
- target flow versus actual desired flow
- sawtooth candidate detection

### Current status

`heating-history` has now been moved substantially toward the same compact Flux-first standard as the revised DHW path.
Current behaviour now includes:
- compact Flux summaries for Leather / Aldora / outside / curve / desired flow / actual flow / return
- Flux/state-change period construction for comfort misses and DHW overlap periods
- compact controller event shaping from InfluxDB
- recent cadence estimates in the review output

Rust still performs the final intent join and human / JSON presentation, but the default path no longer depends on large Rust-side raw-series reconstruction for the main heating review facts.

### Remaining heating work

The first pushdown-first refactor of `heating-history` is now in place.
Implemented scope:
- Leather / Aldora / outside / curve / desired flow / actual flow / return compact summaries
- comfort-miss periods
- DHW overlap periods
- compact controller event list
- sawtooth-supporting metrics
- likely overnight preheat start detection
- recent cadence estimates
- batched compact numeric summaries
- profiler mode via `--profile-queries`
- batched DHW charge-period summaries and boundary lookups

Remaining work is now about refinement rather than the initial architectural move:
- batch more of the remaining boundary/event queries, especially repeated heating-side event subqueries
- add tests/regression checks around period-boundary stability
- make overnight-planner verdict facts more explicit at top level
- decide whether controller-intent provenance needs richer per-field labelling
- consider parsing profiler output into a compact structured developer summary instead of raw stderr only
- keep reducing any unnecessary Rust-side reconstruction in edge cases

Where controller-intent data comes from runtime state / JSONL rather than Flux-backed measurements, Rust may still join that compact intent evidence onto the Flux-derived plant summaries — but it should still return a compact shaped result, not reconstruct the whole review from raw series.

## Command behaviour policy

### Defaults

These commands should continue to default to **last 7 days ending now**:
- `heating-history`
- `dhw-history`
- `history-review heating|dhw|both`

### Command roles

- `heating-history` is the authoritative fused heating-window command
- `dhw-history` is the authoritative fused DHW-window command
- `history-review` is a higher-level composition / reporting layer over those facts

`history-review` should not reintroduce heavy Rust-side reconstruction or large raw-series processing.

### Output

For `heating-history` and `dhw-history`, default output should be **structured** and stable enough for automation, diffing, and LLM consumption.

`--human` should provide the compact operator summary:
- comprehensive enough for plan review
- compact rather than raw-sample heavy
- aligned with the same facts exposed in structured output

Current state:
- `heating-history` and `dhw-history` already follow this model
- `history-review` now defaults to structured output built from those same compact summaries
- `history-review --human` keeps the operator-facing wrapper presentation

## Execution plan

### Immediate

1. **Tighten DHW compact summary semantics** 🟡
   - file: `src/thermal/history.rs`
   - event-boundary lookups are now in place for charge start/end fields
   - crossover logic is now based on those explicit boundary values
   - remaining work: validate `remaining_litres` boundary attribution and add tests/regression checks for boundary stability

2. **Add DHW drill-down command or mode** ✅
   - implemented as `cargo run --bin heatpump-analysis -- dhw-drilldown --since ... --until ... [--human]`
   - files: `src/main.rs`, `src/thermal/history.rs`
   - event-scoped native-cadence review for one chosen charge/draw window
   - uses native 2s `T1` there
   - kept separate from the default 7-day compact review path

3. **Refactor heating-history to Flux-first** ✅
   - file: `src/thermal/history.rs`
   - compact Flux summaries now used for main heating numeric fields
   - comfort misses and DHW overlap periods now come from Flux/state-change queries rather than Rust-side series reconstruction
   - recent cadence estimates now appear in `heating-history`
   - controller-intent joins remain compact in Rust where needed

### Next

4. **Batch compact summary queries into fewer InfluxDB requests** ✅
   - files: `src/thermal/history.rs`, `src/thermal/influx.rs`
   - heating and DHW compact numeric summaries now batch related count/first/last/min/max work into fewer multi-result Flux scripts
   - keeps result sets compact while reducing HTTP round trips to `/api/v2/query`

5. **Add profiling support for important history queries** ✅
   - files: `src/thermal/history.rs`, `src/thermal/influx.rs`, `src/main.rs`
   - `heating-history` and `dhw-history` now accept `--profile-queries`
   - profiler output is emitted to stderr using Flux `query` and `operator` profilers for key query blocks
   - this is a developer/operator optimisation path, not the default user-facing output mode

6. **Batch more boundary/event queries** 🟡
   - files: `src/thermal/history.rs`, possibly `src/thermal/influx.rs`
   - DHW charge-period summaries and boundary lookups are now batched into a few multi-result Flux requests instead of many per-period round trips
   - heating-side baseline / event subqueries have also been trimmed by replacing repeated summary-style `last()` lookups with direct compact selectors and by reusing already-fetched controller rows for sampling stats
   - testing also exposed missing `tariff` tags on some older `adaptive_heating_mvp` rows; controller-row shaping is now robust to missing `mode` / `action` / `tariff`
   - remaining work is mainly boundary-stability regression coverage and any further heating-side event-path consolidation revealed by profiling

7. **Add structured output to `history-review`** ✅
   - files: `src/main.rs`, `src/thermal/history.rs`, `src/thermal/dhw_sessions.rs`
   - `history-review` now defaults to structured JSON
   - `history-review --human` preserves the previous operator-facing wrapper presentation
   - structured output includes the compact heating / DHW summaries and DHW session summary when requested
   - **known limitation from testing:** embedded `dhw_sessions` still works on rounded `days`, not exact `since` / `until`; this can pull in evidence just outside the requested review window

8. **Make `history-review` session add-ons exact-window aware**
   - files: `src/main.rs`, `src/thermal/dhw_sessions.rs`
   - either teach DHW session analysis to accept explicit `since` / `until`, or label the current day-rounded behaviour clearly / disable it for exact-window reviews
   - this is now the main correctness gap in structured `history-review`

9. **Make `history-review` decision-first**
   - files: `src/main.rs`, `src/thermal/history.rs`, possibly new summary types
   - add top-level verdict sections for heating and DHW
   - each verdict should answer:
     - is it working?
     - what evidence supports that?
     - what caveats reduce confidence?
     - what change should be next?
   - treat this as an experiment scorecard, not a generic report
   - this is now more important than adding more raw metrics

10. **Add explicit experiment metadata / success criteria hooks**
   - files: `src/main.rs`, docs first
   - allow reviews to name the change under review and report against plan-aligned success criteria
   - prefer a small explicit schema over a large general-purpose reporting model

11. **Cut low-value history work from the active plan**
   - do not spend active effort on richer profiler presentation, generic reporting abstractions, or extra raw fields unless they unblock a decision
   - treat those as backlog only, not current roadmap items

12. **Document exact native cadence assumptions**
   - include source and cadence in the operational docs
   - treat cadence drift as something worth rechecking periodically

## Acceptance criteria

This plan is complete when:
- default 7-day review commands are fast enough for routine use
- they do not depend on large client-side CSV processing
- they report plan-relevant performance directly
- authoritative sources are chosen deliberately before cadence decisions are made
- native cadence is used deliberately, not faked and not silently over-aggregated
- outputs distinguish direct observation, inference, and missing evidence via warnings or clear field semantics
- event drill-down is explicit and separate from compact default review

## References

Primary official references used for this plan update:
- InfluxData, **Optimize Flux queries**: https://docs.influxdata.com/influxdb/v2/query-data/optimize-queries/
- InfluxData, **Query with the InfluxDB API**: https://docs.influxdata.com/influxdb/v2/query-data/execute-queries/influx-api/
- InfluxData, **Annotated CSV**: https://docs.influxdata.com/influxdb/v2/reference/syntax/annotated-csv/
- InfluxData, **Join data in InfluxDB with Flux**: https://docs.influxdata.com/influxdb/v2/query-data/flux/join/
- InfluxData, **Schema design**: https://docs.influxdata.com/influxdb/v2/write-data/best-practices/schema-design/
- InfluxData / Flux docs, **yield()**: https://docs.influxdata.com/flux/v0/stdlib/universe/yield/
- InfluxData / Flux docs, **profiler package**: https://docs.influxdata.com/flux/v0/stdlib/profiler/
- InfluxData, **Use parameterized Flux queries**: https://docs.influxdata.com/influxdb/cloud/query-data/parameterized-queries/

## Handoff / restart point

If restarting this work in a new session, the current state is:
- docs have been updated from "Flux-first" wording to **pushdown-first InfluxDB querying** with official InfluxData references
- `heating-history` and `dhw-history` now batch compact numeric summaries into fewer query requests
- `dhw-history` now also batches DHW charge-period summaries and boundary lookups into a few multi-result Flux requests
- heating-side event-path querying has been trimmed further by using direct compact `last()` selectors for baseline checks and by reusing controller rows for sampling stats
- `heating-history` and `dhw-history` support `--profile-queries`
- `history-review` now defaults to structured output, with `--human` preserving the previous wrapper view
- testing found that some older controller rows omit `tariff`; the controller-row query path is now hardened against missing tags
- testing also found that embedded `dhw_sessions` is still day-rounded rather than exact-window bounded
- the current gap is not mainly missing raw facts; it is that the primary output is still too fact-first and not decisive enough for "are my recent changes working?"
- the next highest-value code tasks are now to:
  1. **make `history-review` session add-ons exact-window aware**
  2. **add decision-first verdicts for heating and DHW, including recommended next changes**
  3. **turn those verdicts into explicit experiment scorecards aligned to the heating and DHW plans**
  4. then add boundary-stability regression coverage for decision-critical fields
- explicitly deprioritised for now:
  - richer profiler presentation
  - generic reporting features in `history-review`
  - extra raw fields that do not change verdicts
  - further query micro-optimisation unless routine use becomes slow again

Useful restart commands:
```bash
cargo run --bin heatpump-analysis -- heating-history --human
cargo run --bin heatpump-analysis -- heating-history --human --profile-queries \
  --since 2026-04-02T08:00:00Z --until 2026-04-02T09:00:00Z
cargo run --bin heatpump-analysis -- dhw-history --human
cargo run --bin heatpump-analysis -- dhw-history --human --profile-queries \
  --since 2026-04-02T05:00:00Z --until 2026-04-02T08:00:00Z
cargo run --bin heatpump-analysis -- history-review both
```

Code areas to inspect first in a new session:
- `src/thermal/history.rs`
- `src/thermal/influx.rs`
- `src/main.rs`
- `docs/history-query-architecture-plan.md`
- `docs/history-evidence-plan.md`

## Summary

The target architecture is:
- **7-day-to-now by default**
- **pushdown-first, compact Flux shaping** by default
- **fewer, more effective InfluxDB query requests** where related summaries can be batched together
- **native-cadence drill-down** only for explicitly selected events
- **Rust as orchestration/presentation layer**, not as a substitute database engine
