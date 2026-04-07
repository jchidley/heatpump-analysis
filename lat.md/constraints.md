# Constraints

Hard rules, known pitfalls, and things that must not be changed without careful re-validation.

## Minimum Electrical Input Principle

The sole cost function is total electrical kWh drawn from the grid. All control decisions — overnight trajectory, coast/heat switching, flow-temp selection, DHW scheduling — must minimise this.

COP is a derived intermediate, not a target. If electrical input is minimised, COP is necessarily good. Thermal energy delivered is an output of the physics, not a goal. Do not optimise for “best COP” or “lowest thermal energy” — these can conflict with minimum electrical cost (e.g. coasting saves thermal energy but forces higher flow during reheat, destroying COP and increasing electrical cost).

## Boundaries

Invariants that protect system integrity. Violating these risks silent data corruption or control failures.

- Don't change operating state thresholds without re-validating the full dataset
- Don't mix simulated and real data by default
- Don't commit `heatpump.db` or API keys
- Don't modify `~/github/octopus/` from this project
- Don't modify monitoring infrastructure from here — use SSH to devices directly
- Don't tune Cd or landing ACH independently — they are jointly calibrated
- `thermal_geometry.json` is source of truth for rooms/geometry (consumed by Rust + Python). `config.toml` radiators must match.
- Rust thermal outputs are authoritative when a CLI command exists; Python is for exploratory analysis only
- Thresholds are 5kW-specific — 7kW model's heating rate (20 L/min) overlaps 5kW DHW rate
- 45°C max flow on heating — emitter capacity and COP limit
- No heating above 17°C outside — empirically, solar/internal gains are sufficient
- No runtime learning — `room_offset` EMA ran away to +2.18°C overnight (learned cooling trend as "model error", suppressed preheat by ~8°C). Static calibration only.
- **During a Cosy window, battery SoC / headroom must never gate heating or DHW decisions.** Grid electricity is at its cheapest — it's the best time to run either. The headroom signal only gates decisions in non-Cosy windows (00:00–04:00, 07:00–13:00, 16:00–22:00). The energy-hub headroom value is also unreliable during Cosy because it projects base-load drain from current SoC without accounting for active grid charging.

## InfluxDB-First Analysis

All ad-hoc data analysis must push filtering, aggregation, pivoting, and arithmetic into Flux queries. Client-side code (Python, shell) handles only final formatting and display.

- **Filter and aggregate in Flux**: `filter()`, `aggregateWindow()`, `pivot()`, `map()`, `group()`, `sort()`, `difference()`, `movingAverage()` — use these server-side, not after fetching raw rows.
- **Return only the columns you need**: use `keep()` / `drop()` to minimise transfer. Don't fetch all fields then ignore half in Python.
- **Compute derived values in Flux where possible**: COP (`yield / elec`), ΔT (`flow - return`), rates of change (`derivative()`) — Flux can do these.
- **Client code is for presentation only**: formatting tables, adding emoji markers, printing summaries. If you're writing a `for` loop that filters or aggregates fetched rows, move that logic into the Flux query.
- **Why**: InfluxDB indexes time-series data and evaluates Flux on compressed blocks. Pulling raw 15-second data to the client then filtering wastes bandwidth and is fragile (CSV column ordering, tag vs field confusion, empty-string handling). Flux queries are also self-documenting and reproducible.

## Code Gotchas

Non-obvious code behaviours that have caused bugs or confusion.

- Static domain constants belong in `config.toml` — edit there, not in code. Exception: Octopus unit rates are derived at runtime from the account API via `src/octopus_tariff.rs`; only `battery_coverage` remains in `config.toml`.
- `gaps.rs` bypasses `db.rs` and writes to SQLite directly. `fill_gap_interpolate()` has hardcoded feed IDs
- `ERA5_BIAS_CORRECTION_C` is a Rust constant in `octopus.rs`, not in `config.toml`
- `--all-data` start timestamp is hardcoded in `resolve_time_range()`, duplicating the `config.toml` value
- Polars pinned to 0.46 (0.53 available) — untested on newer versions
- `octopus.rs` reads from `~/github/octopus/data/` — path is hardcoded
- Two HDD base temps exist: 15.5°C (UK standard, in config) vs 17°C (gas-era regression)
- Two binaries — use `cargo run --bin heatpump-analysis -- ...` for thermal commands
- `cosy-scheduler` binary removed from pi5data (2026-03-30). Source in `src/bin/cosy-scheduler.rs` kept for reference. Do not deploy.
- `ebusd-poll.sh` uses `nc | head -1` to avoid ebusd TCP hanging
- DHW auto-trigger removed Mar 2026. `scripts/dhw-auto-trigger.py` is buggy legacy — do not deploy. DHW boost via z2m-hub.
- `Hc1ActualFlowTempDesired` reads 0.0 during HP standby — inner loop must guard against this or it ramps the curve to max
- Cross-compiling for pi5data (aarch64) from WSL2 fails: `gnu` target needs GLIBC 2.39 but pi5data has 2.36 (bookworm). `musl` target exists but is slow to build. Current workflow: dev on laptop (`cargo check`), release build natively on pi5data via `scripts/sync-to-pi5data.sh`. See [[architecture#Implicit Contracts#Deployment Workflow]].
- ebusd container has no persistent volumes — `docker restart` re-downloads config CSVs from CDN. If CDN is unreachable, message definitions are lost. Always use `docker compose restart` (recreates properly) not bare `docker restart`.

## Sensor Gotchas

Known sensor issues that affect data interpretation.

- SNZB-02P v2.1.0 bug: readings freeze at power-on value. v2.2.0 fixes it. Always verify readings vary after deployment.
- `conservatory_temp_humid` Zigbee device has been removed from Z2M and will be re-paired as `outside_temp_humid` when deployed outdoors. Conservatory temperature now uses `ebusd/poll/Z2RoomTemp` (VRC 700 Zone 2 sensor, reads ~1°C below old SNZB-02P position). Updated in `thermal_geometry.json`.
- emonth2 in Leather reports humidity (`emon/emonth2_23/humidity`, `_field="value"`). Useful for overnight moisture analysis (Leather + dog). External temperature probe port is unconnected (reads 0).
- Bathroom sensor was in the airing cupboard until 25 Mar 2026 21:00 — historical data reads ~3°C high before that date
- PV calibration factor 0.087 is for the sloping plane; divide by 1.4 for vertical. P3 CT reads 6.7 kW for a 3.08 kWp array (includes Powerwall).
- Feed `503101` (indoor_temp) is the emonth2 in Leather only, not a whole-house average
- `CurrentCompressorUtil` reads negative values (−29, −55, −89, −102). Unreliable — do not use for control.

## System Pressure

`ebusd/poll/FlowPressure` is the correct register for water system pressure.

- Heating: 2.02 bar, DHW: 1.91 bar, Idle: 2.05 bar
- The 0.11 bar DHW dip is a hydraulic circuit volume effect (3-way valve switches from large radiator circuit to smaller cylinder coil), not thermal expansion or a leak
- Daily mean rock steady at 1.98–2.03 bar over 30 days
- VRC 700 `WaterPressure` register exists but returns empty — do not use
- `RunDataHighPressure` (HMU) is refrigerant high-side pressure, not system water

## eBUS Timer Encoding

VRC 700 timer bytes have non-obvious semantics that cause silent failures if misused.

- Never use `00:00` as a timer end time — use `-:-` instead
- TTM byte `0x00` = start of day (midnight), not "end of period"
- TTM byte `0x90` = `-:-` = "until end of day"
- A timer window with end < start is silently rejected by the VRC 700
- `HwcSFMode` can get stuck on `load` after a DHW boost — monitor and reset to `auto`

See `docs/vrc700-settings-audit.md` for the full register table.

## VRC 700 Black Box Behaviour

The VRC 700 has undocumented internal behaviours that break naive modelling.

- **Optimum Start**: firmware ramps setpoint ~3h before day timer (observed 03:00 for 06:00 timer). `Hc1ActualFlowTempDesired` jumps without any curve or setpoint change. No eBUS register to disable — use `Z1OpMode=night` to eliminate.
- **Hidden floor**: `Hc1MinFlowTempDesired`=20°C acts as undocumented floor on flow temp
- **Effective setpoint**: back-solving gives ~20°C, not the written 19°C or 21°C
- **Curve formula**: `flow = setpoint + curve × (setpoint − outside)^1.25` is a rough approximation only. Vaillant manual says exponent 1.10 but underpredicts by 2.5–3.1°C at curves ≥0.50. IEEE 754 float resolution: 0.01 step = ~0.20°C flow change at SP=19.
- **Do not model the formula for control** — the inner feedback loop closes on `Hc1ActualFlowTempDesired` readback, treating VRC 700 as a black box.

## Data Duplication

Values that exist in two places and must be kept in sync manually.

- Radiator T50 values: `config.toml` `[[radiators]]` (used by analysis.rs) AND `data/canonical/thermal_geometry.json` (used by thermal solver)
- `--all-data` start timestamp: hardcoded in `resolve_time_range()` AND `config.toml` `default_sync_start_ms`

## eBUS Control Flow

VRC 700 sends SetMode to HMU every ~30s with flow temp demand (D1C encoding). All write commands go to VRC 700 (`-c 700`). Direct HMU writes get overwritten within 10s.

VWZ AI is hydraulic only (valve/pump), not in the flow temp control path. It gets separate messages with zeros for flow temp. Can operate standalone without VRC 700 (has own heat curve, setpoints, DHW control via its panel).

eBUS coverage: 247 read + 216 write defs for VRC 700, 117 read + 14 passive for HMU, zero decoded for VWZ AI (raw bytes in grab buffer only). ebusd `--enablehex` and `--enabledefine` are on.

Future option: `SetModeOverride` to HMU to bypass VRC 700 entirely. Message format decoded. Requires disabling or outpacing the 700's 30-second writes.
