# AGENTS.md

## What This Is

Rust CLI + Python thermal model for heat pump analysis. Vaillant Arotherm Plus 5kW at 6 Rhodes Avenue, London N22 7UT.

- emoncms dashboard: `https://emoncms.org/app/view?name=MyHeatpump&readkey=1b00410c57d5df343ede7c09e6aab34f`
- Read API key (read-only): `1b00410c57d5df343ede7c09e6aab34f`

## Commands

| Task | Command |
|------|---------|
| Build | `cargo build` |
| Sync data | `cargo run -- --apikey KEY sync` |
| Analyse (7 days) | `cargo run -- --days 7 summary` |
| Analyse (all data) | `cargo run -- --all-data all` |
| Octopus summary | `cargo run -- octopus` |
| Gas vs HP | `cargo run -- --all-data gas-vs-hp` |
| Overnight optimizer | `cargo run -- --all-data overnight` |
| Thermal rooms | `cargo run --bin heatpump-analysis -- thermal-rooms` |
| Thermal connections | `cargo run --bin heatpump-analysis -- thermal-connections` |
| Thermal analyse | `cargo run --bin heatpump-analysis -- thermal-analyse --config model/thermal-config.toml` |
| Thermal equilibrium | `cargo run --bin heatpump-analysis -- thermal-equilibrium --outside 0 --mwt 40` |
| Thermal moisture | `cargo run --bin heatpump-analysis -- thermal-moisture --config model/thermal-config.toml` |
| Thermal calibrate | `cargo run --bin heatpump-analysis -- thermal-calibrate --config model/thermal-config.toml` |
| Thermal validate | `cargo run --bin heatpump-analysis -- thermal-validate --config model/thermal-config.toml` |
| Thermal fit diagnostics | `cargo run --bin heatpump-analysis -- thermal-fit-diagnostics --config model/thermal-config.toml` |
| Thermal operational | `cargo run --bin heatpump-analysis -- thermal-operational --config model/thermal-config.toml` |
| Thermal snapshot | `cargo run --bin heatpump-analysis -- thermal-snapshot export --config model/thermal-config.toml --signoff-reason "reason" --approved-by-human` |
| Regression check | `bash scripts/thermal-regression-ci.sh` |


`--apikey` only needed for `feeds` and `sync`. Two binaries: use `cargo run --bin heatpump-analysis` for thermal commands.

## Architecture

See `docs/code-truth/` for detailed architecture, patterns, and decisions.

```
config.toml          → Domain constants, thresholds, feed IDs, radiators
src/analysis.rs      → State machine + Polars queries
src/thermal.rs       → Thin facade (re-exports public entry points)
src/thermal/         → 15 submodules: config, geometry, physics, solar, wind, calibration,
                       validation, diagnostics, operational, artifact, snapshot,
                       display, error, influx, report
src/overnight.rs     → Overnight strategy backtest

data/canonical/thermal_geometry.json → Room geometry (single source of truth, consumed by Rust + Python)
model/thermal-config.toml → Thermal model config (InfluxDB, test nights, bounds)
```

## Monitoring Infrastructure

| Device | IP | Role |
|---|---|---|
| emonpi | 10.0.1.117 | EmonPi2 (3× CT), DS18B20, Z2M (21 Zigbee devices) |
| emonhp | 10.0.1.169 | Heat meter + SDM120 → emoncms.org |
| emondhw | 10.0.1.46 | Multical DHW meter |
| pi5data | 10.0.1.230 | Central hub: Docker (Mosquitto, InfluxDB, Telegraf, Grafana, ebusd) + systemd |

MQTT credentials: `emonpi` / `emonpimqtt2016`. Z2M: `ws://emonpi:8080/api` (no auth).

See `heating-monitoring-setup.md` for full details, `docs/emon-installation-runbook.md` for rebuild procedures.

## Key Domain Model

Operating states classified by flow rate (5kW fixed pump = 14.3 L/min):
- **Heating**: flow_rate 14.0–14.5, DT > 0, heat > 0
- **DHW**: flow_rate ≥ 15.0 (enter) / < 14.7 (exit), DT > 0, heat > 0
- **Defrost**: heat ≤ 0 OR DT < −0.5
- **Idle**: elec ≤ 50W

Thresholds in `config.toml` `[thresholds]`. Tightened from 16.0/15.0 to 15.0/14.7 in Mar 2026 (y-filter sludge). See `docs/hydraulic-analysis.md`.

### eBUS state classification (Rust thermal model)

`thermal-operational` uses `BuildingCircuitFlow` (L/h): > 900 = DHW, 780–900 = heating, < 100 = off.

**⚠ `StatuscodeNum` is unreliable for DHW detection.** Code 134 appears during both off/frost standby AND active DHW. Never mean-aggregate status codes — use `last()`.

## Key Facts

- **13 rooms**, all sensored. 12× SNZB-02P (v2.2.0) + 1 emonth2 (leather). See `docs/house-layout.md`.
- **15 radiators**, no TRVs. Kitchen and Landing have no radiator. Sterling rad OFF.
- **House**: HTC 261 W/K, 180m², 1930s solid brick + 2010 loft. HP maxes out at ~2°C outside.
- **Cosy tariff**: THREE windows (04:00–07:00, 13:00–16:00, 22:00–00:00). Battery effective rate 14.63p/kWh.
- **Overnight**: 19°C setback 00:00–04:00. DHW windows: 05:30–07:00, 13:00–15:00, 22:00–00:00. See `docs/overnight-strategy-analysis.md`.
- **DHW**: 300L Kingspan Albion, usable 161L, 45°C target, eco/normal manual seasonal switch. CylinderChargeHyst=5K (triggers at 40°C). See `docs/dhw-cylinder-analysis.md`.
- **DHW cylinder sensors**: T1 (`emon/multical/dhw_t1`) = cylinder top / hot out. T2 (`emon/multical/dhw_t2`) = mains inlet / cold in. VR 10 NTC in dry pocket above bottom coil (`ebusd/poll/HwcStorageTemp`) = what VRC 700 uses for charging decisions. See `docs/dhw-fixes.md`.
- **DHW system**: 3 eBUS devices — HMU (outdoor unit), VWZ AI (indoor unit, has SP1 cylinder sensor), VRC 700 (controller, scheduling brain). See `docs/vrc700-settings-audit.md`.
- **⚠ eBUS timer encoding**: Never use `00:00` as a timer end time — use `-:-` instead. TTM byte `0x00` = start of day (not end). Byte `0x90` = `-:-` = "until end of day". A window with end < start is silently rejected by the VRC 700. `HwcSFMode` can get stuck on `load` after boost — monitor and reset to `auto`. See `docs/vrc700-settings-audit.md`.
- **eBUS control flow**: VRC 700 sends SetMode to VWZ AI (not HMU directly). VWZ AI translates to 1280 real-time parameter messages to HMU. All write commands go to VRC 700 (`-c 700`). Direct HMU writes get overwritten within 10s. See `docs/pico-ebus-plan.md`.
- **Thermal model**: calibrated Night 1/Night 2 (24-26 Mar 2026). Cd=0.20, landing ACH=1.30. See `docs/room-thermal-model.md`.
- **Annual saving**: £565 (46%) vs gas combi at current Cosy tariff.
- **Octopus data**: `~/github/octopus/` — refresh via `cd ~/github/octopus && npm run cli -- refresh`

## Feed Notes

- `503101` (indoor_temp) = emonth2 in **Leather room only**, not whole-house
- `503093` (outside_temp) = Met Office hourly. For real-time, prefer `ebusd/poll/OutsideTemp` (30s)
- `512889` (DHW_flag) = dead since Dec 2024

## Gotchas

- All domain constants in `config.toml` — edit there, not in code
- `gaps.rs` bypasses `db.rs` — writes to SQLite directly. `fill_gap_interpolate()` has hardcoded feed IDs
- `ERA5_BIAS_CORRECTION_C` is a Rust constant in octopus.rs, not in config.toml
- `--all-data` start timestamp hardcoded in `resolve_time_range()`, duplicates config.toml value
- Polars pinned to 0.46 (0.53 available) — untested on newer versions
- Thresholds are 5kW-specific — 7kW model's heating rate (20 L/min) overlaps 5kW DHW rate
- Two HDD base temps: 15.5°C (UK standard) vs 17°C (gas-era regression)
- `octopus.rs` reads from `~/github/octopus/data/` — path hardcoded
- Radiator T50 values duplicated in `config.toml` (analysis.rs) AND `thermal_geometry.json` (thermal.rs) — keep in sync
- SNZB-02P v2.1.0 bug: readings freeze at power-on value. v2.2.0 fixes it. Verify readings vary.
- Bathroom sensor was in airing cupboard until 25 Mar 2026 21:00 — historical data reads ~3°C high
- `emon/heatpump/heatmeter_FlowRate` reads ~1 L/min constantly — DHW circuit meter, useless for state classification. Use `BuildingCircuitFlow`.
- PV calibration 0.087 is for sloping plane, ÷1.4 for vertical. P3 CT reads 6.7kW for 3.08kWp array (includes Powerwall).
- Conservatory excluded from thermal scoring (30m² glass, sub-hour time constant). Landing excluded (chimney model wrong for heating).
- Two binaries — use `cargo run --bin heatpump-analysis -- ...` for thermal commands
- DHW auto-trigger removed Mar 2026. `scripts/dhw-auto-trigger.py` is buggy legacy — do not deploy. DHW boost via z2m-hub.
- `cosy-scheduler` binary removed from pi5data (2026-03-30). Source in `src/bin/cosy-scheduler.rs` kept for reference. Do not deploy.
- `ebusd-poll.sh` uses `nc | head -1` to avoid ebusd TCP hanging

## Boundaries

- Don't change operating state thresholds without re-validating full dataset
- Don't mix simulated and real data by default
- Don't commit `heatpump.db` or API keys
- Don't modify `~/github/octopus/` from this project
- Don't modify monitoring infrastructure from here — use SSH to devices directly
- Don't tune Cd or landing ACH independently — jointly calibrated
- Thermal model: `thermal_geometry.json` is source of truth for rooms/geometry (consumed by Rust + Python). `config.toml` radiators must match.
- Rust thermal outputs are authoritative when command exists; Python for exploratory only
