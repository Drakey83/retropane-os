//! The cards over MTP (ADR-0037 rule 2): the SD and CF cards as stores of
//! `rp-usb-mtp`'s responder, on the USB layout of rule 3 (serial + MTP).
//! The loader keeps the responder and what it knows about the cards in
//! [`Tables`] (the RAM pool, free while the loader runs: never its stack,
//! rule 7) and lends its cards for each poll ([`LoaderCards`]), so the
//! rest of the loop keeps using them between polls.

use core::fmt::Write;

use rp_drv_pxa_keypad::{Button, Keypad, Pressed};
use rp_drv_pxa_udc::{Layout, Udc};
use rp_fat::AnyVolume;
use rp_fat::node::{Node, ROOT};
use rp_hal_pxa27x::{Mmio, rt};
use rp_hal_x51v::carve::{carve, carve_with};
use rp_usb_msc::Storage;
use rp_usb_mtp::fat::{Cards, FatDevice, FatState};
use rp_usb_mtp::{
    Consent, Identity, Kind, Level, MAX_DEPTH, Mtp, PACKET, Pipe, Slot, StoreError, Text,
};

use super::{
    BOOT_FILES, CF_BOOT_FILES, CF_DEVICE_ICON, CfLun, DEVICE_ICON, Fed, Line, Lun, MSC_SERIAL,
    SCRATCH, format_command, mount_any, mount_cf, now_ms,
};

/// Objects the handle table holds (rule 7). From the owner's cards: the
/// main SD card held 4,654 files and folders (`card check`, 2026-10-17,
/// docs/overnight.md). The table holds 14 times that: both cards listed
/// whole (a photo import walks everything), with the files a session copies
/// on, in 1.5 MiB of the RAM pool. The CF card's count comes with the first
/// device run (R53). A full table answers Device_Busy and is counted
/// (`drive status`), never a crash.
pub const HANDLES: usize = 65_536;
/// The responder's staging buffer: 32 sectors, one card command per
/// SendObject or GetObject step.
const STAGING: usize = 16 * 1024;
/// Card commands' scratch (FAT searches, directory sectors).
const CARD_SCRATCH: usize = 16 * 1024;
/// The device icon's read buffer (Q5: Icons-Land's PDA, R60). The icon is
/// served straight from the card as Windows asks for it (at every
/// connection), so its size costs no RAM: this holds one request (at most a
/// staging buffer) and the sector an unaligned piece starts in, read in one
/// go.
const ICON_SCRATCH: usize = STAGING + 512;
/// RAM for a format's kept files (ADR-0036's `format`): a format from the
/// PC is only for a card holding no boot files, so this holds at most
/// `autorun.inf` and its icon. Not the image area: an image may be arriving.
const FORMAT_RAM: usize = 1024 * 1024;
/// Path segments followed for the loader's own writes' events.
const DEPTH: usize = 8;
/// How long the PC may go without opening an MTP session before an
/// automatic layout offers the drive instead (rule 4; Windows never waits
/// for it: it reads the OS 2.0 set). A first value: the wait is measured on
/// Windows 11 and a Mac on the first device run (R53) before it is fixed.
pub const PATIENCE_MS: u32 = 8_000;

/// How the PC gets the cards (`cargo xtask drive mtp|msc|auto`, rule 4).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// MTP first, the drive when the PC opens no session.
    Auto,
    /// MTP only.
    Mtp,
    /// The drive only (ADR-0014): this loader's default until the X51v
    /// proves MTP (R53), so the owner's daily drive doesn't change first.
    Msc,
}

impl Mode {
    /// The layout a fresh connection starts with.
    pub fn first(self) -> Layout {
        match self {
            Mode::Auto | Mode::Mtp => Layout::Mtp,
            Mode::Msc => Layout::Drive,
        }
    }

    /// The layout to offer when the PC didn't configure `now` (no host
    /// can lock the development link out: serial is always on the way).
    pub fn after(self, now: Layout) -> Layout {
        match (self, now) {
            (Mode::Auto, Layout::Mtp) => Layout::Drive,
            (Mode::Msc, Layout::Serial) => Layout::Drive,
            (Mode::Auto | Mode::Mtp, Layout::Serial) => Layout::Mtp,
            _ => Layout::Serial,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Mode::Auto => "auto",
            Mode::Mtp => "mtp",
            Mode::Msc => "msc",
        }
    }
}

/// The format consent's screen state and the device's name.
pub struct Ui {
    keypad: Option<Keypad<Mmio>>,
    /// The buttons held when the question was asked (a press is a change).
    held: Pressed,
    /// The friendly name (Q4): kept while the loader runs; the OS proper's
    /// settings keep it across starts (quality audit C87).
    name: Text,
    /// The device icon's card and size (`None`: not looked for since the
    /// last card change; a size of 0: on neither card).
    icon_at: Option<(u8, u32)>,
}

/// Everything MTP keeps, in the RAM pool.
pub struct Tables {
    pub mtp: Mtp<'static>,
    pub state: &'static mut FatState,
    pub scratch: &'static mut [u8],
    pub icon: &'static mut [u8],
    pub format_ram: &'static mut [u8],
    pub ui: Ui,
}

impl Tables {
    /// Carves the tables off the RAM pool, themselves included (the
    /// loader's stack holds a pointer); `None` without it.
    pub fn new(pool: &'static mut [u32]) -> Option<&'static mut Self> {
        let (slots, pool) = carve(pool, HANDLES, Slot::default())?;
        let (index, pool) = carve(pool, 2 * HANDLES, 0u32)?;
        let (walk, pool) = carve::<Level>(pool, MAX_DEPTH, (0, 0, 0, false))?;
        let (staging, pool) = carve(pool, STAGING, 0u8)?;
        let (scratch, pool) = carve(pool, CARD_SCRATCH, 0u8)?;
        let (icon, pool) = carve(pool, ICON_SCRATCH, 0u8)?;
        let (format_ram, pool) = carve(pool, FORMAT_RAM, 0u8)?;
        let (state, pool) = carve(pool, 1, FatState::new())?;
        let mut serial = [b'0'; 32];
        serial[..MSC_SERIAL.len()].copy_from_slice(MSC_SERIAL);
        let id = Identity {
            manufacturer: "Dell",
            model: rp_hal_x51v::MODEL,
            version: env!("CARGO_PKG_VERSION"),
            serial,
        };
        let (tables, _) = carve_with(pool, || Self {
            mtp: Mtp::new(id, staging, slots, index, walk),
            state: &mut state[0],
            scratch,
            icon,
            format_ram,
            ui: Ui {
                keypad: None,
                held: Pressed::NONE,
                name: Text::from_text(rp_hal_x51v::MODEL).unwrap_or_default(),
                icon_at: None,
            },
        })?;
        Some(tables)
    }

    /// Store `s`'s card changed under the PC (swapped, formatted, repaired
    /// or written by the loader's tools): what is known of it goes, and the
    /// PC lists it afresh (rule 10).
    #[inline(never)]
    pub fn card_changed(&mut self, s: u8, present: bool) {
        self.state.forget(s);
        // Either card may hold the device icon.
        self.ui.icon_at = None;
        self.mtp.store_changed(s, false);
        if present {
            self.mtp.store_changed(s, true);
        }
    }

    /// One turn of the responder: the controller's class requests, then
    /// whatever can move.
    #[inline(never)]
    pub fn poll(
        &mut self,
        udc: &mut Udc<Mmio>,
        lun: &mut Lun,
        cf: &mut CfLun,
        log: &mut dyn Write,
    ) {
        let Self {
            mtp,
            state,
            scratch,
            icon,
            format_ram,
            ui,
        } = self;
        let mut cards = LoaderCards {
            lun,
            cf,
            ui,
            icon,
            format_ram,
            log,
        };
        let mut dev = FatDevice::new(&mut cards, scratch, state);
        let mut pipe = UsbPipe(udc);
        if let Some(tid) = pipe.0.take_mtp_cancel() {
            mtp.cancel(&mut dev, &mut pipe, tid);
        }
        if pipe.0.take_mtp_reset() {
            mtp.reset(&mut dev, &mut pipe);
        }
        if pipe.0.take_mtp_cleared() {
            mtp.cleared();
        }
        mtp.poll(&mut dev, &mut pipe);
        let mut st = [0u8; 12];
        mtp.device_status(&mut st);
        pipe.0.set_mtp_status(u16::from_le_bytes([st[2], st[3]]));
    }

    /// A USB bus reset or the cable gone: the session closes, a file being
    /// sent is dropped (rule 6).
    #[inline(never)]
    pub fn bus_reset(
        &mut self,
        udc: &mut Udc<Mmio>,
        lun: &mut Lun,
        cf: &mut CfLun,
        log: &mut dyn Write,
    ) {
        let Self {
            mtp,
            state,
            scratch,
            icon,
            format_ram,
            ui,
        } = self;
        let mut cards = LoaderCards {
            lun,
            cf,
            ui,
            icon,
            format_ram,
            log,
        };
        let mut dev = FatDevice::new(&mut cards, scratch, state);
        mtp.reset(&mut dev, &mut UsbPipe(udc));
    }

    /// The loader removed `loc` from store `s` (rule 10).
    #[inline(never)]
    pub fn removed(&mut self, s: u8, loc: u64) {
        self.state.space_changed(s);
        self.mtp.object_removed(s, loc);
    }

    /// The loader's `put` wrote `seen`'s path (rule 10): the PC hears of the
    /// new file or folder, or of the file's change.
    #[inline(never)]
    pub fn put_done(&mut self, seen: &Seen, lun: &mut Lun, cf: &mut CfLun, log: &mut dyn Write) {
        let s = seen.unit;
        let n = seen.after_n;
        if n == 0 {
            return;
        }
        // A file the PC is sending keeps its reservation: only the free
        // space is read again.
        self.state.space_changed(s);
        let Self {
            mtp,
            state,
            scratch,
            icon,
            format_ram,
            ui,
        } = self;
        let mut cards = LoaderCards {
            lun,
            cf,
            ui,
            icon,
            format_ram,
            log,
        };
        let mut dev = FatDevice::new(&mut cards, scratch, state);
        let parent = |k: usize| if k == 0 { ROOT } else { seen.after[k - 1] };
        if seen.before_n >= n {
            let (old, new) = (seen.before[n - 1], seen.after[n - 1]);
            if old == new {
                mtp.object_changed(s, new);
            } else {
                mtp.object_removed(s, old);
                mtp.object_added(&mut dev, s, parent(n - 1), new);
            }
        } else {
            // The first segment that is new: the PC lists what's inside it
            // when it opens it.
            let k = seen.before_n;
            mtp.object_added(&mut dev, s, parent(k), seen.after[k]);
        }
    }
}

/// Where `path` is on the SD card (the object the loader's `rm` removes).
#[inline(never)]
pub fn locate(lun: &mut Lun, path: &str) -> Option<u64> {
    let mut vol = mount_any(lun.card.as_mut()?).ok()?;
    let mut at = [0u64; DEPTH];
    let want = path.split(['/', '\\']).filter(|s| !s.is_empty()).count();
    (want > 0 && node_path(&mut vol, path, &mut at) == want).then(|| at[want - 1])
}

/// Where a path of the loader's own writes was, before and after (rule 10).
pub struct Seen {
    pub unit: u8,
    pub before: [u64; DEPTH],
    pub before_n: usize,
    pub after: [u64; DEPTH],
    pub after_n: usize,
}

impl Seen {
    pub const fn new() -> Self {
        Self {
            unit: 0,
            before: [0; DEPTH],
            before_n: 0,
            after: [0; DEPTH],
            after_n: 0,
        }
    }
}

/// The locations of `path`'s leading segments that exist on `vol`, into
/// `out`; how many.
#[inline(never)]
pub fn node_path(vol: &mut AnyVolume<Fed<'_>>, path: &str, out: &mut [u64; DEPTH]) -> usize {
    let mut dir = ROOT;
    let mut n = 0;
    let mut node = Node::default();
    for seg in path.split(['/', '\\']).filter(|s| !s.is_empty()) {
        if n == DEPTH {
            break;
        }
        let mut cursor = 0;
        let mut found = None;
        while let Ok(true) = vol.node_child(dir, &mut cursor, &mut node) {
            if same_name(node.name(), seg) {
                found = Some(node.loc);
                break;
            }
        }
        let Some(loc) = found else { break };
        out[n] = loc;
        n += 1;
        dir = loc;
    }
    n
}

/// A card name (UTF-16) against an ASCII path segment, ignoring case as
/// FAT does for ASCII.
fn same_name(units: &[u16], seg: &str) -> bool {
    units.len() == seg.len()
        && units
            .iter()
            .zip(seg.bytes())
            .all(|(&u, b)| u < 0x80 && (u as u8).eq_ignore_ascii_case(&b))
}

/// Whether any of `names` exists on `vol` (the boot files).
fn holds_any(vol: &mut AnyVolume<Fed<'_>>, names: &[&str]) -> bool {
    names.iter().any(|name| {
        let mut at = [0u64; DEPTH];
        let want = name.split(['/', '\\']).filter(|s| !s.is_empty()).count();
        node_path(vol, name, &mut at) == want
    })
}

/// The cards as MTP's stores, lent for one poll: 0 the SD card, 1 the CF
/// card.
struct LoaderCards<'a> {
    lun: &'a mut Lun,
    cf: &'a mut CfLun,
    ui: &'a mut Ui,
    icon: &'a mut [u8],
    format_ram: &'a mut [u8],
    log: &'a mut dyn Write,
}

impl LoaderCards<'_> {
    fn card_bytes(&self, s: u8) -> u64 {
        u64::from(if s == 0 {
            self.lun.blocks()
        } else {
            self.cf.blocks()
        }) * 512
    }

    /// Store `s`'s volume, when its card is in and mounts.
    fn volume<'b>(lun: &'b mut Lun, cf: &'b mut CfLun, s: u8) -> Option<AnyVolume<Fed<'b>>> {
        if s == 0 {
            if !lun.present() {
                return None;
            }
            mount_any(lun.card.as_mut()?).ok()
        } else {
            mount_cf(cf).ok()
        }
    }

    /// The device icon's card and size, looked for once per card change:
    /// the SD card's, else the CF card's.
    fn device_icon(&mut self) -> (u8, u32) {
        if let Some(at) = self.ui.icon_at {
            return at;
        }
        let mut at = (0, 0);
        for s in 0..2u8 {
            let mut sector = [0u8; 512];
            let size = Self::volume(self.lun, self.cf, s).and_then(|mut v| {
                v.read_range(ICON[usize::from(s)], 0, 0, &mut sector, &mut |_| {})
                    .ok()
            });
            // MTP's sizes are 32-bit: a bigger file is no icon.
            if let Some((size, _)) = size
                && let Ok(n @ 1..) = u32::try_from(size)
            {
                at = (s, n);
                break;
            }
        }
        self.ui.icon_at = Some(at);
        at
    }
}

/// The device icon on each card ([`DEVICE_ICON`], [`CF_DEVICE_ICON`]).
const ICON: [&str; 2] = [DEVICE_ICON, CF_DEVICE_ICON];

impl Cards for LoaderCards<'_> {
    type Disk<'b>
        = Fed<'b>
    where
        Self: 'b;

    fn stores(&self) -> u8 {
        2
    }

    fn disk(&mut self, s: u8) -> Option<(Self::Disk<'_>, u32)> {
        // The partition's start as the guard's rebuild read it (every card
        // change rebuilds it): no sector buffer on the stack per call.
        match s {
            0 if self.lun.present() => {
                let start = self.lun.part_start?;
                Some((Fed::PcSd(&mut *self.lun), start))
            }
            1 if self.cf.present() => {
                let start = self.cf.part_start?;
                Some((Fed::PcCf(&mut *self.cf), start))
            }
            _ => None,
        }
    }

    fn kind(&self, _s: u8) -> Kind {
        Kind::Removable
    }

    fn writable(&self, s: u8) -> bool {
        if s == 0 {
            !self.lun.read_only()
        } else {
            !self.cf.read_only()
        }
    }

    fn protected(&self, s: u8, sector: u32, offset: usize) -> bool {
        let guard = if s == 0 {
            self.lun.guard.as_ref()
        } else {
            self.cf.guard.as_ref()
        };
        guard.is_some_and(|g| g.protects_entry(sector, offset))
    }

    fn boots_from(&mut self, s: u8) -> bool {
        // A card that can't be read is never formatted from the PC: it
        // might be the boot card (Q3).
        if s == 0 {
            let Some(sd) = self.lun.card.as_mut() else {
                return true;
            };
            mount_any(sd).map_or(true, |mut v| holds_any(&mut v, &BOOT_FILES))
        } else {
            mount_cf(self.cf).map_or(true, |mut v| holds_any(&mut v, &CF_BOOT_FILES))
        }
    }

    fn description(&self, s: u8) -> &'static str {
        if s == 0 { "SD Card" } else { "CF Card" }
    }

    fn format_consent(&mut self, s: u8, start: bool) -> Consent {
        if start {
            if self.ui.keypad.is_none()
                && let Some(bus) = rt::take_keypad()
            {
                let mut k = Keypad::new(bus);
                k.enable();
                self.ui.keypad = Some(k);
            }
            let Some(k) = self.ui.keypad.as_mut() else {
                let _ = writeln!(self.log, "MTP: format refused (no keypad to confirm on)");
                return Consent::No;
            };
            self.ui.held = k.pressed();
            let mb = self.card_bytes(s) / 1_000_000;
            let _ = writeln!(
                self.log,
                "The PC asks to ERASE the {} ({mb} MB).",
                self.description(s)
            );
            let _ = writeln!(
                self.log,
                "Press the d-pad center to erase it, any other button to keep it (30 s)."
            );
            return Consent::Waiting;
        }
        let Some(k) = self.ui.keypad.as_mut() else {
            return Consent::No;
        };
        let now = k.pressed();
        let press = self.ui.held.changes(now).find(|&(_, down)| down);
        self.ui.held = now;
        match press {
            Some((Button::Enter, _)) => {
                let _ = writeln!(self.log, "Erasing the {} for the PC", self.description(s));
                Consent::Yes
            }
            Some(_) => {
                let _ = writeln!(
                    self.log,
                    "Kept the {}: the PC was refused",
                    self.description(s)
                );
                Consent::No
            }
            None => Consent::Waiting,
        }
    }

    fn format(&mut self, s: u8) -> Result<(), StoreError> {
        let mut reply = Line::new();
        let args: &[u8] = if s == 0 { b"sd auto" } else { b"cf auto" };
        let (ram, work) = self.format_ram.split_at_mut(FORMAT_RAM - SCRATCH);
        let wrote = format_command(self.lun, self.cf, args, ram, work, &mut reply);
        let _ = writeln!(self.log, "MTP format: {}", reply.as_str().trim_ascii_end());
        if s == 0 {
            self.lun.refresh_guard();
        } else {
            self.cf.refresh_guard();
        }
        if wrote {
            Ok(())
        } else {
            Err(StoreError::Device)
        }
    }

    fn friendly_name(&self) -> Text {
        self.ui.name
    }

    fn set_friendly_name(&mut self, name: &Text) {
        self.ui.name = *name;
    }

    fn icon_len(&mut self) -> u32 {
        self.device_icon().1
    }

    fn icon_read(&mut self, offset: u32, buf: &mut [u8]) -> usize {
        let (s, size) = self.device_icon();
        let want = buf.len().min(size.saturating_sub(offset) as usize);
        let mut k = 0;
        if let Some(mut vol) = Self::volume(self.lun, self.cf, s) {
            let _ = vol.read_range(
                ICON[usize::from(s)],
                u64::from(offset),
                want as u64,
                self.icon,
                &mut |p| {
                    let n = p.len().min(want - k);
                    buf[k..k + n].copy_from_slice(&p[..n]);
                    k += n;
                },
            );
        }
        if k < want {
            // The card went (or the file changed) mid-way: the length is
            // already announced, so the rest goes as zeros (Windows shows
            // its own icon) and the session keeps its framing.
            buf[k..want].fill(0);
            let _ = writeln!(
                self.log,
                "MTP: device icon cut short at {}",
                offset as usize + k
            );
        }
        want
    }

    fn now_ms(&self) -> u32 {
        now_ms()
    }
}

/// The controller's MTP endpoints as the responder's pipe: the bulk pair
/// (slots D and E, the drive's) and the event endpoint (slot F). CPU
/// packets for now; DMA as the drive has it is quality audit C86.
struct UsbPipe<'a>(&'a mut Udc<Mmio>);

impl Pipe for UsbPipe<'_> {
    fn read_packet(&mut self, buf: &mut [u8; PACKET]) -> Option<usize> {
        self.0.msc_read(buf)
    }
    fn can_write(&mut self) -> bool {
        self.0.msc_can_write()
    }
    fn write_packet(&mut self, data: &[u8]) {
        self.0.msc_write(data);
    }
    fn event_can_write(&mut self) -> bool {
        self.0.mtp_event_can_write()
    }
    fn event_write(&mut self, data: &[u8]) {
        self.0.mtp_event_write(data);
    }
    fn stall(&mut self) {
        self.0.mtp_stall();
    }
    fn flush(&mut self) {
        self.0.mtp_flush();
    }
    fn service(&mut self) {
        self.0.poll();
    }
}
