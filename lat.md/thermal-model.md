# Thermal Model

13-room thermal network calibrated from Zigbee sensors and InfluxDB data. Powers the adaptive controller via `bisect_mwt_for_room()`.

## Purpose

Calculate equilibrium room temperatures for any outside temp and mean water temp (MWT), enabling the adaptive controller to find the minimum flow temp that holds Leather at target.

Minimum flow directly minimises electrical input: lower flow → better COP → less electricity. See [[constraints#Minimum Electrical Input Principle]].

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
| Trickle vents (stack) | 0.70 (model) | Elvina | Proxy-network overnight analysis implies ACH ≈ 1.0 (6.8× Aldora). This remains the accepted operating baseline: vents stay open, purifier already runs for allergies, and the colder winter room is tolerated. Ventilation UA ≈ 21 W/K — nearly all the excess over model |
| Infiltration (high) | 0.50 | Hall | Front door + stairwell base |
| Open doors + draft | 0.30–0.35 | Kitchen, Conservatory, Front | RH drops overnight |
| Sealed but inadequate | 0.30 | Aldora | Moisture: AH rises with 1 person. Mould risk |
| Closed, slight leakage | 0.15–0.20 | Leather, Landing, Office, Shower | Closed doors or interior rooms |
| Sealed modern | 0.10–0.15 | Sterling | Triple glazed, door closed |

### Moisture Validation

Moisture balance independently cross-checks ventilation rates. Outside AH is now available from the deployed outdoor SNZB-02P (`outside_temp_humid`), with indoor data from the per-room SNZB-02P sensors plus emonth2 (Leather humidity).

The original Elvina/Aldora evidence was built from an unoccupied-room proxy network (5 rooms: hall 7.66, kitchen 7.74, office 8.08, landing 8.11, front 8.22 g/m³). Mean 7.96 g/m³ = outside RH ≈ 82%. That proxy baseline remains historically useful, but the outdoor SNZB-02P now provides the direct AH_out reference for ongoing validation and before/after vent-intervention comparisons.

Overnight occupied-room findings (01:00–05:00, 6 nights, known occupancy):

| Room | Occupants | Mean AH | ΔAH vs proxy | Implied ACH | Model ACH | Finding |
|---|---|---|---|---|---|---|
| Aldora | 1 child | 9.92 | +1.97 | 0.35 (anchored at 0.30) | 0.30 | Sealed room, calibration anchor. Mould risk (58.8% RH) |
| Elvina | 1 child | 8.21 | +0.29 | ≈1.0 (ratio-method) | 0.51 | 3/6 nights drier than hallway. Ventilation is 6.8× Aldora |
| J&C | 2 adults | 8.31 | +0.37 | unreliable | 0.80 | Door closed; ΔAH too small for 2 adults in 34 m³ — humidity method breaks down. τ-derived UA (21 W/K) is below model (31 W/K), contradicting high ACH |
| Leather | 1 dog (PRT) | 8.35 | +0.39 | 0.5–0.7 | 0.67 | Consistent but dog M too uncertain to constrain |

J&C demonstrates two limitations of the moisture method: (1) with 2 adults generating ~60–100 g/h in only 34 m³, even modest M and AH_out uncertainties produce implausible ACH (4+); (2) overnight outdoor AH is not constant — post-sunset dew formation and temperature drop shift AH_out by ~0.3 g/m³ over the night, which is comparable to J&C’s entire ΔAH signal. The τ-derived total UA (21.2 W/K) contradicts the high implied ACH — if ACH were 4+, the room would cool far faster than its measured τ=57h. Moisture balance is most reliable for rooms with a large ΔAH signal: sealed rooms (Aldora, ΔAH 1.97) or highly ventilated rooms with 1 occupant (Elvina, ΔAH 0.29 but robustly below proxy rooms on 3/6 nights).

## Thermal Mass

Construction-based estimates per room. Total house: **48,090 kJ/K**.

| Construction | Rooms | C range (kJ/K) |
|---|---|---|
| Brick + concrete slab | Kitchen, Conservatory | 4,810–6,308 |
| Brick + suspended timber | Hall, Front, Leather | 3,761–4,985 |
| Brick + timber (1st floor) | Bathroom, J&C, Office, Sterling | 2,226–5,202 |
| Timber (loft/landing) | Elvina, Aldora, Shower, Landing | 880–3,778 |

Ground-floor brick rooms cool much slower than loft timber rooms. This dominates cooling behaviour more than fabric U-values.

## Empirical Room Time Constants

Fitted from 5–8 independent cooling segments per room (2 calibration nights, DHW events, controller coast phases) using Newton’s law: T(t) = T_out + (T₀ − T_out)·exp(−t/τ). Conservatory excluded (cannot be closed off, glazed roof).

| Room | τ operational | τ doors-closed | Ratio | Segments | Interpretation |
|---|---|---|---|---|---|
| jackcarol | 57h | 87h | 1.5× | 6 | Well-insulated 1st floor |
| shower | 42h | 82h | 2.0× | 6 | Tiny loft room, low UA |
| elvina | 34h | 66h | 1.9× | 5 | **Model underpredicts loss by 30%** — proxy-network says 6.8× Aldora ventilation (ACH≈1.0). Fabric residual 11 W/K < model 14.5. Roof insulation fine; trickle vents are the entire problem |
| aldora | 41h | 101h | 2.5× | 6 | Inter-room dominated (sealed, no trickle vent) |
| leather | 36h | 48h | 1.4× | 8 | Primary control room, 26% inter-room loss |
| landing | 35h | 70h | 2.0× | 6 | Stairwell, no radiator |
| hall | 29h | 41h | 1.4× | 6 | Stairwell base, consistent |
| kitchen | 27h | 86h | 3.2× | 7 | No radiator — 66% of cooling is inter-room |
| front | 28h | 44h | 1.6× | 6 | Most consistent fits (R² > 0.93) |
| bathroom | 25h | 69h | 2.8× | 6 | MVHR ventilation |
| office | 22h | 30h | 1.4× | 6 | Fastest occupied room, high ACH |

“Operational” = doors normal, outside ~10°C (median of coast/DHW segments). “Doors-closed” = Night 2 (1.4°C, all internal doors closed). The ratio reveals how much cooling comes from inter-room transfer vs external fabric loss. Sterling omitted (bimodal: τ=6h vs 160h — door-closed isolation makes it nearly adiabatic to outside).

C-weighted mean operational τ: 35h. This means the coast-then-hold strategy (minimise electrical input by holding the comfort floor at equilibrium flow) applies to the whole house, not just Leather.

Elvina is the key outlier: the model predicts τ=44h but empirical is 34h. Full overnight moisture analysis (13 sensors, 6 nights) shows the entire 7.4 W/K excess is ventilation (ACH ≈ 1.0 vs model 0.51). Fabric residual (11 W/K) is actually below model (14.5) — roof insulation appears fine. Closing trickle vents (with HEPA purifier running) would likely cut UA from 32 to ~17 W/K, gaining ~3°C overnight, but that intervention is not the current operating plan because the occupant prefers vents open, door closed, and accepts the colder winter room. See [[plan#Plan#Heating Controller#Active Work#Open: Elvina Overnight Comfort (Accepted Occupant Preference)]].

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
| Fabric U-values (most rooms) | High | Measured areas + standard U-values |
| Fabric U-values (Elvina roof) | **Medium** | Ratio-method analysis suggests fabric UA ≈ 14 W/K, close to model 14.5 — roof may be near spec. Absolute moisture ACH is too uncertain (ΔAH ≈ sensor noise) to separate fabric from ventilation reliably |
| MVHR performance | High | Spec validated by moisture (0.16 vs 0.17 ACH) |
| Pipe topology / radiator T50 | High | Physical survey |
| Thermal mass (brick rooms) | Medium | Construction-based, not directly measured |
| Ventilation (moisture-validated rooms) | Medium | Aldora (model anchor ACH 0.30, M_child ≈ 18 g/h), Elvina (ACH ≈ 1.0, 6.8× Aldora rate — nearly all excess UA). J&C unreliable (small room + 2 adults → ΔAH too small vs uncertainty). Original analysis used a 5-room proxy network (≈82% RH); ongoing validation can now use the deployed outdoor sensor directly |
| Ventilation (other rooms) | Low-Medium | Estimated, consistent with humidity trends |
| Inter-room doorway exchange | Medium | Buoyancy Cd model with canonical doorway geometry |
| Landing convective model | Medium-Low | Explicit chimney links modelled; reasonable fit |
| Leather ground floor loss | Low | Spiral cellar creates uncertain air gap |

### Known Discrepancies

Model HTC (261 W/K) overpredicts actual overnight heat loss by ~30% vs 466 nights showing ~190 W/K.

The model’s original Leather τ=15h was wrong by 3.3×. Empirical overnight τ=36h (8 segments, operational conditions) is used for control. The earlier 50h figure was from daytime segments with reduced inter-room loss; see [[thermal-model#Empirical Room Time Constants]] for the full per-room analysis showing τ varies with door state and outside temperature.

**Elvina per-room discrepancy**: model UA = 24.6 W/K, empirical UA = 32.0 W/K (+30%). The total UA is solid (τ fitted from cooling curves, R² > 0.95). Full proxy-network analysis (5 unoccupied rooms as AH_out reference, 6 nights) shows Elvina ΔAH = +0.29 g/m³ vs Aldora +1.97 — a ratio of 0.15, meaning Elvina ventilates **6.8× faster** than Aldora. On 3 of 6 nights Elvina was drier than the unoccupied hallway despite a child sleeping in it — outside air flushes through faster than the child adds moisture. Anchoring on Aldora ACH=0.30: implied Elvina ACH ≈ 1.0, ventilation UA ≈ 21 W/K, fabric residual ≈ 11 W/K (below model’s 14.5 — roof insulation appears fine). Nearly all the excess UA is trickle-vent ventilation. A HEPA purifier (LEVOIT Core 300, CADR 187 m³/h = 3.1 filtered ACH, 20W) already runs; closing vents would cut UA from 32 to ~17 W/K (+3°C overnight) while improving allergen control (no outdoor pollen ingress).

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
