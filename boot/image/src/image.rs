//! The device side: the crash path, the panic handler and the end.

use core::fmt::Write;

use rp_drv_mbx_display::crash::emergency_console;
use rp_drv_pxa_uart::PxaUart;
use rp_hal_pxa27x::rt::{self, Global};
use rp_hal_x51v::devices;

/// How many crashes this run has started ([`super::steps`]).
static DEPTH: Global<u32> = Global::new(0);
/// The image's put-back step ([`on_crash`]).
static PUT_BACK: Global<Option<fn()>> = Global::new(None);
/// The image's extra crash line ([`crash_note`]).
static NOTE: Global<Option<fn(&mut dyn Write) -> core::fmt::Result>> = Global::new(None);
/// The crash box's writer, set by [`take_mbx`]: only an image that takes
/// the display links the box's console and font (the kernel and others
/// with no screen of their own don't carry them).
static SCREEN: Global<Option<fn(&dyn Fn(&mut dyn Write)) -> bool>> = Global::new(None);

/// Installs the crash path as the fault handler. [`entry!`](crate::entry)
/// calls it before the image's `main`.
pub fn install() {
    rt::set_fault_handler(fault);
}

/// Sets a step to run first on a crash: what the image changed that must
/// not outlive it (say, the clocks it set). It runs on the first crash only:
/// if it faults, the report goes out without it.
pub fn on_crash(put_back: fn()) {
    PUT_BACK.with(|p| *p = Some(put_back));
}

/// Sets a line written after every crash's own line, to every place the
/// crash goes (say, the kernel's reason for a page fault). Skipped if the
/// first try at the report faulted (it may read what faulted).
pub fn crash_note(note: fn(&mut dyn Write) -> core::fmt::Result) {
    NOTE.with(|n| *n = Some(note));
}

/// Takes the 2700G (registers and local memory) for the image, as
/// `devices::take_mbx`, and turns the crash box on: from here a crash also
/// shows on screen. Images take the display only this way (`cargo xtask
/// lint`), so none that has a screen can go without the box.
pub fn take_mbx() -> Option<(rp_hal_pxa27x::Mmio, &'static mut [u8])> {
    SCREEN.with(|s| *s = Some(screen_box));
    devices::take_mbx()
}

/// Writes the crash box on the plane on screen; false if the display isn't
/// up (nothing taken yet).
fn screen_box(lines: &dyn Fn(&mut dyn Write)) -> bool {
    let Some((regs, vram)) = devices::emergency_mbx() else {
        return false;
    };
    let mut c = emergency_console(
        regs,
        vram,
        crate::BOX_AT,
        crate::BOX_SIZE,
        crate::BOX_SCALE,
        crate::BOX_COLORS,
    );
    lines(&mut c);
    true
}

/// A fault's report (`FAULT <kind> at <address> word .. spsr ..`) as a
/// crash: the runtime's fault handler, and a kernel's for a fault nothing
/// else handles. The word is read only on the first crash: `try_read`
/// can't fault out (its abort resumes at a fixup), but if it ever did, a
/// second report goes without it instead of reading again.
pub fn fault(kind: rt::Fault, addr: u32) -> ! {
    let first = DEPTH.with(|d| *d == 0);
    let detail = crate::FaultDetail {
        word: if first {
            rt::try_read(addr as usize)
        } else {
            None
        },
        spsr: rt::fault_spsr(),
    };
    report(Line::Fault(kind, addr, detail))
}

/// A crash with a message: `rp_image::crash(format_args!(...))`.
#[inline(never)]
pub fn crash(msg: core::fmt::Arguments) -> ! {
    report(Line::Message(msg))
}

/// `v`'s value, or a crash naming `what` and the caller's line: for a call
/// that refuses what it was given (a 2D command whose rectangle the engine
/// can't draw, `rp_drv_blit2d::drawable`, quality audit C47) in a test
/// image, where carrying on would test something else.
#[track_caller]
pub fn need<T>(v: Option<T>, what: &str) -> T {
    match v {
        Some(v) => v,
        None => {
            let at = core::panic::Location::caller();
            crash(format_args!(
                "{what} refused at {}:{}",
                at.file(),
                at.line()
            ))
        }
    }
}

/// A setup step that failed: `SETUP-FAIL <what>` reported as a crash, never a
/// silent stop (which the loader would see as a hang).
#[inline(never)]
pub fn fail(what: &str) -> ! {
    report(Line::Failed(what))
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    report(Line::Panic(info))
}

/// A crash's own line. One type (not a closure per kind): the line's code
/// is in the image once, not once a kind plus a call shim each.
#[derive(Clone, Copy)]
enum Line<'a> {
    Fault(rt::Fault, u32, crate::FaultDetail),
    Message(core::fmt::Arguments<'a>),
    Failed(&'a str),
    Panic(&'a core::panic::PanicInfo<'a>),
}

impl Line<'_> {
    fn write(self, w: &mut dyn Write) -> core::fmt::Result {
        match self {
            Self::Fault(kind, addr, detail) => crate::fault_line(w, kind.name(), addr, detail),
            Self::Message(msg) => w.write_fmt(msg),
            Self::Failed(what) => {
                w.write_str("SETUP-FAIL ")?;
                w.write_str(what)
            }
            Self::Panic(info) => {
                w.write_str("PANIC ")?;
                write!(w, "{}", info.message())?;
                if let Some(l) = info.location() {
                    w.write_str(" at ")?;
                    w.write_str(l.file())?;
                    write!(w, ":{}", l.line())?;
                }
                Ok(())
            }
        }
    }
}

/// The crash path, in its fixed order (the crate's docs).
fn report(what: Line) -> ! {
    let depth = DEPTH.with(|d| {
        let was = *d;
        *d = d.saturating_add(1);
        was
    });
    let steps = crate::steps(depth);
    let note = if steps.note { NOTE.with(|n| *n) } else { None };
    let both = |w: &mut dyn Write| {
        let mut lines = || -> core::fmt::Result {
            what.write(w)?;
            w.write_char('\n')?;
            if let Some(note) = note {
                note(w)?;
                w.write_char('\n')?;
            }
            Ok(())
        };
        // Ignored on purpose: on the crash path there is nowhere left to
        // report a failed write, and each place (serial, screen, run log) is
        // written on its own, so one failing never silences the others.
        let _ = lines();
    };
    if steps.put_back
        && let Some(put_back) = PUT_BACK.with(|p| *p)
    {
        put_back();
    }
    if steps.report {
        let mut uart = PxaUart::new(rt::emergency_uart());
        // The report starts on a line of its own (ignored as in `both`).
        let _ = uart.write_char('\n');
        both(&mut uart);
    }
    let shown = steps.screen && SCREEN.with(|s| *s).is_some_and(|screen| screen(&both));
    if steps.report {
        both(&mut devices::RunLog);
        if shown && devices::loader_present() {
            rt::delay_us(crate::BOX_HOLD_US);
        }
        devices::crash_return();
    }
    stop()
}

/// The clean end: the deepest stack into the run log, then back to the
/// loader (which sends the log to the PC). Without a loader, stops here.
pub fn finish() -> ! {
    devices::return_to_loader();
    stop()
}

/// Stops for good (no loader to go back to). Spins rather than idles: after
/// a fault the core is in abort mode with IRQs masked (the exception entry,
/// hal/pxa27x/src/rt.rs), so no interrupt would wake it.
pub fn stop() -> ! {
    loop {
        core::hint::spin_loop();
    }
}
