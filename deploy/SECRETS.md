# Secrets Management — adaptive-heating-mvp

Referenced by `docs/heating-plan.md` and `docs/dhw-plan.md` for the controller's current production secret handling.

The live controller now reads and writes via PostgreSQL. Repo-local migration state is tracked in `../lat.md/tsdb-migration.md`; shared-platform sequencing remains in `~/git/energy-hub/lat.md/tsdb-migration.md`.

## Production (pi5data systemd service)

The controller needs **PostgreSQL conninfo** plus the existing Octopus account credentials.

### Where it lives

```text
/etc/adaptive-heating-mvp.env              # Octopus creds + TIMESCALEDB_CONNINFO
/etc/systemd/system/adaptive-heating-mvp.service
```

The systemd unit reads the environment file with:

```ini
EnvironmentFile=-/etc/adaptive-heating-mvp.env
```

### Required environment variables

```bash
OCTOPUS_API_KEY=...
OCTOPUS_ACCOUNT_NUMBER=...
TIMESCALEDB_CONNINFO="host=127.0.0.1 port=5432 user=... password=... dbname=..."
```

### Setup / rotation on pi5data

Derive TimescaleDB connection details from the local container and rewrite the env file entry:

```bash
env_dump=$(docker inspect timescaledb --format '{{range .Config.Env}}{{println .}}{{end}}')
pg_user=$(printf '%s\n' "$env_dump" | grep '^POSTGRES_USER=' | cut -d= -f2-)
pg_pass=$(printf '%s\n' "$env_dump" | grep '^POSTGRES_PASSWORD=' | cut -d= -f2-)
pg_db=$(printf '%s\n' "$env_dump" | grep '^POSTGRES_DB=' | cut -d= -f2-)
conninfo="host=127.0.0.1 port=5432 user=${pg_user} password=${pg_pass} dbname=${pg_db}"

{
  sudo grep -v '^TIMESCALEDB_CONNINFO=' /etc/adaptive-heating-mvp.env 2>/dev/null || true
  echo "TIMESCALEDB_CONNINFO=${conninfo}"
} | sudo tee /etc/adaptive-heating-mvp.env >/dev/null

sudo systemctl daemon-reload
sudo systemctl restart adaptive-heating-mvp
```

### Verification

```bash
systemctl is-active adaptive-heating-mvp
curl -fsS http://127.0.0.1:3031/status
sudo grep '^TIMESCALEDB_CONNINFO=' /etc/adaptive-heating-mvp.env
```

The service should stay up and `/status` should respond with the configured PostgreSQL conninfo.

## Development (WSL2 / local testing)

Local controller runs now need `TIMESCALEDB_CONNINFO` when using `status` or `run`, alongside any existing Octopus environment variables.

## Other credentials

| Credential | Used by | Where stored | Notes |
|---|---|---|---|
| PostgreSQL conninfo | Controller latest-value reads + decision writes | `/etc/adaptive-heating-mvp.env` | Local container-backed TimescaleDB conninfo |
| Octopus API key | Controller tariff fetch | `/etc/adaptive-heating-mvp.env` | Existing account API credential |
| eBUS | Controller (read/write VRC 700) | None needed | Unauthenticated TCP to ebusd on localhost:8888 |
| MQTT | Not used directly | N/A | Z2M sensors arrive through the shared data hub |
| Open-Meteo | Controller (weather forecast) | None needed | Public API, no auth |

## What NOT to do

- Don't hardcode tokens in source code or config files tracked by git
- Don't rely on `ak` for production — it's a dev-machine tool
- Don't put tokens in `model/adaptive-heating-mvp.toml` — that file is in the repo
- Don't add extra database credentials back to the controller; production uses `TIMESCALEDB_CONNINFO`
