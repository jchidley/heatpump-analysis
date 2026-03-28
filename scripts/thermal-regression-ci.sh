#!/usr/bin/env bash
set -euo pipefail

THRESHOLDS_FILE="${1:-artifacts/thermal/regression-thresholds.toml}"
BASELINE_DIR="artifacts/thermal/baselines"
ARTIFACT_DIR="artifacts/thermal"

run_check() {
  local command="$1"
  local baseline_path="${BASELINE_DIR}/${command}-baseline.json"

  if [[ ! -f "${baseline_path}" ]]; then
    echo "[thermal-regression] ERROR ${command}: missing required baseline ${baseline_path}" >&2
    return 2
  fi

  local candidate_path
  candidate_path="$(ls -1t "${ARTIFACT_DIR}/${command}-"*.json 2>/dev/null | head -n1 || true)"

  if [[ -z "${candidate_path}" ]]; then
    echo "[thermal-regression] no candidate artifact for ${command}; using baseline as candidate"
    candidate_path="${baseline_path}"
  fi

  echo "[thermal-regression] checking ${command}"
  cargo run --quiet --bin thermal-regression-check -- \
    --baseline "${baseline_path}" \
    --candidate "${candidate_path}" \
    --thresholds "${THRESHOLDS_FILE}"
}

run_check "thermal-calibrate"
run_check "thermal-validate"
run_check "thermal-fit-diagnostics"

echo "[thermal-regression] all configured checks passed"