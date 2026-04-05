# Thermal Model

13-room thermal network calibrated from Zigbee sensors and InfluxDB data. Powers the adaptive controller via `bisect_mwt_for_room()`.

## Purpose

Calculate equilibrium room temperatures for any outside temp and mean water temp (MWT), enabling the adaptive controller to find the minimum flow temp that holds Leather at target.

Secondary uses: radiator flow distribution analysis, fabric improvement predictions, FRV recommendations, kitchen radiator decision. See `docs/room-thermal-model.md` for full methodology and experimental results.

## Energy Balance

Each room's temperature evolves according to a coupled differential equation solved at 5-minute timesteps.

```
C_i × dT_i/dt = Q_rad_i - Q_fabric_i - Q_vent_i + Σ Q_transfer_ij + Q_gains_i
```

- `Q_rad = T50 × ((MWT - T_room) / 50)^1.3` — radiator output (0 if no rad or rad off)
- `Q_fabric = Σ(U × A) × (T_room - T_outside)` — external envelope loss
- `Q_vent = 0.33 × ACH × Volume × (T_room - T_outside)` — ventilation/infiltration
- `Q_transfer` — inter-room conduction through walls + buoyancy exchange through doorways
- `Q_gains` — internal (people ~80W, appliances) + solar

13 rooms → 13 coupled equations. Solver iterates to equilibrium or steps forward in time.

## Calibration

Jointly calibrated from two controlled cooldown nights (24–26 Mar 2026) with heating off via eBUS.

### Controlled Experiments

Two nights with heating off via eBUS, varying door states to separate coupled and isolated behaviour.

| Night | Date | Conditions | Door state | Purpose |
|---|---|---|---|---|
| Night 1 | 24→25 Mar | 10→7.5°C, windy | Normal (mixed) | Coupled system cooldown |
| Night 2 | 25→26 Mar | ~1.4°C, calm | All closed | Per-room external HLC isolation |

Night 2 isolates each room's heat loss to outside. Night 1 − Night 2 difference reveals doorway exchange rates.

### Fitted Parameters

Two parameters require joint fitting — changing one affects the other's optimum.

| Parameter | Value | Role |
|---|---|---|
| **Cd** (discharge coefficient) | 0.20 | Scales buoyancy-driven doorway air exchange |
| **Landing ACH** | 1.30 | Stairwell ventilation rate (chimney effect) |

⚠ Do not tune Cd or landing ACH independently. They are jointly calibrated against Night 1 + Night 2 residuals.

### Ventilation Groups

Ventilation rates assigned by room type, validated by moisture balance analysis.

| Group | ACH | Rooms | Evidence |
|---|---|---|---|
| MVHR (measured) | 0.75 (effective 0.16) | Bathroom | Spec validated by moisture (0.15 ACH) |
| Very leaky (bay window) | 0.80 | Jack & Carol | Moisture: AH drops with 2 occupants |
| Trickle vents (stack) | 0.70 | Elvina | Moisture: barely maintains AH with 1 person |
| Infiltration (high) | 0.50 | Hall | Front door + stairwell base |
| Open doors + draft | 0.30–0.35 | Kitchen, Conservatory, Front | RH drops overnight |
| Sealed but inadequate | 0.30 | Aldora | Moisture: AH rises with 1 person. Mould risk |
| Closed, slight leakage | 0.15–0.20 | Leather, Landing, Office, Shower | Closed doors or interior rooms |
| Sealed modern | 0.10–0.15 | Sterling | Triple glazed, door closed |

### Moisture Validation

Moisture balance independently cross-checks ventilation rates. Outside absolute humidity from Open-Meteo, indoor from SNZB-02P sensors.

Moisture ACH measures infiltration to outside only. Thermal ACH includes inter-room doorway exchange. The difference between them IS the doorway exchange rate.

Key finding: Aldora at 58.8% RH (surface ~71% at ΔT=3°C) = mould warning. Needs trickle vent (Part F requirement for bedroom).

## Thermal Mass

Construction-based estimates per room. Total house: **48,090 kJ/K**.

| Construction | Rooms | C range (kJ/K) |
|---|---|---|
| Brick + concrete slab | Kitchen, Conservatory | 4,810–6,308 |
| Brick + suspended timber | Hall, Front, Leather | 3,761–4,985 |
| Brick + timber (1st floor) | Bathroom, J&C, Office, Sterling | 2,226–5,202 |
| Timber (loft/landing) | Elvina, Aldora, Shower, Landing | 880–3,778 |

Ground-floor brick rooms cool much slower than loft timber rooms. This dominates cooling behaviour more than fabric U-values.

## Solver Functions

The equilibrium solver and MWT bisection live in `src/thermal/display.rs`.

- [[src/thermal/display.rs#solve_equilibrium_temps]] — given outside temp and MWT, iterates the 13-room system to steady state
- [[src/thermal/display.rs#bisect_mwt_for_room]] — binary search for the minimum MWT that holds a target room at a target temperature. Called by the adaptive controller every outer cycle.
- [[src/thermal/display.rs#generate_control_table]] — legacy: produces a lookup table (replaced by live solver in Phase 1b)

The solver loads room geometry via [[src/thermal/geometry.rs#thermal_geometry_path]] and uses the calibrated Cd and ACH parameters. Radiator output follows the T50 model in [[src/thermal/physics.rs#radiator_output]].

## Model Accuracy

Energy balance: model total loss 4,374W vs HP meter 3,989W — 10% over-prediction.

| Parameter | Confidence | Source |
|---|---|---|
| Fabric U-values | High | Measured areas + standard U-values |
| MVHR performance | High | Spec validated by moisture (0.16 vs 0.17 ACH) |
| Pipe topology / radiator T50 | High | Physical survey |
| Thermal mass (brick rooms) | Medium | Construction-based, not directly measured |
| Ventilation (moisture-validated rooms) | Medium | Aldora, Elvina, J&C — some uncertainty in outside AH |
| Ventilation (other rooms) | Low-Medium | Estimated, consistent with humidity trends |
| Inter-room doorway exchange | Medium | Buoyancy Cd model with canonical doorway geometry |
| Landing convective model | Medium-Low | Explicit chimney links modelled; reasonable fit |
| Leather ground floor loss | Low | Spiral cellar creates uncertain air gap |

### Known Discrepancies

Model HTC (261 W/K) overpredicts actual overnight heat loss by ~30% vs 466 nights showing ~190 W/K.

This may partly explain why the model's Leather τ=15h was wrong — lower real heat loss means slower cooling. The empirical τ=50h (53 segments) is used for control instead.

## Reproducibility and Regression

Thermal outputs are treated as reproducible artifacts. Baselines, thresholds, and config hashes gate changes before they are trusted operationally.

### Snapshots

`thermal-snapshot export` / `import` copy the baseline artifacts, regression thresholds, and chosen thermal config into a signed manifest bundle via [[src/thermal/snapshot.rs#snapshot_export]] and [[src/thermal/snapshot.rs#snapshot_import]].

Both commands require a non-empty signoff reason and explicit `--approved-by-human`. Snapshot import verifies SHA-256 hashes before copying files back into the repo.

### Regression Gates

[[src/bin/thermal-regression-check.rs]] compares candidate thermal artifacts against baselines using `artifacts/thermal/regression-thresholds.toml`. The CI wrapper is `scripts/thermal-regression-ci.sh`.

The default gate set checks four artifact families: calibrate, validate, fit-diagnostics, and operational. Global gates require the command name and config SHA-256 to match the baseline before metric tolerances are considered.

## Submodules

The thermal model spans 17 Rust source files in `src/thermal/`.

| Module | Responsibility |
|---|---|
| `config.rs` | TOML config from `model/thermal-config.toml` |
| `geometry.rs` | Room/connection/doorway types + JSON loading |
| `physics.rs` | Constants, thermal mass, energy balance equations |
| `solar.rs` | Solar position + irradiance model |
| `wind.rs` | Open-Meteo wind + ventilation multiplier |
| `calibration.rs` | Grid search calibration ([[src/thermal/calibration.rs#calibrate]]) |
| `validation.rs` | Metrics, residuals, holdout validation |
| `diagnostics.rs` | Cooldown detection + fit diagnostics |
| `operational.rs` | HP state classification, operational validation |
| `artifact.rs` | Artifact types, git metadata, build/write |
| `snapshot.rs` | Export/import manifests with human signoff |
| `display.rs` | CLI output, **equilibrium solver**, **MWT bisection** |
| `report.rs` | Table printer and RMSE |
| `influx.rs` | InfluxDB query builders ([[src/thermal/influx.rs#query_room_temps]]) |
| `history.rs` | Heating/DHW history reconstruction. Comfort miss detection clipped to waking hours (07:00–23:00) via [[src/thermal/history.rs#clip_period_to_waking_hours]] — overnight cooling is expected, not a miss |
| `dhw_sessions.rs` | DHW draw/charge session analysis |
| `error.rs` | `ThermalError` enum (thiserror) |
