//! Safe, single-owner access to X51v board devices for bare-metal images.
//!
//! Like `rp_hal_pxa27x::rt::take_ffuart`, each device can be taken once, before
//! interrupts are enabled, with the MMU off (physical = virtual).

use core::ptr::NonNull;
use rp_hal_pxa27x::devmap::va;

use rp_hal_pxa27x::Mmio;

use crate::{MBX_REG_OFFSET, MBX_REG_SIZE, MBX_VRAM_SIZE, region};

static mut MBX_TAKEN: bool = false;
static mut CPLD_TAKEN: bool = false;
static mut CF_TAKEN: bool = false;
static mut SCRATCH_TAKEN: bool = false;

/// Hands out the Intel 2700G once: its register block and its local memory.
pub fn take_mbx() -> Option<(Mmio, &'static mut [u8])> {
    // SAFETY: single core, IRQs masked by `_start` and not re-enabled yet, so
    // nothing can race this read-modify-write of the flag.
    let taken = unsafe { core::ptr::addr_of_mut!(MBX_TAKEN).replace(true) };
    if taken {
        return None;
    }
    // Let the 2700G settle after the hand-off before its registers are read.
    // Measured on the real X51v: images that read them at once hung after
    // HaRET's "Jumping to Kernel..." in some runs (rp-probe, rp-usb), while
    // those that first spent ~1 s writing its memory never did (rp-light,
    // rp-probe with start-up colours). Cause not yet known (docs/perf.md
    // open question); half a second after the hand-off is cheap insurance.
    // Counted from the image's start, so work done before this call (card
    // set-up, the USB detach) overlaps it instead of adding to it.
    let elapsed_us = rp_hal_pxa27x::rt::since_start() / 13 * 4;
    if elapsed_us < SETTLE_US {
        rp_hal_pxa27x::rt::delay_us(SETTLE_US - elapsed_us);
    }
    let cs3 = va(region("cs3_2700g")?.base as usize) as u32;
    let regs = NonNull::new((cs3 + MBX_REG_OFFSET) as *mut u32)?;
    // SAFETY: the register block is a device window on CS3 (docs/hw/2700g.md),
    // identity-mapped with the MMU off, and the flag guarantees one owner.
    let regs = unsafe { Mmio::new(regs, MBX_REG_SIZE as usize) };
    // SAFETY: CS3 starts with the 2700G's 16 MiB local memory, which nothing
    // else in RetroPane references; the flag guarantees this is the only slice.
    let vram = unsafe { core::slice::from_raw_parts_mut(cs3 as *mut u8, MBX_VRAM_SIZE as usize) };
    Some((regs, vram))
}

/// How long [`take_mbx`] waits before the 2700G's registers are first used.
const SETTLE_US: u32 = 500_000;

/// The 2700G for crash reporting only, once [`take_mbx`] has handed it out
/// (so the display is known to be set up): its register block, to find the
/// plane actually on screen (`rp_drv_mbx_display::Display::on_screen`;
/// Windows Mobile leaves it at offset 0), and its local memory. The real
/// X51v has no serial port, so faults and panics are drawn on screen.
#[must_use]
pub fn emergency_mbx() -> Option<(Mmio, &'static mut [u8])> {
    // SAFETY: a plain read of the flag; single core.
    if !unsafe { core::ptr::addr_of!(MBX_TAKEN).read() } {
        return None;
    }
    let cs3 = va(region("cs3_2700g")?.base as usize) as u32;
    let regs = NonNull::new((cs3 + MBX_REG_OFFSET) as *mut u32)?;
    // SAFETY: the same register block and 16 MiB of 2700G memory as
    // `take_mbx`. These second handles alias the owner's, which is
    // acceptable only because they're used on paths that never return to
    // the owner (fault and panic handlers), like `rt::emergency_uart`.
    let regs = unsafe { Mmio::new(regs, MBX_REG_SIZE as usize) };
    // SAFETY: as above.
    let vram = unsafe { core::slice::from_raw_parts_mut(cs3 as *mut u8, MBX_VRAM_SIZE as usize) };
    Some((regs, vram))
}

/// Where a plane the display scans out can start, as far as the beacon
/// needs to know: offset 0 (what Windows Mobile leaves, confirmed on the
/// owner's X51v) or the conventional `0x60000` (docs/hw/2700g.md). The
/// first MiB covers a 480x640 RGB565 plane at either.
const BEACON_BYTES: usize = 1 << 20;

/// Bring-up beacon: fills the start of the 2700G's local memory (where a
/// scanned-out plane lies, [`BEACON_BYTES`]) with alternating 32 KiB bands
/// of `a` and `b` (RGB565; equal for a solid colour), so it shows without
/// knowing the plane's layout, before any display setup and with the MMU on
/// or off. It once filled all 16 MiB: 64 MiB of slow uncached writes over a
/// loader's four steps, most of its boot time (530 of 742 ms, emulator).
/// Diagnostic only: like [`emergency_mbx`], it aliases the memory
/// [`take_mbx`] hands out, so callers use it only where nothing else draws.
pub fn beacon(a: u16, b: u16) {
    let Some(r) = region("cs3_2700g") else {
        return;
    };
    let base = va(r.base as usize) as *mut u32;
    let (wa, wb) = (u32::from(a) * 0x1_0001, u32::from(b) * 0x1_0001);
    for i in 0..BEACON_BYTES / 4 {
        let w = if (i >> 13) & 1 == 0 { wa } else { wb };
        // SAFETY: CS3 starts with the 2700G's 16 MiB local memory (mapped
        // identity with the MMU off, write-combining under the bring-up map);
        // `i` stays inside it. See above for the aliasing.
        unsafe { base.add(i).write_volatile(w) };
    }
}

/// Hands out the board CPLD's register window once.
pub fn take_cpld() -> Option<Mmio> {
    // SAFETY: single core, IRQs masked by `_start` and not re-enabled yet.
    let taken = unsafe { core::ptr::addr_of_mut!(CPLD_TAKEN).replace(true) };
    if taken {
        return None;
    }
    let base = NonNull::new(va(region("cs4_cpld")?.base as usize) as *mut u32)?;
    // SAFETY: CS4 holds the CPLD's 0x20-byte register window (docs/hw/cpld.md),
    // identity-mapped with the MMU off; the flag guarantees one owner.
    Some(unsafe { Mmio::new(base, 0x20) })
}

/// CompactFlash socket (PXA27x PC Card socket 1, docs/hw/cf.md): offsets of
/// its attribute and common memory windows in the socket's region.
const CF_ATTR: usize = 0x0800_0000;
const CF_COMMON: usize = 0x0C00_0000;
/// Memory controller (docs/hw/cf.md): `MECR`, and socket 1's timings.
const MECR: usize = 0x14;
const MCMEM1: usize = 0x2C;
const MCATT1: usize = 0x34;
const MCIO1: usize = 0x3C;

/// Hands out the CompactFlash socket's attribute and common memory windows
/// once (enough for the CIS, the configuration registers and the ATA task
/// file), with the socket enabled (`MECR`: two sockets, card inserted) and
/// timed as Windows Mobile times it (the probe's readings). Whether a card
/// is in the slot is GPIO13's to say (low = in); touch the windows only
/// then.
pub fn take_cf() -> Option<(Mmio, Mmio)> {
    // SAFETY: single core, IRQs masked by `_start` and not re-enabled yet.
    let taken = unsafe { core::ptr::addr_of_mut!(CF_TAKEN).replace(true) };
    if taken {
        return None;
    }
    let mc = va(region("memory_ctrl")?.base as usize) as *mut u32;
    for (reg, value) in [
        (MCMEM1, 0x0002_0508),
        (MCATT1, 0x0002_0E08),
        (MCIO1, 0x0002_0510),
        (MECR, 0x0000_0003),
    ] {
        // SAFETY: memory controller registers (PXA27x memory map,
        // identity-mapped device memory); these four are the PC Card
        // socket's and are only written here, under the flag above.
        unsafe { mc.byte_add(reg).write_volatile(value) };
    }
    let base = va(region("pcmcia1_cf")?.base as usize);
    let attr = NonNull::new((base + CF_ATTR) as *mut u32)?;
    let common = NonNull::new((base + CF_COMMON) as *mut u32)?;
    // SAFETY: the socket's attribute and common memory windows (64 MiB
    // each; 4 KiB handed out), identity-mapped device memory; the flag
    // guarantees one owner.
    Some(unsafe { (Mmio::new(attr, 0x1000), Mmio::new(common, 0x1000)) })
}

/// Where every image is loaded and started (HaRET's `ramaddr` + 32 KiB).
pub const IMAGE_BASE: usize = 0xA800_8000;
/// The most an image may occupy: up to the reserved vector page (boot/link.x).
pub const IMAGE_MAX: usize = 0xA80F_0000 - IMAGE_BASE;
/// The resident loader's home (ADR-0012 U3): 1 MiB reserved at 4 MiB into
/// SDRAM, clear of images, the scratch region and the kernel's RAM pool.
pub const LOADER_BASE: usize = 0xA840_0000;
/// Size of the loader's reservation (code, data, stacks).
pub const LOADER_SIZE: usize = 1024 * 1024;

static mut IMAGE_AREA_TAKEN: bool = false;

/// Hands out the image area once, for the resident loader to receive the
/// next image into. `None` when called from code that is itself running in
/// that area (any image but the loader), which would overwrite itself.
pub fn take_image_area() -> Option<&'static mut [u8]> {
    let here = take_image_area as *const () as usize;
    if (IMAGE_BASE..IMAGE_BASE + IMAGE_MAX).contains(&here) {
        return None;
    }
    // SAFETY: single core, IRQs masked by `_start` and not re-enabled yet.
    let taken = unsafe { core::ptr::addr_of_mut!(IMAGE_AREA_TAKEN).replace(true) };
    if taken {
        return None;
    }
    // SAFETY: IMAGE_BASE..+IMAGE_MAX is SDRAM (identity-mapped) that holds
    // no code or data of the caller (checked above: it runs elsewhere), and
    // the flag makes this the only slice.
    Some(unsafe { core::slice::from_raw_parts_mut(IMAGE_BASE as *mut u8, IMAGE_MAX) })
}

/// The run log (ADR-0012): what an image started by the loader printed,
/// kept in RAM for the loader to send to the PC when the image returns.
/// 64 KiB reserved right after the loader's home.
pub const LOG_BASE: usize = 0xA850_0000;
/// Size of the run log reservation, header included.
pub const LOG_SIZE: usize = 64 * 1024;
const LOG_MAGIC: u32 = u32::from_le_bytes(*b"RPLG");
/// Header words: magic, bytes used, flags (bit 0 overflow, bit 1 the image
/// returned to the loader, bit 2 it crashed), !magic.
const LOG_HEADER: usize = 16;
const LOG_CAPACITY: usize = LOG_SIZE - LOG_HEADER;

fn log_word(i: usize) -> *mut u32 {
    (LOG_BASE + 4 * i) as *mut u32
}

/// The log header, if a loader started this run (magic and its complement
/// intact, length in range); otherwise `None` and logging is off.
fn log_used() -> Option<usize> {
    // SAFETY: LOG_BASE..+LOG_SIZE is SDRAM reserved for the run log
    // (boot/link.x), identity-mapped; reads of plain words.
    let (magic, used, check) = unsafe {
        (
            log_word(0).read_volatile(),
            log_word(1).read_volatile() as usize,
            log_word(3).read_volatile(),
        )
    };
    (magic == LOG_MAGIC && check == !LOG_MAGIC && used <= LOG_CAPACITY).then_some(used)
}

/// Starts an empty run log (the loader, just before starting an image).
#[inline(never)]
pub fn log_start() {
    // SAFETY: the run log's reserved header words (see `log_used`).
    unsafe {
        log_word(0).write_volatile(LOG_MAGIC);
        log_word(1).write_volatile(0);
        log_word(2).write_volatile(0);
        log_word(3).write_volatile(!LOG_MAGIC);
    }
}

/// Appends to the run log, if this run has one. Past the end, bytes are
/// dropped and the overflow flag is set (reported by the loader: never
/// silently).
#[inline(never)]
pub fn log_write(data: &[u8]) {
    let Some(used) = log_used() else { return };
    let n = data.len().min(LOG_CAPACITY - used);
    // SAFETY: LOG_BASE + LOG_HEADER + used.. stays inside the reservation
    // (n is clamped to the capacity left); single core, so no other writer.
    unsafe {
        let dst = (LOG_BASE + LOG_HEADER + used) as *mut u8;
        core::ptr::copy_nonoverlapping(data.as_ptr(), dst, n);
        log_word(1).write_volatile((used + n) as u32);
        if n < data.len() {
            log_word(2).write_volatile(log_word(2).read_volatile() | 1);
        }
    }
}

/// A `core::fmt::Write` sink for [`log_write`].
pub struct RunLog;

impl core::fmt::Write for RunLog {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        log_write(s.as_bytes());
        Ok(())
    }
}

/// The previous run, as the loader finds it on start.
pub struct PreviousRun {
    /// What the image printed.
    pub log: &'static [u8],
    /// Output was dropped (the log was full).
    pub overflow: bool,
    /// The image came back by itself (finished or crashed); if not, it hung
    /// and the watchdog (or a manual reset) brought the device back.
    pub returned: bool,
    /// It came back because it crashed (a fault or a panic).
    pub crashed: bool,
}

/// Takes the previous run's log (the loader, on start). The log is closed,
/// so it's reported once.
pub fn log_take() -> Option<PreviousRun> {
    let used = log_used()?;
    // SAFETY: reading the run log's header and its `used` bytes, all inside
    // the reservation; closing it (magic cleared) makes this the only taker.
    unsafe {
        let flags = log_word(2).read_volatile();
        log_word(0).write_volatile(0);
        let log = core::slice::from_raw_parts((LOG_BASE + LOG_HEADER) as *const u8, used);
        Some(PreviousRun {
            log,
            overflow: flags & 1 != 0,
            returned: flags & 2 != 0,
            crashed: flags & 4 != 0,
        })
    }
}

/// Whether the resident loader is in memory and started this run.
#[must_use]
#[inline(never)]
pub fn loader_present() -> bool {
    // SAFETY: reads 8 bytes of the loader's reserved home.
    let sig = unsafe { core::ptr::read_volatile((LOADER_BASE + 4) as *const [u8; 8]) };
    sig == rp_hal_pxa27x::handoff::LOADER_SIGNATURE && log_used().is_some()
}

/// Ends this run: back to the resident loader if it started it (which then
/// sends the run log to the PC); otherwise returns, and the caller halts.
#[inline(never)]
pub fn return_to_loader() {
    if loader_present() {
        // SAFETY: the run log's flags word (valid: checked just above).
        unsafe { log_word(2).write_volatile(log_word(2).read_volatile() | 2) };
        rp_hal_pxa27x::handoff::launch(LOADER_BASE as u32);
    }
}

/// A crash (fault or panic) in an image the loader started: the message
/// into the run log, the run marked as crashed, and straight back to the
/// loader, which sends the report to the PC and keeps it on the card. No
/// watchdog reset, so no trip through Dell's factory-restore prompt.
/// Returns only when no loader started this run (the caller then halts).
#[inline(never)]
pub fn crash_to_loader(msg: core::fmt::Arguments) {
    use core::fmt::Write;
    let _ = writeln!(RunLog, "{msg}");
    if loader_present() {
        // SAFETY: the run log's flags word (valid: checked just above).
        unsafe { log_word(2).write_volatile(log_word(2).read_volatile() | 4) };
        return_to_loader();
    }
}

/// Start of the SDRAM scratch region handed out by [`take_scratch`]: 8 MiB into
/// SDRAM, far above bare-metal images (linked at `0xA8008000`, a few tens of KiB).
pub const SCRATCH_BASE: usize = 0xA880_0000;
/// Size of the scratch region.
pub const SCRATCH_SIZE: usize = 1024 * 1024;

/// Hands out 1 MiB of otherwise unused SDRAM once, for buffers in bare-metal
/// images (benchmarks, bring-up) that shouldn't grow the image itself.
pub fn take_scratch() -> Option<&'static mut [u8]> {
    // SAFETY: single core, IRQs masked by `_start` and not re-enabled yet.
    let taken = unsafe { core::ptr::addr_of_mut!(SCRATCH_TAKEN).replace(true) };
    if taken {
        return None;
    }
    // SAFETY: SCRATCH_BASE..+SCRATCH_SIZE is SDRAM (64 MiB from 0xA8000000)
    // that no bare-metal image's sections or stack reach (see boot/link.x),
    // identity-mapped with the MMU off; the flag guarantees one owner.
    Some(unsafe { core::slice::from_raw_parts_mut(SCRATCH_BASE as *mut u8, SCRATCH_SIZE) })
}

/// Start of the RAM pool handed out by [`take_ram_pool`]: SDRAM from 16 MiB
/// up (above the scratch region), 16 KiB aligned so first-level page tables
/// can be carved from it.
pub const RAM_POOL_BASE: usize = 0xA900_0000;
/// Size of the RAM pool: the top 48 MiB of SDRAM.
pub const RAM_POOL_SIZE: usize = 48 * 1024 * 1024;

static mut POOL_TAKEN: bool = false;

/// Hands out the RAM pool once, as words: the kernel's physical frames
/// (page tables, process memory). Word-typed so page-table entries can be
/// read and written without casts.
pub fn take_ram_pool() -> Option<&'static mut [u32]> {
    // SAFETY: single core; called from main code with IRQs masked or before
    // any interrupt handler touches the flag.
    let taken = unsafe { core::ptr::addr_of_mut!(POOL_TAKEN).replace(true) };
    if taken {
        return None;
    }
    // SAFETY: RAM_POOL_BASE..+RAM_POOL_SIZE is SDRAM (docs/hw/memory-map.md)
    // that no image section, stack or other handout reaches (images live in
    // the first MiB, scratch at 8 MiB); word aligned; identity-mapped; the
    // flag guarantees one owner.
    Some(unsafe { core::slice::from_raw_parts_mut(RAM_POOL_BASE as *mut u32, RAM_POOL_SIZE / 4) })
}
