<!-- code-truth: 9c24a09 -->

# Decisions

## Structural Decisions

### D1: Flow rate for state classification (not temperature or flags)

**Status:** active

**What**: Classify operating states using flow rate thresholds with hysteresis.

**Where**: `analysis.rs::classify_states()`, `config.toml` `[thresholds]`

**Alternatives rejected**: Flow temperature, DHW_flag feed (dead), eBUS StatuscodeNum (unreliable — code 134 appears during both standby and DHW).

**Consequences**: Any new classification approach must be validated against 448k+ running samples.

### D2: External TOML configuration

**Status:** active

**What**: All domain constants in `config.toml`, thermal config in `model/thermal-config.toml`, adaptive heating in `model/adaptive-heating-mvp.toml`. Three independent config files for three independent concerns.

### D3: Thermal geometry in JSON, not TOML

**Status:** active

**What**: Room dimensions, fabric, connections in `data/canonical/thermal_geometry.json`. Single source of truth for both Rust thermal model and (formerly) Python.

### D4: Typed errors in thermal module, anyhow at CLI boundary

**Status:** active

### D5: Adaptive heating MVP is a separate binary, not a subcommand

**Status:** active, evolving

**What**: `src/bin/adaptive-heating-mvp.rs` is a standalone binary with its own config, own dependencies (Axum, Tokio), and own deployment path.

**Why**: Different runtime model (long-lived service vs short-lived CLI), different deployment (pi5data vs dev machine), different dependency profile (async vs blocking).

**Evolution**: Phase 1b created `src/lib.rs` to share the thermal solver (`bisect_mwt_for_room`) with the adaptive controller. The binary remains separate but depends on the thermal module. On pi5data, the Cargo.toml uses `[lib] name = "heatpump_analysis"` so the import path matches. Thermal source files + `thermal_geometry.json` are synced to pi5data for compilation.

### D6: VRC 700 as steerable state machine, not replaced

**Status:** active

**What**: The adaptive controller writes strategic inputs to the VRC 700 (`Hc1HeatCurve`, `Z1OpMode`, `HwcSFMode`) and observes its downstream behaviour via `Hc1ActualFlowTempDesired`. It does not bypass the VRC 700.

**Why**: VRC 700 handles 10-second heartbeat, safety fallbacks, valve control, VWZ AI communication. Replacing it requires sending SetMode every 10s — much higher complexity and risk.

**Where**: `docs/heating-plan.md`, `docs/pico-ebus-plan.md`

### D7: Baseline restore on stop/kill

**Status:** active

**What**: On shutdown, restore `Z1OpMode=auto` + `Hc1HeatCurve=0.55` + `Hc1MinFlowTempDesired=20`. VRC 700 resumes timer control with factory defaults.

**Where**: `src/bin/adaptive-heating-mvp.rs::restore_baseline()`, `model/adaptive-heating-mvp.toml` `[baseline]`

### D8: Z1OpMode=night (SP=19) + MinFlow=19 during V2 operation

**Status:** active

**What**: On startup, set `Z1OpMode=night` (value 3) + `Hc1MinFlowTempDesired=19`. VRC 700 uses `Z1NightTemp` (19°C) permanently. Disables Optimum Start, day/night transitions, timer interference. MinFlow=19 removes the hidden 20°C floor that prevented genuine coast.

**Why**: SP=19 with `Z1OpMode=off` gives genuine zero heating. Previous approach (curve 0.10 at MinFlow=20) still produced 20°C+ flow — the MinFlow floor was invisible until the first coast night.

**Where**: `src/bin/adaptive-heating-mvp.rs` startup + `control_loop()`, `docs/heating-plan.md`

### D9: Inner loop replaces all EMAs

**Status:** active

**What**: The inner loop (proportional feedback on `Hc1ActualFlowTempDesired`) is the only feedback mechanism. No `flow_offset` EMA, no `room_offset` EMA.

**Why**: `room_offset` ran away to +2.18°C overnight — it learned overnight cooling as model error, suppressed preheat target_flow by ~8°C. `flow_offset` was redundant with the inner loop. The inner loop converges in 1 tick regardless of model accuracy.

**Where**: `src/bin/adaptive-heating-mvp.rs::run_inner_cycle()`

## Pragmatic Decisions

### D10: Leather room as primary comfort target

**Status:** active

**What**: Leather room (emonth2 sensor) is the primary reference. Comfort band 20–21°C.

### D11: Conservatory excluded from optimisation

**Status:** active

**What**: 30m² glass, sub-hour time constant, massive solar/wind sensitivity.

### D12: DHW must remain socially reliable

**Status:** active, strategy evolving

**What**: DHW availability is a hard constraint. Phase 1a: Cosy windows + HwcStorageTemp < 40°C → `HwcSFMode=load`. Phase 2 will use Multical T1 for DHW decisions (0.01°C/2s at actual hot outlet vs VR10 NTC 0.5°C/30s at 600mm).

**Key finding**: eco mode fails below 5°C (95% hit 120-min timeout). Normal mode avg 60 min, works everywhere. See DHW duration model in `docs/dhw-plan.md`.

**Preferred Phase 2 strategy**: charge at 22:00 Cosy window, monitor T1 overnight (0.23°C/h standby drop, P75 of 47 measured segments), top up at 04:00 Cosy if T1 below comfort threshold. Cosy windows preferred to reduce battery pressure on cold days.

### D13: Legionella as monitored risk, not constant high setpoint

**Status:** active (monitoring not yet implemented)

### D14: Every control action is data

**Status:** active

**What**: Every write to the VRC 700 is an observation about system response. Data enables iterative improvement regardless of control logic quality.

### D15: VRC 700 curve is IEEE 754 float

**Status:** confirmed

**What**: `Hc1HeatCurve` stored as 32-bit float. 0.01 step = ~0.20°C flow change at SP=19, outside 7°C. Measured: 0.55→29.88°C, 0.56→30.08°C. No quantization to 0.05 steps.

**Consequence**: `round2()` to 0.01 precision in the controller code is appropriate. The inner loop's gain produces meaningful adjustments at this resolution.

## Open Questions

Empirical and hardware unknowns (OQ1–OQ6) now live in `../../lat.md/plan.md#Open Questions` where they can be tracked alongside other active items.
