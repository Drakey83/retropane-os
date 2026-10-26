//! The tile accelerator's set-up (audit C81, `rp-tabench` runs 5-8): the
//! parameter memory manager is set up once, after a core reset, in a
//! parameter memory of the TA's own, and never again between scenes.
//!
//! Measured on the X51v: set up again before every frame, about half the
//! TA's renders hung the 3D stage (runs 5 and 6), whatever else was done;
//! set up once, 148 of 150 drew right (run 7), the two that hung each the
//! first after binner frames rendered into the same parameter memory; with
//! no binner before them, 200 of 200 set-ups drew their first frames right
//! (run 8). So [`set_up`] is the only way to a [`Ready`], which
//! [`super::ta::Feed::start`] needs; it refuses a parameter memory that
//! overlaps any region the caller keeps for something else (the CPU
//! binner's), and [`Ready::recover`] is the one way to set it up again,
//! after a render that hung.

use rp_drv_blit2d::{MBX, reg, reset_core};
use rp_hal_api::RegisterBus;
use rp_hal_mem::store32_volatile;

use super::ta_feed::Clock;
use super::{PM_REACH, PM_READY, pm_init_table};

/// The TA's parameter memory: its pages and the manager's 16 KiB block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParamMemory {
    /// The memory (4 KiB aligned; its first 4 KiB the manager's header).
    pub at: u32,
    /// Its size in bytes.
    pub size: u32,
    /// The manager's block (16 KiB aligned, 16 KiB, zeroed at set-up).
    pub table: u32,
}

/// The manager's block's size.
const TABLE_BYTES: u32 = 16 * 1024;

/// Why [`set_up`] refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetUpError {
    /// Misaligned, empty, or past the manager's 8 MiB reach
    /// ([`PM_REACH`]: its pages alias lower beyond it).
    BadPlace,
    /// The parameter memory or the block overlaps a region kept for
    /// something else (the binner's parameter memory: run 7's hangs).
    Overlap,
    /// The manager never said ready (status bit 6).
    NotReady,
}

/// Proof that the manager is set up for the TA; [`super::ta::Feed::start`]
/// takes it. Made only by [`set_up`].
#[derive(Debug)]
pub struct Ready {
    pm: ParamMemory,
}

impl Ready {
    /// The parameter memory it was set up with (the 3D stage's `0x60C`).
    #[must_use]
    pub const fn param_memory(&self) -> ParamMemory {
        self.pm
    }

    /// After a render that hung (and only then): the core reset and the
    /// manager set up again in the same place.
    ///
    /// # Errors
    ///
    /// [`SetUpError::NotReady`] if the manager doesn't say ready.
    pub fn recover<B: RegisterBus>(
        &mut self,
        regs: &mut B,
        vram: &mut [u8],
        delay_us: impl FnMut(u32),
        drain: fn(),
        clock: Clock,
    ) -> Result<(), SetUpError> {
        bring_up(regs, vram, self.pm, delay_us, drain, clock)
    }

    /// For a test image whose own proven sequence already set the manager
    /// up at `pm` after a core reset (`rp-glesfeat`, kept word for word
    /// with its device runs): the token without a second set-up.
    #[must_use]
    pub const fn set_up_by_caller(pm: ParamMemory) -> Self {
        Self { pm }
    }
}

fn overlaps(a: (u32, u32), b: (u32, u32)) -> bool {
    a.0 < b.0.saturating_add(b.1) && b.0 < a.0.saturating_add(a.1)
}

/// The TA's one set-up: `pm` checked (aligned, within the manager's reach,
/// clear of every `(start, bytes)` in `keep`), the core reset, the
/// manager's block zeroed in `vram` and the manager set up, its ready bit
/// waited for (100 ms at most). `drain` empties the CPU's write buffer
/// between the block's zeroes and the manager's start (the zeroes must be
/// in memory before the manager reads them).
///
/// # Errors
///
/// [`SetUpError`]: the place refused, or the manager not ready.
pub fn set_up<B: RegisterBus>(
    regs: &mut B,
    vram: &mut [u8],
    pm: ParamMemory,
    keep: &[(u32, u32)],
    delay_us: impl FnMut(u32),
    drain: fn(),
    clock: Clock,
) -> Result<Ready, SetUpError> {
    let aligned = pm.at.is_multiple_of(0x1000)
        && pm.table.is_multiple_of(TABLE_BYTES)
        && pm.size >= 0x2000
        && pm.size.is_multiple_of(0x1000);
    let within = (pm.at as usize).saturating_add(pm.size as usize) <= PM_REACH;
    if !aligned || !within {
        return Err(SetUpError::BadPlace);
    }
    let mine = [(pm.at, pm.size), (pm.table, TABLE_BYTES)];
    if overlaps(mine[0], mine[1]) || keep.iter().any(|&k| mine.iter().any(|&m| overlaps(m, k))) {
        return Err(SetUpError::Overlap);
    }
    bring_up(regs, vram, pm, delay_us, drain, clock)?;
    Ok(Ready { pm })
}

/// The core reset, the block zeroed, the manager set up, its ready waited.
fn bring_up<B: RegisterBus>(
    regs: &mut B,
    vram: &mut [u8],
    pm: ParamMemory,
    delay_us: impl FnMut(u32),
    drain: fn(),
    clock: Clock,
) -> Result<(), SetUpError> {
    reset_core(regs, delay_us);
    for k in 0..(TABLE_BYTES / 4) as usize {
        if store32_volatile(vram, pm.table as usize + 4 * k, 0).is_none() {
            return Err(SetUpError::BadPlace);
        }
    }
    drain();
    let en = regs.read32(MBX + reg::ENABLE);
    regs.write32(MBX + reg::ENABLE, en | PM_READY);
    regs.write32(MBX + reg::CLEAR, PM_READY);
    for (off, v) in pm_init_table(pm.at, pm.size, pm.table) {
        regs.write32(MBX + off, v);
    }
    let t0 = (clock.now)();
    let mut ready = false;
    loop {
        if regs.read32(MBX + reg::STATUS) & PM_READY != 0 {
            ready = true;
            break;
        }
        let t = u64::from((clock.now)().wrapping_sub(t0));
        if t * 1000 / u64::from(clock.ticks_per_ms.max(1)) > 100_000 {
            break;
        }
    }
    regs.write32(MBX + reg::CLEAR, PM_READY);
    regs.write32(MBX + reg::ENABLE, en);
    if ready {
        Ok(())
    } else {
        Err(SetUpError::NotReady)
    }
}
