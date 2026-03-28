# Rust thermal formula parity audit (vs `model/house.py`)

Date: 2026-03-28

This document verifies core thermal formulas in Rust against Python and records intentional deltas.

## Scope checked

- `radiator_output`
- `external_loss`
- `ventilation_loss`
- `wall_conduction`
- `doorway_exchange`
- `estimate_thermal_mass`
- `room_energy_balance` composition/sign convention

## Formula parity summary

## 1) Radiator output

- Python: `Q = T50 * (ΔT/50)^1.3`, clamped to 0 when `ΔT <= 0`
- Rust: same formula and same clamp

Status: **Matched**

## 2) External fabric loss

- Python: `Σ(U * A * (T_room - T_ref))`, with `T_ref = GROUND_TEMP_C` for ground elements, else outside
- Rust: same implementation

Status: **Matched**

## 3) Ventilation loss

- Python: `VENT_FACTOR * ACH * V * (T_room - T_out) * (1 - η)`
- Rust: same base formula, with optional `wind_multiplier`

Status: **Matched with intentional extension** (wind multiplier)

## 4) Internal wall conduction

- Python: `UA * (T_a - T_b)`
- Rust: same

Status: **Matched**

## 5) Doorway buoyancy exchange

- Python: `(Cd/3) * W * sqrt(g * H^3 * |ΔT| / T_mean) * ρ * Cp * ΔT`
- Rust: same equation, same closed/partial behavior, same small-ΔT deadband

Status: **Matched**

## 6) Thermal mass estimate

- Python: identical component model (air, external walls, internal walls via implied area, floor, ceiling plaster, furniture)
- Rust: same structure and constants

Status: **Matched**

## 7) Room energy balance composition/sign

Both implementations use the same sign convention and decomposition:

- External loss: negative (out)
- Ventilation: negative (out)
- Walls/doorways: signed exchange
- Body heat: positive
- DHW parasitic heat in bathroom: positive

Status: **Matched for cooldown-calibration path**

## Intentional deltas (documented)

1. **Wind multiplier in Rust calibration path**
   - Rust supports optional Open-Meteo wind multiplier via config (`[wind]`).
   - Python baseline formula has no multiplier in the core function.
   - Rationale: controlled optional extension for testing sensitivity.

2. **Cooldown calibration keeps radiator and solar at zero**
   - Rust `room_energy_balance` in current calibration/validation path sets `q_rad = 0`, `q_solar = 0`.
   - This matches the cooldown-oriented intent and command behavior.
   - Full daytime/solar analysis parity is deferred to planned `thermal-analyse`/`thermal-equilibrium` parity commands.

3. **Runtime source for Influx token**
   - Rust uses `token_env` from TOML and environment variable.
   - Python file currently includes a literal token constant.
   - This is a security/operational improvement, not a physics delta.

## Additional Rust-only functions (28 Mar 2026)

The following functions exist in Rust `thermal-operational` but not in Python:

### 8) `full_room_energy_balance` (operational mode)

Extension of `room_energy_balance` with:
- Actual MWT from eBUS `FlowTemp`/`ReturnTemp` → radiator output computed
- Solar gain from PV (SW) + Open-Meteo DNI/DHI decomposition (NE)
- Body heat: sleeping (70W) vs active (100W) by time of day

### 9) `solar_gain_full`

Per-room solar gain using orientation-specific irradiance:
- SW vertical irradiance from PV (calibrated: `PV_gen × 0.087 / 1.4`)
- NE vertical + horizontal from Open-Meteo solar geometry decomposition
- Tilt corrections: 1.0× vertical, 1.4× sloping, 1.2× horizontal

### 10) Solar geometry (`solar_position`, `surface_irradiance`)

Spencer (1971) solar position + isotropic sky model for computing irradiance on arbitrary oriented surfaces from DNI + DHI.

### 11) `classify_hp_state_from_flow` / `segment_by_flow`

HP state classification from `BuildingCircuitFlow` (L/h): > 900 = DHW, 780–900 = heating, < 100 = off. Replaces unreliable `StatuscodeNum`-based classification.

## Conclusion

For the implemented Rust thermal commands (`thermal-calibrate`, `thermal-validate`, `thermal-fit-diagnostics`, `thermal-operational`), core physics formulas are parity-matched where applicable, with documented intentional deltas and Rust-only extensions for operational validation.
