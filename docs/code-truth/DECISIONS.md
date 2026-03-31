<!-- code-truth: e67fc92 -->

# Decisions

## Structural Decisions

### D1: Flow rate for state classification (not temperature or flags)

**Status:** active

**What**: Classify operating states using flow rate thresholds with hysteresis.

**Why**: The Arotherm 5kW has a fixed-speed pump. Diverter valve position gives a clean bimodal signal: 14.3–14.5 L/min heating, 16.5+ L/min DHW.

**Where**: `analysis.rs::classify_states()`, `config.toml` `[thresholds]`

**Alternatives rejected**:
- *Flow temperature > 38°C for DHW*: Missed ramp-up, late-stage DHW, and mild-weather DHW.
- *DHW_flag feed (512889)*: Dead since December 2024.
- *eBUS StatuscodeNum*: **Unreliable for DHW detection** — code 134 appears during both off/frost standby AND active DHW. The Rust `thermal-operational` command uses `BuildingCircuitFlow` (L/h) instead: > 900 = DHW, 780–900 = heating, < 100 = off. The emoncms flow-rate state machine remains primary for the main analysis tool.

**Consequences**: Any new classification approach must be validated against 448k+ running samples.

### D2: External TOML configuration

**Status:** active

**What**: All domain constants in `config.toml`, loaded at runtime via `once_cell`.

**Where**: `config.toml`, `config.rs`

**Consequences**: Runtime dependency on config file being present.

### D4: Cumulative meters as gap-fill constraint

**Status:** active

**What**: Scale synthetic power estimates so their time-integral matches cumulative energy meters.

**Where**: `gaps.rs::fill_gap()`

**Consequences**: Total energy during gaps is exact (from meters). Minute-by-minute profile is approximate.

### D5: Separate simulated_samples table

**Status:** active

**What**: Gap-filled data in `simulated_samples`, included only with `--include-simulated`.

**Where**: `gaps.rs`, `db.rs`

### D11: DHW threshold tightening (March 2026)

**Status:** active

**What**: DHW entry 16.0→15.0 L/min, exit 15.0→14.7 L/min.

**Why**: Y-filter sludge reduced DHW flow to 16.8 L/min. Tighter thresholds safe because heating is software-clamped at 14.3 L/min. Flow recovered after filter clean (21.3 L/min) but tighter thresholds retained.

**Where**: `config.toml` `[thresholds]`

### D20: Symmetric internal connections in thermal model

**Status:** active

**What**: Wall/floor/doorway connections defined **once** per pair, applied to both rooms.

**Where**: `data/canonical/thermal_geometry.json`, consumed by `thermal/geometry.rs::build_connections/doorways()`

**Consequences**: Prevents double-counting. Adding a new room requires defining all its connections.

### D21: Chimney effect as landing ACH, not pairwise doorway exchange

**Status:** active

**What**: Stairwell chimney modelled as increased ventilation ACH (1.30) for landing, not buoyancy doorways.

**Why**: Multi-storey stack flow doesn't fit pairwise exchange. Calibrated from Night 1 vs Night 2 (RMSE=0.057°C/h). Valid for cooldown calibration but **structurally wrong for operational mode** — predicts wrong sign 9/14 heating periods. Landing excluded from operational scoring.

**Where**: `data/canonical/thermal_geometry.json` (landing ACH), stairwell doorways marked `state="chimney"` in doorway list

### D25: Two separate state classifiers for emoncms vs eBUS data

**Status:** active

**What**: `analysis.rs::classify_states()` uses emoncms flow rate (L/min). `thermal.rs::classify_hp_state_from_flow()` uses eBUS `BuildingCircuitFlow` (L/h). They serve different data sources and are not connected.

**Why**: The emoncms classifier works on `heatpump.db` data (flow rate from MBUS heat meter). The thermal classifier works on InfluxDB data (eBUS `BuildingCircuitFlow`). Different units, different systems, different availability.

**Consequences**: Threshold changes in one don't automatically propagate to the other.

### D26: Canonical geometry shared between Rust and Python

**Status:** active

**What**: `data/canonical/thermal_geometry.json` is the single source of truth for room geometry, consumed by Rust (`thermal/geometry.rs`).

**Why**: Eliminates drift between the two implementations. Geometry extracted from building plans and XLSX via `extract_house_inventory.py`, audited by `audit_model_dimensions.py` (509 checks, 0 mismatches).

**Where**: `data/canonical/thermal_geometry.json`, provenance in `model/data/inventory/canonical_geometry_provenance.csv`

## Pragmatic Decisions

### D3: No tests

**Status:** active

**What**: No unit or integration tests. Validation via real data + regression baselines for thermal model.

**Risk**: Regressions caught only by running commands and comparing output. Thermal regression baselines provide partial CI guard.

### D6: Global config singleton (once_cell)

**Status:** active

**What**: Load config once in `main()`, access via `config::config()` anywhere.

**Consequences**: Implicit global state. Unit testing would need singleton initialisation.

### D7: Polars 0.46 (pinned)

**Status:** active — 0.53+ available but untested.

### D8: Blocking HTTP client

**Status:** active — async would add complexity for no benefit in a sequential CLI.

### D9: Two HDD base temperatures

**Status:** active

**What**: 15.5°C (UK standard, in `config.toml`) vs 17.0°C (gas-era regression, in `config.toml` under `house`).

**Risk**: Wrong base temp for a comparison → misleading efficiency ratios.

### D10: ERA5 bias correction as constant

**Status:** active

**What**: +1.0°C added to ERA5-Land temperatures. In `octopus.rs` as Rust constant, not in config.toml.

### D12: gaps.rs bypasses db.rs

**Status:** active

**What**: Gap-fill writes directly to SQLite, managing its own schema.

**Risk**: Feed ID conventions must stay consistent between modules.

### D13: Monitoring scripts as shell on pi5data

**Status:** active

**What**: `ebusd-poll.sh` runs as systemd service on pi5data host. Complex automation (Z2M, DHW tracking) moved to z2m-hub Rust server.

### D14: DHW auto-trigger removed (March 2026)

**Status:** removed

**What**: Was `scripts/dhw-auto-trigger.sh` on pi5data. Replaced by manual boost via z2m-hub mobile dashboard.

### D16: DHW target temperature 45°C is optimal

**Status:** active

**What**: Cost per shower nearly constant (±5%) across 40–51°C range. 45°C is ~1°C above practical minimum.

**Where**: Analysis in `docs/dhw-cylinder-analysis.md`

### D18: Python and Rust thermal models coexist

**Status:** active (Rust primary for calibration/validation/operational; Python for equilibrium/moisture)

**What**: Core physics parity-matched between Rust and Python (formula audit completed, doc archived to git history). Remaining Python-only: `thermal-rooms`, `thermal-analyse`, `thermal-equilibrium`, `thermal-moisture`. Migration plan in `docs/rust-migration-plan.md`.

### D22: Doorway Cd=0.20 and landing ACH=1.30 jointly calibrated

**Status:** active

**What**: Calibrated together against Night 1 (doors normal, T_out 8.5°C) vs Night 2 (all doors closed, T_out 5.0°C). RMSE=0.057°C/h.

**Consequences**: Don't tune independently — re-run joint calibration.

### D23: Overnight strategy — 19°C setback (29 March 2026)

**Status:** active

**What**: `Z1NightTemp`=19°C, `Z1DayTemp`=21°C. Night mode 00:00–04:00. DHW windows: 05:30–07:00, 13:00–15:00, 22:00–00:00.

**History**: 17°C setback (house never dropped that far) → OFF trial (rejected, £6/yr) → 19°C (costs ~£20/yr, fires only on coldest nights).

**Key finding**: HP at capacity on cold days. Battery captures most tariff arbitrage (£15–40/yr total scheduling opportunity).

**Where**: VRC 700 via eBUS on pi5data. Full analysis: `docs/overnight-strategy-analysis.md`

### D24: Solar gain calibrated from PV P3 channel

**Status:** active

**What**: SW irradiance from EmonPi2 P3 CT (PV + Powerwall). Calibration: 0.087 W/m² per W on sloping plane, ÷1.4 for vertical reference. NE irradiance from Open-Meteo DNI/DHI decomposition via solar geometry.

**Where**: `thermal.rs::solar_gain_full()`, `pv_to_sw_vertical_irradiance()`

**Consequences**: ⚠ P3 CT reads 6.7kW for 3.08kWp array (includes Powerwall). Used as relative proxy only.

### D27: Conservatory and Landing excluded from operational scoring

**Status:** active

**What**: `thermal-operational` scores 11 of 13 rooms. Conservatory excluded (30m² glass, sub-hour time constant, massive solar/wind sensitivity). Landing excluded (chimney model structurally wrong for heated operation — wrong sign 9/14 periods).

**Where**: `model/thermal-config.toml` `exclude_rooms`

## Open Questions

- **`fill_gap_interpolate()` hardcoded IDs**: Linear interpolation path in gaps.rs uses hardcoded feed ID strings. Should use `config().emoncms.feed_id()`.
- **Octopus data path**: `~/github/octopus/data/` hardcoded in `default_data_dir()`. Should it move to config.toml?
- **`--all-data` start timestamp**: `resolve_time_range()` in main.rs hardcodes `1_729_555_200`, duplicating `config.toml` value.
- **ERA5 bias correction location**: Rust constant in octopus.rs vs config.toml. Should it be externalised?
- **Kitchen equilibrium undershoot**: Model predicts kitchen 2.2°C colder than measured — likely needs more doorway exchange from hall/conservatory. [UNCERTAIN]
- **P3 CT scaling**: Reads 6.7kW for 3.08kWp array. Wrong CT ratio or Powerwall discharge contributing. Worked around by calibrated scaling factor. [UNCERTAIN]
- **Landing chimney model**: ACH-to-outside works for cooldown but structurally wrong for operational use. Needs bidirectional inter-floor air exchange model. [UNCERTAIN]
- ~~**`thermal.rs` is 3,500 lines**~~: Module split completed 2026-03-29 → 15 focused submodules (config, geometry, physics, solar, wind, calibration, validation, diagnostics, operational, artifact, snapshot + existing error/influx/report). Facade is 23 lines.
