# Pico W eBUS Adapter — Build Plan

## Goal

Replace ebusd and john30's closed-source ESP32 firmware with our own
Rust/Embassy firmware on a Pi Pico W. Log all raw eBUS telegrams to
InfluxDB via MQTT. Send commands via MQTT when needed. No ebusd dependency.

## Current related truth in `lat.md`

This document is a future build plan. The current live eBUS stack and controller assumptions are documented in `lat.md`.

- [`lat.md/infrastructure.md#eBUS Stack`](../lat.md/infrastructure.md#ebus-stack) — the live ESP32 → ebusd → MQTT stack and the planned Pico W replacement
- [`lat.md/architecture.md#eBUS Availability`](../lat.md/architecture.md#ebus-availability) — current controller dependency on ebusd TCP availability
- [`lat.md/constraints.md#eBUS Timer Encoding`](../lat.md/constraints.md#ebus-timer-encoding) — timer byte semantics already learned from VRC 700 work
- [`lat.md/constraints.md#eBUS Control Flow`](../lat.md/constraints.md#ebus-control-flow) — why writes must go via the VRC 700 rather than directly to the HMU
- [`lat.md/heating-control.md#Writable eBUS Registers`](../lat.md/heating-control.md#writable-ebus-registers) — the current write surface used by the live controller

When this Pico-based replacement becomes operational truth rather than a plan, the corresponding current-state details should move into `lat.md`.

## Why

john30's ESP32 firmware (`ebusd-esp` and `ebusd-esp32` repos) is
**closed-source** — both repos contain only pre-built binaries, no source
code, no license file. We can't inspect, audit, or fix the firmware running
on our heating system's control bus.

The ebusd ecosystem is also over-engineered for our needs. We have one
Vaillant system with 3 known devices and ~30 values we care about. ebusd
provides auto-scanning, a config CDN, KNX bridging, multi-client TCP,
ACL-based access control, and Home Assistant MQTT Discovery — none of which
we use. The entire stack (closed-source ESP32 firmware → TCP bridge →
ebusd daemon → MQTT) can be replaced by a Pico W reading bytes off a wire.

The open-source firmware alternatives (danielkucera's `esp-arduino-ebus`)
struggle with arbitration timing because they run WiFi and eBUS protocol
on a single-core ESP32. The Pico W solves this with dual cores and PIO.

## What ebusd does that we don't need

| ebusd feature | Our alternative |
|---|---|
| Auto-scan 25 master addresses | We know our 4 devices (3 Vaillant + ebusd) |
| Config CDN (downloads CSVs by scan result) | ~30 commands hardcoded |
| Message caching + HTTP API | InfluxDB + Grafana |
| Poll scheduling | Most values broadcast passively; rest via MQTT timer |
| KNX bridge | Don't have KNX |
| HA MQTT Discovery | Use Grafana, not HA for this |
| Multi-client TCP (port 8888) | MQTT handles fan-out |
| ACL access control | Single-purpose device |
| Hex dump/replay | `ebus/raw` MQTT topic + InfluxDB |

**One thing to handle**: Some values are not broadcast passively — the VRC 700
and HMU only exchange them when polled. Phase 3 discovers which values need
active polling by comparing passive captures against ebusd's polled output.

## Architecture

```
Vaillant eBUS (2-wire, 20V)
        │
┌───────┴────────────┐
│ xyzroe eBus-TTL    │  Bus-side: powered from eBUS, LM2903 comparator,
│ adapter (isolated) │  BC817 transistor, LTV357T-B optocouplers
│                    │  TTL-side: 3.3V from Pico, clean UART signal
└───────┬────────────┘
   4 wires (GND, 3V3, RX, TX)
        │
┌───────┴────────────┐
│ Pi Pico W          │  Core 0: PIO UART RX/TX → Framer → telegram queue
│ (Rust/Embassy)     │  Core 1: CYW43 WiFi → MQTT publish/subscribe
│                    │  Powered from emonhp USB
└───────┬────────────┘
        │ WiFi / MQTT
        ▼
┌────────────────────┐
│ pi5data            │  Mosquitto → decoder service → InfluxDB → Grafana
│ (10.0.1.230)       │  Sends commands to ebus/send topic
└────────────────────┘
```

## Hardware

### xyzroe eBus-TTL Adapter

[github.com/xyzroe/eBus-TTL-adapter](https://github.com/xyzroe/eBus-TTL-adapter)

Galvanically isolated eBUS ↔ TTL bridge. Two LTV357T-B optocouplers
isolate RX and TX paths. Bus-side powered from eBUS via zener clamp
(no external supply needed). TTL side powered from Pico's 3.3V rail.

**Key components (bus side):**
- D1 bridge rectifier — polarity independence
- VD1/VD2 zeners (7.5V) — voltage clamping
- Q1/Q2 BC817 — TX current sink (pulls bus low)
- LM2903 comparator — detects bus voltage transitions → RX signal
- R3/R5 (51kΩ) voltage divider — sets comparator threshold

**Connector X2 (4-pin header):**

| Pin | Signal | Wire to Pico W |
|-----|--------|----------------|
| 1   | GND    | GND (pin 38)   |
| 2   | RX     | GP5 (pin 7) — PIO input |
| 3   | TX     | GP4 (pin 6) — PIO output |
| 4   | +V     | **3V3_OUT (pin 36)** — NOT VBUS |

> **3V3 not 5V**: RP2040 GPIOs are 3.3V and not 5V tolerant. The
> LTV357T-B optocouplers work fine at 3.3V. If RX output swing is
> insufficient, drop R11 (10kΩ pull-up) to ~4.7kΩ.

### Power

Pico W powered via USB from emonhp (10.0.1.169, Pi 4).
Pico W draws ~150mA typical with WiFi, ~250mA peak.
emonhp USB budget: ~550mA free after existing devices (SanDisk boot
stick 896mA declared / ~100mA actual, two serial adapters ~50mA each).
No problem.

### Wiring

- eBUS cable: 2-wire to Vaillant eBUS screw terminals. **Polarity
  doesn't matter** (bridge rectifier on xyzroe board).
- Cable spec: eBUS minimum 0.34mm². We use 1mm² — massive overkill.
- Distance: eBUS spec supports 100m. Ours is <20m. No signal concerns.
- **Don't run alongside mains** cable unless both rated for higher voltage.
- **Screw terminals tight** — loose connections cause intermittent issues.
- Multiple adapters can share the same eBUS terminals (spec supports
  multiple participants). john30's v5 adapter stays connected during
  development.

### Test Equipment

- **Saleae Logic** — digital timing on GP4/GP5, UART decode
- **PicoScope** — analogue signal quality on eBUS lines and comparator
- **john30 Adapter v5 + ebusd** — known-good reference running in parallel

## eBUS Protocol

Source: [eBUS Spec v1.3.1](https://adapter.ebusd.eu/Spec_Prot_12_V1_3_1.pdf)

### Wire Format

- **2400 baud, 8N1** — standard UART, 416.67μs per bit, 4166.7μs per byte
- **Bus idle**: 15–24V (logical 1). **Bus active**: 9–12V (logical 0)
- **SYN (0xAA)**: delimiter between telegrams. AUTO-SYN every 35ms if idle.

### Telegram Structure

```
Master part:  QQ ZZ PB SB NN [DB₀..DBₙ] CRC
              │  │  │  │  │              └─ CRC-8 (poly 0x9B)
              │  │  │  │  └─ data length 0-16
              │  │  └──┘ command bytes (primary + secondary)
              │  └─ destination (0xFE = broadcast)
              └─ source (master address)

Slave response (master-slave only):
              ACK NN [DB₀..DBₙ] CRC ACK
              │                     └─ master ACK (0x00=ok, 0xFF=retry)
              └─ slave ACK (0x00=ok, 0xFF=retry)

Byte stuffing (all bytes except SYN):
              0xAA → 0xA9 0x01
              0xA9 → 0xA9 0x00
```

### Addresses

25 valid master addresses. Each nibble must satisfy `((n+1) & n) == 0`
(valid values: 0x0, 0x1, 0x3, 0x7, 0xF). Slave = master + 5.

Our bus (from `ebusd info` / `scan result`):

| Master | Slave | ID | Device | Firmware | Role |
|--------|-------|----|--------|----------|------|
| 0x10 | 0x15 | 70000 | VRC 700 | SW 06.14, HW 69.03 | System controller — scheduling, weather comp, UI. Sends SetMode to HMU every ~10s. |
| 0x03 | 0x08 | HMU00 | aroTHERM plus VWL 55/6 | SW 09.02, HW 51.03 | Heat pump outdoor unit — compressor, fan, refrigerant. Executes commands. |
| 0x71 | 0x76 | VWZIO | VWZ AI | SW 02.02, HW 01.03 | Indoor unit — circulation pump, 3-way diverter, SP1 cylinder sensor. **Active master**: sends extensive `71→08` traffic to HMU (calibration, parameters, real-time control). |
| 0x31 | 0x36 | — | ebusd | — | Our current adapter. Will be replaced by Pico W. |

Live bus stats: symbol rate ~40/s, max 209/s. Arbitration: 0–44µs (spec allows 60–104µs). Signal: acquired.

We listen as 0xFF (passive). For Phase 5 (active sending), we'll need an unused master address.

### Arbitration (writing only)

After SYN, write your address within **4300–4456μs** of the SYN start bit.
Read back: if it matches, you won. If different address with same priority
class (lower nibble), retry next SYN. Otherwise, lost — back off.

**This is the only hard part**, and only matters when sending. PIO solves
it with cycle-accurate start-bit timestamps.

## Why PIO

The fundamental problem every eBUS adapter struggles with: **knowing exactly
when the SYN byte arrived.** 

- Linux UART FIFO: minimum 4-byte trigger = 16.7ms latency (ttyebus kernel
  module exists solely to work around this — 991 lines of kernel code)
- ESP32 UART: FIFO buffering + WiFi interrupts = unpredictable jitter
  (danielkucera's arbitration issue #22 — 60% write failures)
- john30's solution: closed-source "enhanced protocol" firmware

PIO on RP2040: custom state machine detects start-bit falling edge with
**cycle-accurate timing** (~8ns at 125MHz). No FIFO, no interrupts, no
contention with WiFi (runs on Core 1). The 156μs arbitration window is
trivially wide compared to PIO's precision.

## PIO UART Design

Two PIO state machines at 2400 baud. Clock divisor: 125MHz / 2400 = 52,083.

**SM0: RX** — detects start bit, samples 8 data bits mid-bit, pushes byte
to FIFO. Start-bit timestamp captured via DMA to free-running timer or
companion SM acting as cycle counter.

**SM1: TX** — pulls bytes from FIFO, sends start bit + 8 data bits + stop
bit. Only enabled when actively sending (Phase 5).

PIO programs are ~15 instructions each. Saleae verifies timing.

## Rust Crate: `ebus-core` (no_std) — IMPLEMENTED

Ported from `yuhu-ebus/` submodule — only the parts we need. **Phase 1 complete**: crate lives at `ebus-core/` in the repo with 22 passing tests.

### What was ported (~710 lines of Rust)

| yuhu-ebus source | Rust module | What | Status |
|-----------------|-------------|------|--------|
| `Utils/Common.cpp` (155 lines) | `crc.rs`, `address.rs` | CRC-8 table, `is_master()`, `slave_of()` | ✅ Done |
| `Core/Sequence.cpp` (170 lines) | `sequence.rs` | Byte stuffing / unstuffing | ✅ Done |
| `Core/Telegram.cpp` (462 lines) | `telegram.rs` | Parse master + slave parts | ✅ Done |
| — (new) | `framer.rs` | SYN-delimited byte stream → frames | ✅ Done |
| `Core/Request.cpp` (219 lines) | `arbitration.rs` | Bus request FSM (Phase 5 only) | ⏳ Deferred |

### What we DON'T port

| yuhu-ebus component | Why not |
|---------------------|---------|
| `Handler.cpp` (735 lines, 15-state FSM) | Reactive mode — responding when addressed. Nothing addresses us. |
| `Controller.cpp` | PIMPL lifecycle wrapper. Unnecessary. |
| `Scheduler.cpp` | Priority queue for outbound messages. We send one at a time via MQTT. |
| `PollManager.cpp` | Periodic polling. We listen passively — the bus devices poll each other. |
| `ClientManager.cpp` | TCP bridge to ebusd. We use MQTT. |
| `DeviceScanner.cpp` | Bus scanning. We know our 3 devices. |
| `BusPosix.cpp` / `BusFreeRtos.cpp` | Platform UART abstraction. We have PIO. |

### New component: `Framer`

Not in yuhu-ebus. Byte-at-a-time stateful consumer: feeds bytes in, emits
complete `RawTelegram` structs when a SYN delimiter is seen. This is the
primary interface for passive listening.

```rust
pub struct Framer {
    buf: [u8; 64],
    len: usize,
}

impl Framer {
    /// Feed a byte. Returns Some(telegram) when a complete
    /// telegram has been delimited by SYN.
    pub fn feed(&mut self, byte: u8) -> Option<RawTelegram> { ... }
}
```

### Crate structure

```
ebus-core/
├── src/
│   ├── lib.rs
│   ├── symbols.rs       — SYN, ACK, NAK, escape constants
│   ├── crc.rs           — CRC-8 table (256 bytes) + calc_crc()
│   ├── address.rs       — is_master(), is_slave(), master_of(), slave_of()
│   ├── sequence.rs      — extend() / reduce() byte stuffing
│   ├── telegram.rs      — RawTelegram, parse master/slave, validate CRC
│   ├── framer.rs        — SYN-delimited stream → RawTelegram
│   └── arbitration.rs   — RequestState FSM (observe/first/retry/second)
├── tests/               — test vectors from yuhu-ebus/tests/
└── Cargo.toml           — #![no_std], optional defmt
```

## Firmware: `pico-ebus`

```
Core 0                          Core 1
──────                          ──────
┌──────────────┐               ┌──────────────┐
│  ebus_task   │               │  wifi_task   │
│              │               │  CYW43       │
│  PIO RX FIFO │               │  connect/    │
│      ↓       │               │  reconnect   │
│  Framer      │               └──────────────┘
│      ↓       │               ┌──────────────┐
│  Channel ────┼──────────────→│  mqtt_task   │
│              │  telegram     │  publish to  │
│  PIO TX ←────┼───────────────│  ebus/*      │
│              │  send cmd     │  subscribe   │
└──────────────┘               │  ebus/send   │
                               └──────────────┘
```

## MQTT Topics

| Topic | Direction | Payload |
|-------|-----------|---------|
| `ebus/telegram` | Pico → pi5data | Hex-encoded raw telegram bytes |
| `ebus/error` | Pico → pi5data | CRC failures, framing errors |
| `ebus/send` | pi5data → Pico | Hex bytes to transmit |
| `ebus/result` | Pico → pi5data | Send result (won/lost/error) |
| `ebus/status` | Pico → pi5data | Heartbeat, uptime, telegram count |

## Decoding (on pi5data, not on Pico)

Small Rust service or Telegraf exec plugin. Subscribes to `ebus/telegram`,
matches command bytes against a lookup table derived from ebusd-configuration
CSVs, writes decoded values to InfluxDB.

Example: telegram with PB=0xB5, SB=0x1A, data prefix `05 FF 32 26` →
this is `hmu/RunDataFlowTemp`, slave response bytes decode as D2C (signed
16-bit ÷ 256) → flow temperature in °C.

We only need ~30 command definitions for our Vaillant system, hardcoded
from the CSVs. No runtime CSV parsing.

## Phases

### Phase 1: `ebus-core` crate (no hardware) — ✅ COMPLETE (5 Apr 2026)

Port protocol primitives. Test on desktop with `cargo test` using test
vectors from `yuhu-ebus/tests/Core/test_telegram.cpp`.

**Delivered** (`ebus-core/` in repo):

| Module | What | Tests |
|--------|------|-------|
| `symbols.rs` | SYN, ACK, NAK, ESC, BROADCAST constants | — |
| `crc.rs` | CRC-8 table (poly 0x9B) + `calc_crc()` / `crc_bytes()` | 4 |
| `address.rs` | `is_master()`, `is_slave()`, `master_of()`, `slave_of()` — 25 valid master addrs | 5 |
| `sequence.rs` | `extend()` / `reduce()` byte stuffing, CRC over extended bytes | 3 |
| `telegram.rs` | Parse broadcast, master-master, master-slave; CRC validation; NAK retry stubs | 6 |
| `framer.rs` | SYN-delimited `Framer` — feed bytes, emit `RawFrame` | 4 |

22 tests total. Builds `no_std` on `thumbv6m-none-eabi`. `Cargo.toml` has optional `defmt` feature for embedded logging.

**Not ported** (as planned): Handler, Controller, Scheduler, PollManager, ClientManager, DeviceScanner, Bus platform layers. NAK retry parsing is stubbed — returns error for now (sufficient for passive listening in Phase 3; full retry parsing needed only for Phase 5 active sending).

### Phase 2: PIO UART (Pico W, no eBUS) — NEXT

Still waiting on hardware/test-bench time. PIO RX + TX at 2400/8N1. Test with loopback wire (GP4→GP5).
Verify timing with Saleae.

**Prerequisites**: Pico W board, xyzroe eBus-TTL adapter (confirm purchased),
Saleae Logic for timing verification. Embassy runtime + PIO crate setup.

### Phase 3: Passive listener (first connection to real bus)

PIO RX → Framer → USB serial output. Compare against ebusd running on
john30's v5 adapter in parallel on the same bus. PicoScope if signal
quality issues.

**Key discovery task**: Determine which values appear passively (from
normal VRC 700 ↔ HMU/VWZ AI chatter) vs which need active polling. Run
both adapters simultaneously — compare Pico's passive captures against
ebusd's polled output.

Known passive traffic patterns (from `grab result` analysis, 30 Mar 2026):
- `10→08` (VRC 700 → HMU): SetMode (~10s), Status01, Status02, parameters
- `10→76` (VRC 700 → VWZ AI): status reads, parameters
- `10→fe` (VRC 700 → broadcast): date/time, outside temp
- `71→08` (VWZ AI → HMU): extensive real-time control traffic (command `b51a`,
  hundreds of sub-addresses — calibration data, compressor parameters)

Much of what ebusd-poll.sh actively polls may already be visible passively
in the VRC 700 and VWZ AI traffic. Phase 3 quantifies this.

### Phase 4: WiFi + MQTT

Publish telegrams to pi5data. Decoder service writes to InfluxDB.
Verify completeness against ebusd.

### Phase 5: Active sending (optional, later)

Enable PIO TX. Implement arbitration. Subscribe to `ebus/send`.
Verify timing with Saleae. Test with safe read commands first.

**Important**: All write commands should target the VRC 700 (address 0x15),
not the HMU or VWZ AI directly. The VRC 700 relays decisions downstream
via SetMode every ~10s. Direct HMU writes get overwritten. See
"Bus hierarchy" section above.

## Reference Code

| Repo (submodule) | What to study |
|-----------------|---------------|
| `yuhu-ebus/` | Protocol FSM, CRC, telegram parsing, arbitration — **our blueprint**. Roland Jax (author) also maintains the protocol engine inside `esp-arduino-ebus/`. Actively maintained (last commit 29 Mar 2026). 8.8k LOC + 3.7k test LOC. |
| `esp-arduino-ebus/` | danielkucera's firmware. **Uses yuhu-ebus as its protocol engine.** Arbitration impl, bus timing comments, standalone INTERNAL mode (proves ebusd can be eliminated). |
| `ebusd/` | Timing constants in `protocol.h`, message format in `data.cpp`, **TTM data type encoding** in `datatype.cpp` (critical — see Vaillant timer encoding below). |
| `ttyebus/` | Why Linux UART latency kills eBUS timing — motivation for PIO |

## Vaillant Command Reference

From `ebusd find -f -c hmu` / `ebusd find -f -c 700` on running system:

| Name | Circuit | Dst | PB SB | Data prefix | Response type |
|------|---------|-----|-------|-------------|--------------|
| RunDataFlowTemp | hmu | 08 | B5 1A | 05FF3226 | D2C (÷256 = °C) |
| RunDataReturnTemp | hmu | 08 | B5 1A | 05FF3227 | D2C |
| BuildingCircuitFlow | hmu | 08 | B5 1A | 05FF323C | UIN (l/h) |
| RunDataStatuscode | hmu | 08 | B5 1A | ... | status enum |
| DisplayedOutsideTemp | 700 | 15 | B5 09 | ... | D2C |
| HwcStorageTemp | 700 | 15 | B5 09 | ... | D2C |
| HwcSFMode (write) | 700 | 15 | B5 23 | ... | "load"/"auto" |

> Full command table to be extracted from ebusd-configuration CSVs before
> Phase 3. Run `echo 'find -f' | nc pi5data 8888` to dump all known
> commands with their raw byte patterns.

## Vaillant-specific knowledge (learned the hard way)

### Bus hierarchy

The VRC 700 is the **scheduling brain** — it decides when to heat and when
to charge DHW, then sends `SetMode` to the HMU every ~30 seconds
(confirmed from ebusd `grab result all`: 25,700 occurrences over a
multi-day session). The flow temperature demand goes directly to the
HMU (0x08), not via the VWZ AI. The VWZ AI (0x76) receives separate
messages with zeros for the flow temp field — it handles valve/pump
commands only:

```
SetMode QQ=10: auto;28.5;-;-;0;1;1;0;0;0
                     │         │
                     │         └─ HwcDemand (0=no, 1=yes)
                     └─ flow temp demand
```

The VWZ AI (0x71) is an **active real-time controller** — it independently
sends extensive `71→08` traffic to the HMU (calibration, parameters, valve
control). It is NOT a passive relay.

The HMU (0x08) **executes what it's told** by both masters. Direct writes
to the HMU from a third party get **overwritten by the VRC 700 within 10
seconds**.

**Rule: all commands go to the VRC 700 (0x15). Let it relay downstream.**

### TTM timer encoding (critical)

Timer registers use the TTM data type: 8-bit, 10-minute resolution.

| Value | Byte | Meaning |
|-------|------|---------|
| `00:00` | `0x00` | Midnight at **start** of day |
| `04:00` | `0x18` | 04:00 (4×6 = 24) |
| `22:00` | `0x84` | 22:00 (22×6 = 132) |
| `23:50` | `0x8F` | Last valid time (143) |
| `-:-` | `0x90` | Replacement / not set / **"end of day"** |

**`00:00` is NOT "end of day".** It's start of day (byte 0x00). A window
from `04:00` to `00:00` has end < start and is **silently rejected** by
the VRC 700. Use `-:-` (0x90) for "until end of day".

The VRC 700 receives all 6 TTM bytes (3 window pairs) atomically in one
eBUS write. If any window is invalid, the entire timer may be rejected.

Factory default periods from VRC 700 manual: `06:00–08:00`, `16:30–18:00`,
`20:00–22:30` — all end times well before midnight.

Working CcTimer (unchanged): `06:00;22:00;-:-;-:-;-:-;-:-` — explicit end
before midnight, unused slots use `-:-`.

Sources:
- `ebusd/src/lib/ebus/datatype.cpp` line 1337
- `docs/ebus-specs/vaillant_ebus_v0.5.0.pdf` section 3.1.3 (GetTimerProgram)
  and section 3.2.2 (SetTimerProgram) — explicitly documents `90h = 24:00h`
  and slave response `NN = 00h` (zero data bytes = the "empty" we see)
- Live testing 30 March 2026 (fix confirmed immediately)

### SetMode (Room Controller → Heat Pump)

The VRC 700 sends `B5h 10h` (or its modern equivalent) every ~10s.
From the Vaillant eBUS spec (section 3.5), the older VRS620 format is:

```
M8:  Lead water target temperature (°C, DATA1c)
M9:  Service water target temperature (°C, DATA1c)
M12: Status bits (bit 0 = heating, bit 1 = DHW) ← this is HwcDemand
```

Our VRC 700 uses a newer format (`SetMode QQ=10`) with more fields, but
the core is the same: flow temp demand + DHW demand flag + status bits.
The `grab result` output shows this as `10→08` traffic with count ~1570
(every ~10s over 4+ hours).

### Write responses

Timer writes (`TTM` type) return `empty` — this is **normal**. The VRC 700
ACKs the eBUS transaction but returns zero data bytes. ebusd reports this
as "empty". Other writes (e.g. `HwcSFMode`) return `done`.

### ebusd-configuration knowledge chain

The Vaillant register definitions live in:
1. **TypeSpec source**: `src/vaillant/15.700.tsp` in ebusd-configuration
2. **Compiled to CSV**: served via CDN at ebus.github.io, or fetched by
   ebusd `--scanconfig` at startup
3. **Data types**: defined in ebusd C++ source (`datatype.cpp`), not in
   the CSVs themselves

Timer fields use type `slot1_3` (defined in `_templates.tsp`) which maps
to 6× TTM bytes. The TTM encoding is in ebusd's C++ code, independently
confirmed by the Vaillant eBUS extensions spec.

### VRC 700 firmware

SW 06.14, HW 69.03. Firmware can only be updated via a VR 920/921
(sensoNET) internet gateway connected to the eBUS — we don't have one.
No way to update via ebusd or third-party adapters.

### Registers that can't be written

- `hmu HwcMode` (eco/normal) — read-only from external masters. Must be
  changed on the aroTHERM controller physically. Confirmed by hex write
  testing and ebusd GitHub issues.
- `hmu HwcModeW` — ebusd constructs hex writes but the HMU ignores them.

## eBUS ecosystem and compatibility

### Who uses eBUS

eBUS was invented by Karl Dungs and standardised by the eBUS Interest Group
(FH Ostfalia). Manufacturers using eBUS:

| Manufacturer ID | Name | Notes |
|---|---|---|
| 0x06 | Dungs | Original inventor |
| 0x10 | TEM | ebusd definitions available |
| 0x19 | Wolf | Boilers, heat pumps. ebusd definitions available |
| 0x40 | ENCON | ebusd definitions available |
| 0x50 | Kromschröder | Burner controls |
| 0x60 | Eberle | Thermostats |
| 0xB5 | **Vaillant Group** | Largest eBUS user. Proprietary extensions (PB=0xB5). |
| 0xC5 | Weishaupt | Boilers |
| — | Ochsner | Heat pumps. ebusd definitions available |

### Vaillant Group brands (same eBUS protocol)

All use identical Vaillant eBUS extensions — same byte encoding, same
register structure, same PB=0xB5 command set:

- **Vaillant** (Germany, UK)
- **Saunier Duval** (France)
- **Bulex** (Belgium)
- **AWB** (Netherlands)
- **Protherm** (Eastern Europe)
- **Glow-worm** (UK, budget range)

### Vaillant controller family (all at address 0x15)

| ebusd file | Controller | Era |
|---|---|---|
| `15.140.tsp` | calorMATIC 140 | ~2007 |
| `15.370.tsp` | calorMATIC 370 | ~2010 |
| `15.470.tsp` | calorMATIC 470 | ~2014 |
| `15.700.tsp` | **VRC 700 (multiMATIC)** | ~2016 — **ours** |
| `15.720.tsp` | VRC 720 (sensoCOMFORT) | ~2018 — symlink to 15.700.tsp |

All use the same TTM timer encoding, same `B5h 24h` command structure,
same data types. The VRC 700 is an evolution, not a redesign.

Vaillant states all eBUS-equipped systems from 2007 onwards are compatible.

### Implications for our firmware

The `ebus-core` Rust crate and Pico W firmware are **not specific to our
aroTHERM + VRC 700**. Because:

1. **Protocol layer** (CRC, framing, arbitration, byte stuffing) is the
   open eBUS standard — works with any eBUS device from any manufacturer.

2. **Vaillant command layer** (PB=0xB5, register encoding, TTM, SetMode)
   is shared across all Vaillant Group brands and all controllers from
   calorMATIC 140 through VRC 720. Our firmware works with any of them.

3. **Device-specific registers** (which registers exist at which addresses)
   vary by device but follow the same structure. The ebusd-configuration
   repo has definitions for ~30 Vaillant device types. Our ~30 hardcoded
   commands are specific to our system, but adding support for other
   Vaillant devices is just adding more entries to the lookup table.

4. **Non-Vaillant eBUS devices** (Wolf, Weishaupt, Ochsner, etc.) use the
   open eBUS standard for protocol and their own command bytes for
   application data. The Pico W firmware's protocol layer works as-is;
   only the decode/command tables differ.

So the firmware is portable across the entire Vaillant range (thousands
of installations with ebusd adapters), and the protocol core works with
any eBUS device. This matters if the project is ever open-sourced —
the potential user base is large.

### Address 0x08: gas boilers and heat pumps share the same interface

The heat source always sits at address 0x08. Gas boilers (`08.bai.tsp`)
and heat pumps (`08.hmu.tsp`) both import `hcmode_inc.tsp` which defines
the core operational registers:

- **SetMode** (PB=B5, SB=10) — the command the VRC 700 sends every ~10s:
  `hcmode`, `flowtempdesired`, `hwctempdesired`, `disablehc`, `disablehwcload`
- **Status01** (PB=B5, SB=11) — the heat source's response:
  flow temp, return temp, outside temp, DHW temp, storage temp, pump status
- **Status02** (PB=B5, SB=11, block 2) — hot water mode, max/current temps

This is why you can swap a gas boiler for a heat pump and keep the same
VRC 700 controller — the eBUS control interface is identical at the
SetMode/Status level. Our HMU adds heat-pump-specific registers on top
(compressor hours, COP, EEV steps, BuildingCircuitFlow, etc.) but the
core control loop is shared.

This means our firmware's SetMode decoder works for **any Vaillant system**,
not just heat pumps. The ~30 device-specific registers we poll are
heat-pump-only, but the passive traffic decoding covers everything.

### Address 0x76: VWZ AI is heat-pump-specific

The VWZ AI indoor hydraulic station (0x76) exists only in heat pump systems.
Gas boiler systems don't have a separate indoor unit — the boiler handles
everything at address 0x08.

### Corrected control flow (from bus traffic analysis, 30 March 2026)

The VRC 700 does **NOT** send SetMode to the HMU. It sends SetMode to
the **VWZ AI**, which translates it into real-time control of the HMU:

```
VRC 700 (10) ──B510 SetMode──→ VWZ AI (76)
  "auto; 31°C flow; no DHW"       │
                                  │ translates into
                                  │
                          VWZ AI (71) ──B51A──→ HMU (08)
                            256 registers × 5 blocks
                            = 1280 real-time parameter msgs
                            (compressor, valves, defrost...)

VRC 700 (10) ──B507/B511──→ HMU (08)
  Read-only: energy counters, status, temperatures
```

The VRC 700 **reads** from the HMU but never **controls** it directly.
The VWZ AI is the real controller. The VRC 700 is just a scheduler.

### Full VRC 700 → VWZ AI command set

| Command | Data | Count | Decoded? | Purpose |
|---------|------|-------|----------|---------|
| B5 10 (SetMode) | `auto;31.0;-;-;0;1;1;0;0;0` | ~13k (every ~10s) | **Yes** | Flow temp demand, heating/DHW mode, disable bits |
| B5 11 01 (Status01) | `31.0;27.0;-;-;-;on` | ~13k (every ~10s) | **Yes** | Flow, return, outside, DHW, storage temps, pump status |
| B5 12 (parameters) | 2 sub-addresses (0f0001, 0f0002) | ~13k | **No** | Likely real-time status readback from VWZ AI |
| B5 13 (parameters) | sub-address 040d00 | ~2.2k | **No** | Unknown |
| B5 04 01 (operational data) | block 01 | ~2.2k | **No** | Unknown |
| B5 09 (config set) | 2 one-time writes | 2 | **No** | Startup configuration |
| B5 16 (identification) | version data | ~31 | Partial | Device identification |

The undecoded B512/B513 traffic is the **return channel** — the VRC 700
reading VWZ AI state to inform its next SetMode decision. The community
hasn't reverse-engineered these because most ebusd users have gas boilers
(no VWZ AI). This is a prime target for Phase 3 passive capture analysis.

### Control strategy: steer the VRC 700, don't replace it

The VRC 700 handles the hard parts — 10-second heartbeat, SetMode
translation, VWZ AI communication, safety fallbacks. We keep all of that.
Our system provides the intelligence the VRC 700 lacks.

**The VRC 700 as a steerable state machine:**

The VRC 700 reads the outdoor temperature, applies the heat curve, checks
its timers, and produces a flow temp demand. We adjust its inputs to get
the output we want:

| Register we adjust | What it changes | When |
|---|---|---|
| `Hc1HeatCurve` | Flow temp for a given outdoor temp | Wind, solar, tariff window |
| `Z1DayTemp` / `Z1NightTemp` | Room temp target | Occupancy, tariff, thermal model |
| `HwcSFMode` | Force DHW charge | Tariff optimisation |
| `Z1Timer` / `HwcTimer` | Day/night/DHW windows | Seasonal, Cosy schedule |
| `HwcTempDesired` | DHW target temp | Seasonal (45°C summer, higher winter) |

**What our system knows that the VRC 700 doesn't:**

| Data source | What it tells us | How we use it |
|---|---|---|
| 13 room sensors (Zigbee) | Per-room temperatures and humidity | Detect rooms not meeting target, adjust setpoint |
| PV generation | Solar gain on south-facing rooms | Reduce flow temp demand when sun is heating the house |
| Wind speed (Met Office / forecast) | Effective HTC increase on solid brick walls | Bump heat curve gradient in windy conditions |
| Weather forecast | Tomorrow's conditions | Pre-heat before a cold night, back off before a warm day |
| Cosy tariff windows | Electricity price right now | Overdrive flow temp during cheap periods, coast during peak |
| Battery state (Powerwall) | Can we afford to run at peak rates? | Avoid compressor during battery-depleted peak |
| Thermal model (calibrated HTC 261 W/K) | How fast is the house cooling? | Predict recovery time, adjust setback depth |
| emonhp heat meter | Actual COP and heat output | Verify the VRC 700's demand is producing expected results |
| HwcStorageTemp + Multical T1/T2 | Real cylinder state | Time DHW charges optimally, not just when VRC 700 decides |

**Example scenarios:**

| Condition | Same outdoor temp | VRC 700 does | We do instead |
|---|---|---|---|
| Calm sunny day, 8°C | 8°C | Fixed curve → 33°C flow | Drop curve to 0.45 — solar gain is heating south rooms |
| Windy overcast, 8°C | 8°C | Same 33°C flow | Bump curve to 0.65 — wind is doubling the effective heat loss |
| 4am Cosy window, 5°C | 5°C | Normal curve → 37°C flow | Bump to 0.70 — cheap electricity, pre-heat before peak |
| Peak period, battery low | Any | Normal curve | Drop day temp by 1°C — avoid expensive compressor use |
| Leather room 2°C below target | Any | Doesn't know | Bump setpoint temporarily — conservatory door was opened |

**The feedback loop:**

Our Pico W (or the adaptive-heating-mvp Rust service already running on pi5data — see `docs/heating-plan.md`) monitors:
- SetMode from VRC 700 → VWZ AI (the flow temp it's actually demanding)
- Status01 from VWZ AI (actual flow/return temps achieved)
- Room temperatures (13 Zigbee sensors)
- HMU status (compressor on/off, COP)

If the house isn't responding as the model predicts, we adjust. The VRC 700
remains the reliable real-time executor — it handles the 10-second loop,
the safety systems, and the VWZ AI communication. We provide the strategy.

### Future option: replace the VRC 700 entirely

If the VRC 700 ever becomes a bottleneck (e.g. can't adjust heat curve
fast enough, or its firmware has bugs we can't work around), we can send
SetMode directly to the VWZ AI. To do this, the Pico W would need to:
1. Send **B5 10 SetMode** to the VWZ AI (address 0x76) every ~10s
2. Read **B5 11 Status01** from the VWZ AI for feedback
3. Optionally read **B5 07/B5 11** from the HMU for energy/status data

The SetMode fields are simple:

| Field | Type | What to send |
|-------|------|-------------|
| `hcmode` | UCH | 0=auto, 1=off, 2=heat, 3=water |
| `flowtempdesired` | D1C (°C) | Weather-compensated flow temp |
| `hwctempdesired` | D1C (°C) | DHW target (45°C) or `-` |
| `hwcflowtempdesired` | UCH (°C) | DHW flow temp or `-` |
| `disablehc` | bit | 0=allow heating |
| `disablehwctapping` | bit | 1=no DHW tapping |
| `disablehwcload` | bit | 1=no DHW loading |
| `remotecontrolhcpump` | bit | 0=normal |
| `releasebackup` | bit | 0=no backup heater |
| `releasecooling` | bit | 0=no cooling |

The VWZ AI handles all the complexity — compressor modulation, defrost,
valve control, refrigerant management. You just tell it the temperature
you want and the mode. It's a clean abstraction.
