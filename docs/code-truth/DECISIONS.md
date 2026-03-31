<!-- code-truth: 7b6bfed -->

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

**Where**: `config.rs`, `src/thermal/config.rs`, `src/bin/adaptive-heating-mvp.rs`

### D3: Thermal geometry in JSON, not TOML

**Status:** active

**What**: Room dimensions, fabric, connections in `data/canonical/thermal_geometry.json`. Single source of truth for both Rust thermal model and (formerly) Python.

**Where**: `data/canonical/thermal_geometry.json`, `src/thermal/geometry.rs`

### D4: Typed errors in thermal module, anyhow at CLI boundary

**Status:** active

**What**: `ThermalError` enum with `thiserror`. `anyhow` only in `main.rs`.

### D5: Adaptive heating MVP is a separate binary, not a subcommand

**Status:** active

**What**: `src/bin/adaptive-heating-mvp.rs` is a standalone binary with its own config, own dependencies (Axum, Tokio), and own deployment path. Not integrated into the main `heatpump-analysis` CLI.

**Why**: The MVP runs as a long-lived service on `pi5data`. The main CLI is a short-lived analysis tool run from a development machine. Different runtime models, different deployment, different dependency profiles (async vs blocking).

**Consequences**: No shared code between the analysis CLI and the adaptive controller. InfluxDB query patterns are duplicated (thermal/influx.rs vs adaptive-heating-mvp). This is intentional — the controller should be independently deployable.

### D6: VRC 700 as steerable state machine, not replaced

**Status:** active

**What**: The adaptive controller writes strategic inputs to the VRC 700 and observes its downstream behaviour. It does not bypass the VRC 700 or send commands directly to the VWZ AI / HMU.

**Why**: The VRC 700 handles the 10-second heartbeat, safety fallbacks, valve control, and VWZ AI communication. Replacing it requires sending SetMode every 10s — much higher complexity and risk. Steering its inputs is safer and sufficient for the current objectives.

**Where**: `docs/adaptive-heating-control.md`, `docs/pico-ebus-plan.md` (future option to replace documented but not planned)

### D7: Baseline restore on stop/kill

**Status:** active

**What**: When the MVP stops, crashes, or is manually killed, it restores known-good VRC 700 register values. The VRC 700 then operates on its own timer/curve schedule as before.

**Why**: The manually-tuned baseline works. The MVP is a pilot on top of a known-good foundation. If the pilot misbehaves, reverting to the baseline is always safe.

**Where**: `src/bin/adaptive-heating-mvp.rs::restore_baseline()`, `model/adaptive-heating-mvp.toml` `[baseline]`

## Pragmatic Decisions

### D8: Leather room as primary comfort target

**Status:** active

**What**: Leather room (emonth2 sensor) is the primary reference for control decisions. Comfort band 20–21°C. Aldora is fallback but must not drive control until its proxy band is derived from historical data.

**Why**: Leather is the room where "good" is known most clearly from lived experience.

**Where**: `src/bin/adaptive-heating-mvp.rs::run_control_cycle()` (Occupied branch), `docs/adaptive-heating-mvp.md`

### D9: Conservatory excluded from optimisation

**Status:** active

**What**: Conservatory is treated as a heat sink / boundary room, not a comfort target.

**Why**: 30m² glass, sub-hour time constant, massive solar/wind sensitivity. Including it would distort whole-house optimisation.

### D10: Trust the VRC 700's accepted range

**Status:** active

**What**: If the VRC 700 accepts a written value, the MVP treats it as safe. No additional software bounds in V1.

**Why**: The VRC 700 has its own internal validation, anti-cycling, and safety logic. Adding extra conservative bounds on top would be second-guessing an algorithm we can't inspect.

### D11: DHW must remain socially reliable

**Status:** active

**What**: DHW availability is a hard practical constraint. Cosy windows are preferred charging opportunities, but charging only happens when the cylinder actually needs it (HwcStorageTemp below trigger). No pointless reheats.

**Why**: If DHW breaks, the pilot fails regardless of heating efficiency improvements.

### D12: Legionella as monitored risk, not constant high setpoint

**Status:** active (monitoring not yet implemented)

**What**: Legionella control should not mean permanent high cylinder temperature. Instead, monitor turnover and stagnation, trigger targeted hygiene cycles only when risk rises.

**Where**: `docs/adaptive-heating-control.md`, `docs/dhw-fixes.md`

### D13: Every control action is data

**Status:** active

**What**: The house is a physical system. Every write to the VRC 700 is an observation about how the system responds. The pilot generates data whether or not the control logic is optimal.

**Why**: The realistic savings from smarter control are modest (£50–100/year). The primary value is better comfort, less cycling, and understanding the system. The data enables iterative improvement.

## Open Questions

### OQ1: What is Aldora's proxy comfort band?

Need to query historical data for Aldora temperature when Leather is in the 20–21°C band. Until this is derived, Aldora must not drive control.

### OQ2: Does the VRC 700 behave differently after many rapid writes?

The MVP writes at most once per 15 minutes per register. But we don't know if the VRC 700 accumulates state or changes behaviour after extended periods of external writes. This is part of what the pilot observes.

### OQ3: What does `CurrentCompressorUtil = -121` mean?

The register appears to use signed encoding that wraps negative. The raw value is not meaningful as a utilisation percentage. For cycling detection, `RunDataStatuscode` transitions are more reliable.

### OQ4: Can `Hc1HeatCurve` effect be verified when HP is in Standby?

During the control-surface testing session, `Hc1ActualFlowTempDesired` showed 0.0 during Standby. The curve effect was only confirmed when the HP was actively heating. This is expected but means the controller can't verify its curve writes took effect until the next heating cycle.
