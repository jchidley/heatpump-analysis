<!-- code-truth: 07b9b7f -->

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

**Consequences:** The thresholds are specific to the 5kW Arotherm. A different model (7kW at 20.0 L/min, or 10kW at 33.3 L/min) would need different values. Defrost detection relies on negative heat output or negative delta-T, which works because defrost reverses the refrigerant cycle and extracts heat from the water.

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

**Why:** The cumulative meters run continuously even when the monitoring logger drops out. They provide a ground-truth constraint on total energy consumed/produced during each gap. Without scaling, the model's average power estimates would not integrate to the correct total.

**Where:** `gaps.rs` — `fill_gap()`, variables `elec_scale` and `heat_scale`.

**Consequences:** The minute-by-minute power profile during gaps is approximate (based on outside temperature bins), but the total energy is exact. COP during gaps inherits the scaling ratio, which is correct for aggregate COP but the instantaneous values are modelled.

---

### Hardcoded Feed IDs

**Status:** active

**What:** Emoncms feed IDs (e.g. `503094` for electric power) are hardcoded as string constants in `db.rs`, `analysis.rs` (removed), and `gaps.rs`.

**Why:** This is a single-installation tool. The feed IDs are stable identifiers assigned by emoncms.org.

**Where:** `db.rs: FEED_IDS`, `db.rs: load_dataframe_inner()`, `gaps.rs` (multiple SQL queries).

**Consequences:** The tool cannot be used with a different emoncms installation without modifying source code. Feed IDs appear in multiple files — `db.rs` is the closest thing to a single source, but `gaps.rs` has its own hardcoded references.

---

### No Tests

**Status:** active

**What:** There are no automated tests of any kind.

**Why:** The project evolved through rapid experimentation with an LLM agent, validating against live data output rather than unit tests. [INFERRED]

**Where:** No `#[cfg(test)]` modules anywhere in `src/`.

**Consequences:** Refactoring carries risk. The state machine logic and gap-filling model are the most complex and most likely to regress. The operating model was validated empirically against 448k real samples but has no regression tests.

---

### Polars Pinned to 0.46

**Status:** active

**What:** `Cargo.toml` pins `polars = "0.46"` despite 0.53 being available.

**Why:** The project was built iteratively with an LLM agent. The Polars API changes significantly between minor versions (renamed methods, changed trait bounds, different feature flags). Upgrading requires testing all DataFrame operations. [INFERRED]

**Where:** `Cargo.toml`.

**Consequences:** Missing newer Polars features and performance improvements. The `mul` workaround in `analysis.rs` (using `* lit(2)` instead of `.mul(lit(2))`) was needed due to trait bound issues in 0.46 — this may be fixed in newer versions.

---

### 5kW-Specific Thresholds

**Status:** active

**What:** All operating state thresholds assume a 5kW Arotherm (fixed pump at 14.3 L/min). The 7kW model runs at 20.0 L/min for heating — which overlaps with the 5kW's DHW flow rate range.

**Why:** Single-installation tool, no need for multi-model support.

**Where:** `analysis.rs` constants, `gaps.rs` hardcoded `flow_rate >= 16.0`.

**Consequences:** Cannot be used with a different Arotherm size without changing thresholds. A 7kW adaptation would need an entirely different classification approach since the flow rate bimodal gap doesn't exist in the same way.

---

## Open Questions

- The `gaps.rs` module accesses SQLite tables directly rather than going through `db.rs` functions. It's unclear whether this is intentional (performance, avoiding circular dependencies) or just organic growth. If adding new tables or queries, it's ambiguous which pattern to follow. [UNCERTAIN]

- The `BinStats` struct in `gaps.rs` has a `_count` field (renamed from `count` to suppress a warning). The field is populated but never read. It may have been intended for confidence weighting or model diagnostics. [UNCERTAIN]

- Feed IDs are duplicated across `db.rs` and `gaps.rs`. A change to feed IDs requires updating both files, and there's no compile-time check that they're consistent. [INFERRED]
