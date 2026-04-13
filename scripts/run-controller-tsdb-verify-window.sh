#!/usr/bin/env bash
set -euo pipefail

PI_HOST="${1:-pi5data}"
WINDOW_SECS="${2:-200}"
REMOTE_DIR="/home/jack/adaptive-heating-mvp"
REMOTE_STAGED_CONFIG="${REMOTE_DIR}/model/adaptive-heating-mvp.postgres-verify.toml"
REMOTE_BIN="${REMOTE_DIR}/target/release/adaptive-heating-mvp"
REMOTE_LOG_DIR="${REMOTE_DIR}/logs"
REMOTE_LOG="${REMOTE_LOG_DIR}/tsdb-verify-window.log"

ssh "$PI_HOST" \
  WINDOW_SECS="$WINDOW_SECS" \
  REMOTE_DIR="$REMOTE_DIR" \
  REMOTE_STAGED_CONFIG="$REMOTE_STAGED_CONFIG" \
  REMOTE_BIN="$REMOTE_BIN" \
  REMOTE_LOG_DIR="$REMOTE_LOG_DIR" \
  REMOTE_LOG="$REMOTE_LOG" \
  'bash -s' <<'REMOTE'
set -euo pipefail

window_secs="$WINDOW_SECS"
repo="$REMOTE_DIR"
staged_config="$REMOTE_STAGED_CONFIG"
bin="$REMOTE_BIN"
fresh_bin="$REMOTE_DIR/target/release/heatpump-analysis"
log_dir="$REMOTE_LOG_DIR"
log_file="$REMOTE_LOG"
transient_unit="adaptive-heating-mvp-tsdb-verify.service"

if [ ! -x "$bin" ]; then
  echo "missing controller binary: $bin" >&2
  exit 1
fi
if [ ! -f "$staged_config" ]; then
  echo "missing staged config: $staged_config" >&2
  exit 1
fi

mkdir -p "$log_dir"
: > "$log_file"

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

count_rows() {
  docker exec -e PGPASSWORD="$PGPASSWORD" timescaledb \
    psql -U "$pg_user" -d "$pg_db" -Atqc \
    "select count(*) from adaptive_heating_mvp where time >= now() - interval '1 day';"
}

latest_row() {
  docker exec -e PGPASSWORD="$PGPASSWORD" timescaledb \
    psql -U "$pg_user" -d "$pg_db" -Atqc \
    "select to_char(time at time zone 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), coalesce(mode,'null'), coalesce(action,'null'), coalesce(tariff,'null'), coalesce(target_flow_c::text,'null'), coalesce(curve_after::text,'null') from adaptive_heating_mvp order by time desc limit 1;"
}

before_rows="$(count_rows)"
echo "before_last_24h=$before_rows"

echo "stopping systemd service"
sudo systemctl stop adaptive-heating-mvp
systemctl is-active adaptive-heating-mvp || true

if [ -x "$fresh_bin" ]; then
  cp "$fresh_bin" "$bin"
fi

echo "running staged controller via transient systemd unit for $window_secs seconds"
sudo systemctl reset-failed "$transient_unit" >/dev/null 2>&1 || true
sudo systemctl stop "$transient_unit" >/dev/null 2>&1 || true
sudo systemd-run \
  --unit="$transient_unit" \
  --property=User=jack \
  --setenv=TIMESCALEDB_CONNINFO="$TIMESCALEDB_CONNINFO" \
  /bin/bash -lc 'cd "$0" && exec "$1" --config "$2" run' "$repo" "$bin" "$staged_config"

sleep 5
if ! sudo systemctl --quiet is-active "$transient_unit"; then
  echo "transient verification unit failed during startup; tail follows" >&2
  journalctl -u "$transient_unit" --since '-5 minutes' --no-pager >&2 || true
  "$bin" --config "$staged_config" restore-baseline >>"$log_file" 2>&1 || true
  sudo systemctl start adaptive-heating-mvp
  exit 1
fi

sleep "$window_secs"

echo "stopping transient verification unit"
sudo systemctl stop "$transient_unit" || true
sudo systemctl is-active "$transient_unit" || true
journalctl -u "$transient_unit" --since '-10 minutes' --no-pager >"$log_file" 2>&1 || true

echo "restoring baseline after transient run"
cd "$repo"
"$bin" --config "$staged_config" restore-baseline >>"$log_file" 2>&1 || true

after_rows="$(count_rows)"
latest="$(latest_row)"

echo "after_last_24h=$after_rows"
echo "latest_row=$latest"

echo "restarting systemd service"
sudo systemctl start adaptive-heating-mvp
systemctl is-active adaptive-heating-mvp
systemctl status adaptive-heating-mvp --no-pager -n 8 | sed -n '1,20p'

echo "transient_log=$log_file"
echo "recent_transient_log_tail:"
tail -n 60 "$log_file" || true
REMOTE
