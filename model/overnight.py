"""
Overnight heating strategy optimizer for 6 Rhodes Avenue.

Uses the calibrated thermal model (house.py) to simulate room temperatures
through various overnight heating strategies, combined with weather forecast
data from Open-Meteo, to find the cheapest schedule that achieves target
temperatures by morning.

Decision variables (15-minute resolution):
  - T_off:  when to turn heating off (evening)
  - T_dhw:  when to start DHW cycle (within Cosy window)
  - DHW mode: normal (~60 min) or eco (~115 min)
  - T_heat: when to restart space heating

The algorithm:
  1. Fetch hourly temperature forecast from Open-Meteo for tonight/tomorrow
  2. Read current room temperatures from Zigbee sensors (InfluxDB)
  3. For each candidate schedule, simulate the full thermal evolution:
     - Cooling phase: multi-room model with thermal mass, inter-room exchange
     - DHW phase: HP serves cylinder (no space heating), rooms continue cooling
     - Recovery phase: HP serves radiators at weather-compensated MWT
  4. Score: electricity cost (Cosy 14.05p vs blended ~17p), constrained by
     all scored rooms ≥ target temp at 07:00
  5. Report optimal schedule + predicted room temps

Usage:
    uv run --with influxdb-client --with numpy --with scipy \\
        python model/overnight.py [--target 19.5] [--evening-off 21:00]

Requires: house.py in same directory, InfluxDB accessible on pi5data.
"""

import sys
import json
import math
import urllib.request
from dataclasses import dataclass
from datetime import datetime, timezone, timedelta
from pathlib import Path

# Import thermal model components from house.py
sys.path.insert(0, str(Path(__file__).parent))
from house import (
    build_rooms, build_connections, build_doorways, build_sensor_map,
    estimate_thermal_mass, room_energy_balance,
    INFLUX_URL, INFLUX_TOKEN, INFLUX_ORG, INFLUX_BUCKET,
)


# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

# Tariff (Octopus Cosy + Powerwall effective rates)
COSY_RATE = 14.05       # p/kWh during Cosy windows
BLENDED_RATE = 17.0     # p/kWh effective rate outside Cosy (battery smoothed)

# Cosy morning window (local time)
COSY_START_H = 4        # 04:00
COSY_END_H = 7          # 07:00

# DHW parameters (measured)
DHW_NORMAL_MINUTES = 60     # Normal mode duration
DHW_ECO_MINUTES = 115       # Eco mode duration
DHW_HEAT_KWH = 6.0          # Total heat delivered to cylinder
DHW_ELEC_KWH = 1.9          # Total electricity consumed
DHW_COP = DHW_HEAT_KWH / DHW_ELEC_KWH  # ~3.16

# HP space heating performance model
# Weather compensation: MWT = a - b × T_outside
# Calibrated from emonhp data: at 7°C outside → MWT ~31°C, at 0°C → ~36°C
# These are steady-state values; recovery uses ~3°C higher MWT
WC_MWT_INTERCEPT = 38.0     # MWT at 0°C outside (steady state)
WC_MWT_SLOPE = -1.0         # MWT drops 1°C per °C warmer outside
RECOVERY_MWT_ADDER = 3.0    # Extra MWT during recovery (first 2°C deficit)

# COP model: COP = f(outside_temp, MWT)
# From Arotherm spec data + measured performance:
#   COP ≈ base_cop × (1 + 0.025 × T_outside) × (55/MWT)^0.4
# This gives COP ~4.8 at T_out=7, MWT=31 (matches measured)
# and COP ~3.1 at T_out=-3, MWT=50 (matches spec)
COP_BASE = 3.8              # Reference COP
COP_TOUT_COEFF = 0.035      # COP improves 3.5% per °C warmer outside
COP_MWT_REF = 40.0          # Reference MWT for COP_BASE
COP_MWT_EXPONENT = 0.5      # COP sensitivity to MWT

# Simulation
SIM_STEP_MINUTES = 5        # Simulation timestep
SCHEDULE_STEP_MINUTES = 15  # Decision variable resolution

# Target
DEFAULT_TARGET_TEMP = 19.5  # °C minimum at 07:00

# Rooms to exclude from target scoring (match house.py convention)
EXCLUDE_ROOMS = {"landing", "conservatory"}

# Location (for Open-Meteo)
LATITUDE = 51.59
LONGITUDE = -0.14


# ---------------------------------------------------------------------------
# COP model
# ---------------------------------------------------------------------------

def estimate_cop(outside_temp: float, mwt: float) -> float:
    """Estimate space-heating COP from outside temp and MWT.

    Calibrated against:
      - Measured: COP 4.8 at T_out=7°C, MWT=31°C (steady state)
      - Measured: COP 3.7 at T_out=7°C, MWT=38°C (recovery)
      - Spec: COP 4.48 at T_out=-3°C, MWT=33°C (35°C flow)
      - Spec: COP 3.06 at T_out=-3°C, MWT=52°C (55°C flow)
    """
    if mwt <= 0:
        return 1.0
    tout_factor = 1.0 + COP_TOUT_COEFF * outside_temp
    mwt_factor = (COP_MWT_REF / mwt) ** COP_MWT_EXPONENT
    return max(1.5, COP_BASE * tout_factor * mwt_factor)


def weather_comp_mwt(outside_temp: float, recovering: bool = False) -> float:
    """Weather-compensated MWT for space heating.

    The Arotherm adjusts flow temperature based on outside temp.
    During recovery (room temp below setpoint), MWT runs higher.
    """
    mwt = WC_MWT_INTERCEPT + WC_MWT_SLOPE * outside_temp
    if recovering:
        mwt += RECOVERY_MWT_ADDER
    return max(25.0, min(55.0, mwt))


# ---------------------------------------------------------------------------
# Weather forecast
# ---------------------------------------------------------------------------

def fetch_hourly_forecast() -> dict[int, float]:
    """Fetch tonight's hourly temperature forecast from Open-Meteo.

    Returns dict mapping hour (0-23) to temperature °C for tonight/tomorrow morning.
    Falls back to seasonal average if API unavailable.
    """
    now = datetime.now(timezone.utc)
    today = now.strftime("%Y-%m-%d")
    tomorrow = (now + timedelta(days=1)).strftime("%Y-%m-%d")

    url = (
        f"https://api.open-meteo.com/v1/forecast?"
        f"latitude={LATITUDE}&longitude={LONGITUDE}"
        f"&hourly=temperature_2m"
        f"&timezone=Europe/London"
        f"&start_date={today}&end_date={tomorrow}"
    )

    try:
        with urllib.request.urlopen(url, timeout=10) as resp:
            data = json.loads(resp.read())

        # Build hour → temp map for the overnight period
        # We want tonight (today 18:00 → tomorrow 12:00)
        hourly = {}
        for i, time_str in enumerate(data["hourly"]["time"]):
            temp = data["hourly"]["temperature_2m"][i]
            dt = datetime.fromisoformat(time_str)
            # Key: hours from midnight tonight
            if dt.strftime("%Y-%m-%d") == today and dt.hour >= 18:
                hourly[dt.hour] = temp
            elif dt.strftime("%Y-%m-%d") == tomorrow and dt.hour <= 12:
                hourly[24 + dt.hour] = temp  # 24=midnight, 25=01:00, etc.
        return hourly

    except Exception as e:
        print(f"  ⚠ Weather API failed ({e}), using seasonal average")
        # Seasonal average overnight profile for London, Dec-Mar
        return {
            18: 8.5, 19: 8.2, 20: 7.9, 21: 7.7, 22: 7.5, 23: 7.3,
            24: 7.1, 25: 6.9, 26: 6.7, 27: 6.6, 28: 6.5, 29: 6.5,
            30: 6.5, 31: 6.6, 32: 6.7, 33: 6.9, 34: 7.2, 35: 7.5,
            36: 7.9,
        }


def interpolate_temp(forecast: dict[int, float], hour_decimal: float) -> float:
    """Interpolate forecast temperature at fractional hour.

    hour_decimal: hours since 18:00 today. So 18.5 = 18:30, 28.0 = 04:00.
    """
    # Convert to forecast key space (18 = 6pm today, 24 = midnight, 28 = 4am)
    abs_hour = 18.0 + hour_decimal
    h_low = int(abs_hour)
    h_high = h_low + 1
    frac = abs_hour - h_low

    t_low = forecast.get(h_low)
    t_high = forecast.get(h_high)

    if t_low is None and t_high is None:
        return 7.0  # fallback
    if t_low is None:
        return t_high
    if t_high is None:
        return t_low

    return t_low + frac * (t_high - t_low)


# ---------------------------------------------------------------------------
# Fetch current room temperatures from InfluxDB
# ---------------------------------------------------------------------------

def fetch_current_room_temps() -> dict[str, float]:
    """Get latest room temperatures from InfluxDB Zigbee sensors."""
    from influxdb_client import InfluxDBClient

    rooms = build_rooms()
    sensor_map = build_sensor_map(rooms)

    client = InfluxDBClient(url=INFLUX_URL, token=INFLUX_TOKEN, org=INFLUX_ORG)
    api = client.query_api()

    # Query last reading for each sensor
    query = f'''
    from(bucket: "{INFLUX_BUCKET}")
      |> range(start: -1h)
      |> filter(fn: (r) => r._measurement == "zigbee" or r._measurement == "emonth2")
      |> filter(fn: (r) => r._field == "temperature")
      |> last()
    '''

    temps = {}
    try:
        tables = api.query(query)
        for table in tables:
            for record in table.records:
                topic = record.values.get("topic", "")
                for sensor_topic, room_name in sensor_map.items():
                    if sensor_topic in topic or topic in sensor_topic:
                        temps[room_name] = float(record.get_value())
    except Exception as e:
        print(f"  ⚠ InfluxDB query failed ({e}), using 20.5°C default")

    # Fill missing rooms with reasonable default
    for name in rooms:
        if name not in temps:
            temps[name] = 20.5

    client.close()
    return temps


def fetch_current_outside_temp() -> float:
    """Get latest outside temp from eBUS via InfluxDB."""
    from influxdb_client import InfluxDBClient

    client = InfluxDBClient(url=INFLUX_URL, token=INFLUX_TOKEN, org=INFLUX_ORG)
    api = client.query_api()

    query = f'''
    from(bucket: "{INFLUX_BUCKET}")
      |> range(start: -30m)
      |> filter(fn: (r) => r.topic == "ebusd/poll/OutsideTemp")
      |> last()
    '''

    try:
        tables = api.query(query)
        for table in tables:
            for record in table.records:
                client.close()
                return float(record.get_value())
    except Exception:
        pass

    client.close()
    return 7.0  # seasonal fallback


# ---------------------------------------------------------------------------
# Schedule representation
# ---------------------------------------------------------------------------

@dataclass
class Schedule:
    """An overnight heating schedule to evaluate."""
    heat_off_hour: float     # Hour (decimal) to turn heating off (e.g., 22.0)
    dhw_start_hour: float    # Hour (decimal) to start DHW (e.g., 28.0 = 04:00)
    dhw_mode: str            # "normal" or "eco"
    heat_on_hour: float      # Hour (decimal) to restart space heating

    @property
    def dhw_duration_hours(self) -> float:
        return (DHW_NORMAL_MINUTES if self.dhw_mode == "normal"
                else DHW_ECO_MINUTES) / 60.0

    @property
    def dhw_end_hour(self) -> float:
        return self.dhw_start_hour + self.dhw_duration_hours

    def describe(self) -> str:
        def fmt(h):
            hh = int(h) % 24
            mm = int((h % 1) * 60)
            return f"{hh:02d}:{mm:02d}"
        return (f"OFF {fmt(self.heat_off_hour)} → "
                f"DHW {self.dhw_mode} {fmt(self.dhw_start_hour)}–{fmt(self.dhw_end_hour)} → "
                f"Heat ON {fmt(self.heat_on_hour)}")


# ---------------------------------------------------------------------------
# Thermal simulation
# ---------------------------------------------------------------------------

def simulate_schedule(
    schedule: Schedule,
    initial_temps: dict[str, float],
    forecast: dict[int, float],
    rooms: dict,
    connections: list,
    doorways: list,
) -> dict:
    """Simulate room temperatures through an overnight schedule.

    Returns dict with:
      - temps_07: dict of room temps at 07:00
      - min_temp_07: minimum scored room temp at 07:00
      - electricity_kwh: total electricity consumed
      - cost_pence: total cost accounting for Cosy/blended rates
      - cosy_kwh: electricity in Cosy window
      - blended_kwh: electricity outside Cosy window
      - trace: list of (hour, avg_temp, mode) for plotting
    """
    dt_hours = SIM_STEP_MINUTES / 60.0
    dt_seconds = SIM_STEP_MINUTES * 60.0

    # Pre-compute thermal masses (kJ/K → J/K)
    thermal_mass = {}
    for name, room in rooms.items():
        thermal_mass[name] = estimate_thermal_mass(room, connections) * 1000.0  # kJ→J

    # State: current room temperatures
    temps = dict(initial_temps)

    # Accumulators
    total_elec_kwh = 0.0
    cosy_elec_kwh = 0.0
    blended_elec_kwh = 0.0
    trace = []

    # Simulate from heat_off_hour to 07:30 (31.5 in our hour space)
    sim_start = schedule.heat_off_hour
    sim_end = 31.5  # 07:30 next day

    t = sim_start
    while t < sim_end:
        # Current absolute hour (18-based → clock hour)
        clock_hour = (18.0 + (t - 18.0)) % 24.0 if t < 24 else t - 24.0

        # What's the HP doing?
        in_dhw = schedule.dhw_start_hour <= t < schedule.dhw_end_hour
        in_heating = (not in_dhw) and (t >= schedule.heat_on_hour)
        in_cooling = (not in_dhw) and (not in_heating)

        # Outside temperature
        t_out = interpolate_temp(forecast, t - 18.0)

        # MWT for radiators (0 if cooling or DHW)
        if in_heating:
            # Check if any scored room is below target → recovery mode
            scored_temps = [temps[n] for n in rooms if n not in EXCLUDE_ROOMS]
            min_scored = min(scored_temps) if scored_temps else 20.0
            recovering = min_scored < DEFAULT_TARGET_TEMP + 0.5
            mwt = weather_comp_mwt(t_out, recovering=recovering)
        else:
            mwt = 0.0

        # Calculate energy balance for each room
        for name, room in rooms.items():
            bal = room_energy_balance(
                room, temps[name], t_out, temps,
                connections, doorways,
                mwt=mwt,
                sleeping=(clock_hour >= 22 or clock_hour < 7),
                irradiance_sw=0.0,  # nighttime
                irradiance_ne=0.0,
            )

            # Update temperature: dT = Q_total × dt / C
            C = thermal_mass[name]
            if C > 0:
                dT = bal["total"] * dt_seconds / C
                temps[name] += dT

        # Electricity consumption
        if in_dhw:
            # DHW electricity spread evenly over duration
            elec_this_step = DHW_ELEC_KWH * (dt_hours / schedule.dhw_duration_hours)
        elif in_heating:
            # HP electricity = total radiator output / COP
            total_rad_w = 0.0
            for name, room in rooms.items():
                for rad in room.radiators:
                    if rad.active and mwt > 0:
                        from house import radiator_output
                        total_rad_w += radiator_output(rad.t50, mwt, temps[name])
            cop = estimate_cop(t_out, mwt)
            elec_this_step = (total_rad_w / 1000.0) * dt_hours / cop  # kWh
        else:
            elec_this_step = 0.0

        # Classify tariff period
        is_cosy = COSY_START_H <= clock_hour < COSY_END_H
        if is_cosy:
            cosy_elec_kwh += elec_this_step
        else:
            blended_elec_kwh += elec_this_step
        total_elec_kwh += elec_this_step

        # Trace
        scored = [temps[n] for n in rooms if n not in EXCLUDE_ROOMS]
        mode = "DHW" if in_dhw else ("HEAT" if in_heating else "OFF")
        trace.append((t, sum(scored) / len(scored) if scored else 20.0, mode))

        t += dt_hours

    # Final temperatures at 07:00 (hour 31.0)
    temps_07 = dict(temps)
    scored_07 = {n: temps_07[n] for n in rooms if n not in EXCLUDE_ROOMS}
    min_temp_07 = min(scored_07.values()) if scored_07 else 20.0

    cost = cosy_elec_kwh * COSY_RATE + blended_elec_kwh * BLENDED_RATE

    return {
        "temps_07": temps_07,
        "scored_07": scored_07,
        "min_temp_07": min_temp_07,
        "electricity_kwh": total_elec_kwh,
        "cost_pence": cost,
        "cosy_kwh": cosy_elec_kwh,
        "blended_kwh": blended_elec_kwh,
        "trace": trace,
    }


# ---------------------------------------------------------------------------
# Schedule enumeration and optimization
# ---------------------------------------------------------------------------

def generate_schedules(earliest_off: float = 21.0, latest_off: float = 24.0) -> list[Schedule]:
    """Generate all candidate schedules to evaluate.

    Constraints:
      - Heating off between earliest_off and latest_off
      - DHW must start within Cosy window (28.0–31.0 = 04:00–07:00)
      - DHW must end before Cosy window ends (31.0 = 07:00)
      - Space heating restarts after DHW ends (or at Cosy start if DHW is after)
      - Also include: DHW before heating, heating before DHW, no-heating variants
    """
    step = SCHEDULE_STEP_MINUTES / 60.0  # 0.25h
    schedules = []

    # Hour mapping: 18=6pm, 22=10pm, 24=midnight, 28=4am, 31=7am
    cosy_start = 28.0  # 04:00
    cosy_end = 31.0    # 07:00

    for off_h in frange(earliest_off, latest_off + step, step):
        for dhw_mode in ["normal", "eco"]:
            dhw_dur = (DHW_NORMAL_MINUTES if dhw_mode == "normal"
                       else DHW_ECO_MINUTES) / 60.0

            # Strategy A: DHW first in Cosy, then heat
            for dhw_start in frange(cosy_start, cosy_end - dhw_dur + step, step):
                dhw_end = dhw_start + dhw_dur
                if dhw_end > cosy_end + 0.01:
                    continue
                heat_on = dhw_end
                schedules.append(Schedule(off_h, dhw_start, dhw_mode, heat_on))

            # Strategy B: Heat first in Cosy, then DHW
            for heat_on in frange(cosy_start, cosy_end - 0.5, step):
                # DHW starts after some heating, must fit in window
                for dhw_start in frange(heat_on + 0.5, cosy_end - dhw_dur + step, step):
                    dhw_end = dhw_start + dhw_dur
                    if dhw_end > cosy_end + 0.5:  # Allow slight overshoot
                        continue
                    schedules.append(Schedule(off_h, dhw_start, dhw_mode, heat_on))

            # Strategy C: DHW before Cosy (at blended rate), heat entire Cosy window
            dhw_pre_start = cosy_start - dhw_dur
            if dhw_pre_start >= off_h + 1.0:  # At least 1h of cooling
                schedules.append(Schedule(off_h, dhw_pre_start, dhw_mode, cosy_start))

        # Strategy D: No heating restart (heating stays off until after 07:00)
        # Just DHW in Cosy window — useful for mild nights
        for dhw_mode in ["normal", "eco"]:
            dhw_dur = (DHW_NORMAL_MINUTES if dhw_mode == "normal"
                       else DHW_ECO_MINUTES) / 60.0
            schedules.append(Schedule(off_h, cosy_start, dhw_mode, 32.0))  # heat at 08:00

    return schedules


def frange(start, stop, step):
    """Float range generator."""
    vals = []
    v = start
    while v < stop - step/10:
        vals.append(round(v, 4))
        v += step
    return vals


# ---------------------------------------------------------------------------
# Main optimizer
# ---------------------------------------------------------------------------

def optimize(target_temp: float = DEFAULT_TARGET_TEMP,
             earliest_off: float = 21.0,
             latest_off: float = 24.0,
             verbose: bool = True) -> dict:
    """Find the optimal overnight schedule.

    Returns the best schedule and simulation results.
    """
    if verbose:
        print("=" * 70)
        print("OVERNIGHT HEATING OPTIMIZER")
        print("=" * 70)

    # Load thermal model
    if verbose:
        print("\n1. Loading thermal model...")
    rooms = build_rooms()
    connections = build_connections()
    doorways = build_doorways()

    if verbose:
        print(f"   {len(rooms)} rooms, {len(connections)} connections, "
              f"{len(doorways)} doorways")

    # Fetch current state
    if verbose:
        print("\n2. Fetching current conditions...")

    room_temps = fetch_current_room_temps()
    outside_temp = fetch_current_outside_temp()

    if verbose:
        scored = {n: t for n, t in room_temps.items() if n not in EXCLUDE_ROOMS}
        print(f"   Outside: {outside_temp:.1f}°C")
        print(f"   Room avg: {sum(scored.values())/len(scored):.1f}°C "
              f"(min {min(scored.values()):.1f}°C {min(scored, key=scored.get)}, "
              f"max {max(scored.values()):.1f}°C {max(scored, key=scored.get)})")

    # Fetch weather forecast
    if verbose:
        print("\n3. Fetching weather forecast...")
    forecast = fetch_hourly_forecast()

    if verbose:
        overnight_temps = [interpolate_temp(forecast, h)
                          for h in range(4, 14)]  # 22:00 to 08:00
        print(f"   Tonight: {min(overnight_temps):.1f}–{max(overnight_temps):.1f}°C "
              f"(avg {sum(overnight_temps)/len(overnight_temps):.1f}°C)")

    # Generate candidate schedules
    schedules = generate_schedules(earliest_off, latest_off)
    if verbose:
        print(f"\n4. Evaluating {len(schedules)} candidate schedules...")

    # Simulate all schedules
    results = []
    for i, sched in enumerate(schedules):
        result = simulate_schedule(
            sched, room_temps, forecast,
            rooms, connections, doorways,
        )
        result["schedule"] = sched
        results.append(result)

        if verbose and (i + 1) % 100 == 0:
            print(f"   ... {i+1}/{len(schedules)}", end="\r")

    if verbose:
        print(f"   ... {len(schedules)}/{len(schedules)} complete")

    # Filter feasible (meet target temperature)
    feasible = [r for r in results if r["min_temp_07"] >= target_temp]

    if verbose:
        print(f"\n5. Results: {len(feasible)}/{len(results)} meet "
              f"≥{target_temp}°C target at 07:00")

    if not feasible:
        # Relax constraint: find least-bad option
        if verbose:
            print("   ⚠ No schedule meets target — showing least-cold option")
        feasible = sorted(results, key=lambda r: -r["min_temp_07"])[:10]

    # Sort by cost
    feasible.sort(key=lambda r: r["cost_pence"])
    best = feasible[0]

    if verbose:
        print(f"\n{'=' * 70}")
        print("OPTIMAL SCHEDULE")
        print(f"{'=' * 70}")
        print(f"\n  {best['schedule'].describe()}")
        print(f"\n  Electricity: {best['electricity_kwh']:.1f} kWh "
              f"(Cosy: {best['cosy_kwh']:.1f}, blended: {best['blended_kwh']:.1f})")
        print(f"  Cost: {best['cost_pence']:.0f}p "
              f"(Cosy: {best['cosy_kwh'] * COSY_RATE:.0f}p + "
              f"blended: {best['blended_kwh'] * BLENDED_RATE:.0f}p)")

        # Room temps at 07:00
        print(f"\n  Room temperatures at 07:00:")
        for name in sorted(best["scored_07"].keys()):
            t = best["scored_07"][name]
            marker = " ✓" if t >= target_temp else " ✗ COLD"
            print(f"    {name:<14} {t:.1f}°C{marker}")
        print(f"    {'─' * 25}")
        print(f"    {'Min scored':<14} {best['min_temp_07']:.1f}°C")

        # Show top 5 alternatives
        print(f"\n  Top 5 schedules by cost:")
        print(f"  {'Schedule':<55} {'Cost':>5} {'Min°C':>5} {'Elec':>5}")
        print(f"  {'─' * 75}")
        for r in feasible[:5]:
            s = r["schedule"]
            print(f"  {s.describe():<55} {r['cost_pence']:>4.0f}p "
                  f"{r['min_temp_07']:>4.1f}° {r['electricity_kwh']:>4.1f}")

        # Show temperature trace for best schedule
        print(f"\n  Temperature trace (best schedule):")
        print(f"  {'Time':>5} {'Avg°C':>5} {'Mode':<5}")
        print(f"  {'─' * 20}")
        for hour, avg_t, mode in best["trace"]:
            clock = (18 + int(hour - 18)) % 24
            mins = int((hour % 1) * 60)
            if mins == 0:  # Print on the hour
                print(f"  {clock:02d}:{mins:02d} {avg_t:>5.1f} {mode}")

        # eBUS commands
        print(f"\n{'=' * 70}")
        print("eBUS COMMANDS (for pi5data)")
        print(f"{'=' * 70}")

        off_h = int(best["schedule"].heat_off_hour) % 24
        off_m = int((best["schedule"].heat_off_hour % 1) * 60)
        on_h = int(best["schedule"].heat_on_hour) % 24
        on_m = int((best["schedule"].heat_on_hour % 1) * 60)

        print(f"\n  # Turn heating off at {off_h:02d}:{off_m:02d}")
        print(f"  echo \"at {off_h:02d}:{off_m:02d} <<< 'echo \\\"write -c 700 Z1OpMode off\\\" | nc -w 2 localhost 8888'\" | at -M now")
        print(f"\n  # Turn heating on at {on_h:02d}:{on_m:02d}")
        print(f"  echo \"at {on_h:02d}:{on_m:02d} <<< 'echo \\\"write -c 700 Z1OpMode auto\\\" | nc -w 2 localhost 8888'\" | at -M now")

    return best


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

def main():
    target = DEFAULT_TARGET_TEMP
    earliest_off = 21.0

    # Parse simple CLI args
    args = sys.argv[1:]
    i = 0
    while i < len(args):
        if args[i] == "--target" and i + 1 < len(args):
            target = float(args[i + 1])
            i += 2
        elif args[i] == "--evening-off" and i + 1 < len(args):
            h, m = args[i + 1].split(":")
            earliest_off = int(h) + int(m) / 60.0
            i += 2
        elif args[i] == "--help":
            print("Usage: python model/overnight.py [--target TEMP] [--evening-off HH:MM]")
            print(f"\n  --target      Minimum room temp at 07:00 (default: {DEFAULT_TARGET_TEMP}°C)")
            print(f"  --evening-off Earliest time to turn heating off (default: 21:00)")
            return
        else:
            i += 1

    optimize(target_temp=target, earliest_off=earliest_off)


if __name__ == "__main__":
    main()
