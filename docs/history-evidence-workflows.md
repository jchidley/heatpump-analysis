# History Evidence Workflows

This document is the **how-to guide** for reconstructing historical evidence behind the heating and DHW plans. The canonical command boundaries and default review shape now live in `lat.md/history-evidence.md`; this file keeps the operator workflow and examples.

Use this alongside:
- `heating-plan.md` — space-heating strategy, questions to answer, and live heating checks
- `dhw-plan.md` — DHW strategy, questions to answer, and live DHW checks

## Before you start

Historical fused commands in this repo often need InfluxDB access.

```bash
export INFLUX_TOKEN=$(ak get influxdb)
```

This is typically required for:
- `cargo run --bin heatpump-analysis -- heating-history ...`
- `cargo run --bin heatpump-analysis -- dhw-history ...`
- `cargo run --bin heatpump-analysis -- dhw-sessions ...`

## Standard investigation window (default for both heating and DHW)

Unless a command forces a different shape, **all historical investigations should default to a rolling 7-day window ending now**.

Always anchor the review with an explicit current UTC timestamp first:

```bash
date -u
```

Then use the history commands directly — they now default to **last 7 days ending now**:

```bash
cargo run --bin heatpump-analysis -- heating-history
cargo run --bin heatpump-analysis -- dhw-history
cargo run --bin heatpump-analysis -- dhw-sessions --days 7 --format json
```

Use a shorter or custom window only when:
- the command itself requires it
- you are drilling into one already-identified event inside the 7-day review
- you are reproducing a named historical anchor for documentation or regression checking

## How to use this guide

1. **Decide whether the question is live or historical**
   - live heating now → use the live heating checks in `heating-plan.md`
   - live DHW now → use the live DHW checks in `dhw-plan.md`
   - chosen past window → stay in this guide
2. **Pick the domain**
   - heating
   - DHW
   - joined heating + DHW interaction
3. **Start with the rolling 7-day-to-now window**
   - confirm `date -u` first
   - use the built-in 7-day-to-now default as the first pass for both heating and DHW
4. **Replay a named anchor window if needed**
   - reuse a documented window when validating a named claim or regression case
   - nominate a new window only when no anchor fits the question
5. **Only then drill into one specific event if needed**
   - drill-down is for one nominated charge / overlap / comfort-miss / disturbance event
   - do not start by pulling native-resolution detail for the whole review window
   - for DHW-native detail, use `cargo run --bin heatpump-analysis -- dhw-drilldown --since ... --until ...`
6. **Run the fused command or recipe**
   - prefer `heating-history` / `dhw-history` over manual source stitching
   - treat `history-review` as a summarising layer over those facts, not the primary reconstruction engine
   - note that `history-review` default structured output is the real LLM/machine interface
   - use `--human` only for a compact operator summary, not for LLM workflows
   - its scorecard logic is still an early heuristic layer rather than the final explicit experiment model
7. **Check the listed fields and warnings**
   - separate direct observations, inferred fields, and missing inputs
8. **Assess confounders before drawing conclusions**
9. **Classify the result**
   - confirms the plan
   - weakly supports the plan
   - contradicts the plan
   - inconclusive / needs a cleaner window

## Window review template

When reviewing output, explicitly note which important facts are:
- **directly observed**
- **derived / interpreted**
- **warning-backed because evidence is missing, stale, or incomplete**

## Window review template

Use this structure when reviewing a historical window for a plan change or for a new canonical anchor.

| Field | What to record |
|---|---|
| Window | exact `since` / `until` |
| Question | what you are trying to prove or disprove |
| Command | exact command(s) run |
| Key observations | the important raw outputs |
| Confounders | DHW overlap, doors open, missing fields, inferred mode, etc. |
| Confidence | high / medium / low |
| Interpretation | what the window actually supports |
| Impact on plan | keep / strengthen / weaken / no change yet |
| Next evidence needed | what cleaner or repeat windows are still needed |

## Confounders and confidence

Before concluding that a controller, model, or operational rule is wrong, check whether the window is contaminated by stronger explanations.

Common confounders:
- **DHW overlap** during heating windows
- **door-open or occupancy disturbances** during room-comfort windows
- **missing or sparse samples** in controller, eBUS, or Influx-backed fields
- **derived/inferred mode detection** where direct controller truth is unavailable
- **single-window bias** where a claim is based on one dramatic trace rather than a repeatable pattern

Use this confidence guide:
- **High** — key fields present, confounders small, interpretation mostly direct, repeated or strongly representative window
- **Medium** — some inferred fields or moderate confounders, but still operationally useful
- **Low** — major overlap/disturbance, sparse data, or conclusion depends mostly on inference

A valid outcome of a review is:
- **supports change**
- **supports no change yet**
- **needs cleaner window**
- **contradicted**

## Heating workflows

### Assess whether the overnight planner worked or was disrupted

Run:

```bash
date -u
export INFLUX_TOKEN=$(ak get influxdb)
cargo run --bin heatpump-analysis -- heating-history
```

Check:
- `events.likely_preheat_start`
- `events.dhw_overlap_periods`
- `events.comfort_miss_periods`
- `leather_c`
- `target_flow_c`
- `actual_flow_desired_c`
- `events.likely_sawtooth`
- `warnings`

Interpretation:
- if preheat starts at a sensible time but a long DHW overlap follows, treat the window first as a **planner + DHW interaction** question, not immediately as a planner failure
- if Leather still misses comfort without major DHW overlap, that is stronger evidence that the planner/reheat model needs retuning
- if target flow, actual desired flow, and curve alternate aggressively, mark the window as a sawtooth candidate — but do not retune from one disturbance-heavy window alone
- always separate **controller intent**, **actuator response**, and **comfort outcome**

Default review pattern:

```bash
date -u
export INFLUX_TOKEN=$(ak get influxdb)
cargo run --bin heatpump-analysis -- heating-history
```

For a named historical regression anchor, you may still use a fixed window such as:

```bash
cargo run --bin heatpump-analysis -- heating-history \
  --since 2026-04-02T00:00:00Z --until 2026-04-02T09:00:00Z
```

That anchor window showed:
- likely preheat start at **03:06**
- DHW overlap from **04:14:30–05:37:00**
- likely sawtooth behaviour
- Leather comfort miss from **05:56:59** onward

Use `heating-plan.md` for why the anchor matters and what decision it is meant to inform.

Query-efficiency note:
- treat these commands as **pushdown-first InfluxDB reviews**, not just "Flux-first" wrappers
- prefer the fused history commands because they are where query shaping, batching, and operator-cost discipline should live
- official optimisation references: InfluxData, *Optimize Flux queries* (<https://docs.influxdata.com/influxdb/v2/query-data/optimize-queries/>) and *Query with the InfluxDB API* (<https://docs.influxdata.com/influxdb/v2/query-data/execute-queries/influx-api/>)

## DHW workflows

### Assess whether lower-cylinder hysteresis matches practical comfort

Run:

```bash
date -u
export INFLUX_TOKEN=$(ak get influxdb)
cargo run --bin heatpump-analysis -- dhw-history
```

Check:
- `charges_detected[*].crossover`
- `t1_c`
- `hwc_storage_c`
- `remaining_litres`
- `events.large_t1_hwc_divergence`
- `warnings`

Interpretation:
- if `crossover=true`, the charge completed by the operational rule in the DHW plan
- if `T1` remains high while `HwcStorageTemp` collapses, lower-cylinder hysteresis is **not** a direct comfort truth
- if `remaining_litres` stays materially positive with high `T1`, the cylinder may still be practically fine for showers even when the lower sensor looks cold
- if warnings indicate large divergence, treat that as evidence in favour of T1-based trigger logic rather than as a sensor fault by default

Default review pattern:

```bash
date -u
export INFLUX_TOKEN=$(ak get influxdb)
cargo run --bin heatpump-analysis -- dhw-history
```

For a named historical regression anchor, you may still use a fixed window such as:

```bash
cargo run --bin heatpump-analysis -- dhw-history \
  --since 2026-04-02T05:00:00Z --until 2026-04-02T08:00:00Z
```

That anchor window showed:
- a completed **36 min** top-up charge
- `T1` rising to ~45.5°C
- later `HwcStorageTemp` falling to **27°C**
- z2m-hub still estimating **~118 L** remaining

Use `dhw-plan.md` for why the anchor matters and how it fits the broader DHW strategy.

## Joined workflows

### Was a heating comfort miss caused by DHW overlap?

Run both windows over the same period:

```bash
date -u
export INFLUX_TOKEN=$(ak get influxdb)
cargo run --bin heatpump-analysis -- heating-history
cargo run --bin heatpump-analysis -- dhw-history
```

Check:
- `events.likely_preheat_start` in `heating-history`
- `events.dhw_overlap_periods` in `heating-history`
- `events.comfort_miss_periods` and `leather_c` in `heating-history`
- `charges_detected`, `crossover`, `t1_c`, and `remaining_litres` in `dhw-history`

Interpretation:
- if the comfort miss aligns with a long DHW overlap, treat the window first as a **heating + DHW interaction** question
- if DHW completed cleanly and consumed most of the preheat window, the evidence supports coordination changes before retuning the heating planner itself
- if there is little or no DHW overlap, a comfort miss is stronger evidence against the heating planner/reheat assumptions

### Should a historical observation be promoted into a plan doc?

Promote an observation into `heating-plan.md` or `dhw-plan.md` only if:
1. it is reproducible from a documented command or recipe
2. it is either representative **or** clearly labelled as a named anchor/example
3. confounders are stated explicitly
4. the conclusion is operationally relevant, not just visually interesting

Otherwise keep it in ad hoc analysis notes until the evidence is stronger, then promote it into `heating-plan.md`, `dhw-plan.md`, or this workflow doc when it becomes operationally useful.
