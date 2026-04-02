# Secrets Management — adaptive-heating-mvp

Referenced by `docs/heating-plan.md` and `docs/dhw-plan.md` for production and development InfluxDB token handling.

## Production (pi5data systemd service)

The only secret is the InfluxDB API token (for reading sensor data and writing decisions).

### Where it lives

```
/etc/adaptive-heating-mvp.env    # root:root 0600
```

Contents:
```
INFLUX_TOKEN=<influxdb-api-token>
```

The systemd unit loads this via `EnvironmentFile=-/etc/adaptive-heating-mvp.env`.

### Where to get the token

InfluxDB runs locally on pi5data (Docker container `influxdb`). The token is the same one Telegraf uses:

```bash
docker exec telegraf cat /etc/telegraf/telegraf.conf | grep token
```

### Setup on fresh install

```bash
# Get token from telegraf config
TOKEN=$(sudo docker exec telegraf grep -oP 'token = "\K[^"]+' /etc/telegraf/telegraf.conf)

# Write env file
echo "INFLUX_TOKEN=$TOKEN" | sudo tee /etc/adaptive-heating-mvp.env
sudo chmod 600 /etc/adaptive-heating-mvp.env
sudo chown root:root /etc/adaptive-heating-mvp.env

# Reload systemd
sudo systemctl daemon-reload
sudo systemctl restart adaptive-heating-mvp
```

## Development (WSL2 / local testing)

When `INFLUX_TOKEN` is not set in the environment, the code falls back to:

```bash
ak get influxdb    # GPG-encrypted keystore on dev machine
```

This only works on the dev machine where `ak` is installed. On pi5data without the env file, the service will fail with a clear error message pointing to `/etc/adaptive-heating-mvp.env`.

## Other credentials

| Credential | Used by | Where stored | Notes |
|---|---|---|---|
| InfluxDB token | Controller (read sensors, write decisions) | `/etc/adaptive-heating-mvp.env` | Same token as Telegraf |
| eBUS | Controller (read/write VRC 700) | None needed | Unauthenticated TCP to ebusd on localhost:8888 |
| MQTT | Not used directly | N/A | Z2M sensors come via Telegraf→InfluxDB |
| Open-Meteo | Controller (weather forecast) | None needed | Public API, no auth |

## What NOT to do

- Don't hardcode tokens in source code or config files tracked by git
- Don't rely on `ak` for production — it's a dev-machine tool
- Don't put tokens in `model/adaptive-heating-mvp.toml` — that file is in the repo
- The `influx_token_env` config field names the environment variable, not the token itself
