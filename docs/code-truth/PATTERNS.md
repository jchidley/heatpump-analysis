<!-- code-truth: 296afce -->

# Patterns

## Configuration: TOML + Global Singleton (main CLI)

All domain constants live in `config.toml` and are accessed via `config::config()`. Singleton initialised once in `main()`.

**Cost to break**: Every analysis module depends on `config()`. Replacing with dependency injection would touch every function.

**Exception**: `fill_gap_interpolate()` in gaps.rs uses hardcoded feed ID strings. `ERA5_BIAS_CORRECTION_C` in octopus.rs is a Rust constant.

## Configuration: Separate TOML (thermal module)

`thermal.rs` loads its own `ThermalConfig` from `model/thermal-config.toml`. Not connected to the `config.rs` singleton.

**Cost to break**: Merging into `config.toml` would couple thermal commands to the emoncms config structure.

## Configuration: Separate TOML (adaptive heating MVP)

`adaptive-heating-mvp` loads its own `Config` from `model/adaptive-heating-mvp.toml`. Fully independent of both `config.rs` and `ThermalConfig`. Includes baseline values, eBUS host, InfluxDB connection, room topics, inner loop tuning parameters.

**Cost to break**: None — this is intentionally standalone. Phase 1b will add a dependency on the thermal solver via `src/lib.rs`, but config remains separate.

## Two-Loop Control Architecture (V2)

The adaptive controller uses two nested loops:
- **Outer loop** (900s): forecast + model → target_flow_c. Writes initial curve guess.
- **Inner loop** (60s): proportional feedback on `Hc1ActualFlowTempDesired` toward target_flow_c. Converges in 1 tick (~60s).

The outer loop sets `state.target_flow_c`. The inner loop reads it. When `target_flow_c` is `None` (overnight coast), the inner loop does nothing.

**Cost to break**: The two loops are tightly coupled through `RuntimeState.target_flow_c`. Changing one requires understanding the other.

## Analysis Functions: DataFrame In, Stdout Out

Every analysis function follows the same shape: receive an enriched Polars DataFrame, compute via lazy queries, print to stdout. No return values for composition. Output format is human-readable text tables.

**Cost to break**: Would need to add return types to ~15 analysis functions.

## Thermal Module: Typed Errors, Structured Artifacts

All thermal functions return `ThermalResult<T>`. JSON artifacts are produced by `artifact.rs` with git SHA, config hash, and timestamp. Artifacts go to `artifacts/thermal/`.

**Cost to break**: Would need to convert all `?` chains. Artifact schema is consumed by regression CI.

## Adaptive Heating MVP: eBUS via Raw TCP

The MVP talks to ebusd via raw TCP (`TcpStream` to localhost:8888). Sends a command string, reads the response. No library abstraction — just formatted strings and response parsing.

**Cost to break**: Low — the TCP protocol is trivial. When the Pico W eBUS adapter replaces ebusd, this is the code that changes.

## Adaptive Heating MVP: Blocking Control Loop + Async HTTP

The MVP uses a split architecture:
- Background thread runs a blocking control loop (sleep + `run_outer_cycle()`/`run_inner_cycle()` with blocking reqwest)
- Main thread runs Tokio + Axum for the HTTP API
- Shared state via `Arc<Mutex<RuntimeState>>`

**Cost to break**: Would need to either make the control loop fully async or restructure the shared state.

## Adaptive Heating MVP: Mode-Driven Control

Each control mode (`Occupied`, `ShortAbsence`, `AwayUntil`, `Disabled`, `MonitorOnly`) has its own branch in `run_outer_cycle()`. Mode is persisted as TOML in the state file and can be changed via HTTP API.

**Cost to break**: Adding a new mode requires a new enum variant, new branch in run_outer_cycle(), new HTTP handler, and z2m-hub proxy route + dashboard button.

## eBUS Command Pattern

eBUS reads and writes use a consistent pattern:
- `ebusd_read(config, circuit, register)` → `Result<String>`
- `ebusd_write(config, circuit, register, value)` → `Result<String>`
- Both use `ebusd_command()` which opens a TCP connection, sends one command, reads response

Each call is a separate TCP connection (no persistent connection).

**Cost to break**: If eBUS commands need batching or persistent connections, all callers change.

## Logging: Dual Sink (InfluxDB + JSONL)

Every control decision is logged to both InfluxDB (for dashboards/analysis) and local JSONL (for agent inspection/audit/replay). Missing sensor values are omitted from InfluxDB line protocol (not written as 0).

**Cost to break**: Low — the two sinks are independent. Either can be removed or replaced.

## Decision Log: Full Before/After State

Each `DecisionLog` captures the complete before-state (curve, room temps, HP state, compressor, power, yield, flow, return, tariff period, forecast conditions) and the after-state (what was written, target_flow, model calculations). This enables post-hoc analysis of every decision's context and effect.

**Cost to break**: Adding a new logged field requires updating the struct, the InfluxDB write, and the JSONL serialisation. All in one file.

## VRC 700 Heat Curve Formula

`flow = setpoint + curve × (setpoint - outside)^exponent` where exponent=1.25 (measured, not the Vaillant manual value of 1.10). Inverse: `curve = (flow - setpoint) / (setpoint - outside)^exponent`.

The formula is used only as the initial guess. The inner loop converges regardless of formula accuracy.

**Cost to break**: Changing the exponent only affects initial guess quality. Inner loop compensates.
