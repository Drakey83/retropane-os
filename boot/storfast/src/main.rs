//! Device test for the storage speed-ups (docs/research/storage-speed.md,
//! items 1-4), before and after numbers in one run:
//!
//! - **C1-C3, CompactFlash** (reads only; nothing is written to the card):
//!   the attach with the 100 µs RESET pulse, the read path the attach check
//!   picked; 4 MiB from LBA 0 by the 16-bit port, 32-bit window loads and
//!   DMA from the window, each sector compared with the port's; 20 power
//!   and reset cycles, each attach timed and 64 sectors compared.
//! - **S1, SD reads by size** (item 1): every size class 2 blocks to 4 MiB
//!   by DMA, each one DMA transfer and every byte checked; 1 MiB and 4 MiB
//!   commands timed by DMA and by the FIFO.
//! - **S2, SD write soak** (item 2): DMA writes of every size from 1 to 256
//!   blocks and every 8th to 2048, at varying places in the test file,
//!   each read back by DMA and every fourth through the FIFO too; 64 KiB
//!   and 1 MiB writes timed by DMA and by the FIFO.
//! - **S3, card swap and lock switch**: the screen asks the owner to take
//!   the card out and put it back (the last writes must still read back,
//!   then more soak), then to slide the lock on (reads only while locked)
//!   and off again (more soak).
//!
//! The shared card (docs: the loader kit, other sessions' files): this
//! writes only inside `/rp-storage-fast/` (its two test files, made here
//! through the FAT layer, written raw only inside their own clusters,
//! which must be contiguous), and removes them and the folder at the end.
//!
//! The loader resets the SD card and re-attaches the CF card when this
//! returns.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed with the verdict.
pub const MARKER: &str = "RETROPANE-STORFAST";

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_cf::{CfCard, ReadPath};
    use rp_drv_cpld::{Cpld, Delay};
    use rp_drv_mbx_display::Display;
    use rp_drv_mbx_display::console::Console;
    use rp_drv_pxa_gpio::Gpio;
    use rp_drv_pxa_mmc::PxaMmc;
    use rp_drv_sdcard::{BLOCK, SdCard, mbr_partitions};
    use rp_fat::Volume;
    use rp_hal_api::BlockDevice;
    use rp_hal_pxa27x::{Mmio, dma::Dma, rt};
    use rp_hal_x51v::MBX_VRAM_SIZE;
    use rp_hal_x51v::devices::{self, RunLog};

    use super::MARKER;

    rp_hal_pxa27x::entry!(main);

    macro_rules! log {
        ($($t:tt)*) => {{ let _ = writeln!(RunLog, $($t)*); }};
    }

    type Card = SdCard<PxaMmc<Mmio, Dma>>;
    type Cf = CfCard<Mmio, Mmio, Option<Dma>>;

    /// SD slot switches (docs/hw/sd.md) and the CF slot (docs/hw/cf.md).
    const SD_DETECT: u8 = 12;
    const SD_LOCK: u8 = 27;
    const CF_DETECT: u8 = 13;
    const CF_READY: u8 = 84;

    /// The test's folder and files: nothing outside them is written.
    const READ_FILE: &str = "rp-storage-fast/READ.BIN";
    const SOAK_FILE: &str = "rp-storage-fast/SOAK.BIN";
    const DIR: &str = "rp-storage-fast";
    const MIB: usize = 1024 * 1024;
    /// On the SD card's root, this file makes the run short (the emulator
    /// check, `cargo xtask emu-storfast`: the emulated FIFO moves bytes
    /// slowly, and nobody is there for the prompts).
    const QUICK_FILE: &str = "storfast-quick.txt";
    static QUICK: rt::Global<bool> = rt::Global::new(false);

    fn quick() -> bool {
        QUICK.with(|q| *q)
    }

    /// The read test file: 4 MiB (2 MiB quick).
    fn read_blocks() -> u32 {
        if quick() { 4096 } else { 8192 }
    }

    /// The soak file: 16 MiB (2 MiB quick).
    fn soak_blocks() -> u32 {
        if quick() { 4096 } else { 32768 }
    }

    /// How long the owner gets for each step on screen.
    fn prompt_secs() -> u32 {
        if quick() { 2 } else { 120 }
    }

    fn ms(ticks: u32) -> u32 {
        (u64::from(ticks) * 1000 / u64::from(rt::TICK_HZ)) as u32
    }

    fn kib_s(bytes: usize, ticks: u32) -> u64 {
        (bytes as u64 * u64::from(rt::TICK_HZ)) / (u64::from(ticks.max(1)) * 1024)
    }

    /// Word `w` of block `b` of a pattern `seed` names.
    fn pattern(seed: u32, b: u32, w: u32) -> u32 {
        (b ^ seed.rotate_left(16)).wrapping_mul(0x9E37_79B1)
            ^ w.wrapping_mul(0x85EB_CA6B)
            ^ seed.wrapping_mul(0xC2B2_AE35)
            ^ 0x5D7E_5A1E
    }

    fn fill(buf: &mut [u8], seed: u32, first: u32) {
        for (k, block) in buf.as_chunks_mut::<BLOCK>().0.iter_mut().enumerate() {
            for (w, v) in block.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                *v = pattern(seed, first + k as u32, w as u32).to_le_bytes();
            }
        }
    }

    /// Blocks of `buf` that don't hold pattern `seed` from block `first`.
    fn differs(buf: &[u8], seed: u32, first: u32) -> u32 {
        let mut bad = 0;
        for (k, block) in buf.as_chunks::<BLOCK>().0.iter().enumerate() {
            let ok =
                block.as_chunks::<4>().0.iter().enumerate().all(|(w, v)| {
                    u32::from_le_bytes(*v) == pattern(seed, first + k as u32, w as u32)
                });
            if !ok {
                bad += 1;
            }
        }
        bad
    }

    struct Wait;
    impl Delay for Wait {
        fn delay_us(&mut self, us: u32) {
            rt::delay_us(us);
        }
    }

    /// Text for the owner, on screen when there is one, and in the log.
    struct Screen<'a>(Option<Console<'a>>);

    impl Screen<'_> {
        fn say(&mut self, args: core::fmt::Arguments) {
            log!("{args}");
            if let Some(c) = self.0.as_mut() {
                let _ = c.write_fmt(args);
                let _ = c.write_str("\n");
            }
        }
    }

    fn main() -> ! {
        rt::set_fault_handler(on_fault);
        rt::init_mmu();
        log!("{MARKER} 1: storage speed-ups, device check (docs/research/storage-speed.md)");
        let screen = devices::take_mbx().map(|(regs, vram)| {
            let (plane, _) = Display::new(regs).adopt(MBX_VRAM_SIZE as usize);
            let end = plane.offset + plane.stride * usize::from(plane.height);
            Console::new(
                &mut vram[plane.offset..end],
                plane.stride,
                (0, 0),
                (480, 640),
                2,
                (0xFFFF, 0x0000),
            )
        });
        let mut screen = Screen(screen);
        screen.say(format_args!("Storage test: please wait"));
        run(&mut screen);
        screen.say(format_args!("Storage test done."));
        log!("{MARKER}: done");
        devices::return_to_loader();
        loop {
            core::hint::spin_loop();
        }
    }

    fn run(screen: &mut Screen<'_>) {
        let Some(mut gpio) = rt::take_gpio().map(Gpio::new) else {
            log!("no GPIO");
            return;
        };
        let Some(mut cpld) = devices::take_cpld().map(Cpld::new) else {
            log!("no CPLD");
            return;
        };
        // The CPLD doesn't read back: keep what the loader keeps on.
        cpld.set_backlight_enable(true);
        cpld.set_sd_power(true);
        let (Some(scratch), Some(pool)) = (devices::take_scratch(), devices::take_ram_pool_bytes())
        else {
            log!("no scratch or RAM pool");
            return;
        };
        gpio.make_input(SD_DETECT);
        gpio.make_input(SD_LOCK);
        // The SD card first, for the quick-run file; its tests come last.
        let card = sd_card(&mut gpio);
        if gpio.read(CF_DETECT) {
            log!("C0 no CF card (GPIO13 high): CF part skipped");
        } else {
            compact_flash(&mut gpio, &mut cpld, pool);
        }
        rt::check_in();
        if let Some((card, start)) = card {
            sd(screen, &mut gpio, scratch, pool, card, start);
        }
    }

    /// The SD card and its partition's start; sets the quick flag if the
    /// root has [`QUICK_FILE`].
    fn sd_card(gpio: &mut Gpio<Mmio>) -> Option<(Card, u32)> {
        if gpio.read(SD_DETECT) {
            log!("S0 no SD card: SD part skipped");
            return None;
        }
        let (Some(mmc), Some(dma)) = (rt::take_mmc(), rt::take_dma(0)) else {
            log!("S0 no MMC or DMA");
            return None;
        };
        let mut card = match SdCard::try_init(PxaMmc::with_dma(mmc, dma)) {
            Ok(c) => c,
            Err((e, _)) => {
                log!("S0 card init failed: {}", e.name());
                return None;
            }
        };
        card.host_mut().set_dma(true, false);
        let start = partition(&mut card)?;
        if Volume::mount(&mut card, start).is_ok_and(|mut v| v.find(QUICK_FILE).is_ok()) {
            QUICK.with(|q| *q = true);
            log!("S0 {QUICK_FILE} found: a short run");
        }
        Some((card, start))
    }

    // ---- CompactFlash -------------------------------------------------

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

    /// Power-cycles the slot as the loader does, then takes the card (the
    /// RESET pulse inside); the attach (reset to ready to IDENTIFY and the
    /// path checks) timed.
    fn attach(
        gpio: &mut Gpio<Mmio>,
        cpld: &mut Cpld<Mmio>,
        attr: Mmio,
        common: Mmio,
        dma: Option<Dma>,
    ) -> Result<(Cf, u32), (Mmio, Mmio, Option<Dma>)> {
        cpld.set_cf_power(false, &mut Wait);
        rt::delay_us(308_000);
        cpld.set_cf_power(true, &mut Wait);
        rt::delay_us(100_000);
        rt::check_in();
        let t0 = rt::ticks();
        let taken = CfCard::init(attr, common, &mut Lines { gpio, cpld }, &mut |us| {
            rt::delay_us(us);
        });
        match taken {
            Ok((card, _)) => {
                let phys = devices::cf_common_phys().unwrap_or(0);
                let card = card.with_dma(dma, phys);
                Ok((card, rt::ticks().wrapping_sub(t0)))
            }
            Err((e, attr, common)) => {
                log!("C attach failed: {}", e.name());
                Err((attr, common, dma))
            }
        }
    }

    fn compact_flash(gpio: &mut Gpio<Mmio>, cpld: &mut Cpld<Mmio>, pool: &mut [u8]) {
        let Some((attr, common)) = devices::take_cf() else {
            log!("C0 no CF windows");
            return;
        };
        let dma = rt::take_dma(2);
        log!(
            "C0 DMA channel 2 {}",
            if dma.is_some() { "taken" } else { "MISSING" }
        );
        let (mut cf, t) = match attach(gpio, cpld, attr, common, dma) {
            Ok(c) => c,
            Err(_) => return,
        };
        log!(
            "C1 attach (RESET 100 us, was 100 ms): {} ms reset to ready; {} sectors; reads by {} ({} paths refused)",
            ms(t),
            cf.sectors(),
            cf.read_path().name(),
            cf.path_mismatches()
        );
        // C2: 4 MiB three ways, each sector against the port's.
        let len = if quick() { MIB } else { 4 * MIB };
        let (buf, rest) = pool.split_at_mut(len);
        let reference = &mut rest[..len];
        let proven = cf.read_path();
        for path in [ReadPath::Port16, ReadPath::Window32, ReadPath::Dma] {
            rt::check_in();
            if !cf.set_read_path(path) {
                log!("C2 {}: not proven on this card, skipped", path.name());
                continue;
            }
            buf.fill(0xA5);
            let refused = cf.dma_refused();
            let t0 = rt::ticks();
            let mut failed = 0;
            for (i, run) in buf.chunks_mut(64 * 1024).enumerate() {
                rt::check_in();
                if cf.read_blocks((i * 128) as u32, run).is_err() {
                    failed += 1;
                }
            }
            let t = rt::ticks().wrapping_sub(t0);
            let mut bad = 0;
            if path == ReadPath::Port16 {
                reference.copy_from_slice(buf);
            } else {
                bad = buf
                    .as_chunks::<512>()
                    .0
                    .iter()
                    .zip(reference.as_chunks::<512>().0)
                    .filter(|(a, b)| a != b)
                    .count();
            }
            log!(
                "C2 read {} KiB by {}: {} KiB/s, {failed} commands failed, {bad} sectors differ from the port, {} DMA refusals",
                len / 1024,
                path.name(),
                kib_s(len, t),
                cf.dma_refused() - refused
            );
        }
        let _ = cf.set_read_path(proven);
        // C3: 20 attach cycles, 64 sectors compared each time.
        let mut slowest = 0;
        let mut total = 0;
        let mut good = 0;
        let cycles = if quick() { 3 } else { 20 };
        for cycle in 1..=cycles {
            rt::check_in();
            let (attr, common, dma) = cf.into_parts();
            match attach(gpio, cpld, attr, common, dma) {
                Ok((c, t)) => {
                    cf = c;
                    slowest = slowest.max(ms(t));
                    total += ms(t);
                    let run = &mut buf[..64 * 512];
                    let read = cf.read_blocks(0, run).is_ok();
                    let same = *run == reference[..64 * 512];
                    if read && same && cf.read_path() == proven {
                        good += 1;
                    } else {
                        log!(
                            "C3 cycle {cycle}: read {read}, same {same}, path {}",
                            cf.read_path().name()
                        );
                    }
                }
                Err(_) => {
                    log!("C3 cycle {cycle}: attach failed, stopping");
                    return;
                }
            }
        }
        log!(
            "C3 {cycles} attach cycles: {}; reset to ready {} ms average, {slowest} ms slowest",
            if good == cycles {
                "all good"
            } else {
                "SOME FAILED"
            },
            total / cycles
        );
    }

    // ---- SD -----------------------------------------------------------

    fn sd(
        screen: &mut Screen<'_>,
        gpio: &mut Gpio<Mmio>,
        scratch: &mut [u8],
        pool: &mut [u8],
        mut card: Card,
        start: u32,
    ) {
        let (work, _) = scratch.split_at_mut(256 * 1024);
        // The test files: made through the FAT layer with the FIFO (DMA
        // writes are what S2 tests), checked contiguous.
        card.host_mut().set_dma(true, false);
        let Some(read_lba) = test_file(&mut card, start, READ_FILE, read_blocks(), pool, work)
        else {
            cleanup(&mut card, start, work);
            return;
        };
        let Some(soak_lba) = test_file(&mut card, start, SOAK_FILE, soak_blocks(), pool, work)
        else {
            cleanup(&mut card, start, work);
            return;
        };
        reads(&mut card, read_lba, pool);
        // Timed first: its writes would overwrite the soak's, which S3
        // reads back after the swap.
        write_speed(&mut card, soak_lba, pool);
        let mut recent = Recent::default();
        soak(&mut card, soak_lba, pool, 0, &mut recent);
        if let Some(c) = swap_and_lock(screen, gpio, card, soak_lba, pool, &mut recent) {
            card = c;
        } else {
            return;
        }
        cleanup(&mut card, start, work);
    }

    fn partition(card: &mut Card) -> Option<u32> {
        let mut block = [0u8; BLOCK];
        if card.read(0, &mut block).is_err() {
            log!("S0 block 0 unreadable");
            return None;
        }
        Some(
            mbr_partitions(&block)
                .and_then(|p| p[0])
                .map_or(0, |p| p.start),
        )
    }

    /// Makes `path` (`blocks` of pattern 0) in the test folder, finds its
    /// first block and checks its clusters are one run; reads it back by
    /// the FIFO against the pattern.
    fn test_file(
        card: &mut Card,
        start: u32,
        path: &str,
        blocks: u32,
        pool: &mut [u8],
        work: &mut [u8],
    ) -> Option<u32> {
        let data = &mut pool[..blocks as usize * BLOCK];
        fill(data, 0, 0);
        let first = {
            let mut vol = match Volume::mount(&mut *card, start) {
                Ok(v) => v,
                Err(e) => {
                    log!("S0 not FAT32 ({})", e.name());
                    return None;
                }
            };
            rt::check_in();
            let t0 = rt::ticks();
            if let Err(e) = vol.put(path, data, work) {
                log!("S0 put {path}: {}", e.name());
                return None;
            }
            log!("S0 {path} made ({} ms)", ms(rt::ticks().wrapping_sub(t0)));
            match vol.find(path) {
                Ok(f) if f.size() == blocks * BLOCK as u32 && f.first_cluster() >= 2 => {
                    f.first_cluster()
                }
                _ => {
                    log!("S0 {path} not as written");
                    return None;
                }
            }
        };
        let lba = contiguous(card, start, first, blocks, work)?;
        // Back through the FIFO (DMA reads are what S1 tests).
        card.host_mut().set_dma(false, false);
        let mut bad = 0;
        for c in 0..blocks / 256 {
            rt::check_in();
            let buf = &mut pool[..256 * BLOCK];
            if card.read_many(lba + c * 256, buf).is_err() {
                bad += 256;
            } else {
                bad += differs(buf, 0, c * 256);
            }
        }
        card.host_mut().set_dma(true, false);
        if bad != 0 {
            log!("S0 {path}: {bad} blocks differ after writing it");
            return None;
        }
        log!("S0 {path}: LBA {lba}, {blocks} blocks, contiguous, read back exact");
        Some(lba)
    }

    /// The LBA of the file's first block if its clusters are one run.
    fn contiguous(
        card: &mut Card,
        start: u32,
        first: u32,
        blocks: u32,
        work: &mut [u8],
    ) -> Option<u32> {
        let bpb = &mut work[..BLOCK];
        card.read_many(start, bpb).ok()?;
        let spc = u32::from(bpb[13]);
        let reserved = u32::from(u16::from_le_bytes([bpb[14], bpb[15]]));
        let fats = u32::from(bpb[16]);
        let fat_sz = u32::from_le_bytes([bpb[36], bpb[37], bpb[38], bpb[39]]);
        if spc == 0 || !spc.is_power_of_two() || fats == 0 {
            log!("S0 boot sector odd: spc {spc} fats {fats}");
            return None;
        }
        let fat_start = start + reserved;
        let data_start = fat_start + fats * fat_sz;
        let clusters = blocks.div_ceil(spc);
        let mut sector = u32::MAX;
        let (_, fat) = work.split_at_mut(BLOCK);
        let fat = &mut fat[..BLOCK];
        for k in 0..clusters {
            let c = first + k;
            let s = fat_start + c / 128;
            if s != sector {
                card.read_many(s, fat).ok()?;
                sector = s;
            }
            let i = (c % 128) as usize * 4;
            let next =
                u32::from_le_bytes([fat[i], fat[i + 1], fat[i + 2], fat[i + 3]]) & 0x0FFF_FFFF;
            let end = k + 1 == clusters;
            if (end && next < 0x0FFF_FFF8) || (!end && next != c + 1) {
                log!("S0 file isn't contiguous (cluster {c} -> {next:#x}): no raw writes");
                return None;
            }
        }
        Some(data_start + (first - 2) * spc)
    }

    /// S1: every size class by DMA, one transfer each, every byte checked;
    /// then 1 MiB and 4 MiB commands timed both ways.
    fn reads(card: &mut Card, lba: u32, pool: &mut [u8]) {
        card.host_mut().set_dma(true, false);
        let mut ok = 0;
        let sizes = [2u32, 3, 8, 63, 64, 255, 256, 257, 1024, 2048, 4096, 8192];
        let sizes = &sizes[..if quick() { 11 } else { 12 }];
        for &blocks in sizes {
            rt::check_in();
            let buf = &mut pool[..blocks as usize * BLOCK];
            buf.fill(0xA5);
            let at = (blocks * 7) % (read_blocks() - blocks + 1);
            let before = card.host().dma_transfers();
            let r = card.read_many(lba + at, buf);
            let n = card.host().dma_transfers() - before;
            let bad = differs(buf, 0, at);
            if r.is_ok() && n == 1 && bad == 0 {
                ok += 1;
            } else {
                log!(
                    "S1 {blocks} blocks: {:?}, {n} DMA transfers, {bad} blocks differ",
                    r.err()
                );
            }
        }
        log!(
            "S1 every size class by DMA ({} sizes, 2 blocks to {} KiB): {}, one transfer each",
            sizes.len(),
            sizes[sizes.len() - 1] / 2,
            if ok == sizes.len() {
                "all right"
            } else {
                "SOME WRONG"
            }
        );
        for (dma, name) in [(false, "FIFO (before)"), (true, "DMA (after)")] {
            card.host_mut().set_dma(dma, false);
            let runs = if quick() {
                [(2048usize, 1usize), (4096, 1)]
            } else {
                [(2048, 8), (8192, 2)]
            };
            for (blocks, n) in runs {
                let t0 = rt::ticks();
                let mut failed = 0;
                for i in 0..n {
                    rt::check_in();
                    let at = (i * blocks) as u32 % read_blocks();
                    let at = at.min(read_blocks() - blocks as u32);
                    if card
                        .read_many(lba + at, &mut pool[..blocks * BLOCK])
                        .is_err()
                    {
                        failed += 1;
                    }
                }
                let t = rt::ticks().wrapping_sub(t0);
                log!(
                    "S1 read {} KiB commands x{n} by {name}: {} KiB/s, {failed} failed",
                    blocks / 2,
                    kib_s(n * blocks * BLOCK, t)
                );
            }
        }
        card.host_mut().set_dma(true, false);
    }

    /// The last writes, to read back after the card was taken out.
    #[derive(Default)]
    struct Recent {
        items: [(u32, u32, u32); 16],
        next: usize,
        len: usize,
    }

    impl Recent {
        fn push(&mut self, at: u32, blocks: u32, seed: u32) {
            self.items[self.next] = (at, blocks, seed);
            self.next = (self.next + 1) % self.items.len();
            self.len = (self.len + 1).min(self.items.len());
        }

        /// Reads back each remembered write that no later one overlaps:
        /// (writes checked, blocks that differ).
        fn verify(&self, card: &mut Card, lba: u32, pool: &mut [u8]) -> (u32, u32) {
            let n = self.items.len();
            let mut checked = 0;
            let mut bad = 0;
            for age in 0..self.len {
                let (at, blocks, seed) = self.items[(self.next + n - 1 - age) % n];
                let overlapped = (0..age).any(|newer| {
                    let (b, len, _) = self.items[(self.next + n - 1 - newer) % n];
                    at < b + len && b < at + blocks
                });
                if overlapped {
                    continue;
                }
                rt::check_in();
                let buf = &mut pool[..blocks as usize * BLOCK];
                if card.read_many(lba + at, buf).is_err() {
                    bad += blocks;
                } else {
                    bad += differs(buf, seed, at);
                }
                checked += 1;
            }
            (checked, bad)
        }
    }

    #[derive(Default)]
    struct Soaked {
        writes: u32,
        bytes: u64,
        write_errors: u32,
        read_errors: u32,
        bad_blocks: u32,
        fifo_checks: u32,
        dma: u32,
    }

    /// One write by DMA of pattern `seed` to `blocks` at `at`, and its
    /// read-back (by DMA; also by the FIFO when `fifo_too`).
    fn soak_one(
        card: &mut Card,
        lba: u32,
        (at, blocks, seed): (u32, u32, u32),
        fifo_too: bool,
        pool: &mut [u8],
        s: &mut Soaked,
    ) {
        let (data, back) = pool.split_at_mut(4 * MIB);
        let data = &mut data[..blocks as usize * BLOCK];
        let back = &mut back[..blocks as usize * BLOCK];
        fill(data, seed, at);
        card.host_mut().set_dma(true, true);
        let before = card.host().dma_transfers();
        if card.write_many(lba + at, data).is_err() {
            s.write_errors += 1;
        }
        back.fill(0x5A);
        if card.read_many(lba + at, back).is_err() {
            s.read_errors += 1;
        }
        s.dma += card.host().dma_transfers() - before;
        s.bad_blocks += differs(back, seed, at);
        if fifo_too {
            card.host_mut().set_dma(false, true);
            back.fill(0x3C);
            if card.read_many(lba + at, back).is_err() {
                s.read_errors += 1;
            }
            s.bad_blocks += differs(back, seed, at);
            s.fifo_checks += 1;
            card.host_mut().set_dma(true, true);
        }
        s.writes += 1;
        s.bytes += u64::from(blocks) * BLOCK as u64;
    }

    /// S2: every size 1-256 blocks, then every 8th to 2048 (and 2047,
    /// 2048), at places spread through the file; `round` changes the
    /// pattern so a write that didn't land shows.
    fn soak(card: &mut Card, lba: u32, pool: &mut [u8], round: u32, recent: &mut Recent) -> Soaked {
        let mut s = Soaked::default();
        let quick_sizes = [1u32, 2, 3, 7, 8, 9, 63, 255, 256, 257, 2047, 2048];
        let full = (1..=256u32).chain((264..=2048).step_by(8)).chain([2047]);
        let n = if quick() { 0 } else { usize::MAX };
        let sizes = full
            .take(n)
            .chain(quick_sizes.into_iter().take(if quick() { 12 } else { 0 }));
        let mut x = 0x2545_F491u32 ^ round;
        let t0 = rt::ticks();
        for (k, blocks) in (0u32..).zip(sizes) {
            rt::check_in();
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let at = x % (soak_blocks() - blocks + 1);
            let seed = round.wrapping_mul(0x1_0000) + k + 1;
            soak_one(
                card,
                lba,
                (at, blocks, seed),
                k.is_multiple_of(4),
                pool,
                &mut s,
            );
            recent.push(at, blocks, seed);
        }
        let t = rt::ticks().wrapping_sub(t0);
        log!(
            "S2 soak round {round}: {} writes, {} KiB written and read back in {} ms; {} DMA transfers; {} write errors, {} read errors, {} blocks differ ({} also read by the FIFO)",
            s.writes,
            s.bytes / 1024,
            ms(t),
            s.dma,
            s.write_errors,
            s.read_errors,
            s.bad_blocks,
            s.fifo_checks
        );
        s
    }

    /// Write speed before (FIFO) and after (DMA), as the driver writes.
    fn write_speed(card: &mut Card, lba: u32, pool: &mut [u8]) {
        for (dma, name) in [(false, "FIFO (before)"), (true, "DMA (after)")] {
            card.host_mut().set_dma(true, dma);
            let runs = if quick() {
                [(128usize, 2usize), (2048, 1)]
            } else {
                [(128, 32), (2048, 4)]
            };
            for (blocks, n) in runs {
                let buf = &mut pool[..blocks * BLOCK];
                let mut failed = 0;
                let t0 = rt::ticks();
                for i in 0..n {
                    rt::check_in();
                    let at = (i * blocks) as u32;
                    fill(buf, 0x77, at);
                    if card.write_many(lba + at, buf).is_err() {
                        failed += 1;
                    }
                }
                let t = rt::ticks().wrapping_sub(t0);
                log!(
                    "S2 write {} KiB commands x{n} by {name}: {} KiB/s, {failed} failed",
                    blocks / 2,
                    kib_s(n * blocks * BLOCK, t)
                );
            }
        }
        // Read it all back.
        card.host_mut().set_dma(true, true);
        let mut bad = 0;
        for i in 0..if quick() { 1 } else { 4 } {
            rt::check_in();
            let buf = &mut pool[..2048 * BLOCK];
            if card.read_many(lba + i * 2048, buf).is_err() {
                bad += 2048;
            } else {
                bad += differs(buf, 0x77, i * 2048);
            }
        }
        log!("S2 the speed writes read back: {bad} blocks differ");
    }

    /// Waits for a slot switch to read `want` (twice, 100 ms apart).
    fn wait_for(gpio: &mut Gpio<Mmio>, pin: u8, want: bool) -> Option<u32> {
        let t0 = rt::ticks();
        for _ in 0..prompt_secs() * 10 {
            rt::check_in();
            if gpio.read(pin) == want {
                rt::delay_us(100_000);
                if gpio.read(pin) == want {
                    return Some(ms(rt::ticks().wrapping_sub(t0)));
                }
            }
            rt::delay_us(100_000);
        }
        None
    }

    /// S3: the card taken out and put back, then the lock switch.
    fn swap_and_lock(
        screen: &mut Screen<'_>,
        gpio: &mut Gpio<Mmio>,
        card: Card,
        lba: u32,
        pool: &mut [u8],
        recent: &mut Recent,
    ) -> Option<Card> {
        screen.say(format_args!("\nTAKE THE SD CARD OUT now."));
        let mut host = card.into_inner();
        match wait_for(gpio, SD_DETECT, true) {
            Some(t) => log!("S3 card out after {t} ms"),
            None => {
                screen.say(format_args!("No card removal seen: skipped."));
            }
        }
        screen.say(format_args!("PUT THE SD CARD BACK in."));
        if wait_for(gpio, SD_DETECT, false).is_none() {
            screen.say(format_args!("No card seen: stopping (files stay)."));
            log!(
                "S3 no card after {} s: the test files are left in {DIR}",
                prompt_secs()
            );
            return None;
        }
        rt::delay_us(200_000);
        let mut card = match SdCard::try_init(host) {
            Ok(c) => c,
            Err((e, h)) => {
                log!("S3 card back, init failed: {}; once more", e.name());
                host = h;
                rt::delay_us(500_000);
                match SdCard::try_init(host) {
                    Ok(c) => c,
                    Err((e, _)) => {
                        log!("S3 init failed again: {}", e.name());
                        return None;
                    }
                }
            }
        };
        screen.say(format_args!("Card back. Checking..."));
        card.host_mut().set_dma(true, true);
        let (checked, bad) = recent.verify(&mut card, lba, pool);
        log!("S3 after the swap: {checked} of the last writes read back, {bad} blocks differ");
        soak(&mut card, lba, pool, 1, recent);
        screen.say(format_args!(
            "\nSLIDE THE SD CARD'S LOCK SWITCH ON (locked), card stays in."
        ));
        match wait_for(gpio, SD_LOCK, true) {
            Some(t) => {
                log!("S3 lock on after {t} ms (GPIO27 high)");
                // Locked: reads only (the lock is the owner's: nothing writes).
                let (checked, bad) = recent.verify(&mut card, lba, pool);
                log!(
                    "S3 while locked, {checked} of the last writes read by DMA: {bad} blocks differ"
                );
                screen.say(format_args!("Now SLIDE THE LOCK SWITCH OFF again."));
                if wait_for(gpio, SD_LOCK, false).is_none() {
                    screen.say(format_args!("Still locked: stopping (files stay)."));
                    log!(
                        "S3 still locked after {} s: the test files are left in {DIR}",
                        prompt_secs()
                    );
                    return None;
                }
                log!("S3 lock off");
            }
            None => {
                screen.say(format_args!("No lock seen (GPIO27 stayed low): skipped."));
                log!("S3 GPIO27 never read high in {} s", prompt_secs());
            }
        }
        screen.say(format_args!("Thank you: last checks..."));
        soak(&mut card, lba, pool, 2, recent);
        Some(card)
    }

    /// Removes the test files and the folder (only them).
    fn cleanup(card: &mut Card, start: u32, _work: &mut [u8]) {
        card.host_mut().set_dma(true, false);
        let Ok(mut vol) = Volume::mount(&mut *card, start) else {
            log!("S9 mount failed: the test files are left in {DIR}");
            return;
        };
        for path in [READ_FILE, SOAK_FILE, DIR] {
            match vol.remove(path) {
                Ok(()) | Err(rp_fat::Error::NotFound) => {}
                Err(e) => log!("S9 remove {path}: {}", e.name()),
            }
        }
        log!("S9 test files and folder removed");
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
