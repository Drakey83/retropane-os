//! The crash path's backtrace: a bounded walk of the frame-pointer chain
//! (ADR-0001, "The crash-screen backtrace is mode-dependent").
//!
//! Only images built with frame pointers walk (`--cfg rp_frame_pointers`,
//! xtask's frame-pointer list): there LLVM gives every function, leaves
//! included, a frame record of two words, `r11` pointing at the first, the
//! caller's `r11`, and the return address (`lr`) in the second (ARM code,
//! checked by disassembling `rp-crash` built that way: `push {.., r11,
//! lr}` then `r11` at the saved `r11`). `_start` clears `r11` before
//! `rp_main`, so a chain that reaches 0 has ended where the image began.
//!
//! Every other image (the kernel, the loader, the benchmark images whose
//! numbers are the perf ratchet's floors) is built may-omit: `r11` is an
//! ordinary register there, and no walker is in the image at all; its crash
//! report says [`NO_CHAIN`].

/// What a crash report says in an image built without frame pointers.
pub const NO_CHAIN: &str = "no frame chain (built may-omit)";

/// The most calls a chain lists (ROADMAP: the backtrace walker).
#[cfg(any(not(target_os = "none"), rp_frame_pointers))]
pub const MAX_CALLS: usize = 16;

/// Why a walk stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg(any(not(target_os = "none"), rp_frame_pointers))]
pub enum End {
    /// The chain reached 0: `_start`'s, where the image began.
    Start,
    /// [`MAX_CALLS`] listed and the chain goes on.
    Cut,
    /// A frame pointer no frame record can be at: the chain is broken
    /// there (the value is the frame pointer).
    Broken(Broken, u32),
}

/// What was wrong with a frame pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg(any(not(target_os = "none"), rp_frame_pointers))]
pub enum Broken {
    /// Not a multiple of 4.
    NotAligned,
    /// Its record (two words) isn't all in RAM.
    NotInRam,
    /// Not above the frame before it (a stack grows down, so each caller's
    /// record is higher up).
    NotRising,
    /// The record couldn't be read.
    Unreadable,
}

/// The calls a walk found, innermost first, and why it stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg(any(not(target_os = "none"), rp_frame_pointers))]
pub struct Chain {
    /// The call instructions' addresses (each return address less 4: ARM
    /// code's `bl` is 4 bytes, and a call to a function that never returns
    /// can be a function's last instruction, so the return address may be
    /// the next function's first).
    pub calls: [u32; MAX_CALLS],
    /// How many of `calls` are filled.
    pub len: usize,
    /// Why the walk stopped.
    pub end: End,
}

#[cfg(any(not(target_os = "none"), rp_frame_pointers))]
impl Chain {
    /// The calls found, innermost first.
    #[must_use]
    pub fn calls(&self) -> &[u32] {
        &self.calls[..self.len]
    }
}

/// Walks the chain from frame pointer `fp`. A record is read only once its
/// address is known to be word-aligned and inside `ram` (on the device,
/// SDRAM: `mmu::SDRAM`), with `read` (one
/// word; `None` if it can't be read: on the device, `rt::try_read`, which
/// can't fault out). The walk is bounded: at most [`MAX_CALLS`] records,
/// each strictly above the one before.
#[cfg(any(not(target_os = "none"), rp_frame_pointers))]
#[must_use]
pub fn walk(
    fp: u32,
    ram: core::ops::Range<u32>,
    read: &mut dyn FnMut(u32) -> Option<u32>,
) -> Chain {
    let mut chain = Chain {
        calls: [0; MAX_CALLS],
        len: 0,
        end: End::Start,
    };
    let mut fp = fp;
    // The frame pointer before `fp`, which `fp` must be above (none yet).
    let mut below: Option<u32> = None;
    chain.end = loop {
        if fp == 0 {
            break End::Start;
        }
        if below.is_some_and(|b| fp <= b) {
            break End::Broken(Broken::NotRising, fp);
        }
        if !fp.is_multiple_of(4) {
            break End::Broken(Broken::NotAligned, fp);
        }
        // The record's two words, fp and fp + 4, both in RAM.
        if fp < ram.start || ram.end.saturating_sub(8) < fp {
            break End::Broken(Broken::NotInRam, fp);
        }
        if chain.len == MAX_CALLS {
            break End::Cut;
        }
        let (Some(next), Some(ret)) = (read(fp), read(fp + 4)) else {
            break End::Broken(Broken::Unreadable, fp);
        };
        chain.calls[chain.len] = ret.wrapping_sub(4);
        chain.len += 1;
        below = Some(fp);
        fp = next;
    };
    chain
}

/// Writes a chain's line: `CHAIN <call>... <end>`, the end one of `start`
/// (the chain reached where the image began), `cut` (more than
/// [`MAX_CALLS`]) or `broken: fp <value> <why>`. Numbers by integer
/// formatting, words by `write_str` (ADR-0005 rule 8).
#[cfg(any(not(target_os = "none"), rp_frame_pointers))]
pub fn chain_line(w: &mut dyn core::fmt::Write, chain: &Chain) -> core::fmt::Result {
    w.write_str("CHAIN")?;
    for call in chain.calls() {
        write!(w, " {call:#010x}")?;
    }
    match chain.end {
        End::Start => w.write_str(" start"),
        End::Cut => w.write_str(" cut"),
        End::Broken(why, fp) => {
            write!(w, " broken: fp {fp:#010x} ")?;
            w.write_str(match why {
                Broken::NotAligned => "not word-aligned",
                Broken::NotInRam => "not in RAM",
                Broken::NotRising => "not above the frame before",
                Broken::Unreadable => "unreadable",
            })
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use std::string::String;
    use std::vec::Vec;

    use super::*;

    /// A small RAM for the tests: 64 words at 0x1000.
    const RAM: core::ops::Range<u32> = 0x1000..0x1100;

    /// Memory for `walk`: (address, word) pairs; anything else unreadable.
    /// Counts the reads, and fails a read outside `RAM` (the walker must
    /// check before it reads).
    struct Mem {
        words: Vec<(u32, u32)>,
        reads: usize,
    }

    impl Mem {
        /// A chain of records at `fps` (innermost first), each linking to
        /// the next and the last to `last`; record `i` returns to
        /// `0x100 + 0x10 * i`.
        fn chain(fps: &[u32], last: u32) -> Self {
            let mut words = Vec::new();
            for (i, &fp) in fps.iter().enumerate() {
                let next = fps.get(i + 1).copied().unwrap_or(last);
                words.push((fp, next));
                words.push((fp + 4, 0x100 + 0x10 * i as u32));
            }
            Self { words, reads: 0 }
        }

        fn walk(&mut self, fp: u32) -> Chain {
            walk(fp, RAM, &mut |a| {
                assert!(RAM.contains(&a), "read outside RAM at {a:#x}");
                self.reads += 1;
                self.words.iter().find(|(at, _)| *at == a).map(|(_, v)| *v)
            })
        }
    }

    fn line(chain: &Chain) -> String {
        let mut s = String::new();
        chain_line(&mut s, chain).unwrap();
        s
    }

    #[test]
    fn a_valid_chain_lists_every_call_to_the_start() {
        let mut m = Mem::chain(&[0x1010, 0x1030, 0x1070], 0);
        let c = m.walk(0x1010);
        // Each return address less 4: the call instruction.
        assert_eq!(c.calls(), &[0xFC, 0x10C, 0x11C]);
        assert_eq!(c.end, End::Start);
        assert_eq!(line(&c), "CHAIN 0x000000fc 0x0000010c 0x0000011c start");
    }

    #[test]
    fn no_frame_at_all_is_an_empty_chain() {
        let c = Mem::chain(&[], 0).walk(0);
        assert_eq!(c.calls(), &[] as &[u32]);
        assert_eq!(line(&c), "CHAIN start");
    }

    #[test]
    fn a_chain_corrupt_mid_way_keeps_what_came_before() {
        // The second record's link is overwritten with each kind of bad
        // value; the first call is kept and the break is named.
        for (bad, why) in [
            (0x1052, Broken::NotAligned),
            (0x2000, Broken::NotInRam),
            (0xFFFF_FFF8, Broken::NotInRam),
            // Below and equal: a loop back down or onto itself.
            (0x1010, Broken::NotRising),
            (0x1030, Broken::NotRising),
        ] {
            let mut m = Mem::chain(&[0x1010, 0x1030], bad);
            let c = m.walk(0x1010);
            assert_eq!(c.calls(), &[0xFC, 0x10C], "link {bad:#x}");
            assert_eq!(c.end, End::Broken(why, bad), "link {bad:#x}");
        }
        let c = Mem::chain(&[0x1010], 0x1052).walk(0x1010);
        assert_eq!(
            line(&c),
            "CHAIN 0x000000fc broken: fp 0x00001052 not word-aligned"
        );
    }

    #[test]
    fn a_record_at_the_edge_of_ram_is_read_whole_or_not_at_all() {
        // The last whole record in RAM is walked; one starting 4 bytes
        // later would have its return address outside.
        let c = Mem::chain(&[0x10F8], 0).walk(0x10F8);
        assert_eq!((c.len, c.end), (1, End::Start));
        let c = Mem::chain(&[], 0).walk(0x10FC);
        assert_eq!(c.end, End::Broken(Broken::NotInRam, 0x10FC));
        // The first word in RAM is fine; the word below isn't.
        let c = Mem::chain(&[0x1000], 0).walk(0x1000);
        assert_eq!((c.len, c.end), (1, End::Start));
        let c = Mem::chain(&[], 0).walk(0x0FFC);
        assert_eq!(c.end, End::Broken(Broken::NotInRam, 0x0FFC));
    }

    #[test]
    fn an_unreadable_record_breaks_the_chain() {
        let mut m = Mem::chain(&[0x1010], 0x1040);
        let c = m.walk(0x1010);
        assert_eq!(c.calls(), &[0xFC]);
        assert_eq!(c.end, End::Broken(Broken::Unreadable, 0x1040));
        assert_eq!(
            line(&c),
            "CHAIN 0x000000fc broken: fp 0x00001040 unreadable"
        );
    }

    #[test]
    fn a_long_chain_is_cut_at_the_limit() {
        // MAX_CALLS + 4 records, every one valid: 16 listed, then "cut".
        let fps: Vec<u32> = (0..MAX_CALLS as u32 + 4).map(|i| 0x1000 + 8 * i).collect();
        let mut m = Mem::chain(&fps, 0);
        let c = m.walk(0x1000);
        assert_eq!(c.len, MAX_CALLS);
        assert_eq!(c.end, End::Cut);
        assert_eq!(
            c.calls()[MAX_CALLS - 1],
            0x100 + 0x10 * (MAX_CALLS as u32 - 1) - 4
        );
        assert!(line(&c).ends_with(" cut"));
        assert_eq!(m.reads, 2 * MAX_CALLS, "nothing read past the limit");
        // Exactly MAX_CALLS records then the start: not cut.
        let c = Mem::chain(&fps[..MAX_CALLS], 0).walk(0x1000);
        assert_eq!((c.len, c.end), (MAX_CALLS, End::Start));
    }

    #[test]
    fn a_truncated_stack_ends_at_its_last_readable_record() {
        // The stack's top records were never written (a fault early in a
        // copy, say): the chain links to a word nothing holds.
        let c = Mem::chain(&[0x1010, 0x1020], 0x10F0).walk(0x1010);
        assert_eq!(c.calls(), &[0xFC, 0x10C]);
        assert_eq!(c.end, End::Broken(Broken::Unreadable, 0x10F0));
    }

    #[test]
    fn may_omit_garbage_never_reads_outside_ram_and_stops() {
        // A may-omit image's r11 is any value: walked anyway (as a check of
        // the bounds; such an image has no walker), every start and every
        // link either breaks at once or rises, so the walk ends within
        // MAX_CALLS records without a read outside RAM (Mem::walk asserts).
        let mut seed = 0x1234_5678_u32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        for _ in 0..2000 {
            // Some values made to land in RAM, so the walk goes on a while.
            let pick = |v: u32| {
                if v & 1 == 0 {
                    0x1000 + (v >> 8) % 0x100
                } else {
                    v
                }
            };
            let words: Vec<(u32, u32)> = (0..64).map(|i| (0x1000 + 4 * i, pick(next()))).collect();
            let mut m = Mem { words, reads: 0 };
            let fp = pick(next());
            let c = m.walk(fp);
            assert!(c.len <= MAX_CALLS);
            assert!(m.reads <= 2 * MAX_CALLS);
            assert_ne!(c.end, End::Start, "garbage never links to 0 here");
        }
    }
}
