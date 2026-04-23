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

### Cold nights override overnight battery gating for DHW

This spec verifies that sub-2C overnight conditions force an immediate normal-mode DHW launch even when battery headroom is inadequate, so the scheduler does not gamble on a cold morning cylinder.

### Large overnight T1 deficits override battery gating for DHW

This spec verifies that a sufficiently depleted predicted T1 launches normal-mode overnight DHW immediately even in mild weather, preventing severe morning hot-water shortfalls from waiting for the next Cosy slot.

### Low remaining litres triggers DHW even when T1 looks safe

This spec verifies that the scheduler now treats low `dhw.remaining_litres` as an independent recharge trigger, so a deceptively warm T1 cannot hide an already-depleted practical hot-water budget.

### Recommended full litres caps optimistic remaining estimate

This spec verifies that the scheduler caps `remaining_litres` by the latest `dhw_capacity.recommended_full_litres`, preventing an over-optimistic remaining-volume estimate from suppressing a needed recharge.

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

### parse_f64 handles success error and non-numeric

This spec verifies the eBUS sensor read helper parses valid floats, trims whitespace, and returns None for both error results and non-numeric strings.

### parse_time accepts valid rejects invalid

This spec verifies the time parser accepts valid HH:MM strings, rejects out-of-range hours (24:00), and returns None for empty or garbage input.

### within_window classifies in-window out-of-window and bad input

This spec verifies the tariff window check includes both boundaries (start and end inclusive), rejects times outside the window, and returns false when window strings are unparseable.

### weekday_name covers all seven days

This spec verifies the weekday-to-string mapping returns the correct English day name for all seven weekdays, preventing silent eBUS register misaddressing.

### sorted_cosy_windows returns windows in time order

This spec verifies that Cosy windows are sorted by start time regardless of config file ordering, so tariff classification iterates windows in the expected sequence.

### morning_dhw_windows_enabled excludes waking-end window

This spec verifies that the morning DHW filter drops the Cosy window whose end time matches waking_start and keeps all others, controlling which timer slots are written to the VRC 700.

### hours_until_time is always in 0 to 24 range

This property test verifies across random time pairs that hours_until_time is always non-negative and strictly less than 24, and returns zero when now equals target.

### predict_t1 always decays from initial value

This property test verifies across random initial temperatures and time gaps that the predicted cylinder-top temperature never exceeds the starting value, matching the constant-rate decay model.

### Forecast cache fetches requested hour from API response

This spec verifies that forecast refresh stores the hourly API response and returns the requested hour from the cached result.

### Forecast refresh failure keeps stale cache

This spec verifies that when a forecast refresh fails, the controller keeps serving any stale cached hour instead of going blind immediately.

### Forecast branch uses forecast temperature and solar

This spec verifies that the forecast-driven controller branch feeds forecast outside temperature and solar inputs into the thermal solver path, so target flow is chosen from predicted rather than current conditions.

### Default configuration values are sane

This spec verifies that the controller's built-in default configuration stays internally consistent and within expected operational bounds, so startup without overrides does not begin from nonsense parameters.

### Activating from monitor-only clears DHW timer dedup state

This spec verifies that moving from `MonitorOnly` back to an active mode clears the remembered DHW timer weekday/enable flags so the controller re-evaluates morning timer fallback rails on resume.

### Active-to-active mode changes keep DHW timer dedup state

This spec verifies that changing between active modes does not clear the remembered DHW timer weekday/enable flags, because only disabled or monitor-only activation should force a fresh timer rewrite pass.

## Controller tariff and timer helpers

These tests pin the tariff-window and DHW timer helper rules that keep fallback rails aligned with the live scheduling logic.

### Tariff period classification follows sorted windows

This spec verifies that tariff classification sorts configured Cosy and peak windows before labelling the current period, so unsorted runtime/API data still maps to the correct Cosy slot.

### DHW slot mapping respects tariff boundaries

This spec verifies that DHW scheduling maps overnight, Cosy, and non-Cosy gaps to the expected slot keys at the tariff boundaries.

### Morning DHW timer skip uses dash-colon padding

This spec verifies that disabling the morning fallback timer removes that window and that any end-of-day evening slot is encoded as `-:-`, with unused slots also padded as `-:-` to match VRC 700 eBUS semantics.

### Midnight tariff-window end normalizes for runtime matching

This spec verifies that any imported tariff window ending at `00:00` is normalized to a same-day runtime end (`23:59`) before controller slot matching, so the evening Cosy slot still matches times like 22:30.

### Midnight tariff-window end encodes as dash-colon for eBUS writes

This spec verifies that any end-of-day tariff/timer window is encoded as `-:-` before VRC 700 timer writes, whether it still appears as raw `00:00` or has already been normalized to same-day `23:59` for runtime matching.

### DHW timer weekday rolls after waking

This spec verifies that timer rewrites target the current weekday before waking time and the next weekday afterwards, so fallback rails are written for the correct morning.

### Timer dedup skips unchanged morning rewrite state

This spec verifies that when the computed weekday and morning-enabled flag match the remembered timer state, the controller skips the eBUS write instead of needlessly rewriting the same fallback rails.

### Timer write failures clear dedup state for retry

This spec verifies that an eBUS write result containing `ERR:` clears the remembered timer weekday/enable state so the next controller tick retries the fallback-rail write.

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

### Already warm targets return the minimum MWT

This spec verifies that if the requested room is already warm enough at the solver's minimum allowed MWT, including the exact-equality boundary, the bisection shortcut returns that lower bound instead of searching upward.

### Unknown rooms return no MWT

This spec verifies that asking for a room absent from the solved equilibrium map returns no result rather than fabricating a flow temperature for a nonexistent target.

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

### Radiator output regression anchor at dt50

This spec verifies the radiator-output helper still returns the expected anchored wattage at a representative DT50 operating point, so formula or exponent drift is caught by a fixed regression check.

### Ventilation loss scales with temperature difference

This spec verifies that increasing the indoor-outdoor temperature difference increases ventilation heat loss for fixed ACH and volume, preserving the linear energy-balance relationship.

### Wind multiplier is monotonic in wind speed

This property test verifies that for a fixed calibration the wind correction multiplier cannot decrease as average wind speed rises, preventing inverted infiltration behaviour.

### Thermal mass primitives scale with area

This spec verifies that the thermal-mass helper components scale with the relevant room area or volume inputs, so calibration and validation calculations preserve the intended geometric relationships.

### External and ventilation loss follow temperature difference

This spec verifies that external-fabric and ventilation losses increase with indoor-outdoor temperature difference, including the ground-temperature path for ground-coupled elements.

### Wall conduction is proportional to temperature difference

This spec verifies that inter-room wall conduction scales linearly with temperature difference, preserving the coupled-room energy-balance model.

### Solar gain follows orientation and PV irradiance conversion

This spec verifies that solar-gain helpers respect glazing orientation and the PV-derived SW irradiance conversion, so room gains respond consistently to directional inputs.

### Door state override preserves chimney state

This spec verifies that blanket door-closing logic leaves the chimney doorway exception untouched, preserving the model's permanent stack-effect path.

## Solar position and irradiance helpers

These tests pin the pure astronomical and window-averaging helpers that turn weather inputs into orientation-specific solar gains for the thermal model.

### Solar position varies with time and season

This spec verifies that solar altitude and azimuth change sensibly across day and season, so impossible sun positions cannot leak into irradiance calculations.

### Surface irradiance is non-negative and respects geometry

This spec verifies that plane-of-array irradiance stays non-negative and responds correctly to surface tilt and azimuth, including zero output when the sun is below the horizon and half-diffuse-only gain for back-facing vertical surfaces.

### Window irradiance averaging handles partial and empty windows

This spec verifies that irradiance window averaging uses in-window samples when available and falls back safely for partial or empty windows, selecting the full nearest tuple by window-midpoint distance when no sample lands inside.

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

### DHW enter threshold is inclusive

This spec verifies that a flow rate exactly on the DHW enter threshold immediately classifies as DHW from heating, preserving the intended inclusive entry edge of the hysteresis band.

### DHW exit threshold is exclusive

This spec verifies that a flow rate exactly on the DHW exit threshold stays in DHW until it falls below the boundary, preserving the asymmetric hysteresis that prevents chatter.

### Defrost entry from heating preserves heating as pre-defrost

This spec verifies that when defrost occurs from a heating state, recovery in the transition band returns to heating (the pre-defrost circuit) rather than guessing DHW.

### Missing flow rate defaults to heating not DHW

This spec verifies that when flow rate readings are absent (None), the state machine defaults to heating rather than misclassifying as DHW.

### Defrost DT boundary is exclusive

This spec verifies that the defrost DT threshold is strictly less-than (exclusive), so a sample exactly at the threshold stays in the current circuit rather than entering defrost.

### Enrich derives state delta-T and running-only COP

This spec verifies that analysis row enrichment adds `state` and `delta_t`, while leaving COP unset for non-running rows so idle samples do not fabricate efficiency.

### Enrich keeps COP unset at the running-power threshold

This spec verifies that a sample exactly on the running-power threshold stays non-running for COP purposes, so the strict greater-than boundary cannot fabricate efficiency at the idle edge.

### Enrich preserves null delta-T when temperatures are missing

This spec verifies that enriched output keeps `delta_t` null when flow or return temperatures are missing, even though the classifier internally uses a zero fallback to avoid crashing on sparse rows.

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

### parse_ts_val handles RFC3339 and naive timestamp formats

This spec verifies that the timestamp-value parser accepts standard RFC3339, Z-suffixed, and naive ISO formats, uses alternative column names (_time/time, _value/value), and skips rows with unparseable values.

### best_volume prefers definitive then hint then cumulative

This spec verifies the volume priority chain: definitive_cumulative is preferred, then hint_cumulative, then cumulative_since_charge as fallback, ensuring the most reliable measurement is always used.

### epoch_to_dt converts Unix epoch to DateTime

This spec verifies that Unix epoch seconds are correctly converted to a UTC-offset DateTime for consistent timestamp formatting in DHW session output.

### Empty capacity input returns no_data recommendation

This spec verifies that compute_recommended_capacity returns None with method "no_data" when given an empty slice, so callers never produce a spurious recommendation from zero measurements.

### Low variance cold mains falls back to conservative ratio

This spec verifies that nearly identical cold-mains temperatures (variance <= 0.1) skip regression and apply the conservative 3% haircut instead.

### Cold mains regression extrapolates above measured maximum

This spec verifies that with sufficient cold-mains temperature spread, the linear regression extrapolates usable volume upward to a projected WWHR inlet of 25C, producing a recommendation above the highest measured cold value.

### JSON summary serialises capacity and draw counts

This spec verifies that json_summary produces the expected JSON structure with max_usable_litres, geometric_max_litres, plug_flow_efficiency, recommended fields, draw type counts, and total_draws from a set of inflection results.

### JSON summary handles empty results

This spec verifies that json_summary produces null for capacity fields and zero for all counts when given an empty results slice, so JSON consumers always get a well-formed response.

## Overnight optimizer helpers

These tests pin pure helper rules used by the historical overnight optimiser, especially where tariff-window scheduling and temperature-bin lookup must remain stable.

### Heating lookup clamps to edge bins outside the calibrated range

This spec verifies that outside temperatures below or above the calibrated heating bins reuse the nearest edge bin rather than falling through to unrealistic defaults.

### Generated overnight schedules keep DHW inside the Cosy window

This spec verifies that generated overnight strategies only place DHW starts whose full duration fits inside the supplied morning Cosy tariff window.

### Generated schedules omit DHW modes that cannot fit the Cosy window

This spec verifies that schedule generation drops DHW options whose full duration cannot fit inside the supplied Cosy tariff window, rather than emitting impossible starts.

### offset_for_hour and fmt_offset round-trip correctly

This spec verifies the hour-to-minute-offset conversion handles both post-20:00 and pre-20:00 branches correctly, and that fmt_offset reverses the mapping back to HH:MM strings.

### Empty heating bins return hardcoded fallback

This spec verifies that lookup_heating with an empty bin slice returns the hardcoded safe default (3500 W heat, 700 W elec, COP 5.0) rather than panicking or returning zero.

### calibrate_dhw with no nights returns safe defaults

This spec verifies that calibrate_dhw with no overnight data returns the conservative fallback (60 min duration, 0 cycles) rather than dividing by zero.

### Narrow Cosy window omits all DHW options

This spec verifies that when the Cosy window is too short for any DHW mode duration, all generated schedules have no DHW start rather than emitting impossible schedules.

### minute_timestamp_utc exact match returns stored timestamp

This spec verifies that binary-search timestamp lookup returns the stored timestamp when the requested offset exactly matches a minute in the night, and interpolates from the first minute when the offset is absent.

### calibrate_heating bins by outside temperature and classifies recovery vs maintenance

This spec verifies that heating calibration bins minute-resolution data by outside temperature and classifies recovery (rising indoor_t) vs maintenance (flat indoor_t), filtering non-heating states and requiring sufficient samples per bin.

### calibrate_dhw extracts valid DHW cycles from nights

This spec verifies that DHW calibration extracts cycles within the valid duration range (30–180 minutes), computes correct average duration and electricity, and filters short top-up cycles.

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

### summaries_from_batch_rows pivots metrics into NumericSummary

This spec verifies that batched Flux CSV rows are correctly pivoted by series and metric into NumericSummary structs, including single-sample series retention.

### summaries_from_batch_rows drops zero-sample series

This spec verifies that series with no count metric row (implying zero samples) are dropped from the output, preventing empty summaries from polluting history output.

### numeric_values_from_batch_rows parses keyed values

This spec verifies that batched numeric selector rows are parsed into (series, metric) keyed values, skipping header sentinels and empty values.

### string_values_from_batch_rows skips empty and sentinel values

This spec verifies that string value extraction skips empty strings and the CSV header sentinel "_value", retaining only meaningful values.

### controller_rows_target_series filters None targets

This spec verifies that target-flow series extraction drops rows where target_flow_c is None, producing only rows with actual flow targets.

### Controller rows match between Flux and PostgreSQL on a representative window

This ignored integration spec verifies that `query_controller_rows` returns the same controller events from Flux and PostgreSQL over one representative live window.

It preserves timestamps, labels, and numeric optionals while the legacy adaptive-heating-mvp reader path still exists.

### numeric_points_from_series maps DateTime-f64 pairs

This spec verifies that numeric time-series helpers preserve timestamp ordering and pair each DateTime with its numeric value when converting summary evidence into display-ready points.

### numeric_points_from_series returns empty for empty input

This spec verifies that numeric point extraction returns an empty vector for missing series data, avoiding invented placeholder points in history output.

### string_points_from_series maps DateTime-String pairs

This spec verifies that string time-series helpers preserve timestamp ordering and pair each DateTime with the original string payload for status-style evidence.

### string_points_from_series returns empty for empty input

This spec verifies that string point extraction returns an empty vector for missing series data, keeping absent categorical evidence distinct from empty strings.

### controller_event_from_row copies all fields

This spec verifies that controller-event row parsing copies every present field into the typed event struct so history summaries retain full mode, target, and explanatory context.

### controller_event_from_row preserves None optionals

This spec verifies that controller-event parsing leaves optional fields unset when the source row omits them, rather than synthesising misleading defaults.

### batch_summary_union_flux builds union with all metrics

This spec verifies that batched summary-query construction unions every requested metric subquery into one Flux program, preserving per-series summary coverage while reducing query round-trips.

### batch_metric_selector_union_flux builds union from specs

This spec verifies that selector-query batching unions the generated Flux fragments for all requested series and metrics into one executable query.

### batch_metric_selector_union_flux returns empty for empty input

This spec verifies that selector-query batching returns an empty query for an empty spec list, avoiding malformed Flux when no selectors were requested.

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

### Aggregate metrics flatten residuals across windows

This spec verifies validation aggregates recompute metrics from every room residual across all windows, rather than averaging per-window summaries that could distort RMSE or tolerance fractions.

### Aggregate whole-house contributors merge repeated rooms across windows

This spec verifies aggregate whole-house reporting merges repeated room contributions across windows before sorting contributors, so recurring problem rooms are ranked by their cumulative error.

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

### Validation gate requires aggregate pass when enabled

This spec verifies that `thermal-validate` comparisons fail when the candidate artifact reports `aggregate_pass=false` and the default `require_aggregate_pass` gate is still enabled.

### Validation gate can disable aggregate pass enforcement

This spec verifies that `thermal-validate` can skip the aggregate-pass boolean gate when thresholds explicitly disable it, while still keeping the numeric regression checks active.

### Operational artifacts fail on large record drops or param drift

This spec verifies that `thermal-operational` comparisons reject candidates when record count drops beyond the configured fraction or calibrated parameters drift past their absolute-delta gate.

### Calibrate artifacts fail on score or parameter drift

This spec verifies that `thermal-calibrate` comparisons reject candidates when calibration score or calibrated parameters drift beyond their absolute-delta gates.

### Fit diagnostics artifacts fail on count or parameter drift

This spec verifies that `thermal-fit-diagnostics` comparisons reject candidates when true-cooling counts drop beyond their configured fraction or calibrated parameters drift past their absolute-delta gate.

### Fit diagnostics med_ratio delta fails when present

This spec verifies that when both fit-diagnostics artifacts report numeric `med_ratio` values, the comparison enforces the configured absolute-delta gate instead of silently skipping the metric.

### Fit diagnostics records drop fails when baseline is nonzero

This spec verifies that `thermal-fit-diagnostics` comparisons reject candidates whose overall record count drops beyond the configured fraction when the baseline artifact has a nonzero record set.

## InfluxDB wire-format parsing

These tests are now legacy-compatibility specs for the remaining Flux/CSV migration tail. They should stay only while raw Influx parsing still exists; end-state PostgreSQL work belongs in [[tsdb-migration]].

### Empty CSV input returns empty vec

This spec verifies that an empty string produces zero rows, establishing the base case for the parser.

### Annotation lines are skipped

This spec verifies that InfluxDB annotation lines (starting with `#`) are excluded from output and that data rows after annotations are correctly parsed with their header-keyed values.

### Multi-table CSV resets headers per table

This spec verifies that when InfluxDB emits multiple result tables separated by blank lines and new annotation blocks, rows from each table are independently parsed and included in the output.

### Duplicate header rows are not emitted as data

This spec verifies that when InfluxDB repeats the header row between result blocks, those duplicate rows are recognized and excluded from the output rather than appearing as data.

### Empty-key columns are excluded from output map

This spec verifies that CSV columns with empty header names (common for the InfluxDB annotation column) are not included in the output HashMap, preventing empty-string keys from polluting consumer logic.

### All-annotation CSV returns empty vec

This spec verifies that a CSV containing only annotation lines and no data rows produces an empty result rather than erroring.

### parse_dt accepts standard RFC3339 formats

This spec verifies that the timestamp parser handles Z-suffixed, explicit +00:00, and non-zero offset RFC3339 formats, producing correct and equivalent Unix timestamps.

### parse_dt rejects invalid timestamp input

This spec verifies that incomplete timestamps without timezone information, bare text, and empty strings are rejected so row adapters fail loudly on malformed source data.

## Query return contracts

These tests pin the typed output shape that TSDB readers must preserve across the migration. The transport may change, but the returned field names, types, and sort order must remain stable for callers.

### Room temps extracts timestamp-topic-value triples

This spec verifies that room temperature query results are exposed as (DateTime, topic_string, f64) triples sorted by timestamp, preserving both the topic identifier and the numeric value for multi-sensor queries.

### Outside temp extracts timestamp-value pairs sorted by time

This spec verifies that outside temperature query results are exposed as (DateTime, f64) pairs sorted by timestamp, matching the contract that calibration and display modules depend on.

### Status codes round float to integer

This spec verifies that status code values (categorical, not numeric) are exposed as rounded integers, matching the HP state classification contract.

### MWT CSV with flow and return produces averaged pairs

This spec verifies that the mean water temperature query produces (DateTime, f64) pairs from the pre-computed flow/return average, matching the contract that the thermal solver depends on.

Despite the heading's historical wording, this is now a transport-agnostic reader contract.

### Missing required column returns MissingColumn error

This spec verifies that when a required value column is absent from a row-shaped response, the consumer returns the explicit `MissingColumn` variant with the expected column/context rather than silently producing bad data.

### Unparseable float in value column returns FloatParse error

This spec verifies that non-numeric values in the _value column are detected at parse time and reported via the explicit `FloatParse` variant with the original bad value, rather than propagating as NaN or zero.

### Multi-topic query builds OR conditions with correct field names

This spec verifies that multi-topic queries build correct per-topic _field conditions (temperature for Zigbee, value for emonth/ebusd) joined with OR. Documents the routing SQL must replicate.

### Wide-row CSV with NULL columns parses present fields only

This spec verifies that CSV rows with empty _value fields (from wide-row NULLs like ct_monitor P7–P12 on 6-channel devices) parse without error but don't produce valid floats, so consumers can distinguish present from absent data.

### Single-value CSV parsing extracts last value

This spec verifies that a single-value latest-reading result is correctly reduced to the returned value, matching the contract used by the adaptive-heating-mvp live daemon.

### Empty result from last query returns no rows

This spec verifies that when a sensor has no data in the lookback window, the query adapter returns zero rows rather than erroring, so callers can safely handle the None case.

## Topic to table routing

These tests document the canonical mapping from logical telemetry topics to PostgreSQL/TimescaleDB tables and columns. Historical Influx topic/tag shapes matter only because the PostgreSQL routing must preserve the same reader semantics.

### Room sensor topics use correct field name

This spec verifies that the _field distinction is preserved: Zigbee sensors use "temperature", emonth2_23 and ebusd/poll use "value". Incorrect routing silently returns empty results.

### Topic prefix maps to TimescaleDB table

This spec verifies the complete topic-to-table mapping: emon/EmonPi2 → ct_monitor, emon/tesla → tesla, ebusd/poll → ebusd_poll, zigbee2mqtt → zigbee, and all other sensor topic prefixes to their correct tables.

### Live eBUS topics map to ebusd field names

This spec verifies that live `ebusd/<circuit>/<field>` topics such as `ebusd/hmu/CurrentYieldPower` route to the shared `ebusd` field/value table by using the final path segment as the field name.

### PV power topic maps to ct_monitor P3 column

This spec verifies that the emon/EmonPi2/P3 topic is decomposed into source=EmonPi2 and column=P3 in the wide ct_monitor table.

## Timestamp migration contracts

These tests pin timestamp handling constraints that the PostgreSQL/TimescaleDB path must preserve while the remaining legacy Flux/Influx compatibility tail still exists.

### Microsecond truncation preserves 10s-interval data

This spec verifies that truncating InfluxDB nanosecond timestamps to TimescaleDB microsecond precision does not alter the seconds-level timestamp, which is safe at 10s sample intervals.

### PostgreSQL TIMESTAMPTZ offset formats parse correctly

This spec verifies that the shared timestamp parser accepts PostgreSQL TIMESTAMPTZ text forms with space separators, short offsets, and fractional seconds so migration-path readers do not depend on RFC3339-only formatting.

## DHW write contracts

These tests pin DHW session write semantics across the migration. Legacy line-protocol coverage remains as a compatibility harness, but PostgreSQL column population is the target contract.

### dhw_inflection LP line contains all required fields

This spec verifies that the dhw_inflection LP line includes all 11 numeric fields and 3 tag fields that the TimescaleDB schema defines, so no column is silently NULL after migration.

### parse_ts_val handles naive timestamps from PostgreSQL

This spec verifies that the NaiveDateTime fallback in parse_ts_val correctly parses ISO timestamps without timezone offset, which is how PostgreSQL may return TIMESTAMPTZ values depending on client configuration.

### 10s resolution query produces one sample per 10 seconds

This spec verifies that DHW event-detection queries at 10s resolution produce exactly 6 samples per minute with 10s spacing, documenting the resolution contract the SQL migration must match.

### LP tag spaces replaced with underscores

This spec verifies that DHW LP tag values (category, draw_type) never contain spaces, preventing LP format parsing errors.

### find_events measurement filter routes to correct PG tables

This spec verifies that dhw_sessions find_events measurement-based filters route to the correct PostgreSQL tables: emon+dhw_ fields → multical, ebusd_poll → ebusd_poll. This is distinct from influx.rs topic-based routing.

### find_events uses triple-field filter for emon measurements

This spec verifies that emon measurement queries use the triple-filter pattern (_measurement + _field="value" + field=name). In PostgreSQL this collapses to a direct column SELECT from the multical table.

### Postgres inflection row maps all LP tags and fields to columns

This spec verifies that the PostgreSQL dhw_inflection mirror uses the same category/crossover/draw_type tags and 11 numeric values as the legacy LP row, preserving row-equivalent semantics across transports.

### dhw_capacity LP line maps to TimescaleDB columns

This spec verifies that the dhw_capacity LP line includes recommended_full_litres and method fields matching the TimescaleDB schema.

### Optional postgres conninfo is read from env when configured

This spec verifies that the staged TSDB migration can enable `dhw_capacity` TimescaleDB mirroring via `[postgres].conninfo_env` without putting database credentials in TOML.

## Adaptive heating write contracts

These tests pin PostgreSQL-first adaptive-heating decision write semantics now that the controller no longer relies on Flux fallback reads or Influx line-protocol mirroring.

### Decision PostgreSQL row maps tags and boolean fields correctly

This spec verifies that the PostgreSQL write-row helper normalizes tags, maps battery adequacy to FLOAT8 1.0/0.0, and preserves NULL when the signal is absent.

### Decision PostgreSQL row keeps line-protocol second precision

This spec verifies that the PostgreSQL write-row helper truncates controller decision timestamps to whole seconds so staged SQL rows can match the existing Influx line-protocol write precision during parity checks.

### Real PostgreSQL decision insert includes explicit timestamp

This spec verifies that a real TimescaleDB insert for `adaptive_heating_mvp` writes an explicit recent timestamp rather than relying on LP-style server-time behaviour.

### Real PostgreSQL decision insert preserves column types and values

This spec verifies that a real TimescaleDB insert round-trips the expected TEXT and FLOAT8 values for controller decision rows, including tag columns and the boolean-as-float field.

### Room temp field routing matches influx.rs contract

This spec verifies that the adaptive-heating-mvp latest-room-temperature helpers use the same value-vs-temperature field split and PostgreSQL topic routing as `influx.rs`, so controller reads stay aligned with the shared TSDB reader contract.

### Latest topic routing covers Tesla and Multical sources

This spec verifies that latest-value topic routing sends `emon/tesla/*` topics to Tesla columns and `emon/multical/*` topics to Multical columns, including the `dhw_volume_V1` name-normalization edge.

### DHW T1 query uses value field

This spec verifies that DHW T1 latest-value reads keep the emon measurement `value` field semantics and route to the PostgreSQL `multical.dhw_t1` column, including the `dhw_volume_V1` name-normalization edge.

### Measurement routing covers ebusd_poll and direct tables

This spec verifies that latest-value measurement reads keep `ebusd_poll` on the field/value table while routing other measurements through direct table/column selection.

### Controller resolves PostgreSQL conninfo from configured env name

This spec verifies that the controller reads PostgreSQL conninfo from whatever `postgres.conninfo_env` names, so service config can move the secret without code changes.

### Missing controller PostgreSQL conninfo names the expected env

This spec verifies that when the configured controller PostgreSQL env var is absent, the error names that exact env var so startup failures point operators at the right secret.

### Controller lookback cutoff supports minute hour and day windows

This spec verifies that latest-value read lookbacks accept the controller's supported `-Nm`, `-Nh`, and `-Nd` forms and convert them into approximately the expected UTC cutoff offsets.

### Controller lookback cutoff rejects malformed windows

This spec verifies that malformed latest-value lookbacks fail fast and mention the original bad input, so TSDB-read regressions surface as diagnosable startup or status errors.

### Controller quoted identifiers wrap simple column names

This spec verifies that dynamic PostgreSQL column identifiers are wrapped in double quotes, so latest-value queries keep exact column names instead of relying on unsafe bare identifiers.

### Controller quoted identifiers escape embedded quotes

This spec verifies that embedded double quotes inside dynamic PostgreSQL identifiers are doubled before interpolation, preventing malformed SQL when unusual identifier text is quoted.

### Unsupported latest topic routes fail before PostgreSQL connect

This spec verifies that an unsupported latest-value topic is rejected before any PostgreSQL connection attempt, so route bugs surface as deterministic contract failures rather than misleading connection errors.

### Unsupported latest topic route errors name the offending topic

This spec verifies that unsupported latest-value topic errors include the exact offending topic string, so operators can diagnose misconfigured controller topics from the failure message alone.

## History filter variant routing

These tests document the three history.rs filter patterns and their PostgreSQL table routing implications. Remaining Flux-backed paths here are compatibility/parity tail work rather than the default operator route.

### Topic filter routes by topic prefix and field name

This spec verifies that TopicSummarySpec queries route by topic prefix to the correct PG table and that the _field distinction (value for emonth, temperature for zigbee) is preserved.

### Measurement filter routes by measurement name and field tag

This spec verifies that MeasurementSummarySpec queries route by _measurement to the correct PG table. Notably, measurement="emon" with dhw_ fields routes to multical, not emon.

It also preserves the `dhw_volume_V1` to `dhw_volume_v1` column-normalization edge.

### Measurement filter routing covers live ebusd fields

This spec verifies that numeric MeasurementSummarySpec queries route live `ebusd` fields through the shared `ebusd.value` table with the field name carried as the selector.

### Text ebusd_poll fields route to ebusd_poll_text

This spec verifies that string-valued `ebusd_poll` history fields such as `Statuscode` route to the sibling `ebusd_poll_text` hypertable under PostgreSQL, while numeric `ebusd_poll` reads stay on the numeric table.

### Native text measurements route to their direct tables

This spec verifies that repo-native text measurements stay on their own PostgreSQL tables instead of being misrouted through `ebusd_poll_text`.

It covers `dhw.charge_state`, `dhw_capacity.method`, `dhw_inflection.category/draw_type`, and adaptive controller labels.

### HwcSFMode reads use ebusd live field semantics

This spec verifies that `HwcSFMode` history evidence reads from the live `ebusd` field (`circuit=700`, `field=HwcSFMode`) for both Flux and PostgreSQL, because that signal is not actually stored as a string-valued `ebusd_poll` series.

### Plain measurement filter uses underscore field

This spec verifies that PlainMeasurementSummarySpec queries use r._field (underscore), a third distinct pattern where _measurement maps directly to the PG table name and _field maps directly to the column name.

### Active-series periods respect baseline carry and minimum duration

This spec verifies that active/inactive boolean series expand into periods that can start active at the window boundary and drop sub-threshold durations.

### Numeric summaries choose extrema by value not recency

This spec verifies that numeric summary extrema come from the lowest and highest values even when they are not the first or last samples, while `latest` still tracks the final sample.

## Display migration contracts

These tests pin display-module routing contracts that the PostgreSQL path must preserve once the migration tail is removed.

### Humidity query skips emonth2 topic

This spec verifies that humidity queries skip the emon/emonth2_23/temperature topic because that sensor does not report humidity, preventing empty or error results in the PostgreSQL equivalent.

### Humidity uses humidity field not temperature

This spec verifies that humidity queries use _field="humidity" for all topics, distinct from temperature queries. In PostgreSQL, this maps to the humidity column in the zigbee table.
