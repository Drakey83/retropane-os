//! Device test for the storage speed-ups (docs/research/storage-speed.md,
//! items 1-4), before and after numbers in one run:
//!
//! - **C1-C3, CompactFlash** (reads only; nothing is written to the card):
//!   the attach with the 100 µs RESET pulse, the read path the attach check
//!   picked; 4 MiB from LBA 0 by the 16-bit port, 32-bit window loads and
//!   DMA from the window, each sector compared with the port's; 20 power
//!   and reset cycles, each attach timed and 64 sectors compared.
//! - **C4, the card's own timing** (reminder R7, the owner's ruling M;
//!   reads only): `CfCard::tune` finds the shortest timing the card reads
//!   right at and backs off 1.5x (timed), then each read path's speed and
//!   every sector against the port's at Windows Mobile's timing. Windows
//!   Mobile's timing is put back at the end and by the crash path.
//! - **C7, CF writes at a faster timing** (reminder R28): a 64-sector file
//!   made in `/rp-cf-write/` on the CF card (its FAT32, at Windows Mobile's
//!   timing), then written again and again with only the data words at a
//!   shorter strobe (`CfCard::write_data_at`: the command and address at
//!   Windows Mobile's timing, so a too-fast step spoils only that file's
//!   data), each read back at the socket's slowest; the shortest right one
//!   with the margin, three more rounds at it, the speed against Windows
//!   Mobile's; then **C9**: the same file written with one field at a
//!   time shorter than Windows Mobile's (the strobe, the hold, the set-up,
//!   the others kept), each down to the card's read limit in it, every
//!   step timed and read back (C7's faster steps all wrote at ~240 KiB/s
//!   against ~4,700: which field does it); then **C10**: the same write
//!   with each phase timed (the command, the switch to the data timing,
//!   the card's DRQ waits, the data words, the switch back, its finish) at
//!   Windows Mobile's data timing and at strobes 9 and 1, twice each; then
//!   **C11**: a 512 KiB file reserved beside it, twelve regions written
//!   once each with the timings rotated, one region rewritten six times
//!   (the card's rewrite cost), a fresh region after a read at the slowest
//!   timing and one at strobe 1 after the rewrites, all read back; the
//!   files and folder removed.
//!   Nothing else on the card is written.
//! - **C8, the CPLD's interrupt line** (reminder R24, reads only): the
//!   owner pulls the CF card and puts it back while GPIO13 and CPLD `0x00`,
//!   `0x04`, `0x06`, `0x14` are read every millisecond, each change logged.
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

    use rp_drv_cf::timing::{self, Timing};
    use rp_drv_cf::{CfCard, ReadPath, SocketTiming, Tuned, WritePhase};
    use rp_drv_cpld::{Cpld, Delay, handover};
    use rp_drv_mbx_display::Display;
    use rp_drv_mbx_display::console::Console;
    use rp_drv_pxa_gpio::Gpio;
    use rp_drv_pxa_mmc::PxaMmc;
    use rp_drv_sdcard::{BLOCK, SdCard, mbr_partitions};
    use rp_fat::Volume;
    use rp_hal_api::BlockDevice;
    use rp_hal_pxa27x::clocks;
    use rp_hal_pxa27x::{Mmio, dma::Dma, dma_map, rt};
    use rp_hal_x51v::MBX_VRAM_SIZE;
    use rp_hal_x51v::devices::{self, RunLog};

    use super::MARKER;

    /// The CPLD, with the shadow table every program shares.
    type BoardCpld = Cpld<Mmio, handover::Table<'static>>;

    rp_image::entry!(main);

    macro_rules! log {
        ($($t:tt)*) => {{ let _ = writeln!(RunLog, $($t)*); }};
    }

    type Card = SdCard<PxaMmc<Mmio, Dma>>;
    type Cf = CfCard<Mmio, Mmio, Option<Dma>>;

    /// SD slot switches (docs/hw/sd.md) and the CF slot (docs/hw/cf.md).
    const SD_DETECT: u8 = 12;
    const SD_LOCK: u8 = 27;
    /// `RP_STORFAST_CF_ONLY`: the CF part alone (a card swapped in for a
    /// quick check; the SD card isn't touched).
    const CF_ONLY: bool = option_env!("RP_STORFAST_CF_ONLY").is_some();
    const CF_READY: u8 = 84;

    /// The test's folder and files: nothing outside them is written.
    const READ_FILE: &str = "rp-storage-fast/READ.BIN";
    const SOAK_FILE: &str = "rp-storage-fast/SOAK.BIN";
    const DIR: &str = "rp-storage-fast";
    /// C7's folder and file on the CF card: nothing outside them is written.
    const CF_DIR: &str = "rp-cf-write";
    const CF_FILE: &str = "rp-cf-write/WRITE.BIN";
    /// C7's file: 64 sectors.
    const CF_BLOCKS: u32 = 64;
    /// The PXA GPIO the CPLD's interrupt reaches (docs/hw/interrupts.md).
    const CPLD_IRQ_GPIO: u8 = 13;
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
        rt::init_mmu();
        log!("{MARKER} 1: storage speed-ups, device check (docs/research/storage-speed.md)");
        let screen = rp_image::take_mbx().map(|(regs, vram)| {
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
        rp_image::finish()
    }

    fn run(screen: &mut Screen<'_>) {
        let Some(mut gpio) = rt::take_gpio().map(Gpio::new) else {
            log!("no GPIO");
            return;
        };
        let Some(mut cpld) = devices::take_cpld().map(|(b, t)| Cpld::adopt(b, t).0) else {
            log!("no CPLD");
            return;
        };
        // R8: the CPLD's bus as `take_cpld` set it (RetroPane's RDF 6:
        // `MSC2` low half 0x7F6C; Windows Mobile's 0x7FFC).
        log!(
            "C0 MSC2 {:#010x} (the CPLD's bus, CS4 the low half)",
            rp_hal_pxa27x::diag::msc2()
        );
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
        // The SD card first, for the quick-run file; its tests come last
        // (the CF-only build skips them).
        let card = if CF_ONLY { None } else { sd_card(&mut gpio) };
        // The card is seen by the CPLD's bit, as the loader sees it: GPIO13
        // sticks at "no card" after a swap (cf.md, 2026-10-16).
        if !cpld.cf_present() {
            log!("C0 no CF card (CPLD 0x04 bit 1): CF part skipped");
            screen.say(format_args!("No CF card found."));
        } else {
            screen.say(format_args!(
                "CF card: reading it at each timing (about a minute)"
            ));
            compact_flash(screen, &mut gpio, &mut cpld, pool);
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
        let (Some(mmc), Some(dma)) = (rt::take_mmc(), rt::take_dma(dma_map::SD)) else {
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
        cpld: &'a mut BoardCpld,
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
        cpld: &mut BoardCpld,
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

    fn compact_flash(
        screen: &mut Screen<'_>,
        gpio: &mut Gpio<Mmio>,
        cpld: &mut BoardCpld,
        pool: &mut [u8],
    ) {
        let Some((attr, common)) = devices::take_cf() else {
            log!("C0 no CF windows");
            return;
        };
        let dma = rt::take_dma(dma_map::CF);
        log!(
            "C0 DMA channel 2 {}",
            if dma.is_some() { "taken" } else { "MISSING" }
        );
        rp_image::on_crash(put_back_cf_timing);
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
        let tuned = own_timing(&mut cf, buf, reference);
        put_back_cf_timing();
        log!(
            "C6 Windows Mobile's timing put back: MCMEM1 {:#010x}",
            devices::cf_mem_timing().unwrap_or(0)
        );
        if let Some((now, read_limit)) = tuned {
            screen.say(format_args!(
                "CF card: writing its test file at faster timings"
            ));
            write_test(&mut cf, now, read_limit, buf);
            put_back_cf_timing();
        }
        cpld_irq(screen, gpio, cpld);
    }

    fn set_cf_timing(value: u32) {
        let _ = devices::set_cf_mem_timing(value);
    }

    /// Where a card's first partition starts (0: no partition table).
    fn first_partition<B: BlockDevice>(dev: &mut B) -> Option<u32> {
        let mut block = [0u8; BLOCK];
        dev.read_blocks(0, &mut block).ok()?;
        Some(
            mbr_partitions(&block)
                .and_then(|p| p[0])
                .map_or(0, |p| p.start),
        )
    }

    /// C7: CF writes with only the data words at a shorter strobe (R28).
    fn write_test(cf: &mut Cf, tuned: u32, read_limit: u32, pool: &mut [u8]) {
        let wm = devices::CF_MEM_TIMING_WM;
        let high = wm & !0xF_FFFF;
        let slowest = high | Timing::SLOWEST.mcmem();
        let (data, rest) = pool.split_at_mut(CF_BLOCKS as usize * BLOCK);
        let (work, back) = rest.split_at_mut(64 * 1024);
        let back = &mut back[..data.len()];
        // The file, at Windows Mobile's timing.
        set_cf_timing(wm);
        let Some(start) = first_partition(cf) else {
            log!("C7 the CF card's block 0 unreadable: skipped");
            return;
        };
        fill(data, 0x5EED, 0);
        let first = {
            let Ok(mut vol) = Volume::mount(&mut *cf, start) else {
                log!("C7 the CF card isn't FAT32: skipped (nothing written)");
                return;
            };
            if let Err(e) = vol.put(CF_FILE, data, work) {
                log!("C7 put {CF_FILE}: {}", e.name());
                return;
            }
            match vol.find(CF_FILE) {
                Ok(f) if f.size() == CF_BLOCKS * BLOCK as u32 => f.first_cluster(),
                _ => {
                    log!("C7 {CF_FILE} not as written");
                    return;
                }
            }
        };
        let Some(lba) = contiguous(cf, start, first, CF_BLOCKS, work) else {
            remove_cf_file(cf, start, work);
            return;
        };
        log!("C7 {CF_FILE}: LBA {lba}, {CF_BLOCKS} sectors, contiguous");
        let t = Timing::from_mcmem(tuned);
        let len = data.len();
        // One round: write at `data_t` (command at Windows Mobile's), read
        // back at the slowest; (time, failed, sectors differ).
        let mut round = |cf: &mut Cf, data_t: Timing, seed: u32| -> (u32, bool, u32) {
            fill(data, seed, 0);
            cf.write_data_at(set_cf_timing, wm, wm, high | data_t.mcmem());
            let t0 = rt::ticks();
            let failed = cf.write_blocks(lba, data).is_err();
            let took = rt::ticks().wrapping_sub(t0);
            set_cf_timing(slowest);
            back.fill(0);
            let read = cf.read_blocks(lba, back).is_ok();
            set_cf_timing(wm);
            rt::check_in();
            let bad = if read {
                differs(back, seed, 0)
            } else {
                CF_BLOCKS
            };
            (took, failed, bad)
        };
        let wm_t = Timing::WINDOWS_MOBILE;
        let (t_wm, f_wm, b_wm) = round(cf, wm_t, 1);
        log!(
            "C7 at Windows Mobile's timing: {} KiB/s, failed {f_wm}, {b_wm} sectors differ",
            kib_s(len, t_wm)
        );
        // The strobe from Windows Mobile's down, set-up and hold as the
        // card's tuned read timing.
        let mut limit = None;
        // Down to the card's read limit's strobe: the DRQ polls in the data
        // phase are reads at the data timing.
        let floor = Timing::from_mcmem(read_limit).asst;
        for asst in (floor..=wm_t.asst).rev() {
            let step = Timing { asst, ..t };
            let (took, failed, bad) = round(cf, step, 100 + asst);
            log!(
                "C7 data strobe {asst} ({} cycles): {} KiB/s, failed {failed}, {bad} sectors differ",
                step.cycle_cycles(),
                kib_s(len, took)
            );
            if failed || bad != 0 || cf.is_dead() {
                break;
            }
            limit = Some(step);
        }
        match limit {
            Some(l) => {
                let (m, clamped) = timing::with_margin(l);
                let mut bad_all = 0;
                let mut took_all = 0u32;
                for r in 0..3 {
                    let (took, failed, bad) = round(cf, m, 200 + r);
                    bad_all += bad + u32::from(failed) * CF_BLOCKS;
                    took_all = took_all.wrapping_add(took);
                }
                log!(
                    "C7 the shortest right data strobe {} ({} cycles); with the margin {:#010x} ({} cycles, clamped {clamped}): 3 rounds, {bad_all} sectors differ, {} KiB/s against {} at Windows Mobile's",
                    l.asst,
                    l.cycle_cycles(),
                    high | m.mcmem(),
                    m.cycle_cycles(),
                    kib_s(3 * len, took_all),
                    kib_s(len, t_wm)
                );
            }
            None => log!("C7 no strobe shorter than Windows Mobile's wrote right"),
        }
        // C9 (R28 after C7: every faster step wrote at ~240 KiB/s against
        // ~4,700 at Windows Mobile's, the strobe step at Windows Mobile's
        // value too, so the tuned set-up and hold are the suspects): one
        // field at a time from Windows Mobile's timing, the others kept,
        // down to the card's read limit in that field; each step's speed
        // and sectors, a field stopped at its first wrong write. Results
        // only: the owner chooses the loader's write timing from them.
        let floor = Timing::from_mcmem(read_limit);
        for field in timing::FIELDS {
            let (name, value, low): (&str, fn(Timing) -> u32, u32) = match field {
                timing::Field::Strobe => ("strobe", |t| t.asst, floor.asst),
                timing::Field::Hold => ("hold", |t| t.hold, floor.hold),
                timing::Field::SetUp => ("set-up", |t| t.set, floor.set),
            };
            let mut step = wm_t;
            let mut best = None;
            while let Some(next) = timing::shorter(step, field).filter(|s| value(*s) >= low) {
                step = next;
                let (took, failed, bad) = round(cf, step, 300 + value(step));
                log!(
                    "C9 {name} {} ({} cycles), the rest Windows Mobile's: {} KiB/s, failed {failed}, {bad} sectors differ",
                    value(step),
                    step.cycle_cycles(),
                    kib_s(len, took)
                );
                if failed || bad != 0 || cf.is_dead() {
                    break;
                }
                best = Some((step, took));
            }
            match best {
                Some((b, took)) => log!(
                    "C9 {name}: the shortest right {} ({} cycles): {} KiB/s against {} at Windows Mobile's",
                    value(b),
                    b.cycle_cycles(),
                    kib_s(len, took),
                    kib_s(len, t_wm)
                ),
                None => log!("C9 {name}: no step shorter than Windows Mobile's wrote right"),
            }
            if cf.is_dead() {
                break;
            }
        }
        // C10 (R28 after C9: ~234 KiB/s whenever the data timing differs
        // from the command's, whatever it is): the same write with each
        // phase timed, at Windows Mobile's data timing (the register still
        // written, to its own value) and at strobes 9 and 1, twice each.
        // Where the ~2 ms a sector goes: the switches, the card's DRQ
        // waits, the data words, or its programming at the end.
        if !cf.is_dead() {
            for (label, data_t) in [
                ("Windows Mobile's", wm_t),
                ("strobe 9", Timing { asst: 9, ..wm_t }),
                ("strobe 1", Timing { asst: 1, ..wm_t }),
            ] {
                for r in 0..2u32 {
                    phases(cf, label, data_t, 400 + r, wm, slowest, lba, data, back);
                }
            }
        }
        if !cf.is_dead() {
            fresh_test(cf, start, wm, slowest, data, back, work);
        }
        cf.write_data_at(set_cf_timing, wm, wm, wm);
        remove_cf_file(cf, start, work);
    }

    /// C11's file, beside C7's: [`FRESH_REGIONS`] regions of [`CF_BLOCKS`].
    const FRESH_FILE: &str = "rp-cf-write/FRESH.BIN";
    const FRESH_REGIONS: u32 = 16;

    /// C11 (R28 after C10: every write waited ~132 ms for its first DRQ,
    /// at Windows Mobile's timing too, once the same sectors had been
    /// written before): which explains it, the card rewriting the same
    /// sectors again and again (A), or something kept after the first
    /// read-back at the slowest timing (B)? A file reserved (written once
    /// as zeros, as C7's first, fast, round wrote its sectors a second
    /// time), then: twelve regions written once each, the timings rotated
    /// (Windows Mobile's, strobe 9, strobe 1) with no read-back between;
    /// one region rewritten six times at Windows Mobile's (the rewrite
    /// cost); a read at the slowest timing, then a fresh region at Windows
    /// Mobile's (B); a fresh region at strobe 1 after all that; every
    /// region read back at the slowest timing; the file removed.
    fn fresh_test(
        cf: &mut Cf,
        start: u32,
        wm: u32,
        slowest: u32,
        data: &mut [u8],
        back: &mut [u8],
        work: &mut [u8],
    ) {
        let high = wm & !0xF_FFFF;
        let wm_t = Timing::WINDOWS_MOBILE;
        let timings = [
            ("Windows Mobile's", wm_t),
            ("strobe 9", Timing { asst: 9, ..wm_t }),
            ("strobe 1", Timing { asst: 1, ..wm_t }),
        ];
        set_cf_timing(wm);
        let base = {
            let Ok(mut vol) = Volume::mount(&mut *cf, start) else {
                log!("C11 mount failed: skipped");
                return;
            };
            match vol.reserve(
                FRESH_FILE,
                (FRESH_REGIONS * CF_BLOCKS) as usize * BLOCK,
                work,
            ) {
                Ok(r) => r.lba,
                Err(e) => {
                    log!("C11 reserve {FRESH_FILE}: {}: skipped", e.name());
                    return;
                }
            }
        };
        log!(
            "C11 {FRESH_FILE}: LBA {base}, {} sectors, contiguous, written once (zeros)",
            FRESH_REGIONS * CF_BLOCKS
        );
        let us = |t: u32| u64::from(t) * 1_000_000 / u64::from(rt::TICK_HZ);
        // One write of region `r` with seed `seed` at `t`: (ticks, the
        // longest DRQ wait, written).
        let mut write = |cf: &mut Cf, r: u32, t: Timing, seed: u32| -> (u32, u32, bool) {
            fill(data, seed, 0);
            cf.write_data_at(set_cf_timing, wm, wm, high | t.mcmem());
            let mut drq = 0u32;
            let t0 = rt::ticks();
            let mut last = t0;
            let ok = cf
                .write_blocks_marked(base + r * CF_BLOCKS, data, &mut |p| {
                    let now = rt::ticks();
                    if p == WritePhase::Drq {
                        drq = drq.max(now.wrapping_sub(last));
                    }
                    last = now;
                })
                .is_ok();
            let took = rt::ticks().wrapping_sub(t0);
            cf.write_data_at(set_cf_timing, wm, wm, wm);
            rt::check_in();
            (took, drq, ok)
        };
        let len = CF_BLOCKS as usize * BLOCK;
        // Each region's seed, for the read-back (0: not written by C11).
        let mut seeds = [0u32; FRESH_REGIONS as usize];
        let mut totals = [0u32; 3];
        for pass in 0..4u32 {
            for k in 0..3u32 {
                let which = ((k + pass) % 3) as usize;
                let (label, t) = timings[which];
                let r = pass * 3 + k;
                let seed = 500 + r;
                let (took, drq, ok) = write(cf, r, t, seed);
                seeds[r as usize] = seed;
                totals[which] = totals[which].wrapping_add(took);
                log!(
                    "C11 fresh region {r} at {label}: {} KiB/s, the longest DRQ wait {} us, ok {ok}",
                    kib_s(len, took),
                    us(drq)
                );
            }
        }
        for (k, (label, _)) in timings.iter().enumerate() {
            log!(
                "C11 fresh at {label}: {} KiB/s over 4 regions",
                kib_s(4 * len, totals[k])
            );
        }
        for n in 1..=6u32 {
            let seed = 600 + n;
            let (took, drq, ok) = write(cf, 12, wm_t, seed);
            seeds[12] = seed;
            log!(
                "C11 rewrite {n} of region 12 at Windows Mobile's: {} KiB/s, the longest DRQ wait {} us, ok {ok}",
                kib_s(len, took),
                us(drq)
            );
        }
        set_cf_timing(slowest);
        let _ = cf.read_blocks(base, &mut back[..len]);
        set_cf_timing(wm);
        let (took, drq, ok) = write(cf, 13, wm_t, 700);
        seeds[13] = 700;
        log!(
            "C11 after a read at the slowest timing, fresh region 13 at Windows Mobile's: {} KiB/s, the longest DRQ wait {} us, ok {ok}",
            kib_s(len, took),
            us(drq)
        );
        let (took, drq, ok) = write(cf, 14, timings[2].1, 701);
        seeds[14] = 701;
        log!(
            "C11 fresh region 14 at strobe 1, after the rewrites: {} KiB/s, the longest DRQ wait {} us, ok {ok}",
            kib_s(len, took),
            us(drq)
        );
        // Every region as written, read at the slowest timing.
        let mut bad = 0;
        let mut checked = 0;
        set_cf_timing(slowest);
        for (r, &seed) in seeds.iter().enumerate().filter(|(_, s)| **s != 0) {
            let back = &mut back[..len];
            back.fill(0);
            bad += if cf.read_blocks(base + r as u32 * CF_BLOCKS, back).is_ok() {
                differs(back, seed, 0)
            } else {
                CF_BLOCKS
            };
            checked += CF_BLOCKS;
        }
        set_cf_timing(wm);
        log!("C11 read back at the slowest timing: {bad} of {checked} sectors differ");
        let removed = Volume::mount(&mut *cf, start)
            .ok()
            .is_some_and(|mut vol| vol.remove(FRESH_FILE).is_ok());
        log!("C11 {FRESH_FILE} removed: {removed}");
    }

    /// One C10 write: `data` to `lba` with the data phase at `data_t` (the
    /// command at `wm`), each phase timed, read back at `slowest`.
    #[allow(clippy::too_many_arguments)]
    fn phases(
        cf: &mut Cf,
        label: &str,
        data_t: Timing,
        seed: u32,
        wm: u32,
        slowest: u32,
        lba: u32,
        data: &mut [u8],
        back: &mut [u8],
    ) {
        let high = wm & !0xF_FFFF;
        fill(data, seed, 0);
        cf.write_data_at(set_cf_timing, wm, wm, high | data_t.mcmem());
        // Ticks in: the command, the switch to the data timing, the DRQ
        // waits, the data words, the switch back, the finish; the longest
        // DRQ wait.
        let mut sums = [0u32; 7];
        let t0 = rt::ticks();
        let mut last = t0;
        let ok = cf
            .write_blocks_marked(lba, data, &mut |p| {
                let now = rt::ticks();
                let d = now.wrapping_sub(last);
                last = now;
                let k = match p {
                    WritePhase::Command => 0,
                    WritePhase::DataTiming => 1,
                    WritePhase::Drq => {
                        sums[6] = sums[6].max(d);
                        2
                    }
                    WritePhase::Data => 3,
                    WritePhase::CommandTiming => 4,
                    WritePhase::Done => 5,
                };
                sums[k] = sums[k].wrapping_add(d);
            })
            .is_ok();
        let took = rt::ticks().wrapping_sub(t0);
        set_cf_timing(slowest);
        back.fill(0);
        let read = cf.read_blocks(lba, back).is_ok();
        set_cf_timing(wm);
        rt::check_in();
        let bad = if read {
            differs(back, seed, 0)
        } else {
            CF_BLOCKS
        };
        let us = |t: u32| u64::from(t) * 1_000_000 / u64::from(rt::TICK_HZ);
        log!(
            "C10 data at {label}: {} KiB/s, ok {ok}, {bad} sectors differ; command {} us, to the data timing {} us, DRQ waits {} us (longest {}), data words {} us, back to the command timing {} us, finish {} us",
            kib_s(data.len(), took),
            us(sums[0]),
            us(sums[1]),
            us(sums[2]),
            us(sums[6]),
            us(sums[3]),
            us(sums[4]),
            us(sums[5])
        );
    }

    /// C7's file and folder off the CF card, at Windows Mobile's timing.
    fn remove_cf_file(cf: &mut Cf, start: u32, _work: &mut [u8]) {
        set_cf_timing(devices::CF_MEM_TIMING_WM);
        let Ok(mut vol) = Volume::mount(&mut *cf, start) else {
            log!("C7 mount failed: {CF_FILE} is left on the CF card");
            return;
        };
        for path in [CF_FILE, CF_DIR] {
            match vol.remove(path) {
                Ok(()) | Err(rp_fat::Error::NotFound) => {}
                Err(e) => log!("C7 remove {path}: {}", e.name()),
            }
        }
        log!("C7 {CF_DIR} removed from the CF card");
    }

    /// C8: the CPLD's interrupt line and status registers while the owner
    /// pulls the CF card and puts it back (R24, reads only).
    fn cpld_irq(screen: &mut Screen<'_>, gpio: &mut Gpio<Mmio>, cpld: &mut BoardCpld) {
        const REGS: [usize; 4] = [0x00, 0x04, 0x06, 0x14];
        gpio.make_input(CPLD_IRQ_GPIO);
        let read = |gpio: &mut Gpio<Mmio>, cpld: &mut BoardCpld| {
            (gpio.read(CPLD_IRQ_GPIO), REGS.map(|r| cpld.read(r)))
        };
        let secs = prompt_secs().min(40);
        screen.say(format_args!(
            "\nPULL THE CF CARD OUT, wait 2 seconds, PUT IT BACK ({secs} s)."
        ));
        let mut last = read(gpio, cpld);
        log!(
            "C8 at the start: GPIO13 {} 00={:04x} 04={:04x} 06={:04x} 14={:04x}",
            u8::from(last.0),
            last.1[0],
            last.1[1],
            last.1[2],
            last.1[3]
        );
        let t0 = rt::ticks();
        let mut changes = 0;
        while rt::ticks().wrapping_sub(t0) < secs * rt::TICK_HZ {
            rt::delay_us(1_000);
            rt::check_in();
            let now = read(gpio, cpld);
            if now != last {
                changes += 1;
                if changes <= 64 {
                    log!(
                        "C8 +{} ms: GPIO13 {} 00={:04x} 04={:04x} 06={:04x} 14={:04x}",
                        ms(rt::ticks().wrapping_sub(t0)),
                        u8::from(now.0),
                        now.1[0],
                        now.1[1],
                        now.1[2],
                        now.1[3]
                    );
                }
                last = now;
            }
        }
        log!("C8 done: {changes} changes in {secs} s");
        screen.say(format_args!("Done with the CF card."));
    }

    /// The image's put-back step: the socket at Windows Mobile's timing.
    fn put_back_cf_timing() {
        let _ = devices::set_cf_mem_timing(devices::CF_MEM_TIMING_WM);
    }

    /// The socket's timing register for `CfCard::tune`.
    struct Socket;

    impl SocketTiming for Socket {
        fn read(&mut self) -> u32 {
            devices::cf_mem_timing().unwrap_or(devices::CF_MEM_TIMING_WM)
        }
        fn write(&mut self, value: u32) {
            let _ = devices::set_cf_mem_timing(value);
        }
    }

    impl Socket {
        fn clk_ps(&mut self) -> u32 {
            // Rounded down: a cycle taken as shorter only lengthens the
            // timing computed from it.
            1_000_000_000 / clocks::read().point().mem_khz().max(1)
        }
    }

    /// Reads `buf.len()` bytes from LBA 0 by the card's path: the time,
    /// failed commands, and sectors unlike `reference`.
    fn read_all(cf: &mut Cf, buf: &mut [u8], reference: &[u8]) -> (u32, u32, usize) {
        buf.fill(0xA5);
        let t0 = rt::ticks();
        let mut failed = 0;
        for (i, run) in buf.chunks_mut(64 * 1024).enumerate() {
            rt::check_in();
            if cf.read_blocks((i * 128) as u32, run).is_err() {
                failed += 1;
            }
        }
        let t = rt::ticks().wrapping_sub(t0);
        let bad = buf
            .as_chunks::<512>()
            .0
            .iter()
            .zip(reference.as_chunks::<512>().0)
            .filter(|(a, b)| a != b)
            .count();
        (t, failed, bad)
    }

    /// C4: the card's own timing, measured by `tune` (reads only).
    fn own_timing(cf: &mut Cf, buf: &mut [u8], reference: &[u8]) -> Option<(u32, u32)> {
        let mut socket = Socket;
        let was = socket.read();
        let clk_ps = socket.clk_ps();
        log!(
            "C4 rated {} ns (CIS); memory clock {} ps; MCMEM1 {was:#010x} ({} cycles)",
            cf.rated_ns().unwrap_or(0),
            clk_ps,
            Timing::from_mcmem(was).cycle_cycles()
        );
        let proven = cf.read_path();
        let (t, failed, bad) = read_all(cf, buf, reference);
        let before = kib_s(buf.len(), t);
        log!(
            "C4 before, by {}: {before} KiB/s, {failed} failed, {bad} sectors differ",
            proven.name()
        );
        let t0 = rt::ticks();
        let tuned = cf.tune(&mut socket);
        let tune_ms = ms(rt::ticks().wrapping_sub(t0));
        let read_limit;
        let now = match tuned {
            Tuned::Applied {
                was,
                now,
                limit,
                clamped,
            } => {
                read_limit = limit;
                log!(
                    "C4 tune (measured, {tune_ms} ms): the card's limit {limit:#010x} ({} cycles, {} ns), with the margin {now:#010x} (clamped {clamped}); was {was:#010x}",
                    Timing::from_mcmem(limit).cycle_cycles(),
                    Timing::from_mcmem(limit).cycle_cycles() * clk_ps / 1000
                );
                now
            }
            Tuned::Refused {
                was,
                tried,
                card_ok,
            } => {
                log!(
                    "C4 tune (measured, {tune_ms} ms): Refused, {tried:#010x} read differently; {was:#010x} kept, card reads as before {card_ok}"
                );
                return None;
            }
            Tuned::Kept(why) => {
                log!("C4 tune (measured, {tune_ms} ms): Kept ({why})");
                return None;
            }
        };
        log!("C4 now {} cycles", Timing::from_mcmem(now).cycle_cycles());
        for path in [ReadPath::Port16, ReadPath::Window32, ReadPath::Dma] {
            if !cf.set_read_path(path) {
                log!("C4 {}: not proven on this card, skipped", path.name());
                continue;
            }
            for pass in 1..=3 {
                let (t, failed, bad) = read_all(cf, buf, reference);
                log!(
                    "C4 own timing, {} pass {pass}: {} KiB/s ({before} before), {failed} failed, {bad} sectors differ",
                    path.name(),
                    kib_s(buf.len(), t)
                );
            }
        }
        let _ = cf.set_read_path(proven);
        Some((now, read_limit))
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
    fn contiguous<B: BlockDevice>(
        card: &mut B,
        start: u32,
        first: u32,
        blocks: u32,
        work: &mut [u8],
    ) -> Option<u32> {
        let bpb = &mut work[..BLOCK];
        card.read_blocks(start, bpb).ok()?;
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
                card.read_blocks(s, fat).ok()?;
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

    /// One swap: the owner takes the card out (doing `what` to it, if
    /// anything) and puts it back; the card is set up again. `None` (the
    /// test stops, files stay) if no card came back or it won't start.
    fn reinsert(
        screen: &mut Screen<'_>,
        gpio: &mut Gpio<Mmio>,
        card: Card,
        what: &str,
    ) -> Option<Card> {
        screen.say(format_args!("\nTAKE THE SD CARD OUT{what}."));
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
        Some(card)
    }

    /// S3: the card taken out and put back, then locked and unlocked. The
    /// lock tab can't be reached with the card in the X51v's slot (the first
    /// device run asked for that, 2026-10-16), so each lock change is a swap.
    fn swap_and_lock(
        screen: &mut Screen<'_>,
        gpio: &mut Gpio<Mmio>,
        card: Card,
        lba: u32,
        pool: &mut [u8],
        recent: &mut Recent,
    ) -> Option<Card> {
        let mut card = reinsert(screen, gpio, card, " now")?;
        let (checked, bad) = recent.verify(&mut card, lba, pool);
        log!("S3 after the swap: {checked} of the last writes read back, {bad} blocks differ");
        soak(&mut card, lba, pool, 1, recent);
        let mut card = reinsert(screen, gpio, card, ", SLIDE ITS LOCK ON (locked)")?;
        if gpio.read(SD_LOCK) {
            log!("S3 lock on (GPIO27 high)");
            // Locked: reads only (the lock is the owner's: nothing writes).
            let (checked, bad) = recent.verify(&mut card, lba, pool);
            log!("S3 while locked, {checked} of the last writes read by DMA: {bad} blocks differ");
        } else {
            log!("S3 no lock seen (GPIO27 low with the card back): lock check skipped");
        }
        let mut card = reinsert(screen, gpio, card, ", SLIDE ITS LOCK OFF")?;
        if gpio.read(SD_LOCK) {
            screen.say(format_args!("Still locked: stopping (files stay)."));
            log!("S3 still locked: the test files are left in {DIR}");
            return None;
        }
        log!("S3 lock off");
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
}

#[cfg(not(target_os = "none"))]
fn main() {}
