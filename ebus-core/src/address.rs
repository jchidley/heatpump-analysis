//! eBUS address classification.
//!
//! Master addresses are formed from a priority class (lower nibble) and a
//! sub-address (upper nibble). Each nibble must satisfy `((n+1) & n) == 0`,
//! giving 5 valid values per nibble (0x0, 0x1, 0x3, 0x7, 0xF) and 25 valid
//! master addresses total. Slave address = master address + 5.

use crate::symbols::{BROADCAST, ESC, SYN};

/// Returns `true` if `byte` is a valid eBUS master address.
pub fn is_master(byte: u8) -> bool {
    let hi = (byte >> 4) & 0x0F;
    let lo = byte & 0x0F;
    ((hi.wrapping_add(1)) & hi) == 0 && ((lo.wrapping_add(1)) & lo) == 0
}

/// Returns `true` if `byte` is a valid slave address (not master, not SYN,
/// not ESC, not broadcast).
pub fn is_slave(byte: u8) -> bool {
    !is_master(byte) && byte != SYN && byte != ESC && byte != BROADCAST
}

/// Returns `true` if `byte` is a valid telegram target (anything except SYN
/// and ESC).
pub fn is_target(byte: u8) -> bool {
    byte != SYN && byte != ESC
}

/// Given a slave address, return the corresponding master address.
/// If the input is already a master address, return it unchanged.
pub fn master_of(byte: u8) -> u8 {
    if is_master(byte.wrapping_sub(5)) {
        byte.wrapping_sub(5)
    } else {
        byte
    }
}

/// Given a master address, return the corresponding slave address.
/// If the input is already a slave address, return it unchanged.
pub fn slave_of(byte: u8) -> u8 {
    if is_master(byte) {
        byte.wrapping_add(5)
    } else {
        byte
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_masters() {
        assert!(is_master(0x00));
        assert!(is_master(0x01));
        assert!(is_master(0x03));
        assert!(is_master(0x10));
        assert!(is_master(0x37));
        assert!(is_master(0xFF));
    }

    #[test]
    fn invalid_masters() {
        assert!(!is_master(0x02));
        assert!(!is_master(0x04));
        assert!(!is_master(0x05));
        assert!(!is_master(SYN)); // 0xAA
    }

    #[test]
    fn slave_classification() {
        assert!(is_slave(0x05));
        assert!(!is_slave(0x00)); // master
        assert!(!is_slave(SYN));
    }

    #[test]
    fn master_slave_round_trip() {
        assert_eq!(master_of(0x05), 0x00);
        assert_eq!(master_of(0x00), 0x00); // already master
        assert_eq!(slave_of(0x00), 0x05);
        assert_eq!(slave_of(0x05), 0x05); // already slave
    }

    #[test]
    fn exactly_25_master_addresses() {
        let count = (0u16..=255).filter(|&b| is_master(b as u8)).count();
        assert_eq!(count, 25);
    }
}
