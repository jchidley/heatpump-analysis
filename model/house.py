"""
Thermal network model of 6 Rhodes Avenue.

Physics:
    For each room i, the energy balance is:
        C_i × dT_i/dt = Q_rad_i + Q_body_i + Q_solar_i
                       - Q_ext_i                    (fabric to outside)
                       - Q_vent_i                   (ventilation to outside)
                       - Σ_j Q_wall_ij              (conduction to adjacent rooms)
                       - Σ_j Q_door_ij              (convective exchange through doorways)

    All inter-room connections are SYMMETRIC — defined once, applied to both rooms.
    Doorway exchange uses buoyancy-driven convection physics, not U-value hacks.

Known: fabric U×A (from spreadsheet), radiator T50s, room adjacencies, pipe topology.
Measured: 13 room temps (Zigbee + emonth2), outside temp (eBUS), HP heat output.
Fitted: ventilation rates (from cooldown experiments).
Calculated: thermal mass (from construction), inter-room conduction (from U×A).

Usage:
    uv run --with influxdb-client --with numpy --with scipy python model/house.py fetch [hours]
    uv run --with influxdb-client --with numpy --with scipy python model/house.py rooms
    uv run --with influxdb-client --with numpy --with scipy python model/house.py analyse
    uv run --with influxdb-client --with numpy --with scipy python model/house.py fit
    uv run --with influxdb-client --with numpy --with scipy python model/house.py moisture
"""

import os
import sys
import json
import csv
import math
import statistics
from dataclasses import dataclass, field
from pathlib import Path
from datetime import datetime, timezone, timedelta


# ---------------------------------------------------------------------------
# Physical constants
# ---------------------------------------------------------------------------

AIR_DENSITY = 1.2       # kg/m³
AIR_CP = 1005           # J/kg·K
VENT_FACTOR = AIR_DENSITY * AIR_CP / 3600  # 0.335 W per m³/h per °C

GROUND_TEMP_C = 10.5    # Assumed constant ground temperature

# Radiator exponent (EN442 standard)
RAD_EXPONENT = 1.3

# Internal wall U-value: 100mm brick + 12mm plaster both sides
# R = 0.13 + 0.012/0.57 + 0.100/0.84 + 0.012/0.57 + 0.13 = 0.421
# U = 1/0.421 = 2.37 W/m²K
U_INTERNAL_WALL = 2.37  # W/m²K — single brick, plastered both sides

# Uninsulated timber floor/ceiling (joist + boards + plasterboard)
# R = 0.10 + 0.025/0.13 + 0.18 (air gap) + 0.0125/0.21 + 0.10 = 0.632
# U = 1/0.632 = 1.58 W/m²K
U_TIMBER_FLOOR = 1.58   # W/m²K — uninsulated timber floor between rooms

# Buoyancy-driven doorway exchange constants.
# Calibrated from Night 1 (doors normal) vs Night 2 (all doors closed),
# 24-26 Mar 2026. Joint calibration: landing_ach=1.30, Cd=0.20 gives
# RMSE=0.057°/h, all 13 rooms within factor 2.
#
# The stairwell chimney (hall↔landing↔shower; top-landing proxy via shower)
# is now modelled explicitly as buoyancy links using doorway state="chimney".
# This replaces the previous "disabled chimney doorway + high landing ACH" hack.
# Other doorways use the same Cd with their own geometry/state.
DOORWAY_CD = 0.20       # Discharge coefficient (calibrated from Night 1 vs Night 2)
DOORWAY_G = 9.81        # m/s²


# ---------------------------------------------------------------------------
# Thermal mass constants (kJ/K per m² of surface)
# ---------------------------------------------------------------------------

THERMAL_MASS = {
    "air":           1.2,    # per m³ (ρ×Cp/1000)
    "brick_int":    72.0,    # half of 100mm brick wall (shared with neighbour)
    "brick_ext":    72.0,    # inner leaf of external wall
    "concrete":    200.0,    # ground floor slab, top 100mm
    "timber_floor": 50.0,    # joists + boards
    "plaster":      17.0,    # 12mm plaster layer
    "furniture":    15.0,    # contents per m² floor area
    "timber_stud":  10.0,    # lightweight stud wall per m²
}


# ---------------------------------------------------------------------------
# Data types
# ---------------------------------------------------------------------------

@dataclass
class RadiatorDef:
    """A single radiator."""
    t50: float          # W at ΔT=50K
    pipe: str           # "22mm" or "15mm_branch1" or "15mm_branch2"
    active: bool = True


@dataclass
class ExternalElement:
    """A fabric element losing heat to outside or ground."""
    description: str
    area: float         # m²
    u_value: float      # W/m²K
    to_ground: bool = False  # True = ΔT to ground temp, False = ΔT to outside


@dataclass
class InternalConnection:
    """Heat conduction between two rooms through a shared wall/floor/ceiling.

    Defined ONCE — the physics is symmetric (same U×A applies in both directions).
    """
    room_a: str
    room_b: str
    ua: float           # U × A in W/K
    description: str


@dataclass
class Doorway:
    """Convective air exchange through an open doorway or stairwell opening.

    Uses buoyancy-driven flow: Q = f(ΔT^1.5), not linear U×A.
    Defined ONCE per connection. State can change between experiments.
    """
    room_a: str
    room_b: str
    width: float        # m
    height: float       # m
    state: str = "open"  # "open", "closed", "partial"


@dataclass
class SolarGlazing:
    """Solar gain properties for a room's glazing."""
    area: float             # m² of glazing on this face
    orientation: str        # "SW" or "NE"
    tilt: str = "vertical"  # "vertical", "sloping", "horizontal" (roof)
    g_value: float = 0.7    # Solar transmittance (0.7 old DG, 0.5 triple)
    shading: float = 1.0    # 1.0 = unshaded, 0.0 = fully shaded


@dataclass
class RoomDef:
    """Definition of a room — physical properties only, no analysis results."""
    name: str
    floor: str                          # "Gnd", "1st", "Loft"
    floor_area: float                   # m²
    ceiling_height: float               # m
    construction: str = "brick"         # "brick", "brick_suspended", "timber"
    radiators: list[RadiatorDef] = field(default_factory=list)
    external_fabric: list[ExternalElement] = field(default_factory=list)
    solar: list[SolarGlazing] = field(default_factory=list)
    sensor: str = ""                    # InfluxDB topic
    ventilation_ach: float = 0.3        # Air changes/hour to OUTSIDE only
    heat_recovery: float = 0.0          # 0.0 = none, 0.78 = MVHR
    overnight_occupants: int = 0        # For body heat and moisture


# ---------------------------------------------------------------------------
# Occupant heat
# ---------------------------------------------------------------------------

BODY_HEAT_SLEEPING_W = 70       # W per person, sleeping
BODY_HEAT_ACTIVE_W = 100        # W per person, sedentary

# DHW cylinder and pipework parasitic heat to bathroom.
# Kingspan Albion 300L at ~45°C in ~22°C room.
# Measured from T1 standby decline: UA ≈ 1.6 W/K (161L effective mass).
# Pipes: 8m of 35mm insulated (to HP) + 8m of 22mm insulated (to loft rads).
#   Time-weighted pipe loss: ~42W (insulated copper, CH + DHW modes).
# Showers: 2/day, ~16W time-averaged residual heat in tiles/air.
# Remaining ~216W deficit unexplained — bathroom external UA may be overstated,
# or 23:00 measurement was not true equilibrium (post-shower thermal mass).
DHW_CYLINDER_UA = 1.6           # W/K (cylinder insulation loss)
DHW_CYLINDER_TEMP = 44.0        # °C (average T1 during standby)
DHW_PIPE_LOSS_W = 42            # W (16m insulated copper, time-weighted CH+DHW)
DHW_SHOWER_W = 16               # W (2 showers/day, time-averaged residual)
DHW_ROOM = "bathroom"           # Room containing the cylinder

# Moisture generation (g/h per person)
MOISTURE_PERSON_SLEEPING = 40
MOISTURE_PERSON_ACTIVE = 55


# ---------------------------------------------------------------------------
# Thermal mass estimation
# ---------------------------------------------------------------------------

def estimate_thermal_mass(room: RoomDef, connections: list[InternalConnection]) -> float:
    """Estimate thermal mass in kJ/K from construction.

    For overnight cooldowns (8h), thermal penetration depth in brick is ~180mm,
    so the full 100mm internal wall participates.
    """
    vol = room.floor_area * room.ceiling_height
    C = 0.0

    # Air
    C += THERMAL_MASS["air"] * vol

    # External walls — get areas from external fabric
    for elem in room.external_fabric:
        if "wall" in elem.description.lower():
            if room.construction in ("brick", "brick_suspended"):
                C += THERMAL_MASS["brick_ext"] * elem.area
            else:
                C += THERMAL_MASS["timber_stud"] * elem.area
            C += THERMAL_MASS["plaster"] * elem.area

    # Internal walls — get areas from connections (each room gets half)
    for conn in connections:
        if conn.room_a == room.name or conn.room_b == room.name:
            # Infer wall area from UA: area = UA / U_internal_wall
            if conn.ua > 0:
                implied_area = conn.ua / U_INTERNAL_WALL
                if room.construction in ("brick", "brick_suspended"):
                    C += THERMAL_MASS["brick_int"] * implied_area
                else:
                    C += THERMAL_MASS["timber_stud"] * implied_area
                C += THERMAL_MASS["plaster"] * implied_area

    # Floor
    if room.floor == "Gnd" and room.construction != "brick_suspended":
        C += THERMAL_MASS["concrete"] * room.floor_area
    else:
        C += THERMAL_MASS["timber_floor"] * room.floor_area

    # Ceiling (plaster)
    C += THERMAL_MASS["plaster"] * room.floor_area

    # Furniture and contents
    C += THERMAL_MASS["furniture"] * room.floor_area

    return C


# ---------------------------------------------------------------------------
# Physics calculations
# ---------------------------------------------------------------------------

def radiator_output(t50: float, mwt: float, room_temp: float) -> float:
    """Radiator heat output in Watts. EN442: Q = T50 × (ΔT/50)^1.3"""
    dt = mwt - room_temp
    if dt <= 0:
        return 0.0
    return t50 * (dt / 50.0) ** RAD_EXPONENT


def external_loss(elements: list[ExternalElement], room_temp: float,
                  outside_temp: float) -> float:
    """Total fabric heat loss to outside/ground in Watts."""
    total = 0.0
    for elem in elements:
        ref_temp = GROUND_TEMP_C if elem.to_ground else outside_temp
        total += elem.u_value * elem.area * (room_temp - ref_temp)
    return total


def ventilation_loss(ach: float, volume: float, room_temp: float,
                     outside_temp: float, heat_recovery: float = 0.0) -> float:
    """Ventilation heat loss to outside in Watts.
    Q = 0.335 × ACH × V × ΔT × (1 - η)
    """
    return VENT_FACTOR * ach * volume * (room_temp - outside_temp) * (1.0 - heat_recovery)


def wall_conduction(ua: float, temp_a: float, temp_b: float) -> float:
    """Heat flow through internal wall from A to B in Watts.
    Positive = heat flows A→B (A is warmer).
    """
    return ua * (temp_a - temp_b)


def virtual_room_temp(name: str, all_temps: dict[str, float]) -> float | None:
    """Return measured or virtual room temperature for model-only nodes."""
    if name in all_temps:
        return all_temps[name]
    if name == "top_landing":
        t_landing = all_temps.get("landing")
        t_shower = all_temps.get("shower")
        if t_landing is not None and t_shower is not None:
            return (t_landing + t_shower) / 2.0
        return t_landing if t_landing is not None else t_shower
    return None


def doorway_exchange(door: Doorway, temp_a: float, temp_b: float) -> float:
    """Buoyancy-driven convective heat exchange through a doorway in Watts.

    Uses the standard formula for bi-directional flow through an opening:
        Q = (Cd/3) × W × sqrt(g × H³ × |ΔT| / T_mean) × ρ × Cp × ΔT

    Returns heat flow from A to B (positive = A is warmer).
    For a closed door, returns 0.
    """
    if door.state == "closed":
        return 0.0

    dt = temp_a - temp_b
    if abs(dt) < 0.01:
        return 0.0

    t_mean = (temp_a + temp_b) / 2 + 273.15  # Kelvin

    width = door.width
    if door.state == "partial":
        width *= 0.5  # Half-open approximation

    # Volume flow rate through one half of the doorway (m³/s)
    flow = (DOORWAY_CD / 3.0) * width * math.sqrt(
        DOORWAY_G * door.height ** 3 * abs(dt) / t_mean
    )

    # Heat exchange (W)
    return flow * AIR_DENSITY * AIR_CP * dt


def occupant_heat(room: RoomDef, sleeping: bool = True) -> float:
    """Occupant body heat in Watts."""
    rate = BODY_HEAT_SLEEPING_W if sleeping else BODY_HEAT_ACTIVE_W
    return room.overnight_occupants * rate


def solar_gain(room: RoomDef, irradiance_sw: float = 0.0, irradiance_ne: float = 0.0) -> float:
    """Solar heat gain through glazing in Watts.

    irradiance_sw: W/m² on a vertical SW-facing surface.
    irradiance_ne: W/m² on a vertical NE-facing surface.

    Tilt correction: sloping glazing (e.g., velux on 45° roof) gets ~1.4×
    more irradiance than vertical at moderate solar altitudes. Horizontal
    roof glazing gets more in summer, less in winter — use 1.2× average.
    """
    TILT_FACTOR = {"vertical": 1.0, "sloping": 1.4, "horizontal": 1.2}

    total = 0.0
    for sg in room.solar:
        irr = irradiance_sw if sg.orientation == "SW" else irradiance_ne
        tilt = TILT_FACTOR.get(sg.tilt, 1.0)
        total += irr * sg.area * sg.g_value * sg.shading * tilt
    return total


# ---------------------------------------------------------------------------
# House definition
# ---------------------------------------------------------------------------

def _thermal_geometry_path() -> Path:
    return Path(__file__).resolve().parents[1] / "data" / "canonical" / "thermal_geometry.json"


_THERMAL_GEOMETRY: dict | None = None


def load_thermal_geometry() -> dict:
    global _THERMAL_GEOMETRY
    if _THERMAL_GEOMETRY is None:
        path = _thermal_geometry_path()
        _THERMAL_GEOMETRY = json.loads(path.read_text())
    return _THERMAL_GEOMETRY


# ---------------------------------------------------------------------------
# House definition (loaded from canonical geometry file)
# ---------------------------------------------------------------------------

def build_rooms() -> dict[str, RoomDef]:
    """Define all rooms from canonical geometry file (no hardcoded dimensions)."""
    geo = load_thermal_geometry()
    rooms: dict[str, RoomDef] = {}

    for r in geo["rooms"]:
        rooms[r["name"]] = RoomDef(
            name=r["name"],
            floor=r["floor"],
            floor_area=float(r["floor_area"]),
            ceiling_height=float(r["ceiling_height"]),
            construction=r.get("construction", "brick"),
            sensor=r.get("sensor", ""),
            ventilation_ach=float(r.get("ventilation_ach", 0.0)),
            heat_recovery=float(r.get("heat_recovery", 0.0)),
            overnight_occupants=int(r.get("overnight_occupants", 0)),
            radiators=[
                RadiatorDef(
                    t50=float(rad["t50"]),
                    pipe=rad.get("pipe", "22mm"),
                    active=bool(rad.get("active", True)),
                )
                for rad in r.get("radiators", [])
            ],
            external_fabric=[
                ExternalElement(
                    description=e["description"],
                    area=float(e["area"]),
                    u_value=float(e["u_value"]),
                    to_ground=bool(e.get("to_ground", False)),
                )
                for e in r.get("external_fabric", [])
            ],
            solar=[
                SolarGlazing(
                    area=float(s["area"]),
                    orientation=s["orientation"],
                    tilt=s.get("tilt", "vertical"),
                    g_value=float(s.get("g_value", 0.7)),
                    shading=float(s.get("shading", 1.0)),
                )
                for s in r.get("solar", [])
            ],
        )

    return rooms


def build_connections() -> list[InternalConnection]:
    """Define all internal wall/floor/ceiling connections from canonical geometry file."""
    geo = load_thermal_geometry()
    return [
        InternalConnection(
            room_a=c["room_a"],
            room_b=c["room_b"],
            ua=float(c["ua"]),
            description=c.get("description", ""),
        )
        for c in geo.get("connections", [])
    ]


def build_doorways() -> list[Doorway]:
    """Define all doorway/stairwell openings from canonical geometry file."""
    geo = load_thermal_geometry()
    return [
        Doorway(
            room_a=d["room_a"],
            room_b=d["room_b"],
            width=float(d["width"]),
            height=float(d["height"]),
            state=d["state"],
        )
        for d in geo.get("doorways", [])
    ]


PIPE_BRANCHES = {
    "22mm": [
        "leather.0", "leather.1", "conservatory.0", "conservatory.1",
        "front.1", "bathroom.0", "bathroom.1", "sterling.0",
        "elvina.0", "aldora.0", "shower.0",
    ],
    "15mm_branch1": ["front.0", "hall.0"],
    "15mm_branch2": ["jackcarol.0", "office.0"],
}


# ---------------------------------------------------------------------------
# Sensor → room mapping (built once, used everywhere)
# ---------------------------------------------------------------------------

def build_sensor_map(rooms: dict[str, RoomDef]) -> dict[str, str]:
    """Map sensor topic → room name. Used by all analysis functions."""
    return {room.sensor: name for name, room in rooms.items() if room.sensor}


# ---------------------------------------------------------------------------
# Complete energy balance for a single room
# ---------------------------------------------------------------------------

def room_energy_balance(
    room: RoomDef,
    room_temp: float,
    outside_temp: float,
    all_temps: dict[str, float],
    connections: list[InternalConnection],
    doorways: list[Doorway],
    mwt: float = 0.0,
    sleeping: bool = True,
    irradiance_sw: float = 0.0,
    irradiance_ne: float = 0.0,
) -> dict[str, float]:
    """Calculate all heat flows for a room. Returns dict of component flows in Watts.

    Positive values = heat INTO the room. Negative = heat OUT.
    irradiance_sw/ne: solar irradiance in W/m² on vertical SW/NE surfaces.
    """
    name = room.name
    vol = room.floor_area * room.ceiling_height

    # External fabric loss (always out)
    q_ext = -external_loss(room.external_fabric, room_temp, outside_temp)

    # Ventilation to outside (always out)
    q_vent = -ventilation_loss(room.ventilation_ach, vol, room_temp,
                                outside_temp, room.heat_recovery)

    # Radiator heat input
    q_rad = sum(radiator_output(r.t50, mwt, room_temp)
                for r in room.radiators if r.active) if mwt > 0 else 0.0

    # Occupant body heat
    q_body = occupant_heat(room, sleeping)

    # Solar gain through glazing
    q_solar = solar_gain(room, irradiance_sw, irradiance_ne)

    # DHW cylinder parasitic heat (bathroom only)
    q_dhw = 0.0
    if name == DHW_ROOM:
        q_dhw = (DHW_CYLINDER_UA * max(0, DHW_CYLINDER_TEMP - room_temp)
                 + DHW_PIPE_LOSS_W + DHW_SHOWER_W)

    # Internal wall conduction (can be + or -)
    q_walls = 0.0
    for conn in connections:
        if conn.room_a == name:
            other_t = virtual_room_temp(conn.room_b, all_temps)
            if other_t is not None:
                q_walls -= wall_conduction(conn.ua, room_temp, other_t)
        elif conn.room_b == name:
            other_t = virtual_room_temp(conn.room_a, all_temps)
            if other_t is not None:
                q_walls -= wall_conduction(conn.ua, room_temp, other_t)

    # Doorway exchange (can be + or -)
    q_doors = 0.0
    for door in doorways:
        if door.room_a == name:
            other_t = virtual_room_temp(door.room_b, all_temps)
            if other_t is not None:
                q_doors -= doorway_exchange(door, room_temp, other_t)
        elif door.room_b == name:
            other_t = virtual_room_temp(door.room_a, all_temps)
            if other_t is not None:
                q_doors -= doorway_exchange(door, room_temp, other_t)

    return {
        "external": q_ext,
        "ventilation": q_vent,
        "radiator": q_rad,
        "body": q_body,
        "solar": q_solar,
        "dhw": q_dhw,
        "walls": q_walls,
        "doorways": q_doors,
        "total": q_ext + q_vent + q_rad + q_body + q_solar + q_dhw + q_walls + q_doors,
    }


# ---------------------------------------------------------------------------
# Data fetching from InfluxDB
# ---------------------------------------------------------------------------

DATA_DIR = Path(__file__).parent / "data"

INFLUX_URL = "http://pi5data:8086"
INFLUX_TOKEN = os.environ.get("INFLUX_TOKEN", "")
INFLUX_ORG = "home"
INFLUX_BUCKET = "energy"

if not INFLUX_TOKEN:
    import subprocess
    try:
        INFLUX_TOKEN = subprocess.check_output(["ak", "get", "influxdb"], text=True).strip()
    except (FileNotFoundError, subprocess.CalledProcessError):
        pass

if not INFLUX_TOKEN:
    print("WARNING: INFLUX_TOKEN not set. Run: export INFLUX_TOKEN=$(ak get influxdb)", file=sys.stderr)


def fetch_data(hours: int = 24):
    """Fetch room temps, outside temp, and HP state from InfluxDB."""
    from influxdb_client import InfluxDBClient

    DATA_DIR.mkdir(exist_ok=True)
    client = InfluxDBClient(url=INFLUX_URL, token=INFLUX_TOKEN, org=INFLUX_ORG)
    api = client.query_api()

    # Room temperatures (zigbee + emonth2)
    query = f'''
    from(bucket: "{INFLUX_BUCKET}")
      |> range(start: -{hours}h)
      |> filter(fn: (r) =>
           (r.topic =~ /temp_humid/ and (r._field == "temperature" or r._field == "humidity")) or
           (r.topic == "emon/emonth2_23/temperature" and r._field == "value") or
           (r.topic == "emon/emonth2_23/humidity" and r._field == "value")
         )
      |> aggregateWindow(every: 5m, fn: mean, createEmpty: false)
      |> keep(columns: ["_time", "topic", "_field", "_value"])
    '''
    _write_query(api, query, DATA_DIR / "room_temps.csv",
                 ["time", "topic", "field", "value"],
                 lambda r: [r.get_time().isoformat(), r.values.get("topic", ""),
                           r.values.get("_field", ""), r.get_value()])

    # Outside temperature (eBUS)
    query = f'''
    from(bucket: "{INFLUX_BUCKET}")
      |> range(start: -{hours}h)
      |> filter(fn: (r) => r.topic == "ebusd/poll/OutsideTemp")
      |> aggregateWindow(every: 5m, fn: mean, createEmpty: false)
      |> keep(columns: ["_time", "_value"])
    '''
    _write_query(api, query, DATA_DIR / "outside_temp.csv",
                 ["time", "value"],
                 lambda r: [r.get_time().isoformat(), r.get_value()])

    # HP state
    query = f'''
    from(bucket: "{INFLUX_BUCKET}")
      |> range(start: -{hours}h)
      |> filter(fn: (r) =>
           r.topic == "emon/heatpump/heatmeter_Power" or
           r.topic == "emon/heatpump/heatmeter_FlowT" or
           r.topic == "emon/heatpump/heatmeter_ReturnT" or
           r.topic == "emon/heatpump/heatmeter_FlowRate" or
           r.topic == "emon/heatpump/electric_Power"
         )
      |> aggregateWindow(every: 5m, fn: mean, createEmpty: false)
      |> keep(columns: ["_time", "topic", "_value"])
    '''
    _write_query(api, query, DATA_DIR / "hp_state.csv",
                 ["time", "topic", "value"],
                 lambda r: [r.get_time().isoformat(), r.values.get("topic", ""),
                           r.get_value()])

    # eBUS status code
    query = f'''
    from(bucket: "{INFLUX_BUCKET}")
      |> range(start: -{hours}h)
      |> filter(fn: (r) => r.topic == "ebusd/poll/StatuscodeNum")
      |> aggregateWindow(every: 5m, fn: mean, createEmpty: false)
      |> keep(columns: ["_time", "_value"])
    '''
    _write_query(api, query, DATA_DIR / "hp_status.csv",
                 ["time", "value"],
                 lambda r: [r.get_time().isoformat(), r.get_value()])

    client.close()
    print(f"Fetched {hours}h of data to {DATA_DIR}/")


def _write_query(api, query: str, path: Path, headers: list, row_fn):
    """Execute a Flux query and write results to CSV."""
    tables = api.query(query)
    with open(path, "w", newline="") as f:
        writer = csv.writer(f)
        writer.writerow(headers)
        for table in tables:
            for record in table.records:
                writer.writerow(row_fn(record))
    print(f"  Wrote {path.name}")


# ---------------------------------------------------------------------------
# Data loading
# ---------------------------------------------------------------------------

def load_csv(filename: str) -> list[dict]:
    """Load a CSV file from the data directory."""
    path = DATA_DIR / filename
    if not path.exists():
        print(f"ERROR: {path} not found. Run 'fetch' first.")
        sys.exit(1)
    with open(path) as f:
        return list(csv.DictReader(f))


def parse_time(s: str) -> datetime:
    """Parse ISO timestamp."""
    s = s.replace("Z", "+00:00")
    return datetime.fromisoformat(s)


def load_room_temps(room_data: list[dict], sensor_map: dict[str, str]) -> dict[str, float]:
    """Extract latest room temperatures from CSV data. Returns {room: temp}."""
    temps = {}
    for row in room_data:
        topic = row["topic"]
        field = row["field"]
        is_temp = (field == "temperature") or (field == "value" and "temperature" in topic)
        if is_temp and topic in sensor_map:
            temps[sensor_map[topic]] = float(row["value"])
    return temps


def load_room_series(room_data: list[dict], sensor_map: dict[str, str]) -> dict[str, list]:
    """Build time series per room: {room: [(datetime, temp), ...]}."""
    series = {}
    for row in room_data:
        topic = row["topic"]
        field = row["field"]
        is_temp = (field == "temperature") or (field == "value" and "temperature" in topic)
        if not is_temp or topic not in sensor_map:
            continue
        room_name = sensor_map[topic]
        series.setdefault(room_name, []).append((parse_time(row["time"]), float(row["value"])))
    for name in series:
        series[name].sort(key=lambda x: x[0])
    return series


# ---------------------------------------------------------------------------
# Steady-state analysis
# ---------------------------------------------------------------------------

def analyse():
    """Energy balance per room at latest snapshot."""
    rooms = build_rooms()
    connections = build_connections()
    doorways = build_doorways()
    sensor_map = build_sensor_map(rooms)

    room_data = load_csv("room_temps.csv")
    outside_data = load_csv("outside_temp.csv")
    hp_data = load_csv("hp_state.csv")

    room_temps = load_room_temps(room_data, sensor_map)
    outside_temp = float(outside_data[-1]["value"]) if outside_data else 10.0

    # HP state
    hp_vals = {}
    for row in hp_data:
        hp_vals[row["topic"].split("/")[-1]] = float(row["value"])
    hp_heat = hp_vals.get("heatmeter_Power", 0)
    hp_flow_t = hp_vals.get("heatmeter_FlowT", 30)
    hp_return_t = hp_vals.get("heatmeter_ReturnT", 27)
    hp_elec = hp_vals.get("electric_Power", 0)
    mwt = (hp_flow_t + hp_return_t) / 2

    print("=" * 110)
    print("STEADY-STATE ENERGY BALANCE")
    print(f"Outside: {outside_temp:.1f}°C | HP: {hp_heat:.0f}W heat, {hp_elec:.0f}W elec | "
          f"Flow: {hp_flow_t:.1f}°C, Return: {hp_return_t:.1f}°C, MWT: {mwt:.1f}°C")
    print("=" * 110)

    hdr = f"{'Room':<14} {'T°C':>5} {'ExtFab':>7} {'Vent':>6} {'Walls':>7} {'Doors':>6} {'Body':>5} {'DHW':>5} {'NetLoss':>8} {'Rad':>6} {'Resid':>7}"
    print(f"\n{hdr}")
    print("─" * len(hdr))

    sum_keys = ["external", "ventilation", "walls", "doorways", "body", "solar", "dhw", "radiator"]
    totals = {k: 0.0 for k in sum_keys}

    for name in sorted(rooms.keys()):
        t = room_temps.get(name)
        if t is None:
            continue
        bal = room_energy_balance(rooms[name], t, outside_temp, room_temps,
                                  connections, doorways, mwt, sleeping=False)
        for k in totals:
            totals[k] += bal[k]

        net_loss = -(bal["external"] + bal["ventilation"] + bal["walls"] + bal["doorways"])
        dhw_str = f"{bal['dhw']:>5.0f}" if bal["dhw"] > 0 else "     "
        print(f"{name:<14} {t:>5.1f} {-bal['external']:>7.0f} {-bal['ventilation']:>6.0f} "
              f"{-bal['walls']:>7.0f} {-bal['doorways']:>6.0f} {bal['body']:>5.0f} "
              f"{dhw_str} {net_loss - bal['body'] - bal['dhw']:>8.0f} {bal['radiator']:>6.0f} {bal['total']:>+7.0f}")

    print("─" * len(hdr))
    total_loss = -(totals["external"] + totals["ventilation"] + totals["walls"] + totals["doorways"])
    print(f"{'Total':<14} {'':>5} {-totals['external']:>7.0f} {-totals['ventilation']:>6.0f} "
          f"{-totals['walls']:>7.0f} {-totals['doorways']:>6.0f} {totals['body']:>5.0f} "
          f"{totals['dhw']:>5.0f} {total_loss - totals['body'] - totals['dhw']:>8.0f} {totals['radiator']:>6.0f}")
    print(f"{'HP meter':<14} {'':>5} {'':>7} {'':>6} {'':>7} {'':>6} {'':>5} {'':>5} {'':>8} {hp_heat:>6.0f}")


# ---------------------------------------------------------------------------
# Cooldown fitting
# ---------------------------------------------------------------------------

def fit():
    """Analyse cooldown periods: compare measured dT/dt with model prediction."""
    rooms = build_rooms()
    connections = build_connections()
    doorways = build_doorways()
    sensor_map = build_sensor_map(rooms)

    room_data = load_csv("room_temps.csv")
    outside_data = load_csv("outside_temp.csv")
    status_data = load_csv("hp_status.csv")

    room_series = load_room_series(room_data, sensor_map)
    outside_series = [(parse_time(r["time"]), float(r["value"])) for r in outside_data]
    outside_series.sort(key=lambda x: x[0])

    # Find heating-off periods from eBUS status
    HEATING_OFF_CODES = {100, 101, 103, 134}  # standby + pump overrun + DHW
    status_series = [(parse_time(r["time"]), float(r["value"])) for r in status_data]
    status_series.sort(key=lambda x: x[0])

    cooldown_periods = []
    in_cooldown = False
    start = None
    for t, code in status_series:
        if int(code) in HEATING_OFF_CODES:
            if not in_cooldown:
                start = t
                in_cooldown = True
        else:
            if in_cooldown and start:
                duration = (t - start).total_seconds() / 3600
                if duration > 0.25:
                    cooldown_periods.append((start, t))
            in_cooldown = False
    if in_cooldown and start:
        end = status_series[-1][0]
        if (end - start).total_seconds() / 3600 > 0.25:
            cooldown_periods.append((start, end))

    if not cooldown_periods:
        print("No cooldown periods found. Run 'fetch' after a heating-off period.")
        return

    print(f"Found {len(cooldown_periods)} cooldown periods:")
    for s, e in cooldown_periods:
        hours = (e - s).total_seconds() / 3600
        print(f"  {s.strftime('%H:%M')} → {e.strftime('%H:%M')} ({hours:.1f}h)")

    # For each cooldown period, compare measured vs predicted cooling rates
    print(f"\n{'Room':<14} {'Start':>7} {'End':>7} {'Meas':>7} {'Pred':>7} {'Ratio':>6} {'Body':>5} {'Period'}")
    print(f"{'':14} {'°C':>7} {'°C':>7} {'°C/hr':>7} {'°C/hr':>7} {'P/M':>6} {'W':>5}")
    print("─" * 80)

    MIN_MEAS_COOLING = 0.03  # °C/hr: below this, treat as weak/non-cooling for fit confidence

    records = []

    for period_start, period_end in cooldown_periods:
        # Get average outside temp during period
        outside_in_period = [v for t, v in outside_series if period_start <= t <= period_end]
        avg_outside = sum(outside_in_period) / len(outside_in_period) if outside_in_period else 8.0

        for room_name, series in sorted(room_series.items()):
            temps_in_period = [(t, v) for t, v in series if period_start <= t <= period_end]
            if len(temps_in_period) < 2:
                continue
            t_start = temps_in_period[0][1]
            t_end = temps_in_period[-1][1]
            hours = (temps_in_period[-1][0] - temps_in_period[0][0]).total_seconds() / 3600
            if hours < 0.25:
                continue

            meas_rate = (t_start - t_end) / hours

            # Model prediction: get average temps for all rooms during period
            avg_temps = {}
            for rn, rs in room_series.items():
                vals = [v for t, v in rs if period_start <= t <= period_end]
                if vals:
                    avg_temps[rn] = sum(vals) / len(vals)

            room = rooms[room_name]
            avg_t = avg_temps.get(room_name, (t_start + t_end) / 2)
            C = estimate_thermal_mass(room, connections)

            bal = room_energy_balance(room, avg_t, avg_outside, avg_temps,
                                       connections, doorways, mwt=0, sleeping=True)
            # Predicted rate: dT/dt = Q_total / C (converting W and kJ/K)
            pred_rate = -bal["total"] * 3.6 / C if C > 0 else 0

            body_w = occupant_heat(room, sleeping=True)
            ratio = pred_rate / meas_rate if abs(meas_rate) > 0.01 else float("nan")
            is_true_cooling = meas_rate >= MIN_MEAS_COOLING
            marker = " " if is_true_cooling else "*"

            period_str = f"{period_start.strftime('%H:%M')}→{period_end.strftime('%H:%M')}"
            ratio_str = f"{ratio:>6.2f}" if math.isfinite(ratio) else "   nan"
            print(f"{room_name:<14} {t_start:>7.2f} {t_end:>7.2f} {meas_rate:>7.3f} {pred_rate:>7.3f} "
                  f"{ratio_str} {body_w:>5.0f} {period_str}{marker}")

            records.append(
                {
                    "room": room_name,
                    "meas": meas_rate,
                    "pred": pred_rate,
                    "ratio": ratio,
                    "true_cooling": is_true_cooling,
                }
            )

    def summarize(rows):
        if not rows:
            return {"n": 0, "rmse": float("nan"), "mae": float("nan"), "med_ratio": float("nan")}
        errs = [r["pred"] - r["meas"] for r in rows]
        rmse = math.sqrt(sum(e * e for e in errs) / len(errs))
        mae = sum(abs(e) for e in errs) / len(errs)
        ratios = [r["ratio"] for r in rows if math.isfinite(r["ratio"])]
        med_ratio = statistics.median(ratios) if ratios else float("nan")
        return {"n": len(rows), "rmse": rmse, "mae": mae, "med_ratio": med_ratio}

    print("\n* marks weak/non-cooling measured periods (meas_rate < 0.03 °C/hr)\n")

    all_stats = summarize(records)
    good = [r for r in records if r["true_cooling"]]
    good_stats = summarize(good)

    print("Summary (all records):")
    print(f"  N={all_stats['n']}  RMSE={all_stats['rmse']:.3f} °C/hr  MAE={all_stats['mae']:.3f} °C/hr  Median ratio={all_stats['med_ratio']:.2f}")
    print("Summary (true cooling only):")
    print(f"  N={good_stats['n']}  RMSE={good_stats['rmse']:.3f} °C/hr  MAE={good_stats['mae']:.3f} °C/hr  Median ratio={good_stats['med_ratio']:.2f}")

    print("\nPer-room summary (true cooling only):")
    print(f"{'Room':<14} {'N':>4} {'RMSE':>8} {'MAE':>8} {'MedRat':>8}")
    print("─" * 46)
    by_room = {}
    for r in good:
        by_room.setdefault(r["room"], []).append(r)
    for room_name in sorted(by_room):
        st = summarize(by_room[room_name])
        print(f"{room_name:<14} {st['n']:>4d} {st['rmse']:>8.3f} {st['mae']:>8.3f} {st['med_ratio']:>8.2f}")


# ---------------------------------------------------------------------------
# Moisture analysis
# ---------------------------------------------------------------------------

def moisture_analysis():
    """Analyse overnight humidity to cross-validate ventilation rates."""
    rooms = build_rooms()
    sensor_map = build_sensor_map(rooms)

    room_data = load_csv("room_temps.csv")
    outside_data = load_csv("outside_temp.csv")

    outside_temps = [float(r["value"]) for r in outside_data]
    avg_outside = sum(outside_temps) / len(outside_temps) if outside_temps else 8.0

    # Build {room: {time_str: {temp:, rh:}}}
    room_readings = {}
    for row in room_data:
        topic = row["topic"]
        if topic not in sensor_map:
            continue
        room_name = sensor_map[topic]
        t_key = row["time"][:16]
        room_readings.setdefault(room_name, {}).setdefault(t_key, {})

        field = row["field"]
        is_temp = (field == "temperature") or (field == "value" and "temperature" in topic)
        is_humid = (field == "humidity") or (field == "value" and "humidity" in topic)
        if is_temp:
            room_readings[room_name][t_key]["temp"] = float(row["value"])
        elif is_humid:
            room_readings[room_name][t_key]["rh"] = float(row["value"])

    # Outside AH from Open-Meteo
    outside_ah, outside_rh = _fetch_outside_humidity(outside_data, avg_outside)

    print("=" * 100)
    print("MOISTURE ANALYSIS")
    print(f"Outside: {avg_outside:.1f}°C, ~{outside_rh:.0f}% RH → AH {outside_ah:.1f} g/m³")
    print("=" * 100)

    # Current snapshot — surface RH uses physics-based surface temp
    # T_surface = T_air - U_max × Rsi × (T_air - T_outside)
    # where U_max is the highest U-value external element (coldest surface)
    Rsi = 0.13  # Internal surface resistance, m²K/W
    print(f"\n{'Room':<14} {'T°C':>5} {'RH%':>5} {'AH g/m³':>8} {'U_max':>6} {'T_surf':>6} {'SurfRH':>7} {'Risk':>6}")
    print("─" * 65)
    for room_name in sorted(rooms.keys()):
        room = rooms[room_name]
        if room_name not in room_readings:
            continue
        latest = _latest_reading(room_readings[room_name])
        if not latest:
            continue
        ah_val = _absolute_humidity(latest["temp"], latest["rh"])
        # Find worst-case surface: highest U-value external element
        u_max = max((e.u_value for e in room.external_fabric if not e.to_ground), default=0)
        if u_max > 0:
            t_surface = latest["temp"] - u_max * Rsi * (latest["temp"] - avg_outside)
        else:
            t_surface = latest["temp"] - 1.0  # Internal room, minimal surface depression
        s_rh = _surface_rh(latest["temp"], latest["rh"], t_surface)
        risk = "HIGH" if s_rh > 80 else "WARN" if s_rh > 70 else "watch" if latest["rh"] > 60 else "OK"
        print(f"{room_name:<14} {latest['temp']:>5.1f} {latest['rh']:>5.1f} {ah_val:>8.1f} "
              f"{u_max:>6.2f} {t_surface:>6.1f} {s_rh:>7.1f} {risk:>6}")

    # Overnight moisture balance
    print(f"\n{'─' * 100}")
    print("OVERNIGHT MOISTURE BALANCE")
    print(f"{'─' * 100}")
    print(f"\n{'Room':<14} {'Occ':>3} {'AH_23':>7} {'AH_06':>7} {'ΔAH':>6} {'ACH_moist':>10} {'ACH_therm':>10} {'Match':>6}")
    print(f"{'':14} {'':>3} {'g/m³':>7} {'g/m³':>7} {'g/m³':>6} {'(total)':>10} {'(to out)':>10}")
    print("─" * 75)

    for room_name in sorted(rooms.keys()):
        if room_name not in room_readings:
            continue
        room = rooms[room_name]
        vol = room.floor_area * room.ceiling_height
        occ = room.overnight_occupants

        ah_23, ah_06 = _overnight_ah(room_readings[room_name])
        if ah_23 is None or ah_06 is None:
            continue

        delta_ah = ah_06 - ah_23
        hours = 7.0
        moisture_rate = occ * MOISTURE_PERSON_SLEEPING / vol  # g/m³/h
        observed_rate = delta_ah / hours
        vent_removal = moisture_rate - observed_rate
        ah_avg = (ah_23 + ah_06) / 2
        ah_diff = ah_avg - outside_ah
        ach_moisture = vent_removal / ah_diff if ah_diff > 0.5 else 0

        # Cross-validate against thermal model ACH
        ach_thermal = room.ventilation_ach * (1.0 - room.heat_recovery)
        if occ > 0 and ach_moisture > 0:
            # Moisture ACH includes inter-room exchange; thermal ACH is to-outside only
            # Moisture ACH ≥ thermal ACH always (doorway exchange adds to moisture)
            match = "✓" if abs(ach_moisture - ach_thermal) < 0.3 else "≠"
        else:
            match = "-"  # Can't validate without occupants

        print(f"{room_name:<14} {occ:>3} {ah_23:>7.2f} {ah_06:>7.2f} {delta_ah:>+6.2f} "
              f"{ach_moisture:>10.2f} {ach_thermal:>10.2f} {match:>6}")

    print(f"\n  ACH_moist = total air exchange (to outside + inter-room), from humidity change")
    print(f"  ACH_therm = to outside only, from thermal model")
    print(f"  ACH_moist ≥ ACH_therm expected (doorway exchange adds to moisture but not thermal)")
    print(f"  Moisture rate: {MOISTURE_PERSON_SLEEPING} g/h/person (±25% → ±50% ACH uncertainty)")


def _absolute_humidity(temp_c: float, rh_pct: float) -> float:
    """Absolute humidity in g/m³ from T and RH (Magnus formula)."""
    es = 6.112 * math.exp(17.67 * temp_c / (temp_c + 243.5))
    return 217.0 * (rh_pct / 100.0) * es / (temp_c + 273.15)


def _surface_rh(air_temp: float, air_rh: float, surface_temp: float) -> float:
    """RH at a surface colder than room air."""
    es_air = 6.112 * math.exp(17.67 * air_temp / (air_temp + 243.5))
    e = (air_rh / 100.0) * es_air
    es_surface = 6.112 * math.exp(17.67 * surface_temp / (surface_temp + 243.5))
    return min(100.0, (e / es_surface) * 100.0)


def _latest_reading(readings: dict) -> dict | None:
    """Get latest reading with both temp and rh."""
    for t in sorted(readings.keys(), reverse=True):
        if "temp" in readings[t] and "rh" in readings[t]:
            return readings[t]
    return None


def _overnight_ah(readings: dict) -> tuple[float | None, float | None]:
    """Get absolute humidity at ~23:00 and ~06:00."""
    ah_23 = ah_06 = None
    for t_str in sorted(readings.keys()):
        r = readings[t_str]
        if "temp" not in r or "rh" not in r:
            continue
        h = int(t_str[11:13])
        ah = _absolute_humidity(r["temp"], r["rh"])
        if h == 23 and ah_23 is None:
            ah_23 = ah
        if h == 6:
            ah_06 = ah
    return ah_23, ah_06


def _fetch_outside_humidity(outside_data, avg_outside):
    """Get outside AH from Open-Meteo. Falls back to 75% RH."""
    import urllib.request
    try:
        first_date = outside_data[0]["time"][:10] if outside_data else "2026-03-25"
        last_date = outside_data[-1]["time"][:10] if outside_data else "2026-03-25"
        url = (f"https://api.open-meteo.com/v1/forecast?"
               f"latitude=51.59&longitude=-0.14"
               f"&hourly=relative_humidity_2m,temperature_2m"
               f"&timezone=Europe/London"
               f"&start_date={first_date}&end_date={last_date}")
        with urllib.request.urlopen(url, timeout=10) as resp:
            meteo = json.loads(resp.read())
        ah_vals, rh_vals = [], []
        for i, t_str in enumerate(meteo["hourly"]["time"]):
            h = int(t_str[11:13])
            if h >= 22 or h <= 7:
                t = meteo["hourly"]["temperature_2m"][i]
                rh = meteo["hourly"]["relative_humidity_2m"][i]
                ah_vals.append(_absolute_humidity(t, rh))
                rh_vals.append(rh)
        if ah_vals:
            return sum(ah_vals) / len(ah_vals), sum(rh_vals) / len(rh_vals)
    except Exception:
        pass
    return _absolute_humidity(avg_outside, 75.0), 75.0


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

def cmd_rooms():
    """Print room summary table."""
    rooms = build_rooms()
    connections = build_connections()

    print(f"{'Room':<14} {'Flr':>4} {'Area':>5} {'Vol':>5} {'C kJ/K':>7} {'T50':>6} "
          f"{'extUA':>7} {'ACH':>5} {'effACH':>6} {'Occ':>3} {'Pipe':>15}")
    print("─" * 100)

    total_C = 0
    for name in sorted(rooms.keys()):
        room = rooms[name]
        vol = room.floor_area * room.ceiling_height
        C = estimate_thermal_mass(room, connections)
        total_C += C
        total_t50 = sum(r.t50 for r in room.radiators if r.active)
        ext_ua = sum(e.u_value * e.area for e in room.external_fabric)
        eff_ach = room.ventilation_ach * (1.0 - room.heat_recovery)
        pipe = room.radiators[0].pipe if room.radiators else "none"
        print(f"{name:<14} {room.floor:>4} {room.floor_area:>4.1f}m² {vol:>4.0f}m³ "
              f"{C:>6.0f} {total_t50:>5.0f}W {ext_ua:>6.1f}W/K "
              f"{room.ventilation_ach:>5.2f} {eff_ach:>6.2f} {room.overnight_occupants:>3} "
              f"{pipe:>15}")

    print(f"{'─' * 100}")
    print(f"{'Total':<14} {'':>4} {'':>5} {'':>5} {total_C:>6.0f}")


def cmd_connections():
    """Print all inter-room connections."""
    connections = build_connections()
    doorways = build_doorways()

    print("INTERNAL WALL/FLOOR CONNECTIONS (symmetric)")
    print(f"{'A↔B':<30} {'UA W/K':>8} {'Description'}")
    print("─" * 60)
    for c in connections:
        print(f"{c.room_a}↔{c.room_b:<16} {c.ua:>8.1f} {c.description}")

    print(f"\nDOORWAY EXCHANGES (buoyancy-driven)")
    print(f"{'A↔B':<30} {'W×H':>8} {'State':>8}")
    print("─" * 50)
    for d in doorways:
        print(f"{d.room_a}↔{d.room_b:<16} {d.width:.1f}×{d.height:.1f} {d.state:>8}")


def cmd_equilibrium():
    """Solve for equilibrium room temperatures at given outside temp and MWT."""
    import numpy as np
    from scipy.optimize import fsolve

    rooms = build_rooms()
    connections = build_connections()
    doorways = build_doorways()
    sensor_map = build_sensor_map(rooms)
    room_names = sorted(rooms.keys())
    N = len(room_names)

    # Load data for current conditions
    outside_data = load_csv("outside_temp.csv")
    hp_data = load_csv("hp_state.csv")

    outside_temp = float(outside_data[-1]["value"]) if outside_data else 10.0
    hp_vals = {}
    for row in hp_data:
        hp_vals[row["topic"].split("/")[-1]] = float(row["value"])
    hp_flow_t = hp_vals.get("heatmeter_FlowT", 33)
    hp_return_t = hp_vals.get("heatmeter_ReturnT", 29)
    mwt = (hp_flow_t + hp_return_t) / 2

    # Override from CLI args: equilibrium [T_out] [MWT] [solar_sw] [solar_ne]
    if len(sys.argv) > 2:
        outside_temp = float(sys.argv[2])
    if len(sys.argv) > 3:
        mwt = float(sys.argv[3])
    irr_sw = float(sys.argv[4]) if len(sys.argv) > 4 else 0.0
    irr_ne = float(sys.argv[5]) if len(sys.argv) > 5 else 0.0

    def equations(temps_arr):
        temps = {name: temps_arr[i] for i, name in enumerate(room_names)}
        res = np.zeros(N)
        for i, name in enumerate(room_names):
            bal = room_energy_balance(rooms[name], temps[name], outside_temp, temps,
                                      connections, doorways, mwt=mwt, sleeping=False,
                                      irradiance_sw=irr_sw, irradiance_ne=irr_ne)
            res[i] = bal["total"]
        return res

    x0 = np.full(N, 19.0)
    solution = fsolve(equations, x0)
    pred = {name: solution[i] for i, name in enumerate(room_names)}

    print("=" * 70)
    print(f"EQUILIBRIUM TEMPERATURES (T_out={outside_temp:.1f}°C, MWT={mwt:.1f}°C)")
    print("=" * 70)

    print(f"\n{'Room':<14} {'Temp':>6} {'Rad_in':>7} {'Ext_out':>8} {'Vent_out':>9} {'Notes'}")
    print("─" * 60)
    for name in room_names:
        t = pred[name]
        bal = room_energy_balance(rooms[name], t, outside_temp, pred,
                                  connections, doorways, mwt=mwt, sleeping=True)
        notes = ""
        if not rooms[name].radiators or not any(r.active for r in rooms[name].radiators):
            notes = "no active rad"
        elif t < 18:
            notes = "COLD"
        print(f"{name:<14} {t:>5.1f}° {bal['radiator']:>6.0f}W {-bal['external']:>7.0f}W "
              f"{-bal['ventilation']:>8.0f}W  {notes}")

    # Design summary
    heated = [n for n in room_names
              if rooms[n].radiators and any(r.active for r in rooms[n].radiators)]
    if heated:
        coldest = min(heated, key=lambda n: pred[n])
        print(f"\nColdest heated room: {coldest} at {pred[coldest]:.1f}°C")
        if pred[coldest] < 18:
            print(f"  → needs higher MWT to reach 18°C")


def main():
    if len(sys.argv) < 2:
        print("Usage: python model/house.py <command>")
        print("Commands: fetch [hours], rooms, connections, analyse, fit,")
        print("          equilibrium [T_out] [MWT] [solar_sw W/m²] [solar_ne W/m²],")
        print("          moisture")
        sys.exit(1)

    cmd = sys.argv[1]
    if cmd == "fetch":
        hours = int(sys.argv[2]) if len(sys.argv) > 2 else 24
        fetch_data(hours)
    elif cmd == "rooms":
        cmd_rooms()
    elif cmd == "connections":
        cmd_connections()
    elif cmd == "analyse":
        analyse()
    elif cmd == "fit":
        fit()
    elif cmd == "equilibrium":
        cmd_equilibrium()
    elif cmd == "moisture":
        moisture_analysis()
    else:
        print(f"Unknown command: {cmd}")
        sys.exit(1)


if __name__ == "__main__":
    main()
