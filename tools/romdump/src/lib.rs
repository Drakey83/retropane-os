//! romdump: unpacks Dell Axim X51v Windows Mobile ROM images (`.nb0`) and
//! rebuilds the driver modules, for interoperability research
//! (docs/CLEANROOM.md §4).
//!
//! Output goes under `local/`, which is git-ignored. Extracted files are
//! proprietary and must never be committed.

#![forbid(unsafe_code)]

pub mod bytes;
pub mod imgfs;
pub mod nb0;
pub mod pe;
pub mod xip;

/// Everything that can go wrong reading an image.
#[derive(Debug)]
pub enum Error {
    /// A read ran past the end of the data, at this offset.
    Truncated(usize),
    /// The data doesn't match the expected format.
    Format(String),
    /// An I/O error.
    Io(std::io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated(off) => write!(f, "truncated data at offset {off:#x}"),
            Self::Format(msg) => write!(f, "{msg}"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
