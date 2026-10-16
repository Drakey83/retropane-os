//! Report-only test image (docs/research/storage-speed.md): is the CompactFlash
//! socket running at what the card can do, or at Windows Mobile's values?
//! Nothing is written to the card's media; the socket's timing registers
//! are only ever made *slower* than Windows Mobile's (for calibration), and
//! put back.
//!
//! - **R1 registers:** the clocks and the socket timings as found (the
//!   loader sets Windows Mobile's), decoded into SET / ASST / HOLD.
//! - **R2 card:** the slot powered and the card reset as the loader does,
//!   then the raw CIS (attribute memory, 256 even bytes) and its tuples.
//! - **R3 bus cycle:** 1,024 single accesses timed: attribute bytes as set,
//!   then with SET, ASST and HOLD each made longer, which gives each
//!   field's cost per unit in ns (so the decode is measured, not assumed);
//!   then common memory (memory mode) bytes and half-words, and common
//!   memory with ASST made longer.
//! - **R4 IDENTIFY:** all 256 words, and the ones that rate the card
//!   (PIO modes, cycle times, multi-sector, DMA, CF advanced timing).
//! - **R5 read paths:** 1,024 sectors from LBA 0 each way, timed with the
//!   wait for the card (DRQ) apart from the data's time on the bus, every
//!   sector's checksum compared with path A's: A today's driver (READ
//!   SECTORS, 16-bit data register), B 32-bit loads from the data window
//!   (0x400-0x7FF), C READ MULTIPLE (the card's largest block, up to 32)
//!   with B's loads, D a memory-to-memory DMA from the data window after
//!   each DRQ, E D with READ MULTIPLE blocks. READY (GPIO84) is sampled
//!   while path A waits (could it be the card's busy line, an interrupt?).
//!
//! The loader power-cycles and resets the card again when this returns.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed with the verdict.
pub const MARKER: &str = "RETROPANE-CFTIME";

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_cpld::{Cpld, Delay};
    use rp_drv_pxa_gpio::Gpio;
    use rp_hal_api::{DmaEngine, RegisterBus, RegisterBus8, RegisterBus16};
    use rp_hal_pxa27x::{Mmio, diag, rt};
    use rp_hal_x51v::devices::{self, RunLog};

    use super::MARKER;

    rp_hal_pxa27x::entry!(main);

    macro_rules! log {
        ($($t:tt)*) => {{ let _ = writeln!(RunLog, $($t)*); }};
    }

    /// Card detect (low = in) and slot power / READY (docs/hw/cf.md).
    const CF_DETECT: u8 = 13;
    const CF_READY: u8 = 84;
    /// Memory controller: socket 1's timing registers (docs/hw/cf.md).
    const MCMEM1: usize = 0x4800_002C;
    const MCATT1: usize = 0x4800_0034;
    /// Socket 1's common memory, physical (the DMA controller's view).
    const COMMON_PHYS: u32 = 0x3C00_0000;
    /// Memory mode: 0x400-0x7FF all read the data register (CF
    /// specification, memory-mapped addressing).
    const DATA_WINDOW: usize = 0x400;

    const DATA: usize = 0x0;
    const ERROR_FEATURES: usize = 0x1;
    const COUNT: usize = 0x2;
    const LBA0: usize = 0x3;
    const LBA1: usize = 0x4;
    const LBA2: usize = 0x5;
    const HEAD: usize = 0x6;
    const STATUS_COMMAND: usize = 0x7;
    const ALT_STATUS: usize = 0xE;
    const BSY: u8 = 0x80;
    const DRDY: u8 = 0x40;
    const DF: u8 = 0x20;
    const DRQ: u8 = 0x08;
    const ERR: u8 = 0x01;

    const READ_SECTORS: u8 = 0x20;
    const READ_MULTIPLE: u8 = 0xC4;
    const SET_MULTIPLE: u8 = 0xC6;
    const IDENTIFY: u8 = 0xEC;

    /// Sectors each path reads (512 KiB), 256 a command.
    const SECTORS: usize = 1024;
    /// Accesses per bus-cycle measurement.
    const ACCESSES: u32 = 1024;
    /// Status polls before a wait counts as failed.
    const SPIN: u32 = 2_000_000;

    struct Wait;
    impl Delay for Wait {
        fn delay_us(&mut self, us: u32) {
            rt::delay_us(us);
        }
    }

    struct Lines<'a> {
        gpio: &'a mut Gpio<Mmio>,
        cpld: &'a mut Cpld<Mmio>,
    }

    impl rp_drv_cf::CfLines for Lines<'_> {
        fn set_reset(&mut self, held: bool) {
            self.cpld.set_cf_reset(held);
        }
        fn ready(&mut self) -> bool {
            self.gpio.read(CF_READY)
        }
    }

    /// OS timer ticks (3.25 MHz) to ns.
    fn ns(ticks: u32) -> u64 {
        u64::from(ticks) * 4000 / 13
    }

    /// KiB/s for `bytes` in `ticks`.
    fn kib_s(bytes: usize, ticks: u32) -> u64 {
        (bytes as u64 * u64::from(rt::TICK_HZ)) / (u64::from(ticks.max(1)) * 1024)
    }

    fn fields(v: u32) -> (u32, u32, u32) {
        (v & 0x7F, (v >> 7) & 0x1F, (v >> 14) & 0x3F)
    }

    fn mc_read(addr: usize) -> u32 {
        rt::try_read(addr).unwrap_or(0xDEAD_DEAD)
    }

    fn mc_write(addr: usize, v: u32) {
        if !rt::try_write(addr, v) {
            log!("write to {addr:#x} failed");
        }
    }

    /// ns per access, `ACCESSES` calls of `f`.
    fn per_access(mut f: impl FnMut()) -> u64 {
        let t0 = rt::ticks();
        for _ in 0..ACCESSES {
            f();
        }
        ns(rt::ticks().wrapping_sub(t0)) / u64::from(ACCESSES)
    }

    fn checksum(words: impl Iterator<Item = u32>) -> u32 {
        words.fold(0x811C_9DC5, |h, w| (h ^ w).wrapping_mul(0x0100_0193))
    }

    /// The card in memory mode.
    struct Card {
        common: Mmio,
        gpio: Gpio<Mmio>,
    }

    /// Why a path stopped.
    enum Stop {
        Timeout(&'static str),
        Device(u8, u8),
        Dma,
    }

    impl core::fmt::Debug for Stop {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            match self {
                Self::Timeout(what) => write!(f, "timeout ({what})"),
                Self::Device(s, e) => write!(f, "card error: status {s:#04x} error {e:#04x}"),
                Self::Dma => write!(f, "DMA refused or failed"),
            }
        }
    }

    impl Card {
        fn status(&mut self) -> u8 {
            self.common.read8(STATUS_COMMAND)
        }

        fn idle(&mut self) -> Result<u8, Stop> {
            for _ in 0..SPIN {
                let s = self.status();
                if s & BSY == 0 {
                    return Ok(s);
                }
            }
            Err(Stop::Timeout("idle"))
        }

        fn command(&mut self, cmd: u8, lba: u32, count: u8) -> Result<(), Stop> {
            self.idle()?;
            self.common.write8(COUNT, count);
            self.common.write8(LBA0, lba as u8);
            self.common.write8(LBA1, (lba >> 8) as u8);
            self.common.write8(LBA2, (lba >> 16) as u8);
            self.common.write8(HEAD, 0xE0 | ((lba >> 24) as u8 & 0x0F));
            self.common.write8(STATUS_COMMAND, cmd);
            Ok(())
        }

        /// Waits for DRQ; also counts READY (GPIO84) low among `samples`.
        fn drq(&mut self, ready: Option<&mut (u32, u32)>) -> Result<(), Stop> {
            let mut ready = ready;
            for _ in 0..SPIN {
                if let Some(r) = ready.as_deref_mut() {
                    r.0 += 1;
                    if !self.gpio.read(CF_READY) {
                        r.1 += 1;
                    }
                }
                let s = self.status();
                if s & BSY != 0 {
                    continue;
                }
                if s & (ERR | DF) != 0 {
                    return Err(Stop::Device(s, self.common.read8(ERROR_FEATURES)));
                }
                if s & DRQ != 0 {
                    return Ok(());
                }
            }
            Err(Stop::Timeout("drq"))
        }

        fn finish(&mut self) -> Result<(), Stop> {
            let s = self.idle()?;
            if s & (ERR | DF) != 0 {
                return Err(Stop::Device(s, self.common.read8(ERROR_FEATURES)));
            }
            Ok(())
        }
    }

    /// How a path moves a block's data off the card.
    #[derive(Clone, Copy, PartialEq)]
    enum Data {
        /// 16-bit reads of the data register (today's driver).
        Port16,
        /// 32-bit loads across the data window.
        Window32,
        /// Memory-to-memory DMA from the data window.
        Dma,
    }

    /// One path's totals.
    #[derive(Default)]
    struct Totals {
        all: u32,
        wait: u32,
        data: u32,
        mismatches: u32,
    }

    /// Reads `SECTORS` sectors from LBA 0 with `cmd` (blocks of `block`
    /// sectors per DRQ), checksums each sector into `sums` (or compares).
    fn path(
        card: &mut Card,
        dma: &mut Option<rp_hal_pxa27x::dma::Dma>,
        buf: &mut [u8],
        cmd: u8,
        block: usize,
        data: Data,
        sums: &mut [u32; SECTORS],
        record: bool,
        ready: Option<&mut (u32, u32)>,
    ) -> Result<Totals, Stop> {
        let mut t = Totals::default();
        let mut ready = ready;
        let start = rt::ticks();
        let mut sector = 0usize;
        while sector < SECTORS {
            rt::check_in();
            card.command(cmd, sector as u32, 0)?; // 0 = 256 sectors
            let end = sector + 256;
            while sector < end {
                let t0 = rt::ticks();
                card.drq(ready.as_deref_mut())?;
                let t1 = rt::ticks();
                let bytes = block * 512;
                let out = &mut buf[..bytes];
                match data {
                    Data::Port16 => {
                        for pair in out.as_chunks_mut::<2>().0 {
                            *pair = card.common.read16(DATA).to_le_bytes();
                        }
                    }
                    Data::Window32 => {
                        for s in out.as_chunks_mut::<512>().0 {
                            for (k, w) in s.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                                *w = card.common.read32(DATA_WINDOW + 4 * k).to_le_bytes();
                            }
                        }
                    }
                    Data::Dma => {
                        let Some(d) = dma.as_mut() else {
                            return Err(Stop::Dma);
                        };
                        let src = COMMON_PHYS + DATA_WINDOW as u32;
                        if !d.arm_copy_from_device_window(src, 512, out) {
                            return Err(Stop::Dma);
                        }
                        let mut n = 0u32;
                        let ok = loop {
                            if let Some(ok) = d.done() {
                                break ok;
                            }
                            n += 1;
                            if n > SPIN {
                                d.abort();
                                break false;
                            }
                        };
                        if !ok {
                            return Err(Stop::Dma);
                        }
                    }
                }
                let t2 = rt::ticks();
                t.wait = t.wait.wrapping_add(t1.wrapping_sub(t0));
                t.data = t.data.wrapping_add(t2.wrapping_sub(t1));
                for s in buf[..bytes].as_chunks::<512>().0 {
                    let sum = checksum(s.as_chunks::<4>().0.iter().map(|w| u32::from_le_bytes(*w)));
                    if record {
                        sums[sector] = sum;
                    } else if sums[sector] != sum {
                        t.mismatches += 1;
                    }
                    sector += 1;
                }
            }
            card.finish()?;
        }
        t.all = rt::ticks().wrapping_sub(start);
        Ok(t)
    }

    fn report(name: &str, r: Result<Totals, Stop>) {
        match r {
            Ok(t) => {
                let bytes = SECTORS * 512;
                log!(
                    "{name}: {} KiB/s overall ({} us), wait for DRQ {} us, data {} us ({} ns/sector, {} KiB/s on the bus), mismatches {}",
                    kib_s(bytes, t.all),
                    ns(t.all) / 1000,
                    ns(t.wait) / 1000,
                    ns(t.data) / 1000,
                    ns(t.data) / SECTORS as u64,
                    kib_s(bytes, t.data),
                    t.mismatches
                );
            }
            Err(e) => log!("{name}: stopped: {e:?}"),
        }
    }

    fn main() -> ! {
        // R1: before anything changes them.
        let mut soc = [0u32; diag::COUNT];
        diag::snapshot(&mut soc);
        rt::set_fault_handler(on_fault);
        rt::init_mmu();
        log!("{MARKER} 1: CF socket timing, report only (docs/research/storage-speed.md)");
        let mut i = 0;
        for (_, regs) in diag::GROUPS {
            for r in *regs {
                if matches!(
                    r.name,
                    "CCCR"
                        | "CCSR"
                        | "MDREFR"
                        | "MDCNFG"
                        | "MECR"
                        | "MCMEM0"
                        | "MCMEM1"
                        | "MCATT0"
                        | "MCATT1"
                        | "MCIO0"
                        | "MCIO1"
                ) {
                    let v = soc[i];
                    if r.name.starts_with("MC") {
                        let (set, asst, hold) = fields(v);
                        log!(
                            "R1 {} = {v:#010x}: SET {set} ASST {asst} HOLD {hold}",
                            r.name
                        );
                    } else {
                        log!("R1 {} = {v:#010x}", r.name);
                    }
                }
                i += 1;
            }
        }
        run();
        log!("{MARKER}: done");
        devices::return_to_loader();
        loop {
            core::hint::spin_loop();
        }
    }

    fn run() {
        let Some(mut gpio) = rt::take_gpio().map(Gpio::new) else {
            log!("no GPIO");
            return;
        };
        if gpio.read(CF_DETECT) {
            log!("no CF card (GPIO13 high): nothing to measure");
            return;
        }
        let Some(mut cpld) = devices::take_cpld().map(Cpld::new) else {
            log!("no CPLD");
            return;
        };
        // The CPLD doesn't read back: keep what the loader keeps on.
        cpld.set_backlight_enable(true);
        cpld.set_sd_power(true);
        let Some((mut attr, common)) = devices::take_cf() else {
            log!("no CF windows");
            return;
        };
        log!(
            "R1 as set by take_cf: MCMEM1 {:#010x} MCATT1 {:#010x}",
            mc_read(MCMEM1),
            mc_read(MCATT1)
        );

        // R2: power and reset as the loader does.
        cpld.set_cf_power(false, &mut Wait);
        rt::delay_us(308_000);
        cpld.set_cf_power(true, &mut Wait);
        rt::delay_us(100_000);
        rt::check_in();
        let seen = rp_drv_cf::hard_reset(
            &mut Lines {
                gpio: &mut gpio,
                cpld: &mut cpld,
            },
            &mut |ms| rt::delay_us(ms * 1000),
        );
        log!("R2 reset: {seen:?}");
        if seen.is_err() {
            return;
        }
        let mut cis = [0u8; 256];
        for (k, b) in cis.iter_mut().enumerate() {
            *b = attr.read8(2 * k);
        }
        for (k, row) in cis.chunks(32).enumerate() {
            let mut line = Line::new();
            for b in row {
                let _ = write!(line, "{b:02x}");
            }
            log!("R2 cis {:03x}: {}", k * 64, line.as_str());
        }
        let mut config = None;
        let mut at = 0usize;
        while at + 1 < cis.len() {
            let code = cis[at];
            if code == 0xFF {
                break;
            }
            if code == 0 {
                at += 1;
                continue;
            }
            let link = usize::from(cis[at + 1]);
            let body = &cis[(at + 2).min(cis.len())..(at + 2 + link).min(cis.len())];
            let mut line = Line::new();
            for b in body {
                let _ = write!(line, " {b:02x}");
            }
            log!("R2 tuple {code:02x} len {link}:{}", line.as_str());
            if code == 0x1A && body.len() >= 3 {
                let rasz = usize::from(body[0] & 3) + 1;
                if body.len() >= 2 + rasz {
                    let mut base = 0u32;
                    for k in 0..rasz {
                        base |= u32::from(body[2 + k]) << (8 * k);
                    }
                    config = Some(base as usize);
                }
            }
            at += 2 + link;
        }
        let Some(config) = config.filter(|&c| c < 0x1000 && c % 2 == 0) else {
            log!("R2 no CISTPL_CONFIG: not measured further");
            return;
        };

        // R3: attribute memory, as set and each field made longer.
        let att = mc_read(MCATT1);
        let (set, asst, hold) = fields(att);
        let first = cis[0];
        let mut bad = 0u32;
        let att_read = |attr: &mut Mmio, bad: &mut u32| {
            per_access(|| {
                if attr.read8(0) != first {
                    *bad += 1;
                }
            })
        };
        let base_ns = att_read(&mut attr, &mut bad);
        log!("R3 attr read8 as set (SET {set} ASST {asst} HOLD {hold}): {base_ns} ns");
        let longer = [
            ("SET+32", (att & !0x7F) | (set + 32).min(0x7F)),
            ("ASST+3", (att & !(0x1F << 7)) | (asst + 3).min(0x1F) << 7),
            (
                "HOLD+16",
                (att & !(0x3F << 14)) | (hold + 16).min(0x3F) << 14,
            ),
        ];
        for (name, v) in longer {
            mc_write(MCATT1, v);
            let t = att_read(&mut attr, &mut bad);
            mc_write(MCATT1, att);
            log!(
                "R3 attr read8 {name} ({v:#010x}): {t} ns (+{} ns)",
                t.saturating_sub(base_ns)
            );
        }
        log!("R3 attr reads that differed from the CIS's first byte: {bad}");

        // Memory mode (Configuration Option Register index 0), interrupts off.
        attr.write8(config, 0x80);
        rt::delay_us(1000);
        attr.write8(config, 0x00);
        let mut card = Card { common, gpio };
        card.common.write8(ALT_STATUS, 0x02);
        let mut ok = false;
        for _ in 0..3000 {
            let s = card.status();
            if s & BSY == 0 && s & DRDY != 0 {
                ok = true;
                break;
            }
            rt::delay_us(1000);
        }
        if !ok {
            log!("R3 card never ready in memory mode");
            return;
        }
        let mem = mc_read(MCMEM1);
        let (mset, masst, mhold) = fields(mem);
        let m8 = per_access(|| {
            let _ = card.common.read8(ALT_STATUS);
        });
        let m16 = per_access(|| {
            let _ = card.common.read16(ALT_STATUS);
        });
        log!(
            "R3 common read8 as set (SET {mset} ASST {masst} HOLD {mhold}): {m8} ns; read16 {m16} ns"
        );
        mc_write(MCMEM1, (mem & !(0x1F << 7)) | (masst + 8).min(0x1F) << 7);
        let m8l = per_access(|| {
            let _ = card.common.read8(ALT_STATUS);
        });
        mc_write(MCMEM1, mem);
        log!(
            "R3 common read8 ASST+8: {m8l} ns (+{} ns)",
            m8l.saturating_sub(m8)
        );
        log!(
            "R3 MCMEM1 {:#010x} MCATT1 {:#010x} (restored)",
            mc_read(MCMEM1),
            mc_read(MCATT1)
        );

        // R4: IDENTIFY.
        let mut id = [0u16; 256];
        let r = card
            .command(IDENTIFY, 0, 0)
            .and_then(|()| card.drq(None))
            .map(|()| {
                for w in &mut id {
                    *w = card.common.read16(DATA);
                }
            })
            .and_then(|()| card.finish());
        if let Err(e) = r {
            log!("R4 IDENTIFY failed: {e:?}");
            return;
        }
        for (k, row) in id.chunks(16).enumerate() {
            let mut line = Line::new();
            for w in row {
                let _ = write!(line, " {w:04x}");
            }
            log!("R4 id {:03}:{}", k * 16, line.as_str());
        }
        let mut model = [b' '; 40];
        for (k, w) in id[27..47].iter().enumerate() {
            let [a, b] = w.to_be_bytes();
            model[2 * k] = if a.is_ascii_graphic() { a } else { b' ' };
            model[2 * k + 1] = if b.is_ascii_graphic() { b } else { b' ' };
        }
        log!(
            "R4 model '{}' sectors {}",
            core::str::from_utf8(&model).unwrap_or("?").trim_end(),
            u32::from(id[60]) | u32::from(id[61]) << 16
        );
        log!(
            "R4 w0 {:#06x} w47 {:#06x} w49 {:#06x} w51 {:#06x} w53 {:#06x} w59 {:#06x} w63 {:#06x} w64 {:#06x}",
            id[0],
            id[47],
            id[49],
            id[51],
            id[53],
            id[59],
            id[63],
            id[64]
        );
        log!(
            "R4 w65 {} w66 {} w67 {} w68 {} (ns) w80 {:#06x} w82 {:#06x} w83 {:#06x} w88 {:#06x}",
            id[65],
            id[66],
            id[67],
            id[68],
            id[80],
            id[82],
            id[83],
            id[88]
        );
        log!(
            "R4 CF w160 {:#06x} w162 {:#06x} w163 {:#06x} w164 {:#06x}",
            id[160],
            id[162],
            id[163],
            id[164]
        );

        // R5: read paths. Buffers in the scratch area (whole cache lines,
        // identity-mapped: the DMA controller's view).
        let Some(scratch) = devices::take_scratch() else {
            log!("R5 no scratch");
            return;
        };
        let buf = &mut scratch[..32 * 512];
        let mut sums = [0u32; SECTORS];
        let mut dma = rt::take_dma(1);
        let mut ready = (0u32, 0u32);
        let a = path(
            &mut card,
            &mut dma,
            buf,
            READ_SECTORS,
            1,
            Data::Port16,
            &mut sums,
            true,
            Some(&mut ready),
        );
        let a_ok = a.is_ok();
        report("R5 A READ SECTORS, 16-bit port (today)", a);
        log!(
            "R5 A READY (GPIO84) low in {} of {} polls while waiting",
            ready.1,
            ready.0
        );
        if !a_ok {
            return;
        }
        let b = path(
            &mut card,
            &mut dma,
            buf,
            READ_SECTORS,
            1,
            Data::Window32,
            &mut sums,
            false,
            None,
        );
        report("R5 B READ SECTORS, 32-bit loads from the data window", b);
        let d = path(
            &mut card,
            &mut dma,
            buf,
            READ_SECTORS,
            1,
            Data::Dma,
            &mut sums,
            false,
            None,
        );
        report("R5 D READ SECTORS, DMA from the data window", d);

        let most = usize::from(id[47] & 0xFF);
        let block = [32usize, 16, 8, 4, 2]
            .into_iter()
            .find(|&b| b <= most)
            .unwrap_or(1);
        if block < 2 {
            log!(
                "R5 C/E: the card has no READ MULTIPLE (w47 {:#06x})",
                id[47]
            );
            return;
        }
        let set = card
            .command(SET_MULTIPLE, 0, block as u8)
            .and_then(|()| card.finish());
        if let Err(e) = set {
            log!("R5 SET MULTIPLE {block} refused: {e:?}");
            return;
        }
        log!("R5 SET MULTIPLE {block}");
        let c = path(
            &mut card,
            &mut dma,
            buf,
            READ_MULTIPLE,
            block,
            Data::Window32,
            &mut sums,
            false,
            None,
        );
        report("R5 C READ MULTIPLE, 32-bit loads from the data window", c);
        let e = path(
            &mut card,
            &mut dma,
            buf,
            READ_MULTIPLE,
            block,
            Data::Dma,
            &mut sums,
            false,
            None,
        );
        report("R5 E READ MULTIPLE, DMA from the data window", e);
    }

    /// A short log line built in place.
    struct Line {
        buf: [u8; 800],
        len: usize,
    }

    impl Line {
        fn new() -> Self {
            Self {
                buf: [0; 800],
                len: 0,
            }
        }
        fn as_str(&self) -> &str {
            core::str::from_utf8(&self.buf[..self.len]).unwrap_or("?")
        }
    }

    impl Write for Line {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let b = s.as_bytes();
            let n = b.len().min(self.buf.len() - self.len);
            self.buf[self.len..self.len + n].copy_from_slice(&b[..n]);
            self.len += n;
            Ok(())
        }
    }

    fn on_fault(kind: rt::Fault, addr: u32) -> ! {
        crash(format_args!("FAULT {} at {addr:#010x}", kind.name()))
    }

    #[inline(never)]
    fn crash(msg: core::fmt::Arguments) -> ! {
        devices::crash_to_loader(msg);
        loop {
            core::hint::spin_loop();
        }
    }

    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        crash(format_args!("PANIC {}", info.message()))
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
