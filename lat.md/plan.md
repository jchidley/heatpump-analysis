# Plan

Open items, next steps, and links to the detailed human-readable plan documents in `docs/`. Last status refresh: **2026-04-09 08:55 BST**.

## Heating Controller

V2 model-predictive controller is live. See [[heating-control]] for current behaviour and [[heating-control#Pilot History]] for operational findings.

Detailed plan: [`docs/heating-plan.md`](../docs/heating-plan.md)

### Fixed: Forecast Nulls During DHW

Was: controller went blind during DHW charges - forecast and model fields all null. Fixed by separating model calculation from actuation. **Deployed 7 Apr 10:34.**

Root cause: the `!is_dhw` guard on the heating control block skipped the entire model calculation, not just the eBUS writes. Fix: removed `!is_dhw` from the guard; the model (forecast + thermal solver) now runs every tick regardless of HP mode. During DHW, writes are suppressed but `target_flow_c` stays populated and action is logged as `dhw_active` with full model fields. Confirmed on both 5 and 6 Apr nights (up to 12 blind ticks per night). The 6 Apr overnight occurrence contributed to a 0.4°C comfort miss.

### Open: Headroom Unreliable During Cosy

Energy-hub headroom doesn't account for active grid charging during Cosy windows.

Shows large negatives early in Cosy, swings positive as battery fills. No impact on control (controller ignores headroom during Cosy) but misleading for observability. Fix: return null or project through remaining Cosy charging.

**8 Apr data review (08:33 BST)**: the upstream Tesla headroom feed itself looked sane during Cosy morning (SoC ~91→100%, headroom +6.5→+9.1 kWh), but the controller could not read it because the production INFLUX_TOKEN outage left `battery_headroom_to_next_cosy_kwh` null on every outer tick. Item remains open, but controller-side validation is blocked by the outage.

**9 Apr data review (08:55 BST)**: The morning Cosy window (04:00–07:00 BST) showed positive headroom (+8.5 to +9.1 kWh) rather than negative, but this was because the battery started the window at 95% SoC and filled quickly. The underlying logic issue during active charging from lower SoCs remains unpatched. Item remains open.

### Fixed: Overnight Ramp Replaced with Coast-Then-Hold

Linear ramp back-loaded the hardest temperature rise into the final hours; the controller could never catch up (0.3°C below target at 07:00 on 7 Apr). Replaced with flat comfort-floor target (20.0°C). **Deployed 7 Apr 10:34.**

Physics: total electrical cost = ∫ Q_hp/COP(T_flow) dt. COP degrades with flow temp, so the minimum-electrical strategy is to coast for free then hold the comfort-band floor at the lowest possible flow. Simulation at outside 9.5°C: ramp used 2.82 kWh; coast-then-hold uses 1.86 kWh (-34%). See [[heating-control#Overnight Strategy#Trajectory Logic]] for the full rationale.

### Progressing: Overnight Data Growth

Evidence collection is no longer blocked, but the first post-deploy coast-then-hold night (7–8 Apr) was lost because the controller was blind for the whole window.

Still open because we need a clean controller-evidence night, ideally one cold (<5°C) night and one warmer >12°C heating day. Earlier nights used the old ramp.

4 Apr confounded (MinFlow=20), 5 Apr success at 9–12°C, 6 Apr success at 7–9°C (slight undershoot due to DHW contention during preheat). **6–7 Apr was the last ramp night and the strongest trajectory night**: model drove curve 0.76→0.51 over 5 hours (01:07–06:00), maintaining Leather at 20.1–20.2°C with outside 8.5–10°C. Flow temps held at 28.4–29.9°C throughout. Average overnight COP = 5.81 across 16 active ticks. A 2.6-hour coast phase (22:30–01:07) showed Leather dropping 20.7→20.1°C with heating off, implying τ≈44h (consistent with operational τ=36h within single-observation variance). Battery reached 100% by 03:57. This night's data confirmed the ramp problem and motivated the coast-then-hold fix. **7–8 Apr will be the first coast-then-hold night.**

**7 Apr data review (11:14 BST)**: 6–7 Apr overnight confirmed: coast 00:33–01:07 (20.5→20.1°C), model held 20.1–20.2°C for 8h, 07:00 miss of −0.3°C vs 20.5°C target. Forecast API degraded from ~05:00 (Open-Meteo errors). Aldora sensor offline all night. Post-deploy at 10:34: DHW fix confirmed (model fields populated during dhw_active). See `docs/data-review-log.md` for full details.

**7 Apr data review (19:07 BST)**: ⚠️ **7–8 Apr coast-then-hold night is BLOCKED** — see critical issue below. Outer loop has been blind since 16:44 BST (INFLUX_TOKEN stripped from production env during tech-debt restarts). No decision log written since 16:38 BST tick. Tonight's overnight data will be lost unless the token is restored before ~22:00 BST.

**8 Apr data review (08:33 BST)**: 7–8 Apr did **not** add usable controller evidence. Journal shows 50 outer ticks from 19:22 BST to 08:18 BST with Leather/Aldora/T1/battery/forecast/model fields all null, action always `hold`, and every decision-log write failing. Even so, the plant and weather compensation kept Leather at 20.5°C by 08:00 (min 20.5°C), Aldora at 20.6–20.8°C, and outside at 10.0–12.3°C. Treat this as a comfort-preserved outage night, not a coast-then-hold validation night.

**8 Apr data review (16:50 BST)**: production evidence collection has recovered (31 controller events recorded since the credential fix, with decision logs and forecast refreshes back), but this review window only covers daytime operation. Leather stayed 20.6→21.6°C with zero comfort misses, yet we still do **not** have a clean post-recovery overnight coast-then-hold window. Status stays progressing; the next useful checkpoint is the 8–9 Apr night.

**9 Apr data review (08:55 BST)**: The 8–9 Apr night provided a clean post-recovery evidence window. The controller correctly initiated an `overnight_coast` at 23:04 BST (Leather 21.9°C, outside 18.3°C) and held it for 6.5 hours until 05:34 BST (Leather 20.7°C, outside 15.1°C), validating the new strategy logic. However, this was an exceptionally warm night with zero frost risk. Keep this item progressing until a cold (<5°C) night is captured.

### Fixed: DHW Timer Dedup Bug

VRC 700 fired DHW at 04:00 during preheat because morning timer window was left enabled. **Deployed 7 Apr 10:34.**

Root cause on 6 Apr: `sync_morning_dhw_timer` correctly decided to skip the morning window (T1 41.5°C predicted, above 40°C trigger) but the eBUS write failed ("ERR: element not found"). Dedup state was updated anyway, suppressing retries. Then `restore_baseline` on restart re-enabled all timer windows without clearing the dedup state, so the skip was never retried. VRC 700 saw HwcStorageTemp 37.5°C < 45°C target and fired its own charge at 04:00. Two fixes: (1) `sync_morning_dhw_timer` now checks for `ERR:` in the response and clears dedup state on failure, (2) `control_loop` startup clears dedup state so the first tick always re-evaluates.

### Actionable: Elvina Overnight Comfort

Child's bedroom hits 16.4-17.5°C at 07:00 on mild nights. Trickle vents are the entire problem - roof insulation is fine.

Full proxy-network moisture analysis (13 sensors, 6 nights): Elvina ventilates 6.8× faster than Aldora (ACH ≈ 1.0 vs model 0.51). Fabric residual 11 W/K is below model 14.5.

**Proposed fix: close trickle vents, rely on HEPA purifier for allergen control.** The LEVOIT Core 300 (CADR 187 m3/h = 3.1 filtered ACH in 60 m3 room, 20W) already runs and provides better allergen control with vents closed (no outdoor pollen ingress, 99.97% HEPA per pass). Closing vents cuts UA from ~32 to ~17 W/K, raising overnight temp by ~3°C. Part F fresh air shortfall with vents closed: need ~14 L/s, infiltration provides ~2.5 L/s. Mitigate with door ajar or morning purge vent. Validate with £15 CO2 monitor (≤ 1000 ppm target). No controller changes needed - room simply retains more heat at the same flow temperature.

**Measurement plan**: (1) ~~deploy SNZB-02P to shaded SE wall as `outside_temp_humid`~~ **done 7 Apr** (paired, LQ=6 — monitor); (2) record baseline Elvina overnight AH and temperature for 1 week; (3) close trickle vents; (4) record post-change for 1 week; (5) compare ΔAH rise (confirms ACH reduction), temperature gain, and CO2 (if monitor fitted).

**8 Apr data review (08:33 BST)**: `outside_temp_humid` stayed online through the first full night after pairing, so the baseline week has started. Elvina averaged ~21.6°C around midnight and ~20.9°C at 08:00 while `outside_temp_humid` fell from ~12.6°C to ~11.0°C. No vent intervention yet — keep collecting the baseline week before changing anything.

**9 Apr data review (08:55 BST)**: The baseline week continues. The warm spell persists, and there are no signs of sensor dropout. No intervention yet — keep collecting baseline data.

### Fixed: Production Influx Secret Migration

The blind-controller outage was first fixed by restoring `INFLUX_TOKEN` in `/etc/adaptive-heating-mvp.env`, then hardened properly by migrating the controller to a dedicated systemd credential. **Restored 8 Apr 08:39 BST; hardened 8 Apr 08:57 BST.**

Root cause: systemd had been depending on `/etc/adaptive-heating-mvp.env`, and that file was overwritten without `INFLUX_TOKEN`. The controller's dev fallback (`ak get influxdb`) cannot work under systemd because there is no gpg-agent session. Permanent fix: create a dedicated InfluxDB auth for `adaptive-heating-mvp`, store it in `/etc/adaptive-heating-mvp/influx.token` (root-only), and load it via `LoadCredential=influx_token:/etc/adaptive-heating-mvp/influx.token`. `/etc/adaptive-heating-mvp.env` now remains only for non-Influx env vars such as Octopus credentials.

Verification: the running service restarts with the credential present, without relying on `INFLUX_TOKEN` in its environment. **7–8 Apr controller evidence is still lost**, but future outer ticks and decision-log writes now depend on the dedicated local credential rather than a copied env var.

### Open: Forecast API Reliability

Open-Meteo API failed intermittently from ~05:00 on 7 Apr (connection and decoding errors).

Controller degrades gracefully (uses cached forecast, falls back to live outside_temp) but prolonged outages leave forecast_outside_c stale. Consider: local caching with longer TTL, fallback to a second weather API, or alerting when forecast age exceeds threshold.

**8 Apr data review (08:33 BST)**: no fresh Open-Meteo errors appeared in controller logs after the previous review. Current `forecast_outside_c:null` entries in the journal are dominated by the wider blind-controller condition, so this item remains open but did not materially advance in this window.

**8 Apr data review (16:50 BST)**: still quiet. Journal shows successful forecast refreshes at 10:02, 11:03, 12:09, 13:12, and 14:48 BST with no new connection or decode errors, and `forecast_outside_c` remained populated on all post-recovery outer ticks. Keep this open as a known intermittent upstream risk, but it did not regress in this review window.

**9 Apr data review (08:55 BST)**: Still quiet. No Open-Meteo connection or decoding errors were logged overnight or into the morning.

### Open: Wind and PV Tuning

Wind compensation and PV-aware curve adjustment are modelled but not tuned against real data. Low urgency until weather provides test cases (windy cold day, sustained high PV day with heating demand).

**8 Apr data review (16:50 BST)**: this window gave a sunny warm spell, but not a useful tuning case. Leather stayed above target (mostly 21.2–21.6°C), the compressor spent much of the afternoon in standby or DHW, and there was no meaningful space-heating demand to separate wind/PV effects from simple no-load conditions. Item remains open.

**9 Apr data review (08:55 BST)**: The warm weather continues (overnight outside temp ~15-18°C), providing no opportunity to tune cold-weather/windy curve dynamics. Item remains open.

### Progressing: Active-Heating Outer/Inner Loop Conflict

Morning active-heating traces showed a separate issue from the warm-end saturation bug: the outer loop kept reapplying a lower model seed even while the inner loop was still correcting a real positive flow error.

Root cause: every 15 minutes the outer loop wrote the model seed (~1.1–1.3) back to `Hc1HeatCurve` even when `Hc1ActualFlowTempDesired` remained >0.5°C below `target_flow_c`. The 60-second inner loop then had to climb back through the same range, generating repetitive warnings and stretching convergence. Repo fix (9 Apr, not yet validated live): defer downward outer-loop resets while the VRC still wants materially less flow than target. A regression test now replays the captured 9 Apr relearn-cycle samples to lock this behaviour in.

**9 Apr data review (08:55 BST)**: this was visible repeatedly during the morning heating phase. Example: at 06:36 BST the outer loop wrote `Hc1HeatCurve=1.09` for `target_flow_c=28.3°C`, but `flow_desired_c` was only 25.9°C; the inner loop then climbed 1.21→1.62 before the next outer tick reset it again. Similar relearn cycles occurred at 06:51, 07:06, 07:22, and 07:37 BST, eventually reaching 2.04. Status stays progressing until the repo fix is deployed and a morning heating window shows the curve being held instead of repeatedly reset downward.

### Progressing: Warm-End Outer-Loop Curve Saturation

Very warm low-load daytime conditions exposed a bad outer-loop seed, not a general proof that the controller needs extreme curves in mild weather.

Root cause: the inverse curve formula divided by `(setpoint - outside)^1.25` with only a tiny lower bound. When forecast outside approached or exceeded the 19°C VRC setpoint, the denominator collapsed toward zero, so modest target-flow requests (~24–26°C) produced absurd raw curve values that were then clamped to the 4.00 ceiling. This is a warm-end inversion artefact, not evidence that mild-weather heating genuinely needs curve 4.00.

Repo fix (9 Apr, not yet validated live): when forecast outside is at or above setpoint, the outer loop now bypasses the inverse formula and seeds the known-safe baseline curve (0.55). The inner loop remains unchanged and still closes on `Hc1ActualFlowTempDesired` during real heating demand.

Observed evidence before the fix: on 8 Apr after the late-morning DHW cycle, outer ticks wrote `Hc1HeatCurve=3.47` at 12:37 BST and then `Hc1HeatCurve=4.00` at 14:12 BST while `target_flow_c` had fallen to only ~24.8–26.7°C and the compressor was mostly in standby. A service restart at 14:45 briefly reset the baseline curve to 0.55, then the next outer tick immediately wrote 4.00 again.

**9 Apr data review (08:55 BST)**: The issue repeated during the 8 Apr evening. Between 17:00 and 20:00 BST, the controller continued to log `model_required_curve: 4.0` while `target_flow_c` was only 23.7–25.2°C. Separately, the 9 Apr morning heating phase produced inner-loop warnings as the curve climbed to 2.04 while chasing a real ~28°C target flow. Treat those active-heating warnings as a distinct calibration/black-box question, not the same warm-end saturation bug. Status stays progressing until the repo fix is deployed and the next warm spell confirms sane outer-loop seeds.

## DHW Scheduling

DHW scheduling operational within the adaptive controller. See [[heating-control#Overnight Strategy#Active DHW Scheduling]] for current logic and [[domain#DHW Cylinder]] for cylinder facts.

Detailed plan: [`docs/dhw-plan.md`](../docs/dhw-plan.md)

### Open: Volume-Aware DHW Demand Prediction

This remains the main actionable DHW software item. T1 standby decay is calibrated but the model still assumes no draws occur.

On 47% of nights there's an overnight shower (avg 62L, max 120L). The 27 Mar night showed the risk: a 120L shower at 23:23 dropped T1 from 43.5→~37°C, below the 40°C comfort floor, and the model would have predicted 41.8°C. Demand slots aligned to Cosy charge windows: morning 07:00-13:00 (71% of days, avg 89L), afternoon 16:00-22:00 (24%, avg 72L), overnight 22:00-04:00 (47%, avg 62L). Next step: budget expected demand per slot using `dhw_capacity` from InfluxDB alongside T1.

**8 Apr data review (08:33 BST)**: one evening charge ran 22:23–00:02 BST without T1/Hwc crossover; T1 bottomed at ~40.5°C and remaining litres recovered to 177L by 07:30 BST. No overnight draw event appeared in this window, so it did not test the volume-budgeting gap.

**8 Apr data review (16:50 BST)**: the daytime window showed two more charges in the joined history review, with one full and one partial outcome, and a large T1/HwcStorageTemp divergence peak of 15.9°C. Hot water still ended the window practical (`T1` ~45.1°C), but there was still no overnight draw event to test whether slot budgeting against expected demand would have changed the plan. Status remains open.

**9 Apr data review (08:55 BST)**: A DHW cycle triggered at 22:00 BST (HwcStorageTemp 29.5°C), completing by 00:04 BST (43.0°C). No overnight draws occurred — `dhw_t1_c` fell only from 43.9°C to 43.5°C via expected standby decay. Status remains open as there's still no overnight shower event to test the volume gap.

### Open: Seasonal Eco→Normal Switch

Still manual / calendar-driven. `hmu HwcMode` is read-only from eBUS - must be changed physically on the aroTHERM controller. Switch to normal (2.4 kWh charges) around November. No software fix possible.

**8 Apr data review (16:50 BST)**: no change. The system remained in `hwc_mode:"eco"` throughout this review window, which is still the correct seasonal setting.

**9 Apr data review (08:55 BST)**: No change.

## Pico eBUS Adapter

Replacing the closed-source ESP32 firmware with Rust/Embassy on a Pi Pico W. Phase 1 (`ebus-core/` crate, 22 tests) complete. See [[infrastructure#eBUS Stack]] for the current live stack.

Detailed plan: [`docs/pico-ebus-plan.md`](../docs/pico-ebus-plan.md)

### Next: Phase 2 - PIO UART

Still waiting on hardware/test-bench time. Next step is PIO RX + TX at 2400/8N1 on the Pico W, validated by loopback and Saleae timing checks. Prerequisites: Pico W board, xyzroe eBus-TTL adapter, and Embassy + PIO setup.

**8 Apr data review (16:50 BST)**: no change in this software/data review window. The controller/log evidence does not affect Pico adapter status.

**9 Apr data review (08:55 BST)**: No change.
