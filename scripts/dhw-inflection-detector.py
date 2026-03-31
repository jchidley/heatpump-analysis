#!/usr/bin/env python3
"""
DHW T1 inflection detector.

Processes every large draw event at 2-second Multical resolution to find
the exact volume at which T1 begins dropping — the empirical usable capacity.

Outputs:
  - Always writes results to InfluxDB (dhw_inflection measurement)
  - Default: JSON to stdout (for piping to other tools / z2m-hub)
  - --human: human-readable summary instead of JSON
  - --verbose: full per-draw detail table (implies --human)

Usage:
  uv run --with requests python scripts/dhw-inflection-detector.py --days 7
  uv run --with requests python scripts/dhw-inflection-detector.py --days 7 --human
  uv run --with requests python scripts/dhw-inflection-detector.py --days 12 --verbose
  uv run --with requests python scripts/dhw-inflection-detector.py --days 7 --no-write
"""
import bisect
import csv
import json
import sys
from dataclasses import dataclass
from datetime import datetime, timedelta
import requests

INFLUX_URL = "http://10.0.1.230:8086"
INFLUX_TOKEN = "jPTPrwcprKfDzt8IFr7gkn6shpBy15j8hFeyjLaBIaJ0IwcgQeXJ4LtrvVBJ5aIPYuzEfeDw5e-cmtAuvZ-Xmw=="
INFLUX_ORG = "home"
BUCKET = "energy"

DRAW_FLOW_MIN = 100
DRAW_MIN_VOLUME = 40
CHARGE_BC_FLOW = 900
CHARGE_MIN_DURATION = 300
HINT_RATE = -0.003
SIGNAL_RATE = -0.01
ROLLING_WINDOW = 10.0
GEOMETRIC_MAX = 243.0


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
        print(f"InfluxDB write failed: {resp.status_code} {resp.text}", file=sys.stderr)


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
    t1_pre: float
    hwc_end: float
    t1_end: float
    crossover: bool
    volume_at_end: float


@dataclass
class DrawEvent:
    start: datetime
    end: datetime
    volume_register_start: float
    volume_register_end: float
    volume_drawn: float
    preceding_charge: ChargeEvent | None
    cumulative_since_charge: float
    gap_hours: float
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
        charge = self.draw.preceding_charge
        if self.definitive_cumulative is not None:
            if charge and charge.crossover and charge.t1_end >= 43.0:
                return "capacity"
            else:
                return "partial"
        elif self.hint_cumulative is not None:
            return "lower_bound"
        else:
            return "lower_bound"

    @property
    def best_volume(self) -> float | None:
        if self.definitive_cumulative is not None:
            return self.definitive_cumulative
        if self.hint_cumulative is not None:
            return self.hint_cumulative
        return self.draw.cumulative_since_charge

    def to_dict(self) -> dict:
        d = self.draw
        c = d.preceding_charge
        return {
            "time": d.start.isoformat(),
            "category": self.category,
            "volume_drawn": round(d.volume_drawn),
            "cumulative_since_charge": round(d.cumulative_since_charge),
            "inflection_volume": round(self.definitive_cumulative) if self.definitive_cumulative else None,
            "hint_volume": round(self.hint_cumulative) if self.hint_cumulative else None,
            "best_volume": round(self.best_volume) if self.best_volume else None,
            "t1_start": round(self.t1_start, 1),
            "mains_temp": round(self.mains_temp, 1),
            "flow_rate": round(self.avg_flow_rate),
            "gap_hours": round(d.gap_hours, 1),
            "charge_crossover": c.crossover if c else None,
            "charge_t1_end": round(c.t1_end, 1) if c else None,
        }


def find_events(days: int = 12) -> tuple[list[ChargeEvent], list[DrawEvent]]:
    """Find all charge and draw events with crossover detection."""
    print(f"Finding events in last {days} days...", file=sys.stderr)

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
                    start=charge_start, end=t, t1_pre=charge_t1_pre,
                    hwc_end=hwc_end, t1_end=t1_end, crossover=crossover,
                    volume_at_end=vol.get(t, 0),
                ))

    print(f"  {len(charges)} charges ({sum(1 for c in charges if c.crossover)} crossover, "
          f"{sum(1 for c in charges if not c.crossover)} partial)", file=sys.stderr)

    draws: list[DrawEvent] = []
    in_draw = False
    draw_start = None
    draw_start_vol = None
    prev_draw_end = None

    def find_preceding_charge(t: datetime) -> ChargeEvent | None:
        best = None
        for c in charges:
            if c.end <= t and (best is None or c.end > best.end):
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
                    start=draw_start, end=t,
                    volume_register_start=draw_start_vol, volume_register_end=v,
                    volume_drawn=drawn, preceding_charge=charge,
                    cumulative_since_charge=v - charge_vol,
                    gap_hours=gap, prev_draw_end=prev_draw_end,
                ))

            prev_draw_end = t

    print(f"  {len(draws)} draws ≥{DRAW_MIN_VOLUME}L", file=sys.stderr)
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


def write_to_influxdb(results: list[InflectionResult]):
    """Write capacity/partial measurements and recommended capacity to InfluxDB."""
    to_write = [r for r in results if r.definitive_cumulative is not None]
    if not to_write:
        print("No inflection measurements to write.", file=sys.stderr)
        return

    print(f"Writing {len(to_write)} measurements to InfluxDB...", file=sys.stderr)
    for r in to_write:
        ts = int(r.draw.start.timestamp())
        cat = r.category
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

    # Write recommended capacity (z2m-hub reads this on startup)
    capacity = [r for r in results if r.category == "capacity"]
    rec = compute_recommended_capacity(capacity)
    if rec["recommended_full_litres"] is not None:
        val = rec["recommended_full_litres"]
        method = rec["method"]
        write_influx(f'dhw_capacity recommended_full_litres={val:.1f},method="{method}"')
        print(f"  Recommended capacity: {val}L ({method})", file=sys.stderr)

    print(f"  Done.", file=sys.stderr)


# WWHR threshold: T2 above this = WWHR active (warmer inlet from drain heat exchanger)
WWHR_T2_THRESHOLD = 20.0


def compute_recommended_capacity(capacity: list[InflectionResult]) -> dict:
    """Compute the recommended full_litres for z2m-hub config.

    WWHR warms the inlet (T2 ~25°C vs ~16°C mains), reducing density contrast
    and causing earlier inflection. Since most draws are showers (WWHR active),
    we report the WWHR capacity as the recommended value.

    With only cold-mains measurements, we estimate the WWHR capacity by
    regression on T2 vs inflection volume, or fall back to a conservative ratio.
    """
    if not capacity:
        return {"recommended_full_litres": None, "method": "no_data"}

    wwhr = [r for r in capacity if r.mains_temp >= WWHR_T2_THRESHOLD]
    cold = [r for r in capacity if r.mains_temp < WWHR_T2_THRESHOLD]

    if wwhr:
        # Direct WWHR measurements available — use the best
        best_wwhr = max(r.definitive_cumulative for r in wwhr)
        return {
            "recommended_full_litres": round(best_wwhr),
            "method": "direct_wwhr",
            "wwhr_measurements": len(wwhr),
            "cold_measurements": len(cold),
        }

    if len(cold) >= 2:
        # Multiple cold-mains measurements — regress T2 vs volume
        # and extrapolate to WWHR T2 (~25°C)
        import statistics
        t2s = [r.mains_temp for r in cold]
        vols = [r.definitive_cumulative for r in cold]
        n = len(t2s)
        mean_t2 = statistics.mean(t2s)
        mean_vol = statistics.mean(vols)
        cov = sum((t - mean_t2) * (v - mean_vol) for t, v in zip(t2s, vols)) / n
        var_t2 = sum((t - mean_t2) ** 2 for t in t2s) / n
        if var_t2 > 0.1:
            slope = cov / var_t2  # litres per °C of inlet temp
            wwhr_estimate = max(cold, key=lambda r: r.definitive_cumulative).definitive_cumulative + slope * (25.0 - max(r.mains_temp for r in cold))
            return {
                "recommended_full_litres": round(max(wwhr_estimate, min(vols))),
                "method": "regression",
                "slope_litres_per_degC": round(slope, 1),
                "cold_measurements": len(cold),
            }

    # Fallback: single cold-mains measurement, apply conservative 3% reduction
    best_cold = max(r.definitive_cumulative for r in cold) if cold else max(r.definitive_cumulative for r in capacity)
    return {
        "recommended_full_litres": round(best_cold * 0.97),
        "method": "conservative_ratio",
        "cold_measurements": len(cold),
    }


def output_json(results: list[InflectionResult]):
    """JSON output to stdout — for piping to other tools."""
    capacity = [r for r in results if r.category == "capacity"]
    partial = [r for r in results if r.category == "partial"]
    lower_bounds = [r for r in results if r.category == "lower_bound"]

    lb_crossover = [r for r in lower_bounds
                    if r.draw.preceding_charge and r.draw.preceding_charge.crossover]

    recommendation = compute_recommended_capacity(capacity)

    out = {
        "max_usable_litres": round(max(r.definitive_cumulative for r in capacity)) if capacity else None,
        "geometric_max_litres": round(GEOMETRIC_MAX),
        "plug_flow_efficiency": round(max(r.definitive_cumulative for r in capacity) / GEOMETRIC_MAX, 3) if capacity else None,
        "highest_lower_bound": round(max(r.best_volume for r in lb_crossover)) if lb_crossover else None,
        "recommended": recommendation,
        "capacity_measurements": [r.to_dict() for r in capacity],
        "partial_measurements": [r.to_dict() for r in partial],
        "lower_bounds_count": len(lower_bounds),
        "total_draws_analysed": len(results),
    }

    json.dump(out, sys.stdout, indent=2)
    print()  # trailing newline


def output_human(results: list[InflectionResult], days: int, verbose: bool):
    """Human-readable output to stdout."""
    capacity = [r for r in results if r.category == "capacity"]
    partial = [r for r in results if r.category == "partial"]
    lower_bounds = [r for r in results if r.category == "lower_bound"]

    if verbose:
        print(f"\n{'='*115}")
        print(f"ALL DRAWS — {len(results)} at 2-second resolution")
        print(f"{'='*115}")
        print(f"{'Draw time':>20s} │ {'Vol':>4s} {'Cumul':>6s} {'Gap':>5s} │ "
              f"{'Hint @':>7s} {'Def @':>7s} │ {'T1':>5s} {'T2':>5s} {'Flow':>5s} │ "
              f"{'Charge':>10s} │ Category")
        print("─" * 115)

        for r in results:
            d = r.draw
            hint_str = f"{r.hint_cumulative:.0f}L" if r.hint_cumulative else "  —"
            def_str = f"{r.definitive_cumulative:.0f}L" if r.definitive_cumulative else "  —"
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

    print(f"\nDHW USABLE VOLUME — {len(results)} draws analysed over {days} days")
    print(f"{'='*70}")

    if capacity:
        best = max(r.definitive_cumulative for r in capacity)
        print(f"\n  Maximum measured usable volume: {best:.0f}L")
        print(f"  (geometric max {GEOMETRIC_MAX:.0f}L, plug flow efficiency {best/GEOMETRIC_MAX:.0%})")
        print()
        for r in capacity:
            print(f"    {r.draw.start.strftime('%d/%m %H:%M')}: "
                  f"{r.definitive_cumulative:.0f}L  "
                  f"T1={r.t1_start:.1f}°  T2={r.mains_temp:.1f}°  "
                  f"flow={r.avg_flow_rate:.0f} L/h  gap={r.draw.gap_hours:.1f}h")
    else:
        print(f"\n  No capacity measurements yet.")
        print(f"  (need a draw >150L from a fully-charged cylinder)")

    lb_crossover = [r for r in lower_bounds
                    if r.draw.preceding_charge and r.draw.preceding_charge.crossover]
    if lb_crossover:
        best_lb = max(r.best_volume for r in lb_crossover if r.best_volume)
        print(f"\n  Highest confirmed lower bound: ≥{best_lb:.0f}L")

    if partial:
        print(f"\n  Partial-state measurements (not full charge):")
        for r in partial:
            c = r.draw.preceding_charge
            ctx = ""
            if c and not c.crossover:
                ctx = f"no crossover (gap {c.t1_pre - c.hwc_end:.1f}°)"
            elif c and c.t1_end < 43:
                ctx = f"crossover but T1 only {c.t1_end:.0f}°"
            print(f"    {r.draw.start.strftime('%d/%m %H:%M')}: "
                  f"{r.definitive_cumulative:.0f}L — {ctx}")


def main():
    import argparse
    parser = argparse.ArgumentParser(
        description="DHW T1 inflection detector",
        epilog="Default: writes to InfluxDB + JSON to stdout. Use --human for readable output."
    )
    parser.add_argument("--days", type=int, default=12, help="Days of history to analyse")
    parser.add_argument("--human", action="store_true", help="Human-readable output instead of JSON")
    parser.add_argument("--verbose", action="store_true", help="Full per-draw table (implies --human)")
    parser.add_argument("--no-write", action="store_true", help="Don't write to InfluxDB")
    args = parser.parse_args()

    if args.verbose:
        args.human = True

    charges, draws = find_events(args.days)

    results: list[InflectionResult] = []
    for draw in draws:
        result = analyse_draw(draw)
        if result is not None:
            results.append(result)

    # Always write to InfluxDB unless --no-write
    if not args.no_write:
        write_to_influxdb(results)

    # Output
    if args.human:
        output_human(results, args.days, args.verbose)
    else:
        output_json(results)


if __name__ == "__main__":
    main()
