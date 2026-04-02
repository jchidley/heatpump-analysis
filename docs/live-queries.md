# Live Queries

This document explains how to fetch **current live readings on demand** without embedding transient values into the plan docs.

Use this alongside:
- `current-production-state.md` — what is deployed/live now
- `heating-plan.md` — heating strategy and rationale
- `dhw-plan.md` — DHW strategy and rationale
- `code-truth/README.md` — where the implementation lives

## Principles

- Treat live values as **queries**, not documentation.
- Keep plan docs for policy and interpretation, not daily snapshots.
- Prefer compact summaries over raw register dumps when checking current state.

## Heating controller snapshot

### Preferred

Run on pi5data (or against the pi5data checkout):

```bash
cd ~/adaptive-heating-mvp
cargo run --bin adaptive-heating-mvp -- --config model/adaptive-heating-mvp.toml status
cargo run --bin adaptive-heating-mvp -- --config model/adaptive-heating-mvp.toml status --human
```

By default this prints a compact TOML snapshot intended for LLM/tool consumption. Add `--human` for a friendlier operator-oriented summary.

This structured snapshot contains:
- runtime mode / away-until / last reason
- current heat curve
- target flow and actual desired flow
- actual flow / return
- outside temp
- leather and aldora temps
- DHW T1 and HwcStorageTemp
- warning strings for missing/stale inputs

### Expected shape

```toml
[runtime]
mode = "occupied"
updated_at = "..."
last_reason = "..."
target_flow_c = 30.5

[service]
state_file = "..."
jsonl_log_file = "..."
runtime_age_minutes = 3

[heating]
current_curve = 0.56
target_flow_c = 30.5
actual_flow_desired_c = 30.1
actual_flow_c = 29.6
return_c = 25.8
outside_c = 8.4
leather_c = 20.1
aldora_c = 19.8
run_status = "heating compressor"

[dhw]
t1_c = 44.2
hwc_storage_c = 33.5
target_c = 45.0
trigger_c = 40.0
likely_active = false

warnings = []
```

## Heating runtime API status

For the lightweight HTTP runtime state only:

```bash
curl -s http://pi5data:3031/status
```

This is useful for:
- current mode
- away-until
- last update timestamp
- last reason
- current target flow

Use the CLI `status` command above for the richer on-demand snapshot.

## DHW live summary

### Preferred

Run from this repo:

```bash
cargo run --bin heatpump-analysis -- dhw-live-status
cargo run --bin heatpump-analysis -- dhw-live-status --human
```

By default this prints JSON intended for LLM/tool consumption. Add `--human` for a friendlier operator-oriented summary.

This queries z2m-hub and returns:
- charge state
- crossover achieved
- remaining litres / full litres
- effective T1 / actual T1
- HwcStorageTemp / target temp
- `sfmode`
- whether it is likely safe for two showers
- warning strings for suspicious or missing state

### Structured form

```bash
cargo run --bin heatpump-analysis -- dhw-live-status
```

### Raw z2m-hub endpoints

```bash
curl -s http://pi5data:3030/api/hot-water
curl -s http://pi5data:3030/api/dhw/status
```

Use these when you want the underlying source JSON rather than the repo-level summary.

## Raw eBUS checks

Use these when you want the source registers directly.

### Heating

```bash
echo 'read -c 700 Hc1HeatCurve' | nc -w 2 localhost 8888
echo 'read -c 700 Hc1ActualFlowTempDesired' | nc -w 2 localhost 8888
echo 'read -c 700 DisplayedOutsideTemp' | nc -w 2 localhost 8888
echo 'read -c hmu RunDataStatuscode' | nc -w 2 localhost 8888
echo 'read -c hmu RunDataFlowTemp' | nc -w 2 localhost 8888
echo 'read -c hmu RunDataReturnTemp' | nc -w 2 localhost 8888
```

### DHW

```bash
echo 'read -c 700 HwcStorageTemp' | nc -w 2 localhost 8888
echo 'read -c 700 HwcSFMode' | nc -w 2 localhost 8888
```

## Combined live snapshot script

Use the helper script in this repo for a quick multi-system view:

```bash
bash scripts/live-state.sh
```

It prints:
- adaptive-heating runtime API status
- adaptive-heating rich CLI status snapshot
- DHW live summary
- z2m-hub hot-water endpoints
- a few raw eBUS readings

## Healthy quick checks

### Heating looks healthy when
- `mode` is what you expect (`occupied`, `away_until`, etc.)
- `current_curve` is present
- `actual_flow_desired_c` is close to `target_flow_c`
- `leather_c` is plausible and changing over time
- `warnings` is empty or only contains known benign gaps

### DHW looks healthy when
- `t1_c` is present and plausible
- `remaining litres` is non-zero after a full charge
- charge state matches observed behaviour
- `HwcSFMode` is normally `auto` unless an active boost is in progress

## Safe fallback commands

### Heating

```bash
echo 'write -c 700 Z1OpMode 1' | nc -w 2 localhost 8888
echo 'write -c 700 Hc1HeatCurve 0.55' | nc -w 2 localhost 8888
```

### DHW boost stuck

```bash
echo 'write -c 700 HwcSFMode auto' | nc -w 2 localhost 8888
```
