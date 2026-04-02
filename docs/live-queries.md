# Live Queries

This document explains how to fetch **current live readings on demand** without embedding transient values into the plan docs.

Use this alongside:
- `current-production-state.md` — what is deployed/live now
- `heating-plan.md` — heating strategy and rationale
- `dhw-plan.md` — DHW strategy and rationale
- `history-evidence-workflows.md` — reconstructing past windows instead of querying live state
- `history-evidence-plan.md` — authority map and evidence roadmap
- `code-truth/README.md` — where the implementation lives

## Principles

- Treat live values as **queries**, not documentation.
- Keep plan docs for policy and interpretation, not daily snapshots.
- Prefer compact summaries over raw register dumps when checking current state.
- If the question is about a **past window**, stop and use `history-evidence-workflows.md` instead.

## Heating controller snapshot

### Preferred

Run on pi5data (or from this repo when the binary/config are present):

```bash
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

This command may warn in local dev if some live integrations are unavailable; for the smallest always-on runtime view, use `curl -s http://pi5data:3031/status`.

### Which heating live command should I use?

| Question | Best command | Why |
|---|---|---|
| What mode / target flow is the live controller using right now? | `curl -s http://pi5data:3031/status` | Smallest always-on runtime state |
| What richer live heating snapshot do I have right now? | `cargo run --bin adaptive-heating-mvp -- --config model/adaptive-heating-mvp.toml status` | Best compact snapshot when live inputs are available |
| Do actuator readings match controller intent? | adaptive CLI `status` | Includes curve, target flow, actual desired flow, flow/return |
| What does the raw Vaillant side say? | raw eBUS heating reads | Source register view |
| Am I checking live state or historical performance? | Live now → `status`; chosen past window → `heating-history` | Avoid mixing live queries with evidence reconstruction |

### Healthy recent example

Recent lightweight runtime state (2026-04-02):
- `mode = occupied`
- `target_flow_c ≈ 28.67`
- `last_reason = "HTTP occupied"`

Interpret this as: the live controller is running in normal occupied mode and exposing runtime state via the HTTP API.

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

These live DHW commands do **not** need `INFLUX_TOKEN`. They are suitable for quick operator checks and LLM live-state queries.

### Which DHW live command should I use?

| Question | Best command | Why |
|---|---|---|
| Do we have enough hot water right now? | `cargo run --bin heatpump-analysis -- dhw-live-status` | Best compact summary: litres, T1, charge state, two-shower safety |
| Is DHW actively charging right now? | `cargo run --bin heatpump-analysis -- dhw-live-status` or `curl -s http://pi5data:3030/api/dhw/status` | Fast yes/no plus controller-facing state |
| What does z2m-hub think in raw form? | `curl -s http://pi5data:3030/api/hot-water` | Underlying derived DHW state |
| What does the Vaillant controller think? | raw eBUS `HwcStorageTemp` / `HwcSFMode` | Source register view |
| Should I trust a low `HwcStorageTemp` reading on its own? | No — cross-check `t1_c`, `remaining_litres`, and charge state | Lower-cylinder temperature is not the same as usable hot-water truth |

### Structured form

```bash
cargo run --bin heatpump-analysis -- dhw-live-status
```

### Healthy recent example

Recent healthy live snapshot (2026-04-02):
- `charge_state = full`
- `t1_c ≈ 45.24`
- `hwc_storage_c ≈ 44.5`
- `remaining_litres = 198`
- `sfmode = auto`
- `warnings = []`

Interpret this as: cylinder is practically full, no boost is stuck, and the household is safe for two showers.

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

### Heating interpretation caveats
- A single live snapshot does **not** tell you whether the overnight planner worked; use `heating-history` for that.
- If the compact CLI `status` output is missing fields in local dev, fall back to `curl -s http://pi5data:3031/status` for runtime state and raw eBUS reads for actuator truth.
- When judging controller quality, distinguish between:
  - **controller intent** (`target_flow_c`, mode, action logs)
  - **actuator output** (`Hc1ActualFlowTempDesired`, heat curve)
  - **comfort outcome** (Leather / room temperatures)
- Reproducible anchor: `heating-history --since 2026-04-02T00:00:00Z --until 2026-04-02T09:00:00Z` showed preheat start at 03:06, DHW overlap 04:15–05:37, and a comfort miss despite the planner running.

## DHW quick checks

### DHW looks healthy when
- `t1_c` is present and plausible
- `remaining litres` is non-zero after a full charge
- charge state matches observed behaviour
- `HwcSFMode` is normally `auto` unless an active boost is in progress
- `warnings` is empty or only contains known benign gaps

### DHW interpretation caveat
- Do **not** treat a low `HwcStorageTemp` alone as proof that usable hot water is low.
- `HwcStorageTemp` is a lower-cylinder control signal, not the household comfort truth.
- For practical DHW availability, prefer `t1_c`, `remaining_litres`, crossover/charge state, and the `safe_for_two_showers` summary.
- Reproducible example: `dhw-history --since 2026-04-02T05:00:00Z --until 2026-04-02T08:00:00Z` showed `T1` ~45°C while `HwcStorageTemp` later fell to 27°C with ~118 L still remaining.

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
