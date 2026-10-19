//! Which DMA channel belongs to whom: one table, checked when the code is
//! compiled, like `rp-vram-map` for the 2700G's memory.
//!
//! Why one table: channels were numbers picked in each driver's corner, and
//! two pieces of work took channel 2 at once (the CF card's reads and the
//! USB controller's drive transfers, 2026-10-16). Now a channel has one
//! owner here, `rt::take_dma` hands out only channels listed, and the build
//! fails if a channel is listed twice or the descriptor chains outgrow
//! their budget.
//!
//! Each channel's chain is a ring of descriptor slots ([`CHUNK`] bytes
//! each) that `dmachain` streams a transfer of any length through, topped
//! up as the owner polls: its size is how far the controller can run ahead
//! of the polls (a transfer that fits needs no top-up). Every chain lives
//! in zeroed RAM in every image that uses DMA, so a chain bigger than its
//! owner needs is RAM taken from all of them.
//!
//! Pure: no hardware access.

/// Bytes per descriptor (under the 8,191 the length field holds).
pub const CHUNK: usize = 4096;

/// One channel's owner and chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Assignment {
    /// The PXA27x DMA channel (0-31).
    pub channel: u32,
    /// Who uses it.
    pub owner: &'static str,
    /// Descriptors in its chain: this many [`CHUNK`]s run without a
    /// top-up poll.
    pub descriptors: usize,
}

const fn a(channel: u32, owner: &'static str, descriptors: usize) -> Assignment {
    Assignment {
        channel,
        owner,
        descriptors,
    }
}

/// The SD card's transfers (`pxa-mmc`; up to 128 KiB). Test images that run
/// alone (the benchmarks, `rp-mbx`'s copies) borrow it.
pub const SD: u32 = 0;
/// The audio ring (`pxa-i2s`, the mixer: 44 KiB in 32 pieces of 1,408
/// bytes, ADR-0032 A6; one piece per slot, `dmaring`).
pub const AUDIO: u32 = 1;
/// The CompactFlash card's reads (the 32-bit data window by DMA).
pub const CF: u32 = 2;
/// The USB controller's drive transfers (`pxa-udc` / `rp-usb-msc`: at most
/// the drive's whole 32 KiB staging buffer, item 74).
pub const UDC: u32 = 3;

/// The table.
pub const MAP: [Assignment; 4] = [
    a(SD, "SD card (pxa-mmc)", 32),
    a(AUDIO, "audio ring (pxa-i2s)", 32),
    a(CF, "CompactFlash reads (pxa-pcmcia window)", 32),
    a(UDC, "USB controller, the drive's transfers (pxa-udc)", 8),
];

/// Bytes all chains may take (zeroed RAM in every image using DMA; the
/// loader's zeroed budget in `xtask/src/size.rs` has room for this much).
pub const CHAIN_BUDGET: usize = 1664;

/// Descriptors in all chains.
pub const TOTAL: usize = total();

const fn total() -> usize {
    let mut n = 0;
    let mut i = 0;
    while i < MAP.len() {
        n += MAP[i].descriptors;
        i += 1;
    }
    n
}

/// Whether the table is sound: channels 0-31, none twice, every chain one
/// to 32 descriptors, all chains within [`CHAIN_BUDGET`].
const fn sound() -> bool {
    let mut i = 0;
    while i < MAP.len() {
        if MAP[i].channel > 31 || MAP[i].descriptors == 0 || MAP[i].descriptors > 32 {
            return false;
        }
        let mut j = i + 1;
        while j < MAP.len() {
            if MAP[i].channel == MAP[j].channel {
                return false;
            }
            j += 1;
        }
        i += 1;
    }
    TOTAL * 16 <= CHAIN_BUDGET
}

const _: () = assert!(
    sound(),
    "DMA channel map: a channel listed twice, out of range, or chains over budget"
);

/// The chain size of channel `ch`, 0 if it isn't listed.
#[must_use]
pub const fn descriptors(ch: u32) -> usize {
    let mut i = 0;
    while i < MAP.len() {
        if MAP[i].channel == ch {
            return MAP[i].descriptors;
        }
        i += 1;
    }
    0
}

/// Where channel `ch`'s chain starts among all descriptors (the listed
/// order).
#[must_use]
pub const fn first(ch: u32) -> usize {
    let mut at = 0;
    let mut i = 0;
    while i < MAP.len() {
        if MAP[i].channel == ch {
            return at;
        }
        at += MAP[i].descriptors;
        i += 1;
    }
    TOTAL
}

/// The largest transfer channel `ch` runs with no top-up poll (one ring).
#[must_use]
pub const fn max_bytes(ch: u32) -> usize {
    descriptors(ch) * CHUNK
}
