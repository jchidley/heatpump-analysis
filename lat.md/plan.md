# Plan

Open items, next steps, and links to the detailed human-readable plan documents in `docs/`. Last status refresh: **2026-05-23 13:14 BST**.

The repo-local PostgreSQL cutover plan now lives in [[tsdb-migration]] and should stay aligned with the shared platform plan in `~/github/energy-hub/lat.md/tsdb-migration.md`.

## Latest Data Review

Most recent controller-log and PostgreSQL review for the open plan items.

Review window: **2026-05-02 14:52 BST** to **2026-05-23 13:14 BST**.

Evidence:
- `adaptive-heating-mvp` is still the same long-running service process, PID **2575348**, started **2026-04-30 09:53 BST** with `NRestarts=0`; PostgreSQL decisions continue through **2026-05-23 13:04 BST**.
- PostgreSQL recorded **1,910** controller decisions: `hold=786`, `overnight_coast=451`, `daytime_model=438`, `dhw_active=183`, `overnight_model=43`, and `dhw_schedule_launch=9`. Leather ranged **20.1-23.0°C**, Aldora **18.5-27.4°C**, and outside ranged up to **33.4°C** with **4** impossible near-zero outside readings.
- Forecast fetching was mostly healthy but not perfect: logs show **437** refreshes and **1** failed fetch (`error decoding response body` on **2026-05-21 23:30 BST**). PostgreSQL still had **245/1,910** rows with null forecast fields, mainly where local telemetry/model fields were also null.
- Cosy headroom is still misleading during active charging: **179** Cosy rows had `battery_power_w < -500W`, with charging down to about **-5.0kW** and headroom still reported between about **-5.6 and +11.3 kWh**.
- Overnight evidence grew substantially: **451** coast rows and **43** overnight model rows now include outside readings down to about **6.0-6.1°C** as well as many warm-night cases; this improves confidence but still does not provide a clean cold winter sign-off window.
- Host-side/eBUS I/O remains noisy despite no service restart: logs contain **7** EAGAIN/resource-unavailable hits, **235** eBUS-style `ERR:`/`SYN received`/timeout/no-signal/arbitration lines, and PostgreSQL shows **4** decision gaps over **20 minutes** with a maximum gap of **47m09s**.
- Warm/sunny curve saturation now looks validated in real warm data: **0** rows met the previous warm high-curve threshold (`outside_temp_c >= 18` and `curve_after > 1.5`), while outside reached **33.4°C** and model curves stayed at baseline in low-load conditions.
- DHW scheduling produced **9** `dhw_schedule_launch` and **183** `dhw_active` rows. `multical` was fresh at **2026-05-23 13:14 BST** and there were **0** zero storage-temperature rows, but **184** rows had null `hwc_storage_temp_c`; logs show **7** DHW timer write failures and **2** failed `HwcSFMode=load` writes.
- The deployed service is **not demonstrably the most current software**: `/home/jack/adaptive-heating-mvp` on `pi5data` is not a git checkout, the binary is from **2026-04-30 09:52 BST**, and this working tree has later uncommitted source/doc changes. The live controller is current only relative to the last explicit deployment, not relative to the current repository state.

Status changes:
- **Headroom unreliable during Cosy:** still **Open**; the larger sample again shows active grid charging with apparently usable or unstable headroom.
- **Overnight data growth:** still **Progressing**; there is now more mild/cool overnight evidence down to about 6°C, but not a clean cold winter validation window.
- **Host-side I/O hang:** still **Open**; no restart occurred, but EAGAINs, eBUS errors, failed writes, null telemetry rows, and 47-minute gaps show the runtime path is still not clean.
- **Forecast API reliability:** still **Open**; upstream failures were rare, so remaining null forecast/model rows are mostly a local telemetry/runtime confounder rather than proven Open-Meteo unreliability.
- **Wind and PV tuning:** still **Open**; warm/sunny validation improved, but wind-specific cold evidence is still missing.
- **Warm-end outer-loop curve saturation:** changed from **Progressing** to **Closed** for the deployed fix; the warm high-curve failure did not recur across a long warm/sunny window.
- **Volume-aware DHW demand prediction:** still **Progressing**; scheduling is active and volume-aware launches happen, but failed writes and null storage inputs still prevent sign-off.
- **Multical stale-data alerting:** still **Open**; `multical` is fresh now, but the review again found null storage/runtime blind spots that need alerting.
- **Transient HwcStorage zero/null during charge:** still **Open**; zero readings did not recur, but **184** null storage rows did.

New issues:
- **Open: Impossible near-zero outside-temperature readings** — four controller rows recorded physically impossible near-zero outside values while forecasts were absent or normal. Treat these like stale/invalid eBUS inputs rather than real weather before they influence control decisions.
- **Deployment provenance gap** — the production directory is not a git checkout, so software currency cannot be proven from a commit hash on the host. Add a build/version stamp or deploy from a checked-out release to make future reviews auditable.

## TSDB Migration

This plan section now mirrors the same three-bucket rule as [[tsdb-migration]]: current state, actions required to complete migration, and backlog after migration.

### Current state

The repo-local PostgreSQL cutover is complete, and the shared Phase 5 shutdown on `pi5data` was completed on 2026-04-23. `heatpump-analysis` no longer blocks any part of the migration.

### Actions required to complete migration

No migration-critical actions remain.

### Backlog after migration

Use [[tsdb-migration#Post-migration backlog]] for the repo-local post-migration queue.

## Heating Controller

V2 model-predictive controller is live, and this section tracks only the remaining controller questions.

Status taxonomy here is: **Open** (known gap), **Progressing** (change in flight / awaiting validation), **Actionable** (ready for a real-world intervention), and **Closed** (validated / no longer active). See [[heating-control]] for current behaviour and [[heating-control#Pilot History]] for durable design lessons.

Detailed plan: [`docs/heating-plan.md`](../docs/heating-plan.md)

### Active Work

These controller items are the live unresolved questions that still need intervention, validation, or further tuning.

#### Open: Headroom Unreliable During Cosy

Energy-hub headroom does not account for active grid charging during Cosy windows.

The signal can stay misleadingly positive when the battery is already effectively full, and can also swing impossibly negative while Cosy charging is active. The 2026-05-23 data review found **179** active Cosy charging rows down to roughly 5kW with headroom still ranging from negative to apparently ample. There is no control impact because the controller ignores headroom during Cosy, but observability is misleading. Because Tesla controls the actual charging behaviour, the safe projection during active Cosy charging is the current SoC, not an assumed continuation of observed charge power. The energy-hub fix now keeps the numeric headroom current-SoC-based and publishes projection metadata rather than returning null.

#### Progressing: Overnight Data Growth

Evidence collection is no longer blocked, but we still need both a clean cold overnight window and a warmer daytime/heating window after the recent controller changes.

The 2026-04-30 review added a colder 2026-04-24 overnight coast/reheat window, with outside temperatures down to about 4.25°C. The 2026-05-23 review added **451** coast rows and **43** overnight-model rows down to about 6°C, so confidence is improving, but sign-off still needs a clean cold run not confounded by controller I/O errors.

#### Open: Host-Side I/O Hang During pi5data Migration

A restart was followed by multi-minute outer-cycle hangs and null telemetry/model rows, so controller observability went partially blind even though the service stayed up.

This currently looks more like a host/runtime path issue affecting controller process I/O, eBUS access, or local scheduling than a confirmed controller algorithm bug. The 2026-05-23 review saw no restart, but did find **7** EAGAIN/resource-unavailable hits, **235** eBUS-style errors, failed DHW writes, null telemetry/model rows, and **4** decision gaps over 20 minutes. Keep correlating host load, ebusd/socket state, adapter health, and controller timing rather than treating those rows as clean controller evidence.

#### Open: Elvina Overnight Comfort (Accepted Occupant Preference)

Elvina still runs too cool overnight on mild nights, but the current occupant preference is to keep the vents open and the internal door closed even if that means the room stays cold in winter.

The room still looks ventilation-dominated rather than emitter-limited, but no controller change is planned because the occupant explicitly accepts that trade-off.

#### Open: Forecast API Reliability

Forecast data still has an unresolved reliability question even though the original upstream outage has not obviously repeated.

The controller can use cached forecast data and partially degrade toward live outside conditions. The 2026-05-23 review found **437** successful refreshes and **1** fetch failure, but **245/1,910** rows still had null forecast fields during local telemetry/model loss. Prolonged null forecast/model rows still need separating into upstream failures versus host/runtime confounders. Consider longer-lived local caching, a second weather API, or alerting on stale forecast age.

#### Open: Wind and PV Tuning

Wind compensation and PV-aware curve adjustment exist in the model but still lack useful real-world tuning cases.

Solar evidence is no longer scarce: late-April warm/sunny days exposed bad high-curve behaviour, and the 2026-05-23 window validates the warm-end fix. Wind tuning still needs a windy cold spell, while PV/solar compensation can now be tuned without conflating it with the closed curve-saturation regression.

#### Closed: Warm-End Outer-Loop Curve Saturation

The warm-end outer-loop seed fix has now survived a long warm/sunny validation window.

The 2026-04-30 review found repeated warm/sunny high-curve rows, including model-required and written curves up to 4.0 while target flows were only about 26-27°C. The deployed fix uses the baseline curve and suppresses inner-loop flow chasing when forecast outside is within 5°C of the VRC setpoint and required flow is ≤28.5°C. The 2026-05-23 review found **0** warm high-curve rows by the prior threshold across a warm window with outside readings up to **33.4°C**, so this specific regression is closed.

#### Open: Impossible Near-Zero Outside-Temperature Readings

A few controller rows contain physically impossible near-zero outside temperatures and should be treated as invalid input.

The 2026-05-23 review found four rows with `outside_temp_c` near zero (`~1e-41` to `~5e-39`) while forecasts were absent or normal. This looks like a bad telemetry decode/stale-input case, not real weather. Controller input validation should reject these values before they influence modelling or review summaries.

#### Open: Deployment Provenance Gap

The live controller host cannot currently prove the exact source commit behind the running binary.

The 2026-05-23 review found `/home/jack/adaptive-heating-mvp` on `pi5data` is not a git checkout and the running binary dates from **2026-04-30 09:52 BST**. Future deploys should include a commit/build stamp or run from a checked-out release so reviews can answer software-currency questions from the host itself.

## DHW Scheduling

DHW scheduling is operational within the adaptive controller. This section uses the same status taxonomy as the controller section, plus **Manual** for items that cannot be automated in software. See [[heating-control#Overnight Strategy#Active DHW Scheduling]] for current logic and [[domain#DHW Cylinder]] for cylinder facts.

Detailed plan: [`docs/dhw-plan.md`](../docs/dhw-plan.md)

### Progressing: Volume-Aware DHW Demand Prediction

This remains the main actionable DHW software item, but the controller no longer relies on T1 alone.

The current guardrail uses `dhw.remaining_litres` and `dhw_capacity.recommended_full_litres` so a warm-looking T1 can no longer hide a practically depleted cylinder. The 2026-05-23 review saw **183** `dhw_active` rows, **9** `dhw_schedule_launch` rows, fresh `multical`, and no storage-temperature zero rows, but failed writes and null storage inputs remain. Remaining work: tune slot budgets, verify no over-trigger regressions, reject impossible or missing storage temperatures as stale input, and decide whether later slots should be demand-ranked more explicitly.

Immediate follow-up still needed:
- keep controller-path coverage for imported `22:00–00:00` tariff windows
- keep the runtime rule that raw `00:00` must normalize to `23:59` for same-day matching and to `-:-` for VRC 700 writes
- keep a deployment/ops smoke check that rejects any observed `HwcTimer_*` write containing raw `00:00`

### Open: Multical stale-data alerting

The `emondhw` source outage showed that DHW history can go blind for days without any local TSDB replay path to repair it.

Follow-up work: add an operational stale-data alert for `multical` freshness and decide whether the first response should be notification-only or an automated restart / reboot path on `emondhw`. The 2026-05-23 review found `multical` fresh at review time, but null eBUS storage inputs and impossible outside readings show this also needs invalid-input alerting. Cross-repo automatic recovery / gap accounting is tracked in `energy-hub`.

### Open: Transient HwcStorage zero/null during charge

Even with fresh Multical rows, the eBUS storage-temperature input can briefly become impossible during DHW charging.

The 2026-04-24 review saw z2m-hub no-crossover decisions with `HwcStorageTemp=0.0` and an adaptive-heating decision with null `hwc_storage_temp_c`. The 2026-05-23 review found **0** zero storage-temperature rows but **184** null storage rows. The model recovers once valid readings return, but controller code should classify impossible or missing storage temperatures as stale input rather than real bottom-zone evidence.

### Manual: Seasonal Eco→Normal Switch

The seasonal Eco→Normal mode change remains manual and calendar-driven.

`hmu HwcMode` is read-only from eBUS, so the switch must still be done physically on the aroTHERM controller. The normal mode threshold remains around November because it changes charges from ~0.8–1.2 kWh eco top-ups to ~2.4 kWh normal charges. No software fix is possible.

## Pico eBUS Adapter

This workstream replaces the closed-source ESP32 firmware with Rust/Embassy on a Pi Pico W. Phase 2 is still waiting on hardware/test-bench time. See [[infrastructure#eBUS Stack]] for the live stack.

Detailed plan: [`docs/pico-ebus-plan.md`](../docs/pico-ebus-plan.md)

### Next: Phase 2 - PIO UART

The next implementation step is still PIO RX + TX at 2400/8N1 on the Pico W, validated by loopback and Saleae timing checks.

Prerequisites remain: Pico W board, xyzroe eBus-TTL adapter, and Embassy + PIO setup.

## Open Questions

Empirical or hardware unknowns that still need real-world evidence before they can inform control decisions.

These were moved out of the former code-truth decisions notes because they are live unknowns rather than static architecture.

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
