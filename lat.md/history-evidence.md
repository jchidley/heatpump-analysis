# History Evidence

High-resolution history commands reconstruct fused heating and DHW evidence from InfluxDB for retrospective analysis and regression anchors.

## Purpose

These commands answer historical questions that live status cannot. They summarise compact evidence first, then support narrow event drilldowns only when needed.

The default investigation shape is a rolling 7-day window ending now. Use exact windows only for named anchors, regression checks, or one already-identified event.

## Heating History

`heating-history` builds a compact record of comfort, controller intent, and actuator response for one time window.

[[src/thermal/history.rs#heating_history_summary]] combines Leather and Aldora temperatures, outside temperature, heat curve, target flow, actual desired flow, actual flow, return temperature, controller mode changes, and controller events.

Derived event fields are designed for controller review rather than raw telemetry replay:

- `comfort_miss_periods` clip to waking hours only via [[src/thermal/history.rs#clip_period_to_waking_hours]]
- `likely_preheat_start` marks the first overnight-preheat style controller action
- `dhw_overlap_periods` detect heating windows contaminated by DHW demand
- `likely_sawtooth` flags repeated control alternation patterns

Default output is machine-readable JSON. `--human` is a compact operator summary, not the primary LLM interface.

## DHW History

`dhw-history` reconstructs one window of DHW evidence around cylinder state, charges, and comfort risk.

[[src/thermal/history.rs#dhw_history_summary]] summarises T1, `HwcStorageTemp`, estimated remaining litres, `HwcSFMode`, active charging state, and detected charge periods.

This command does not yet summarise `hmu HwcMode` or battery adequacy. Those are intended scheduler inputs and must currently be reviewed separately when analysing why one DHW launch time was preferred over another.

The key interpretation rule is that T1 and lower-cylinder storage temperature can diverge sharply after draws. Historical review therefore treats T1, remaining litres, and crossover evidence as separate signals rather than collapsing them into one number.

For one nominated event at native detail, use `dhw-drilldown` instead of widening the whole review window.

## History Review

`history-review` is a summarising layer over the heating and DHW history commands, not the primary reconstruction engine.

[[src/main.rs#run_history_review]] calls the compact history summaries first, then adds heuristic verdicts for heating and/or DHW. Structured JSON is the default machine interface.

When the review uses the default rolling window, it can also include day-rounded `dhw_sessions` context. Exact `since`/`until` windows omit that extra summary because it could pull evidence from outside the requested window.

## Boundaries

Historical review is for retrospective evidence, not live control decisions.

- Start with the default 7-day-to-now window before selecting a special case
- Prefer `heating-history` / `dhw-history` over manual source stitching
- Treat `history-review` verdicts as heuristic summaries, not final truth
- Separate direct observations, derived fields, and warning-backed inferences when promoting facts into plan docs or `lat.md/`
