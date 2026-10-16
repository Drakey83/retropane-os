use super::*;
use std::collections::BTreeMap;

const SERVER: u8 = 3;
const APP: u8 = 7;
const OTHER: u8 = 9;

/// Handle pages from the heap, filled with junk when handed out (so a
/// table that didn't clear its new page would show it), with an optional
/// limit (memory running out) and every page freed checked.
#[derive(Default)]
struct Pages {
    pages: BTreeMap<u32, Box<[u8; 4096]>>,
    next: u32,
    limit: Option<usize>,
}

impl HandlePages for Pages {
    fn alloc_page(&mut self) -> Option<u32> {
        if self.limit.is_some_and(|l| self.pages.len() >= l) {
            return None;
        }
        self.next += 0x1000;
        self.pages.insert(self.next, Box::new([0xEE; 4096]));
        Some(self.next)
    }
    fn free_page(&mut self, page: u32) {
        assert!(self.pages.remove(&page).is_some(), "page freed twice");
    }
    fn page(&mut self, page: u32) -> &mut [u8; 4096] {
        self.pages.get_mut(&page).expect("a live page")
    }
}

/// The tables with their pages, called as the kernel calls them.
#[derive(Default)]
struct T {
    ipc: Ipc,
    hp: Pages,
}

impl T {
    fn new() -> Self {
        Self::default()
    }
    fn open(&mut self, server: u8, entry: u32) -> Result<(PortId, Handle), Error> {
        self.ipc.open(&mut self.hp, server, entry)
    }
    fn install(&mut self, slot: u8, port: PortId, rights: Rights) -> Result<Handle, Error> {
        self.ipc.install(&mut self.hp, slot, port, rights)
    }
    fn grant(&mut self, from: u8, h: Handle, to: u8, rights: Rights) -> Result<Handle, Error> {
        self.ipc.grant(&mut self.hp, from, h, to, rights)
    }
    fn call(&mut self, tid: u8, slot: u8, h: Handle) -> Result<Enter, Error> {
        self.ipc.call(&mut self.hp, tid, slot, h)
    }
    fn ret(&mut self, tid: u8) -> Result<(), Error> {
        self.ipc.ret(tid)
    }
    fn fault(&mut self, tid: u8) -> bool {
        self.ipc.fault(tid)
    }
    fn depth(&self, tid: u8) -> usize {
        self.ipc.depth(tid)
    }
    fn restart(&mut self, port: PortId, supervisor: Option<u8>) -> Result<(), Error> {
        self.ipc.restart(&mut self.hp, port, supervisor)
    }
    fn handle_count(&mut self, slot: u8) -> usize {
        self.ipc.handle_count(&mut self.hp, slot)
    }
    fn ports_of(&self, slot: u8) -> usize {
        self.ipc.ports_of(slot)
    }
    fn exit(&mut self, slot: u8) -> Result<(), Error> {
        self.ipc.exit(&mut self.hp, slot)
    }
    fn close(&mut self, slot: u8, h: Handle) -> Result<(), Error> {
        self.ipc.close(&mut self.hp, slot, h)
    }
    /// The word behind a handle, for checks.
    fn word(&mut self, slot: u8, h: Handle) -> Option<u32> {
        let s = Ipc::slot_ok(slot).ok()?;
        self.ipc.tables[s].get(&mut self.hp, usize::from(h.0))
    }
}

fn setup() -> (T, PortId, Handle) {
    let mut ipc = T::new();
    let (port, _server_h) = ipc.open(SERVER, 0x1000).unwrap();
    let h = ipc.install(APP, port, Rights::CALL).unwrap();
    (ipc, port, h)
}

#[test]
fn a_call_goes_to_the_server_and_comes_back() {
    let (mut ipc, _, h) = setup();
    let e = ipc.call(5, APP, h).unwrap();
    assert_eq!(
        e,
        Enter {
            slot: SERVER,
            entry: 0x1000,
            depth: 1
        }
    );
    assert_eq!(ipc.depth(5), 1);
    assert_eq!(ipc.ret(5), Ok(()));
    assert_eq!(ipc.ret(5), Err(Error::NotInCall));
}

#[test]
fn handles_belong_to_their_process() {
    let (mut ipc, _, h) = setup();
    // The same number in another process's table is nothing.
    assert_eq!(ipc.call(5, OTHER, h), Err(Error::BadHandle));
    // Forged numbers.
    assert_eq!(ipc.call(5, APP, Handle(31)), Err(Error::BadHandle));
    assert_eq!(ipc.call(5, APP, Handle(200)), Err(Error::BadHandle));
    // Out-of-range slots and threads.
    assert_eq!(ipc.call(5, 0, h), Err(Error::Range));
    assert_eq!(ipc.call(5, 64, h), Err(Error::Range));
    // Slot 1 (shared code) and slot 33 (the shared-file area) hold no
    // process.
    assert_eq!(ipc.install(1, PortId(0), Rights::CALL), Err(Error::Range));
    assert_eq!(
        ipc.install(LAST_SLOT + 1, PortId(0), Rights::CALL),
        Err(Error::Range)
    );
    assert_eq!(
        ipc.call(0, APP, h),
        Err(Error::Range),
        "thread 0 is no thread"
    );
    // Slots between CE's and the native ones (the shared area, the kernel)
    // and slot 127 (vectors) hold no process.
    for slot in [33, 64, 95, 127, 255] {
        assert_eq!(
            ipc.install(slot, PortId(0), Rights::CALL),
            Err(Error::Range),
            "slot {slot}"
        );
    }
}

#[test]
fn rights_never_grow() {
    let (mut ipc, port, h) = setup();
    // CALL only: can't hand it on.
    assert_eq!(ipc.grant(APP, h, OTHER, Rights::CALL), Err(Error::NoRight));
    let g = ipc.install(APP, port, Rights::GRANT).unwrap();
    // GRANT only: can hand on GRANT, not CALL it never had.
    assert_eq!(ipc.grant(APP, g, OTHER, Rights::CALL), Err(Error::NoRight));
    let og = ipc.grant(APP, g, OTHER, Rights::GRANT).unwrap();
    assert_eq!(ipc.call(5, OTHER, og), Err(Error::NoRight), "no CALL right");
}

#[test]
fn nesting_is_bounded_and_unwinds() {
    let mut ipc = T::new();
    let mut slots = vec![];
    let mut handles = vec![];
    // A chain of services: slot 10 calls 11 calls 12 ...
    for s in 10..10 + MAX_DEPTH as u8 + 1 {
        let (port, _) = ipc.open(s + 1, u32::from(s)).unwrap();
        handles.push(ipc.install(s, port, Rights::CALL).unwrap());
        slots.push(s);
    }
    for d in 0..MAX_DEPTH {
        let e = ipc.call(1, slots[d], handles[d]).unwrap();
        assert_eq!(e.depth, d + 1);
    }
    assert_eq!(
        ipc.call(1, slots[MAX_DEPTH], handles[MAX_DEPTH]),
        Err(Error::TooDeep)
    );
    for _ in 0..MAX_DEPTH {
        assert_eq!(ipc.ret(1), Ok(()));
    }
}

#[test]
fn a_faulting_server_unwinds_to_its_caller() {
    let (mut ipc, _, h) = setup();
    ipc.call(5, APP, h).unwrap();
    assert!(ipc.fault(5));
    assert_eq!(ipc.depth(5), 0);
    // A fault outside any call is the thread's own process's.
    assert!(!ipc.fault(5));
}

#[test]
fn a_restarted_server_isnt_reached_through_old_handles() {
    let (mut ipc, port, old) = setup();
    ipc.restart(port, None).unwrap();
    assert_eq!(ipc.call(5, APP, old), Err(Error::Stale));
    let new = ipc.install(APP, port, Rights::CALL).unwrap();
    assert!(ipc.call(5, APP, new).is_ok());
    // The server's own handle followed the restart.
    let (mut ipc2, port2, _) = setup();
    ipc2.restart(port2, None).unwrap();
    assert!(ipc2.grant(SERVER, Handle(0), OTHER, Rights::CALL).is_ok());
}

#[test]
fn an_exited_server_closes_its_ports_and_reuse_cant_be_reached_by_old_handles() {
    let (mut ipc, _, h) = setup();
    ipc.exit(SERVER).unwrap();
    assert_eq!(ipc.call(5, APP, h), Err(Error::Closed));
    // A new port takes the old one's place: the old handle is stale.
    let (_, _) = ipc.open(OTHER, 0x2000).unwrap();
    assert_eq!(ipc.call(5, APP, h), Err(Error::Stale));
    // And an exited process's table is empty.
    ipc.exit(APP).unwrap();
    assert_eq!(ipc.call(5, APP, h), Err(Error::BadHandle));
}

#[test]
fn tables_are_bounded() {
    let mut ipc = T::new();
    let (port, _) = ipc.open(SERVER, 0).unwrap();
    // The server's own handle is one of the server's, not the app's.
    for _ in 0..MAX_HANDLES {
        ipc.install(APP, port, Rights::CALL).unwrap();
    }
    assert_eq!(ipc.install(APP, port, Rights::CALL), Err(Error::Full));
    assert_eq!(ipc.ipc.handle_pages(APP), HANDLE_PAGES, "the quota");
    let mut ipc = T::new();
    for i in 0..MAX_PORTS {
        ipc.open(FIRST_SLOT + (i % 31) as u8, 0).unwrap();
    }
    assert_eq!(ipc.open(SERVER, 0), Err(Error::Full));
}

#[test]
fn a_table_grows_into_pages_only_when_it_needs_them() {
    let mut ipc = T::new();
    let (port, _) = ipc.open(SERVER, 0x1000).unwrap();
    let mut handles = Vec::new();
    for _ in 0..INLINE_HANDLES {
        handles.push(ipc.install(APP, port, Rights::CALL).unwrap());
    }
    assert_eq!(ipc.ipc.handle_pages(APP), 0, "inline handles need no page");
    // The next one takes a page, cleared (the allocator fills with junk).
    let h = ipc.install(APP, port, Rights::CALL).unwrap();
    assert_eq!(usize::from(h.0), INLINE_HANDLES);
    assert_eq!(ipc.ipc.handle_pages(APP), 1);
    assert_eq!(ipc.handle_count(APP), INLINE_HANDLES + 1);
    // A handle in a page works like an inline one, and a forged number past
    // the page's handles is nothing.
    assert!(ipc.call(5, APP, h).is_ok());
    ipc.ret(5).unwrap();
    assert_eq!(ipc.call(5, APP, Handle(h.0 + 1)), Err(Error::BadHandle));
    assert_eq!(ipc.call(5, APP, Handle(u16::MAX)), Err(Error::BadHandle));
    // A freed inline handle is reused before the page's.
    ipc.close(APP, handles[3]).unwrap();
    assert_eq!(ipc.install(APP, port, Rights::CALL), Ok(handles[3]));
    // Restarting the server renews page handles too.
    ipc.restart(port, Some(APP)).unwrap();
    assert!(ipc.call(5, APP, h).is_ok());
    ipc.ret(5).unwrap();
    // Exit gives every page back.
    ipc.exit(APP).unwrap();
    assert_eq!(ipc.ipc.handle_pages(APP), 0);
    assert!(ipc.hp.pages.is_empty(), "pages leaked");
    assert_eq!(ipc.call(5, APP, h), Err(Error::BadHandle));
}

#[test]
fn out_of_memory_refuses_the_handle_and_changes_nothing() {
    let mut ipc = T::new();
    ipc.hp.limit = Some(0);
    let (port, _) = ipc.open(SERVER, 0x1000).unwrap();
    for _ in 0..INLINE_HANDLES {
        ipc.install(APP, port, Rights::CALL).unwrap();
    }
    assert_eq!(ipc.install(APP, port, Rights::CALL), Err(Error::Full));
    assert_eq!(ipc.handle_count(APP), INLINE_HANDLES);
    assert_eq!(ipc.ipc.handle_pages(APP), 0);
}

#[test]
fn native_processes_have_their_own_tables() {
    const NATIVE: u8 = FIRST_NATIVE_SLOT;
    let mut ipc = T::new();
    let (port, _) = ipc.open(LAST_NATIVE_SLOT, 0x3000).unwrap();
    let h = ipc.install(NATIVE, port, Rights::CALL).unwrap();
    let e = ipc.call(5, NATIVE, h).unwrap();
    assert_eq!(e.slot, LAST_NATIVE_SLOT);
    ipc.ret(5).unwrap();
    // The same number means nothing in a CE process, and the other way.
    assert_eq!(ipc.call(5, LAST_SLOT, h), Err(Error::BadHandle));
    let c = ipc.install(APP, port, Rights::CALL).unwrap();
    assert_eq!(ipc.handle_count(NATIVE), 1);
    ipc.exit(NATIVE).unwrap();
    assert!(
        ipc.call(5, APP, c).is_ok(),
        "a CE client of a native server"
    );
}

#[test]
fn every_thread_number_can_call() {
    let (mut ipc, _, h) = setup();
    for tid in [1, 128, MAX_THREADS as u8] {
        assert!(ipc.call(tid, APP, h).is_ok(), "thread {tid}");
        assert_eq!(ipc.depth(tid), 1);
        ipc.ret(tid).unwrap();
    }
}

/// Random operations from hostile processes: never a panic, and a call
/// only ever reaches a server through a live handle with CALL.
#[test]
fn random_operations_never_break_the_rules() {
    let mut ipc = T::new();
    let mut x: u64 = 0x1BC0_2026;
    let mut rnd = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let mut ports: Vec<PortId> = Vec::new();
    for _ in 0..100_000 {
        let slot = (rnd() % 128) as u8;
        let tid = rnd() as u8;
        // Mostly small numbers (inline and the first page), some anywhere.
        let h = Handle(if rnd() % 8 == 0 {
            rnd() as u16
        } else {
            (rnd() % 40) as u16
        });
        match rnd() % 9 {
            0 => {
                if let Ok((p, _)) = ipc.open(slot, rnd() as u32) {
                    ports.push(p);
                }
            }
            1 if !ports.is_empty() => {
                let p = ports[(rnd() as usize) % ports.len()];
                let _ = ipc.install(slot, p, Rights((rnd() % 4) as u8));
            }
            2 => {
                let to = (rnd() % 128) as u8;
                let _ = ipc.grant(slot, h, to, Rights((rnd() % 4) as u8));
            }
            3 | 4 => {
                if let Ok(e) = ipc.call(tid, slot, h) {
                    // It was a live handle with CALL in the caller's table.
                    let entry = Entry::decode(ipc.word(slot, h).unwrap()).unwrap();
                    assert!(entry.rights().contains(Rights::CALL));
                    let p = ipc.ipc.ports[usize::from(entry.port())];
                    assert!(
                        p.open == 1 && p.generation == entry.generation() && p.server == e.slot
                    );
                    assert!(e.depth <= MAX_DEPTH);
                }
            }
            5 => {
                let _ = ipc.ret(tid);
            }
            6 if !ports.is_empty() => {
                let _ = ipc.restart(ports[(rnd() as usize) % ports.len()], Some(slot));
            }
            7 => {
                let _ = ipc.exit(slot);
            }
            _ => {
                let _ = ipc.close(slot, h);
            }
        }
    }
}

#[test]
fn a_supervisor_keeps_its_handle_across_a_restart() {
    const MANAGER: u8 = 4;
    let mut ipc = T::new();
    let (port, _) = ipc.open(SERVER, 0x1000).unwrap();
    let m = ipc.install(MANAGER, port, Rights::ALL).unwrap();
    let old = ipc.grant(MANAGER, m, APP, Rights::CALL).unwrap();
    ipc.restart(port, Some(MANAGER)).unwrap();
    // The app's handle is stale; the manager's still works and hands out a
    // fresh one, with no more rights than asked for.
    assert_eq!(ipc.call(5, APP, old), Err(Error::Stale));
    let new = ipc
        .grant(MANAGER, m, APP, Rights::from_bits(0xFFFF_FFF1))
        .unwrap();
    assert!(ipc.call(5, APP, new).is_ok());
    ipc.ret(5).unwrap();
    assert_eq!(
        ipc.grant(APP, new, OTHER, Rights::CALL),
        Err(Error::NoRight),
        "granted CALL only"
    );
    // A supervisor outside the process slots is refused, and nothing changes.
    assert_eq!(ipc.restart(port, Some(0)), Err(Error::Range));
    assert!(ipc.call(5, APP, new).is_ok());
}

#[test]
fn an_exited_process_keeps_nothing() {
    let mut ipc = T::new();
    let (port, _) = ipc.open(SERVER, 0x1000).unwrap();
    ipc.install(APP, port, Rights::CALL).unwrap();
    let (_, _) = ipc.open(APP, 0x2000).unwrap();
    assert_eq!((ipc.handle_count(APP), ipc.ports_of(APP)), (2, 1));
    ipc.exit(APP).unwrap();
    assert_eq!((ipc.handle_count(APP), ipc.ports_of(APP)), (0, 0));
    assert_eq!(ipc.handle_count(0), 0, "out of range counts nothing");
}
