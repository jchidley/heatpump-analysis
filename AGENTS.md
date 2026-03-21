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

Four-device monitoring network documented in `heating-monitoring-setup.md`:
- **emonpi** (eth0 10.0.1.117, wlan0 10.0.1.111) — EmonPi2 (3× CT: DNO grid/house/solar), 2× DS18B20, Zigbee2MQTT (8 devices, 3 active), Pi 4B. Mosquitto open on 0.0.0.0:1883 with auth (user `emonpi`, pass `emonpimqtt2016`).
- **emonhp** (10.0.1.169) — MID-certified MBUS heat meter + SDM120 electric meter + RFM69 room sensor → emoncms.org. Minimal install: emonhub + mosquitto only (local emoncms stack disabled — was unused).
- **emondhw** (10.0.1.46) — Multical DHW meter (emonhub + Mosquitto bridge only). Pi Zero 2 W, 426MB RAM. USB-Modbus adapter has udev rule (`99-multical.rules`) creating stable `/dev/ttyMULTICAL` symlink — prevents data loss on USB reconnect.
- **pi5data** (10.0.1.230) — Central hub: Docker (Mosquitto + InfluxDB + Telegraf + Grafana + ebusd) + systemd (ebusd-poll.sh + dhw-auto-trigger.sh + z2m-automations.sh)

All hostnames resolve via local DNS (dnsmasq on router 10.0.0.1, domain `chidley.home`). Static DHCP reservations for all four devices.

### MQTT Architecture
Each emon device runs local Mosquitto with a bridge to pi5data. Telegraf on pi5data subscribes only to its local Mosquitto — all data arrives via bridges:
```
emonpi  ─── bridge (emon/#, zigbee2mqtt/+) ───┐
emonhp  ─── bridge (emon/#) ──────────────────┼──→ pi5data Mosquitto ──→ Telegraf ──→ InfluxDB
emondhw ─── bridge (emon/#) ──────────────────┘         ↑
                                                         │
eBUS adapter (10.0.1.41:9999) ──→ ebusd (Docker, port 8888 exposed) ──→ ebusd-poll.sh (systemd) ──→ ebusd/poll/* topics
                                                         │
                                              dhw-auto-trigger.sh (systemd) ←── emon/multical/dhw_flow
                                                         │ (on sustained draw, writes via nc to ebusd:8888)
                                                         ╰──→ HwcSFMode load
```
Bridges use QoS 1 + `cleansession false` — messages queue during pi5data outages.
MQTT credentials: user `emonpi`, password `emonpimqtt2016` (all devices).

### Host Package Baseline
All devices (emonpi, emonhp, emondhw, pi5data, pi5nvme) have: `tmux`, `mosquitto-clients`, `netcat-openbsd`.

### Design Principles
- **Shell over Python** for simple MQTT/eBUS glue scripts — `mosquitto_sub`, `mosquitto_pub`, `nc` are sufficient
- **systemd over Docker** for custom scripts — Docker only for upstream software (ebusd, Mosquitto, InfluxDB, Grafana, Telegraf, Zigbee2MQTT)
- **Minimal installs** — emonhp and emondhw run only emonhub + mosquitto. No local emoncms, no Docker (except emonpi for Zigbee2MQTT)
- **Central hub** — pi5data handles all storage, visualization, and automation. Emon devices are data collectors only.

### emonpi Details
- **EmonPi2 firmware**: emon_DB_6CT v2.1.1 (serial `/dev/ttyAMA0`)
- **CT channels**: P1=DNO grid, P2=House consumption, P3=Solar (P4–P6 unused)
- **DS18B20**: `28-00000ee9cb6d` (temp_high), `28-00000ee9e94f` (temp_low) — same space, different heights
- **Zigbee2MQTT**: Docker (v2.9.1), Sonoff USB 3.0 dongle, 8 paired devices (4× SNZB-02P temp/humidity, 3× ZBMINI switches, 1× Aqara RTCGQ14LM motion). **Status**: 3 active (landing, hall, landing_motion), 5 dead since Nov 2024 — need re-pairing after March 2026 rebuild. WebSocket API at `ws://emonpi:8080/api` (no auth). Mosquitto open on 0.0.0.0:1883 with password auth.
- **z2m-hub**: Zigbee automation hub and SPA server — see `~/github/z2m-hub/`. Will replace `z2m-automations.sh` on pi5data.
- **Credentials**: `pi` user, password in GPG store (`ak get emon-pi-credentials`) and Bitwarden ("emon pi, pi credentials")

eBUS provides 25+ values every 30s including operating mode (StatuscodeNum), compressor speed, target flow temp, cylinder temp. See `heating-monitoring-setup.md` for full MQTT topic list and eBUS data dictionary.

## DHW Auto-Trigger

`scripts/dhw-auto-trigger.sh` runs on pi5data as a systemd service. Pure shell script using `mosquitto_sub` + `nc`. Watches Multical DHW flow via MQTT (bridged from emondhw); if flow > 200 L/h sustained for 10 minutes, forces eBUS DHW charge via `nc localhost 8888` (`write -c 700 HwcSFMode load`). Blocks during Cosy peak (16–19). See `docs/dhw-auto-trigger.md` for full details.

Deploy: `scp scripts/dhw-auto-trigger.sh jack@pi5data:/tmp/ && ssh jack@pi5data "sudo cp /tmp/dhw-auto-trigger.sh /usr/local/bin/ && sudo systemctl restart dhw-auto-trigger"`

## eBUS Polling

`scripts/ebusd-poll.sh` runs on pi5data as a systemd service. Pure shell script using `nc` + `mosquitto_pub`. Reads 25 eBUS values every 30s (+ 16 more every 5 min) via `nc localhost 8888` and publishes to `ebusd/poll/*` MQTT topics. Replaces the previous Python-in-Docker version that reinstalled dependencies on every container restart.

Deploy: `scp scripts/ebusd-poll.sh jack@pi5data:/tmp/ && ssh jack@pi5data "sudo cp /tmp/ebusd-poll.sh /usr/local/bin/ && sudo systemctl restart ebusd-poll"`

## Octopus Energy Integration

Data flows from the `~/github/octopus/` project into heatpump-analysis:

```
Octopus REST API → usage CSVs → merge → usage_merged.csv + weather.json + config.json
                                                              ↓
                                                    octopus.rs loads CSV/JSON
                                                    + emoncms DB for HP state machine
```

### Data sources and coverage
- **Electricity**: Apr 2020 → present (half-hourly, 166k+ records)
- **Gas**: Apr 2020 → Jul 2024 (half-hourly, gas supply ended at HP install)
- **Gap**: 102 days Dec 2023 → Mar 2024 (meter/comms outage, unfillable)
- **Weather**: ERA5-Land daily temps + HDD, bias-corrected by +1.0°C

### Temperature hierarchy
1. **emoncms feed 503093** (Met Office hourly) — used for HP era (Oct 2024+), most accurate
2. **ERA5-Land** (weather.json) — used for gas era, bias-corrected by +1.0°C
   - Derived from 507-day overlap: emoncms reads 1.0°C warmer on average
   - ERA5 overstates HDD by ~14% without correction
   - HDD base: 15.5°C (UK standard)

### Refreshing Octopus data
```bash
cd ~/github/octopus && npm run cli -- refresh
```

### Tariff history
| Period | Electricity | Gas |
|--------|------------|-----|
| Apr 2020 → Dec 2020 | Fix 15.03p | Fix 2.56p |
| Dec 2020 → Dec 2022 | Agile (avg 26.35p) | Variable 2.73→3.83p |
| Dec 2022 → Apr 2023 | Flex 51.38p (crisis) | Variable 10.31p |
| Apr 2023 → Apr 2024 | Flex ~30p | Variable 7–12p |
| Apr 2024 → Nov 2024 | Agile (avg 17.98p) | Variable 5–6p |
| Nov 2024 → Oct 2025 | **Cosy** (off 14.63, mid 29.82, peak 44.74p) | — |
| Oct 2025 → present | **Cosy Fix** (off 14.05, mid 28.65, peak 42.97p) | — |

Cosy time slots: off-peak 04–07, 13–16, 22–00 (9h); mid 00–04, 07–13, 19–22 (12h); peak 16–19 (3h).
82.6% of HP-era electricity lands in off-peak slots.

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
- `503093` (outside_temp) = Met Office hourly, not Arotherm OAT sensor. Reads ~1.0°C warmer than ERA5-Land (507-day overlap). Ground truth for HP era; ERA5 bias-corrected +1.0°C for gas era.
- `512889` (DHW_flag) = dead since Dec 2024
- Solar PV + battery system installed (not yet integrated):
    - 7× Trina 440W panels (TSM-440NEG9RC.27), 3.08 kWp, single string
    - Fox ESS F3600 inverter (3.6kW, dual MPPT — one MPPT input used)
    - Tesla Powerwall 2 (13.5 kWh) + Gateway
    - Commissioned: 19/04/2024, Emlite M24 generation meter

## Hydraulic System

Documented in `docs/hydraulic-analysis.md`:
- Pump software-clamps heating at 860 L/h (14.3 L/min)
- DHW flow rate depends on system resistance (post-clean: 21.3 L/min, before clean: 16.8)
- Y-filter on 35mm primary catches magnetite sludge — cleaned 19 March 2026
- **Idle flow rate is the best early warning** of resistance increase (heating flow is masked by software clamp)
- Post-clean baseline: idle 12.6, heating 14.4, DHW 21.3 L/min

## DHW Cylinder

Documented in `docs/dhw-cylinder-analysis.md`:
- Kingspan Albion 300L twin-coil (both coils in series for HP), internal expansion (air bubble, no ext vessel)
- Measured connection heights (from outside bottom): bottom coil 420mm, T2+cold inlet 540mm, top coil 1020mm, T1+draw-off 1580mm
- Usable hot water (T2 to T1/draw-off): ~165L validated. Dead zone below coils: 59L (20%)
- Eco mode cycle: ~115 min, 3.0 kW, primary ΔT 2.1°C
- Standby loss: 13 W (0.3 kWh/day) — far below 93 W rated spec due to stratification + air bubble insulation
- WWHR effectiveness: 41% at steady state (3.5 min ramp-up), lifts mains from 15.8°C to 25°C
- **Validated stratification model (97% accuracy)**: WWHR water inserts at buoyancy-neutral height (~T2 level, 490mm), not at bottom. Volume from T2 to T1 = 165L; T1 step change observed at 161L drawn.
- T1 drops during early charging (coil-driven destratification) when primary flow temp < T1
- Multical T1/T2 sensors at mid-cylinder positions, not extremes

## Reference Data (config.toml)

- House: HTC 261 W/°C, floor area 180m², solid brick + 2010-standard top floor
- Radiators: 15× Stelrad, total T50 = 25,133W, output calculator with correction factor
- Arotherm spec: COP curve at -3°C (35°C→4.48, 55°C→3.06)
- Gas era: 18,702 kWh/yr gas, 90% boiler, 11.82 kWh/day hot water
- Insulation improved between gas and HP eras (heat/HDD dropped ~4%)
- Solid wall insulation planned but not yet done

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
- `scripts/dhw-auto-trigger.py` is the old Python version with an inverted peak-block bug — **do not deploy it**. The active version is `scripts/dhw-auto-trigger.sh` (shell, deployed to pi5data).
- `scripts/ebusd-poll.sh` uses `nc | head -1` to avoid ebusd TCP connection hanging — without `head -1`, each `nc` call waits 5s for the server to close.

## Boundaries

- Don't change operating state thresholds without re-validating against the full dataset
- Don't mix simulated and real data by default
- Don't commit `heatpump.db` or API keys
- Don't modify `~/github/octopus/` from this project — refresh via `npm run cli -- refresh`
- Keep `HDD_BASE_C` in `octopus.rs` in sync with `HDD_BASE_TEMP` in `analysis.rs` (both read from config now)
- Keep `GAS_DHW_KWH_PER_DAY` and `BOILER_EFFICIENCY` in `octopus.rs` in sync with config.toml `[gas_era]`
- Human-facing docs: `docs/` (Diátaxis style) — see `docs/code-truth/` for derived-from-code docs
- This file (`AGENTS.md`) is the single LLM context source. `docs/code-truth/` is for human comprehension.
- InfluxDB `energy` bucket contains: live MQTT data, 12.2M historical emonhp points from emoncms.org (Oct 2024+), 40M historical emonpi points from phpfina backups (Apr 2024+), 149k outside temperature points from Met Office
- Don't modify monitoring infrastructure from this project — use SSH to emonpi/emondhw/emonhp/pi5data directly
- Don't store credentials in plaintext — use `ak get emon-pi-credentials` at runtime

## Planned Enhancements

See [docs/roadmap.md](docs/roadmap.md) for full details:
- **eBUS integration into analysis** — eBUS is physically connected and publishing data (25+ values every 30s). Not yet used by the Rust analysis tool. Could validate or replace the flow-rate state machine using StatuscodeNum.
- **Solar PV + battery** — system installed, details above. Self-consumption analysis, DHW scheduling to solar peak.
- **Cost analysis subcommand** — tariff data and cost calculations could be a proper Rust subcommand.
