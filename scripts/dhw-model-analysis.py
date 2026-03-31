#!/usr/bin/env python3
"""
DHW cylinder energy-balance model analysis.

Pulls all sensor data from InfluxDB and builds a stratified energy model
to predict remaining usable hot water more accurately than volume subtraction.

Sensors:
  T1 (1530mm)     - cylinder top / hot outlet
  T2 (490mm)      - mains inlet / cold feed  
  HwcStorageTemp (~600mm) - VR10 NTC above bottom coil
  dhw_volume_V1   - cumulative litres drawn (10L steps)
  dhw_flow        - instantaneous draw rate (L/h)
  BuildingCircuitFlow - HP pump flow (>900=DHW charging)
"""
import csv
import io
import sys
from dataclasses import dataclass, field
from datetime import datetime, timezone
import requests

INFLUX_URL = "http://10.0.1.230:8086"
INFLUX_TOKEN = "jPTPrwcprKfDzt8IFr7gkn6shpBy15j8hFeyjLaBIaJ0IwcgQeXJ4LtrvVBJ5aIPYuzEfeDw5e-cmtAuvZ-Xmw=="
INFLUX_ORG = "home"
BUCKET = "energy"

# Cylinder geometry (from dhw-cylinder-analysis.md)
INTERNAL_DIAMETER_MM = 450
INTERNAL_HEIGHT_MM = 1932
CROSS_SECTION_M2 = 0.159  # m²
LITRES_PER_MM = 0.159

# Sensor heights (internal, mm from bottom)
T1_HEIGHT = 1530
T2_HEIGHT = 490
HWC_STORAGE_HEIGHT = 600  # VR10 NTC, estimated
TOP_COIL_HEIGHT = 970
BOTTOM_COIL_HEIGHT = 370
WATER_SURFACE_HEIGHT = 1907  # at 45°C

# Thermal properties
CP_WATER = 4.186  # kJ/(kg·K)
DENSITY_WATER = 0.990  # kg/L at ~40°C
UA_STANDBY = 1.1  # W/K measured
AMBIENT_TEMP = 21.0  # typical airing cupboard

# Usable volume: T2 (490mm) to T1 (1530mm) 
USABLE_GEOMETRIC_VOLUME = (T1_HEIGHT - T2_HEIGHT) * LITRES_PER_MM  # 165L
USABLE_VALIDATED_VOLUME = 161.0  # from thermocline inflection measurement

# Minimum useful tap temperature (after ~1.5°C pipe loss)
MIN_USEFUL_TEMP = 38.0  # °C - lowest shower temp anyone would want


def query_influx(flux_query: str) -> list[dict]:
    """Execute a Flux query and return parsed CSV rows as dicts."""
    resp = requests.post(
        f"{INFLUX_URL}/api/v2/query?org={INFLUX_ORG}",
        headers={
            "Authorization": f"Token {INFLUX_TOKEN}",
            "Content-Type": "application/vnd.flux",
            "Accept": "application/csv",
        },
        data=flux_query,
    )
    resp.raise_for_status()
    
    rows = []
    reader = csv.DictReader(
        (line for line in resp.text.splitlines() if line and not line.startswith('#'))
    )
    for row in reader:
        rows.append(row)
    return rows


def fetch_sensor_timeseries(days: int = 12) -> dict[str, list[tuple[datetime, float]]]:
    """Fetch all DHW sensor data, return dict of sensor_name -> [(timestamp, value)]."""
    
    sensors = {}
    
    # Multical sensors: T1, T2, flow, volume
    for field_name in ["dhw_t1", "dhw_t2", "dhw_flow", "dhw_volume_V1"]:
        query = f'''from(bucket: "{BUCKET}")
  |> range(start: -{days}d)
  |> filter(fn: (r) => r._measurement == "emon" and r._field == "value" and r.field == "{field_name}")
  |> sort(columns: ["_time"])'''
        rows = query_influx(query)
        data = []
        for r in rows:
            try:
                ts = datetime.fromisoformat(r["_time"].replace("Z", "+00:00"))
                val = float(r["_value"])
                data.append((ts, val))
            except (ValueError, KeyError):
                continue
        sensors[field_name] = data
        print(f"  {field_name}: {len(data)} points", file=sys.stderr)
    
    # eBUS sensors
    for field_name in ["HwcStorageTemp", "BuildingCircuitFlow"]:
        query = f'''from(bucket: "{BUCKET}")
  |> range(start: -{days}d)
  |> filter(fn: (r) => r._measurement == "ebusd_poll" and r.field == "{field_name}")
  |> sort(columns: ["_time"])'''
        rows = query_influx(query)
        data = []
        for r in rows:
            try:
                ts = datetime.fromisoformat(r["_time"].replace("Z", "+00:00"))
                val = float(r["_value"])
                data.append((ts, val))
            except (ValueError, KeyError):
                continue
        sensors[field_name] = data
        print(f"  {field_name}: {len(data)} points", file=sys.stderr)
    
    # z2m-hub remaining litres (for comparison)
    query = f'''from(bucket: "{BUCKET}")
  |> range(start: -{days}d)
  |> filter(fn: (r) => r._measurement == "dhw" and r._field == "remaining_litres")
  |> sort(columns: ["_time"])'''
    rows = query_influx(query)
    data = []
    for r in rows:
        try:
            ts = datetime.fromisoformat(r["_time"].replace("Z", "+00:00"))
            val = float(r["_value"])
            data.append((ts, val))
        except (ValueError, KeyError):
            continue
    sensors["remaining_litres"] = data
    print(f"  remaining_litres: {len(data)} points", file=sys.stderr)
    
    return sensors


@dataclass
class CylinderState:
    """Snapshot of cylinder state at a point in time."""
    timestamp: datetime
    t1: float  # °C at 1530mm
    t2: float  # °C at 490mm 
    hwc_storage: float  # °C at ~600mm (VR10)
    volume_register: float  # cumulative litres
    flow_rate: float  # L/h instantaneous
    bc_flow: float  # building circuit flow (L/h) - >900 = DHW charging
    z2m_remaining: float  # z2m-hub's estimate for comparison


def interpolate_at(series: list[tuple[datetime, float]], target: datetime) -> float | None:
    """Linear interpolation of a time series at a target timestamp (binary search)."""
    if not series:
        return None
    
    import bisect
    # Binary search for insertion point
    target_ts = target.timestamp()
    # Build timestamp list lazily
    if not hasattr(interpolate_at, '_cache'):
        interpolate_at._cache = {}
    
    cache_key = id(series)
    if cache_key not in interpolate_at._cache:
        interpolate_at._cache[cache_key] = [s[0].timestamp() for s in series]
    
    timestamps = interpolate_at._cache[cache_key]
    idx = bisect.bisect_left(timestamps, target_ts)
    
    if idx == 0:
        return series[0][1]
    if idx >= len(series):
        return series[-1][1]
    
    t0_ts = timestamps[idx - 1]
    t1_ts = timestamps[idx]
    v0 = series[idx - 1][1]
    v1 = series[idx][1]
    
    dt_total = t1_ts - t0_ts
    if dt_total == 0:
        return v0
    frac = (target_ts - t0_ts) / dt_total
    return v0 + frac * (v1 - v0)


def build_aligned_timeline(
    sensors: dict[str, list[tuple[datetime, float]]],
    interval_s: int = 30,
) -> list[CylinderState]:
    """Align all sensors to a common timeline at fixed intervals."""
    
    # Find common time range
    starts = []
    ends = []
    for name, data in sensors.items():
        if data and name != "remaining_litres":
            starts.append(data[0][0])
            ends.append(data[-1][0])
    
    if not starts:
        return []
    
    t_start = max(starts)
    t_end = min(ends)
    
    print(f"\nAligned range: {t_start.isoformat()} to {t_end.isoformat()}", file=sys.stderr)
    print(f"Duration: {(t_end - t_start).total_seconds() / 3600:.1f} hours", file=sys.stderr)
    
    states = []
    from datetime import timedelta
    t = t_start
    while t <= t_end:
        state = CylinderState(
            timestamp=t,
            t1=interpolate_at(sensors["dhw_t1"], t) or 0,
            t2=interpolate_at(sensors["dhw_t2"], t) or 0,
            hwc_storage=interpolate_at(sensors["HwcStorageTemp"], t) or 0,
            volume_register=interpolate_at(sensors["dhw_volume_V1"], t) or 0,
            flow_rate=interpolate_at(sensors["dhw_flow"], t) or 0,
            bc_flow=interpolate_at(sensors["BuildingCircuitFlow"], t) or 0,
            z2m_remaining=interpolate_at(sensors.get("remaining_litres", []), t) or 0,
        )
        states.append(state)
        t += timedelta(seconds=interval_s)
    
    return states


@dataclass
class CylinderProfile:
    """1D temperature profile of the cylinder, modelled as zones."""
    # Zone temperatures (bottom to top)
    # Zone 0: 0 → 370mm (below bottom coil, dead zone, 59L)
    # Zone 1: 370 → 490mm (bottom coil to T2, 19L)
    # Zone 2: 490 → 600mm (T2 to HwcStorageTemp, 17L)
    # Zone 3: 600 → 970mm (HwcStorage to top coil, 59L)
    # Zone 4: 970 → 1530mm (top coil to T1, 89L)
    # Zone 5: 1530 → 1907mm (above T1 to water surface, 60L)
    zone_temps: list[float] = field(default_factory=lambda: [15.0] * 6)
    zone_volumes: list[float] = field(default_factory=lambda: [59.0, 19.0, 17.0, 59.0, 89.0, 60.0])
    zone_heights_mm: list[tuple[int, int]] = field(default_factory=lambda: [
        (0, 370), (370, 490), (490, 600), (600, 970), (970, 1530), (1530, 1907)
    ])
    
    def total_energy_kwh(self, ref_temp: float = 10.0) -> float:
        """Total thermal energy above reference temperature, in kWh."""
        energy_kj = 0.0
        for temp, vol in zip(self.zone_temps, self.zone_volumes):
            if temp > ref_temp:
                energy_kj += vol * DENSITY_WATER * CP_WATER * (temp - ref_temp)
        return energy_kj / 3600.0
    
    def usable_litres(self, min_temp: float = MIN_USEFUL_TEMP) -> float:
        """Litres of water above minimum useful temperature.
        
        For zones above the thermocline (fully hot), counts the entire zone.
        For the zone containing the thermocline, interpolates.
        """
        usable = 0.0
        for temp, vol in zip(self.zone_temps, self.zone_volumes):
            if temp >= min_temp:
                usable += vol
        return usable
    
    def usable_litres_energy_weighted(self, min_temp: float = MIN_USEFUL_TEMP, 
                                       target_shower: float = 40.0,
                                       cold_side: float = 25.0) -> float:
        """Equivalent shower-litres: how many litres of shower water the stored
        energy can produce, accounting for mixing.
        
        Each litre of hot water at T_hot mixed with cold at T_cold produces
        (T_hot - T_cold) / (T_shower - T_cold) litres of shower water.
        """
        if target_shower <= cold_side:
            return 999.0  # trivially satisfied
        
        shower_litres = 0.0
        for temp, vol in zip(self.zone_temps, self.zone_volumes):
            if temp > target_shower:
                # This zone can produce shower water
                # Each litre of this zone water produces:
                mixing_ratio = (temp - cold_side) / (target_shower - cold_side)
                shower_litres += vol * mixing_ratio
            elif temp > cold_side:
                # Warm but below target - still has some energy but can't
                # produce target-temp water alone. In practice this zone
                # would mix with hotter water above and cold below.
                # Conservative: don't count it.
                pass
        return shower_litres

    def from_sensors(self, t1: float, t2: float, hwc_storage: float,
                     mains_temp: float = 15.0) -> "CylinderProfile":
        """Estimate full profile from three sensor readings.
        
        Physics-based interpolation:
        - Below bottom coil: mains temp (dead zone, slowly warming)
        - Bottom coil to T2: linear interpolation
        - T2 to HwcStorage: measured directly
        - HwcStorage to top coil: linear interpolation to coil zone temp
        - Top coil to T1: the main hot zone, ~T1
        - Above T1: same as T1 (well-mixed hot zone)
        """
        # Estimate coil zone temperature from HwcStorage and T1
        # The coil zone is between the two coils, heated from below and above
        coil_zone_temp = (hwc_storage + t1) / 2  # rough approximation
        
        self.zone_temps = [
            mains_temp,           # Zone 0: dead zone below bottom coil
            (mains_temp + t2) / 2,  # Zone 1: bottom coil to T2 (transitional)
            (t2 + hwc_storage) / 2, # Zone 2: T2 to HwcStorage (measured both ends)
            (hwc_storage + coil_zone_temp) / 2,  # Zone 3: HwcStorage to top coil
            t1,                     # Zone 4: top coil to T1 (main hot zone)
            t1,                     # Zone 5: above T1 (same as T1)
        ]
        return self


def detect_events(states: list[CylinderState]) -> list[dict]:
    """Detect charge cycles, draw events, and standby periods."""
    events = []
    in_charge = False
    in_draw = False
    charge_start = None
    draw_start = None
    draw_vol_start = 0.0
    
    for i, s in enumerate(states):
        # DHW charging: BuildingCircuitFlow > 900
        if s.bc_flow > 900 and not in_charge:
            in_charge = True
            charge_start = i
        elif s.bc_flow <= 900 and in_charge:
            in_charge = False
            events.append({
                "type": "charge",
                "start": states[charge_start].timestamp,
                "end": s.timestamp,
                "start_idx": charge_start,
                "end_idx": i,
                "t1_start": states[charge_start].t1,
                "t1_end": s.t1,
                "hwc_start": states[charge_start].hwc_storage,
                "hwc_end": s.hwc_storage,
                "t2_start": states[charge_start].t2,
                "t2_end": s.t2,
                "duration_min": (s.timestamp - states[charge_start].timestamp).total_seconds() / 60,
            })
        
        # Draw event: flow_rate > 50 L/h
        if s.flow_rate > 50 and not in_draw:
            in_draw = True
            draw_start = i
            draw_vol_start = s.volume_register
        elif s.flow_rate <= 50 and in_draw:
            in_draw = False
            vol_drawn = s.volume_register - draw_vol_start
            events.append({
                "type": "draw",
                "start": states[draw_start].timestamp,
                "end": s.timestamp,
                "start_idx": draw_start,
                "end_idx": i,
                "volume": vol_drawn,
                "t1_start": states[draw_start].t1,
                "t1_end": s.t1,
                "t2_start": states[draw_start].t2,
                "t2_end": s.t2,
                "hwc_start": states[draw_start].hwc_storage,
                "hwc_end": s.hwc_storage,
                "duration_min": (s.timestamp - states[draw_start].timestamp).total_seconds() / 60,
                "peak_flow": max(states[j].flow_rate for j in range(draw_start, i + 1)),
            })
    
    return events


def analyse_charge_completions(events: list[dict], states: list[CylinderState]):
    """Analyse what T1 and HwcStorage tell us at end of each charge."""
    print("\n" + "=" * 80)
    print("CHARGE CYCLE ANALYSIS")
    print("=" * 80)
    print(f"{'End Time':>20s} {'Dur':>5s} {'T1 start':>8s} {'T1 end':>7s} {'HwcS start':>10s} {'HwcS end':>8s} {'T2 start':>8s} {'T2 end':>7s}")
    
    charges = [e for e in events if e["type"] == "charge"]
    for c in charges:
        print(f"{c['end'].strftime('%Y-%m-%d %H:%M'):>20s} "
              f"{c['duration_min']:5.0f}m "
              f"{c['t1_start']:7.1f}° {c['t1_end']:6.1f}° "
              f"{c['hwc_start']:9.1f}° {c['hwc_end']:7.1f}° "
              f"{c['t2_start']:7.1f}° {c['t2_end']:6.1f}°")
    
    if charges:
        t1_ends = [c["t1_end"] for c in charges]
        hwc_ends = [c["hwc_end"] for c in charges]
        t2_ends = [c["t2_end"] for c in charges]
        print(f"\nPost-charge T1:  mean={sum(t1_ends)/len(t1_ends):.1f}°C, "
              f"min={min(t1_ends):.1f}°C, max={max(t1_ends):.1f}°C")
        print(f"Post-charge HwcS: mean={sum(hwc_ends)/len(hwc_ends):.1f}°C, "
              f"min={min(hwc_ends):.1f}°C, max={max(hwc_ends):.1f}°C")
        print(f"Post-charge T2:  mean={sum(t2_ends)/len(t2_ends):.1f}°C, "
              f"min={min(t2_ends):.1f}°C, max={max(t2_ends):.1f}°C")


def analyse_draw_events(events: list[dict], states: list[CylinderState]):
    """Analyse draw events and what sensors tell us about remaining capacity."""
    print("\n" + "=" * 80)
    print("DRAW EVENT ANALYSIS")
    print("=" * 80)
    print(f"{'Time':>20s} {'Dur':>5s} {'Vol':>5s} {'Peak':>6s} {'T1 Δ':>6s} {'HwcS Δ':>7s} {'T2 Δ':>6s} {'Type':>8s}")
    
    draws = [e for e in events if e["type"] == "draw"]
    for d in draws:
        t1_delta = d["t1_end"] - d["t1_start"]
        hwc_delta = d["hwc_end"] - d["hwc_start"]
        t2_delta = d["t2_end"] - d["t2_start"]
        
        # Classify draw type
        if d["peak_flow"] > 300:
            draw_type = "shower" if d["duration_min"] > 3 else "bath?"
        elif d["peak_flow"] > 100:
            draw_type = "sink"
        else:
            draw_type = "trickle"
        
        print(f"{d['start'].strftime('%Y-%m-%d %H:%M'):>20s} "
              f"{d['duration_min']:4.1f}m "
              f"{d['volume']:4.0f}L "
              f"{d['peak_flow']:5.0f} "
              f"{t1_delta:+5.1f}° "
              f"{hwc_delta:+6.1f}° "
              f"{t2_delta:+5.1f}° "
              f"{draw_type:>8s}")


def analyse_energy_balance(events: list[dict], states: list[CylinderState]):
    """Compare energy-balance prediction with z2m-hub volume-subtraction."""
    print("\n" + "=" * 80)
    print("ENERGY-BALANCE vs VOLUME-SUBTRACTION COMPARISON")
    print("=" * 80)
    
    charges = [e for e in events if e["type"] == "charge"]
    draws = [e for e in events if e["type"] == "draw"]
    
    if not charges:
        print("No charge events found")
        return
    
    # For each charge→draw→draw→charge cycle, compare predictions
    print(f"\n{'Time':>20s} {'Event':>8s} {'Vol sub':>8s} {'Energy':>8s} {'T1':>5s} {'HwcS':>5s} {'z2m':>6s}")
    
    # Track volume-subtraction model
    vol_remaining = USABLE_VALIDATED_VOLUME
    vol_at_reset = 0.0
    
    for event in sorted(events, key=lambda e: e["start"]):
        if event["type"] == "charge":
            # After charge, estimate remaining from temperatures
            profile = CylinderProfile()
            profile.from_sensors(event["t1_end"], event["t2_end"], event["hwc_end"])
            energy_usable = profile.usable_litres(MIN_USEFUL_TEMP)
            
            vol_remaining = USABLE_VALIDATED_VOLUME  # volume model: reset to full
            
            # Get z2m estimate at this time
            idx = event["end_idx"]
            z2m = states[idx].z2m_remaining if idx < len(states) else 0
            
            print(f"{event['end'].strftime('%Y-%m-%d %H:%M'):>20s} "
                  f"{'CHARGE':>8s} "
                  f"{vol_remaining:7.0f}L "
                  f"{energy_usable:7.0f}L "
                  f"{event['t1_end']:4.1f}° "
                  f"{event['hwc_end']:4.1f}° "
                  f"{z2m:5.0f}L")
            
            vol_at_reset = states[idx].volume_register
            
        elif event["type"] == "draw":
            vol_drawn = event["volume"]
            vol_remaining = max(0, vol_remaining - vol_drawn)
            
            # Energy model: estimate from sensors AFTER draw
            profile = CylinderProfile()
            profile.from_sensors(event["t1_end"], event["t2_end"], event["hwc_end"])
            energy_usable = profile.usable_litres(MIN_USEFUL_TEMP)
            
            idx = event["end_idx"]
            z2m = states[idx].z2m_remaining if idx < len(states) else 0
            
            print(f"{event['end'].strftime('%Y-%m-%d %H:%M'):>20s} "
                  f"{vol_drawn:6.0f}L→ "
                  f"{vol_remaining:7.0f}L "
                  f"{energy_usable:7.0f}L "
                  f"{event['t1_end']:4.1f}° "
                  f"{event['hwc_end']:4.1f}° "
                  f"{z2m:5.0f}L")


def analyse_standby_decay(states: list[CylinderState], events: list[dict]):
    """Find standby periods (no draw, no charge) and measure decay rates."""
    print("\n" + "=" * 80)
    print("STANDBY DECAY ANALYSIS")
    print("=" * 80)
    
    # Find periods >2h with no flow and no charging
    quiet_periods = []
    quiet_start = None
    
    for i, s in enumerate(states):
        is_quiet = s.flow_rate < 5 and s.bc_flow < 900
        if is_quiet and quiet_start is None:
            quiet_start = i
        elif not is_quiet and quiet_start is not None:
            duration_h = (s.timestamp - states[quiet_start].timestamp).total_seconds() / 3600
            if duration_h >= 2.0:
                quiet_periods.append((quiet_start, i))
            quiet_start = None
    
    print(f"\nFound {len(quiet_periods)} standby periods ≥ 2h\n")
    print(f"{'Start':>20s} {'Hours':>6s} {'T1 start':>8s} {'T1 end':>7s} {'T1 rate':>8s} {'HwcS rate':>9s}")
    
    for start_idx, end_idx in quiet_periods[:20]:
        s0 = states[start_idx]
        s1 = states[end_idx - 1]
        hours = (s1.timestamp - s0.timestamp).total_seconds() / 3600
        t1_rate = (s1.t1 - s0.t1) / hours
        hwc_rate = (s1.hwc_storage - s0.hwc_storage) / hours
        
        print(f"{s0.timestamp.strftime('%Y-%m-%d %H:%M'):>20s} "
              f"{hours:5.1f}h "
              f"{s0.t1:7.1f}° {s1.t1:6.1f}° "
              f"{t1_rate:+7.2f}°/h "
              f"{hwc_rate:+8.2f}°/h")


def analyse_t1_as_predictor(states: list[CylinderState], events: list[dict]):
    """Analyse how well T1 alone predicts remaining capacity.
    
    After a full charge, T1 should be ~45°C and remaining ~161L.
    As water is drawn, T1 stays stable (thermocline below T1).
    When T1 starts dropping, the thermocline has reached T1 — remaining is near zero.
    
    Key question: can T1 + HwcStorageTemp together predict remaining better
    than volume subtraction?
    """
    print("\n" + "=" * 80)
    print("T1 AS REMAINING CAPACITY PREDICTOR")
    print("=" * 80)
    
    # Find draw events large enough to move T1
    draws = [e for e in events if e["type"] == "draw" and e["volume"] > 30]
    
    print(f"\nSignificant draws (>30L):")
    print(f"{'Time':>20s} {'Vol':>5s} {'T1→':>10s} {'HwcS→':>12s} {'T1 moved?':>10s}")
    
    for d in draws:
        t1_delta = d["t1_end"] - d["t1_start"]
        hwc_delta = d["hwc_end"] - d["hwc_start"]
        moved = "YES" if abs(t1_delta) > 0.3 else "no"
        
        print(f"{d['start'].strftime('%Y-%m-%d %H:%M'):>20s} "
              f"{d['volume']:4.0f}L "
              f"{d['t1_start']:.1f}→{d['t1_end']:.1f} "
              f"{d['hwc_start']:.1f}→{d['hwc_end']:.1f} "
              f"{moved:>10s}")
    
    print("\n--- T1 stability analysis ---")
    print("T1 should remain stable until ~161L drawn since last charge.")
    print("HwcStorageTemp should crash earlier (it's lower in the cylinder).")
    print("The combination gives thermocline position tracking.")


def propose_improved_model(events: list[dict], states: list[CylinderState]):
    """Summarise findings and propose the improved model."""
    print("\n" + "=" * 80)
    print("PROPOSED IMPROVED MODEL")
    print("=" * 80)
    
    charges = [e for e in events if e["type"] == "charge"]
    
    # Calculate post-charge temperature statistics
    if charges:
        print(f"\nFrom {len(charges)} observed charge completions:")
        for c in charges:
            profile = CylinderProfile()
            profile.from_sensors(c["t1_end"], c["t2_end"], c["hwc_end"])
            
            usable_simple = profile.usable_litres(MIN_USEFUL_TEMP)
            shower_equiv = profile.usable_litres_energy_weighted(
                MIN_USEFUL_TEMP, target_shower=40.0, cold_side=25.0
            )
            total_energy = profile.total_energy_kwh()
            
            print(f"  {c['end'].strftime('%m-%d %H:%M')}: "
                  f"T1={c['t1_end']:.1f}° HwcS={c['hwc_end']:.1f}° T2={c['t2_end']:.1f}° "
                  f"→ usable={usable_simple:.0f}L, shower-equiv={shower_equiv:.0f}L, "
                  f"energy={total_energy:.1f}kWh")
    
    print("""
The improved model should:

1. AFTER CHARGE: Estimate actual remaining from post-charge temperatures
   - Full charge (T1≥44°C, HwcS≥40°C): remaining ≈ 161L (validated)  
   - Partial charge (boost): interpolate based on T2 and HwcStorage rise
   - HwcStorage is the key indicator: it's the VRC 700's own trigger sensor

2. DURING DRAWS: Track thermocline position from two indicators:
   a. Volume drawn (current model) — thermocline rises at known rate
   b. HwcStorageTemp crash — when it drops >2°C in <5min, cold front 
      has reached 600mm. Remaining = (T1_height - 600mm) × litres_per_mm
      = (1530-600) × 0.159 = 148L minus volume already drawn above that
   c. T1 stability — when T1 drops >0.3°C during a draw, thermocline  
      has reached 1530mm. Remaining ≈ 0L usable.

3. DURING STANDBY: Apply measured decay rate
   - T1 drops 0.25°C/h (measured)
   - After 24h standby: T1 ≈ 39°C, still usable but reduced capacity
   - HwcStorageTemp drops faster (less insulation at that height)
   
4. SINK vs SHOWER discrimination:
   - Flow >300 L/h = shower (draws from usable volume)  
   - Flow <200 L/h = sink (draws from dead zone first, ~59L free)
   - Track sink draws separately: first 59L of sink use costs nothing
""")


def main():
    print("Fetching sensor data from InfluxDB...", file=sys.stderr)
    sensors = fetch_sensor_timeseries(days=12)
    
    print("\nBuilding aligned timeline...", file=sys.stderr)
    states = build_aligned_timeline(sensors, interval_s=30)
    print(f"Total aligned states: {len(states)}", file=sys.stderr)
    
    if not states:
        print("ERROR: No aligned data found. Check sensor availability.", file=sys.stderr)
        return
    
    # Summary stats
    print("\n" + "=" * 80)
    print("DATA SUMMARY")
    print("=" * 80)
    print(f"Time range: {states[0].timestamp} to {states[-1].timestamp}")
    print(f"Duration: {(states[-1].timestamp - states[0].timestamp).total_seconds() / 86400:.1f} days")
    print(f"T1 range: {min(s.t1 for s in states):.1f}°C to {max(s.t1 for s in states):.1f}°C")
    print(f"T2 range: {min(s.t2 for s in states):.1f}°C to {max(s.t2 for s in states):.1f}°C")
    print(f"HwcStorage range: {min(s.hwc_storage for s in states):.1f}°C to {max(s.hwc_storage for s in states):.1f}°C")
    print(f"Volume register: {states[0].volume_register:.0f} to {states[-1].volume_register:.0f} "
          f"(Δ{states[-1].volume_register - states[0].volume_register:.0f}L)")
    
    # Detect events
    events = detect_events(states)
    charges = [e for e in events if e["type"] == "charge"]
    draws = [e for e in events if e["type"] == "draw"]
    print(f"\nDetected {len(charges)} charge cycles, {len(draws)} draw events")
    
    # Run analyses
    analyse_charge_completions(events, states)
    analyse_draw_events(events, states)
    analyse_energy_balance(events, states)
    analyse_standby_decay(states, events)
    analyse_t1_as_predictor(states, events)
    propose_improved_model(events, states)


if __name__ == "__main__":
    main()
