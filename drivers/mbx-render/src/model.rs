//! A model of the 2D engine, enough for [`crate::Mbx2dRenderer`]: 32 bpp
//! fills, copies (any signed stride, 0 included) and blends with the blend
//! word `0x09100000`, using the arithmetic fitted to the X51v's results
//! (the fork's model, rp-v0.13.0: alpha weighed `a + (a >> 7)` of 256,
//! `(s × A + d × (256 − A)) >> 8`). Anything else panics, so a test sees
//! the renderer ask for something the model (and the device runs so far)
//! doesn't cover.

extern crate std;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::vec::Vec;

use rp_drv_blit2d::{MBX, PORT, PORT_SIZE, reg};
use rp_hal_api::RegisterBus;

use crate::LocalMem;

/// 2700G local memory, shared by the model and the renderer.
pub type Shared = Rc<RefCell<Vec<u8>>>;

/// 16 MiB of zeroed local memory.
#[must_use]
pub fn memory() -> Shared {
    Rc::new(RefCell::new(std::vec![0; 16 << 20]))
}

/// The renderer's view of [`Shared`] memory.
pub struct Mem(pub Shared);

impl LocalMem for Mem {
    fn store(&mut self, at: usize, words: &[u32]) {
        let mut m = self.0.borrow_mut();
        for (k, w) in words.iter().enumerate() {
            m[at + 4 * k..at + 4 * k + 4].copy_from_slice(&w.to_le_bytes());
        }
    }

    fn load(&self, at: usize) -> Option<u32> {
        let m = self.0.borrow();
        Some(u32::from_le_bytes(m.get(at..at + 4)?.try_into().ok()?))
    }

    fn sync(&mut self) {}
}

#[derive(Default)]
enum Want {
    #[default]
    Command,
    DestAddr,
    SrcAddr,
    Blend,
    Key(u8),
    Colour,
    Xy0,
    Xy1,
}

/// The register window: the command window runs the model, other
/// registers just hold what was written.
pub struct Bus {
    mem: Shared,
    regs: BTreeMap<usize, u32>,
    want: Want,
    dest: (u32, u32),
    src: (u32, u32),
    origin: u32,
    blend: u32,
    op: u32,
    colour: u32,
    xy0: u32,
    /// Commands run (fills, copies, blends; fences included).
    pub ops: u32,
}

impl Bus {
    /// A window over `mem`.
    #[must_use]
    pub fn new(mem: Shared) -> Self {
        Self {
            mem,
            regs: BTreeMap::new(),
            want: Want::Command,
            dest: (0, 0),
            src: (0, 0),
            origin: 0,
            blend: 0,
            op: 0,
            colour: 0,
            xy0: 0,
            ops: 0,
        }
    }

    /// A register's last written value.
    #[must_use]
    pub fn reg(&self, off: usize) -> u32 {
        self.regs.get(&off).copied().unwrap_or(0)
    }

    fn command(&mut self, w: u32) {
        self.want = match core::mem::take(&mut self.want) {
            Want::Command => match w >> 28 {
                0x2 if w == 0x2000_0004 => Want::Blend,
                0x2 if w == 0x2000_0001 => Want::Key(2),
                0x3 => {
                    self.origin = w;
                    Want::Command
                }
                0x6 | 0x7 => Want::Command,
                0x8 => {
                    self.op = w;
                    if w & (1 << 16) != 0 {
                        Want::Xy0
                    } else {
                        Want::Colour
                    }
                }
                0x9 => {
                    self.src.0 = w;
                    Want::SrcAddr
                }
                0xA => {
                    self.dest.0 = w;
                    Want::DestAddr
                }
                _ => panic!("model: command 0x{w:08x}"),
            },
            Want::DestAddr => {
                self.dest.1 = w & 0x03FF_FFFC;
                Want::Command
            }
            Want::SrcAddr => {
                self.src.1 = w & 0x03FF_FFFC;
                Want::Command
            }
            Want::Blend => {
                self.blend = w;
                Want::Command
            }
            Want::Key(n) => {
                assert!(n != 2 || w == 0, "model: colour keys");
                if n > 1 {
                    Want::Key(n - 1)
                } else {
                    Want::Command
                }
            }
            Want::Colour => {
                self.colour = w;
                Want::Xy0
            }
            Want::Xy0 => {
                self.xy0 = w;
                Want::Xy1
            }
            Want::Xy1 => {
                self.run(w);
                Want::Command
            }
        };
    }

    fn run(&mut self, xy1: u32) {
        self.ops += 1;
        let stride = |s: u32| -> i64 {
            let v = i64::from(s & 0x7FFF);
            if v & 0x4000 != 0 { v - 0x8000 } else { v }
        };
        let fmt = |s: u32| (s >> 15) & 0x1F;
        assert_eq!(fmt(self.dest.0), 0xC, "model: 32 bpp destinations only");
        let (x0, y0) = (
            i64::from((self.xy0 >> 16) & 0x1FFF),
            i64::from(self.xy0 & 0x1FFF),
        );
        let (x1, y1) = (i64::from((xy1 >> 16) & 0x1FFF), i64::from(xy1 & 0x1FFF));
        let (ds, da) = (stride(self.dest.0), i64::from(self.dest.1));
        let rop = self.op & 0xFFFF;
        let blend = self.op & (1 << 17) != 0;
        let mut m = self.mem.borrow_mut();
        let rd = |m: &Vec<u8>, at: i64| -> u32 {
            let at = usize::try_from(at).expect("model: address below 0");
            u32::from_le_bytes(m[at..at + 4].try_into().expect("4 bytes"))
        };
        let (sx, sy) = (
            i64::from((self.origin >> 14) & 0x1FFF),
            i64::from(self.origin & 0x3FFF),
        );
        for y in y0..y1 {
            for x in x0..x1 {
                let d_at = da + y * ds + x * 4;
                let v = match (rop, blend) {
                    (0xF0F0, false) => self.colour,
                    (0xCCCC, b) => {
                        assert_eq!(fmt(self.src.0), 0xC, "model: 32 bpp sources only");
                        let s_at = i64::from(self.src.1)
                            + (sy + y - y0) * stride(self.src.0)
                            + (sx + x - x0) * 4;
                        let s = rd(&m, s_at);
                        if b {
                            assert_eq!(self.blend, 0x0910_0000, "model: 'over' only");
                            over(s, rd(&m, d_at))
                        } else {
                            s
                        }
                    }
                    _ => panic!("model: operation 0x{:08x}", self.op),
                };
                let at = usize::try_from(d_at).expect("model: address below 0");
                m[at..at + 4].copy_from_slice(&v.to_le_bytes());
            }
        }
    }
}

/// The engine's "over" (blend word `0x09100000`) on 8-bit channels.
#[must_use]
pub fn over(s: u32, d: u32) -> u32 {
    let a = s >> 24;
    let k = a + (a >> 7);
    let ch = |sh: u32| (((s >> sh) & 0xFF) * k + ((d >> sh) & 0xFF) * (256 - k)) >> 8;
    0xFF00_0000 | ch(16) << 16 | ch(8) << 8 | ch(0)
}

impl RegisterBus for Bus {
    fn read32(&mut self, offset: usize) -> u32 {
        if offset == MBX + reg::STATUS {
            return 0;
        }
        self.reg(offset)
    }

    fn write32(&mut self, offset: usize, value: u32) {
        if (PORT..PORT + PORT_SIZE).contains(&offset) {
            self.command(value);
        } else {
            self.regs.insert(offset, value);
        }
    }
}
