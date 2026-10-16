//! Making the PC see the X51v unplug and come back (docs/hw/udc.md, "The
//! first enumeration after a hand-over from Windows Mobile", T4).
//!
//! The X51v's D+ pull-up is hard-wired: nothing in software takes it away, so
//! the PC never notices when a new owner takes the controller (HaRET's
//! hand-over from Windows Mobile, a watchdog reboot): it keeps the old
//! device and the new one is never enumerated. But the USB connector's
//! transceiver (port 2) can be given to the PXA27x's own USB host controller
//! (`UP2OCR.HXS`), and a root-hub port reset there drives SE0 onto the bus;
//! a driven SE0 beats the pull-up, and the PC's hub takes it as an unplug.
//! Switched back to the device controller, the device is "plugged in" again
//! and the PC enumerates it afresh. Measured on the X51v (`rp-unplug` run 3,
//! 2026-10-16): with the PC idle, 2 bus resets and 32 SETUPs (a full
//! enumeration) followed, and Windows logged the old device removed.
//!
//! The sequence is the measured one, step for step.

use rp_hal_api::RegisterBus;

use crate::reg::{HXOE, UDCCR, UDE, UP2OCR};

/// `UP2OCR`: transceiver output select, 1 = the host controller.
pub const HXS: u32 = 1 << 16;
/// `UP2OCR`: the D+ and D- pull-downs (a host port's 15 kOhm).
pub const DPPDE: u32 = 1 << 2;
/// See [`DPPDE`].
pub const DMPDE: u32 = 1 << 3;

/// The USB host controller's registers (OHCI, base `0x4C000000`).
pub mod uhc {
    /// `HcControl`.
    pub const CONTROL: usize = 0x04;
    /// `HcCommandStatus`.
    pub const COMMAND: usize = 0x08;
    /// `HcInterruptDisable`.
    pub const INT_DISABLE: usize = 0x14;
    /// `HcHCCA`: the communication area's address.
    pub const HCCA: usize = 0x18;
    /// `HcRhDescriptorA`.
    pub const RH_DESC_A: usize = 0x48;
    /// `HcRhPortStatus` for port 2.
    pub const RH_PORT2: usize = 0x58;
    /// The PXA27x's host reset register `UHCHR`.
    pub const UHCHR: usize = 0x64;
    /// `UHCHR`: force a bus-interface reset (self-clearing).
    pub const FSBIR: u32 = 1 << 0;
    /// `UHCHR`: hold the host controller in reset.
    pub const FHR: u32 = 1 << 1;
    /// `UHCHR`: sleep standby, and its per-port bits (ports 1-3).
    pub const SSE: u32 = 1 << 5;
    /// See [`SSE`].
    pub const SSEP: u32 = 0b111 << 9;
    /// `HcControl.HCFS` = operational, every list off.
    pub const OPERATIONAL: u32 = 2 << 6;
    /// `HcCommandStatus.HCR`: host controller reset.
    pub const HCR: u32 = 1;
    /// `HcRhDescriptorA.NPS`: ports always powered.
    pub const NPS: u32 = 1 << 9;
    /// Port status write: set port reset.
    pub const SET_PORT_RESET: u32 = 1 << 4;
    /// Port status write: set port power.
    pub const SET_PORT_POWER: u32 = 1 << 8;
}

/// The host controller's communication area: it writes the frame number
/// here every frame while operational, so it must be RAM we own (256-byte
/// aligned), never the reset value 0 (the boot flash on the X51v).
#[repr(C, align(256))]
struct Hcca([u32; 64]);

/// Port 2 resets driven (each ~10 ms of SE0), and the gap after each.
const RESETS: u32 = 5;
const RESET_GAP_US: u32 = 30_000;

/// Unplugs the device and plugs it back in, as the PC sees it: the UDC off,
/// port 2 given to the host controller, its port reset driven, the host
/// controller stopped and held in reset again, port 2 back to the device
/// controller with only its output enabled (`HXOE`, as Windows Mobile
/// leaves it). The caller enables the UDC afterwards; the PC enumerates it
/// as a newly plugged device. Takes ~155 ms. `udc` is the device
/// controller's register window, `uhc` the host controller's (its clock
/// on); `delay_us` waits at least that many microseconds. Returns port 2's
/// status after the resets (`0x00110103` on the X51v: connected, reset
/// done, enabled).
pub fn unplug(
    udc: &mut impl RegisterBus,
    uhc: &mut impl RegisterBus,
    delay_us: &mut impl FnMut(u32),
) -> u32 {
    use uhc::*;
    let cr = udc.read32(UDCCR);
    udc.write32(UDCCR, cr & crate::reg::UDCCR_WRITABLE & !UDE);
    let hcca = Hcca([0; 64]);
    // Out of reset: hold FHR, release it, then a bus-interface reset, with
    // the ports out of sleep standby.
    let hr = uhc.read32(UHCHR);
    uhc.write32(UHCHR, hr | FHR);
    delay_us(20);
    uhc.write32(UHCHR, hr & !FHR);
    uhc.write32(UHCHR, (hr & !(FHR | SSE | SSEP)) | FSBIR);
    let mut spins = 0;
    while uhc.read32(UHCHR) & FSBIR != 0 && spins < 1000 {
        delay_us(10);
        spins += 1;
    }
    uhc.write32(INT_DISABLE, 0xFFFF_FFFF);
    uhc.write32(HCCA, core::ptr::addr_of!(hcca) as usize as u32);
    let da = uhc.read32(RH_DESC_A);
    uhc.write32(RH_DESC_A, da | NPS);
    uhc.write32(CONTROL, OPERATIONAL);
    udc.write32(UP2OCR, HXS | HXOE | DPPDE | DMPDE);
    uhc.write32(RH_PORT2, SET_PORT_POWER);
    delay_us(5_000);
    for _ in 0..RESETS {
        uhc.write32(RH_PORT2, SET_PORT_RESET);
        delay_us(RESET_GAP_US);
    }
    let port = uhc.read32(RH_PORT2);
    // Stopped before `hcca` goes out of scope: no more writes to it.
    uhc.write32(CONTROL, 0);
    uhc.write32(COMMAND, HCR);
    delay_us(100);
    let hr = uhc.read32(UHCHR);
    uhc.write32(UHCHR, hr | FHR);
    udc.write32(UP2OCR, HXOE);
    core::hint::black_box(&hcca);
    port
}

#[cfg(test)]
mod tests {
    extern crate std;
    use std::cell::RefCell;
    use std::vec::Vec;

    use super::uhc::*;
    use super::*;

    /// Every access to either block, in order: (block, offset, value, write).
    type Log = RefCell<Vec<(char, usize, u32, bool)>>;

    struct Bus<'a> {
        name: char,
        log: &'a Log,
        regs: std::collections::HashMap<usize, u32>,
    }

    impl RegisterBus for Bus<'_> {
        fn read32(&mut self, offset: usize) -> u32 {
            let v = self.regs.get(&offset).copied().unwrap_or(0);
            self.log.borrow_mut().push((self.name, offset, v, false));
            v
        }
        fn write32(&mut self, offset: usize, value: u32) {
            self.log.borrow_mut().push((self.name, offset, value, true));
            // FSBIR clears itself; everything else reads back.
            let v = if self.name == 'h' && offset == UHCHR {
                value & !FSBIR
            } else {
                value
            };
            self.regs.insert(offset, v);
        }
    }

    fn run(udccr: u32, up2ocr: u32) -> (Vec<(char, usize, u32, bool)>, u32) {
        let log = Log::default();
        let mut udc = Bus {
            name: 'u',
            log: &log,
            regs: Default::default(),
        };
        udc.regs.insert(UDCCR, udccr);
        udc.regs.insert(UP2OCR, up2ocr);
        let mut uhc = Bus {
            name: 'h',
            log: &log,
            regs: Default::default(),
        };
        uhc.regs.insert(UHCHR, FHR | 0x4);
        let mut waited = 0;
        unplug(&mut udc, &mut uhc, &mut |us| waited += us);
        (log.into_inner(), waited)
    }

    fn writes(log: &[(char, usize, u32, bool)]) -> Vec<(char, usize, u32)> {
        log.iter()
            .filter(|e| e.3)
            .map(|e| (e.0, e.1, e.2))
            .collect()
    }

    fn pos(w: &[(char, usize, u32)], what: (char, usize, u32)) -> usize {
        w.iter()
            .position(|e| *e == what)
            .unwrap_or_else(|| panic!("no write {what:x?}"))
    }

    #[test]
    fn drives_port_2_reset_with_the_transceiver_on_the_host_controller() {
        let (log, _) = run(UDE | 1 << 1, HXOE);
        let w = writes(&log);
        let udc_off = pos(&w, ('u', UDCCR, 0));
        let to_host = pos(&w, ('u', UP2OCR, HXS | HXOE | DPPDE | DMPDE));
        let first_reset = pos(&w, ('h', RH_PORT2, SET_PORT_RESET));
        let back = pos(&w, ('u', UP2OCR, HXOE));
        assert!(udc_off < to_host && to_host < first_reset && first_reset < back);
        let resets = w
            .iter()
            .filter(|e| **e == ('h', RH_PORT2, SET_PORT_RESET))
            .count();
        assert_eq!(resets, RESETS as usize);
        // The device controller stays off: the caller enables it.
        assert!(
            !w.iter()
                .any(|e| e.0 == 'u' && e.1 == UDCCR && e.2 & UDE != 0)
        );
    }

    #[test]
    fn the_host_controller_never_runs_without_our_communication_area() {
        let (log, _) = run(UDE, HXOE);
        let w = writes(&log);
        let hcca = w
            .iter()
            .find(|e| e.0 == 'h' && e.1 == HCCA)
            .expect("HCCA set");
        assert!(hcca.2 != 0 && hcca.2 % 256 == 0, "HCCA {:x}", hcca.2);
        let set = pos(&w, ('h', HCCA, hcca.2));
        let operational = pos(&w, ('h', CONTROL, OPERATIONAL));
        assert!(set < operational);
        // Stopped, reset and held in reset before returning.
        let stop = pos(&w, ('h', CONTROL, 0));
        let hcr = pos(&w, ('h', COMMAND, HCR));
        let held = w
            .iter()
            .rposition(|e| e.0 == 'h' && e.1 == UHCHR && e.2 & FHR != 0)
            .unwrap();
        assert!(operational < stop && stop < hcr && hcr < held);
        assert_eq!(w.last(), Some(&('u', UP2OCR, HXOE)));
    }

    #[test]
    fn ends_with_port_2_back_on_the_device_controller_whatever_it_found() {
        // After a full reset UP2OCR reads HXOE | HXS (the host selected).
        let (log, waited) = run(0, HXOE | HXS);
        assert_eq!(writes(&log).last(), Some(&('u', UP2OCR, HXOE)));
        assert!((150_000..200_000).contains(&waited), "{waited} us");
    }
}
