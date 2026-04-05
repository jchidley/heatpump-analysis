# Emon Rebuild / Minimal Install Runbook

This runbook covers rebuilds and reprovisioning for `emonpi`, `emonhp`, `emondhw`, and `pi5data`. For the current live topology and host roles, use [`../lat.md/infrastructure.md`](../lat.md/infrastructure.md).

## Principles

- keep installs minimal
- use local Mosquitto on each emon host with bridge-to-`pi5data`
- keep storage, dashboards, and controller logic centralized on `pi5data`
- store secrets outside git
- run long operations in `tmux`

## Canonical current-state references

Before changing a host, check:

- [`../lat.md/infrastructure.md`](../lat.md/infrastructure.md)
- [`../lat.md/constraints.md`](../lat.md/constraints.md)
- [`../deploy/SECRETS.md`](../deploy/SECRETS.md)

## Target architecture

```text
emonhp  ──bridge──┐
emondhw ──bridge──┼──> pi5data mosquitto -> telegraf -> influxdb -> grafana
emonpi  ──bridge──┘                │
                                   └── ebusd (Docker) -> ebusd-poll.sh (systemd)
```

## Host summaries

### emonpi

Purpose: EmonPi2 serial ingest, DS18B20, Zigbee2MQTT.

Install/keep:
- `mosquitto`, `mosquitto-clients`
- `docker.io`, `docker-compose`
- `emonhub`
- `emonPiLCD`

Important config/files:
- `/etc/emonhub/emonhub.conf`
- `/etc/mosquitto/conf.d/bridge.conf`
- `/home/pi/data/configuration.yaml`
- `/boot/firmware/config.txt`

Operational notes:
- bridge `emon/#` and the required Zigbee topics to `pi5data`
- keep `console=serial0,115200` out of `cmdline.txt` so the EmonPi2 board can use `ttyAMA0`
- restore the DS18B20 overlay after first boot

Required checks:
- `emonhub` active
- `mosquitto` active
- Zigbee2MQTT container healthy
- CT data and DS18B20 data visible in InfluxDB on `pi5data`

### emonhp

Purpose: heat meter, SDM120, emonth2, bridge to `pi5data`, and emoncms.org feed updates.

Install/keep:
- `mosquitto`, `mosquitto-clients`
- `emonhub`

Prefer not to run local extras unless needed:
- no local dashboard stack if not actively used
- no Docker unless a concrete need appears

Required checks:
- `emonhub` active
- `mosquitto` active
- `emon/heatpump/*` data reaching `pi5data`
- emoncms.org feed timestamps advancing

### emondhw

Purpose: Multical DHW meter + MQTT bridge.

Install/keep:
- `mosquitto`, `mosquitto-clients`, `netcat-openbsd`, `tmux`, `git`
- `emonhub`
- Python packages needed by the Multical interfacer

Important config/files:
- `/etc/emonhub/emonhub.conf`
- `/etc/mosquitto/conf.d/bridge.conf`
- `/etc/udev/rules.d/99-multical.rules`
- `/dev/ttyMULTICAL`

Critical operational rules:
- always use a stable udev symlink for the Multical USB adapter
- keep the node minimal; no local dashboard stack and no ebusd here

Example udev rule workflow:

```bash
udevadm info /dev/ttyACM0 | grep -E 'ID_SERIAL_SHORT|ID_VENDOR_ID|ID_MODEL_ID'
```

Then create/update:

```bash
sudo tee /etc/udev/rules.d/99-multical.rules <<'EOF'
SUBSYSTEM=="tty", ATTRS{idVendor}=="1a86", ATTRS{idProduct}=="55d3", ATTRS{serial}=="586D012855", SYMLINK+="ttyMULTICAL"
EOF
sudo udevadm control --reload-rules
sudo udevadm trigger
```

Required checks:
- `ls -l /dev/ttyMULTICAL`
- `systemctl status emonhub mosquitto`
- `tail -50 /var/log/emonhub/emonhub.log`

### pi5data

Purpose: central broker, storage, dashboards, ebusd, polling, controller services.

Docker services:
- mosquitto
- influxdb
- telegraf
- grafana
- ebusd

Systemd services of interest:
- `ebusd-poll`
- `adaptive-heating-mvp`
- `z2m-hub`

Important files:
- `~/monitoring/docker-compose.yml`
- `~/monitoring/telegraf/telegraf.conf`
- `/usr/local/bin/ebusd-poll.sh`
- `/etc/adaptive-heating-mvp.env`

Required checks:

```bash
docker ps
echo info | nc -w 2 localhost 8888
systemctl status ebusd-poll adaptive-heating-mvp --no-pager
```

## Secrets policy

Never commit secrets. Use:

```bash
ak get emon-pi-credentials
ak get influxdb
```

Production controller token lives in:

```text
/etc/adaptive-heating-mvp.env
```

See [`../deploy/SECRETS.md`](../deploy/SECRETS.md).

## tmux policy

Always run long or fragile host operations in `tmux`.

```bash
tmux new -s provision
```

Detached example:

```bash
tmux new-session -d -s provision 'bash /tmp/provision.sh 2>&1 | tee /tmp/provision.log'
```

## Backup before major changes

Before a rebuild or risky edit, capture at least:

- `/etc/emonhub/emonhub.conf`
- `/etc/mosquitto/`
- Zigbee2MQTT data dir on `emonpi`
- any custom systemd units or scripts
- emoncms DB/feed storage if still in use on that host

For full SD-card imaging, use [`../heating-monitoring-setup.md`](../heating-monitoring-setup.md) and `scripts/backup-sdcard.sh`.

## Headless provisioning (Pi OS Lite)

Use `custom.toml` on the boot partition. This is the supported Bookworm-era method for hostname, user, SSH, Wi‑Fi, and timezone.

Minimal pattern:

```toml
config_version = 1

[system]
hostname = "emondhw"

[user]
name = "pi"
password = "your-password-here"
password_encrypted = false

[ssh]
enabled = true

[wlan]
ssid = "YourSSID"
password = "your-wifi-password"
password_encrypted = false
country = "GB"

[locale]
timezone = "Europe/London"
```

After first boot, add device-specific overlays only where needed.

### emonpi post-boot overlay reminder

```bash
sudo tee -a /boot/firmware/config.txt <<'EOF'

[all]
dtoverlay=w1-gpio,gpiopin=17
dtparam=i2c_arm=on
EOF
```

## Acceptance checklists

### emonpi

- hostname correct
- SSH key auth works
- `mosquitto` active
- `emonhub` active
- Zigbee2MQTT healthy
- CT and DS18B20 data visible in InfluxDB

### emonhp

- hostname correct
- SSH key auth works
- `mosquitto` active
- `emonhub` active
- heat pump telemetry visible in InfluxDB
- emoncms.org feed updates confirmed

### emondhw

- hostname correct
- SSH key auth works
- `/dev/ttyMULTICAL` resolves
- `mosquitto` active
- `emonhub` active
- `emon/multical/*` data visible in InfluxDB

### pi5data

- Docker services running
- ebusd reachable on localhost:8888
- `ebusd-poll` active
- bridged emon data arriving
- controller services healthy

## Troubleshooting shortcuts

### Multical stops reading

```bash
ls -l /dev/ttyMULTICAL
systemctl restart emonhub
tail -50 /var/log/emonhub/emonhub.log
```

### emonhub logfile permission error

```bash
sudo chown -R pi:pi /var/log/emonhub
sudo systemctl restart emonhub
```

### eBUS stack unhealthy on pi5data

```bash
docker ps | grep ebusd
echo info | nc -w 2 localhost 8888
systemctl status ebusd-poll --no-pager
```

## Related documents

- [`../heating-monitoring-setup.md`](../heating-monitoring-setup.md)
- [`vrc700-settings-audit.md`](vrc700-settings-audit.md)
- [`pico-ebus-plan.md`](pico-ebus-plan.md)
