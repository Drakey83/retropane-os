//! The 2700G's full reset and rebuild (run Z, `boot/gpubench`, proven on
//! the X51v 2026-10-20: runs Z4 and Z5, the second bringing back a display
//! G4 had left dark): `SYSRST`, Linux's `SYSCFG`, the clocks, the local
//! memory in Linux mbxfb's `setup_memc` order, the system block, then the
//! display with its enable window (`DHDET`/`DVDET`) and the sync generator
//! last; every write 1 ms apart and nothing read until the memory runs.
//!
//! R344/R340 (2026-11-20, runs 276-278): a chip whose host-to-memory path
//! wedged takes no DMA or CPU writes into its own memory through any core
//! reset or clock change; only this (or a device reset) brings it back.
//! The local memory's contents are lost (the picture, textures, the TA's
//! memory): the caller sets up and uploads again.
//!
//! Rule 0: Linux runs this sequence only at probe and resume, Windows
//! Mobile's driver never (a wedged chip there needs the reset button), a
//! desktop driver resets the GPU and tells the program its memory is lost
//! (`GL_ARB_robustness`). Ours is the last, in ~0.3 s on a chip a game
//! still has open, from the register set measured on this device (run S),
//! so the program re-places its textures and plays on.

use rp_hal_api::RegisterBus;

use crate::reg::{DINTRS, DSCTRL};
use crate::sig::DSIG;

/// A working X51v's 2700G (run S, `local/rp-gpubench-runS1-healthy.txt`,
/// 2026-10-20, after a soft reset into Windows Mobile and the loader):
/// name, offset, value.
pub const HEALTHY: [(&str, usize, u32); 146] = [
    ("SYSCFG", 0x0000, 0x0),
    ("PFBASE", 0x0004, 0x0),
    ("PFCEIL", 0x0008, 0x3FDFFE0),
    ("POLLFLAG", 0x000C, 0x0),
    ("NINTPW", 0x0014, 0x1C),
    ("MINTENABLE", 0x0018, 0x800F),
    ("MINTSTAT", 0x001C, 0x0),
    ("SINTENABLE", 0x0020, 0x0),
    ("SINTSTAT", 0x0024, 0x10),
    ("SYSCLKSRC", 0x002C, 0x2),
    ("PIXCLKSRC", 0x0030, 0x1),
    ("CLKSLEEP", 0x0034, 0x0),
    ("COREPLL", 0x0038, 0xBB1),
    ("DISPPLL", 0x003C, 0xFC5),
    ("PLLSTAT", 0x0040, 0x7),
    ("VOVRCLK", 0x0044, 0x0),
    ("PIXCLK", 0x0048, 0x1),
    ("MEMCLK", 0x004C, 0x1),
    ("M24CLK", 0x0050, 0x2),
    ("MBXCLK", 0x0054, 0x5),
    ("SDCLK", 0x0058, 0x1),
    ("PIXCLKDIV", 0x005C, 0x1),
    ("LCD_CONFIG", 0x0060, 0x2EFA0000),
    ("ODFBPWR", 0x0064, 0x0),
    ("ODFBSTAT", 0x0068, 0x4),
    ("GPIOCGF", 0x006C, 0x1),
    ("GPIOHI", 0x0070, 0x0),
    ("GPIOLO", 0x0074, 0x0),
    ("GPIOSTAT", 0x0078, 0x0),
    ("PWMCFG", 0x0204, 0x3),
    ("PWM0DIV", 0x0210, 0x0),
    ("PWM0DUTY", 0x0214, 0x0),
    ("PWM0PER", 0x0218, 0x4),
    ("PWM1DIV", 0x0220, 0x0),
    ("PWM1DUTY", 0x0224, 0x0),
    ("PWM1PER", 0x0228, 0x4),
    ("ID", 0x0FF0, 0x1727189),
    ("LMCFG", 0x1004, 0x27),
    ("LMPWR", 0x1008, 0x0),
    ("LMPWRSTAT", 0x100C, 0x0),
    ("LMCEMR", 0x1010, 0x0),
    ("LMTYPE", 0x1014, 0xAC8),
    ("LMTIM", 0x1018, 0x73392),
    ("LMREFRESH", 0x101C, 0x615),
    ("LMPROTMIN", 0x1020, 0x0),
    ("LMPROTMAX", 0x1024, 0x3FDFFFC),
    ("LMPROTCFG", 0x1028, 0x0),
    ("LMPROTERR", 0x102C, 0x0),
    ("GSCTRL", 0x2000, 0x3C0EFA7F),
    ("VSCTRL", 0x2004, 0x600780F),
    ("GBBASE", 0x2020, 0x0),
    ("VBBASE", 0x2024, 0x0),
    ("GDRCTRL", 0x2040, 0xFFFFFF),
    ("VCMSK", 0x2044, 0xFFFFFF),
    ("GSCADR", 0x2060, 0x80000000),
    ("VSCADR", 0x2064, 0x1000000),
    ("VUBASE", 0x2084, 0x0),
    ("VVBASE", 0x20A4, 0x0),
    ("GSADR", 0x20C0, 0xEC00000),
    ("VSADR", 0x20C4, 0x0),
    ("HCCTRL", 0x2100, 0x0),
    ("HCSIZE", 0x2110, 0x40000000),
    ("HCPOS", 0x2120, 0x0),
    ("HCBADR", 0x2130, 0x0),
    ("HCCKMSK", 0x2140, 0xFFFFFF),
    ("GPLUT", 0x2150, 0x0),
    ("DSCTRL", 0x2154, 0x8000002A),
    ("DHT01", 0x2158, 0x400288),
    ("DHT02", 0x215C, 0x500050),
    ("DHT03", 0x2160, 0x2300230),
    ("DVT01", 0x2164, 0x20284),
    ("DVT02", 0x2168, 0x30003),
    ("DVT03", 0x216C, 0x2830283),
    ("DBCOL", 0x2170, 0x0),
    ("BGCOLOR", 0x2174, 0x0),
    ("DINTRS", 0x2178, 0x5),
    ("DINTRE", 0x217C, 0x0),
    ("DINTRCNT", 0x2180, 0x0),
    ("DSIG", 0x2184, 0xFE6D32F),
    ("DMCTRL", 0x2188, 0x10303010),
    ("CLIPCTRL", 0x218C, 0x0),
    ("SPOCTRL", 0x2190, 0xC8030000),
    ("SVCTRL", 0x2194, 0x0),
    ("VSCOEFF0", 0x2198, 0x0),
    ("VSCOEFF1", 0x219C, 0x0),
    ("VSCOEFF2", 0x21A0, 0x0),
    ("VSCOEFF3", 0x21A4, 0x0),
    ("VSCOEFF4", 0x21A8, 0x0),
    ("SHCTRL", 0x21B0, 0x20000000),
    ("HSCOEFF0", 0x21B4, 0x0),
    ("HSCOEFF1", 0x21B8, 0x0),
    ("HSCOEFF2", 0x21BC, 0x0),
    ("HSCOEFF3", 0x21C0, 0x0),
    ("HSCOEFF4", 0x21C4, 0x0),
    ("HSCOEFF5", 0x21C8, 0x0),
    ("HSCOEFF6", 0x21CC, 0x0),
    ("HSCOEFF7", 0x21D0, 0x0),
    ("HSCOEFF8", 0x21D4, 0x0),
    ("SSSIZE", 0x21D8, 0x0),
    ("VIDGAM0", 0x2200, 0x0),
    ("VIDGAM1", 0x2204, 0x101010),
    ("VIDGAM2", 0x2208, 0x202020),
    ("VIDGAM3", 0x220C, 0x303030),
    ("VIDGAM4", 0x2210, 0x404040),
    ("VIDGAM5", 0x2214, 0x505050),
    ("VIDGAM6", 0x2218, 0x606060),
    ("VIDGAM7", 0x221C, 0x6F6F6F),
    ("VIDGAM8", 0x2220, 0x808080),
    ("VIDGAM9", 0x2224, 0x909090),
    ("VIDGAM10", 0x2228, 0xA0A0A0),
    ("VIDGAM11", 0x222C, 0xB0B0B0),
    ("VIDGAM12", 0x2230, 0xC0C0C0),
    ("VIDGAM13", 0x2234, 0xCFCFCF),
    ("VIDGAM14", 0x2238, 0xDFDFDF),
    ("VIDGAM15", 0x223C, 0xF0F0F0),
    ("VIDGAM16", 0x2240, 0xFFFFFF),
    ("GFXGAM0", 0x2250, 0x0),
    ("GFXGAM1", 0x2254, 0xF0F0F),
    ("GFXGAM2", 0x2258, 0x1F1F1F),
    ("GFXGAM3", 0x225C, 0x2F2F2F),
    ("GFXGAM4", 0x2260, 0x3F3F3F),
    ("GFXGAM5", 0x2264, 0x4F4F4F),
    ("GFXGAM6", 0x2268, 0x5F5F5F),
    ("GFXGAM7", 0x226C, 0x6F6F6F),
    ("GFXGAM8", 0x2270, 0x7F7F7F),
    ("GFXGAM9", 0x2274, 0x8E8E8E),
    ("GFXGAM10", 0x2278, 0x9E9E9E),
    ("GFXGAM11", 0x227C, 0xAEAEAE),
    ("GFXGAM12", 0x2280, 0xBEBEBE),
    ("GFXGAM13", 0x2284, 0xCECECE),
    ("GFXGAM14", 0x2288, 0xDEDEDE),
    ("GFXGAM15", 0x228C, 0xEEEEEE),
    ("GFXGAM16", 0x2290, 0xFDFDFD),
    ("DLSTS", 0x2300, 0x800000),
    ("DLLCTRL", 0x2304, 0x0),
    ("DVLNUM", 0x2308, 0xE2),
    ("DUCTRL", 0x230C, 0x0),
    ("DVECTRL", 0x2310, 0x2830002),
    ("DHDET", 0x2314, 0x500230),
    ("DVDET", 0x2318, 0x30283),
    ("DODMSK", 0x231C, 0x0),
    ("CSC01", 0x2330, 0x12A),
    ("CSC02", 0x2334, 0x95199),
    ("CSC03", 0x2338, 0x39879C),
    ("CSC04", 0x233C, 0x10292A),
    ("CSC05", 0x2340, 0x0),
];

/// The reset bit's register (`SYSRST`).
pub const SYSRST: usize = 0x010;
/// The system block's first word (`SYSCFG`).
pub const SYSCFG: usize = 0x000;
/// Linux mbxfb's first write after the reset ("setup a timeout, raise
/// drive strength"): without a time-out an access the reset chip never
/// answers holds the bus for good (run Z's first two tries froze).
pub const SYSCFG_AFTER_RESET: u32 = 0xFFFF_FF0C;

/// The clock sources and PLLs, then the clocks (Z2).
const SOURCES: [&str; 5] = ["SYSCLKSRC", "PIXCLKSRC", "CLKSLEEP", "COREPLL", "DISPPLL"];
const CLOCKS: [&str; 7] = [
    "VOVRCLK",
    "PIXCLK",
    "MEMCLK",
    "M24CLK",
    "MBXCLK",
    "SDCLK",
    "PIXCLKDIV",
];
/// Linux mbxfb's `setup_memc` order (Z3: `LMPWR` twice).
const MEMORY: [&str; 6] = ["LMCFG", "LMPWR", "LMTIM", "LMREFRESH", "LMTYPE", "LMPWR"];
/// Written again and read back once the memory runs (Z3).
const CHECKED: [&str; 21] = [
    "SYSCLKSRC",
    "PIXCLKSRC",
    "CLKSLEEP",
    "COREPLL",
    "DISPPLL",
    "VOVRCLK",
    "PIXCLK",
    "MEMCLK",
    "M24CLK",
    "MBXCLK",
    "SDCLK",
    "PIXCLKDIV",
    "LMCFG",
    "LMPWR",
    "LMTIM",
    "LMREFRESH",
    "LMTYPE",
    "LMCEMR",
    "LMPROTMIN",
    "LMPROTMAX",
    "LMPROTCFG",
];
/// The system block (Z4; `SYSCFG` stays Linux's).
const SYSTEM: [&str; 17] = [
    "PFBASE",
    "PFCEIL",
    "NINTPW",
    "MINTENABLE",
    "SINTENABLE",
    "LCD_CONFIG",
    "ODFBPWR",
    "GPIOCGF",
    "GPIOHI",
    "GPIOLO",
    "PWMCFG",
    "PWM0DIV",
    "PWM0DUTY",
    "PWM0PER",
    "PWM1DIV",
    "PWM1DUTY",
    "PWM1PER",
];
/// The display's status and counters (Z5 writes every other display
/// register; `DSCTRL` last, off then on).
const DISPLAY_SKIP: [&str; 5] = ["DINTRS", "DSIG", "DVLNUM", "DLSTS", "DSCTRL"];

/// The healthy value of `name` (0 if it is not in [`HEALTHY`]).
#[must_use]
pub fn healthy(name: &str) -> u32 {
    HEALTHY.iter().find(|r| r.0 == name).map_or(0, |r| r.2)
}

/// What [`run`] found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rebuilt {
    /// `PLLSTAT`, the first read after the reset.
    pub pllstat: u32,
    /// `LMPWRSTAT` (0: the local memory active).
    pub lmpwrstat: u32,
    /// Writes read back differently (run Z4: 1, `DUCTRL`'s update flag).
    pub off_by: u32,
    /// `DINTRS` at the end.
    pub dintrs: u32,
    /// `DSIG` twice 20 ms apart at the end: the display sends when either
    /// is not 0.
    pub dsig: (u32, u32),
}

impl Rebuilt {
    /// The display sends again.
    #[must_use]
    pub const fn sending(&self) -> bool {
        self.dsig.0 | self.dsig.1 != 0
    }
}

fn write_healthy<B: RegisterBus>(bus: &mut B, name: &str, delay_us: &mut impl FnMut(u32)) {
    if let Some(&(_, off, v)) = HEALTHY.iter().find(|r| r.0 == name) {
        bus.write32(off, v);
    }
    delay_us(1_000);
}

/// Each of `names` written in the table's order, 1 ms after each, and read
/// back: how many read differently.
fn put_healthy<B: RegisterBus>(bus: &mut B, names: &[&str], delay_us: &mut impl FnMut(u32)) -> u32 {
    let mut off_by = 0;
    for &(name, off, v) in &HEALTHY {
        if names.contains(&name) {
            bus.write32(off, v);
            delay_us(1_000);
            off_by += u32::from(bus.read32(off) != v);
        }
    }
    off_by
}

/// Run Z's reset and rebuild (Z1-Z6 without the test image's backlight
/// blinks and logs): ~0.3 s. The local memory's contents are lost.
pub fn run<B: RegisterBus>(bus: &mut B, mut delay_us: impl FnMut(u32)) -> Rebuilt {
    // Z1: the reset, then Linux's time-out at once (writes only until Z3)
    bus.write32(SYSRST, 1);
    delay_us(10_000);
    bus.write32(SYSCFG, SYSCFG_AFTER_RESET);
    delay_us(1_000);
    // Z2: the clocks
    for name in SOURCES {
        write_healthy(bus, name, &mut delay_us);
    }
    delay_us(10_000);
    for name in CLOCKS {
        write_healthy(bus, name, &mut delay_us);
    }
    // Z3: the local memory, Linux's order
    for name in MEMORY {
        write_healthy(bus, name, &mut delay_us);
    }
    let mut r = Rebuilt {
        pllstat: bus.read32(0x040),
        lmpwrstat: bus.read32(0x100C),
        ..Rebuilt::default()
    };
    r.off_by += put_healthy(bus, &CHECKED, &mut delay_us);
    // Z4: the system block
    r.off_by += put_healthy(bus, &SYSTEM, &mut delay_us);
    // Z5: the display (DHDET/DVDET among them: the enable window), the
    // sync generator last
    let mut display = [""; HEALTHY.len()];
    let mut nd = 0;
    for &(name, off, _) in &HEALTHY {
        if (0x2000..0x3000).contains(&off) && !DISPLAY_SKIP.contains(&name) {
            display[nd] = name;
            nd += 1;
        }
    }
    r.off_by += put_healthy(bus, &display[..nd], &mut delay_us);
    let ds = healthy("DSCTRL");
    bus.write32(DSCTRL, ds & !(1 << 31));
    delay_us(1_000);
    bus.write32(DSCTRL, ds);
    delay_us(40_000);
    // Z6: is it sending
    r.dintrs = bus.read32(DINTRS);
    let s0 = bus.read32(DSIG);
    delay_us(20_000);
    r.dsig = (s0, bus.read32(DSIG));
    r
}

#[cfg(test)]
mod tests {
    extern crate std;

    use std::vec::Vec;

    use rp_hal_mock::{Access, MockBus};

    use super::*;

    fn offset(name: &str) -> usize {
        HEALTHY.iter().find(|r| r.0 == name).map(|r| r.1).unwrap()
    }

    fn at(a: &Access) -> usize {
        match *a {
            Access::Write { offset, .. } | Access::Read { offset, .. } => offset,
        }
    }

    #[test]
    fn the_reset_then_linuxs_timeout_then_writes_only_until_the_memory_runs() {
        let mut bus = MockBus::new();
        let mut waited = 0u32;
        let _ = run(&mut bus, |us| waited += us);
        let log = bus.log();
        assert_eq!(
            log[0],
            Access::Write {
                offset: SYSRST,
                value: 1
            }
        );
        assert_eq!(
            log[1],
            Access::Write {
                offset: SYSCFG,
                value: SYSCFG_AFTER_RESET
            }
        );
        // the first read after the 5 sources, 7 clocks and 6 memory writes
        let first_read = log
            .iter()
            .position(|a| matches!(a, Access::Read { .. }))
            .unwrap();
        assert_eq!(first_read, 2 + 5 + 7 + 6);
        let memory: Vec<usize> = log[14..20].iter().map(at).collect();
        let want: Vec<usize> = MEMORY.iter().map(|n| offset(n)).collect();
        assert_eq!(memory, want);
        // well under a second: a stall costs about a second at most
        assert!((200_000..1_000_000).contains(&waited), "{waited}");
    }

    #[test]
    fn the_display_gets_its_enable_window_and_the_sync_generator_last() {
        let mut bus = MockBus::new();
        let _ = run(&mut bus, |_| {});
        let w = bus.writes();
        assert!(w.contains(&(offset("DHDET"), 0x50_0230)));
        assert!(w.contains(&(offset("DVDET"), 0x3_0283)));
        let n = w.len();
        let ds = healthy("DSCTRL");
        assert_eq!(w[n - 2], (DSCTRL, ds & !(1 << 31)));
        assert_eq!(w[n - 1], (DSCTRL, ds));
        // the status and counters never written
        for name in ["DINTRS", "DSIG", "DVLNUM", "DLSTS"] {
            let off = offset(name);
            assert!(w.iter().all(|&(o, _)| o != off), "{name} written");
        }
        // SYSCFG stays Linux's: never the table's 0 again
        assert_eq!(w.iter().filter(|&&(o, _)| o == SYSCFG).count(), 1);
    }

    #[test]
    fn a_sending_display_is_told_from_a_dark_one() {
        let mut bus = MockBus::new();
        bus.script_reads(DSIG, &[0, 0]);
        assert!(!run(&mut bus, |_| {}).sending());
        let mut bus = MockBus::new();
        bus.script_reads(DSIG, &[0, 0x1_0283]);
        assert!(run(&mut bus, |_| {}).sending());
    }
}
