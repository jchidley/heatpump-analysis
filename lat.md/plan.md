# Plan

Open items, next steps, and links to the detailed human-readable plan documents in `docs/`. Last status refresh: **2026-04-23 17:30 BST**.

The repo-local PostgreSQL cutover plan now lives in [[tsdb-migration]] and should stay aligned with the shared platform plan in `~/github/energy-hub/lat.md/tsdb-migration.md`.

## TSDB Migration

This plan section now mirrors the same three-bucket rule as [[tsdb-migration]]: current state, actions required to complete migration, and backlog after migration.

### Current state

The repo-local PostgreSQL cutover is complete. `heatpump-analysis` no longer blocks the shared cutover.

### Actions required to complete migration

Only the shared Phase 5 shutdown work remains: retire Telegraf's v2 output, remove the Grafana v2 datasource, stop/remove InfluxDB v2, and archive the v2 volume.

### Backlog after migration

Use [[tsdb-migration#New work backlog once migration is done]] for the repo-local post-migration queue.

## Heating Controller

V2 model-predictive controller is live, and this section tracks only the remaining controller questions plus their fixed baseline context.

Status taxonomy here is: **Open** (known gap), **Progressing** (change in flight / awaiting validation), **Actionable** (ready for a real-world intervention), and **Fixed** (done, but still important context). See [[heating-control]] for current behaviour and [[heating-control#Pilot History]] for operational findings.

Detailed plan: [`docs/heating-plan.md`](../docs/heating-plan.md)

### Active Work

These controller items still need evidence, tuning, or a real-world intervention.

#### Open: Headroom Unreliable During Cosy

Energy-hub headroom does not account for active grid charging during Cosy windows.

Observed behaviour now spans both morning and afternoon cases: when the battery starts Cosy already very full the signal can stay misleadingly positive, while active charging from lower effective headroom produces impossible negative swings even around ~92% SoC. There is no control impact because the controller ignores headroom during Cosy, but observability is misleading. Fix: return null or project through remaining Cosy charging.

**11 Apr data review (08:39 BST)**: The impossible negative did not recur, but the signal is still not trustworthy during Cosy. In `cosy_morning` the controller logged positive headroom while the battery was already effectively full — e.g. **05:13 BST** showed **100% SoC** with `battery_headroom_to_next_cosy_kwh=9.1`, and **05:29 BST** repeated **100% SoC** with **9.1 kWh**. Because Cosy charging was still active, the metric remains observability-only and still needs a proper null/projection fix. Item stays open.

#### Progressing: Overnight Data Growth

Evidence collection is no longer blocked, but we still need both a clean cold overnight window and a warmer daytime/heating window after the recent controller changes.

Current evidence ladder: 4 Apr was confounded by MinFlow=20, 5 Apr gave the first successful trajectory night at 9–12°C, 6–7 Apr was the strongest final ramp night and proved the ramp was back-loading the hard work, 7–8 Apr was lost to the Influx credential outage, 8–9 Apr was the first clean post-recovery coast-then-hold night, 9–10 Apr added a cooler-but-still-mild overnight with the new outer/inner fix validated, and 10–11 Apr added another mild overnight but exposed a new restart/hang confounder.

**11 Apr data review (08:39 BST)**: The **10–11 Apr overnight** window still supports coast-then-hold on a mild night: after the late-evening restart, Leather sat around **21.0→20.8°C** while outside stayed roughly **10.4–11.2°C**, and the morning heating recovery woke at **20.8°C** with no new waking-hours comfort miss. However, this is not a clean regression anchor because `history-review` for the exact review window logged **79 controller events**, **3 DHW overlaps**, and a new **22:45–02:31 BST** blind period with repeated null rows / long outer-cycle hangs. Treat that blind spell as a likely **pi5data migration / host-performance confounder** until proven otherwise, not a confirmed controller-logic regression. Status stays progressing.

#### Open: Host-Side I/O Hang During pi5data Migration

A late-evening service restart was followed by multi-minute outer-cycle hangs and null telemetry/model rows for hours, so controller evidence went partially blind again even though the service stayed up.

The exact review window shows the restart at **22:52 BST**, `startup: reinitialize_ebus failed: Connection refused` at **22:52:48 BST**, then repeated `outer cycle took ... possible I/O hang` warnings from **23:45 BST** through **02:31 BST**. During that span the controller kept emitting `action:"hold"` rows with `leather_temp_c`, `forecast_outside_c`, `model_required_*`, and other fields all null. Normal telemetry only resumed at **02:36 BST** when forecast refreshes and populated overnight-coast rows returned. Given the known pi5data migration effort since yesterday afternoon, treat this first as a **host / migration-performance confounder** affecting the controller process, eBUS access, or local I/O path — not yet as evidence of a controller algorithm bug.

**11 Apr data review (08:39 BST)**: New item opened from the review window. Immediate next step: correlate the migration work with ebus/influx/socket state, system load, and service restarts on pi5data to see whether infrastructure churn rather than controller logic stretched outer-loop work into **148–962s** hangs.

#### Open: Elvina Overnight Comfort (Accepted Occupant Preference)

Elvina still runs too cool overnight on mild nights, but the current occupant preference is to keep the vents open and the internal door closed even if that means the room stays cold in winter.

Child's bedroom hits 16.4–17.5°C at 07:00 on mild nights. Full proxy-network moisture analysis (13 sensors, 6 nights) says Elvina ventilates **6.8× faster** than Aldora (ACH ≈ 1.0 vs model 0.51), while the fabric residual is only 11 W/K versus model 14.5. The evidence still points to excessive ventilation rather than insufficient heat input, but this is now an accepted comfort trade-off rather than a pending intervention.

The previously proposed vent-closure intervention is no longer the working plan: Elvina has confirmed that she wants the trickle vents left open, wants the bedroom door kept closed, and accepts that the room will therefore be cold in winter. No controller change is planned for this room on that basis.

**11 Apr status update**: Occupant preference supersedes the earlier vent-closure proposal. Keep the evidence because it explains why the room is cold, but treat this item as a documented accepted condition unless preferences change later.

#### Open: Forecast API Reliability

Forecast data still has an unresolved reliability question even though the 7 Apr upstream outage itself has not obviously repeated.

Controller behaviour degrades gracefully: it uses cached forecast data and can fall back toward live outside conditions, but prolonged outages leave `forecast_outside_c` stale. The original 7 Apr issue looked like an upstream Open-Meteo reliability problem; the latest review window suggests there is also a host-side confounder because null forecast/model fields can still appear even when refreshes are succeeding. Consider: local caching with longer TTL, a second weather API, or alerting when forecast age exceeds threshold.

**11 Apr data review (08:39 BST)**: The upstream API still refreshed successfully, but null `forecast_outside_c` / `model_required_*` rows reappeared. The journal showed forecast refreshes at **22:05, 22:55, 02:36, 03:39, 04:41, 05:47, and 06:51 BST**, yet the controller still emitted null forecast/model fields during the **22:45–02:31 BST** hang period and again at **06:00 BST** (`action:"hold"`, `reason:"no rule fired"`). Given the known pi5data migration work, treat this first as part of the broader **host-side migration / I/O hang confounder**, not as fresh evidence of another upstream forecast outage.

#### Open: Wind and PV Tuning

Wind compensation and PV-aware curve adjustment exist in the model but still lack real-world tuning cases.

This is low urgency until weather provides a useful test day such as a windy cold spell or a sustained high-PV day with real space-heating demand.

**11 Apr data review (08:39 BST)**: Still no useful tuning case. The review window topped out at only **16.0°C** outside, overnight controller rows carried **0 W/m²** forecast solar, and the morning recovery was a standard mild-heating case rather than a windy-cold or high-PV test day. Item remains open.

#### Progressing: Warm-End Outer-Loop Curve Saturation

Very warm low-load daytime conditions exposed a bad outer-loop seed, not a general need for extreme curves in mild weather.

The inverse curve formula became unstable near the warm end. When forecast outside approached the 19°C VRC setpoint, modest target-flow requests (~24–27°C) could still explode into clamped curve requests near **4.00**. Before the fix, this was already reproduced in warm daytime standby conditions with modest target flows producing absurd outer-loop seeds of 3.5–4.0. A warm-end fallback was deployed during the **10:51 BST** restart on 9 Apr: when forecast outside is at or above setpoint, the outer loop now seeds the known-safe baseline curve **0.55** instead of using the unstable inversion.

**11 Apr data review (08:39 BST)**: No new validation case arrived in this window. Once populated, `forecast_outside_c` stayed around **9.3–11.0°C** in the overnight/morning heating rows, so the controller never re-entered the near-setpoint warm-end regime that triggered the bug. Status stays progressing pending the next genuinely warm heating day.

### Recent Fixed Baseline

These fixes are complete but still belong in the plan because active items depend on the history and constraints they introduced.

#### Fixed: Active-Heating Outer/Inner Loop Conflict

The outer loop used to reset the curve seed while the inner loop was still correcting a real flow deficit during morning active heating. This is now fixed. **Deployed 9 Apr 10:51 BST.**

The fix defers downward outer-loop resets while the VRC still wants materially less flow than `target_flow_c`. The key validation finally arrived on **10 Apr**: at **05:56 BST** and **06:12 BST** the outer loop explicitly logged deferred resets (`0.60→0.57` and `0.78→0.59`) while `flow_desired_c` still sat below target (**29.6<32.5** and **32.3<32.9**). Later downward writes at **06:28** and **06:44 BST** happened only once `flow_desired_c` was above target again, which is the intended over-target correction path.

#### Fixed: Forecast Nulls During DHW

The controller used to go blind during DHW charges, leaving forecast and model fields null. This was fixed by separating model calculation from actuation. **Deployed 7 Apr 10:34.**

The `!is_dhw` guard on the heating control block had skipped the entire model calculation, not just the eBUS writes. Removing that guard means the model (forecast + thermal solver) now runs every tick regardless of HP mode. During DHW, writes are suppressed but `target_flow_c` stays populated and action is logged as `dhw_active` with full model fields. Confirmed on both 5 and 6 Apr nights (up to 12 blind ticks per night). The 6 Apr overnight occurrence contributed to a 0.4°C comfort miss.

#### Fixed: Overnight Ramp Replaced with Coast-Then-Hold

The old linear overnight ramp back-loaded the hardest temperature rise into the final hours, so the controller could never catch up. It was replaced with a flat comfort-floor target (20.0°C). **Deployed 7 Apr 10:34.**

At 07:00 on 7 Apr the old strategy still missed by 0.3°C. Physics argument: total electrical cost = ∫ Q_hp/COP(T_flow) dt, and COP degrades with flow temp, so the minimum-electrical strategy is to coast for free then hold the comfort-band floor at the lowest possible flow. Simulation at outside 9.5°C: ramp used 2.82 kWh; coast-then-hold uses 1.86 kWh (-34%). See [[heating-control#Overnight Strategy#Trajectory Logic]] for the full rationale.

#### Fixed: DHW Timer Dedup Bug

A VRC 700 morning DHW timer could survive a failed disable attempt and then fire during preheat. This is fixed. **Deployed 7 Apr 10:34.**

On 6 Apr `sync_morning_dhw_timer` correctly decided to skip the morning window (T1 41.5°C predicted, above the 40°C trigger) but the eBUS write failed with `ERR: element not found`. Dedup state was updated anyway, suppressing retries. Then `restore_baseline` on restart re-enabled all timer windows without clearing the dedup state, so the skip was never retried. VRC 700 saw `HwcStorageTemp=37.5°C < 45°C` and fired its own charge at 04:00. Fixes: (1) `sync_morning_dhw_timer` now checks for `ERR:` in the response and clears dedup state on failure, (2) `control_loop` startup clears dedup state so the first tick always re-evaluates.

#### Fixed: Production Influx Secret Migration

The blind-controller outage was first fixed by restoring `INFLUX_TOKEN` in `/etc/adaptive-heating-mvp.env`, then hardened by moving the controller to a dedicated systemd credential. **Restored 8 Apr 08:39 BST; hardened 8 Apr 08:57 BST.**

Root cause: systemd had been depending on `/etc/adaptive-heating-mvp.env`, and that file was overwritten without `INFLUX_TOKEN`. The controller's dev fallback (`ak get influxdb`) cannot work under systemd because there is no gpg-agent session. Current deployment stores a dedicated InfluxDB auth for `adaptive-heating-mvp` in `/etc/adaptive-heating-mvp/influx.token` (root-only) and loads it via `LoadCredential=influx_token:/etc/adaptive-heating-mvp/influx.token`. Repo-wide secret policy now lives in [[infrastructure#Secrets]]: prefer encrypted systemd credentials (`systemd-creds encrypt` + `SetCredentialEncrypted=`) where supported, while this service remains on the root-only `LoadCredential=` path until migrated. `/etc/adaptive-heating-mvp.env` now remains only for non-Influx env vars such as Octopus credentials.

Verification: the running service restarts with the credential present, without relying on `INFLUX_TOKEN` in its environment. **7–8 Apr controller evidence is still lost**, but future outer ticks and decision-log writes now depend on the dedicated local credential rather than a copied env var.

## DHW Scheduling

DHW scheduling is operational within the adaptive controller. This section uses the same status taxonomy as the controller section, plus **Manual** for items that cannot be automated in software. See [[heating-control#Overnight Strategy#Active DHW Scheduling]] for current logic and [[domain#DHW Cylinder]] for cylinder facts.

Detailed plan: [`docs/dhw-plan.md`](../docs/dhw-plan.md)

### Progressing: Volume-Aware DHW Demand Prediction

This remains the main actionable DHW software item, but the controller no longer relies on T1 alone.

### Open: Multical stale-data alerting

The `emondhw` source outage showed that DHW history can go blind for days without any local TSDB replay path to repair it.

The 2026-04-16 → 2026-04-23 gap was caused by the Multical USB/Modbus device disappearing on `emondhw`, so both PostgreSQL and legacy Influx stopped advancing together. The immediate recovery was a reboot, but the durable gap is irrecoverable from local migration sources. Follow-up work: add an operational stale-data alert for `multical` freshness and define whether the first response should be notification-only or an automated `emondhw` restart/reboot path.

On 47% of nights there's an overnight shower (avg 62L, max 120L). The 27 Mar night showed the risk: a 120L shower at 23:23 dropped T1 from 43.5→~37°C, below the 40°C comfort floor, and the old model would have predicted 41.8°C. The controller now adds a first practical draw-aware budget: it reads `dhw.remaining_litres` plus the latest `dhw_capacity.recommended_full_litres`, caps optimistic remaining-volume estimates by that practical full-capacity value, and compares the resulting remaining litres with slot demand budgets aligned to Cosy charge windows (morning 89L, afternoon 72L, overnight 62L). That means a warm-looking T1 no longer suppresses a recharge when practical hot-water volume is already too low.

This is still not the final DHW demand model. It is a volume-budget guardrail, not a full probabilistic draw predictor, and it still depends on the quality/timeliness of the upstream `dhw` + `dhw_capacity` feeds. Remaining work: tune slot budgets against recent evidence, verify no over-trigger regressions, and decide whether later slots should become more explicitly demand-ranked rather than fixed-budget thresholds.

Immediate regression-test follow-up from the 12 Apr evening incident:
- keep the new unit coverage for imported `22:00–00:00` tariff windows (`00:00` must normalize to `23:59` for runtime matching and to `-:-` for VRC 700 writes)
- add at least one higher-level controller-path test that starts from raw imported tariff windows and proves the evening slot is classified as active at 22:xx while the emitted `HwcTimer_*` payload still uses `-:-`
- add a deployment/ops smoke check that rejects any observed `HwcTimer_*` write containing raw `00:00`

**Deployed 12 Apr 2026 22:47 BST on `pi5data`**: the live `adaptive-heating-mvp` service was rebuilt and restarted with the remaining-litres / recommended-capacity guardrail active. Immediate post-restart checks showed the service listening on port 3031, startup eBUS writes succeeding, and fresh controller logs resuming without the old dependency-sync build failure.

**Follow-up fix deployed from the same evening**: Octopus-derived tariff windows could still arrive with an evening end of `00:00`, which is invalid for VRC 700 end-of-day timer encoding and also breaks same-day controller slot matching. The live controller now treats write-time normalization as the hard safety boundary (`00:00` → `-:-` for eBUS writes) and also normalizes imported `00:00` ends to `23:59` for same-day runtime matching.

### Manual: Seasonal Eco→Normal Switch

The seasonal Eco→Normal mode change remains manual and calendar-driven.

`hmu HwcMode` is read-only from eBUS, so the switch must still be done physically on the aroTHERM controller. The normal mode threshold remains around November because it changes charges from ~0.8–1.2 kWh eco top-ups to ~2.4 kWh normal charges. No software fix is possible.

**11 Apr data review (08:39 BST)**: No seasonal change. The usable controller rows still showed `hwc_mode:"eco"`; the only deviations were transient read failures (`ERR: no signal`, `ERR: read timeout`) during the broader telemetry-hang issue, not a real mode switch.

## Pico eBUS Adapter

This workstream replaces the closed-source ESP32 firmware with Rust/Embassy on a Pi Pico W. Phase 1 (`ebus-core/` crate, 22 tests) is complete; Phase 2 is still waiting on hardware/test-bench time. See [[infrastructure#eBUS Stack]] for the live stack.

Detailed plan: [`docs/pico-ebus-plan.md`](../docs/pico-ebus-plan.md)

### Next: Phase 2 - PIO UART

The next implementation step is still PIO RX + TX at 2400/8N1 on the Pico W, validated by loopback and Saleae timing checks.

Prerequisites remain: Pico W board, xyzroe eBus-TTL adapter, and Embassy + PIO setup.

**11 Apr data review (08:39 BST)**: No change in this controller/data review window.

## Open Questions

Empirical or hardware unknowns that still need real-world evidence before they can inform control decisions.

These were moved out of the former code-truth decisions notes, now preserved under `docs/implementation-maps/`, because they are live unknowns rather than static architecture.

### OQ1: Aldora Proxy Comfort Band

Need to query historical data for Aldora temperature when Leather is in the 20–21°C band. Until derived, Aldora must not drive control.

### OQ2: Minimum Acceptable T1 for Morning Showers

45°C is definitely fine. 43°C might be. Needs household experiment. Determines whether a 22:00 charge to 45°C (→ ~42.9°C by morning after 0.23°C/h standby decay) is acceptable, or whether to charge to 47–48°C.

### OQ3: Overnight Coast Empirical K

Code uses K=7500, empirical K≈20,600 from 27 segments. Code is conservative (overpredicts reheat time → preheats too early). Each genuine coast night validates.

### OQ4: HwcMode (eco/normal) Writable via eBUS?

Currently read-only via `hmu HwcMode`. VWZ AI (0x76) has undecoded B512/B513 register traffic.

A grab session while toggling eco↔normal on the aroTHERM would reveal which bytes change. There may be a writable register on the VWZ AI control panel.

### OQ5: Eco/Normal Crossover Temperature

At what outside temp does total system cost (DHW COP saving from eco vs heating recovery cost from longer steal) favour normal mode? Below ~8°C the 22:00 window avoids the trade-off. More academic than practical.

### OQ6: CurrentCompressorUtil Meaning

Signed encoding wraps negative (`-57`). Not meaningful as utilisation %. For compressor state, `RunDataStatuscode` transitions are more reliable.
