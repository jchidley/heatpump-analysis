# Plan

Open items, next steps, and links to the detailed human-readable plan documents in `docs/`.

## Heating Controller

V2 model-predictive controller is live. See [[heating-control]] for current behaviour and [[heating-control#Pilot History]] for operational findings.

Detailed plan: [`docs/heating-plan.md`](../docs/heating-plan.md)

### Fixed: Forecast Nulls During DHW

Was: controller went blind during DHW charges — forecast and model fields all null. Fixed by separating model calculation from actuation.

Root cause: the `!is_dhw` guard on the heating control block skipped the entire model calculation, not just the eBUS writes. Fix: removed `!is_dhw` from the guard; the model (forecast + thermal solver) now runs every tick regardless of HP mode. During DHW, writes are suppressed but `target_flow_c` stays populated and action is logged as `dhw_active` with full model fields. Confirmed on both 5 and 6 Apr nights (up to 12 blind ticks per night). The 6 Apr overnight occurrence contributed to a 0.4°C comfort miss.

### Open: Headroom Unreliable During Cosy

Energy-hub headroom doesn’t account for active grid charging during Cosy windows.

Shows large negatives early in Cosy, swings positive as battery fills. No impact on control (controller ignores headroom during Cosy) but misleading for observability. Fix: return null or project through remaining Cosy charging.

### Fixed: Overnight Ramp Replaced with Coast-Then-Hold

Linear ramp back-loaded the hardest temperature rise into the final hours; the controller could never catch up (0.3°C below target at 07:00 on 7 Apr). Replaced with flat comfort-floor target (20.0°C). **Needs cross-compile and deploy to pi5data.**

Physics: total electrical cost = ∫ Q_hp/COP(T_flow) dt. COP degrades with flow temp, so the minimum-electrical strategy is to coast for free then hold the comfort-band floor at the lowest possible flow. Simulation at outside 9.5°C: ramp used 2.82 kWh; coast-then-hold uses 1.86 kWh (−34%). See [[heating-control#Overnight Strategy#Trajectory Logic]] for the full rationale.

### Progressing: Overnight Data Growing

Now 5+ overnight-model nights logged. Still need cold (<5°C) night (unlikely before next winter).

4 Apr confounded (MinFlow=20), 5 Apr success at 9–12°C, 6 Apr success at 7–9°C (slight undershoot due to DHW contention during preheat). **6–7 Apr was the strongest trajectory night yet**: model drove curve 0.76→0.51 over 5 hours (01:07–06:00), maintaining Leather at 20.1–20.2°C with outside 8.5–10°C. Flow temps held at 28.4–29.9°C throughout. Average overnight COP = 5.81 across 16 active ticks. A 2.6-hour coast phase (22:30–01:07) showed Leather dropping 20.7→20.1°C with heating off, implying τ≈44h (consistent with operational τ=36h within single-observation variance). Battery reached 100% by 03:57. This night’s data confirmed the ramp problem and motivated the coast-then-hold fix above.

### Fixed: DHW Timer Dedup Bug

VRC 700 fired DHW at 04:00 during preheat because morning timer window was left enabled.

Root cause on 6 Apr: `sync_morning_dhw_timer` correctly decided to skip the morning window (T1 41.5°C predicted, above 40°C trigger) but the eBUS write failed ("ERR: element not found"). Dedup state was updated anyway, suppressing retries. Then `restore_baseline` on restart re-enabled all timer windows without clearing the dedup state, so the skip was never retried. VRC 700 saw HwcStorageTemp 37.5°C < 45°C target and fired its own charge at 04:00. Two fixes: (1) `sync_morning_dhw_timer` now checks for `ERR:` in the response and clears dedup state on failure, (2) `control_loop` startup clears dedup state so the first tick always re-evaluates.

### Actionable: Elvina Overnight Comfort

Child’s bedroom hits 16.4–17.5°C at 07:00 on mild nights. Trickle vents are the entire problem — roof insulation is fine.

Full proxy-network moisture analysis (13 sensors, 6 nights): Elvina ventilates 6.8× faster than Aldora (ACH ≈ 1.0 vs model 0.51). Fabric residual 11 W/K is below model 14.5.

**Proposed fix: close trickle vents, rely on HEPA purifier for allergen control.** The LEVOIT Core 300 (CADR 187 m³/h = 3.1 filtered ACH in 60 m³ room, 20W) already runs and provides better allergen control with vents closed (no outdoor pollen ingress, 99.97% HEPA per pass). Closing vents cuts UA from ~32 to ~17 W/K, raising overnight temp by ~3°C. Part F fresh air shortfall with vents closed: need ~14 L/s, infiltration provides ~2.5 L/s. Mitigate with door ajar or morning purge vent. Validate with £15 CO2 monitor (≤ 1000 ppm target). No controller changes needed — room simply retains more heat at the same flow temperature.

**Measurement plan**: (1) deploy SNZB-02P to shaded SE wall as `outside_temp_humid` (conservatory covered by VRC 700 Z2RoomTemp), giving direct AH_out + OAT cross-check; (2) record baseline Elvina overnight AH and temperature for 1 week; (3) close trickle vents; (4) record post-change for 1 week; (5) compare ΔAH rise (confirms ACH reduction), temperature gain, and CO2 (if monitor fitted).

### Open: Wind and PV Tuning

Wind compensation and PV-aware curve adjustment are modelled but not tuned against real data. Low urgency until weather provides test cases (windy cold day, sustained high PV day with heating demand).

## DHW Scheduling

DHW scheduling operational within the adaptive controller. See [[heating-control#Overnight Strategy#Active DHW Scheduling]] for current logic and [[domain#DHW Cylinder]] for cylinder facts.

Detailed plan: [`docs/dhw-plan.md`](../docs/dhw-plan.md)

### Open: Volume-Aware DHW Demand Prediction

T1 standby decay is calibrated but the model assumes no draws occur.

On 47% of nights there's an overnight shower (avg 62L, max 120L). The 27 Mar night showed the risk: a 120L shower at 23:23 dropped T1 from 43.5→~37°C, below the 40°C comfort floor, and the model would have predicted 41.8°C. Demand slots aligned to Cosy charge windows: morning 07:00–13:00 (71% of days, avg 89L), afternoon 16:00–22:00 (24%, avg 72L), overnight 22:00–04:00 (47%, avg 62L). Next step: budget expected demand per slot using `dhw_capacity` from InfluxDB alongside T1.

### Open: Seasonal Eco→Normal Switch

`hmu HwcMode` is read-only from eBUS — must be changed physically on the aroTHERM controller. Switch to normal (2.4 kWh charges) around November. No software fix possible.

## Pico eBUS Adapter

Replacing the closed-source ESP32 firmware with Rust/Embassy on a Pi Pico W. Phase 1 (`ebus-core/` crate, 22 tests) complete. See [[infrastructure#eBUS Stack]] for the current live stack.

Detailed plan: [`docs/pico-ebus-plan.md`](../docs/pico-ebus-plan.md)

### Next: Phase 2 — PIO UART

PIO RX + TX at 2400/8N1 on the Pico W. Test with loopback wire (GP4→GP5), verify timing with Saleae. Prerequisites: Pico W board, xyzroe eBus-TTL adapter (confirm purchased), Embassy runtime + PIO crate setup.
