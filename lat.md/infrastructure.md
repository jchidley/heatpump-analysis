# Infrastructure

Four monitoring devices feeding a central data hub. Sensors → MQTT → pi5data TSDB services → Grafana/controller.

## Devices

Four hosts: three emon monitors and one central data hub.

| Device | IP | Role |
|---|---|---|
| **emonpi** | 10.0.1.117 | EmonPi2 (3× CT), DS18B20, Zigbee2MQTT (21 devices) |
| **emonhp** | 10.0.1.169 | Heat meter (MBUS) + SDM120 electricity meter → emoncms.org |
| **emondhw** | 10.0.1.46 | Multical 403 DHW meter (T1, T2, flow, volume) |
| **Serial hardware note** | — | `emondhw` and `emonhp` both currently use QinHeng/WCH `1a86:55d3` USB CDC ACM adapters on one measurement path. `emondhw` uses it for the Multical 403 Modbus link; `emonhp` uses the same adapter family for the SDM120 path, while its MBUS path is a separate Prolific USB serial device and its emonTxV5 path is Silicon Labs CP2102N. |
| **pi5data** | 10.0.1.230 | Central hub: Docker/system services for Mosquitto, TimescaleDB/PostgreSQL, Telegraf, Grafana, ebusd, plus legacy InfluxDB coexistence, `z2m-hub` (:3030), and `adaptive-heating-mvp` (:3031) |

emonpi also runs `energy-hub-tesla.service` — a Python collector that polls the local Powerwall Gateway API every 10s and publishes raw metrics plus derived signals to MQTT as `emon/tesla/+`.

MQTT credentials: `emonpi` / `emonpimqtt2016`. Z2M WebSocket: `ws://emonpi:8080/api` (no auth).

## Data Path

Each emon host runs local Mosquitto, bridging to central Mosquitto on pi5data. PostgreSQL/TimescaleDB is the intended store for `heatpump-analysis`; legacy InfluxDB still coexists only so the final cutover can complete without changing field devices.

```
emonhp  ──bridge──┐
emondhw ──bridge──┼──> pi5data mosquitto → TSDB ingest/services → TimescaleDB/PostgreSQL (+ legacy Influx coexistence) → Grafana/controller
emonpi  ──bridge──┘           │
                              └── ebusd (Docker) → ebusd-poll.sh (systemd)
```

Telegraf subscribes to `emon/+/+` and other MQTT topics and feeds the shared ingest path on pi5data. `heatpump-analysis` is being cut over to PostgreSQL by command/config path, while any remaining Influx-only behaviour belongs in [[tsdb-migration]]. `z2m-hub` polls eBUS via TCP and MQTT for DHW tracking.

### Multical outage boundary and recovery

`multical` gaps can originate upstream on `emondhw`, not just in TimescaleDB migration plumbing.

The 2026-04-16 → 2026-04-23 DHW gap proved that if `emondhw` loses `/dev/ttyMULTICAL`, both PostgreSQL and legacy Influx stop receiving fresh `emon/multical/*` data. That outage window cannot be backfilled from local TSDB sources because the source data never reached either store. Treat any future stale `multical` window the same way unless another external archive is known to exist.

#### Multical stale-data checks

Use these checks in order when DHW data looks stale.

1. **Confirm stale PostgreSQL data on pi5data**
   ```bash
   ssh pi5data "docker exec timescaledb psql -U energy -d energy -Atc \"SELECT max(time) FROM multical;\""
   ```
2. **Check the source host**
   ```bash
   ssh pi@emondhw 'ls -l /dev/ttyMULTICAL'
   ssh pi@emondhw 'systemctl --no-pager --full status emonhub mosquitto'
   ssh pi@emondhw 'tail -50 /var/log/emonhub/emonhub.log'
   ```
3. **If the serial path is missing, inspect kernel evidence**
   ```bash
   ssh pi@emondhw 'dmesg | grep -Ei "ttyACM0|ttyMULTICAL|error -71|usb 1-1" | tail -n 80'
   ```

#### Multical recovery action

If `/dev/ttyMULTICAL` is missing and `emonhub` is logging `Not connected to modbus device` / `Could not find Modbus device`, reboot `emondhw` first.

```bash
ssh pi@emondhw 'sudo reboot'
sleep 35
ssh pi@emondhw 'ls -l /dev/ttyMULTICAL && tail -30 /var/log/emonhub/emonhub.log'
ssh pi5data "docker exec timescaledb psql -U energy -d energy -Atc \"SELECT max(time) FROM multical;\""
```

On the recovered system the QinHeng adapter should re-enumerate as `/dev/ttyACM0`, `/dev/ttyMULTICAL` should point back to it, `emonhub` should resume publishing `emon/multical/*`, and fresh `multical` rows should start appearing again in PostgreSQL.

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

11× SONOFF SNZB-02P (v2.2.0) indoor + 1 outdoor + 1 emonth2. Data: Z2M → MQTT → pi5data Telegraf → TSDB ingest, with PostgreSQL/TimescaleDB as the migration target and legacy Influx shadowing still in the tail.

12/13 rooms have dedicated sensors (Office + Landing added 24 Mar 2026). Conservatory uses `ebusd/poll/Z2RoomTemp` from the VRC 700 (tracks within 1°C of the former SNZB-02P). `outside_temp_humid` (0x842712fffe772723) paired 7 Apr 2026, deployed to shaded SE wall near VRC 700 OAT sensor. Zigbee routers (ZBMINI switches) at hall, landing, kitchen, top_landing provide mesh coverage for battery sensors.

### Outside Sensors

Temperature and humidity from separate sources.

- **Temperature** (real-time): `ebusd/poll/OutsideTemp` — VRC 700 OAT sensor on shaded SE wall (well-sited, no compressor or solar influence), 30s interval
- **Temperature** (cross-check): emoncms feed 503093 — Met Office hourly
- **Humidity** (live): `outside_temp_humid` SNZB-02P on shaded SE wall near OAT sensor. Paired 7 Apr 2026. Provides: (a) direct AH_out for absolute ACH in all occupied bedrooms, (b) before/after evidence for Elvina trickle vent closure. **Note**: link quality low (6) at initial pairing — monitor for dropouts. **Microclimate**: runs ~5–9°C warmer than ebusd OAT during afternoon (different wall position, less exposed to airflow) — these are two distinct microclimates. **AH analysis rule**: always use SNZB-02P's own (T, RH) pair together to compute AH_out — never combine ebusd OAT temperature with SNZB-02P humidity, as they measure different points.
- **Conservatory temperature**: `ebusd/poll/Z2RoomTemp` — VRC 700 Zone 2 room sensor, mounted in conservatory. Reads ~1°C below the former SNZB-02P position. Updated in `thermal_geometry.json`.
- **Leather humidity**: `emon/emonth2_23/humidity` — emonth2 in Leather. Provides 4th occupied-room data point for overnight moisture network (Parson Russell Terrier, ~10 g/h).

## Secrets

Secrets follow device class and trust boundary.

Pi/Linux services should hold stronger runtime secrets in systemd-managed credentials where practical, but the current `adaptive-heating-mvp` production deployment on pi5data uses a root-only environment file (`/etc/adaptive-heating-mvp.env`) for `TIMESCALEDB_CONNINFO` plus Octopus credentials. It no longer loads any Influx token or systemd Influx credential. Do not store secrets in TOML, pass them on command lines, or check them into the repo.

Dev/test may use one-shot `ak`-sourced environment injection on the trusted machine only, e.g. `PGPASSWORD=$(ak get timescaledb) ...` or `export TIMESCALEDB_CONNINFO=...`. This is a local operator convenience for verification, not a production secret-distribution mechanism. Legacy ad-hoc Influx access likewise uses `ak get influxdb` only while migration-tail diagnostics still exist. See `deploy/SECRETS.md`.

MCUs should prefer a gateway pattern via MQTT or a Pi-owned API and should not hold database or cloud secrets unless unavoidable. Any device that must access PostgreSQL, MQTT, or another backend directly gets its own least-privilege credential. Assume MCU secrets may be extractable, so use per-device rotation and revocation.

Many field devices already publish to Pi-side services over MQTT, so stronger secrets should stay on the Pi side.

Grafana on pi5data is part of the Docker monitoring stack rather than a native systemd service. Its admin bootstrap password must therefore come from a host-side secret file (`/etc/monitoring/grafana_admin_password`) referenced by the compose stack, not from a checked-in `admin` literal. If `/home/jack/monitoring/grafana` is recreated, Grafana will seed the admin user from that host secret on first start.

### Octopus API Credentials

Credentials for the `octopus-tariff` crate resolve in order: env vars → `~/.octopus-api-key` file → `~/github/octopus/.envrc` sourced via bash.

- `OCTOPUS_API_KEY` / `OCTOPUS_ACCOUNT_NUMBER` env vars take priority
- `~/.octopus-api-key` plain-text file (API key only, chmod 600) — used on emonpi where no `.envrc` is present
- `~/github/octopus/.envrc` sourced via bash — canonical store on dev machines, shared across `octopus`, `octopus-tariff`, and `heatpump-analysis`

On pi5data the Octopus env vars and `TIMESCALEDB_CONNINFO` are injected by the systemd `EnvironmentFile` at `/etc/adaptive-heating-mvp.env`.

### Ad-hoc PostgreSQL Queries from Dev Machine

Query TimescaleDB on pi5data from WSL using `TIMESCALEDB_CONNINFO`. PostgreSQL is the intended operator query surface for this repo.

See [[constraints#PostgreSQL-First Analysis]]: all filtering, aggregation, windowing, and arithmetic belong in SQL — client code is for formatting only.

```bash
# Good: SQL does the heavy lifting, client just prints
export TIMESCALEDB_CONNINFO='host=pi5data dbname=energy user=... password=...'
psql "$TIMESCALEDB_CONNINFO" -c "
  SELECT time, action, target_flow_c, leather_temp_c, outside_temp_c
  FROM adaptive_heating_mvp
  WHERE time >= now() - interval '12 hours'
  ORDER BY time;
"
```

Representative tables: `ebusd` (heat pump / VRC 700), `ebusd_poll` (numeric polled registers), `ebusd_poll_text` (string-valued polled registers), `zigbee` (room sensors), `adaptive_heating_mvp` (controller decisions), `multical` (heat meter + DHW meter fields), `dhw_inflection`, and `dhw_capacity`.

For DHW investigations, check `multical` freshness first. A stale `multical` table with current `ebusd` / `adaptive_heating_mvp` usually means an upstream `emondhw` source outage rather than a PostgreSQL-wide ingest failure.

If a verification task still requires raw Flux or profiler output, treat that as migration-tail work tracked in [[tsdb-migration]] rather than the default operator path.

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
| HwcTimer (all days) | 04:00;07:00;13:00;16:00;22:00;-:- | Three Cosy windows matching tariff. Runtime slot matching may still normalize imported evening tariff windows to `23:59` for same-day comparisons, but anything written to the VRC 700 must encode end-of-day as `-:-`. `sync_morning_dhw_timer` may rewrite one weekday to `13:00;16:00;22:00;-:-;-:-;-:-` when predicted T1 at 07:00 is ≥40°C. Dedup state cleared on write failure and on startup |
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
