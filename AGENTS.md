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
| Adaptive heating MVP | `cargo run --bin adaptive-heating-mvp -- --config model/adaptive-heating-mvp.toml run` |
| Adaptive heating status | `cargo run --bin adaptive-heating-mvp -- --config model/adaptive-heating-mvp.toml status` |
| Adaptive heating restore | `cargo run --bin adaptive-heating-mvp -- --config model/adaptive-heating-mvp.toml restore-baseline` |


`--apikey` only needed for `feeds` and `sync`. Two binaries: use `cargo run --bin heatpump-analysis` for thermal commands. Three binaries total: `adaptive-heating-mvp` is the live pilot controller.

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
model/adaptive-heating-mvp.toml → Adaptive heating MVP config
deploy/adaptive-heating-mvp.service → systemd unit for pi5data
```

## Monitoring Infrastructure

| Device | IP | Role |
|---|---|---|
| emonpi | 10.0.1.117 | EmonPi2 (3× CT), DS18B20, Z2M (21 Zigbee devices) |
| emonhp | 10.0.1.169 | Heat meter + SDM120 → emoncms.org |
| emondhw | 10.0.1.46 | Multical DHW meter |
| pi5data | 10.0.1.230 | Central hub: Docker (Mosquitto, InfluxDB, Telegraf, Grafana, ebusd) + systemd (z2m-hub :3030, adaptive-heating-mvp :3031) |

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
- **DHW**: 300L Kingspan Albion, usable 177–183L from full charge (243L geometric max, ~75% plug flow efficiency), 45°C target, eco/normal manual seasonal switch. CylinderChargeHyst=5K (triggers at 40°C). HwcStorage crossover (≥ T1_pre) = definitive "full" signal. See `docs/dhw-cylinder-analysis.md`.
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
- **DHW inflection detector** (`scripts/dhw-inflection-detector.py`) deployed to pi5data `/usr/local/bin/`. Weekly cron (Sunday 3am) analyses draws at 2s resolution, writes inflection measurements to InfluxDB (`dhw_inflection`) and recommended capacity to `dhw_capacity`. z2m-hub v0.2.0 autoloads `recommended_full_litres` on startup. Run manually: `uv run --with requests python scripts/dhw-inflection-detector.py --days 14 --verbose`.
- **Adaptive heating MVP** deployed on pi5data as systemd service. HTTP API on port 3031. Mobile controls proxied via z2m-hub (:3030). Config: `model/adaptive-heating-mvp.toml`. Spec: `docs/adaptive-heating-mvp.md`. Kill switch restores known-good baseline.
- **V1 pilot findings** (31 Mar–1 Apr 2026): Bang-bang control (±0.10 curve every 15 min) ping-ponged 0.55→0.10→1.00. Leather τ=15h means 15-min adjustments are noise. VRC 700 curve floor is 0.10. Null-read bug fixed.
- **VRC 700 heat curve formula**: `flow = setpoint + curve × (setpoint - outside)^1.25` (RMSE 0.83°C from manual + 17 pilot data points, curves 0.10–1.00, outside 12–16°C). Inverse: `curve = (target_flow - setpoint) / (setpoint - outside)^1.25`. Online error correction handles the ±1°C residual.
- **V2 design** (`docs/adaptive-heating-v2-design.md`): Model-predictive control using thermal equilibrium solver + heat curve formula + Open-Meteo 24h forecast (temp, solar, humidity). Controller follows the hourly forecast trajectory — curve profile rises/falls with predicted outside temp and solar gain. Overnight planner calculates latest heating start time for 20°C by 07:00. Event-driven recalculation for unexpected deviations only.
- **Real control objective**: Leather 20–21°C during waking hours (07:00–23:00) at minimum cost. Overnight temp is a free variable. DHW charges only during Cosy windows when cylinder needs it.
- **VRC 700 architecture**: All inputs (curve, setpoint, limits) produce one output: `Hc1ActualFlowTempDesired`. This goes directly to the HMU via decoded SetMode (D1C encoding). VWZ AI is hydraulic only (valve/pump), not in the flow temp control path.
- **eBUS coverage**: 247 read + 216 write defs for VRC 700, 117 read + 14 passive for HMU, zero decoded for VWZ AI (raw bytes in grab buffer only). ebusd `--enablehex` and `--enabledefine` are on. `grab result all` shows all raw bus traffic including undecoded VWZ AI messages.
- **eBUS bus hierarchy**: VRC 700 sends SetMode to HMU every ~30s with flow temp demand (D1C encoding). VWZ AI gets separate messages with zeros for flow temp — it’s hydraulic only (valve/pump), not in the flow temp control path. VWZ AI can operate standalone without the VRC 700 (has own heat curve, setpoints, DHW control via its control panel).
- **Future option**: SetModeOverride to HMU to bypass VRC 700 entirely and set flow temp directly. Message format is decoded. Requires disabling or outpacing the 700's 30-second writes.
- **Vaillant manuals**: 10 curated PDFs in `C:\Users\jackc\OneDrive\Library\` (Vaillant filenames start with `arotherm`, `monoblock`, `vrc`, or `0020262548`). Pruned from 16 originals on 1 Apr 2026 — removed duplicates, superseded spec sheets, wrong-model VWZ AI, boiler-only schematics, and marketing fluff. Full inventory:
  - `arotherm-plus-vwl-35-75-a-s2-installation-operation-manual-0020330791-03-2806789.pdf` — **Main aroTHERM Plus manual** (operating + installation + maintenance). Covers our VWL 55/6 A S2.
  - `arotherm-plus-installer-quick-guide-2848532.pdf` — Installer quick reference (44 pages, planning + commissioning)
  - `arotherm-plus-tech-sheet-nov-2024-2965654.pdf` — Spec sheet (latest, Nov 2024)
  - `arotherm-plus-system-schematics-setup-2831195.pdf` — Top 10 schematics with VRC 700/sensoCOMFORT setup guidance (76 pages)
  - `system-schematics-for-the-arotherm-plus-2831194.pdf` — Complete schematic collection, all configurations (99 pages)
  - `monoblock-heat-pump-system-vwz-ai-heat-pump-appliance-interface-2685948.pdf` — **VWZ AI operating + installation**. Documents standalone mode (p22: menu functions without system control). 44 pages.
  - `0020262548-01vrc7006-wired-thermostat-operating-instructions-2652464.pdf` — VRC 700 user operating guide (party mode, away, boost, eco)
  - `vrc-700-installation-instructions-1968307.pdf` — **VRC 700 installer guide**. Has heat curve chart (p15) and complete settings table with min/max/defaults (p28–29). 40 pages.
  - `vrc-700-tech-sheet-apr-20-web-1741263.pdf` — VRC 700 features/specs summary
  - `vrc-720-gb-0020287900-00-1714662.pdf` — **VRC 720 sensoCOMFORT** operating + installation. Same eBUS address (0x15), drop-in replacement for VRC 700. Up to 5 zones. Has dew point monitoring.
  - Vaillant simulators: VRC 700 `https://simulatorvaillant.com/VRC_700_6/gb/`, sensoCOMFORT `https://simulatorvaillant.com/VRC_720_2/gb/`.
- z2m-hub patched to proxy adaptive-heating-mvp mode controls. Phone dashboard at `http://pi5data:3030` has heating mode buttons.
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
