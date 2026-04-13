#!/bin/bash
# Sync adaptive-heating-mvp sources to pi5data for native build.
# Dev on laptop (fast cargo check), build release on pi5data (correct glibc).
set -euo pipefail

LOCAL_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REMOTE="pi5data"
REMOTE_DIR="/home/jack/adaptive-heating-mvp"
OCTOPUS_TARIFF_SRC="${HOME}/github/octopus-tariff"
OCTOPUS_TARIFF_REMOTE="/home/jack/github/octopus-tariff"

echo "=== Syncing sources to ${REMOTE}:${REMOTE_DIR} ==="

# Binary source → main.rs on pi5data
scp "${LOCAL_ROOT}/src/bin/adaptive-heating-mvp.rs" "${REMOTE}:${REMOTE_DIR}/src/main.rs"

# Thermal library modules
scp ${LOCAL_ROOT}/src/thermal/*.rs "${REMOTE}:${REMOTE_DIR}/src/thermal/"

# lib.rs (re-exports thermal + octopus_tariff modules)
scp "${LOCAL_ROOT}/src/lib.rs" "${REMOTE}:${REMOTE_DIR}/src/lib.rs"

# thermal.rs (module declaration)
scp "${LOCAL_ROOT}/src/thermal.rs" "${REMOTE}:${REMOTE_DIR}/src/thermal.rs"

# octopus_tariff.rs — thin re-export of the shared octopus-tariff crate
scp "${LOCAL_ROOT}/src/octopus_tariff.rs" "${REMOTE}:${REMOTE_DIR}/src/octopus_tariff.rs"

# Cargo manifest/lockfile — required when dependencies change (for example TSDB postgres support)
scp "${LOCAL_ROOT}/Cargo.toml" "${REMOTE}:${REMOTE_DIR}/Cargo.toml"
scp "${LOCAL_ROOT}/Cargo.lock" "${REMOTE}:${REMOTE_DIR}/Cargo.lock"

# Shared octopus-tariff crate (path dependency — must exist on pi5data)
ssh "${REMOTE}" "mkdir -p ${OCTOPUS_TARIFF_REMOTE}/src"
scp "${OCTOPUS_TARIFF_SRC}/Cargo.toml" "${REMOTE}:${OCTOPUS_TARIFF_REMOTE}/Cargo.toml"
scp "${OCTOPUS_TARIFF_SRC}/src/"*.rs "${REMOTE}:${OCTOPUS_TARIFF_REMOTE}/src/"

# Data files
scp "${LOCAL_ROOT}/data/canonical/thermal_geometry.json" \
    "${REMOTE}:${REMOTE_DIR}/data/canonical/thermal_geometry.json"

# Config
scp "${LOCAL_ROOT}/model/adaptive-heating-mvp.toml" \
    "${REMOTE}:${REMOTE_DIR}/model/adaptive-heating-mvp.toml"

echo "=== Sources synced. Now build on pi5data: ==="
echo "  ssh pi5data 'cd ${REMOTE_DIR} && . ~/.cargo/env && cargo build --release && cp target/release/heatpump-analysis target/release/adaptive-heating-mvp'"
echo ""
echo "Then restart:"
echo "  ssh pi5data 'sudo systemctl restart adaptive-heating-mvp'"
