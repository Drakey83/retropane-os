//! Safe, single-owner access to X51v board devices for bare-metal images.
//!
//! Like `rp_hal_pxa27x::rt::take_ffuart`, each device can be taken once, before
//! interrupts are enabled, with the MMU off (physical = virtual).

use core::ptr::NonNull;
use rp_hal_pxa27x::devmap::va;
use rp_hal_pxa27x::diag;

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
    // The registers are read once nothing the previous owner left running
    // still moves data. History (docs/hw/2700g.md "After the hand-off"):
    // images that read them at once hung after HaRET's hand-off in some
    // runs, and a fixed 500 ms wait went in; 21 minutes later the cause was
    // found (images hung at random early points, not only here, only while
    // docked: Windows Mobile's USB DMA still writing RAM) and `_start` began
    // stopping all DMA and the UDC. The fixed wait stayed and every image
    // paid it. What remains to wait for is a channel finishing the
    // transfer in flight after `RUN` is cleared: microseconds, bounded.
    let stopped = rp_hal_pxa27x::rt::wait_handoff_dma(DMA_STOP_TIMEOUT_US);
    // SAFETY: single core; written once here, before anyone reads it.
    unsafe {
        core::ptr::addr_of_mut!(MBX_HANDOFF).write(Some(MbxHandoff {
            found: rp_hal_pxa27x::rt::handoff(),
            stopped,
            at: rp_hal_pxa27x::rt::since_start(),
        }));
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

/// The longest [`take_mbx`] waits for the hand-off's DMA channels to stop.
/// With `RUN` cleared a channel stops once the transfer in flight is done
/// (a burst of at most 32 bytes); 10 ms is far more than that takes on any
/// peripheral. A channel still running after it won't stop by waiting: the
/// wait gives up, and [`mbx_handoff`] says so.
const DMA_STOP_TIMEOUT_US: u32 = 10_000;

/// What [`take_mbx`] found and waited for at the hand-off.
#[derive(Clone, Copy, Debug)]
pub struct MbxHandoff {
    /// What `_start` found running.
    pub found: rp_hal_pxa27x::rt::Handoff,
    /// How waiting for those DMA channels to stop went.
    pub stopped: rp_hal_pxa27x::rt::DmaStopped,
    /// When the 2700G was handed out (OS timer ticks since the image's start).
    pub at: u32,
}

static mut MBX_HANDOFF: Option<MbxHandoff> = None;

/// What [`take_mbx`] found at the hand-off, once it has run (boot-time and
/// hand-off diagnostics: the loader's `handoff` command).
#[must_use]
pub fn mbx_handoff() -> Option<MbxHandoff> {
    // SAFETY: a plain read; written once by `take_mbx` before any reader.
    unsafe { core::ptr::addr_of!(MBX_HANDOFF).read() }
}

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

/// The CPLD's shared shadow table in RAM (`rp_drv_cpld::handover`):
/// Windows Mobile's at HaRET's hand-off, RetroPane's from the first program
/// that adopts it, shared by the loader and every image it starts.
const CPLD_TABLE: usize = 0xA800_1320;
/// The table's size in words (`rp_drv_cpld::handover::WORDS`).
pub const CPLD_TABLE_WORDS: usize = 10;

/// `MSC2` as Windows Mobile leaves it: CS4 (the CPLD) `0x7FFC`, the slowest
/// VLIO timing the PXA27x has (probe 2026-10-11).
// inherited: R8, Windows Mobile's MSC2 read by the probe (probe-2026-10-11.md)
pub const CPLD_TIMING_WM: (u32, u32, u32) = (15, 15, 7);

/// RetroPane's CS4 timing for the CPLD (the owner's ruling, 2026-10-24,
/// reminder R8): `RDF` 6, `RDN` and `RRR` as Windows Mobile's. On the X51v
/// (`rp-cpldbus`) a CPLD read takes 215 ns from `RDF` 6 down against 327 at
/// Windows Mobile's 15 (1.52x), and no shorter `RDF`, `RDN` or `RRR` reads
/// faster; reads and the backlight's writes were right to `RDF` 3, `RDN` 2,
/// `RRR` 0, so this keeps the most margin for other X51vs at the full speed.
pub const CPLD_TIMING: (u32, u32, u32) = (6, 15, 7);
// Both timings are legal for CS4 as Windows Mobile sets it up (16-bit
// VLIO), checked as the HAL is built.
const _: () =
    assert!(diag::cs4_vlio(0x7FF4_7FFC, CPLD_TIMING.0, CPLD_TIMING.1, CPLD_TIMING.2).is_some());
const _: () = assert!(matches!(
    diag::cs4_vlio(
        0x7FF4_7FFC,
        CPLD_TIMING_WM.0,
        CPLD_TIMING_WM.1,
        CPLD_TIMING_WM.2
    ),
    Some(0x7FF4_7FFC)
));

/// Hands out the board CPLD once: its register window and the shared table
/// of what is latched in it (its registers don't read back; pass both to
/// `rp_drv_cpld::handover::Table::adopt` and `Cpld::new`).
pub fn take_cpld() -> Option<(Mmio, &'static mut [u32; CPLD_TABLE_WORDS])> {
    // SAFETY: single core, IRQs masked by `_start` and not re-enabled yet.
    let taken = unsafe { core::ptr::addr_of_mut!(CPLD_TAKEN).replace(true) };
    if taken {
        return None;
    }
    let base = NonNull::new(va(region("cs4_cpld")?.base as usize) as *mut u32)?;
    // The CPLD's bus at RetroPane's timing, from Windows Mobile's or ours
    // only: anything else (a test that moved it) is left as found.
    let m = diag::msc2();
    let with = |(rdf, rdn, rrr)| diag::cs4_vlio(m, rdf, rdn, rrr);
    if [with(CPLD_TIMING_WM), with(CPLD_TIMING)].contains(&Some(m)) {
        let (rdf, rdn, rrr) = CPLD_TIMING;
        let _ = diag::set_cs4_vlio(rdf, rdn, rrr);
    }
    // SAFETY: CS4 holds the CPLD's 0x20-byte register window (docs/hw/cpld.md),
    // identity-mapped with the MMU off; the flag guarantees one owner.
    let regs = unsafe { Mmio::new(base, 0x20) };
    // SAFETY: 40 bytes of SDRAM that RetroPane reserves for this table
    // (below every image at 0xA8008000, above HaRET's boot tags; docs/hw/
    // memory-map.md), identity-mapped, word-aligned; the flag makes this
    // the only reference in this program. Programs run one at a time, and
    // the loader cleans the D-cache before starting the next one.
    let table = unsafe { &mut *(CPLD_TABLE as *mut [u32; CPLD_TABLE_WORDS]) };
    Some((regs, table))
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
/// Windows Mobile's common-memory timing for the socket, what [`take_cf`]
/// sets (`MCMEM1`; docs/research/storage-speed.md A1).
// inherited: R7, Windows Mobile's MCMEM1 read by the probe (storage-speed.md A1)
pub const CF_MEM_TIMING_WM: u32 = 0x0002_0508;

/// The physical address of the CompactFlash socket's common memory (what
/// a DMA engine reads the card's data window from).
#[must_use]
pub fn cf_common_phys() -> Option<u32> {
    Some(region("pcmcia1_cf")?.base + CF_COMMON as u32)
}

/// Hands out the CompactFlash socket's attribute and common memory windows
/// once (enough for the CIS, the configuration registers and the ATA task
/// file), with the socket enabled (`MECR`: two sockets, card inserted) and
/// timed as Windows Mobile times it (the probe's readings). Whether a card
/// is in the slot is GPIO13's to say (low = in); touch the windows only
/// then.
pub fn take_cf() -> Option<(Mmio, Mmio)> {
    // A sector the card log sent is programmed first (quality audit C59).
    if cf_if_untaken().is_some() {
        crate::cardsink::settle();
    }
    // SAFETY: single core, IRQs masked by `_start` and not re-enabled yet.
    let taken = unsafe { core::ptr::addr_of_mut!(CF_TAKEN).replace(true) };
    if taken {
        return None;
    }
    let mc = va(region("memory_ctrl")?.base as usize) as *mut u32;
    for (reg, value) in [
        (MCMEM1, CF_MEM_TIMING_WM),
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

/// The CompactFlash socket's common-memory timing (`MCMEM1`), once this
/// program has taken the socket ([`take_cf`]): what `rp_drv_cf`'s
/// `CfCard::tune` reads.
#[must_use]
pub fn cf_mem_timing() -> Option<u32> {
    let mc = cf_timing_reg()?;
    // SAFETY: `MCMEM1`, a memory controller register (identity-mapped
    // device memory); reading it has no effect.
    Some(unsafe { mc.read_volatile() })
}

/// Sets the CompactFlash socket's common-memory timing (`MCMEM1`, its
/// timing fields and the rest as given), once this program has taken the
/// socket ([`take_cf`]): for `CfCard::tune`, and an image's put-back step
/// ([`CF_MEM_TIMING_WM`]). The attribute and I/O timings are left alone.
pub fn set_cf_mem_timing(value: u32) -> Option<()> {
    let mc = cf_timing_reg()?;
    // SAFETY: `MCMEM1`, written only by the socket's owner (the flag
    // checked in `cf_timing_reg`), on a single core; the socket's own
    // timing, which no other device uses.
    unsafe { mc.write_volatile(value) };
    Some(())
}

/// `MCMEM1`'s address, only after [`take_cf`].
fn cf_timing_reg() -> Option<*mut u32> {
    // SAFETY: a plain read of the flag; single core.
    if !unsafe { core::ptr::addr_of!(CF_TAKEN).read() } {
        return None;
    }
    let mc = va(region("memory_ctrl")?.base as usize) as *mut u32;
    // SAFETY: an offset inside the memory controller's register block.
    Some(unsafe { mc.byte_add(MCMEM1) })
}

/// The CompactFlash socket's common memory (the task file) for the card log
/// (`crate::cardlog`), only while this program hasn't taken them
/// ([`take_cf`]), with the socket enabled as the loader left it and its
/// common-memory timing set to Windows Mobile's each call (the timing CF
/// writes are proven at; the loader may have tuned reads faster). Like
/// `rt::mmc_if_untaken`: no owner exists, each use is one sector write.
pub(crate) fn cf_if_untaken() -> Option<Mmio> {
    // SAFETY: a plain read of the flag; single core.
    if unsafe { core::ptr::addr_of!(CF_TAKEN).read() } {
        return None;
    }
    // The card log writes at Windows Mobile's timing, the one CF writes
    // are proven at, whatever the loader tuned the card's reads to
    // (CfCard::write_at; reminder R7).
    let mc = va(region("memory_ctrl")?.base as usize) as *mut u32;
    // SAFETY: `MCMEM1`, the CF socket's own timing; no owner exists in this
    // program (the flag above), single core.
    unsafe { mc.byte_add(MCMEM1).write_volatile(CF_MEM_TIMING_WM) };
    let base = va(region("pcmcia1_cf")?.base as usize);
    let common = NonNull::new((base + CF_COMMON) as *mut u32)?;
    // SAFETY: as in `take_cf` (its common memory window); the flag
    // (checked above) says no handle from `take_cf` exists in this program.
    Some(unsafe { Mmio::new(common, 0x1000) })
}

/// Power manager `PCFR` and its GPIO reset enable (docs/hw/README.md
/// "Software reset").
const PCFR: usize = 0x00F0_001C;
const PCFR_GPR_EN: u32 = 1 << 4;
/// `GAFR3_L` (GPIO96-111 functions): GPIO99's field is bits 7:6.
const GAFR3_L: usize = 0x00E0_006C;
const GAFR3_L_GPIO99: u32 = 3 << 6;

/// Readies the board's software reset as Windows Mobile's reboot does
/// (docs/hw/README.md "Software reset"): the PXA27x GPIO reset enabled
/// (`PCFR` GPR_EN) and GPIO99 a plain GPIO. The caller then drives GPIO15
/// low and GPIO99 high (outputs), with CPLD `0x08` bit 1 set first.
pub fn arm_board_reset() -> Option<()> {
    let p = va(region("peripherals")?.base as usize) as *mut u32;
    // SAFETY: power manager and GPIO registers (PXA27x memory map,
    // identity-mapped device memory); read-modify-write of two registers
    // on a single core with nothing else changing them meanwhile, right
    // before the board is reset.
    unsafe {
        let pcfr = p.byte_add(PCFR);
        pcfr.write_volatile(pcfr.read_volatile() | PCFR_GPR_EN);
        let gafr = p.byte_add(GAFR3_L);
        gafr.write_volatile(gafr.read_volatile() & !GAFR3_L_GPIO99);
    }
    Some(())
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

/// The run log's flags (bit 0 overflow, 1 returned, 2 crashed, 3 a
/// hand-over), if a run log is there: the loader reads how the last run
/// ended before [`log_take`] closes it (the card log's closing line).
#[must_use]
pub fn log_flags() -> Option<u32> {
    log_used()?;
    // SAFETY: the run log's flags word (valid: checked just above).
    Some(unsafe { log_word(2).read_volatile() })
}

/// Marks the run just started as a hand-over to a new loader (the loader,
/// before starting an image that carries [`LOADER_SIGNATURE`]): the new
/// loader then doesn't report its predecessor's run as a hang (it had: a
/// "did not return" report saved over the card's crashlog.txt, 2026-10-16).
///
/// [`LOADER_SIGNATURE`]: rp_hal_pxa27x::handoff::LOADER_SIGNATURE
#[inline(never)]
pub fn log_mark_handover() {
    if log_used().is_some() {
        // SAFETY: the run log's flags word (valid: checked just above).
        unsafe { log_word(2).write_volatile(log_word(2).read_volatile() | 8) };
    }
}

/// Appends to the run log, if this run has one. Past the end, bytes are
/// dropped and the overflow flag is set (reported by the loader: never
/// silently). With a card log on (`crate::cardlog`, every run the loader
/// starts with a card in), each line also goes onto a card as it ends, and
/// what doesn't fit in the run log still does.
#[inline(never)]
pub fn log_write(data: &[u8]) {
    if log_append(data) {
        crate::cardsink::after_write(data);
    }
}

/// [`log_write`] without the card: the card log's own notes (it is the
/// one writing), and the loader's lines (it syncs the card itself, and so
/// carries none of a program's card writes). Whether the run has a log.
#[inline(never)]
pub fn log_append(data: &[u8]) -> bool {
    let Some(used) = log_used() else { return false };
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
    if n < data.len() {
        crate::cardsink::more(&data[n..]);
    }
    true
}

/// The run log's length so far (0 without one).
pub(crate) fn log_len() -> usize {
    log_used().unwrap_or(0)
}

/// The run log's text so far.
pub(crate) fn log_text() -> &'static [u8] {
    // SAFETY: the run log's `used` bytes, inside the reservation; read-only
    // (appends only add after them).
    unsafe { core::slice::from_raw_parts((LOG_BASE + LOG_HEADER) as *const u8, log_len()) }
}

/// A `core::fmt::Write` sink for [`log_write`].
pub struct RunLog;

impl core::fmt::Write for RunLog {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        log_write(s.as_bytes());
        Ok(())
    }
}

/// [`RunLog`] that also writes the kept log while it's on
/// ([`kept_start`]); a separate sink so images that don't keep a log don't
/// carry its code.
pub struct KeptRunLog;

impl core::fmt::Write for KeptRunLog {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        log_write(s.as_bytes());
        kept_write(s.as_bytes());
        Ok(())
    }
}

/// The internal SRAM's address while the kept log is on ([`kept_start`]),
/// else 0. Single core, written only by the main thread.
static mut KEPT_SRAM: usize = 0;

fn kept_base() -> usize {
    // SAFETY: a plain word read; single core (see KEPT_SRAM).
    unsafe { core::ptr::addr_of!(KEPT_SRAM).read_volatile() }
}

/// Takes the internal SRAM for the kept log ([`crate::kept`]) and reports
/// each copy an earlier run left there: `report(copy, text or why not)`.
/// Nothing is written yet. `false` if the SRAM was already taken.
pub fn kept_open(mut report: impl FnMut(usize, Result<&str, &'static str>)) -> bool {
    let Some(sram) = rp_hal_pxa27x::diag::take_sram() else {
        return false;
    };
    for (k, off) in crate::kept::COPIES.iter().enumerate() {
        report(k, crate::kept::parse(&sram[*off..*off + crate::kept::SIZE]));
    }
    // SAFETY: as in `kept_base`.
    unsafe { core::ptr::addr_of_mut!(KEPT_SRAM).write_volatile(sram.as_mut_ptr() as usize) };
    true
}

static mut KEPT_SPARE_TAKEN: bool = false;

/// While the kept log holds the internal SRAM ([`kept_open`]), the part
/// below its first copy (`kept::COPIES[0]`, 64 KiB), which it never
/// writes: handed out once, as words. `None` without the kept log, or the
/// second time. For staging buffers beside the kept log (the performance
/// gate's T2).
pub fn kept_spare() -> Option<&'static mut [u32]> {
    let base = kept_base() & !1;
    if base == 0 {
        return None;
    }
    // SAFETY: single core; the flag makes this the only slice.
    if unsafe { core::ptr::addr_of_mut!(KEPT_SPARE_TAKEN).replace(true) } {
        return None;
    }
    // SAFETY: SRAM from `base` (word-aligned, mapped by the bring-up map,
    // owned by the kept log) up to its first copy, which nothing else
    // uses: the kept log writes only its copies; the flag hands it out once.
    Some(unsafe { core::slice::from_raw_parts_mut(base as *mut u32, crate::kept::COPIES[0] / 4) })
}

/// Starts an empty kept log in both copies: from here every [`RunLog`]
/// write goes there too ([`KeptRunLog`]'s; each cleaned from the cache to the SRAM at once,
/// so a reset loses nothing written). Only [`KeptRunLog`] writes it.
pub fn kept_start() {
    let base = kept_base();
    if base == 0 {
        return;
    }
    for off in crate::kept::COPIES {
        kept_header(base + off, crate::kept::header(0, 0));
    }
    // Marks the log on (bit 0: the base is 256 KiB aligned).
    // SAFETY: as in `kept_base`.
    unsafe { core::ptr::addr_of_mut!(KEPT_SRAM).write_volatile(base | 1) };
}

/// Ends the kept log and clears both copies (the run came back: the
/// loader has its log).
pub fn kept_stop() {
    let base = kept_base() & !1;
    if base == 0 {
        return;
    }
    // SAFETY: as in `kept_base`.
    unsafe { core::ptr::addr_of_mut!(KEPT_SRAM).write_volatile(base) };
    for off in crate::kept::COPIES {
        kept_header(base + off, [0; 4]);
    }
}

fn kept_header(at: usize, h: [u32; 4]) {
    for (i, w) in h.iter().enumerate() {
        // SAFETY: the copy's header words, inside the SRAM taken by
        // `kept_open` (COPIES and SIZE fit its 256 KiB: host-tested).
        unsafe { ((at + 4 * i) as *mut u32).write_volatile(*w) };
    }
    rp_hal_pxa27x::cpu::clean_range(at as u32, crate::kept::HEADER as u32);
}

fn kept_write(data: &[u8]) {
    let base = kept_base();
    if base & 1 == 0 {
        return;
    }
    let base = base & !1;
    for off in crate::kept::COPIES {
        let at = base + off;
        // SAFETY: the copy's header and `used + n <= CAPACITY` text bytes
        // after it, inside the SRAM taken by `kept_open`.
        let (used, total) = unsafe {
            (
                (at as *const u32).add(1).read_volatile() as usize,
                (at as *const u32).add(2).read_volatile(),
            )
        };
        let n = data.len().min(crate::kept::CAPACITY.saturating_sub(used));
        let dst = at + crate::kept::HEADER + used;
        // SAFETY: as above.
        unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), dst as *mut u8, n) };
        rp_hal_pxa27x::cpu::clean_range(dst as u32, n as u32);
        let total = crate::kept::sum(total, &data[..n]);
        kept_header(at, crate::kept::header(used + n, total));
    }
}

/// The previous run, as the loader finds it on start.
#[derive(Clone, Copy)]
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
    /// The "image" was a new loader, which took over: it never returns, and
    /// that is no hang ([`log_mark_handover`]).
    pub handover: bool,
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
            handover: flags & 8 != 0,
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
    // Every image reports its deepest stack (the class of silent overflows:
    // nothing guards the 16 KiB SVC stack).
    {
        use core::fmt::Write;
        let (used, size) = rp_hal_pxa27x::rt::stack_used();
        let warn = if used >= size {
            " - OVERFLOWED: .bss below it is corrupt"
        } else if used * 4 > size * 3 {
            " - over 75%"
        } else {
            ""
        };
        // The words by write_str, the numbers by integer formatting: no
        // `&str` Display (and its padding code) in every image (ADR-0005
        // rule 8). Ignored results: RunLog's writes can't fail.
        let _ = write!(RunLog, "stack: deepest {used} of {size} bytes");
        let _ = RunLog.write_str(warn);
        let _ = RunLog.write_char('\n');
    }
    // The loader's drivers find the cards ready (quality audit C59).
    crate::cardsink::settle();
    if loader_present() {
        // SAFETY: the run log's flags word (valid: checked just above).
        unsafe { log_word(2).write_volatile(log_word(2).read_volatile() | 2) };
        rp_hal_pxa27x::handoff::launch(LOADER_BASE as u32);
    }
}

/// A crash (fault, panic or failed setup) in an image the loader started,
/// its lines already in the run log: the run marked as crashed and straight
/// back to the loader, which sends the report to the PC and keeps it on the
/// card. No watchdog reset, so no trip through Dell's factory-restore
/// prompt. Returns only when no loader started this run (the caller then
/// stops). Images reach it through `rp-image`'s crash path (quality audit
/// C38), never directly.
#[inline(never)]
pub fn crash_return() {
    if loader_present() {
        // SAFETY: the run log's flags word (valid: checked just above).
        unsafe { log_word(2).write_volatile(log_word(2).read_volatile() | 4) };
        return_to_loader();
    }
}

/// The trace region (item 54): the kernel's trace ring (`rp-kernel-trace`
/// `Persistent`), right after the run log, so it outlives the program like
/// the log: the loader starts it with each program and sends it to the PC
/// afterwards (`cargo xtask trace`), even after a hang and reset.
pub const TRACE_BASE: usize = 0xA851_0000;
/// Size of the trace region (header and 255 records).
pub const TRACE_SIZE: usize = 4096;

static mut TRACE_TAKEN: bool = false;

/// Hands out the trace region once.
pub fn take_trace_area() -> Option<&'static mut [u32]> {
    // SAFETY: single core; called during setup.
    let taken = unsafe { core::ptr::addr_of_mut!(TRACE_TAKEN).replace(true) };
    if taken {
        return None;
    }
    // SAFETY: TRACE_BASE..+TRACE_SIZE is SDRAM reserved for the trace ring
    // (boot/link.x), identity-mapped, word-aligned; the flag makes this the
    // only slice in this program.
    Some(unsafe { core::slice::from_raw_parts_mut(TRACE_BASE as *mut u32, TRACE_SIZE / 4) })
}

/// The board record (quality audit C39, [`crate::board`]): what the
/// resident loader found at its first start, put back after every image.
/// 4 KiB reserved after the card log's state (boot/link.x).
pub const BOARD_BASE: usize = 0xA851_2000;
/// Size of the board record's reservation.
pub const BOARD_SIZE: usize = 4096;
const _: () = assert!(crate::board::WORDS * 4 <= BOARD_SIZE);

/// The board record as kept, if one is there and intact.
#[must_use]
pub fn board_record() -> Option<crate::board::Board> {
    // SAFETY: BOARD_BASE..+BOARD_SIZE is SDRAM reserved for the record
    // (boot/link.x), identity-mapped, word-aligned; read whole.
    let w: [u32; crate::board::WORDS] = core::array::from_fn(|k| unsafe {
        core::ptr::read_volatile((BOARD_BASE + 4 * k) as *const u32)
    });
    crate::board::unpack(&w)
}

/// Keeps `b` as the board record.
pub fn set_board_record(b: &crate::board::Board) {
    for (k, w) in crate::board::pack(b).into_iter().enumerate() {
        // SAFETY: as in `board_record`.
        unsafe { core::ptr::write_volatile((BOARD_BASE + 4 * k) as *mut u32, w) };
    }
}

/// In the loader's `.data`: `true` in the image HaRET loads from the card,
/// `false` once [`loader_first_start`] has run. `_start` zeroes `.bss` only,
/// so on every return from an image (the same copy at the loader's home)
/// it stays `false`; a soft reset or a watchdog reset starts a fresh copy.
#[unsafe(link_section = ".data")]
static mut LOADER_FRESH: bool = true;

/// Whether this is the resident loader's first start since HaRET loaded it
/// (Windows Mobile's board), not a return from an image. True once.
pub fn loader_first_start() -> bool {
    // SAFETY: single core; the loader calls it once, early in `main`.
    unsafe { core::ptr::addr_of_mut!(LOADER_FRESH).replace(false) }
}

/// Start of the SDRAM scratch region handed out by [`take_scratch`]: 8 MiB into
/// SDRAM, far above bare-metal images (linked at `0xA8008000`, a few tens of KiB).
pub const SCRATCH_BASE: usize = 0xA880_0000;
/// Size of the scratch region.
pub const SCRATCH_SIZE: usize = 4 * 1024 * 1024;

/// Hands out 4 MiB of otherwise unused SDRAM once (enough for a full-screen
/// ARGB8888 interface layer), for buffers in bare-metal
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

/// [`take_ram_pool`] as bytes, for test images that need large buffers
/// (storage soaks): the same pool, the same one owner.
pub fn take_ram_pool_bytes() -> Option<&'static mut [u8]> {
    take_ram_pool().map(|w| {
        let len = w.len() * 4;
        // SAFETY: the pool's own words, reborrowed as bytes (u8 has no
        // alignment or validity requirement); `w` is consumed, so this is
        // the one reference.
        unsafe { core::slice::from_raw_parts_mut(w.as_mut_ptr().cast::<u8>(), len) }
    })
}
