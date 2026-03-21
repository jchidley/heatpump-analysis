# Emon Rebuild / Minimal Install Runbook

## Scope
This runbook covers rebuilding and standardizing these hosts:

- `emonpi` (3x CT + 2x DS18B20 + Zigbee2MQTT + LCD)
- `emonhp` (heat pump / MBUS / SDM120)
- `emondhw` (Multical DHW meter + bridge)
- `pi5data` (central MQTT + InfluxDB + Grafana + Telegraf + ebusd + automation scripts)

Goal: **minimal installs**, reproducible configs, centralized data flow, and safe credential handling.

---

## 1) Architecture (target)

Each emon host runs a **local Mosquitto** for local producers.
Each emon host bridges required topics to central Mosquitto on `pi5data`.

```text
emonhp  ──bridge──┐
emondhw ──bridge──┼──> pi5data mosquitto -> telegraf -> influxdb -> grafana
emonpi  ──bridge──┘                │
                                   ├── ebusd (Docker) -> ebusd-poll.sh (systemd)
                                   └── dhw-auto-trigger.sh (systemd)
```

Why:
- Local resilience on each device
- Central storage/query on pi5data
- Simpler Telegraf (single broker)

---

## 2) What we install (and why)

## `emonpi` (minimal but complete)

Installed:
- `mosquitto`, `mosquitto-clients` (local broker + bridge)
- `docker.io`, `docker-compose` (run Zigbee2MQTT)
- `emonhub` (serial ingest from emonPi2 AVR board)
- `emonPiLCD` (OLED/LCD + button handling)
- Python deps for emonhub/interfacers

Configured:
- emonhub OEM serial on `/dev/ttyAMA0`
- DS18B20 sensor IDs + names (`temp_high`, `temp_low`)
- MQTT publish to `emon/...`
- Bridge topics:
  - `emon/#`
  - `zigbee2mqtt/+`
- Zigbee2MQTT config with `homeassistant: false`
- **Known issue (March 2026)**: Z2M bridge is online but all 7 devices show lastSeen Nov 2024 — need re-pairing after rebuild

Not installed by default:
- Full local emoncms stack (unless explicitly needed)

## `emondhw` (minimal — Multical + bridge only)

After headless provisioning (section 8), SSH in and run the following.

### Package install
```bash
sudo apt-get update
sudo apt-get install -y mosquitto mosquitto-clients netcat-openbsd python3-pip python3-serial git tmux
sudo pip3 install --break-system-packages minimalmodbus configobj paho-mqtt pymodbus
```

### emonhub install
```bash
sudo git clone https://github.com/openenergymonitor/emonhub.git /opt/openenergymonitor/emonhub
sudo mkdir -p /etc/emonhub /var/log/emonhub
sudo chown -R pi:pi /var/log/emonhub
```

### emonhub systemd service
```bash
sudo tee /etc/systemd/system/emonhub.service << 'EOF'
[Unit]
Description=emonHub data multiplexer
After=network-online.target mosquitto.service
Wants=network-online.target

[Service]
Type=simple
ExecStart=/usr/bin/python3 /opt/openenergymonitor/emonhub/src/emonhub.py --config-file=/etc/emonhub/emonhub.conf --logfile=/var/log/emonhub/emonhub.log
User=pi
Restart=always
RestartSec=10

[Install]
WantedBy=multi-user.target
EOF
```

### emonhub.conf (Kamstrup Multical 403)
```bash
sudo tee /etc/emonhub/emonhub.conf << 'EOF'
[hub]
    loglevel = DEBUG
    autoconf = 0

[interfacers]
    [[multical]]
        Type = EmonHubMinimalModbusInterfacer
        [[[init_settings]]]
            device = /dev/ttyACM0
            baud = 19200
            parity = even
            datatype = float
        [[[runtimesettings]]]
            pubchannels = ToEmonCMS,
            read_interval = 2
            nodename = multical
            [[[[meters]]]]
                [[[[[dhw]]]]]
                    device_type = MULTICAL
                    address = 8
                    registers = 0,4,6,8,14,16,20,48,52,72,78
                    names = flow, power, t1, t2, t1-t2, P1, E1, heat_energy_A1, volume_V1, QP_average_time, mass_m1
                    scales = 1,1,1,1,1,1,1,1,1000,1,1

    [[MQTT]]
        Type = EmonHubMqttInterfacer
        [[[init_settings]]]
            mqtt_host = 127.0.0.1
            mqtt_port = 1883
            mqtt_user = emonpi
            mqtt_passwd = emonpimqtt2016
        [[[runtimesettings]]]
            pubchannels = ToRFM12,
            subchannels = ToEmonCMS,
            node_format_enable = 0
            nodevar_format_enable = 1
            nodevar_format_basetopic = emon/
            node_JSON_enable = 0

[nodes]
EOF
```

### Mosquitto (password + bridge to pi5data)
```bash
sudo mosquitto_passwd -b -c /etc/mosquitto/passwd emonpi emonpimqtt2016

sudo tee /etc/mosquitto/conf.d/bridge.conf << 'EOF'
connection pi5data
address pi5data:1883
remote_username emonpi
remote_password emonpimqtt2016
topic emon/# out 0
bridge_protocol_version mqttv311
start_type automatic
restart_timeout 10
EOF

sudo tee -a /etc/mosquitto/mosquitto.conf << 'EOF'
allow_anonymous false
password_file /etc/mosquitto/passwd
listener 1883
EOF
```

### Enable and start
```bash
sudo systemctl daemon-reload
sudo systemctl restart mosquitto
sudo systemctl enable --now emonhub
```

### Verify
```bash
systemctl is-active mosquitto emonhub     # both should say "active"
tail -10 /var/log/emonhub/emonhub.log     # should show emon/multical/dhw_* publishing
```

### Not installed (by design)
- ebusd — runs on pi5data Docker, not emondhw
- emoncms — not needed, data goes to InfluxDB via bridge
- Docker — Pi Zero 2 W has only 426MB RAM

## `emonhp` (minimal — sensors + bridge only)

Runs emonhub (MBUS heat meter, SDM120 electric meter, RFM69 room sensor, DS18B20) + mosquitto bridge. Sends to emoncms.org via emonhub HTTP interfacer.

**Local emoncms stack disabled** (March 2026): Apache, MariaDB, Redis, feedwriter, emoncms_mqtt, emoncms_sync, service-runner, Docker, ShellHub — all disabled. Were consuming ~250MB RAM for unused local dashboards. Data flows via emonhub HTTP interfacer (emoncms.org) and MQTT bridge (pi5data) — neither depends on local emoncms.

Active services: `emonhub`, `mosquitto` only.

Ensure:
- local mosquitto active
- bridge config present and using hostname `pi5data` (not hardcoded IP)
- emonhub HTTP interfacer sending to emoncms.org (check feed timestamps)

## `pi5data` (central hub)

Docker services (`~/monitoring/docker-compose.yml`):
- `mosquitto` — central MQTT broker, receives bridges from all emon devices
- `influxdb` — time-series storage
- `grafana` — dashboards
- `telegraf` — MQTT → InfluxDB
- `ebusd` — eBUS daemon, connects to adapter at 10.0.1.41:9999, port 8888 exposed to host

Systemd services (shell scripts on host):
- `ebusd-poll` — reads 25+ eBUS values every 30s via `nc localhost 8888`, publishes to `ebusd/poll/*` MQTT topics
- `dhw-auto-trigger` — watches `emon/multical/dhw_flow` via `mosquitto_sub`, triggers DHW charge via eBUS on sustained draw

Host packages: `mosquitto-clients`, `netcat-openbsd`, `tmux`

### Deploy scripts
```bash
# ebusd-poll
scp scripts/ebusd-poll.sh jack@pi5data:/tmp/
scp scripts/ebusd-poll.service jack@pi5data:/tmp/
ssh jack@pi5data "sudo cp /tmp/ebusd-poll.sh /usr/local/bin/ && sudo chmod +x /usr/local/bin/ebusd-poll.sh && \
  sudo cp /tmp/ebusd-poll.service /etc/systemd/system/ && \
  sudo systemctl daemon-reload && sudo systemctl enable --now ebusd-poll"

# dhw-auto-trigger
scp scripts/dhw-auto-trigger.sh jack@pi5data:/tmp/
scp scripts/dhw-auto-trigger.service jack@pi5data:/tmp/
ssh jack@pi5data "sudo cp /tmp/dhw-auto-trigger.sh /usr/local/bin/ && sudo chmod +x /usr/local/bin/dhw-auto-trigger.sh && \
  sudo cp /tmp/dhw-auto-trigger.service /etc/systemd/system/ && \
  sudo systemctl daemon-reload && sudo systemctl enable --now dhw-auto-trigger"
```

---

## 3) Repository/submodules used

In this repo we keep OEM upstream sources as submodules for reproducibility:

- `EmonScripts`
- `emonhub`
- `emonPiLCD`
- `avrdb_firmware`
- `emoncms` (reference / optional local stack)

Use these as pinned sources for install and firmware ops.

---

## 4) Password and secret management

## Rule
Never store plaintext credentials in repo files.

## Store
Use local GPG-backed `ak` store.

Service name used:
- `emon-pi-credentials`

Example:
```bash
ak get emon-pi-credentials
```

Apply to host:
```bash
NEWPASS="$(ak get emon-pi-credentials)"
echo "pi:${NEWPASS}" | sudo chpasswd
```

---

## 5) tmux policy for all emon operations

Always run long/fragile tasks in remote tmux (apt, cloning, flashing, provision scripts).

On remote host:
```bash
tmux new -s provision
# run long command
# detach: Ctrl-b d
# reattach later:
tmux attach -t provision
```

Run script detached:
```bash
tmux new-session -d -s provision 'bash /tmp/provision.sh 2>&1 | tee /tmp/provision.log'
```

Why:
- SSH disconnect safe
- no timeout loss
- logs preserved

---

## 6) Backup policy before major changes

Backup destination: `pi5data`.

Minimum backup set per host:
- `/etc/emonhub/emonhub.conf`
- `/etc/mosquitto/` (incl bridge conf)
- Zigbee2MQTT data dir (`/home/pi/data` on emonpi)
- any custom scripts/services (e.g. dhw trigger)
- emoncms DB + feed storage if present

---

## 7) Imaging strategy (dd → PiShrink → xz)

### Backup script

`scripts/backup-sdcard.sh` automates the full pipeline. Prerequisites: `pishrink.sh` and `xz` in PATH.

```bash
# Run inside tmux so the job survives SSH disconnects (~10 min):
ssh -t pi5nvme 'tmux new -s backup "sudo backup-sdcard.sh /dev/sda ~/backups/images/devicename-$(date +%Y%m%d)"'

# Check progress from another session:
ssh pi5nvme 'tmux capture-pane -t backup -p'

# Produces: devicename.img.xz + devicename.partition-table.dump
```

`scripts/backup-sdcard.sh` is the **source of truth** for imaging behavior.

Current pipeline (3 steps):
1. **Save partition table** (`sfdisk -d`)
2. **dd full disk** to sparse file
3. **PiShrink `-Za`** — shrink + free-space zeroing + xz compress (multithreaded)

Typical results: 15GB → 1.6GB, 30GB → 1.3GB.

### Restore

```bash
xz -dc devicename.img.xz | sudo dd of=/dev/sdX bs=4M status=progress
# Partition auto-expands on first boot (PiShrink feature)
```

### Worked example: backup emondhw before rebuild

```bash
# 1. Shut down emondhw
ssh pi@emondhw 'sudo shutdown -h now'

# 2. Move SD card from Pi Zero 2 W to pi5nvme USB reader

# 3. Run backup inside tmux (survives SSH drops, ~10 min)
ssh -t pi5nvme 'tmux new -s backup "sudo backup-sdcard.sh /dev/sda ~/backups/images/old-emondhw-sdcard/old-emondhw"'

# 4. Verify
ssh pi5nvme 'ls -lh ~/backups/images/old-emondhw-sdcard/'
# old-emondhw.img.xz              (compressed image)
# old-emondhw.partition-table.dump (partition layout)

# 5. Flash new image onto the same card for rebuild
xz -dc /tmp/pios-lite.img.xz | sudo dd of=/dev/sda bs=4M conv=fsync status=progress
# Then mount boot partition and add custom.toml (see section 8)
```

---

## 8) Headless SD card provisioning (Pi OS Lite Bookworm arm64)

### Method: `custom.toml` on boot partition

This is the official Raspberry Pi Foundation method — it's what Raspberry Pi Imager generates internally. A `custom.toml` file on the boot partition is read by the firstboot system (`raspberrypi-sys-mods` package) which configures WiFi, user, SSH, hostname, and timezone, then reboots.

Pre-Bookworm methods (`wpa_supplicant.conf`, manual NM connection files, kernel params) do not work.

### Complete worked example (emondhw on Pi Zero 2 W)

```bash
# 1. Flash Pi OS Lite Bookworm arm64
xz -dc pios-lite.img.xz | sudo dd of=/dev/sda bs=4M conv=fsync status=progress

# 2. Mount boot partition
sudo partprobe /dev/sda
sudo mount /dev/sda1 /mnt/sdboot

# 3. Remove serial console (causes boot hang on Pi Zero 2 W,
#    conflicts with EmonPi2 serial on Pi 4B)
sudo sed -i 's/console=serial0,115200 //' /mnt/sdboot/cmdline.txt

# 4. Create custom.toml (the ONLY file you need to add)
sudo tee /mnt/sdboot/custom.toml << 'EOF'
config_version = 1

[system]
hostname = "emondhw"

[user]
name = "pi"
password = "your-password-here"
password_encrypted = false

[ssh]
enabled = true
authorized_keys = [
  "ssh-ed25519 AAAA... user@host",
]

[wlan]
ssid = "YourSSID"
password = "your-wifi-password"
password_encrypted = false
hidden = false
country = "GB"

[locale]
timezone = "Europe/London"
EOF

# 5. Unmount and sync
sync && sudo umount /mnt/sdboot

# 6. Move card to target Pi, power on, wait up to 5 minutes
#    Firstboot configures everything, reboots, then Pi is on WiFi with SSH
```

### `custom.toml` field reference

| Section | Key | Required | Notes |
|---|---|---|---|
| (root) | `config_version` | yes | Must be `1` |
| `[system]` | `hostname` | yes | Letters, digits, hyphens only |
| `[user]` | `name` | yes | Username (e.g. `pi`) |
| `[user]` | `password` | yes | Plaintext or hash |
| `[user]` | `password_encrypted` | yes | **Set `false` for plaintext.** Default is `true` (expects SHA-512 hash). |
| `[ssh]` | `enabled` | yes | `true` to enable SSH |
| `[ssh]` | `authorized_keys` | no | Array of public key strings |
| `[wlan]` | `ssid` | yes | WiFi network name |
| `[wlan]` | `password` | yes | Plaintext or PSK hex |
| `[wlan]` | `password_encrypted` | yes | **Set `false` for plaintext.** Default is `true` (expects 64-char hex PSK). If `false`, firstboot hashes it via `wpa_passphrase`. |
| `[wlan]` | `country` | yes | ISO 3166-1 alpha-2 (e.g. `GB`). **Required** — WiFi radio is soft-blocked until country is set. |
| `[wlan]` | `hidden` | no | `true` for hidden SSIDs |
| `[locale]` | `timezone` | no | e.g. `Europe/London` |

### What happens on first boot

1. Kernel runs `init=/usr/lib/raspberrypi-sys-mods/firstboot` (set in stock `cmdline.txt`)
2. `firstboot` finds `/boot/firmware/custom.toml`, passes it to `init_config` (Python)
3. `init_config` calls `imager_custom` for each section (hostname, user, SSH, WiFi, locale)
4. `imager_custom set_wlan` creates NM connection, sets regulatory domain, unblocks rfkill
5. `firstboot` removes `init=` from cmdline.txt (won't run again), reboots
6. On second boot, Pi connects to WiFi normally

### Additional config.txt overlays (post-firstboot)

After the Pi is running and accessible via SSH, add device-specific overlays:

**emonpi** (Pi 4B with EmonPi2 board):
```bash
sudo tee -a /boot/firmware/config.txt << 'EOF'

[all]
dtoverlay=w1-gpio,gpiopin=17
dtparam=i2c_arm=on
EOF
sudo reboot
```

**emondhw** (Pi Zero 2 W): no additional overlays needed.

---

## 9) emonpi firmware requirement

emonPi2 AVR firmware should be current (DB 6CT single-phase expected).
Verify via serial `v` command when emonhub is stopped.

Expected style:
- `firmware = emon_DB_6CT`
- `version = 2.1.1` (or newer)

---

## 10) Minimal acceptance checklist (`emonpi`)

- [ ] hostname `emonpi`
- [ ] SSH key auth works
- [ ] `mosquitto` active + bridge active
- [ ] `emonhub` active, publishing `emon/EmonPi2/P1..P3`
- [ ] DS18B20 publishes `emon/sensors/temp_high|temp_low`
- [ ] `zigbee2mqtt` container up and publishing device data (not just bridge/* topics)
- [ ] Zigbee devices paired and reporting (check `zigbee2mqtt/+` for device payloads)
- [ ] `emonPiLCD` active
- [ ] Firmware version verified current
- [ ] Data visible in Influx on `pi5data`

## 11) Minimal acceptance checklist (`emondhw`)

- [x] hostname `emondhw`
- [x] SSH key auth works
- [x] `mosquitto` active + bridge to pi5data active (`emon/#`)
- [x] `emonhub` active, publishing `emon/multical/dhw_*`
- [x] Multical 403 data visible in InfluxDB on pi5data (measurement `emon`, source `multical`)
- Note: ebusd and dhw-auto-trigger run on pi5data, not on emondhw

## 11b) Minimal acceptance checklist (`emonhp`)

- [ ] hostname `emonhp`
- [ ] SSH key auth works
- [ ] `mosquitto` active + bridge to pi5data active (`emon/#`)
- [ ] `emonhub` active, publishing `emon/heatpump/electric_Power`, `emon/heatpump/heatmeter_*`
- [ ] emoncms.org feeds updating (check feed timestamps via API)
- [ ] Data visible in InfluxDB on pi5data
- [ ] Local emoncms stack disabled (Apache, MariaDB, Redis, feedwriter, emoncms_mqtt, emoncms_sync, service-runner, Docker)

## 11c) Minimal acceptance checklist (`pi5data`)

- [ ] Docker services running: mosquitto, influxdb, grafana, telegraf, ebusd
- [ ] ebusd port 8888 exposed and reachable from host (`echo info | nc -w 2 localhost 8888`)
- [ ] `ebusd-poll` systemd service active, publishing `ebusd/poll/*` to MQTT
- [ ] `dhw-auto-trigger` systemd service active, subscribed to `emon/multical/dhw_flow`
- [ ] emon data arriving from all bridges (emonpi, emonhp, emondhw)
- [ ] Host packages: `tmux`, `mosquitto-clients`, `netcat-openbsd`

---

## 12) InfluxDB taxonomy

All buckets use a consistent measurement naming scheme:

| Measurement | Tags | Data | Source |
|---|---|---|---|
| `emon` | `source`, `field` | All emon numeric data (CT power, temperatures, heat meter, Multical) | Bridged from emonpi, emonhp, emondhw |
| `ebusd_poll` | `field` | eBUS polled numeric values (25+ HP parameters) | ebusd Docker on pi5data |
| `ebusd` | `circuit`, `field` | eBUS raw string data (broadcasts) | ebusd Docker on pi5data |
| `zigbee` | `device` | Zigbee2MQTT sensor/switch data (JSON) | Bridged from emonpi |

The `emon` measurement `source` tag identifies the device:
- `EmonPi2` — emonpi CT channels (P1–P6, V1, E1–E6)
- `sensors` — emonpi DS18B20 (temp_high, temp_low)
- `heatpump` — emonhp MBUS/SDM120 (electric_Power, heatmeter_*)
- `multical` — emondhw Kamstrup Multical 403 (dhw_t1, dhw_t2, dhw_flow, etc.)
- `emonth2_23` — emonhp RFM69 room sensor (temperature, humidity)
- `eas` — eBUS adapter stats

Historical data in `emonpi-apr2024` and `emonpi-nov2024` buckets also uses the `emon` measurement (migrated from legacy `emonpi` and `mqtt_consumer` names on 2026-03-21).

---

## 13) Host package baseline

All devices (emonpi, emonhp, emondhw, pi5data, pi5nvme) must have:

```bash
sudo apt-get install -y tmux mosquitto-clients netcat-openbsd
```

- `tmux` — SSH-disconnect-safe sessions for long operations
- `mosquitto-clients` — `mosquitto_sub`/`mosquitto_pub` for MQTT debugging and automation scripts
- `netcat-openbsd` — `nc` for ebusd TCP protocol and general network debugging

## 14) Design principles

- **Shell over Python** for simple MQTT/eBUS glue scripts — `mosquitto_sub`, `mosquitto_pub`, `nc` are sufficient. No pip packages, no runtime dependencies.
- **systemd over Docker** for custom scripts — Docker only for upstream software (ebusd, Mosquitto, InfluxDB, Grafana, Telegraf, Zigbee2MQTT).
- **Minimal installs** — emonhp and emondhw run only emonhub + mosquitto. No local emoncms, no Docker (except emonpi for Zigbee2MQTT).
- **Central hub** — pi5data handles all storage, visualization, and automation. Emon devices are data collectors only.
- **Hostnames over IPs** — use local DNS (`pi5data`, `emonpi`, `emonhp`, `emondhw`).

## 15) Notes

- If DNS fails on a host, fix resolver/search-domain first (`UseDomains=yes` for systemd-networkd hosts).
- Keep installs minimal; add components only when required by a concrete data path.
- The old Python `dhw-auto-trigger.py` has an inverted peak-block bug — do not deploy it. Use `dhw-auto-trigger.sh` instead.
