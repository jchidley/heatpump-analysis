<!-- code-truth: 3af9fd0 -->

# Decisions

## Structural Decisions

### D1: Flow rate for state classification (not temperature or flags)

**Status:** active

**What**: Classify operating states using flow rate thresholds with hysteresis.

**Why**: The Arotherm 5kW has a fixed-speed pump. The only thing that changes flow rate is the diverter valve position (heating circuit vs DHW cylinder coil). This gives a clean bimodal signal: 14.3–14.5 L/min for heating, 16.5+ L/min for DHW, with a near-empty transition zone.

**Where**: `analysis.rs::classify_states()`, `config.toml` `[thresholds]`

**Alternatives rejected**:
- *Flow temperature > 38°C for DHW*: Missed ramp-up periods, late-stage DHW, and mild-weather DHW. Abandoned early.
- *DHW_flag feed (512889)*: Only has data until December 2024. Cannot be used for the full dataset.
- *eBUS StatuscodeNum*: Now available (104=heating, 134=DHW, 516=defrost) but not yet integrated into analysis. Could replace or validate the flow-rate state machine.

**Consequences**: Any new classification approach must be validated against 448k+ running samples. The existing state machine produces COP figures consistent with manufacturer expectations.

### D2: External TOML configuration

**Status:** active

**What**: All domain constants, feed IDs, thresholds, house data, radiator specs, and gas-era history live in `config.toml`, loaded at runtime.

**Why**: Previously spread across 6 `.rs` files as hardcoded constants. Changing a feed ID or threshold required recompilation. TOML is Rust's native config format (used by Cargo itself) and supports comments for documenting each value.

**Where**: `config.toml`, `config.rs`

**Consequences**: Adds a runtime dependency on a config file being present. The executable won't start without `config.toml` accessible.

### D4: Cumulative meters as gap-fill constraint

**Status:** active

**What**: When filling monitoring gaps, scale synthetic power estimates so their time-integral matches the cumulative energy meters (elec_kwh, heat_kwh).

**Why**: The cumulative meters run continuously even when the logger drops out. This means total energy during any gap is known exactly. The gap-fill model only needs to distribute this energy realistically across minutes, not estimate the total.

**Where**: `gaps.rs::fill_gap()`

**Consequences**: Without this constraint, gap-filled energy totals would be purely modelled and could drift significantly from reality.

### D5: Separate simulated_samples table

**Status:** active

**What**: Gap-filled data is stored in `simulated_samples`, never in `samples`. Analysis includes it only when `--include-simulated` is passed.

**Why**: Mixing synthetic and real data silently would contaminate COP and energy analysis. Keeping them separate ensures the user consciously opts in.

**Where**: `gaps.rs::ensure_schema()`, `db.rs::load_dataframe_with_simulated()`

### D6: Global config singleton (once_cell)

**Status:** active

**What**: Load config once in `main()`, store in a `OnceCell`, access via `config::config()` anywhere.

**Why**: The config is immutable after startup and needed by every module. Passing `&Config` through every function signature would be invasive. The singleton pattern keeps function signatures clean.

**Where**: `config.rs`

**Consequences**: Implicit global state. Functions silently depend on config being loaded. Unit testing (if ever added) would need to initialise the singleton.

### D11: DHW threshold tightening (March 2026)

**Status:** active

**What**: DHW entry threshold lowered from 16.0 to 15.0 L/min, exit from 15.0 to 14.7 L/min.

**Why**: DHW flow dropped from 21.0 to 16.8 L/min due to y-filter magnetite sludge buildup over winter 2025-26. At 16.8, the original 16.0 entry threshold had only 0.8 L/min margin. The tighter thresholds work because heating is software-clamped at 14.3 L/min and cannot false-trigger DHW entry.

**Where**: `config.toml` `[thresholds]`, documented in `docs/hydraulic-analysis.md`

**Consequences**: After y-filter cleaning (19 March 2026), DHW flow recovered to 21.3 L/min. The tighter thresholds are retained as they're safe with clamped heating. Should be reviewed if system changes (e.g., different HP model where heating flow rate could reach 15+ L/min).

### D20: Symmetric internal connections in thermal model

**Status:** active

**What**: All wall/floor/ceiling conduction and doorway exchange between rooms is defined **once** per pair (as `InternalConnection` or `Doorway`), not in each room's definition.

**Why**: Prevents double-counting. If room A loses 50W to room B through a shared wall, room B gains 50W from room A — it's one physical quantity. Defining in both rooms would double the transfer. The symmetric approach makes the connection list authoritative.

**Where**: `model/house.py` `build_connections()`, `build_doorways()`, `room_energy_balance()`

**Consequences**: Adding a new room requires defining all its connections in `build_connections()` and doorways in `build_doorways()`, not just the room definition.

### D21: Chimney effect as landing ACH, not pairwise doorway exchange

**Status:** active

**What**: The stairwell chimney (hall→landing→shower) is modelled as increased ventilation ACH for landing (1.30), not as doorway-driven buoyancy exchange.

**Why**: Pairwise buoyancy exchange between adjacent rooms doesn't capture multi-storey stack-driven flow. The chimney draws air from ground floor through first floor to loft — it's a whole-building phenomenon. Calibrated from Night 1 vs Night 2: RMSE=0.057°C/h with chimney ACH, vs poor fit with pairwise doorways.

**Where**: `model/house.py` — landing `ventilation_ach=1.30`, stairwell doorways marked `state="chimney"` (returns 0 in `doorway_exchange()`)

**Consequences**: Changing door states doesn't affect the chimney — it's always present. In reality, closing the hall→landing doorway would reduce chimney flow, but this isn't modelled.

## Pragmatic Decisions

### D3: No tests

**Status:** active

**What**: No unit or integration tests in either Rust or Python. Validation is done by running analysis against real data and checking output.

**Why**: The core logic (state machine, COP calculations, degree days) operates on real-world data with complex interactions. Mock data would not capture the subtleties. The full dataset serves as the integration test. The Python thermal model is validated against two controlled overnight experiments.

**Risk**: Regressions can only be caught by re-running commands and comparing output. No CI guard.

### D7: Polars 0.46 (pinned)

**Status:** active

**What**: Polars is pinned to 0.46 despite 0.53+ being available.

**Why**: Polars has frequent breaking API changes between minor versions. The current version works; upgrading would require auditing all lazy queries.

### D8: Blocking HTTP client

**Status:** active

**What**: `reqwest::blocking` rather than async.

**Why**: The CLI is sequential — it syncs feeds one at a time in 7-day chunks. Async would add complexity (tokio runtime, async main) for no benefit. The 100ms throttle between API calls is intentional politeness.

### D9: Two HDD base temperatures

**Status:** active

**What**: Two different base temperatures coexist:
- **15.5°C** (UK standard) — used for HDD analysis and Octopus comparison
- **17.0°C** — from gas-era regression analysis, used for gas-vs-HP comparisons

**Where**: `config.toml` (`thresholds.hdd_base_temp_c` = 15.5, `house.base_temp_gas_era_c` = 17.0)

**Risk**: Using the wrong base temp for a comparison produces misleading efficiency ratios.

### D10: ERA5 bias correction as a single constant

**Status:** active

**What**: ERA5-Land temperatures are corrected by adding 1.0°C for the entire dataset.

**Why**: The overlap period (507 days where both emoncms and ERA5 data exist) shows a systematic +1.0°C offset. Monthly variation ranges from +0.6 to +1.8°C but a single constant is adequate for seasonal/annual analysis.

**Where**: `octopus.rs` `ERA5_BIAS_CORRECTION_C` (Rust constant, not in config.toml)

**[INFERRED]** A monthly correction table would improve accuracy for monthly comparisons but hasn't been implemented.

### D12: gaps.rs bypasses db.rs

**Status:** active

**What**: `gaps.rs` writes directly to SQLite (`simulated_samples`, `gap_log` tables) rather than going through `db.rs`.

**Why**: The gap-fill workflow is conceptually separate from sync — it reads from `samples` and writes to its own tables. Routing everything through `db.rs` would couple the two concerns.

**Risk**: Schema and feed ID conventions must stay consistent between the two modules.

### D13: Monitoring scripts as shell on pi5data

**Status:** active (updated March 2026)

**What**: eBUS polling runs as a standalone shell script on pi5data. Z2M automations and DHW tracking moved to z2m-hub Rust server.

**Why**: ebusd-poll.sh needs to run 24/7 on pi5data (the central hub). Shell (`mosquitto_pub`, `nc`) is sufficient for read-poll-publish. z2m-hub replaced shell scripts for more complex logic (state tracking, WebSocket, HTTP dashboard).

**Where**: `scripts/ebusd-poll.sh` — deployed to `/usr/local/bin/` on pi5data as systemd service.

**Consequences**: ebusd-poll.sh constants are in the shell script, not config.toml.

### D14: DHW remaining litres — moved to z2m-hub (March 2026)

**Status:** active

**What**: Originally an InfluxDB Flux task. Disabled Mar 2026. Replaced by DHW tracking in z2m-hub, which polls ebusd directly via TCP, detects charge completion, and tracks usage via Multical volume register.

**Where**: z2m-hub `~/github/z2m-hub/`, writes `dhw.remaining_litres` to InfluxDB

**Consequences**: The 161L capacity constant lives in z2m-hub (`DHW_FULL_LITRES`).

### D15: PHE + secondary return rejected (March 2026)

**Status:** rejected

**What**: Plate heat exchanger on HP primary side with secondary pump. Evaluated but not implemented.

**Why rejected**: COP doesn't change. Maximum saving ~£7-8/year vs complexity, fouling risk, and additional failure points.

**Where**: Analysis in `docs/dhw-cylinder-analysis.md`

### D16: DHW target temperature 45°C is optimal (March 2026)

**Status:** active

**What**: The current 45°C `HwcTempDesired` is the right setting. Cost per shower is nearly constant (±5%) across 40-51°C because higher temp = worse COP but less hot water drawn.

**Where**: Analysis in `docs/dhw-cylinder-analysis.md`

### D17: eBUS OutsideTemp as primary, Met Office as control

**Status:** active

**What**: `ebusd/poll/OutsideTemp` (30s resolution) for real-time analysis. emoncms feed 503093 (Met Office hourly) as control.

**Where**: `model/house.py` uses eBUS. Rust analysis tool uses emoncms (historical).

### D18: Python for thermal model, not Rust

**Status:** active (may migrate key calculations to Rust later)

**What**: Room thermal model in Python, not added to the Rust tool.

**Why**: Exploration speed. Parameters still being calibrated. Python with NumPy/SciPy allows rapid iteration without recompilation. Once stable, key calculations may migrate to Rust.

**Where**: `model/house.py`, `docs/room-thermal-model.md`

### D19: SNZB-02P v2.2.0 mandatory

**Status:** active

**What**: All sensors must be on v2.2.0 (8704). v2.1.0 has a bug causing readings to freeze.

**Where**: Z2M OTA, InfluxDB cleanup via `influx delete`

### D22: Doorway Cd=0.20 and landing ACH=1.30 jointly calibrated

**Status:** active

**What**: These two parameters were calibrated together against Night 1 (doors normal) vs Night 2 (all doors closed) cooling rates for all 13 rooms. RMSE=0.057°C/h.

**Why**: Doorway exchange and chimney effect interact — warm air from ground floor rises through doorways and stairwell. Changing either parameter independently breaks the fit for multiple rooms.

**Where**: `model/house.py` `DOORWAY_CD`, landing `ventilation_ach`

**Consequences**: Don't tune one without re-running the joint calibration. Night 1 data: T_out avg 8.5°C, doors normal. Night 2 data: T_out avg 5.0°C, all doors closed.

### D23: Overnight strategy revised (29 March 2026)

**Status:** active (replaces trial of 26 March)

**What**: `Z1NightTemp` = 19°C, `Z1DayTemp` = 21°C. Night mode 00:00–04:00 (aligned to mid-peak dead zone). DHW windows: 05:30–07:00, 13:00–15:00, 22:00–00:00 (Cosy-aligned). DHW mode: eco year-round, manually switch to normal from first cold morning (~November) to March.

**History**: Originally 17°C setback (house never dropped that far — paying for nothing). Briefly trialled full OFF via crontab (26–29 Mar) — rejected (elvina dropped to 15.8°C, £6/yr saving). Settled on 19°C setback which costs ~£20/yr and only fires on coldest nights.

**Key finding**: HP is at capacity on cold days — house stabilises at 19.5–20°C regardless of strategy. Morning DHW delayed to 05:30 so HP heats house for 1.5h first at Cosy rate. 100% of Normal DHW cycles complete within Cosy window. See `docs/overnight-strategy-analysis.md` for full analysis.

**Where**: VRC 700 via eBUS on pi5data. Revert: `echo 'write -c 700 Z1NightTemp 17' | nc -w 2 localhost 8888`

### D24: Solar gain calibrated from PV P3 channel

**Status:** active

**What**: Solar irradiance estimated from EmonPi2 P3 CT channel (PV + Powerwall). Scaling factor 0.087 W/m² per W calibrated from elvina's temperature response.

**Why**: No pyranometer available. PV generation is a reasonable proxy for solar irradiance on the same (SW) face. P3 reads 6.7kW peak for 3.08kWp array — includes Powerwall discharge, so absolute values are wrong, but relative profile is correct.

**Where**: `model/house.py` `solar_gain()`, room `solar` lists in `build_rooms()`

**Consequences**: ⚠ P3 CT scaling is incorrect. Don't use absolute P3 values. The calibrated shading factors absorb the scaling error for equilibrium/warmup analysis.

## Open Questions

- **`fill_gap_interpolate()` hardcoded IDs**: The linear interpolation path in gaps.rs still uses hardcoded feed ID strings. Should be migrated to use `config().emoncms.feed_id()` for consistency.
- **Octopus data path**: `~/github/octopus/data/` is hardcoded in `default_data_dir()`. Should this move to config.toml?
- **`--all-data` start timestamp**: `resolve_time_range()` in main.rs hardcodes `1_729_555_200` (Oct 22 2024), duplicating the value in `config.toml`. Should be unified.
- **ERA5 bias correction location**: `ERA5_BIAS_CORRECTION_C` is a Rust constant in octopus.rs, not in config.toml. Should it be externalised?
- **eBUS state machine validation**: With eBUS providing definitive operating mode (StatuscodeNum), the flow-rate state machine could be validated or replaced. The thermal model already uses StatuscodeNum for free-cooling detection.
- **Thermal model and Rust tool should share room definitions**: `config.toml` has radiator data, `model/house.py` has fabric and ventilation data. These could be unified into config.toml for consistency, but the Python model is still actively evolving.
- **Kitchen equilibrium undershoot**: Model predicts kitchen 2.2°C colder than measured — likely needs more doorway exchange from hall/conservatory. [UNCERTAIN]
- **Shower equilibrium overshoot**: Model predicts shower 2.1°C warmer than measured — may be losing heat to ventilation not captured by the calibrated ACH. [UNCERTAIN]
- **P3 CT scaling**: Reads 6.7kW for 3.08kWp array. Needs fixing in emonpi config — wrong CT ratio or Powerwall discharge contributing. Currently worked around by calibrated scaling factor. [UNCERTAIN]
- **Body heat uncertainty in ACH derivation**: 2 people × 70W = 140W ±50% → ±0.4 ACH uncertainty. Thermal-derived ACH for jackcarol (0.29) vs moisture-derived (1.00) disagrees significantly. Moisture method captures inter-room exchange too, making comparison non-trivial. [UNCERTAIN]
