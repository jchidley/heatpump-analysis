# Adaptive Heating MVP

Date: 31 March 2026

## Purpose

This MVP is not a lab harness or a simulation exercise. It is a real controller pilot for the actual house.

The goal is to **optimise the control strategy for the real house using the VRC 700 as the writable policy layer**, while running on `pi5data`, observing real telemetry, and refining the strategy over time from measured outcomes.

The VRC 700 is treated as a steerable state machine:

`700 writable inputs → VRC 700 internal policy → downstream eBUS commands → VWZ AI / HMU response → house response`

## MVP scope

V1 includes both **space heating** and **active DHW control** from day one.

This is a live pilot, not a dry-run-first release.

## Current implementation status

### Completed on 31 March 2026

- MVP spec written in this document
- Rust binary implemented: `src/bin/adaptive-heating-mvp.rs`
- Runtime config added: `model/adaptive-heating-mvp.toml`
- systemd unit added: `deploy/adaptive-heating-mvp.service`
- Service built and installed on `pi5data`
- HTTP control API implemented and verified:
  - `GET /status`
  - `POST /mode/occupied`
  - `POST /mode/short-absence`
  - `POST /mode/away`
  - `POST /mode/disabled`
  - `POST /mode/monitor-only`
  - `POST /kill`
- CLI commands implemented and verified:
  - `run`
  - `status`
  - `restore-baseline`
- Baseline restore verified against the live VRC 700 for:
  - `Hc1HeatCurve`
  - `Z1DayTemp`
  - `Z1NightTemp`
  - `HwcTempDesired`
  - `Z1OpMode`
  - `HwcOpMode`
- Logging implemented to:
  - local JSONL on `pi5data`
  - InfluxDB
- Existing `z2m-hub` identified as the intended mobile control surface to reuse

### Confirmed writable live control levers

These have been confirmed writable on the real VRC 700 by direct eBUS write + readback:

- `Hc1HeatCurve`
- `Z1DayTemp`
- `Z1NightTemp`
- `Hc1MaxFlowTempDesired`
- `Hc1MinFlowTempDesired`
- `Z1OpMode`
- `HwcOpMode`
- `HwcTempDesired`
- `HwcSFMode`
- `Hc1SummerTempLimit`
- `Hc1RoomTempSwitchOn`
- `AdaptHeatCurve`
- `ContinuosHeating`
- `HolidayTemp`
- `Z1HolidayTemp`
- `Z1QuickVetoTemp`
- `CylinderChargeHyst`
- `CylinderChargeOffset`
- `HwcLockTime`
- `MaxCylinderChargeTime`
- `HwcParallelLoading`
- `HwcMaxFlowTempDesired`
- `FrostOverRideTime`
- `Hc1AutoOffMode`
- timers such as `Z1Timer_*` and `HwcTimer_*` (TTM writes return `empty`, readback confirms persistence)

### Confirmed live effect so far

- `Hc1HeatCurve` is confirmed to be a **real dynamic heating lever**:
  - write accepted
  - readback changed
  - `Hc1ActualFlowTempDesired` changed on the live system

This is the first strong proof that a writable VRC 700 register changes actual controller demand rather than just stored configuration.

### Allowed control levers in V1

All confirmed writable levers are fair game for the MVP if they prove useful:

- `Hc1HeatCurve`
- `Z1DayTemp`
- `Z1NightTemp`
- `HwcSFMode`
- `HwcTempDesired`
- `Z1QuickVetoTemp`
- `HwcOpMode`
- `Z1OpMode`
- `Hc1MaxFlowTempDesired`
- `Hc1MinFlowTempDesired`
- timers (`Z1Timer_*`, `HwcTimer_*`)
- any other VRC 700 writable register that is accepted, read back, and shown to affect useful downstream behaviour

The intended Vaillant purpose of a register does not matter. What matters is whether it helps achieve better house-level outcomes.

## MVP operating modes

V1 should support all of these modes:

- `occupied`
- `short_absence`
- `away_until`
- `disabled`
- `monitor_only`

Initial real usage will mainly be:
- `occupied`
- `short_absence`

## Mode control surface

Use the existing Rust/Axum mobile control surface in:

- `~/github/z2m-hub/`

The MVP should expose mode changes through the same mobile-friendly surface already used for lights and DHW.

Expected mode control endpoints / actions:
- set `occupied`
- set `short_absence`
- set `away_until` with return time
- set `disabled`
- trigger kill switch / restore baseline

Static defaults live in repo config.
Live runtime state lives on `pi5data` and is controlled via the mobile/Axum service.

## Comfort targeting

### Primary room targets

The controller must not optimise for a whole-house average.

#### Primary reference room
- **Leather room** is the primary comfort target.
- Leather is the room where "good" is known most clearly.
- Leather comfort band:
  - **below 20°C feels cold**
  - **above 21°C feels hot**
- Therefore the Leather target band is:
  - **20.0–21.0°C**

#### Secondary / fallback reference
- **Aldora** is the fallback reference room.
- However, Aldora must **not** drive control yet until its proxy comfort band has been derived from historical data collected when Leather is in its good band.
- Other reasonable rooms may help infer that band offline, but should not be primary drivers in V1.

#### Excluded room
- **Conservatory** is excluded from direct optimisation.
- Treat it as a **boundary / heat sink** that mostly follows outdoor conditions, strongly modified by solar gain.

#### Other rooms
- All other rooms are constraints, modifiers, and evidence of distribution problems.
- They should not be treated as equal comfort targets in V1.

### Leather door rule

When Leather door sensors are installed:

- **Leather doors open => Leather is disqualified as primary target entirely**

Reason: opening the doors changes the room state immediately and couples it strongly to adjacent spaces, making it a poor lead signal for whole-house control.

## Timing and cadence

### Decision cadence
- read / aggregate state every **1 minute**
- make control decisions every **15 minutes**

This is the fixed cadence for the MVP.

### Step size
- minimum practical heat-curve step for MVP: **0.10**

Rationale: smaller steps risk disappearing into the noise, especially on mild days.

### Allowed value ranges
The MVP should trust the accepted range enforced by the VRC 700.

Rule:
- **if the VRC 700 accepts the value, treat it as safe enough to pilot**

Do not add tighter software bounds in V1 unless operational evidence later shows the need.

### Operational setpoint intent
- occupied operation should normally live around **20–21°C**
- away mode may go to **15°C**
- preheat above 21°C is allowed, but should be the **smallest increase that achieves the objective**, not an arbitrary fixed bump

## Action blocking / refusal conditions

The MVP should block or defer inappropriate actions when any of these are true:

- DHW active, if the proposed action is unrelated and would interfere
- defrost active
- key sensors missing
- startup / reboot grace period
- controller disabled

The controller should not try to be clever when core state is missing or transient.

## Stop / failure behaviour

If the MVP crashes, is stopped, or is manually killed:

- restore the **known-good baseline**

Do not simply stop writing and leave altered state behind.

### Known-good baseline
Current agreed baseline is:
- `Hc1HeatCurve = 0.55`
- `Z1DayTemp = 21`
- `Z1NightTemp = 19`
- current known-good timers and DHW settings as already validated on the VRC 700

## Kill switch

Kill switch semantics:
- **restore known-good baseline**

Trigger methods required in V1:
- mobile / HTTP action
- local CLI command on `pi5data`

Systemd stop may also restore baseline as part of shutdown handling.

## Logging

Use both:

### InfluxDB
For:
- timeseries
- dashboards
- operational analysis
- comparing actions against power, COP, temperatures, and timing

### Local JSONL on `pi5data`
For:
- structured decision logs
- audit trail
- agent / LLM inspection
- debugging and replay

### Minimum decision log fields
Each decision should log:
- timestamp
- mode
- occupancy state
- tariff period
- outside temp
- key room temps (Leather, Aldora, other important rooms)
- DHW state
- chosen control action(s)
- reason / rule that fired
- write success / failure
- readback values
- resulting VRC 700 demand values
- relevant HMU/VWZ response values

## Deployment

V1 is deployed as a **systemd service from day one** on `pi5data`.

This is a real pilot controller, not a manual CLI-only prototype.

### Implemented service

The current MVP implementation is:
- Rust binary: `src/bin/adaptive-heating-mvp.rs`
- Config: `model/adaptive-heating-mvp.toml`
- systemd unit: `deploy/adaptive-heating-mvp.service`

Installed runtime paths on `pi5data`:
- app dir: `/home/jack/adaptive-heating-mvp`
- HTTP API: `http://pi5data:3031`
- state file: `/home/jack/.local/state/adaptive-heating-mvp/state.toml`
- JSONL log: `/home/jack/.local/state/adaptive-heating-mvp/actions.jsonl`

### Implemented control API

- `GET /status`
- `POST /mode/occupied`
- `POST /mode/short-absence`
- `POST /mode/away` with JSON body `{ "return_at": "...UTC..." }`
- `POST /mode/disabled`
- `POST /mode/monitor-only`
- `POST /kill` — restore known-good baseline and disable controller

### Local CLI

- `adaptive-heating-mvp run`
- `adaptive-heating-mvp status`
- `adaptive-heating-mvp restore-baseline`

## Outstanding work

### Immediate next tasks

1. **Integrate the MVP controls into `z2m-hub` mobile UI**
   - Add occupied / short-absence / away / disabled controls
   - Add kill / restore-baseline control
   - Surface current mode and away-until state on the existing phone dashboard

2. **Derive the Aldora proxy band from historical data**
   - Query periods where Leather is in the good band (20–21°C)
   - Measure corresponding Aldora distribution
   - Record a first empirical fallback band for use when Leather is unavailable

3. **Verify InfluxDB logging on the live service**
   - Confirm the measurement is being written successfully
   - Build a first Grafana view for control actions vs room temps / COP / status

4. **Inspect the first JSONL and Influx logs after several control cycles**
   - Check that actions, reasons, and readbacks are sensible
   - Confirm the service is not thrashing or making duplicate pointless writes

5. **Observe actual live behaviour over several cycles**
   - Check whether heating actions are being taken in expected states
   - Verify DHW behaviour remains acceptable in real operation
   - Confirm baseline restore still works under normal service-stop conditions

### Deliberately deferred

These are intentionally not complete yet:

- Leather door-sensor integration
- Aldora-driven fallback control
- richer downstream eBUS `SetMode` capture and decoding
- forecast-aware control
- occupancy sensor weighting
- DHW hygiene / legionella risk monitor
- more advanced DHW strategy by outside temperature / turnover history
- tuning the policy from measured pilot results

## Pilot status

The MVP is now in the **live pilot installed** state:
- code exists
- service is deployed
- restore path is verified
- control API is live
- logging is enabled

What remains is not "build the MVP" but **observe, integrate, and refine the pilot**.

## DHW policy in MVP

DHW must remain socially reliable — if DHW breaks, the MVP fails regardless of elegance.

### Practical DHW rule
- maintain reliable DHW service
- use Cosy windows as the **preferred charging opportunities**
- respect actual cylinder state
- do **not** force pointless reheats when the cylinder is already hot enough

Specifically:
- if the cylinder is already above the meaningful trigger threshold, do nothing
- if it is already above target, definitely do not heat it further just because a Cosy window is open
- the controller should optimise **timing and targeting**, not blindly force reheats

### Hygiene / legionella
Legionella is treated as a monitored risk, not a constant high setpoint target.

The MVP should eventually monitor:
- DHW turnover
- cylinder temperature history
- time since last sufficiently hot hygiene cycle

But the main principle is already fixed:
- **service first, targeted hygiene intervention only when needed**

## Week-1 pilot success criteria

The first week is considered successful if the MVP produces:

- **lower electricity cost**
- **less cycling**
- **useful data for refinement**

These are the primary success criteria.

Better COP is expected to fall out of those, but the explicit top-level week-1 goals are cost, cycling, and learning.

## MVP philosophy

This MVP is intended to generate high-quality real-world data while making useful control decisions on the live system.

It is not conservative merely for appearances. It should make meaningful moves that are large enough to teach us something, while remaining recoverable, logged, and bounded by known-good baseline restore behaviour.

At the end of the day, the purpose is simple:

> **optimise the actual house, using the VRC 700, on the live system, with real data.**
