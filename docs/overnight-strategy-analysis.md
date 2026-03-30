# Overnight heating strategy analysis

Date: 29 March 2026 (revised — replaces initial analysis of 28 March)

## Context

Vaillant Arotherm Plus 5kW heat pump, Octopus Cosy tariff, Tesla Powerwall 2, 13 rooms with Zigbee sensors, eBUS monitoring. House is 1930s solid brick, 180m², HTC ~261 W/K.

This analysis uses 512 days of measured emonhp data (324 winter nights), a calibrated Rust backtest model (`src/overnight.rs`), and a one-night live trial (28–29 March) to determine the optimal overnight and DHW strategy.

## Octopus Cosy tariff — three rate periods

| Rate | Price | Times |
|---|---|---|
| **Cosy (off-peak)** | 14.05p/kWh | 04:00–07:00, 13:00–16:00, **22:00–00:00** |
| **Mid-peak** | 28.65p/kWh | 00:00–04:00, 07:00–13:00, 19:00–22:00 |
| **Peak** | 42.97p/kWh | 16:00–19:00 |

Tesla Powerwall (13.5 kWh) covers ~95% of non-Cosy usage at effective Cosy rate. The 5% leakage hits grid at mid/peak rates. Effective blended rate: **14.63p/kWh**.

## Key finding: the HP is at capacity on cold days

The most important discovery: on cold days (<6°C), the 5kW Arotherm cannot maintain 21°C. The leather room (emonth2 canary sensor) stabilises at **19.5–20°C** regardless of overnight strategy. The HP runs flat out and the house temperature is limited by HP sizing, not scheduling.

Evidence from 134 winter nights (Nov 2025 – Mar 2026):
- Leather room at 08:00: avg 20.0°C, min 17.7°C
- Only **7% of nights** reached 21°C by 08:00 (under 4°C setback)
- On <0°C nights: avg 19.1°C at 08:00, **never** reached 21°C — not even by midday
- On 0–3°C nights: reaches 21°C at **15:00 on average** (7 out of 15 days)
- On 6–9°C nights: reaches 21°C at **12:00 on average** (31 out of 44 days)

This means overnight strategy debates are secondary — the HP capacity is the binding constraint.

## What we control

1. **Z1 heating schedule** — day temp (21°C) vs night temp (setback) and when to switch
2. **DHW timer windows** — when the VRC 700 is allowed to fire DHW
3. **DHW mode** — eco (~2h, lower MWT, better COP) vs normal (~1h, higher MWT, worse COP). Set manually on the Arotherm controller — not writable via eBUS.

## What we don't control

- Cosy tariff rates and times (fixed by Octopus)
- The 21°C day setpoint (fixed in VRC 700)
- HP capacity on cold days (~5kW max, equilibrium ~20°C at 0°C outside)
- DHW mode via automation (hmu HwcMode is read-only on eBUS)

## Implemented configuration

### Heating schedule (via eBUS, Z1 timer)

| Period | Temp | Tariff band | Rationale |
|---|---|---|---|
| 00:00–04:00 | **19°C** (night setback) | Mid-peak (dead zone) | Battery likely depleted; HP only fires if house drops below 19°C. Costs ~£20/yr. House naturally sits at 18.5–19.5°C so setback rarely triggers on mild nights. |
| 04:00–00:00 | **21°C** (day mode) | Cosy + mid + peak | HP runs at target. On cold days, can't actually reach 21°C — stabilises at ~20°C. |

Previous setup was 17°C setback (4°C drop). Data showed the house never drops to 17°C naturally, so the old setback was paying for nothing. 19°C setback catches only the coldest nights and costs £20/yr vs £0 for the old 17°C.

eBUS commands (already set):
```bash
echo 'write -c 700 Z1NightTemp 19' | nc -w 2 localhost 8888
echo 'write -c 700 Z1DayTemp 21' | nc -w 2 localhost 8888
# Timer: day mode from 04:00 (use -:- not 00:00 for end — see vrc700-settings-audit.md issue #5)
echo 'write -c 700 Z1Timer_Monday 04:00;-:-;-:-;-:-;-:-;-:-' | nc -w 2 localhost 8888
# (same for all days)
```

### DHW timer windows (via eBUS, aligned to Cosy periods)

| Window | Cosy period | Rationale |
|---|---|---|
| **05:30–07:00** | Morning Cosy | Main DHW cycle. Delayed to 05:30 — the latest start where 100% of Normal cycles finish within Cosy (worst case 06:58). HP heats the house for 1.5h first (04:00–05:30) at Cosy rate. Eco spills ~30 min past 07:00 but costs 40p/year — not worth seasonal adjustment. |
| **13:00–15:00** | Afternoon Cosy | Top-up. Was 13:00–16:00, shortened to prevent spills into 16:00 Peak. Data showed 18 historical peak spills under old schedule. |
| **22:00–00:00** | Evening Cosy | New. Top-up after evening showers. ~6% of days had evening DHW. |

eBUS commands (already set):
```bash
echo 'write -c 700 HwcTimer_Monday 05:30;07:00;13:00;15:00;22:00;-:-' | nc -w 2 localhost 8888
# (same for all days)
```

### DHW mode — seasonal manual switch

| Period | Mode | Rationale |
|---|---|---|
| **Cold season** (when house feels cold in morning) | **Normal** (~1h) | Faster DHW frees more Cosy time for space heating. Data: 19/20 eco cycles on cold mornings never recovered in 3 hours. Normal recovers 48/65 on cool days. |
| **Mild season** (when house is warm through morning) | **Eco** (~2h) | Better COP (3.1 vs 2.5) saves ~£12/yr. House barely cools during eco DHW on mild days (0.2°C drop). |

Switch trigger: **when you first notice the house isn't warm by mid-morning, switch to Normal**. Switch back when it feels fine. Typically November → March based on temperature data.

Cannot be automated — hmu HwcMode is read-only via eBUS (confirmed by testing hex writes, -def writes, and searching ebusd GitHub issues).

## Analysis journey and dead ends

### 1. OFF overnight + Cosy recovery (rejected)

Initial idea: turn HP OFF via eBUS at night, recover during morning Cosy window at cheap rate.

Built `src/overnight.rs` — Rust backtest with:
- Cooling model calibrated from DHW events (k=0.039/hr, not idle cycles)
- Recovery/maintenance heating bins separated
- Three-rate Cosy tariff with 95% battery coverage

Results: adaptive strategy saves **£6/year** at 19.5°C target. The 4°C setback was already near-optimal. Live trial (28–29 March, OFF 23:15→04:00): leather room barely moved (21.6→21.1°C) but elvina dropped to 15.8°C.

**Rejected** because: savings are trivial, cold rooms (elvina, office) suffer badly, and the HP can't recover them in the Cosy window anyway.

### 2. Midnight OFF + 04:00 ON via crontab (trialled and removed)

Deployed `cosy-scheduler` (Rust binary on pi5data) with crontab. Ran for one night. Removed because the 19°C setback via VRC 700 timer achieves the same thing without external automation — the HP just doesn't fire if the house is above 19°C.

The `cosy-scheduler` binary remains at `/usr/local/bin/cosy-scheduler` on pi5data for potential future use.

### 3. DHW timing optimisation (partially implemented)

Moving DHW windows to align with Cosy periods: **implemented**. The afternoon window shortened to 13:00–15:00 to avoid peak spills.

Moving DHW later in the morning window (heat first, DHW at 06:00): **no benefit**. Data showed the HP can't recover the house even with 2h of pre-DHW heating. The measured heating rate during morning Cosy is **negative** at all outside temperatures — the house cools even with the HP running flat out.

### 4. Thermal pre-charging before peak (not worth it)

Eliminating all HP usage during 16:00–19:00 peak saves **£7/year**. The battery already covers 95% of peak usage. Not worth the complexity.

### 5. Battery makes scheduling mostly irrelevant

The Powerwall covers 95% of non-Cosy HP usage at effective Cosy rate (14.63p/kWh). Total HP electricity by tariff band (512 days):
- Cosy: 1992 kWh (32%)
- Mid-peak: 3549 kWh (57%)
- Peak: 696 kWh (11%)

With battery: effective cost £912 vs £1596 naive (no battery). The battery has already captured most of the tariff arbitrage. Scheduling optimisation yields £15–40/year total.

## Calibrated model parameters

### Cooling (from 27,047 DHW minutes + 40,479 long-idle minutes)

- k = 0.039/hr (house cools 0.039°C/hr per °C of indoor-outdoor ΔT)
- Thermal capacity: 6,723 Wh/°C (τ = 25.8 hours)
- At 12°C ΔT: 0.47°C/hr cooling → 3.3°C drop over 7h off

Note: calibrated from DHW events (genuine "no space heating" conditions), not short idle cycles. The idle-cycle rate (k=0.014) was 3× too slow because surrounding rooms were still warm from recent heating.

### Heating recovery (from emonhp data, heating state with indoor_t rising)

| T_out | Heat W | Elec W | COP | MWT |
|---|---|---|---|---|
| -2–0°C | 5700 | 1849 | 3.08 | 30.5°C |
| 2–4°C | 5180 | 1420 | 3.65 | 31.3°C |
| 6–8°C | 4045 | 841 | 4.81 | 30.2°C |
| 10–12°C | 2913 | 481 | 6.06 | 28.3°C |

### DHW (436 cycles ≥30 min)

| Mode | Duration | Electricity | COP | House temp drop |
|---|---|---|---|---|
| Normal | 58 min avg | 1.19 kWh | ~2.5 | 0.2°C (cold), 0.1°C (mild) |
| Eco | 108 min avg | 1.66 kWh (cold) / 1.7 kWh (mild) | ~3.3 | 0.5°C (cold), 0.2°C (mild) |

## Revert instructions

```bash
# Restore old 4°C setback:
echo 'write -c 700 Z1NightTemp 17' | nc -w 2 localhost 8888

# Restore old heating timer (day from 05:00):
for day in Monday Tuesday Wednesday Thursday Friday Saturday Sunday; do
  echo "write -c 700 Z1Timer_${day} 05:00;-:-;-:-;-:-;-:-;-:-" | nc -w 2 localhost 8888
done

# Restore old DHW timers (pre-optimisation):
for day in Monday Tuesday Wednesday Thursday Friday Saturday Sunday; do
  echo "write -c 700 HwcTimer_${day} 05:00;07:00;13:00;16:00;-:-;-:-" | nc -w 2 localhost 8888
done
```

## Related files

- `src/overnight.rs` — Rust backtest model (30 strategies × 324 nights)
- `cosy-scheduler/` — standalone Rust binary for pi5data (deployed but unused)
- `model/overnight.py` — initial Python model (superseded by Rust version)
- `AGENTS.md` — setback history, eBUS commands, operational model
