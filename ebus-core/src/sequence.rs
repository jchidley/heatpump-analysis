//! Byte stuffing (extend) and unstuffing (reduce) for eBUS sequences.
//!
//! On the wire, reserved bytes are escaped:
//! - `0xAA` (SYN) → `[0xA9, 0x01]`
//! - `0xA9` (ESC) → `[0xA9, 0x00]`
//!
//! CRC is always computed over the *extended* (wire-format) bytes.

use crate::crc;
use crate::symbols::{ESC, ESC_ESC, ESC_SYN, SYN};

/// Maximum wire-format telegram size: header(5) + data(16) + CRC(1) + ACK(1),
/// worst-case doubled by escaping, plus slave part similarly.
const MAX_EXTENDED: usize = 128;

/// Extend (byte-stuff) a reduced sequence into wire format.
///
/// Writes into `out` and returns the number of bytes written.
/// Returns `None` if the output buffer is too small.
pub fn extend(reduced: &[u8], out: &mut [u8]) -> Option<usize> {
    let mut j = 0;
    for &b in reduced {
        match b {
            SYN => {
                if j + 2 > out.len() {
                    return None;
                }
                out[j] = ESC;
                out[j + 1] = ESC_SYN;
                j += 2;
            }
            ESC => {
                if j + 2 > out.len() {
                    return None;
                }
                out[j] = ESC;
                out[j + 1] = ESC_ESC;
                j += 2;
            }
            _ => {
                if j >= out.len() {
                    return None;
                }
                out[j] = b;
                j += 1;
            }
        }
    }
    Some(j)
}

/// Reduce (un-stuff) an extended wire-format sequence back to raw bytes.
///
/// Writes into `out` and returns the number of bytes written.
/// Returns `None` if the output buffer is too small.
pub fn reduce(extended: &[u8], out: &mut [u8]) -> Option<usize> {
    let mut i = 0;
    let mut j = 0;
    while i < extended.len() {
        if j >= out.len() {
            return None;
        }
        if extended[i] == ESC {
            if i + 1 < extended.len() {
                i += 1;
                match extended[i] {
                    ESC_SYN => out[j] = SYN,
                    ESC_ESC => out[j] = ESC,
                    other => {
                        // Invalid escape sequence — preserve both bytes
                        out[j] = ESC;
                        j += 1;
                        if j >= out.len() {
                            return None;
                        }
                        out[j] = other;
                    }
                }
            } else {
                // Dangling escape at end
                out[j] = ESC;
            }
        } else {
            out[j] = extended[i];
        }
        i += 1;
        j += 1;
    }
    Some(j)
}

/// Extend into a stack-allocated buffer and return it as a fixed-size array.
pub fn extend_vec(reduced: &[u8]) -> ([u8; MAX_EXTENDED], usize) {
    let mut buf = [0u8; MAX_EXTENDED];
    let len = extend(reduced, &mut buf).unwrap_or(0);
    (buf, len)
}

/// Reduce into a stack-allocated buffer and return it.
pub fn reduce_vec(extended: &[u8]) -> ([u8; MAX_EXTENDED], usize) {
    let mut buf = [0u8; MAX_EXTENDED];
    let len = reduce(extended, &mut buf).unwrap_or(0);
    (buf, len)
}

/// Calculate CRC-8 over data, extending (byte-stuffing) first as per eBUS spec.
pub fn crc_of_reduced(reduced: &[u8]) -> u8 {
    let (ext, len) = extend_vec(reduced);
    crc::crc_bytes(&ext[..len])
}

/// Calculate CRC-8 over wire-format (already extended) data.
pub fn crc_of_extended(extended: &[u8]) -> u8 {
    crc::crc_bytes(extended)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn extend_reduce_round_trips() {
        let cases: &[(&str, &str)] = &[
            ("", ""),
            ("010203", "010203"),
            ("01aa03", "01a90103"),
            ("01a903", "01a90003"),
            ("aa01a9", "a90101a900"),
            ("aa0102", "a9010102"),
            ("0102aa", "0102a901"),
            ("a90102", "a9000102"),
            ("0102a9", "0102a900"),
            ("aaaa", "a901a901"),
            ("a9a9", "a900a900"),
            ("aaa9aa", "a901a900a901"),
        ];

        for &(reduced_hex, extended_hex) in cases {
            let reduced = hex(reduced_hex);
            let extended = hex(extended_hex);

            // Test extend
            let mut out = [0u8; 64];
            let len = extend(&reduced, &mut out).unwrap();
            assert_eq!(
                &out[..len], &extended[..],
                "extend failed for {reduced_hex}"
            );

            // Test reduce
            let len = reduce(&extended, &mut out).unwrap();
            assert_eq!(
                &out[..len], &reduced[..],
                "reduce failed for {extended_hex}"
            );
        }
    }

    #[test]
    fn crc_on_reduced_sequence() {
        // 10 08 b5 11 02 03 00 -> CRC 0x1e (no escaping needed)
        let data = hex("1008b511020300");
        assert_eq!(crc_of_reduced(&data), 0x1e);
    }

    #[test]
    fn crc_with_byte_stuffing() {
        // reduced: 01 aa 03 -> extended: 01 a9 01 03
        // CRC must be calculated on extended bytes
        let data = hex("01aa03");
        assert_eq!(crc_of_reduced(&data), 0x22);
    }
}
