---
lat:
  require-code-mention: true
---
# Tests

Targeted executable specs for controller invariants that are safety- or cost-sensitive enough to require explicit code coverage.

## Adaptive heating controller

These tests pin the controller rules that must not drift silently as overnight and DHW scheduling logic evolves.

### Overnight coast guard near waking

This spec verifies the overnight coast rule stops at the 30-minute pre-wake boundary so the controller cannot coast through the final warm-up window.

### Battery headroom threshold depends on DHW mode

This spec verifies Eco and normal DHW modes use different energy budgets when deciding whether battery headroom can carry a charge event to the next Cosy slot.

### Cosy windows ignore battery gating for DHW

This spec verifies DHW launches in every Cosy slot even when the battery signal is low or missing, matching the rule that cheap-grid periods must not be headroom-gated.

### Battery headroom adequacy is monotonic

This spec verifies that increasing discretionary battery headroom cannot make the adequacy decision worse for a fixed DHW mode.

### Eco-mode overnight DHW bypasses the battery gate

This spec verifies that overnight DHW in eco mode launches even when the discretionary headroom signal is inadequate, because the smaller eco charge should not be deferred to the next Cosy slot.

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

## History evidence helpers

These tests pin the pure helper logic behind retrospective heating-history summaries, especially where overnight periods must be interpreted differently from waking-hour comfort issues.

### Waking-hour clipping excludes overnight-only time

This spec verifies that comfort-miss periods are clipped to 07:00–23:00 and split by day boundaries, so expected overnight cooling is not misreported as a comfort failure.

### Sawtooth detection requires repeated alternation

This spec verifies that history review only flags sawtooth behaviour after repeated significant curve-direction alternations, avoiding false positives from one-off adjustments.

### Sampling stats report interval summary

This spec verifies that timestamp sampling summaries report sample count plus min, median, and max step sizes for irregular but ordered telemetry.

## Thermal regression gates

These tests pin the JSON-artifact comparison gates that protect thermal baselines from being compared across incompatible commands or configs.

### Global regression gate requires matching command and config

This spec verifies that regression checking fails before metric comparisons when artifact commands or config hashes differ, matching the reproducibility contract for thermal snapshots.
