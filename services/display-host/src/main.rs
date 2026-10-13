//! The display+input driver host (ADR-0016 decision 5), first driver: the
//! backlight.
//!
//! A supervised user process. The kernel maps it PWM0's registers (its
//! grant, `rp_hal_user::Device::Pwm0`) and grants it the power button's
//! interrupt; nothing else of the hardware is reachable from here.
//!
//! - **Calls** (clients through its port): [`OP_SET`] a level (0-100 %),
//!   [`OP_LEVEL`] the level now; [`OP_TEST_FAULT`] makes this activation
//!   fault (the test kernel's supervision check).
//! - **Its thread** waits for the power button and turns the light off and
//!   back on, as Windows Mobile does.
//!
//! No driver state lives in this process: the level is the PWM's duty
//! register, read back each time. A restarted host (fresh memory) carries
//! on from what the hardware shows, with no flicker and nothing to save.
//!
//! The same image holds the test kernel's display client (thread role
//! [`ROLE_TEST_CLIENT`], in its own process): it sets a level, makes the
//! host fault, opens the restarted host through the service manager and
//! reads the level back.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Set the level (`r2`, percent): answers the level now set.
pub const OP_SET: u32 = 1;
/// The level now.
pub const OP_LEVEL: u32 = 2;
/// Test kernel only: this activation faults.
pub const OP_TEST_FAULT: u32 = 3;
/// An unknown operation's answer.
pub const ERR_OP: u32 = rp_hal_user::ERR | 0x10;

/// Thread roles (`r1` when the kernel starts a thread).
/// The host's own thread (the power button).
pub const ROLE_HOST: u32 = 0;
/// The test kernel's display client.
pub const ROLE_TEST_CLIENT: u32 = 1;
/// Test kernel only: a thread of the host that faults at once (the
/// supervision of a host's own threads, outside any call).
pub const ROLE_TEST_FAULT: u32 = 2;

/// The interrupt line granted to this host: the power button.
pub const LINE_POWER_BUTTON: u32 = 0;

/// Whether a power-button signal is a press, from the pin's level after it
/// (1 = down) and whether it was down at the last signal; and whether it is
/// down now.
///
/// The line is masked from when it fires until the next wait, so an edge
/// in between is lost and the level read may already be the release: a
/// signal is a press if the pin is down now, or if it is up and was up
/// before (a whole tap happened while masked); up after down is the
/// release.
#[must_use]
pub const fn button_press(level: u32, was_down: bool) -> (bool, bool) {
    let down = level == 1;
    (down || !was_down, down)
}

#[cfg(target_os = "none")]
mod program {
    use super::*;
    use rp_drv_pxa_pwm::Backlight;
    use rp_hal_user::{Device, Window};

    /// The test client's handles (installed by the kernel) and the manager's
    /// operation that opens this host.
    const CLIENT_DISPLAY: u32 = 0;
    const CLIENT_MANAGER: u32 = 1;
    const MANAGER_OPEN_DISPLAY: u32 = 3;
    /// The level the test client sets (the test kernel checks it).
    const TEST_LEVEL: u32 = 30;

    fn backlight() -> Backlight<Window> {
        Backlight::new(Window::of(Device::Pwm0))
    }

    /// One call: `op` with argument `arg`.
    fn serve(op: u32, arg: u32) -> u32 {
        match op {
            OP_SET => {
                let mut b = backlight();
                b.set(arg.min(100));
                b.level()
            }
            OP_LEVEL => backlight().level(),
            OP_TEST_FAULT => panic!("test fault"),
            _ => ERR_OP,
        }
    }

    /// The host's thread: the power button turns the light off and back on
    /// ([`button_press`]).
    fn host() -> ! {
        let mut restore = 100;
        let mut was_down = false;
        loop {
            let level = rp_hal_user::irq_wait(LINE_POWER_BUTTON);
            if rp_hal_user::is_err(level) {
                continue;
            }
            let (pressed, down) = button_press(level, was_down);
            was_down = down;
            if !pressed {
                continue;
            }
            let mut b = backlight();
            let now = b.level();
            if now > 0 {
                restore = now;
                b.set(0);
            } else {
                b.set(restore);
            }
        }
    }

    /// The test kernel's display client. It may not wait for the host's
    /// interrupt (granted to the host alone): if it could, it reports a
    /// level that fails the kernel's check.
    fn test_client() -> ! {
        use rp_hal_user::call;
        let refused = rp_hal_user::is_err(rp_hal_user::irq_wait(LINE_POWER_BUTTON));
        let set = if refused {
            call(CLIENT_DISPLAY, OP_SET, TEST_LEVEL, 0)
        } else {
            ERR_OP
        };
        let fault = call(CLIENT_DISPLAY, OP_TEST_FAULT, 0, 0);
        let handle = call(CLIENT_MANAGER, MANAGER_OPEN_DISPLAY, 0, 0);
        let after = if rp_hal_user::is_err(handle) {
            handle
        } else {
            call(handle, OP_LEVEL, 0, 0)
        };
        rp_hal_user::report(set, fault, handle, after)
    }

    /// Every entry: a thread start (`r0 = 0`, `r1` its role) or a call (`r0`
    /// the caller's slot, `r1` the operation, `r2` its argument).
    fn start(caller: u32, op: u32, arg: u32, _: u32) -> ! {
        if caller != 0 {
            rp_hal_user::ret(serve(op, arg))
        }
        match op {
            ROLE_HOST => host(),
            ROLE_TEST_CLIENT => test_client(),
            ROLE_TEST_FAULT => panic!("test fault"),
            _ => rp_hal_user::exit(ERR_OP),
        }
    }

    rp_hal_user::entry!(start);
}

#[cfg(not(target_os = "none"))]
fn main() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presses_releases_and_taps_lost_while_masked() {
        // Down: a press; up after down: its release.
        assert_eq!(button_press(1, false), (true, true));
        assert_eq!(button_press(0, true), (false, false));
        // Up after up: the whole tap happened while the line was masked.
        assert_eq!(button_press(0, false), (true, false));
        // Down after down (the release edge was lost): a new press.
        assert_eq!(button_press(1, true), (true, true));
        // A tap, seen as one signal, then the next tap the same way: two
        // presses (off, then on).
        let (p1, d1) = button_press(0, false);
        let (p2, _) = button_press(0, d1);
        assert!(p1 && p2);
    }
}
