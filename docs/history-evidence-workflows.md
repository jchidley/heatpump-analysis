# History Evidence Workflows

This document is the operator-facing how-to for reconstructing historical heating and DHW evidence. The canonical command boundaries, output intent, and review rules live in [`../lat.md/history-evidence.md`](../lat.md/history-evidence.md).

## Use this file for

- practical command recipes
- operator workflow
- review discipline for retrospective investigations

## Start with the canonical rules

Read first:

- [`../lat.md/history-evidence.md`](../lat.md/history-evidence.md)
- [Heating plan](heating-plan.md)
- [DHW plan](dhw-plan.md)

## Default investigation shape

Unless you have a named anchor or regression case, start with a rolling 7-day window ending now.

```bash
date -u
export INFLUX_TOKEN=$(ak get influxdb)
cargo run --bin heatpump-analysis -- heating-history
cargo run --bin heatpump-analysis -- dhw-history
cargo run --bin heatpump-analysis -- dhw-sessions --days 7 --format json
```

## Basic workflow

1. decide whether the question is live or historical
2. choose heating, DHW, or joined review
3. start with the default rolling 7-day window
4. only then drill into one named anchor or event if needed
5. separate direct observations from derived/inferred fields

## Confidence discipline

Before promoting a finding into a plan or `lat.md`, check for:

- DHW overlap contaminating a heating window
- door-open / occupancy disturbances
- sparse or missing controller/eBUS inputs
- overreliance on one dramatic trace

## Heating review recipe

```bash
date -u
export INFLUX_TOKEN=$(ak get influxdb)
cargo run --bin heatpump-analysis -- heating-history
```

Typical fields to inspect:

- `likely_preheat_start`
- `dhw_overlap_periods`
- `comfort_miss_periods`
- `leather_c`
- `target_flow_c`
- `actual_flow_desired_c`
- `likely_sawtooth`
- `warnings`

Typical questions:

- did the overnight planner start early enough?
- was a comfort miss real, or was it driven by DHW overlap or open-door disturbance?
- did the controller hold target flow sensibly, or did the inner loop chase noise?

## DHW review recipe

```bash
date -u
export INFLUX_TOKEN=$(ak get influxdb)
cargo run --bin heatpump-analysis -- dhw-history
```

Typical fields to inspect:

- `charges_detected`
- `crossover`
- `t1_c`
- `hwc_storage_c`
- `remaining_litres`
- `large_t1_hwc_divergence`
- `warnings`

Typical questions:

- did the charge actually deliver useful comfort, even if it never reached textbook crossover?
- was a timeout weather-driven, demand-driven, or genuinely poor performance?
- did household draw during charge explain an apparently odd result?

## Joined review recipe

```bash
date -u
export INFLUX_TOKEN=$(ak get influxdb)
cargo run --bin heatpump-analysis -- heating-history
cargo run --bin heatpump-analysis -- dhw-history
```

Use joined review when a comfort miss may really be a heating+DHW coordination issue rather than a pure heating-planner problem.

## Review template

| Field | What to record |
|---|---|
| Window | exact `since` / `until` |
| Question | what you are testing |
| Command | exact command run |
| Key observations | important raw outputs |
| Confounders | DHW overlap, door-open, missing fields, etc. |
| Confidence | high / medium / low |
| Interpretation | what the evidence actually supports |
| Impact on plan | keep / strengthen / weaken / no change |
| Next evidence needed | what would make the claim stronger |

## Named-anchor windows

Use exact `--since` / `--until` windows only when:

- reproducing a documented anchor
- checking a regression case
- drilling into one already-identified event

## Related documents

- [Heating plan](heating-plan.md)
- [DHW plan](dhw-plan.md)
- [`../lat.md/history-evidence.md`](../lat.md/history-evidence.md)
