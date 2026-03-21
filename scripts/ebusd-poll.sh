#!/bin/sh
# ebusd-poll — reads eBUS values via ebusctl and publishes to MQTT.
#
# Runs as a systemd service on pi5data. Every 30s reads real-time
# operational data; every 5 min (10th cycle) adds slower energy/COP values.
#
# Replaces the Docker-based Python version that reinstalled dependencies
# on every container restart.

set -u

# ── Configuration ────────────────────────────────────────────────────────
EBUSD_HOST="${EBUSD_HOST:-localhost}"
EBUSD_PORT="${EBUSD_PORT:-8888}"
MQTT_HOST="${MQTT_HOST:-localhost}"
MQTT_USER="${MQTT_USER:-emonpi}"
MQTT_PASS="${MQTT_PASS:-emonpimqtt2016}"
POLL_INTERVAL=30
SLOW_EVERY=10   # every 10th cycle = 5 min

# ── Helpers ──────────────────────────────────────────────────────────────
log() { echo "$(date '+%Y-%m-%d %H:%M:%S') $*"; }

ebus_read() {
    # $1=circuit $2=command $3=flags (optional, e.g. -n)
    circuit="$1" cmd="$2" flags="${3:-}"
    if [ -n "$flags" ]; then
        result=$(echo "read $flags -c $circuit $cmd" | nc -w 5 "$EBUSD_HOST" "$EBUSD_PORT" 2>/dev/null | head -1)
    else
        result=$(echo "read -c $circuit $cmd" | nc -w 5 "$EBUSD_HOST" "$EBUSD_PORT" 2>/dev/null | head -1)
    fi
    # Filter errors and empty results
    case "$result" in
        ERR*|""|"-") return 1 ;;
        *) echo "$result"; return 0 ;;
    esac
}

mqtt_pub() {
    # $1=topic $2=payload
    mosquitto_pub -h "$MQTT_HOST" -u "$MQTT_USER" -P "$MQTT_PASS" -t "$1" -m "$2" 2>/dev/null
}

poll_one() {
    # $1=name $2=circuit $3=command $4=flags
    name="$1" circuit="$2" cmd="$3" flags="${4:-}"
    val=$(ebus_read "$circuit" "$cmd" "$flags") || return
    mqtt_pub "ebusd/poll/$name" "$val"
}

# ── Value definitions ────────────────────────────────────────────────────
poll_fast() {
    # Real-time operational data — every 30s
    poll_one FlowTemp             hmu RunDataFlowTemp
    poll_one ReturnTemp           hmu RunDataReturnTemp
    poll_one StatuscodeNum        hmu RunDataStatuscode -n
    poll_one Statuscode           hmu RunDataStatuscode
    poll_one ElectricPower_W      hmu RunDataElectricPowerConsumption
    poll_one ConsumedPower_kW     hmu CurrentConsumedPower
    poll_one YieldPower_kW        hmu CurrentYieldPower
    poll_one CompressorSpeed      hmu RunDataCompressorSpeed
    poll_one CompressorOutletTemp hmu RunDataCompressorOutletTemp
    poll_one CompressorInletTemp  hmu RunDataCompressorInletTemp
    poll_one TargetFlowTemp       hmu TargetFlowTemp
    poll_one AirInletTemp         hmu RunDataAirInletTemp
    poll_one HighPressure         hmu RunDataHighPressure
    poll_one Fan1Speed            hmu RunDataFan1Speed
    poll_one EEVPosition          hmu RunDataEEVPositionAbs
    poll_one CircPumpPower        hmu RunDataBuildingCPumpPower
    poll_one BuildingCircuitFlow  hmu BuildingCircuitFlow
    poll_one EnergyIntegral       hmu EnergyIntegral
    poll_one FlowPressure         hmu FlowPressure
    poll_one CompressorUtil       hmu CurrentCompressorUtil
    poll_one OutsideTemp          700 DisplayedOutsideTemp
    poll_one Hc1FlowTemp          700 Hc1FlowTemp
    poll_one Hc1FlowTempDesired   700 Hc1ActualFlowTempDesired
    poll_one HwcStorageTemp       700 HwcStorageTemp
    poll_one Hc1PumpStatus        700 Hc1PumpStatus
}

poll_slow() {
    # Energy/COP/mode data — every 5 min
    poll_one CopHc            hmu CopHc
    poll_one CopHwc           hmu CopHwc
    poll_one CopHcMonth       hmu CopHcMonth
    poll_one CopHwcMonth      hmu CopHwcMonth
    poll_one YieldHcDay       hmu YieldHcDay
    poll_one YieldHwcDay      hmu YieldHwcDay
    poll_one TotalEnergyUsage hmu TotalEnergyUsage
    poll_one YieldHc          hmu YieldHc
    poll_one YieldHwc         hmu YieldHwc
    poll_one OutsideTempAvg   700 OutsideTempAvg
    poll_one HwcTempDesired   700 HwcTempDesired
    poll_one HwcOpMode        700 HwcOpMode
    poll_one OpMode           700 OpMode
    poll_one Z1DayTemp        700 Z1DayTemp
    poll_one Hc1HeatCurve     700 Hc1HeatCurve
    poll_one HwcMode          hmu HwcMode
}

# ── Main loop ────────────────────────────────────────────────────────────
counter=0
log "Starting: interval=${POLL_INTERVAL}s, slow every ${SLOW_EVERY} cycles"

while true; do
    counter=$((counter + 1))
    count=0

    poll_fast
    # Count isn't trivial in sh without subshells — just log the cycle
    if [ $((counter % SLOW_EVERY)) -eq 0 ]; then
        poll_slow
        log "[${counter}] Polled fast + slow values"
    else
        log "[${counter}] Polled fast values"
    fi

    sleep "$POLL_INTERVAL"
done
