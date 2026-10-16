//! Report-only test image (docs/research/storage-speed.md): how fast is the
//! SD card on the X51v, and where does the time go?
//!
//! - **S1 registers:** CID, CSD (deselect, CMD9/CMD10, select again), SCR
//!   (ACMD51), SD status (ACMD13: speed class, AU size), the CMD6 function
//!   query (mode 0: asks, switches nothing).
//! - **S2 reads** by the driver (`read_many`), PIO and DMA: 4 KiB, 64 KiB,
//!   1 MiB commands, 4 MiB each way, plus single blocks (CMD17); then one
//!   raw CMD18 split into its phases (command, data, CMD12).
//! - **S3 clock codes:** 64 blocks at `CLKRT` 0 (19.5 MHz, the documented
//!   top) and at the reserved code 7, compared and timed; reads only.
//! - **S0 the test file:** `SDTIME.BIN` (2 MiB of a fixed pattern) is
//!   created through the FAT layer if missing (timed: a FAT write), and
//!   must be contiguous.
//! - **S4 writes**, only inside `SDTIME.BIN`, and only after every one of
//!   its blocks matched the pattern: the same bytes are written back, so a cut leaves
//!   the file as it was. 4 KiB, 64 KiB and 1 MiB commands, PIO and DMA,
//!   with and without ACMD23 (pre-erase); each split into the data phase,
//!   CMD12 (the card's busy) and the CMD13 polls after. Single-block
//!   writes too. Then the whole file is read back against the pattern.
//! - **S5 the FAT layer:** a 1 MiB file put and removed (`SDFAT.TMP`), and
//!   `SDTIME.BIN` read through `read_file`.
//!
//! Everything it writes is inside `/rp-storage-bench/`, which it removes
//! at the end (file, then the empty folder); nothing else on the card is
//! written, replaced or deleted. The loader resets the card when this
//! returns.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed with the verdict.
pub const MARKER: &str = "RETROPANE-SDTIME";

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_cpld::Cpld;
    use rp_drv_pxa_mmc::PxaMmc;
    use rp_drv_sdcard::{BLOCK, Command, Response, SdCard, SdHost, mbr_partitions};
    use rp_fat::Volume;
    use rp_hal_pxa27x::{Mmio, dma::Dma, rt};
    use rp_hal_x51v::devices::{self, RunLog};

    use super::MARKER;

    rp_hal_pxa27x::entry!(main);

    macro_rules! log {
        ($($t:tt)*) => {{ let _ = writeln!(RunLog, $($t)*); }};
    }

    type Host = PxaMmc<Mmio, Dma>;
    type Card = SdCard<Host>;

    /// The test file and its size (blocks).
    /// The image's own folder: it writes and deletes nothing outside it
    /// (the card holds the owner's and other sessions' files).
    const DIR: &str = "rp-storage-bench";
    const FILE: &str = "rp-storage-bench/SDTIME.BIN";
    const FILE_BLOCKS: u32 = 4096;
    /// The FAT test's file.
    const FAT_FILE: &str = "rp-storage-bench/SDFAT.TMP";
    /// Bytes each speed measurement moves.
    const TOTAL: usize = 4 << 20;
    /// Bytes each write measurement moves.
    const WRITE_TOTAL: usize = 1 << 20;
    /// CMD13 polls before a write counts as stuck.
    const READY_POLLS: u32 = 200_000;

    /// SDTIME.BIN's word `w` of block `b`.
    fn pattern(b: u32, w: u32) -> u32 {
        b.wrapping_mul(0x9E37_79B1) ^ w.wrapping_mul(0x85EB_CA6B) ^ 0x5D7E_5A1E
    }

    fn ns(ticks: u32) -> u64 {
        u64::from(ticks) * 4000 / 13
    }

    fn kib_s(bytes: usize, ticks: u32) -> u64 {
        (bytes as u64 * u64::from(rt::TICK_HZ)) / (u64::from(ticks.max(1)) * 1024)
    }

    fn checksum(data: &[u8]) -> u32 {
        data.as_chunks::<4>().0.iter().fold(0x811C_9DC5, |h, w| {
            (h ^ u32::from_le_bytes(*w)).wrapping_mul(0x0100_0193)
        })
    }

    fn main() -> ! {
        rt::set_fault_handler(on_fault);
        rt::init_mmu();
        log!("{MARKER} 1: SD speed, report only (docs/research/storage-speed.md)");
        run();
        log!("{MARKER}: done");
        devices::return_to_loader();
        loop {
            core::hint::spin_loop();
        }
    }

    fn run() {
        let Some(mut cpld) = devices::take_cpld().map(Cpld::new) else {
            log!("no CPLD");
            return;
        };
        cpld.set_backlight_enable(true);
        cpld.set_sd_power(true);
        rt::delay_us(10_000);
        let (Some(mmc), Some(dma)) = (rt::take_mmc(), rt::take_dma(0)) else {
            log!("no MMC or DMA");
            return;
        };
        let mut card = match SdCard::try_init(PxaMmc::with_dma(mmc, dma)) {
            Ok(c) => c,
            Err((e, _)) => {
                log!("S0 card init failed: {}", e.name());
                return;
            }
        };
        log!(
            "S0 card: {} blocks, high capacity {}, 4-bit {}, rca {:#06x}",
            card.blocks(),
            card.is_high_capacity(),
            card.is_wide(),
            card.rca()
        );
        let Some(scratch) = devices::take_scratch() else {
            log!("no scratch");
            return;
        };
        registers(&mut card);
        let start = match card_partition(&mut card, scratch) {
            Some(s) => s,
            None => return,
        };
        let Some(lba) = test_file(&mut card, start, scratch) else {
            return;
        };
        reads(&mut card, lba, scratch);
        clock_codes(&mut card, lba, scratch);
        writes(&mut card, lba, scratch);
        fat_layer(&mut card, start, scratch);
        clean_up(&mut card, start);
    }

    /// Removes the test file and the image's folder, nothing else.
    fn clean_up(card: &mut Card, start: u32) {
        let Ok(mut vol) = Volume::mount(&mut *card, start) else {
            log!("S6 clean-up: mount failed; {DIR} left on the card");
            return;
        };
        let file = vol.remove(FILE);
        let dir = vol.remove(DIR);
        log!(
            "S6 clean-up: {FILE} {}, {DIR} {}",
            file.map_or_else(|e| e.name(), |()| "removed"),
            dir.map_or_else(|e| e.name(), |()| "removed")
        );
    }

    fn addr(card: &Card, lba: u32) -> u32 {
        if card.is_high_capacity() {
            lba
        } else {
            lba * BLOCK as u32
        }
    }

    fn hex(tag: &str, data: &[u8]) {
        for (k, row) in data.chunks(32).enumerate() {
            let mut line = Line::new();
            for b in row {
                let _ = write!(line, "{b:02x}");
            }
            log!("{tag} {:02x}: {}", k * 32, line.as_str());
        }
    }

    /// S1.
    fn registers(card: &mut Card) {
        let rca = u32::from(card.rca()) << 16;
        let h = card.host_mut();
        let deselect = h.command(Command::new(7, 0, Response::None));
        let csd = h.command(Command::new(9, rca, Response::R2));
        let cid = h.command(Command::new(10, rca, Response::R2));
        let select = h.command(Command::new(7, rca, Response::R1b));
        log!(
            "S1 deselect {:?} select {:?}",
            deselect.is_ok(),
            select.is_ok()
        );
        match csd {
            Ok(r) => log!("S1 CSD {:08x}{:08x}{:08x}{:08x}", r[0], r[1], r[2], r[3]),
            Err(e) => log!("S1 CSD failed {e:?}"),
        }
        match cid {
            Ok(r) => log!("S1 CID {:08x}{:08x}{:08x}{:08x}", r[0], r[1], r[2], r[3]),
            Err(e) => log!("S1 CID failed {e:?}"),
        }
        let mut scr = [0u8; 8];
        let r = h
            .command(Command::new(55, rca, Response::R1))
            .and_then(|_| h.read_short(Command::new(51, 0, Response::R1), &mut scr));
        match r {
            Ok(_) => hex("S1 SCR", &scr),
            Err(e) => log!("S1 SCR failed {e:?}"),
        }
        let mut status = [0u8; 64];
        let r = h
            .command(Command::new(55, rca, Response::R1))
            .and_then(|_| h.read_short(Command::new(13, 0, Response::R1), &mut status));
        match r {
            Ok(_) => hex("S1 SD status", &status),
            Err(e) => log!("S1 SD status failed {e:?}"),
        }
        let mut f = [0u8; 64];
        match h.read_short(Command::new(6, 0x00FF_FFFF, Response::R1), &mut f) {
            Ok(_) => hex("S1 CMD6 query", &f),
            Err(e) => log!("S1 CMD6 query failed {e:?}"),
        }
        // One CMD13: what a busy poll costs.
        let t0 = rt::ticks();
        for _ in 0..100 {
            let _ = h.command(Command::new(13, rca, Response::R1));
        }
        log!(
            "S1 CMD13 costs {} ns",
            ns(rt::ticks().wrapping_sub(t0)) / 100
        );
    }

    fn card_partition(card: &mut Card, scratch: &mut [u8]) -> Option<u32> {
        let block: &mut [u8; BLOCK] = (&mut scratch[..BLOCK]).try_into().ok()?;
        if card.read(0, block).is_err() {
            log!("S0 block 0 unreadable");
            return None;
        }
        let start = mbr_partitions(block)
            .and_then(|p| p[0])
            .map_or(0, |p| p.start);
        log!("S0 partition 1 at {start}");
        Some(start)
    }

    /// Finds SDTIME.BIN, checks it is contiguous and holds the pattern
    /// throughout; its first block's LBA.
    fn test_file(card: &mut Card, start: u32, scratch: &mut [u8]) -> Option<u32> {
        let file = {
            let mut vol = match Volume::mount(&mut *card, start) {
                Ok(v) => v,
                Err(e) => {
                    log!("S0 not FAT32 ({}): no write or FAT tests", e.name());
                    return None;
                }
            };
            match vol.find(FILE) {
                Ok(f) => f,
                Err(rp_fat::Error::NotFound) => {
                    // Made here: the loader's `put` takes under 1 MiB.
                    let (data, rest) = scratch.split_at_mut(FILE_BLOCKS as usize * BLOCK);
                    fill(data, 0);
                    rt::check_in();
                    let t0 = rt::ticks();
                    let put = vol.put(FILE, data, &mut rest[..64 * 1024]);
                    let t = rt::ticks().wrapping_sub(t0);
                    log!(
                        "S0 {FILE} made through the FAT layer: {} KiB/s ({} us), {}",
                        kib_s(data.len(), t),
                        ns(t) / 1000,
                        put.as_ref().map_or_else(|e| e.name(), |_| "ok")
                    );
                    if put.is_err() {
                        return None;
                    }
                    match vol.find(FILE) {
                        Ok(f) => f,
                        Err(e) => {
                            log!("S0 {FILE} after put: {}", e.name());
                            return None;
                        }
                    }
                }
                Err(e) => {
                    log!("S0 {FILE}: {}", e.name());
                    return None;
                }
            }
        };
        if file.size() != FILE_BLOCKS * BLOCK as u32 || file.first_cluster() < 2 {
            log!(
                "S0 {FILE} is {} bytes, not {}",
                file.size(),
                FILE_BLOCKS * 512
            );
            return None;
        }
        // The boot sector's geometry (FAT32, 512-byte sectors).
        let bpb = &mut scratch[..BLOCK];
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
        let first = file.first_cluster();
        let clusters = FILE_BLOCKS.div_ceil(spc);
        // Contiguous: each cluster's FAT entry names the next.
        let mut sector = u32::MAX;
        let fat = &mut scratch[BLOCK..2 * BLOCK];
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
            let want_end = k + 1 == clusters;
            if (want_end && next < 0x0FFF_FFF8) || (!want_end && next != c + 1) {
                log!("S0 {FILE} isn't contiguous (cluster {c} -> {next:#x}): no write tests");
                return None;
            }
        }
        let lba = data_start + (first - 2) * spc;
        log!(
            "S0 {FILE}: cluster {first}, {spc} sectors a cluster, LBA {lba}..{}",
            lba + FILE_BLOCKS
        );
        let bad = verify(card, lba, scratch);
        if bad != 0 {
            log!("S0 {FILE}: {bad} blocks don't hold the pattern: no write tests");
            return None;
        }
        log!("S0 {FILE}: every block holds the pattern");
        Some(lba)
    }

    /// Blocks of the test file that don't hold the pattern.
    fn verify(card: &mut Card, lba: u32, scratch: &mut [u8]) -> u32 {
        let chunk = 128u32;
        let mut bad = 0;
        for c in 0..FILE_BLOCKS / chunk {
            rt::check_in();
            let buf = &mut scratch[..chunk as usize * BLOCK];
            if card.read_many(lba + c * chunk, buf).is_err() {
                bad += chunk;
                continue;
            }
            for (k, block) in buf.as_chunks::<BLOCK>().0.iter().enumerate() {
                let b = c * chunk + k as u32;
                let ok = block
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .enumerate()
                    .all(|(w, v)| u32::from_le_bytes(*v) == pattern(b, w as u32));
                if !ok {
                    bad += 1;
                }
            }
        }
        bad
    }

    /// S2.
    fn reads(card: &mut Card, lba: u32, scratch: &mut [u8]) {
        for dma in [false, true] {
            card.host_mut().set_dma(dma, false);
            for blocks in [8usize, 128, 2048] {
                let bytes = blocks * BLOCK;
                let n = TOTAL / bytes;
                let before = card.host().dma_transfers();
                let t0 = rt::ticks();
                let mut failed = 0;
                for i in 0..n {
                    rt::check_in();
                    let at = lba + ((i * blocks) as u32 % FILE_BLOCKS);
                    if card.read_many(at, &mut scratch[..bytes]).is_err() {
                        failed += 1;
                    }
                }
                let t = rt::ticks().wrapping_sub(t0);
                log!(
                    "S2 read {} KiB x{n} dma {dma}: {} KiB/s, {} us a command, {} by DMA, {failed} failed",
                    bytes / 1024,
                    kib_s(TOTAL, t),
                    ns(t) / 1000 / n as u64,
                    card.host().dma_transfers() - before
                );
            }
        }
        card.host_mut().set_dma(false, false);
        let t0 = rt::ticks();
        for i in 0..256u32 {
            let block: &mut [u8; BLOCK] = (&mut scratch[..BLOCK]).try_into().expect("a block");
            let _ = card.read(lba + i, block);
        }
        let t = rt::ticks().wrapping_sub(t0);
        log!(
            "S2 read single blocks x256: {} KiB/s, {} us each",
            kib_s(256 * BLOCK, t),
            ns(t) / 1000 / 256
        );
        // One raw CMD18 of 128 blocks by phase, PIO and DMA.
        for dma in [false, true] {
            card.host_mut().set_dma(dma, false);
            let a = addr(card, lba);
            let h = card.host_mut();
            let t0 = rt::ticks();
            let r = h.read_blocks(
                Command::new(18, a, Response::R1),
                &mut scratch[..128 * BLOCK],
            );
            let t1 = rt::ticks();
            let stop = h.command(Command::new(12, 0, Response::R1b));
            let t2 = rt::ticks();
            log!(
                "S2 raw CMD18 128 blocks dma {dma}: command+data {} us ({} ns a block; the 4-bit bus at 19.5 MHz needs 52.5 us a block), CMD12 {} us, ok {} {}",
                ns(t1.wrapping_sub(t0)) / 1000,
                ns(t1.wrapping_sub(t0)) / 128,
                ns(t2.wrapping_sub(t1)) / 1000,
                r.is_ok(),
                stop.is_ok()
            );
        }
        card.host_mut().set_dma(false, false);
        // Command start-up: CMD18 of 1 block vs 8.
        for blocks in [1usize, 2, 8] {
            let a = addr(card, lba);
            let h = card.host_mut();
            let t0 = rt::ticks();
            let r = h.read_blocks(
                Command::new(18, a, Response::R1),
                &mut scratch[..blocks * BLOCK],
            );
            let _ = h.command(Command::new(12, 0, Response::R1b));
            log!(
                "S2 raw CMD18 {blocks} blocks + CMD12: {} us, ok {}",
                ns(rt::ticks().wrapping_sub(t0)) / 1000,
                r.is_ok()
            );
        }
    }

    /// S3: the documented top clock code and the reserved one, reads only.
    fn clock_codes(card: &mut Card, lba: u32, scratch: &mut [u8]) {
        card.host_mut().set_dma(false, false);
        let mut want = 0;
        for code in [0u32, 7, 0] {
            card.host_mut().set_clkrt_raw(code);
            let a = addr(card, lba);
            let h = card.host_mut();
            let buf = &mut scratch[..64 * BLOCK];
            buf.fill(0);
            let t0 = rt::ticks();
            let r = h.read_blocks(Command::new(18, a, Response::R1), buf);
            let t1 = rt::ticks();
            let stop = h.command(Command::new(12, 0, Response::R1b));
            let sum = checksum(buf);
            if code == 0 && want == 0 {
                want = sum;
            }
            log!(
                "S3 CLKRT {code}: 64 blocks in {} us ({} KiB/s), read {:?}, stop {}, data {}",
                ns(t1.wrapping_sub(t0)) / 1000,
                kib_s(64 * BLOCK, t1.wrapping_sub(t0)),
                r.err(),
                stop.is_ok(),
                if sum == want { "same" } else { "DIFFERENT" }
            );
            if r.is_err() || stop.is_err() {
                // Back to a known state: the card's status, then on.
                let rca = u32::from(card.rca()) << 16;
                let _ = card.host_mut().command(Command::new(13, rca, Response::R1));
            }
        }
        card.host_mut().set_clock(rp_drv_sdcard::TRANSFER_HZ);
    }

    /// Fills `buf` with the pattern of the test file's blocks from `first`.
    fn fill(buf: &mut [u8], first: u32) {
        for (k, block) in buf.as_chunks_mut::<BLOCK>().0.iter_mut().enumerate() {
            for (w, v) in block.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                *v = pattern(first + k as u32, w as u32).to_le_bytes();
            }
        }
    }

    #[derive(Default)]
    struct Phases {
        data: u32,
        stop: u32,
        ready: u32,
        polls: u32,
        failed: u32,
        dma: u32,
    }

    /// One raw multi-block write of the pattern back over itself.
    fn write_raw(
        card: &mut Card,
        lba: u32,
        at: u32,
        buf: &mut [u8],
        pre_erase: bool,
        p: &mut Phases,
    ) {
        let blocks = (buf.len() / BLOCK) as u32;
        fill(buf, at);
        let rca = u32::from(card.rca()) << 16;
        let a = addr(card, lba + at);
        let before = card.host().dma_transfers();
        let h = card.host_mut();
        if pre_erase {
            let ok = h
                .command(Command::new(55, rca, Response::R1))
                .and_then(|_| h.command(Command::new(23, blocks, Response::R1)))
                .is_ok();
            if !ok {
                p.failed += 1;
            }
        }
        let t0 = rt::ticks();
        let r = h.write_blocks(Command::new(25, a, Response::R1), buf);
        let t1 = rt::ticks();
        let stop = h.command(Command::new(12, 0, Response::R1b));
        let t2 = rt::ticks();
        let mut polls = 0;
        loop {
            polls += 1;
            match h.command(Command::new(13, rca, Response::R1)) {
                Ok(s) if s[0] & (1 << 8) != 0 && (s[0] >> 9) & 0xF != 7 => break,
                _ if polls >= READY_POLLS => {
                    p.failed += 1;
                    break;
                }
                _ => {}
            }
        }
        let t3 = rt::ticks();
        if r.is_err() || stop.is_err() {
            p.failed += 1;
        }
        p.data = p.data.wrapping_add(t1.wrapping_sub(t0));
        p.stop = p.stop.wrapping_add(t2.wrapping_sub(t1));
        p.ready = p.ready.wrapping_add(t3.wrapping_sub(t2));
        p.polls += polls;
        p.dma += card.host().dma_transfers() - before;
    }

    /// S4.
    fn writes(card: &mut Card, lba: u32, scratch: &mut [u8]) {
        for dma in [false, true] {
            for pre_erase in [false, true] {
                card.host_mut().set_dma(false, dma);
                for blocks in [8usize, 128, 2048] {
                    let bytes = blocks * BLOCK;
                    let n = WRITE_TOTAL / bytes;
                    let mut p = Phases::default();
                    let t0 = rt::ticks();
                    for i in 0..n {
                        rt::check_in();
                        let at = (i * blocks) as u32 % FILE_BLOCKS;
                        write_raw(card, lba, at, &mut scratch[..bytes], pre_erase, &mut p);
                    }
                    let t = rt::ticks().wrapping_sub(t0);
                    log!(
                        "S4 write {} KiB x{n} dma {dma} ACMD23 {pre_erase}: {} KiB/s; data {} us, CMD12 (busy) {} us, CMD13 {} us in {} polls; {} by DMA, {} failed",
                        bytes / 1024,
                        kib_s(WRITE_TOTAL, t),
                        ns(p.data) / 1000,
                        ns(p.stop) / 1000,
                        ns(p.ready) / 1000,
                        p.polls,
                        p.dma,
                        p.failed
                    );
                }
            }
        }
        card.host_mut().set_dma(false, false);
        // Single-block writes (CMD24), 64 of them.
        let t0 = rt::ticks();
        let mut failed = 0;
        for i in 0..64u32 {
            rt::check_in();
            let block: &mut [u8; BLOCK] = (&mut scratch[..BLOCK]).try_into().expect("a block");
            fill(block, i);
            if card.write(lba + i, block).is_err() {
                failed += 1;
            }
        }
        let t = rt::ticks().wrapping_sub(t0);
        log!(
            "S4 write single blocks x64: {} KiB/s, {} us each, {failed} failed",
            kib_s(64 * BLOCK, t),
            ns(t) / 1000 / 64
        );
        // The driver's own path (ACMD23 on, PIO writes), 64 KiB x16.
        let t0 = rt::ticks();
        let mut failed = 0;
        for i in 0..16u32 {
            rt::check_in();
            let buf = &mut scratch[..128 * BLOCK];
            fill(buf, i * 128);
            if card.write_many(lba + i * 128, buf).is_err() {
                failed += 1;
            }
        }
        let t = rt::ticks().wrapping_sub(t0);
        log!(
            "S4 driver write_many 64 KiB x16: {} KiB/s, {failed} failed",
            kib_s(WRITE_TOTAL, t)
        );
        let bad = verify(card, lba, scratch);
        log!("S4 read back: {bad} blocks of {FILE_BLOCKS} differ from the pattern");
    }

    /// S5.
    fn fat_layer(card: &mut Card, start: u32, scratch: &mut [u8]) {
        let (data, rest) = scratch.split_at_mut(1 << 20);
        let (work, _) = rest.split_at_mut(64 * 1024);
        fill(data, 0);
        let mut vol = match Volume::mount(&mut *card, start) {
            Ok(v) => v,
            Err(e) => {
                log!("S5 mount: {}", e.name());
                return;
            }
        };
        rt::check_in();
        let t0 = rt::ticks();
        let put = vol.put(FAT_FILE, data, work);
        let t1 = rt::ticks();
        rt::check_in();
        let removed = vol.remove(FAT_FILE);
        let t2 = rt::ticks();
        log!(
            "S5 put {FAT_FILE} 1 MiB: {} KiB/s ({} us), {}; remove {} us, {}",
            kib_s(1 << 20, t1.wrapping_sub(t0)),
            ns(t1.wrapping_sub(t0)) / 1000,
            put.map_or_else(|e| e.name(), |_| "ok"),
            ns(t2.wrapping_sub(t1)) / 1000,
            removed.map_or_else(|e| e.name(), |()| "ok")
        );
        let file = match vol.find(FILE) {
            Ok(f) => f,
            Err(e) => {
                log!("S5 {FILE}: {}", e.name());
                return;
            }
        };
        for (name, len) in [("4 KiB", 4096usize), ("64 KiB", 64 * 1024)] {
            rt::check_in();
            let mut sum = 0u32;
            let t0 = rt::ticks();
            let r = vol.read_file(&file, &mut work[..len], |piece| {
                sum = sum.wrapping_add(piece.len() as u32);
            });
            let t = rt::ticks().wrapping_sub(t0);
            log!(
                "S5 read_file {FILE} ({name} scratch): {} KiB/s, {} bytes, {}",
                kib_s(sum as usize, t),
                sum,
                r.map_or_else(|e| e.name(), |()| "ok")
            );
        }
    }

    /// A short log line built in place.
    struct Line {
        buf: [u8; 96],
        len: usize,
    }

    impl Line {
        fn new() -> Self {
            Self {
                buf: [0; 96],
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
