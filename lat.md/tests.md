---
lat:
  require-code-mention: true
---
# Tests

Targeted executable specs for controller invariants that are safety- or cost-sensitive enough to require explicit code coverage.

## Adaptive heating controller

These tests pin the controller rules that must not drift silently as overnight and DHW scheduling logic evolves.

### Overnight target stays at the comfort floor until waking

This spec verifies the overnight room target stays flat at the comfort-band floor until waking time, then steps back to the daytime comfort target.

### Overnight coast guard near waking

This spec verifies the overnight coast rule stops at the 30-minute pre-wake boundary so the controller cannot coast through the final warm-up window.

### Overnight coast requires mild weather and headroom above the floor

This spec verifies overnight coast is only allowed when Leather is still above the comfort-floor margin and outdoor conditions are mild enough to avoid a cold-night underheat.

### Battery headroom threshold depends on DHW mode

This spec verifies Eco and normal DHW modes use different energy budgets when deciding whether battery headroom can carry a charge event to the next Cosy slot.

### Cosy windows ignore battery gating for DHW

This spec verifies DHW launches in every Cosy slot even when the battery signal is low or missing, matching the rule that cheap-grid periods must not be headroom-gated.

### Battery headroom adequacy is monotonic

This spec verifies that increasing discretionary battery headroom cannot make the adequacy decision worse for a fixed DHW mode.

### Eco-mode overnight DHW bypasses the battery gate

This spec verifies that overnight DHW in eco mode launches even when the discretionary headroom signal is inadequate, because the smaller eco charge should not be deferred to the next Cosy slot.

### Overnight battery DHW waits without adequate headroom

This spec verifies that a normal-mode overnight DHW event is deferred to the next Cosy slot when the battery headroom signal is insufficient and no cold-night override applies.

### Overnight battery DHW launches when headroom is sufficient

This spec verifies that a normal-mode overnight DHW event launches immediately in the overnight battery-backed slot once discretionary headroom is sufficient to cover the expected charge.

### Warm-end curve fallback uses the baseline seed

This spec verifies that when forecast outside temperature is at or above the VRC setpoint, the outer loop seeds the known-safe baseline curve instead of inverting the ill-conditioned heat-curve formula.

### Outer loop defers downward resets until flow converges

This spec verifies that the 15-minute outer loop does not ratchet the curve back down while `Hc1ActualFlowTempDesired` still lags the target flow, preventing relearn cycles against the inner loop.

### Heat curve inverse returns floor for tiny delta

This spec verifies that when target flow is below the VRC setpoint, the raw curve value would be negative but is clamped to the configured floor, preventing nonsensical curve writes.

### Heat curve inverse is positive for moderate conditions

This spec verifies that for typical winter conditions (5°C outside, 30°C flow) the heat curve inverse produces a reasonable positive value within the expected range.

### Round2 preserves two decimal places

This spec verifies the rounding helper truncates to exactly two decimal places for consistent eBUS curve writes.

### Clamp curve stays within floor and ceiling

This spec verifies curve clamping enforces both the VRC 700 minimum (0.10) and maximum (4.00) bounds.

### Hours until time wraps across midnight

This spec verifies time arithmetic correctly wraps across midnight and returns zero when the target equals the current time.

### Solar irradiance conversion is non-negative

This spec verifies horizontal-to-vertical solar irradiance conversion is non-negative and applies the UK latitude correction factor.

### Waking hours detection respects boundaries

This spec verifies the waking-hours check includes the start time, excludes the end time, and correctly classifies boundary moments.

### DHW energy estimate depends on mode

This spec verifies that eco mode returns the lower energy budget and normal mode returns the higher budget, with None defaulting to normal.

## Controller tariff and timer helpers

These tests pin the tariff-window and DHW timer helper rules that keep fallback rails aligned with the live scheduling logic.

### Tariff period classification follows sorted windows

This spec verifies that tariff classification sorts configured Cosy and peak windows before labelling the current period, so unsorted runtime/API data still maps to the correct Cosy slot.

### DHW slot mapping respects tariff boundaries

This spec verifies that DHW scheduling maps overnight, Cosy, and non-Cosy gaps to the expected slot keys at the tariff boundaries.

### Morning DHW timer skip uses dash-colon padding

This spec verifies that disabling the morning fallback timer removes that window and pads the unused VRC 700 timer slots with `-:-`, matching the required eBUS encoding.

### DHW timer weekday rolls after waking

This spec verifies that timer rewrites target the current weekday before waking time and the next weekday afterwards, so fallback rails are written for the correct morning.

### T1 prediction wraps across midnight

This spec verifies that cylinder-top decay prediction uses elapsed hours across midnight rather than going negative or resetting at 00:00.

## Thermal solver

These tests pin the equilibrium and MWT-bisection invariants that the live controller depends on when choosing the minimum viable flow temperature.

### Leather equilibrium rises with MWT

This spec verifies that increasing mean water temperature raises Leather's equilibrium temperature for fixed outdoor and solar conditions.

### MWT bisection hits the requested room target

This spec verifies that the solver's MWT bisection returns a near-minimum value that actually lands the requested room close to the target temperature.

### Unreachable targets return no MWT

This spec verifies that the bisection solver returns no result when the requested room target is above the maximum achievable equilibrium temperature in the allowed MWT range.

### Leather equilibrium is monotonic in MWT

This spec verifies across a range of outdoor conditions that raising mean water temperature cannot lower Leather's equilibrium temperature.

## Thermal physics primitives

These tests pin the lower-level heat-transfer helpers that the solver builds on, so primitive regressions cannot hide behind higher-level equilibrium tests.

### Radiator output is monotonic above room temperature

This spec verifies that increasing mean water temperature cannot reduce radiator output once the emitter is above room temperature.

### Doorway exchange scales with opening state

This spec verifies that closed doors block buoyancy exchange and partially open doors transfer less heat than fully open ones for the same geometry and temperature difference.

### Top landing falls back to adjacent sensors

This spec verifies `top_landing` uses its own sensor when present, otherwise derives a stable fallback from landing and shower so doorway and wall coupling do not drop the virtual room.

### Energy balance breakdown matches scalar helper

This spec verifies the componentised full-room balance sums back to the scalar helper and excludes inactive radiators, so diagnostics cannot drift from the solver's net heat calculation.

### Absolute humidity rises with temperature

This spec verifies that at fixed relative humidity, absolute humidity increases with temperature, matching the Magnus formula expectation that warmer air holds more water vapour.

### Surface RH reaches 100 pct at dew point

This spec verifies that a cold surface saturates at 100% RH, and that a surface at air temperature returns the air's relative humidity.

### Absolute humidity is monotonic in temperature

This property test verifies across a range of temperatures and humidities that raising temperature at fixed RH never decreases absolute humidity.

### Surface RH equals air RH at same temperature

This property test verifies across a range of conditions that when the surface temperature equals the air temperature, the surface RH equals the air RH.

## CLI state classification

These tests pin the hysteresis state machine that labels heat-pump rows, so analysis summaries do not silently misclassify DHW, heating, or defrost samples.

### DHW classification holds through the transition band

This spec verifies that once DHW has been entered, transition-band flow rates stay classified as DHW until the lower exit threshold is crossed.

### Defrost recovery preserves the pre-defrost circuit state

This spec verifies that when a defrost ends while flow remains in the transition band, classification returns to the pre-defrost circuit rather than guessing from an ambiguous flow rate.

### Idle precedence wins over defrost-like noise

This spec verifies that samples below the running-power threshold stay idle even if heat or DT briefly look defrost-like, so analysis does not invent activity from sensor noise while the unit is off.

### Heating to DHW to heating cycle transitions cleanly

This spec verifies the full heating→DHW→heating cycle: DHW is entered when flow exceeds the enter threshold, maintained through transition-band flow, and exited when flow drops below the exit threshold.

### Defrost entry from heating preserves heating as pre-defrost

This spec verifies that when defrost occurs from a heating state, recovery in the transition band returns to heating (the pre-defrost circuit) rather than guessing DHW.

### Missing flow rate defaults to heating not DHW

This spec verifies that when flow rate readings are absent (None), the state machine defaults to heating rather than misclassifying as DHW.

### Defrost DT boundary is exclusive

This spec verifies that the defrost DT threshold is strictly less-than (exclusive), so a sample exactly at the threshold stays in the current circuit rather than entering defrost.

## DHW session analysis

These tests pin the helper rules behind historical DHW capacity analysis, where dead-leg sensor bias and mixed WWHR conditions can otherwise skew recommendations.

### Settled mains temperature uses the flushed tail of the draw

This spec verifies that settled mains temperature is taken from the last quartile of in-flow T2 readings, so stale dead-leg warm water does not dominate the estimate.

### WWHR capacity recommendation prefers direct measurements

This spec verifies that when WWHR-backed capacity measurements exist, the recommendation uses the strongest direct WWHR inflection instead of regressing from colder mains cases.

### Single cold capacity measurement stays conservative

This spec verifies that when only one cold-mains capacity measurement exists, the recommendation applies the conservative reduction rather than pretending to know the WWHR uplift.

### Cold mains regression never undercuts measured capacity

This spec verifies that multi-point cold-mains regression never recommends less than the strongest measured cold capacity, so extrapolation toward warmer inlet temperatures cannot reduce a known safe volume.

### Draw type classifies bath shower and tap by flow rate

This spec verifies that draw type classification uses peak flow rate thresholds (bath >= 650, shower >= 350 with volume >= 20L) and falls back to tap for small or low-flow draws.

### Inflection category depends on charge state and T1

This spec verifies that inflection categorisation uses charge crossover state and T1 end temperature to distinguish Capacity, Partial, and LowerBound classifications.

### Last known value finds nearest preceding sample

This spec verifies that binary-search last-known-value lookup returns the most recent sample at or before the query timestamp, returns None for queries before the first sample, and handles empty inputs.

### To sorted deduplicates and orders by timestamp

This spec verifies that time-series conversion sorts by epoch timestamp and deduplicates entries with the same timestamp, producing a deterministic ordered sequence.

## Overnight optimizer helpers

These tests pin pure helper rules used by the historical overnight optimiser, especially where tariff-window scheduling and temperature-bin lookup must remain stable.

### Heating lookup clamps to edge bins outside the calibrated range

This spec verifies that outside temperatures below or above the calibrated heating bins reuse the nearest edge bin rather than falling through to unrealistic defaults.

### Generated overnight schedules keep DHW inside the Cosy window

This spec verifies that generated overnight strategies only place DHW starts whose full duration fits inside the supplied morning Cosy tariff window.

### Generated schedules omit DHW modes that cannot fit the Cosy window

This spec verifies that schedule generation drops DHW options whose full duration cannot fit inside the supplied Cosy tariff window, rather than emitting impossible starts.

## History evidence helpers

These tests pin the pure helper logic behind retrospective heating-history summaries, especially where overnight periods must be interpreted differently from waking-hour comfort issues.

### Waking-hour clipping excludes overnight-only time

This spec verifies that comfort-miss periods are clipped to 07:00–23:00 and split by day boundaries, so expected overnight cooling is not misreported as a comfort failure.

### Sawtooth detection requires repeated alternation

This spec verifies that history review only flags sawtooth behaviour after repeated significant curve-direction alternations, avoiding false positives from one-off adjustments.

### Sawtooth detection ignores sub-threshold noise

This spec verifies that curve jitter below the significance threshold is ignored entirely, so tiny oscillations do not count toward sawtooth alternations.

### Sampling stats report interval summary

This spec verifies that timestamp sampling summaries report sample count plus min, median, and max step sizes for irregular but ordered telemetry.

### Mode changes only emit on actual transitions

This spec verifies that controller history compresses consecutive identical modes into true transition events, so review output reflects state changes rather than raw sample count.

### Numeric summaries preserve extrema and recency

This spec verifies that numeric series summaries preserve the first, last, min, max, and latest points so human and machine review can trust the compact evidence.

### Recent-end lookback ignores invalid periods

This spec verifies that recent-end checks only match valid periods ending inside the requested lookback window, avoiding false recency signals from malformed data.

### Missing-data warnings only flag absent evidence

This spec verifies that warning helpers add messages only when a numeric series or summary is actually missing, so history output does not over-report gaps.

### Period from times computes correct duration

This spec verifies that period construction from two timestamps produces correct RFC3339 strings and computes duration in minutes from the time difference.

### Period duration seconds round-trips with period from times

This spec verifies that `period_duration_seconds` correctly parses the RFC3339 timestamps produced by `period_from_times` and returns the expected seconds, and returns zero for unparseable periods.

### Summary has min below detects threshold crossings

This spec verifies that the minimum-below-threshold helper correctly detects when a numeric summary's minimum point falls below a given threshold, and returns false for missing summaries.

## Thermal calibration helpers

These tests pin pure helper rules behind thermal calibration so grid search and window preparation stay stable even when the Influx-backed CLI path is not exercised in unit tests.

### Calibration ranges include the rounded upper bound

This spec verifies the floating range helper rounds step accumulation and still includes the configured upper bound, so candidate grids do not silently miss edge values.

### Measured rates skip inadequate room samples and require outside data

This spec verifies cooldown-rate extraction ignores room windows that are too short or sparse and errors when the outside series is empty, matching the calibration assumptions.

### Calibration parameter setter updates named rooms and fails on missing geometry

This spec verifies the ACH setter writes the targeted room ventilation values and fails fast if required rooms are absent from the geometry map.

### Window averaging helpers use defaults for missing data

This spec verifies scalar and per-room averaging only uses samples inside the requested window, returning the supplied default or omitting empty rooms when no in-window data exists.

### Room series map known sensor topics and sort samples

This spec verifies calibration prep only maps configured room sensor topics and sorts each room series by timestamp, so later windowing and rate calculations are deterministic.

## Thermal validation helpers

These tests pin the residual and metric aggregation rules that turn per-room validation outputs into pass/fail evidence and whole-house error summaries.

### Residual aggregation skips excluded and missing predictions

This spec verifies residual generation ignores excluded rooms and drops rooms without predictions so validation summaries only score comparable data.

### Whole-house metrics weight errors by thermal mass

This spec verifies whole-house error aggregation converts room cooling rates into watts using thermal mass and ranks the largest absolute contributors first.

### Metrics summaries handle empty inputs and tolerance buckets

This spec verifies metric summaries emit conservative sentinels for empty inputs and compute RMSE, bias, and tolerance fractions correctly for populated residual sets.

### Whole-house ratio stays undefined when measured load cancels out

This spec verifies whole-house `pred_over_meas` stays undefined when measured room loads sum to zero, avoiding meaningless divide-by-zero ratios in validation output.

### Residuals without thermal mass stay rate-only

This spec verifies residual aggregation keeps rooms even when thermal-mass evidence is missing, but leaves their whole-house watt contribution at zero so validation does not invent weighted loads.

### Metrics magnitudes are symmetric under sign inversion

This spec verifies that negating every residual flips bias sign but preserves RMSE, MAE, max absolute error, and tolerance-bucket fractions, because those metrics depend only on magnitudes.

## Thermal regression gates

These tests pin the JSON-artifact comparison gates that protect thermal baselines from being compared across incompatible commands or configs.

### Global regression gate requires matching command and config

This spec verifies that regression checking fails before metric comparisons when artifact commands or config hashes differ, matching the reproducibility contract for thermal snapshots.

### Fit diagnostics med_ratio gate skips null values

This spec verifies that fit-diagnostics comparison treats null `med_ratio` values as missing evidence and skips that gate instead of failing an otherwise comparable artifact.

### Drop gates skip zero-sized baselines

This spec verifies that record-count drop gates are skipped when the baseline has zero items, avoiding meaningless percentage-drop failures during early or sparse artifact generation.
