//! What every bare-metal image shares (quality audit C38): one entry, one
//! crash path and one end, so no image can report a crash as a hang or a
//! clean finish.
//!
//! - [`entry!`] starts the image with the crash path already in place: a
//!   fault (data abort, prefetch abort, undefined instruction, the run
//!   limit) is reported from the image's first instruction, before it sets
//!   anything up.
//! - [`crash`] and [`fail`] are the image's own stops (a panic, a fault, a
//!   setup step that failed). In this order: the image's put-back step
//!   ([`on_crash`], say the clocks it changed), the serial line, the crash
//!   box on screen (if the display is up: the real X51v has no serial
//!   port), the run-log line, the crash flag, back to the loader, which
//!   sends the report to the PC and keeps it on the card. Without a loader
//!   it stops there, on screen and on the serial port.
//! - [`finish`] is the clean end: the deepest stack in the run log, the
//!   run marked as returned, back to the loader.
//!
//! The panic handler is here too: an image links this crate and has none
//! of its own. `cargo xtask lint` refuses a hand-written fault handler,
//! panic handler or fault-handler install anywhere else (the loader, which
//! receives crashes, keeps its own).

#![cfg_attr(target_os = "none", no_std)]
#![forbid(unsafe_code)]

/// Where the crash box goes: the bottom 64 lines of the 480 x 640 screen.
pub const BOX_AT: (usize, usize) = (0, 576);
/// The crash box's size.
pub const BOX_SIZE: (usize, usize) = (480, 64);
/// The crash box's text scale (16-pixel letters: two lines of 30).
pub const BOX_SCALE: usize = 2;
/// The crash box's colours (RGB565): white on dark red.
pub const BOX_COLORS: (u16, u16) = (0xFFFF, 0xA800);
/// How long the crash box stays before the loader's crash screen replaces
/// it (long enough for `cargo xtask emu` and a person to see it).
pub const BOX_HOLD_US: u32 = 3_000_000;

/// How much of the crash path runs on the `depth`-th crash of a run (0 for
/// the first): a crash inside the crash path (the put-back step or the
/// screen faulting) still gets its report out without what failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Steps {
    /// The image's put-back step ([`on_crash`]).
    pub put_back: bool,
    /// The image's extra line ([`crash_note`]): it reads the image's state,
    /// which may be what faulted.
    pub note: bool,
    /// The crash box on screen.
    pub screen: bool,
    /// The serial line, the run-log line, the crash flag and the return.
    pub report: bool,
}

/// The steps for the `depth`-th crash.
#[must_use]
pub const fn steps(depth: u32) -> Steps {
    match depth {
        0 => Steps {
            put_back: true,
            note: true,
            screen: true,
            report: true,
        },
        // The first try faulted part way: the report alone.
        1 => Steps {
            put_back: false,
            note: false,
            screen: false,
            report: true,
        },
        // The report itself faults: nothing more can be said.
        _ => Steps {
            put_back: false,
            note: false,
            screen: false,
            report: false,
        },
    }
}

/// What a fault's line says beyond its kind and address (the faulting
/// instruction's, as the runtime reports it): the word there, so the
/// instruction itself (`None`: it can't be read, as after a jump to an
/// unmapped address), and the CPU's status where it faulted (the SPSR).
/// From probe's fault line (bring-up), now every image's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FaultDetail {
    /// The word at the faulting instruction's address.
    pub word: Option<u32>,
    /// The SPSR: the mode, the interrupt masks and the flags at the fault.
    pub spsr: u32,
}

/// Writes a fault's line: `FAULT <kind> at <address> word <word or
/// unreadable> spsr <spsr>`. Words by `write_str`, numbers by integer
/// formatting (no `&str` padding code in the image, ADR-0005 rule 8).
pub fn fault_line(
    w: &mut dyn core::fmt::Write,
    kind: &str,
    addr: u32,
    detail: FaultDetail,
) -> core::fmt::Result {
    w.write_str("FAULT ")?;
    w.write_str(kind)?;
    write!(w, " at {addr:#010x}")?;
    match detail.word {
        Some(word) => write!(w, " word {word:#010x}")?,
        None => w.write_str(" word unreadable")?,
    }
    write!(w, " spsr {:#010x}", detail.spsr)
}

#[cfg(target_os = "none")]
mod image;
#[cfg(target_os = "none")]
pub use image::*;

#[cfg(target_os = "none")]
#[doc(hidden)]
pub use rp_hal_pxa27x as __pxa27x;

/// The image's entry: `rp_image::entry!(main)`, `main: fn() -> !`. The
/// crash path is installed before `main` runs.
#[cfg(target_os = "none")]
#[macro_export]
macro_rules! entry {
    ($path:path) => {
        $crate::__pxa27x::entry!(__rp_image_main);
        fn __rp_image_main() -> ! {
            $crate::install();
            let f: fn() -> ! = $path;
            f()
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_crash_inside_the_crash_path_still_reports() {
        assert_eq!(
            steps(0),
            Steps {
                put_back: true,
                note: true,
                screen: true,
                report: true
            }
        );
        // The put-back step, the note or the screen faulted: report
        // without them.
        let second = steps(1);
        assert!(second.report && !second.put_back && !second.note && !second.screen);
        // The report faulted: stop, don't loop through it again.
        assert!(!steps(2).report && !steps(u32::MAX - 1).report && !steps(u32::MAX).report);
    }

    #[test]
    fn fault_line_names_the_kind_address_word_and_status() {
        let mut s = String::new();
        let d = FaultDetail {
            word: Some(0xE591_0000),
            spsr: 0x6000_00D3,
        };
        fault_line(&mut s, "DataAbort", 0xDEAD_0004, d).unwrap();
        assert_eq!(
            s,
            "FAULT DataAbort at 0xdead0004 word 0xe5910000 spsr 0x600000d3"
        );
        // Every number's edges written directly: all ten characters always.
        for (n, want) in [
            (0, "0x00000000"),
            (1, "0x00000001"),
            (u32::MAX - 1, "0xfffffffe"),
            (u32::MAX, "0xffffffff"),
        ] {
            s.clear();
            let d = FaultDetail {
                word: Some(n),
                spsr: n,
            };
            fault_line(&mut s, "Undefined", n, d).unwrap();
            assert_eq!(
                s,
                format!("FAULT Undefined at {want} word {want} spsr {want}")
            );
        }
        // An address that can't be read says so (a word of 0 would mislead).
        s.clear();
        let d = FaultDetail {
            word: None,
            spsr: 0,
        };
        fault_line(&mut s, "PrefetchAbort", 0x8000_0000, d).unwrap();
        assert_eq!(
            s,
            "FAULT PrefetchAbort at 0x80000000 word unreadable spsr 0x00000000"
        );
    }

    #[test]
    fn the_box_fits_the_screen() {
        assert!(BOX_AT.0 + BOX_SIZE.0 <= 480);
        assert!(BOX_AT.1 + BOX_SIZE.1 <= 640);
    }
}
