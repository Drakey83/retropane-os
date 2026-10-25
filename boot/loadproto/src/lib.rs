//! The resident loader's wire protocol (ADR-0012 step U3): how the PC hands a
//! new image to the X51v over any byte stream (USB serial on the device, the
//! emulator's serial port in CI).
//!
//! The device announces itself with [`READY`] lines. The host sends one frame:
//!
//! | Bytes | Field |
//! |---|---|
//! | 4 | magic: `RPL1` (an image to run), `RPF1` (a file for the SD card) or `RPC1` (a command) |
//! | 4 | payload length, little-endian |
//! | 4 | CRC-32 (IEEE) of the payload, little-endian |
//! | n | payload |
//!
//! An image's payload is the raw image, loaded at the images' link address.
//! A file's payload is the name's length (one byte), the name, then the
//! contents; the device replaces the root-directory file of that name on
//! the card (ADR-0013) and stays in the loader. A command's payload is a
//! short ASCII command (`drive rw`, `drive ro`: whether the PC may write
//! the card as a drive, ADR-0014).
//!
//! The device answers [`OK`] (with details after it for a file) or an
//! `RPL ERR` line and, for an image, starts it. Bytes before a magic are
//! skipped, so a terminal's stray input or a half-sent earlier frame can't
//! wedge it.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod b64;

#[cfg(all(feature = "std", not(test)))]
extern crate std;

/// Frame start: an image to run.
pub const MAGIC: [u8; 4] = *b"RPL1";
/// Frame start: a file for the card.
pub const FILE_MAGIC: [u8; 4] = *b"RPF1";
/// Frame start: a command.
pub const CONTROL_MAGIC: [u8; 4] = *b"RPC1";
/// Longest command: 64 bytes since 2026-10-25 (32 before), for `format`'s
/// options (`format sd fat32 cluster 32 label RETROPANE wipe-boot`) and
/// card paths. An older loader answers a longer one "bad command".
pub const COMMAND_MAX: usize = 64;
/// Longest file name in a file frame.
pub const NAME_MAX: usize = 64;
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
    /// A file frame's name is missing, too long, or longer than the payload.
    BadName,
    /// A command frame is too long or not a known command.
    BadCommand,
}

impl LoadError {
    /// The device's reply line.
    #[must_use]
    pub const fn reply(self) -> &'static str {
        match self {
            Self::TooBig => "RPL ERR too big\r\n",
            Self::BadCrc => "RPL ERR bad crc\r\n",
            Self::BadName => "RPL ERR bad name\r\n",
            Self::BadCommand => "RPL ERR bad command\r\n",
        }
    }
}

/// What a frame carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// An image to run.
    Image,
    /// A file to write to the card.
    File,
    /// A command for the loader.
    Control,
}

impl Kind {
    /// The kind a magic's third byte names.
    const fn from_tag(b: u8) -> Option<Self> {
        match b {
            b'L' => Some(Self::Image),
            b'F' => Some(Self::File),
            b'C' => Some(Self::Control),
            _ => None,
        }
    }
}

/// A frame that arrived intact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    /// What it carries.
    pub kind: Kind,
    /// Payload length (in `dest[..len]`).
    pub len: usize,
}

impl Frame {
    /// A command frame's command (ASCII, at most [`COMMAND_MAX`] bytes).
    pub fn command<'d>(&self, dest: &'d [u8]) -> Result<&'d [u8], LoadError> {
        if self.kind != Kind::Control || self.len == 0 || self.len > COMMAND_MAX {
            return Err(LoadError::BadCommand);
        }
        let c = &dest[..self.len];
        if c.iter().all(|b| b.is_ascii_graphic() || *b == b' ') {
            Ok(c)
        } else {
            Err(LoadError::BadCommand)
        }
    }

    /// Splits a file frame's payload into its name and contents.
    pub fn file<'d>(&self, dest: &'d [u8]) -> Result<(&'d str, &'d [u8]), LoadError> {
        if self.kind != Kind::File || self.len == 0 {
            return Err(LoadError::BadName);
        }
        let n = usize::from(dest[0]);
        if n == 0 || n > NAME_MAX || 1 + n > self.len {
            return Err(LoadError::BadName);
        }
        let name = core::str::from_utf8(&dest[1..1 + n]).map_err(|_| LoadError::BadName)?;
        Ok((name, &dest[1 + n..self.len]))
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
    kind: Kind,
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
            kind: Kind::Image,
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
    /// frame when it completes intact, or why it was refused; either way the
    /// receiver is ready for the next frame.
    pub fn feed(&mut self, b: u8, dest: &mut [u8]) -> Option<Result<Frame, LoadError>> {
        match self.state {
            State::Magic(n) => {
                // "RP", one byte naming the kind, then "1".
                let matched = match n {
                    0 => b == MAGIC[0],
                    1 => b == MAGIC[1],
                    2 => match Kind::from_tag(b) {
                        Some(k) => {
                            self.kind = k;
                            true
                        }
                        None => false,
                    },
                    _ => b == MAGIC[3],
                };
                self.state = if matched {
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
    fn finish(&mut self) -> Result<Frame, LoadError> {
        self.state = State::Magic(0);
        Ok(Frame {
            kind: self.kind,
            len: self.len,
        })
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
) -> (usize, Option<Result<Frame, LoadError>>) {
    for (i, &b) in data.iter().enumerate() {
        if let Some(r) = rx.feed(b, dest) {
            let r = r.and_then(|f| {
                if crc32(&dest[..f.len]) == rx.expected_crc() {
                    Ok(f)
                } else {
                    Err(LoadError::BadCrc)
                }
            });
            return (i + 1, Some(r));
        }
    }
    (data.len(), None)
}

/// Builds an image frame (the host side, and tests).
#[cfg(any(test, feature = "std"))]
#[must_use]
pub fn frame(payload: &[u8]) -> std::vec::Vec<u8> {
    framed(MAGIC, payload)
}

/// Builds a file frame for the card's root-directory file `name`.
///
/// # Panics
/// If the name is empty or longer than [`NAME_MAX`].
#[cfg(any(test, feature = "std"))]
#[must_use]
pub fn file_frame(name: &str, contents: &[u8]) -> std::vec::Vec<u8> {
    assert!(!name.is_empty() && name.len() <= NAME_MAX, "bad file name");
    let mut payload = std::vec![name.len() as u8];
    payload.extend_from_slice(name.as_bytes());
    payload.extend_from_slice(contents);
    framed(FILE_MAGIC, &payload)
}

/// Builds a command frame.
#[cfg(any(test, feature = "std"))]
#[must_use]
pub fn control_frame(command: &str) -> std::vec::Vec<u8> {
    framed(CONTROL_MAGIC, command.as_bytes())
}

#[cfg(any(test, feature = "std"))]
fn framed(magic: [u8; 4], payload: &[u8]) -> std::vec::Vec<u8> {
    let mut f = magic.to_vec();
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
        assert_eq!(
            r,
            Some(Ok(Frame {
                kind: Kind::Image,
                len: 1000
            }))
        );
        assert_eq!(&dest[..1000], &payload[..]);
    }

    #[test]
    fn receives_a_file_frame_and_splits_its_name() {
        let f = file_frame("retropane.bin", b"contents");
        let mut dest = [0u8; 64];
        let mut rx = Receiver::new();
        let (_, r) = feed_slice(&mut rx, &f, &mut dest);
        let frame = r.unwrap().unwrap();
        assert_eq!(frame.kind, Kind::File);
        assert_eq!(frame.file(&dest), Ok(("retropane.bin", &b"contents"[..])));
        // And an image frame next, after a file: the kind follows the magic.
        let g = frame_for_test(b"img");
        let (_, r) = feed_slice(&mut rx, &g, &mut dest);
        assert_eq!(r.unwrap().unwrap().kind, Kind::Image);
    }

    fn frame_for_test(p: &[u8]) -> Vec<u8> {
        frame(p)
    }

    #[test]
    fn command_frames_carry_short_ascii_commands() {
        // Room for more than the longest command, so "too long" is the
        // command's fault, not the buffer's.
        let mut dest = [0u8; 2 * COMMAND_MAX];
        let mut rx = Receiver::new();
        let (_, r) = feed_slice(&mut rx, &control_frame("drive rw"), &mut dest);
        let f = r.unwrap().unwrap();
        assert_eq!(f.kind, Kind::Control);
        assert_eq!(f.command(&dest), Ok(&b"drive rw"[..]));
        // The longest taken whole.
        let at = "y".repeat(COMMAND_MAX);
        let (_, r) = feed_slice(&mut rx, &control_frame(&at), &mut dest);
        assert_eq!(r.unwrap().unwrap().command(&dest), Ok(at.as_bytes()));
        // Too long, or binary: refused.
        let long = "x".repeat(COMMAND_MAX + 1);
        let (_, r) = feed_slice(&mut rx, &control_frame(&long), &mut dest);
        assert_eq!(
            r.unwrap().unwrap().command(&dest),
            Err(LoadError::BadCommand)
        );
        let (_, r) = feed_slice(&mut rx, &framed(CONTROL_MAGIC, &[0, 1, 2]), &mut dest);
        assert_eq!(
            r.unwrap().unwrap().command(&dest),
            Err(LoadError::BadCommand)
        );
        // A file frame is not a command.
        let (_, r) = feed_slice(&mut rx, &file_frame("a", b"b"), &mut dest);
        assert_eq!(
            r.unwrap().unwrap().command(&dest),
            Err(LoadError::BadCommand)
        );
    }

    #[test]
    fn unknown_kinds_and_near_misses_are_skipped() {
        let mut stream = b"RPX1RPRPRPL0RPC".to_vec();
        stream.extend(frame(b"ok"));
        let mut dest = [0u8; 16];
        let mut rx = Receiver::new();
        let (_, r) = feed_slice(&mut rx, &stream, &mut dest);
        let f = r.unwrap().unwrap();
        assert_eq!((f.kind, f.len), (Kind::Image, 2));
    }

    /// Hostile input: random streams (with real magics sprinkled in) never
    /// panic, never write past the destination, and frames that complete
    /// either check out or are refused.
    #[test]
    fn random_streams_never_panic() {
        let mut x: u64 = 0x5250_4C31;
        let mut rnd = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let mut dest = [0u8; 200];
        let mut rx = Receiver::new();
        let magics = [MAGIC, FILE_MAGIC, CONTROL_MAGIC];
        for _ in 0..20_000 {
            let mut chunk: Vec<u8> = (0..(rnd() % 64)).map(|_| rnd() as u8).collect();
            if rnd() % 4 == 0 {
                chunk.extend(magics[(rnd() % 3) as usize]);
                chunk.extend((rnd() as u32 % 300).to_le_bytes());
            }
            let mut data = &chunk[..];
            while !data.is_empty() {
                let (used, r) = feed_slice(&mut rx, data, &mut dest);
                assert!(used >= 1 && used <= data.len());
                if let Some(Ok(f)) = r {
                    assert!(f.len <= dest.len());
                    let _ = f.file(&dest);
                    let _ = f.command(&dest);
                }
                data = &data[used..];
            }
        }
    }

    #[test]
    fn a_file_frame_with_a_bad_name_is_refused_by_the_split() {
        let mut dest = [0u8; 64];
        let mut rx = Receiver::new();
        // Name length says 10, payload has 3 bytes.
        let f = framed(FILE_MAGIC, &[10, b'a', b'b']);
        let (_, r) = feed_slice(&mut rx, &f, &mut dest);
        let frame = r.unwrap().unwrap();
        assert_eq!(frame.file(&dest), Err(LoadError::BadName));
        assert_eq!(LoadError::BadName.reply(), "RPL ERR bad name\r\n");
    }

    #[test]
    fn skips_noise_and_a_partial_magic_before_the_frame() {
        let payload = b"image".to_vec();
        let mut stream = b"hello\r\nRPRPL".to_vec();
        stream.extend(frame(&payload));
        let mut dest = [0u8; 64];
        let mut rx = Receiver::new();
        let (_, r) = feed_slice(&mut rx, &stream, &mut dest);
        assert_eq!(r.unwrap().unwrap().len, 5);
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
        assert_eq!(
            feed_slice(&mut rx, &g, &mut dest).1.unwrap().unwrap().len,
            4
        );
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
        assert_eq!(r.unwrap().unwrap().len, 3);
        let (_, r) = feed_slice(&mut rx, &stream[used..], &mut dest);
        assert_eq!(r.unwrap().unwrap().len, 3);
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
