//! Host tests: an in-memory device with two stores, and a scripted PC that
//! speaks the Still Image transport packet by packet.

use super::*;
use std::collections::VecDeque;

// --- The device ----------------------------------------------------------

#[derive(Clone, Debug)]
struct Node {
    store: u8,
    parent: Loc,
    name: String,
    folder: bool,
    data: Vec<u8>,
    protected: bool,
    hidden: bool,
    gone: bool,
    /// Reserved (not yet committed): invisible.
    reserved: bool,
    modified: Option<Time>,
}

struct Mem {
    nodes: Vec<Node>,
    present: [bool; 2],
    capacity: [u64; 2],
    writable: [bool; 2],
    boots: [bool; 2],
    consent: VecDeque<Consent>,
    asked: u32,
    formatted: Vec<u8>,
    now: u32,
    name: Text,
    icon: Vec<u8>,
    initiator: Option<String>,
    /// Fail the next write with this.
    fail_write: Option<StoreError>,
    writes: u32,
}

fn t(s: &str) -> Text {
    Text::from_text(s).unwrap()
}

impl Mem {
    fn new() -> Self {
        let mut m = Self {
            nodes: Vec::new(),
            present: [true, true],
            capacity: [1 << 20, 1 << 20],
            writable: [true, true],
            boots: [true, false],
            consent: VecDeque::new(),
            asked: 0,
            formatted: Vec::new(),
            now: 0,
            name: t("Axim X51v"),
            icon: Vec::new(),
            initiator: None,
            fail_write: None,
            writes: 0,
        };
        m.add(0, ROOT, "haret.exe", false, b"HARET", true);
        let docs = m.add(0, ROOT, "Docs", true, b"", false);
        m.add(0, docs, "a.txt", false, b"alpha", false);
        let sub = m.add(0, docs, "Sub", true, b"", false);
        m.add(0, sub, "deep.png", false, &[7u8; 300], false);
        m.add(
            0,
            ROOT,
            "big.bin",
            false,
            &(0..5000u32).map(|i| i as u8).collect::<Vec<_>>(),
            false,
        );
        m.add(1, ROOT, "cf.txt", false, b"cf", false);
        m
    }

    fn add(
        &mut self,
        store: u8,
        parent: Loc,
        name: &str,
        folder: bool,
        data: &[u8],
        protected: bool,
    ) -> Loc {
        self.nodes.push(Node {
            store,
            parent,
            name: name.into(),
            folder,
            data: data.to_vec(),
            protected,
            hidden: protected,
            gone: false,
            reserved: false,
            modified: None,
        });
        self.nodes.len() as Loc
    }

    fn node(&self, s: u8, loc: Loc) -> Result<&Node, StoreError> {
        let n = self
            .nodes
            .get(loc as usize - 1)
            .ok_or(StoreError::NotFound)?;
        if n.store != s || n.gone {
            return Err(StoreError::NotFound);
        }
        Ok(n)
    }

    fn used(&self, s: u8) -> u64 {
        self.nodes
            .iter()
            .filter(|n| n.store == s && !n.gone)
            .map(|n| {
                n.data
                    .len()
                    .max(if n.reserved { n.data.capacity() } else { 0 }) as u64
            })
            .sum()
    }

    fn find(&self, s: u8, path: &str) -> Option<Loc> {
        let mut dir = ROOT;
        let mut found = None;
        for part in path.split('/') {
            found = self
                .nodes
                .iter()
                .enumerate()
                .find(|(_, n)| {
                    n.store == s && !n.gone && !n.reserved && n.parent == dir && n.name == part
                })
                .map(|(i, _)| i as Loc + 1);
            dir = found?;
        }
        found
    }

    fn visible(&self) -> usize {
        self.nodes.iter().filter(|n| !n.gone && !n.reserved).count()
    }

    fn entry(&self, loc: Loc, out: &mut Entry) {
        let n = &self.nodes[loc as usize - 1];
        *out = Entry {
            loc,
            name: t(&n.name),
            long_name: false,
            folder: n.folder,
            size: n.data.len() as u64,
            created: None,
            modified: n.modified,
            hidden: n.hidden,
            system: n.protected,
            protected: n.protected,
            uid: 0x1000 + u128::from(loc),
        };
    }

    fn taken(&self, s: u8, dir: Loc, name: &str) -> bool {
        self.nodes.iter().any(|n| {
            n.store == s && !n.gone && n.parent == dir && n.name.eq_ignore_ascii_case(name)
        })
    }
}

fn bad_name(n: &Text) -> bool {
    n.units()
        .iter()
        .any(|&c| "\\/:*?\"<>|".encode_utf16().any(|b| b == c))
}

impl Device for Mem {
    fn stores(&self) -> u8 {
        2
    }
    fn info(&mut self, s: u8) -> Option<StoreInfo> {
        let i = usize::from(s);
        self.present[i].then(|| StoreInfo {
            kind: Kind::Removable,
            writable: self.writable[i],
            capacity: self.capacity[i],
            free: self.capacity[i].saturating_sub(self.used(s)),
            description: t(if s == 0 { "SD Card" } else { "CF Card" }),
            volume_id: t(if s == 0 { "SD-1234" } else { "CF-5678" }),
            max_file: u64::from(u32::MAX),
        })
    }
    fn child(
        &mut self,
        s: u8,
        dir: Loc,
        cursor: &mut u64,
        out: &mut Entry,
    ) -> Result<bool, StoreError> {
        if !self.present[usize::from(s)] {
            return Err(StoreError::Absent);
        }
        if dir != ROOT {
            self.node(s, dir)?;
        }
        while (*cursor as usize) < self.nodes.len() {
            let i = *cursor as usize;
            *cursor += 1;
            let n = &self.nodes[i];
            if n.store == s && !n.gone && !n.reserved && n.parent == dir {
                self.entry(i as Loc + 1, out);
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn stat(&mut self, s: u8, loc: Loc, out: &mut Entry) -> Result<(), StoreError> {
        self.node(s, loc)?;
        self.entry(loc, out);
        Ok(())
    }
    fn read(&mut self, s: u8, loc: Loc, offset: u64, buf: &mut [u8]) -> Result<usize, StoreError> {
        let n = self.node(s, loc)?;
        let off = (offset as usize).min(n.data.len());
        let k = buf.len().min(n.data.len() - off);
        buf[..k].copy_from_slice(&n.data[off..off + k]);
        Ok(k)
    }
    fn create(&mut self, s: u8, dir: Loc, new: &NewObject) -> Result<Loc, StoreError> {
        if !self.writable[usize::from(s)] {
            return Err(StoreError::ReadOnly);
        }
        let name = format!("{:?}", new.name);
        if bad_name(&new.name) {
            return Err(StoreError::BadName);
        }
        if self.taken(s, dir, &name) {
            return Err(StoreError::Exists);
        }
        let free = self.capacity[usize::from(s)].saturating_sub(self.used(s));
        if new.size > free {
            return Err(StoreError::Full);
        }
        let loc = self.add(s, dir, &name, new.folder, b"", false);
        let n = self.nodes.last_mut().unwrap();
        n.hidden = new.hidden;
        n.modified = new.modified;
        if !new.folder && new.size > 0 {
            n.reserved = true;
            n.data = vec![0; new.size as usize];
        }
        Ok(loc)
    }
    fn write(&mut self, s: u8, loc: Loc, offset: u64, data: &[u8]) -> Result<(), StoreError> {
        if let Some(e) = self.fail_write.take() {
            return Err(e);
        }
        // As a card: every write starts on a sector (the responder keeps
        // the tail for the next one).
        if !offset.is_multiple_of(512) {
            return Err(StoreError::Device);
        }
        self.writes += 1;
        self.node(s, loc)?;
        let n = &mut self.nodes[loc as usize - 1];
        let off = offset as usize;
        if off + data.len() > n.data.len() {
            return Err(StoreError::Full);
        }
        n.data[off..off + data.len()].copy_from_slice(data);
        Ok(())
    }
    fn commit(&mut self, s: u8, loc: Loc) -> Result<Loc, StoreError> {
        self.node(s, loc)?;
        self.nodes[loc as usize - 1].reserved = false;
        Ok(loc)
    }
    fn abandon(&mut self, s: u8, loc: Loc) {
        if self.node(s, loc).is_ok() {
            self.nodes[loc as usize - 1].gone = true;
        }
    }
    fn delete(&mut self, s: u8, loc: Loc) -> Result<(), StoreError> {
        let n = self.node(s, loc)?;
        if n.protected {
            return Err(StoreError::Protected);
        }
        if n.folder
            && self
                .nodes
                .iter()
                .any(|c| !c.gone && c.store == s && c.parent == loc)
        {
            return Err(StoreError::NotEmpty);
        }
        self.nodes[loc as usize - 1].gone = true;
        Ok(())
    }
    fn rename(&mut self, s: u8, dir: Loc, loc: Loc, name: &Text) -> Result<Loc, StoreError> {
        let n = self.node(s, loc)?;
        assert_eq!(n.parent, dir, "the responder names the folder it is in");
        if n.protected {
            return Err(StoreError::Protected);
        }
        let parent = n.parent;
        let name = format!("{name:?}");
        if bad_name(&t(&name)) {
            return Err(StoreError::BadName);
        }
        if self.taken(s, parent, &name) {
            return Err(StoreError::Exists);
        }
        self.nodes[loc as usize - 1].name = name;
        Ok(loc)
    }
    fn relocate(&mut self, s: u8, from: Loc, loc: Loc, dir: Loc) -> Result<Loc, StoreError> {
        let n = self.node(s, loc)?.clone();
        assert_eq!(n.parent, from, "the responder names the folder it is in");
        if n.protected {
            return Err(StoreError::Protected);
        }
        if self.taken(s, dir, &n.name) {
            return Err(StoreError::Exists);
        }
        // A real store moves the entry: a new location (the handle stays).
        self.nodes[loc as usize - 1].gone = true;
        let new = self.add(s, dir, &n.name, n.folder, &n.data, false);
        for c in &mut self.nodes {
            if c.parent == loc && c.store == s {
                c.parent = new;
            }
        }
        Ok(new)
    }
    fn boots_from(&mut self, s: u8) -> bool {
        self.boots[usize::from(s)]
    }
    fn format_consent(&mut self, _s: u8, start: bool) -> Consent {
        if start {
            self.asked += 1;
        }
        self.now += 1000;
        self.consent.pop_front().unwrap_or(Consent::Waiting)
    }
    fn format(&mut self, s: u8) -> Result<(), StoreError> {
        for n in &mut self.nodes {
            if n.store == s {
                n.gone = true;
            }
        }
        self.formatted.push(s);
        Ok(())
    }
    fn friendly_name(&self) -> Text {
        self.name
    }
    fn set_friendly_name(&mut self, name: &Text) {
        self.name = *name;
    }
    fn icon_len(&mut self) -> u32 {
        self.icon.len() as u32
    }
    fn icon_read(&mut self, offset: u32, buf: &mut [u8]) -> usize {
        let off = (offset as usize).min(self.icon.len());
        let k = buf.len().min(self.icon.len() - off);
        buf[..k].copy_from_slice(&self.icon[off..off + k]);
        k
    }
    fn initiator(&mut self, who: &Text) {
        self.initiator = Some(format!("{who:?}"));
    }
    fn now_ms(&self) -> u32 {
        self.now
    }
}

// --- The pipe and the PC -------------------------------------------------

#[derive(Default)]
struct TestPipe {
    out: VecDeque<Vec<u8>>,
    inp: VecDeque<Vec<u8>>,
    events: Vec<Vec<u8>>,
    stalls: u32,
    flushes: u32,
    /// IN packets the pipe accepts before refusing (a slow host).
    in_room: Option<usize>,
}

impl Pipe for TestPipe {
    fn read_packet(&mut self, buf: &mut [u8; PACKET]) -> Option<usize> {
        let p = self.out.pop_front()?;
        buf[..p.len()].copy_from_slice(&p);
        Some(p.len())
    }
    fn can_write(&mut self) -> bool {
        self.in_room.is_none_or(|n| n > 0)
    }
    fn write_packet(&mut self, data: &[u8]) {
        assert!(data.len() <= PACKET);
        if let Some(n) = &mut self.in_room {
            *n -= 1;
        }
        self.inp.push_back(data.to_vec());
    }
    fn event_can_write(&mut self) -> bool {
        true
    }
    fn event_write(&mut self, data: &[u8]) {
        self.events.push(data.to_vec());
    }
    fn stall(&mut self) {
        self.stalls += 1;
    }
    fn flush(&mut self) {
        self.flushes += 1;
        self.out.clear();
    }
}

fn container(kind: u16, code: u16, tid: u32, payload: &[u8]) -> Vec<u8> {
    let mut c = Vec::new();
    c.extend_from_slice(&((HEADER + payload.len()) as u32).to_le_bytes());
    c.extend_from_slice(&kind.to_le_bytes());
    c.extend_from_slice(&code.to_le_bytes());
    c.extend_from_slice(&tid.to_le_bytes());
    c.extend_from_slice(payload);
    c
}

fn packets(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut v: Vec<Vec<u8>> = bytes.chunks(PACKET).map(<[u8]>::to_vec).collect();
    if bytes.len().is_multiple_of(PACKET) {
        v.push(Vec::new());
    }
    v
}

#[derive(Debug)]
struct Answer {
    rc: u16,
    params: Vec<u32>,
    data: Vec<u8>,
}

impl Answer {
    fn u32_at(&self, o: usize) -> u32 {
        u32::from_le_bytes(self.data[o..o + 4].try_into().unwrap())
    }
    fn array32(&self) -> Vec<u32> {
        let n = self.u32_at(0) as usize;
        (0..n).map(|i| self.u32_at(4 + 4 * i)).collect()
    }
}

struct Pc<'b, D = Mem> {
    mtp: Mtp<'b>,
    dev: D,
    pipe: TestPipe,
    tid: u32,
}

fn identity() -> Identity {
    Identity {
        manufacturer: "Dell",
        model: "Axim X51v",
        version: "RetroPane 26",
        serial: *b"52503236303030315250323630303031",
    }
}

macro_rules! pc {
    ($pc:ident) => {
        pc!($pc, 64)
    };
    ($pc:ident, $slots:expr) => {
        let mut buf = vec![0u8; 4096];
        let mut slots = vec![Slot::default(); $slots];
        let mut index = vec![0u32; 2 * $slots];
        #[allow(unused_mut)]
        let mut $pc = Pc {
            mtp: Mtp::new(identity(), &mut buf, &mut slots, &mut index),
            dev: Mem::new(),
            pipe: TestPipe::default(),
            tid: 0,
        };
    };
}

impl<D: Device> Pc<'_, D> {
    fn run(&mut self) {
        for _ in 0..100_000 {
            self.mtp.poll(&mut self.dev, &mut self.pipe);
            if !self.mtp.busy() && self.pipe.out.is_empty() {
                // One more to send anything queued.
                self.mtp.poll(&mut self.dev, &mut self.pipe);
                return;
            }
        }
        panic!("the responder never finished");
    }

    /// Sends a command (and data), runs the responder, collects the data
    /// and the response.
    fn op(&mut self, code: u16, params: &[u32], data: Option<&[u8]>) -> Answer {
        self.tid += 1;
        let tid = self.tid;
        self.send(code, tid, params, data);
        self.run();
        self.collect(tid)
    }

    fn send(&mut self, code: u16, tid: u32, params: &[u32], data: Option<&[u8]>) {
        let mut p = Vec::new();
        for x in params {
            p.extend_from_slice(&x.to_le_bytes());
        }
        self.pipe
            .out
            .push_back(container(container::COMMAND, code, tid, &p));
        if let Some(d) = data {
            for pk in packets(&container(container::DATA, code, tid, d)) {
                self.pipe.out.push_back(pk);
            }
        }
    }

    fn collect(&mut self, tid: u32) -> Answer {
        let mut data = Vec::new();
        let mut first = true;
        let mut data_len = None;
        loop {
            let pk = self.pipe.inp.pop_front().expect("no response");
            if first
                && pk.len() >= HEADER
                && u16::from_le_bytes([pk[4], pk[5]]) == container::RESPONSE
            {
                let len = u32::from_le_bytes(pk[..4].try_into().unwrap()) as usize;
                assert_eq!(len, pk.len(), "a response is one packet");
                assert_eq!(u32::from_le_bytes(pk[8..12].try_into().unwrap()), tid);
                let params = pk[HEADER..]
                    .chunks(4)
                    .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
                    .collect();
                assert!(self.pipe.inp.is_empty(), "nothing after the response");
                return Answer {
                    rc: u16::from_le_bytes([pk[6], pk[7]]),
                    params,
                    data,
                };
            }
            if first {
                assert_eq!(u16::from_le_bytes([pk[4], pk[5]]), container::DATA);
                assert_eq!(u32::from_le_bytes(pk[8..12].try_into().unwrap()), tid);
                data_len = Some(u32::from_le_bytes(pk[..4].try_into().unwrap()));
                data.extend_from_slice(&pk[HEADER..]);
                first = false;
            } else {
                data.extend_from_slice(&pk);
            }
            if pk.len() < PACKET {
                // The data phase ended: what follows is the response.
                if let Some(l) = data_len.filter(|&l| l != u32::MAX) {
                    assert_eq!(l as usize - HEADER, data.len(), "data length as announced");
                }
                first = true;
                data_len = None;
            }
        }
    }

    fn open(&mut self) {
        assert_eq!(self.op(op::OPEN_SESSION, &[1], None).rc, rc::OK);
    }

    /// Handles in a folder (0xFFFFFFFF: the root) of store 0.
    fn list(&mut self, parent: u32) -> Vec<u32> {
        let a = self.op(
            op::GET_OBJECT_HANDLES,
            &[store_id(0, true), 0, parent],
            None,
        );
        assert_eq!(a.rc, rc::OK);
        a.array32()
    }

    fn name_of(&mut self, h: u32) -> String {
        let a = self.op(
            op::GET_OBJECT_PROP_VALUE,
            &[h, u32::from(prop::OBJECT_FILE_NAME)],
            None,
        );
        assert_eq!(a.rc, rc::OK);
        format!("{:?}", Reader::new(&a.data).text().unwrap())
    }

    fn handle(&mut self, parent: u32, name: &str) -> u32 {
        for h in self.list(parent) {
            if self.name_of(h) == name {
                return h;
            }
        }
        panic!("{name} not listed");
    }
}

fn object_info(name: &str, size: u32, format: u16) -> Vec<u8> {
    let mut b = vec![0u8; 256];
    let mut w = Writer::new(&mut b);
    w.u32(0);
    w.u16(format);
    w.u16(0);
    w.u32(size);
    w.u16(0); // thumbnail format
    for _ in 0..6 {
        w.u32(0); // thumbnail size, width, height; image width, height, depth
    }
    w.u32(0); // parent
    w.u16(u16::from(format == fmt::ASSOCIATION));
    w.u32(0);
    w.u32(0);
    w.text(&t(name));
    w.text(&Text::new());
    w.ascii(b"20261025T221500");
    w.u8(0);
    let n = w.len();
    b.truncate(n);
    b
}

// --- Device and session --------------------------------------------------

#[test]
fn device_info_says_mtp_with_microsofts_extension_and_a_32_digit_serial() {
    pc!(pc);
    let a = pc.op(op::GET_DEVICE_INFO, &[], None);
    assert_eq!(a.rc, rc::OK);
    let mut r = Reader::new(&a.data);
    assert_eq!(r.u16().unwrap(), 100);
    assert_eq!(
        r.u32().unwrap(),
        6,
        "Windows and libmtp take 0xFFFFFFFF for a camera"
    );
    assert_eq!(r.u16().unwrap(), 100);
    assert_eq!(format!("{:?}", r.text().unwrap()), "microsoft.com: 1.0; ");
    assert_eq!(r.u16().unwrap(), 0);
    let ops = r.u32().unwrap();
    let mut listed = Vec::new();
    for _ in 0..ops {
        listed.push(r.u16().unwrap());
    }
    for must in [
        op::GET_DEVICE_INFO,
        op::OPEN_SESSION,
        op::GET_STORAGE_IDS,
        op::GET_OBJECT_PROP_LIST,
        op::SEND_OBJECT_PROP_LIST,
        op::FORMAT_STORE,
    ] {
        assert!(listed.contains(&must), "{must:#x}");
    }
    for never in [op::POWER_DOWN, op::SET_OBJECT_PROTECTION, op::GET_THUMB] {
        assert!(!listed.contains(&never), "{never:#x}");
    }
    let events = r.u32().unwrap();
    r.skip(2 * events as usize).unwrap();
    let props = r.u32().unwrap();
    r.skip(2 * props as usize).unwrap();
    assert_eq!(r.u32().unwrap(), 0, "no capture formats");
    let formats = r.u32().unwrap();
    r.skip(2 * formats as usize).unwrap();
    assert_eq!(format!("{:?}", r.text().unwrap()), "Dell");
    assert_eq!(format!("{:?}", r.text().unwrap()), "Axim X51v");
    r.text().unwrap();
    let serial = format!("{:?}", r.text().unwrap());
    assert_eq!(serial.len(), 32);
    assert!(serial.chars().all(|c| c.is_ascii_hexdigit()));
    assert_eq!(r.left(), 0);
}

#[test]
fn sessions_open_once_and_operations_need_one() {
    pc!(pc);
    assert_eq!(
        pc.op(op::GET_STORAGE_IDS, &[], None).rc,
        rc::SESSION_NOT_OPEN
    );
    // An operation that sends data still has it taken [MTP 4.6].
    let a = pc.op(
        op::SEND_OBJECT_INFO,
        &[0, 0],
        Some(&object_info("x.txt", 1, fmt::TEXT)),
    );
    assert_eq!(a.rc, rc::SESSION_NOT_OPEN);
    assert!(pc.pipe.out.is_empty());
    assert_eq!(
        pc.op(op::OPEN_SESSION, &[0], None).rc,
        rc::INVALID_PARAMETER
    );
    pc.open();
    let again = pc.op(op::OPEN_SESSION, &[9], None);
    assert_eq!((again.rc, again.params[0]), (rc::SESSION_ALREADY_OPEN, 1));
    assert_eq!(pc.op(op::CLOSE_SESSION, &[], None).rc, rc::OK);
    assert_eq!(
        pc.op(op::GET_STORAGE_IDS, &[], None).rc,
        rc::SESSION_NOT_OPEN
    );
    assert_eq!(pc.op(0x1234, &[], None).rc, rc::OPERATION_NOT_SUPPORTED);
}

#[test]
fn an_empty_slot_is_listed_without_media_and_unavailable() {
    pc!(pc);
    pc.dev.present[1] = false;
    pc.open();
    let ids = pc.op(op::GET_STORAGE_IDS, &[], None).array32();
    assert_eq!(ids, vec![0x0001_0001, 0x0002_0000]);
    assert_eq!(
        pc.op(op::GET_STORAGE_INFO, &[0x0002_0001], None).rc,
        rc::STORE_NOT_AVAILABLE
    );
    assert_eq!(
        pc.op(op::GET_STORAGE_INFO, &[0x0003_0001], None).rc,
        rc::INVALID_STORAGE_ID
    );
    assert_eq!(
        pc.op(op::GET_STORAGE_INFO, &[0x0001_0000], None).rc,
        rc::INVALID_STORAGE_ID
    );
    let a = pc.op(op::GET_STORAGE_INFO, &[0x0001_0001], None);
    assert_eq!(a.rc, rc::OK);
    let mut r = Reader::new(&a.data);
    assert_eq!(r.u16().unwrap(), 4, "removable");
    assert_eq!(r.u16().unwrap(), 2, "hierarchical");
    assert_eq!(r.u16().unwrap(), 0, "read-write");
    assert_eq!(r.u64().unwrap(), 1 << 20);
    let free = r.u64().unwrap();
    assert!(free < 1 << 20);
    assert_eq!(r.u32().unwrap(), u32::MAX);
    assert_eq!(format!("{:?}", r.text().unwrap()), "SD Card");
    assert_eq!(format!("{:?}", r.text().unwrap()), "SD-1234");
}

// --- Listing ---------------------------------------------------------------

#[test]
fn folders_list_their_own_children_and_handles_stay_the_same() {
    pc!(pc);
    pc.open();
    let root = pc.list(u32::MAX);
    assert_eq!(root.len(), 3);
    let names: Vec<String> = root.iter().map(|&h| pc.name_of(h)).collect();
    assert_eq!(names, ["haret.exe", "Docs", "big.bin"]);
    assert_eq!(pc.list(u32::MAX), root, "the same handles every time");
    let docs = pc.handle(u32::MAX, "Docs");
    assert_eq!(pc.list(docs).len(), 2);
    let n = pc.op(op::GET_NUM_OBJECTS, &[store_id(0, true), 0, docs], None);
    assert_eq!((n.rc, n.params[0]), (rc::OK, 2));
    // A file isn't a folder.
    let a_txt = pc.handle(docs, "a.txt");
    assert_eq!(
        pc.op(op::GET_OBJECT_HANDLES, &[store_id(0, true), 0, a_txt], None)
            .rc,
        rc::INVALID_PARENT_OBJECT
    );
    assert_eq!(
        pc.op(op::GET_OBJECT_HANDLES, &[store_id(0, true), 0, 999], None)
            .rc,
        rc::INVALID_OBJECT_HANDLE
    );
}

#[test]
fn a_whole_store_and_every_store_walk_in_one_operation() {
    pc!(pc);
    pc.open();
    // Parent 0: every object on the store, at any depth.
    let all = pc
        .op(op::GET_OBJECT_HANDLES, &[store_id(0, true), 0, 0], None)
        .array32();
    assert_eq!(all.len(), 6);
    let mut unique = all.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), 6, "every object once");
    let every = pc
        .op(op::GET_OBJECT_HANDLES, &[u32::MAX, 0, 0], None)
        .array32();
    assert_eq!(every.len(), 7);
    // By format: the one PNG.
    let png = pc
        .op(
            op::GET_OBJECT_HANDLES,
            &[u32::MAX, u32::from(fmt::PNG), 0],
            None,
        )
        .array32();
    assert_eq!(png.len(), 1);
    assert_eq!(pc.name_of(png[0]), "deep.png");
}

#[test]
fn object_info_carries_the_name_size_parent_and_protection() {
    pc!(pc);
    pc.open();
    let haret = pc.handle(u32::MAX, "haret.exe");
    let a = pc.op(op::GET_OBJECT_INFO, &[haret], None);
    assert_eq!(a.rc, rc::OK);
    let mut r = Reader::new(&a.data);
    assert_eq!(r.u32().unwrap(), 0x0001_0001);
    assert_eq!(r.u16().unwrap(), fmt::UNDEFINED);
    assert_eq!(r.u16().unwrap(), 0x0001, "a boot file is read-only");
    assert_eq!(r.u32().unwrap(), 5);
    r.skip(2 + 4 * 6).unwrap();
    assert_eq!(r.u32().unwrap(), 0, "in the root");
    assert_eq!(r.u16().unwrap(), 0);
    r.skip(8).unwrap();
    assert_eq!(format!("{:?}", r.text().unwrap()), "haret.exe");
    let docs = pc.handle(u32::MAX, "Docs");
    let a = pc.op(op::GET_OBJECT_INFO, &[docs], None);
    let mut r = Reader::new(&a.data);
    r.skip(4).unwrap();
    assert_eq!(r.u16().unwrap(), fmt::ASSOCIATION);
    r.skip(2 + 4 + 2 + 4 * 6 + 4).unwrap();
    assert_eq!(r.u16().unwrap(), 1, "a generic folder");
    // Hidden and System for the boot file.
    let hidden = pc.op(
        op::GET_OBJECT_PROP_VALUE,
        &[haret, u32::from(prop::HIDDEN)],
        None,
    );
    assert_eq!(hidden.data, [1, 0]);
    let system = pc.op(
        op::GET_OBJECT_PROP_VALUE,
        &[haret, u32::from(prop::SYSTEM_OBJECT)],
        None,
    );
    assert_eq!(system.data, [1, 0]);
}

// --- Reading -----------------------------------------------------------------

#[test]
fn files_read_whole_and_in_parts_and_folders_dont() {
    pc!(pc);
    pc.open();
    let big = pc.handle(u32::MAX, "big.bin");
    let want: Vec<u8> = (0..5000u32).map(|i| i as u8).collect();
    let a = pc.op(op::GET_OBJECT, &[big], None);
    assert_eq!(a.rc, rc::OK);
    assert_eq!(a.data, want);
    let p = pc.op(op::GET_PARTIAL_OBJECT, &[big, 4000, 2000], None);
    assert_eq!((p.rc, p.params[0]), (rc::OK, 1000));
    assert_eq!(p.data, want[4000..]);
    let p = pc.op(op::GET_PARTIAL_OBJECT, &[big, 10, u32::MAX], None);
    assert_eq!(p.data, want[10..]);
    assert_eq!(
        pc.op(op::GET_PARTIAL_OBJECT, &[big, 5001, 1], None).rc,
        rc::INVALID_PARAMETER
    );
    let docs = pc.handle(u32::MAX, "Docs");
    assert_eq!(
        pc.op(op::GET_OBJECT, &[docs], None).rc,
        rc::INVALID_OBJECT_HANDLE
    );
}

#[test]
fn a_transfer_of_whole_packets_ends_with_an_empty_one() {
    pc!(pc);
    pc.open();
    // 52 bytes + the 12-byte header: exactly one packet.
    let loc = pc.dev.add(0, ROOT, "x.bin", false, &[3u8; 52], false);
    let _ = loc;
    let h = pc.handle(u32::MAX, "x.bin");
    pc.tid += 1;
    let tid = pc.tid;
    pc.send(op::GET_OBJECT, tid, &[h], None);
    pc.run();
    let lens: Vec<usize> = pc.pipe.inp.iter().map(Vec::len).collect();
    assert_eq!(lens, [64, 0, 12], "data, the empty packet, the response");
    pc.pipe.inp.clear();
}

#[test]
fn a_slow_host_gets_every_byte() {
    pc!(pc);
    pc.open();
    let big = pc.handle(u32::MAX, "big.bin");
    pc.tid += 1;
    let tid = pc.tid;
    pc.send(op::GET_OBJECT, tid, &[big], None);
    for _ in 0..1000 {
        pc.pipe.in_room = Some(1);
        pc.mtp.poll(&mut pc.dev, &mut pc.pipe);
    }
    pc.pipe.in_room = None;
    pc.run();
    let a = pc.collect(tid);
    assert_eq!(a.data.len(), 5000);
}

// --- Refusals ------------------------------------------------------------------

#[test]
fn a_boot_file_is_refused_at_once_and_stays() {
    pc!(pc);
    pc.open();
    let haret = pc.handle(u32::MAX, "haret.exe");
    assert_eq!(
        pc.op(op::DELETE_OBJECT, &[haret, 0], None).rc,
        rc::OBJECT_WRITE_PROTECTED
    );
    let name = Writer::new(&mut [0u8; 64]).len();
    let _ = name;
    let mut b = vec![0u8; 64];
    let mut w = Writer::new(&mut b);
    w.text(&t("evil.exe"));
    let n = w.len();
    let r = pc.op(
        op::SET_OBJECT_PROP_VALUE,
        &[haret, u32::from(prop::OBJECT_FILE_NAME)],
        Some(&b[..n]),
    );
    assert_eq!(r.rc, rc::OBJECT_WRITE_PROTECTED);
    let docs = pc.handle(u32::MAX, "Docs");
    assert_eq!(
        pc.op(op::MOVE_OBJECT, &[haret, store_id(0, true), docs], None)
            .rc,
        rc::OBJECT_WRITE_PROTECTED
    );
    assert_eq!(
        pc.dev
            .find(0, "haret.exe")
            .map(|l| pc.dev.nodes[l as usize - 1].data.clone()),
        Some(b"HARET".to_vec())
    );
    // It can still be copied off (a backup), and copied on the device.
    assert_eq!(pc.op(op::GET_OBJECT, &[haret], None).data, b"HARET");
}

#[test]
fn delete_everything_is_never_done_from_a_cable() {
    pc!(pc);
    pc.open();
    let before = pc.dev.visible();
    assert_eq!(
        pc.op(op::DELETE_OBJECT, &[u32::MAX, 0], None).rc,
        rc::INVALID_OBJECT_HANDLE
    );
    assert_eq!(pc.dev.visible(), before);
}

#[test]
fn a_deleted_objects_handle_never_comes_back() {
    pc!(pc);
    pc.open();
    let big = pc.handle(u32::MAX, "big.bin");
    assert_eq!(pc.op(op::DELETE_OBJECT, &[big, 0], None).rc, rc::OK);
    assert!(pc.dev.find(0, "big.bin").is_none());
    assert_eq!(
        pc.op(op::GET_OBJECT_INFO, &[big], None).rc,
        rc::INVALID_OBJECT_HANDLE
    );
    pc.dev.add(0, ROOT, "big.bin", false, b"new", false);
    let again = pc.handle(u32::MAX, "big.bin");
    assert_ne!(
        again, big,
        "a handle isn't reused within the session [MTP 3.4.1]"
    );
}

#[test]
fn a_folder_deletes_with_its_contents_but_keeps_what_protects_it() {
    pc!(pc);
    pc.open();
    let docs = pc.handle(u32::MAX, "Docs");
    assert_eq!(pc.op(op::DELETE_OBJECT, &[docs, 0], None).rc, rc::OK);
    assert!(pc.dev.find(0, "Docs").is_none());
    assert!(pc.dev.find(0, "Docs/Sub/deep.png").is_none());
    // A folder with a boot file inside: the file and its folder stay.
    let boot = pc.dev.add(0, ROOT, "RetroPane", true, b"", false);
    pc.dev.add(0, boot, "retropane.bin", false, b"K", true);
    pc.dev.add(0, boot, "notes.txt", false, b"n", false);
    let rp = pc.handle(u32::MAX, "RetroPane");
    assert_eq!(
        pc.op(op::DELETE_OBJECT, &[rp, 0], None).rc,
        rc::PARTIAL_DELETION
    );
    assert!(pc.dev.find(0, "RetroPane/retropane.bin").is_some());
    assert!(pc.dev.find(0, "RetroPane/notes.txt").is_none());
}

#[test]
fn a_read_only_store_refuses_changes() {
    pc!(pc);
    pc.dev.writable[0] = false;
    pc.open();
    let big = pc.handle(u32::MAX, "big.bin");
    assert_eq!(
        pc.op(op::DELETE_OBJECT, &[big, 0], None).rc,
        rc::STORE_READ_ONLY
    );
    let a = pc.op(
        op::SEND_OBJECT_INFO,
        &[store_id(0, true), u32::MAX],
        Some(&object_info("n.txt", 3, fmt::TEXT)),
    );
    assert_eq!(a.rc, rc::STORE_READ_ONLY);
}

// --- Writing -------------------------------------------------------------------

#[test]
fn a_sent_file_appears_only_once_complete() {
    pc!(pc);
    pc.open();
    let body: Vec<u8> = (0..3000u32).map(|i| (i * 7) as u8).collect();
    let a = pc.op(
        op::SEND_OBJECT_INFO,
        &[store_id(0, true), u32::MAX],
        Some(&object_info("new.txt", 3000, fmt::TEXT)),
    );
    assert_eq!(a.rc, rc::OK);
    assert_eq!(a.params[0], store_id(0, true));
    assert_eq!(a.params[1], u32::MAX, "the root");
    let h = a.params[2];
    // Reserved, but not visible on the card yet.
    assert!(pc.dev.find(0, "new.txt").is_none());
    assert_eq!(pc.op(op::SEND_OBJECT, &[], Some(&body)).rc, rc::OK);
    let loc = pc.dev.find(0, "new.txt").expect("committed");
    assert_eq!(pc.dev.nodes[loc as usize - 1].data, body);
    assert_eq!(
        pc.dev.nodes[loc as usize - 1].modified.map(|t| t.year),
        Some(2026)
    );
    assert_eq!(pc.name_of(h), "new.txt");
    assert_eq!(pc.op(op::GET_OBJECT, &[h], None).data, body);
}

#[test]
fn no_room_is_said_before_a_byte_moves() {
    pc!(pc);
    pc.open();
    let a = pc.op(
        op::SEND_OBJECT_INFO,
        &[store_id(0, true), u32::MAX],
        Some(&object_info("huge.bin", 2 << 20, fmt::UNDEFINED)),
    );
    assert_eq!(a.rc, rc::STORE_FULL);
    assert_eq!(
        pc.op(op::SEND_OBJECT, &[], Some(b"x")).rc,
        rc::NO_VALID_OBJECT_INFO
    );
    assert_eq!(pc.dev.writes, 0);
}

#[test]
fn more_bytes_than_announced_are_refused_and_nothing_is_named() {
    pc!(pc);
    pc.open();
    assert_eq!(
        pc.op(
            op::SEND_OBJECT_INFO,
            &[store_id(0, true), u32::MAX],
            Some(&object_info("s.txt", 10, fmt::TEXT))
        )
        .rc,
        rc::OK
    );
    assert_eq!(
        pc.op(op::SEND_OBJECT, &[], Some(&[1u8; 200])).rc,
        rc::STORE_FULL
    );
    assert!(pc.dev.find(0, "s.txt").is_none());
    assert!(pc.pipe.out.is_empty(), "the rest was drained");
    // The reservation stays for a retry with the right size [MTP D.2.13].
    assert_eq!(pc.op(op::SEND_OBJECT, &[], Some(&[2u8; 10])).rc, rc::OK);
    assert_eq!(
        pc.dev
            .find(0, "s.txt")
            .map(|l| pc.dev.nodes[l as usize - 1].data.clone()),
        Some(vec![2u8; 10])
    );
}

#[test]
fn a_cancelled_or_unplugged_copy_leaves_nothing() {
    pc!(pc);
    pc.open();
    assert_eq!(
        pc.op(
            op::SEND_OBJECT_INFO,
            &[store_id(0, true), u32::MAX],
            Some(&object_info("c.bin", 4000, fmt::UNDEFINED))
        )
        .rc,
        rc::OK
    );
    // Half the data, then the host cancels.
    pc.tid += 1;
    let tid = pc.tid;
    let all = packets(&container(
        container::DATA,
        op::SEND_OBJECT,
        tid,
        &[9u8; 4000],
    ));
    pc.pipe
        .out
        .push_back(container(container::COMMAND, op::SEND_OBJECT, tid, &[]));
    for p in &all[..30] {
        pc.pipe.out.push_back(p.clone());
    }
    for _ in 0..100 {
        pc.mtp.poll(&mut pc.dev, &mut pc.pipe);
    }
    assert!(pc.mtp.busy());
    pc.mtp.cancel(&mut pc.dev, &mut pc.pipe, tid);
    assert!(!pc.mtp.busy());
    assert!(
        pc.pipe.inp.is_empty(),
        "no response to a cancelled transfer"
    );
    assert!(pc.dev.find(0, "c.bin").is_none());
    // A retry completes it.
    assert_eq!(pc.op(op::SEND_OBJECT, &[], Some(&[9u8; 4000])).rc, rc::OK);
    assert!(pc.dev.find(0, "c.bin").is_some());
    // Another, cut off by the cable: the reservation is dropped.
    assert_eq!(
        pc.op(
            op::SEND_OBJECT_INFO,
            &[store_id(0, true), u32::MAX],
            Some(&object_info("u.bin", 4000, fmt::UNDEFINED))
        )
        .rc,
        rc::OK
    );
    let reserved = pc
        .dev
        .nodes
        .iter()
        .filter(|n| n.reserved && !n.gone)
        .count();
    assert_eq!(reserved, 1);
    pc.mtp.reset(&mut pc.dev, &mut pc.pipe);
    assert_eq!(
        pc.dev
            .nodes
            .iter()
            .filter(|n| n.reserved && !n.gone)
            .count(),
        0,
        "nothing half-written stays"
    );
    assert_eq!(pc.mtp.session(), 0);
}

#[test]
fn a_short_transfer_is_incomplete_and_kept_for_a_retry() {
    pc!(pc);
    pc.open();
    assert_eq!(
        pc.op(
            op::SEND_OBJECT_INFO,
            &[store_id(0, true), u32::MAX],
            Some(&object_info("p.bin", 100, fmt::UNDEFINED))
        )
        .rc,
        rc::OK
    );
    assert_eq!(
        pc.op(op::SEND_OBJECT, &[], Some(&[1u8; 60])).rc,
        rc::INCOMPLETE_TRANSFER
    );
    assert!(pc.dev.find(0, "p.bin").is_none());
    assert_eq!(pc.op(op::SEND_OBJECT, &[], Some(&[1u8; 100])).rc, rc::OK);
}

#[test]
fn a_card_write_failing_mid_copy_names_nothing() {
    pc!(pc);
    pc.open();
    assert_eq!(
        pc.op(
            op::SEND_OBJECT_INFO,
            &[store_id(0, true), u32::MAX],
            Some(&object_info("f.bin", 9000, fmt::UNDEFINED))
        )
        .rc,
        rc::OK
    );
    pc.dev.fail_write = Some(StoreError::Device);
    assert_eq!(
        pc.op(op::SEND_OBJECT, &[], Some(&[5u8; 9000])).rc,
        rc::GENERAL_ERROR
    );
    assert!(pc.dev.find(0, "f.bin").is_none());
    assert!(pc.pipe.out.is_empty());
}

#[test]
fn another_operation_drops_a_reservation_and_its_handle() {
    pc!(pc);
    pc.open();
    let a = pc.op(
        op::SEND_OBJECT_INFO,
        &[store_id(0, true), u32::MAX],
        Some(&object_info("d.bin", 50, fmt::UNDEFINED)),
    );
    let h = a.params[2];
    pc.list(u32::MAX);
    assert_eq!(
        pc.dev
            .nodes
            .iter()
            .filter(|n| n.reserved && !n.gone)
            .count(),
        0
    );
    assert_eq!(
        pc.op(op::SEND_OBJECT, &[], Some(&[0; 50])).rc,
        rc::NO_VALID_OBJECT_INFO
    );
    assert_eq!(
        pc.op(op::GET_OBJECT_INFO, &[h], None).rc,
        rc::INVALID_OBJECT_HANDLE
    );
}

#[test]
fn folders_and_empty_files_exist_at_once() {
    pc!(pc);
    pc.open();
    let a = pc.op(
        op::SEND_OBJECT_INFO,
        &[store_id(0, true), u32::MAX],
        Some(&object_info("New folder", 0, fmt::ASSOCIATION)),
    );
    assert_eq!(a.rc, rc::OK);
    let folder = a.params[2];
    assert!(
        pc.dev
            .find(0, "New folder")
            .is_some_and(|l| pc.dev.nodes[l as usize - 1].folder)
    );
    let b = pc.op(
        op::SEND_OBJECT_INFO,
        &[store_id(0, true), folder],
        Some(&object_info("empty.txt", 0, fmt::TEXT)),
    );
    assert_eq!((b.rc, b.params[1]), (rc::OK, folder));
    assert!(pc.dev.find(0, "New folder/empty.txt").is_some());
    // A SendObject of nothing after it is fine.
    assert_eq!(
        pc.op(op::SEND_OBJECT, &[], Some(&[])).rc,
        rc::NO_VALID_OBJECT_INFO
    );
}

#[test]
fn bad_names_and_taken_names_are_refused() {
    pc!(pc);
    pc.open();
    let a = pc.op(
        op::SEND_OBJECT_INFO,
        &[store_id(0, true), u32::MAX],
        Some(&object_info("a:b.txt", 1, fmt::TEXT)),
    );
    assert_eq!(a.rc, rc::INVALID_DATASET);
    let b = pc.op(
        op::SEND_OBJECT_INFO,
        &[store_id(0, true), u32::MAX],
        Some(&object_info("BIG.BIN", 1, fmt::TEXT)),
    );
    assert_eq!(b.rc, rc::INVALID_DATASET);
    let c = pc.op(
        op::SEND_OBJECT_INFO,
        &[store_id(0, true), u32::MAX],
        Some(&object_info("", 1, fmt::TEXT)),
    );
    assert_eq!(c.rc, rc::INVALID_DATASET);
}

#[test]
fn send_object_prop_list_carries_a_64_bit_size_and_dates() {
    pc!(pc);
    pc.open();
    let mut b = vec![0u8; 256];
    let mut w = Writer::new(&mut b);
    w.u32(2);
    w.u32(0);
    w.u16(prop::OBJECT_FILE_NAME);
    w.u16(ty::STR);
    w.text(&t("list.txt"));
    w.u32(0);
    w.u16(prop::DATE_MODIFIED);
    w.u16(ty::STR);
    w.ascii(b"20261025T230000.5+0100");
    let n = w.len();
    let a = pc.op(
        op::SEND_OBJECT_PROP_LIST,
        &[store_id(0, true), u32::MAX, u32::from(fmt::TEXT), 0, 40],
        Some(&b[..n]),
    );
    assert_eq!(a.rc, rc::OK);
    assert_eq!(pc.op(op::SEND_OBJECT, &[], Some(&[4u8; 40])).rc, rc::OK);
    let l = pc.dev.find(0, "list.txt").unwrap();
    let m = pc.dev.nodes[l as usize - 1].modified.unwrap();
    assert_eq!((m.hour, m.tenths, m.zone), (23, Some(5), Some(60)));
    // An unknown property is refused with its index.
    let mut b = vec![0u8; 64];
    let mut w = Writer::new(&mut b);
    w.u32(2);
    w.u32(0);
    w.u16(prop::OBJECT_FILE_NAME);
    w.u16(ty::STR);
    w.text(&t("x.txt"));
    w.u32(0);
    w.u16(0xDC46); // Artist
    w.u16(ty::STR);
    w.text(&t("me"));
    let n = w.len();
    let a = pc.op(
        op::SEND_OBJECT_PROP_LIST,
        &[store_id(0, true), u32::MAX, u32::from(fmt::TEXT), 0, 4],
        Some(&b[..n]),
    );
    assert_eq!((a.rc, a.params[3]), (rc::OBJECT_PROP_NOT_SUPPORTED, 1));
    // Over 4 GB: too large for this store.
    let a = pc.op(
        op::SEND_OBJECT_PROP_LIST,
        &[store_id(0, true), u32::MAX, u32::from(fmt::TEXT), 1, 0],
        Some(&b[..n]),
    );
    assert_eq!(
        a.rc,
        rc::OBJECT_PROP_NOT_SUPPORTED,
        "the dataset is checked first"
    );
}

// --- Rename, move, copy ----------------------------------------------------------

#[test]
fn renames_go_through_both_property_operations() {
    pc!(pc);
    pc.open();
    let big = pc.handle(u32::MAX, "big.bin");
    let mut b = vec![0u8; 64];
    let mut w = Writer::new(&mut b);
    w.text(&t("renamed.bin"));
    let n = w.len();
    assert_eq!(
        pc.op(
            op::SET_OBJECT_PROP_VALUE,
            &[big, u32::from(prop::OBJECT_FILE_NAME)],
            Some(&b[..n])
        )
        .rc,
        rc::OK
    );
    assert_eq!(pc.name_of(big), "renamed.bin");
    assert_eq!(
        pc.op(
            op::SET_OBJECT_PROP_VALUE,
            &[big, u32::from(prop::OBJECT_SIZE)],
            Some(&[0; 8])
        )
        .rc,
        rc::ACCESS_DENIED
    );
    let mut b = vec![0u8; 64];
    let mut w = Writer::new(&mut b);
    w.u32(1);
    w.u32(big);
    w.u16(prop::OBJECT_FILE_NAME);
    w.u16(ty::STR);
    w.text(&t("bad/name"));
    let n = w.len();
    let a = pc.op(op::SET_OBJECT_PROP_LIST, &[], Some(&b[..n]));
    assert_eq!((a.rc, a.params[0]), (rc::INVALID_OBJECT_PROP_VALUE, 0));
}

#[test]
fn moves_keep_the_handle_and_folders_cant_go_into_themselves() {
    pc!(pc);
    pc.open();
    let docs = pc.handle(u32::MAX, "Docs");
    let sub = pc.handle(docs, "Sub");
    let big = pc.handle(u32::MAX, "big.bin");
    assert_eq!(
        pc.op(op::MOVE_OBJECT, &[big, store_id(0, true), sub], None)
            .rc,
        rc::OK
    );
    assert!(pc.dev.find(0, "Docs/Sub/big.bin").is_some());
    assert_eq!(pc.name_of(big), "big.bin", "the same handle after a move");
    assert!(pc.list(sub).contains(&big));
    assert_eq!(
        pc.op(op::MOVE_OBJECT, &[docs, store_id(0, true), sub], None)
            .rc,
        rc::INVALID_PARENT_OBJECT
    );
    assert_eq!(
        pc.op(op::MOVE_OBJECT, &[docs, store_id(0, true), docs], None)
            .rc,
        rc::INVALID_PARENT_OBJECT
    );
    // To the other card: a copy, then the original goes; the handle stays.
    assert_eq!(
        pc.op(op::MOVE_OBJECT, &[docs, store_id(1, true), 0], None)
            .rc,
        rc::OK
    );
    assert!(pc.dev.find(0, "Docs").is_none());
    assert!(pc.dev.find(1, "Docs/Sub/big.bin").is_some());
    assert_eq!(pc.name_of(docs), "Docs");
}

#[test]
fn copies_happen_on_the_device() {
    pc!(pc);
    pc.open();
    let docs = pc.handle(u32::MAX, "Docs");
    let a = pc.op(op::COPY_OBJECT, &[docs, store_id(1, true), 0], None);
    assert_eq!(a.rc, rc::OK);
    assert!(pc.dev.find(0, "Docs/Sub/deep.png").is_some());
    let l = pc.dev.find(1, "Docs/Sub/deep.png").unwrap();
    assert_eq!(pc.dev.nodes[l as usize - 1].data, vec![7u8; 300]);
    assert_eq!(pc.name_of(a.params[0]), "Docs");
    // A boot file may be copied (only changed is refused).
    let haret = pc.handle(u32::MAX, "haret.exe");
    assert_eq!(
        pc.op(op::COPY_OBJECT, &[haret, store_id(1, true), 0], None)
            .rc,
        rc::OK
    );
}

// --- Property lists ----------------------------------------------------------------

fn prop_list_objects(a: &Answer) -> Vec<(u32, u16)> {
    let mut r = Reader::new(&a.data);
    let n = r.u32().unwrap();
    let mut out = Vec::new();
    for _ in 0..n {
        let h = r.u32().unwrap();
        let code = r.u16().unwrap();
        let t = r.u16().unwrap();
        r.skip_value(t).unwrap();
        out.push((h, code));
    }
    assert_eq!(r.left(), 0);
    out
}

#[test]
fn property_lists_of_any_depth_come_in_one_operation() {
    pc!(pc);
    pc.open();
    let docs = pc.handle(u32::MAX, "Docs");
    // Docs and its children: three objects, every property.
    let a = pc.op(op::GET_OBJECT_PROP_LIST, &[docs, 0, u32::MAX, 0, 1], None);
    assert_eq!(a.rc, rc::OK);
    let items = prop_list_objects(&a);
    assert_eq!(items.len(), 3 * PROPS.len());
    // The whole tree under Docs.
    let a = pc.op(
        op::GET_OBJECT_PROP_LIST,
        &[docs, 0, u32::from(prop::OBJECT_FILE_NAME), 0, u32::MAX],
        None,
    );
    assert_eq!(prop_list_objects(&a).len(), 4);
    // Every object on every store, one property.
    let a = pc.op(
        op::GET_OBJECT_PROP_LIST,
        &[u32::MAX, 0, u32::from(prop::OBJECT_SIZE), 0, 0],
        None,
    );
    assert_eq!(prop_list_objects(&a).len(), 7);
    // The root objects of every store.
    let a = pc.op(
        op::GET_OBJECT_PROP_LIST,
        &[0, 0, u32::from(prop::NAME), 0, 1],
        None,
    );
    assert_eq!(prop_list_objects(&a).len(), 4);
    // Two levels: the roots and the children of Docs, not Sub's file.
    let a = pc.op(
        op::GET_OBJECT_PROP_LIST,
        &[0, 0, u32::from(prop::NAME), 0, 2],
        None,
    );
    assert_eq!(prop_list_objects(&a).len(), 6);
    // The root with no depth: empty.
    let a = pc.op(op::GET_OBJECT_PROP_LIST, &[0, 0, u32::MAX, 0, 0], None);
    assert_eq!(prop_list_objects(&a).len(), 0);
    // By group (all of ours are group 1), and an unknown group.
    let a = pc.op(op::GET_OBJECT_PROP_LIST, &[docs, 0, 0, 1, 0], None);
    assert_eq!(prop_list_objects(&a).len(), PROPS.len());
    assert_eq!(
        pc.op(op::GET_OBJECT_PROP_LIST, &[docs, 0, 0, 7, 0], None)
            .rc,
        rc::GROUP_NOT_SUPPORTED
    );
    assert_eq!(
        pc.op(op::GET_OBJECT_PROP_LIST, &[docs, 0, 0, 0, 0], None)
            .rc,
        rc::PARAMETER_NOT_SUPPORTED
    );
}

#[test]
fn a_property_list_bigger_than_the_buffer_streams_whole() {
    pc!(pc, 512);
    for i in 0..200 {
        pc.dev.add(
            0,
            ROOT,
            &format!("file-with-a-longer-name-{i:03}.txt"),
            false,
            b"x",
            false,
        );
    }
    pc.open();
    let a = pc.op(op::GET_OBJECT_PROP_LIST, &[0, 0, u32::MAX, 0, 1], None);
    assert_eq!(a.rc, rc::OK);
    assert!(a.data.len() > 4096 * 4, "several buffers' worth");
    assert_eq!(prop_list_objects(&a).len(), 204 * PROPS.len());
}

#[test]
fn property_descriptions_say_what_the_pc_may_change() {
    pc!(pc);
    pc.open();
    let a = pc.op(
        op::GET_OBJECT_PROP_DESC,
        &[u32::from(prop::OBJECT_FILE_NAME), u32::from(fmt::TEXT)],
        None,
    );
    let mut r = Reader::new(&a.data);
    assert_eq!(r.u16().unwrap(), prop::OBJECT_FILE_NAME);
    assert_eq!(r.u16().unwrap(), ty::STR);
    assert_eq!(r.u8().unwrap(), 1, "a rename is allowed");
    r.text().unwrap();
    assert_eq!(r.u32().unwrap(), 1);
    assert_eq!(r.u8().unwrap(), 5, "a regular expression of legal names");
    assert_eq!(format!("{:?}", r.text().unwrap()), NAME_PATTERN);
    let a = pc.op(
        op::GET_OBJECT_PROP_DESC,
        &[u32::from(prop::PROTECTION_STATUS), u32::from(fmt::TEXT)],
        None,
    );
    let mut r = Reader::new(&a.data);
    r.skip(4).unwrap();
    assert_eq!(r.u8().unwrap(), 0, "protection is the device's");
    let a = pc.op(
        op::GET_OBJECT_PROPS_SUPPORTED,
        &[u32::from(fmt::UNDEFINED)],
        None,
    );
    let mut r = Reader::new(&a.data);
    assert_eq!(r.u32().unwrap() as usize, PROPS.len());
    assert_eq!(
        pc.op(
            op::GET_OBJECT_PROP_DESC,
            &[0xDC46, u32::from(fmt::TEXT)],
            None
        )
        .rc,
        rc::INVALID_OBJECT_PROP_CODE
    );
}

// --- Device properties -----------------------------------------------------------

#[test]
fn the_device_name_reads_and_changes_and_the_icon_streams() {
    pc!(pc);
    pc.dev.icon = (0..10_000u32).map(|i| (i % 251) as u8).collect();
    pc.open();
    let a = pc.op(
        op::GET_DEVICE_PROP_VALUE,
        &[u32::from(dprop::FRIENDLY_NAME)],
        None,
    );
    assert_eq!(
        format!("{:?}", Reader::new(&a.data).text().unwrap()),
        "Axim X51v"
    );
    let mut b = vec![0u8; 64];
    let mut w = Writer::new(&mut b);
    w.text(&t("Reid's Axim"));
    let n = w.len();
    assert_eq!(
        pc.op(
            op::SET_DEVICE_PROP_VALUE,
            &[u32::from(dprop::FRIENDLY_NAME)],
            Some(&b[..n])
        )
        .rc,
        rc::OK
    );
    assert_eq!(format!("{:?}", pc.dev.name), "Reid's Axim");
    let a = pc.op(
        op::GET_DEVICE_PROP_VALUE,
        &[u32::from(dprop::DEVICE_ICON)],
        None,
    );
    assert_eq!(a.u32_at(0), 10_000);
    assert_eq!(a.data[4..], pc.dev.icon[..]);
    let d = pc.op(
        op::GET_DEVICE_PROP_DESC,
        &[u32::from(dprop::DEVICE_ICON)],
        None,
    );
    assert_eq!(d.data.len(), 2 + 2 + 1 + 4 + 4 + 10_000 + 1);
    assert_eq!(*d.data.last().unwrap(), 0);
    let mut b = vec![0u8; 128];
    let mut w = Writer::new(&mut b);
    w.text(&t("Windows/10.0.26300 MTPClassDriver/10.0"));
    let n = w.len();
    assert_eq!(
        pc.op(
            op::SET_DEVICE_PROP_VALUE,
            &[u32::from(dprop::SESSION_INITIATOR)],
            Some(&b[..n])
        )
        .rc,
        rc::OK
    );
    assert_eq!(
        pc.dev.initiator.as_deref(),
        Some("Windows/10.0.26300 MTPClassDriver/10.0")
    );
    let p = pc.op(
        op::GET_DEVICE_PROP_VALUE,
        &[u32::from(dprop::PERCEIVED_TYPE)],
        None,
    );
    assert_eq!(p.u32_at(0), 5, "a PDA");
    assert_eq!(
        pc.op(op::GET_DEVICE_PROP_VALUE, &[0x5001], None).rc,
        rc::DEVICE_PROP_NOT_SUPPORTED
    );
}

// --- Format --------------------------------------------------------------------------

#[test]
fn the_boot_card_is_never_formatted_and_others_need_the_users_yes() {
    pc!(pc);
    pc.open();
    assert_eq!(
        pc.op(op::FORMAT_STORE, &[store_id(0, true), 0], None).rc,
        rc::STORE_READ_ONLY
    );
    assert_eq!(pc.dev.asked, 0, "not even asked");
    pc.dev
        .consent
        .extend([Consent::Waiting, Consent::Waiting, Consent::No]);
    assert_eq!(
        pc.op(op::FORMAT_STORE, &[store_id(1, true), 0], None).rc,
        rc::ACCESS_DENIED
    );
    assert!(pc.dev.formatted.is_empty());
    pc.dev.consent.extend([Consent::Waiting, Consent::Yes]);
    assert_eq!(
        pc.op(op::FORMAT_STORE, &[store_id(1, true), 0], None).rc,
        rc::OK
    );
    assert_eq!(pc.dev.formatted, [1]);
    // No answer within the time: refused.
    pc.dev.present[1] = true;
    let a = pc.op(op::FORMAT_STORE, &[store_id(1, true), 0], None);
    assert_eq!(a.rc, rc::ACCESS_DENIED);
    assert_eq!(pc.dev.formatted, [1]);
}

// --- Events --------------------------------------------------------------------------

#[test]
fn device_side_changes_and_cards_reach_the_pc_as_events() {
    pc!(pc);
    // No session: no events.
    pc.mtp.store_changed(1, false);
    pc.run();
    assert!(pc.pipe.events.is_empty());
    pc.open();
    pc.list(u32::MAX);
    let loc = pc.dev.add(0, ROOT, "put.txt", false, b"p", false);
    pc.mtp.object_added(&mut pc.dev, 0, ROOT, loc);
    pc.mtp.store_changed(1, false);
    pc.mtp.store_changed(1, true);
    for _ in 0..10 {
        pc.mtp.poll(&mut pc.dev, &mut pc.pipe);
    }
    let codes: Vec<u16> = pc
        .pipe
        .events
        .iter()
        .map(|e| u16::from_le_bytes([e[6], e[7]]))
        .collect();
    assert_eq!(
        codes,
        [
            ev::OBJECT_ADDED,
            ev::STORAGE_INFO_CHANGED,
            ev::STORE_REMOVED,
            ev::STORE_ADDED
        ]
    );
    let added = u32::from_le_bytes(pc.pipe.events[0][12..16].try_into().unwrap());
    assert_eq!(pc.name_of(added), "put.txt");
    let removed = u32::from_le_bytes(pc.pipe.events[2][12..16].try_into().unwrap());
    assert_eq!(removed, 0x0002_FFFF, "the whole physical store");
    // Removing it again, from the device.
    pc.pipe.events.clear();
    pc.mtp.object_removed(0, loc);
    pc.mtp.poll(&mut pc.dev, &mut pc.pipe);
    assert_eq!(
        u16::from_le_bytes([pc.pipe.events[0][6], pc.pipe.events[0][7]]),
        ev::OBJECT_REMOVED
    );
}

#[test]
fn too_many_events_become_one_unreported_status() {
    pc!(pc);
    pc.open();
    for _ in 0..40 {
        pc.mtp.store_changed(1, true);
    }
    for _ in 0..40 {
        pc.mtp.poll(&mut pc.dev, &mut pc.pipe);
    }
    let codes: Vec<u16> = pc
        .pipe
        .events
        .iter()
        .map(|e| u16::from_le_bytes([e[6], e[7]]))
        .collect();
    assert_eq!(codes, [ev::UNREPORTED_STATUS]);
    assert_eq!(pc.mtp.stats().event_overflows, 1);
}

// --- Hostile input ---------------------------------------------------------------------

#[test]
fn a_malformed_command_stalls_and_the_next_one_works() {
    pc!(pc);
    pc.pipe.out.push_back(vec![1, 2, 3]);
    pc.pipe
        .out
        .push_back(container(container::DATA, op::OPEN_SESSION, 1, &[]));
    let mut long = container(container::COMMAND, op::OPEN_SESSION, 1, &[0; 24]);
    long.truncate(36);
    pc.pipe.out.push_back(long);
    // Says 20 bytes, sends 16.
    let mut short = container(
        container::COMMAND,
        op::OPEN_SESSION,
        1,
        &[1, 0, 0, 0, 0, 0, 0, 0],
    );
    short.truncate(16);
    pc.pipe.out.push_back(short);
    pc.run();
    assert_eq!(pc.pipe.stalls, 4);
    assert_eq!(pc.mtp.stats().bad_commands, 4);
    let mut st = [0u8; 12];
    pc.mtp.device_status(&mut st);
    assert_eq!(&st[2..4], &[0x1F, 0x20]);
    pc.mtp.cleared();
    pc.mtp.device_status(&mut st);
    assert_eq!(&st[2..4], &[0x01, 0x20]);
    pc.open();
}

#[test]
fn data_for_another_operation_is_drained_and_refused() {
    pc!(pc);
    pc.open();
    pc.tid += 1;
    let tid = pc.tid;
    pc.pipe.out.push_back(container(
        container::COMMAND,
        op::SEND_OBJECT_INFO,
        tid,
        &store_id(0, true).to_le_bytes(),
    ));
    for p in packets(&container(
        container::DATA,
        op::SEND_OBJECT,
        tid,
        &object_info("w.txt", 1, fmt::TEXT),
    )) {
        pc.pipe.out.push_back(p);
    }
    pc.run();
    assert_eq!(pc.collect(tid).rc, rc::INVALID_DATASET);
    assert!(pc.dev.find(0, "w.txt").is_none());
}

#[test]
fn truncated_datasets_are_refused_cleanly() {
    pc!(pc);
    pc.open();
    let full = object_info("trunc.txt", 1, fmt::TEXT);
    for cut in [0, 1, 4, 20, 52, 53, full.len() - 3] {
        let a = pc.op(
            op::SEND_OBJECT_INFO,
            &[store_id(0, true), u32::MAX],
            Some(&full[..cut]),
        );
        assert_eq!(a.rc, rc::INVALID_DATASET, "cut at {cut}");
    }
    // A string without its terminator.
    let mut bad = full.clone();
    // The name's count byte is at 52 [MTP 5.3.1]; its terminator after 9
    // units.
    assert_eq!(bad[52], 10);
    bad[53 + 2 * 9] = b'x';
    assert_eq!(
        pc.op(
            op::SEND_OBJECT_INFO,
            &[store_id(0, true), u32::MAX],
            Some(&bad)
        )
        .rc,
        rc::INVALID_DATASET
    );
    assert!(pc.dev.find(0, "trunc.txt").is_none());
}

#[test]
fn a_full_handle_table_says_busy_not_crashes() {
    pc!(pc, 4);
    pc.open();
    let a = pc.op(op::GET_OBJECT_HANDLES, &[u32::MAX, 0, 0], None);
    assert_eq!(a.rc, rc::DEVICE_BUSY);
    assert!(pc.mtp.stats().table_full > 0);
    // What already has handles still lists.
    assert_eq!(pc.list(u32::MAX).len(), 3);
}

#[test]
fn five_thousand_random_packets_never_panic_or_hang() {
    pc!(pc);
    let mut x = 0x2545_F491_4F6C_DD1Du64;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let codes = [
        0x1001u16, 0x1002, 0x1004, 0x1007, 0x1008, 0x1009, 0x100B, 0x100C, 0x100D, 0x9805, 0x9808,
        0x101B, 0x1019,
    ];
    for i in 0..5000 {
        let r = next();
        let mut pkt: Vec<u8> = (0..(r % 65) as usize).map(|_| next() as u8).collect();
        if i % 3 == 0 && pkt.len() >= 16 {
            // Mostly well-formed commands with random parameters.
            let n = pkt.len().min(32) & !3;
            pkt.truncate(n.max(12));
            let n = pkt.len();
            pkt[..4].copy_from_slice(&(n as u32).to_le_bytes());
            pkt[4..6].copy_from_slice(&container::COMMAND.to_le_bytes());
            pkt[6..8].copy_from_slice(&codes[(r >> 8) as usize % codes.len()].to_le_bytes());
        }
        pc.pipe.out.push_back(pkt);
        if r % 7 == 0 {
            pc.mtp
                .cancel(&mut pc.dev, &mut pc.pipe, (r >> 16) as u32 % 4);
        }
        for _ in 0..4 {
            pc.mtp.poll(&mut pc.dev, &mut pc.pipe);
        }
        pc.pipe.inp.clear();
        pc.pipe.events.clear();
        if r % 101 == 0 {
            pc.mtp.reset(&mut pc.dev, &mut pc.pipe);
        }
    }
    pc.mtp.reset(&mut pc.dev, &mut pc.pipe);
    pc.pipe.out.clear();
    pc.pipe.inp.clear();
    pc.open();
    assert_eq!(pc.op(op::GET_STORAGE_IDS, &[], None).rc, rc::OK);
    assert!(
        pc.dev.find(0, "haret.exe").is_some(),
        "the boot file survived"
    );
}

// --- Types ---------------------------------------------------------------------------

#[test]
fn datetimes_follow_the_pattern_both_ways() {
    for s in [
        "20261025T221500",
        "20261025T221500.7",
        "20261025T221500Z",
        "20261025T221500-0530",
        "00010101T000000.0+2359",
    ] {
        let t = Time::parse(&t(s)).unwrap().unwrap();
        let mut b = [0u8; 22];
        let n = t.format(&mut b);
        assert_eq!(std::str::from_utf8(&b[..n]).unwrap(), s);
    }
    for s in [
        "2026102T221500",
        "20261325T221500",
        "20261025 221500",
        "20261025T246000",
        "20261025T221500.",
        "20261025T221500+01",
        "20261025T221500Zx",
        "20261032T000000",
    ] {
        assert!(Time::parse(&t(s)).is_err(), "{s}");
    }
    assert_eq!(Time::parse(&Text::new()), Ok(None));
}

#[test]
fn strings_are_bounded_and_terminated() {
    assert!(Text::from_units(&[65; 254]).is_some());
    assert!(
        Text::from_units(&[65; 255]).is_none(),
        "255 units leave no room for the terminator"
    );
    assert!(Text::from_units(&[65, 0, 66]).is_none());
    let mut b = [0u8; 16];
    let mut w = Writer::new(&mut b);
    w.text(&t("A"));
    assert_eq!(&b[..5], &[2, 0x41, 0, 0, 0]);
    let mut empty = [9u8; 1];
    Writer::new(&mut empty).text(&Text::new());
    assert_eq!(empty, [0]);
    assert!(Reader::new(&[2, 0x41, 0]).text().is_err(), "cut short");
    assert!(
        Reader::new(&[3, 0x41, 0, 0, 0, 0x42, 0]).text().is_err(),
        "a NUL inside"
    );
    assert!(Reader::new(&[1, 1, 0]).text().is_err(), "no terminator");
    let mut tiny = [0u8; 3];
    let mut w = Writer::new(&mut tiny);
    w.u32(1);
    assert!(w.overflowed());
}

#[test]
fn formats_come_from_extensions() {
    let mut e = Entry::default();
    for (name, f) in [
        ("a.JPG", fmt::EXIF_JPEG),
        ("b.mp3", fmt::MP3),
        ("c", fmt::UNDEFINED),
        ("d.tar.gz", fmt::UNDEFINED),
        ("e.Txt", fmt::TEXT),
    ] {
        e.name = t(name);
        assert_eq!(format_of(&e), f, "{name}");
    }
    e.folder = true;
    assert_eq!(format_of(&e), fmt::ASSOCIATION);
}

#[test]
fn the_handle_index_survives_deletes_and_moves() {
    let mut slots = vec![Slot::default(); 300];
    let mut index = vec![0u32; 600];
    let mut h = Handles::new(&mut slots, &mut index);
    let hs: Vec<u32> = (0..300u64)
        .map(|i| h.get_or_add((i % 2) as u8, i * 4096, 0, false).unwrap())
        .collect();
    assert_eq!(h.get_or_add(0, 99_999_999, 0, false), Err(handles::Full));
    for (i, &x) in hs.iter().enumerate().step_by(3) {
        h.kill(x);
        assert!(h.get(x).is_none());
        assert_eq!(h.find((i % 2) as u8, i as u64 * 4096), None);
    }
    for (i, &x) in hs.iter().enumerate() {
        if i % 3 != 0 {
            assert_eq!(h.find((i % 2) as u8, i as u64 * 4096), Some(x), "{i}");
        }
    }
    h.relocate(hs[1], 1, 7, 0);
    assert_eq!(h.find(1, 7), Some(hs[1]));
    assert_eq!(h.find(1, 4096), None);
    h.kill_store(1);
    assert_eq!(h.find(1, 7), None);
    assert_eq!(h.find(0, 2 * 4096), Some(hs[2]));
}

#[path = "fat_tests.rs"]
mod fat_tests;
