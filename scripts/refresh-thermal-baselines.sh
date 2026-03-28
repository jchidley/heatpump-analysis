#!/usr/bin/env bash
set -euo pipefail

ARTIFACT_DIR="artifacts/thermal"
BASELINE_DIR="${ARTIFACT_DIR}/baselines"

mkdir -p "${BASELINE_DIR}"

copy_latest() {
  local command="$1"
  local latest

  latest="$(ls -1t "${ARTIFACT_DIR}/${command}-"*.json 2>/dev/null | head -n1 || true)"
  if [[ -z "${latest}" ]]; then
    echo "[refresh-baselines] ERROR: no artifact found for ${command} in ${ARTIFACT_DIR}" >&2
    return 2
  fi

  local target="${BASELINE_DIR}/${command}-baseline.json"
  cp "${latest}" "${target}"
  echo "[refresh-baselines] ${command}: ${latest} -> ${target}"
}

copy_latest "thermal-calibrate"
copy_latest "thermal-validate"
copy_latest "thermal-fit-diagnostics"

echo "[refresh-baselines] done"
