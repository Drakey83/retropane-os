//! RetroPane's resident loader (ADR-0012 step U3).
//!
//! Started once by HaRET, it moves itself to its reserved home at
//! `0xA8400000` and waits for an image from the PC, over the USB serial port
//! (the device) or the FFUART (the emulator, and an optional cable). A
//! complete, CRC-checked image goes to `0xA8008000` and starts exactly as if
//! HaRET had started it. So a new build reaches the device in about a second,
//! with nobody copying files or tapping HaRET.
//!
//! A file frame instead replaces a file on the SD card (ADR-0013): the
//! loader itself, HaRET, its script. Copy-then-swap in `rp-fat`, then the
//! contents are read back and checked against the frame's CRC before the
//! loader says OK, so what the PC sent is what the card holds.
//!
//! Meanwhile the SD card is a drive on the PC (ADR-0014): a USB mass
//! storage interface next to the serial port. The two never write the card
//! at once: a file frame is refused while the PC is writing, and after one
//! the PC is told the medium changed, so it re-reads the volume.
//!
//! Protocol: `rp-loadproto`. PC side: `cargo xtask load <image>`,
//! `cargo xtask put <file>`.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed on serial once the loader is waiting (also the READY line).
pub const MARKER: &str = "RPLOAD1 READY";

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_cf::CfCard;
    use rp_drv_cpld::Cpld;
    use rp_drv_mbx_display::console::Console;
    use rp_drv_mbx_display::crash;
    use rp_drv_mbx_display::{Display, PixelFormat, Plane, X51V_VGA, reg};
    use rp_drv_pxa_gpio::Gpio;
    use rp_drv_pxa_i2c::PxaI2c;
    use rp_drv_pxa_i2s::{PxaI2s, Rate, Tone, stream};
    use rp_drv_pxa_mmc::PxaMmc;
    use rp_drv_pxa_pwm::Backlight;
    use rp_drv_pxa_uart::PxaUart;
    use rp_drv_pxa_udc::Udc;
    use rp_drv_sdcard::{SdCard, mbr_partitions};
    use rp_drv_wm8750::{Output, Wm8750};
    use rp_fat::{AnyVolume, Volume};
    use rp_hal_api::{BlockDevice, BlockError, RegisterBus};
    use rp_hal_pxa27x::dma::Dma;
    use rp_hal_pxa27x::{Mmio, handoff, rt};
    use rp_hal_x51v::MBX_VRAM_SIZE;
    use rp_hal_x51v::devices::{self, IMAGE_BASE};
    use rp_loadproto::{Frame, Kind, LoadError, READY, Receiver, crc32_update, feed_slice};
    use rp_usb_msc::{BulkPipe, Msc, Storage};

    rp_hal_pxa27x::entry!(main);

    /// D+ pull-up and cable detect (docs/hw/udc.md).
    const USB_PULLUP: u8 = 2;
    const USB_DETECT: u8 = 14;
    /// How long the USB pull-up stays off at start (OS timer ticks, 200 ms),
    /// so the PC sees the device leave and come back.
    const DETACH: u32 = 650_000;
    /// OS timer ticks (3.25 MHz) between READY lines and status redraws.
    const TICK_1S: u32 = 3_250_000;
    const STATUS_H: usize = 6 * 16;
    /// Quiet time (no input, no drive commands, nothing to send) before the
    /// loader sleeps between polls: 50 ms.
    const QUIET: u32 = TICK_1S / 20;
    /// The crash screen's height (docs: crash screen), when the previous
    /// program crashed or hung.
    const CRASH_H: usize = 320;
    /// How long a loaded image may run without returning or feeding the
    /// watchdog (the probe, the slowest, takes about 15 s on the device).
    const RUN_LIMIT: u32 = 30 * TICK_1S;
    /// The end of the image area, kept for searching the FAT for free
    /// clusters and for reading a written file back (the bigger it is, the
    /// fewer card commands each takes).
    const SCRATCH: usize = 64 * 1024;

    /// The SD card.
    type Sd = SdCard<PxaMmc<Mmio, Dma>>;

    /// Staging buffer for the drive's block transfers (in the scratch
    /// area, not the image: 64 sectors per card command).
    const MSC_BUF: usize = 32 * 1024;
    /// The drive's serial number: the USB serial string (16 hex digits).
    const MSC_SERIAL: &[u8] = b"5250323630303031";
    /// How long the PC may try to enumerate a layout without configuring it
    /// before the loader offers the other one (composite <-> serial only).
    const LAYOUT_PATIENCE: u32 = 25 * TICK_1S;

    /// The SD card as the drive's logical unit 0 (absent when no card
    /// answered at startup). Read-only to the PC until it asks otherwise
    /// (`cargo xtask drive rw`): the card holds the loader and HaRET, and a
    /// PC writes a mounted drive on its own (docs/security.md).
    struct Lun {
        card: Option<Sd>,
        /// The controller, kept when no card initialised, for a card
        /// inserted later.
        spare: Option<PxaMmc<Mmio, Dma>>,
        writable: bool,
        /// Offered to the PC (`drive off` makes it "no medium", like an
        /// empty card reader: the PC stops using it at once).
        offered: bool,
        /// A card is in the slot (GPIO12 low, docs/hw/sd.md).
        inserted: bool,
        /// The card's lock switch is on (GPIO27 high).
        locked: bool,
        /// The slot's last reading, for debouncing: acted on only when two
        /// readings a second apart agree.
        seen: (bool, bool),
    }

    /// The power button (GPIO0, docs/hw/README.md).
    const POWER_BUTTON: u8 = 0;
    /// A press must hold this long (OS timer ticks: 30 ms) to count.
    const DEBOUNCE: u32 = 3_250_000 / 1000 * 30;

    /// The power button: a debounced press is a change away from the level
    /// the button rested at when the loader started, so its polarity needn't
    /// be known (unconfirmed on the device). A press turns the screen off,
    /// the next one back on at the level it had, as phones do.
    struct PowerButton {
        rest: bool,
        down: bool,
        since: u32,
        saved_level: Option<u32>,
    }

    impl PowerButton {
        fn new(gpio: &mut Gpio<Mmio>) -> Self {
            gpio.make_input(POWER_BUTTON);
            Self {
                rest: gpio.read(POWER_BUTTON),
                down: false,
                since: rt::ticks(),
                saved_level: None,
            }
        }

        /// Returns `true` once per press (when it has held for [`DEBOUNCE`]).
        fn pressed(&mut self, gpio: &mut Gpio<Mmio>) -> bool {
            let down = gpio.read(POWER_BUTTON) != self.rest;
            if down != self.down {
                self.down = down;
                self.since = rt::ticks();
                return false;
            }
            if down && self.since != 0 && rt::ticks().wrapping_sub(self.since) >= DEBOUNCE {
                self.since = 0; // reported; wait for the release
                return true;
            }
            false
        }

        /// Screen off (remembering the level) or back on.
        fn toggle(&mut self, bl: &mut Backlight<Mmio>) -> bool {
            match self.saved_level.take() {
                Some(level) => {
                    bl.fade_to(level, &mut |us| rt::delay_us(us));
                    true
                }
                None => {
                    self.saved_level = Some(bl.level());
                    bl.fade_to(0, &mut |us| rt::delay_us(us));
                    false
                }
            }
        }
    }

    /// SD slot switches (measured on the owner's X51v, docs/hw/sd.md).
    const SD_DETECT: u8 = 12;
    /// CompactFlash card detect (docs/hw/cf.md): low = a card in the slot.
    const CF_DETECT: u8 = 13;
    /// Headphone jack (docs/hw/audio.md): high = plugged in (assumed;
    /// unconfirmed on the device, so changes are logged with the level).
    const HP_DETECT: u8 = 83;

    /// The headphone jack, debounced over two one-second polls.
    struct Jack {
        plugged: bool,
        seen: bool,
    }

    impl Jack {
        /// The new state when the jack changed.
        fn poll(&mut self, gpio: &mut Gpio<Mmio>) -> Option<bool> {
            let now = gpio.read(HP_DETECT);
            let stable = now == self.seen;
            self.seen = now;
            (stable && now != self.plugged).then(|| {
                self.plugged = now;
                now
            })
        }

        fn output(&self) -> Output {
            if self.plugged {
                Output::Headphones
            } else {
                Output::Speaker
            }
        }
    }
    const SD_LOCK: u8 = 27;

    impl Lun {
        /// Reads the slot's switches once a second; a change that holds for
        /// two readings is acted on: a card taken out is no longer offered,
        /// a card put in is initialised (hot swap). Returns whether the PC
        /// should be told the medium changed.
        #[inline(never)]
        fn poll_slot(&mut self, gpio: &mut Gpio<Mmio>) -> bool {
            let now = (!gpio.read(SD_DETECT), gpio.read(SD_LOCK));
            let stable = now == self.seen;
            self.seen = now;
            if !stable || (now.0 == self.inserted && now.1 == self.locked) {
                return false;
            }
            let (inserted, locked) = now;
            if inserted && !self.inserted {
                // A (new) card: initialise it on the controller we have.
                let host = match self.card.take() {
                    Some(old) => Some(old.into_inner()),
                    None => self.spare.take(),
                };
                if let Some(host) = host {
                    match SdCard::try_init(host) {
                        Ok(card) => self.card = Some(card),
                        Err((_, host)) => self.spare = Some(host),
                    }
                }
            }
            self.inserted = inserted;
            self.locked = locked;
            true
        }
    }

    impl BlockDevice for Lun {
        fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), BlockError> {
            self.card
                .as_mut()
                .ok_or(BlockError::Device)?
                .read_blocks(lba, buf)
        }
        fn write_blocks(&mut self, lba: u32, buf: &[u8]) -> Result<(), BlockError> {
            if !self.writable || self.locked {
                return Err(BlockError::ReadOnly);
            }
            self.card
                .as_mut()
                .ok_or(BlockError::Device)?
                .write_blocks(lba, buf)
        }
    }

    impl Storage for Lun {
        fn blocks(&self) -> u32 {
            self.card.as_ref().map_or(0, SdCard::blocks)
        }
        fn present(&self) -> bool {
            self.offered && self.inserted && self.card.as_ref().is_some_and(|c| c.blocks() > 0)
        }
        fn read_only(&self) -> bool {
            !self.writable || self.locked
        }
        fn product(&self) -> &[u8; 16] {
            b"SD card         "
        }
    }

    /// The UDC's mass storage endpoints as the drive's bulk pipe.
    struct Pipe<'a>(&'a mut Udc<Mmio>);

    impl BulkPipe for Pipe<'_> {
        fn read_packet(&mut self, buf: &mut [u8; 64]) -> Option<usize> {
            self.0.msc_read(buf)
        }
        fn can_write(&mut self) -> bool {
            self.0.msc_can_write()
        }
        fn write_packet(&mut self, data: &[u8]) {
            self.0.msc_write(data);
        }
    }

    fn now_ms() -> u32 {
        rt::ticks() / (TICK_1S / 1000)
    }

    /// A reply line built on the stack.
    struct Line {
        buf: [u8; 256],
        len: usize,
    }

    impl Line {
        const fn new() -> Self {
            Self {
                buf: [0; 256],
                len: 0,
            }
        }
        fn as_str(&self) -> &str {
            core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
        }
    }

    impl Write for Line {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let n = s.len().min(self.buf.len() - self.len);
            self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
            self.len += n;
            Ok(())
        }
    }

    /// Powers the card and identifies it.
    #[inline(never)]
    fn open_card() -> (Option<Sd>, Option<PxaMmc<Mmio, Dma>>) {
        let (Some(cpld), Some(mmc), Some(dma)) =
            (devices::take_cpld(), rt::take_mmc(), rt::take_dma(0))
        else {
            return (None, None);
        };
        Cpld::new(cpld).set_sd_power(true);
        rt::delay_us(10_000);
        match SdCard::try_init(PxaMmc::with_dma(mmc, dma)) {
            Ok(card) => (Some(card), None),
            Err((_, host)) => (None, Some(host)),
        }
    }

    /// Where the card's first partition starts (0: no partition table).
    fn partition_start(sd: &mut Sd) -> Result<u32, &'static str> {
        let mut block = [0u8; rp_drv_sdcard::BLOCK];
        sd.read(0, &mut block).map_err(|_| "card read")?;
        Ok(mbr_partitions(&block)
            .and_then(|p| p[0])
            .map_or(0, |p| p.start))
    }

    /// Mounts the card's first partition afresh: the PC may have written
    /// it (as a drive) since the last time, so nothing cached is trusted.
    fn mount(sd: &mut Sd) -> Result<Volume<&mut Sd>, &'static str> {
        let start = partition_start(sd)?;
        Volume::mount(sd, start).map_err(|_| "not FAT32")
    }

    /// Mounts the first partition as FAT32 or exFAT, for storing files.
    fn mount_any(sd: &mut Sd) -> Result<AnyVolume<&mut Sd>, &'static str> {
        let start = partition_start(sd)?;
        AnyVolume::mount(sd, start).map_err(|_| "not FAT32 or exFAT")
    }

    /// Writes a file frame to the card and checks it back; the reply line.
    #[inline(never)]
    fn put_file(lun: &mut Lun, msc: &mut Msc<'_>, frame: Frame, dest: &mut [u8], reply: &mut Line) {
        let (payload, scratch) = dest.split_at_mut(dest.len() - SCRATCH);
        if frame.len > payload.len() {
            let _ = reply.write_str(LoadError::TooBig.reply());
            return;
        }
        let (name, contents) = match frame.file(payload) {
            Ok(x) => x,
            Err(e) => {
                let _ = reply.write_str(e.reply());
                return;
            }
        };
        // The lock switch is the owner's: nothing writes a locked card.
        if lun.locked {
            let _ = reply.write_str("RPL ERR card is write-protected (lock switch)\r\n");
            return;
        }
        // Never write the card while the PC (as a drive) is writing it.
        if msc.host_writing(now_ms()) {
            let _ = reply.write_str("RPL ERR card busy: the PC is writing\r\n");
            return;
        }
        let Some(sd) = lun.card.as_mut() else {
            let _ = reply.write_str("RPL ERR card: no card\r\n");
            return;
        };
        let mut vol = match mount_any(sd) {
            Ok(v) => v,
            Err(why) => {
                let _ = write!(reply, "RPL ERR card: {why}\r\n");
                return;
            }
        };
        // Whatever happens next, the PC must re-read the volume.
        msc.medium_changed(0);
        let t0 = rt::ticks();
        let reads0 = vol.device().reads();
        let written = match vol.put(name, contents, scratch) {
            Ok(w) => w,
            Err(e) => {
                let _ = write!(reply, "RPL ERR {}{}\r\n", e.name(), card_note(vol.device()));
                return;
            }
        };
        let t_write = rt::ticks().wrapping_sub(t0);
        let reads = vol.device().reads().wrapping_sub(reads0);
        // Read back through the FAT code and compare with what was sent;
        // the SHA-256 of what the card now holds goes back to the PC.
        let mut crc = !0u32;
        let mut got = 0usize;
        let mut sha = rp_crypto::Sha256::new();
        let check = vol.read_back(name, scratch, &mut |chunk| {
            crc = crc32_update(crc, chunk);
            sha.update(chunk);
            got += chunk.len();
        });
        let digest = rp_crypto::hex(&sha.finish());
        let want = crc32_update(!0, contents);
        match check {
            Ok(()) if crc == want && got == contents.len() => {
                let _ = write!(
                    reply,
                    "RPL OK {} bytes in {} ms, {} cluster{} {} ({} reads), read back in {} ms, sha256 {}\r\n",
                    contents.len(),
                    t_write / (TICK_1S / 1000),
                    written.clusters,
                    if written.clusters == 1 { "" } else { "s" },
                    if written.contiguous {
                        "contiguous"
                    } else {
                        "scattered"
                    },
                    reads,
                    rt::ticks().wrapping_sub(t0 + t_write) / (TICK_1S / 1000),
                    core::str::from_utf8(&digest).unwrap_or(""),
                );
            }
            Ok(()) => {
                let _ = write!(
                    reply,
                    "RPL ERR verify: {got} bytes read back, crc differs\r\n"
                );
            }
            Err(e) => {
                let _ = write!(
                    reply,
                    "RPL ERR verify: {}{}\r\n",
                    e.name(),
                    card_note(vol.device())
                );
            }
        }
    }

    /// `card scan <lba> <count>` (at most 4096 blocks): reads the range one
    /// block at a time and reports failures, the first failing block with
    /// the card's error, and the slowest read: finds slow or unreadable
    /// areas without the PC's own retries in the way.
    #[inline(never)]
    /// `feed` keeps the watchdog fed as the scan goes: a scan of a slow
    /// area can outlast the run limit (it did on the owner's card: a reset
    /// into Dell's factory prompt).
    fn card_scan(lun: &mut Lun, args: &[u8], out: &mut Line, feed: &mut dyn FnMut()) {
        card_scan_inner(lun, args, out, feed);
    }

    /// `card repair <lba>`: restores a damaged first-FAT sector from its
    /// mirror in the second FAT, as a disk check would, but only when the
    /// evidence says the first copy is the broken one: the FATs are
    /// mirrored (so the second is current, not stale; `fat_mirror` checks),
    /// every entry in the mirror is valid (free, end of chain, bad, or a
    /// real cluster), and at least 90% of the entries where the copies
    /// differ are invalid in the first FAT: random damage, which can land a
    /// few values in range by chance (the owner's card: one in ~110), where
    /// a stale mirror would differ in valid entries. Written, read back and
    /// compared.
    #[inline(never)]
    fn card_repair(lun: &mut Lun, args: &[u8], out: &mut Line) {
        let lba = parse_number(args);
        let (Some(lba), Some(sd)) = (lba, lun.card.as_mut()) else {
            let _ = out.write_str(LoadError::BadCommand.reply());
            return;
        };
        let (mirror, clusters) = match mount(sd) {
            Ok(vol) => (vol.fat_mirror(lba), vol.clusters()),
            Err(why) => {
                let _ = write!(out, "RPL ERR repair: {why}\r\n");
                return;
            }
        };
        let Some(mirror) = mirror else {
            let _ =
                out.write_str("RPL ERR repair: only first-FAT sectors (they have a mirror)\r\n");
            return;
        };
        let mut damaged = [0u8; rp_drv_sdcard::BLOCK];
        let mut good = [0u8; rp_drv_sdcard::BLOCK];
        let t = rt::ticks();
        if sd.read(lba, &mut damaged).is_err() {
            let _ = write!(
                out,
                "RPL ERR repair: the block didn't read{}\r\n",
                card_note(sd)
            );
            return;
        }
        let before = rt::ticks().wrapping_sub(t) / 3;
        if sd.read(mirror, &mut good).is_err() {
            let _ = write!(out, "RPL ERR repair: mirror {mirror} unreadable\r\n");
            return;
        }
        if damaged == good {
            let _ = write!(
                out,
                "RPL OK repair {lba}: already matches mirror {mirror}; nothing written\r\n"
            );
            return;
        }
        let entry = |s: &[u8; rp_drv_sdcard::BLOCK], i: usize| {
            u32::from_le_bytes([s[4 * i], s[4 * i + 1], s[4 * i + 2], s[4 * i + 3]]) & 0x0FFF_FFFF
        };
        let valid = |v: u32| v == 0 || v >= 0x0FFF_FFF7 || (2..2 + clusters).contains(&v);
        let (mut differing, mut invalid) = (0u32, 0u32);
        for i in 0..rp_drv_sdcard::BLOCK / 4 {
            if !valid(entry(&good, i)) {
                let _ = write!(
                    out,
                    "RPL ERR repair: the mirror's entry {i} is invalid too: not written\r\n"
                );
                return;
            }
            if entry(&damaged, i) != entry(&good, i) {
                differing += 1;
                invalid += u32::from(!valid(entry(&damaged, i)));
            }
        }
        if invalid * 10 < differing * 9 {
            let _ = write!(
                out,
                "RPL ERR repair: only {invalid} of {differing} differing entries are invalid: a stale mirror, not damage? not written\r\n"
            );
            return;
        }
        if sd.write(lba, &good).is_err() {
            let _ = write!(out, "RPL ERR repair: write failed{}\r\n", card_note(sd));
            return;
        }
        let t = rt::ticks();
        let ok = sd.read(lba, &mut damaged).is_ok() && damaged == good;
        let after = rt::ticks().wrapping_sub(t) / 3;
        if ok {
            let _ = write!(
                out,
                "RPL OK repair {lba}: {differing} damaged entries ({invalid} invalid) restored from mirror {mirror}; read {before} us before, {after} us after\r\n"
            );
        } else {
            let _ = write!(
                out,
                "RPL ERR repair: read-back differs (mirror {mirror} still good)\r\n"
            );
        }
    }

    /// How SD DMA is used: reads once the start-up self-test has shown DMA
    /// reads match programmed I/O on this hardware; writes only once the
    /// owner turns them on (`card dma writes`), until a device run has
    /// proven them (a verified `put`). Nothing unproven touches the card.
    struct DmaPolicy {
        /// The self-test's verdict.
        verdict: &'static str,
    }

    /// DMA self-test: the first blocks read by DMA (into a buffer filled
    /// with a pattern first, so a DMA that writes nothing shows) and one by
    /// one by programmed I/O must match. DMA stays off for the session if
    /// they don't, or if there is no card to test with.
    #[inline(never)]
    fn check_dma(lun: &mut Lun, buf: &mut [u8]) -> DmaPolicy {
        const BLOCKS: usize = 8;
        let Some(sd) = lun.card.as_mut() else {
            if let Some(host) = lun.spare.as_mut() {
                host.set_dma(false, false);
            }
            return DmaPolicy {
                verdict: "off (no card to test with)",
            };
        };
        sd.host_mut().set_dma(true, false);
        let many = &mut buf[..BLOCKS * rp_drv_sdcard::BLOCK];
        many.fill(0xA5);
        let before = sd.host().dma_transfers();
        let mut one = [0u8; rp_drv_sdcard::BLOCK];
        let ok = sd.read_many(0, many).is_ok()
            && sd.host().dma_transfers() != before
            && many
                .chunks_exact(rp_drv_sdcard::BLOCK)
                .enumerate()
                .all(|(lba, want)| sd.read(lba as u32, &mut one).is_ok() && one[..] == *want);
        if ok {
            DmaPolicy {
                verdict: "on (self-test passed)",
            }
        } else {
            sd.host_mut().set_dma(false, false);
            DmaPolicy {
                verdict: "off (self-test: DMA reads differed)",
            }
        }
    }

    /// The path a command names (spaces inside it kept; "" is the root).
    fn arg_path(args: &[u8]) -> Option<&str> {
        core::str::from_utf8(args.trim_ascii()).ok()
    }

    /// `ls [folder]`: one `RPD` line per entry (`d` for a folder, else its
    /// size in bytes, then the name), then the reply. Read-only.
    #[inline(never)]
    fn ls_command(lun: &mut Lun, args: &[u8], out: &mut dyn FnMut(&[u8]), reply: &mut Line) {
        let Some(path) = arg_path(args) else {
            let _ = reply.write_str("RPL ERR bad name\r\n");
            return;
        };
        let Some(sd) = lun.card.as_mut() else {
            let _ = reply.write_str("RPL ERR card: no card\r\n");
            return;
        };
        let mut vol = match mount_any(sd) {
            Ok(v) => v,
            Err(why) => {
                let _ = write!(reply, "RPL ERR card: {why}\r\n");
                return;
            }
        };
        let mut n = 0u32;
        let r = vol.list(path, &mut |name, dir, size| {
            let mut l = Line::new();
            let _ = l.write_str("RPD ");
            if dir {
                let _ = l.write_str("d ");
            } else {
                let _ = write!(l, "{size} ");
            }
            for &b in name {
                let _ = l.write_char(if b.is_ascii_graphic() || b == b' ' {
                    char::from(b)
                } else {
                    '?'
                });
            }
            let _ = l.write_str("\r\n");
            out(l.as_str().as_bytes());
            n += 1;
        });
        match r {
            Ok(()) => {
                let _ = write!(reply, "RPL OK {n} entries\r\n");
            }
            Err(e) => {
                let _ = write!(reply, "RPL ERR {}\r\n", e.name());
            }
        }
    }

    /// What boots the device from the card (HaRET, its scripts, this
    /// loader): `rm` won't take them, so a slip can't brick the card.
    const BOOT_FILES: [&str; 4] = ["retropane.bin", "haret.exe", "startup.txt", "default.txt"];

    /// `rm <path>`: deletes a file or an empty folder (crash-safe), never a
    /// boot file, never on a locked card.
    #[inline(never)]
    fn rm_command(lun: &mut Lun, args: &[u8], reply: &mut Line) {
        let Some(path) = arg_path(args).filter(|p| !p.is_empty()) else {
            let _ = reply.write_str("RPL ERR usage: rm <file or empty folder>\r\n");
            return;
        };
        let leaf = path.trim_start_matches(['/', '\\']);
        if BOOT_FILES.iter().any(|b| leaf.eq_ignore_ascii_case(b)) {
            let _ = reply.write_str("RPL ERR rm: that file boots the device\r\n");
            return;
        }
        if lun.locked {
            let _ = reply.write_str("RPL ERR card is write-protected (lock switch)\r\n");
            return;
        }
        let Some(sd) = lun.card.as_mut() else {
            let _ = reply.write_str("RPL ERR card: no card\r\n");
            return;
        };
        let mut vol = match mount_any(sd) {
            Ok(v) => v,
            Err(why) => {
                let _ = write!(reply, "RPL ERR card: {why}\r\n");
                return;
            }
        };
        match vol.remove(path) {
            Ok(()) => {
                let _ = write!(reply, "RPL OK removed {path}\r\n");
            }
            Err(e) => {
                let _ = write!(reply, "RPL ERR rm: {}\r\n", e.name());
            }
        }
    }

    /// `card check`: a read-only check of the card's file system (FAT32 or
    /// exFAT): what it found, counted. `seen` holds a bit per cluster.
    #[inline(never)]
    fn card_check(lun: &mut Lun, seen: &mut [u8], scratch: &mut [u8], reply: &mut Line) {
        let Some(sd) = lun.card.as_mut() else {
            let _ = reply.write_str("RPL ERR card: no card\r\n");
            return;
        };
        let mut vol = match mount_any(sd) {
            Ok(v) => v,
            Err(why) => {
                let _ = write!(reply, "RPL ERR card: {why}\r\n");
                return;
            }
        };
        match vol.check(seen, scratch) {
            Ok(h) => {
                let _ = write!(
                    reply,
                    "RPL OK check {}: {} files, {} folders, {} clusters used, {} lost (cross-linked {}, bad chains {}, size mismatches {}, free but used {}, bad sets {}, FAT copies differ {}, unchecked {})\r\n",
                    if h.clean() { "clean" } else { "PROBLEMS" },
                    h.files,
                    h.folders,
                    h.used,
                    h.lost,
                    h.cross_linked,
                    h.bad_chains,
                    h.size_mismatch,
                    h.free_but_used,
                    h.bad_sets,
                    h.fat_copies_differ,
                    h.unchecked
                );
            }
            Err(e) => {
                let _ = write!(reply, "RPL ERR check: {}\r\n", e.name());
            }
        }
    }

    /// `get <file>`: the file as base64 `RPD` lines, then `RPL OK <bytes>
    /// bytes crc <crc32>` so the PC checks what it got. Read-only.
    #[inline(never)]
    fn get_command(
        lun: &mut Lun,
        args: &[u8],
        scratch: &mut [u8],
        out: &mut dyn FnMut(&[u8]),
        reply: &mut Line,
    ) {
        let Some(path) = arg_path(args).filter(|p| !p.is_empty()) else {
            let _ = reply.write_str("RPL ERR usage: get <file>\r\n");
            return;
        };
        let Some(sd) = lun.card.as_mut() else {
            let _ = reply.write_str("RPL ERR card: no card\r\n");
            return;
        };
        let mut vol = match mount_any(sd) {
            Ok(v) => v,
            Err(why) => {
                let _ = write!(reply, "RPL ERR card: {why}\r\n");
                return;
            }
        };
        let mut enc = rp_loadproto::b64::Encoder::new();
        let mut crc = !0u32;
        let mut total = 0u64;
        let mut line = |l: &[u8]| {
            out(b"RPD ");
            out(l);
            out(b"\r\n");
        };
        let r = vol.read_back(path, scratch, &mut |piece| {
            crc = crc32_update(crc, piece);
            total += piece.len() as u64;
            enc.feed(piece, &mut line);
        });
        enc.finish(&mut line);
        match r {
            Ok(()) => {
                let _ = write!(reply, "RPL OK {total} bytes crc {:08x}\r\n", !crc);
            }
            Err(e) => {
                let _ = write!(reply, "RPL ERR {}\r\n", e.name());
            }
        }
    }

    /// The CompactFlash slot: not looked at until the owner asks (`cf`), so
    /// nothing touches the socket unasked; then the card, or why not.
    enum CfSlot {
        Untried,
        Card(CfCard<Mmio, Mmio>),
        /// Taken out after it was attached: the socket's windows, kept for
        /// the next `cf`.
        Removed(Mmio, Mmio),
        Failed(&'static str),
    }

    /// The CompactFlash card as the drive's second unit (LUN 1): an empty
    /// card reader until the owner attaches the card (`cf`), read-only
    /// until `drive rw`, as the SD card is.
    struct CfLun {
        slot: CfSlot,
        /// GPIO13 says a card is in (debounced over two polls).
        inserted: bool,
        seen: bool,
        writable: bool,
    }

    impl CfLun {
        /// Follows the detect line; `true` when the card came or went (the
        /// PC is told the medium changed). A removed card must be attached
        /// again with `cf`: nothing touches the socket unasked.
        fn poll_detect(&mut self, gpio: &mut Gpio<Mmio>) -> bool {
            let now = !gpio.read(CF_DETECT);
            let stable = now == self.seen;
            self.seen = now;
            if !stable || now == self.inserted {
                return false;
            }
            self.inserted = now;
            if !now && let CfSlot::Card(_) = self.slot {
                let CfSlot::Card(card) = core::mem::replace(&mut self.slot, CfSlot::Untried) else {
                    return true;
                };
                let (attr, common) = card.into_inner();
                self.slot = CfSlot::Removed(attr, common);
            }
            true
        }
    }

    impl BlockDevice for CfLun {
        fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), BlockError> {
            match &mut self.slot {
                CfSlot::Card(c) if self.inserted => c.read_blocks(lba, buf),
                _ => Err(BlockError::Device),
            }
        }
        fn write_blocks(&mut self, lba: u32, buf: &[u8]) -> Result<(), BlockError> {
            if !self.writable {
                return Err(BlockError::ReadOnly);
            }
            match &mut self.slot {
                CfSlot::Card(c) if self.inserted => c.write_blocks(lba, buf),
                _ => Err(BlockError::Device),
            }
        }
    }

    impl Storage for CfLun {
        fn blocks(&self) -> u32 {
            match &self.slot {
                CfSlot::Card(c) => c.sectors(),
                _ => 0,
            }
        }
        fn present(&self) -> bool {
            self.inserted && matches!(self.slot, CfSlot::Card(_))
        }
        fn read_only(&self) -> bool {
            !self.writable
        }
        fn product(&self) -> &[u8; 16] {
            b"CF card         "
        }
    }

    /// `cf`: identifies the CompactFlash card, read-only: model, capacity,
    /// whether its write cache is off, and its first partition. The socket
    /// is touched only when GPIO13 says a card is in it.
    #[inline(never)]
    fn cf_command(lun: &mut CfLun, gpio: &mut Gpio<Mmio>, out: &mut Line) {
        if gpio.read(CF_DETECT) {
            let _ = write!(out, "RPL ERR cf: no card (GPIO13 high)\r\n");
            return;
        }
        lun.inserted = true;
        lun.seen = true;
        let slot = &mut lun.slot;
        let windows = match core::mem::replace(slot, CfSlot::Untried) {
            CfSlot::Untried => devices::take_cf(),
            CfSlot::Removed(attr, common) => Some((attr, common)),
            other => {
                *slot = other;
                None
            }
        };
        if let CfSlot::Untried = slot {
            *slot = match windows {
                None => CfSlot::Failed("socket unavailable"),
                Some((attr, common)) => {
                    match CfCard::init(attr, common, &mut |ms| rt::delay_us(ms * 1000)) {
                        Ok(card) => CfSlot::Card(card),
                        Err((e, _, _)) => CfSlot::Failed(e.name()),
                    }
                }
            };
        }
        let card = match slot {
            CfSlot::Card(c) => c,
            CfSlot::Failed(why) => {
                let _ = write!(out, "RPL ERR cf: {why}\r\n");
                return;
            }
            CfSlot::Untried | CfSlot::Removed(..) => return,
        };
        let mut block = [0u8; rp_drv_cf::SECTOR];
        let first = match card.read_blocks(0, &mut block) {
            Err(_) => "sector 0 unreadable",
            Ok(()) => match mbr_partitions(&block).and_then(|p| p[0]) {
                Some(p) if p.kind == 0x0C || p.kind == 0x0B => "FAT32 partition",
                Some(p) if p.kind == 0x07 => "exFAT/NTFS partition",
                Some(p) if p.kind == 0x06 || p.kind == 0x04 || p.kind == 0x0E => "FAT16 partition",
                Some(_) => "other partition",
                None => "no partition table",
            },
        };
        let _ = write!(
            out,
            "RPL OK cf: {} MB ({} sectors), write cache {}, {first}, model ",
            card.sectors() / 2048,
            card.sectors(),
            if card.write_cache_off() { "off" } else { "on" },
        );
        for &b in card.model().iter().take(40) {
            let _ = out.write_char(char::from(b));
        }
        let _ = out.write_str("\r\n");
    }

    /// Audio: set up on the first `tone`, so nothing touches the I2C bus or
    /// the codec unasked.
    enum Audio {
        Untried,
        /// The codec, the I2S, and a DMA channel to stream with if one was
        /// free.
        Ready(Wm8750<PxaI2c<Mmio>>, PxaI2s<Mmio>, Option<Dma>),
        Failed(&'static str),
    }

    /// `tone <hz> <ms>`: a sine (20-20,000 Hz, up to 5 s) through the
    /// WM8750 at 48 kHz, streamed by DMA from `ring` (the CPU refills it by
    /// halves); the reply says how long it really took (the codec's clock
    /// paces it) and how much of that the CPU was busy (a polled FIFO would
    /// keep it busy throughout). Headphones: the speaker amplifier
    /// (GPIO82, CPLD 0x14) is left alone until a device run with the owner
    /// there.
    #[inline(never)]
    fn tone_command(
        audio: &mut Audio,
        jack: &Jack,
        args: &[u8],
        ring: &mut [u8],
        feed: &mut dyn FnMut(),
        out: &mut Line,
    ) {
        let mut it = args.split(|&b| b == b' ').filter(|a| !a.is_empty());
        let (Some(hz), Some(ms)) = (
            it.next()
                .and_then(parse_number)
                .filter(|h| (20..=20_000).contains(h)),
            it.next()
                .and_then(parse_number)
                .filter(|m| (1..=5_000).contains(m)),
        ) else {
            let _ = write!(out, "RPL ERR usage: tone <20-20000 Hz> <1-5000 ms>\r\n");
            return;
        };
        if let Audio::Untried = audio {
            *audio = match (rt::take_i2c(), rt::take_i2s()) {
                (Some(i2c), Some(i2s)) => match Wm8750::init(PxaI2c::new(i2c), 60) {
                    Ok(mut codec) => {
                        let _ = codec.route(jack.output());
                        Audio::Ready(codec, PxaI2s::new(i2s), rt::take_dma(1))
                    }
                    Err(_) => Audio::Failed("no codec answered on I2C (0x1a, 0x1b)"),
                },
                _ => Audio::Failed("I2C or I2S unavailable"),
            };
        }
        let (codec, i2s, dma) = match audio {
            Audio::Ready(c, i, d) => (c, i, d),
            Audio::Failed(why) => {
                let _ = write!(out, "RPL ERR tone: {why}\r\n");
                return;
            }
            Audio::Untried => return,
        };
        let rate = Rate::Khz48;
        let frames = rate.hz() / 1000 * ms;
        let mut source = Tone::new(hz, rate.hz(), 12_000);
        let Some(dma) = dma else {
            let _ = write!(out, "RPL ERR tone: no DMA channel\r\n");
            return;
        };
        i2s.start(rate);
        let r = stream(
            dma,
            ring,
            &mut source,
            frames,
            &mut rt::ticks,
            TICK_1S / 5,
            feed,
        );
        i2s.stop();
        match r {
            Ok(s) => {
                let _ = write!(
                    out,
                    "RPL OK tone {hz} Hz {ms} ms ({frames} frames in {} ms, by DMA, CPU busy {}%), codec at {:#04x}, to {}\r\n",
                    s.total / (TICK_1S / 1000),
                    u64::from(s.busy) * 100 / u64::from(s.total.max(1)),
                    codec.address(),
                    if codec.output() == Output::Headphones {
                        "headphones"
                    } else {
                        "speaker"
                    }
                );
            }
            Err(e) => {
                let why = match e {
                    rp_drv_pxa_i2s::StreamError::Refused => "refused",
                    rp_drv_pxa_i2s::StreamError::Stalled => "stalled",
                };
                let _ = write!(out, "RPL ERR tone: DMA stream {why}\r\n");
            }
        }
    }

    /// `card dma` reports the DMA policy; `card dma writes` lets writes use
    /// DMA for this session (only when reads passed their self-test).
    #[inline(never)]
    fn card_dma(lun: &mut Lun, policy: &DmaPolicy, args: &[u8], out: &mut Line) {
        let host = match (lun.card.as_mut(), lun.spare.as_mut()) {
            (Some(sd), _) => sd.host_mut(),
            (None, Some(h)) => h,
            (None, None) => {
                let _ = write!(out, "RPL ERR dma: no SD controller\r\n");
                return;
            }
        };
        let (reads, _) = host.dma_allowed();
        if args == b" writes" {
            if !reads {
                let _ = write!(
                    out,
                    "RPL ERR dma: reads {}, so writes stay off\r\n",
                    policy.verdict
                );
                return;
            }
            host.set_dma(true, true);
        } else if !args.is_empty() {
            let _ = write!(out, "RPL ERR usage: card dma [writes]\r\n");
            return;
        }
        let (_, writes_now) = host.dma_allowed();
        let _ = write!(
            out,
            "RPL OK dma reads {}, writes {}, {} transfers\r\n",
            if reads { policy.verdict } else { "off" },
            if writes_now { "on" } else { "off" },
            host.dma_transfers()
        );
    }

    /// `card peek <lba> <offset>`: 16 bytes of a block as hex (read-only),
    /// to look at boot sector flags or FAT entries.
    #[inline(never)]
    fn card_peek(lun: &mut Lun, args: &[u8], out: &mut Line) {
        let mut words = args.split(|&b| b == b' ').filter(|w| !w.is_empty());
        let (Some(Some(lba)), Some(Some(off)), Some(sd)) = (
            words.next().map(parse_number),
            words.next().map(parse_number),
            lun.card.as_mut(),
        ) else {
            let _ = out.write_str(LoadError::BadCommand.reply());
            return;
        };
        let off = (off as usize).min(rp_drv_sdcard::BLOCK - 16) & !15;
        let mut b = [0u8; rp_drv_sdcard::BLOCK];
        if sd.read(lba, &mut b).is_err() {
            let _ = write!(out, "RPL ERR peek: didn't read{}\r\n", card_note(sd));
            return;
        }
        let _ = write!(out, "RPL OK peek {lba}+{off}:");
        for x in &b[off..off + 16] {
            let _ = write!(out, " {x:02x}");
        }
        let _ = out.write_str("\r\n");
    }

    /// `backlight` reports the level; `backlight <0-100>` fades to it
    /// (perceptual percent, rp-drv-pxa-pwm).
    #[inline(never)]
    fn backlight_command(bl: Option<&mut Backlight<Mmio>>, args: &[u8], out: &mut Line) {
        let Some(bl) = bl else {
            let _ = out.write_str("RPL ERR backlight: PWM0 unavailable\r\n");
            return;
        };
        if args.iter().any(|b| !b.is_ascii_whitespace()) {
            let Some(level) = parse_number(args).filter(|&l| l <= 100) else {
                let _ = out.write_str(LoadError::BadCommand.reply());
                return;
            };
            bl.fade_to(level, &mut |us| rt::delay_us(us));
        }
        let level = bl.level();
        let (on, counts) = bl.duty();
        let _ = write!(out, "RPL OK backlight {level}% (duty {on}/{counts})\r\n");
    }

    /// A decimal number from a command's argument (spaces ignored).
    fn parse_number(args: &[u8]) -> Option<u32> {
        args.iter()
            .filter(|b| !b.is_ascii_whitespace())
            .try_fold(0u32, |n, &d| {
                d.is_ascii_digit()
                    .then(|| n.checked_mul(10)?.checked_add(u32::from(d - b'0')))
                    .flatten()
            })
    }

    /// `card scrub <lba>`: rewrites a weak block with its own contents, so
    /// the card stores it in a fresh page (a slow block reads fast again).
    /// Only a first-FAT sector, and only when its one read matches its
    /// mirror in the second FAT: the card's own backup, so the data is
    /// verified before and recoverable after. Read back and compared, with
    /// the read time before and after.
    #[inline(never)]
    fn card_scrub(lun: &mut Lun, args: &[u8], out: &mut Line) {
        let lba = args
            .iter()
            .filter(|b| !b.is_ascii_whitespace())
            .try_fold(0u32, |n, &d| {
                d.is_ascii_digit()
                    .then(|| n.checked_mul(10)?.checked_add(u32::from(d - b'0')))
                    .flatten()
            });
        let (Some(lba), Some(sd)) = (lba, lun.card.as_mut()) else {
            let _ = out.write_str(LoadError::BadCommand.reply());
            return;
        };
        let mirror = match mount(sd) {
            Ok(vol) => vol.fat_mirror(lba),
            Err(why) => {
                let _ = write!(out, "RPL ERR scrub: {why}\r\n");
                return;
            }
        };
        let Some(mirror) = mirror else {
            let _ = out.write_str("RPL ERR scrub: only first-FAT sectors (they have a mirror)\r\n");
            return;
        };
        let mut a = [0u8; rp_drv_sdcard::BLOCK];
        let mut b = [0u8; rp_drv_sdcard::BLOCK];
        let t = rt::ticks();
        let first = sd.read(lba, &mut a);
        let before = rt::ticks().wrapping_sub(t) / 3;
        // One read: on the owner's card, reading a weak block again right
        // away is what makes it fail; the mirror is the independent check.
        if first.is_err() {
            let _ = write!(
                out,
                "RPL ERR scrub: the block didn't read{}\r\n",
                card_note(sd)
            );
            return;
        }
        if sd.read(mirror, &mut b).is_err() {
            let _ = write!(
                out,
                "RPL ERR scrub: mirror {mirror} unreadable: not written\r\n"
            );
            return;
        }
        if a != b {
            // How they differ: count, first and last offsets, and the words
            // there (FAT entries are 32-bit), so the cause can be judged.
            let diff: u32 = a.iter().zip(&b).map(|(x, y)| u32::from(x != y)).sum();
            let first = a.iter().zip(&b).position(|(x, y)| x != y).unwrap_or(0);
            let last = a.iter().zip(&b).rposition(|(x, y)| x != y).unwrap_or(0);
            let w = first & !3;
            let word = |s: &[u8; rp_drv_sdcard::BLOCK]| {
                u32::from_le_bytes([s[w], s[w + 1], s[w + 2], s[w + 3]])
            };
            let _ = write!(
                out,
                "RPL ERR scrub: differs from mirror {mirror} in {diff} bytes ({first}..{last}; at {w}: {:08x} vs {:08x}): not written\r\n",
                word(&a),
                word(&b)
            );
            return;
        }
        if sd.write(lba, &a).is_err() {
            let _ = write!(
                out,
                "RPL ERR scrub: write failed{} (mirror {mirror} still holds the data)\r\n",
                card_note(sd)
            );
            return;
        }
        let t = rt::ticks();
        let ok = sd.read(lba, &mut b).is_ok() && a == b;
        let after = rt::ticks().wrapping_sub(t) / 3;
        if ok {
            let _ = write!(
                out,
                "RPL OK scrub {lba}: rewritten and verified against mirror {mirror}; read {before} us before, {after} us after\r\n"
            );
        } else {
            let _ = write!(
                out,
                "RPL ERR scrub: read-back differs (mirror {mirror} holds the data)\r\n"
            );
        }
    }

    #[inline(never)]
    fn card_scan_inner(lun: &mut Lun, args: &[u8], out: &mut Line, feed: &mut dyn FnMut()) {
        let mut nums = args
            .split(|&b| b == b' ')
            .filter(|w| !w.is_empty())
            .map(|w| {
                w.iter().try_fold(0u32, |n, &d| {
                    d.is_ascii_digit()
                        .then(|| n.checked_mul(10)?.checked_add(u32::from(d - b'0')))
                        .flatten()
                })
            });
        let (Some(Some(lba)), Some(Some(count)), None) = (nums.next(), nums.next(), nums.next())
        else {
            let _ = out.write_str(LoadError::BadCommand.reply());
            return;
        };
        let Some(sd) = lun.card.as_mut() else {
            let _ = out.write_str("RPL ERR card: no card\r\n");
            return;
        };
        let count = count.min(4096);
        let mut block = [0u8; rp_drv_sdcard::BLOCK];
        let (mut failed, mut first, mut slowest, mut slowest_at) = (0u32, None, 0u32, 0u32);
        for b in lba..lba.saturating_add(count) {
            feed();
            let t = rt::ticks();
            let r = sd.read(b, &mut block);
            let us = rt::ticks().wrapping_sub(t) / 3;
            if us > slowest {
                slowest = us;
                slowest_at = b;
            }
            if r.is_err() {
                failed += 1;
                first.get_or_insert(b);
            }
        }
        let _ = write!(
            out,
            "RPL OK scan {lba}+{count}: {failed} failed (first {}), slowest {slowest} us at {slowest_at}{}\r\n",
            first.unwrap_or(0),
            if failed > 0 {
                card_note(sd)
            } else {
                Note {
                    what: "",
                    failure: None,
                    status: 0,
                }
            },
        );
    }

    /// The card's own error behind a device error, for the reply.
    fn card_note(sd: &Sd) -> Note {
        let what = match sd.last_error() {
            Some(rp_drv_sdcard::Error::Host(rp_drv_sdcard::HostError::Timeout)) => "timeout",
            Some(rp_drv_sdcard::Error::Host(rp_drv_sdcard::HostError::Crc)) => "CRC",
            Some(rp_drv_sdcard::Error::Host(rp_drv_sdcard::HostError::Data)) => "data",
            Some(rp_drv_sdcard::Error::CardStatus(v)) => {
                return Note {
                    what: "status",
                    failure: None,
                    status: v,
                };
            }
            Some(_) => "error",
            None => "",
        };
        Note {
            what,
            failure: sd.host().last_failure(),
            status: 0,
        }
    }

    /// `card_note`'s text: the error and, for a timeout, which command and
    /// the status register it left.
    struct Note {
        what: &'static str,
        failure: Option<(u8, u32)>,
        /// The card's status word, for a card status error.
        status: u32,
    }

    impl core::fmt::Display for Note {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            if self.what.is_empty() {
                return Ok(());
            }
            write!(f, " (card: {}", self.what)?;
            if self.status != 0 {
                write!(f, " {:#010x}", self.status)?;
            }
            if let Some((cmd, stat)) = self.failure {
                write!(f, " on CMD{cmd} stat {stat:#x}")?;
            }
            f.write_str(")")
        }
    }

    fn on_fault(kind: rt::Fault, addr: u32) -> ! {
        crash(format_args!("FAULT {} at {addr:#010x}", kind.name()))
    }

    #[inline(never)]
    fn crash(msg: core::fmt::Arguments) -> ! {
        let mut uart = PxaUart::new(rt::emergency_uart());
        let _ = writeln!(uart, "\n{msg}");
        if let Some(vram) = devices::emergency_vram() {
            let plane = Plane::fullscreen(&X51V_VGA, PixelFormat::Rgb565);
            let mut c = Console::new(
                &mut vram[plane.offset..],
                plane.stride,
                (0, 576),
                (480, 64),
                2,
                (0xFFFF, 0xA800),
            );
            let _ = write!(c, "{msg}");
        }
        halt()
    }

    /// Start-up colours (as in rp-light): a hang leaves the last one on
    /// screen. Red: main entered (the relocation worked); yellow: FFUART
    /// set up; green: MMU on; cyan: 2700G answered.
    #[inline(never)]
    fn step(i: usize) {
        const COLOURS: [u16; 4] = [0xF800, 0xFFE0, 0x07E0, 0x07FF];
        devices::beacon(COLOURS[i], COLOURS[i]);
    }

    fn main() -> ! {
        step(0);
        let Some(bus) = rt::take_ffuart() else { halt() };
        let mut uart = PxaUart::new(bus);
        uart.init(115_200);
        step(1);
        rt::set_fault_handler(on_fault);
        rt::init_mmu();
        step(2);
        // GPIO, the USB detach and the card first: they run during the
        // display's settle time after the hand-off (`devices::take_mbx`)
        // instead of after it (boot time, docs/design-brief.md).
        let Some(gpio_bus) = rt::take_gpio() else {
            halt()
        };
        let mut gpio = Gpio::new(gpio_bus);
        gpio.make_input(USB_DETECT);
        gpio.make_output(USB_PULLUP, false);
        // The PC must see the device gone for DETACH before it comes back;
        // card set-up runs meanwhile, and only what's left is waited for.
        let detached_at = rt::ticks();
        let Some(udc_bus) = rt::take_udc() else {
            halt()
        };
        // The card first: once the pull-up is on, the PC expects answers
        // within its control-transfer timeout, and card set-up (or giving up
        // on an empty slot) must not stall them (seen in the emulator: the
        // host gave up on the first descriptor request).
        gpio.make_input(SD_DETECT);
        gpio.make_input(SD_LOCK);
        let (card, spare) = open_card();
        let Some(scratch) = devices::take_scratch() else {
            halt()
        };
        let (msc_buf, fat_scratch) = scratch.split_at_mut(MSC_BUF);
        // In: the switch says so, or a card answered anyway (so a wrong
        // reading of the switch can't hide a working card).
        let inserted = !gpio.read(SD_DETECT) || card.is_some();
        let locked = gpio.read(SD_LOCK);
        let mut lun = Lun {
            card,
            spare,
            writable: false,
            offered: true,
            inserted,
            locked,
            seen: (inserted, locked),
        };
        let dma = check_dma(&mut lun, msc_buf);

        let Some((mut regs, vram)) = devices::take_mbx() else {
            halt()
        };
        step(3);
        let plane = Plane::decode(
            regs.read32(reg::GSCTRL),
            regs.read32(reg::GSCADR),
            regs.read32(reg::GSADR),
            MBX_VRAM_SIZE as usize,
        )
        .filter(|p| p.format == PixelFormat::Rgb565 && p.width == 480 && p.height == 640)
        .unwrap_or_else(|| {
            let p = Plane::fullscreen(&X51V_VGA, PixelFormat::Rgb565);
            Display::new(regs).enable(&X51V_VGA, &p);
            p
        });
        let end = plane.offset + plane.stride * usize::from(plane.height);
        let (top, rest) = vram[plane.offset..end].split_at_mut(plane.stride * STATUS_H);
        // The previous program's run: if it crashed or hung, the crash
        // screen goes above the log.
        let mut pending = devices::log_take();
        let trouble = pending.as_ref().is_some_and(|r| r.crashed || !r.returned);
        let panel_h = if trouble { CRASH_H } else { 0 };
        let (panel, rest) = rest.split_at_mut(plane.stride * panel_h);
        let mut log = Console::new(
            rest,
            plane.stride,
            (0, 0),
            (480, 640 - STATUS_H - panel_h),
            2,
            (0xFFFF, 0x0000),
        );

        let Some(dest) = devices::take_image_area() else {
            let _ = writeln!(log, "loader not at its home address");
            halt()
        };

        let waited = rt::ticks().wrapping_sub(detached_at);
        if waited < DETACH {
            rt::delay_us((DETACH - waited) / 13 * 4);
        }
        let mut udc = Udc::new(udc_bus);
        let usb = udc.enable().is_ok();
        if usb {
            gpio.write(USB_PULLUP, true);
        }
        let _ = writeln!(log, "RetroPane loader at {:#x}", devices::LOADER_BASE);
        let _ = writeln!(log, "USB {}", if usb { "connected" } else { "unavailable" });
        let _ = writeln!(log, "SD DMA reads {}, writes off", dma.verdict);
        let _ = writeln!(log, "waiting for an image: cargo xtask load");

        // The previous run's log (an image this loader started, now back):
        // to the FFUART at once, and over USB once the PC has the port open
        // again (the device reconnected when it came back).
        if let Some(run) = &pending {
            let how = if run.crashed {
                "CRASHED"
            } else if run.returned {
                "returned"
            } else {
                "did NOT return"
            };
            let _ = writeln!(log, "previous run {how}: {} bytes of log", run.log.len());
            // A crash or a hang is kept on the card first, so the report
            // survives whatever happens next (the PC may not be there).
            if run.crashed || !run.returned {
                let saved = save_report(&mut lun, run, &mut fat_scratch[..SCRATCH]);
                let _ = writeln!(
                    log,
                    "report {} on the card",
                    if saved { "saved" } else { "NOT saved" }
                );
                crash::draw(
                    panel,
                    plane.stride,
                    (480, CRASH_H),
                    if run.crashed {
                        "The program crashed"
                    } else {
                        "The program stopped responding"
                    },
                    if saved {
                        "Report on the card: crashlog.txt"
                    } else {
                        "Report sent to the PC"
                    },
                    run.log,
                );
            }
            send_log(&mut |b| uart_bytes(&mut uart, b), run);
        }

        let mut rx = Receiver::new();
        let mut last = rt::ticks().wrapping_sub(TICK_1S);
        // Idling: after QUIET of nothing happening the loop sleeps until the
        // next 1 ms tick instead of spinning (battery and heat), so traffic
        // runs at full speed and a waiting loader is mostly asleep.
        rt::start_tick(1000);
        let mut active = rt::ticks();
        let mut msc_seen = 0u32;
        let (mut asleep, mut idle_pct) = (0u32, 0u32);
        let mut buf = [0u8; 64];
        let mut watchdog = rt::watchdog();
        let mut msc = Msc::new(msc_buf, MSC_SERIAL, 2);
        // The backlight, left as Windows Mobile set it until asked.
        let mut backlight = rt::take_pwm0().map(Backlight::new);
        let mut audio = Audio::Untried;
        gpio.make_input(HP_DETECT);
        let mut jack = Jack {
            plugged: gpio.read(HP_DETECT),
            seen: gpio.read(HP_DETECT),
        };
        let mut cf = CfLun {
            slot: CfSlot::Untried,
            inserted: false,
            seen: false,
            writable: false,
        };
        let mut power = PowerButton::new(&mut gpio);
        udc.set_msc_luns(2);
        // Layout fallback: if the PC keeps trying (bus resets) but never
        // configures the composite device, offer the serial-only layout it
        // already knows, so no host can lock the loop out; and back again.
        let mut layout_since = rt::ticks();
        let mut layout_resets = udc.stats().resets;
        let _ = writeln!(
            log,
            "SD card: {} (a drive on the PC)",
            if lun.present() { "ready" } else { "none" }
        );
        // Boot time (docs/design-brief.md): entry to the waiting loop, every
        // driver set up that the loop needs now (audio and CF start later,
        // on first use).
        let _ = writeln!(uart, "BENCH boot_loader_ready {} 1", rt::since_start());
        loop {
            // Armed by an earlier run (it can't be disarmed): keep it fed
            // while waiting.
            if watchdog.armed() {
                watchdog.feed(RUN_LIMIT);
            }
            if rt::ticks().wrapping_sub(active) > QUIET {
                let t = rt::ticks();
                rt::wait_tick();
                asleep = asleep.wrapping_add(rt::ticks().wrapping_sub(t));
            }
            udc.poll();
            if udc.take_msc_reset() {
                msc.reset();
            }
            if udc.configured() {
                layout_since = rt::ticks();
                layout_resets = udc.stats().resets;
            } else if udc.stats().resets != layout_resets
                && rt::ticks().wrapping_sub(layout_since) > LAYOUT_PATIENCE
            {
                let composite = !udc.composite();
                udc.disable();
                udc.set_composite(composite);
                let _ = udc.enable();
                layout_since = rt::ticks();
                layout_resets = udc.stats().resets;
                let _ = writeln!(
                    log,
                    "PC didn't configure the device: offering the {} layout",
                    if composite {
                        "serial + drive"
                    } else {
                        "serial-only"
                    }
                );
                let _ = writeln!(
                    uart,
                    "USB layout: {}",
                    if composite {
                        "composite"
                    } else {
                        "serial-only"
                    }
                );
            }
            msc.poll(&mut Pipe(&mut udc), &mut [&mut lun, &mut cf], now_ms());
            if udc.host_open() {
                if let Some(run) = pending.take() {
                    send_log(&mut |b| usb_bytes(&mut udc, b), &run);
                }
            }
            let mut outcome = None;
            let mut heard = false;
            while outcome.is_none() {
                let Some(b) = uart.read_byte() else { break };
                heard = true;
                outcome = feed_slice(&mut rx, &[b], dest).1;
            }
            if outcome.is_none() {
                let n = udc.read(&mut buf);
                heard |= n > 0;
                let mut used = 0;
                while outcome.is_none() && used < n {
                    let (u, r) = feed_slice(&mut rx, &buf[used..n], dest);
                    used += u;
                    outcome = r;
                }
            }
            let msc_now = msc.stats().commands;
            if heard
                || outcome.is_some()
                || msc_now != msc_seen
                || msc.busy()
                || udc.tx_pending() > 0
            {
                active = rt::ticks();
                msc_seen = msc_now;
            }
            match outcome {
                Some(Ok(Frame {
                    kind: Kind::File, ..
                })) => {
                    let frame = outcome.and_then(Result::ok).unwrap_or(Frame {
                        kind: Kind::File,
                        len: 0,
                    });
                    // A card write can take seconds: keep the watchdog fed.
                    if watchdog.armed() {
                        watchdog.feed(RUN_LIMIT);
                    }
                    let mut reply = Line::new();
                    put_file(&mut lun, &mut msc, frame, dest, &mut reply);
                    send(&mut uart, &mut udc, reply.as_str());
                    let _ = writeln!(log, "{}", reply.as_str().trim_ascii_end());
                }
                Some(Ok(
                    f @ Frame {
                        kind: Kind::Control,
                        ..
                    },
                )) => {
                    let mut status = Line::new();
                    let reply = match f.command(dest) {
                        Ok(b"drive status") => {
                            let s = msc.stats();
                            let _ = write!(
                                status,
                                "RPL OK drive {} cmds {} failed, {} blocks read, {} errors (last at {}+{}), {} card retries, {} block-by-block, {} slow-clock, {} weak ranges{}, idle {}%\r\n",
                                s.commands,
                                s.failed,
                                s.blocks_read,
                                s.io_errors,
                                s.last_error.0,
                                s.last_error.1,
                                lun.card.as_ref().map_or(0, SdCard::read_retries),
                                lun.card.as_ref().map_or(0, SdCard::read_fallbacks),
                                lun.card.as_ref().map_or(0, SdCard::slow_reads),
                                lun.card.as_ref().map_or(0, SdCard::weak_ranges),
                                lun.card.as_ref().map_or(
                                    Note {
                                        what: "",
                                        failure: None,
                                        status: 0,
                                    },
                                    card_note
                                ),
                                idle_pct.min(100),
                            );
                            status.as_str()
                        }
                        Ok(c) if c.starts_with(b"backlight") => {
                            backlight_command(backlight.as_mut(), &c[9..], &mut status);
                            status.as_str()
                        }
                        Ok(c) if c.starts_with(b"tone") => {
                            tone_command(
                                &mut audio,
                                &jack,
                                &c[4..],
                                &mut fat_scratch[..8192],
                                &mut || {
                                    if watchdog.armed() {
                                        watchdog.feed(RUN_LIMIT);
                                    }
                                },
                                &mut status,
                            );
                            status.as_str()
                        }
                        Ok(c) if c == b"ls" || c.starts_with(b"ls ") => {
                            if msc.host_writing(now_ms()) {
                                "RPL ERR card busy: the PC is writing\r\n"
                            } else {
                                ls_command(
                                    &mut lun,
                                    &c[2..],
                                    &mut |b| usb_bytes(&mut udc, b),
                                    &mut status,
                                );
                                status.as_str()
                            }
                        }
                        Ok(c) if c.starts_with(b"get ") => {
                            if msc.host_writing(now_ms()) {
                                "RPL ERR card busy: the PC is writing\r\n"
                            } else {
                                get_command(
                                    &mut lun,
                                    &c[4..],
                                    &mut fat_scratch[..SCRATCH],
                                    &mut |b| usb_bytes(&mut udc, b),
                                    &mut status,
                                );
                                status.as_str()
                            }
                        }
                        Ok(b"cf") => {
                            cf_command(&mut cf, &mut gpio, &mut status);
                            // Attached (or not): the PC looks at LUN 1 again.
                            msc.medium_changed(1);
                            status.as_str()
                        }
                        Ok(c) if c.starts_with(b"rm ") => {
                            if msc.host_writing(now_ms()) {
                                "RPL ERR card busy: the PC is writing\r\n"
                            } else {
                                rm_command(&mut lun, &c[3..], &mut status);
                                // The PC must re-read the volume.
                                msc.medium_changed(0);
                                status.as_str()
                            }
                        }
                        Ok(b"card check") => {
                            if msc.host_writing(now_ms()) {
                                "RPL ERR card busy: the PC is writing\r\n"
                            } else {
                                let (scratch, seen) = fat_scratch.split_at_mut(SCRATCH);
                                card_check(&mut lun, seen, scratch, &mut status);
                                status.as_str()
                            }
                        }
                        Ok(c) if c.starts_with(b"card dma") => {
                            card_dma(&mut lun, &dma, &c[8..], &mut status);
                            status.as_str()
                        }
                        Ok(c) if c.starts_with(b"card peek ") => {
                            card_peek(&mut lun, &c[10..], &mut status);
                            status.as_str()
                        }
                        Ok(c) if c.starts_with(b"card repair ") => {
                            if watchdog.armed() {
                                watchdog.feed(RUN_LIMIT);
                            }
                            if msc.host_writing(now_ms()) {
                                "RPL ERR card busy: the PC is writing\r\n"
                            } else {
                                card_repair(&mut lun, &c[12..], &mut status);
                                msc.medium_changed(0);
                                status.as_str()
                            }
                        }
                        Ok(c) if c.starts_with(b"card scrub ") => {
                            if watchdog.armed() {
                                watchdog.feed(RUN_LIMIT);
                            }
                            if msc.host_writing(now_ms()) {
                                "RPL ERR card busy: the PC is writing\r\n"
                            } else {
                                card_scrub(&mut lun, &c[11..], &mut status);
                                msc.medium_changed(0);
                                status.as_str()
                            }
                        }
                        Ok(c) if c.starts_with(b"card scan ") => {
                            if watchdog.armed() {
                                watchdog.feed(RUN_LIMIT);
                            }
                            card_scan(&mut lun, &c[10..], &mut status, &mut || {
                                if watchdog.armed() {
                                    watchdog.feed(RUN_LIMIT);
                                }
                            });
                            status.as_str()
                        }
                        Ok(b"drive off") | Ok(b"drive on") => {
                            lun.offered = f.command(dest) == Ok(b"drive on");
                            msc.medium_changed(0);
                            if lun.offered {
                                "RPL OK drive offered to the PC\r\n"
                            } else {
                                "RPL OK drive off: no medium for the PC\r\n"
                            }
                        }
                        Ok(b"drive rw") | Ok(b"drive ro") => {
                            lun.writable = f.command(dest) == Ok(b"drive rw");
                            cf.writable = lun.writable;
                            // The PC re-reads the media and their protection.
                            msc.medium_changed(0);
                            msc.medium_changed(1);
                            if lun.writable {
                                "RPL OK drive read-write\r\n"
                            } else {
                                "RPL OK drive read-only\r\n"
                            }
                        }
                        _ => LoadError::BadCommand.reply(),
                    };
                    send(&mut uart, &mut udc, reply);
                    let _ = writeln!(log, "{}", reply.trim_ascii_end());
                }
                Some(Ok(Frame { len, .. })) => {
                    // Bring-up check: does the image survive the cache flush
                    // the launch does? Flush, drop the cache, re-read from
                    // memory and compare with the CRC the sender gave.
                    rp_hal_pxa27x::cpu::clean_invalidate_all();
                    let intact = rp_loadproto::crc32(&dest[..len]) == rx.expected_crc();
                    // A fingerprint of what will run, for the PC to compare.
                    let mut ok = Line::new();
                    let digest = rp_crypto::hex(&rp_crypto::Sha256::digest(&dest[..len]));
                    let _ = ok.write_str("RPL OK sha256 ");
                    let _ = ok.write_str(core::str::from_utf8(&digest).unwrap_or(""));
                    let _ = ok.write_str("\r\n");
                    send(&mut uart, &mut udc, ok.as_str());
                    let _ = writeln!(log, "image OK, {len} bytes: starting");
                    // Let the reply leave over USB before disconnecting: on
                    // the device, 50 ms was not enough for the PC's program to
                    // read it before the port went away.
                    let t = rt::ticks();
                    while rt::ticks().wrapping_sub(t) < TICK_1S / 2 {
                        udc.poll();
                    }
                    gpio.write(USB_PULLUP, false);
                    udc.disable();
                    devices::log_start();
                    // The verdict goes at the top of the image's run log, so
                    // it reaches the PC whatever the image then does.
                    devices::log_write(if intact {
                        b"[loader: image intact after the cache flush]\n"
                    } else {
                        b"[loader: image CHANGED after the cache flush]\n"
                    });
                    // A run limit: an image that hangs (never returns, never
                    // feeds the watchdog) resets the device, and the loader
                    // comes back (started again by Windows Mobile's StartUp
                    // folder on the device; the emulator restarts it).
                    watchdog.feed(RUN_LIMIT);
                    // Nothing of the loader's may fire under the image.
                    rt::stop_tick();
                    rt::irq_disable();
                    handoff::launch(IMAGE_BASE as u32);
                }
                Some(Err(e)) => {
                    send(&mut uart, &mut udc, e.reply());
                    let _ = writeln!(log, "refused: {}", what(e));
                }
                None => {}
            }
            if power.pressed(&mut gpio)
                && let Some(bl) = backlight.as_mut()
            {
                let on = power.toggle(bl);
                let _ = writeln!(
                    uart,
                    "power button: screen {}",
                    if on { "on" } else { "off" }
                );
            }
            if rt::ticks().wrapping_sub(last) >= TICK_1S
                && let Some(plugged) = jack.poll(&mut gpio)
            {
                if let Audio::Ready(codec, ..) = &mut audio {
                    let _ = codec.route(jack.output());
                }
                let _ = writeln!(
                    log,
                    "headphones {} (GPIO83 {})",
                    if plugged { "in" } else { "out" },
                    u8::from(plugged)
                );
            }
            if rt::ticks().wrapping_sub(last) >= TICK_1S && cf.poll_detect(&mut gpio) {
                msc.medium_changed(1);
                let _ = writeln!(log, "CF card {}", if cf.inserted { "in" } else { "out" });
            }
            if rt::ticks().wrapping_sub(last) >= TICK_1S && lun.poll_slot(&mut gpio) {
                msc.medium_changed(0);
                let _ = writeln!(
                    log,
                    "SD card {}{}",
                    if lun.inserted { "in" } else { "out" },
                    if lun.locked { ", locked" } else { "" }
                );
            }
            if rt::ticks().wrapping_sub(last) >= TICK_1S {
                // The share of the last second spent asleep.
                idle_pct = asleep / (rt::ticks().wrapping_sub(last).max(1) / 100).max(1);
                asleep = 0;
                last = rt::ticks();
                if rx.progress().is_none() {
                    // Over USB only once the previous run's log has gone,
                    // so the PC can tell "back with a log" from "restarted
                    // without one" (a hang whose log the reset wiped).
                    let _ = uart.write_str(READY);
                    // At most one READY waits in the USB ring: while no
                    // program on the PC reads the port nothing drains, and
                    // a ring full of stale READYs would cut the reply to the
                    // next frame short (seen on the device after an hour
                    // idle: 36 old lines, one torn, and the reply lost).
                    if pending.is_none() && udc.tx_pending() == 0 {
                        udc.write(READY.as_bytes());
                    }
                }
                let s = udc.stats();
                let mut st = Console::new(
                    &mut *top,
                    plane.stride,
                    (0, 0),
                    (480, STATUS_H),
                    2,
                    (0x0000, 0x07FF),
                );
                let _ = writeln!(st, "RetroPane 26 loader");
                let _ = writeln!(
                    st,
                    "cable {} configured {} open {}",
                    u8::from(gpio.read(USB_DETECT)),
                    yes(udc.configured()),
                    yes(udc.host_open())
                );
                let _ = writeln!(st, "usb resets {} setups {}", s.resets, s.setups);
                match rx.progress() {
                    Some((got, len)) => {
                        let _ = writeln!(st, "receiving {got}/{len}");
                    }
                    None => {
                        let _ = writeln!(st, "idle");
                    }
                }
            }
        }
    }

    /// A line to the host on both transports; over USB it goes whole, the
    /// ring draining as the PC reads (never cut short to fit).
    fn send(uart: &mut PxaUart<rp_hal_pxa27x::Mmio>, udc: &mut Udc<rp_hal_pxa27x::Mmio>, s: &str) {
        let _ = uart.write_str(s);
        usb_bytes(udc, s.as_bytes());
    }

    /// The run log, framed for `cargo xtask load`; a run that didn't
    /// return (a hang the watchdog reset) says so before the end marker.
    /// Keeps a crash or hang report on the card as `crashlog.txt`
    /// (replaced each time, crash-safe like every card write).
    #[inline(never)]
    fn save_report(lun: &mut Lun, run: &devices::PreviousRun, scratch: &mut [u8]) -> bool {
        let Some(sd) = lun.card.as_mut() else {
            return false;
        };
        let Ok(mut vol) = mount_any(sd) else {
            return false;
        };
        vol.put("crashlog.txt", run.log, scratch).is_ok()
    }

    fn send_log(out: &mut dyn FnMut(&[u8]), run: &devices::PreviousRun) {
        out(b"RPL LOG\r\n");
        out(run.log);
        if run.overflow {
            out(b"\r\n[run log full: later output dropped]");
        }
        if run.crashed {
            out(b"\r\n[the image crashed: report kept on the card as crashlog.txt]");
        }
        if !run.returned {
            out(b"\r\n[the image did not return: it hung and the device was reset]");
        }
        out(b"\r\nRPL END\r\n");
    }

    fn uart_bytes(uart: &mut PxaUart<rp_hal_pxa27x::Mmio>, data: &[u8]) {
        for &b in data {
            uart.write_byte(b);
        }
    }

    /// All of `data` over USB, polling as the transmit ring drains (gives
    /// up after 5 s or if the PC goes away).
    fn usb_bytes(udc: &mut Udc<rp_hal_pxa27x::Mmio>, mut data: &[u8]) {
        let start = rt::ticks();
        while !data.is_empty() && udc.configured() && rt::ticks().wrapping_sub(start) < 5 * TICK_1S
        {
            let n = udc.write(data);
            data = &data[n..];
            udc.poll();
        }
    }

    fn what(e: LoadError) -> &'static str {
        match e {
            LoadError::TooBig => "too big",
            LoadError::BadCrc => "bad CRC",
            LoadError::BadName => "bad file name",
            LoadError::BadCommand => "bad command",
        }
    }

    fn yes(b: bool) -> &'static str {
        if b { "yes" } else { "no" }
    }

    fn halt() -> ! {
        loop {
            core::hint::spin_loop();
        }
    }

    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        match info.location() {
            Some(l) => crash(format_args!("PANIC {}:{}", l.file(), l.line())),
            None => crash(format_args!("PANIC")),
        }
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
