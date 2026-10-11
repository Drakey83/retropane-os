//! Intel 2700G display controller: panel timing and the graphics plane.
//!
//! Written from `docs/hw/2700g.md` ("Display programming model"). Clock/PLL
//! and local-memory power-up aren't specified yet; under Windows Mobile they're
//! already configured when RetroPane starts from the SD card.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod console;
pub mod font;
pub mod pattern;

use rp_hal_api::{Field, RegisterBus};

/// Register offsets within the 2700G register block.
pub mod reg {
    /// Graphics surface control.
    pub const GSCTRL: usize = 0x2000;
    /// Graphics base.
    pub const GBBASE: usize = 0x2020;
    /// Graphics draw control.
    pub const GDRCTRL: usize = 0x2040;
    /// Graphics stream control / start address.
    pub const GSCADR: usize = 0x2060;
    /// Graphics stride / position.
    pub const GSADR: usize = 0x20C0;
    /// Display sync control.
    pub const DSCTRL: usize = 0x2154;
    /// Horizontal timing 1-3.
    pub const DHT01: usize = 0x2158;
    /// Horizontal timing 2.
    pub const DHT02: usize = 0x215C;
    /// Horizontal timing 3.
    pub const DHT03: usize = 0x2160;
    /// Vertical timing 1.
    pub const DVT01: usize = 0x2164;
    /// Vertical timing 2.
    pub const DVT02: usize = 0x2168;
    /// Vertical timing 3.
    pub const DVT03: usize = 0x216C;
    /// Vertical event control.
    pub const DVECTRL: usize = 0x2310;
    /// Horizontal display enable.
    pub const DHDET: usize = 0x2314;
    /// Vertical display enable.
    pub const DVDET: usize = 0x2318;
}

const HI12: Field = Field::new(16, 12);
const LO12: Field = Field::new(0, 12);
const GSCTRL_FMT: Field = Field::new(27, 4);
const GSCTRL_WIDTH: Field = Field::new(11, 11);
const GSCTRL_HEIGHT: Field = Field::new(0, 11);
const GSCADR_STR_EN: Field = Field::bit(31);
const GSCADR_BASE: Field = Field::new(0, 23);
const GSADR_STRIDE: Field = Field::new(22, 10);
const DSCTRL_SYNCGEN_EN: Field = Field::bit(31);

/// Local-memory offset conventionally used for the framebuffer.
pub const FRAMEBUFFER_OFFSET: usize = 0x6_0000;

/// LCD panel timing, in pixels/lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PanelTiming {
    /// Active width.
    pub hactive: u16,
    /// Horizontal front porch.
    pub hfp: u16,
    /// Horizontal sync width.
    pub hsync: u16,
    /// Horizontal back porch.
    pub hbp: u16,
    /// Active height.
    pub vactive: u16,
    /// Vertical front porch.
    pub vfp: u16,
    /// Vertical sync width.
    pub vsync: u16,
    /// Vertical back porch.
    pub vbp: u16,
    /// Pixel clock in Hz.
    pub pixel_clock_hz: u32,
}

/// The Dell Axim X51v's 3.7" VGA panel (docs/hw/2700g.md, "LCD panel timing").
pub const X51V_VGA: PanelTiming = PanelTiming {
    hactive: 480,
    hfp: 88,
    hsync: 64,
    hbp: 16,
    vactive: 640,
    vfp: 1,
    vsync: 2,
    vbp: 1,
    pixel_clock_hz: 23_562_500,
};

/// The display timing registers for a panel, as `(offset, value)` pairs in
/// programming order.
#[must_use]
pub fn timing_registers(p: &PanelTiming) -> [(usize, u32); 9] {
    let hbps = u32::from(p.hsync);
    let has = hbps + u32::from(p.hbp);
    let hfps = has + u32::from(p.hactive);
    let ht = hfps + u32::from(p.hfp);
    let vbps = u32::from(p.vsync);
    let vas = vbps + u32::from(p.vbp);
    let vfps = vas + u32::from(p.vactive);
    let vt = vfps + u32::from(p.vfp);
    let pair = |hi: u32, lo: u32| HI12.insert(LO12.insert(0, lo), hi);
    [
        (reg::DHT01, pair(hbps, ht)),
        (reg::DHT02, pair(has, has)),
        (reg::DHT03, pair(hfps, hfps)),
        (reg::DHDET, pair(has, hfps)),
        (reg::DVT01, pair(vbps, vt)),
        (reg::DVT02, pair(vas, vas)),
        (reg::DVT03, pair(vfps, vfps)),
        (reg::DVDET, pair(vas, vfps)),
        (reg::DVECTRL, pair(vfps, vbps)),
    ]
}

/// Graphics plane pixel formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    /// 16-bit RGB 5:6:5.
    Rgb565 = 7,
    /// 32-bit ARGB 8:8:8:8.
    Argb8888 = 8,
}

impl PixelFormat {
    /// Bytes per pixel.
    #[must_use]
    pub const fn bytes(self) -> usize {
        match self {
            Self::Rgb565 => 2,
            Self::Argb8888 => 4,
        }
    }
}

/// The graphics plane: what the display scans out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plane {
    /// Pixel format.
    pub format: PixelFormat,
    /// Width in pixels.
    pub width: u16,
    /// Height in pixels.
    pub height: u16,
    /// Start offset in 2700G local memory (16-byte aligned).
    pub offset: usize,
    /// Bytes per line (multiple of 16).
    pub stride: usize,
}

impl Plane {
    /// A full-screen plane for `panel` at [`FRAMEBUFFER_OFFSET`].
    #[must_use]
    pub const fn fullscreen(panel: &PanelTiming, format: PixelFormat) -> Self {
        let stride = (panel.hactive as usize * format.bytes()).div_ceil(16) * 16;
        Self {
            format,
            width: panel.hactive,
            height: panel.vactive,
            offset: FRAMEBUFFER_OFFSET,
            stride,
        }
    }

    /// The plane registers as `(offset, value)` pairs in programming order.
    ///
    /// # Panics
    /// If the offset or stride isn't 16-byte aligned, or a size is zero or out
    /// of range for the hardware fields.
    #[must_use]
    pub fn registers(&self) -> [(usize, u32); 5] {
        assert!(
            self.offset.is_multiple_of(16) && self.stride.is_multiple_of(16),
            "unaligned plane"
        );
        assert!(self.width >= 1 && self.width <= 2048 && self.height >= 1 && self.height <= 2048);
        assert!(
            self.stride >= 16 && self.stride / 16 <= 1024,
            "stride out of range"
        );
        let gsctrl = GSCTRL_FMT.insert(0, self.format as u32);
        let gsctrl = GSCTRL_WIDTH.insert(gsctrl, u32::from(self.width) - 1);
        let gsctrl = GSCTRL_HEIGHT.insert(gsctrl, u32::from(self.height) - 1);
        let gscadr = GSCADR_STR_EN.insert(GSCADR_BASE.insert(0, (self.offset / 16) as u32), 1);
        let gsadr = GSADR_STRIDE.insert(0, (self.stride / 16 - 1) as u32);
        [
            (reg::GSCTRL, gsctrl),
            (reg::GBBASE, 0),
            (reg::GDRCTRL, 0x00FF_FFFF),
            (reg::GSCADR, gscadr),
            (reg::GSADR, gsadr),
        ]
    }
}

impl Plane {
    /// The plane described by register values read back from the hardware
    /// (the inverse of [`registers`](Self::registers)): used to keep drawing
    /// into the framebuffer Windows Mobile left configured. `None` if the
    /// format is unknown, scanning is off, or the plane doesn't fit in
    /// `vram_size` bytes of local memory.
    #[must_use]
    pub fn decode(gsctrl: u32, gscadr: u32, gsadr: u32, vram_size: usize) -> Option<Self> {
        let format = match GSCTRL_FMT.extract(gsctrl) {
            7 => PixelFormat::Rgb565,
            8 => PixelFormat::Argb8888,
            _ => return None,
        };
        if GSCADR_STR_EN.extract(gscadr) == 0 {
            return None;
        }
        let width = GSCTRL_WIDTH.extract(gsctrl) + 1;
        let height = GSCTRL_HEIGHT.extract(gsctrl) + 1;
        let plane = Self {
            format,
            width: u16::try_from(width).ok()?,
            height: u16::try_from(height).ok()?,
            offset: GSCADR_BASE.extract(gscadr) as usize * 16,
            stride: (GSADR_STRIDE.extract(gsadr) as usize + 1) * 16,
        };
        let row = plane.width as usize * format.bytes();
        let end = plane.offset + plane.stride * plane.height as usize;
        (row <= plane.stride && end <= vram_size).then_some(plane)
    }
}

/// Fills a `w` × `h` rectangle at `(x, y)` of an RGB565 framebuffer
/// (`stride` bytes per line) with `color`. Each row is one burst fill
/// (`rp_hal_mem::fill32`). The rectangle is clipped to `buf`.
pub fn fill_rect_rgb565(
    buf: &mut [u8],
    stride: usize,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    color: u16,
) {
    let word = u32::from(color) * 0x0001_0001;
    for row in y..y + h {
        let start = row * stride + x * 2;
        let Some(line) = buf.get_mut(start..start + w * 2) else {
            return;
        };
        rp_hal_mem::fill32(line, word);
    }
}

/// The 2700G display controller.
#[derive(Debug)]
pub struct Display<B> {
    regs: B,
}

impl<B: RegisterBus> Display<B> {
    /// Takes the 2700G register block.
    pub fn new(regs: B) -> Self {
        Self { regs }
    }

    /// Programs panel timing and the graphics plane, then enables scan-out.
    pub fn enable(&mut self, panel: &PanelTiming, plane: &Plane) {
        self.regs.modify(reg::DSCTRL, DSCTRL_SYNCGEN_EN, 0);
        for (offset, value) in timing_registers(panel).into_iter().chain(plane.registers()) {
            self.regs.write32(offset, value);
        }
        self.regs.modify(reg::DSCTRL, DSCTRL_SYNCGEN_EN, 1);
    }

    /// Stops scan-out.
    pub fn disable(&mut self) {
        self.regs.modify(reg::DSCTRL, DSCTRL_SYNCGEN_EN, 0);
    }

    /// Gives the register block back.
    pub fn into_inner(self) -> B {
        self.regs
    }
}

/// The 2700G registers Dell's driver saves and restores across suspend
/// (docs/hw/2700g.md): its own list of essential, safely readable state.
pub const SAVED_REGS: &[(&str, usize)] = &[
    ("SYSCFG", 0x000),
    ("PIXCLK", 0x048),
    ("MEMCLK", 0x04C),
    ("M24CLK", 0x050),
    ("MBXCLK", 0x054),
    ("SDCLK", 0x058),
    ("PIXCLKDIV", 0x05C),
    ("LCDCONFIG", 0x060),
    ("LMCFG", 0x1004),
    ("LMPWR", 0x1008),
    ("LMCEMR", 0x1010),
    ("LMTYPE", 0x1014),
    ("LMTIM", 0x1018),
    ("LMREFRESH", 0x101C),
    ("LMPROTMIN", 0x1020),
    ("LMPROTMAX", 0x1024),
    ("LMPROTCFG", 0x1028),
    ("GSCTRL", 0x2000),
    ("GBBASE", 0x2020),
    ("GDRCTRL", 0x2040),
    ("GSCADR", 0x2060),
    ("GSADR", 0x20C0),
    ("GPLUT", 0x2150),
    ("VSCTRL", 0x2004),
    ("VBBASE", 0x2024),
    ("VCMSK", 0x2044),
    ("VSCADR", 0x2064),
    ("VUBASE", 0x2084),
    ("VSADR", 0x20C4),
    ("BGCOLOR", 0x2174),
    ("HCCTRL", 0x2100),
    ("HCSIZE", 0x2110),
    ("HCPOS", 0x2120),
    ("HCBADR", 0x2130),
    ("HCCKMSK", 0x2140),
    ("DSCTRL", 0x2154),
    ("DUCTRL", 0x230C),
    ("DHT01", 0x2158),
    ("DHT02", 0x215C),
    ("DHT03", 0x2160),
    ("DVT01", 0x2164),
    ("DVT02", 0x2168),
    ("DVT03", 0x216C),
    ("DVECTRL", 0x2310),
    ("DHDET", 0x2314),
    ("DVDET", 0x2318),
    ("DODMSK", 0x231C),
    ("DBCOL", 0x2170),
    ("DSIG", 0x2184),
    ("DVLNUM", 0x2308),
    ("DMCTRL", 0x2188),
    ("DINTRE", 0x217C),
    ("DINTRCNT", 0x2180),
    ("DLLCTRL", 0x2304),
    ("CLIPCTRL", 0x218C),
    ("SPOCTRL", 0x2190),
    ("SVCTRL", 0x2194),
    ("SHCTRL", 0x21B0),
    ("SSSIZE", 0x21D8),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_inverts_registers() {
        for plane in [
            Plane::fullscreen(&X51V_VGA, PixelFormat::Rgb565),
            Plane {
                format: PixelFormat::Argb8888,
                width: 240,
                height: 320,
                offset: 0x10_0000,
                stride: 960,
            },
        ] {
            let r = plane.registers();
            let get = |off| r.iter().find(|(o, _)| *o == off).unwrap().1;
            assert_eq!(
                Plane::decode(
                    get(reg::GSCTRL),
                    get(reg::GSCADR),
                    get(reg::GSADR),
                    16 << 20
                ),
                Some(plane)
            );
        }
    }

    #[test]
    fn decode_refuses_what_it_cannot_draw_into() {
        let p = Plane::fullscreen(&X51V_VGA, PixelFormat::Rgb565);
        let r = p.registers();
        let (gsctrl, gscadr, gsadr) = (r[0].1, r[3].1, r[4].1);
        assert_eq!(
            Plane::decode(GSCTRL_FMT.insert(gsctrl, 3), gscadr, gsadr, 16 << 20),
            None
        );
        assert_eq!(
            Plane::decode(gsctrl, GSCADR_STR_EN.insert(gscadr, 0), gsadr, 16 << 20),
            None
        );
        assert_eq!(
            Plane::decode(gsctrl, gscadr, gsadr, 0x60000),
            None,
            "past local memory"
        );
        assert_eq!(
            Plane::decode(gsctrl, gscadr, 0, 16 << 20),
            None,
            "stride narrower than a row"
        );
    }
    use rp_hal_mock::MockBus;

    fn split(v: u32) -> (u32, u32) {
        (HI12.extract(v), LO12.extract(v))
    }

    #[test]
    fn x51v_timing_matches_spec() {
        let r = timing_registers(&X51V_VGA);
        let get = |off| split(r.iter().find(|(o, _)| *o == off).unwrap().1);
        assert_eq!(get(reg::DHT01), (64, 648)); // HBPS, HT
        assert_eq!(get(reg::DHT02), (80, 80)); // HAS
        assert_eq!(get(reg::DHT03), (560, 560)); // HFPS
        assert_eq!(get(reg::DHDET), (80, 560));
        assert_eq!(get(reg::DVT01), (2, 644)); // VBPS, VT
        assert_eq!(get(reg::DVT02), (3, 3)); // VAS
        assert_eq!(get(reg::DVT03), (643, 643)); // VFPS
        assert_eq!(get(reg::DVDET), (3, 643));
        assert_eq!(get(reg::DVECTRL), (643, 2));
    }

    #[test]
    fn vesa_vga_totals_are_standard() {
        let vga = PanelTiming {
            hactive: 640,
            hfp: 16,
            hsync: 96,
            hbp: 48,
            vactive: 480,
            vfp: 10,
            vsync: 2,
            vbp: 33,
            pixel_clock_hz: 25_175_000,
        };
        let r = timing_registers(&vga);
        assert_eq!(split(r[0].1).1, 800);
        assert_eq!(split(r[4].1).1, 525);
    }

    #[test]
    fn fullscreen_rgb565_plane_matches_spec() {
        let p = Plane::fullscreen(&X51V_VGA, PixelFormat::Rgb565);
        assert_eq!(p.stride, 960);
        let r = p.registers();
        let gsctrl = r[0].1;
        assert_eq!(GSCTRL_FMT.extract(gsctrl), 7);
        assert_eq!(GSCTRL_WIDTH.extract(gsctrl), 479);
        assert_eq!(GSCTRL_HEIGHT.extract(gsctrl), 639);
        assert_eq!(r[3].1, 0x8000_6000); // stream enable, 0x60000 / 16
        assert_eq!(GSADR_STRIDE.extract(r[4].1), 59);
    }

    #[test]
    fn stride_rounds_up_to_16_bytes() {
        let odd = PanelTiming {
            hactive: 241,
            ..X51V_VGA
        };
        assert_eq!(Plane::fullscreen(&odd, PixelFormat::Rgb565).stride, 496);
    }

    #[test]
    #[should_panic(expected = "unaligned")]
    fn rejects_unaligned_plane() {
        let mut p = Plane::fullscreen(&X51V_VGA, PixelFormat::Rgb565);
        p.offset += 4;
        let _ = p.registers();
    }

    #[test]
    fn enable_turns_sync_off_programs_then_turns_it_on() {
        let mut bus = MockBus::new();
        bus.preset(reg::DSCTRL, 0x8000_0020); // already running, blank polarity set
        let mut d = Display::new(bus);
        d.enable(
            &X51V_VGA,
            &Plane::fullscreen(&X51V_VGA, PixelFormat::Rgb565),
        );
        let w = d.into_inner().writes();
        assert_eq!(w.first(), Some(&(reg::DSCTRL, 0x0000_0020)));
        assert_eq!(w.last(), Some(&(reg::DSCTRL, 0x8000_0020)));
        assert_eq!(w.len(), 2 + 9 + 5);
        assert!(w.contains(&(reg::GSCADR, 0x8000_6000)));
    }

    #[test]
    fn fill_rect_touches_only_the_rectangle() {
        let stride = 16; // 8 pixels per line
        let mut buf = vec![0u8; stride * 4];
        fill_rect_rgb565(&mut buf, stride, 1, 1, 3, 2, 0xF800);
        for y in 0..4 {
            for x in 0..8 {
                let px = u16::from_le_bytes([buf[y * stride + x * 2], buf[y * stride + x * 2 + 1]]);
                let inside = (1..4).contains(&x) && (1..3).contains(&y);
                assert_eq!(px, if inside { 0xF800 } else { 0 }, "({x},{y})");
            }
        }
    }

    #[test]
    fn fill_rect_clips_at_the_buffer_end() {
        let mut buf = vec![0u8; 16 * 2];
        fill_rect_rgb565(&mut buf, 16, 0, 1, 8, 5, 0xFFFF);
        assert!(buf[16..].iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn disable_clears_only_sync_enable() {
        let mut bus = MockBus::new();
        bus.preset(reg::DSCTRL, 0x8000_000A);
        let mut d = Display::new(bus);
        d.disable();
        assert_eq!(d.into_inner().value(reg::DSCTRL), 0x0000_000A);
    }
}
