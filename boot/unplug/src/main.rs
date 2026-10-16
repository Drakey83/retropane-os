//! USB disconnect hunt (docs/hw/udc.md, "The first enumeration after a
//! hand-over from Windows Mobile", T4): which action makes the PC see the
//! X51v unplug and come back?
//!
//! The PC never sees HaRET hand the device from Windows Mobile to the
//! loader: the D+ pull-up is hard-wired, so the port stays connected and the
//! loader is invisible until something on the PC fails or the cable is
//! re-plugged (traced 2026-10-16). A board reset does disconnect, so the PC
//! can be made to see one. `rp-reconnect` (2026-10-11) ruled out GPIO2, the
//! internal pull-up and pull-down, CPLD `0x1C` bit 3 and the UDC on/off.
//!
//! This image, started by the loader, enables the UDC, then for each
//! candidate measures the PC's bus resets and SETUPs for 1 s, applies the
//! candidate for 1 s, undoes it and counts again for 3 s: a real disconnect
//! makes the PC's hub reset the port within ~0.1 s of the reconnect.
//! Candidates, safest first:
//!
//! 1. the PXA27x's own USB host controller driving SE0 on port 2 (the
//!    transceiver switched to it with `UP2OCR.HXS`, a root-hub port reset):
//!    a driven SE0 beats the pull-up, and the PC reads SE0 as a disconnect;
//! 2. the port 2 transceiver fully off (`UP2OCR` 0) with the UDC off;
//! 3. CPLD bits Windows Mobile leaves set whose purpose isn't known, one at
//!    a time (never SD, CF, backlight, display or the reboot bit). A bit
//!    that isn't set at start is skipped, so nothing new is switched on.
//!
//! Results go to the screen, the serial port and the run log.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// The first line, printed at start.
pub const TITLE: &str = "RetroPane 26 USB disconnect hunt";
/// Printed on serial when every candidate has been tried.
pub const MARKER: &str = "RETROPANE-UNPLUG-DONE";

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_cpld::{Cpld, handover};
    use rp_drv_mbx_display::Display;
    use rp_drv_mbx_display::console::Console;
    use rp_drv_pxa_uart::PxaUart;
    use rp_drv_pxa_udc::Udc;
    use rp_drv_pxa_udc::reg::{HXOE, UP2OCR};
    use rp_hal_api::RegisterBus;
    use rp_hal_pxa27x::Mmio;
    use rp_hal_pxa27x::rt;
    use rp_hal_x51v::MBX_VRAM_SIZE;
    use rp_hal_x51v::devices::{self, RunLog};

    rp_hal_pxa27x::entry!(main);

    const TICK_1S: u32 = 3_250_000;

    /// `UP2OCR`: transceiver output select, 1 = the host controller.
    const HXS: u32 = 1 << 16;
    /// `UP2OCR`: D+ and D- pull-downs (a host port's 15 kOhm).
    const DPPDE: u32 = 1 << 2;
    const DMPDE: u32 = 1 << 3;

    /// USB host controller (OHCI, PXA27x manual §20): control, command
    /// status, interrupt disable, HCCA, root-hub descriptor A, port 2 status.
    const HC_CONTROL: usize = 0x04;
    const HC_COMMAND: usize = 0x08;
    const HC_INT_DISABLE: usize = 0x14;
    const HC_HCCA: usize = 0x18;
    const HC_RH_DESC_A: usize = 0x48;
    const HC_RH_PORT2: usize = 0x58;
    /// The PXA27x's host reset register: `FSBIR` (bit 0, self-clearing) and
    /// `FHR` (bit 1, held), `SSE` (bit 5) and the per-port sleep standby
    /// bits `SSEP1-3` (bits 9-11).
    const UHCHR: usize = 0x64;
    const FSBIR: u32 = 1 << 0;
    const FHR: u32 = 1 << 1;
    const SSE: u32 = 1 << 5;
    const SSEP: u32 = 0b111 << 9;
    /// `HcControl.HCFS` = USBOPERATIONAL, every list off.
    const OPERATIONAL: u32 = 2 << 6;
    /// `HcCommandStatus.HCR`: host controller reset.
    const HCR: u32 = 1;
    /// `HcRhDescriptorA.NPS`: ports always powered.
    const NPS: u32 = 1 << 9;
    /// Port status writes: set port reset, set port power; reads: connected.
    const SET_PORT_RESET: u32 = 1 << 4;
    const SET_PORT_POWER: u32 = 1 << 8;

    /// The host controller's communication area: it writes the frame number
    /// and done head here every frame while operational, so it must be RAM
    /// we own (256-byte aligned), never the reset value 0 (the boot flash).
    #[repr(C, align(256))]
    struct Hcca([u32; 64]);

    enum Act {
        HostSe0,
        TransceiverOff,
        Cpld(usize, u16),
    }

    const CANDIDATES: [(&str, Act); 12] = [
        ("host controller SE0 on port 2", Act::HostSe0),
        ("UP2OCR 0 with the UDC off", Act::TransceiverOff),
        ("CPLD 1C bit 2", Act::Cpld(0x1C, 0x0004)),
        ("CPLD 14 bit 3", Act::Cpld(0x14, 0x0008)),
        ("CPLD 1A bit 1", Act::Cpld(0x1A, 0x0002)),
        ("CPLD 1A bit 2", Act::Cpld(0x1A, 0x0004)),
        ("CPLD 02 bit 0", Act::Cpld(0x02, 0x0001)),
        ("CPLD 02 bit 2", Act::Cpld(0x02, 0x0004)),
        ("CPLD 02 bit 3", Act::Cpld(0x02, 0x0008)),
        ("CPLD 02 bit 4", Act::Cpld(0x02, 0x0010)),
        ("CPLD 02 bit 5", Act::Cpld(0x02, 0x0020)),
        ("CPLD 02 bit 7", Act::Cpld(0x02, 0x0080)),
    ];

    struct Rig {
        udc: Udc<Mmio>,
        uhc: Mmio,
        cpld: Cpld<Mmio, handover::Table<'static>>,
        hcca: u32,
    }

    impl Rig {
        /// The "unplug" half; what it saw, for the log.
        fn off(&mut self, a: &Act, out: &mut impl Write) {
            match *a {
                Act::HostSe0 => {
                    self.udc.disable();
                    let up2 = self.udc.read_reg(UP2OCR);
                    // The host controller out of reset (PXA27x: hold FHR,
                    // release it, then a bus-interface reset), its ports
                    // out of sleep standby and always powered.
                    let hr = self.uhc.read32(UHCHR);
                    self.uhc.write32(UHCHR, hr | FHR);
                    rt::delay_us(20);
                    self.uhc.write32(UHCHR, hr & !FHR);
                    self.uhc.write32(UHCHR, (hr & !(FHR | SSE | SSEP)) | FSBIR);
                    let t = rt::ticks();
                    while self.uhc.read32(UHCHR) & FSBIR != 0
                        && rt::ticks().wrapping_sub(t) < TICK_1S / 100
                    {}
                    self.uhc.write32(HC_INT_DISABLE, 0xFFFF_FFFF);
                    self.uhc.write32(HC_HCCA, self.hcca);
                    let da = self.uhc.read32(HC_RH_DESC_A);
                    self.uhc.write32(HC_RH_DESC_A, da | NPS);
                    self.uhc.write32(HC_CONTROL, OPERATIONAL);
                    // Port 2 to the host controller, with a host's pull-downs.
                    self.udc.write_reg(UP2OCR, HXS | HXOE | DPPDE | DMPDE);
                    self.uhc.write32(HC_RH_PORT2, SET_PORT_POWER);
                    rt::delay_us(5_000);
                    let before = self.uhc.read32(HC_RH_PORT2);
                    // Five resets in the window: each drives SE0 for ~10 ms.
                    for _ in 0..5 {
                        self.uhc.write32(HC_RH_PORT2, SET_PORT_RESET);
                        rt::delay_us(30_000);
                    }
                    let after = self.uhc.read32(HC_RH_PORT2);
                    let _ = write!(
                        out,
                        "  UP2OCR was {up2:08X}, UHCHR {hr:08X}; port 2 {before:08X} -> {after:08X}\n"
                    );
                }
                Act::TransceiverOff => {
                    self.udc.disable();
                    self.udc.write_reg(UP2OCR, 0);
                }
                Act::Cpld(reg, bits) => self.cpld.clear(reg, bits),
            }
        }

        /// The "plug back in" half: everything as before the candidate.
        fn on(&mut self, a: &Act) {
            match *a {
                Act::HostSe0 => {
                    self.uhc.write32(HC_CONTROL, 0);
                    self.uhc.write32(HC_COMMAND, HCR);
                    rt::delay_us(100);
                    let hr = self.uhc.read32(UHCHR);
                    self.uhc.write32(UHCHR, hr | FHR);
                    self.udc.write_reg(UP2OCR, HXOE);
                    let _ = self.udc.enable();
                }
                Act::TransceiverOff => {
                    self.udc.write_reg(UP2OCR, HXOE);
                    let _ = self.udc.enable();
                }
                Act::Cpld(reg, bits) => self.cpld.set(reg, bits),
            }
        }
    }

    fn on_fault(kind: rt::Fault, addr: u32) -> ! {
        let mut uart = PxaUart::new(rt::emergency_uart());
        let _ = writeln!(uart, "\nFAULT {} at {addr:#010x}", kind.name());
        let _ = writeln!(RunLog, "FAULT {} at {addr:#010x}", kind.name());
        devices::return_to_loader();
        halt()
    }

    fn main() -> ! {
        let Some(bus) = rt::take_ffuart() else { halt() };
        let mut uart = PxaUart::new(bus);
        uart.init(115_200);
        rt::set_fault_handler(on_fault);
        rt::init_mmu();

        let Some((regs, vram)) = devices::take_mbx() else {
            halt()
        };
        let (plane, _) = Display::new(regs).adopt(MBX_VRAM_SIZE as usize);
        let end = plane.offset + plane.stride * usize::from(plane.height);
        let mut con = Console::new(
            &mut vram[plane.offset..end],
            plane.stride,
            (0, 0),
            (480, 640),
            2,
            (0xFFFF, 0x0000),
        );
        let mut out = Tee(&mut uart, &mut con);
        let _ = writeln!(out, "{}", super::TITLE);

        let (Some(udc_bus), Some(uhc), Some(cpld_bus)) =
            (rt::take_udc(), rt::take_uhc(), devices::take_cpld())
        else {
            let _ = writeln!(out, "devices unavailable");
            devices::return_to_loader();
            halt()
        };
        let hcca = Hcca([0; 64]);
        let mut rig = Rig {
            udc: Udc::new(udc_bus),
            uhc,
            cpld: Cpld::adopt(cpld_bus.0, cpld_bus.1).0,
            hcca: core::ptr::addr_of!(hcca) as u32,
        };
        if rig.udc.enable().is_err() {
            let _ = writeln!(out, "UDC enable failed");
            devices::return_to_loader();
            halt()
        }
        let _ = writeln!(
            out,
            "UDC on, UP2OCR {:08X}; CPLD 02={:04X} 14={:04X} 1A={:04X} 1C={:04X}",
            rig.udc.read_reg(UP2OCR),
            rig.cpld.shadow(0x02),
            rig.cpld.shadow(0x14),
            rig.cpld.shadow(0x1A),
            rig.cpld.shadow(0x1C),
        );

        for (name, act) in &CANDIDATES {
            if let Act::Cpld(reg, bits) = *act
                && rig.cpld.shadow(reg) & bits != bits
            {
                let _ = writeln!(out, "{name}: not set at start, skipped");
                continue;
            }
            // Settle, then measure what the PC does on its own first.
            let quiet = settle(&mut rig, TICK_1S);
            let before = settle(&mut rig, TICK_1S);
            rig.off(act, &mut out);
            settle(&mut rig, TICK_1S);
            rig.on(act);
            let after = settle(&mut rig, 3 * TICK_1S);
            let _ = writeln!(
                out,
                "{name}: resets {} setups {} (idle 1 s before: {}/{}) configured {}",
                after.resets.wrapping_sub(before.resets),
                after.setups.wrapping_sub(before.setups),
                before.resets.wrapping_sub(quiet.resets),
                before.setups.wrapping_sub(quiet.setups),
                u8::from(rig.udc.configured())
            );
        }
        let _ = writeln!(out, "{}", super::MARKER);
        core::hint::black_box(&hcca);
        rt::delay_us(3_000_000);
        devices::return_to_loader();
        halt()
    }

    /// Polls the UDC for `ticks`, then returns its counters. Checks in with
    /// the loader's run limits meanwhile (the run takes ~70 s; the limit is
    /// 30 s without check-ins).
    fn settle(rig: &mut Rig, ticks: u32) -> rp_drv_pxa_udc::Stats {
        let t = rt::ticks();
        while rt::ticks().wrapping_sub(t) < ticks {
            rt::check_in();
            rig.udc.poll();
        }
        rig.udc.stats()
    }

    /// Writes to the UART, the screen and the run log.
    struct Tee<'a, 'b>(&'a mut PxaUart<Mmio>, &'a mut Console<'b>);

    impl Write for Tee<'_, '_> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let _ = self.1.write_str(s);
            devices::log_write(s.as_bytes());
            self.0.write_str(s)
        }
    }

    fn halt() -> ! {
        loop {
            core::hint::spin_loop();
        }
    }

    #[panic_handler]
    fn panic(_: &core::panic::PanicInfo) -> ! {
        let _ = writeln!(RunLog, "PANIC");
        devices::return_to_loader();
        halt()
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
