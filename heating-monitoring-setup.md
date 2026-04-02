# Heating Monitoring System Setup

*Date: 2026-03-19, updated 2026-03-20 (emonpi rebuild, MQTT architecture, backups)*

## Overview

Central monitoring of a Vaillant heat pump system using eBUS, MBUS heat meter, Multical DHW meter, and emon ecosystem devices. Data collected on pi5data via MQTT → InfluxDB → Grafana.

```
┌─────────────┐    Modbus        ┌──────────────┐
│ Multical    │─────────────────│   emondhw    │
│ DHW meter   │  /dev/ttyACM0   │  10.0.1.46   │
│ (403W702UK) │                 │  emonhub     │    bridge
└─────────────┘                 │  dhw-trigger │────emon/#──────────┐
                                │  Mosquitto   │                    │
                                └──────────────┘                    │
                                                                      │
┌─────────────┐    MBUS/Modbus   ┌──────────────┐                    │
│ Heat Meter  │─────────────────│   emonhp      │    bridge          │
│ (Kamstrup)  │                 │  10.0.1.169   │────emon/#──────────┤
│ SDM120      │                 │  emonhub      │                    │
│ emonTh2     │                 │  Mosquitto    │                    │
└─────────────┘                 └──────────────┘                    │
                                                                      │
┌─────────────┐                 ┌──────────────┐    bridge           │
│ EmonPi2     │─── serial ──────│   emonpi      │────emon/#──────────┤
│ 3×CT + V    │  /dev/ttyAMA0   │  10.0.1.117   │────zigbee2mqtt/#──┤
│ DS18B20     │─── 1-wire ──────│  emonhub      │                    │
│ Zigbee      │─── USB ────────│  Z2M (Docker) │                    │
│  (8 devices)│  /dev/ttyUSB0   │  Mosquitto    │                    │
└─────────────┘                 └──────────────┘                    │
                                                                      ▼
┌─────────────┐     eBUS      ┌──────────────┐              ┌──────────────┐
│  Vaillant    │──────────────│  eBUS Adapter │──TCP:9999──│   pi5data    │
│  Heat Pump   │              │  10.0.1.41    │              │  10.0.1.230  │
│  (aroTHERM)  │              │  Shield v1.24 │              │  Mosquitto   │
└─────────────┘              └──────────────┘              │  ebusd (Docker)
                                                            │  ebusd-poll   │
                                                            │  Telegraf    │
                                                            │  InfluxDB    │
                                                            │  Grafana     │
                                                            └──────────────┘

DNS: all hostnames resolve via dnsmasq on router (10.0.0.1), domain chidley.home
DHCP: static reservations for emonpi, emonhp, emondhw, pi5data
```

## Devices

### pi5data (10.0.1.230) — Central Monitoring Hub
- **Hardware**: Raspberry Pi 5, 8GB RAM, NVMe storage
- **OS**: Debian Bookworm (aarch64), kernel 6.12
- **Role**: Central data collection, storage, visualisation
- **Services** (Docker Compose in `~/monitoring/`):
  - Mosquitto MQTT broker (port 1883)
  - InfluxDB 2 (port 8086)
  - Telegraf (MQTT → InfluxDB bridge)
  - Grafana (port 3000)
  - ebusd v26.1 (Docker, connects to eBUS adapter at 10.0.1.41:9999 over network)
  - ebusd-poll (polls 25+ eBUS values every 30s, publishes to local MQTT)
- **SSH**: `ssh jack@pi5data` or `ssh jack@10.0.1.230`

### emondhw (10.0.1.46) — DHW Metering
- **Hardware**: Raspberry Pi Zero 2 W (arm64, 426MB RAM)
- **OS**: Debian 12 Bookworm arm64 (clean minimal rebuild in progress)
- **Role**: DHW heat metering (Kamstrup Multical 403W702UK)
- **Services**:
  - **emonhub** — reads Multical heat meter via Modbus (`/dev/ttyACM0`, QinHeng CH34x USB serial, 19200 baud, even parity, address 8)
  - **Mosquitto** — local MQTT with bridge to pi5data (`emon/#`)
- **Note**: dhw-auto-trigger removed Mar 2026 — DHW boost now handled by z2m-hub on pi5data
- **Note**: ebusd moved to pi5data Docker (was previously on emondhw)
- **SSH**: `ssh pi@emondhw`
- **Credentials**: `ak get emon-pi-credentials` / Bitwarden "emon pi, pi credentials"
- **MQTT credentials**: `emonpi` / `emonpimqtt2016`

### emonhp (10.0.1.169) — Heat Pump Direct Monitoring
- **Hardware**: Raspberry Pi (armv7l), emonSD
- **OS**: Debian Buster, kernel 5.x
- **Role**: Direct heat pump monitoring via MBUS heat meter, SDM120 electric meter, RF sensors
- **Services**:
  - **emonhub** — reads:
    - MBUS heat meter via `/dev/ttyUSB0` (Prolific PL2303) — flow/return temps, power, energy, flow rate
    - SDM120 electric meter via `/dev/ttyACM0` (CH340) — electric power & energy
    - RFM69 SPI radio — emonth2 room sensor (temp, humidity)
  - **Mosquitto** — local MQTT with bridge to pi5data
  - **emoncms** — local web UI at http://10.0.1.169
  - Also publishes to **emoncms.org**
- **SSH**: `ssh pi@emonhp` or `ssh pi@10.0.1.169`
- **Emoncms login**: http://10.0.1.169 (user: jack)
- **emoncms.org**: https://emoncms.org/app/view?name=MyHeatpump&readkey=1b00410c57d5df343ede7c09e6aab34f

### eBUS Adapter Shield v1.24 (10.0.1.41)
- **Hardware**: ESP32-C3, PCB v1.24.4
- **Firmware**: ebusd-esp32 v5, build 6311 (updated 2026-03-17, from build 20241027)
- **MAC**: 80:65:99:9a:04:78
- **Hostname**: ebus-9a0478
- **WiFi**: SSID "C2"
- **Web UI**: http://10.0.1.41 (no auth)
- **eBUS mode**: Enhanced, TCP port 9999
- **MQTT**: Connected to emondhw.local, topic `emon/eas`
- **Config backup**: `C:\Users\jackc\tmp\ebus-backup\`

### emonpi (eth0 10.0.1.117, wlan0 10.0.1.111) — EmonPi2 Energy Monitor
- **Hardware**: Raspberry Pi 4B Rev 1.5 + EmonPi2 AVR-DB board
- **OS**: Debian 12 Bookworm (arm64), clean minimal install (rebuilt 2026-03-20)
- **MACs**: d8:3a:dd:9b:0a:21 (eth0), d8:3a:dd:9b:0a:22 (wlan0)
- **Firmware**: emon_DB_6CT v2.1.1 on `/dev/ttyAMA0`
- **Role**: 3-channel CT energy monitor, DS18B20 temperatures, Zigbee2MQTT gateway
- **CT channels**: P1=DNO grid (+import/−export), P2=House consumption, P3=Solar (P4–P6 unused)
- **DS18B20**: temp_high (`28-00000ee9cb6d`), temp_low (`28-00000ee9e94f`) — 1-wire on GPIO17, same space different heights
- **Zigbee2MQTT**: Docker container (v2.9.1), Sonoff USB 3.0 dongle on `/dev/ttyUSB0` (`zstack` adapter), **21 paired devices** (updated Mar 2026):
  - 12× SONOFF SNZB-02P temp/humidity (all rooms except leather which uses emonth2)
  - 4× SONOFF ZBMINI switches: hall, kitchen, landing, top_landing
  - 3× SONOFF S60ZBTPG smart plugs
  - 2× Aqara motion sensors (landing_motion, hall_motion)
  - **WebSocket API**: `ws://emonpi:8080/api` (no auth) — pushes all cached device state on connect
  - All SNZB-02P on firmware v2.2.0 (fixes v2.1.0 freeze bug)
- **Mosquitto**: listening on `0.0.0.0:1883` with password auth (user `emonpi`, pass `emonpimqtt2016`). Config in `/etc/mosquitto/conf.d/network.conf`.
- **Services**: emonhub, mosquitto, Docker (Z2M), emonPiLCD (I2C 0x3c)
- **SSH**: `ssh pi@emonpi`
- **Credentials**: `ak get emon-pi-credentials` / Bitwarden "emon pi, pi credentials"
- **MQTT bridge topics**: `emon/#` (out), `zigbee2mqtt/#` (both — bidirectional)
- **Note**: `cmdline.txt` must NOT contain `console=serial0,115200` — conflicts with EmonPi2 board on ttyAMA0
- **Note**: `/boot/firmware/config.txt` must have `dtoverlay=w1-gpio,gpiopin=17` for DS18B20

## Data Flow

### MQTT Topics

**emondhw → pi5data** (bridged):
| Topic | Source | Data |
|---|---|---|
| `emon/multical/dhw_t1` | Multical meter | T1 Cylinder Top — hot water outlet (°C). Kamstrup register 0x0006 "Temp. 1 Inlet" (inlet to meter = hot from cylinder) |
| `emon/multical/dhw_t2` | Multical meter | T2 Mains Inlet — cold water in, post-WWHR (°C). Kamstrup register 0x0008 "Temp. 2 Outlet" (cold reference) |
| `emon/multical/dhw_t1-t2` | Multical meter | Delta T (°C) |
| `emon/multical/dhw_flow` | Multical meter | Flow rate (l/h) |
| `emon/multical/dhw_power` | Multical meter | Thermal power (kW) |
| `emon/multical/dhw_E1` | Multical meter | Energy (MWh) |
| `emon/multical/dhw_heat_energy_A1` | Multical meter | Heat energy (MWh) |
| `emon/multical/dhw_volume_V1` | Multical meter | Volume (litres) |

**ebusd → pi5data Mosquitto** (direct, ebusd runs in Docker on pi5data):
| Topic | Source | Data |
|---|---|---|
| `ebusd/poll/*` | ebusd-poll.sh | 25+ heat pump values (see below) |

**emonpi → pi5data** (bridged):
| Topic | Source | Data |
|---|---|---|
| `emon/EmonPi2/P1` | EmonPi2 CT1 | DNO grid power (W, +import/−export) |
| `emon/EmonPi2/P2` | EmonPi2 CT2 | House consumption (W) |
| `emon/EmonPi2/P3` | EmonPi2 CT3 | Solar power (W, negative=generating) |
| `emon/sensors/temp_high` | DS18B20 | Temperature upper sensor (°C) |
| `emon/sensors/temp_low` | DS18B20 | Temperature lower sensor (°C) |
| `zigbee2mqtt/bathroom_temp_humid` | SNZB-02P | temperature, humidity, battery |
| `zigbee2mqtt/shower_temp_humid` | SNZB-02P | temperature, humidity, battery |
| `zigbee2mqtt/front_temp_humid` | SNZB-02P | temperature, humidity, battery |
| `zigbee2mqtt/conservatory_temp_humid` | SNZB-02P | temperature, humidity, battery |
| `zigbee2mqtt/hall` | ZBMINI | state (ON/OFF) |
| `zigbee2mqtt/kitchen` | ZBMINI | state (ON/OFF) |
| `zigbee2mqtt/landing` | ZBMINI | state (ON/OFF) |
| `zigbee2mqtt/landing_motion` | RTCGQ14LM | occupancy, illuminance, battery, temperature, motion_sensitivity |

**emonhp → pi5data** (bridged):
| Topic | Source | Data |
|---|---|---|
| `emon/heatpump/electric_Power` | SDM120 | Electric power (W) |
| `emon/heatpump/electric_Energy` | SDM120 | Electric energy (kWh) |
| `emon/heatpump/heatmeter_FlowT` | MBUS | Flow temperature (°C) |
| `emon/heatpump/heatmeter_ReturnT` | MBUS | Return temperature (°C) |
| `emon/heatpump/heatmeter_DeltaT` | MBUS | Delta T (°C) |
| `emon/heatpump/heatmeter_Power` | MBUS | Thermal power (W) |
| `emon/heatpump/heatmeter_Energy` | MBUS | Total heat energy (kWh) |
| `emon/heatpump/heatmeter_FlowRate` | MBUS | Flow rate (m³/h) |
| `emon/heatpump/heatmeter_Volume` | MBUS | Total volume (m³) |
| `emon/emonth2_23/temperature` | RFM69 | Room temperature (°C) |
| `emon/emonth2_23/humidity` | RFM69 | Room humidity (%) |

### eBUS Poll Script Data (`ebusd/poll/*`)

**Every 30 seconds** (fast tier):
| MQTT Field | Description | Unit |
|---|---|---|
| FlowTemp | HP flow temperature | °C |
| ReturnTemp | HP return temperature | °C |
| StatuscodeNum | Operating mode (numeric) | enum |
| Statuscode | Operating mode (text) | string |
| ElectricPower_W | Electric consumption | W |
| ConsumedPower_kW | Consumed power | kW |
| YieldPower_kW | Thermal yield power | kW |
| CompressorSpeed | Compressor speed | % |
| CompressorOutletTemp | Hot gas temperature | °C |
| CompressorInletTemp | Suction temperature | °C |
| TargetFlowTemp | Target flow setpoint | °C |
| AirInletTemp | Outdoor unit air inlet | °C |
| HighPressure | Refrigerant high pressure | bar |
| Fan1Speed | Fan speed | rpm |
| EEVPosition | Expansion valve position | steps |
| CircPumpPower | Circulation pump power | % |
| BuildingCircuitFlow | Heating circuit flow rate | l/h |
| EnergyIntegral | Control energy integral | °min |
| FlowPressure | System water pressure | bar |
| CompressorUtil | Compressor utilisation | % |
| OutsideTemp | Outside temperature | °C |
| Hc1FlowTemp | HC1 flow temperature | °C |
| Hc1FlowTempDesired | HC1 flow setpoint | °C |
| HwcStorageTemp | Cylinder Temp — VR 10 NTC in dry stat pocket, just above bottom coil. VRC 700 uses this for charging decisions (triggers at target minus CylinderChargeHyst). Reads lower-cylinder stratification zone, NOT cylinder top. | °C |
| Hc1PumpStatus | Heating pump on/off | 0/1 |

**Every 5 minutes** (slow tier, counter % 10 == 0):
| MQTT Field | Description | Unit |
|---|---|---|
| CopHc | Heating COP (lifetime) | ratio |
| CopHwc | DHW COP (lifetime) | ratio |
| CopHcMonth | Heating COP (this month) | ratio |
| CopHwcMonth | DHW COP (this month) | ratio |
| YieldHcDay | Heating yield today | kWh |
| YieldHwcDay | DHW yield today | kWh |
| TotalEnergyUsage | Total electric consumption | kWh |
| YieldHc | Total heating yield | kWh |
| YieldHwc | Total DHW yield | kWh |
| OutsideTempAvg | Filtered outside temp | °C |
| HwcTempDesired | DHW target temp | °C |
| HwcOpMode | DHW operating mode | string |
| OpMode | System operating mode | string |
| Z1DayTemp | Zone 1 day setpoint | °C |
| Hc1HeatCurve | Heating curve gradient | ratio |
| HwcMode | HMU DHW mode (eco/normal) | string |

### Statuscode Values (StatuscodeNum)
| Code | Meaning |
|---|---|
| 34 | Frost protection |
| 100 | Standby |
| 101-107 | Heating (shutdown/blocked/prerun/**active**/overrun) |
| 111-117 | Cooling cycle |
| 125 | Heating immersion heater |
| 132-137 | **DHW** (blocked/prerun/active/immersion/overrun). **⚠ Code 134 is unreliable** — appears during both off/frost standby AND active DHW. Use `BuildingCircuitFlow` for DHW detection. |
| 202 | Air purging |
| 240 | Compressor oil heating |
| 516 | **Defrost active** |
| 252-590 | Various faults |

Key: **104** = heating, **134** = off/frost standby (**⚠ also appears during DHW — unreliable for DHW detection; use BuildingCircuitFlow instead**), **100** = standby, **516** = defrost. See AGENTS.md for the definitive eBUS state classification using BuildingCircuitFlow.

### InfluxDB Buckets & Measurements

| Bucket | Content | Period |
|---|---|---|
| `energy` | Live data from all MQTT sources | 2026-03-19 → ongoing |
| `emonpi-apr2024` | Historical emonPi2 data (working card) | Apr 2024 → Nov 2024 |
| `emonpi-nov2024` | Historical emonPi2 data (dead card backup) | Nov 2024 → Mar 2026 |

**Bucket `energy` measurements** (taxonomy unified 2026-03-21):

| Measurement | Tags | Data Type | Source |
|---|---|---|---|
| `emon` | source, field | float | All emon devices via MQTT bridges + ebusd adapter stats |
| `ebusd_poll` | field | float | ebusd Docker on pi5data (direct to local Mosquitto) |
| `ebusd` | circuit, field | string (raw) | ebusd Docker on pi5data |
| `zigbee` | device | JSON (float + string) | emonpi Zigbee2MQTT via bridge |

`emon` source tags: `EmonPi2`, `sensors`, `heatpump`, `multical`, `emonth2_23`, `eas`, `metoffice`, `bridgecheck`

Note: Telegraf on pi5data subscribes only to its local Mosquitto. All emon data arrives via MQTT bridges. ebusd publishes directly (Docker on pi5data).

Legacy measurement names `mqtt_consumer` and `emonpi` were migrated to `emon` and deleted on 2026-03-21.

**Bucket `emonpi-apr2024` feeds:** MSG, V1, DNO, E1, DS18B20, Kitchen, Up_Sockets, Down_Sockets, Loft, E3, E5, E4, E6, House_Load, Solar, emonth2_temp, emonth2_humidity, emonth2_battery, E1_tx4, E2_tx4, Hob, E2
*Note: CT assignments were different in this period — feed names may not match current physical circuits*

**Bucket `emonpi-nov2024` feeds:** V1, DNO_Power (Ch1, +ve=import/-ve=export), House_Power (Ch2, always +ve), House_Energy, Solar_Energy, DNO_Energy
*Note: Feed names verified against data patterns — see Observations section*

## Credentials

### SSH Keys (all emon devices)
```
ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOQNUht02iWRxIgqz+Y3WCzEdj35mO3z7Zy3Wu4iDSvK silver_surface
ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFNMsODtVaUjEzqZHU63lvBxyfDUyl8oYgNwhe7BRJUq jack@chidley.org
ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAID28fJRm+b8dcQuYr+Kf8RVNzf2BlGtfDRZFuUZg2quL jchidley
ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIIRr6a2wlHtijLayhRcwZPb7ZlSaOpccTiC/Om6m+rER jack@chidley.org
ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBNKLmlEGVdeH7qjZbVvWXDMMP8aj6kqZR7fh2XLD5wjRt5tyU888yVWg9b+YRuO3Q9Csk0MJx6sOecDHn5WWCPU= putty-ecdsa-key-20231106
```

### Service Credentials
| Service | Username | Password |
|---|---|---|
| Grafana (pi5data:3000) | admin | admin (change on first login) |
| InfluxDB (pi5data:8086) | admin | emonpimqtt2016 |
| MQTT (all devices) | emonpi | emonpimqtt2016 |
| MQTT (telegraf→mosquitto) | telegraf | telegraf2024 |
| emonhp emoncms web | jack | (set by user) |
| emon devices SSH (all) | pi | `ak get emon-pi-credentials` (Bitwarden: "emon pi, pi credentials") |

### InfluxDB API Token
Stored in `~/monitoring/telegraf/telegraf.conf` on pi5data. View with:
```bash
ssh jack@pi5data "grep token ~/monitoring/telegraf/telegraf.conf"
```

## Grafana Dashboards

| Dashboard | URL | Content |
|---|---|---|
| emonpi Live | /d/emonpi-live | Grid/House/Solar power, DS18B20 temps, Zigbee temps/humidity, light switches, mains voltage |
| DHW Hot Water | /d/dhw-hotwater | T1/T2 temps, delta T, flow, power, volume, energy |
| Heat Pump eBUS | /d/hp-ebus | Flow/return/target temps, electric/thermal power, outside/air temps, compressor speed |
| emonhp Heat Pump | /d/emonhp | MBUS thermal vs electric power, flow/return temps, flow rate, delta T, room temp |
| emonPi2 Historical (Apr-Nov 2024) | /d/emonpi-apr2024 | V1, Kitchen, Up/Down Sockets, Loft, Hob, Solar, DNO, energy accumulators, emonth2 |
| emonPi2 Historical (Nov 2024-Mar 2026) | /d/emonpi-nov2024 | DNO power, House consumption, V1, energy accumulators (DNO/House/Solar) |

Base URL: http://pi5data:3000

### Colour Standard
All dashboards use consistent colours for the same physical measurement, regardless of data source:

| Measurement | Colour | Hex | Examples |
|---|---|---|---|
| Flow temperature | Red | `#E02F44` | heatmeter_FlowT, FlowTemp, dhw_t1 |
| Return temperature | Blue | `#3274D9` | heatmeter_ReturnT, ReturnTemp, dhw_t2 |
| Target/setpoint | Orange | `#FF9830` | TargetFlowTemp, Hc1FlowTempDesired |
| Outside temperature | Purple | `#8F3BB8` | OutsideTemp |
| Delta T | Green | `#56A64B` | heatmeter_DeltaT, dhw_t1-t2 |
| DHW/cylinder temp | Dark red | `#C4162A` | HwcStorageTemp |
| Electric power | Yellow | `#FADE2A` | electric_Power, ElectricPower_W, P1 (grid) |
| Thermal power | Green | `#73BF69` | heatmeter_Power, YieldPower_kW, P3 (solar) |
| House consumption | Orange | `#FF9830` | P2 |
| Flow rate | Cyan | `#33B5E5` | heatmeter_FlowRate, BuildingCircuitFlow, dhw_flow |
| Humidity | Light blue | `#5794F2` | humidity |

**Datasources:**
| Name | UID | Bucket |
|---|---|---|
| InfluxDB | efgj42nd95728f | energy |
| InfluxDB-Apr2024 | dfgl5v0jox5vka | emonpi-apr2024 |
| InfluxDB-Nov2024 | cfgl6j8ps3v28f | emonpi-nov2024 |

## Backup & Image Strategy

### Pipeline: dd → PiShrink → xz
SD card backups use a **single read pass** to image the card, then optimize the **image file** (not the SD card) with PiShrink and `xz`. PiShrink auto-expands on first boot when restored.

**Tools required** (install on imaging host, e.g. pi5nvme):
```bash
sudo apt install xz-utils
wget -q https://raw.githubusercontent.com/Drewsif/PiShrink/master/pishrink.sh \
  -O /usr/local/bin/pishrink.sh && chmod +x /usr/local/bin/pishrink.sh
```

**Creating a backup** (use `scripts/backup-sdcard.sh`):
```bash
# On the imaging host (e.g. pi5nvme), run inside tmux so the job
# survives SSH disconnects. The full pipeline takes ~10 minutes.
ssh -t pi5nvme 'tmux new -s backup "sudo backup-sdcard.sh /dev/sda ~/backups/images/devicename-$(date +%Y%m%d)"'

# Or if already SSH'd in:
tmux new -s backup 'sudo backup-sdcard.sh /dev/sda ~/backups/images/devicename-$(date +%Y%m%d)'

# Check progress from another session:
ssh pi5nvme 'tmux capture-pane -t backup -p'

# Produces: devicename.img.xz + devicename.partition-table.dump
```

`scripts/backup-sdcard.sh` is the **source of truth** for imaging behavior.

Current pipeline (3 steps):
1. Save partition table (`sfdisk -d`)
2. `dd` full disk to sparse file
3. PiShrink `-Za` (shrink + free-space zeroing + xz compress, multithreaded)

**Restoring to SD card:**
```bash
xz -dc ~/backups/devicename.img.xz | sudo dd of=/dev/sdX bs=4M status=progress
# Partition auto-expands on first boot (PiShrink feature)
```

**Restoring to sparse file for mounting/inspection:**
```bash
xz -dc ~/backups/devicename.img.xz | dd of=devicename.img bs=4M conv=sparse
# Then mount:
LOOP=$(sudo losetup -fP --show devicename.img)
sudo mount /dev/${LOOP##*/}p2 /mnt/rootfs
sudo mount /dev/${LOOP##*/}p1 /mnt/boot
# When done:
sudo umount /mnt/rootfs /mnt/boot
sudo losetup -d $LOOP
rm devicename.img
```

⚠️ When copying sparse files use `rsync --sparse` or `cp --sparse=always`. `scp` expands them to full size.

### Why no pre-zeroing on SD cards?
For SD cards, writing zeros to free space before imaging is usually counterproductive:

- extra write wear on flash media,
- extra time (full additional write pass),
- redundant with PiShrink's own image-level optimization.

Use `dd → PiShrink → xz` and let PiShrink handle free-space zeroing on the loop-mounted image file.

### Compression comparison (29GB emondhw image)
| Method | Size | Notes |
|---|---|---|
| Raw dd image | 29 GB | |
| PiShrink only | 5.2 GB | Shrinks last partition |
| PiShrink + xz | **1.3 GB** | Best for archival, auto-expands on boot |

### Backup Script
`scripts/backup-sdcard.sh` in this repo automates the full pipeline:
```bash
sudo ./scripts/backup-sdcard.sh /dev/sda /path/to/output-name
```

### Current Backups

**pi5data** (`/home/jack/backups/emon-configs/20260320/`):
| File | Size | Content |
|---|---|---|
| `emonpi/emonpi-configs.tar.gz` | 27K | emonhub, mosquitto, emoncms, boot configs |
| `emonpi/emonpi-emoncms-db.sql.gz` | 7K | MySQL database dump |
| `emonpi/emonpi-phpfina.tar.gz` | 13M | Feed timeseries (44 files) |
| `emonpi/emonpi-z2m.tar.gz` | 13K | Z2M config, database, coordinator backup |
| `emonhp/emonhp-configs.tar.gz` | 27K | emonhub, mosquitto, emoncms, boot configs |
| `emonhp/emonhp-emoncms-db.sql.gz` | 6K | MySQL database dump |
| `emonhp/emonhp-emonhub.conf` | 5K | Full emonhub config |
| `emondhw/emondhw-configs.tar.gz` | 28K | emonhub, mosquitto, emoncms, boot configs |
| `emondhw/emondhw-emoncms-db.sql.gz` | 3K | MySQL database dump |
| `emondhw/emondhw-phpfina.tar.gz` | 1.5M | Feed timeseries |
| `emondhw/emondhw-extras.tar.gz` | 3K | ebusd config, dhw-auto-trigger |
| `pi5data-configs.tar.gz` | 2K | docker-compose, telegraf, mosquitto configs |

**pi5data** (`~/backup/`):
| File | Size | Content |
|---|---|---|
| `oem_heat_backup_complete.img.xz` | 850MB | emonhp full SD card (PiShrink + xz) |

**pi5nvme** (`~/backups/old-emonpi-sdcard/`):
| File | Size | Content |
|---|---|---|
| `old-emonpi.img.xz` | 1.6GB | Old emonpi SD card (PiShrink + xz, auto-expands) |
| `partition-table.dump` | 257B | sfdisk partition table dump |

## Key Files

### pi5nvme (backup host)
| Path | Purpose |
|---|---|
| `~/emonpi-backup/emonhub.conf` | emonpi emonhub config (from dead card) |
| `~/emonpi-backup/settings.ini` | emoncms DB credentials (from dead card) |
| `~/emonpi-backup/config.txt` | Boot config (from dead card) |
| `~/emonpi-backup/phpfina/` | Old emonpi phpfina data (7 feeds, Nov 2024 → Mar 2026) |
| `~/emonpi-backup/emonpi-phpfina-backup.tar.gz` | Working card phpfina backup (22 feeds, 17MB compressed) |
| `~/emonpi-backup/emonpi-feeds.json` | Feed ID → name mapping from working card |

### pi5data
| Path | Purpose |
|---|---|
| `~/monitoring/docker-compose.yml` | Docker stack definition |
| `~/monitoring/mosquitto/config/mosquitto.conf` | Mosquitto config |
| `~/monitoring/mosquitto/config/passwd` | Mosquitto password file |
| `~/monitoring/telegraf/telegraf.conf` | Telegraf MQTT→InfluxDB config |
| `~/monitoring/influxdb/` | InfluxDB data directory |
| `~/monitoring/grafana/` | Grafana data directory |

### emonpi
| Path | Purpose |
|---|---|
| `/etc/emonhub/emonhub.conf` | emonhub config (EmonPi2 serial, DS18B20) |
| `/etc/mosquitto/conf.d/bridge.conf` | MQTT bridge to pi5data (`emon/#`, `zigbee2mqtt/+`) |
| `/home/pi/data/configuration.yaml` | Zigbee2MQTT config (Z2M Docker volume) |
| `/boot/firmware/config.txt` | Boot config (UART, 1-wire overlays) |

### emondhw
| Path | Purpose |
|---|---|
| `/etc/emonhub/emonhub.conf` | emonhub config (Multical 403 via MinimalModbus) |
| `/etc/mosquitto/conf.d/bridge.conf` | MQTT bridge to pi5data (`emon/#` only) |
| *(dhw-auto-trigger removed Mar 2026 — replaced by z2m-hub on pi5data)* | |

### pi5data (ebusd)
| Path | Purpose |
|---|---|
| `~/monitoring/docker-compose.yml` | Docker stack (includes ebusd + ebusd-poll containers) |
| `/usr/local/bin/ebusd-poll.sh` | eBUS polling script (systemd service on host, replaced Docker Python version) |

### emonhp
| Path | Purpose |
|---|---|
| `/etc/emonhub/emonhub.conf` | emonhub config (MBUS, SDM120, RFM69, DS18B20) |
| `/etc/mosquitto/conf.d/bridge.conf` | MQTT bridge to pi5data |

## Physical Setup Notes

### DHW Metering (Multical on emondhw)
- **T1** (`dhw_t1`) = hot water outlet, cylinder top (1580mm). Kamstrup register 0x0006 "Temp. 1 Inlet" (inlet to the meter = hot from cylinder). ~42°C when charged.
- **T2** (`dhw_t2`) = cold water inlet, after shower WWHR (540mm). Kamstrup register 0x0008 "Temp. 2 Outlet" (cold reference). ~24°C.
- Measures heat drawn from cylinder by DHW usage (not heat added by HP)
- WWHR pre-heats mains cold from ~10°C to ~29°C before the cylinder
- **Note:** Kamstrup naming is from the meter's perspective — "Inlet" = hot, "Outlet" = cold. Counterintuitive for a DHW application. See `docs/dhw-cylinder-analysis.md`.

### DHW Cylinder Temperature (VR 10 on VWZ AI)
- **HwcStorageTemp** (`ebusd/poll/HwcStorageTemp`) = VR 10 NTC probe in dry stat pocket, just above bottom coil (~600mm)
- This is what the VRC 700 uses for DHW charging decisions (target 45°C minus CylinderChargeHyst 5K = triggers at 40°C)
- Reads the stratification zone: ~42°C when fully charged, crashes to ~23°C after large draws (90L+)
- NOT the same as cylinder top temperature (T1) — can read 10°C lower due to stratification

### Heat Pump (Vaillant aroTHERM)
- 3 eBUS masters detected: 0x10, 0x71, 0x03
- HW ID: 0020184838 (Vaillant)
- Controller: VWZIO (0010031644)
- Zone 1 "HOUSE": day 21°C, night 19°C (revised 29 Mar 2026, was 17°C), heat curve 0.55
- DHW: target 45°C, eco mode (manual switch to normal in cold season), schedule 05:30-07:00 + 13:00-15:00 + 22:00-00:00 (Cosy-aligned, revised 29 Mar 2026)
- Typical DHW cycle: 30-45 minutes, starts at ~36-38°C return, reaches 53-55°C flow

## eBUS DHW Boost Command

The `HwcSFMode` (Hot Water Cylinder Special Function Mode) on the VRC 700 forces a one-off DHW cylinder charge:

```bash
# Force DHW charge (from pi5data host):
echo "write -c 700 HwcSFMode load" | nc -w 5 localhost 8888 | head -1

# Reset to auto (optional — HP resets automatically after charge):
echo "write -c 700 HwcSFMode auto" | nc -w 5 localhost 8888 | head -1
```

| Value | Effect |
|-------|--------|
| 0 = auto | Normal scheduled operation |
| 6 = load | **Force immediate DHW cylinder charge** |

z2m-hub uses this command for the DHW boost button on the mobile dashboard. The HP charges the cylinder to target temperature then returns to normal operation automatically.

## Setup History

System built 2026-03-19 to 2026-03-21. Full journal entries archived in git history (commit range for this file). Key outcomes captured in the current state of this document and in `docs/emon-installation-runbook.md`.

## emonhp vs eBUS — What Each Provides

### Overlap (both measure)
| Data | emonhp (MBUS/SDM120) | eBUS |
|---|---|---|
| Flow temp | heatmeter_FlowT | FlowTemp |
| Return temp | heatmeter_ReturnT | ReturnTemp |
| Flow rate | heatmeter_FlowRate (m³/h) | BuildingCircuitFlow (l/h) |
| Thermal power | heatmeter_Power (W) | YieldPower_kW |
| Electric power | electric_Power (SDM120, W) | ElectricPower_W |
| Total heat energy | heatmeter_Energy (kWh) | YieldHc + YieldHwc |
| Total electric energy | electric_Energy (kWh) | TotalEnergyUsage |

### Only emonhp
- **Independent calibrated energy metering** — MBUS + SDM120 are MID-certified meters, legal "truth" for energy accounting. The HP's own eBUS numbers are estimates.
- **Room temperature & humidity** — emonth2 wireless sensor
- **emoncms.org historical data** — years of history already there

### Only eBUS
- **Operating mode** (StatuscodeNum) — Heating (104) / DHW (134) / Standby (100) / Defrost (516)
- **Compressor** — speed, inlet/outlet temps, utilisation
- **Refrigerant circuit** — high pressure, EEV position
- **Fan speed** — outdoor unit operation
- **Energy integral** — internal control logic
- **COP values** — HP's own COP calculation (HC/HWC/monthly)
- **Target flow temp** — weather compensation demand
- **Heat curve & setpoints** — configuration
- **HWC storage temp** — cylinder temperature
- **Pump power %** — what the circulation pump is actually doing

### Summary
**emonhp** = independent auditor (calibrated meters — what actually happened).
**eBUS** = inside view (operating modes, control decisions, refrigerant circuit, fault detection).
Both needed: emonhp alone can't distinguish heating from DHW. eBUS alone can't give independently verified energy numbers.

## Observations

### Heating Filter
- Cleaned on 2026-03-19
- BuildingCircuitFlow at idle: **8.6 l/min before → 12.7 l/min after** (48% improvement)
- Both MBUS and eBUS confirm same flow rate (760 l/h vs 759 l/h)
- Expected benefits: lower pump energy, lower flow temps, better COP, shorter run times
- **Monitor**: if BuildingCircuitFlow drops below ~600 l/h (10 l/min) at idle, filter needs cleaning again

### DHW Heating Cycles (from emoncms.org historical data)
- Analysed 181 cycles over 90 days
- Start return temp: avg 37.3°C (min 31, max 44) — this is the cylinder temperature the HP sees at start of DHW cycle
- Correlates with Multical T2 (~29°C) which measures further upstream before the cylinder coil
- Typical cycle: 30-45 min, flow reaches 53-55°C
- Schedule: daily at ~05:15 (morning) and ~13:15 (afternoon)
- Last few days before setup: longer cycles (45-90 min) with lower max flow temps — filter blockage effect

### DHW Metering (Multical on emondhw)
- T1 (`dhw_t1`) = cylinder top / hot water outlet (1580mm height). ~42°C when charged.
- T2 (`dhw_t2`) = mains inlet / cold water in, post-WWHR (540mm height). ~24°C.
- HwcStorageTemp (eBUS) = VR 10 NTC in dry stat pocket, just above bottom coil (~600mm). VRC 700 charging trigger.
- Measures heat drawn from cylinder by DHW usage (not heat added by HP)
- WWHR pre-heats mains cold from ~10°C to ~29°C before the cylinder
- dhw_P1 and dhw_mass_m1 return 4294967296 (0xFFFFFFFF) — register read errors, those Modbus registers not valid for this Multical model

### Octopus Cosy Tariff Pattern (from Nov 2024 data)
- Verified in InfluxDB data for Feb 2026
- **4-7am**: 1.8-4.7kW importing — Cosy morning cheap slot (heat pump pre-heating)
- **1-4pm**: 1.5-6.9kW importing — Cosy afternoon cheap slot
- **10pm-midnight**: 5.6-6.4kW importing — scheduled DHW or additional cheap period
- **Off-peak hours**: near zero (-7 to +50W) — house barely importing
- DNO power convention: **positive = importing from grid, negative = exporting to grid**

### emonPi2 CT Channel Assignments (Nov 2024 onwards)
- **Ch1** = DNO grid (positive=import, negative=export)
- **Ch2** = House consumption (always positive)
- **Ch3** = Solar (energy accumulator)
- *Note: Apr-Jun 2024 had different CT assignments — feeds were rearranged in Nov 2024*


## Outstanding TODO

- [ ] Rebuild emonhp from clean minimal install
- [ ] Set up InfluxDB retention policy / downsampling for long-term storage
- [ ] Set up Grafana alert for BuildingCircuitFlow < 600 l/h (filter cleaning reminder)
- [ ] Change Grafana admin password
