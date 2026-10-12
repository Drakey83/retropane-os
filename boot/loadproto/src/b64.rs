//! Base64 (RFC 4648, standard alphabet, padded) for data the loader sends
//! the PC as text lines (`RPD ` lines: a third bigger than the bytes, not
//! double as hex). Streaming on the encoding side: bytes go in in any
//! pieces, whole 76-character lines come out.

const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
/// Bytes per line (76 characters).
pub const LINE_BYTES: usize = 57;

/// Encodes a stream as base64 lines.
pub struct Encoder {
    pending: [u8; LINE_BYTES],
    n: usize,
}

impl Default for Encoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Encoder {
    /// An encoder with nothing pending.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pending: [0; LINE_BYTES],
            n: 0,
        }
    }

    /// Adds `data`, calling `line` with each complete line (no line end).
    pub fn feed(&mut self, mut data: &[u8], line: &mut dyn FnMut(&[u8])) {
        while !data.is_empty() {
            let take = (LINE_BYTES - self.n).min(data.len());
            self.pending[self.n..self.n + take].copy_from_slice(&data[..take]);
            self.n += take;
            data = &data[take..];
            if self.n == LINE_BYTES {
                self.flush(line);
            }
        }
    }

    /// Emits what is pending as a last (shorter, padded) line, if anything.
    pub fn finish(&mut self, line: &mut dyn FnMut(&[u8])) {
        if self.n > 0 {
            self.flush(line);
        }
    }

    fn flush(&mut self, line: &mut dyn FnMut(&[u8])) {
        let mut out = [0u8; LINE_BYTES / 3 * 4];
        let len = encode(&self.pending[..self.n], &mut out);
        line(&out[..len]);
        self.n = 0;
    }
}

/// Encodes `data` (at most 57 bytes) into `out`; returns the length.
fn encode(data: &[u8], out: &mut [u8; LINE_BYTES / 3 * 4]) -> usize {
    let mut o = 0;
    for chunk in data.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |a, (i, &b)| a | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            out[o] = if i <= chunk.len() {
                ABC[(n >> (18 - 6 * i)) as usize & 63]
            } else {
                b'='
            };
            o += 1;
        }
    }
    o
}

/// Decodes one base64 line into `out`; returns the bytes written, or
/// `None` for anything that isn't base64 (or doesn't fit).
#[must_use]
pub fn decode(line: &[u8], out: &mut [u8]) -> Option<usize> {
    if !line.len().is_multiple_of(4) {
        return None;
    }
    let mut o = 0;
    for quad in line.chunks(4) {
        let mut n = 0u32;
        let mut pad = 0;
        for (i, &c) in quad.iter().enumerate() {
            let v = match c {
                b'A'..=b'Z' => c - b'A',
                b'a'..=b'z' => c - b'a' + 26,
                b'0'..=b'9' => c - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                b'=' if i >= 2 => {
                    pad += 1;
                    0
                }
                _ => return None,
            };
            if pad > 0 && c != b'=' {
                return None; // data after padding
            }
            n |= u32::from(v) << (18 - 6 * i);
        }
        let bytes = 3 - pad;
        if o + bytes > out.len() {
            return None;
        }
        for k in 0..bytes {
            out[o + k] = (n >> (16 - 8 * k)) as u8;
        }
        o += bytes;
    }
    Some(o)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(data: &[u8], pieces: usize) -> Vec<u8> {
        let mut enc = Encoder::new();
        let mut lines = Vec::new();
        for chunk in data.chunks(pieces.max(1)) {
            enc.feed(chunk, &mut |l| lines.push(l.to_vec()));
        }
        enc.finish(&mut |l| lines.push(l.to_vec()));
        assert!(lines.iter().all(|l| l.len() <= 76));
        let mut out = Vec::new();
        for l in lines {
            let mut buf = [0u8; 57];
            let n = decode(&l, &mut buf).expect("decodes");
            out.extend_from_slice(&buf[..n]);
        }
        out
    }

    #[test]
    fn known_values() {
        let mut out = [0u8; 76];
        for (data, want) in [
            (&b"Man"[..], &b"TWFu"[..]),
            (b"Ma", b"TWE="),
            (b"M", b"TQ=="),
        ] {
            let n = encode(data, &mut out);
            assert_eq!(&out[..n], want);
        }
        let mut d = [0u8; 3];
        assert_eq!(decode(b"TWE=", &mut d), Some(2));
        assert_eq!(&d[..2], b"Ma");
    }

    #[test]
    fn any_data_in_any_pieces_round_trips() {
        let mut x = 0x5EED_u32;
        for len in [0, 1, 2, 3, 56, 57, 58, 114, 1000, 4097] {
            let data: Vec<u8> = (0..len)
                .map(|_| {
                    x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
                    (x >> 16) as u8
                })
                .collect();
            for pieces in [1, 7, 57, 512, 5000] {
                assert_eq!(roundtrip(&data, pieces), data, "len {len} pieces {pieces}");
            }
        }
    }

    #[test]
    fn rubbish_is_refused_not_misread() {
        let mut out = [0u8; 64];
        for bad in [&b"abc"[..], b"ab$=", b"a===", b"ab=c", b"TWFu\n"] {
            assert_eq!(decode(bad, &mut out), None, "{bad:?}");
        }
        assert_eq!(decode(b"TWFuTWFu", &mut out[..3]), None, "doesn't fit");
    }
}
