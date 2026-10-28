//! Intel 2700G display controller: panel timing, the graphics plane, and
//! (in [`layers`]) the video plane and how the planes are mixed.
//!
//! Written from `docs/hw/2700g.md` ("Display programming model", "Plane
//! composition"). Clock/PLL and local-memory power-up aren't specified yet;
//! under Windows Mobile they're already configured when RetroPane starts
//! from the SD card.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod blendtest;
pub mod clock;
pub mod console;
pub mod crash;
pub mod filter;
pub mod font;
pub mod layers;
pub mod pattern;
pub mod scanout;

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
    /// The graphics plane's gamma table: [`super::scanout::GAMMA_ENTRIES`]
    /// words `0x00RRGGBB` from here (`GFXGAM0`-`16`), used while `GSCTRL`
    /// bit 26 is set.
    pub const GFXGAM0: usize = 0x2250;
    /// Display sync control.
    pub const DSCTRL: usize = 0x2154;
    /// The display's memory control (mbxfb's name; Windows Mobile leaves
    /// `0x10303010`, Linux writes `0xD0303010`: four byte fields, their
    /// meaning unread; rp-gpubench run GC2BC measures them as the fetch's
    /// burst or FIFO thresholds).
    pub const DMCTRL: usize = 0x2188;
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
    /// Display interrupt status: a plain read/write register (measured
    /// 2026-10-14): events set bits, a write stores its value, so writing
    /// a 1 *sets* a bit. Clear only with [`super::ack_events`].
    pub const DINTRS: usize = 0x2178;
    /// Display interrupt enable.
    pub const DINTRE: usize = 0x217C;
    /// The current scan line (bits 0-11).
    pub const DVLNUM: usize = 0x2308;
    /// The display PLL: `13 MHz x M / (N x 2^P)`, M bits 12-7, N 6-4, P
    /// 3-1, bit 0 enable (mbxfb; docs/hw/2700g.md S26c).
    pub const DISPPLL: usize = 0x03C;
    /// PLL status: bit 0 the display PLL locked.
    pub const PLLSTAT: usize = 0x040;
}

/// The display PLL settings RetroPane knows (display calibration C3,
/// docs/design/display-calibration.md).
pub mod pll {
    /// Windows Mobile's: M 29, N 4, P 2 = 23.5625 MHz, 56.4 Hz on the X51v
    /// (under the Sharp panel's 24.0-26.3 MHz).
    pub const WM: u32 = 0xEC5;
    /// RetroPane's: M 31 = 25.1875 MHz, the panel's typical clock, 60.3 Hz
    /// (device-proven 2026-10-17 with `rp-refresh`: no under/overrun at 32
    /// bpp). A 56 Hz compatibility switch per legacy program is planned for
    /// a game that times itself by the refresh.
    pub const SIXTY: u32 = 0xFC5;
}

/// `DINTRS`/`DINTRE` bits (mbxfb's names; measured on the X51v 2026-10-14).
pub mod event {
    /// Horizontal blank: once a line.
    pub const HBLNK0: u32 = 1 << 0;
    /// Once a frame, on the line in `DVECTRL` bits 16-27 (moveable).
    pub const VEVENT0: u32 = 1 << 2;
    /// Underruns (bits 4-6) and overruns (16-18) of the two streams and
    /// the cursor.
    pub const UNDER_OVERRUN: u32 = 0x0007_0070;
}

/// The graphics plane's fetch from local memory on or off (`GSCADR` bit
/// 31, its stream enable; the panel's timing runs on, so the screen shows
/// no picture while off). Returns whether it was on. Test G16: the display
/// kept off local memory while the 3D core renders.
pub fn graphics_stream<B: RegisterBus>(regs: &mut B, on: bool) -> bool {
    let was = GSCADR_STR_EN.extract(regs.read32(reg::GSCADR)) != 0;
    regs.modify(reg::GSCADR, GSCADR_STR_EN, u32::from(on));
    was
}

/// Every register Linux mbxfb names below the MBX block (its header,
/// `local/re/mbxfb_regs.h`), the write-only resets and clears left out:
/// every one read safely under the loader on the X51v (`rp-gpubench` runs
/// S and Z, 2026-10-20; shared with `rp-glesfeat` G17).
pub const SNAPSHOT_REGS: [(&str, usize); 146] = [
    ("SYSCFG", 0x0000),
    ("PFBASE", 0x0004),
    ("PFCEIL", 0x0008),
    ("POLLFLAG", 0x000C),
    ("NINTPW", 0x0014),
    ("MINTENABLE", 0x0018),
    ("MINTSTAT", 0x001C),
    ("SINTENABLE", 0x0020),
    ("SINTSTAT", 0x0024),
    ("SYSCLKSRC", 0x002C),
    ("PIXCLKSRC", 0x0030),
    ("CLKSLEEP", 0x0034),
    ("COREPLL", 0x0038),
    ("DISPPLL", 0x003C),
    ("PLLSTAT", 0x0040),
    ("VOVRCLK", 0x0044),
    ("PIXCLK", 0x0048),
    ("MEMCLK", 0x004C),
    ("M24CLK", 0x0050),
    ("MBXCLK", 0x0054),
    ("SDCLK", 0x0058),
    ("PIXCLKDIV", 0x005C),
    ("LCD_CONFIG", 0x0060),
    ("ODFBPWR", 0x0064),
    ("ODFBSTAT", 0x0068),
    ("GPIOCGF", 0x006C),
    ("GPIOHI", 0x0070),
    ("GPIOLO", 0x0074),
    ("GPIOSTAT", 0x0078),
    ("PWMCFG", 0x0204),
    ("PWM0DIV", 0x0210),
    ("PWM0DUTY", 0x0214),
    ("PWM0PER", 0x0218),
    ("PWM1DIV", 0x0220),
    ("PWM1DUTY", 0x0224),
    ("PWM1PER", 0x0228),
    ("ID", 0x0FF0),
    ("LMCFG", 0x1004),
    ("LMPWR", 0x1008),
    ("LMPWRSTAT", 0x100C),
    ("LMCEMR", 0x1010),
    ("LMTYPE", 0x1014),
    ("LMTIM", 0x1018),
    ("LMREFRESH", 0x101C),
    ("LMPROTMIN", 0x1020),
    ("LMPROTMAX", 0x1024),
    ("LMPROTCFG", 0x1028),
    ("LMPROTERR", 0x102C),
    ("GSCTRL", 0x2000),
    ("VSCTRL", 0x2004),
    ("GBBASE", 0x2020),
    ("VBBASE", 0x2024),
    ("GDRCTRL", 0x2040),
    ("VCMSK", 0x2044),
    ("GSCADR", 0x2060),
    ("VSCADR", 0x2064),
    ("VUBASE", 0x2084),
    ("VVBASE", 0x20A4),
    ("GSADR", 0x20C0),
    ("VSADR", 0x20C4),
    ("HCCTRL", 0x2100),
    ("HCSIZE", 0x2110),
    ("HCPOS", 0x2120),
    ("HCBADR", 0x2130),
    ("HCCKMSK", 0x2140),
    ("GPLUT", 0x2150),
    ("DSCTRL", 0x2154),
    ("DHT01", 0x2158),
    ("DHT02", 0x215C),
    ("DHT03", 0x2160),
    ("DVT01", 0x2164),
    ("DVT02", 0x2168),
    ("DVT03", 0x216C),
    ("DBCOL", 0x2170),
    ("BGCOLOR", 0x2174),
    ("DINTRS", 0x2178),
    ("DINTRE", 0x217C),
    ("DINTRCNT", 0x2180),
    ("DSIG", 0x2184),
    ("DMCTRL", 0x2188),
    ("CLIPCTRL", 0x218C),
    ("SPOCTRL", 0x2190),
    ("SVCTRL", 0x2194),
    ("VSCOEFF0", 0x2198),
    ("VSCOEFF1", 0x219C),
    ("VSCOEFF2", 0x21A0),
    ("VSCOEFF3", 0x21A4),
    ("VSCOEFF4", 0x21A8),
    ("SHCTRL", 0x21B0),
    ("HSCOEFF0", 0x21B4),
    ("HSCOEFF1", 0x21B8),
    ("HSCOEFF2", 0x21BC),
    ("HSCOEFF3", 0x21C0),
    ("HSCOEFF4", 0x21C4),
    ("HSCOEFF5", 0x21C8),
    ("HSCOEFF6", 0x21CC),
    ("HSCOEFF7", 0x21D0),
    ("HSCOEFF8", 0x21D4),
    ("SSSIZE", 0x21D8),
    ("VIDGAM0", 0x2200),
    ("VIDGAM1", 0x2204),
    ("VIDGAM2", 0x2208),
    ("VIDGAM3", 0x220C),
    ("VIDGAM4", 0x2210),
    ("VIDGAM5", 0x2214),
    ("VIDGAM6", 0x2218),
    ("VIDGAM7", 0x221C),
    ("VIDGAM8", 0x2220),
    ("VIDGAM9", 0x2224),
    ("VIDGAM10", 0x2228),
    ("VIDGAM11", 0x222C),
    ("VIDGAM12", 0x2230),
    ("VIDGAM13", 0x2234),
    ("VIDGAM14", 0x2238),
    ("VIDGAM15", 0x223C),
    ("VIDGAM16", 0x2240),
    ("GFXGAM0", 0x2250),
    ("GFXGAM1", 0x2254),
    ("GFXGAM2", 0x2258),
    ("GFXGAM3", 0x225C),
    ("GFXGAM4", 0x2260),
    ("GFXGAM5", 0x2264),
    ("GFXGAM6", 0x2268),
    ("GFXGAM7", 0x226C),
    ("GFXGAM8", 0x2270),
    ("GFXGAM9", 0x2274),
    ("GFXGAM10", 0x2278),
    ("GFXGAM11", 0x227C),
    ("GFXGAM12", 0x2280),
    ("GFXGAM13", 0x2284),
    ("GFXGAM14", 0x2288),
    ("GFXGAM15", 0x228C),
    ("GFXGAM16", 0x2290),
    ("DLSTS", 0x2300),
    ("DLLCTRL", 0x2304),
    ("DVLNUM", 0x2308),
    ("DUCTRL", 0x230C),
    ("DVECTRL", 0x2310),
    ("DHDET", 0x2314),
    ("DVDET", 0x2318),
    ("DODMSK", 0x231C),
    ("CSC01", 0x2330),
    ("CSC02", 0x2334),
    ("CSC03", 0x2338),
    ("CSC04", 0x233C),
    ("CSC05", 0x2340),
];

/// [`Display::show_video_scaled`] on a register block held elsewhere (a
/// test image's engine): the stream off, the overlay clock on, then the
/// plane's registers; the gamma bit follows the graphics plane's.
pub fn video_show<B: RegisterBus>(
    regs: &mut B,
    video: &layers::VideoPlane,
    layer: layers::Layer,
    scale: layers::Scale,
) {
    let gamma = regs.read_field(reg::GSCTRL, GSCTRL_GAMMA_EN) != 0;
    regs.modify(layers::reg::VSCADR, layers::SCADR_STR_EN, 0);
    layers::video_clock(regs, true);
    if video.scales(scale) {
        for (offset, value) in video.filter_words(scale) {
            regs.write32(offset, value);
        }
    }
    for (offset, value) in video.registers_scaled(layer, gamma, scale) {
        regs.write32(offset, value);
    }
}

/// [`Display::set_graphics`] on a register block held elsewhere (a test
/// image's engine), with `GDRCTRL`'s pixel and line doubling on or off
/// (Dell's 2x for 320 x 240 programs, R16; `rp-gpubench` GC2CA shows a
/// 240 x 320 game frame through it). `GSCTRL`'s gamma bit is kept.
pub fn graphics_set<B: RegisterBus>(
    regs: &mut B,
    plane: &Plane,
    layer: layers::Layer,
    double: bool,
) {
    let gamma = regs.read32(reg::GSCTRL) & GSCTRL_GAMMA_EN.mask();
    for (offset, value) in plane.registers() {
        let value = match offset {
            reg::GSCTRL => value | gamma,
            reg::GSCADR => layer.scadr(value),
            reg::GBBASE => layer.bbase(value),
            reg::GDRCTRL if double => value | blendtest::GDRCTRL_DOUBLE,
            _ => value,
        };
        regs.write32(offset, value);
    }
}

/// [`Display::hide_video`] on a register block held elsewhere.
pub fn video_hide<B: RegisterBus>(regs: &mut B) {
    regs.modify(layers::reg::VSCADR, layers::SCADR_STR_EN, 0);
    layers::video_clock(regs, false);
}

/// Clears the `handled` bits of `DINTRS` the only safe way: read the
/// status, write it back without them (Dell's way). Writing anything else
/// sets every bit written as 1, which made an earlier probe see events
/// that weren't there. Returns the status read.
pub fn ack_events<B: RegisterBus>(regs: &mut B, handled: u32) -> u32 {
    let s = regs.read32(reg::DINTRS);
    regs.write32(reg::DINTRS, s & !handled);
    s
}

const HI12: Field = Field::new(16, 12);
const LO12: Field = Field::new(0, 12);
const GSCTRL_FMT: Field = Field::new(27, 4);
const GSCTRL_GAMMA_EN: Field = Field::bit(26);
const GSCTRL_WIDTH: Field = Field::new(11, 11);
const GSCTRL_HEIGHT: Field = Field::new(0, 11);
const GSCADR_STR_EN: Field = layers::SCADR_STR_EN;
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
    /// 16-bit ARGB 4:4:4:4: 16 levels of alpha, for translucent interface.
    Argb4444 = 4,
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
            Self::Argb4444 | Self::Rgb565 => 2,
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
            4 => PixelFormat::Argb4444,
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

    /// Programs panel timing and the graphics plane, alone and opaque
    /// (video plane and cursor off), then enables scan-out.
    pub fn enable(&mut self, panel: &PanelTiming, plane: &Plane) {
        self.regs.modify(reg::DSCTRL, DSCTRL_SYNCGEN_EN, 0);
        for (offset, value) in timing_registers(panel).into_iter().chain(plane.registers()) {
            self.regs.write32(offset, value);
        }
        layers::solo(&mut self.regs);
        self.regs.modify(reg::DSCTRL, DSCTRL_SYNCGEN_EN, 1);
    }

    /// Takes the screen for drawing, the one way every image and the loader
    /// do it: keeps a 480 × 640 RGB565 plane already scanning out (no
    /// flicker; what Windows Mobile leaves) or sets one up, and in both
    /// cases makes it the only layer, opaque. Without that, a video plane or
    /// blend mode a previous program left on would cover the screen.
    /// Also says whether the plane was kept (`true`) or set up anew.
    pub fn adopt(&mut self, vram_size: usize) -> (Plane, bool) {
        self.refresh_60();
        if let Some(p) = self.scanned_out(vram_size) {
            layers::solo(&mut self.regs);
            (p, true)
        } else {
            let p = Plane::fullscreen(&X51V_VGA, PixelFormat::Rgb565);
            self.enable(&X51V_VGA, &p);
            (p, false)
        }
    }

    /// Moves the display from Windows Mobile's 56 Hz pixel clock to the
    /// panel's typical one, 60 Hz ([`pll::SIXTY`]), and waits for the PLL's
    /// lock (at once on the device; bounded). Only from exactly
    /// [`pll::WM`]: any other value (a cold 2700G, a setting this code
    /// doesn't know) is left alone. Says whether it switched.
    pub fn refresh_60(&mut self) -> bool {
        if self.regs.read32(reg::DISPPLL) != pll::WM {
            return false;
        }
        self.regs.write32(reg::DISPPLL, pll::SIXTY);
        for _ in 0..10_000 {
            if self.regs.read32(reg::PLLSTAT) & 1 != 0 {
                break;
            }
        }
        true
    }

    /// The 480 × 640 RGB565 plane scanning out now, if any: the plane
    /// [`adopt`](Self::adopt) keeps. Reads registers only.
    fn scanned_out(&mut self, vram_size: usize) -> Option<Plane> {
        Plane::decode(
            self.regs.read32(reg::GSCTRL),
            self.regs.read32(reg::GSCADR),
            self.regs.read32(reg::GSADR),
            vram_size,
        )
        .filter(|p| p.format == PixelFormat::Rgb565 && p.width == 480 && p.height == 640)
    }

    /// Where to draw so it shows, for crash and fault paths: the plane
    /// scanning out now (Windows Mobile leaves it at offset 0, not
    /// [`FRAMEBUFFER_OFFSET`]), or [`Plane::fullscreen`] if none can be
    /// drawn into. Reads registers only, never programs the display, so it
    /// is safe on a path that never returns to the display's owner.
    pub fn on_screen(&mut self, vram_size: usize) -> Plane {
        self.scanned_out(vram_size)
            .unwrap_or(Plane::fullscreen(&X51V_VGA, PixelFormat::Rgb565))
    }

    /// Points the graphics plane at `plane`, in `layer`, without touching
    /// the panel timing. `GSCTRL`'s gamma bit is kept.
    pub fn set_graphics(&mut self, plane: &Plane, layer: layers::Layer) {
        graphics_set(&mut self.regs, plane, layer, false);
    }

    /// Shows the video plane `video` in `layer`: its overlay clock on, then
    /// its registers in [`VideoPlane::registers`](layers::VideoPlane::registers)'s
    /// order. The gamma bit follows the graphics plane's.
    pub fn show_video(&mut self, video: &layers::VideoPlane, layer: layers::Layer) {
        self.show_video_scaled(video, layer, layers::Scale::NONE);
    }

    /// As [`Self::show_video`], through the scaler at `scale`'s size and
    /// place ([`layers::VideoPlane::registers_scaled`]).
    pub fn show_video_scaled(
        &mut self,
        video: &layers::VideoPlane,
        layer: layers::Layer,
        scale: layers::Scale,
    ) {
        video_show(&mut self.regs, video, layer, scale);
    }

    /// Hides the video plane and stops its overlay clock.
    pub fn hide_video(&mut self) {
        video_hide(&mut self.regs);
    }

    /// The register block, for reads and snapshots.
    pub fn regs(&mut self) -> &mut B {
        &mut self.regs
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
    fn graphics_set_doubles_only_when_asked_and_keeps_the_gamma_bit() {
        let plane = Plane {
            format: PixelFormat::Rgb565,
            width: 240,
            height: 320,
            offset: 0xF000,
            stride: 480,
        };
        for double in [false, true] {
            let mut bus = rp_hal_mock::MockBus::new();
            bus.write32(reg::GSCTRL, GSCTRL_GAMMA_EN.mask());
            graphics_set(&mut bus, &plane, layers::Layer::SOLO, double);
            let w = bus.writes();
            let last = |r: usize| w.iter().rev().find(|(o, _)| *o == r).map(|(_, v)| *v);
            let want = if double { 0xA0FF_FFFF } else { 0x00FF_FFFF };
            assert_eq!(last(reg::GDRCTRL), Some(want));
            let gsctrl = last(reg::GSCTRL).unwrap_or(0);
            assert_ne!(gsctrl & GSCTRL_GAMMA_EN.mask(), 0);
            assert_eq!(GSCTRL_WIDTH.extract(gsctrl), 239);
            assert_eq!(GSCTRL_HEIGHT.extract(gsctrl), 319);
        }
    }

    #[test]
    fn the_scaler_filter_loads_only_when_the_plane_scales_and_before_it_does() {
        let v = layers::VideoPlane::new(layers::VideoFormat::Uyvy, 240, 320, 0x40_0000);
        let filter = |w: &[(usize, u32)]| {
            w.iter()
                .filter(|a| layers::SCALER_FILTER.contains(a))
                .count()
        };
        // 1:1, and an explicit size equal to the source's: no filter.
        for scale in [
            layers::Scale::NONE,
            layers::Scale {
                out_w: 240,
                out_h: 320,
                ..layers::Scale::NONE
            },
        ] {
            let mut bus = rp_hal_mock::MockBus::new();
            video_show(&mut bus, &v, layers::Layer::SOLO, scale);
            assert_eq!(filter(&bus.writes()), 0, "{scale:?}");
        }
        // Up 2x, or one direction alone: all fifteen, before SPOCTRL.
        for (w, h) in [(480, 640), (480, 0), (0, 640)] {
            let mut bus = rp_hal_mock::MockBus::new();
            let scale = layers::Scale {
                out_w: w,
                out_h: h,
                ..layers::Scale::NONE
            };
            video_show(&mut bus, &v, layers::Layer::SOLO, scale);
            let writes = bus.writes();
            assert_eq!(filter(&writes), 15, "{scale:?}");
            let last_filter = writes
                .iter()
                .rposition(|a| layers::SCALER_FILTER.contains(a))
                .unwrap();
            let spoctrl = writes
                .iter()
                .position(|(o, _)| *o == layers::reg::SPOCTRL)
                .unwrap();
            assert!(last_filter < spoctrl, "{scale:?}");
        }
    }

    #[test]
    fn ack_writes_back_only_the_rest() {
        let mut bus = rp_hal_mock::MockBus::new();
        bus.preset(reg::DINTRS, event::VEVENT0 | event::HBLNK0 | 0x10);
        assert_eq!(ack_events(&mut bus, event::VEVENT0), 0x15);
        // Never a 1 the status didn't already hold: a 1 would set it.
        assert_eq!(bus.writes(), [(reg::DINTRS, event::HBLNK0 | 0x10)]);
    }

    #[test]
    fn the_graphics_stream_switches_bit_31_alone() {
        let mut bus = rp_hal_mock::MockBus::new();
        // A plane at 0x40000 / 16 scanning out (stream on).
        bus.preset(reg::GSCADR, 1 << 31 | 0x4000);
        assert!(graphics_stream(&mut bus, false), "it was on");
        assert_eq!(bus.read32(reg::GSCADR), 0x4000, "the address kept");
        assert!(!graphics_stream(&mut bus, true), "then off");
        assert_eq!(bus.read32(reg::GSCADR), 1 << 31 | 0x4000);
    }

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
    fn adopt_keeps_wm_plane_but_switches_off_left_over_layers() {
        // WM's plane, plus what a test image might leave: video on top,
        // graphics in per-pixel-alpha mode.
        let wm = Plane {
            offset: 0,
            ..Plane::fullscreen(&X51V_VGA, PixelFormat::Rgb565)
        };
        let r = wm.registers();
        let mut bus = MockBus::new();
        bus.preset(reg::GSCTRL, r[0].1);
        bus.preset(reg::GSCADR, r[3].1 | (3 << 27));
        bus.preset(reg::GSADR, r[4].1);
        bus.preset(layers::reg::VSCADR, 0x8100_0000 | 0x5_0000);
        let mut d = Display::new(bus);
        assert_eq!(d.adopt(16 << 20), (wm, true));
        let bus = d.into_inner();
        assert_eq!(bus.value(reg::GSCADR), r[3].1, "opaque, bottom slot");
        assert_eq!(bus.value(layers::reg::VSCADR) >> 31, 0, "video off");
        assert!(!bus.writes().contains(&(reg::DSCTRL, 0)), "no blanking");

        // A plane it can't draw into (ARGB4444) is replaced, video off too.
        let mut bus = MockBus::new();
        bus.preset(reg::GSCTRL, 4 << 27 | r[0].1 & 0x07FF_FFFF);
        bus.preset(reg::GSCADR, r[3].1);
        bus.preset(reg::GSADR, r[4].1);
        bus.preset(layers::reg::VSCADR, 0x8000_0000);
        let mut d = Display::new(bus);
        let (p, kept) = d.adopt(16 << 20);
        assert!(!kept);
        assert_eq!(p, Plane::fullscreen(&X51V_VGA, PixelFormat::Rgb565));
        assert_eq!(d.into_inner().value(layers::reg::VSCADR) >> 31, 0);
    }

    #[test]
    fn adopt_moves_windows_mobiles_clock_to_60_hz_and_nothing_else() {
        let mut bus = MockBus::new();
        bus.preset(reg::DISPPLL, pll::WM);
        bus.preset(reg::PLLSTAT, 7);
        let mut d = Display::new(bus);
        let _ = d.adopt(16 << 20);
        assert_eq!(d.into_inner().value(reg::DISPPLL), pll::SIXTY);
        // Any other value stays: a cold chip, or a setting not known here.
        for other in [0, 0xEC4, 0xBB1, pll::SIXTY] {
            let mut bus = MockBus::new();
            bus.preset(reg::DISPPLL, other);
            let mut d = Display::new(bus);
            assert!(!d.refresh_60());
            let bus = d.into_inner();
            assert_eq!(bus.value(reg::DISPPLL), other);
            assert!(!bus.writes().iter().any(|w| w.0 == reg::DISPPLL));
        }
    }

    #[test]
    fn on_screen_finds_wm_plane_at_offset_0_without_writing() {
        // As measured on a real X51v after HaRET (docs/hw/2700g.md).
        let mut bus = MockBus::new();
        bus.preset(reg::GSCTRL, 0x3C0E_FA7F);
        bus.preset(reg::GSCADR, 0x8000_0000);
        bus.preset(reg::GSADR, 0x0EC0_0000);
        let mut d = Display::new(bus);
        let p = d.on_screen(16 << 20);
        assert_eq!(p.offset, 0);
        assert_eq!(
            p,
            Plane {
                offset: 0,
                ..Plane::fullscreen(&X51V_VGA, PixelFormat::Rgb565)
            }
        );
        assert!(d.into_inner().writes().is_empty(), "reads only");

        // Scanning off (the emulator from reset): the conventional plane.
        let mut d = Display::new(MockBus::new());
        assert_eq!(
            d.on_screen(16 << 20),
            Plane::fullscreen(&X51V_VGA, PixelFormat::Rgb565)
        );
        assert!(d.into_inner().writes().is_empty());
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
