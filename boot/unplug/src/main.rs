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
//! Candidates:
//!
//! 1. the PXA27x's own USB host controller driving SE0 on port 2 (the
//!    transceiver switched to it with `UP2OCR.HXS`, a root-hub port reset):
//!    a driven SE0 beats the pull-up, and the PC reads SE0 as a disconnect;
//! 2. the port 2 transceiver fully off (`UP2OCR` 0) with the UDC off.
//!
//! Run 1 (2026-10-16) also cleared CPLD bits of unknown purpose; one wedged
//! the bus and the watchdog reset the Axim into Dell's factory prompt, so
//! those are left out until their purpose is known (docs/hw/udc.md T4).
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

    use rp_drv_mbx_display::Display;
    use rp_drv_mbx_display::console::Console;
    use rp_drv_pxa_uart::PxaUart;
    use rp_drv_pxa_udc::Udc;
    use rp_drv_pxa_udc::reg::{HXOE, UP2OCR};
    use rp_hal_api::RegisterBus;
    use rp_hal_pxa27x::Mmio;
    use rp_hal_pxa27x::rt;
    use rp_hal_x51v::MBX_VRAM_SIZE;
    use rp_hal_x51v::devices;

    rp_image::entry!(main);

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
    }

    /// Run 2 (2026-10-16): the transceiver candidates only. Run 1 also
    /// cleared CPLD bits Windows Mobile leaves set; one of them (`0x1A`
    /// bit 1 or `0x14` bit 3, by the timing) wedged the bus, the hardware
    /// watchdog reset the Axim into Dell's factory prompt, and the run log
    /// was lost. Bits of unknown purpose are not switched blind again
    /// (docs/hw/udc.md T4).
    const CANDIDATES: [(&str, Act); 2] = [
        ("host controller SE0 on port 2", Act::HostSe0),
        ("UP2OCR 0 with the UDC off", Act::TransceiverOff),
    ];

    struct Rig {
        udc: Udc<Mmio>,
        uhc: Mmio,
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
            }
        }
    }

    fn main() -> ! {
        let Some(bus) = rt::take_ffuart() else {
            rp_image::fail("no UART")
        };
        let mut uart = PxaUart::new(bus);
        uart.init(115_200);
        rt::init_mmu();

        let Some((regs, vram)) = rp_image::take_mbx() else {
            rp_image::fail("no display")
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

        let (Some(udc_bus), Some(uhc)) = (rt::take_udc(), rt::take_uhc()) else {
            let _ = writeln!(out, "devices unavailable");
            rp_image::fail("UDC or USB host unavailable")
        };
        let hcca = Hcca([0; 64]);
        let mut rig = Rig {
            udc: Udc::new(udc_bus),
            uhc,
            hcca: core::ptr::addr_of!(hcca) as u32,
        };
        if rig.udc.enable().is_err() {
            let _ = writeln!(out, "UDC enable failed");
            rp_image::fail("UDC enable")
        }
        let _ = writeln!(out, "UDC on, UP2OCR {:08X}", rig.udc.read_reg(UP2OCR));

        let start = rt::ticks();
        let ms = |t: u32| t.wrapping_sub(start) / (TICK_1S / 1000);
        for (name, act) in &CANDIDATES {
            // The PC must be done with the device first (configured, then
            // 2 s without a reset or a SETUP): run 2's counts mixed the
            // PC's late enumeration of this image with the candidate.
            let (calm, waited) = quiet(&mut rig, 20 * TICK_1S);
            let base = settle(&mut rig, 0);
            let before = settle(&mut rig, TICK_1S);
            let at = ms(rt::ticks());
            rig.off(act, &mut out);
            settle(&mut rig, TICK_1S);
            rig.on(act);
            let after = settle(&mut rig, 3 * TICK_1S);
            let _ = writeln!(
                out,
                "{name} at {at} ms: resets {} setups {} (PC quiet: {} after {} ms; 1 s before: {}/{}) configured {}",
                after.resets.wrapping_sub(before.resets),
                after.setups.wrapping_sub(before.setups),
                if calm { "yes" } else { "NO" },
                waited / (TICK_1S / 1000),
                before.resets.wrapping_sub(base.resets),
                before.setups.wrapping_sub(base.setups),
                u8::from(rig.udc.configured())
            );
        }
        let _ = writeln!(out, "{}", super::MARKER);
        core::hint::black_box(&hcca);
        rt::delay_us(3_000_000);
        rp_image::finish()
    }

    /// Polls the UDC until the PC has configured the device and sent no
    /// reset or SETUP for 2 s, or `limit` passes. Whether it got quiet, and
    /// after how many ticks.
    fn quiet(rig: &mut Rig, limit: u32) -> (bool, u32) {
        let t = rt::ticks();
        let mut last = rig.udc.stats();
        let mut since = rt::ticks();
        loop {
            rt::check_in();
            rig.udc.poll();
            let now = rt::ticks();
            let s = rig.udc.stats();
            if s.resets != last.resets || s.setups != last.setups {
                last = s;
                since = now;
            }
            if rig.udc.configured() && now.wrapping_sub(since) >= 2 * TICK_1S {
                return (true, now.wrapping_sub(t));
            }
            if now.wrapping_sub(t) >= limit {
                return (false, now.wrapping_sub(t));
            }
        }
    }

    /// Polls the UDC for `ticks`, then returns its counters. Checks in with
    /// the loader's run limits meanwhile.
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
            // The run log first: it reaches the loader even if a screen
            // write is the last thing this image does.
            devices::log_write(s.as_bytes());
            let _ = self.1.write_str(s);
            self.0.write_str(s)
        }
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
