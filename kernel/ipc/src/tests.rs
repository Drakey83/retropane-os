use super::*;

const SERVER: u8 = 3;
const APP: u8 = 7;
const OTHER: u8 = 9;

fn setup() -> (Ipc, PortId, Handle) {
    let mut ipc = Ipc::new();
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
    assert_eq!(ipc.ret(5), Ok(Return { slot: APP }));
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
    assert_eq!(ipc.call(200, APP, h), Err(Error::Range));
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
fn nesting_is_bounded_and_unwinds_in_order() {
    let mut ipc = Ipc::new();
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
    for d in (0..MAX_DEPTH).rev() {
        assert_eq!(ipc.ret(1), Ok(Return { slot: slots[d] }));
    }
}

#[test]
fn a_faulting_server_unwinds_to_its_caller() {
    let (mut ipc, _, h) = setup();
    ipc.call(5, APP, h).unwrap();
    assert_eq!(ipc.fault(5), Some(Return { slot: APP }));
    assert_eq!(ipc.depth(5), 0);
    // A fault outside any call is the thread's own process's.
    assert_eq!(ipc.fault(5), None);
}

#[test]
fn a_restarted_server_isnt_reached_through_old_handles() {
    let (mut ipc, port, old) = setup();
    ipc.restart(port).unwrap();
    assert_eq!(ipc.call(5, APP, old), Err(Error::Stale));
    let new = ipc.install(APP, port, Rights::CALL).unwrap();
    assert!(ipc.call(5, APP, new).is_ok());
    // The server's own handle followed the restart.
    let (mut ipc2, port2, _) = setup();
    ipc2.restart(port2).unwrap();
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
    let mut ipc = Ipc::new();
    let (port, _) = ipc.open(SERVER, 0).unwrap();
    for _ in 0..MAX_HANDLES {
        ipc.install(APP, port, Rights::CALL).unwrap();
    }
    assert_eq!(ipc.install(APP, port, Rights::CALL), Err(Error::Full));
    let mut ipc = Ipc::new();
    for i in 0..MAX_PORTS {
        ipc.open(1 + (i % 60) as u8, 0).unwrap();
    }
    assert_eq!(ipc.open(SERVER, 0), Err(Error::Full));
}

/// Random operations from hostile processes: never a panic, and a call
/// only ever reaches a server through a live handle with CALL.
#[test]
fn random_operations_never_break_the_rules() {
    let mut ipc = Ipc::new();
    let mut x: u64 = 0x1BC0_2026;
    let mut rnd = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let mut ports: Vec<PortId> = Vec::new();
    for _ in 0..100_000 {
        let slot = (rnd() % 66) as u8;
        let tid = (rnd() % 130) as u8;
        let h = Handle((rnd() % 40) as u8);
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
                let to = (rnd() % 66) as u8;
                let _ = ipc.grant(slot, h, to, Rights((rnd() % 4) as u8));
            }
            3 | 4 => {
                if let Ok(e) = ipc.call(tid, slot, h) {
                    // It was a live handle with CALL in the caller's table.
                    let entry = ipc.tables[usize::from(slot)][usize::from(h.0)].unwrap();
                    assert!(entry.rights.contains(Rights::CALL));
                    let p = ipc.ports[usize::from(entry.port)].unwrap();
                    assert!(p.open && p.generation == entry.generation && p.server == e.slot);
                    assert!(e.depth <= MAX_DEPTH);
                }
            }
            5 => {
                let _ = ipc.ret(tid);
            }
            6 if !ports.is_empty() => {
                let _ = ipc.restart(ports[(rnd() as usize) % ports.len()]);
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
