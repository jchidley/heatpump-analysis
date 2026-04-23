#!/usr/bin/env bash
set -euo pipefail

PI_HOST="${1:-pi5data}"
REMOTE_DIR="/home/jack/adaptive-heating-mvp"
REMOTE_CONFIG="${REMOTE_DIR}/model/adaptive-heating-mvp.toml"
REMOTE_STAGED_CONFIG="${REMOTE_DIR}/model/adaptive-heating-mvp.postgres-verify.toml"
REMOTE_BIN="${REMOTE_DIR}/target/release/adaptive-heating-mvp"
REMOTE_FALLBACK_BIN="${REMOTE_DIR}/target/release/adaptive-heating-mvp.pre-tsdb"

"$(dirname "$0")/sync-to-pi5data.sh"

ssh "$PI_HOST" "bash -s" <<REMOTE
set -euo pipefail

repo="$REMOTE_DIR"
config="$REMOTE_CONFIG"
staged_config="$REMOTE_STAGED_CONFIG"
bin="$REMOTE_BIN"
fallback_bin="$REMOTE_FALLBACK_BIN"

if [ -x "\$bin" ]; then
  cp "\$bin" "\$fallback_bin"
fi

cp "\$config" "\$staged_config"

cd "\$repo"
. ~/.cargo/env
cargo build --release --bin adaptive-heating-mvp

echo "=== Staged PostgreSQL verification config ready ==="
echo "staged_config=\$staged_config"
echo "fresh_bin=\$bin"
echo "controller_bin=\$bin"
echo "fallback_bin=\$fallback_bin"
REMOTE

"$(dirname "$0")/controller-tsdb-predeploy-check.sh" "$PI_HOST"

echo
echo "No systemd restart was performed."
echo "Staged config on ${PI_HOST}: ${REMOTE_STAGED_CONFIG}"
echo "Fallback binary on ${PI_HOST}: ${REMOTE_FALLBACK_BIN}"
