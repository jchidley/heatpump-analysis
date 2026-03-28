# Phase 5 house-input revalidation (2026-03-28)

This report records execution of Phase 5 revalidation steps using current canonical tooling and source artifacts.

## Inputs and sources re-audited

- XLSX sources:
  - `Heating needs for the house.xlsx`
  - `Utility - Gas Electric-Jack_Laptop.xlsx` (supporting)
- Scan/plan sources already encoded in inventory pipeline:
  - BAC survey/order scans
  - Sunlight loft overview + part scans (`IMG_0829` set)
  - canonical transcribed points in `scan_dimension_points()`

## Commands run

```bash
uv run --with openpyxl python model/extract_house_inventory.py
uv run python model/audit_model_dimensions.py
```

Both completed successfully.

### Extract/inventory outputs produced

Under `model/data/inventory/`:
- `house_inventory.json`
- `canonical_geometry_provenance.csv`
- `room_dimension_summary.csv`
- `dimension_cross_reference.csv`
- `scan_dimension_points.csv`
- `canonical_dimensions.csv`
- radiator inventory CSVs

### Audit result

`model/audit_model_dimensions.py` result:
- checks: 509
- mismatches: 0

This confirms canonical geometry/provenance coverage and Rust/Python wiring consistency.

## Reconciliation outcome

- Canonical geometry remains the single source of truth (`data/canonical/thermal_geometry.json`).
- Provenance is complete for all geometry leaf fields.
- Rust (`src/thermal.rs`) and Python (`model/house.py`) both consume canonical geometry.

## Fudge-factor review

No new ad-hoc fudge factors were introduced.
Remaining calibrated/assumed values are explicit and traceable (e.g., calibration params, occupancy assumptions, optional wind multiplier), not hidden constants.

## Post-revalidation thermal reruns

After revalidation, thermal commands were re-run:

```bash
cargo run --bin heatpump-analysis -- thermal-calibrate --config model/thermal-config.toml
cargo run --bin heatpump-analysis -- thermal-validate --config model/thermal-config.toml
bash scripts/thermal-regression-ci.sh
```

Artifacts written:
- `artifacts/thermal/thermal-calibrate-20260328T061334Z.json`
- `artifacts/thermal/thermal-validate-20260328T061400Z.json`

Regression gate status:
- all configured checks passed.

## Conclusion

Phase 5 revalidation checks completed against available XLSX + scan-derived sources and canonical provenance.
