//! Game Mode's measurements from the emulator (design B, docs/design/game-mode-b.md,
//! "Measurements B needs before any code", item 2: LZ4's ratio on real heap
//! pages).
//!
//! `cargo xtask emu-heap-ratio` boots the Today screen under the loader,
//! uses it as a person would for a while (the same taps as `emu-today-steps`),
//! then saves all 64 MiB of the guest's SDRAM with QEMU's `pmemsave` and
//! prices every 4 KiB page the way the kernel's fast tier would store it:
//! the kernel's own LZ4 (`rp_lz4::compress`, the codec `Vm::compress` runs)
//! and the compressed store's own size classes (`rp_kernel_mm::zpool`: the
//! compressed bytes plus a 2-byte length, rounded up to 64-byte granules;
//! anything over `MAX_OBJECT` stays resident). The bytes are the image's own
//! (same code, same card, same taps as the device): compression is a
//! function of the bytes alone, so the ratio is the device's for this
//! session. What the emulator can't give: a session with the owner's real
//! weather and his own apps' data (a device run's dump: reminder R364).
//!
//! Rule 0 (written before building, CLAUDE.md): Linux's zram and zswap and
//! iOS's compressor are measured on servers and phones with gigabytes of
//! mixed pages, and the common figures (2-3x for zram on Android) are not
//! this OS's heap; Windows Mobile has no compressor at all. The best way
//! today: price this OS's own pages with its own codec and store, from a
//! session that ran its real code, and split out the pages a better rule
//! would not compress at all (all-zero pages: unmapped back to demand-zero,
//! 0 bytes; pages of one repeated word: a page-table entry's worth, as
//! zram's `ZRAM_SAME` and zswap's same-filled pages do). The numbers decide
//! whether the freeze needs those rules (it does if they are common).

#[allow(clippy::wildcard_imports)]
use super::*;

/// A page.
const PAGE: usize = 4096;

/// How the fast tier would hold the pages of one stretch of RAM.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) struct PageCosts {
    /// Pages looked at.
    pub pages: u64,
    /// Pages that are all zero: unmapped to demand-zero, 0 bytes kept.
    pub zero: u64,
    /// Pages of one repeated 32-bit word other than zero.
    pub same: u64,
    /// Pages the store takes (compressed small enough).
    pub stored: u64,
    /// Pages that stay resident (compressed over `MAX_OBJECT`).
    pub resident: u64,
    /// Bytes the store's slots take for `stored` pages.
    pub slot_bytes: u64,
    /// LZ4's output bytes for `stored` pages (before the slot rounding).
    pub lz4_bytes: u64,
    /// Bytes kept in all if zero and same pages went through the store too
    /// (today's `Vm::compress`: every page compressed, none special-cased).
    pub naive_bytes: u64,
}

impl PageCosts {
    /// Bytes kept with the zero and same rules: the store's slots, the
    /// resident pages whole, a word per same-filled page.
    pub fn kept_bytes(&self) -> u64 {
        self.slot_bytes + self.resident * PAGE as u64 + self.same * 4
    }

    /// Pages that would be frozen (not zero).
    pub fn nonzero(&self) -> u64 {
        self.pages - self.zero
    }

    fn add(&mut self, o: &PageCosts) {
        self.pages += o.pages;
        self.zero += o.zero;
        self.same += o.same;
        self.stored += o.stored;
        self.resident += o.resident;
        self.slot_bytes += o.slot_bytes;
        self.lz4_bytes += o.lz4_bytes;
        self.naive_bytes += o.naive_bytes;
    }
}

/// The store's slot for `n` compressed bytes, or `None` if it would refuse
/// them (the page stays resident): `rp_kernel_mm::zpool::ZPool::store`'s rule.
pub(crate) fn slot_for(n: usize) -> Option<usize> {
    use rp_kernel_mm::zpool::{GRANULE, MAX_OBJECT};
    (n <= MAX_OBJECT).then(|| (n + 2).div_ceil(GRANULE).max(1) * GRANULE)
}

/// One page of one repeated word: that word.
pub(crate) fn same_word(page: &[u8]) -> Option<u32> {
    let first = u32::from_le_bytes(page.get(..4)?.try_into().ok()?);
    page.as_chunks::<4>()
        .0
        .iter()
        .all(|w| *w == first.to_le_bytes())
        .then_some(first)
}

/// Prices one page.
pub(crate) fn price(page: &[u8], out: &mut [u8]) -> PageCosts {
    let mut c = PageCosts {
        pages: 1,
        ..PageCosts::default()
    };
    let n = rp_lz4::compress(page, out).ok();
    let naive = n.and_then(slot_for).unwrap_or(PAGE) as u64;
    match same_word(page) {
        Some(0) => {
            c.zero = 1;
            c.naive_bytes = naive;
            return c;
        }
        Some(_) => {
            c.same = 1;
            c.naive_bytes = naive;
            return c;
        }
        None => {}
    }
    match n.and_then(|n| slot_for(n).map(|s| (n, s))) {
        Some((n, s)) => {
            c.stored = 1;
            c.slot_bytes = s as u64;
            c.lz4_bytes = n as u64;
        }
        None => c.resident = 1,
    }
    c.naive_bytes = naive;
    c
}

/// Prices every whole page of `ram`, per `band` bytes (a row per band) and
/// in all.
pub(crate) fn price_ram(ram: &[u8], band: usize) -> (Vec<PageCosts>, PageCosts) {
    let mut out = vec![0u8; rp_lz4::max_compressed(PAGE)];
    let mut bands = Vec::new();
    let mut all = PageCosts::default();
    for chunk in ram.chunks(band) {
        let mut b = PageCosts::default();
        for page in chunk.as_chunks::<PAGE>().0 {
            b.add(&price(page, &mut out));
        }
        all.add(&b);
        bands.push(b);
    }
    (bands, all)
}

/// The report's lines for `ram` (based at `base`).
pub(crate) fn report(ram: &[u8], base: u32) -> Vec<String> {
    const BAND: usize = 1 << 20;
    let (bands, all) = price_ram(ram, BAND);
    let mib = |b: u64| b as f64 / f64::from(1u32 << 20);
    let mut lines = vec![format!(
        "heap-ratio: {} pages: {} zero, {} one-word, {} stored, {} stay resident",
        all.pages, all.zero, all.same, all.stored, all.resident
    )];
    let nz = all.nonzero();
    if nz > 0 {
        let held = mib(nz * PAGE as u64);
        let kept = mib(all.kept_bytes());
        lines.push(format!(
            "heap-ratio: non-zero pages {held:.2} MiB kept as {kept:.2} MiB ({:.2}x; LZ4 alone {:.2}x on the stored pages)",
            held / kept.max(f64::MIN_POSITIVE),
            (all.stored * PAGE as u64) as f64 / (all.lz4_bytes.max(1)) as f64,
        ));
        lines.push(format!(
            "heap-ratio: every page through the store as today (zero pages too): {:.2} MiB; with the zero and one-word rules {:.2} MiB",
            mib(all.naive_bytes),
            kept,
        ));
    }
    for (i, b) in bands.iter().enumerate() {
        if b.nonzero() == 0 {
            continue;
        }
        let at = u64::from(base) + (i * BAND) as u64;
        lines.push(format!(
            "heap-ratio: 0x{at:08X} {:4} live pages -> {:7} bytes ({:.2}x), {} resident, {} one-word",
            b.nonzero(),
            b.kept_bytes(),
            (b.nonzero() * PAGE as u64) as f64 / b.kept_bytes().max(1) as f64,
            b.resident,
            b.same,
        ));
    }
    lines
}

/// The guest's SDRAM: the X51v's is on SDCS2 (`MDCNFG`, docs/hw/emulator.md),
/// at 0xA8000000, not the PXA270's first bank.
const SDRAM: u32 = 0xA800_0000;
/// Its size on the X51v.
const SDRAM_LEN: u32 = 64 << 20;

/// `cargo xtask emu-heap-ratio`: the Today screen used for a while, its RAM
/// saved and priced (see the module's words). Writes the report to
/// `target/emulator/heap-ratio.txt` and prints it.
pub fn heap_ratio(root: &Path) -> Result<(), String> {
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-today.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-today"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let sd = today_card(root, &release)?;
    let wsl = matches!(emulator(root)?, Emulator::Wsl);
    let dump = scratch(root, "heap-ratio-ram.bin")?;
    let _ = std::fs::remove_file(&dump);
    let steps = || -> Result<(), String> {
        let hold = |ms: u32| {
            guest_hold(
                &mut crate::screen::Monitor::connect(monitor_port())?,
                ms,
                "heap-ratio",
            )
        };
        let mut qmp = crate::screen::Qmp::connect(monitor_port() + 1)?;
        hold(8000)?;
        // The weather page, its place menu, the hourly and now views and
        // home again: every page the shell has built at least once.
        let taps: [Option<(u16, u16)>; 6] = [
            Some((240, 150)),
            Some((180, 580)),
            Some((60, 580)),
            None,
            Some((240, 150)),
            None,
        ];
        for at in taps {
            match at {
                Some((x, y)) => {
                    touch(&mut qmp, &Action::TouchDown(x, y))?;
                    hold(150)?;
                    touch(&mut qmp, &Action::TouchUp)?;
                }
                None => {
                    qmp.key("f4", true)?;
                    hold(150)?;
                    qmp.key("f4", false)?;
                }
            }
            hold(2500)?;
        }
        // QEMU's monitor reads a quoted path's backslashes as escapes.
        let guest = if wsl {
            to_wsl_path(&dump)
        } else {
            dump.display().to_string().replace('\\', "/")
        };
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.query("stop")?;
        mon.query(&format!(
            "pmemsave 0x{SDRAM:08X} 0x{SDRAM_LEN:X} \"{guest}\""
        ))?;
        let deadline = Instant::now() + Duration::from_secs(60);
        while std::fs::metadata(&dump).map_or(0, |m| m.len()) < u64::from(SDRAM_LEN) {
            if Instant::now() > deadline {
                return Err(format!("no RAM dump at {}", dump.display()));
            }
            poll_pause(Duration::from_millis(200));
        }
        mon.query("cont")?;
        Ok(())
    };
    let _ = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: sd.as_deref(),
            steps: &[],
            extra: &["-icount", "shift=0"],
            input: Some((&rp_loadproto::frame(&payload), rp_today::MARKER)),
            during: Some(&steps),
            then: None,
            timeout: Duration::from_secs(600),
        },
    );
    let ram = std::fs::read(&dump).map_err(|e| format!("{}: {e}", dump.display()))?;
    if ram.len() < SDRAM_LEN as usize {
        return Err(format!("RAM dump {} bytes, not 64 MiB", ram.len()));
    }
    let lines = report(&ram, SDRAM);
    let out = scratch(root, "heap-ratio.txt")?;
    std::fs::write(&out, lines.join("\n") + "\n").map_err(|e| format!("{}: {e}", out.display()))?;
    for l in &lines {
        println!("    {l}");
    }
    let _ = std::fs::remove_file(&dump);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_follow_the_store() {
        // The store's own edges: 1 byte is one granule, 62 still one,
        // 63 two (the 2-byte length), MAX_OBJECT the last it takes.
        use rp_kernel_mm::zpool::{GRANULE, MAX_OBJECT};
        assert_eq!(slot_for(1), Some(GRANULE));
        assert_eq!(slot_for(GRANULE - 2), Some(GRANULE));
        assert_eq!(slot_for(GRANULE - 1), Some(2 * GRANULE));
        assert!(slot_for(MAX_OBJECT).is_some());
        assert_eq!(slot_for(MAX_OBJECT + 1), None);
    }

    #[test]
    fn zero_same_and_random_pages_priced() {
        let mut out = vec![0u8; rp_lz4::max_compressed(PAGE)];
        let zero = vec![0u8; PAGE];
        let c = price(&zero, &mut out);
        assert_eq!((c.zero, c.same, c.stored, c.resident), (1, 0, 0, 0));
        assert_eq!(c.kept_bytes(), 0);
        assert!(c.naive_bytes > 0, "today's path stores even a zero page");

        let same: Vec<u8> = (0..PAGE).map(|i| [0xEF, 0xBE, 0xAD, 0xDE][i % 4]).collect();
        let c = price(&same, &mut out);
        assert_eq!((c.zero, c.same), (0, 1));
        assert_eq!(c.kept_bytes(), 4);

        // A page differing in one byte is not one word.
        let mut nearly = same.clone();
        nearly[PAGE - 1] ^= 1;
        assert_eq!(same_word(&nearly), None);
        let c = price(&nearly, &mut out);
        assert_eq!((c.same, c.stored), (0, 1));

        // Noise doesn't compress: it stays resident, whole.
        let mut x = 0x1234_5678u32;
        let noise: Vec<u8> = (0..PAGE)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect();
        let c = price(&noise, &mut out);
        assert_eq!((c.stored, c.resident), (0, 1));
        assert_eq!(c.kept_bytes(), PAGE as u64);
    }

    #[test]
    fn bands_add_up_and_skip_partial_pages() {
        let mut ram = vec![0u8; 3 * PAGE + 100];
        ram[PAGE..2 * PAGE].copy_from_slice(&[7u8; PAGE]);
        for (i, b) in ram[2 * PAGE..3 * PAGE].iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        let (bands, all) = price_ram(&ram, 2 * PAGE);
        assert_eq!(bands.len(), 2);
        assert_eq!(all.pages, 3, "the 100-byte tail isn't a page");
        assert_eq!((all.zero, all.same, all.stored), (1, 1, 1));
        assert_eq!(bands[0].pages + bands[1].pages, all.pages);
        let lines = report(&ram, SDRAM);
        assert!(
            lines[0].contains("3 pages: 1 zero, 1 one-word, 1 stored"),
            "{lines:?}"
        );
    }
}
