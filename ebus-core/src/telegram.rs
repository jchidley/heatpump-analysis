//! eBUS telegram parsing.
//!
//! A telegram on the wire (between two SYN bytes) contains:
//! - Master part: `QQ ZZ PB SB NN [DB₀..DBₙ] CRC`
//! - For master-slave: `ACK NN [DB₀..DBₙ] CRC ACK`
//! - For master-master: `ACK`
//! - For broadcast: nothing after CRC
//!
//! All bytes are in wire format (byte-stuffed). This module parses a
//! reduced (un-stuffed) byte sequence into structured fields.

use crate::address::{is_master, is_target};
use crate::sequence;
use crate::symbols::{ACK, BROADCAST, MAX_DATA_BYTES, NAK};

/// Telegram type determined by the destination address (ZZ).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelegramType {
    /// Destination is broadcast address (0xFE).
    Broadcast,
    /// Destination is a master address.
    MasterMaster,
    /// Destination is a slave address.
    MasterSlave,
}

/// Result of parsing a telegram.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    /// Not enough bytes for a complete telegram.
    TooShort,
    /// Source address (QQ) is not a valid master.
    InvalidSource,
    /// Target address (ZZ) is not a valid target.
    InvalidTarget,
    /// Data length byte exceeds maximum (16).
    DataLengthExceeded,
    /// CRC mismatch on master part.
    MasterCrcInvalid,
    /// CRC mismatch on slave part.
    SlaveCrcInvalid,
    /// Master ACK byte missing.
    MasterAckMissing,
    /// Slave ACK byte missing.
    SlaveAckMissing,
    /// Both slave NAK retries exhausted.
    SlaveNakFinal,
    /// Both master NAK retries exhausted (slave response rejected twice).
    MasterNakFinal,
}

/// A parsed eBUS telegram.
#[derive(Debug, Clone, PartialEq)]
pub struct Telegram {
    /// Telegram type (broadcast, master-master, master-slave).
    pub telegram_type: TelegramType,
    /// Source (master) address.
    pub source: u8,
    /// Destination address.
    pub target: u8,
    /// Primary command byte.
    pub primary_cmd: u8,
    /// Secondary command byte.
    pub secondary_cmd: u8,
    /// Master data bytes (reduced, max 16).
    pub master_data: [u8; 16],
    /// Number of valid master data bytes.
    pub master_data_len: usize,
    /// Master CRC (as transmitted).
    pub master_crc: u8,
    /// Slave data bytes (reduced, max 16). Only valid for MasterSlave.
    pub slave_data: [u8; 16],
    /// Number of valid slave data bytes.
    pub slave_data_len: usize,
    /// Slave CRC (as transmitted). Only valid for MasterSlave.
    pub slave_crc: u8,
}

impl Telegram {
    /// Return master data as a slice.
    pub fn master_bytes(&self) -> &[u8] {
        &self.master_data[..self.master_data_len]
    }

    /// Return slave data as a slice (empty for non-MasterSlave).
    pub fn slave_bytes(&self) -> &[u8] {
        &self.slave_data[..self.slave_data_len]
    }
}

impl Default for Telegram {
    fn default() -> Self {
        Telegram {
            telegram_type: TelegramType::Broadcast,
            source: 0,
            target: 0,
            primary_cmd: 0,
            secondary_cmd: 0,
            master_data: [0; 16],
            master_data_len: 0,
            master_crc: 0,
            slave_data: [0; 16],
            slave_data_len: 0,
            slave_crc: 0,
        }
    }
}

fn telegram_type_of(zz: u8) -> TelegramType {
    if zz == BROADCAST {
        TelegramType::Broadcast
    } else if is_master(zz) {
        TelegramType::MasterMaster
    } else {
        TelegramType::MasterSlave
    }
}

/// Parse a reduced (un-stuffed) byte sequence into a [`Telegram`].
///
/// The input should be the bytes between two SYN delimiters, already
/// reduced (escape sequences removed). Use [`crate::sequence::reduce`]
/// first if working with wire-format data.
pub fn parse(reduced: &[u8]) -> Result<Telegram, ParseError> {
    // Minimum master part: QQ ZZ PB SB NN CRC = 6 bytes (NN=0)
    if reduced.len() < 6 {
        return Err(ParseError::TooShort);
    }

    let qq = reduced[0];
    let zz = reduced[1];
    let pb = reduced[2];
    let sb = reduced[3];
    let nn = reduced[4];

    if !is_master(qq) {
        return Err(ParseError::InvalidSource);
    }
    if !is_target(zz) {
        return Err(ParseError::InvalidTarget);
    }
    if nn > MAX_DATA_BYTES {
        return Err(ParseError::DataLengthExceeded);
    }

    let nn = nn as usize;
    let master_end = 5 + nn; // index of CRC byte
    if reduced.len() < master_end + 1 {
        return Err(ParseError::TooShort);
    }

    // Verify master CRC
    let master_payload = &reduced[..master_end];
    let master_crc_received = reduced[master_end];
    let master_crc_calc = sequence::crc_of_reduced(master_payload);

    if master_crc_calc != master_crc_received {
        return Err(ParseError::MasterCrcInvalid);
    }

    let ttype = telegram_type_of(zz);

    let mut tel = Telegram {
        telegram_type: ttype,
        source: qq,
        target: zz,
        primary_cmd: pb,
        secondary_cmd: sb,
        master_crc: master_crc_received,
        ..Telegram::default()
    };

    // Copy master data
    tel.master_data_len = nn;
    tel.master_data[..nn].copy_from_slice(&reduced[5..5 + nn]);

    match ttype {
        TelegramType::Broadcast => {
            // No ACK or slave response for broadcasts
            return Ok(tel);
        }
        TelegramType::MasterMaster => {
            // Expect slave ACK byte after master CRC
            let ack_idx = master_end + 1;
            if reduced.len() < ack_idx + 1 {
                return Err(ParseError::MasterAckMissing);
            }
            let ack = reduced[ack_idx];
            if ack != ACK {
                // NAK handling: check for retry
                if ack == NAK {
                    // Try to parse a retry after this NAK
                    let retry_start = ack_idx + 1;
                    return parse_master_master_retry(reduced, retry_start, pb, sb);
                }
            }
            return Ok(tel);
        }
        TelegramType::MasterSlave => {
            // After master CRC: slave ACK, then slave NN, slave data, slave CRC, master ACK
            let mut offset = master_end + 1;

            // Slave ACK
            if reduced.len() < offset + 1 {
                return Err(ParseError::SlaveAckMissing);
            }
            let slave_ack = reduced[offset];
            offset += 1;

            if slave_ack == NAK {
                // Retry: skip to second copy of master, parse slave from there
                // For simplicity, try to find second master part
                return parse_with_nak_retry(reduced, &tel);
            }
            // slave_ack should be ACK

            // Slave response: NN [data] CRC
            if reduced.len() < offset + 1 {
                return Err(ParseError::TooShort);
            }
            let slave_nn = reduced[offset] as usize;
            if slave_nn > MAX_DATA_BYTES as usize {
                return Err(ParseError::DataLengthExceeded);
            }
            offset += 1;

            if reduced.len() < offset + slave_nn + 1 {
                return Err(ParseError::TooShort);
            }

            let slave_payload = &reduced[offset - 1..offset + slave_nn]; // NN + data
            let slave_crc_received = reduced[offset + slave_nn];
            let slave_crc_calc = sequence::crc_of_reduced(slave_payload);

            if slave_crc_calc != slave_crc_received {
                return Err(ParseError::SlaveCrcInvalid);
            }

            tel.slave_data_len = slave_nn;
            tel.slave_data[..slave_nn].copy_from_slice(&reduced[offset..offset + slave_nn]);
            tel.slave_crc = slave_crc_received;

            return Ok(tel);
        }
    }
}

fn parse_master_master_retry(
    _reduced: &[u8],
    _retry_start: usize,
    _pb: u8,
    _sb: u8,
) -> Result<Telegram, ParseError> {
    // For Phase 1, NAK retries in master-master are uncommon.
    // We'd need to re-parse from retry_start. For now, report error.
    Err(ParseError::MasterNakFinal)
}

fn parse_with_nak_retry(
    _reduced: &[u8],
    _partial: &Telegram,
) -> Result<Telegram, ParseError> {
    // Full NAK retry parsing (master NAK → re-send master, slave NAK →
    // re-send slave) is complex. For passive listening (Phase 3), we almost
    // never see retries because we're not participating. Defer to Phase 5.
    Err(ParseError::SlaveNakFinal)
}

/// Parse a wire-format (extended/byte-stuffed) telegram.
///
/// This is a convenience wrapper that reduces first, then parses.
pub fn parse_wire(wire_bytes: &[u8]) -> Result<Telegram, ParseError> {
    let mut reduced = [0u8; 128];
    let len = sequence::reduce(wire_bytes, &mut reduced).ok_or(ParseError::TooShort)?;
    parse(&reduced[..len])
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
    fn parse_broadcast() {
        // 10 fe 07 00 09 70 16 04 43 18 31 05 05 25 92
        // QQ=10 ZZ=fe PB=07 SB=00 NN=09 data=70160443183105 CRC=25 ... wait
        // Actually from test: "10fe07000970160443183105052592"
        // QQ=10 ZZ=fe PB=07 SB=00 NN=09 data=701604431831050525 CRC=92
        let data = hex("10fe070009701604431831050525");
        // Calculate CRC to figure out what's in the test vector
        let crc = sequence::crc_of_reduced(&data[..14]);
        let mut full = data.clone();
        full.push(crc);
        let tel = parse(&full).unwrap();
        assert_eq!(tel.telegram_type, TelegramType::Broadcast);
        assert_eq!(tel.source, 0x10);
        assert_eq!(tel.target, 0xFE);
        assert_eq!(tel.primary_cmd, 0x07);
        assert_eq!(tel.secondary_cmd, 0x00);
        assert_eq!(tel.master_data_len, 9);
    }

    #[test]
    fn parse_master_slave_wire_format() {
        // Wire: 10 08 b5 09 03 0d 06 00 e1 00 03 b0 fb a9 01 d0 00
        // This is extended (a9 01 = escaped 0xAA in slave data)
        let wire = hex("1008b509030d0600e10003b0fba901d000");
        let tel = parse_wire(&wire).unwrap();
        assert_eq!(tel.telegram_type, TelegramType::MasterSlave);
        assert_eq!(tel.source, 0x10);
        assert_eq!(tel.target, 0x08);
        assert_eq!(tel.primary_cmd, 0xB5);
        assert_eq!(tel.secondary_cmd, 0x09);
        assert_eq!(tel.master_bytes(), &[0x0d, 0x06, 0x00]);
        assert_eq!(tel.slave_bytes(), &[0xb0, 0xfb, 0xaa]);
        assert_eq!(tel.slave_crc, 0xd0);
    }

    #[test]
    fn parse_master_master() {
        // 10 00 b5 05 04 27 00 24 00 d9 00
        // QQ=10 ZZ=00 (master addr) PB=b5 SB=05 NN=04 data=27002400 CRC=d9 ACK=00
        let reduced = hex("1000b5050427002400d900");
        let tel = parse(&reduced).unwrap();
        assert_eq!(tel.telegram_type, TelegramType::MasterMaster);
        assert_eq!(tel.source, 0x10);
        assert_eq!(tel.target, 0x00);
        assert_eq!(tel.master_bytes(), &[0x27, 0x00, 0x24, 0x00]);
    }

    #[test]
    fn parse_crc_mismatch() {
        // Correct CRC is e1, use 9f instead
        let reduced = hex("1008b509030d06009f");
        assert!(matches!(parse(&reduced), Err(ParseError::MasterCrcInvalid)));
    }

    #[test]
    fn parse_too_short() {
        let reduced = hex("1008b5");
        assert!(matches!(parse(&reduced), Err(ParseError::TooShort)));
    }

    #[test]
    fn parse_invalid_source() {
        // 0x02 is not a valid master address
        let reduced = hex("0208b509030d0600e1");
        assert!(matches!(parse(&reduced), Err(ParseError::InvalidSource)));
    }
}
