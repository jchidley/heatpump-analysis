#!/usr/bin/env bash
set -euo pipefail

PI_HOST="${1:-pi5data}"

ssh "$PI_HOST" 'bash -s' <<'REMOTE'
set -euo pipefail

section() {
  printf "\n== %s ==\n" "$1"
}

repo="$HOME/adaptive-heating-mvp"
config="$repo/model/adaptive-heating-mvp.toml"
bin="$repo/target/release/heatpump-analysis"

if [ ! -x "$bin" ]; then
  echo "missing controller binary: $bin" >&2
  exit 1
fi

if [ ! -f "$config" ]; then
  echo "missing controller config: $config" >&2
  exit 1
fi

pg_env() {
  local key="$1"
  docker inspect timescaledb --format '{{range .Config.Env}}{{println .}}{{end}}' \
    | awk -F= -v key="$key" '$1 == key { print substr($0, index($0, "=") + 1) }'
}

pg_password="$(pg_env POSTGRES_PASSWORD)"
pg_user="$(pg_env POSTGRES_USER)"
pg_db="$(pg_env POSTGRES_DB)"
export PGPASSWORD="$pg_password"
export TIMESCALEDB_CONNINFO="host=127.0.0.1 port=5432 user=${pg_user} password=${pg_password} dbname=${pg_db}"

section "systemd service"
systemctl is-active adaptive-heating-mvp
systemctl status adaptive-heating-mvp --no-pager -n 6 | sed -n '1,20p'

section "standard status (current live config)"
"$bin" --config "$config" status --human

section "PostgreSQL status (read-only predeploy check)"
"$bin" --config "$config" status --human

section "adaptive_heating_mvp rows in TimescaleDB"
docker exec -e PGPASSWORD="$PGPASSWORD" timescaledb \
  psql -U "$pg_user" -d "$pg_db" -Atqc \
  "select 'total=' || count(*) from adaptive_heating_mvp; select 'last_24h=' || count(*) from adaptive_heating_mvp where time >= now() - interval '1 day';"

section "ebusd_poll_text presence"
docker exec -e PGPASSWORD="$PGPASSWORD" timescaledb \
  psql -U "$pg_user" -d "$pg_db" -Atqc \
  "select case when exists (select 1 from information_schema.tables where table_schema = 'public' and table_name = 'ebusd_poll_text') then 'present' else 'missing' end;"
REMOTE
