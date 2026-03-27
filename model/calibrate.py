"""
Calibration using direct InfluxDB data + external TOML config.

Usage:
  INFLUX_TOKEN=... uv run --with influxdb-client python model/calibrate.py
  uv run --with influxdb-client python model/calibrate.py model/thermal-config.toml
"""

from __future__ import annotations

import csv
import math
import os
import sys
import tomllib
from copy import deepcopy
from dataclasses import dataclass
from datetime import datetime, timedelta
from pathlib import Path

from influxdb_client import InfluxDBClient

sys.path.insert(0, str(Path(__file__).parent))
import house
from house import (
    Doorway,
    build_connections,
    build_doorways,
    build_rooms,
    build_sensor_map,
    estimate_thermal_mass,
    room_energy_balance,
)


@dataclass
class Window:
    start: datetime
    end: datetime


@dataclass
class Bounds:
    leather_ach: tuple[float, float, float]
    landing_ach: tuple[float, float, float]
    conservatory_ach: tuple[float, float, float]
    office_ach: tuple[float, float, float]
    doorway_cd: tuple[float, float, float]


@dataclass
class Priors:
    landing_ach: float
    doorway_cd: float


@dataclass
class Config:
    influx_url: str
    influx_org: str
    influx_bucket: str
    influx_token_env: str
    night1: Window
    night2: Window
    exclude_rooms: set[str]
    prior_weight: float
    priors: Priors
    bounds: Bounds


def parse_dt(s: str) -> datetime:
    return datetime.fromisoformat(s.replace("Z", "+00:00"))


def load_config(path: Path) -> Config:
    with open(path, "rb") as f:
        raw = tomllib.load(f)

    n = raw["test_nights"]
    b = raw["bounds"]
    p = raw["priors"]

    return Config(
        influx_url=raw["influx"]["url"],
        influx_org=raw["influx"]["org"],
        influx_bucket=raw["influx"]["bucket"],
        influx_token_env=raw["influx"]["token_env"],
        night1=Window(parse_dt(n["night1_start"]), parse_dt(n["night1_end"])),
        night2=Window(parse_dt(n["night2_start"]), parse_dt(n["night2_end"])),
        exclude_rooms=set(raw["objective"].get("exclude_rooms", [])),
        prior_weight=float(raw["objective"].get("prior_weight", 0.0)),
        priors=Priors(
            landing_ach=float(p["landing_ach"]),
            doorway_cd=float(p["doorway_cd"]),
        ),
        bounds=Bounds(
            leather_ach=(b["leather_ach_min"], b["leather_ach_max"], b["leather_ach_step"]),
            landing_ach=(b["landing_ach_min"], b["landing_ach_max"], b["landing_ach_step"]),
            conservatory_ach=(b["conservatory_ach_min"], b["conservatory_ach_max"], b["conservatory_ach_step"]),
            office_ach=(b["office_ach_min"], b["office_ach_max"], b["office_ach_step"]),
            doorway_cd=(b["doorway_cd_min"], b["doorway_cd_max"], b["doorway_cd_step"]),
        ),
    )


def frange(start: float, end: float, step: float) -> list[float]:
    out = []
    x = start
    while x <= end + 1e-12:
        out.append(round(x, 6))
        x += step
    return out


def make_night2_doors_closed(doors: list[Doorway]) -> list[Doorway]:
    out = []
    for d in doors:
        d2 = Doorway(d.room_a, d.room_b, d.width, d.height, d.state)
        if d2.state != "chimney":
            d2.state = "closed"
        out.append(d2)
    return out


def fetch_influx_series(cfg: Config, sensor_topics: set[str], start: datetime, stop: datetime):
    token = os.getenv(cfg.influx_token_env)
    if not token:
        raise RuntimeError(
            f"Missing env var {cfg.influx_token_env}. Set it (e.g. export {cfg.influx_token_env}=...)."
        )

    client = InfluxDBClient(url=cfg.influx_url, token=token, org=cfg.influx_org)
    api = client.query_api()

    # Build regex for sensor topics
    escaped = [t.replace("/", "\\/") for t in sorted(sensor_topics)]
    topics_re = "|".join(escaped)

    start_s = start.isoformat()
    stop_s = stop.isoformat()

    room_query = f'''
    from(bucket: "{cfg.influx_bucket}")
      |> range(start: {start_s}, stop: {stop_s})
      |> filter(fn: (r) =>
           (r._field == "temperature" and r.topic =~ /{topics_re}/) or
           (r.topic == "emon/emonth2_23/temperature" and r._field == "value")
      )
      |> aggregateWindow(every: 5m, fn: mean, createEmpty: false)
      |> keep(columns: ["_time", "topic", "_value", "_field"])
    '''

    outside_query = f'''
    from(bucket: "{cfg.influx_bucket}")
      |> range(start: {start_s}, stop: {stop_s})
      |> filter(fn: (r) => r.topic == "ebusd/poll/OutsideTemp")
      |> aggregateWindow(every: 5m, fn: mean, createEmpty: false)
      |> keep(columns: ["_time", "_value"])
    '''

    room_rows: list[tuple[datetime, str, float]] = []
    out_rows: list[tuple[datetime, float]] = []

    for table in api.query(room_query):
        for rec in table.records:
            t = rec.get_time()
            topic = rec.values.get("topic", "")
            val = float(rec.get_value())
            room_rows.append((t, topic, val))

    for table in api.query(outside_query):
        for rec in table.records:
            out_rows.append((rec.get_time(), float(rec.get_value())))

    client.close()
    room_rows.sort(key=lambda x: x[0])
    out_rows.sort(key=lambda x: x[0])
    return room_rows, out_rows


def build_room_series(room_rows: list[tuple[datetime, str, float]], sensor_map: dict[str, str]):
    series: dict[str, list[tuple[datetime, float]]] = {}
    for t, topic, value in room_rows:
        if topic not in sensor_map:
            continue
        room = sensor_map[topic]
        series.setdefault(room, []).append((t, value))

    for room in series:
        series[room].sort(key=lambda x: x[0])
    return series


def measured_rates(window: Window, room_series, outside_series):
    outside_vals = [v for t, v in outside_series if window.start <= t <= window.end]
    if not outside_vals:
        raise RuntimeError(f"No outside temp data for window {window.start} -> {window.end}")
    outside_avg = sum(outside_vals) / len(outside_vals)

    rates = {}
    avg_temps = {}
    for room, pts in room_series.items():
        p = [(t, v) for t, v in pts if window.start <= t <= window.end]
        if len(p) < 2:
            continue
        hours = (p[-1][0] - p[0][0]).total_seconds() / 3600
        if hours < 0.5:
            continue
        rates[room] = (p[0][1] - p[-1][1]) / hours
        avg_temps[room] = sum(v for _, v in p) / len(p)

    return rates, avg_temps, outside_avg


def predict_rates(
    rooms_base,
    connections,
    doorways,
    avg_temps,
    outside_temp,
    leather_ach,
    landing_ach,
    conservatory_ach,
    office_ach,
    doorway_cd,
):
    rooms = deepcopy(rooms_base)
    rooms["leather"].ventilation_ach = leather_ach
    rooms["landing"].ventilation_ach = landing_ach
    rooms["conservatory"].ventilation_ach = conservatory_ach
    rooms["office"].ventilation_ach = office_ach

    old_cd = house.DOORWAY_CD
    house.DOORWAY_CD = doorway_cd

    pred = {}
    for name, room in rooms.items():
        if name not in avg_temps:
            continue
        C = estimate_thermal_mass(room, connections)
        bal = room_energy_balance(room, avg_temps[name], outside_temp, avg_temps, connections, doorways, mwt=0, sleeping=True)
        pred[name] = -bal["total"] * 3.6 / C if C > 0 else 0.0

    house.DOORWAY_CD = old_cd
    return pred


def rmse(measured, predicted, exclude_rooms: set[str]) -> float:
    errs = []
    for room, m in measured.items():
        if room in exclude_rooms:
            continue
        if room not in predicted:
            continue
        errs.append((m - predicted[room]) ** 2)
    if not errs:
        return 999.0
    return math.sqrt(sum(errs) / len(errs))


def report_table(title, measured, predicted):
    print(f"\n{title}")
    print(f"{'Room':<14} {'Measured':>8} {'Pred':>8} {'Ratio':>6} {'Err':>8}")
    print("─" * 50)
    for room in sorted(measured.keys()):
        m = measured[room]
        p = predicted.get(room, float("nan"))
        ratio = p / m if abs(m) > 1e-9 else 0.0
        err = p - m
        print(f"{room:<14} {m:>8.3f} {p:>8.3f} {ratio:>6.2f} {err:>+8.3f}")


def main():
    cfg_path = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("model/thermal-config.toml")
    cfg = load_config(cfg_path)

    rooms = build_rooms()
    connections = build_connections()
    doors_n1 = build_doorways()
    doors_n2 = make_night2_doors_closed(build_doorways())
    sensor_map = build_sensor_map(rooms)

    # Fetch direct from Influx for just the needed range (+ buffer)
    start = min(cfg.night1.start, cfg.night2.start) - timedelta(hours=1)
    stop = max(cfg.night1.end, cfg.night2.end) + timedelta(hours=1)
    room_rows, outside_rows = fetch_influx_series(cfg, set(sensor_map.keys()), start, stop)

    room_series = build_room_series(room_rows, sensor_map)
    meas1, avg1, out1 = measured_rates(cfg.night1, room_series, outside_rows)
    meas2, avg2, out2 = measured_rates(cfg.night2, room_series, outside_rows)

    print(f"Config: {cfg_path}")
    print(f"Night1: {cfg.night1.start} -> {cfg.night1.end} (outside avg {out1:.1f}°C)")
    print(f"Night2: {cfg.night2.start} -> {cfg.night2.end} (outside avg {out2:.1f}°C)")
    print(f"Exclude rooms in objective: {sorted(cfg.exclude_rooms) if cfg.exclude_rooms else 'none'}")

    b = cfg.bounds
    best = None

    for leather in frange(*b.leather_ach):
        for landing in frange(*b.landing_ach):
            for cons in frange(*b.conservatory_ach):
                for office in frange(*b.office_ach):
                    for cd in frange(*b.doorway_cd):
                        p1 = predict_rates(rooms, connections, doors_n1, avg1, out1, leather, landing, cons, office, cd)
                        p2 = predict_rates(rooms, connections, doors_n2, avg2, out2, leather, landing, cons, office, cd)
                        r1 = rmse(meas1, p1, cfg.exclude_rooms)
                        r2 = rmse(meas2, p2, cfg.exclude_rooms)
                        base_score = (r1 + r2) / 2

                        prior_penalty = cfg.prior_weight * (
                            ((landing - cfg.priors.landing_ach) / 0.3) ** 2
                            + ((cd - cfg.priors.doorway_cd) / 0.08) ** 2
                        )
                        score = base_score + prior_penalty

                        if best is None or score < best[0]:
                            best = (score, base_score, r1, r2, leather, landing, cons, office, cd, p1, p2)

    score, base_score, r1, r2, leather, landing, cons, office, cd, p1, p2 = best

    print("\n" + "=" * 72)
    print("BEST FIT (direct Influx + config-driven bounds)")
    print("=" * 72)
    print(f"leather_ach      = {leather:.2f}")
    print(f"landing_ach      = {landing:.2f}")
    print(f"conservatory_ach = {cons:.2f}")
    print(f"office_ach       = {office:.2f}")
    print(f"doorway_cd       = {cd:.2f}")
    print(f"rmse_night1      = {r1:.4f}")
    print(f"rmse_night2      = {r2:.4f}")
    print(f"base_score       = {base_score:.4f}")
    print(f"final_score      = {score:.4f}")

    report_table("Night 1 fit", meas1, p1)
    report_table("Night 2 fit", meas2, p2)


if __name__ == "__main__":
    main()
