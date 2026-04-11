# Secrets Management — adaptive-heating-mvp

Referenced by `docs/heating-plan.md` and `docs/dhw-plan.md` for production and development InfluxDB token handling.

## Production (pi5data systemd service)

The controller now uses a **dedicated InfluxDB token** delivered via a **systemd credential**, not an environment variable.

### Where it lives

```text
/etc/adaptive-heating-mvp/influx.token     # root:root 0600, dedicated controller token
/etc/adaptive-heating-mvp.env              # optional non-Influx env vars only (for example Octopus creds)
```

The systemd unit loads the token with:

```ini
LoadCredential=influx_token:/etc/adaptive-heating-mvp/influx.token
```

At runtime systemd exposes the secret at:

```text
$CREDENTIALS_DIRECTORY/influx_token
```

### Token policy

- **Do not reuse the Telegraf token** for the controller
- Create a dedicated token described as `adaptive-heating-mvp-controller`
- Grant only:
  - read access to bucket `energy`
  - write access to bucket `energy`

This keeps the controller isolated from Telegraf and avoids putting the secret in the process environment.

### Setup / rotation on pi5data

Run on `pi5data` using the local Telegraf/Influx bootstrap token only for auth creation:

```bash
BOOTSTRAP_TOKEN=$(sudo docker exec telegraf grep -oP 'token = "\K[^"]+' /etc/telegraf/telegraf.conf)
BUCKET_ID=$(docker exec influxdb influx bucket list -t "$BOOTSTRAP_TOKEN" | awk '$2=="energy" {print $1}')

# Remove older controller auths with the same description
for id in $(docker exec influxdb influx auth list -t "$BOOTSTRAP_TOKEN" | awk '$2=="adaptive-heating-mvp-controller" {print $1}'); do
  docker exec influxdb influx auth delete -t "$BOOTSTRAP_TOKEN" --id "$id"
done

AUTH_TABLE=$(mktemp)
docker exec influxdb influx auth create \
  -t "$BOOTSTRAP_TOKEN" \
  --org home \
  --description adaptive-heating-mvp-controller \
  --read-bucket "$BUCKET_ID" \
  --write-bucket "$BUCKET_ID" > "$AUTH_TABLE"

NEW_AUTH_ID=$(awk 'NR==2 {print $1}' "$AUTH_TABLE")
NEW_TOKEN=$(awk 'NR==2 {print $3}' "$AUTH_TABLE")
rm -f "$AUTH_TABLE"

test -n "$NEW_AUTH_ID" && test -n "$NEW_TOKEN"

sudo install -d -m 700 -o root -g root /etc/adaptive-heating-mvp
printf '%s\n' "$NEW_TOKEN" | sudo tee /etc/adaptive-heating-mvp/influx.token >/dev/null
sudo chmod 600 /etc/adaptive-heating-mvp/influx.token
printf '%s\n' "$NEW_AUTH_ID" | sudo tee /etc/adaptive-heating-mvp/influx.auth-id >/dev/null
sudo chmod 600 /etc/adaptive-heating-mvp/influx.auth-id

# Optional after migration: remove legacy env injection
sudo sed -i '/^INFLUX_TOKEN=/d' /etc/adaptive-heating-mvp.env

sudo systemctl daemon-reload
sudo systemctl restart adaptive-heating-mvp
```

### Verification

```bash
sudo systemctl show adaptive-heating-mvp -p MainPID
sudo journalctl -u adaptive-heating-mvp -n 50 --no-pager
sudo test -s /etc/adaptive-heating-mvp/influx.token
```

The service should restart without `INFLUX_TOKEN not set in environment` warnings.

## Development (WSL2 / local testing)

When neither a systemd credential, `INFLUX_TOKEN`, nor `INFLUX_TOKEN_FILE` is available, the code falls back to:

```bash
ak get influxdb    # GPG-encrypted keystore on dev machine
```

This only works on the dev machine where `ak` is installed. On pi5data, production should use the systemd credential file instead.

## Other credentials

| Credential | Used by | Where stored | Notes |
|---|---|---|---|
| InfluxDB token | Controller (read sensors, write decisions) | `/etc/adaptive-heating-mvp/influx.token` | Dedicated controller token via systemd credential |
| eBUS | Controller (read/write VRC 700) | None needed | Unauthenticated TCP to ebusd on localhost:8888 |
| MQTT | Not used directly | N/A | Z2M sensors come via Telegraf→InfluxDB |
| Open-Meteo | Controller (weather forecast) | None needed | Public API, no auth |

## Ad-hoc InfluxDB queries from dev machine (WSL2)

To query InfluxDB on pi5data from the dev machine without SSH:

```bash
INFLUX_TOKEN=$(ak get influxdb)
curl -s -H "Authorization: Token $INFLUX_TOKEN" \
  "http://pi5data:8086/api/v2/query?org=home" \
  -H "Content-Type: application/vnd.flux" \
  -H "Accept: application/csv" \
  --data-raw 'from(bucket:"energy") |> range(start: -2h) |> filter(fn: (r) => r._measurement == "ebusd") |> last()'
```

Key details:
- **API**: InfluxDB v2 Flux, `http://pi5data:8086`
- **Org**: `home`, **Bucket**: `energy`
- **Token source**: `ak get influxdb` (GPG-encrypted keystore)
- **Measurements**: `ebusd`, `zigbee` (`_field="temperature"`), `adaptive_heating`, `tesla`, `dhw_inflection`, `dhw_capacity`

Do NOT use the InfluxDB v1 compatibility endpoint (`/query?db=`) — it requires separate auth and the v2 Flux API works directly.

## What NOT to do

- Don't hardcode tokens in source code or config files tracked by git
- Don't rely on `ak` for production — it's a dev-machine tool
- Don't reuse the Telegraf token for the controller once the dedicated token has been created
- Don't put tokens in `model/adaptive-heating-mvp.toml` — that file is in the repo
- The `influx_token_env` config field names the environment variable, not the token itself
