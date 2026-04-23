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

V2 model-predictive controller is live, and this section tracks only the remaining controller questions.

Status taxonomy here is: **Open** (known gap), **Progressing** (change in flight / awaiting validation), and **Actionable** (ready for a real-world intervention). See [[heating-control]] for current behaviour and [[heating-control#Pilot History]] for durable design lessons.

Detailed plan: [`docs/heating-plan.md`](../docs/heating-plan.md)

### Active Work

These controller items are the live unresolved questions that still need intervention, validation, or further tuning.

#### Open: Headroom Unreliable During Cosy

Energy-hub headroom does not account for active grid charging during Cosy windows.

The signal can stay misleadingly positive when the battery is already effectively full, and can also swing impossibly negative while Cosy charging is active. There is no control impact because the controller ignores headroom during Cosy, but observability is misleading. Fix: return null or project through remaining Cosy charging.

#### Progressing: Overnight Data Growth

Evidence collection is no longer blocked, but we still need both a clean cold overnight window and a warmer daytime/heating window after the recent controller changes.

The overnight strategy and recent controller fixes need a genuinely clean regression window rather than another mild or infrastructure-confounded night.

#### Open: Host-Side I/O Hang During pi5data Migration

A restart was followed by multi-minute outer-cycle hangs and null telemetry/model rows, so controller observability went partially blind even though the service stayed up.

This currently looks more like a host / migration-performance confounder affecting controller process I/O, eBUS access, or the local runtime path than a confirmed controller algorithm bug. Next step: correlate host load, service restarts, ebus/socket state, and controller timing.

#### Open: Elvina Overnight Comfort (Accepted Occupant Preference)

Elvina still runs too cool overnight on mild nights, but the current occupant preference is to keep the vents open and the internal door closed even if that means the room stays cold in winter.

The room still looks ventilation-dominated rather than emitter-limited, but no controller change is planned because the occupant explicitly accepts that trade-off.

#### Open: Forecast API Reliability

Forecast data still has an unresolved reliability question even though the original upstream outage has not obviously repeated.

The controller can use cached forecast data and partially degrade toward live outside conditions, but prolonged null forecast/model rows still need separating into upstream failures versus host/runtime confounders. Consider longer-lived local caching, a second weather API, or alerting on stale forecast age.

#### Open: Wind and PV Tuning

Wind compensation and PV-aware curve adjustment exist in the model but still lack useful real-world tuning cases.

This remains low urgency until weather provides a genuinely windy cold spell or a sustained high-PV heating day.

#### Progressing: Warm-End Outer-Loop Curve Saturation

Very warm low-load daytime conditions exposed a bad outer-loop seed, not a general need for extreme curves in mild weather.

The current warm-end fallback avoids the worst inversion behaviour, but it still needs validation on a genuinely warm heating day.

## DHW Scheduling

DHW scheduling is operational within the adaptive controller. This section uses the same status taxonomy as the controller section, plus **Manual** for items that cannot be automated in software. See [[heating-control#Overnight Strategy#Active DHW Scheduling]] for current logic and [[domain#DHW Cylinder]] for cylinder facts.

Detailed plan: [`docs/dhw-plan.md`](../docs/dhw-plan.md)

### Progressing: Volume-Aware DHW Demand Prediction

This remains the main actionable DHW software item, but the controller no longer relies on T1 alone.

The current guardrail uses `dhw.remaining_litres` and `dhw_capacity.recommended_full_litres` so a warm-looking T1 can no longer hide a practically depleted cylinder. Remaining work: tune slot budgets, verify no over-trigger regressions, and decide whether later slots should be demand-ranked more explicitly.

Immediate follow-up still needed:
- keep controller-path coverage for imported `22:00–00:00` tariff windows
- keep the runtime rule that raw `00:00` must normalize to `23:59` for same-day matching and to `-:-` for VRC 700 writes
- keep a deployment/ops smoke check that rejects any observed `HwcTimer_*` write containing raw `00:00`

### Open: Multical stale-data alerting

The `emondhw` source outage showed that DHW history can go blind for days without any local TSDB replay path to repair it.

Follow-up work: add an operational stale-data alert for `multical` freshness and decide whether the first response should be notification-only or an automated restart / reboot path on `emondhw`.

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
