# House Layout & Building Physics

Detailed building-fabric and room-to-room notes for the thermal model. The condensed current house facts used elsewhere in the repo live in [`../lat.md/domain.md`](../lat.md/domain.md).

## What this file is for

Use this file as the human building-survey companion to the thermal model. It should keep structural and physical context, not duplicate the current concise house truth from `lat.md`.

## Current concise truth lives in

- [`../lat.md/domain.md#House`](../lat.md/domain.md#house)
- [`../lat.md/domain.md#House#Key Thermal Relationships`](../lat.md/domain.md#house#key-thermal-relationships)

## Building summary

- 1930s solid-brick house with 2010 loft extension
- 13 rooms, all sensored
- 15 radiators, with kitchen and landings lacking direct emitters
- mixed heavy brick / lighter loft construction means thermal mass varies strongly by room

## Vertical stacking

| Loft (2010 insulated) | 1st Floor | Ground Floor |
|---|---|---|
| Elvina | Sterling | Leather |
| Aldora | Jack & Carol | Front |
| Shower | Bathroom | Kitchen |
| stairwell | Office | Hall |
|  | stairwell | stairwell |

## Connectivity summary

```text
LOFT
  Elvina ─ wall ─ Aldora ─ wall ─ Shower
      \                       /
       └──── top stairwell ───┘

1ST FLOOR
  Sterling ─ wall ─ Jack & Carol ─ wall ─ Office
      \                             |
       └──────── landing ────────────┘
                     |
                 Bathroom

GROUND FLOOR
  Leather ─ wall ─ Front
      |             |
      |             └─ part-closed connection to Hall
      |                         |
      └─ SG door ─ Conservatory ┴─ open Kitchen
```

The main interpretation point is that the hall, landing, and top landing behave as one vertical coupling path, while Leather exports heat sideways into several neighbours before it noticeably warms itself.

## Construction notes

1930s ground and first-floor internal walls are solid single brick. Loft rooms and loft landing are insulated timber-stud construction from the 2010 extension.

| Element type | Typical rooms / effect |
|---|---|
| Concrete slab on ground | kitchen and conservatory; steady sink to ground |
| Suspended timber floor | hall, front, leather; more coupled to underfloor void / cellar context |
| Older inter-floor timber | Sterling↔Leather, Jack & Carol↔Front, Bathroom↔Kitchen |
| Insulated extension floor | Office↔Hall and landing↔Hall |
| Insulated loft envelope | Elvina, Aldora, Shower |

Thermal mass remains the dominant practical lesson: the heavy brick ground-floor rooms cool much slower than the loft rooms even when loft insulation is much better.

## Door and ventilation states

These states matter because they often dominate room behaviour more than emitter size.

- **Usually closed:** Elvina, Aldora, Sterling, Leather
- **Open by day / closed by night:** Jack & Carol
- **Part-closed:** Front
- **Usually open:** Bathroom, Office, Kitchen↔Hall, Kitchen↔Conservatory, Shower↔stairwell, stairwell chain

Ventilation features worth remembering:

- Bathroom MVHR creates a slight negative-pressure draw through the stairwell chain
- Elvina trickle vents materially increase heat loss but help moisture control
- Aldora is sealed enough to become the mould-risk bedroom without deliberate ventilation

## Enduring physical themes

A few relationships continue to dominate interpretation:

- **Leather** behaves like the main ground-floor heat hub and exports heat to neighbours before warming itself much
- **Kitchen** has no radiator and relies on adjacent rooms plus pipe losses
- **Hall / landing / top landing** behave as a vertical chimney-like coupling path
- **Sterling** is sustained largely by floor heat from Leather with its radiator off
- **Conservatory** is the fastest-cooling room and behaves differently enough that it is excluded from thermal scoring
- **Elvina** is strongly affected by trickle-vent ventilation
- **Aldora** is well sealed enough to create humidity / mould-risk concerns without extra ventilation

## Glazing and envelope clues

| Element | Rooms | Practical lesson |
|---|---|---|
| Triple glazing, flat wall | Sterling, Bathroom | best-performing window type in the house |
| Bay windows | Front, Jack & Carol | noticeably leakier because of joints and exposure |
| Conservatory glazed roof | Conservatory | dominates overnight losses far more than wall glazing |
| Single-glazed timber door | Leather↔Conservatory | important coupling path when not isolated |

## Radiator / sensor context

Useful context retained here:

- no TRVs are fitted; valves are effectively manual and mostly wide open
- Sterling radiator is intentionally off
- kitchen and landings have no direct emitters
- the room sensor estate is complete at 13/13 rooms, with Leather covered by emonth2 and the others mainly via Zigbee
- outside temperature truth for control and thermal work comes primarily from eBUS `OutsideTemp`

## Related documents

- [Room thermal model](room-thermal-model.md)
- [`../lat.md/domain.md`](../lat.md/domain.md)
- [`../lat.md/thermal-model.md`](../lat.md/thermal-model.md)
