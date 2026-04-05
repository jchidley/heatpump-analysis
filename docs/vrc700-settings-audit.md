# VRC 700 Settings Audit — 30 March 2026

This file keeps the audit trail, root-cause investigation, and recovery details behind the VRC 700 configuration. The canonical current baseline settings and timer rules now live in `lat.md/infrastructure.md` and `lat.md/constraints.md`.

## System components

Three units on the eBUS, plus external sensors:

| eBUS address | ID | Device | Role |
|---|---|---|---|
| 08 (slave 11) | HMU00 (SW 0902, HW 5103) | **aroTHERM plus VWL 55/6** outdoor unit | Heat pump — compressor, fan, refrigerant circuit |
| 76 (slave 9) | VWZIO (SW 0202, HW 0103) | **VWZ AI** indoor unit / hydraulic station | Circulation pump, 3-way diverter valve, SP1 cylinder sensor terminal, eBUS bridge to outdoor unit |
| 15 (slave 2) | 70000 (SW 0614, HW 6903) | **VRC 700** system controller | User interface, timer scheduling, weather compensation, room temperature sensor (Zone 2) |
| 31 (master 8) | — | **ebusd** v26.1 (ESP32 adapter v5, build 6311) | Monitoring and commands via TCP port 8888 |

External sensors:
- **Multical 403** (on emondhw) — DHW heat meter on cylinder secondary side. T1 = hot water outlet (cylinder top). T2 = cold water inlet (mains, after WWHR).
- **VR 10 NTC** on SP1 terminal of VWZ AI — cylinder temperature sensor in dry stat pocket, just above bottom coil. Physically confirmed 30 March 2026.
- **Outdoor temperature sensor** — connected to VWZ AI (DCF/AF terminal)
- **12× SNZB-02P + 1× emonth2** — room temperature sensors (Zigbee, not on eBUS)

The VRC 700 is the **scheduling brain** — it decides when to request heating and DHW based on its timers, setpoints, and the data it receives from the other units via eBUS. The HMU and VWZ execute what the VRC 700 requests.

## Context

On 29 March 2026, a pi session programmatically wrote new timer and setpoint values to the VRC 700 via eBUS (ebusd on pi5data). The intention was to move to a timer-only configuration with no external automation (cosy-scheduler, dhw-auto-trigger all retired).

The eBUS writes appeared to succeed — values read back correctly via `read -f -c 700`. However, the VRC 700's internal scheduler is **not acting on the new timers**:

- **Z1Timer** (heating): Day mode should start at 04:00. At 08:10 BST on 30 March, `Z1ActualRoomTempDesired` still shows 19°C (setback), not 21°C (day). The HP ran setback flow temps all night and all morning.
- **HwcTimer** (DHW): Three windows programmed. The VRC 700 never set `HwcDemand=1` in any window on 29 March. DHW only fired via manual boost (`HwcSFMode=load`).
- **HwcSFMode** got stuck on `load` after a boost — the VRC 700 didn't auto-revert because the charge completed with the cylinder above hysteresis. This may have blocked timer-driven DHW until manually reset to `auto`.

The eBUS register values are correct. The VRC 700's internal scheduling engine is not using them. The controller likely needs the timers to be confirmed via its own UI, or may need a full reset and re-entry of settings.

## Known-good target configuration

These are the settings derived from the heating and DHW plans (`docs/heating-plan.md`, `docs/dhw-plan.md`). If the controller needs to be reset, re-enter these values.

### Heating — Zone 1

| Setting | Value | Notes |
|---|---|---|
| Z1OpMode | auto | Timer-driven |
| Z1DayTemp | 21°C | Comfort setpoint |
| Z1NightTemp | 19°C | Setback — only fires on coldest nights |
| Z1Timer (all days) | 04:00–-:- | Day mode from 04:00 (Cosy start), night until 04:00. **Use `-:-` not `00:00` for end time.** |
| Hc1HeatCurve | 0.55 | Weather compensation gradient |
| Hc1MaxFlowTempDesired | 45°C | |
| Hc1MinFlowTempDesired | 20°C | |

### DHW

| Setting | Value | Notes |
|---|---|---|
| HwcOpMode | auto | Timer-driven |
| HwcTempDesired | 45°C | Optimal per analysis (docs/dhw-plan.md) |
| HwcSFMode | auto | Must be auto for timer scheduling to work. Boost sets to `load`, should auto-revert after charge completes. |
| HwcMode (hmu) | eco | Mild season. Manually switch to normal Nov–Mar on the Arotherm controller (not writable via eBUS). |
| CylinderChargeHyst | 5K | Charge triggers when HwcStorageTemp < (45 − 5) = 40°C |
| CylinderChargeOffset | 0 | |
| MaxCylinderChargeTime | 120 min | |
| HwcLockTime | 60 min | Anti-cycle lockout after charge |
| HwcParallelLoading | off | |

### DHW Timer (all days identical)

| Window | Cosy period | Rationale |
|---|---|---|
| 05:30–07:00 | Morning Cosy (04:00–07:00) | Main DHW. Delayed 1.5h so HP heats house first at Cosy rate. |
| 13:00–15:00 | Afternoon Cosy (13:00–16:00) | Top-up. Shortened from 16:00 to prevent peak spills. |
| 22:00–-:- | Evening Cosy (22:00–00:00) | Post-shower top-up. **Use `-:-` not `00:00` for end time.** |

### Circulation Timer (CcTimer)

| Day | Window | Notes |
|---|---|---|
| Mon–Fri | 06:00–22:00 | Not changed by us — pre-existing |
| Saturday | 07:30–23:30 | |
| Sunday | 07:30–22:00 | |

### Zone 2

| Setting | Value | Notes |
|---|---|---|
| Z2RoomZoneMapping | VRC700 | VRC 700's internal thermometer |
| Z2OpMode | auto | |
| Z2DayTemp | 20°C | |
| Z2NightTemp | 21°C | ⚠️ Appears swapped — day < night. Pre-existing, not changed by us. |
| Z2RoomTemp | ~20.3°C | Live reading from VRC 700 sensor |

### System

| Setting | Value |
|---|---|
| OpMode | auto |
| GlobalSystemOff | no |
| HolidayTemp | 15°C |
| Hc1SummerTempLimit | 17°C |
| Hc1AutoOffMode | night |
| Hc1RoomTempSwitchOn | off |
| ContinuosHeating | -26°C |
| AdaptHeatCurve | no |
| Hc1CircuitType | mixer |
| HydraulicScheme | 8 |

## eBUS commands to re-apply settings

If entering via eBUS (may not be reliable — see issues below):

```bash
# Connect to ebusd on pi5data
# Heating setpoints
echo 'write -c 700 Z1DayTemp 21' | nc -w3 localhost 8888
echo 'write -c 700 Z1NightTemp 19' | nc -w3 localhost 8888

# Heating timers (all days: day mode 04:00-00:00)
for day in Monday Tuesday Wednesday Thursday Friday Saturday Sunday; do
  echo "write -c 700 Z1Timer_${day} 04:00;-:-;-:-;-:-;-:-;-:-" | nc -w3 localhost 8888
done

# DHW timers (all days: 05:30-07:00, 13:00-15:00, 22:00-00:00)
for day in Monday Tuesday Wednesday Thursday Friday Saturday Sunday; do
  echo "write -c 700 HwcTimer_${day} 05:30;07:00;13:00;15:00;22:00;-:-" | nc -w3 localhost 8888
done

# DHW settings
echo 'write -c 700 HwcOpMode auto' | nc -w3 localhost 8888
echo 'write -c 700 HwcTempDesired 45' | nc -w3 localhost 8888
echo 'write -c 700 HwcSFMode auto' | nc -w3 localhost 8888

# Clock (BST)
echo "write -c 700 Time $(date '+%H:%M:%S')" | nc -w3 localhost 8888
echo "write -c 700 Date $(date '+%d.%m.%Y')" | nc -w3 localhost 8888
```

## Known issues with eBUS timer writes

1. **Timer writes return "empty"** — this is normal for the TTM data type. The VRC 700 doesn't send a payload in its ACK. The write IS accepted — values read back correctly with `read -f`.

2. **Timer writes may not activate the VRC 700's internal scheduler.** On 29 March, all timers were written successfully (verified by readback) but the VRC 700 did not act on them. `Z1ActualRoomTempDesired` stayed at 19°C (setback) past 04:00, and `HwcDemand` was never set to 1 in any DHW window. **Root cause identified: see issue #5 below — `00:00` as end time is invalid in TTM encoding.** The register values were stored correctly but represented invalid (backwards) time windows.

3. **HwcSFMode can get stuck on `load`.** If a boost is sent (`HwcSFMode=load`) but the VRC 700 decides the cylinder doesn't need charging (e.g., HwcStorageTemp already above target minus hysteresis), the charge never starts, never completes, and `HwcSFMode` never auto-reverts to `auto`. This blocks timer-driven DHW until manually reset.

4. **Multiple rapid timer writes.** The earlier session wrote HwcTimer three times in 10 minutes (05:30 start, then 05:00, then 05:30 again). This may have confused the VRC 700.

5. **`00:00` as end time is INVALID — confirmed root cause.** The TTM (Truncated Time) data type in ebusd uses 8 bits with 10-minute resolution. Byte `0x00` = `00:00` (midnight at start of day). Byte `0x90` = `-:-` (replacement/not-set), which is also what `24:00` would encode to — they're the same byte. There is no way to represent "end of day" as a time value; you must use `-:-` (the replacement) instead.

   The Z1Timer was written as `04:00;00:00` — meaning window from 04:00 to 00:00. Since `00:00` = start of day, the end is before the start, and the VRC 700 treats it as invalid/empty. This is why `Z1ActualRoomTempDesired` stays at 19°C (setback) past 04:00. The same bug affects HwcTimer's third window `22:00;00:00`.

   The old working value `05:00;-:-` used `-:-` as the end time, meaning "no end specified" — the VRC 700 interprets this as "until end of day". **All timers using `00:00` as an end time must be changed to `-:-`.**

   **Open question:** The invalid `00:00` end time clearly explains Z1Timer (single window, entirely broken). For HwcTimer, windows 1 and 2 (`05:30;07:00` and `13:00;15:00`) have valid start/end pairs — only window 3 (`22:00;00:00`) is backwards. Yet DHW never triggered in *any* window. Either: (a) the VRC 700 validates the entire 6-byte timer pattern and rejects all windows if any one is invalid, or (b) there's a second issue. Testing the `-:-` fix will distinguish these cases.

   **Additional evidence:** The CcTimer (circulation timers, unchanged and working) all use explicit end times before midnight (`06:00;22:00`, `07:30;23:30`) or `-:-` for unused slots — never `00:00`. The old working Z1Timer was `05:00;-:-`. The VRC 700 factory default time periods are `06:00–08:00`, `16:30–18:00`, `20:00–22:30` (from the operating manual p10) — again, all end times well before midnight. The timer message sends all 6 TTM bytes atomically in one eBUS write (data prefix `04...` + 6 bytes), so the VRC 700 receives and can validate the entire pattern at once.

   Sources:
   - `ebusd/src/lib/ebus/datatype.cpp` line 1337 — `DateTimeDataType("TTM", 8, 0, 0x90, false, true, 10)`
   - `ebusd/src/lib/ebus/test/test_data.cpp` lines 234–242
   - `docs/ebus-specs/vaillant_ebus_v0.5.0.pdf` section 3.1.3 (GetTimerProgram) — independently confirms timer byte range `0..90h`, unit 10min, replacement `90h`

## What HwcStorageTemp actually measures

`HwcStorageTemp` is reported by the VRC 700 (circuit 700, register HwcStorageTemp). The VRC 700 receives this value from the VWZ AI indoor unit. The VWZ AI manual (0020291573_01) shows terminal SP1 is designated "Domestic hot water cylinder temperature sensor", designed for an optional VR 10 NTC probe inserted into the cylinder's dry pocket.

The VRC 700 uses `HwcStorageTemp` to decide whether to trigger DHW charging: it compares this value against `HwcTempDesired` minus `CylinderChargeHyst`. With current settings (45°C target, 5K hysteresis), charging triggers when `HwcStorageTemp` drops below 40°C.

Observed behaviour of `HwcStorageTemp`:
- When cylinder fully charged: ~42–43°C (close to T1)
- Standing losses: drops ~0.3°C/hour with no draws
- Small draws (<30L): barely moves — cold front hasn’t reached sensor height
- Large draws (90L bath): crashes from 35→23.5°C in 20 minutes as cold mains floods past sensor
- After large draw: stabilises at mains water temperature (~23°C) until next charge

The sensor reads real cylinder water temperature at its height (~600mm), but being in the lower stratification zone it can read very differently from the top of the cylinder. Snapshot at 08:30 BST 30 March: HwcStorageTemp=32.5°C while Multical T1 (cylinder top)=42.5°C.

**Confirmed 30 March 2026:** A VR 10 NTC sensor IS fitted in the cylinder's dry stat pocket, located just above the bottom coil. This is connected to the VWZ AI SP1 terminal.

Three sensors on the cylinder:

| Sensor | Location | Register/topic | Role |
|---|---|---|---|
| Multical T1 | Hot water outlet (cylinder top) | `emon/multical/dhw_t1` | True usable hot water temperature |
| Multical T2 | Cold water inlet (mains, after WWHR) | `emon/multical/dhw_t2` | Inlet temperature |
| VR 10 NTC (SP1) | Dry stat pocket, just above bottom coil | `ebusd/poll/HwcStorageTemp` | VRC 700 uses this for DHW charging decisions |

Snapshot at 08:30 BST 30 March:
- T1 (cylinder top) = 42.3°C
- HwcStorageTemp (dry pocket, above bottom coil) = 32.5°C
- T2 (mains inlet) = 24.2°C

The VRC 700 uses HwcStorageTemp for its charging decisions. With 5K hysteresis and 45°C target, it should trigger a charge when HwcStorageTemp drops below 40°C.

## What was running on pi5data (29 March)

| Service | Status | Writes to eBUS? | Needed? |
|---|---|---|---|
| ebusd-poll.service | Running | No (read-only) | ✅ Monitoring |
| z2m-hub.service | Running | Yes — `/api/dhw/boost` sends `HwcSFMode load` | ⚠️ Manual boost only |
| cosy-scheduler (binary) | Removed from pi5data 30 Mar | Yes — writes `hmu HwcMode` and `700 Z1OpMode` | ❌ Retired — binary deleted |
| dhw-auto-trigger.sh | Not running | Yes — sends `HwcSFMode load` on sustained draw | ❌ Retired |

## Manuals and specifications

### Vaillant product manuals

| Document | Reference | Covers | Device |
|---|---|---|---|
| VRC 700 Installation Instructions | 0020262579_01 | Installer settings, hysteresis, timers, hydraulic schemes | VRC 700 controller |
| VRC 700 / multiMATIC 700 Operating Instructions | 0020200782_00 | User settings, operating modes, time periods | VRC 700 controller |
| VWZ AI Installation & Operating Instructions | 0020291573_01 | Indoor unit, SP1 sensor terminal, electrical connections, status codes | VWZ AI indoor unit |
| aroTHERM plus VWL 35–75/6 Installation & Maintenance | 0020330791_03 | Outdoor unit, refrigerant circuit | HMU outdoor unit |

### eBUS protocol specifications

eBUS is an open, documented standard (2-wire serial bus, 2400 baud, 9–24V) with published specs for the physical, data-link, and application layers. Vaillant implements the standard but adds their own unpublished extensions for device-specific commands and registers. These extensions have been reverse-engineered by the community.

| Document | Location | Notes |
|---|---|---|
| eBUS Physical & Data Link Layer v1.3.1 | `docs/ebus-specs/` | Open standard (eBUS Interest Group, 2007) |
| eBUS Application Layer v1.6.1 | `docs/ebus-specs/` | Open standard (eBUS Interest Group, 2007) |
| Vaillant eBUS Extensions v0.5.0 | `docs/ebus-specs/` | Community reverse-engineered (2014). Confirms timer byte encoding: range `0..0x90`, 10min resolution, `0x90` = replacement. |
| Vaillant eBUS Extensions (pittnerovi) | `docs/ebus-specs/` | Community reverse-engineered. Based on VRS620. |
| ebusd-configuration CSVs | Runtime (fetched by ebusd `--scanconfig`) | Device-specific register maps. Our VRC 700 uses `15.700.csv`. |

See `docs/ebus-specs/SOURCES.md` for download URLs, full provenance, and a comparison of all known eBUS implementations.

### Planned stack simplification: Pico W + xyzroe eBus-TTL adapter

The current eBUS chain is: **ESP32 adapter (closed firmware, separate PSU)** → TCP → **ebusd (Docker on pi5data)** → MQTT.

This will be replaced by: **xyzroe eBus-TTL adapter (galvanically isolated, bus-powered)** → **Pico W (Rust/Embassy firmware)** → MQTT directly.

See `docs/pico-ebus-plan.md` for the full build plan. Protocol reference code in submodules `yuhu-ebus/` (protocol engine) and `esp-arduino-ebus/` (danielkucera's firmware, uses yuhu-ebus). See `docs/ebus-specs/SOURCES.md` for all eBUS specs and implementations.

Download links:
- VRC 700 Installation: https://professional.vaillant.co.uk/downloads/product-manuals/vrc-700/vrc-700-installation-instructions-1968307.pdf
- VRC 700 Operating: https://elearning.vaillant.com/vrc700/ci/en/documents/uk/infopool/Operating_instructions.pdf
- VWZ AI: https://professional.vaillant.co.uk/downloads/aproducts/renewables-1/arotherm-plus/monoblock-heat-pump-system-vwz-ai-heat-pump-appliance-interface-2685948.pdf
- aroTHERM plus: https://professional.vaillant.co.uk/downloads/aproducts/renewables-1/arotherm-plus/arotherm-plus-vwl-35-75-a-s2-installation-operation-manual-0020330791-03-2806789.pdf

Suggested local filenames:
- `Vaillant_VRC700_Installation_Instructions_0020262579_01.pdf`
- `Vaillant_VRC700_multiMATIC700_Operating_Instructions_0020200782_00.pdf`
- `Vaillant_VWZ_AI_Installation_Operating_0020291573_01.pdf`
- `Vaillant_aroTHERM_plus_VWL_35-75_Installation_Maintenance_0020330791_03.pdf`

## Current state (30 March 2026)

The VRC 700 has invalid timer data that was written via eBUS on 28–29 March. The timer register values read back correctly but contain `00:00` (byte `0x00`) as end times, which the VRC 700 treats as invalid (end before start). This is the root cause of both the heating and DHW scheduling failures.

All other VRC 700 settings (OpMode, setpoints, HwcSFMode, clock) are correct. The VWZ AI and HMU are unaffected — we never sent commands to either (the one `write -c hmu HwcMode` was rejected by the HMU as read-only). The VWZ AI and HMU have been operating correctly based on whatever the VRC 700 tells them via its SetMode messages every ~10 seconds — but those messages have been wrong because the VRC 700's timers are broken.

**What is broken right now:**

| Register | Current value | Bytes on bus | Problem | Correct value | Correct bytes |
|---|---|---|---|---|---|
| Z1Timer (all days) | `04:00;00:00;-:-;-:-;-:-;-:-` | `18 00 90 90 90 90` | `0x00` end = start of day, before `0x18` (04:00) | `04:00;-:-;-:-;-:-;-:-;-:-` | `18 90 90 90 90 90` |
| HwcTimer (all days) | `05:30;07:00;13:00;15:00;22:00;00:00` | `21 2A 4E 5A 84 00` | Window 3: `0x00` end before `0x84` (22:00); may invalidate all 3 windows | `05:30;07:00;13:00;15:00;22:00;-:-` | `21 2A 4E 5A 84 90` |

**Effect:** `Z1ActualRoomTempDesired` stuck at 19°C (setback) 24/7. `HwcDemand` never set to 1 in any DHW window. Heat pump runs at setback flow temperatures all day. DHW only fires via manual boost (`HwcSFMode=load`).

**The fix is a single byte change in each timer:** `0x00` → `0x90` in the last TTM position.

## Fix commands

These commands replace `00:00` with `-:-` (byte `0x90` = replacement/not-set = "until end of day") in all timer end times. Run from any machine that can reach pi5data port 8888:

```bash
# Fix Z1Timer — heating day mode from 04:00, no explicit end
for day in Monday Tuesday Wednesday Thursday Friday Saturday Sunday; do
  echo "write -c 700 Z1Timer_${day} 04:00;-:-;-:-;-:-;-:-;-:-" | nc -w3 pi5data 8888
done

# Fix HwcTimer — 3 windows, window 3 end changed from 00:00 to -:-
for day in Monday Tuesday Wednesday Thursday Friday Saturday Sunday; do
  echo "write -c 700 HwcTimer_${day} 05:30;07:00;13:00;15:00;22:00;-:-" | nc -w3 pi5data 8888
done
```

Expected response: `empty` for each write (normal for TTM timer writes — see issue #1 above).

## Verification after fix

**Immediate (within 10 minutes):**

```bash
# Readback — should show -:- instead of 00:00
echo "read -f -c 700 Z1Timer_Monday" | nc -w3 pi5data 8888
# Expected: 04:00;-:-;-:-;-:-;-:-;-:-

echo "read -f -c 700 HwcTimer_Monday" | nc -w3 pi5data 8888
# Expected: 05:30;07:00;13:00;15:00;22:00;-:-

# If between 04:00–00:00 BST, Z1ActualRoomTempDesired should switch to 21°C
echo "read -f -c 700 Z1ActualRoomTempDesired" | nc -w3 pi5data 8888
# Expected: 21 (if day mode) or 19 (if night 00:00–04:00)
```

**Within 24 hours — confirm scheduling works:**

| Check | Command | Expected | When |
|---|---|---|---|
| Heating day mode | `read -c 700 Z1ActualRoomTempDesired` | 21 | After 04:00 BST |
| Heating setback | `read -c 700 Z1ActualRoomTempDesired` | 19 | 00:00–04:00 BST |
| DHW demand | Sniff `SetMode` from VRC 700 (QQ=10) | `HwcDemand=1` | During DHW windows when HwcStorageTemp < 40°C |
| DHW charge | `read -c hmu RunDataStatuscode` | `Warm_Water_Compressor_active` | During a timer-triggered charge |

**Resolved 30 March 2026:** Timers confirmed on VRC 700 display and are now working correctly. Both heating (Z1Timer day/night switching) and DHW (timer-triggered charges) operational.

## Ongoing rules

1. **Do not use cosy-scheduler.** Binary removed from pi5data 30 Mar 2026. Source in `src/bin/cosy-scheduler.rs` kept for reference only. Do not redeploy.

2. **Monitor HwcSFMode** after any manual boost — if it gets stuck on `load`, reset with `write -c 700 HwcSFMode auto`.

3. **Never use `00:00` as a timer end time.** Use `-:-` for "until end of day". See issue #5 above for the full explanation.

4. **All commands go to the VRC 700 (`-c 700`).** The VRC 700 sends SetMode to the HMU every ~30 seconds (25,700 occurrences in a grab session). The flow temperature demand goes **directly to the HMU** (address 0x08), not via the VWZ AI. The VWZ AI (0x76) receives separate messages with zeros for flow temp — it only handles valve/pump commands, not flow temperature control. Do not write directly to the HMU — the VRC 700 will overwrite within 30 seconds.
