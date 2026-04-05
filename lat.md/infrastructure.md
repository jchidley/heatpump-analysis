# Infrastructure

Four monitoring devices feeding a central data hub. Sensors → MQTT → InfluxDB → Grafana/controller.

## Devices

Four hosts: three emon monitors and one central data hub.

| Device | IP | Role |
|---|---|---|
| **emonpi** | 10.0.1.117 | EmonPi2 (3× CT), DS18B20, Zigbee2MQTT (21 devices) |
| **emonhp** | 10.0.1.169 | Heat meter (MBUS) + SDM120 electricity meter → emoncms.org |
| **emondhw** | 10.0.1.46 | Multical 403 DHW meter (T1, T2, flow, volume) |
| **pi5data** | 10.0.1.230 | Central hub: Docker (Mosquitto, InfluxDB, Telegraf, Grafana, ebusd) + systemd (z2m-hub :3030, adaptive-heating-mvp :3031) |

MQTT credentials: `emonpi` / `emonpimqtt2016`. Z2M WebSocket: `ws://emonpi:8080/api` (no auth).

## Data Path

Each emon host runs local Mosquitto, bridging to central Mosquitto on pi5data.

```
emonhp  ──bridge──┐
emondhw ──bridge──┼──> pi5data mosquitto → telegraf → InfluxDB (bucket "energy") → Grafana
emonpi  ──bridge──┘           │
                              └── ebusd (Docker) → ebusd-poll.sh (systemd)
```

Telegraf subscribes to MQTT topics, writes to InfluxDB. The adaptive controller and thermal model both query InfluxDB directly. z2m-hub polls eBUS via TCP and MQTT for DHW tracking.

## eBUS Stack

Current: ESP32 adapter (closed-source firmware) → TCP → ebusd (Docker on pi5data, port 8888) → MQTT.

Three eBUS devices:

| Address | ID | Device | Role |
|---|---|---|---|
| 08 (slave 11) | HMU | aroTHERM Plus VWL 55/6 | Heat pump outdoor unit |
| 76 (slave 9) | VWZ AI | Indoor hydraulic station | Pump, 3-way valve, SP1 sensor |
| 15 (slave 2) | VRC 700 | System controller | Scheduling, weather compensation, UI |

ebusd config: `--enablehex`, `--enabledefine` are on. `grab result all` shows raw bus traffic including undecoded VWZ AI messages. eBUS coverage: 247 read + 216 write for VRC 700, 117 read + 14 passive for HMU, zero decoded for VWZ AI.

Planned replacement: xyzroe eBus-TTL adapter → Pico W (Rust/Embassy firmware) → MQTT directly. See `docs/pico-ebus-plan.md`.

## Room Sensors

12× SONOFF SNZB-02P (v2.2.0) + 1 emonth2. Data: Z2M → MQTT → pi5data Telegraf → InfluxDB.

13/13 room coverage complete (Office + Landing added 24 Mar 2026). Zigbee routers (ZBMINI switches) at hall, landing, kitchen, top_landing provide mesh coverage for battery sensors.

### Outside Temperature

Two sources at different resolutions for different purposes.

- **Primary** (real-time): `ebusd/poll/OutsideTemp` — Arotherm OAT sensor, 30s interval
- **Historical** (cross-check): emoncms feed 503093 — Met Office hourly

## Secrets

InfluxDB token on pi5data: `/etc/adaptive-heating-mvp.env` (root:root 0600, systemd EnvironmentFile). Same token as Telegraf. Dev fallback: `ak get influxdb` (warns if used). See `deploy/SECRETS.md`.

## VRC 700 Baseline Settings

Known-good register values for [[src/bin/adaptive-heating-mvp.rs#restore_baseline]] and manual recovery. Derived from `docs/vrc700-settings-audit.md`.

### Heating — Zone 1

Zone 1 heating registers. `Z1OpMode=night` during adaptive control, `auto` for standalone VRC 700.

| Register | Value | Notes |
|---|---|---|
| Z1OpMode | auto | Timer-driven (3=night during adaptive control) |
| Z1DayTemp | 21°C | Comfort setpoint |
| Z1NightTemp | 19°C | Setback |
| Z1Timer (all days) | 04:00;-:-;-:-;-:-;-:-;-:- | Day from 04:00 Cosy. **End = -:- not 00:00** |
| Hc1HeatCurve | 0.55 | Weather compensation gradient |
| Hc1MaxFlowTempDesired | 45°C | Emitter capacity + COP limit |
| Hc1MinFlowTempDesired | 20°C | Factory default (19 during adaptive control) |

### DHW

DHW timer windows aligned to Cosy tariff. All end times use `-:-` not `00:00`.

| Register | Value | Notes |
|---|---|---|
| HwcOpMode | auto | Timer-driven |
| HwcTempDesired | 45°C | Optimal per analysis |
| HwcSFMode | auto | Must be auto for timers. Boost = `load`, should auto-revert |
| HwcTimer (all days) | 05:30;07:00;13:00;15:00;22:00;-:- | Three Cosy windows. **End = -:- not 00:00** |
| CylinderChargeHyst | 5K | Triggers at 40°C |
| MaxCylinderChargeTime | 120 min | |
| HwcLockTime | 60 min | Anti-cycle lockout |

### System

| Register | Value |
|---|---|
| Hc1SummerTempLimit | 17°C |
| ContinuosHeating | −26°C |
| AdaptHeatCurve | no |
| HydraulicScheme | 8 |
