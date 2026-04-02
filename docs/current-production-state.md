# Current Production State

_Last updated: 2026-04-02_

This document is the compact reference for **what is live on pi5data now**.

For strategy and rationale, see:
- `heating-plan.md`
- `dhw-plan.md`

For live queries and historical reconstruction, see:
- `live-queries.md`
- `history-evidence-workflows.md`

For code locations and module structure, see:
- `code-truth/README.md`
- `code-truth/REPOSITORY_MAP.md`
- `code-truth/ARCHITECTURE.md`

For secrets handling, see:
- `../deploy/SECRETS.md`

## Services and ownership

| Service / system | Role | Port | Source of truth |
|---|---|---|---|
| `adaptive-heating-mvp` | Live space-heating controller | 3031 | `heating-plan.md`, `model/adaptive-heating-mvp.toml` |
| `z2m-hub` | Mobile dashboard, DHW tracking/boost, heating mode proxy | 3030 | `dhw-plan.md`, separate repo `~/github/z2m-hub/` |
| `ebusd` | eBUS read/write transport | 8888 | pi5data runtime |
| InfluxDB | sensor/history store | 8086 | pi5data Docker |
| Telegraf | ingest pipeline into InfluxDB | — | pi5data Docker |

## Heating: live state

| Item | Current production state |
|---|---|
| Control mode | Adaptive Heating V2 live |
| Objective | Leather 20–21°C during 07:00–23:00 at minimum cost |
| Outer loop | 15 min thermal-solver cycle |
| Inner loop | 60 s curve nudge on `Hc1ActualFlowTempDesired` |
| Controller actuator | `Hc1HeatCurve` via VRC 700 circuit `700` |
| Startup operating mode | `Z1OpMode=night` |
| Baseline restore on shutdown | `Z1OpMode=auto`, `Hc1HeatCurve=0.55` |
| Heat curve exponent in config | `1.25` |
| Overnight strategy | Phase 2 latest-start planner active |
| Open-Meteo forecast | Designed / configured, not yet fully operationally validated |
| Door sensors | Hardware in hand, not yet integrated live |
| Default historical review command | `date -u`; then `cargo run --bin heatpump-analysis -- heating-history` |
| Named regression anchor | `heating-history --since 2026-04-02T00:00:00Z --until 2026-04-02T09:00:00Z` → likely preheat start 03:06, DHW overlap 04:15–05:37, comfort miss 05:35–09:05, Leather 19.63°C by 09:00 |

## DHW: live state

| Item | Current production state |
|---|---|
| Trigger owner today | VRC 700 timer windows + `HwcStorageTemp` hysteresis |
| Timer windows | `05:30–07:00`, `13:00–15:00`, `22:00–00:00` |
| DHW target | 45°C |
| Authoritative comfort signal | Multical `T1` |
| Completion truth | `HwcStorageTemp >= T1_at_charge_start` crossover |
| Live tracking | `z2m-hub` polls every 10 s |
| Remaining litres | z2m-hub model + Influx-backed recommended full litres |
| Manual boost path | `POST /api/dhw/boost` via z2m-hub |
| Planned next step | T1-based charge triggering instead of pure VRC hysteresis |
| Default historical review command | `date -u`; then `cargo run --bin heatpump-analysis -- dhw-history` |
| Named regression anchor | `dhw-history --since 2026-04-02T05:00:00Z --until 2026-04-02T08:00:00Z` → completed 36 min top-up; later `T1` stayed ~45°C while `HwcStorageTemp` fell to 27°C with ~118 L still remaining |

## Deployment paths on pi5data

The live controller is deployed from a checkout at `/home/jack/adaptive-heating-mvp` on pi5data.

| Component | Location |
|---|---|
| Binary | `/home/jack/adaptive-heating-mvp/target/release/adaptive-heating-mvp` |
| Config | `/home/jack/adaptive-heating-mvp/model/adaptive-heating-mvp.toml` |
| Thermal geometry | `/home/jack/adaptive-heating-mvp/data/canonical/thermal_geometry.json` |
| Systemd unit | `/etc/systemd/system/adaptive-heating-mvp.service` |
| Secrets env file | `/etc/adaptive-heating-mvp.env` |
| Runtime state | `/home/jack/.local/state/adaptive-heating-mvp/state.toml` |
| JSONL action log | `/home/jack/.local/state/adaptive-heating-mvp/actions.jsonl` |

## Health checks

### Heating

```bash
systemctl status adaptive-heating-mvp
journalctl -u adaptive-heating-mvp -n 50 --no-pager
curl -s http://pi5data:3031/status
cargo run --bin adaptive-heating-mvp -- --config model/adaptive-heating-mvp.toml status
```

### DHW / dashboard

```bash
cargo run --bin heatpump-analysis -- dhw-live-status          # structured / LLM-friendly
cargo run --bin heatpump-analysis -- dhw-live-status --human  # operator-friendly
curl -s http://pi5data:3030/api/hot-water
curl -s http://pi5data:3030/api/dhw/status
```

### eBUS / secrets / data path

```bash
echo 'read -c 700 Hc1HeatCurve' | nc -w 2 localhost 8888
sudo test -f /etc/adaptive-heating-mvp.env && echo ok
```

## Safe fallback

### Heating controller off, return to autonomous VRC 700

```bash
echo 'write -c 700 Z1OpMode 1' | nc -w 2 localhost 8888
echo 'write -c 700 Hc1HeatCurve 0.55' | nc -w 2 localhost 8888
```

### DHW boost stuck

```bash
echo 'write -c 700 HwcSFMode auto' | nc -w 2 localhost 8888
```

## Known live caveats

- VRC 700 is treated as a black-box actuator; inner-loop readback is authoritative.
- `StatuscodeNum` is not reliable for DHW detection.
- `CurrentCompressorUtil` has shown negative values; do not trust it for control limits.
- `z2m-hub` is a separate repo/service even though it is part of the live DHW/heating operating stack.
