"""
Thermal network model of 6 Rhodes Avenue.

Known: fabric U×A, radiator T50s, room adjacencies, pipe topology.
Measured: room temps (11 sensors), outside temp (eBUS), HP heat output.
Fitted: thermal mass per room, ventilation rates, effective radiator outputs.

Usage:
    uv run --with influxdb-client --with numpy --with scipy python model/house.py fetch
    uv run --with influxdb-client --with numpy --with scipy python model/house.py analyse
    uv run --with influxdb-client --with numpy --with scipy python model/house.py fit
"""

import sys
import json
import csv
from dataclasses import dataclass, field
from pathlib import Path
from datetime import datetime, timezone, timedelta

# ---------------------------------------------------------------------------
# Room and building definitions — all from spreadsheet + physical survey
# ---------------------------------------------------------------------------

@dataclass
class RadiatorDef:
    """A single radiator."""
    t50: float          # W at ΔT=50K
    area: float         # m² surface area
    pipe: str           # "22mm" or "15mm_branch1" or "15mm_branch2"
    rad_type: str       # e.g. "DP DF", "TP TF", "Towel", "SP SF"
    active: bool = True # Sterling is OFF

@dataclass
class FabricElement:
    """A single fabric element (external wall, window, floor, roof, etc.)."""
    description: str
    area: float         # m²
    u_value: float      # W/m²K
    dt_type: str        # "external" (to outside), "ground" (to ground), "internal" (to named room)
    adjacent_room: str = ""  # for internal elements

@dataclass
class RoomDef:
    """Definition of a room."""
    name: str
    floor: str                          # "Gnd", "1st", "Loft"
    floor_area: float                   # m²
    ceiling_height: float               # m
    radiators: list[RadiatorDef] = field(default_factory=list)
    fabric: list[FabricElement] = field(default_factory=list)
    sensor: str = ""                    # InfluxDB topic or emoncms feed
    door_state: str = "open"            # "open", "closed", "partial", "sometimes"
    notes: str = ""

# Ground temp assumption: outside - (outside_temp - 10.5) * 0.5 ≈ 10.5°C average
GROUND_TEMP_OFFSET = 10.5  # °C — ground temp assumed constant at this


# ---------------------------------------------------------------------------
# Ventilation model
# ---------------------------------------------------------------------------
# Three groups calibrated from known data points:
#   1. MVHR room (bathroom) — known: 9 L/s extract, 78% heat recovery
#   2. Stairwell-connected (open door to hall) — driven partly by MVHR draft
#   3. Closed rooms — background infiltration only
#
# The MVHR creates a defined airflow path through the house:
#   Outside → front door → hall → stairwell → landing → bathroom → MVHR → outside
# Rooms with open doors to the stairwell are part of this circuit.
# Rooms with closed doors are decoupled (infiltration only).

@dataclass
class VentilationDef:
    """Ventilation parameters for a room."""
    ach: float              # Air changes per hour (total, before heat recovery)
    heat_recovery: float    # 0.0 = none, 0.78 = MVHR
    source: str             # "measured" (MVHR), "calibrated" (from Sterling/kitchen), "estimated"
    notes: str = ""

    @property
    def effective_ach(self) -> float:
        """ACH after heat recovery — this is what drives heat loss."""
        return self.ach * (1.0 - self.heat_recovery)


# MVHR: Vent-Axia Tempra LP, 9 L/s continuous, 78% heat recovery
MVHR_FLOW_LS = 9.0          # L/s
MVHR_FLOW_M3H = MVHR_FLOW_LS * 3.6  # 32.4 m³/h
MVHR_HEAT_RECOVERY = 0.78   # 78% temperature efficiency
BATHROOM_VOLUME = 18.0 * 2.4  # 43.2 m³
MVHR_ACH = MVHR_FLOW_M3H / BATHROOM_VOLUME  # 0.75 ACH

# Ventilation parameters per room
# These are initial estimates — Sterling and Kitchen values will be fitted from data
VENTILATION = {
    # MVHR room — KNOWN
    "bathroom": VentilationDef(
        ach=MVHR_ACH, heat_recovery=MVHR_HEAT_RECOVERY,
        source="measured",
        notes=f"Vent-Axia Tempra LP, {MVHR_FLOW_LS} L/s continuous, {MVHR_HEAT_RECOVERY*100:.0f}% recovery"
    ),

    # Stairwell-connected rooms (open doors) — ventilation depends on bathroom door state.
    # Bathroom door OPEN: MVHR pulls 32.4 m³/h through the house via:
    #   Outside → front door → hall → stairwell → landing → bathroom → MVHR → outside
    #   This adds ~0.2-0.3 ACH equivalent to stairwell-connected rooms.
    # Bathroom door CLOSED (e.g. after evening showers):
    #   MVHR pulls through door gaps only. Stairwell draft greatly reduced.
    #   Rooms revert to natural infiltration rates.
    #
    # Current state: bathroom door CLOSED since ~21:00.
    # Using reduced ventilation rates for stairwell-connected rooms.
    "hall": VentilationDef(
        ach=0.5, heat_recovery=0.0, source="estimated",
        notes="Front door SE face + stairwell base. MVHR draft reduced (bathroom door closed)."
    ),
    "kitchen": VentilationDef(
        ach=0.35, heat_recovery=0.0, source="calibrate_from_data",
        notes="Open to hall + conservatory. MVHR draft (bathroom door dependent). "
              "Hob hood extract fan during cooking (intermittent, not modelled overnight). "
              "Absolute humidity data: -1.5 g/m³ overnight = highest moisture loss in house, "
              "all from infiltration via hall connection. TO BE FITTED."
    ),
    "conservatory": VentilationDef(
        ach=0.3, heat_recovery=0.0, source="estimated",
        notes="Open to kitchen. Glazed roof seals. MVHR draft reduced."
    ),
    "shower": VentilationDef(
        ach=0.2, heat_recovery=0.0, source="estimated",
        notes="Door open to top of stairwell. MVHR draft reduced."
    ),

    # Closed-door rooms — background infiltration only
    # Calibrated from Sterling (rad off, door closed, triple glazed)
    "sterling": VentilationDef(
        ach=0.15, heat_recovery=0.0, source="calibrate_from_data",
        notes="Rad OFF, door closed, triple glazed. Lowest ventilation. TO BE FITTED."
    ),
    "leather": VentilationDef(
        ach=0.2, heat_recovery=0.0, source="estimated",
        notes="Door closed. SG door to conservatory may leak. Background infiltration."
    ),
    "front": VentilationDef(
        ach=0.3, heat_recovery=0.0, source="estimated",
        notes="Partial door. Bay window SE face. Some infiltration."
    ),
    "jackcarol": VentilationDef(
        ach=0.35, heat_recovery=0.0, source="estimated_from_humidity",
        notes="Door closed at night, open daytime. Bay window SE face — leaky. "
              "Humidity evidence: 2 people yet RH DROPPED 0.4% overnight = significant "
              "infiltration through bay window frame joints. Similar leakiness to Elvina."
    ),
    "office": VentilationDef(
        ach=0.2, heat_recovery=0.0, source="estimated",
        notes="Door closed. SE face."
    ),
    "elvina": VentilationDef(
        ach=0.5, heat_recovery=0.0, source="estimated_from_humidity",
        notes="Door closed but TRICKLE VENTS OPEN. Sloping roof = warm air rises to ceiling "
              "and exits via vents. This is the primary heat loss mechanism, not fabric. "
              "Humidity: 1 person only +6% RH in 61m³ = high air exchange. "
              "Closing trickle vents would dramatically reduce heat loss (but need moisture path)."
    ),
    "aldora": VentilationDef(
        ach=0.10, heat_recovery=0.0, source="estimated_from_humidity",
        notes="Door closed. Flat roof — very well sealed. Humidity evidence: "
              "1 person +10% RH in 31m³ = minimal ventilation. Moisture risk >60% overnight. "
              "Needs trickle vent or door opened periodically."
    ),
}

def build_house() -> dict[str, RoomDef]:
    """Build the complete house model from known data."""

    rooms = {}

    # ── GROUND FLOOR ──────────────────────────────────────────────

    rooms["hall"] = RoomDef(
        name="hall", floor="Gnd", floor_area=9.72, ceiling_height=2.6,
        sensor="zigbee2mqtt/hall_temp_humid",
        door_state="open",
        notes="Stairwell base, front door SE face. Continuous to landing/top_landing.",
        radiators=[
            RadiatorDef(t50=2376, area=1.08, pipe="15mm_branch1", rad_type="DP DF"),
        ],
        fabric=[
            # Ground floor section
            FabricElement("External Wall (solid brick)", 16.80, 2.11, "external"),
            FabricElement("External Floor", 9.72, 0.77, "ground"),
            FabricElement("Ceiling (to 1st floor)", 9.72, 1.71, "internal", "office"),
            FabricElement("Internal Wall", 18.72, 1.76, "internal", "_open_zone"),  # open to kitchen etc
            FabricElement("Windows (old double)", 1.92, 1.9, "external"),
            # Loft-level stairwell section (2010 insulation)
            FabricElement("Loft Wall (insulated)", 10.66, 0.15, "external"),
            FabricElement("Loft Ceiling", 3.3, 0.066, "external"),
            FabricElement("Loft Windows", 1.44, 1.5, "external"),
            FabricElement("Loft External Floor", 13.2, 0.77, "ground"),
        ],
    )

    rooms["kitchen"] = RoomDef(
        name="kitchen", floor="Gnd", floor_area=8.8, ceiling_height=2.6,
        sensor="zigbee2mqtt/kitchen_temp_humid",
        door_state="open",
        notes="NO radiator. Open to hall and conservatory. Below bathroom.",
        radiators=[],  # No radiator!
        fabric=[
            FabricElement("External Wall (solid brick)", 8.96, 2.11, "external"),
            FabricElement("External Floor", 8.8, 0.77, "ground"),
            FabricElement("Ceiling (to bathroom)", 8.8, 1.71, "internal", "bathroom"),
            FabricElement("Internal Wall", 21.84, 1.76, "internal", "_open_zone"),
            FabricElement("Windows (old double)", 1.44, 1.9, "external"),
        ],
    )

    rooms["leather"] = RoomDef(
        name="leather", floor="Gnd", floor_area=17.0, ceiling_height=2.6,
        sensor="emon/emonth2_23/temperature",
        door_state="closed",
        notes="Heat hub. No external walls. Door closed. Below Sterling. SG door to conservatory.",
        radiators=[
            RadiatorDef(t50=2376, area=1.08, pipe="22mm", rad_type="DP DF"),
            RadiatorDef(t50=2376, area=1.08, pipe="22mm", rad_type="DP DF"),
        ],
        fabric=[
            FabricElement("Ceiling/floor (to Sterling)", 17.0, 1.41, "internal", "sterling"),
            FabricElement("External Floor", 17.0, 0.77, "ground"),
            FabricElement("Internal Wall (to front)", 10.0, 1.76, "internal", "front"),  # estimate
            FabricElement("Internal Wall (to kitchen)", 8.0, 1.76, "internal", "kitchen"),  # estimate
            FabricElement("Internal Wall (to hall)", 8.0, 1.76, "internal", "hall"),  # estimate
            FabricElement("Internal Wall (small brick)", 3.0, 2.11, "internal", "_misc"),
            FabricElement("Windows/SG door to conservatory", 4.8, 1.9, "internal", "conservatory"),
        ],
    )

    rooms["front"] = RoomDef(
        name="front", floor="Gnd", floor_area=16.34, ceiling_height=2.6,
        sensor="zigbee2mqtt/front_temp_humid",
        door_state="partial",
        notes="Bay window SE face. Shared wall with leather and hall. Below Jack&Carol.",
        radiators=[
            RadiatorDef(t50=2425, area=0.84, pipe="15mm_branch1", rad_type="DP DF"),  # horizontal, shared 15mm with hall
            RadiatorDef(t50=2376, area=1.08, pipe="22mm", rad_type="DP DF"),  # vertical, short tail off 22mm
        ],
        fabric=[
            FabricElement("External Wall (solid brick, bay)", 8.14, 2.11, "external"),
            FabricElement("External Floor", 16.34, 0.77, "ground"),
            FabricElement("Ceiling (to Jack&Carol)", 16.34, 1.71, "internal", "jackcarol"),
            FabricElement("Internal Wall", 32.24, 1.76, "internal", "_mixed"),  # leather, hall
            FabricElement("Windows (new double, bay)", 7.2, 1.2, "external"),
        ],
    )

    rooms["conservatory"] = RoomDef(
        name="conservatory", floor="Gnd", floor_area=21.0, ceiling_height=2.6,
        sensor="zigbee2mqtt/conservatory_temp_humid",
        door_state="open",
        notes="Dining room. DG walls (U=0.5, yr 2000) + DG roof (U=2.4). Open to kitchen. SG door from leather.",
        radiators=[
            RadiatorDef(t50=2833, area=0.60, pipe="22mm", rad_type="TP TF"),
            RadiatorDef(t50=2867, area=0.72, pipe="22mm", rad_type="TP TF"),
        ],
        fabric=[
            FabricElement("External Wall (DG, yr 2000)", 15.4, 0.5, "external"),
            FabricElement("External Floor", 21.0, 0.77, "ground"),
            FabricElement("Glazed Roof (DG)", 21.0, 2.4, "external"),
            FabricElement("Windows (DG)", 9.0, 1.9, "external"),
        ],
    )

    # ── FIRST FLOOR ───────────────────────────────────────────────

    rooms["sterling"] = RoomDef(
        name="sterling", floor="1st", floor_area=18.0, ceiling_height=2.4,
        sensor="zigbee2mqtt/Sterling_temp_humid",
        door_state="closed",
        notes="Rad OFF. Triple glazed flat wall single unit. Above leather. Door closed.",
        radiators=[
            RadiatorDef(t50=1176, area=0.7254, pipe="22mm", rad_type="SP SF", active=False),
        ],
        fabric=[
            FabricElement("Ceiling (to loft)", 18.0, 0.44, "external"),  # loft above
            FabricElement("External Wall (solid brick)", 6.12, 2.11, "external"),
            FabricElement("Floor (from leather below)", 18.0, 1.41, "internal", "leather"),
            FabricElement("Internal Wall", 32.64, 1.76, "internal", "_mixed"),  # bathroom, jackcarol
            FabricElement("Windows (triple, flat wall)", 2.52, 1.0, "external"),
        ],
    )

    rooms["jackcarol"] = RoomDef(
        name="jackcarol", floor="1st", floor_area=14.28, ceiling_height=2.4,
        sensor="zigbee2mqtt/jackcarol_temp_humid",
        door_state="open_day_closed_night",
        notes="Bay window SE face. Above front. Shares 15mm branch with office.",
        radiators=[
            RadiatorDef(t50=1950, area=0.72, pipe="15mm_branch2", rad_type="DP DF"),
        ],
        fabric=[
            FabricElement("Ceiling (to loft)", 14.28, 0.44, "external"),
            FabricElement("External Wall (solid brick, bay)", 6.69, 2.11, "external"),
            FabricElement("Floor (from front below)", 14.28, 1.41, "internal", "front"),
            FabricElement("Internal Wall", 28.56, 1.76, "internal", "_mixed"),  # sterling, office
            FabricElement("Windows (new double, bay)", 6.75, 1.2, "external"),
        ],
    )

    rooms["bathroom"] = RoomDef(
        name="bathroom", floor="1st", floor_area=18.0, ceiling_height=2.4,
        sensor="zigbee2mqtt/bathroom_temp_humid",
        door_state="sometimes",
        notes="Triple glazed flat wall. 2 exposed walls. Above kitchen. Door sometimes closed.",
        radiators=[
            RadiatorDef(t50=614, area=1.08, pipe="22mm", rad_type="Towel"),
            RadiatorDef(t50=382, area=0.72, pipe="22mm", rad_type="Towel"),
        ],
        fabric=[
            FabricElement("Ceiling (to loft)", 18.0, 0.44, "external"),
            FabricElement("External Wall (solid brick)", 10.92, 2.11, "external"),
            FabricElement("Floor (from kitchen below)", 18.0, 1.41, "internal", "kitchen"),
            FabricElement("Internal Wall", 13.44, 1.76, "internal", "_mixed"),
            FabricElement("Windows (triple, flat wall)", 2.52, 1.0, "external"),
        ],
    )

    rooms["office"] = RoomDef(
        name="office", floor="1st", floor_area=5.28, ceiling_height=2.4,
        sensor="",  # No sensor!
        door_state="closed",
        notes="Above hall. Shares 15mm branch with Jack&Carol. No sensor.",
        radiators=[
            RadiatorDef(t50=1345, area=0.60, pipe="15mm_branch2", rad_type="DP SF"),
        ],
        fabric=[
            FabricElement("Ceiling (to loft)", 5.28, 0.44, "external"),
            FabricElement("External Wall (solid brick)", 8.94, 2.11, "external"),
            FabricElement("Floor (from hall below)", 5.28, 1.41, "internal", "hall"),
            FabricElement("Internal Wall", 11.04, 1.76, "internal", "_mixed"),
            FabricElement("Windows (new double)", 2.1, 1.2, "external"),
        ],
    )

    # ── LOFT (2010 standard) ──────────────────────────────────────

    rooms["elvina"] = RoomDef(
        name="elvina", floor="Loft", floor_area=27.5, ceiling_height=2.2,
        sensor="zigbee2mqtt/elvina_temp_humid",
        door_state="closed",
        notes="2010 insulated. Sloping roof over 50% of area. TRICKLE VENTS OPEN — this "
              "explains faster-than-expected cooling (1.2°C overnight vs Aldora 0.7°C). "
              "Insulation likely fine — the high heat loss is ventilation through trickle vents, "
              "not poor fabric. Warm air rises to sloping ceiling and exits via vents. "
              "Humidity confirms: 1 person only +6% RH in 61m³ = significant air exchange. "
              "Sensor at 1.7m (near ceiling — reads warm side of stratification).",
        radiators=[
            RadiatorDef(t50=909, area=0.30, pipe="22mm", rad_type="DP DF"),
        ],
        fabric=[
            FabricElement("External Wall (insulated)", 53.73, 0.15, "external"),
            FabricElement("Roof (sloping, insulation OK)", 26.64, 0.066, "external"),
            FabricElement("Velux", 0.858, 1.0, "external"),
            FabricElement("Windows", 2.37, 1.6, "external"),
        ],
    )

    rooms["aldora"] = RoomDef(
        name="aldora", floor="Loft", floor_area=14.0, ceiling_height=2.2,
        sensor="zigbee2mqtt/aldora_temp_humid",
        door_state="closed",
        notes="2010 insulated, FLAT roof — well fitted, tight construction. "
              "Overnight cooldown only 0.7°C (with 1 person adding ~80W). Very well sealed — "
              "humidity rises to 61% overnight with 1 person (mould threshold). "
              "Needs trickle vent or door opened periodically. Sensor at 1.7m.",
        radiators=[
            RadiatorDef(t50=376, area=0.45, pipe="22mm", rad_type="Towel"),
        ],
        fabric=[
            FabricElement("External Wall (insulated)", 30.84, 0.15, "external"),
            FabricElement("Roof (flat, well fitted insulation)", 13.57, 0.066, "external"),
            FabricElement("Velux", 0.429, 1.0, "external"),
            FabricElement("Windows", 2.16, 1.5, "external"),
        ],
    )

    rooms["shower"] = RoomDef(
        name="shower", floor="Loft", floor_area=4.14, ceiling_height=2.2,
        sensor="zigbee2mqtt/shower_temp_humid",
        door_state="open",
        notes="2010 insulated. Tiny losses. Door open to stairwell.",
        radiators=[
            RadiatorDef(t50=752, area=0.45, pipe="22mm", rad_type="Towel"),
        ],
        fabric=[
            FabricElement("External Wall (insulated)", 19.62, 0.15, "external"),
            FabricElement("Roof (insulated)", 3.71, 0.066, "external"),
            FabricElement("Velux", 0.429, 1.0, "external"),
            FabricElement("Windows", 0.84, 1.5, "external"),
        ],
    )

    return rooms


# ---------------------------------------------------------------------------
# Pipe topology — which radiators share branches
# ---------------------------------------------------------------------------

PIPE_BRANCHES = {
    "22mm": {
        "description": "22mm primary with short 15mm tails",
        "radiators": [
            "leather.0", "leather.1",
            "conservatory.0", "conservatory.1",
            "front.1",  # vertical K2
            "bathroom.0", "bathroom.1",
            "sterling.0",
            "elvina.0", "aldora.0", "shower.0",
        ],
    },
    "15mm_branch1": {
        "description": "4m of 15mm, shared between front-horizontal and hall",
        "radiators": ["front.0", "hall.0"],
    },
    "15mm_branch2": {
        "description": "4m of 15mm, shared between jackcarol and office",
        "radiators": ["jackcarol.0", "office.0"],
    },
}


# ---------------------------------------------------------------------------
# Physics calculations
# ---------------------------------------------------------------------------

def radiator_output(t50: float, mwt: float, room_temp: float) -> float:
    """Radiator heat output in Watts using T50 correction factor.
    Q = T50 × ((MWT - T_room) / 50)^1.3
    """
    dt = mwt - room_temp
    if dt <= 0:
        return 0.0
    return t50 * (dt / 50.0) ** 1.3


def fabric_loss(elements: list[FabricElement], room_temp: float,
                outside_temp: float, room_temps: dict[str, float]) -> float:
    """Total fabric heat loss for a room in Watts."""
    total = 0.0
    for elem in elements:
        if elem.dt_type == "external":
            dt = room_temp - outside_temp
        elif elem.dt_type == "ground":
            dt = room_temp - GROUND_TEMP_OFFSET
        elif elem.dt_type == "internal":
            if elem.adjacent_room.startswith("_"):
                # Generic internal — use average of all rooms as approximation
                avg_t = sum(room_temps.values()) / len(room_temps) if room_temps else room_temp
                dt = room_temp - avg_t
            elif elem.adjacent_room in room_temps:
                dt = room_temp - room_temps[elem.adjacent_room]
            else:
                dt = 0.0  # Unknown adjacent room temp
        else:
            dt = 0.0
        total += elem.u_value * elem.area * dt
    return total


def ventilation_loss(ach: float, volume: float, room_temp: float,
                     outside_temp: float, heat_recovery: float = 0.0) -> float:
    """Ventilation heat loss in Watts, accounting for heat recovery.
    Q = 0.33 × ACH × Volume × ΔT × (1 - heat_recovery)
    """
    return 0.33 * ach * volume * (room_temp - outside_temp) * (1.0 - heat_recovery)


def get_ventilation(room_name: str) -> VentilationDef:
    """Get ventilation parameters for a room."""
    return VENTILATION.get(room_name, VentilationDef(
        ach=0.3, heat_recovery=0.0, source="default", notes="No specific data"
    ))


def room_volume(room: RoomDef) -> float:
    """Room volume in m³."""
    return room.floor_area * room.ceiling_height


# ---------------------------------------------------------------------------
# Data fetching from InfluxDB
# ---------------------------------------------------------------------------

DATA_DIR = Path(__file__).parent / "data"

INFLUX_URL = "http://pi5data:8086"
INFLUX_TOKEN = "jPTPrwcprKfDzt8IFr7gkn6shpBy15j8hFeyjLaBIaJ0IwcgQeXJ4LtrvVBJ5aIPYuzEfeDw5e-cmtAuvZ-Xmw=="
INFLUX_ORG = "home"
INFLUX_BUCKET = "energy"


def fetch_data(hours: int = 24):
    """Fetch room temps, outside temp, and HP state from InfluxDB.
    Saves to CSV files in data/ directory.
    """
    from influxdb_client import InfluxDBClient

    DATA_DIR.mkdir(exist_ok=True)
    client = InfluxDBClient(url=INFLUX_URL, token=INFLUX_TOKEN, org=INFLUX_ORG)
    query_api = client.query_api()

    # Room temperatures (all zigbee + emonth2)
    query = f'''
    from(bucket: "{INFLUX_BUCKET}")
      |> range(start: -{hours}h)
      |> filter(fn: (r) => r._field == "temperature" or r._field == "humidity")
      |> filter(fn: (r) =>
           r.topic =~ /temp_humid/ or
           r.topic == "emon/emonth2_23/temperature" or
           r.topic == "emon/emonth2_23/humidity"
         )
      |> aggregateWindow(every: 5m, fn: mean, createEmpty: false)
      |> keep(columns: ["_time", "topic", "_field", "_value"])
    '''
    tables = query_api.query(query)
    with open(DATA_DIR / "room_temps.csv", "w", newline="") as f:
        writer = csv.writer(f)
        writer.writerow(["time", "topic", "field", "value"])
        for table in tables:
            for record in table.records:
                writer.writerow([
                    record.get_time().isoformat(),
                    record.values.get("topic", ""),
                    record.values.get("_field", ""),
                    record.get_value(),
                ])
    print(f"Wrote room_temps.csv")

    # Outside temperature (eBUS)
    query = f'''
    from(bucket: "{INFLUX_BUCKET}")
      |> range(start: -{hours}h)
      |> filter(fn: (r) => r.topic == "ebusd/poll/OutsideTemp")
      |> aggregateWindow(every: 5m, fn: mean, createEmpty: false)
      |> keep(columns: ["_time", "_value"])
    '''
    tables = query_api.query(query)
    with open(DATA_DIR / "outside_temp.csv", "w", newline="") as f:
        writer = csv.writer(f)
        writer.writerow(["time", "value"])
        for table in tables:
            for record in table.records:
                writer.writerow([record.get_time().isoformat(), record.get_value()])
    print(f"Wrote outside_temp.csv")

    # HP state (heat meter power, flow, return, flow rate, electric)
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
    tables = query_api.query(query)
    with open(DATA_DIR / "hp_state.csv", "w", newline="") as f:
        writer = csv.writer(f)
        writer.writerow(["time", "topic", "value"])
        for table in tables:
            for record in table.records:
                writer.writerow([
                    record.get_time().isoformat(),
                    record.values.get("topic", ""),
                    record.get_value(),
                ])
    print(f"Wrote hp_state.csv")

    # eBUS status code (heating/DHW/idle)
    query = f'''
    from(bucket: "{INFLUX_BUCKET}")
      |> range(start: -{hours}h)
      |> filter(fn: (r) => r.topic == "ebusd/poll/StatuscodeNum")
      |> aggregateWindow(every: 5m, fn: mean, createEmpty: false)
      |> keep(columns: ["_time", "_value"])
    '''
    tables = query_api.query(query)
    with open(DATA_DIR / "hp_status.csv", "w", newline="") as f:
        writer = csv.writer(f)
        writer.writerow(["time", "value"])
        for table in tables:
            for record in table.records:
                writer.writerow([record.get_time().isoformat(), record.get_value()])
    print(f"Wrote hp_status.csv")

    client.close()
    print(f"\nFetched {hours}h of data to {DATA_DIR}/")


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
    # Handle various formats
    s = s.replace("Z", "+00:00")
    if "+" not in s and s.endswith(":00"):
        s += "+00:00"
    return datetime.fromisoformat(s)


# ---------------------------------------------------------------------------
# Steady-state analysis
# ---------------------------------------------------------------------------

def analyse():
    """Analyse the latest data: energy balance per room at steady state."""
    import numpy as np

    house = build_house()

    # Load data
    room_data = load_csv("room_temps.csv")
    outside_data = load_csv("outside_temp.csv")
    hp_data = load_csv("hp_state.csv")
    status_data = load_csv("hp_status.csv")

    # Get latest room temperatures
    room_temps = {}
    sensor_to_room = {}
    for room_name, room in house.items():
        if room.sensor:
            sensor_to_room[room.sensor] = room_name

    for row in room_data:
        topic = row["topic"]
        field = row["field"]
        if field == "temperature" and topic in sensor_to_room:
            room_temps[sensor_to_room[topic]] = float(row["value"])

    # Estimate office (no sensor) — average of hall and jackcarol
    if "hall" in room_temps and "jackcarol" in room_temps:
        room_temps["office"] = (room_temps["hall"] + room_temps["jackcarol"]) / 2

    # Latest outside temp
    outside_temp = float(outside_data[-1]["value"]) if outside_data else 10.0

    # Latest HP state
    hp_heat = 0.0
    hp_flow_t = 30.0
    hp_return_t = 27.0
    hp_elec = 0.0
    for row in hp_data:
        topic = row["topic"]
        val = float(row["value"])
        if "Power" in topic and "electric" not in topic:
            hp_heat = val
        elif "FlowT" in topic:
            hp_flow_t = val
        elif "ReturnT" in topic:
            hp_return_t = val
        elif "electric" in topic:
            hp_elec = val

    mwt = (hp_flow_t + hp_return_t) / 2

    print("=" * 90)
    print("STEADY-STATE ENERGY BALANCE")
    print(f"Outside: {outside_temp:.1f}°C | HP: {hp_heat:.0f}W heat, {hp_elec:.0f}W elec")
    print(f"Flow: {hp_flow_t:.1f}°C, Return: {hp_return_t:.1f}°C, MWT: {mwt:.1f}°C")
    print("=" * 90)

    print(f"\n{'Room':<14} {'T°C':>5} {'Fabric':>7} {'Vent':>6} {'Total':>7} {'Rad_max':>8} {'Residual':>9} {'Notes'}")
    print(f"{'':14} {'':>5} {'W':>7} {'W':>6} {'W':>7} {'W':>8} {'W':>9}")
    print("─" * 100)

    total_fabric = 0
    total_vent = 0
    total_rad_max = 0

    for room_name in sorted(house.keys()):
        room = house[room_name]
        t = room_temps.get(room_name)
        if t is None:
            continue

        # Fabric loss
        f_loss = fabric_loss(room.fabric, t, outside_temp, room_temps)
        total_fabric += f_loss

        # Ventilation loss (with heat recovery if applicable)
        vent = get_ventilation(room_name)
        vol = room_volume(room)
        v_loss = ventilation_loss(vent.ach, vol, t, outside_temp, vent.heat_recovery)
        total_vent += v_loss

        total_loss = f_loss + v_loss

        # Maximum radiator output (assumes adequate flow)
        rad_out = sum(
            radiator_output(r.t50, mwt, t)
            for r in room.radiators if r.active
        )
        total_rad_max += rad_out

        # Residual = rad_output - total_loss
        # Positive = room exports heat (to neighbours)
        # Negative = room imports heat (from neighbours) or radiator is under-performing
        residual = rad_out - total_loss

        notes = ""
        vent_tag = ""
        if vent.source == "measured":
            vent_tag = " [MVHR]"
        elif vent.source == "calibrate_from_data":
            vent_tag = " [FIT]"

        if not room.radiators:
            notes = "NO RAD"
        elif not any(r.active for r in room.radiators):
            notes = "RAD OFF"
        elif residual < -50:
            notes = f"← needs {-residual:.0f}W"
        elif residual > 50:
            notes = f"→ exports {residual:.0f}W"

        print(f"{room_name:<14} {t:>5.1f} {f_loss:>7.0f} {v_loss:>6.0f} {total_loss:>7.0f} {rad_out:>8.0f} {residual:>+9.0f}  {notes}{vent_tag}")

    print(f"{'─' * 100}")
    print(f"{'Total':<14} {'':>5} {total_fabric:>7.0f} {total_vent:>6.0f} {total_fabric+total_vent:>7.0f} {total_rad_max:>8.0f} {total_rad_max - total_fabric - total_vent:>+9.0f}")
    print(f"{'HP meter':<14} {'':>5} {'':>7} {'':>6} {'':>7} {hp_heat:>8.0f}")

    if total_rad_max > 0:
        ratio = hp_heat / total_rad_max
        print(f"\nHP output / max rad output = {ratio:.2f}")
        print("(< 1.0 means rads are flow-limited and can't deliver their T50 potential)")


# ---------------------------------------------------------------------------
# Parameter fitting (placeholder — needs overnight data)
# ---------------------------------------------------------------------------

def fit():
    """Fit thermal mass and ventilation rates from overnight cooldown data."""
    import numpy as np
    from scipy.optimize import minimize

    house = build_house()

    # Load data
    room_data = load_csv("room_temps.csv")
    outside_data = load_csv("outside_temp.csv")
    hp_data = load_csv("hp_state.csv")
    status_data = load_csv("hp_status.csv")

    # Build time series per room (5-min resolution)
    sensor_to_room = {}
    for room_name, room in house.items():
        if room.sensor:
            sensor_to_room[room.sensor] = room_name

    # Parse into {room: [(time, temp), ...]}
    room_series = {}
    for row in room_data:
        if row["field"] != "temperature":
            continue
        topic = row["topic"]
        if topic not in sensor_to_room:
            continue
        room_name = sensor_to_room[topic]
        t = parse_time(row["time"])
        temp = float(row["value"])
        room_series.setdefault(room_name, []).append((t, temp))

    # Sort by time
    for room_name in room_series:
        room_series[room_name].sort(key=lambda x: x[0])

    # Parse outside temp series
    outside_series = [(parse_time(r["time"]), float(r["value"])) for r in outside_data]
    outside_series.sort(key=lambda x: x[0])

    # Parse HP heat output series
    hp_heat_series = []
    for row in hp_data:
        if "Power" in row["topic"] and "electric" not in row["topic"]:
            hp_heat_series.append((parse_time(row["time"]), float(row["value"])))
    hp_heat_series.sort(key=lambda x: x[0])

    # Find periods where space heating is off.
    # Use eBUS status code:
    #   101, 103 = standby (HP off, setback)
    #   134 = DHW (HP running but radiators have standing water — no space heating)
    #   104 = heating compressor active
    #   105, 107, 114 = transitions
    # Both standby and DHW are "free-cooling" periods for room temperature fitting.
    status_series = [(parse_time(r["time"]), float(r["value"])) for r in status_data]
    status_series.sort(key=lambda x: x[0])

    SPACE_HEATING_OFF_CODES = {101, 103, 134}  # standby + DHW

    cooldown_periods = []
    in_cooldown = False
    start = None
    for t, code in status_series:
        if int(code) in SPACE_HEATING_OFF_CODES:
            if not in_cooldown:
                start = t
                in_cooldown = True
        else:
            if in_cooldown and start:
                duration = (t - start).total_seconds() / 3600
                if duration > 0.25:  # at least 15 min
                    cooldown_periods.append((start, t))
            in_cooldown = False
    # Handle case where we're still in cooldown at end of data
    if in_cooldown and start:
        end = status_series[-1][0]
        duration = (end - start).total_seconds() / 3600
        if duration > 0.25:
            cooldown_periods.append((start, end))

    if not cooldown_periods:
        print("No cooldown periods found (HP heat < 100W for > 30min)")
        print("Need more data — run fetch again after an overnight setback cycle.")
        return

    print(f"Found {len(cooldown_periods)} cooldown periods:")
    for start, end in cooldown_periods:
        hours = (end - start).total_seconds() / 3600
        print(f"  {start.strftime('%H:%M')} → {end.strftime('%H:%M')} ({hours:.1f}h)")

    # For each cooldown period, calculate dT/dt per room
    print(f"\n{'Room':<14} {'Start°C':>8} {'End°C':>7} {'Drop':>6} {'Rate':>8} {'Period'}")
    print("─" * 70)

    for start, end in cooldown_periods[:3]:  # First 3
        for room_name, series in sorted(room_series.items()):
            # Find temps at start and end of cooldown
            temps_in_period = [(t, temp) for t, temp in series if start <= t <= end]
            if len(temps_in_period) < 2:
                continue
            t_start = temps_in_period[0][1]
            t_end = temps_in_period[-1][1]
            hours = (temps_in_period[-1][0] - temps_in_period[0][0]).total_seconds() / 3600
            if hours < 0.25:
                continue
            drop = t_start - t_end
            rate = drop / hours
            period_str = f"{start.strftime('%H:%M')}→{end.strftime('%H:%M')}"
            print(f"{room_name:<14} {t_start:>8.2f} {t_end:>7.2f} {drop:>6.2f} {rate:>7.3f}°/h  {period_str}")

    print("\n[Full parameter fitting will run once sufficient overnight data is available]")


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

def main():
    if len(sys.argv) < 2:
        print("Usage: python model/house.py <command>")
        print("Commands: fetch [hours], analyse, fit, rooms")
        sys.exit(1)

    cmd = sys.argv[1]

    if cmd == "fetch":
        hours = int(sys.argv[2]) if len(sys.argv) > 2 else 24
        fetch_data(hours)
    elif cmd == "analyse":
        analyse()
    elif cmd == "fit":
        fit()
    elif cmd == "rooms":
        house = build_house()
        print(f"{'Room':<14} {'Flr':>4} {'Area':>6} {'Vol':>5} {'T50':>6} {'extUA':>7} {'ACH':>5} {'eff':>5} {'Pipe':>15} {'Snsr':>4} {'Vent source'}")
        print("─" * 110)
        for name, room in sorted(house.items()):
            vol = room_volume(room)
            total_t50 = sum(r.t50 for r in room.radiators if r.active)
            ext_ua = sum(e.u_value * e.area for e in room.fabric if e.dt_type == "external")
            vent = get_ventilation(name)
            print(f"{name:<14} {room.floor:>4} {room.floor_area:>5.1f}m² {vol:>4.0f}m³ "
                  f"{total_t50:>5.0f}W {ext_ua:>6.1f}W/K "
                  f"{vent.ach:>5.2f} {vent.effective_ach:>5.2f} "
                  f"{room.radiators[0].pipe if room.radiators else 'none':>15} "
                  f"{'✓' if room.sensor else '✗':>4} "
                  f"{vent.source}")
    else:
        print(f"Unknown command: {cmd}")
        sys.exit(1)


if __name__ == "__main__":
    main()
