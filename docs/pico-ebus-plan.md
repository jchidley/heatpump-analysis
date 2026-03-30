# Pico W eBUS Adapter — Build Plan

## Overview

Custom Rust/Embassy firmware for a Raspberry Pi Pico W, connected to an
[xyzroe eBus-TTL adapter](https://github.com/xyzroe/eBus-TTL-adapter)
(galvanically isolated via LTV357T-B optocouplers). Passive eBUS listener
with optional active sending. Publishes raw telegrams to MQTT on pi5data.

## Hardware

### xyzroe eBus-TTL Adapter

Galvanically isolated eBUS ↔ TTL bridge. Bus-side powered from eBUS (via
zener clamp), TTL side powered from Pico's 3V3.

**Connector X2 (4-pin header):**

| Pin | Signal | Wire to Pico W |
|-----|--------|----------------|
| 1   | GND    | GND (pin 38)   |
| 2   | RX     | GP5 (pin 7) — UART1 RX |
| 3   | TX     | GP4 (pin 6) — UART1 TX |
| 4   | +5V    | **3V3_OUT (pin 36)** — NOT VBUS |

> **3V3 not 5V**: The Pico's RP2040 GPIOs are 3.3V and NOT 5V tolerant.
> Powering the TTL side from 3V3 ensures the optocoupler outputs stay
> within spec. The LTV357T-B works fine at 3.3V. If the RX output swing
> is insufficient, drop R11 (10kΩ pull-up) to ~4.7kΩ.

### Power

Pico W powered via USB from emonhp (10.0.1.169). Draws ~150mA typical
with WiFi active — well within the Pi 4's USB budget alongside existing
devices (SanDisk boot stick, two serial adapters).

### Test Equipment

- **Saleae Logic** — digital timing verification on GP4/GP5
- **PicoScope** — analogue signal quality on eBUS lines and comparator output
- **john30 Adapter v5** — running ebusd in parallel as known-good reference

## eBUS Protocol Summary

Source: [eBUS Spec v1.3.1](https://adapter.ebusd.eu/Spec_Prot_12_V1_3_1.pdf),
reference implementation: `yuhu-ebus/` submodule (1,741 lines of core protocol).

### Physical

| Parameter | Value |
|-----------|-------|
| Baud rate | 2400, 8N1 |
| Bus idle (logical 1) | 15–24V (typical 20V) |
| Bus active (logical 0) | 9–12V |
| Bit time | 416.67μs |
| Byte time (start + 8 data + stop) | 4,166.7μs |
| AUTO-SYN interval | 35ms (if no traffic) |

### Framing

```
SYN (0xAA) = telegram delimiter

Master telegram:
  QQ ZZ PB SB NN [DB₀..DBₙ] CRC

  QQ  = source address (master)
  ZZ  = destination address (0xFE = broadcast)
  PB  = primary command byte
  SB  = secondary command byte  
  NN  = data length (0-16)
  DB  = data bytes
  CRC = CRC-8 (polynomial 0x9B)

Slave response (master-slave telegrams only):
  ACK NN [DB₀..DBₙ] CRC ACK

  ACK = 0x00 (positive) or 0xFF (negative)

Byte stuffing (applied to all bytes except SYN):
  0xAA → 0xA9 0x01
  0xA9 → 0xA9 0x00
```

### Addresses

25 valid master addresses. Formed from priority class (lower nibble) and
sub-address (upper nibble). Valid nibble values: 0x0, 0x1, 0x3, 0x7, 0xF.
Check: `((nibble + 1) & nibble) == 0`.

Slave address = master address + 5.

### Telegram Types

| Type | Condition | Flow |
|------|-----------|------|
| Broadcast | ZZ = 0xFE | Master → CRC → SYN |
| Master-Master | ZZ is master | Master → CRC → ACK → SYN |
| Master-Slave | ZZ is slave | Master → CRC → ACK → Slave response → CRC → ACK → SYN |

### Arbitration (for writing only)

1. Observe SYN byte on bus
2. Write your master address within **4300–4456μs** of SYN start bit
3. Read back what appeared:
   - **Your address** → you won, send telegram
   - **Different address, same priority class** → retry at next SYN (second round)
   - **Different address, different class** → you lost, wait
4. Lock counter: after winning, wait N SYNs before next attempt (fairness)

### CRC-8

Polynomial: x⁸ + x⁷ + x⁴ + x³ + x + 1 (0x9B). Table-driven, 256-byte lookup.
Calculated over the **expanded** (byte-stuffed) sequence.

## PIO UART Design

Custom PIO program instead of hardware UART. Two state machines:

### SM0: eBUS RX (with start-bit timestamping)

```
; eBUS PIO RX — 2400 baud, 8N1, with start-bit timestamp
;
; Pushes 32-bit words to FIFO:
;   [31:8] = cycle counter at start-bit falling edge
;   [7:0]  = received byte
;
; This gives the main core exact timing of when each byte arrived,
; which is the critical information for arbitration.

.program ebus_rx
.wrap_target
wait_for_start:
    wait 1 pin 0        ; wait for line idle (high)
    wait 0 pin 0        ; falling edge = start bit
    
    ; Capture cycle counter (from a free-running timer)
    mov x, ~null         ; placeholder — actual timestamp from DMA or side-set
    
    ; Wait half a bit time to sample mid-bit (208μs at 2400 baud)
    ; At 125MHz system clock: 208μs × 125 = 26,042 cycles
    set y, 7         [25] ; wait ~208μs (tuned with autopull divisor)
    
    ; Sample 8 data bits, LSB first
bitloop:
    in pins, 1       [51] ; sample bit, wait full bit time (~417μs)
    jmp y-- bitloop
    
    ; Wait for stop bit (half bit time)
    nop              [25]
    
    ; Push byte to FIFO
    push noblock
.wrap
```

> **Note**: The actual PIO program will be refined during Phase 2. The clock
> divisor handles the 2400 baud timing. Start-bit timestamp can be captured
> via DMA to a free-running timer, or by using a second SM as a cycle counter.
> The Saleae will verify timing accuracy.

### SM1: eBUS TX

```
; eBUS PIO TX — 2400 baud, 8N1
;
; Pulls bytes from FIFO and transmits with start + stop bits.
; Only used when actively sending (Phase 5).

.program ebus_tx
.wrap_target
    pull block            ; wait for data from core
    set pins, 0      [51] ; start bit (low), wait one bit time
    set y, 7
bitloop:
    out pins, 1      [51] ; shift out data bit, LSB first
    jmp y-- bitloop
    set pins, 1      [51] ; stop bit (high)
.wrap
```

### Clock Divisor

System clock 125MHz ÷ 2400 baud = 52,083.3 cycles per bit.
PIO divisor: `125_000_000 / 2400 = 52083.33` → set `div_int = 52083, div_frac = 85`.
Each PIO instruction then takes exactly one bit time.

## Rust Crate Architecture

### `ebus-core` — no_std protocol library

Ported from yuhu-ebus's Core layer. Testable on desktop, no hardware dependency.

```
ebus-core/
├── src/
│   ├── lib.rs           — public API
│   ├── symbols.rs       — protocol constants (SYN, ACK, NAK, etc.)
│   ├── crc.rs           — CRC-8 table + calc_crc()
│   ├── sequence.rs      — byte buffer with stuffing/unstuffing
│   ├── telegram.rs      — telegram parser (master + slave)
│   ├── address.rs       — is_master(), is_slave(), master_of(), slave_of()
│   ├── framer.rs        — SYN-delimited byte stream → complete telegrams
│   └── arbitration.rs   — request bus FSM (observe/first/retry/second)
├── tests/
│   ├── test_crc.rs
│   ├── test_sequence.rs
│   ├── test_telegram.rs
│   ├── test_framer.rs
│   └── test_arbitration.rs
└── Cargo.toml           — no_std, no dependencies
```

**Key design decisions:**
- `Framer` is the new thing — not in yuhu-ebus. Stateful byte-at-a-time
  consumer that accumulates between SYNs and emits complete `RawTelegram`
  structs. This is the primary interface for passive listening.
- `Arbitration` maps to yuhu-ebus's `Request` FSM (4 states, 12 results).
- `Telegram` parser maps to yuhu-ebus's `Telegram::createMaster/createSlave`.
- Handler FSM (15 states) is NOT ported initially — it's only needed for
  reactive mode (responding to messages addressed to us). We're passive.

### `pico-ebus` — firmware

```
pico-ebus/
├── src/
│   ├── main.rs          — embassy entry, spawns tasks
│   ├── pio_uart.rs      — PIO program setup (RX + TX state machines)
│   ├── ebus_task.rs     — core 0: reads PIO FIFO, feeds Framer, queues telegrams
│   ├── wifi_task.rs     — core 1: WiFi + MQTT connection management
│   ├── mqtt_task.rs     — publishes queued telegrams, subscribes for commands
│   └── led.rs           — status LED (onboard)
├── ebus_rx.pio          — PIO RX program
├── ebus_tx.pio          — PIO TX program  
├── Cargo.toml
├── build.rs             — PIO assembly
└── memory.x             — linker script
```

**Task architecture:**
```
Core 0                          Core 1
──────                          ──────
┌──────────────┐               ┌──────────────┐
│  ebus_task   │               │  wifi_task   │
│              │               │              │
│  PIO RX FIFO │               │  CYW43 WiFi  │
│      ↓       │               │  connect/    │
│  Framer      │               │  reconnect   │
│      ↓       │               └──────────────┘
│  Channel ────┼──────────────→┌──────────────┐
│              │  (telegram    │  mqtt_task   │
│  PIO TX ←────┼───────────────│              │
│              │  (send cmd)   │  Publish raw │
└──────────────┘               │  Subscribe   │
                               └──────────────┘
```

## MQTT Topics

| Topic | Direction | Payload |
|-------|-----------|---------|
| `ebus/telegram` | Pico → pi5data | Raw telegram bytes (hex string or binary) |
| `ebus/telegram/decoded` | Pico → pi5data | JSON: `{"src":"10","dst":"50","cmd":"b509","data":"...","crc_ok":true}` |
| `ebus/error` | Pico → pi5data | Protocol errors, CRC failures |
| `ebus/send` | pi5data → Pico | Telegram to send (hex bytes) |
| `ebus/status` | Pico → pi5data | Heartbeat, uptime, bus stats |

## Phases

### Phase 1: `ebus-core` Rust crate (no hardware needed)

**Goal**: Complete, tested eBUS protocol library.

1. Port CRC table + `calc_crc()` from `yuhu-ebus/src/Ebus/Utils/Common.cpp`
2. Port `Sequence` (byte stuffing/unstuffing) from `yuhu-ebus/src/Ebus/Core/Sequence.cpp`
3. Port address validation from `Common.cpp` (`is_master`, `is_slave`, `master_of`, `slave_of`)
4. Port `Telegram` parser from `yuhu-ebus/src/Ebus/Core/Telegram.cpp`
5. Write `Framer` — new component, byte-at-a-time SYN-delimited stream parser
6. Port `Request` arbitration FSM from `yuhu-ebus/src/Ebus/Core/Request.cpp`
7. Test against yuhu-ebus's test vectors from `yuhu-ebus/tests/Core/test_telegram.cpp`:
   ```
   "ff52b509030d0600430003b0fba901d000"  — passive master-slave
   "1000b5050427002400d900"              — passive master-master
   ```

**Deliverable**: `cargo test` passes on desktop. Zero hardware dependency.

### Phase 2: PIO UART (hardware, no eBUS yet)

**Goal**: Working PIO UART at 2400 baud with start-bit timestamps.

1. Set up Embassy Pico W project (`embassy-rp`, `cyw43`)
2. Write PIO RX program — receive bytes at 2400/8N1
3. Write PIO TX program — transmit bytes at 2400/8N1
4. Test with **loopback** — connect GP4 (TX) to GP5 (RX) with a wire
5. Verify with Saleae:
   - Bit timing accuracy (expect 416.67μs ± <1μs)
   - Start-bit timestamp accuracy
   - Correct byte values
6. Test at sustained throughput — eBUS peak is ~240 bytes/sec (one SYN +
   max telegram every ~30ms), verify no FIFO overruns

**Deliverable**: PIO UART sending and receiving bytes correctly at 2400 baud.
Saleae captures proving timing.

### Phase 3: Passive eBUS listener (first connection to real bus)

**Goal**: Read all eBUS traffic, decode telegrams, output via USB serial.

1. Connect xyzroe adapter to eBUS (2 wires to Vaillant)
2. Connect xyzroe X2 header to Pico W (4 wires)
3. Power Pico from emonhp USB (or any USB supply for bench testing)
4. Firmware: PIO RX → `Framer` → print decoded telegrams on USB serial
5. Verify against ebusd running on john30's v5 adapter:
   - Same telegrams seen?
   - Same byte sequences?
   - CRC validation matches?
6. Use PicoScope to check analogue signal quality if any issues
7. Use Saleae to verify PIO timing against real eBUS traffic

**Deliverable**: USB serial output showing all eBUS telegrams with correct
decoding. Matches ebusd output.

### Phase 4: WiFi + MQTT

**Goal**: Publish raw telegrams to MQTT on pi5data.

1. Add CYW43 WiFi connection (Core 1)
2. Add MQTT client — connect to pi5data Mosquitto (`emonpi`/`emonpimqtt2016`)
3. Publish each decoded telegram to `ebus/telegram`
4. Add reconnection logic (WiFi drops, MQTT disconnects)
5. Add heartbeat/status topic
6. Verify: Telegraf/InfluxDB on pi5data receives all telegrams
7. Compare against ebusd MQTT output for completeness

**Deliverable**: All eBUS traffic flowing into InfluxDB via MQTT. No ebusd
required.

### Phase 5: Active sending (optional, later)

**Goal**: Send commands to eBUS when requested via MQTT.

1. Enable PIO TX
2. Implement arbitration FSM (from `ebus-core::arbitration`)
3. Subscribe to `ebus/send` MQTT topic
4. On command: wait for SYN, arbitrate, send telegram, report result
5. Verify with Saleae:
   - Arbitration timing within 4300–4456μs window
   - Echo check (read back sent byte, compare)
   - Correct ACK/NAK handling
6. Test with a safe read command first (e.g., `read -c 700 DisplayedOutsideTemp`)
7. Test write command (`write -c 700 HwcSFMode load` for DHW boost)

**Deliverable**: Can send arbitrary eBUS commands via MQTT.

## Verification Matrix

| What | Tool | Expected |
|------|------|----------|
| PIO bit timing | Saleae | 416.67μs ± 1μs per bit |
| PIO start-bit timestamp | Saleae | < 1μs accuracy |
| Arbitration window | Saleae | Write occurs at 4300–4456μs after SYN start bit |
| Bus signal quality | PicoScope | Clean 10V/20V transitions, < 50μs edges |
| Comparator output | PicoScope | Clean 0V/3.3V, matches bus transitions |
| Telegram decoding | ebusd comparison | Byte-for-byte match |
| MQTT completeness | InfluxDB query | No missing telegrams vs ebusd |
| WiFi stability | Uptime counter | No drops over 24h |

## Your Vaillant Bus Participants

| Device | Master Address | Slave Address | Role |
|--------|---------------|---------------|------|
| VRC 700 | 0x10 | 0x15 | Controller |
| HMU (Arotherm Plus) | 0x03 | 0x08 | Outdoor unit |
| VWZ AI | 0x05 | 0x0A | Indoor unit |
| Pico W (us) | 0xFF (passive) | — | Listener only (Phase 1-4) |

For Phase 5 (active sending), we'd claim master address 0x31 or similar
(unused priority class, won't conflict).

## Dependencies

```toml
# ebus-core/Cargo.toml
[package]
name = "ebus-core"
edition = "2021"

[features]
default = []
std = []       # enable for desktop testing
defmt = ["dep:defmt"]  # enable for embedded logging

[dependencies]
defmt = { version = "0.3", optional = true }

# pico-ebus/Cargo.toml
[package]
name = "pico-ebus"
edition = "2021"

[dependencies]
ebus-core = { path = "../ebus-core" }
embassy-executor = { version = "0.7", features = ["arch-cortex-m", "executor-thread"] }
embassy-rp = { version = "0.4", features = ["time-driver"] }
embassy-time = "0.4"
embassy-sync = "0.6"
embassy-net = { version = "0.6", features = ["tcp", "dns", "dhcpv4"] }
cyw43 = { version = "0.3", features = ["firmware-logs"] }
cyw43-pio = "0.3"
rust-mqtt = "0.3"
defmt = "0.3"
defmt-rtt = "0.4"
cortex-m-rt = "0.7"
panic-probe = { version = "0.3", features = ["print-defmt"] }
pio = "0.2"
pio-proc = "0.2"
```

## Files to Study

| Source | What to port | Lines |
|--------|-------------|-------|
| `yuhu-ebus/src/Ebus/Utils/Common.cpp` | CRC, address validation | 155 |
| `yuhu-ebus/src/Ebus/Core/Sequence.cpp` | Byte stuffing | 170 |
| `yuhu-ebus/src/Ebus/Core/Telegram.cpp` | Telegram parsing | 462 |
| `yuhu-ebus/src/Ebus/Core/Request.cpp` | Arbitration FSM | 219 |
| `yuhu-ebus/src/Ebus/Core/Handler.cpp` | Full protocol FSM (defer) | 735 |
| `yuhu-ebus/tests/Core/test_telegram.cpp` | Test vectors | — |
| `esp-arduino-ebus/src/Arbitration.cpp` | Alternative arbitration ref | 161 |
| `ttyebus/ttyebusm.c` | Why Linux UART is too slow | 991 |
| `ebusd/src/lib/ebus/protocol.h` | Timing constants | — |
