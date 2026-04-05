# House Layout & Building Physics

Detailed building-fabric and room-to-room notes for the thermal model. The condensed current house facts used elsewhere in the repo now live in `lat.md/domain.md`; this document keeps the full survey detail.

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
  │carpet/timber/plaster floor         │door open
  │(above leather)                     │
  │              Bathroom              │
  │              │door open (normally) │
  └── STAIRWELL (landing) ─────────────┘

GROUND FLOOR
  Leather ──1930s SG door (single-glazed panels in timber frame, U≈4.4)── Conservatory
  │door closed  │      │open
  ├──wall── Front    Kitchen (no heater)
  │         │part.closed  │open
  └──wall───┴── HALL ─────┘
                │front door (SE, weather face)
                ↕ stairwell to all floors
```

### Construction

1930s solid brick house with 2010 loft extension. All internal walls on ground and first floors are **solid single brick**. Loft rooms and landing are **timber stud with insulation**.

| Floor type | Rooms |
|---|---|
| **Concrete slab on ground** | Kitchen (tile over concrete), Conservatory (yr 2000 extension, clay substrate) |
| **Suspended timber** | Hall (fitted parquet), Front, Leather (spiral cellar below) |
| **Timber between floors** (U≈1.7) | Sterling↔Leather, Jack&Carol↔Front, Bathroom↔Kitchen |
| **100mm insulated floor** (U≈0.25) | Office↔Hall, Landing↔Hall (2010 extension above original house) |
| **Insulated between loft and 1st** | Elvina, Aldora, Shower (2010 extension) |

Thermal mass: total house **48,090 kJ/K**. Ground floor brick rooms (4,000-6,300 kJ/K) cool much slower than loft timber rooms (880-3,800 kJ/K). This dominates cooling behaviour more than fabric U-values.

### SE Face (front of house)

Most exposed — takes prevailing wind and morning/midday sun. Hall, Front, Jack & Carol, Office, Elvina. Front and Jack & Carol have large bay windows (more frame joints, more infiltration paths than flat-wall windows).

### Door States

- **Always closed**: Elvina, Aldora, Sterling, Leather
- **Open day / closed night**: Jack & Carol
- **Partially closed**: Front
- **Always open**: Bathroom (except during/after showers), Office, Kitchen↔Hall, Kitchen↔Conservatory, Shower↔stairwell, Hall↔stairwell (all floors)

## Key Thermal Relationships

- **Leather** (door closed except morning for dog, 2×DP DF, no external walls) = heat hub. Heats Sterling through floor (carpet/timber/plaster), Kitchen through shared wall, Front through shared wall, Conservatory through SG door. Warmup data (25 Mar): only +0.3°C in 2.5h despite biggest 22mm rads — heat exports to 5 neighbouring rooms before it can warm itself. Doors open in morning for dog, then closed during day when occupied.
- **Kitchen** has NO radiator. Heated by adjacent rooms + **bare CH pipes in floor void** (2m of 35mm flow + return, bare copper, between kitchen ceiling and bathroom floor — ~25W each side at MWT=31) (Leather wall, Conservatory connection, Bathroom above). Cools at 0.29°C/h during free-cooling — identical to hall (thermally coupled through open doorway).
- **Hall/Landing/Top Landing** = one continuous stairwell column, 3 floors. Only 1 radiator (Hall, ground floor). No radiators on landing or top landing. Hall drops even while HP is heating — flow-starved 15mm branch confirmed.
- **Sterling** has rad turned OFF, door closed. Gets ~19°C from Leather's floor heat alone (Night 1: 0.06°/h, slowest in house). Occupant prefers cold, opens windows when home — leather's heat goes straight outside. **Floor insulation** (mineral wool between joists) is the best single-room intervention after EWI: leather keeps its heat, Sterling gets the cold room he wants, HP saves energy.
- **Conservatory** = dining room, cannot be closed off. 2×K3 rads (largest in house) but cools fastest overnight (-1.9°C). Glazed roof (U=2.4) dominates after dark.
- **Elvina** — sloping roof over 50% of area. Cools faster than jackcarol (solid brick) despite 2010 insulation spec. Cause: **trickle vents open**, not poor insulation. Humidity confirms: 1 person only +6% RH overnight (vs Aldora +10%). The trickle vents provide necessary moisture extraction but cost significant heat.
- **Aldora** — flat roof, very well sealed. Only 0.7°C overnight drop. But too sealed: humidity reaches 58.8% RH (surface ~71% = mould warning) overnight with 1 occupant. **Needs trickle vent** (Part F requirement for bedroom) + **radiator upgrade** from 376W towel to 909W DP DF to compensate for added ventilation losses.
- **Jack & Carol** — door open daytime donates heat to landing/stairwell. Observed 20.8→19.2°C drop over 20h of continuous heating (24 Mar 2026). Not flow-starvation — air exchange through open door is the dominant daytime loss. Door closed overnight: only 0.08°/h cooling rate.
- **Bathroom** — door open 24h except during/after showers. MVHR creates negative pressure pulling air from stairwell. This drives whole-house airflow: outside → front door → hall → stairwell → bathroom → MVHR → outside. Closing bathroom door reduces stairwell draft by ~51W.
- **Office** — door normally open. Well insulated ceiling + 50mm insulated floor (plaster + 50mm wood + floating floor). Minimal fabric losses but air exchange through open door connects it to landing/stairwell.
- In cold weather, hall, kitchen, and conservatory are the cold rooms. HP maxes out at ~2°C outside (95% runtime, Jan 2025 data).

## Glazing Comparison (from sensor data)

| Type | Rooms | Performance |
|---|---|---|
| Triple, flat wall, single sealed unit | Sterling, Bathroom | Best. Sterling holds 19.2°C with rad OFF. |
| Bay windows (more joints, SE face) | Front, Jack & Carol | Very leaky. J&C moisture balance: ACH 0.80-1.80 through closed bay window. Only occupied room where humidity *drops* overnight with 2 people. Draught-proofing would save ~60W. |
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

**Added 24 Mar 2026**: Office (`office_temp_humid`), Landing (`landing_temp_humid`) → 13/13 room coverage complete. Landing has no radiator — key heat sink node where stairwell connects ground and loft floors.

Zigbee routers (ZBMINI switches) at hall, landing, kitchen, and top_landing provide mesh coverage for the battery sensors. The top_landing ZBMINI was added Mar 2026 to extend range to loft sensors.

### Outside Temperature

- **Primary**: `ebusd/poll/OutsideTemp` — Arotherm OAT sensor, real-time every 30s
- **Control**: emoncms feed 503093 (Met Office hourly) — cross-check and historical analysis
- VRC 700 controller (in conservatory) displays the Arotherm OAT, not a separate room sensor
