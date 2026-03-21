#!/bin/sh
# DHW auto-trigger — watches Multical DHW flow via MQTT and forces a DHW
# charge via eBUS when a sustained draw is detected.
#
# Runs as a systemd service on pi5data. Talks to:
#   - Mosquitto (localhost:1883) for Multical flow data (bridged from emondhw)
#   - ebusd (localhost:8888) for heat pump control (Docker, port exposed)
#
# Logic: if DHW flow > 200 L/h for 10 continuous minutes, and we're
# outside Cosy peak (16–19), trigger HwcSFMode=load. One-hour cooldown
# prevents re-triggering.

set -u

# ── Configuration ────────────────────────────────────────────────────────
MQTT_HOST="${MQTT_HOST:-localhost}"
MQTT_USER="${MQTT_USER:-emonpi}"
MQTT_PASS="${MQTT_PASS:-emonpimqtt2016}"
MQTT_TOPIC="${MQTT_TOPIC:-emon/multical/dhw_flow}"

EBUSD_HOST="${EBUSD_HOST:-localhost}"
EBUSD_PORT="${EBUSD_PORT:-8888}"

FLOW_THRESHOLD=200        # L/h — above sink use, catches showers/baths
SUSTAIN_SECONDS=600       # 10 minutes
COOLDOWN_SECONDS=3600     # 1 hour
PEAK_START=16             # Cosy peak starts (local time)
PEAK_END=19               # Cosy peak ends (local time)

FIFO="/tmp/dhw-trigger-fifo"

# ── Helpers ──────────────────────────────────────────────────────────────
log() { echo "$(date '+%Y-%m-%d %H:%M:%S') $*"; }

ebus_write() {
    result=$(echo "$1" | nc -w 5 "$EBUSD_HOST" "$EBUSD_PORT" 2>&1 | head -1)
    log "eBUS: $1 → $result"
    case "$result" in
        done*) return 0 ;;
        *)     return 1 ;;
    esac
}

is_peak() {
    hour=$(date +%-H)
    [ "$hour" -ge "$PEAK_START" ] && [ "$hour" -lt "$PEAK_END" ]
}

cleanup() {
    log "Shutting down"
    rm -f "$FIFO"
    kill 0 2>/dev/null
    exit 0
}

trap cleanup INT TERM

# ── State ────────────────────────────────────────────────────────────────
draw_start=0              # epoch when flow first exceeded threshold (0 = no draw)
last_trigger=0            # epoch of last trigger
triggered=0               # 1 if we've triggered for current draw

# ── Main loop (FIFO so while-read runs in main shell, not subshell) ─────
rm -f "$FIFO"
mkfifo "$FIFO"

mosquitto_sub -h "$MQTT_HOST" -u "$MQTT_USER" -P "$MQTT_PASS" -t "$MQTT_TOPIC" > "$FIFO" &
SUB_PID=$!

log "Starting: threshold=${FLOW_THRESHOLD} L/h, sustain=${SUSTAIN_SECONDS}s, cooldown=${COOLDOWN_SECONDS}s, sub_pid=${SUB_PID}"

while read -r flow; do
    now=$(date +%s)

    # Integer comparison — truncate decimal
    flow_int=$(echo "$flow" | awk '{printf "%d", $1}')

    if [ "$flow_int" -gt "$FLOW_THRESHOLD" ]; then
        # Flow is above threshold
        if [ "$draw_start" -eq 0 ]; then
            draw_start=$now
            log "Draw detected: ${flow} L/h"
        fi

        elapsed=$((now - draw_start))

        if [ "$elapsed" -ge "$SUSTAIN_SECONDS" ] && [ "$triggered" -eq 0 ]; then
            cooldown_elapsed=$((now - last_trigger))

            if [ "$cooldown_elapsed" -le "$COOLDOWN_SECONDS" ] && [ "$last_trigger" -gt 0 ]; then
                log "COOLDOWN: ${elapsed}s sustained but only ${cooldown_elapsed}s since last trigger"
            elif is_peak; then
                log "BLOCKED by peak tariff (${PEAK_START}:00–${PEAK_END}:00): ${flow} L/h for ${elapsed}s"
            else
                log "TRIGGERING DHW charge: ${flow} L/h sustained for ${elapsed}s"
                if ebus_write "write -c 700 HwcSFMode load"; then
                    triggered=1
                    last_trigger=$now
                fi
            fi
        fi
    else
        # Flow dropped below threshold
        if [ "$draw_start" -gt 0 ]; then
            elapsed=$((now - draw_start))
            log "Draw ended after ${elapsed}s (triggered=${triggered})"
            draw_start=0
            triggered=0
        fi
    fi
done < "$FIFO"
