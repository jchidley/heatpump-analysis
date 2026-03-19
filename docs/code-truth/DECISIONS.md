<!-- code-truth: 9d6d3e3 -->

# Decisions

## D1: Flow rate for state classification (not temperature or flags)

**Choice**: Classify operating states using flow rate thresholds with hysteresis.

**Why**: The Arotherm 5kW has a fixed-speed pump. The only thing that changes flow rate is the diverter valve position (heating circuit vs DHW cylinder coil). This gives a clean bimodal signal: 14.3–14.5 L/min for heating, 16.5+ L/min for DHW, with a near-empty transition zone.

**Alternatives rejected**:
- *Flow temperature > 38°C for DHW*: Missed ramp-up periods, late-stage DHW, and mild-weather DHW. Abandoned early.
- *DHW_flag feed (512889)*: Only has data until December 2024. Cannot be used for the full dataset.

**Impact of undoing**: Any new classification approach must be validated against 448k+ running samples. The existing state machine produces COP figures consistent with manufacturer expectations.

## D2: External TOML configuration

**Choice**: All domain constants, feed IDs, thresholds, house data, radiator specs, and gas-era history live in `config.toml`, loaded at runtime.

**Why**: Previously spread across 6 `.rs` files as hardcoded constants. Changing a feed ID or threshold required recompilation. TOML is Rust's native config format (used by Cargo itself) and supports comments for documenting each value.

**Trade-off**: Adds a runtime dependency on a config file being present. The executable won't start without `config.toml` accessible.

## D3: No tests

**Choice**: No unit or integration tests. Validation is done by running analysis against the full real dataset and checking output.

**Why**: The core logic (state machine, COP calculations, degree days) operates on real-world data with complex interactions. Mock data would not capture the subtleties (e.g. defrost during DHW, diverter valve transition timing). The full dataset serves as the integration test.

**Risk**: Regressions can only be caught by re-running commands and comparing output. No CI guard.

## D4: Cumulative meters as gap-fill constraint

**Choice**: When filling monitoring gaps, scale synthetic power estimates so their time-integral matches the cumulative energy meters (elec_kwh, heat_kwh).

**Why**: The cumulative meters run continuously even when the logger drops out. This means total energy during any gap is known exactly. The gap-fill model only needs to distribute this energy realistically across minutes, not estimate the total.

**Impact of undoing**: Without this constraint, gap-filled energy totals would be purely modelled and could drift significantly from reality.

## D5: Separate simulated_samples table

**Choice**: Gap-filled data is stored in `simulated_samples`, never in `samples`. Analysis includes it only when `--include-simulated` is passed.

**Why**: Mixing synthetic and real data silently would contaminate COP and energy analysis. Keeping them separate ensures the user consciously opts in.

## D6: Global config singleton (once_cell)

**Choice**: Load config once in `main()`, store in a `OnceCell`, access via `config::config()` anywhere.

**Why**: The config is immutable after startup and needed by every module. Passing `&Config` through every function signature would be invasive. The singleton pattern keeps function signatures clean.

**Trade-off**: Implicit global state. Functions silently depend on config being loaded. Unit testing (if ever added) would need to initialise the singleton.

## D7: Polars 0.46 (pinned)

**Choice**: Polars is pinned to 0.46 despite 0.53+ being available.

**Why**: Polars has frequent breaking API changes between minor versions. The current version works; upgrading would require auditing all lazy queries.

## D8: Blocking HTTP client

**Choice**: `reqwest::blocking` rather than async.

**Why**: The CLI is sequential — it syncs feeds one at a time in 7-day chunks. Async would add complexity (tokio runtime, async main) for no benefit. The 100ms throttle between API calls is intentional politeness.

## D9: Two HDD base temperatures

**Choice**: Two different base temperatures coexist:
- **15.5°C** (UK standard) — used for HDD analysis and Octopus comparison
- **17.0°C** — from gas-era regression analysis, used for gas-vs-HP comparisons

**Why**: The gas-era regression on actual consumption data yielded a base temp of 17°C for that house. The UK standard of 15.5°C (18.5°C indoor − 3°C internal gains) is used for normalisation that should be comparable to other properties.

**Risk**: Using the wrong base temp for a comparison produces misleading efficiency ratios. Both values are now in `config.toml` (`thresholds.hdd_base_temp_c` = 15.5, `house.base_temp_gas_era_c` = 17.0) making the distinction explicit.

## D10: ERA5 bias correction as a single constant

**Choice**: ERA5-Land temperatures are corrected by adding 1.0°C for the entire dataset.

**Why**: The overlap period (507 days where both emoncms and ERA5 data exist) shows a systematic +1.0°C offset. Monthly variation ranges from +0.6 to +1.8°C but a single constant is adequate for seasonal/annual analysis.

**[INFERRED]** A monthly correction table would improve accuracy for monthly comparisons but hasn't been implemented.

## D11: gaps.rs bypasses db.rs

**Choice**: `gaps.rs` writes directly to SQLite (`simulated_samples`, `gap_log` tables) rather than going through `db.rs`.

**Why**: The gap-fill workflow is conceptually separate from sync — it reads from `samples` and writes to its own tables. Routing everything through `db.rs` would couple the two concerns.

**Risk**: Schema and feed ID conventions must stay consistent between the two modules. The config migration (D2) addressed the feed ID synchronisation problem.

## Open Questions

- **`fill_gap_interpolate()` hardcoded IDs**: The linear interpolation path in gaps.rs still uses hardcoded feed ID strings. Should be migrated to use `config().emoncms.feed_id()` for consistency.
- **Octopus data path**: `~/github/octopus/dist/data/` is hardcoded in `default_data_dir()`. Should this move to config.toml?
- **`--all-data` start timestamp**: `resolve_time_range()` in main.rs hardcodes `1_729_555_200` (Oct 22 2024) as the earliest data, duplicating the value in `config.toml`. These should be unified.
