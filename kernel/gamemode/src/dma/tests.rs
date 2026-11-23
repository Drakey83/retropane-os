use super::*;

/// A game's memory: `pages` (virtual page, physical page, writable) and
/// one device window (virtual base, physical base, 4 KiB).
struct World {
    pages: Vec<(u32, u32, bool)>,
    window: Option<(u32, u32)>,
}

impl GameMemory for World {
    fn phys(&mut self, va: u32, write: bool) -> Option<u32> {
        let (_, pa, w) = self.pages.iter().find(|p| p.0 == va & !0xFFF)?;
        (!write || *w).then_some(pa + (va & 0xFFF))
    }
    fn device(&mut self, va: u32) -> Option<(u32, u32)> {
        let (vb, pb) = self.window?;
        (va >= vb && va - vb < 4096).then(|| (pb + (va - vb), 4096 - (va - vb)))
    }
}

/// Four pages at 0x7800_0000: the first two contiguous, the third
/// elsewhere, the fourth read-only; a window at 0x0060_1000 for the
/// device at 0x4110_0000.
fn world() -> World {
    World {
        pages: vec![
            (0x7800_0000, 0xA200_0000, true),
            (0x7800_1000, 0xA200_1000, true),
            (0x7800_2000, 0xA300_0000, true),
            (0x7800_3000, 0xA300_1000, false),
        ],
        window: Some((0x0060_1000, 0x4110_0000)),
    }
}

const MEM: u32 = dcmd::INC_SRC | dcmd::INC_TRG;
const OUT: u32 = 0xA800_0100;

fn go(w: &World, chain: &[Desc]) -> Result<Vec<HwDesc>, DmaError> {
    let mut w = World {
        pages: w.pages.clone(),
        window: w.window,
    };
    let mut out = [[0xDEAD; 4]; MAX_OUT];
    let n = translate(&mut w, chain, &mut out, OUT)?;
    Ok(out[..n].to_vec())
}

#[test]
fn a_contiguous_copy_is_one_descriptor_with_physical_addresses() {
    let w = world();
    let got = go(
        &w,
        &[Desc {
            src: 0x7800_0100,
            dst: 0x7800_1100,
            cmd: MEM | 0x400,
        }],
    )
    .unwrap();
    assert_eq!(got, [[STOP, 0xA200_0100, 0xA200_1100, MEM | 0x400]]);
}

#[test]
fn a_descriptor_splits_where_its_pages_are_not_contiguous() {
    let w = world();
    // 0x7800_1F00..0x7800_2100 crosses into the page placed elsewhere.
    let got = go(
        &w,
        &[Desc {
            src: 0x7800_1F00,
            dst: 0x7800_0000,
            cmd: MEM | 0x200,
        }],
    )
    .unwrap();
    assert_eq!(
        got,
        [
            [OUT + 16, 0xA200_1F00, 0xA200_0000, MEM | 0x100],
            [STOP, 0xA300_0000, 0xA200_0100, MEM | 0x100],
        ]
    );
}

#[test]
fn both_sides_split_at_their_own_edges() {
    let w = world();
    // The source crosses 0x7800_2000 after 0x80 bytes; the target crosses
    // 0x7800_2000 after 0x100: three pieces.
    let got = go(
        &w,
        &[Desc {
            src: 0x7800_1F80,
            dst: 0x7800_1F00,
            cmd: MEM | 0x200,
        }],
    )
    .unwrap();
    let lens: Vec<u32> = got.iter().map(|d| d[3] & dcmd::LEN).collect();
    assert_eq!(lens, [0x80, 0x80, 0x100]);
    assert_eq!(got[1][1], 0xA300_0000);
    assert_eq!(got[2][2], 0xA300_0000);
    assert_eq!(lens.iter().sum::<u32>(), 0x200);
}

#[test]
fn a_chain_is_linked_in_order_and_ends_with_stop() {
    let w = world();
    let d = Desc {
        src: 0x7800_0000,
        dst: 0x7800_1000,
        cmd: MEM | 0x40,
    };
    let got = go(&w, &[d, d, d]).unwrap();
    let links: Vec<u32> = got.iter().map(|d| d[0]).collect();
    assert_eq!(links, [OUT + 16, OUT + 32, STOP]);
}

#[test]
fn memory_not_the_games_or_read_only_is_refused() {
    let w = world();
    // The kernel's own memory, named by its physical address.
    let kernel = Desc {
        src: 0x7800_0000,
        dst: 0xA800_8000,
        cmd: MEM | 0x40,
    };
    assert_eq!(go(&w, &[kernel]), Err(DmaError::NotYours(0)));
    // Running off the end of the game's pages.
    let off_end = Desc {
        src: 0x7800_0000,
        dst: 0x7800_2F00,
        cmd: MEM | 0x200,
    };
    assert_eq!(go(&w, &[off_end]), Err(DmaError::NotYours(0)));
    // A target in the read-only page; a source there is fine.
    let ro = Desc {
        src: 0x7800_0000,
        dst: 0x7800_3000,
        cmd: MEM | 0x40,
    };
    assert_eq!(go(&w, &[ro]), Err(DmaError::NotYours(0)));
    let from_ro = Desc {
        src: 0x7800_3000,
        dst: 0x7800_0000,
        cmd: MEM | 0x40,
    };
    assert!(go(&w, &[from_ro]).is_ok());
    // An address past the top wraps nowhere.
    let wrap = Desc {
        src: 0xFFFF_FFF0,
        dst: 0x7800_0000,
        cmd: MEM | 0x40,
    };
    assert_eq!(go(&w, &[wrap]), Err(DmaError::NotYours(0)));
}

#[test]
fn a_refusal_anywhere_writes_nothing() {
    let good = Desc {
        src: 0x7800_0000,
        dst: 0x7800_1000,
        cmd: MEM | 0x40,
    };
    let bad = Desc {
        dst: 0xA800_8000,
        ..good
    };
    let mut out = [[0xDEAD; 4]; MAX_OUT];
    assert_eq!(
        translate(&mut world(), &[good, good, bad], &mut out, OUT),
        Err(DmaError::NotYours(2))
    );
    assert!(out.iter().all(|d| *d == [0xDEAD; 4]));
}

#[test]
fn a_device_side_must_be_a_granted_window() {
    let w = world();
    // Memory into the device's FIFO: the target holds still.
    let to_fifo = Desc {
        src: 0x7800_0000,
        dst: 0x0060_1040,
        cmd: dcmd::INC_SRC | dcmd::FLOW_TRG | 0x1000,
    };
    assert_eq!(
        go(&w, &[to_fifo]).unwrap(),
        [[
            STOP,
            0xA200_0000,
            0x4110_0040,
            dcmd::INC_SRC | dcmd::FLOW_TRG | 0x1000
        ]]
    );
    // A FIFO address outside the window, or a window not granted.
    let elsewhere = Desc {
        dst: 0x4110_0040,
        ..to_fifo
    };
    assert_eq!(go(&w, &[elsewhere]), Err(DmaError::NoGrant(0)));
    let none = World {
        window: None,
        ..world()
    };
    assert_eq!(go(&none, &[to_fifo]), Err(DmaError::NoGrant(0)));
    // The window's last bytes hold no whole word.
    let edge = Desc {
        dst: 0x0060_1FFE,
        ..to_fifo
    };
    assert_eq!(go(&w, &[edge]), Err(DmaError::NoGrant(0)));
    // From the device into memory, split at the memory side's edge only.
    let from_fifo = Desc {
        src: 0x0060_1040,
        dst: 0x7800_1F00,
        cmd: dcmd::INC_TRG | dcmd::FLOW_SRC | 0x200,
    };
    let got = go(&w, &[from_fifo]).unwrap();
    assert_eq!(got.len(), 2);
    assert!(got.iter().all(|d| d[1] == 0x4110_0040));
}

#[test]
fn refused_flags_lengths_and_counts() {
    let w = world();
    let d = Desc {
        src: 0x7800_0000,
        dst: 0x7800_1000,
        cmd: MEM | 0x40,
    };
    for bad in [
        dcmd::END_IRQ,
        dcmd::START_IRQ,
        dcmd::FLY_BY_S,
        dcmd::FLY_BY_T,
        dcmd::CMP_EN,
        dcmd::ADDR_MODE,
    ] {
        let x = Desc {
            cmd: d.cmd | bad,
            ..d
        };
        assert_eq!(go(&w, &[d, x]), Err(DmaError::Flags(1)), "{bad:#x}");
    }
    let empty = Desc { cmd: MEM, ..d };
    assert_eq!(go(&w, &[empty]), Err(DmaError::Flags(0)));
    let still = Desc { cmd: 0x40, ..d };
    assert_eq!(go(&w, &[still]), Err(DmaError::Flags(0)));
    assert_eq!(go(&w, &[]), Err(DmaError::Count));
    assert_eq!(go(&w, &[d; MAX_IN + 1]), Err(DmaError::Count));
    assert_eq!(go(&w, &[d; MAX_IN]).map(|c| c.len()), Ok(MAX_IN));
}

#[test]
fn a_chain_that_splits_past_the_kernels_room_is_refused() {
    // Every page apart: each 0x1100-byte copy from page offset 0xF80
    // splits three ways on its source.
    let w = World {
        pages: (0..8u32)
            .map(|k| (0x7800_0000 + k * 0x1000, 0xA200_0000 + k * 0x2000, true))
            .collect(),
        window: None,
    };
    let d = Desc {
        src: 0x7800_0F80,
        dst: 0x7800_4000,
        cmd: MEM | 0x1100,
    };
    let three = go(&w, &[d]).unwrap();
    assert_eq!(three.len(), 4, "the target splits too: {three:x?}");
    let fits = MAX_OUT / 4;
    assert!(go(&w, &vec![d; fits]).is_ok());
    assert_eq!(go(&w, &vec![d; fits + 1]), Err(DmaError::TooLong));
}

#[test]
fn cache_calls_cover_only_the_games_memory() {
    let mut w = world();
    assert!(check_range(&mut w, CacheOp::Clean, 0x7800_0F00, 0x200));
    assert!(check_range(&mut w, CacheOp::Clean, 0x7800_3000, 0x40));
    assert!(!check_range(&mut w, CacheOp::Invalidate, 0x7800_3000, 0x40));
    assert!(!check_range(
        &mut w,
        CacheOp::CleanInvalidate,
        0x7800_2F00,
        0x200
    ));
    assert!(!check_range(&mut w, CacheOp::Clean, 0x7800_3F00, 0x200));
    assert!(!check_range(&mut w, CacheOp::Clean, 0xA800_8000, 0x40));
    assert!(!check_range(&mut w, CacheOp::Clean, 0x7800_0000, 0));
    assert!(!check_range(&mut w, CacheOp::Clean, 0xFFFF_FFC0, 0x80));
    // Past the most one call covers, though every page is the game's.
    let mut big = World {
        pages: (0..32u32)
            .map(|k| (0x7800_0000 + k * 0x1000, 0xA200_0000 + k * 0x1000, true))
            .collect(),
        window: None,
    };
    assert!(check_range(
        &mut big,
        CacheOp::Clean,
        0x7800_0000,
        CACHE_MOST
    ));
    assert!(!check_range(
        &mut big,
        CacheOp::Clean,
        0x7800_0000,
        CACHE_MOST + 1
    ));
    assert!(check_range(&mut w, CacheOp::Drain, 0, 0));
    assert_eq!(CacheOp::from_u32(3), Some(CacheOp::Drain));
    assert_eq!(CacheOp::from_u32(4), None);
}
