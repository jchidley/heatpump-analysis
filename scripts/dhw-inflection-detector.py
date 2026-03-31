#!/usr/bin/env python3
"""
DHW T1 inflection detector.

Processes every large draw event at 2-second Multical resolution to find
the exact volume at which T1 begins dropping — the empirical usable capacity.

Classifies each measurement by charge context:
  - CAPACITY: after a crossover charge (full cylinder) — true capacity measurement
  - PARTIAL:  after a no-crossover charge — measures remaining from partial state
  - LOWER BOUND: draw ended before T1 dropped — usable is at least this much

Writes results to InfluxDB with --write for Grafana tracking.
"""
import bisect
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
CHARGE_MIN_DURATION = 300  # seconds — ignore charges < 5 min
HINT_RATE = -0.003      # °C/L — dT1/dV for "first hint"
SIGNAL_RATE = -0.01     # °C/L — dT1/dV for "definitive"
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
class ChargeEvent:
    start: datetime
    end: datetime
    t1_pre: float       # T1 when charge began
    hwc_end: float      # HwcStorageTemp when charge ended
    t1_end: float        # T1 when charge ended
    crossover: bool      # HwcStorage reached T1_pre during this charge
    volume_at_end: float # volume register at charge end


@dataclass
class DrawEvent:
    start: datetime
    end: datetime
    volume_register_start: float
    volume_register_end: float
    volume_drawn: float
    preceding_charge: ChargeEvent | None
    cumulative_since_charge: float
    gap_hours: float     # hours since previous draw ended
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

    @property
    def category(self) -> str:
        """Classify this result for the summary."""
        charge = self.draw.preceding_charge
        if self.definitive_cumulative is not None:
            if charge and charge.crossover and charge.t1_end >= 43.0:
                return "CAPACITY"
            else:
                return "PARTIAL"
        elif self.hint_cumulative is not None:
            return "LOWER BOUND (hint)"
        else:
            return "LOWER BOUND (stable)"

    @property
    def best_volume(self) -> float | None:
        """Best volume estimate: definitive if available, else hint, else cumulative drawn."""
        if self.definitive_cumulative is not None:
            return self.definitive_cumulative
        if self.hint_cumulative is not None:
            return self.hint_cumulative
        return self.draw.cumulative_since_charge


def find_events(days: int = 12) -> tuple[list[ChargeEvent], list[DrawEvent]]:
    """Find all charge and draw events with crossover detection."""
    print(f"Finding events in last {days} days...", file=sys.stderr)

    # 1-min aggregated data for event detection
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

    t1_data = parse_ts_val(query_influx(f'''from(bucket: "{BUCKET}")
      |> range(start: -{days}d)
      |> filter(fn: (r) => r._measurement == "emon" and r._field == "value" and r.field == "dhw_t1")
      |> aggregateWindow(every: 1m, fn: mean, createEmpty: false)'''))

    hwc_data = parse_ts_val(query_influx(f'''from(bucket: "{BUCKET}")
      |> range(start: -{days}d)
      |> filter(fn: (r) => r._measurement == "ebusd_poll" and r.field == "HwcStorageTemp")
      |> aggregateWindow(every: 1m, fn: last, createEmpty: false)'''))

    def to_dict(data):
        d = {}
        for ts, val in data:
            d[ts.replace(second=0, microsecond=0)] = val
        return d

    flow = to_dict(flow_data)
    vol = to_dict(vol_data)
    bc = to_dict(bc_data)
    t1 = to_dict(t1_data)
    hwc = to_dict(hwc_data)

    all_times = sorted(set(flow.keys()) & set(vol.keys()))

    # --- Detect charges with crossover ---
    charges: list[ChargeEvent] = []
    in_charge = False
    charge_start = None
    charge_t1_pre = None
    charge_hwc_max = None

    for t in all_times:
        b = bc.get(t, 0)
        if b > CHARGE_BC_FLOW and not in_charge:
            in_charge = True
            charge_start = t
            charge_t1_pre = t1.get(t)
            # Look back up to 3 min for T1 if not available at this minute
            if charge_t1_pre is None:
                for offset in range(1, 4):
                    charge_t1_pre = t1.get(t - timedelta(minutes=offset))
                    if charge_t1_pre is not None:
                        break
            charge_hwc_max = hwc.get(t, 0)
        elif b > CHARGE_BC_FLOW and in_charge:
            h = hwc.get(t)
            if h is not None and h > (charge_hwc_max or 0):
                charge_hwc_max = h
        elif in_charge:
            in_charge = False
            duration = (t - charge_start).total_seconds() if charge_start else 0
            if duration >= CHARGE_MIN_DURATION and charge_t1_pre is not None:
                hwc_end = hwc.get(t) or hwc.get(t - timedelta(minutes=1)) or charge_hwc_max or 0
                t1_end = t1.get(t) or t1.get(t - timedelta(minutes=1)) or 0
                crossover = (charge_hwc_max or 0) >= charge_t1_pre
                charges.append(ChargeEvent(
                    start=charge_start,
                    end=t,
                    t1_pre=charge_t1_pre,
                    hwc_end=hwc_end,
                    t1_end=t1_end,
                    crossover=crossover,
                    volume_at_end=vol.get(t, 0),
                ))

    print(f"  Charges: {len(charges)} ({sum(1 for c in charges if c.crossover)} crossover, "
          f"{sum(1 for c in charges if not c.crossover)} partial)", file=sys.stderr)

    # --- Detect draws with charge context ---
    draws: list[DrawEvent] = []
    in_draw = False
    draw_start = None
    draw_start_vol = None
    prev_draw_end = None

    def find_preceding_charge(t: datetime) -> ChargeEvent | None:
        """Find the most recent charge that ended before time t."""
        best = None
        for c in charges:
            if c.end <= t:
                if best is None or c.end > best.end:
                    best = c
        return best

    for t in all_times:
        f = flow.get(t, 0)
        v = vol.get(t, 0)
        b = bc.get(t, 0)

        if f > DRAW_FLOW_MIN and not in_draw and b <= CHARGE_BC_FLOW:
            in_draw = True
            draw_start = t
            draw_start_vol = v
        elif (f <= DRAW_FLOW_MIN or b > CHARGE_BC_FLOW) and in_draw:
            in_draw = False
            drawn = v - draw_start_vol

            if drawn >= DRAW_MIN_VOLUME:
                charge = find_preceding_charge(draw_start)
                charge_vol = charge.volume_at_end if charge else 0
                gap = (draw_start - prev_draw_end).total_seconds() / 3600 if prev_draw_end else 999

                draws.append(DrawEvent(
                    start=draw_start,
                    end=t,
                    volume_register_start=draw_start_vol,
                    volume_register_end=v,
                    volume_drawn=drawn,
                    preceding_charge=charge,
                    cumulative_since_charge=v - charge_vol,
                    gap_hours=gap,
                    prev_draw_end=prev_draw_end,
                ))

            prev_draw_end = t

    print(f"  Draws: {len(draws)} ≥{DRAW_MIN_VOLUME}L", file=sys.stderr)
    return charges, draws


def analyse_draw(draw: DrawEvent) -> InflectionResult | None:
    """Analyse a single draw at 2-second resolution for T1 inflection."""
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
    fi = []
    for i in range(1, len(flow_raw)):
        dt = (flow_raw[i][0] - flow_raw[i - 1][0]).total_seconds()
        avg_lph = (flow_raw[i][1] + flow_raw[i - 1][1]) / 2
        cumul += avg_lph * dt / 3600
        fi.append((flow_raw[i][0].timestamp(), cumul))

    if not fi:
        return None

    cumul_before_draw = draw.volume_register_start - (
        draw.preceding_charge.volume_at_end if draw.preceding_charge else 0
    )

    fi_times = [f[0] for f in fi]
    fi_vols = [f[1] for f in fi]

    t1_vs_cumul = []
    for ts, t1_val in t1_raw:
        ts_e = ts.timestamp()
        idx = bisect.bisect_left(fi_times, ts_e)
        if 0 < idx < len(fi_vols):
            frac = (
                (ts_e - fi_times[idx - 1]) / (fi_times[idx] - fi_times[idx - 1])
                if fi_times[idx] != fi_times[idx - 1] else 0
            )
            draw_vol = fi_vols[idx - 1] + frac * (fi_vols[idx] - fi_vols[idx - 1])
            total_cumul = cumul_before_draw + draw_vol
            t1_vs_cumul.append((total_cumul, draw_vol, t1_val))

    if len(t1_vs_cumul) < 20:
        return None

    t1_start = t1_vs_cumul[0][2]
    mains_temp = t2_raw[0][1] if t2_raw else 15.0
    avg_flow = sum(f[1] for f in flow_raw) / len(flow_raw)

    # Rolling dT1/dV
    hint_result = None
    definitive_result = None

    for i in range(len(t1_vs_cumul)):
        cumul_i, draw_i, t1_i = t1_vs_cumul[i]
        for j in range(i - 1, -1, -1):
            cumul_j, _, t1_j = t1_vs_cumul[j]
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

    charges, draws = find_events(args.days)

    # Analyse each draw
    results: list[InflectionResult] = []
    for draw in draws:
        result = analyse_draw(draw)
        if result is not None:
            results.append(result)

    # --- Detailed output ---
    charge_col = "Charge"
    print(f"\n{'='*115}")
    print(f"T1 INFLECTION ANALYSIS — {len(results)} draws at 2-second resolution")
    print(f"{'='*115}")
    print(f"{'Draw time':>20s} │ {'Vol':>4s} {'Cumul':>6s} {'Gap':>5s} │ "
          f"{'Hint @':>7s} {'Def @':>7s} │ {'T1':>5s} {'T2':>5s} {'Flow':>5s} │ "
          f"{charge_col:>10s} │ Category")
    print("─" * 115)

    for r in results:
        d = r.draw
        hint_str = f"{r.hint_cumulative:.0f}L" if r.hint_cumulative else "  —"
        def_str = f"{r.definitive_cumulative:.0f}L" if r.definitive_cumulative else "  —"

        charge_str = ""
        if d.preceding_charge:
            if d.preceding_charge.crossover:
                charge_str = f"✓ {d.preceding_charge.t1_end:.0f}°"
            else:
                gap = d.preceding_charge.t1_pre - d.preceding_charge.hwc_end
                charge_str = f"✗ gap {gap:.0f}°"
        else:
            charge_str = "?"

        print(f"{d.start.strftime('%d/%m %H:%M'):>20s} │ "
              f"{d.volume_drawn:3.0f}L {d.cumulative_since_charge:5.0f}L "
              f"{d.gap_hours:4.1f}h │ "
              f"{hint_str:>7s} {def_str:>7s} │ "
              f"{r.t1_start:4.1f}° {r.mains_temp:4.1f}° {r.avg_flow_rate:4.0f} │ "
              f"{charge_str:>10s} │ {r.category}")

    # --- Summary by category ---
    capacity = [r for r in results if r.category == "CAPACITY"]
    partial = [r for r in results if r.category == "PARTIAL"]
    lb_hint = [r for r in results if r.category == "LOWER BOUND (hint)"]
    lb_stable = [r for r in results if r.category == "LOWER BOUND (stable)"]

    print(f"\n{'='*115}")
    print("SUMMARY")
    print(f"{'='*115}")
    print(f"Total draws analysed: {len(results)}")

    # Capacity measurements — the key output
    print(f"\n  CAPACITY measurements (T1 dropped after crossover charge, T1≥43°):")
    if capacity:
        for r in capacity:
            print(f"    {r.draw.start.strftime('%d/%m %H:%M')}: "
                  f"{r.definitive_cumulative:.0f}L "
                  f"(gap={r.draw.gap_hours:.1f}h, T1={r.t1_start:.1f}°, "
                  f"T2={r.mains_temp:.1f}°, flow={r.avg_flow_rate:.0f} L/h)")
        vols = [r.definitive_cumulative for r in capacity]
        print(f"    ───")
        print(f"    Range: {min(vols):.0f}–{max(vols):.0f}L   Mean: {sum(vols)/len(vols):.0f}L   "
              f"Count: {len(vols)}")
    else:
        print(f"    (none — need draws >150L from a fully-charged cylinder)")

    # Partial state measurements
    print(f"\n  PARTIAL state (T1 dropped, but charge was incomplete or T1 was low):")
    if partial:
        for r in partial:
            c = r.draw.preceding_charge
            ctx = ""
            if c and not c.crossover:
                ctx = f"no crossover (gap {c.t1_pre - c.hwc_end:.1f}°)"
            elif c and c.t1_end < 43:
                ctx = f"crossover but T1 only {c.t1_end:.0f}°"
            print(f"    {r.draw.start.strftime('%d/%m %H:%M')}: "
                  f"{r.definitive_cumulative:.0f}L — {ctx}")
    else:
        print(f"    (none)")

    # Lower bounds — usable was at least this much
    print(f"\n  LOWER BOUNDS (draw ended before T1 dropped — capacity exceeds this):")
    if lb_hint or lb_stable:
        bounds = lb_hint + lb_stable
        bounds.sort(key=lambda r: r.best_volume or 0, reverse=True)
        for r in bounds[:8]:  # show top 8
            v = r.best_volume
            kind = "hint" if r.hint_cumulative else "stable"
            c_ok = "✓" if (r.draw.preceding_charge and r.draw.preceding_charge.crossover) else "✗"
            print(f"    {r.draw.start.strftime('%d/%m %H:%M')}: "
                  f"≥{v:.0f}L ({kind}, charge {c_ok}, T1={r.t1_start:.1f}°)")
        if len(bounds) > 8:
            print(f"    ... and {len(bounds) - 8} more")
        best = max(r.best_volume for r in bounds if r.best_volume)
        print(f"    ───")
        print(f"    Highest lower bound: ≥{best:.0f}L")
    else:
        print(f"    (none)")

    # Overall assessment
    print(f"\n  OVERALL:")
    all_capacity_vols = [r.definitive_cumulative for r in capacity]
    all_lower_bounds = [r.best_volume for r in (lb_hint + lb_stable)
                        if r.best_volume and r.draw.preceding_charge
                        and r.draw.preceding_charge.crossover]

    if all_capacity_vols:
        print(f"    Best estimate of full-charge usable volume: "
              f"{min(all_capacity_vols):.0f}–{max(all_capacity_vols):.0f}L "
              f"(from {len(all_capacity_vols)} measurements)")
    if all_lower_bounds:
        print(f"    Highest confirmed lower bound (from crossover charge): "
              f"≥{max(all_lower_bounds):.0f}L")
    print(f"    Geometric maximum (dip pipe to draw-off): 243L")

    # Write to InfluxDB
    if args.write:
        to_write = capacity + partial  # only write actual inflection measurements
        if to_write:
            print(f"\nWriting {len(to_write)} inflection measurements to InfluxDB...")
            for r in to_write:
                ts = int(r.draw.start.timestamp())
                cat = "capacity" if r.category == "CAPACITY" else "partial"
                crossover = "true" if (r.draw.preceding_charge and r.draw.preceding_charge.crossover) else "false"
                line = (
                    f'dhw_inflection,category={cat},crossover={crossover} '
                    f'cumulative_volume={r.definitive_cumulative:.1f},'
                    f'draw_volume={r.definitive_draw_vol:.1f},'
                    f'gap_hours={r.draw.gap_hours:.2f},'
                    f't1_start={r.t1_start:.2f},'
                    f't1_at_inflection={r.t1_at_definitive:.2f},'
                    f'mains_temp={r.mains_temp:.1f},'
                    f'flow_rate={r.avg_flow_rate:.0f},'
                    f'rate={r.definitive_rate:.5f} '
                    f'{ts}'
                )
                write_influx(line)
            print("  Done.")
        else:
            print("\nNo inflection measurements to write.")


if __name__ == "__main__":
    main()
