//! `emu-mtp` (ADR-0037 plan step 4): the loader's cards over MTP, through
//! the emulator's USB host playing Windows (it reads Microsoft's OS
//! descriptors, binds the interface named "MTP" and bridges it to a
//! chardev) and then a Mac (it binds nothing and must get the drive).
//!
//! Over the MTP chardev this is the PC's initiator, built on
//! `rp-usb-mtp`'s codes and data readers: it lists, reads, writes, cancels
//! a copy part-way, is refused a boot file's delete, hears the loader's own
//! `put` and a CF card's swap as events, and reads the device's name and
//! icon.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::time::{Duration, Instant};

use rp_usb_mtp::codes::{container, dprop, ev, fmt, op, rc};
use rp_usb_mtp::{Reader, Text, Writer};

use super::{
    Boot, Bot, LOADER_MARKER, boot, device_has, device_has_runtime, drive_arg, msc_args,
    poll_pause, raw_image, scratch, usb_args, usb_connect, usb_port, usb_read_line, usb_read_until,
    wm_cpld_table_args,
};

/// The stores' IDs: the SD card's and the CF card's [MTP 5.2.1].
const SD: u32 = 0x0001_0001;
const CF: u32 = 0x0002_0001;
/// The MTP interface (class requests' wIndex, errata 2007).
const IFACE: u8 = 2;

/// QEMU arguments for the UDC model's MTP chardev (retropane-qemu after
/// rp-v0.30.0).
fn mtp_args(n: u16) -> [String; 4] {
    [
        "-chardev".into(),
        format!(
            "socket,id=mtp,host=127.0.0.1,port={},server=on,wait=off",
            usb_port(n)
        ),
        "-global".into(),
        "pxa27x-udc.mtp=mtp".into(),
    ]
}

/// What the model sends: a bulk IN packet, an event, a control reply, a
/// stalled endpoint.
enum Frame {
    In(Vec<u8>),
    Event(Vec<u8>),
    Control(bool, Vec<u8>),
    Stall(u8),
}

/// One operation's outcome.
struct Answer {
    rc: u16,
    params: Vec<u32>,
    data: Vec<u8>,
}

/// The PC's side of MTP over the model's tagged frames.
struct Initiator {
    s: TcpStream,
    tid: u32,
    /// Events as they came: code and parameters.
    events: Vec<(u16, Vec<u32>)>,
    ins: std::collections::VecDeque<Vec<u8>>,
    controls: std::collections::VecDeque<(bool, Vec<u8>)>,
    stalls: Vec<u8>,
}

impl Initiator {
    fn connect(n: u16) -> Result<Self, String> {
        Ok(Self {
            s: usb_connect(n)?,
            tid: 0,
            events: Vec::new(),
            ins: Default::default(),
            controls: Default::default(),
            stalls: Vec::new(),
        })
    }

    fn read_exact(&mut self, buf: &mut [u8], deadline: Instant) -> Result<bool, String> {
        let mut got = 0;
        while got < buf.len() {
            if Instant::now() > deadline {
                if got == 0 {
                    return Ok(false);
                }
                return Err("MTP: a frame cut short".into());
            }
            match self.s.read(&mut buf[got..]) {
                Ok(0) => return Err("MTP: connection closed".into()),
                Ok(n) => got += n,
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        Ok(true)
    }

    /// The next frame, sorted into its queue; false when none came in time.
    fn pump(&mut self, deadline: Instant) -> Result<bool, String> {
        let mut h = [0u8; 3];
        if !self.read_exact(&mut h, deadline)? {
            return Ok(false);
        }
        let mut p = vec![0u8; usize::from(u16::from_le_bytes([h[1], h[2]]))];
        let end = Instant::now() + Duration::from_secs(5);
        if !p.is_empty() && !self.read_exact(&mut p, end)? {
            return Err("MTP: a frame's payload never came".into());
        }
        match (h[0], self.frame(h[0], p)?) {
            (_, Frame::In(p)) => self.ins.push_back(p),
            (_, Frame::Event(p)) => {
                if p.len() < 12 {
                    return Err(format!("MTP: a short event {p:x?}"));
                }
                let params = p[12..]
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|c| u32::from_le_bytes(*c))
                    .collect();
                self.events.push((u16::from_le_bytes([p[6], p[7]]), params));
            }
            (_, Frame::Control(stalled, d)) => self.controls.push_back((stalled, d)),
            (_, Frame::Stall(ep)) => self.stalls.push(ep),
        }
        Ok(true)
    }

    fn frame(&self, kind: u8, p: Vec<u8>) -> Result<Frame, String> {
        Ok(match kind {
            0x81 => Frame::In(p),
            0x82 => Frame::Event(p),
            0x83 => {
                let stalled = p.first().is_some_and(|&b| b != 0);
                Frame::Control(stalled, p.get(1..).unwrap_or(&[]).to_vec())
            }
            0x84 => Frame::Stall(p.first().copied().unwrap_or(0)),
            k => return Err(format!("MTP: an unknown frame type {k:#x}")),
        })
    }

    /// Sends a bulk OUT transfer (`whole`: the model closes it with a
    /// zero-length packet when it ends on a whole one).
    fn send(&mut self, data: &[u8], whole: bool) -> Result<(), String> {
        let mut f = vec![if whole { 0x01 } else { 0x03 }];
        f.extend_from_slice(&(data.len() as u32).to_le_bytes());
        f.extend_from_slice(data);
        self.s.write_all(&f).map_err(|e| e.to_string())
    }

    /// A control request on endpoint 0: whether it stalled, and its data.
    fn control(&mut self, setup: [u8; 8], data: &[u8]) -> Result<(bool, Vec<u8>), String> {
        let mut f = vec![0x02];
        f.extend_from_slice(&((8 + data.len()) as u32).to_le_bytes());
        f.extend_from_slice(&setup);
        f.extend_from_slice(data);
        self.s.write_all(&f).map_err(|e| e.to_string())?;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(r) = self.controls.pop_front() {
                return Ok(r);
            }
            if !self.pump(deadline)? {
                return Err(format!("MTP: no reply to control request {setup:x?}"));
            }
        }
    }

    fn packet(&mut self, deadline: Instant) -> Result<Vec<u8>, String> {
        loop {
            if let Some(p) = self.ins.pop_front() {
                return Ok(p);
            }
            if let Some(ep) = self.stalls.pop() {
                return Err(format!("MTP: endpoint {ep:#x} stalled"));
            }
            if !self.pump(deadline)? {
                return Err("MTP: no answer".into());
            }
        }
    }

    /// One container from the device, its closing zero-length packet taken.
    fn container_in(&mut self, deadline: Instant) -> Result<Vec<u8>, String> {
        let mut c = self.packet(deadline)?;
        if c.len() < 12 {
            return Err(format!("MTP: a container of {} bytes", c.len()));
        }
        let len = u32::from_le_bytes([c[0], c[1], c[2], c[3]]) as usize;
        let mut last = c.len();
        while c.len() < len {
            let p = self.packet(deadline)?;
            last = p.len();
            if p.is_empty() {
                return Err(format!("MTP: a container ended at {} of {len}", c.len()));
            }
            c.extend_from_slice(&p);
        }
        if last == 64 {
            let zlp = self.packet(deadline)?;
            if !zlp.is_empty() {
                return Err(format!("MTP: no zero-length packet after {len} bytes"));
            }
        }
        Ok(c)
    }

    fn command(&mut self, code: u16, params: &[u32]) -> Result<u32, String> {
        let tid = if code == op::OPEN_SESSION || (code == op::GET_DEVICE_INFO && self.tid == 0) {
            0
        } else {
            self.tid += 1;
            self.tid
        };
        let mut c = container_head(container::COMMAND, code, tid, 4 * params.len());
        for p in params {
            c.extend_from_slice(&p.to_le_bytes());
        }
        self.send(&c, true)?;
        Ok(tid)
    }

    /// Runs one operation: command, the data out if any, the data in if
    /// any, the response.
    fn op(&mut self, code: u16, params: &[u32], out: Option<&[u8]>) -> Result<Answer, String> {
        let tid = self.command(code, params)?;
        if let Some(d) = out {
            let mut c = container_head(container::DATA, code, tid, d.len());
            c.extend_from_slice(d);
            self.send(&c, true)?;
        }
        self.finish(code, tid)
    }

    fn finish(&mut self, code: u16, tid: u32) -> Result<Answer, String> {
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut data = Vec::new();
        loop {
            let c = self.container_in(deadline)?;
            let kind = u16::from_le_bytes([c[4], c[5]]);
            let ctid = u32::from_le_bytes([c[8], c[9], c[10], c[11]]);
            if ctid != tid {
                return Err(format!("MTP {code:#06x}: transaction {ctid}, wanted {tid}"));
            }
            match kind {
                container::DATA => data = c[12..].to_vec(),
                container::RESPONSE => {
                    let params = c[12..]
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|p| u32::from_le_bytes(*p))
                        .collect();
                    return Ok(Answer {
                        rc: u16::from_le_bytes([c[6], c[7]]),
                        params,
                        data,
                    });
                }
                k => return Err(format!("MTP {code:#06x}: container type {k}")),
            }
        }
    }

    /// An operation that must answer OK.
    fn ok(&mut self, code: u16, params: &[u32], out: Option<&[u8]>) -> Result<Answer, String> {
        let a = self.op(code, params, out)?;
        if a.rc != rc::OK {
            return Err(format!("MTP {code:#06x} {params:x?}: {:#06x}", a.rc));
        }
        Ok(a)
    }

    /// Waits for an event `code` (with `param` first, if given).
    fn event(&mut self, code: u16, param: Option<u32>, secs: u64) -> Result<Vec<u32>, String> {
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            if let Some(i) = self
                .events
                .iter()
                .position(|(c, p)| *c == code && param.is_none_or(|v| p.first() == Some(&v)))
            {
                return Ok(self.events.remove(i).1);
            }
            if Instant::now() > deadline {
                return Err(format!(
                    "MTP: no event {code:#06x} {param:x?}; got {:x?}",
                    self.events
                ));
            }
            self.pump(Instant::now() + Duration::from_millis(200))?;
        }
    }

    /// Get Device Status [SIC 5.2.4]: the code.
    fn status(&mut self) -> Result<u16, String> {
        let (stalled, d) = self.control([0xA1, 0x67, 0, 0, IFACE, 0, 12, 0], &[])?;
        if stalled || d.len() < 4 {
            return Err(format!("Get Device Status: stalled {stalled}, {d:x?}"));
        }
        Ok(u16::from_le_bytes([d[2], d[3]]))
    }
}

fn container_head(kind: u16, code: u16, tid: u32, payload: usize) -> Vec<u8> {
    let mut c = ((12 + payload) as u32).to_le_bytes().to_vec();
    c.extend_from_slice(&kind.to_le_bytes());
    c.extend_from_slice(&code.to_le_bytes());
    c.extend_from_slice(&tid.to_le_bytes());
    c
}

fn text(r: &mut Reader<'_>) -> Result<String, String> {
    let t = r.text().map_err(|_| "MTP: a bad string")?;
    Ok(String::from_utf16_lossy(t.units()))
}

/// What GetObjectInfo says [MTP 5.3.1].
struct Info {
    format: u16,
    protection: u16,
    size: u32,
    parent: u32,
    name: String,
}

fn object_info(d: &[u8]) -> Result<Info, String> {
    let bad = |_| "MTP: a short ObjectInfo".to_string();
    let mut r = Reader::new(d);
    r.u32().map_err(bad)?;
    let format = r.u16().map_err(bad)?;
    let protection = r.u16().map_err(bad)?;
    let size = r.u32().map_err(bad)?;
    r.skip(2 + 6 * 4).map_err(bad)?;
    let parent = r.u32().map_err(bad)?;
    r.skip(2 + 4 + 4).map_err(bad)?;
    Ok(Info {
        format,
        protection,
        size,
        parent,
        name: text(&mut r)?,
    })
}

/// An ObjectInfo for SendObjectInfo.
fn new_object(name: &str, size: u32) -> Vec<u8> {
    let mut b = vec![0u8; 512];
    let mut w = Writer::new(&mut b);
    w.u32(0);
    w.u16(fmt::UNDEFINED);
    w.u16(0);
    w.u32(size);
    w.u16(0);
    for _ in 0..6 {
        w.u32(0);
    }
    w.u32(0);
    w.u16(0);
    w.u32(0);
    w.u32(0);
    w.text(&Text::from_text(name).unwrap_or_default());
    w.text(&Text::new());
    w.ascii(b"20261025T221500");
    w.u8(0);
    let n = w.len();
    b.truncate(n);
    b
}

/// Free bytes on `store` (GetStorageInfo [MTP 5.2.2]).
fn free_space(m: &mut Initiator, store: u32) -> Result<(u64, String), String> {
    let a = m.ok(op::GET_STORAGE_INFO, &[store], None)?;
    let bad = |_| "MTP: a short StorageInfo".to_string();
    let mut r = Reader::new(&a.data);
    r.skip(6).map_err(bad)?;
    r.u64().map_err(bad)?;
    let free = r.u64().map_err(bad)?;
    r.u32().map_err(bad)?;
    Ok((free, text(&mut r)?))
}

/// `store`'s StorageType and FilesystemType (GetStorageInfo [MTP 5.2.2]).
fn storage_codes(m: &mut Initiator, store: u32) -> Result<(u16, u16), String> {
    let a = m.ok(op::GET_STORAGE_INFO, &[store], None)?;
    let bad = |_| "MTP: a short StorageInfo".to_string();
    let mut r = Reader::new(&a.data);
    Ok((r.u16().map_err(bad)?, r.u16().map_err(bad)?))
}

/// The store's root, by name.
fn listing(m: &mut Initiator, store: u32) -> Result<Vec<(u32, Info)>, String> {
    let a = m.ok(op::GET_OBJECT_HANDLES, &[store, 0, u32::MAX], None)?;
    let mut r = Reader::new(&a.data);
    let n = r.u32().map_err(|_| "MTP: a short handle list")?;
    let mut out = Vec::new();
    for _ in 0..n {
        let h = r.u32().map_err(|_| "MTP: a short handle list")?;
        let i = m.ok(op::GET_OBJECT_INFO, &[h], None)?;
        out.push((h, object_info(&i.data)?));
    }
    Ok(out)
}

fn find<'a>(list: &'a [(u32, Info)], name: &str) -> Result<&'a (u32, Info), String> {
    list.iter()
        .find(|(_, i)| i.name == name)
        .ok_or_else(|| format!("MTP: {name} not listed"))
}

fn pattern(len: usize, seed: u32) -> Vec<u8> {
    (0..len as u32)
        .map(|i| (i.wrapping_mul(29) ^ seed) as u8)
        .collect()
}

/// The loader's reply to `cmd` (its `RPL` line).
fn say(s: &mut TcpStream, cmd: &str, want: &str) -> Result<String, String> {
    s.write_all(&rp_loadproto::control_frame(cmd))
        .map_err(|e| e.to_string())?;
    let reply = usb_read_line(s, "RPL ", 20)?;
    let line = reply
        .lines()
        .find(|l| l.starts_with("RPL "))
        .unwrap_or("")
        .to_string();
    if !line.starts_with(want) {
        return Err(format!("`{cmd}`: {line:?} (wanted {want:?})"));
    }
    Ok(line)
}

/// A number after `key` in `line`.
fn field(line: &str, key: &str) -> Option<u64> {
    let t = line.split(key).nth(1)?.trim_start();
    let t = t.split([' ', '\r']).next()?;
    match t.strip_prefix("0x") {
        Some(h) => u64::from_str_radix(h, 16).ok(),
        None => t.parse().ok(),
    }
}

pub fn boot_loader_mtp(root: &Path) -> Result<(), String> {
    if !device_has(root, "pxa27x-udc", "mtp") {
        eprintln!(
            "    MTP: skipped (this emulator's USB host has no MTP model: retropane-qemu after rp-v0.30.0)"
        );
        return Ok(());
    }
    windows(root)?;
    split(root)?;
    mac(root)
}

/// The SD card as the owner's: boot files, a README, the drive's icon and
/// the device's (the real one: 432 KB, served in many unaligned pieces).
fn sd_card(root: &Path, name: &str) -> Result<(std::path::PathBuf, Vec<u8>), String> {
    let entry = |name, data| rp_fat::format::Entry { name, data };
    let icon = std::fs::read(root.join(crate::sdkit::DEVICE_ICON_ASSET))
        .map_err(|e| format!("{}: {e}", crate::sdkit::DEVICE_ICON_ASSET))?;
    let drive_icon = pattern(5000, 0x1C0);
    let image = rp_fat::format::build(&rp_fat::format::Image {
        total_sectors: 64 * 2048,
        sectors_per_cluster: 8,
        files: &[
            entry("retropane.bin", &pattern(70_000, 1)),
            entry("haret.exe", &pattern(30_000, 2)),
            entry("startup.txt", b"set ramaddr 0xa8000000\n"),
            entry(
                "autorun.inf",
                b"[autorun]\r\nicon=card-sd.ico\r\nlabel=X51V\r\n",
            ),
            entry("card-sd.ico", &drive_icon),
            entry("device.ico", &icon),
            entry("README.txt", b"hello from the card"),
        ],
        blocked_clusters: &[],
        ballast: 0,
    });
    let sd = scratch(root, name)?;
    std::fs::write(&sd, image).map_err(|e| format!("{}: {e}", sd.display()))?;
    Ok((sd, icon))
}

fn cf_card(root: &Path, name: &str) -> Result<std::path::PathBuf, String> {
    let image = rp_fat::format::build(&rp_fat::format::Image {
        total_sectors: 32 * 1024,
        sectors_per_cluster: 8,
        files: &[rp_fat::format::Entry {
            name: "on-cf.txt",
            data: b"compact flash",
        }],
        blocked_clusters: &[],
        ballast: 0,
    });
    let cf = scratch(root, name)?;
    std::fs::write(&cf, image).map_err(|e| format!("{}: {e}", cf.display()))?;
    Ok(cf)
}

/// A host like Windows 11: it reads Microsoft's descriptors (the OS 2.0
/// set too) and opens a session.
fn windows(root: &Path) -> Result<(), String> {
    let loader = raw_image(root, "rp-loader")?;
    let (sd, icon) = sd_card(root, "sd-mtp.img")?;
    let cf = cf_card(root, "cf-mtp.img")?;
    let mut args: Vec<String> = usb_args(17).into();
    args.extend(mtp_args(18));
    args.push("-drive".into());
    args.push(drive_arg(root, "ide", &cf)?);
    args.extend(wm_cpld_table_args());
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let check = || -> Result<(), String> {
        let mut s = usb_connect(17)?;
        usb_read_until(&mut s, LOADER_MARKER, 10)?;
        eprintln!(
            "    stack at start: {}",
            say(&mut s, "loader stack", "RPL OK stack ")?
        );
        say(&mut s, "drive auto", "RPL OK drive auto")?;
        let mut m = Initiator::connect(18)?;
        // Rule 4: past the wait with no session, still MTP (only Windows
        // reads the OS 2.0 set, so the device knows); the time is the
        // guest's own, read from the loader.
        let deadline = Instant::now() + Duration::from_secs(60);
        let line = loop {
            let line = say(&mut s, "drive mtp status", "RPL OK mtp")?;
            if line.contains("layout mtp")
                && field(&line, "no session for ").is_some_and(|ms| ms > 9_000)
            {
                break line;
            }
            if Instant::now() > deadline {
                return Err(format!(
                    "drive auto: never 9 s in MTP without a session: {line:?}"
                ));
            }
            poll_pause(Duration::from_millis(500));
        };
        if field(&line, "ms descriptors ") != Some(0xF) {
            return Err(format!("Microsoft's descriptors not all read: {line:?}"));
        }
        // GetDeviceInfo, before a session [MTP 5.1.1].
        let a = m.ok(op::GET_DEVICE_INFO, &[], None)?;
        let mut r = Reader::new(&a.data);
        let bad = |_| "MTP: a short DeviceInfo".to_string();
        r.u16().map_err(bad)?;
        let vendor = r.u32().map_err(bad)?;
        let version = r.u16().map_err(bad)?;
        let ext = text(&mut r)?;
        if (vendor, version, ext.as_str()) != (6, 100, "microsoft.com: 1.0; ") {
            return Err(format!("DeviceInfo extension: {vendor} {version} {ext:?}"));
        }
        r.u16().map_err(bad)?;
        for _ in 0..5 {
            let n = r.u32().map_err(bad)?;
            r.skip(2 * n as usize).map_err(bad)?;
        }
        let who = [text(&mut r)?, text(&mut r)?, text(&mut r)?, text(&mut r)?];
        if who[0] != "Dell" || who[1] != "Axim X51v" || who[3].len() != 32 {
            return Err(format!("DeviceInfo identity: {who:?}"));
        }
        m.ok(op::OPEN_SESSION, &[1], None)?;
        let ids = m.ok(op::GET_STORAGE_IDS, &[], None)?.data;
        if ids != [2, 0, 0, 0, 1, 0, 1, 0, 1, 0, 2, 0] {
            return Err(format!("storage IDs: {ids:x?}"));
        }
        let (free0, desc) = free_space(&mut m, SD)?;
        if desc != "SD Card" {
            return Err(format!("the SD store's description: {desc:?}"));
        }
        // Listing and reading; the boot files read-only (Q2).
        let list = listing(&mut m, SD)?;
        let (boot, info) = find(&list, "retropane.bin")?;
        if info.protection != 0x0001 || info.size != 70_000 || info.parent != 0 {
            return Err(format!(
                "retropane.bin: protection {:#x}, {} bytes, parent {}",
                info.protection, info.size, info.parent
            ));
        }
        let boot = *boot;
        let (_, info) = find(&list, "device.ico")?;
        if info.protection != 0x0001 {
            return Err("the device's icon listed writable".into());
        }
        let (readme, info) = find(&list, "README.txt")?;
        if info.protection != 0 {
            return Err("README.txt listed read-only".into());
        }
        if m.ok(op::GET_OBJECT, &[*readme], None)?.data != b"hello from the card" {
            return Err("README.txt reads wrong".into());
        }
        if list.iter().any(|(_, i)| i.name.starts_with("~RPMT")) {
            return Err("a temporary entry is listed".into());
        }
        // A file in, read back, the space it took gone.
        let body = pattern(300_000, 7);
        let a = m.ok(
            op::SEND_OBJECT_INFO,
            &[SD, u32::MAX],
            Some(&new_object("new.bin", 300_000)),
        )?;
        let new = a.params[2];
        m.ok(op::SEND_OBJECT, &[], Some(&body))?;
        if m.ok(op::GET_OBJECT, &[new], None)?.data != body {
            return Err("new.bin reads back wrong".into());
        }
        let (free1, _) = free_space(&mut m, SD)?;
        if free1 >= free0 {
            return Err(format!("free space after new.bin: {free1} (was {free0})"));
        }
        // A boot file can't be deleted: refused at once (rule 5).
        let a = m.op(op::DELETE_OBJECT, &[boot, 0], None)?;
        if a.rc != rc::OBJECT_WRITE_PROTECTED {
            return Err(format!("deleting retropane.bin: {:#06x}", a.rc));
        }
        if m.ok(op::GET_OBJECT, &[boot], None)?.data != pattern(70_000, 1) {
            return Err("retropane.bin changed".into());
        }
        m.ok(op::DELETE_OBJECT, &[new, 0], None)?;
        // A copy cancelled part-way (rule 6): no file, and once the session
        // ends, no space lost.
        let a = m.ok(
            op::SEND_OBJECT_INFO,
            &[SD, u32::MAX],
            Some(&new_object("cancel.bin", 200_000)),
        )?;
        let _ = a.params[2];
        let tid = m.command(op::SEND_OBJECT, &[])?;
        let mut part = container_head(container::DATA, op::SEND_OBJECT, tid, 200_000);
        part.extend_from_slice(&pattern(64 * 100 - 12, 9));
        m.send(&part, false)?;
        let mut cancel = vec![0x01, 0x40];
        cancel.extend_from_slice(&tid.to_le_bytes());
        let (stalled, _) = m.control([0x21, 0x64, 0, 0, IFACE, 0, 6, 0], &cancel)?;
        if stalled {
            return Err("the Cancel request stalled".into());
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let code = m.status()?;
            if code == rc::OK {
                break;
            }
            // The device may halt its bulk pair to end the transfer: the
            // host clears what it lists (as Windows does) and asks again.
            if code == rc::TRANSACTION_CANCELLED {
                let (_, d) = m.control([0xA1, 0x67, 0, 0, IFACE, 0, 12, 0], &[])?;
                for ep in d.get(4..).unwrap_or(&[]).as_chunks::<4>().0 {
                    let (stalled, _) = m.control([0x02, 0x01, 0, 0, ep[0], 0, 0, 0], &[])?;
                    if stalled {
                        return Err(format!("clearing endpoint {:#x}'s halt stalled", ep[0]));
                    }
                }
                m.stalls.clear();
            }
            if Instant::now() > deadline {
                return Err(format!(
                    "after a cancel: the device says {code:#06x}, never OK (stalls {:x?})",
                    m.stalls
                ));
            }
            poll_pause(Duration::from_millis(100));
        }
        m.ins.clear();
        let list = listing(&mut m, SD)?;
        if list
            .iter()
            .any(|(_, i)| i.name == "cancel.bin" || i.name.starts_with("~RPMT"))
        {
            return Err("a cancelled copy left a file".into());
        }
        m.ok(op::CLOSE_SESSION, &[], None)?;
        m.ok(op::OPEN_SESSION, &[2], None)?;
        let (free2, _) = free_space(&mut m, SD)?;
        if free2 != free0 {
            return Err(format!("free space after the cancel: {free2}, was {free0}"));
        }
        // The loader's own `put`: an event, at once (rule 10).
        listing(&mut m, SD)?;
        s.write_all(&rp_loadproto::file_frame("put.txt", b"from the loader"))
            .map_err(|e| e.to_string())?;
        usb_read_line(&mut s, "RPL OK ", 30)?;
        let p = m.event(ev::OBJECT_ADDED, None, 20)?;
        let info = object_info(&m.ok(op::GET_OBJECT_INFO, &[p[0]], None)?.data)?;
        if info.name != "put.txt" || info.format == fmt::ASSOCIATION {
            return Err(format!("the put's event names {:?}", info.name));
        }
        // The device's name (Q4) and icon (Q5: device.ico byte for byte,
        // not the drive's icon).
        let a = m.ok(
            op::GET_DEVICE_PROP_VALUE,
            &[u32::from(dprop::FRIENDLY_NAME)],
            None,
        )?;
        let name = text(&mut Reader::new(&a.data))?;
        if name != "Axim X51v" {
            return Err(format!("friendly name: {name:?}"));
        }
        let a = m.ok(
            op::GET_DEVICE_PROP_VALUE,
            &[u32::from(dprop::DEVICE_ICON)],
            None,
        )?;
        if a.data.get(4..) != Some(&icon[..]) {
            return Err(format!("device icon: {} bytes", a.data.len()));
        }
        // The CF card pulled out and pushed back in: the PC hears both.
        let mut mon = crate::screen::Monitor::connect(super::monitor_port())?;
        if device_has_runtime(&mut mon, "/machine/peripheral/cfslot", "pressed") {
            mon.query("qom-set /machine/peripheral/cfslot pressed true")?;
            m.event(ev::STORE_REMOVED, Some(0x0002_FFFF), 30)?;
            mon.query("qom-set /machine/peripheral/cfslot pressed false")?;
            m.event(ev::STORE_ADDED, Some(CF), 30)?;
        }
        // The icon experiment (R61): a probe's codes reach the PC with the
        // events that make it read them again; probe 0 puts the standard
        // ones back.
        say(&mut s, "drive mtp probe 1", "RPL OK mtp probe 1")?;
        m.event(ev::STORE_ADDED, Some(SD), 30)?;
        if storage_codes(&mut m, SD)? != (0x0004, 0x0003) {
            return Err("probe 1: the SD store doesn't report DCF".into());
        }
        say(&mut s, "drive mtp probe 0", "RPL OK mtp probe 0")?;
        m.event(ev::STORE_ADDED, Some(SD), 30)?;
        if storage_codes(&mut m, SD)? != (0x0004, 0x0002) {
            return Err("probe 0: the SD store's standard codes aren't back".into());
        }
        // The loader's stack after all of it: under 75%.
        let line = say(&mut s, "loader stack", "RPL OK stack ")?;
        let used = field(&line, "RPL OK stack ").unwrap_or(u64::MAX);
        let size = field(&line, " of ").unwrap_or(0);
        eprintln!("    loader stack after MTP: {used} of {size} bytes");
        if used * 4 > size * 3 {
            return Err(format!("the loader's stack after MTP: {line:?} (over 75%)"));
        }
        let line = say(&mut s, "drive mtp status", "RPL OK mtp")?;
        eprintln!("    {line}");
        Ok(())
    };
    boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: Some(&sd),
            steps: &[],
            extra: &extra,
            input: None,
            during: Some(&check),
            then: None,
            timeout: Duration::from_secs(240),
        },
    )?;
    eprintln!(
        "    MTP OK (Windows): stayed MTP past the wait (OS 2.0 set read); device info, both stores, listed and read, a file in and back, a boot file's delete refused, a cancelled copy left nothing, the loader's put and a CF swap as events, name and icon"
    );
    Ok(())
}

/// ADR-0037's split try-out on a host like Windows 11: the cards come as
/// drives (both units mount) and the device over MTP beside them, listing
/// no store, with the PDA icon read off the card the drive serves.
fn split(root: &Path) -> Result<(), String> {
    let loader = raw_image(root, "rp-loader")?;
    let (sd, icon) = sd_card(root, "sd-mtp-split.img")?;
    let cf = cf_card(root, "cf-mtp-split.img")?;
    let mut args: Vec<String> = usb_args(17).into();
    args.extend(mtp_args(18));
    args.extend(msc_args(20));
    args.push("-drive".into());
    args.push(drive_arg(root, "ide", &cf)?);
    args.extend(wm_cpld_table_args());
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let check = || -> Result<(), String> {
        let mut s = usb_connect(17)?;
        usb_read_until(&mut s, LOADER_MARKER, 10)?;
        say(&mut s, "drive split", "RPL OK drive split")?;
        let mut m = Initiator::connect(18)?;
        let mut bot = Bot::connect(20)?;
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let line = say(&mut s, "drive mtp status", "RPL OK mtp")?;
            if line.contains("mode split layout split")
                && field(&line, "ms descriptors ") == Some(0xF)
            {
                break;
            }
            if Instant::now() > deadline {
                return Err(format!("drive split: never configured as split: {line:?}"));
            }
            poll_pause(Duration::from_millis(500));
        }
        // The device: a session, no store, the PDA icon.
        m.ok(op::GET_DEVICE_INFO, &[], None)?;
        m.ok(op::OPEN_SESSION, &[1], None)?;
        let ids = m.ok(op::GET_STORAGE_IDS, &[], None)?.data;
        if ids != [0, 0, 0, 0] {
            return Err(format!("split: MTP lists stores: {ids:x?}"));
        }
        let a = m.ok(
            op::GET_DEVICE_PROP_VALUE,
            &[u32::from(dprop::DEVICE_ICON)],
            None,
        )?;
        if a.data.get(4..) != Some(&icon[..]) {
            return Err(format!("split: device icon {} bytes", a.data.len()));
        }
        // The cards: both units mount as drives.
        let sd_sectors = bot.mount()?;
        bot.lun = 1;
        let cf_sectors = bot.mount()?;
        if sd_sectors != 64 * 2048 || cf_sectors == 0 {
            return Err(format!(
                "split: drives of {sd_sectors} and {cf_sectors} sectors"
            ));
        }
        let line = say(&mut s, "drive mtp status", "RPL OK mtp")?;
        eprintln!("    {line}");
        Ok(())
    };
    boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: Some(&sd),
            steps: &[],
            extra: &extra,
            input: None,
            during: Some(&check),
            then: None,
            timeout: Duration::from_secs(240),
        },
    )?;
    eprintln!(
        "    MTP OK (split): the device over MTP with no store and the PDA icon, both cards mounted as drives beside it"
    );
    Ok(())
}

/// A host like a Mac: no Microsoft descriptors, no MTP: the drive comes.
fn mac(root: &Path) -> Result<(), String> {
    let loader = raw_image(root, "rp-loader")?;
    let (sd, _) = sd_card(root, "sd-mtp-mac.img")?;
    let mut args: Vec<String> = usb_args(19).into();
    args.extend(msc_args(20));
    args.push("-global".into());
    args.push("pxa27x-udc.mtp-host=off".into());
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let check = || -> Result<(), String> {
        let mut s = usb_connect(19)?;
        usb_read_until(&mut s, LOADER_MARKER, 10)?;
        say(&mut s, "drive auto", "RPL OK drive auto")?;
        let mut bot = Bot::connect(20)?;
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            let line = say(&mut s, "drive mtp status", "RPL OK mtp")?;
            if line.contains("layout composite") {
                if field(&line, "ms descriptors ") != Some(0) {
                    return Err(format!("a Mac read Microsoft's descriptors: {line:?}"));
                }
                break;
            }
            if Instant::now() > deadline {
                return Err(format!("drive auto on a Mac: no drive offered: {line:?}"));
            }
            poll_pause(Duration::from_millis(500));
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        while bot.command(&[0; 6], 0, false, &[])?.status != 0 {
            let _ = bot.sense()?;
            if Instant::now() > deadline {
                return Err("the drive never became ready on the Mac".into());
            }
            poll_pause(Duration::from_millis(200));
        }
        Ok(())
    };
    boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: Some(&sd),
            steps: &[],
            extra: &extra,
            input: None,
            during: Some(&check),
            then: None,
            timeout: Duration::from_secs(150),
        },
    )?;
    eprintln!(
        "    MTP OK (Mac): no session, no Microsoft descriptors: the drive offered and ready"
    );
    Ok(())
}
