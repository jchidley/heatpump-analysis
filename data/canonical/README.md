# Canonical Geometry Data

- `thermal_geometry.json` is the single source of truth for thermal model geometry.
- Consumed by:
  - `model/house.py` (Python model)
  - `src/thermal.rs` (Rust thermal calibration model)

All room dimensions, external fabric areas/U-values, radiators, internal
connections, and doorways should be edited here (not in source code).
