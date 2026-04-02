#!/usr/bin/env bash
set -euo pipefail

section() {
  printf '\n== %s ==\n' "$1"
}

section "adaptive-heating HTTP /status"
if curl -fsS http://pi5data:3031/status; then
  printf '\n'
else
  echo "unavailable"
fi

section "adaptive-heating rich CLI status"
if [ -d "$HOME/adaptive-heating-mvp" ]; then
  (
    cd "$HOME/adaptive-heating-mvp"
    cargo run --quiet --bin adaptive-heating-mvp -- --config model/adaptive-heating-mvp.toml status --human || echo "status command failed"
  )
else
  REPO="/home/jack/projects/heatpump-analysis"
  [ -d "$REPO" ] || REPO="$HOME/projects/heatpump-analysis"
  if [ -d "$REPO" ]; then
    (
      cd "$REPO"
      TMP_CONFIG=$(mktemp)
      cp model/adaptive-heating-mvp.toml "$TMP_CONFIG"
      sed -i 's#127.0.0.1#pi5data#g' "$TMP_CONFIG"
      cargo run --quiet --bin adaptive-heating-mvp -- --config "$TMP_CONFIG" status --human || echo "status command failed"
      rm -f "$TMP_CONFIG"
    )
  else
    echo "no suitable checkout found"
  fi
fi

section "heatpump-analysis DHW live summary"
if command -v cargo >/dev/null 2>&1; then
  (
    cd /home/jack/projects/heatpump-analysis 2>/dev/null || cd "$HOME/projects/heatpump-analysis" 2>/dev/null || exit 1
    cargo run --quiet --bin heatpump-analysis -- dhw-live-status --human || echo "dhw-live-status failed"
  ) || echo "repo checkout unavailable"
else
  echo "cargo unavailable"
fi

section "z2m-hub /api/hot-water"
if curl -fsS http://pi5data:3030/api/hot-water; then
  printf '\n'
else
  echo "unavailable"
fi

section "z2m-hub /api/dhw/status"
if curl -fsS http://pi5data:3030/api/dhw/status; then
  printf '\n'
else
  echo "unavailable"
fi

section "raw eBUS spot checks"
for cmd in \
  'read -c 700 Hc1HeatCurve' \
  'read -c 700 Hc1ActualFlowTempDesired' \
  'read -c 700 DisplayedOutsideTemp' \
  'read -c hmu RunDataStatuscode' \
  'read -c 700 HwcStorageTemp' \
  'read -c 700 HwcSFMode'
  do
    printf '> %s\n' "$cmd"
    if ! printf '%s\n' "$cmd" | nc -w 2 pi5data 8888; then
      echo "unavailable"
    fi
  done
