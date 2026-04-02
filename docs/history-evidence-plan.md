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
- **Current live state:** `current-production-state.md`
- **Current live query commands:** `live-queries.md`
- **Code locations / implementation structure:** `code-truth/README.md`, `code-truth/REPOSITORY_MAP.md`, `code-truth/ARCHITECTURE.md`
- **Secrets handling:** `../deploy/SECRETS.md`

This document is intentionally self-contained apart from those references.

## Objective

Make the evidence behind the heating and DHW plans **reproducible on demand** for both humans and LLMs.

The desired end state is:
1. plan docs explain **what matters and why**
2. evidence commands reconstruct **what happened** for a chosen period
3. live-query commands show **what is happening now**
4. no important historical conclusion depends on undocumented operator memory

## Principles

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

Add `--human` when a more readable operator summary is useful.

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
- optional `source` and `confidence` fields later if needed

### Use an existing command when it is already good enough

Not every evidence need requires a new command.

There are two valid reproducibility paths:
1. **document an existing command or recipe** when one command already answers the question well enough for an LLM
2. **add a new fused command** when the answer currently requires stitching together multiple tools or undocumented operator knowledge

The standard should be: an LLM should be able to reproduce the evidence from this document and its references without hidden context.

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

These already have good-enough commands and should be documented rather than replaced.

| Topic | Command | Notes |
|---|---|---|
| Current heating state | `cargo run --bin adaptive-heating-mvp -- --config model/adaptive-heating-mvp.toml status` | structured by default |
| Current heating state (human) | `... status --human` | operator view |
| Current DHW state | `cargo run --bin heatpump-analysis -- dhw-live-status` | structured by default |
| Current DHW state (human) | `... dhw-live-status --human` | operator view |
| Historical DHW session analysis | `cargo run --bin heatpump-analysis -- dhw-sessions --days 14 --format json` | inflection / capacity / draw type evidence |
| Fused heating history | `cargo run --bin heatpump-analysis -- heating-history --since ... --until ...` | structured by default; add `--human` for operator view |
| Fused DHW history | `cargo run --bin heatpump-analysis -- dhw-history --since ... --until ...` | structured by default; add `--human` for operator view |
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

## Priority historical windows to reproduce

These are the first concrete evidence slices that should be reproducible because they are repeatedly referenced in the plans and recent notes.

### Heating priorities

1. **Overnight planner window** — did preheat start at the right time, and was Leather ≥ 20°C by 07:00?
2. **DHW-interference window** — did DHW steal preheat or delay comfort recovery?
3. **Sawtooth window** — did the outer loop and inner loop fight each other, or was the behaviour a valid response to extra load?
4. **Door-open explanation window** — once door sensors are live, did door state explain the observed room underperformance?

### DHW priorities

1. **Morning charge window** — how did `T1`, `HwcStorageTemp`, and charge completion evolve during a representative charge?
2. **Low-`T1` trigger window** — did the VRC 700 trigger based on low lower-cylinder temperature even while top-of-cylinder comfort remained high?
3. **Capacity / inflection window** — where did `T1` start to collapse, and how many usable litres were delivered?
4. **Eco vs normal comparison window** — how did charge duration, completion, and top temperature differ by inferred mode?

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

Current state: first-pass command exists; some fields are still inferred heuristically.

## DHW gaps

`dhw-history` now reconstructs a chosen DHW window, but later iterations should improve controller-state provenance and inferred mode detection.

Still needed:
- how a charge evolved
- whether crossover happened
- how `T1` and `HwcStorageTemp` diverged
- whether the cylinder was practically full
- whether a threshold or trigger decision looked sensible

Current state: first-pass command exists; some evidence may still be incomplete or warning-backed.

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

## `heating-history`

Purpose:
- reconstruct a chosen heating window from fused historical inputs

Suggested interface:

```bash
cargo run --bin heatpump-analysis -- heating-history --since 2026-04-02T00:00:00Z --until 2026-04-02T09:00:00Z
cargo run --bin heatpump-analysis -- heating-history --since ... --until ... --human
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

## `dhw-history`

Purpose:
- reconstruct a chosen DHW window from fused historical inputs

Suggested interface:

```bash
cargo run --bin heatpump-analysis -- dhw-history --since 2026-03-21T05:00:00Z --until 2026-03-21T08:00:00Z
cargo run --bin heatpump-analysis -- dhw-history --since ... --until ... --human
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

First-pass event detection:
- no crossover
- low `T1`
- `HwcSFMode=load` stuck
- large `T1` / `HwcStorageTemp` divergence

## Link to next steps in the heating plan

From `heating-plan.md`, the next steps need the following evidence.

| Next step | Evidence needed | Best command |
|---|---|---|
| Review overnight planner run | preheat timing, Leather by 07:00, DHW overlap, target vs actual desired flow | `heating-history` |
| More overnight data | repeatable overnight windows across temperatures | `heating-history` |
| Check outer/inner loop sawtooth | curve vs target vs actual desired flow over time | `heating-history` |
| Replace `CurrentCompressorUtil` with power-based clamp | compare util vs power vs comfort outcomes | `heating-history` |
| Pre-DHW banking | quantify comfort dip and recovery around DHW charges | `heating-history` + `dhw-history` |
| Door-open impact | door-state overlays with room response | `heating-history` once sensors are live |

## Link to next steps in the DHW plan

From `dhw-plan.md`, the next steps need the following evidence.

| Next step | Evidence needed | Best command |
|---|---|---|
| T1-based charge decisions | repeated examples of T1 vs HwcStorage vs trigger timing | `dhw-history` |
| Summer mains temp repeat | monthly T2 / capacity / WWHR shifts | `dhw-sessions` initially, later `dhw-history` summaries |
| Legionella monitor | turnover + sufficiently hot cycle history | later dedicated hygiene command |
| Eco/normal detection | inferred mode over charge windows | `dhw-history` |
| Predictive DHW compensation | DHW event linked to heating comfort dip | `dhw-history` + `heating-history` |

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
