# House Layout & Building Physics

Explanation of the building fabric, room connectivity, heating distribution, ventilation, and sensor coverage at 6 Rhodes Avenue, London N22.

## Room Connectivity

### Vertical Stacking

| Loft (2010, U=0.15) | 1st Floor | Ground Floor |
|---|---|---|
| Elvina | Sterling | Leather |
| Aldora | Jack & Carol | Front |
| Shower | Bathroom | Kitchen |
| (stairwell) | Office | Hall |
| | (stairwell) | (stairwell) |

### Connectivity Map

```
LOFT (2010 insulated, U=0.15 walls, U=0.066 roof)
  Elvina ──wall── Aldora ──wall── Shower
  │small wall                     │door open
  └── STAIRWELL (top landing) ────┘

1ST FLOOR
  Sterling ──wall── Jack&Carol ──wall── Office
  │carpet/timber/plaster floor         │door closed
  │(above leather)                     │
  │              Bathroom              │
  │              │door sometimes closed│
  └── STAIRWELL (landing) ─────────────┘

GROUND FLOOR
  Leather ──SG door── Conservatory
  │door closed  │      │open
  ├──wall── Front    Kitchen (no heater)
  │         │part.closed  │open
  └──wall───┴── HALL ─────┘
                │front door (SE, weather face)
                ↕ stairwell to all floors
```

### SE Face (front of house)

Most exposed — takes prevailing wind and morning/midday sun. Hall, Front, Jack & Carol, Office, Elvina. Front and Jack & Carol have large bay windows (more frame joints, more infiltration paths than flat-wall windows).

### Door States

- **Always closed**: Elvina, Aldora, Sterling, Leather
- **Open day / closed night**: Jack & Carol
- **Partially closed**: Front
- **Sometimes closed**: Bathroom
- **Always open**: Kitchen↔Hall, Kitchen↔Conservatory, Shower↔stairwell, Hall↔stairwell (all floors)

## Key Thermal Relationships

- **Leather** (door closed, 2×DP DF, no external walls) = heat hub. Heats Sterling through floor (carpet/timber/plaster), Kitchen through shared wall, Front through shared wall, Conservatory through single glazed door.
- **Kitchen** has NO radiator. Heated entirely by adjacent rooms (Leather wall, Conservatory connection, Bathroom above). Cools at 0.29°C/h during free-cooling — identical to hall (thermally coupled through open doorway).
- **Hall/Landing/Top Landing** = one continuous stairwell column, 3 floors. Only 1 radiator (Hall, ground floor). No radiators on landing or top landing. Hall drops even while HP is heating — flow-starved 15mm branch confirmed.
- **Sterling** has rad turned OFF, door closed. Gets 19.2°C from Leather's floor heat alone. Dropped only 0.1°C overnight — thermally decoupled from heating system.
- **Conservatory** = dining room, cannot be closed off. 2×K3 rads (largest in house) but cools fastest overnight (-1.9°C). Glazed roof (U=2.4) dominates after dark.
- **Elvina** — sloping roof over 50% of area. Cools faster than jackcarol (solid brick) despite 2010 insulation spec. Cause: **trickle vents open**, not poor insulation. Humidity confirms: 1 person only +6% RH overnight (vs Aldora +10%). The trickle vents provide necessary moisture extraction but cost significant heat.
- **Aldora** — flat roof, very well sealed. Only 0.7°C overnight drop. But too sealed: humidity reaches 61% overnight with 1 occupant (mould threshold). Needs trickle vent.
- **Bathroom** — MVHR creates negative pressure pulling air from stairwell. This drives whole-house airflow: outside → front door → hall → stairwell → bathroom → MVHR → outside. Closing bathroom door reduces stairwell draft by ~51W.
- In cold weather, hall, kitchen, and conservatory are the cold rooms. HP maxes out at ~2°C outside (95% runtime, Jan 2025 data).

## Glazing Comparison (from sensor data)

| Type | Rooms | Performance |
|---|---|---|
| Triple, flat wall, single sealed unit | Sterling, Bathroom | Best. Sterling holds 19.2°C with rad OFF. |
| Bay windows (more joints, SE face) | Front, Jack & Carol | Leakier. J&C humidity drops overnight despite 2 occupants — infiltration. |
| Conservatory DG (yr 2000) | Conservatory | Walls OK (U=0.5), roof terrible (U=2.4). |

## Radiator Inventory

15 Stelrad radiators. No TRVs fitted anywhere — all valves wide open (except Sterling manually off).

| Room | Rad# | W×H mm | Type | T50 W | Notes |
|---|---|---|---|---|---|
| Aldora | 1 | 500×900 | Towel | 376 | Stelrad Classic Mini |
| Bathroom | 1 | 1800×600 | Towel | 614 | Stelrad Slimline Chrome |
| Bathroom | 2 | 1200×600 | Towel | 382 | Stelrad Slimline Chrome |
| Jack & Carol | 1 | 1200×600 | DP DF | 1,950 | Stelrad Compact K2 |
| Conservatory | 1 | 2000×300 | TP TF | 2,833 | Stelrad Vita Compact K3 |
| Conservatory | 2 | 1200×600 | TP TF | 2,867 | Stelrad Compact K3 |
| Elvina | 1 | 500×600 | DP DF | 909 | Stelrad Slimline K2 |
| Front | 1 | 1400×600 | DP DF | 2,425 | Stelrad Slimline K2 |
| Front | 2 | 600×1800 | DP DF | 2,376 | Stelrad Compact Vertical K2 |
| Hall | 1 | 600×1800 | DP DF | 2,376 | Stelrad Compact Vertical K2 |
| Leather | 1 | 600×1800 | DP DF | 2,376 | Stelrad Compact Vertical K2 |
| Leather | 2 | 600×1800 | DP DF | 2,376 | Stelrad Compact Vertical K2 |
| Office | 1 | 1000×600 | DP SF | 1,345 | Stelrad Slimline P+ |
| Shower | 1 | 500×900 | Towel | 752 | Stelrad Classic Mini |
| Sterling | 1 | 1170×620 | SP SF | 1,176 | Stelrad Slimline P+ (**OFF**) |

**No radiator**: Kitchen, Landing (any level). Total T50 = 25,133W.

## Pipe Topology

HP pump is software-clamped at 14.3 L/min (860 L/h) during heating. All radiators share this fixed total flow.

**22mm primary with short 15mm tails** (most radiators — good flow):
Leather ×2, Conservatory ×2 (6m of 22mm + short tails), Front vertical, Bathroom ×2, Sterling, Elvina, Aldora, Shower

**15mm Branch 1** (4m of 15mm, two rads competing for flow):
- Front horizontal (DP DF 2425W) + Hall (DP DF 2376W)

**15mm Branch 2** (4m of 15mm, same arrangement):
- Jack & Carol (DP DF 1950W) + Office (DP SF 1345W)

15mm pipe has ~4× the hydraulic resistance of 22mm per metre. The 15mm branch radiators are flow-starved. Hall (coldest room) is on Branch 1 competing with front-horizontal for flow.

## Mechanical Ventilation

**Bathroom**: Vent-Axia Lo-Carbon Tempra LP (403832), 460mm wall version. Runs 24/7.
- Trickle: 9 L/s (32.4 m³/h), boost 15 L/s (pullcord)
- Heat recovery: 78% temperature efficiency
- Power: 3.2W trickle
- ACH: 0.75 (bathroom volume 43.2m³), effective 0.17 after heat recovery
- Creates slight negative pressure → pulls air from rest of house via stairwell
- With door closed: effective extract drops to ~3-4 L/s (door gap limited). Post-shower humidity takes 5h to drop from 83% to 55%. With door open: full 9 L/s, recovery ~2h.

**Kitchen**: Hob hood extract fan. Extract-only (no heat recovery), runs during cooking only.

**Elvina**: Trickle vents open in windows. Provides continuous background ventilation (~0.5 ACH). This is why Elvina cools faster than Aldora despite same insulation spec.

No other rooms have designed ventilation. Aldora, Sterling, and Leather rely entirely on infiltration (near zero).

## Room Sensors

All SONOFF SNZB-02P on firmware v2.2.0. Data → Z2M → MQTT bridge → pi5data Telegraf → InfluxDB.

| Sensor | Room | Floor | Notes |
|---|---|---|---|
| hall_temp_humid | Hall | Gnd | |
| kitchen_temp_humid | Kitchen | Gnd | |
| front_temp_humid | Front | Gnd | |
| conservatory_temp_humid | Conservatory | Gnd | battery 38% (OTA drain) |
| Sterling_temp_humid | Sterling | 1st | |
| jackcarol_temp_humid | Jack & Carol | 1st | |
| bathroom_temp_humid | Bathroom | 1st | |
| shower_temp_humid | Shower | Loft | |
| elvina_temp_humid | Elvina | Loft | sensor at 1.7m height |
| aldora_temp_humid | Aldora | Loft | sensor at 1.7m height |
| emonth2_23 (RFM69) | Leather | Gnd | emoncms feed 503101 |

**Being added**: Office, 1st floor Landing → 13/13 room coverage.

Zigbee routers (ZBMINI switches) at hall, landing, kitchen, and top_landing provide mesh coverage for the battery sensors. The top_landing ZBMINI was added Mar 2026 to extend range to loft sensors.

### Outside Temperature

- **Primary**: `ebusd/poll/OutsideTemp` — Arotherm OAT sensor, real-time every 30s
- **Control**: emoncms feed 503093 (Met Office hourly) — cross-check and historical analysis
- VRC 700 controller (in conservatory) displays the Arotherm OAT, not a separate room sensor
