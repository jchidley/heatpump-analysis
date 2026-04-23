# Room-by-Room Thermal Model

This document is the detailed human companion to the thermal model. The canonical current-state summary of calibrated parameters, solver responsibilities, and regression posture lives in [`../lat.md/thermal-model.md`](../lat.md/thermal-model.md).

## What this file is for

Use this page for methodology, experiment interpretation, and model-building intuition. Do not use it as the source of current calibrated truth.

## Current canonical truth lives in

- [`../lat.md/thermal-model.md`](../lat.md/thermal-model.md)
- [`../lat.md/domain.md#House`](../lat.md/domain.md#house)
- [`../lat.md/architecture.md#Thermal Model Path`](../lat.md/architecture.md#thermal-model-path)

## Model purpose

The thermal model exists to:

- represent room-level heat loss and coupling
- connect geometry, emitter capacity, and measured temperatures
- support calibration / validation / operational analysis
- provide live control support via the solver used by the adaptive controller

## Inputs in plain English

The model combines:

- canonical room geometry from `data/canonical/thermal_geometry.json`
- room temperatures from Zigbee + emonth2 sensors
- outside temperature from eBUS
- HP state and water temperatures from eBUS / shared TSDB data
- radiator and connection context from the house survey

## What we know before fitting

Several parts of the model are anchored by direct survey or inventory work rather than pure optimisation:

- room areas and fabric categories were measured from the house survey spreadsheet
- radiator inventory and T50 outputs were physically catalogued
- the pipework is not symmetric: most emitters sit on 22mm primaries, but hall/front and Jack & Carol/office share 15mm branches
- kitchen has no radiator and Sterling's radiator is intentionally off, making both rooms especially useful calibration anchors

## Main unknowns the model tries to fit

The high-value unknowns are:

1. **thermal mass per room** — why heavy brick rooms coast so differently from lightweight loft rooms
2. **ventilation / infiltration rate** — often the biggest uncertainty in any house model
3. **inter-room air exchange** through open doors and the stairwell chain
4. **effective radiator output** after real hydraulic flow distribution rather than brochure assumptions

## Daily natural experiment

The house provides a repeatable three-phase experiment most days:

1. **evening setback / coast** — rooms start drifting toward lower targets
2. **morning DHW charge** — space-heating flow stops, leaving a clean free-cooling window
3. **space-heating resume** — warmup order reveals which rooms actually receive flow first and most strongly

The DHW phase is especially valuable because radiator input is largely absent apart from brief standing-water decay, so cooldown data becomes much easier to interpret.

## Calibration anchors

### Kitchen

Kitchen has no radiator. That makes it a continuous observation of indirect heating, doorway exchange, fabric loss, and pipe-loss effects rather than a room with hidden emitter input.

### Sterling

Sterling's radiator is off and the door is usually closed. Its temperature mostly reflects floor coupling from Leather plus fabric and ventilation losses, making it the lower-ventilation anchor for the house.

### Together

Kitchen and Sterling effectively bracket the ventilation/exchange range:

- Sterling approximates the closed-room lower bound
- Kitchen approximates a high-coupling internal room
- hall / stairwell behaviour then shows the stronger open-path extreme

## Simplified energy-balance view

For each room, the model conceptually balances:

```text
thermal storage change = radiator input
                       - fabric loss to outside
                       - ventilation loss
                       + transfer from neighbouring rooms
                       + internal gains
```

The important design point is not the exact equation formatting but the coupling: a room can look under-radiated when the real issue is ventilation or heat export to adjacent spaces.

## Persistent qualitative lessons

The detailed experiments repeatedly pointed to the same high-level conclusions:

- thermal mass differences matter more than naïve fabric-only thinking suggests
- open-door / stairwell coupling strongly reshapes room behaviour
- Leather functions more as a heat donor than a self-warming room
- Kitchen and landing behaviour are dominated by indirect heating paths
- conservatory behaviour is exceptional enough that it should not drive comfort scoring
- a useful model is about ranking, diagnosis, and control support, not pretending every room parameter is known perfectly

## What the model is useful for

The model is most useful when asking comparative questions such as:

- which rooms are flow-starved versus fabric-limited?
- which interventions are likely to help most?
- what target flow is needed to hit a chosen room temperature trajectory?
- is a comfort miss more likely to be control, hydraulics, ventilation, or coupling?

That comparative use is more reliable than treating every fitted parameter as lab-grade truth.

## Operational boundary

When changing code or trusting a current parameter value, prefer `lat.md` and the generated artifacts over this narrative file. Treat this page as the long-form notebook explaining how the model got here.

## Useful commands

```bash
cargo run --bin heatpump-analysis -- thermal-rooms
cargo run --bin heatpump-analysis -- thermal-connections
cargo run --bin heatpump-analysis -- thermal-analyse --config model/thermal-config.toml
cargo run --bin heatpump-analysis -- thermal-calibrate --config model/thermal-config.toml
cargo run --bin heatpump-analysis -- thermal-validate --config model/thermal-config.toml
cargo run --bin heatpump-analysis -- thermal-operational --config model/thermal-config.toml
```

## Related documents

- [House layout](house-layout.md)
- [`../lat.md/thermal-model.md`](../lat.md/thermal-model.md)
- [`../lat.md/domain.md`](../lat.md/domain.md)
- [`../lat.md/architecture.md`](../lat.md/architecture.md)
