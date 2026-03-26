# AGENTS.md

## What This Is

Rust CLI tool that syncs heat pump data from emoncms.org to local SQLite, then analyses it with Polars. Vaillant Arotherm Plus 5kW at 6 Rhodes Avenue, London N22 7UT.

Also includes shell-based monitoring scripts deployed to pi5data, and extensive monitoring infrastructure documentation.

## Data Access

- emoncms dashboard: `https://emoncms.org/app/view?name=MyHeatpump&readkey=1b00410c57d5df343ede7c09e6aab34f`
- Read API key (read-only, safe to share): `1b00410c57d5df343ede7c09e6aab34f`

## Commands

| Task | Command |
|------|---------|
| Build | `cargo build` |
| Run | `cargo run -- <subcommand>` |
| Sync data | `cargo run -- --apikey KEY sync` |
| Analyse (7 days) | `cargo run -- --days 7 summary` |
| Analyse (date range) | `cargo run -- --from 2025-01-01 --to 2025-01-31 summary` |
| Analyse (all data) | `cargo run -- --all-data all` |
| With simulated | `cargo run -- --all-data --include-simulated summary` |
| Export CSV | `cargo run -- --days 30 export -o output.csv` |
| Degree days | `cargo run -- --all-data degree-days` |
| Indoor temp | `cargo run -- --all-data indoor-temp` |
| DHW analysis | `cargo run -- --all-data dhw` |
| COP vs spec | `cargo run -- --all-data cop-vs-spec` |
| Design comparison | `cargo run -- --all-data design-comparison` |
| Gap report | `cargo run -- gaps` |
| Fill gaps | `cargo run -- fill-gaps` |
| Octopus summary | `cargo run -- octopus` |
| Gas vs HP comparison | `cargo run -- --all-data gas-vs-hp` |
| Baseload analysis | `cargo run -- --all-data baseload` |
| **Room thermal model** | |
| Fetch sensor data | `uv run --with influxdb-client --with numpy --with scipy python model/house.py fetch [hours]` |
| Room summary | `uv run --with influxdb-client --with numpy --with scipy python model/house.py rooms` |
| Energy balance | `uv run --with influxdb-client --with numpy --with scipy python model/house.py analyse` |
| Fit cooldown | `uv run --with influxdb-client --with numpy --with scipy python model/house.py fit` |
| Connections | `uv run --with influxdb-client --with numpy --with scipy python model/house.py connections` |
| Equilibrium | `uv run --with influxdb-client --with numpy --with scipy python model/house.py equilibrium [T_out] [MWT] [solar_sw W/m²] [solar_ne W/m²]` |
| Moisture analysis | `uv run --with influxdb-client --with numpy --with scipy python model/house.py moisture` |

`--apikey` only needed for `feeds` and `sync`. Analysis reads from `heatpump.db`.
Octopus commands read from `~/github/octopus/data/` (usage_merged.csv, weather.json, config.json).
`gas-vs-hp` and `baseload` also need `heatpump.db` for HP state machine data.

## Architecture

```
config.toml   → All domain constants, thresholds, feed IDs, reference data (TOML)
config.rs     → Deserializes config.toml into typed structs (global singleton)
emoncms.rs    → API client (used only by sync)
db.rs         → SQLite storage + DataFrame loading
analysis.rs   → State machine + all Polars queries (no DB/API dependency)
gaps.rs       → Gap detection + synthetic data (accesses SQLite directly)
octopus.rs    → Octopus Energy integration + gas-vs-HP comparison
main.rs       → CLI routing (20 subcommands)
scripts/      → Shell scripts deployed to pi5data (DHW trigger, eBUS polling)
```

Git submodules:
- `avrdb_firmware/` — AVR-DB firmware (EmonTx4/EmonPi2/EmonTx5), compiled hex files for flashing
- `EmonScripts/` — emonSD install/update scripts, firmware upload tools
- `emonhub/` — data multiplexer (serial/MBUS/MQTT interfacers)
- `ebusd/` — eBUS daemon config

See `docs/code-truth/` for detailed architecture, patterns, and decisions.

## Monitoring Infrastructure

Four devices — see `heating-monitoring-setup.md` for full details (MQTT topics, eBUS data dictionary, credentials).

| Device | IP | Role |
|---|---|---|
| emonpi | 10.0.1.117 | EmonPi2 (3× CT), DS18B20, Z2M (19 Zigbee devices) |
| emonhp | 10.0.1.169 | Heat meter + SDM120 → emoncms.org |
| emondhw | 10.0.1.46 | Multical DHW meter |
| pi5data | 10.0.1.230 | Central hub: Docker (Mosquitto, InfluxDB, Telegraf, Grafana, ebusd) + systemd |

MQTT: each emon device bridges to pi5data. Credentials: `emonpi` / `emonpimqtt2016`.
Z2M: `ws://emonpi:8080/api` (no auth). z2m-hub manages automations (`~/github/z2m-hub/`).
eBUS: 25+ values every 30s via `ebusd-poll.sh` on pi5data.

## DHW Auto-Trigger — REMOVED

Removed Mar 2026. Was `scripts/dhw-auto-trigger.sh` on pi5data. Replaced by manual boost via z2m-hub mobile dashboard (`~/github/z2m-hub/`). Historical documentation in `docs/dhw-auto-trigger.md`.

## eBUS Polling

`scripts/ebusd-poll.sh` runs on pi5data as a systemd service. Pure shell script using `nc` + `mosquitto_pub`. Reads 25 eBUS values every 30s (+ 16 more every 5 min) via `nc localhost 8888` and publishes to `ebusd/poll/*` MQTT topics. Replaces the previous Python-in-Docker version that reinstalled dependencies on every container restart.

Deploy: `scp scripts/ebusd-poll.sh jack@pi5data:/tmp/ && ssh jack@pi5data "sudo cp /tmp/ebusd-poll.sh /usr/local/bin/ && sudo systemctl restart ebusd-poll"`

## Octopus Energy Integration

Data from `~/github/octopus/` — see `docs/octopus-data-inventory.md` for full audit.

```bash
cd ~/github/octopus && npm run cli -- refresh   # Refresh Octopus data
```

- Electricity Apr 2020→present, Gas Apr 2020→Jul 2024 (half-hourly)
- Current tariff: **Cosy Fix** (off 14.05p, mid 28.65p, peak 42.97p). 82.6% HP electricity at off-peak.
- Temperature: eBUS primary (real-time), Met Office control (hourly), ERA5-Land for gas era (+1.0°C bias correction)
- 102-day data gap Dec 2023→Mar 2024 (unfillable)

## Key Measured Performance

All from actual data (state machine + Octopus + emoncms):

| Metric | Value | Source |
|--------|-------|--------|
| Heating COP | 4.74 | State machine, heating days with HDD > 0.5 |
| DHW COP | 3.46 | State machine |
| Overall COP (instantaneous) | 5.09 | analysis.rs summary |
| Heating heat/HDD | 8.8 kWh | HP era, emoncms temps |
| Gas-era heating heat/HDD | 9.2 kWh | Gas × 90% − DHW est., ERA5+1.0°C |
| DHW demand | 11.0 kWh/day | HP heat meter, state machine |
| Annual HDD (5-yr avg) | 1,503 | Bias-corrected ERA5, base 15.5°C |
| House baseload | ~9 kWh/day | Octopus whole-house − HP SDM120 |
| Blended elec rate (HP era) | 17.07p/kWh | Octopus Cosy, consumption-weighted |
| Weighted gas rate (all gas era) | 5.66p/kWh | Octopus tariffs, consumption-weighted |

### Annual cost comparison (current tariffs, typical weather)

| | Gas combi | Heat pump |
|---|---|---|
| Gas consumed | 19,157 kWh | — |
| Electricity consumed | — | 3,951 kWh |
| Fuel cost | £1,125 | £674 |
| Gas standing charge | £115 | £0 |
| **Total** | **£1,239** | **£674** |
| **Annual saving** | | **£565 (46%)** |

## Key Domain Model

Operating states classified by flow rate (Arotherm 5kW fixed pump = 14.3 L/min):
- **Heating**: flow_rate 14.0–14.5, DT > 0, heat > 0
- **DHW**: flow_rate ≥ 15.0 (enter) / < 14.7 (exit), DT > 0, heat > 0
- **Defrost**: heat ≤ 0 OR DT < −0.5 (any flow rate)
- **Idle**: elec ≤ 50W

Thresholds in `config.toml` `[thresholds]`. Originally 16.0/15.0 for DHW — tightened to 15.0/14.7 in March 2026 due to y-filter sludge reducing DHW flow. Safe because heating is software-clamped at 14.3. See `docs/hydraulic-analysis.md`.

## Feed Notes

- `503101` (indoor_temp) = emonth2 sensor in **Leather room only**, not whole-house
- `503093` (outside_temp) = Met Office hourly, not Arotherm OAT sensor. Reads ~1.0°C warmer than ERA5-Land (507-day overlap). Used as control/cross-check for HP era; ERA5 bias-corrected +1.0°C for gas era. For real-time analysis, prefer `ebusd/poll/OutsideTemp` (Arotherm OAT, every 30s).
- `512889` (DHW_flag) = dead since Dec 2024
- Solar PV + battery system installed (not yet integrated):
    - 7× Trina 440W panels (TSM-440NEG9RC.27), 3.08 kWp, single string
    - Fox ESS F3600 inverter (3.6kW, dual MPPT — one MPPT input used)
    - Tesla Powerwall 2 (13.5 kWh) + Gateway
    - Commissioned: 19/04/2024, Emlite M24 generation meter

## Hydraulic System

See `docs/hydraulic-analysis.md`. Key: heating flow clamped at **14.3 L/min**, DHW 21.3 L/min (post y-filter clean Mar 2026). Idle flow rate is the early warning for sludge buildup.

## DHW Cylinder

See `docs/dhw-cylinder-analysis.md` for full analysis. Key numbers: Kingspan Albion 300L, usable **161L**, **45°C target** (optimal), standby loss 13W, eco mode ~115 min at 3.0 kW. DHW tracking via z2m-hub (`DHW_FULL_LITRES = 161`).

## Reference Data (config.toml)

- House: HTC 261 W/°C, floor area 180m², solid brick + 2010-standard top floor
- Radiators: 15× Stelrad, total T50 = 25,133W, output calculator with correction factor. Largest radiators (DP DF, TP TF) were the biggest single cost — more expensive than the HP itself.
- System cost (equipment only): Arotherm Plus 5kW **£3,624** + VRC 700 controller **£198** + Kingspan Albion 300L cylinder **£1,483** = **£5,304** (excludes radiators, installation, pipework)
- Arotherm spec: COP curve at -3°C (35°C→4.48, 55°C→3.06)
- Gas era: 18,702 kWh/yr gas, 90% boiler, 11.82 kWh/day hot water
- Insulation improved between gas and HP eras (heat/HDD dropped ~4%)
- Solid wall insulation planned but not yet done
- Spreadsheet models: `Heating needs for the house.xlsx` (U-values, radiators, HDD), `Utility - Gas Electric-Jack_Laptop.xlsx` (gas/electric history, PV, degree days, hot water)

## House Layout & Room Sensors

See [docs/house-layout.md](docs/house-layout.md) for full building physics: room connectivity, door states, thermal relationships, radiator inventory, pipe topology, ventilation, sensors.

See [docs/room-thermal-model.md](docs/room-thermal-model.md) for HP capacity analysis, EWI opportunity, FRV strategy, overnight data findings.

Key facts for agents:
- **13 rooms**, all sensored (Office + Landing added 24 Mar 2026). All SNZB-02P on v2.2.0.
- **15 radiators**, no TRVs. Kitchen and Landing have no radiator. Sterling rad is OFF.
- **Pipe topology**: 22mm primary (most rads) vs two 15mm branches (hall+front-horizontal, jackcarol+office) — 15mm branches are flow-starved.
- **Bathroom MVHR**: Vent-Axia Tempra LP, 9 L/s, 78% HR, runs 24/7. Door open 24h except during/after showers. Drives whole-house airflow via stairwell.
- **Outside temp**: eBUS `ebusd/poll/OutsideTemp` primary (30s), Met Office feed 503093 as control. Open-Meteo for outside humidity (moisture model).
- **Construction**: 1930s solid brick, all internal walls single brick. Ground floor: kitchen=concrete slab, conservatory=concrete slab (yr 2000), hall/front/leather=suspended timber. Leather has spiral cellar. Office+landing have 100mm insulated floor over hall (U≈0.25). Loft=2010 timber stud.
- **HP capacity**: maxes out at ~2°C outside (95% runtime). EWI on SE wall (50m², £5k DIY) would add 84 W/K = 32% HTC reduction.
- **eBUS heating control**: `write -c 700 Z1OpMode off/auto` toggles heating via ebusd on pi5data. DHW unaffected. Tested 24 Mar 2026. `at` scheduler on pi5data for timed experiments.
- **Bare CH pipes in floor void**: 2m of 35mm flow + return (bare copper) in the gap between kitchen ceiling and bathroom floor. Heats both rooms (~25W each at MWT=31). Not insulated — insulating would save ~50W continuous.
- **Morning demand analysis** (117 winter days, Nov 2025–Feb 2026): HP peaks at 4.8kW avg / 7.5kW max during 06:00-08:00 recovery. COP drops from 4.42 (evening steady) to 3.70 (morning recovery). Recovery MWT 38°C vs evening 32°C. DHW cycle at 06:00-07:00 (114/120 days) immediately precedes heating recovery — double stress on HP.
- **Setback disabled 26 Mar 2026**: `Z1NightTemp` changed from 17°C to 21°C (matches `Z1DayTemp`). Previously had 4°C overnight setback (midnight→05:00) causing recovery spike to 4.8kW/38°C MWT at worst COP. Analysis of 117 winter days showed setback was cost-neutral (£7/winter) but recovery stressed the HP. Without setback: steady 3.5kW/32°C all night, bedrooms cool naturally via closed doors. Revert: `echo 'write -c 700 Z1NightTemp 17' | nc -w 2 localhost 8888` on pi5data.
- **Door states**: Bathroom + Office + Shower normally open. Jack&Carol open day/closed night. Elvina/Aldora/Sterling/Leather always closed. Front partial. Kitchen↔Hall↔Conservatory always open.
- **SNZB-02P v2.1.0 bug**: readings freeze at power-on value. v2.2.0 fixes it. Always verify readings vary before trusting.

## Gotchas

- All domain constants, feed IDs, thresholds, and reference data live in `config.toml` — edit there, not in code
- `config.toml` must be next to the executable or in the current working directory
- `gaps.rs` bypasses `db.rs` — writes to SQLite tables directly
- `fill_gap_interpolate()` in gaps.rs still uses hardcoded feed IDs (`"503094"`, etc.) — not migrated to config
- No tests — validate changes against real data output
- Simulated data in separate table (`simulated_samples`) — never mixed unless `--include-simulated`
- DB schema is `CREATE TABLE IF NOT EXISTS` — no migrations
- Polars pinned to 0.46 (0.53 available) — untested on newer versions
- Outside temp feed (Met Office) is lower resolution (~hourly) than HP feeds (~10s)
- Thresholds are 5kW-specific — 7kW model would need different values (its heating rate = 20 L/min overlaps 5kW DHW rate)
- Two different HDD base temps: 15.5°C (UK standard in thresholds) vs 17°C (gas-era regression in house config)
- `octopus.rs` reads from `~/github/octopus/data/` — path hardcoded in `default_data_dir()`
- `ERA5_BIAS_CORRECTION_C` is a Rust constant in octopus.rs, not in config.toml
- `--all-data` start timestamp hardcoded in `resolve_time_range()`, duplicates `config.toml` value
- `daily_hp_by_state()` assumes exactly 1-minute sample interval (`SAMPLE_HOURS = 1/60`)
- Gas-era DHW estimated at 11.82 kWh/day (from config) — not measured. HP-era DHW is measured by state machine.
- `scripts/dhw-auto-trigger.py` is the old Python version — **do not deploy**. Shell version also removed (Mar 2026). DHW boost now handled by z2m-hub.
- `scripts/ebusd-poll.sh` uses `nc | head -1` to avoid ebusd TCP connection hanging — without `head -1`, each `nc` call waits 5s for the server to close.
- Multical `dhw_volume_V1` register has **10L resolution** — ground truth for draw tracking. `dhw_flow` integration interpolates between steps (resets at each step, clamped 0–9.9L). Use `dhw_flow` at 2s resolution for sub-litre analysis (e.g., thermocline pinpointing).
- DHW remaining uses 161L capacity (z2m-hub `DHW_FULL_LITRES` constant) — validated at 2s resolution by T1 inflection during shower draws. Don't change without re-validating against draw+T1 data at full resolution.
- SNZB-02P sensors on firmware v2.1.0 (8448) have a known bug: readings freeze at power-on value. Always verify sensor readings **vary over time** before using them in analysis. A flat reading across changing conditions = broken sensor, not thermal equilibrium.
- **Bathroom sensor was in airing cupboard until 25 Mar 2026 21:00.** All bathroom temp/humidity data before this time reads ~3°C high and humidity ~15% low. Moved to open wall. Historical bathroom data should NOT be used for model calibration without correction.
- SNZB-02P OTA updates flood InfluxDB with ~4 readings/sec of spam during transfer. Delete the OTA period data from InfluxDB after each update.
- `Heating needs for the house.xlsx`: Leather "Windows" uses ΔT=19°C but faces conservatory (internal, actual ΔT ≈ 0.5°C). All internal wall ΔT=5°C assumptions overestimate by 2-10× vs measured ~1.5°C.

## Boundaries

- Don't change operating state thresholds without re-validating against the full dataset
- Don't mix simulated and real data by default
- Don't commit `heatpump.db` or API keys
- Don't modify `~/github/octopus/` from this project — refresh via `npm run cli -- refresh`
- Keep `HDD_BASE_C` in `octopus.rs` in sync with `HDD_BASE_TEMP` in `analysis.rs` (both read from config now)
- Keep `GAS_DHW_KWH_PER_DAY` and `BOILER_EFFICIENCY` in `octopus.rs` in sync with config.toml `[gas_era]`
- Human-facing docs: `docs/` (Diátaxis style) — see `docs/code-truth/` for derived-from-code docs
- This file (`AGENTS.md`) is the single LLM context source. `docs/code-truth/` is for human comprehension.
- InfluxDB `energy` bucket contains: live MQTT data, 12.2M historical emonhp points from emoncms.org (Oct 2024+), 40M historical emonpi points from phpfina backups (Apr 2024+), 149k outside temperature points from Met Office, `dhw.remaining_litres` (written by z2m-hub), Zigbee room sensor data (10× SNZB-02P temp/humidity from Mar 2026, topic `zigbee2mqtt/*_temp_humid`), eBUS data (25+ values every 30s, topics `ebusd/poll/*` and `ebusd/hmu/*`)
- Don't modify monitoring infrastructure from this project — use SSH to emonpi/emondhw/emonhp/pi5data directly
- Don't store credentials in plaintext — use `ak get emon-pi-credentials` at runtime
- Always verify SNZB-02P sensor readings **vary over time** before using in analysis — stuck readings look like real data
- Thermal model (`model/house.py`) room definitions must match AGENTS.md radiator/pipe/ventilation data — don't update one without the other
- Moisture model uses Open-Meteo for outside humidity — assumes 75% if API unavailable
- Thermal mass estimates are construction-based, not measured — suspended timber floors have much less mass than concrete slabs
- `analyse()` gives misleading results during warmup transients — use for steady-state only
- InfluxDB token for pi5data is in `model/house.py` constants — same token as Telegraf config on pi5data

## Room Thermal Model

Python model in `model/house.py` — see [docs/room-thermal-model.md](docs/room-thermal-model.md) for full documentation.

Lumped-parameter thermal network: 13 room sensors + eBUS outside temp + HP heat meter. Zero free parameters in fabric model — all from physics, measurements, or construction. Two calibrated parameters from controlled experiments (landing chimney ACH, doorway Cd).

**Architecture** (refactored 25 Mar 2026):
- `RoomDef`: physical properties only (floor area, external fabric, radiators, ventilation ACH, occupancy). No analysis results in room definitions.
- `InternalConnection`: symmetric wall/floor conduction between rooms. Defined ONCE per connection. U_internal_wall = 2.37 W/m²K (100mm brick + plaster, calculated).
- `Doorway`: buoyancy-driven convective exchange (Cd=0.20, calibrated). Stairwell marked as "chimney" — modelled as landing ventilation ACH instead.
- `build_sensor_map()`: single function for sensor→room mapping, shared by all analysis.
- DHW cylinder heat: 75W cylinder surface + 42W insulated pipes + 16W shower residual → bathroom.
- Body heat: 70W sleeping, 100W active, per occupant from `overnight_occupants`.

**Calibration (Night 1 vs Night 2, 24-26 Mar 2026)**:
- Night 1: doors normal, T_out avg 8.5°C
- Night 2: all doors closed, T_out avg 5.0°C
- Doorway Cd = 0.20 (buoyancy exchange for standard doors)
- Landing chimney ACH = 1.30 (stairwell stack effect, was 0.15)
- Stairwell doorways (hall↔landing, landing↔shower) disabled — chimney captured by ACH
- Bathroom sensor moved from airing cupboard to room wall (was reading 3°C high)

**Room stacking** (confirmed):
```
Loft:    Aldora    | Elvina        | Shower
1st:     Sterling  | Jack&Carol    | Bathroom  (+ Office, Landing)
Ground:  Leather   | Front         | Kitchen   (+ Conservatory, Hall)
```
1st-floor ceilings are INTERNAL connections to loft rooms (U=0.44), not external.

**Model accuracy (26 Mar 2026)**:

| Mode | RMSE | Good | All within range |
|---|---|---|---|
| Cooldown (8h) | 0.41°C | 9/13 | 13/13 (0.5–2×) |
| Warmup (8h) | 1.16°C | 7/13 | 11/13 (<2°C) |
| Equilibrium | 1.2°C | 7/13 | 11/13 (<2°C) |

Remaining outliers: kitchen equilibrium (−2.2°C, needs more doorway exchange), shower equilibrium (+2.1°C, same), elvina warmup (solar gain not modelled).

**Commands**: `fetch [hours]`, `rooms`, `connections`, `analyse`, `fit`, `equilibrium [T_out] [MWT] [solar_sw] [solar_ne]`, `moisture`

**Solar gain model** (added 26 Mar 2026): `SolarGlazing` per room with area, orientation (SW/NE), tilt, g-value, shading factor. Calibrated from PV (EmonPi2 P3) vs room temp rise. Elvina's solar (1000W at 400 W/m²) exceeds her radiator (909W T50). Warmup RMSE: elvina 4.1→1.2°C. Key shading: office 0.05 (fabric blind), jack&carol 0.20 (blind set back), conservatory 0.14 (NE, shaded from ~11:00). House faces SW (front). **⚠ P3 CT scaling incorrect** — reads 6.7kW for 3.08kWp array. Scaling factor 0.087 W/m² per W calibrated from elvina's temp response. CT ratio needs fixing in emonpi config.

**Measured heat loss** (Night 2, all doors closed, T_out avg 5.0°C — pure fabric + ventilation):

| Room | Measured W | Effective W/K | Radiator T50 | Rad @MWT31 | Status |
|---|---|---|---|---|---|
| Conservatory | 1016 | 78.2 | 5700 | 989 | 27W short |
| Front | 495 | 34.8 | 4801 | 735 | +240W spare |
| Leather | 485 | 28.7 | 4752 | 519 | tight |
| Hall | 452 | 33.4 | 2376 | 392 | 60W short |
| Kitchen | 281 | 20.1 | 0 | — | no rad |
| Office | 267 | 20.2 | 1345 | 229 | 38W short |
| Jack&Carol | 247 | 17.6 | 1950 | 305 | OK with body heat |
| Elvina | 227 | 17.4 | 909 | 158 | 69W short (bottleneck) |
| Bathroom | 209 | 15.3 | 996 | 161 | 48W short (DHW helps) |
| Sterling | 135 | 9.9 | 0 (off) | — | heated by leather below |
| Aldora | 94 | 6.5 | 376 | 56 | 37W short (body heat covers) |
| Landing | 64 | 4.7 | 0 | — | chimney + neighbours |
| Shower | 51 | 3.8 | 752 | 123 | +72W spare |

**Humidity-derived ventilation** (Night 1 windy vs Night 2 calm, occupied rooms):

| Room | Night 1 ACH (windy) | Night 2 ACH (calm) | Wind adds | Model ACH | Notes |
|---|---|---|---|---|---|
| Jack&Carol | 1.89 | 1.00 | +0.89 | 0.80 | Bay window leaks. Fix: draught-strip. |
| Elvina | 0.71 | 0.22 | +0.49 | 0.70 | Trickle vents voluntary (closeable). Sloping roof acts as wind scoop. |
| Aldora | 0.37 | 0.28 | +0.09 | 0.30 | Well-sealed flat roof. Mould risk (needs trickle vent). |

**Doorway effects** (Night 1 − Night 2, normalised for outside temp):

| Room | Effect °C/h | Meaning |
|---|---|---|
| Landing | +0.17 | Chimney: warm air rises ground→loft through stairwell |
| Kitchen | +0.16 | Exports heat through 2 open doorways to hall+conservatory |
| Shower, Front | +0.08 | Borderline (near 0.05 noise floor) |
| Office, Conservatory | ~0 | Receive heat through doors — closing doors hurts them |

**Moisture model**: tracks absolute humidity per room. Surface RH uses physics-based calculation (U_max × Rsi × ΔT, not fixed 3°C). Cross-validates ACH_moisture vs ACH_thermal. Aldora remains at mould risk (RH ~59%, needs trickle vent).

**Bottleneck sequence** (from equilibrium model):
1. **Elvina** (vents open, ACH=0.70): forces MWT to 49°C at -3°C. Voluntary — close vents to fix (FREE).
2. **Aldora** (376W towel rad): forces MWT to 47°C. Fix: upgrade to 909W DP DF (~£150, rad available).
3. **Bathroom** (996W towel rads, 25.6 W/K external): forces MWT to 45°C. DHW cylinder helps (93W). Hardest to fix.

With all three fixed + J&C draught-proofing + EWI SE wall 30m²: MWT drops to 43°C at -3°C (COP 3.76 vs 3.31 baseline).

**Intervention analysis** (equilibrium model, uniform 20°C target):

| Intervention | Cost | MWT at -3°C | Annual heat | Annual cost | Saving |
|---|---|---|---|---|---|
| Current | — | 49°C | 23,670 kWh | £894 | — |
| + Elvina vents closed | £0 | 47°C | 22,640 kWh | £860 | £34 |
| + Aldora rad upgrade | ~£150 | 45°C | 22,640 kWh | £860 | £34 |
| + J&C draught-strip | ~£30 | 45°C | 22,170 kWh | £842 | £52 |
| + EWI SE 30m² | ~£5k | 43°C | 19,139 kWh | £677 | £216 |

EWI is the big win: **19% heat demand reduction**. At same 20°C target, HP runs at 84% capacity (vs 106% currently at 5°C outside). Less cycling, less wear, more headroom. Payback ~23 years on energy, but comfort and HP longevity are the real drivers.

## Planned Enhancements

See [docs/roadmap.md](docs/roadmap.md) for full details:
- **eBUS integration into analysis** — eBUS is physically connected and publishing data (25+ values every 30s). Not yet used by the Rust analysis tool. Could validate or replace the flow-rate state machine using StatuscodeNum. The thermal model already uses StatuscodeNum for free-cooling detection.
- **Solar PV + battery** — system installed, details above. Self-consumption analysis, DHW scheduling to solar peak.
- **Solar gain model** — PV generation (EmonPi2 P3, includes Powerwall) as irradiance proxy. House is **SW-facing** (front). Two solar faces:
    - **SW** (front, hall, jackcarol, office, elvina): afternoon sun, peaks ~14:00-16:00. PV panels on elvina's sloping roof face same way.
    - **NE** (conservatory glazed roof, sterling, aldora, bathroom, shower): morning sun only, shaded by ~11:00.
  Measured 26 Mar 2026: elvina +1456W solar (451 W/m² glazing), front +408W, conservatory +1000W morning only. Elvina gets more solar than any radiator. PV P3 channel peaks 6.7kW for 3.08kWp array — includes Powerwall discharge, use as relative proxy only. Needs per-room orientation + g-values to model.
- **Cost analysis subcommand** — tariff data and cost calculations could be a proper Rust subcommand.
- ~~**Controlled cooldown experiments**~~ — Night 1 (24-25 Mar, doors normal, 7.5°C avg) and Night 2 (25-26 Mar, all doors closed, 5.0°C avg) complete. Calibrated doorway Cd=0.20 and landing chimney ACH=1.30. Bathroom sensor moved from airing cupboard to wall (was 3°C high).
- ~~**Office + Landing sensors**~~ — added 24 Mar 2026. 13/13 room coverage complete.
- **Jack&Carol bay window draught-proofing** — moisture-proven leakage: ACH=1.00 even calm (Night 2), ACH=1.89 windy (Night 1). Wind adds 0.89 ACH. Draught strip frame joints. Saves ~60W calm, ~150W windy. Becomes system bottleneck once elvina closes vents.
- **Aldora rad upgrade** — 909W DP DF replaces 376W towel (rad available). Removes aldora as bottleneck. Trickle vent still needed for mould risk (58.8% RH, Part F required).
- **FRV installation** — model is calibrated. Leather (30°C), front (28°C), shower (25°C) massively overheat at -3°C design day. FRVs on these three 22mm radiators would allow MWT to drop a further 3-5°C, worth more annual savings than the EWI. Calculate exact settings from equilibrium model, install, and verify with sensors.
- **Sterling floor insulation** — mineral wool between joists (leather ceiling / sterling floor). Sterling occupant prefers cold + opens windows → leather's heat goes straight outside. Insulating stops this: leather retains heat, Sterling gets cold room, HP saves energy. Best single-room intervention after EWI.
- **EWI on SE wall** — ~30m² solid brick (hall, front, jack&carol, office), U 2.11→0.23. DIY, before next winter. Model predicts: 19% heat demand reduction (23,670→19,139 kWh/yr), MWT drops 49→43°C at -3°C, annual saving £216/yr. HP runs at 84% capacity vs 106% currently. Main benefit is comfort (20°C achievable everywhere) and HP headroom, not payback (£5k DIY — roughly the same as the entire HP+controller+cylinder cost).
