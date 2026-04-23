# Heating Monitoring System Setup

This file is the operational deep-dive for the monitoring estate. The canonical current inventory, MQTT/data-path summary, eBUS stack, secrets pointer, and baseline VRC 700 settings live in [`lat.md/infrastructure.md`](lat.md/infrastructure.md).

## Use this file for

- operator-oriented host access and service locations
- backup and imaging procedure
- key config file paths on each host
- practical troubleshooting notes

## Do not use this file for

- current architecture truth
- controller policy
- live device inventory or baseline register values

For those, use:

- [`lat.md/infrastructure.md`](lat.md/infrastructure.md)
- [`lat.md/architecture.md`](lat.md/architecture.md)
- [`lat.md/heating-control.md`](lat.md/heating-control.md)
- [`lat.md/domain.md`](lat.md/domain.md)

## Monitoring overview

```text
Multical DHW meter -> emondhw --bridge--┐
Heat meter / SDM120 -> emonhp --bridge--┼-> pi5data mosquitto -> telegraf -> influxdb -> grafana
EmonPi2 / Zigbee -> emonpi --bridge-----┘                     |
Vaillant eBUS -> adapter -> ebusd on pi5data ----------------┘
```

Operationally, the key idea is local collection on each edge host and central storage/query on `pi5data`.

## Host access

| Host | Address | Primary role | SSH |
|---|---|---|---|
| `pi5data` | `10.0.1.230` | central hub: Mosquitto, InfluxDB, Telegraf, Grafana, ebusd, controller services | `ssh jack@pi5data` |
| `emonpi` | `10.0.1.117` | EmonPi2 CTs, DS18B20, Zigbee2MQTT | `ssh pi@emonpi` |
| `emonhp` | `10.0.1.169` | heat pump MBUS + SDM120 + emonth2 | `ssh pi@emonhp` |
| `emondhw` | `10.0.1.46` | Multical DHW meter + MQTT bridge | `ssh pi@emondhw` |

Device credentials for the emon hosts come from:

```bash
ak get emon-pi-credentials
```

## Fast operational map

### pi5data

- Docker stack root: `~/monitoring/`
- Main compose file: `~/monitoring/docker-compose.yml`
- Mosquitto config: `~/monitoring/mosquitto/config/mosquitto.conf`
- Telegraf config: `~/monitoring/telegraf/telegraf.conf`
- InfluxDB token for services: `/etc/adaptive-heating-mvp.env`
- eBUS poll script: `/usr/local/bin/ebusd-poll.sh`
- Services of interest:
  - Docker: mosquitto, influxdb, telegraf, grafana, ebusd
  - systemd: `adaptive-heating-mvp`, `ebusd-poll`, `z2m-hub`

Useful checks:

```bash
ssh jack@pi5data 'docker ps'
ssh jack@pi5data 'systemctl --no-pager --full status adaptive-heating-mvp ebusd-poll'
ssh jack@pi5data 'echo info | nc -w 2 localhost 8888 | head -1'
```

### emonpi

- emonhub config: `/etc/emonhub/emonhub.conf`
- Mosquitto bridge config: `/etc/mosquitto/conf.d/bridge.conf`
- Z2M config volume: `/home/pi/data/configuration.yaml`
- Boot config: `/boot/firmware/config.txt`

Checks:

```bash
ssh pi@emonpi 'systemctl --no-pager --full status emonhub mosquitto'
ssh pi@emonpi 'docker ps --format "table {{.Names}}\t{{.Status}}"'
```

### emonhp

- emonhub config: `/etc/emonhub/emonhub.conf`
- Mosquitto bridge config: `/etc/mosquitto/conf.d/bridge.conf`
- local emoncms (if still present): `http://10.0.1.169`

Checks:

```bash
ssh pi@emonhp 'systemctl --no-pager --full status emonhub mosquitto'
```

### emondhw

- emonhub config: `/etc/emonhub/emonhub.conf`
- Mosquitto bridge config: `/etc/mosquitto/conf.d/bridge.conf`
- stable Multical symlink: `/dev/ttyMULTICAL`
- udev rule: `/etc/udev/rules.d/99-multical.rules`

Checks:

```bash
ssh pi@emondhw 'systemctl --no-pager --full status emonhub mosquitto'
ssh pi@emondhw 'ls -l /dev/ttyMULTICAL'
```

## Data-path reminder

For the current authoritative topology, read [`lat.md/infrastructure.md#Data Path`](lat.md/infrastructure.md#data-path).

Operationally, the important rule is:

- each emon host publishes to local Mosquitto
- bridges send required topics to central Mosquitto on `pi5data`
- `energy-hub-timescaledb-ingest` on `pi5data` writes to PostgreSQL/TimescaleDB
- controller and thermal tooling use the PostgreSQL-backed path

Useful topic families to remember:

- `emon/multical/*` — DHW meter values from `emondhw`
- `emon/heatpump/*` — heat meter / electrical data from `emonhp`
- `ebusd/poll/*` — eBUS polling values from `pi5data`
- `zigbee2mqtt/*` — room-sensor and device payloads from `emonpi`

## eBUS operations

For current eBUS truth and safety rules, use:

- [`lat.md/infrastructure.md#eBUS Stack`](lat.md/infrastructure.md#ebus-stack)
- [`lat.md/constraints.md#eBUS Control Flow`](lat.md/constraints.md#ebus-control-flow)
- [`lat.md/constraints.md#eBUS Timer Encoding`](lat.md/constraints.md#ebus-timer-encoding)

Quick manual DHW boost from `pi5data` host:

```bash
echo 'write -c 700 HwcSFMode load' | nc -w 5 localhost 8888 | head -1
```

Reset if needed:

```bash
echo 'write -c 700 HwcSFMode auto' | nc -w 5 localhost 8888 | head -1
```

## Backup and imaging

`scripts/backup-sdcard.sh` is the source of truth for SD-card imaging behaviour.

### Standard pipeline

1. save partition table
2. `dd` disk to sparse image
3. PiShrink `-Za`
4. keep `.img.xz` plus partition dump together

### Typical usage

```bash
ssh -t pi5nvme 'tmux new -s backup "sudo backup-sdcard.sh /dev/sda ~/backups/images/device-$(date +%Y%m%d)"'
ssh pi5nvme 'tmux capture-pane -t backup -p'
```

### Restore

```bash
xz -dc device.img.xz | sudo dd of=/dev/sdX bs=4M status=progress
```

## Minimal troubleshooting

### No DHW Multical data

On `emondhw`, first verify the stable device path:

```bash
ssh pi@emondhw 'ls -l /dev/ttyMULTICAL'
ssh pi@emondhw 'tail -50 /var/log/emonhub/emonhub.log'
```

If the USB adapter changed identity, update `/etc/udev/rules.d/99-multical.rules`, reload rules, then restart emonhub.

### No eBUS reads on pi5data

```bash
ssh jack@pi5data 'docker ps | grep ebusd'
ssh jack@pi5data 'echo info | nc -w 2 localhost 8888'
ssh jack@pi5data 'systemctl status ebusd-poll --no-pager'
```

### Controller service unhealthy

```bash
ssh jack@pi5data 'systemctl status adaptive-heating-mvp --no-pager'
ssh jack@pi5data 'journalctl -u adaptive-heating-mvp -n 100 --no-pager'
```

### Zigbee2MQTT issues on emonpi

```bash
ssh pi@emonpi 'docker logs --tail 100 zigbee2mqtt'
ssh pi@emonpi 'ls -l /dev/ttyUSB0'
```

## Related documents

- [`docs/emon-installation-runbook.md`](docs/emon-installation-runbook.md) — rebuild/provisioning procedure
- [`deploy/SECRETS.md`](deploy/SECRETS.md) — service token handling
- [`docs/vrc700-settings-audit.md`](docs/vrc700-settings-audit.md) — timer audit trail and recovery context
- [`docs/pico-ebus-plan.md`](docs/pico-ebus-plan.md) — future replacement for the current eBUS adapter stack
