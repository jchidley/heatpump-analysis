#!/usr/bin/env bash
set -euo pipefail

HOST="${1:-emonpi}"

# Optional secret from local GPG store
if command -v ak >/dev/null 2>&1; then
  PI_PASSWORD="${PI_PASSWORD:-$(ak get emon-pi-credentials 2>/dev/null || true)}"
fi

if [[ -z "${PI_PASSWORD:-}" ]]; then
  echo "PI_PASSWORD not set (or emon-pi-credentials missing in ak store)."
  echo "Set it first: export PI_PASSWORD='...'; then re-run."
  exit 1
fi

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
TEMPLATES="$ROOT_DIR/templates"

ssh pi@"$HOST" "echo 'pi:${PI_PASSWORD}' | sudo chpasswd"

scp "$TEMPLATES/emonhub.conf" pi@"$HOST":/tmp/emonhub.conf
scp "$TEMPLATES/mosquitto-bridge.conf" pi@"$HOST":/tmp/bridge.conf
scp "$TEMPLATES/zigbee2mqtt-configuration.yaml" pi@"$HOST":/tmp/zigbee2mqtt-configuration.yaml

ssh pi@"$HOST" 'bash -s' <<'REMOTE'
set -euo pipefail

sudo apt update
sudo apt install -y git mosquitto mosquitto-clients \
  python3-serial python3-configobj python3-pip python3-pymodbus python3-spidev \
  docker.io

sudo systemctl enable --now mosquitto docker

# emonhub latest
sudo mkdir -p /opt/openenergymonitor
if [[ ! -d /opt/openenergymonitor/emonhub/.git ]]; then
  sudo git clone https://github.com/openenergymonitor/emonhub.git /opt/openenergymonitor/emonhub
else
  sudo git -C /opt/openenergymonitor/emonhub pull --ff-only
fi

cd /opt/openenergymonitor/emonhub
printf 'y\ny\n' | sudo bash ./install.sh || true

sudo cp /tmp/emonhub.conf /etc/emonhub/emonhub.conf
sudo cp /tmp/bridge.conf /etc/mosquitto/conf.d/bridge.conf

# Zigbee2MQTT minimal docker setup
sudo mkdir -p /home/pi/data
sudo cp /tmp/zigbee2mqtt-configuration.yaml /home/pi/data/configuration.yaml
sudo chown -R pi:pi /home/pi/data

if [[ ! -f /home/pi/docker-compose.z2m.yml ]]; then
cat <<'EOF' | sudo tee /home/pi/docker-compose.z2m.yml >/dev/null
services:
  zigbee2mqtt:
    image: koenkk/zigbee2mqtt:latest
    container_name: zigbee2mqtt
    restart: unless-stopped
    network_mode: host
    volumes:
      - /home/pi/data:/app/data
      - /run/udev:/run/udev:ro
    devices:
      - /dev/serial/by-id/usb-Silicon_Labs_Sonoff_Zigbee_3.0_USB_Dongle_Plus_0001-if00-port0:/dev/ttyACM0
EOF
fi

cd /home/pi
sudo docker compose -f docker-compose.z2m.yml up -d

sudo systemctl restart mosquitto emonhub
REMOTE

echo "Provision complete on ${HOST}."
