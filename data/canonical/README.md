# Canonical Geometry Data

- `thermal_geometry.json` is the single source of truth for thermal model geometry.
- Consumed by:
  - `model/house.py` (Python model)
  - `src/thermal.rs` (Rust thermal calibration model)

All room dimensions, external fabric areas/U-values, radiators, internal
connections, and doorways should be edited here (not in source code).

`thermal_geometry.json` also contains a top-level `provenance` map with
field-level source references (scan callouts / XLSX cells / model-domain
constants) for every geometry leaf.

Intentional domain split:
- Geometry for thermal room model lives here (`data/canonical/thermal_geometry.json`)
- Broader analysis constants (tariffs, thresholds, house-level non-geometry
  assumptions) remain in `config.toml` by design.
