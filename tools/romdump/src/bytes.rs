//! Bounds-checked little-endian reads. Every parser in romdump goes through
//! these, so a malformed or truncated image produces an error, never a panic.

use crate::Error;

/// Reads a `u16` at `off`.
pub fn u16_at(d: &[u8], off: usize) -> Result<u16, Error> {
    let b = d.get(off..off + 2).ok_or(Error::Truncated(off))?;
    Ok(u16::from_le_bytes([b[0], b[1]]))
}

/// Reads a `u32` at `off`.
pub fn u32_at(d: &[u8], off: usize) -> Result<u32, Error> {
    let b = d.get(off..off + 4).ok_or(Error::Truncated(off))?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Borrows `len` bytes at `off`.
pub fn slice(d: &[u8], off: usize, len: usize) -> Result<&[u8], Error> {
    d.get(off..off.checked_add(len).ok_or(Error::Truncated(off))?)
        .ok_or(Error::Truncated(off))
}

/// Decodes UTF-16LE, stopping at the first NUL.
pub fn utf16(b: &[u8]) -> String {
    let units: Vec<u16> = b
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&c| u16::from_le_bytes(c))
        .take_while(|&u| u != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_little_endian() {
        let d = [0x34, 0x12, 0x78, 0x56];
        assert_eq!(u16_at(&d, 0).unwrap(), 0x1234);
        assert_eq!(u32_at(&d, 0).unwrap(), 0x5678_1234);
    }

    #[test]
    fn out_of_bounds_is_an_error() {
        assert!(matches!(u32_at(&[0; 3], 0), Err(Error::Truncated(0))));
        assert!(slice(&[0; 4], 2, 3).is_err());
        assert!(slice(&[0; 4], usize::MAX, 2).is_err());
    }

    #[test]
    fn utf16_stops_at_nul() {
        let b = [b'h', 0, b'i', 0, 0, 0, b'x', 0];
        assert_eq!(utf16(&b), "hi");
    }
}
