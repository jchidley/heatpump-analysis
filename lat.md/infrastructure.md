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

emonpi also runs `energy-hub-tesla.service` — a Python collector that polls the local Powerwall Gateway API every 10s and publishes raw metrics plus derived signals to MQTT as `emon/tesla/+`.

MQTT credentials: `emonpi` / `emonpimqtt2016`. Z2M WebSocket: `ws://emonpi:8080/api` (no auth).

## Data Path

Each emon host runs local Mosquitto, bridging to central Mosquitto on pi5data.

```
emonhp  ──bridge──┐
emondhw ──bridge──┼──> pi5data mosquitto → telegraf → InfluxDB (bucket "energy") → Grafana
emonpi  ──bridge──┘           │
                              └── ebusd (Docker) → ebusd-poll.sh (systemd)
```

Telegraf subscribes to `emon/+/+` and other MQTT topics, writes to InfluxDB. The adaptive controller and thermal model both query InfluxDB directly. z2m-hub polls eBUS via TCP and MQTT for DHW tracking.

### Tesla MQTT Topics

Published by `energy-hub-tesla.service` on emonpi every ~10s. Telegraf captures them via the `emon/+/+` subscription.

**Raw Powerwall metrics**: `emon/tesla/soc_pct`, `battery_W`, `home_W`, `grid_W`, `solar_W`, `voltage_V`, `frequency_Hz`, plus cumulative `_Wh` and `_import_Wh` / `_export_Wh` counters.

**Derived signals**:

| Topic | What |
|---|---|
| `emon/tesla/discretionary_headroom_to_next_cosy_kWh` | Battery kWh remaining after projected base-load consumption to the next Cosy window. Positive = spare capacity for discretionary loads; negative = deficit (base load alone will exhaust battery). Controllers gate on `headroom >= event_kWh`. |
| `emon/tesla/hours_to_next_cosy_h` | Hours until the next Cosy window opens |
| `emon/tesla/available_above_reserve_kWh` | Usable battery energy above reserve |

## eBUS Stack

Current: ESP32 adapter (closed-source firmware) → TCP → ebusd (Docker on pi5data, port 8888) → MQTT.

Three eBUS devices:

| Address | ID | Device | Role |
|---|---|---|---|
| 08 (slave 11) | HMU | aroTHERM Plus VWL 55/6 | Heat pump outdoor unit |
| 76 (slave 9) | VWZ AI | Indoor hydraulic station | Pump, 3-way valve, SP1 sensor |
| 15 (slave 2) | VRC 700 | System controller | Scheduling, weather compensation, UI |

ebusd config: `--enablehex`, `--enabledefine` are on. `grab result all` shows raw bus traffic including undecoded VWZ AI messages. eBUS coverage: 247 read + 216 write for VRC 700, 117 read + 14 passive for HMU, zero decoded for VWZ AI.

Planned replacement: xyzroe eBus-TTL adapter → Pico W (Rust/Embassy firmware) → MQTT directly. See `docs/pico-ebus-plan.md`. Phase 1 complete: `ebus-core/` crate provides `no_std` CRC, address, byte-stuffing, telegram parsing, and SYN-delimited framing (22 tests). Next: Phase 2 (PIO UART on Pico W).

## Room Sensors

11× SONOFF SNZB-02P (v2.2.0) indoor + 1 outdoor + 1 emonth2. Data: Z2M → MQTT → pi5data Telegraf → InfluxDB.

12/13 rooms have dedicated sensors (Office + Landing added 24 Mar 2026). Conservatory uses `ebusd/poll/Z2RoomTemp` from the VRC 700 (tracks within 1°C of the former SNZB-02P). `outside_temp_humid` (0x842712fffe772723) paired 7 Apr 2026, deployed to shaded SE wall near VRC 700 OAT sensor. Zigbee routers (ZBMINI switches) at hall, landing, kitchen, top_landing provide mesh coverage for battery sensors.

### Outside Sensors

Temperature and humidity from separate sources.

- **Temperature** (real-time): `ebusd/poll/OutsideTemp` — VRC 700 OAT sensor on shaded SE wall (well-sited, no compressor or solar influence), 30s interval
- **Temperature** (cross-check): emoncms feed 503093 — Met Office hourly
- **Humidity** (live): `outside_temp_humid` SNZB-02P on shaded SE wall near OAT sensor. Paired 7 Apr 2026. Provides: (a) direct AH_out for absolute ACH in all occupied bedrooms, (b) OAT temperature cross-check from a nearby but independent position, (c) before/after evidence for Elvina trickle vent closure. **Note**: link quality low (6) at initial pairing — monitor for dropouts.
- **Conservatory temperature**: `ebusd/poll/Z2RoomTemp` — VRC 700 Zone 2 room sensor, mounted in conservatory. Reads ~1°C below the former SNZB-02P position. Updated in `thermal_geometry.json`.
- **Leather humidity**: `emon/emonth2_23/humidity` — emonth2 in Leather. Provides 4th occupied-room data point for overnight moisture network (Parson Russell Terrier, ~10 g/h).

## Secrets

InfluxDB token on pi5data: `/etc/adaptive-heating-mvp.env` (root:root 0600, systemd EnvironmentFile). Same token as Telegraf. Dev machine: `ak get influxdb`. See `deploy/SECRETS.md`.

### Octopus API Credentials

Credentials for the `octopus-tariff` crate resolve in order: env vars → `~/.octopus-api-key` file → `~/github/octopus/.envrc` sourced via bash.

- `OCTOPUS_API_KEY` / `OCTOPUS_ACCOUNT_NUMBER` env vars take priority
- `~/.octopus-api-key` plain-text file (API key only, chmod 600) — used on emonpi where no `.envrc` is present
- `~/github/octopus/.envrc` sourced via bash — canonical store on dev machines, shared across `octopus`, `octopus-tariff`, and `heatpump-analysis`

On pi5data the env vars are injected by the systemd `EnvironmentFile` at `/etc/adaptive-heating-mvp.env`.

### Ad-hoc InfluxDB Queries from Dev Machine

Query InfluxDB on pi5data from WSL using `ak get influxdb` for the token. InfluxDB v2 Flux API at `http://pi5data:8086`, org `home`, bucket `energy`.

See [[constraints#InfluxDB-First Analysis]]: all filtering, aggregation, pivoting, and arithmetic belong in Flux — client code is for formatting only.

```bash
# Good: Flux does the heavy lifting, client just prints
INFLUX_TOKEN=$(ak get influxdb)
curl -s -H "Authorization: Token $INFLUX_TOKEN" \
  "http://pi5data:8086/api/v2/query?org=home" \
  -H "Content-Type: application/vnd.flux" \
  -H "Accept: application/csv" \
  --data-raw '
    from(bucket:"energy")
      |> range(start: -12h)
      |> filter(fn: (r) => r._measurement == "adaptive_heating_mvp")
      |> filter(fn: (r) => r._field == "target_flow_c" or r._field == "leather_temp_c" or r._field == "outside_temp_c")
      |> pivot(rowKey: ["_time"], columnKey: ["_field"], valueColumn: "_value")
      |> keep(columns: ["_time", "action", "target_flow_c", "leather_temp_c", "outside_temp_c"])
      |> sort(columns: ["_time"])
      |> group()
  '
```

Key measurements: `ebusd` (heat pump / VRC 700), `ebusd_poll` (polled registers), `zigbee` (room sensors, `_field="temperature"`), `adaptive_heating_mvp` (controller decisions — `action` is a tag), `emon` (emonpi2 CTs, tesla Powerwall, multical heat meter, emonth2 room sensor), `dhw_inflection` / `dhw_capacity` (DHW sessions).

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
| HwcMode (hmu) | eco / normal | Readable via eBUS for status and scheduler inputs, but read-only from external masters |
| HwcTimer (all days) | 04:00;07:00;13:00;16:00;22:00;23:59 | Three Cosy windows matching tariff. `sync_morning_dhw_timer` may rewrite one weekday to `13:00;16:00;22:00;23:59;-:-;-:-` when predicted T1 at 07:00 is ≥40°C. Dedup state cleared on write failure and on startup |
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
