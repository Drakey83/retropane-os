//! The resident loader's wire protocol (ADR-0012 step U3): how the PC hands a
//! new image to the X51v over any byte stream (USB serial on the device, the
//! emulator's serial port in CI).
//!
//! The device announces itself with [`READY`] lines. The host sends one frame:
//!
//! | Bytes | Field |
//! |---|---|
//! | 4 | magic `RPL1` |
//! | 4 | payload length, little-endian |
//! | 4 | CRC-32 (IEEE) of the payload, little-endian |
//! | n | payload: the raw image, loaded at the images' link address |
//!
//! The device answers [`OK`] or an `RPL ERR` line and, on success, starts the
//! image. Bytes before a magic are skipped, so a terminal's stray input or a
//! half-sent earlier frame can't wedge it.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

#[cfg(all(feature = "std", not(test)))]
extern crate std;

/// Frame start.
pub const MAGIC: [u8; 4] = *b"RPL1";
/// Sent by the device about once a second while it waits.
pub const READY: &str = "RPLOAD1 READY\r\n";
/// Sent once a frame arrived intact, just before the image starts.
pub const OK: &str = "RPL OK\r\n";

/// CRC-32 (IEEE 802.3, reflected, as zlib and PowerShell-free tools compute it).
#[must_use]
pub fn crc32(data: &[u8]) -> u32 {
    crc32_update(!0, data) ^ !0
}

/// Continues a CRC-32 over more data (`state` starts at `!0`; finish with `^ !0`).
#[must_use]
pub fn crc32_update(mut state: u32, data: &[u8]) -> u32 {
    for &b in data {
        state = TABLE[((state ^ u32::from(b)) & 0xFF) as usize] ^ (state >> 8);
    }
    state
}

const TABLE: [u32; 256] = {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
};

/// Why a frame was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadError {
    /// The payload doesn't fit where images are loaded.
    TooBig,
    /// The payload arrived damaged.
    BadCrc,
}

impl LoadError {
    /// The device's reply line.
    #[must_use]
    pub const fn reply(self) -> &'static str {
        match self {
            Self::TooBig => "RPL ERR too big\r\n",
            Self::BadCrc => "RPL ERR bad crc\r\n",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Matching the magic; `n` bytes matched so far.
    Magic(usize),
    /// Reading the 8 header bytes after the magic.
    Header(usize),
    /// Receiving the payload.
    Payload,
}

/// Receives one frame straight into the destination (no staging buffer).
#[derive(Debug)]
pub struct Receiver {
    state: State,
    header: [u8; 8],
    len: usize,
    want_crc: u32,
    got: usize,
}

impl Default for Receiver {
    fn default() -> Self {
        Self::new()
    }
}

impl Receiver {
    /// Waiting for a frame.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: State::Magic(0),
            header: [0; 8],
            len: 0,
            want_crc: 0,
            got: 0,
        }
    }

    /// Whether a frame is partly received (for the screen).
    #[must_use]
    pub fn progress(&self) -> Option<(usize, usize)> {
        (self.state == State::Payload).then_some((self.got, self.len))
    }

    /// Feeds one byte. The payload goes into `dest[..len]`. Returns the
    /// payload length when a frame completes intact, or why it was refused;
    /// either way the receiver is ready for the next frame.
    pub fn feed(&mut self, b: u8, dest: &mut [u8]) -> Option<Result<usize, LoadError>> {
        match self.state {
            State::Magic(n) => {
                self.state = if b == MAGIC[n] {
                    if n + 1 == MAGIC.len() {
                        State::Header(0)
                    } else {
                        State::Magic(n + 1)
                    }
                } else if b == MAGIC[0] {
                    State::Magic(1)
                } else {
                    State::Magic(0)
                };
                None
            }
            State::Header(n) => {
                self.header[n] = b;
                if n + 1 < self.header.len() {
                    self.state = State::Header(n + 1);
                    return None;
                }
                let h = self.header;
                self.len = u32::from_le_bytes([h[0], h[1], h[2], h[3]]) as usize;
                self.want_crc = u32::from_le_bytes([h[4], h[5], h[6], h[7]]);
                self.got = 0;
                if self.len > dest.len() {
                    self.state = State::Magic(0);
                    return Some(Err(LoadError::TooBig));
                }
                if self.len == 0 {
                    return Some(self.finish());
                }
                self.state = State::Payload;
                None
            }
            State::Payload => {
                dest[self.got] = b;
                self.got += 1;
                if self.got < self.len {
                    return None;
                }
                Some(self.finish())
            }
        }
    }

    /// The frame is complete; [`feed_slice`] checks its CRC over `dest`.
    fn finish(&mut self) -> Result<usize, LoadError> {
        self.state = State::Magic(0);
        Ok(self.len)
    }

    /// The CRC the frame's header promised.
    #[must_use]
    pub fn expected_crc(&self) -> u32 {
        self.want_crc
    }
}

/// Feeds `data`, then checks the CRC of a completed frame over `dest`.
/// Returns the outcome of the first frame that completes (later bytes in
/// `data` are left for the next call: the count consumed is returned too).
pub fn feed_slice(
    rx: &mut Receiver,
    data: &[u8],
    dest: &mut [u8],
) -> (usize, Option<Result<usize, LoadError>>) {
    for (i, &b) in data.iter().enumerate() {
        if let Some(r) = rx.feed(b, dest) {
            let r = r.and_then(|len| {
                if crc32(&dest[..len]) == rx.expected_crc() {
                    Ok(len)
                } else {
                    Err(LoadError::BadCrc)
                }
            });
            return (i + 1, Some(r));
        }
    }
    (data.len(), None)
}

/// Builds a frame (the host side, and tests).
#[cfg(any(test, feature = "std"))]
#[must_use]
pub fn frame(payload: &[u8]) -> std::vec::Vec<u8> {
    let mut f = MAGIC.to_vec();
    f.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    f.extend_from_slice(&crc32(payload).to_le_bytes());
    f.extend_from_slice(payload);
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn receives_a_frame() {
        let payload: Vec<u8> = (0..1000u32).map(|i| (i * 7) as u8).collect();
        let mut dest = vec![0u8; 4096];
        let mut rx = Receiver::new();
        let f = frame(&payload);
        let (used, r) = feed_slice(&mut rx, &f, &mut dest);
        assert_eq!(used, f.len());
        assert_eq!(r, Some(Ok(1000)));
        assert_eq!(&dest[..1000], &payload[..]);
    }

    #[test]
    fn skips_noise_and_a_partial_magic_before_the_frame() {
        let payload = b"image".to_vec();
        let mut stream = b"hello\r\nRPRPL".to_vec();
        stream.extend(frame(&payload));
        let mut dest = [0u8; 64];
        let mut rx = Receiver::new();
        let (_, r) = feed_slice(&mut rx, &stream, &mut dest);
        assert_eq!(r, Some(Ok(5)));
        assert_eq!(&dest[..5], b"image");
    }

    #[test]
    fn a_damaged_payload_is_refused() {
        let mut f = frame(b"some image bytes");
        let last = f.len() - 1;
        f[last] ^= 0x01;
        let mut dest = [0u8; 64];
        let mut rx = Receiver::new();
        assert_eq!(
            feed_slice(&mut rx, &f, &mut dest).1,
            Some(Err(LoadError::BadCrc))
        );
        // And the next frame is received normally.
        let g = frame(b"good");
        assert_eq!(feed_slice(&mut rx, &g, &mut dest).1, Some(Ok(4)));
    }

    #[test]
    fn too_big_is_refused_before_any_byte_is_stored() {
        let mut dest = [0xAAu8; 8];
        let mut rx = Receiver::new();
        let f = frame(&[1u8; 9]);
        let (used, r) = feed_slice(&mut rx, &f, &mut dest);
        assert_eq!(r, Some(Err(LoadError::TooBig)));
        assert_eq!(used, 12, "refused right after the header");
        assert_eq!(dest, [0xAA; 8]);
    }

    #[test]
    fn bytes_after_a_frame_are_left_for_the_next_call() {
        let mut stream = frame(b"one");
        let second = frame(b"two");
        stream.extend(&second);
        let mut dest = [0u8; 16];
        let mut rx = Receiver::new();
        let (used, r) = feed_slice(&mut rx, &stream, &mut dest);
        assert_eq!(r, Some(Ok(3)));
        let (_, r) = feed_slice(&mut rx, &stream[used..], &mut dest);
        assert_eq!(r, Some(Ok(3)));
        assert_eq!(&dest[..3], b"two");
    }

    #[test]
    fn progress_is_reported_during_the_payload() {
        let f = frame(&[0u8; 100]);
        let mut dest = [0u8; 128];
        let mut rx = Receiver::new();
        let (_, r) = feed_slice(&mut rx, &f[..50], &mut dest);
        assert_eq!(r, None);
        assert_eq!(rx.progress(), Some((38, 100)));
    }
}
