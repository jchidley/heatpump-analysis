# eBUS Specification Sources

Downloaded 30 March 2026.

## Open eBUS standard (published by eBUS Interest Group)

The eBUS standard was maintained 2001–2009 by the eBUS Interest Group led by Prof. Lawrenz at Fachhochschule Braunschweig/Wolfenbüttel. These are the official, publicly available specifications.

| File | Document | Version | Date | Original URL | Archive URL |
|---|---|---|---|---|---|
| `ebus_spec_physical_datalink_v1.3.1.pdf` | eBUS Specification – Physical & Data Link Layer (OSI 1 & 2) | v1.3.1 | March 2007 | http://ebus-wiki.org/lib/exe/fetch.php/ebus/spec_prot_12_v1_3_1_e.pdf | https://web.archive.org/web/20191122102601/https://ebus-wiki.org/lib/exe/fetch.php/ebus/spec_prot_12_v1_3_1_e.pdf |
| `ebus_spec_application_v1.6.1.pdf` | eBUS Specification – Application Layer (OSI 7) | v1.6.1 | March 2007 | http://ebus-wiki.org/lib/exe/fetch.php/ebus/spec_prot_7_v1_6_1_e.pdf | https://web.archive.org/web/20191122101418/http://ebus-wiki.org/lib/exe/fetch.php/ebus/spec_prot_7_v1_6_1_e.pdf |

## Vaillant-specific extensions (unofficial/community)

Vaillant implements the open eBUS standard but adds their own unpublished extensions for device-specific commands and registers. These documents are not from Vaillant — they are community reverse-engineered from observing bus traffic and cross-referencing Vaillant service manuals.

| File | Document | Source | Notes |
|---|---|---|---|
| `vaillant_ebus_extensions.pdf` | eBUS Specification Application Layer – OSI 7 Vaillant specific extensions | https://www.pittnerovi.com/jiri/hobby/electronics/ebus/Vaillant_ebus.pdf | Community reverse-engineered. References VRS620 manual terminology. |

| `vaillant_ebus_v0.5.0.pdf` | Vaillant eBUS v0.5.0 (2014-05) — master/slave addresses, proprietary command set, timer program encoding | https://ebus-wiki.org/lib/exe/fetch.php/ebus/vaillant_ebus_v0.5.0.pdf (dead) | Downloaded from Wayback Machine: https://web.archive.org/web/2023/https://ebus-wiki.org/lib/exe/fetch.php/ebus/vaillant_ebus_v0.5.0.pdf |

## Related projects

| Project | URL | Licence | Role |
|---|---|---|---|
| **ebusd** (daemon) | https://github.com/john30/ebusd | GPL-3.0 | Encodes/decodes messages per the CSV register definitions, sends them to the ESP32 adapter over TCP. We have the source code for this. |
| **ebusd-configuration** | https://github.com/john30/ebusd-configuration | CC BY-SA 4.0 | Community-maintained CSV register definitions for specific devices (reverse-engineered from bus traffic observation) |
| **ebusd-esp32** (firmware) | https://github.com/john30/ebusd-esp32 | Proprietary (no source, binaries + changelog only) | Firmware on the ESP32 adapter that bridges TCP ↔ eBUS physical layer. We do not have the source code — only pre-built binaries and a changelog. |

Note: the open eBUS spec fully documents the physical interface (voltage levels, UART parameters) and protocol (framing, CRC, arbitration). A custom eBUS adapter could be built from the spec without the closed-source ESP32 firmware — the adapter is a convenience, not a dependency.

### Alternative: danielkucera fully open-source adapter

Daniel Kucera maintains a fully open-source eBUS adapter — hardware, firmware, and software — that is ebusd-compatible.

| Component | Repository | Licence | Notes |
|---|---|---|---|
| **Hardware** (KiCad) | https://github.com/danielkucera/ebus-adapter | Open source | Bus-powered (no external PSU), no opto-isolation needed. ESP32-C3 based. |
| **Firmware** (ESP-Arduino) | https://github.com/danielkucera/esp-arduino-ebus | Open source | TCP socket (ebusd compatible), MQTT, HTTP, Home Assistant autodiscovery. Standalone mode can operate without ebusd. |

Buy assembled: [Elecrow $19 (v6.3)](https://www.elecrow.com/ebus-to-wifi-adapter-module-v5-2.html) or [Lectronz (Slovakia)](https://www.lectronz.com/stores/danman-eu).

This is the natural starting point for a custom Pico 2 W eBUS adapter — the KiCad schematics provide the analog front-end design and the firmware source shows the eBUS protocol timing on a RISC-V ESP32-C3 (same core family as the RP2350 in the Pico 2 W).

The ESP32-C3 is RISC-V and has first-class Rust support via Embassy + esp-hal. The danielkucera Arduino C++ firmware could be replaced with a Rust implementation. His firmware already supports a standalone "INTERNAL" mode that operates as an autonomous eBUS participant without ebusd — the same approach would work in Rust, eliminating the ebusd daemon entirely.

## All known eBUS implementations

### Adapters (hardware + firmware)

| Project | Language | Licence | Hardware | Notes |
|---|---|---|---|---|
| [john30/ebusd-esp32](https://github.com/john30/ebusd-esp32) | Closed source | Proprietary | ESP32-C3/C6 (Shield v5, C6) | **What we have.** Binaries only. Includes paid `micro-ebusd` option (token required) that runs ebusd on-chip. |
| [john30/ebusd-esp](https://github.com/john30/ebusd-esp) | C (Arduino) | Open source | ESP8266/ESP32 (Adapter v2/v3) | **Discontinued.** Predecessor to ebusd-esp32. Source available. |
| [danielkucera/esp-arduino-ebus](https://github.com/danielkucera/esp-arduino-ebus) | C++ (Arduino) | Open source | ESP32-C3 ([danielkucera/ebus-adapter](https://github.com/danielkucera/ebus-adapter), KiCad) | **Fully open source end-to-end.** TCP (ebusd compatible), MQTT, HTTP, HA autodiscovery. Standalone mode without ebusd. Bus-powered. |
| [eBUS/adapter](https://github.com/eBUS/adapter) | — | Open source | PCB design (archived) | Original open-source adapter hardware. Schematics on [OSHWLab](https://oshwlab.com/cresh/eBUS-adapter-2.1). |
| [eBUS/ttyebus](https://github.com/eBUS/ttyebus) | C | Open source | Raspberry Pi kernel module | Real-time Linux kernel module for PL011 UART. Direct GPIO, no ESP needed. |

### Daemons / protocol libraries

| Project | Language | Licence | Notes |
|---|---|---|---|
| [john30/ebusd](https://github.com/john30/ebusd) | C++ | GPL-3.0 | **What we use.** The standard daemon. Talks to adapters over TCP/serial, decodes messages via CSV definitions. |
| [john30/ebusd-configuration](https://github.com/john30/ebusd-configuration) | TypeSpec/CSV | CC BY-SA 4.0 | Community-maintained device register definitions. Reverse-engineered. |
| [yuhu-/ebus](https://github.com/yuhu-/ebus) | C++ | Open source | Standalone C++ library with full protocol engine: FSM, arbitration, scheduler, bus health metrics. Platform abstraction for POSIX/FreeRTOS. Could run directly on embedded. |
| [yvesf/ebus](https://github.com/yvesf/ebus) | **Rust** + Racket | Open source | eBUS protocol parser in Rust (`ebus-rust`). Includes XML protocol definitions and InfluxDB integration. Parser only, not a full daemon. |
| [csowada/ebus](https://github.com/csowada/ebus) | Java | Open source | Java eBUS library. Uses nrjavaserial. |

### Comparison (inspected locally, cloned to `~/github/ebus-reference/`)

| Project | Language | Source LOC | Tests | Last commit | Scope | Portable? |
|---|---|---|---|---|---|---|
| danielkucera/esp-arduino-ebus | C++ (Arduino) | ~8,000 | None | 2026-03-17 | Full product: hardware + firmware + WiFi + MQTT + HA + standalone mode. **Uses yuhu-/ebus as protocol engine.** | ESP32-C3 only |
| yuhu-/ebus | C++ | ~8,800 + 3,675 test | 21 files, comprehensive | 2026-03-29 | Protocol library: FSM, arbitration, scheduling, bus health metrics. **The engine inside esp-arduino-ebus.** | POSIX + FreeRTOS |
| yvesf/ebus | Rust + Racket | ~1,150 Rust | None | 2021-12-31 | Parser only: reads bus dumps, no write/arbitration | Desktop |
| csowada/ebus | Java | ~18,500 | JUnit | 2025-10-23 | Full library with serial | JVM only |

### Key observations

- **ebusd is the dominant daemon** but it's a Linux service that talks to an adapter over TCP/serial. There's no fundamental reason the eBUS protocol can't run directly on the microcontroller.
- **danielkucera's firmware already does this** in standalone "INTERNAL" mode on ESP32-C3. It's the most mature open-source complete product — hardware, firmware, and networking. The $19 assembled board from Elecrow is hard to beat.
- **john30's micro-ebusd** also runs on-chip, but requires a paid token and runs on closed-source firmware.
- **yuhu-/ebus** is the most architecturally sound protocol library — clean separation (Core/App/Platform/Models), comprehensive tests (21 files, 3,675 LOC), FreeRTOS support, and actively maintained (last commit yesterday). Best reference for a Rust port of the protocol engine.
- **yvesf/ebus** has a Rust parser but it's stale (2021) and decode-only — no write or bus arbitration.
- **csowada/ebus** is comprehensive but Java/JVM, not embeddable.

### Knowledge chain for Vaillant register definitions

The Vaillant-specific register definitions (timer encoding, SetMode fields, HwcSFMode values, etc.) live in a separate layer from the eBUS protocol:

| Layer | Source | Maintained by |
|---|---|---|
| eBUS protocol (framing, CRC, arbitration) | `yuhu-/ebus` C++ library, `ebusd` daemon | Roland Jax, john30 |
| Vaillant register definitions | [john30/ebusd-configuration](https://github.com/john30/ebusd-configuration) TypeSpec/CSV files | john30 + community |
| Data type encoding (TTM, BCD, etc.) | `ebusd/src/lib/ebus/datatype.cpp` | john30 |
| Vaillant protocol extensions spec | `docs/ebus-specs/vaillant_ebus_v0.5.0.pdf` (section 3.1.3) | Community reverse-engineered |

The TypeSpec source for our VRC 700 is `src/vaillant/15.700.tsp` in ebusd-configuration. Timer fields use type `slot1_3` which maps to 6× TTM bytes. The TTM encoding (0x00–0x8F = times, 0x90 = replacement) is defined in ebusd's C++ source and independently confirmed by the Vaillant eBUS extensions spec.

**Reverse engineering capability:** With two devices on the eBUS (our ebusd adapter + the VRC 700), we can sniff all traffic between the VRC 700 and the HMU/VWZ. The VRC 700 sends SetMode every ~10 seconds. By comparing raw bytes when timers work (e.g. CcTimer: `24 84 90 90 90 90` = `06:00;22:00;-:-;-:-;-:-;-:-`) vs when they don't (Z1Timer: `18 00 90 90 90 90` = `04:00;00:00;-:-;-:-;-:-;-:-`), we can validate encoding assumptions directly from live bus traffic.

### Path to a Rust embedded eBUS implementation

A Rust Embassy implementation on ESP32-C3 or Pico 2 W would:
1. Use **danielkucera's open KiCad hardware** for the analog front-end
2. Port **yuhu-/ebus** protocol engine design (FSM, arbitration, scheduling) to Rust
3. Reference **yvesf/ebus** for Rust eBUS data type parsing patterns
4. Use **Embassy + esp-hal** (ESP32-C3, RISC-V) or **Embassy + embassy-rp** (Pico 2 W, also RISC-V)
5. Eliminate the ebusd daemon entirely — the device would be a standalone eBUS participant with WiFi/MQTT

### Hardware: buy vs build

| Option | Price | Effort | Notes |
|---|---|---|---|
| danielkucera adapter v6.3 (Elecrow) | $19 | Plug and play | Bus-powered, 2 wires, ESP32-C3, fully open source. Hard to beat. |
| john30 Shield v5/C6 (Elecrow/BerryBase) | €25-30 | Plug and play | What we have. Closed-source firmware. |
| Custom Pico 2 W + analog front-end | ~$10 parts | PCB or dead-bug | Use danielkucera's KiCad schematic for the eBUS interface circuit. |
| eBUS Adapter v2 base board + Pico 2 W | ~$15 | Order PCB + assemble | Open-source PCB on OSHWLab. 3.3V UART breakout. |

## Protocol layer summary

| Layer | Status | Spec |
|---|---|---|
| Physical (2-wire, 2400 baud, 9–24V) | **Open standard** | `ebus_spec_physical_datalink_v1.3.1.pdf` |
| Data-link (addressing, framing, CRC, ACK) | **Open standard** | `ebus_spec_physical_datalink_v1.3.1.pdf` |
| Application (generic heating commands) | **Open standard** | `ebus_spec_application_v1.6.1.pdf` |
| Vaillant command/register set (timers, SetMode, HwcSFMode, etc.) | **Proprietary** — reverse-engineered | `vaillant_ebus_extensions.pdf`, ebusd-configuration CSVs |
