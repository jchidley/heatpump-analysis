# Thermal regression baselines

This directory contains the committed baseline JSON artifacts used by
`scripts/thermal-regression-ci.sh` and CI regression gates.

## Required files

- `thermal-calibrate-baseline.json`
- `thermal-validate-baseline.json`
- `thermal-fit-diagnostics-baseline.json`

Missing any required file is a hard CI failure.

## Refresh procedure (intentional model/config change)

1. Export `INFLUX_TOKEN` from a secure source.
2. Generate fresh artifacts:

   ```bash
   cargo run --bin heatpump-analysis -- thermal-calibrate --config model/thermal-config.toml
   cargo run --bin heatpump-analysis -- thermal-validate --config model/thermal-config.toml
   cargo run --bin heatpump-analysis -- thermal-fit-diagnostics --config model/thermal-config.toml
   ```

3. Refresh baselines:

   ```bash
   bash scripts/refresh-thermal-baselines.sh
   ```

4. Validate gates locally:

   ```bash
   bash scripts/thermal-regression-ci.sh
   ```

5. Commit baseline updates in the same PR as the intentional model/config change.

## Rules

- Do not update baselines for unexplained drift.
- Do not relax thresholds and update model logic in the same unreviewed change.
- Keep threshold changes in `artifacts/thermal/regression-thresholds.toml` tightly justified in PR notes.
