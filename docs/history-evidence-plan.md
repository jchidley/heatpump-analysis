# History and Evidence Plan

This document defines how historical evidence should be reconstructed for the heat-pump system.

It is the canonical reference for:
- which historical data sources exist
- which source is authoritative for each kind of signal
- which existing commands already reproduce evidence
- which new commands should be added
- what evidence is needed to support the next steps in the heating and DHW plans

Use other docs for adjacent needs:
- **Space-heating strategy and next steps:** `heating-plan.md`
- **DHW strategy and next steps:** `dhw-plan.md`
- **Historical evidence workflows / how-to:** `history-evidence-workflows.md`
- **Current live state:** `current-production-state.md`
- **Current live query commands:** `live-queries.md`
- **Code locations / implementation structure:** `code-truth/README.md`, `code-truth/REPOSITORY_MAP.md`, `code-truth/ARCHITECTURE.md`
- **Secrets handling:** `../deploy/SECRETS.md`

This document is intentionally self-contained apart from those references.

## Objective

Make the evidence behind the heating and DHW plans **reproducible on demand** for both humans and LLMs.

But reproducibility is not the final product. The final product is better operational decisions.
The primary historical output should help answer:
- are the recent **heating** changes working?
- are the recent **DHW** changes working?
- what should change next?

The desired end state is:
1. plan docs explain **what matters and why**
2. evidence commands reconstruct **what happened** for a chosen period
3. review commands state **whether the recent plan changes look successful, mixed, or unsuccessful**
4. review commands recommend the **next change or next investigation** when the evidence is strong enough
5. live-query commands show **what is happening now**
6. no important historical conclusion depends on undocumented operator memory

## Scope

This page is the **reference / roadmap** for historical evidence:
- data sources
- authority map
- canonical anchor windows
- command catalog
- maturity / gaps
- links from plan next steps to evidence commands

For step-by-step historical analysis, recipes, and review workflow, use `history-evidence-workflows.md`.

## Experiment-driven use case

The main use case is now:
- make a heating or DHW change
- review a bounded recent period
- decide whether the change is working
- decide what should change next

So historical evidence should be organised around **experiment evaluation**, not generic retrospective reporting.

## Principles

### Authority before cadence

Choose the **authoritative source first** for each field, then use that source at its **native cadence** where relevant.

Do not prefer a faster but less authoritative signal over a slower authoritative one.

### Command roles

- `heating-history` is the authoritative fused heating-window command
- `dhw-history` is the authoritative fused DHW-window command
- `history-review` is the higher-level composition / reporting layer over those facts

`history-review` should summarise or combine those results, not become a separate raw-series reconstruction engine.
Its primary job is to produce a **decision-first review**:
- how is heating working?
- how is DHW working?
- what change should be next?

### Historical evidence should not live mainly in prose

The plan docs should keep:
- conclusions
- rationale
- thresholds
- operating policy
- next-step questions

They should not be the only place where historical evidence can be found.

### Default output should favour LLM consumption

Historical and live commands should default to:
- structured JSON or TOML
- explicit field names
- stable shapes
- machine-readable warnings

For the history command family, the current intended default is:
- `heating-history` / `dhw-history` → structured by default, `--human` optional
- `history-review` → structured by default, `--human` for the operator-facing wrapper view

Add `--human` when a more readable operator summary is useful.

However, the primary output of `history-review` should not just be machine-readable facts.
It should be machine-readable **judgement**:
- verdict
- supporting evidence
- caveats / warnings
- recommended next change

### Reconstruct from all available sources, not one subsystem

Historical reconstruction must fuse the available evidence instead of trusting a single source blindly.

Relevant sources include:
- `emoncms` historical feeds
- `emonhp` heat meter / electrical data
- `emonpi` room and Zigbee-derived sensor data
- `emondhw` Multical DHW sensors
- eBUS registers and topics
- InfluxDB / Telegraf-ingested topics
- `z2m-hub` derived DHW state
- `adaptive-heating-mvp` runtime state and JSONL decisions

### Authority is field-specific

There is no single best source for the whole system. Different fields have different authoritative sources.

Examples:
- `T1` comes from the Multical DHW meter, not from eBUS
- `HwcStorageTemp` comes from eBUS, not from Multical
- controller intent comes from JSONL/runtime state, not from a heat meter
- room comfort comes from room sensors, not from heating flow temperatures

### Historical commands must say what they know and how strongly they know it

Where sources are incomplete or inferred, outputs should include:
- `warnings`
- clear distinction between **direct observation**, **derived interpretation**, and **missing / incomplete evidence**
- optional `source` and `confidence` fields later if needed

### Use an existing command when it is already good enough

Not every evidence need requires a new command.

There are two valid reproducibility paths:
1. **document an existing command or recipe** when one command already answers the question well enough for an LLM
2. **add a new fused command** when the answer currently requires stitching together multiple tools or undocumented operator knowledge

The standard should be: an LLM should be able to reproduce the evidence from this document and its references without hidden context.

### YAGNI / anti-pattern guardrail

Do not add historical machinery that does not help evaluate heating or DHW experiments.

Low-value work should be deprioritised unless it improves correctness or decisions directly:
- extra raw fields without changing the eventual verdict
- broad generic reporting abstractions
- more query micro-optimisation when current performance is already routine-use acceptable
- client-side reconstruction that duplicates work InfluxDB should do

Because this system is InfluxDB-backed, avoid these specific anti-patterns:
- wide raw-series exports followed by Rust-side reconstruction by default
- per-event query loops where one compact batched query can do the job
- heavy Flux transformations before pushdown narrowing
- pretending tiny buckets create real resolution

Concrete cut/defer list for the current roadmap:
- richer profiler presentation
- generic reporting features in `history-review`
- extra machine-readable fields that do not change verdicts or recommendations
- generic abstractions added before the heating/DHW scorecard shape is stable
- further optimisation work unless routine review speed becomes a real problem again

## Historical data sources

## `emoncms`

Use for:
- long historical heat pump performance
- heat / electricity history
- coarse operating-state analysis
- long retention baseline work

Strengths:
- long time range
- already used by the main analysis CLI

Weaknesses:
- does not carry all newer DHW/controller signals
- some feeds are legacy or stale

Typical role in evidence commands:
- long-range baseline and historical backfill
- older windows where newer fused data is unavailable

## `emonhp`

Use for:
- heat meter and electrical data feeding emoncms history
- long-range COP / energy views

Strengths:
- trusted for historical HP performance

Weaknesses:
- not enough on its own for controller-era evidence

Typical role in evidence commands:
- historical HP performance backbone

## `emonpi`

Use for:
- room temperatures
- Zigbee sensors
- future door sensors

Strengths:
- comfort-side truth

Weaknesses:
- sensor cadence and sensor quality vary by device

Typical role in evidence commands:
- comfort truth and, later, door-open context

## `emondhw`

Use for:
- Multical `T1`
- Multical `T2`
- tap-side flow
- tap-side volume

Strengths:
- authoritative DHW comfort and usage truth

Weaknesses:
- does not know controller intent by itself

Typical role in evidence commands:
- authoritative DHW comfort and usage timeline

## eBUS

Use for:
- `HwcStorageTemp`
- `Hc1HeatCurve`
- `Hc1ActualFlowTempDesired`
- actual flow / return
- HP run status
- `HwcSFMode`
- controller-side and plant-side actuator state

Strengths:
- best source for what the Vaillant controls believed and demanded

Weaknesses:
- some registers are misleading or unreliable if used naively
- categorical status values need careful handling

Known caveats:
- `StatuscodeNum` is not reliable for DHW detection
- `CurrentCompressorUtil` has shown nonsense values and should not be trusted without cross-checking

Typical role in evidence commands:
- actuator and plant-side truth for heating and DHW windows

## InfluxDB / Telegraf

Use for:
- merged topic history
- room temperatures
- Multical readings
- eBUS-mirrored topics where available
- controller-side live analysis inputs

Strengths:
- practical integration layer across subsystems

Weaknesses:
- topic naming and field naming vary by source

Typical role in evidence commands:
- main fusion layer for time-aligned topic history

## `z2m-hub`

Use for:
- live and historical derived DHW state in the `dhw` measurement
- remaining litres
- effective `T1`
- charge-state interpretation

Strengths:
- household-facing practical DHW interpretation

Weaknesses:
- derived state, not raw physical truth

Typical role in evidence commands:
- practical DHW interpretation, especially remaining litres and charge-state synthesis

## `adaptive-heating-mvp` state and JSONL logs

Use for:
- controller mode
- controller reasoning
- target flow
- write actions
- decision timing

Strengths:
- best record of what the controller decided to do

Weaknesses:
- not enough on its own to explain plant behaviour

Typical role in evidence commands:
- controller intent, timing, and action audit trail

## Field authority map

## Heating

| Field | Preferred source | Why |
|---|---|---|
| Leather / Aldora / room temps | Influx topics from emonpi / Zigbee | comfort truth |
| Door states | Zigbee topics via Influx | causal context |
| Controller mode / reason | runtime state + JSONL | intent truth |
| Target flow | JSONL / runtime state | controller intent |
| Heat curve | eBUS | actual actuator value |
| Actual desired flow | eBUS | actuator output truth |
| Actual flow / return | eBUS | live hydraulic truth |
| Outside temp | eBUS first, historical fallback as needed | best operational relevance |
| DHW overlap | fused from eBUS + DHW history | avoids single-source mistakes |
| Long-range electricity / heat | emoncms / emonhp history | durable baseline |

## DHW

| Field | Preferred source | Why |
|---|---|---|
| `T1` | Multical on emondhw | authoritative hot-water comfort truth |
| `T2` | Multical on emondhw | authoritative cold-side truth |
| Tap flow / volume | Multical on emondhw | actual DHW usage |
| `HwcStorageTemp` | eBUS | lower-cylinder control truth |
| `HwcSFMode` / DHW target | eBUS | controller-side DHW state |
| Charge completion | derived from `HwcStorageTemp >= T1_at_charge_start` | operationally validated truth |
| Remaining litres | z2m-hub `dhw` measurement | practical household-facing estimate |
| HP-side charge temperatures | eBUS / HP-side sources | charge dynamics |
| Eco / normal inference | derived from charge characteristics / max flow temp | inferred operational mode |

## Existing reproducible evidence

### Standard review protocol

Unless a command forces a different input shape, the default historical review for **both heating and DHW** is:
1. confirm the current UTC time with `date -u`
2. run the fused history commands with their built-in 7-day-to-now defaults
3. replay a documented fixed anchor window when you need to reproduce a named historical case
4. drill into a narrower event-scoped window only after the 7-day sweep or anchor replay identifies something worth zooming into

Canonical shell pattern:

```bash
date -u
export INFLUX_TOKEN=$(ak get influxdb)
cargo run --bin heatpump-analysis -- heating-history
cargo run --bin heatpump-analysis -- dhw-history
cargo run --bin heatpump-analysis -- dhw-sessions --days 7 --format json
```

These already have good-enough commands and should be documented rather than replaced.

| Topic | Command | Notes |
|---|---|---|
| Current heating state | `cargo run --bin adaptive-heating-mvp -- --config model/adaptive-heating-mvp.toml status` | structured by default |
| Current heating state (human) | `... status --human` | operator view |
| Current DHW state | `cargo run --bin heatpump-analysis -- dhw-live-status` | structured by default |
| Current DHW state (human) | `... dhw-live-status --human` | operator view |
| Historical DHW session analysis | `cargo run --bin heatpump-analysis -- dhw-sessions --days 7 --format json` | default rolling 7-day inflection / capacity / draw type evidence |
| Fused heating history | `cargo run --bin heatpump-analysis -- heating-history` | authoritative fused heating-window command; structured by default; defaults to last 7 days ending now; add `--human` for operator view |
| Fused DHW history | `cargo run --bin heatpump-analysis -- dhw-history` | authoritative fused DHW-window command; structured by default; defaults to last 7 days ending now; add `--human` for operator view |
| Native DHW drill-down | `cargo run --bin heatpump-analysis -- dhw-drilldown --since ... --until ...` | bounded native-cadence detail for one chosen DHW event/window; first executed architecture milestone from `history-query-architecture-plan.md` |
| History review layer | `cargo run --bin heatpump-analysis -- history-review heating|dhw|both` | higher-level composition/reporting layer over the fused history commands |
| Analysis CLI summaries from emoncms history | existing `summary`, `daily`, `hourly`, `cop-by-temp`, `overnight`, `thermal-operational` style commands | useful, but not yet a full long-range reproduction layer |
| Live multi-system snapshot | `bash scripts/live-state.sh` | convenience wrapper |

## Historical claim classes

When reviewing a claim from `heating-plan.md` or `dhw-plan.md`, classify it into one of three groups:

| Class | Meaning | Action |
|---|---|---|
| A | already reproducible from one existing command | document the command here or in a linked query doc |
| B | reproducible, but requires several commands or manual source fusion | add a documented recipe first, then replace with a fused command if repeated |
| C | not currently reproducible from documented tooling | add a new command |

The target is to migrate important B-class claims either into documented recipes or new fused commands.

For review workflow, confidence, confounder handling, and step-by-step recipes, use `history-evidence-workflows.md`.

## Priority historical windows to reproduce

These are the first concrete evidence slices that should be reproducible because they are repeatedly referenced in the plans and recent notes.

### Heating priorities

1. **Overnight planner window** — first run the rolling 7-day-to-now `heating-history` review, then ask whether preheat started at the right time and whether Leather reached ≥ 20°C by 07:00 on each overnight segment
   - named regression anchor: `2026-04-02T00:00:00Z` → `2026-04-02T09:00:00Z` via `heating-history` (preheat start 03:06, DHW overlap 04:14:30–05:37:00, comfort miss from 05:56:59)
2. **DHW-interference window** — did DHW steal preheat or delay comfort recovery?
3. **Sawtooth window** — did the outer loop and inner loop fight each other, or was the behaviour a valid response to extra load?
4. **Door-open explanation window** — once door sensors are live, did door state explain the observed room underperformance?

### DHW priorities

1. **Morning charge window** — first run the rolling 7-day-to-now `dhw-history` review, then ask how `T1`, `HwcStorageTemp`, and charge completion evolved during representative charges inside that sweep
   - named regression anchor: `2026-04-02T05:00:00Z` → `2026-04-02T08:00:00Z` via `dhw-history` (completed 36 min top-up, then `T1` stayed ~45°C while `HwcStorageTemp` fell to 27°C and z2m-hub still estimated ~118 L remaining)
2. **Low-`T1` trigger window** — did the VRC 700 trigger based on low lower-cylinder temperature even while top-of-cylinder comfort remained high?
3. **Capacity / inflection window** — where did `T1` start to collapse, and how many usable litres were delivered?
4. **Eco vs normal comparison window** — how did charge duration, completion, and top temperature differ by inferred mode?

## Canonical evidence anchor windows

These are named, reproducible windows that can be reused across plan updates and future analysis.

### Heating anchors

| Window | Why it matters | Command |
|---|---|---|
| 2026-04-02 00:00–09:00 UTC | First reproducible overnight-planner window: likely preheat start at 03:06, DHW overlap 04:14:30–05:37:00, comfort miss from 05:56:59 onward, and likely sawtooth | `cargo run --bin heatpump-analysis -- heating-history --since 2026-04-02T00:00:00Z --until 2026-04-02T09:00:00Z` |
| Next clean overnight without major DHW overlap | Separate planner quality from DHW interference | `heating-history` once a representative window is nominated |
| Next clean doors-closed daytime window | Check whether sawtooth is real control instability or just disturbance response | `heating-history` once door context is available |

### DHW anchors

| Window | Why it matters | Command |
|---|---|---|
| 2026-04-02 05:00–08:00 UTC | Completed morning top-up, then large `T1` / `HwcStorageTemp` divergence while practical hot-water availability remained good | `cargo run --bin heatpump-analysis -- dhw-history --since 2026-04-02T05:00:00Z --until 2026-04-02T08:00:00Z` |
| Next partial / no-crossover charge | Validate partial-charge interpretation and gap-based remaining-litres model | `dhw-history` once a representative window is nominated |
| Next clear capacity / inflection window | Refresh the usable-litres evidence and collapse point | `cargo run --bin heatpump-analysis -- dhw-sessions --days 7 --format json` |
| Next inferred eco vs normal pair | Compare completion, duration, and peak temperatures by inferred mode | `dhw-history` on nominated windows |

## Nominated next anchor windows

These are the next windows worth curating into named anchors once representative examples are identified.

### Heating
- next **clean overnight** without major DHW overlap
- next **comfort-success overnight** where Leather reaches target by 07:00
- next **clean doors-closed daytime** window for sawtooth review

### DHW
- next **partial / no-crossover** charge
- next **low lower-cylinder / high-T1** trigger window
- next **eco vs normal** comparison pair
- next **draw during charging** window

## Gaps in current reproducibility

The current toolset is better than before, but several questions raised by the plans still need refinement.

## Heating gaps

`heating-history` now reconstructs a chosen heating window, but later iterations should improve provenance and add richer causal overlays.

Still needed:
- what the controller wanted
- what the plant did
- whether comfort was met
- whether DHW interfered
- whether behaviour looked stable or sawtoothed

Current maturity by heating question:
- **Overnight planner review** — **B moving toward A**. `heating-history` now uses compact pushdown-first / Flux-shaped summaries and Flux/state-change event construction for the main review facts; 2026-04-02 00:00–09:00 is the first canonical anchor.
- **More overnight data across temperatures** — **B**. Tooling exists, but the evidence base is still small.
- **Sawtooth diagnosis** — **B**. `heating-history` detects likely sawtooth, but disturbance-free windows are still needed before changing control logic.
- **Door-open impact** — **C moving toward B**. The doc/model story exists, but door sensors are not yet live in the evidence stream.
- **Pre-DHW banking** — **B/C**. Heating and DHW commands can already be combined, but joined interpretation still needs a clearer recipe.

Current state: the first pushdown-first / Flux-shaped fused command is now in place for heating-history, with compact summaries, compact controller-event shaping, cadence estimates, and batched compact numeric summaries. `dhw-history` now also batches DHW charge-period summaries and boundary lookups into a few compact multi-result Flux requests, and both `heating-history` and `dhw-history` support `--profile-queries` for raw Flux profiler output. `history-review` now defaults to structured output built from those compact summaries, with `--human` preserving the wrapper view. Testing also exposed two important operational findings: some older `adaptive_heating_mvp` rows omit the `tariff` tag, so controller-row reconstruction must tolerate missing tags; and the embedded `dhw_sessions` add-on is still day-rounded rather than exact-window bounded. The main remaining gap is no longer basic reconstruction; it is that the primary review output is still too fact-first and not decisive enough for the real operator question: are the recent heating and DHW changes working, and what should change next? Remaining work is now mainly exact-window session alignment, decision-first verdicts, explicit experiment scorecards, and boundary-stability coverage for decision-critical fields. Lower-value work such as richer profiler presentation, generic reporting features, and extra raw fields is now explicitly deprioritised.

## DHW gaps

`dhw-history` now reconstructs a chosen DHW window, but later iterations should improve controller-state provenance and inferred mode detection.

Still needed:
- how a charge evolved
- whether crossover happened
- how `T1` and `HwcStorageTemp` diverged
- whether the cylinder was practically full
- whether a threshold or trigger decision looked sensible

Current maturity by DHW question:
- **T1-based charge decisions** — **B moving toward A**. `dhw-history` is already good enough for repeated examples; 2026-04-02 05:00–08:00 is the first canonical anchor.
- **Summer mains temp repeat** — **A/B**. `dhw-sessions` already gives useful repeatable evidence, but monthly interpretation still needs lightweight operator judgement.
- **Legionella monitor** — **C**. Policy exists, but there is no dedicated reproducible hygiene command yet.
- **Eco/normal detection** — **B**. Some windows can be inferred from charge characteristics, but explicit mode evidence is still weak.
- **Predictive DHW compensation** — **B/C**. Needs joined use of `dhw-history` and `heating-history`, and clearer cross-window recipes.

Current state: first-pass command exists; event-boundary semantics have been improved and native-cadence DHW drill-down now exists. Some evidence may still be incomplete or warning-backed, especially around `remaining_litres` boundary attribution and boundary-stability validation.

## Query-efficiency expectations for new commands

New historical commands should follow official InfluxData guidance, not just the repo shorthand of "Flux-first".
In practice this means:
- prefer pushdown-capable query structure first (`range`, static `filter`, selector/aggregate)
- avoid introducing `map`, `pivot`, `union`, or `join` until the candidate row set is already small
- batch related summaries into fewer `/api/v2/query` calls when one multi-result query can return the same shaped facts
- profile expensive query shapes before declaring them "good enough"

Primary references:
- InfluxData, *Optimize Flux queries*: https://docs.influxdata.com/influxdb/v2/query-data/optimize-queries/
- InfluxData, *Query with the InfluxDB API*: https://docs.influxdata.com/influxdb/v2/query-data/execute-queries/influx-api/
- InfluxData, *Join data in InfluxDB with Flux*: https://docs.influxdata.com/influxdb/v2/query-data/flux/join/
- InfluxData, *Schema design*: https://docs.influxdata.com/influxdb/v2/write-data/best-practices/schema-design/

## Minimum provenance expectations for new commands

New fused commands should, at minimum:
- identify the time window explicitly
- include `warnings`
- note when a field is inferred rather than directly observed
- prefer structured output by default

Later improvements can add per-field provenance, for example:
- `source`
- `confidence`
- `resolution`
- `missing_inputs`

## Fused history commands added

The intended command boundary is:
- `heating-history` produces the fused heating evidence
- `dhw-history` produces the fused DHW evidence
- `history-review` summarises or combines those facts and should not become a separate raw-series reconstruction engine

## `heating-history`

Purpose:
- reconstruct a chosen heating window from fused historical inputs

Suggested interface:

```bash
date -u
export INFLUX_TOKEN=$(ak get influxdb)
cargo run --bin heatpump-analysis -- heating-history
cargo run --bin heatpump-analysis -- heating-history --human
```

Default output should be structured.

First-pass fields:
- time window
- Leather / Aldora min-max-latest
- outside min-max-latest
- heat curve min-max-latest
- target flow min-max-latest
- actual desired flow min-max-latest
- actual flow / return min-max-latest
- controller mode changes
- DHW overlap periods
- warnings for missing or stale evidence

First-pass event detection:
- comfort miss during waking hours
- likely overnight preheat start
- DHW during preheat
- likely sawtooth behaviour

For workflow and interpretation recipes, including the baseline overnight-planner example, use `history-evidence-workflows.md`.

## `dhw-history`

Purpose:
- reconstruct a chosen DHW window from fused historical inputs

Suggested interface:

```bash
date -u
export INFLUX_TOKEN=$(ak get influxdb)
cargo run --bin heatpump-analysis -- dhw-history
cargo run --bin heatpump-analysis -- dhw-history --human
```

Default output should be structured.

First-pass fields:
- time window
- charges detected
- `T1` start / peak / end
- `HwcStorageTemp` start / peak / end
- crossover yes / no
- remaining litres start / end if present
- `HwcSFMode`
- charging yes / no over the window
- warnings for suspicious or incomplete evidence

These fields should use explicit event-boundary semantics where applicable. In particular, `T1` / `HwcStorageTemp` charge summaries and pre/post remaining-litres values should represent charge-boundary facts, not accidental first/last values inside a broader outer review window.

First-pass event detection:
- no crossover
- low `T1`
- `HwcSFMode=load` stuck
- large `T1` / `HwcStorageTemp` divergence

For workflow and interpretation recipes, including the baseline T1-vs-lower-cylinder example, use `history-evidence-workflows.md`.

## Link to next steps in the heating plan

From `heating-plan.md`, the next steps need the following evidence.

| Next step | Evidence needed | Best command |
|---|---|---|
| Review overnight planner run | start with rolling 7-day-to-now evidence; then inspect preheat timing, Leather by 07:00, DHW overlap, target vs actual desired flow. Named anchor: 2026-04-02 00:00–09:00 showed preheat start 03:06, DHW overlap 04:14:30–05:37:00, and a comfort miss from 05:56:59 | `heating-history` |
| More overnight data | repeatable overnight windows across temperatures from the latest 7-day sweep | `heating-history` |
| Check outer/inner loop sawtooth | curve vs target vs actual desired flow over the latest 7-day sweep, then zoom in | `heating-history` |
| Replace `CurrentCompressorUtil` with power-based clamp | compare util vs power vs comfort outcomes over the latest 7-day sweep | `heating-history` |
| Pre-DHW banking | quantify comfort dip and recovery around DHW charges, starting from matched 7-day windows | `heating-history` + `dhw-history` |
| Door-open impact | door-state overlays with room response | `heating-history` once sensors are live |

## Link to next steps in the DHW plan

From `dhw-plan.md`, the next steps need the following evidence.

| Next step | Evidence needed | Best command |
|---|---|---|
| T1-based charge decisions | repeated examples of T1 vs HwcStorage vs trigger timing, starting from the latest rolling 7-day sweep. Named anchor: 2026-04-02 05:00–08:00 showed `T1` ~45°C with `HwcStorageTemp` at 27°C and ~118 L still remaining after a completed top-up | `dhw-history` |
| Summer mains temp repeat | rolling 7-day T2 / capacity / WWHR review, repeated monthly | `dhw-sessions` initially, later `dhw-history` summaries |
| Legionella monitor | turnover + sufficiently hot cycle history | later dedicated hygiene command |
| Eco/normal detection | inferred mode over charge windows from the latest rolling 7-day sweep | `dhw-history` |
| Predictive DHW compensation | DHW event linked to heating comfort dip, starting from matched 7-day windows | `dhw-history` + `heating-history` |

## Implementation order

### Phase 1 — document and stabilise what already exists

1. Keep `live-queries.md` as the canonical live query guide
2. Keep `current-production-state.md` as the live-state snapshot
3. Add links from plan docs to this history/evidence plan
4. Avoid adding more one-off historical observations to plan docs unless they include a reproducible path

### Phase 2 — add fused history commands

1. Add `heating-history` ✅
2. Add `dhw-history` ✅
3. Make outputs structured by default and `--human` optional ✅
4. Add explicit window arguments (`--since`, `--until`, later `--hours` if useful) ✅
5. Use rolling 7-day-to-now windows as the documented default investigation pattern ✅

### Phase 3 — connect historical claims back to commands

For each important historical claim in the plan docs:
- either point to an existing command
- or point to a new fused history command
- or explicitly mark the claim as not yet reproducible

## Success criteria

This plan is successful when:
- an LLM can regenerate the evidence behind the heating and DHW plans from documented commands
- live-state and historical-state tooling are clearly separated
- important historical claims no longer depend on hidden operator memory
- new plan updates can cite commands instead of embedding transient readings

## Boundaries

- Do not collapse all sources into one naive “single truth” stream
- Do not treat derived data as raw truth without labelling it appropriately
- Do not make human-readable output the default for new evidence commands
- Do not remove the strategic rationale from `heating-plan.md` or `dhw-plan.md`
- Do not rely on undocumented manual query recipes where a repeated task deserves a command
