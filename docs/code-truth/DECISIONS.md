<!-- code-truth: 33e263a -->

# Decisions

### SQLite for Local Storage

**Status:** active

**What:** All emoncms data is downloaded to a local SQLite database at 1-minute resolution. Analysis runs exclusively from this local copy.

**Why:** The emoncms API is slow for large queries (rate-limited, chunked fetching), and the tool is designed for iterative, experimental analysis. Local storage makes queries instant and offline-capable.

**Where:** `db.rs` — schema in `open()`, sync in `sync_all()`, loading in `load_dataframe_inner()`.

**Consequences:** First sync takes several minutes (7.4M samples). The `.db` file is ~207 MB. Users must run `sync` before any analysis command. The database is gitignored.

---

### Flow-Rate-Based State Classification

**Status:** active

**What:** Operating state (heating/DHW/defrost) is determined by a hysteresis state machine using flow rate, heat output, and delta-T — not by flow temperature or the `DHW_flag` feed.

**Why:** The Vaillant Arotherm 5kW has a fixed pump speed (860 L/h = 14.3 L/min). When the diverter valve switches to the DHW cylinder coil, flow rate jumps to ~20.7 L/min due to lower flow resistance. This produces a clear bimodal distribution with a near-empty gap between 14.5 and 16.0 L/min. Flow temperature was initially used but missed DHW ramp-up periods. The `DHW_flag` feed (512889) only has data until Dec 2024.

**Where:** `analysis.rs` — constants `DHW_ENTER_FLOW_RATE` (16.0), `DHW_EXIT_FLOW_RATE` (15.0), `DEFROST_DT_THRESHOLD` (-0.5), and `classify_states()` function.

**Consequences:** The thresholds are specific to the 5kW Arotherm. A different model (7kW at 20.0 L/min, or 10kW at 33.3 L/min) would need different values. Defrost detection relies on negative heat output or negative delta-T. The state machine output is now used by `octopus.rs::daily_hp_by_state()` for heating/DHW separation in cost analysis.

---

### Simulated Data in Separate Table

**Status:** active

**What:** Gap-filled (synthetic) data is stored in `simulated_samples`, separate from real data in `samples`. An `--include-simulated` flag controls whether it's included in analysis. When included, an `is_simulated` boolean column is added to the DataFrame.

**Why:** The data has a 54-day summer gap (May–Jul 2025) and ~60 smaller gaps. Simulated data should be available for aggregate queries (yearly COP) but must never be confused with measured data.

**Where:** `gaps.rs` — `simulated_samples` and `gap_log` tables. `db.rs` — `load_dataframe_inner()` merges simulated data when flag is set. `main.rs` — `--include-simulated` CLI flag.

**Consequences:** Analysis results can differ depending on whether simulated data is included. The `gap_log` table records the method used (interpolate, model, model_low_confidence) for auditability.

---

### Energy-Scaled Gap Filling

**Status:** active

**What:** Gap-filled power estimates are linearly scaled so their time-integrated energy matches the cumulative meter readings (feeds `503095` elec_energy and `503097` heat_energy) at the gap boundaries.

**Why:** The cumulative meters run continuously even when the monitoring logger drops out. They provide a ground-truth constraint on total energy consumed/produced during each gap.

**Where:** `gaps.rs` — `fill_gap()`, variables `elec_scale` and `heat_scale`.

**Consequences:** The minute-by-minute power profile during gaps is approximate (based on outside temperature bins), but the total energy is exact. COP during gaps inherits the scaling ratio.

---

### Hardcoded Feed IDs

**Status:** active

**What:** Emoncms feed IDs (e.g. `503094` for electric power) are hardcoded as string constants in `db.rs` and `gaps.rs`.

**Why:** This is a single-installation tool. The feed IDs are stable identifiers assigned by emoncms.org.

**Where:** `db.rs: FEED_IDS`, `db.rs: load_dataframe_inner()`, `gaps.rs` (multiple SQL queries).

**Consequences:** The tool cannot be used with a different emoncms installation without modifying source code. Feed IDs appear in multiple files — `db.rs` is the closest thing to a single source, but `gaps.rs` has its own hardcoded references.

---

### Reference Data as Compile-Time Constants

**Status:** active

**What:** House thermal properties, radiator inventory, Arotherm manufacturer spec, and gas-era consumption are hardcoded in `reference.rs` rather than loaded from a config file or database.

**Why:** These are static planning values that change rarely if ever. Encoding them in Rust gives compile-time type checking and makes the tool self-contained.

**Where:** `reference.rs` — four nested modules (`house::`, `arotherm::`, `radiators::`, `gas_era::`). Some values duplicated in `octopus.rs` (`GAS_DHW_KWH_PER_DAY`, `BOILER_EFFICIENCY`).

**Consequences:** Changing any reference value requires recompilation. Duplicated values in `octopus.rs` must be kept in sync manually. If the house undergoes solid wall insulation (planned), the HTC, U-values, and heat loss figures will need updating.

---

### Hybrid Temperature for Octopus Analysis

**Status:** active

**What:** `octopus.rs::load_weather()` builds a unified daily temperature series using emoncms feed 503093 (Met Office hourly) for HP-era dates and ERA5-Land + 1.0°C bias correction for gas-era dates.

**Why:** ERA5-Land reads ~1.0°C colder than the emoncms Met Office sensor on average (measured over 507-day overlap, mean bias +1.00°C, stdev 0.57°C). Without correction, ERA5 overstates HDD by ~14%, distorting gas-vs-HP comparisons. Emoncms data only starts Oct 2024, so ERA5 is the only source for the gas era (Apr 2020 – Oct 2024).

**Where:** `octopus.rs` — `ERA5_BIAS_CORRECTION_C` constant (1.0), `load_weather()` function. Weather DataFrame includes a `source` column ("emoncms" or "ERA5+1.0") for transparency.

**Consequences:** A single +1.0°C constant is an approximation — actual monthly bias ranges from +0.6°C (autumn) to +1.8°C (March). Monthly correction would be more accurate but the constant is adequate for seasonal/annual analysis. If eBUS OAT becomes available, it would be the most accurate source for both eras.

---

### Octopus Data as External JSON

**Status:** active

**What:** Octopus Energy consumption and weather data are loaded from pre-processed JSON files at `~/github/octopus/dist/data/`, produced by the separate `~/github/octopus/` project.

**Why:** The Octopus project already had a pipeline for fetching, merging, and converting consumption data (including legacy parquet, REST API, gas m³→kWh conversion, and weather from Open-Meteo). Rather than duplicate that work, `octopus.rs` reads the output.

**Where:** `octopus.rs` — `default_data_dir()` returns the hardcoded path. `load_consumption()` and `load_weather()` accept an optional `data_dir` override.

**Consequences:** The path `~/github/octopus/dist/data/` is hardcoded. Data must be refreshed externally (`cd ~/github/octopus && bash scripts/run_dashboard.sh`). There's no automatic staleness detection. If the JSON schema changes in the octopus project, `octopus.rs` deserialization will break.

---

### Heating/DHW Separation in Gas-vs-HP Comparison

**Status:** active

**What:** `gas-vs-hp` separates heating from DHW for a fair comparison. HP era uses measured state machine data. Gas era subtracts an estimated 11.82 kWh/day DHW from total gas consumption.

**Why:** Without separating DHW, the heat/HDD metric conflates space heating (weather-dependent) with hot water (roughly constant). The HP state machine gives precise measured DHW for the HP era. For the gas era, the 11.82 kWh/day estimate comes from the planning workbook regression.

**Where:** `octopus.rs` — `daily_hp_by_state()` uses the state machine; `print_gas_vs_hp()` applies the DHW estimate for gas era. `GAS_DHW_KWH_PER_DAY` constant.

**Consequences:** The gas-era DHW estimate is approximate. If actual DHW varied seasonally (more in winter due to colder inlet water), the heating-only gas/HDD would be slightly wrong. The HP-era DHW is measured, giving 11.0 kWh/day — close to the 11.82 estimate, validating the assumption.

---

### Polars Pinned to 0.46

**Status:** active

**What:** `Cargo.toml` pins `polars = "0.46"` despite 0.53 being available.

**Why:** The Polars API changes significantly between minor versions. The project was built iteratively and upgrading requires testing all DataFrame operations. [INFERRED]

**Where:** `Cargo.toml`. The `strings` feature was added for `octopus.rs` string operations.

**Consequences:** Missing newer Polars features and performance improvements.

---

### 5kW-Specific Thresholds

**Status:** active

**What:** All operating state thresholds assume a 5kW Arotherm (fixed pump at 14.3 L/min). The 7kW model runs at 20.0 L/min for heating — which overlaps with the 5kW's DHW flow rate range.

**Why:** Single-installation tool, no need for multi-model support.

**Where:** `analysis.rs` constants, `gaps.rs` hardcoded `flow_rate >= 16.0`.

**Consequences:** Cannot be used with a different Arotherm size without changing thresholds. A 7kW adaptation would need an entirely different classification approach.

---

### UK Standard Degree Day Base Temperature

**Status:** active

**What:** HDD calculated at 15.5°C base (UK standard: 18.5°C design indoor − 3°C internal gains). A separate 17°C base from gas-era regression is used for gas comparison. Both `analysis.rs` and `octopus.rs` use 15.5°C.

**Why:** 15.5°C is the conventional UK base. The gas-era analysis used 17°C because regression on actual gas consumption suggested higher base — possibly due to higher boiler-era heat losses.

**Where:** `analysis.rs` — `HDD_BASE_TEMP` (15.5°C). `octopus.rs` — `HDD_BASE_C` (15.5°C). `reference.rs` — `house::BASE_TEMP_GAS_ERA` (17°C).

**Consequences:** Two HDD base constants must be kept in sync (`analysis.rs` and `octopus.rs`). The estimated base temperature from HP data (~12°C) differs from both values, suggesting the house performs better than either standard assumes.

---

### No Tests

**Status:** active

**What:** There are no automated tests of any kind.

**Why:** The project evolved through rapid experimentation with an LLM agent, validating against live data output. [INFERRED]

**Where:** No `#[cfg(test)]` modules anywhere in `src/`.

**Consequences:** Refactoring carries risk. The state machine logic, gap-filling model, and now the octopus integration are the most complex and most likely to regress.

---

## Open Questions

- The `gaps.rs` module accesses SQLite tables directly rather than going through `db.rs` functions. It's unclear whether this is intentional (performance, avoiding circular dependencies) or just organic growth. [UNCERTAIN]

- The `BinStats` struct in `gaps.rs` has a `_count` field (renamed from `count` to suppress a warning). The field is populated but never read. It may have been intended for confidence weighting or model diagnostics. [UNCERTAIN]

- Feed IDs are duplicated across `db.rs` and `gaps.rs`. A change to feed IDs requires updating both files, and there's no compile-time check that they're consistent. [INFERRED]

- The indoor temperature sensor (emonth2, feed 503101) is in the Leather room only. Analysis treats it as representative of whole-house temperature, which may over- or under-estimate comfort in other rooms. [INFERRED]

- `octopus.rs` duplicates constants from `reference.rs` (`GAS_DHW_KWH_PER_DAY = 11.82`, `BOILER_EFFICIENCY = 0.9`). These could be imported from `reference.rs` instead. [INFERRED]

- The ERA5 bias correction (+1.0°C) is a single constant but the actual bias varies +0.6 to +1.8°C by month. A monthly correction table would improve accuracy for individual months but adds complexity. [INFERRED]

- Tariff rates for cost analysis are not stored anywhere in this project — they were calculated via ad-hoc Python scripts against the Octopus API. A proper `cost` subcommand would need a tariff schedule, either hardcoded or fetched. [INFERRED]
