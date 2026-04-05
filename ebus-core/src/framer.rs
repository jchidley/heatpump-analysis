//! SYN-delimited byte stream → complete telegrams.
//!
//! The [`Framer`] consumes bytes one at a time (as they arrive from a PIO
//! UART or TCP stream) and emits a raw byte buffer each time a SYN delimiter
//! is seen, representing one complete telegram's wire bytes.
//!
//! The caller can then pass the buffer through [`crate::sequence::reduce`]
//! and [`crate::telegram::parse`] to decode it.

use crate::symbols::SYN;

/// Maximum bytes between two SYN delimiters.
///
/// Worst case: master(5+16) + CRC(1) + ACK(1) + slave(1+16) + CRC(1) + ACK(1)
/// = 42 bytes, doubled by escaping = 84. Round up for NAK retries.
const MAX_FRAME: usize = 128;

/// Accumulates bytes between SYN delimiters.
pub struct Framer {
    buf: [u8; MAX_FRAME],
    len: usize,
}

/// A complete frame of wire bytes captured between two SYN delimiters.
#[derive(Debug, Clone)]
pub struct RawFrame {
    data: [u8; MAX_FRAME],
    len: usize,
}

impl RawFrame {
    /// The wire-format bytes (still byte-stuffed).
    pub fn as_bytes(&self) -> &[u8] {
        &self.data[..self.len]
    }

    /// Number of wire bytes in this frame.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns true if the frame is empty (consecutive SYN bytes).
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Framer {
    /// Create a new framer in the initial state.
    pub const fn new() -> Self {
        Framer {
            buf: [0; MAX_FRAME],
            len: 0,
        }
    }

    /// Feed one byte from the bus.
    ///
    /// Returns `Some(RawFrame)` when a SYN delimiter completes a frame.
    /// Empty frames (consecutive SYNs) are also returned — the caller should
    /// filter them with [`RawFrame::is_empty`].
    pub fn feed(&mut self, byte: u8) -> Option<RawFrame> {
        if byte == SYN {
            if self.len == 0 {
                // Consecutive SYN or first SYN — no data to emit
                return None;
            }
            let frame = RawFrame {
                data: self.buf,
                len: self.len,
            };
            self.len = 0;
            Some(frame)
        } else {
            if self.len < MAX_FRAME {
                self.buf[self.len] = byte;
                self.len += 1;
            }
            // If buffer overflows, silently drop bytes (corrupt frame
            // will fail CRC validation downstream).
            None
        }
    }

    /// Reset the framer, discarding any partial frame.
    pub fn reset(&mut self) {
        self.len = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_framing() {
        let mut f = Framer::new();

        // SYN - no frame yet
        assert!(f.feed(SYN).is_none());

        // Some data bytes
        assert!(f.feed(0x10).is_none());
        assert!(f.feed(0x08).is_none());
        assert!(f.feed(0xB5).is_none());

        // SYN terminates the frame
        let frame = f.feed(SYN).unwrap();
        assert_eq!(frame.as_bytes(), &[0x10, 0x08, 0xB5]);
    }

    #[test]
    fn consecutive_syns_produce_no_frames() {
        let mut f = Framer::new();
        assert!(f.feed(SYN).is_none());
        assert!(f.feed(SYN).is_none());
        assert!(f.feed(SYN).is_none());
    }

    #[test]
    fn full_telegram_framing() {
        let mut f = Framer::new();
        // Feed SYN, then a master-slave telegram, then SYN
        let wire: &[u8] = &[
            SYN, 0x10, 0x08, 0xB5, 0x09, 0x03, 0x0D, 0x06, 0x00, 0xE1, 0x00, 0x03, 0xB0, 0xFB,
            0xA9, 0x01, 0xD0, 0x00, SYN,
        ];
        let mut frames = Vec::new();
        for &b in wire {
            if let Some(frame) = f.feed(b) {
                frames.push(frame);
            }
        }
        assert_eq!(frames.len(), 1);
        assert_eq!(
            frames[0].as_bytes(),
            &[0x10, 0x08, 0xB5, 0x09, 0x03, 0x0D, 0x06, 0x00, 0xE1, 0x00, 0x03, 0xB0, 0xFB, 0xA9, 0x01, 0xD0, 0x00]
        );
    }

    #[test]
    fn overflow_handled_gracefully() {
        let mut f = Framer::new();
        f.feed(SYN);
        // Feed 200 non-SYN bytes (exceeds MAX_FRAME)
        for _ in 0..200 {
            f.feed(0x42); // safe byte, not SYN
        }
        // SYN should still produce a frame (truncated to MAX_FRAME)
        let frame = f.feed(SYN).unwrap();
        assert_eq!(frame.len(), MAX_FRAME);
    }
}
