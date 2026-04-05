//! eBUS protocol symbols and constants.

/// Synchronization byte — delimits telegrams on the bus.
pub const SYN: u8 = 0xAA;

/// Escape byte — begins a two-byte escape sequence.
pub const ESC: u8 = 0xA9;

/// Escaped SYN: wire bytes `[0xA9, 0x01]` represent logical `0xAA`.
pub const ESC_SYN: u8 = 0x01;

/// Escaped ESC: wire bytes `[0xA9, 0x00]` represent logical `0xA9`.
pub const ESC_ESC: u8 = 0x00;

/// Positive acknowledge.
pub const ACK: u8 = 0x00;

/// Negative acknowledge.
pub const NAK: u8 = 0xFF;

/// Broadcast destination address.
pub const BROADCAST: u8 = 0xFE;

/// Maximum data bytes in a single telegram part.
pub const MAX_DATA_BYTES: u8 = 16;
