# emonpi Rebuild Status (2026-03-20)

## Scope
This note records the actual state after rebuilding `emonpi` from a clean SD image.

Related generic runbook:
- `docs/emon-installation-runbook.md`

---

## Host identity
- Hostname: `emonpi`
- IPs seen: `10.0.1.117` (eth0), `10.0.1.111` (wlan0)
- OS: Debian 12 (bookworm)

---

## Implemented

## 1) Fresh base
- Old SD card wiped/reflashed with fresh Raspberry Pi OS Lite (bookworm arm64).
- First-boot setup applied via `custom.toml` on boot partition (see `docs/emon-installation-runbook.md` section 8 for the correct method):
  - SSH enabled
  - hostname `emonpi`
  - Wi-Fi (C2, country GB)
  - UART + 1-wire overlays (added to config.txt post-boot)
  - `pi` user with key auth

## 2) Password management
- Password source of truth moved to local GPG/`ak` store:
  - service: `emon-pi-credentials`
- Password rotated and applied to emon hosts.

## 3) Installed components on emonpi
- mosquitto
- docker + docker-compose
- emonhub (from upstream git)
- emonPiLCD (from upstream git)
- required Python/system dependencies

## 4) Firmware
- AVR serial query confirms:
  - `firmware = emon_DB_6CT`
  - `version = 2.1.1`
  - `hardware = emonPi2`

## 5) emonhub config
Configured to ingest:
- EmonPi2 serial on `/dev/ttyAMA0`
- DS18B20 interfacer with:
  - `28-00000ee9cb6d -> temp_high`
  - `28-00000ee9e94f -> temp_low`

MQTT publish (local):
- `emon/EmonPi2/P1..P3` (plus other fields)
- `emon/sensors/temp_high|temp_low`

## 6) OneWire fix applied
Initial failure cause: wrong GPIO.

Working config now:
- `/boot/firmware/config.txt` contains `dtoverlay=w1-gpio,gpiopin=17`

Verified:
- `/sys/bus/w1/devices/` shows both sensor IDs
- temperature values readable
- emonhub publishes temp topics

## 7) LCD
- `emonPiLCD` service active.
- I2C display detected at address `0x3c` on bus 1.

## 8) Zigbee2MQTT
- Running in Docker
- Sonoff coordinator detected on `/dev/ttyUSB0`
- Adapter set explicitly (`zstack`)
- `homeassistant: false`
- 8 known devices restored (3 active: landing, hall, landing_motion; 5 need re-pairing)
- WebSocket API accessible at `ws://emonpi:8080/api` (no auth)

## 9) MQTT bridge
Final state:
- `emonpi` mosquitto bridge points to `pi5data:1883` (direct)
- Bridge config: `emon/#` out, `zigbee2mqtt/#` both (bidirectional)

Verified:
- `ping pi5data` from emonpi works
- TCP 1883 reachable from emonpi
- established mosquitto bridge socket from emonpi to pi5data

## 9a) Mosquitto network access (added 2026-03-21)
- Mosquitto now listens on `0.0.0.0:1883` (was localhost-only)
- Password auth enabled: user `emonpi`, pass `emonpimqtt2016`
- Config: `/etc/mosquitto/conf.d/network.conf`
- Password file: `/etc/mosquitto/passwd`
- Z2M already had credentials in its config — no change needed
- Verified: pi5data can publish commands directly to `zigbee2mqtt/<device>/set`

---

## Temporary workaround that was removed
During mid-build network instability, bridge was temporarily pointed to `emonhp` as relay.
This was reverted.

Current intended path is direct:
`emonpi -> pi5data`

---

## tmux policy (applied)
`tmux` installed on:
- emonpi
- emonhp
- emondhw

Used for long-running remote operations (apt/provisioning/docker pulls) to avoid SSH timeout loss.

---

## Backups
Backups saved under:
- `/home/jack/backups/emon-configs/20260320/` (on pi5data)

Includes configs, DB dumps, feed archives, and z2m data snapshot.

---

## 10) Grafana
- New `emonpi-live` dashboard created (Grid/House/Solar power, DS18B20, Zigbee temps/humidity, switches, voltage)
- Consistent colour standard applied across all dashboards (same physical measurement = same colour)
- InfluxDB `mqtt_consumer` measurement data migrated to `emon` (eliminated duplicate series in panels)
- Fixed garbled UTF-8 units on emonhp Flow Rate and Volume panels

---

## Final checklist (emonpi)
- [x] Fresh minimal OS
- [x] Hostname reused (`emonpi`)
- [x] SSH key auth working
- [x] Password managed via `ak` secret
- [x] emonhub active
- [x] AVR firmware current (2.1.1)
- [x] P1/P2/P3 publishing
- [x] DS18B20 both publishing
- [x] LCD service active
- [x] Zigbee2MQTT running + devices restored (3 of 8 active)
- [x] Direct MQTT bridge to pi5data active (bidirectional for zigbee2mqtt/#)
- [x] Mosquitto open on network (0.0.0.0:1883) with password auth
- [x] Data flowing to InfluxDB (verified)
- [x] Grafana dashboard created with colour standard
