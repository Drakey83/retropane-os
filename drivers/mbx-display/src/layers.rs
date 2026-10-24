//! The display controller's layers: the video plane, and how the planes
//! are mixed during scan-out (docs/hw/2700g.md, "Plane composition").
//!
//! Each plane has a **layer slot** (0 bottom, 1 middle, 2 top) and a
//! **blend mode** saying how it covers what lies below: opaque, with its
//! pixels' alpha, the inverse of that, or one global alpha. Behind every
//! plane is the background colour. Register layouts are from the public
//! Linux `mbxfb` definitions; their meaning is checked on the device by
//! `rp-blend`.

use rp_hal_api::{Field, RegisterBus};

/// Register offsets for the layers.
pub mod reg {
    /// Video overlay clock (bit 0 enables).
    pub const VOVRCLK: usize = 0x0044;
    /// Video surface control.
    pub const VSCTRL: usize = 0x2004;
    /// Video blending base: global alpha `[31:24]`, colour key.
    pub const VBBASE: usize = 0x2024;
    /// Video stream control / start address.
    pub const VSCADR: usize = 0x2064;
    /// Video U (Cb) plane base.
    pub const VUBASE: usize = 0x2084;
    /// Video V (Cr) plane base.
    pub const VVBASE: usize = 0x20A4;
    /// Video stride / position.
    pub const VSADR: usize = 0x20C4;
    /// Hardware cursor control.
    pub const HCCTRL: usize = 0x2100;
    /// Background colour, `0x00RRGGBB`, behind every plane.
    pub const BGCOLOR: usize = 0x2174;
    /// Scaler: pitch and order control.
    pub const SPOCTRL: usize = 0x2190;
    /// Scaler: the vertical filter's two initial phases.
    pub const SVCTRL: usize = 0x2194;
    /// Scaler: the vertical filter's coefficients, five words from here.
    pub const VSCOEFF0: usize = 0x2198;
    /// Scaler: horizontal control.
    pub const SHCTRL: usize = 0x21B0;
    /// Scaler: the horizontal filter's coefficients, nine words from here.
    pub const HSCOEFF0: usize = 0x21B4;
    /// Scaler: output size.
    pub const SSSIZE: usize = 0x21D8;
}

/// Stream enable, in `GSCADR` and `VSCADR`.
pub(crate) const SCADR_STR_EN: Field = Field::bit(31);
const SCADR_COLKEY_EN: Field = Field::bit(30);
const SCADR_BLEND_M: Field = Field::new(27, 2);
const SCADR_BLEND_POS: Field = Field::new(24, 2);
const SCADR_BASE: Field = Field::new(0, 23);
const BBASE_GLALPHA: Field = Field::new(24, 8);
const VSCTRL_FMT: Field = Field::new(27, 4);
const VSCTRL_GAMMA_EN: Field = Field::bit(26);
const VSCTRL_CSC_EN: Field = Field::bit(25);
const VSCTRL_WIDTH: Field = Field::new(11, 11);
const VSCTRL_HEIGHT: Field = Field::new(0, 11);
const VUBASE_UVHALFSTR: Field = Field::bit(31);
const CBASE: Field = Field::new(0, 24);
const VSADR_STRIDE: Field = Field::new(22, 10);
const HCCTRL_CUR_EN: Field = Field::bit(31);
const VOVRCLK_EN: Field = Field::bit(0);
const SPOCTRL_H_BYPASS: u32 = 1 << 31;
const SPOCTRL_V_BYPASS: u32 = 1 << 30;
/// `SPOCTRL` bit 27, mbxfb's `SPOCTRL_VS_UR_C` (cleared by mbxfb, set by
/// Windows Mobile: `0xC8030000`, probe 2026-10-11): meaning undocumented;
/// the gate's P9 run 1 saw the video stream underrun (`DINTRS` bit 5)
/// without it when scaling down.
// inherited: C70, Windows Mobile (SPOCTRL 0xC8030000, probe 2026-10-11)
const SPOCTRL_VS_UR_C: u32 = 1 << 27;
/// The scaler's filter as Linux mbxfb loads it at start-up (`mbxfb_setup`:
/// `SVCTRL`'s two initial phases `1 << 10`, then `VSCOEFF0-4` and
/// `HSCOEFF0-8`, "good for scaling up; for scaling down, the application
/// has to calculate them"). Windows Mobile leaves all fifteen at 0 with the
/// scaler bypassed (`SPOCTRL` `0xC8030000`, run S1), and Dell's drivers
/// never write them (`ddi_2700g` read 2026-10-24): the gate's P9 runs
/// scaled with them at 0 and showed one flat colour. Loaded only when the
/// plane scales ([`VideoPlane::scales`]).
// inherited: C73, Linux mbxfb (drivers/video/fbdev/mbx/mbxfb.c, mbxfb_setup)
pub const SCALER_FILTER: [(usize, u32); 15] = [
    (reg::SVCTRL, (1 << 10) << 16 | 1 << 10),
    (reg::VSCOEFF0, 0xFF00_0100),
    (reg::VSCOEFF0 + 4, 0xFDFC_FDFE),
    (reg::VSCOEFF0 + 8, 0x170D_0500),
    (reg::VSCOEFF0 + 12, 0x3D37_2D22),
    (reg::VSCOEFF0 + 16, 0x0000_0040),
    (reg::HSCOEFF0, 0xFF01_0100),
    (reg::HSCOEFF0 + 4, 0x0000_0000),
    (reg::HSCOEFF0 + 8, 0x0201_0000),
    (reg::HSCOEFF0 + 12, 0x0102_0302),
    (reg::HSCOEFF0 + 16, 0xF9FB_FE00),
    (reg::HSCOEFF0 + 20, 0xFBF7_F6F7),
    (reg::HSCOEFF0 + 24, 0x1C11_0700),
    (reg::HSCOEFF0 + 28, 0x3E39_3127),
    (reg::HSCOEFF0 + 32, 0x0000_0040),
];

/// Scaler pitch for 1:1 (source / output in 5.11 fixed point).
const PITCH_1_TO_1: u32 = 1 << 11;
/// `VSADR`'s screen position (mbxfb's `VSADR_XSTART`/`YSTART`; as the
/// plane's place on screen **assumed** until the gate's P9 run shows it).
const VSADR_X: Field = Field::new(11, 11);
const VSADR_Y: Field = Field::new(0, 11);
/// `SPOCTRL`'s vertical filter taps (mbxfb `SPOCTRL_VORDER`: 0 one tap,
/// 1 two, 3 four) and its vertical pitch, 5.11 (`SPOCTRL_VPITCH`).
const SPOCTRL_VORDER: Field = Field::new(16, 2);
const SPOCTRL_VPITCH: Field = Field::new(0, 16);
/// `SHCTRL`'s first-phase offset (mbxfb sets `4 << 11` at start-up;
/// Windows Mobile's `0x20000000` holds the same: probe 2026-10-11) and its
/// horizontal pitch, 5.11 (`SHCTRL_HPITCH`, 15 bits). The phase `4 << 11`
/// written with it is copied:
// inherited: C70, mbxfb and Windows Mobile (SHCTRL 0x20000000)
const SHCTRL_HINITIAL: Field = Field::new(16, 16);
const SHCTRL_HPITCH: Field = Field::new(0, 15);
/// The largest ratios the scaler is driven at: 2:1 down (mbxfb has a
/// decimate bit for more, not used) and 8:1 up (the pitch then 0x100).
const MAX_DOWN: u32 = 2;
const MAX_UP: u32 = 8;

/// The video plane's scaler and place on screen: the source picture
/// (`VideoPlane`'s size) shown `out_w` x `out_h` with its top left at
/// (`x`, `y`). An output size of 0 keeps the source's in that direction;
/// [`Scale::NONE`] is 1:1 at the corner, the scaler bypassed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scale {
    /// Width on screen, pixels (0: the source's).
    pub out_w: u16,
    /// Height on screen, pixels (0: the source's).
    pub out_h: u16,
    /// Left edge on screen.
    pub x: u16,
    /// Top edge on screen.
    pub y: u16,
    /// Vertical filter taps: 1, 2 or 4.
    pub taps: u8,
    /// `SPOCTRL` bit 27 (`VS_UR_C`), as Windows Mobile sets it.
    pub ur_c: bool,
}

impl Scale {
    /// The source's own size at the top left.
    pub const NONE: Self = Self {
        out_w: 0,
        out_h: 0,
        x: 0,
        y: 0,
        taps: 1,
        ur_c: false,
    };
}

/// A layer slot: which plane covers which.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// Slot 0, drawn first, over the background colour.
    Bottom = 0,
    /// Slot 1.
    Middle = 1,
    /// Slot 2, drawn last.
    Top = 2,
}

/// How a plane covers what lies below it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend {
    /// Fully covers it.
    Opaque,
    /// Each pixel's alpha (ARGB formats; others count as opaque).
    PixelAlpha,
    /// One minus each pixel's alpha.
    InverseAlpha,
    /// One alpha for the whole plane, 0 (invisible) to 255 (opaque).
    Global(u8),
}

impl Blend {
    const fn mode(self) -> u32 {
        match self {
            Self::Opaque => 0,
            Self::InverseAlpha => 1,
            Self::Global(_) => 2,
            Self::PixelAlpha => 3,
        }
    }

    const fn global(self) -> u32 {
        match self {
            Self::Global(a) => a as u32,
            _ => 0xFF,
        }
    }
}

/// A plane's place in the mix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layer {
    /// Where it sits.
    pub slot: Slot,
    /// How it covers what's below.
    pub blend: Blend,
}

impl Layer {
    /// Opaque, at the bottom: how a lone graphics plane is shown.
    pub const SOLO: Self = Self {
        slot: Slot::Bottom,
        blend: Blend::Opaque,
    };

    /// `xSCADR` with this layer's slot and mode, colour keying off.
    #[must_use]
    pub const fn scadr(self, scadr: u32) -> u32 {
        let v = SCADR_BLEND_M.insert(scadr, self.blend.mode());
        let v = SCADR_BLEND_POS.insert(v, self.slot as u32);
        SCADR_COLKEY_EN.insert(v, 0)
    }

    /// `xBBASE` with this layer's global alpha (the colour key kept).
    #[must_use]
    pub const fn bbase(self, bbase: u32) -> u32 {
        BBASE_GLALPHA.insert(bbase, self.blend.global())
    }
}

/// Video plane pixel formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoFormat {
    /// Packed 4:2:2, bytes U Y0 V Y1 ("UYVY"): 2 bytes a pixel.
    Uyvy,
    /// Planar 4:2:0: a Y plane, then U and V planes of half width and half
    /// height ("YV12"/"I420", what JPEG decodes to): 1.5 bytes a pixel.
    Yuv420,
}

/// The video plane: shown under (or over) the graphics plane, in YUV.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoPlane {
    /// Pixel format.
    pub format: VideoFormat,
    /// Width in pixels (even).
    pub width: u16,
    /// Height in pixels (even).
    pub height: u16,
    /// Start of the (Y or packed) plane in 2700G memory, 16-byte aligned.
    pub offset: usize,
    /// Bytes per line of that plane, a multiple of 32 for 4:2:0 (so the
    /// chroma planes, at half the stride, stay 16-byte aligned).
    pub stride: usize,
    /// Start of the U plane (4:2:0 only), 8-byte aligned.
    pub u_offset: usize,
    /// Start of the V plane (4:2:0 only), 8-byte aligned.
    pub v_offset: usize,
}

impl VideoPlane {
    /// A `width` × `height` plane at `offset`, tightly packed, the chroma
    /// planes (4:2:0) right after the Y plane.
    #[must_use]
    pub const fn new(format: VideoFormat, width: u16, height: u16, offset: usize) -> Self {
        let (stride, u_offset, v_offset) = match format {
            VideoFormat::Uyvy => ((width as usize * 2).div_ceil(16) * 16, 0, 0),
            VideoFormat::Yuv420 => {
                let stride = (width as usize).div_ceil(32) * 32;
                let u = offset + stride * height as usize;
                (stride, u, u + stride / 2 * (height as usize / 2))
            }
        };
        Self {
            format,
            width,
            height,
            offset,
            stride,
            u_offset,
            v_offset,
        }
    }

    /// Bytes of 2700G memory it covers, from `offset`.
    #[must_use]
    pub const fn bytes(&self) -> usize {
        match self.format {
            VideoFormat::Uyvy => self.stride * self.height as usize,
            VideoFormat::Yuv420 => {
                self.v_offset + self.stride / 2 * (self.height as usize / 2) - self.offset
            }
        }
    }

    /// The video registers as `(offset, value)` pairs in a safe programming
    /// order: the chroma bases before `VSCTRL` (mbxfb: switching to a planar
    /// format with them unset hangs the chip), the scaler set to 1:1, the
    /// stream enable last. `gamma` keeps the gamma bit Windows Mobile set.
    ///
    /// # Panics
    /// If the layout isn't aligned as documented on the fields, or a size is
    /// out of range for the hardware fields.
    #[must_use]
    pub fn registers(&self, layer: Layer, gamma: bool) -> [(usize, u32); 9] {
        self.registers_scaled(layer, gamma, Scale::NONE)
    }

    /// Whether `scale` makes the scaler change the picture's size (either
    /// direction), so its filter ([`SCALER_FILTER`]) is in use.
    #[must_use]
    pub fn scales(&self, scale: Scale) -> bool {
        (scale.out_w != 0 && scale.out_w != self.width)
            || (scale.out_h != 0 && scale.out_h != self.height)
    }

    /// As [`Self::registers`], shown through the scaler at `scale`'s size
    /// and place (mbxfb's overlay set-up: the output size in `SSSIZE`, each
    /// pitch source / output in 5.11, a direction bypassed when 1:1).
    ///
    /// # Panics
    /// As [`Self::registers`]; also if the output or place leaves the
    /// 11-bit fields, the ratio is past 2:1 down or 8:1 up, or `taps` isn't
    /// 1, 2 or 4.
    #[must_use]
    pub fn registers_scaled(&self, layer: Layer, gamma: bool, scale: Scale) -> [(usize, u32); 9] {
        let w = u32::from(self.width);
        let h = u32::from(self.height);
        assert!(
            (2..=2048).contains(&w)
                && (2..=2048).contains(&h)
                && w.is_multiple_of(2)
                && h.is_multiple_of(2)
        );
        assert!(
            self.offset.is_multiple_of(16) && self.stride.is_multiple_of(16),
            "unaligned video plane"
        );
        assert!(
            self.stride >= 16 && self.stride / 16 <= 1024,
            "stride out of range"
        );
        let (fmt, ubase, vbase) = match self.format {
            VideoFormat::Uyvy => (0xC, 0, 0),
            VideoFormat::Yuv420 => {
                assert!(self.stride.is_multiple_of(32), "chroma stride unaligned");
                assert!(
                    self.u_offset.is_multiple_of(8) && self.v_offset.is_multiple_of(8),
                    "unaligned chroma"
                );
                let base = |o: usize| CBASE.insert(0, (o / 8) as u32);
                (
                    0x9,
                    VUBASE_UVHALFSTR.insert(base(self.u_offset), 1),
                    base(self.v_offset),
                )
            }
        };
        let ow = if scale.out_w == 0 {
            w
        } else {
            u32::from(scale.out_w)
        };
        let oh = if scale.out_h == 0 {
            h
        } else {
            u32::from(scale.out_h)
        };
        let (x, y) = (u32::from(scale.x), u32::from(scale.y));
        assert!(
            ow <= 2048 && oh <= 2048 && x < 2048 && y < 2048,
            "scaled size out of range"
        );
        for (src, out) in [(w, ow), (h, oh)] {
            assert!(
                src <= out * MAX_DOWN && out <= src * MAX_UP,
                "scale ratio out of range"
            );
        }
        let vorder = match scale.taps {
            1 => 0,
            2 => 1,
            4 => 3,
            _ => panic!("taps must be 1, 2 or 4"),
        };
        let pitch = |src: u32, out: u32| (src << 11) / out;
        let mut spoctrl = SPOCTRL_VPITCH.insert(SPOCTRL_VORDER.insert(0, vorder), pitch(h, oh));
        if ow == w {
            spoctrl |= SPOCTRL_H_BYPASS;
        }
        if oh == h {
            spoctrl |= SPOCTRL_V_BYPASS;
        }
        if scale.ur_c {
            spoctrl |= SPOCTRL_VS_UR_C;
        }
        let shctrl = if ow == w {
            PITCH_1_TO_1
        } else {
            SHCTRL_HPITCH.insert(SHCTRL_HINITIAL.insert(0, 4 << 11), pitch(w, ow))
        };
        let vsctrl = VSCTRL_FMT.insert(0, fmt);
        let vsctrl = VSCTRL_CSC_EN.insert(vsctrl, 1);
        let vsctrl = VSCTRL_GAMMA_EN.insert(vsctrl, u32::from(gamma));
        let vsctrl = VSCTRL_WIDTH.insert(vsctrl, w - 1);
        let vsctrl = VSCTRL_HEIGHT.insert(vsctrl, h - 1);
        let vscadr = SCADR_STR_EN.insert(
            layer.scadr(SCADR_BASE.insert(0, (self.offset / 16) as u32)),
            1,
        );
        [
            (reg::VUBASE, ubase),
            (reg::VVBASE, vbase),
            (reg::VSCTRL, vsctrl),
            (
                reg::VSADR,
                VSADR_Y.insert(
                    VSADR_X.insert(VSADR_STRIDE.insert(0, (self.stride / 16 - 1) as u32), x),
                    y,
                ),
            ),
            (reg::SSSIZE, ((ow - 1) << 16) | (oh - 1)),
            (reg::SPOCTRL, spoctrl),
            (reg::SHCTRL, shctrl),
            (reg::VBBASE, layer.bbase(0)),
            (reg::VSCADR, vscadr),
        ]
    }
}

/// Makes the graphics plane the only thing on screen: opaque in the bottom
/// slot, video stream and hardware cursor off. Touches only those bits, so
/// a plane already scanning out doesn't flicker.
pub(crate) fn solo<B: RegisterBus>(regs: &mut B) {
    let g = regs.read32(crate::reg::GSCADR);
    let solo = Layer::SOLO.scadr(g);
    if solo != g {
        regs.write32(crate::reg::GSCADR, solo);
    }
    for (offset, field) in [(reg::VSCADR, SCADR_STR_EN), (reg::HCCTRL, HCCTRL_CUR_EN)] {
        if regs.read_field(offset, field) != 0 {
            regs.modify(offset, field, 0);
        }
    }
}

/// Switches the video stream on (with its overlay clock) or off.
pub(crate) fn video_clock<B: RegisterBus>(regs: &mut B, on: bool) {
    regs.modify(reg::VOVRCLK, VOVRCLK_EN, u32::from(on));
}

/// Every register the layer code may change, in the order to put them
/// back: the video stream off first, the graphics plane last.
pub const TOUCHED: [usize; 32] = [
    reg::VSCADR,
    reg::VOVRCLK,
    reg::VUBASE,
    reg::VVBASE,
    reg::VSCTRL,
    reg::VSADR,
    reg::VBBASE,
    reg::SSSIZE,
    reg::SPOCTRL,
    reg::SHCTRL,
    reg::SVCTRL,
    reg::VSCOEFF0,
    reg::VSCOEFF0 + 4,
    reg::VSCOEFF0 + 8,
    reg::VSCOEFF0 + 12,
    reg::VSCOEFF0 + 16,
    reg::HSCOEFF0,
    reg::HSCOEFF0 + 4,
    reg::HSCOEFF0 + 8,
    reg::HSCOEFF0 + 12,
    reg::HSCOEFF0 + 16,
    reg::HSCOEFF0 + 20,
    reg::HSCOEFF0 + 24,
    reg::HSCOEFF0 + 28,
    reg::HSCOEFF0 + 32,
    reg::HCCTRL,
    reg::BGCOLOR,
    crate::reg::GSCTRL,
    crate::reg::GBBASE,
    crate::reg::GDRCTRL,
    crate::reg::GSADR,
    crate::reg::GSCADR,
];

/// The values of [`TOUCHED`] at one moment, to put back later.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot(pub [u32; TOUCHED.len()]);

impl Snapshot {
    /// Reads every register in [`TOUCHED`].
    pub fn take<B: RegisterBus>(regs: &mut B) -> Self {
        Self(TOUCHED.map(|o| regs.read32(o)))
    }

    /// The saved value of register `offset`, if it is one of [`TOUCHED`].
    #[must_use]
    pub fn get(&self, offset: usize) -> Option<u32> {
        TOUCHED.iter().position(|&o| o == offset).map(|i| self.0[i])
    }

    /// Writes the saved values back. The video stream goes off before
    /// anything else changes and comes back (if it was on) only at the end,
    /// so the chip never scans a half-programmed plane.
    pub fn restore<B: RegisterBus>(&self, regs: &mut B) {
        let vscadr = self.0[0];
        regs.write32(reg::VSCADR, SCADR_STR_EN.insert(vscadr, 0));
        for (&o, &v) in TOUCHED.iter().zip(&self.0).skip(1) {
            regs.write32(o, v);
        }
        regs.write32(reg::VSCADR, vscadr);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rp_hal_mock::MockBus;

    #[test]
    fn layer_bits_match_mbxfb() {
        let top = Layer {
            slot: Slot::Middle,
            blend: Blend::PixelAlpha,
        };
        // mbxfb: BLEND_PIX = 3 << 27, BLEND_VID (slot 1) = 1 << 24.
        assert_eq!(top.scadr(0x8000_0000), 0x8000_0000 | (3 << 27) | (1 << 24));
        // Colour keying is switched off, the address kept.
        assert_eq!(Layer::SOLO.scadr(0xC000_1234), 0x8000_1234);
        let g = Layer {
            slot: Slot::Top,
            blend: Blend::Global(0x80),
        };
        assert_eq!(g.scadr(0) >> 24, (2 << 3) | 2);
        assert_eq!(g.bbase(0x00AB_CDEF), 0x80AB_CDEF);
    }

    #[test]
    fn yuv420_layout_and_registers() {
        let v = VideoPlane::new(VideoFormat::Yuv420, 480, 640, 0x50_0000);
        assert_eq!(v.stride, 480);
        assert_eq!(v.u_offset, 0x50_0000 + 480 * 640);
        assert_eq!(v.v_offset, v.u_offset + 240 * 320);
        assert_eq!(v.bytes(), 480 * 640 * 3 / 2);
        let r = v.registers(
            Layer {
                slot: Slot::Bottom,
                blend: Blend::Opaque,
            },
            false,
        );
        // Chroma bases are written before VSCTRL, the enable last.
        assert_eq!(r[0].0, reg::VUBASE);
        assert_eq!(r[1].0, reg::VVBASE);
        assert_eq!(r[2].0, reg::VSCTRL);
        assert_eq!(r[8], (reg::VSCADR, 0x8000_0000 | (0x50_0000 / 16)));
        assert_eq!(r[0].1, (1 << 31) | (v.u_offset / 8) as u32);
        assert_eq!(VSCTRL_FMT.extract(r[2].1), 9);
        assert_eq!(VSCTRL_CSC_EN.extract(r[2].1), 1);
        assert_eq!(VSCTRL_WIDTH.extract(r[2].1), 479);
        assert_eq!(VSCTRL_HEIGHT.extract(r[2].1), 639);
        assert_eq!(VSADR_STRIDE.extract(r[3].1), 29);
    }

    #[test]
    fn uyvy_layout() {
        let v = VideoPlane::new(VideoFormat::Uyvy, 480, 640, 0x40_0000);
        assert_eq!(v.stride, 960);
        assert_eq!(v.bytes(), 480 * 640 * 2);
        let r = v.registers(Layer::SOLO, true);
        assert_eq!(VSCTRL_FMT.extract(r[2].1), 0xC);
        assert_eq!(VSCTRL_GAMMA_EN.extract(r[2].1), 1);
        assert_eq!(VSADR_STRIDE.extract(r[3].1), 59);
    }

    #[test]
    fn unscaled_registers_are_the_bypassed_one_to_one() {
        let v = VideoPlane::new(VideoFormat::Uyvy, 480, 640, 0x40_0000);
        let r = v.registers(Layer::SOLO, false);
        assert_eq!(r[3], (reg::VSADR, 59 << 22));
        assert_eq!(r[4], (reg::SSSIZE, (479 << 16) | 639));
        assert_eq!(r[5], (reg::SPOCTRL, (1 << 31) | (1 << 30) | 0x800));
        assert_eq!(r[6], (reg::SHCTRL, 0x800));
        assert_eq!(r, v.registers_scaled(Layer::SOLO, false, Scale::NONE));
    }

    #[test]
    fn scaled_registers_follow_mbxfb() {
        // 240 x 320 up to the full screen: both pitches 1:2 (0x400).
        let v = VideoPlane::new(VideoFormat::Uyvy, 240, 320, 0x40_0000);
        let up = Scale {
            out_w: 480,
            out_h: 640,
            x: 0,
            y: 0,
            taps: 4,
            ur_c: false,
        };
        let r = v.registers_scaled(Layer::SOLO, false, up);
        assert_eq!(VSCTRL_WIDTH.extract(r[2].1), 239, "VSCTRL keeps the source");
        assert_eq!(r[4], (reg::SSSIZE, (479 << 16) | 639));
        assert_eq!(r[5], (reg::SPOCTRL, (3 << 16) | 0x400));
        assert_eq!(r[6], (reg::SHCTRL, (0x2000 << 16) | 0x400));
        // 480 x 640 down to 240 x 320 at (120, 160): pitches 2:1 (0x1000).
        let v = VideoPlane::new(VideoFormat::Uyvy, 480, 640, 0x40_0000);
        let down = Scale {
            out_w: 240,
            out_h: 320,
            x: 120,
            y: 160,
            taps: 1,
            ur_c: false,
        };
        let r = v.registers_scaled(Layer::SOLO, false, down);
        assert_eq!(r[3], (reg::VSADR, (59 << 22) | (120 << 11) | 160));
        assert_eq!(r[5], (reg::SPOCTRL, 0x1000));
        assert_eq!(SHCTRL_HPITCH.extract(r[6].1), 0x1000);
        // Windows Mobile's bit 27 on: only SPOCTRL changes.
        let r2 = v.registers_scaled(Layer::SOLO, false, Scale { ur_c: true, ..down });
        assert_eq!(r2[5], (reg::SPOCTRL, 0x1000 | (1 << 27)));
        assert_eq!(r2[..5], r[..5]);
        assert_eq!(r2[6..], r[6..]);
        // Horizontal only: the vertical bypassed, its pitch 1:1.
        let wide = Scale {
            out_w: 720,
            ..Scale::NONE
        };
        let r = v.registers_scaled(Layer::SOLO, false, wide);
        assert_eq!(r[5].1, (1 << 30) | 0x800);
        assert_eq!(SHCTRL_HPITCH.extract(r[6].1), (480 << 11) / 720);
        // Vertical only: the horizontal bypassed and left 1:1.
        let tall = Scale {
            out_h: 960,
            ..Scale::NONE
        };
        let r = v.registers_scaled(Layer::SOLO, false, tall);
        assert_eq!(r[5].1, (1 << 31) | ((640 << 11) / 960));
        assert_eq!(r[6].1, 0x800);
        // Placement alone, 1:1: both bypassed, the place in VSADR.
        let placed = Scale {
            x: 2047,
            y: 2047,
            ..Scale::NONE
        };
        let r = v.registers_scaled(Layer::SOLO, false, placed);
        assert_eq!(r[5].1 >> 30, 3);
        assert_eq!(r[3].1 & 0x3F_FFFF, (2047 << 11) | 2047);
    }

    #[test]
    fn scale_edges() {
        let v = VideoPlane::new(VideoFormat::Uyvy, 480, 640, 0x40_0000);
        let s = |out_w, out_h| Scale {
            out_w,
            out_h,
            ..Scale::NONE
        };
        // Exactly 2:1 down and 8:1 up pass.
        let _ = v.registers_scaled(Layer::SOLO, false, s(240, 320));
        let small = VideoPlane::new(VideoFormat::Uyvy, 64, 64, 0x40_0000);
        let r = small.registers_scaled(Layer::SOLO, false, s(512, 512));
        assert_eq!(SHCTRL_HPITCH.extract(r[6].1), 0x100);
        assert_eq!(SPOCTRL_VPITCH.extract(r[5].1), 0x100);
        let bad = |p: VideoPlane, sc: Scale| {
            std::panic::catch_unwind(move || p.registers_scaled(Layer::SOLO, false, sc)).is_err()
        };
        for sc in [s(239, 640), s(480, 319), s(2049, 640), s(480, 2049)] {
            assert!(bad(v, sc), "{sc:?}");
        }
        assert!(bad(small, s(514, 64)));
        assert!(bad(small, s(64, 514)));
        for sc in [
            Scale {
                x: 2048,
                ..Scale::NONE
            },
            Scale {
                y: 2048,
                ..Scale::NONE
            },
            Scale {
                taps: 3,
                ..Scale::NONE
            },
        ] {
            assert!(bad(v, sc), "{sc:?}");
        }
    }

    #[test]
    fn the_scaler_filter_is_mbxfbs_and_is_put_back() {
        // mbxfb_setup's words, register by register.
        assert_eq!(SCALER_FILTER[0], (0x2194, 0x0400_0400));
        let v: Vec<u32> = SCALER_FILTER[1..6].iter().map(|r| r.1).collect();
        assert_eq!(
            v,
            [0xFF00_0100, 0xFDFC_FDFE, 0x170D_0500, 0x3D37_2D22, 0x40]
        );
        let h: Vec<u32> = SCALER_FILTER[6..].iter().map(|r| r.1).collect();
        assert_eq!(
            h,
            [
                0xFF01_0100,
                0,
                0x0201_0000,
                0x0102_0302,
                0xF9FB_FE00,
                0xFBF7_F6F7,
                0x1C11_0700,
                0x3E39_3127,
                0x40
            ]
        );
        let offsets: Vec<usize> = SCALER_FILTER.iter().map(|r| r.0).collect();
        let want: Vec<usize> = (0x2194..=0x21A8)
            .step_by(4)
            .chain((0x21B4..=0x21D4).step_by(4))
            .collect();
        assert_eq!(offsets, want);
        // Every register the filter writes is snapshotted and put back.
        for (o, _) in SCALER_FILTER {
            assert!(TOUCHED.contains(&o), "{o:#x}");
        }
    }

    #[test]
    fn scales_only_when_a_size_differs() {
        let v = VideoPlane::new(VideoFormat::Uyvy, 240, 320, 0x40_0000);
        let s = |out_w, out_h| Scale {
            out_w,
            out_h,
            ..Scale::NONE
        };
        assert!(!v.scales(Scale::NONE));
        assert!(!v.scales(s(240, 320)));
        assert!(!v.scales(s(240, 0)));
        assert!(!v.scales(s(0, 320)));
        assert!(v.scales(s(241, 320)));
        assert!(v.scales(s(240, 319)));
        assert!(v.scales(s(480, 0)));
        assert!(v.scales(s(0, 160)));
        // A placed 1:1 plane doesn't scale.
        assert!(!v.scales(Scale {
            x: 100,
            y: 100,
            ..Scale::NONE
        }));
    }

    #[test]
    #[should_panic(expected = "unaligned")]
    fn rejects_unaligned_video() {
        let mut v = VideoPlane::new(VideoFormat::Uyvy, 480, 640, 0x40_0000);
        v.offset += 8;
        let _ = v.registers(Layer::SOLO, false);
    }

    #[test]
    fn solo_turns_off_video_and_cursor_without_needless_writes() {
        let mut bus = MockBus::new();
        bus.preset(crate::reg::GSCADR, 0x8000_0000); // WM's: already solo
        bus.preset(reg::VSCADR, 0x0100_0000); // WM's: stream off
        bus.preset(reg::HCCTRL, 0x0600_0000);
        solo(&mut bus);
        assert!(bus.writes().is_empty(), "no flicker on WM's state");

        let mut bus = MockBus::new();
        bus.preset(
            crate::reg::GSCADR,
            0x8000_0000 | (3 << 27) | (1 << 24) | 0x1_0000,
        );
        bus.preset(reg::VSCADR, 0x8000_0000 | 0x5_0000);
        bus.preset(reg::HCCTRL, 0x8600_0000);
        solo(&mut bus);
        assert_eq!(bus.value(crate::reg::GSCADR), 0x8001_0000);
        assert_eq!(bus.value(reg::VSCADR), 0x0005_0000);
        assert_eq!(bus.value(reg::HCCTRL), 0x0600_0000);
    }

    #[test]
    fn snapshot_restores_everything_video_off_first() {
        let mut bus = MockBus::new();
        for (i, &o) in TOUCHED.iter().enumerate() {
            bus.preset(o, 0x8000_0000 | i as u32);
        }
        let snap = Snapshot::take(&mut bus);
        assert_eq!(snap.get(reg::VOVRCLK), Some(0x8000_0001));
        let mut bus = MockBus::new();
        snap.restore(&mut bus);
        let w = bus.writes();
        assert_eq!(w.first(), Some(&(reg::VSCADR, 0)));
        assert_eq!(w.last(), Some(&(reg::VSCADR, 0x8000_0000)));
        for (i, &o) in TOUCHED.iter().enumerate() {
            assert_eq!(bus.value(o), 0x8000_0000 | i as u32);
        }
    }
}
