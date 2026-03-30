# DHW Fixes

Date: 30 March 2026

Issues found during DHW debugging session 29-30 March 2026.

---

## Fix 1: T1/T2 sensor labelling confusion

During the debugging session, confusion arose about which Multical temperature sensor (T1/T2) measures which location. This section records the definitive mapping, verified against live data.

## Physical sensor locations

Three temperature sensors on the 300L Kingspan Albion cylinder:

| Sensor | Physical location | Height | MQTT topic | Typical reading |
|---|---|---|---|---|
| Multical T1 | Hot water draw-off (cylinder top) | 1580mm | `emon/multical/dhw_t1` | ~42°C when charged |
| Multical T2 | Cold water inlet, after WWHR (cylinder bottom) | 540mm | `emon/multical/dhw_t2` | ~24°C (mains via WWHR) |
| VR 10 NTC (SP1) | Dry stat pocket, just above bottom coil | ~600mm est. | `ebusd/poll/HwcStorageTemp` | 23-43°C (see below) |

## Verification (30 March 2026, 08:30 BST, no DHW flow)

```
emon/multical/dhw_t1  = 42.2°C  → hot water at cylinder top ✓
emon/multical/dhw_t2  = 24.7°C  → cold mains inlet ✓
ebusd/poll/HwcStorageTemp = 32.5°C  → stratified zone above bottom coil ✓
```

## Kamstrup Multical 403 naming convention

The Kamstrup Modbus register map defines T1 as "inlet" and T2 as "outlet" from the **meter's** design perspective. However, the T1 and T2 probes are physically installed by the plumber - which probe goes where depends on installation.

In this installation:
- T1 probe → cylinder top (hot) → Modbus register 6
- T2 probe → cylinder bottom (cold inlet) → Modbus register 8

The emonhub config on emondhw correctly maps register 6 → `dhw_t1` and register 8 → `dhw_t2`.

## Existing documentation status

These documents correctly describe T1=hot/top, T2=cold/bottom:
- `heating-monitoring-setup.md` (lines 133-134, 497-498, 586-587) ✓
- `docs/dhw-cylinder-analysis.md` (cylinder diagram, lines 40-42, 137) ✓
- `docs/vrc700-settings-audit.md` (sensor table) ✓

## Grafana dashboard

**TODO:** Verify that the Grafana "DHW Hot Water" dashboard labels T1 and T2 correctly. The Grafana admin password needs to be recovered to check dashboard definitions. The colour convention in `heating-monitoring-setup.md` maps dhw_t1 to red (flow/hot) and dhw_t2 to blue (return/cold), which is correct.

## HwcStorageTemp behaviour

The VR 10 in the dry stat pocket (just above the bottom coil) reads the stratified water temperature at that height:

- When cylinder is fully charged: ~42–43°C (close to T1)
- Gradual standing loss: drops ~0.3°C/hour with no draws
- Small draws (<30L): barely moves — cold front hasn’t reached sensor height
- Large draws (90L bath): **crashes from 35→23.5°C in 20 minutes** as cold mains water floods past the sensor
- After crash: stabilises at mains temperature (~23°C) until next charge

The VRC 700 uses this sensor with `CylinderChargeHyst` = 5K. At 45°C target, DHW charging triggers when HwcStorageTemp drops below 40°C.

---

## Note: z2m-hub DHW remaining litres

The z2m-hub (`~/github/z2m-hub/src/main.rs`) tracks remaining hot water using volume subtraction from the Multical volume register. The 161L usable capacity was properly calibrated from T1 thermocline inflection data (see `docs/dhw-cylinder-analysis.md`). The volume tracking model is sound.

The three cylinder sensors (T1 top, HwcStorageTemp dry pocket, T2 mains inlet) could potentially refine the estimate — particularly after partial charges (boosts) or during long standing losses — but that’s an enhancement for the z2m-hub project (`~/github/z2m-hub/`), not a fix.

---

## Fix 3: Consistent T1/T2 naming across InfluxDB, Grafana, and docs

The Kamstrup Multical 403 Modbus register map (58101500) defines:
- Register 0x0006 = **"Temp. 1 Inlet"**
- Register 0x0008 = **"Temp. 2 Outlet"**

"Inlet" and "Outlet" are from the **meter's energy measurement perspective** — the meter measures heat in the water flowing through it. In this DHW installation, hot water from the cylinder top flows into the meter (T1 = hot), and the cold mains reference is T2.

This is counterintuitive: "Inlet" = hot, "Outlet" = cold. It causes confusion because for the *cylinder*, the inlet is cold mains and the outlet is hot water.

### Current state

| Layer | T1 label | T1 reads | T2 label | T2 reads |
|---|---|---|---|---|
| Kamstrup register | "Temp. 1 Inlet" | 42°C | "Temp. 2 Outlet" | 24°C |
| emonhub config | `t1` | 42°C | `t2` | 24°C |
| MQTT topic | `emon/multical/dhw_t1` | 42°C | `emon/multical/dhw_t2` | 24°C |
| InfluxDB field tag | `dhw_t1` | 42°C | `dhw_t2` | 24°C |
| heating-monitoring-setup.md | "DHW hot water out" | ✓ | "DHW cold in post-WWHR" | ✓ |
| Grafana dashboard | **TODO: check** | ? | **TODO: check** | ? |

### What to keep

- **MQTT topics** (`emon/multical/dhw_t1`, `emon/multical/dhw_t2`) — do NOT rename. Would break InfluxDB history, Telegraf, z2m-hub, all queries.
- **emonhub register mapping** — correct as-is (register 6→t1, register 8→t2)

### What to fix

Display labels wherever T1/T2 appear should use a consistent format that includes both the Kamstrup register name and the physical meaning:

| Sensor | Standard label | Short label |
|---|---|---|
| dhw_t1 | T1 Cylinder Top (Hot Out) | T1 Hot |
| dhw_t2 | T2 Mains Inlet (Cold In) | T2 Cold |
| HwcStorageTemp | Cylinder Pocket (Above Bottom Coil) | Cyl Pocket |

Places to update:
1. **Grafana dashboard panel titles** — need admin password recovery first
2. **z2m-hub dashboard** (`HOME_PAGE` in `~/github/z2m-hub/src/main.rs`) — update any T1/T2 display labels
3. **Any new code/docs** — use the standard labels above

---

## Fix 3b: Grafana DHW Temperatures chart

The emondhw Grafana dashboard "DHW Temperatures" chart should show all three cylinder sensors on a single panel:

| Series | Source | Label |
|---|---|---|
| `emon/multical/dhw_t1` | emondhw Multical | T1 Hot Out |
| `emon/multical/dhw_t2` | emondhw Multical | T2 Cold In |
| `ebusd/poll/HwcStorageTemp` | ebusd on pi5data | Cylinder Temp |

Vaillant calls the SP1 dry pocket sensor reading "Actual DHW cylinder temperature" on the VWZ AI display and "cylinder temperature" in the VRC 700 installer menus. **Cylinder Temp** is the appropriate short label.

This gives a complete picture of the cylinder state at a glance:
- T1 Hot Out = what comes out of the tap
- Cylinder Temp = where the hot/cold boundary is (the VRC 700's charging trigger)
- T2 Cold In = what's replacing the hot water when you draw

Requires Grafana admin password recovery to edit the dashboard.

---

## Fix 4: Add Grafana and InfluxDB passwords to ak keystore

During this session we couldn't access the Grafana API (admin password unknown) and the InfluxDB token was only available hardcoded in `model/house.py`.

Both credentials need to be added to the `ak` GPG-encrypted keystore (`~/tools/api-keys/secrets/`) so they're available at runtime without hardcoding:

```bash
# Add these:
ak set grafana        # Grafana admin password (pi5data:3000)
ak set influxdb       # InfluxDB token (pi5data:8086)

# Then usable as:
export INFLUX_TOKEN=$(ak get influxdb)
curl -s "http://admin:$(ak get grafana)@pi5data:3000/api/search"
```

The InfluxDB token is currently:
- Hardcoded in `model/house.py` (line: `INFLUX_TOKEN = "jPTPrw..."`) — should be removed after ak is set up
- Hardcoded in Telegraf config (`/home/jack/monitoring/docker-compose.yml` or similar on pi5data)
- Used by `model/thermal-config.toml` via `token_env = "INFLUX_TOKEN"` (correct pattern — reads from env)

The Grafana admin password was changed from the default (`admin`) but not recorded anywhere accessible.

---

## Fix 5: HwcStorageTemp description was wrong in earlier analysis

During this debugging session, `HwcStorageTemp` was initially misidentified as:
- Bottom NTC reading ambient/mains temperature (wrong)
- Coil return pipe temperature (wrong)
- Internal VWZ AI sensor (wrong)

**Actual:** VR 10 NTC probe in the cylinder's dry stat pocket, just above the bottom coil, connected to VWZ AI terminal SP1. Physically confirmed 30 March 2026.

The earlier incorrect descriptions led to wrong conclusions about why DHW wasn't triggering. The sensor IS reading real cylinder water temperature — it's just in the lower stratification zone where temperature varies significantly with draw volume.

This is documented correctly in `docs/vrc700-settings-audit.md` now.

---

## Summary of immediate actions

| Priority | Action | Status |
|---|---|---|
| 🔴 | Fix VRC 700 timers — re-write with `-:-` instead of `00:00` as end time (see `vrc700-settings-audit.md` issue #5) | **Blocking** — `00:00` in TTM encoding = start of day, not end. Windows were backwards/invalid. |
| 🔴 | Check `HwcSFMode` is `auto` (gets stuck on `load` after boosts) | Reset to auto on 30 Mar |
| 🟡 | Add `grafana` and `influxdb` to ak keystore | Needed for dashboard fixes |
| 🟡 | Update Grafana DHW chart — add Cylinder Temp, fix T1/T2 labels | Blocked by Grafana password |
| 🟡 | Update z2m-hub dashboard labels | In `~/github/z2m-hub/` |
| 🟢 | Remove hardcoded InfluxDB token from `model/house.py` | After ak is set up |
| 🟢 | Do not run `cosy-scheduler` — retired | Already not a service |
| ⚪ | z2m-hub remaining litres refinement using temperature data | Enhancement, not a fix |
