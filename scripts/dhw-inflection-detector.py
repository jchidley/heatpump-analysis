#!/usr/bin/env python3
"""
DHW T1 inflection detector.

Processes every large draw event at 2-second resolution to find the
exact volume at which T1 begins dropping — the empirical usable capacity.

Runs periodically (cron) or on-demand. Writes results to InfluxDB
for Grafana dashboarding and long-term tracking.

Each measurement captures:
  - inflection_volume_L: cumulative litres since last charge at T1 inflection
  - draw_volume_L: litres drawn in THIS draw at inflection
  - gap_hours: hours since the previous draw ended
  - t1_at_inflection: T1 temperature at inflection point
  - flow_rate_lph: average draw flow rate (L/h)
  - mains_temp_c: T2 at start of draw (proxy for mains/WWHR temp)
  - t1_start: T1 at start of draw
  - method: "hint" (dT1/dV < -0.003) or "definitive" (dT1/dV < -0.01)
"""
import csv
import sys
from dataclasses import dataclass
from datetime import datetime, timedelta
import requests

INFLUX_URL = "http://10.0.1.230:8086"
INFLUX_TOKEN = "jPTPrwcprKfDzt8IFr7gkn6shpBy15j8hFeyjLaBIaJ0IwcgQeXJ4LtrvVBJ5aIPYuzEfeDw5e-cmtAuvZ-Xmw=="
INFLUX_ORG = "home"
BUCKET = "energy"

# Thresholds
DRAW_FLOW_MIN = 100     # L/h — minimum to count as a draw
DRAW_MIN_VOLUME = 40    # L — minimum draw size to analyse
CHARGE_BC_FLOW = 900    # L/h — BuildingCircuitFlow threshold for DHW charging
HINT_RATE = -0.003      # °C/L — dT1/dV threshold for "first hint"
SIGNAL_RATE = -0.01     # °C/L — dT1/dV threshold for "definitive"
ROLLING_WINDOW = 10.0   # L — window for dT1/dV computation


def query_influx(flux: str) -> list[dict]:
    resp = requests.post(
        f"{INFLUX_URL}/api/v2/query?org={INFLUX_ORG}",
        headers={
            "Authorization": f"Token {INFLUX_TOKEN}",
            "Content-Type": "application/vnd.flux",
            "Accept": "application/csv",
        },
        data=flux,
    )
    resp.raise_for_status()
    return list(csv.DictReader(
        line for line in resp.text.splitlines() if line and not line.startswith('#')
    ))


def write_influx(line_protocol: str):
    resp = requests.post(
        f"{INFLUX_URL}/api/v2/write?org={INFLUX_ORG}&bucket={BUCKET}&precision=s",
        headers={
            "Authorization": f"Token {INFLUX_TOKEN}",
            "Content-Type": "text/plain",
        },
        data=line_protocol,
    )
    if not resp.ok:
        print(f"  InfluxDB write failed: {resp.status_code} {resp.text}", file=sys.stderr)


def parse_ts_val(rows):
    data = []
    for r in rows:
        try:
            ts = datetime.fromisoformat(r["_time"].replace("Z", "+00:00"))
            val = float(r["_value"])
            data.append((ts, val))
        except (ValueError, KeyError):
            pass
    return data


@dataclass
class DrawEvent:
    start: datetime
    end: datetime
    volume_register_start: float
    volume_register_end: float
    volume_drawn: float  # from register
    charge_end_time: datetime
    charge_end_volume: float
    cumulative_since_charge: float
    gap_hours: float  # hours since previous draw ended
    prev_draw_end: datetime | None


@dataclass
class InflectionResult:
    draw: DrawEvent
    hint_cumulative: float | None
    hint_draw_vol: float | None
    hint_rate: float | None
    definitive_cumulative: float | None
    definitive_draw_vol: float | None
    definitive_rate: float | None
    t1_start: float
    t1_at_hint: float | None
    t1_at_definitive: float | None
    mains_temp: float
    avg_flow_rate: float


def find_draw_events(days: int = 12) -> list[DrawEvent]:
    """Find all significant draw events with their charge context."""
    print(f"Finding draw events in last {days} days...", file=sys.stderr)
    
    # Get 1-min aggregated data for event detection
    flow_data = parse_ts_val(query_influx(f'''from(bucket: "{BUCKET}")
      |> range(start: -{days}d)
      |> filter(fn: (r) => r._measurement == "emon" and r._field == "value" and r.field == "dhw_flow")
      |> aggregateWindow(every: 1m, fn: mean, createEmpty: false)'''))
    
    vol_data = parse_ts_val(query_influx(f'''from(bucket: "{BUCKET}")
      |> range(start: -{days}d)
      |> filter(fn: (r) => r._measurement == "emon" and r._field == "value" and r.field == "dhw_volume_V1")
      |> aggregateWindow(every: 1m, fn: last, createEmpty: false)'''))
    
    bc_data = parse_ts_val(query_influx(f'''from(bucket: "{BUCKET}")
      |> range(start: -{days}d)
      |> filter(fn: (r) => r._measurement == "ebusd_poll" and r.field == "BuildingCircuitFlow")
      |> aggregateWindow(every: 1m, fn: last, createEmpty: false)'''))
    
    # Index by minute
    def to_dict(data):
        d = {}
        for ts, val in data:
            key = ts.replace(second=0, microsecond=0)
            d[key] = val
        return d
    
    flow = to_dict(flow_data)
    vol = to_dict(vol_data)
    bc = to_dict(bc_data)
    
    all_times = sorted(set(flow.keys()) & set(vol.keys()))
    
    # Track charges and draws
    in_charge = False
    last_charge_end_time = None
    last_charge_end_vol = None
    
    in_draw = False
    draw_start = None
    draw_start_vol = None
    prev_draw_end = None
    
    events = []
    
    for t in all_times:
        f = flow.get(t, 0)
        v = vol.get(t, 0)
        b = bc.get(t, 0)
        
        # Track charges
        if b > CHARGE_BC_FLOW:
            in_charge = True
        elif in_charge:
            in_charge = False
            last_charge_end_time = t
            last_charge_end_vol = v
        
        # Track draws
        if f > DRAW_FLOW_MIN and not in_draw and not in_charge:
            in_draw = True
            draw_start = t
            draw_start_vol = v
        elif (f <= DRAW_FLOW_MIN or in_charge) and in_draw:
            in_draw = False
            drawn = v - draw_start_vol
            
            if drawn >= DRAW_MIN_VOLUME and last_charge_end_vol is not None:
                gap = (draw_start - prev_draw_end).total_seconds() / 3600 if prev_draw_end else 999
                
                events.append(DrawEvent(
                    start=draw_start,
                    end=t,
                    volume_register_start=draw_start_vol,
                    volume_register_end=v,
                    volume_drawn=drawn,
                    charge_end_time=last_charge_end_time,
                    charge_end_volume=last_charge_end_vol,
                    cumulative_since_charge=v - last_charge_end_vol,
                    gap_hours=gap,
                    prev_draw_end=prev_draw_end,
                ))
            
            prev_draw_end = t
    
    print(f"  Found {len(events)} draws ≥{DRAW_MIN_VOLUME}L", file=sys.stderr)
    return events


def analyse_draw(draw: DrawEvent) -> InflectionResult | None:
    """Analyse a single draw at 2-second resolution for T1 inflection."""
    # Fetch raw 2-second data for this draw window (with 2min padding)
    start_iso = (draw.start - timedelta(minutes=2)).strftime('%Y-%m-%dT%H:%M:%SZ')
    end_iso = (draw.end + timedelta(minutes=2)).strftime('%Y-%m-%dT%H:%M:%SZ')
    
    t1_raw = parse_ts_val(query_influx(f'''from(bucket: "{BUCKET}")
      |> range(start: {start_iso}, stop: {end_iso})
      |> filter(fn: (r) => r._measurement == "emon" and r.field == "dhw_t1")'''))
    
    flow_raw = parse_ts_val(query_influx(f'''from(bucket: "{BUCKET}")
      |> range(start: {start_iso}, stop: {end_iso})
      |> filter(fn: (r) => r._measurement == "emon" and r.field == "dhw_flow")'''))
    
    t2_raw = parse_ts_val(query_influx(f'''from(bucket: "{BUCKET}")
      |> range(start: {start_iso}, stop: {end_iso})
      |> filter(fn: (r) => r._measurement == "emon" and r.field == "dhw_t2")'''))
    
    if len(t1_raw) < 10 or len(flow_raw) < 10:
        return None
    
    # Flow integration (trapezoidal)
    cumul = 0.0
    fi = []  # (timestamp_epoch, cumulative_litres_in_this_draw)
    for i in range(1, len(flow_raw)):
        dt = (flow_raw[i][0] - flow_raw[i-1][0]).total_seconds()
        avg_lph = (flow_raw[i][1] + flow_raw[i-1][1]) / 2
        cumul += avg_lph * dt / 3600
        fi.append((flow_raw[i][0].timestamp(), cumul))
    
    if not fi:
        return None
    
    # Volume offset: cumulative from charge
    cumul_before_draw = draw.volume_register_start - draw.charge_end_volume
    
    # Map T1 to cumulative volume
    import bisect
    fi_times = [f[0] for f in fi]
    fi_vols = [f[1] for f in fi]
    
    t1_vs_cumul = []
    for ts, t1_val in t1_raw:
        ts_e = ts.timestamp()
        idx = bisect.bisect_left(fi_times, ts_e)
        if 0 < idx < len(fi_vols):
            frac = (ts_e - fi_times[idx-1]) / (fi_times[idx] - fi_times[idx-1]) if fi_times[idx] != fi_times[idx-1] else 0
            draw_vol = fi_vols[idx-1] + frac * (fi_vols[idx] - fi_vols[idx-1])
            total_cumul = cumul_before_draw + draw_vol
            t1_vs_cumul.append((total_cumul, draw_vol, t1_val))
    
    if len(t1_vs_cumul) < 20:
        return None
    
    # T1 at start, mains temp
    t1_start = t1_vs_cumul[0][2]
    mains_temp = t2_raw[0][1] if t2_raw else 15.0
    avg_flow = sum(f[1] for f in flow_raw) / len(flow_raw)
    
    # Rolling dT1/dV
    hint_result = None
    definitive_result = None
    
    for i in range(len(t1_vs_cumul)):
        cumul_i, draw_i, t1_i = t1_vs_cumul[i]
        for j in range(i-1, -1, -1):
            cumul_j, draw_j, t1_j = t1_vs_cumul[j]
            if cumul_i - cumul_j >= ROLLING_WINDOW:
                rate = (t1_i - t1_j) / (cumul_i - cumul_j)
                if hint_result is None and rate < HINT_RATE:
                    hint_result = (cumul_i, draw_i, rate, t1_i)
                if definitive_result is None and rate < SIGNAL_RATE:
                    definitive_result = (cumul_i, draw_i, rate, t1_i)
                break
    
    return InflectionResult(
        draw=draw,
        hint_cumulative=hint_result[0] if hint_result else None,
        hint_draw_vol=hint_result[1] if hint_result else None,
        hint_rate=hint_result[2] if hint_result else None,
        definitive_cumulative=definitive_result[0] if definitive_result else None,
        definitive_draw_vol=definitive_result[1] if definitive_result else None,
        definitive_rate=definitive_result[2] if definitive_result else None,
        t1_start=t1_start,
        t1_at_hint=hint_result[3] if hint_result else None,
        t1_at_definitive=definitive_result[3] if definitive_result else None,
        mains_temp=mains_temp,
        avg_flow_rate=avg_flow,
    )


def main():
    import argparse
    parser = argparse.ArgumentParser(description="DHW T1 inflection detector")
    parser.add_argument("--days", type=int, default=12, help="Days of history to analyse")
    parser.add_argument("--write", action="store_true", help="Write results to InfluxDB")
    args = parser.parse_args()
    
    draws = find_draw_events(args.days)
    
    print(f"\n{'='*100}")
    print(f"T1 INFLECTION ANALYSIS — {len(draws)} draws at 2-second resolution")
    print(f"{'='*100}")
    print(f"{'Draw time':>20s} │ {'Vol':>4s} {'Cumul':>6s} {'Gap':>5s} │ "
          f"{'Hint @':>7s} {'Def @':>7s} │ {'T1':>5s} {'T2':>5s} {'Flow':>5s} │ Notes")
    print("─" * 100)
    
    results = []
    for draw in draws:
        result = analyse_draw(draw)
        if result is None:
            continue
        results.append(result)
        
        d = result
        hint_str = f"{d.hint_cumulative:.0f}L" if d.hint_cumulative else "  —"
        def_str = f"{d.definitive_cumulative:.0f}L" if d.definitive_cumulative else "  —"
        
        notes = ""
        if d.definitive_cumulative:
            notes = f"T1 dropped at {d.definitive_cumulative:.0f}L cumul ({d.definitive_draw_vol:.0f}L into draw)"
        elif d.hint_cumulative:
            notes = f"hint only at {d.hint_cumulative:.0f}L cumul"
        else:
            notes = "T1 stable throughout"
        
        print(f"{draw.start.strftime('%d/%m %H:%M'):>20s} │ "
              f"{draw.volume_drawn:3.0f}L {draw.cumulative_since_charge:5.0f}L "
              f"{draw.gap_hours:4.1f}h │ "
              f"{hint_str:>7s} {def_str:>7s} │ "
              f"{d.t1_start:4.1f}° {d.mains_temp:4.1f}° {d.avg_flow_rate:4.0f} │ "
              f"{notes}")
    
    # Summary
    definitive_hits = [r for r in results if r.definitive_cumulative is not None]
    hint_hits = [r for r in results if r.hint_cumulative is not None and r.definitive_cumulative is None]
    stable = [r for r in results if r.hint_cumulative is None]
    
    print(f"\n{'='*100}")
    print(f"SUMMARY")
    print(f"{'='*100}")
    print(f"Total draws analysed: {len(results)}")
    print(f"  T1 definitive drop: {len(definitive_hits)}")
    print(f"  T1 hint only:       {len(hint_hits)}")
    print(f"  T1 stable:          {len(stable)}")
    
    if definitive_hits:
        vols = [r.definitive_cumulative for r in definitive_hits]
        print(f"\nDefinitive T1 inflection volumes (cumulative since charge):")
        for r in definitive_hits:
            print(f"  {r.draw.start.strftime('%d/%m %H:%M')}: "
                  f"{r.definitive_cumulative:.0f}L cumul, "
                  f"gap={r.draw.gap_hours:.1f}h, "
                  f"T1={r.t1_start:.1f}°, "
                  f"T2={r.mains_temp:.1f}°, "
                  f"flow={r.avg_flow_rate:.0f} L/h")
        print(f"\n  Range: {min(vols):.0f}–{max(vols):.0f}L")
        print(f"  Mean:  {sum(vols)/len(vols):.0f}L")
    
    # Write to InfluxDB
    if args.write and definitive_hits:
        print(f"\nWriting {len(definitive_hits)} inflection measurements to InfluxDB...")
        for r in definitive_hits:
            ts = int(r.draw.start.timestamp())
            line = (f"dhw_inflection "
                    f"cumulative_volume={r.definitive_cumulative:.1f},"
                    f"draw_volume={r.definitive_draw_vol:.1f},"
                    f"gap_hours={r.draw.gap_hours:.2f},"
                    f"t1_start={r.t1_start:.2f},"
                    f"t1_at_inflection={r.t1_at_definitive:.2f},"
                    f"mains_temp={r.mains_temp:.1f},"
                    f"flow_rate={r.avg_flow_rate:.0f},"
                    f"rate={r.definitive_rate:.5f} "
                    f"{ts}")
            write_influx(line)
        print("  Done.")


if __name__ == "__main__":
    main()
