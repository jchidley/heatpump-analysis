# DHW Auto-Trigger: Emergency Cylinder Recharge on Prolonged Draw

## Problem

The DHW cylinder is recharged 3× daily during off-peak tariff periods (Cosy: 04–07, 13–16, 22–00). This works well for normal use — a 5-minute shower draws ~36 litres from a 300L cylinder and T1 barely drops.

The problem arises when someone has a very long shower or a bath (10+ minutes, 60+ litres), depleting the upper cylinder significantly. If a second person then wants hot water, they have to wait up to 99 minutes for the next scheduled eco mode reheat.

## Solution

A shell script on pi5data watches the Multical DHW flow rate via MQTT (bridged from emondhw). When it detects a prolonged draw, it forces an immediate DHW charge cycle via eBUS, so the heat pump starts replenishing the cylinder while the draw is still happening.

The HP takes 2–3 minutes to start delivering heat after the command. By the time a 10-minute draw finishes, the HP has been running for ~3 minutes and continues charging until the cylinder reaches target temperature. The second person gets hot water 30–40 minutes sooner than waiting for the next scheduled cycle.

## How it works

```
Multical (emon/multical/dhw_flow)
    │
    │  MQTT bridge (emondhw → pi5data), every ~2 seconds
    ▼
dhw-auto-trigger.sh (systemd on pi5data)
    │  uses mosquitto_sub + FIFO + shell state machine
    │
    │  flow > 200 L/h for 10 minutes?
    ▼
nc localhost 8888 → "write -c 700 HwcSFMode load"
    │
    │  ebusd (Docker) → eBUS → Arotherm VWZ controller
    ▼
Heat pump switches to DHW mode
    │
    │  Charges until cylinder reaches target (45°C)
    ▼
Returns to normal operation automatically
```

### Trigger criteria

| Parameter | Value | Rationale |
|-----------|-------|-----------|
| Flow threshold | 200 L/h | Above sink use (~100–150 L/h), catches showers (~430 L/h) and baths |
| Sustain time | 10 minutes | Short showers (5 min) don't trigger — they don't deplete the cylinder enough to matter |
| Cooldown | 1 hour | Prevents repeated triggers from the same event or closely spaced draws |
| **Peak block** | **16:00–19:00** | Never triggers during Octopus Cosy peak (42.97p/kWh vs 14.05p off-peak). Battery is discharging during peak so HP would draw from grid at 3× cost. Logged but blocked. |

### What it doesn't do

- **Normal scheduled DHW cycles are unaffected** — the script only fires `HwcSFMode load` which triggers a one-off charge. The HP's own schedule continues as before.
- **Sink use is ignored** — flow is below 200 L/h and too brief to sustain 10 minutes.
- **Short showers are ignored** — a 5-minute shower ends before the 10-minute threshold. The cylinder barely drops (0.1°C on T1) and doesn't need emergency recharging.
- **Never fires during peak tariff (16:00–19:00)** — Cosy peak is 42.97p/kWh, 3× off-peak. The battery is discharging during peak so the HP would draw from grid. The trigger is logged as "BLOCKED by peak tariff" but not acted on. If someone has a long shower during peak, the next person waits for the scheduled off-peak reheat.

### Cost impact

The cylinder needs reheating regardless — the trigger just shifts *when* the same kWh is consumed, not *how much*. The extra cost is only the tariff differential, if any:

| Scenario | Extra cost per trigger | Notes |
|----------|----------------------|-------|
| Battery has charge (most common) | **~0p** | Runs on stored off-peak electricity. ~10% round-trip loss on Powerwall = 0.3 kWh × 14.05p = 4p |
| Mid-rate, battery depleted | **~44p** | Differential: (28.65 − 14.05) × 3 kWh. Uncommon — battery usually has charge |
| Off-peak hours (22–00, 04–07, 13–16) | **0p** | Already at the cheapest rate |
| Peak (16–19) | **blocked** | Script never fires during Cosy peak |

Expected frequency: ~5 triggers/week (normal), ~7/week (son home from university).

Most long showers happen in the evening (22:00+ = off-peak) or morning (battery full from overnight charge). Realistic annual extra cost is **£0–£50**, likely close to zero. The previous scheduled reheat simply becomes shorter or gets skipped because the cylinder is already at temperature.

## eBUS command

The `HwcSFMode` (Hot Water Cylinder Special Function Mode) on the VRC 700 controller supports:

| Value | Effect |
|-------|--------|
| 0 = auto | Normal scheduled operation |
| 1 = ventilation | — |
| 2 = party | — |
| 3 = veto | — |
| 4 = onedayaway | — |
| 5 = onedayathome | — |
| **6 = load** | **Force immediate DHW cylinder charge** |

`load` is equivalent to pressing the "boost" button on the SensoCOMFORT controller. The HP switches to DHW mode, charges the cylinder to target temperature, then returns to normal operation automatically.

```bash
# Force DHW charge (from pi5data host):
echo "write -c 700 HwcSFMode load" | nc -w 5 localhost 8888 | head -1

# Or via docker exec:
docker exec ebusd ebusctl write -c 700 HwcSFMode load

# Check status:
echo "read -c hmu RunDataStatuscode" | nc -w 5 localhost 8888 | head -1
# → "Warm_Water_Compressor_active" (when charging)
# → "Heating_Compressor_active" (when done, back to heating)

# Reset to auto (optional — HP resets automatically after charge):
echo "write -c 700 HwcSFMode auto" | nc -w 5 localhost 8888 | head -1
```

Tested and confirmed working on ebusd 26.1, Arotherm Plus 5kW, 20 March 2026. The HP switched from heating to DHW mode within seconds of the command.

## Deployment

### Architecture

The script runs on **pi5data** (not emondhw) because:
- ebusd runs in Docker on pi5data — the script needs to write eBUS commands
- Multical DHW flow data reaches pi5data via the emondhw→pi5data MQTT bridge
- pi5data is the central hub for all automation and monitoring

### Prerequisites

- ebusd Docker container on pi5data with port 8888 exposed
- Mosquitto on pi5data receiving bridged `emon/multical/dhw_flow` from emondhw
- Host packages: `mosquitto-clients`, `netcat-openbsd` (installed on all emon devices)

### Files

| File | Location on pi5data | Purpose |
|------|-------------------|---------|
| `scripts/dhw-auto-trigger.sh` | `/usr/local/bin/dhw-auto-trigger.sh` | Main script (shell) |
| `scripts/dhw-auto-trigger.service` | `/etc/systemd/system/dhw-auto-trigger.service` | Systemd unit |

### Install / update

```bash
# From the heatpump-analysis project:
scp scripts/dhw-auto-trigger.sh jack@pi5data:/tmp/
scp scripts/dhw-auto-trigger.service jack@pi5data:/tmp/
ssh jack@pi5data "sudo cp /tmp/dhw-auto-trigger.sh /usr/local/bin/ && \
  sudo chmod +x /usr/local/bin/dhw-auto-trigger.sh && \
  sudo cp /tmp/dhw-auto-trigger.service /etc/systemd/system/ && \
  sudo systemctl daemon-reload && \
  sudo systemctl enable --now dhw-auto-trigger"
```

### Logs

```bash
ssh jack@pi5data "journalctl -u dhw-auto-trigger --since '1 hour ago' --no-pager"
```

Example log output:
```
2026-03-21 22:45:30 Draw detected: 430 L/h
2026-03-21 22:55:30 TRIGGERING DHW charge: 430 L/h sustained for 600s
2026-03-21 22:55:30 eBUS: write -c 700 HwcSFMode load → done
2026-03-21 23:02:45 Draw ended after 1035s (triggered=1)
```

### Service management

```bash
ssh jack@pi5data "systemctl status dhw-auto-trigger"      # check status
ssh jack@pi5data "sudo systemctl restart dhw-auto-trigger" # restart
ssh jack@pi5data "sudo systemctl stop dhw-auto-trigger"    # stop
```

## Configuration

All tunables are environment variables or constants at the top of `dhw-auto-trigger.sh`:

| Constant | Default | Notes |
|----------|---------|-------|
| `FLOW_THRESHOLD` | 200 | L/h — raise if sink use triggers false positives |
| `SUSTAIN_SECONDS` | 600 | 10 minutes — lower for faster response, raise to avoid unnecessary triggers |
| `COOLDOWN_SECONDS` | 3600 | 1 hour — prevents repeated triggers |
| `PEAK_START` | 16 | Local time — start of Cosy peak block |
| `PEAK_END` | 19 | Local time — end of Cosy peak block |
| `MQTT_TOPIC` | `emon/multical/dhw_flow` | Multical flow rate topic |
| `EBUSD_HOST` | `localhost` | ebusd TCP host |
| `EBUSD_PORT` | `8888` | ebusd TCP port (exposed from Docker) |

### Tuning guidance

- If the trigger fires too often (e.g., on normal showers that don't need it), increase `SUSTAIN_SECONDS` to 900 (15 min).
- If the second person still doesn't get hot water fast enough, decrease `SUSTAIN_SECONDS` to 300 (5 min) — but this will trigger on most showers and the HP will run at mid-rate more often.
- Monitor via journalctl and the emonhp flow rate data (feed 503100) — you'll see the HP switch to DHW flow rate (~21 L/min) when the trigger fires.

## Implementation notes

### Shell over Python

The original implementation was a Python script using `paho-mqtt`. It was replaced with a shell script (March 2026) because:
- The only dependencies are `mosquitto_sub` and `nc`, both already installed on all emon devices
- No Python runtime, no pip packages, no Docker container needed
- ~90 lines of `/bin/sh` vs ~130 lines of Python
- Runs directly as a systemd service on the host

### FIFO pattern

The script uses a named FIFO (`mkfifo`) to avoid the shell subshell-in-pipe problem: `cmd | while read` runs the loop in a subshell where variable assignments (state tracking) are lost between iterations. Reading from a FIFO (`while read < fifo`) keeps the loop in the main shell.

### nc connection handling

ebusd's TCP protocol doesn't close the connection after responding. Without `head -1`, each `nc` call waits for the full 5-second timeout. The `| head -1` grabs the response line and closes the pipe immediately (2ms vs 5s).

## Legacy

- `scripts/dhw-auto-trigger.py` — the old Python version. **Do not deploy.** It has an inverted peak-block bug where `run_ebus()` is called inside the `in_peak` branch instead of the `not in_peak` branch, meaning it triggers when it should block and vice versa. Kept in the repo for reference only.
- ebusd was previously on emondhw as a local install. It was moved to pi5data Docker (documented in `heating-monitoring-setup.md`). The trigger script followed — it now runs on pi5data where ebusd is reachable on localhost:8888.
