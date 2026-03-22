<!-- code-truth: 08e43eb -->

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
- *eBUS StatuscodeNum*: Now available (104=heating, 134=DHW, 516=defrost) but not yet integrated into analysis. Could replace or validate the flow-rate state machine. See `heating-monitoring-setup.md` for eBUS data availability.

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

## Pragmatic Decisions

### D3: No tests

**Status:** active

**What**: No unit or integration tests. Validation is done by running analysis against the full real dataset and checking output.

**Why**: The core logic (state machine, COP calculations, degree days) operates on real-world data with complex interactions. Mock data would not capture the subtleties (e.g. defrost during DHW, diverter valve transition timing). The full dataset serves as the integration test.

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

### D13: Monitoring scripts as shell on pi5data (not Python, not integrated into Rust CLI)

**Status:** active (updated March 2026)

**What**: DHW auto-trigger, eBUS polling, and Z2M automations run as standalone shell scripts on pi5data, not integrated into the Rust CLI.

**Why**: They need to run 24/7 on pi5data (the central hub). The Rust CLI runs on a development machine. Different deployment targets, different lifecycle. Shell scripts (`mosquitto_sub`, `mosquitto_pub`, `nc`) are sufficient — no Python runtime needed.

**Where**: `scripts/ebusd-poll.sh` — deployed to `/usr/local/bin/` on pi5data as systemd service.

**History**: Originally `ebusd-poll.py` in Docker, replaced by shell script on pi5data in March 2026. `dhw-auto-trigger.sh` and `z2m-automations.sh` also used this pattern but were removed Mar 2026 — replaced by z2m-hub Rust server (`~/github/z2m-hub/`).

**Consequences**: ebusd-poll.sh constants are in the shell script, not config.toml. Z2M automations and DHW scripts moved to z2m-hub (Mar 2026).

### D14: DHW remaining litres — moved to z2m-hub (March 2026)

**Status:** active

**What**: Originally an InfluxDB Flux task (id `1071306263e06000`, every 1m). **Disabled Mar 2026** due to null crash when no water drawn after charge. Replaced by DHW tracking in z2m-hub (`~/github/z2m-hub/`), which polls ebusd directly via TCP, detects charge completion (scheduled → 161L, manual boost → +50%), and tracks usage via Multical volume register.

**Why**: Provides real-time visibility into DHW capacity so household members can make informed decisions about whether to trigger a manual boost charge. Replaces guesswork with data.

**Where**: InfluxDB on pi5data, documented in `docs/dhw-cylinder-analysis.md`

**Consequences**: The 161L capacity constant now lives in z2m-hub (`DHW_FULL_LITRES`). z2m-hub writes `dhw.remaining_litres` to InfluxDB for Grafana/historical display.

### D15: PHE + secondary return rejected (March 2026)

**Status:** rejected

**What**: Plate heat exchanger on HP primary side with secondary pump injecting heated DHW at the secondary return (F, 1519mm). Evaluated but not implemented.

**Why rejected**: COP doesn't change (HP operating point is unaffected — same total heat, same flow target). The T1 dip during early charging is only 0.3°C. Maximum saving ~£7-8/year vs complexity, fouling risk, and additional failure points. The coil-in-coil heat exchanger is already 90-95% efficient.

**Where**: Analysis in `docs/dhw-cylinder-analysis.md`

### D16: DHW target temperature 45°C is optimal (March 2026)

**Status:** active

**What**: The current 45°C `HwcTempDesired` is the right setting. Analysis shows cost per shower is nearly constant (±5%) across the entire 40-51°C range because higher temp = worse COP but less hot water drawn per shower. The two effects cancel.

**Why**: People always mix to their comfortable temperature regardless of tank setting. 45°C is ~1°C above the practical minimum for a 42°C shower preference with pipe losses. Handles bath + shower with margin. Increasing doesn't help (HP already skips unnecessary charges).

**Where**: Analysis in `docs/dhw-cylinder-analysis.md`

## Open Questions

- **`fill_gap_interpolate()` hardcoded IDs**: The linear interpolation path in gaps.rs still uses hardcoded feed ID strings. Should be migrated to use `config().emoncms.feed_id()` for consistency.
- **Octopus data path**: `~/github/octopus/data/` is hardcoded in `default_data_dir()`. Should this move to config.toml?
- **`--all-data` start timestamp**: `resolve_time_range()` in main.rs hardcodes `1_729_555_200` (Oct 22 2024) as the earliest data, duplicating the value in `config.toml`. These should be unified.
- **ERA5 bias correction location**: `ERA5_BIAS_CORRECTION_C` is a Rust constant in octopus.rs, not in config.toml. Should it be externalised?
- **eBUS state machine validation**: With eBUS now providing definitive operating mode (StatuscodeNum), the flow-rate state machine could be validated or replaced. Not yet investigated.
- **dhw-auto-trigger removed**: Both the Python (`dhw-auto-trigger.py`) and shell (`dhw-auto-trigger.sh`) versions removed Mar 2026. DHW boost now manual via z2m-hub dashboard. The `.py` file remains in repo but should not be deployed.
