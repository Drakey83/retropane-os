use super::*;
use rp_kernel_mm::zpool::Handle;
use std::collections::HashMap;

const N: usize = 2048;

#[derive(Default)]
struct Mem(HashMap<usize, Box<[u8; PAGE]>>);

impl Pages for Mem {
    fn page(&mut self, f: usize) -> &mut [u8; PAGE] {
        self.0.entry(f).or_insert_with(|| Box::new([0xEE; PAGE]))
    }
}

/// The swap file: a megabyte a run, as the card would hold it. Faults on
/// demand: a write or a read refused at a run, a read that comes back with
/// a bit flipped some number of times.
#[derive(Default)]
struct FakeCard {
    file: HashMap<u32, Vec<u8>>,
    fail_write: Option<u32>,
    fail_read: Option<u32>,
    damage: Option<(u32, u32)>,
    writes: Vec<(u32, usize)>,
    reads: u32,
}

impl Card for FakeCard {
    fn write_run<P: Pages>(
        &mut self,
        mem: &mut P,
        run: u32,
        frames: &[u16],
    ) -> Result<(), CardError> {
        if self.fail_write == Some(run) {
            return Err(CardError);
        }
        assert!(frames.len() <= RUN_FRAMES && !frames.is_empty());
        assert!(frames.windows(2).all(|w| w[0] < w[1]), "lowest first");
        let mut out = Vec::with_capacity(frames.len() * PAGE);
        for &f in frames {
            out.extend_from_slice(mem.page(usize::from(f)));
        }
        self.writes.push((run, frames.len()));
        self.file.insert(run, out);
        Ok(())
    }

    fn read_run<P: Pages>(
        &mut self,
        mem: &mut P,
        run: u32,
        frames: &[u16],
    ) -> Result<(), CardError> {
        self.reads += 1;
        if self.fail_read == Some(run) {
            return Err(CardError);
        }
        let data = self.file.get(&run).ok_or(CardError)?;
        assert_eq!(data.len(), frames.len() * PAGE, "the run's length");
        for (k, &f) in frames.iter().enumerate() {
            mem.page(usize::from(f))
                .copy_from_slice(&data[k * PAGE..(k + 1) * PAGE]);
        }
        if let Some((r, times)) = self.damage
            && r == run
            && times > 0
        {
            self.damage = Some((r, times - 1));
            mem.page(usize::from(frames[frames.len() / 2]))[100] ^= 0x10;
        }
        Ok(())
    }
}

struct World {
    zpool: ZPool,
    frames: Box<Frames<N>>,
    mem: Mem,
    card: FakeCard,
    map: Box<TierMap>,
    moves: Moves,
}

impl World {
    fn new() -> Self {
        let mut frames = Box::new(Frames::<N>::new());
        assert!(frames.add_range(0, N).is_ok());
        Self {
            zpool: ZPool::new(),
            frames,
            mem: Mem::default(),
            card: FakeCard::default(),
            map: Box::new(TierMap::new()),
            moves: Moves::new(),
        }
    }

    /// Stores `n` objects of varied sizes and contents: their handles and
    /// bytes.
    fn fill(&mut self, n: usize) -> Vec<(Handle, Vec<u8>)> {
        (0..n)
            .map(|i| {
                let len = 40 + (i * 977) % 3000;
                let data: Vec<u8> = (0..len).map(|k| (i * 31 + k * 7) as u8).collect();
                let h = self
                    .zpool
                    .store(&mut *self.frames, &mut self.mem, &data)
                    .ok()
                    .unwrap();
                (h, data)
            })
            .collect()
    }

    fn out(&mut self) -> Result<Moved, TierError> {
        self.map.swap_out(
            &mut self.zpool,
            &mut *self.frames,
            &mut self.mem,
            &mut self.card,
        )
    }

    fn back(&mut self) -> Result<Moved, TierError> {
        self.map.swap_in(
            &mut self.zpool,
            &mut self.frames,
            &mut self.mem,
            &mut self.card,
            &mut self.moves,
        )
    }

    /// The game: every free frame taken and scribbled over, then given back.
    fn game(&mut self) {
        let mut held = Vec::new();
        while let Some(f) = self.frames.alloc(0) {
            self.mem.page(f).fill(0xCC);
            held.push(f);
        }
        for f in held {
            assert!(self.frames.free(f).is_ok());
        }
    }

    /// Every object reads back as it was (through the moves).
    fn check(&mut self, objects: &[(Handle, Vec<u8>)]) {
        for (h, data) in objects {
            let h = self.moves.rehome(h.raw()).map_or(*h, Handle::from_raw);
            assert_eq!(self.zpool.get(&mut self.mem, h), &data[..]);
        }
    }

    /// Frees every object: the store's lists hold together to the end.
    fn drain(&mut self, objects: &[(Handle, Vec<u8>)]) {
        for (h, _) in objects {
            let h = self.moves.rehome(h.raw()).map_or(*h, Handle::from_raw);
            self.zpool.free(&mut *self.frames, &mut self.mem, h);
        }
    }

    fn empty(&self) {
        assert_eq!(self.zpool.stats(), (0, 0, 0));
        assert!(self.zpool.frame_set().is_empty());
    }
}

#[test]
fn the_store_goes_out_in_megabyte_runs_and_comes_back_where_it_was() {
    let mut w = World::new();
    let objects = w.fill(700);
    let (zframes, count, bytes) = w.zpool.stats();
    assert!(zframes > RUN_FRAMES, "more than one run: {zframes}");
    let free0 = w.frames.free_frames();
    let out = w.out().unwrap();
    let runs = zframes.div_ceil(RUN_FRAMES);
    assert_eq!(
        out,
        Moved {
            frames: zframes as u32,
            runs: runs as u32,
            moved: 0,
            retried: 0
        }
    );
    // Full runs, then the short last one, in order.
    let lens: Vec<_> = w
        .card
        .writes
        .iter()
        .map(|&(r, n)| (r as usize, n))
        .collect();
    let want: Vec<_> = (0..runs)
        .map(|r| (r, RUN_FRAMES.min(zframes - r * RUN_FRAMES)))
        .collect();
    assert_eq!(lens, want);
    // The frames are free for the game, the store empty for its own pages.
    assert_eq!(w.frames.free_frames(), free0 + zframes);
    assert_eq!(w.zpool.stats(), (0, 0, 0));
    assert!(w.map.out);
    assert_eq!(w.out(), Err(TierError::State), "out once");
    w.game();
    let back = w.back().unwrap();
    assert_eq!(back, out, "the same frames, none moved, none retried");
    assert_eq!(w.zpool.stats(), (zframes, count, bytes));
    assert_eq!(w.frames.free_frames(), free0);
    assert!(!w.map.out);
    assert_eq!(w.back(), Err(TierError::State), "back once");
    w.check(&objects);
    // The store goes on working: new objects land in its partial frames.
    let more = w.fill(50);
    w.check(&more);
    w.drain(&objects);
    w.drain(&more);
    w.empty();
    assert_eq!(w.frames.free_frames(), N);
}

#[test]
fn frames_taken_meanwhile_come_back_elsewhere_and_the_store_follows() {
    let mut w = World::new();
    let objects = w.fill(400);
    let zframes = w.zpool.stats().0;
    w.out().unwrap();
    // A kernel allocation kept three of the store's frames, one of them
    // the head of a partial list.
    let set: Vec<usize> = w.map.parked.set.iter().collect();
    let head = (1..=CLASSES)
        .map(|c| w.map.parked.partial[c])
        .find(|&p| p != 0)
        .map(|p| usize::from(p) - 1)
        .unwrap();
    let mut keep = vec![set[0], set[set.len() / 2], head];
    keep.sort_unstable();
    keep.dedup();
    for &f in &keep {
        assert!(w.frames.take(f));
    }
    w.game();
    let back = w.back().unwrap();
    assert_eq!(back.moved, keep.len() as u32);
    for &f in &keep {
        let to = w.moves.get(f).unwrap();
        assert!(!set.contains(&to), "moved to a frame of its own");
        assert!(w.zpool.frame_set().contains(to) && !w.zpool.frame_set().contains(f));
    }
    assert_eq!(w.zpool.stats().0, zframes);
    w.check(&objects);
    let more = w.fill(200);
    w.check(&more);
    w.drain(&objects);
    w.drain(&more);
    w.empty();
    for &f in &keep {
        let _ = w.frames.free(f);
    }
    assert_eq!(w.frames.free_frames(), N);
}

#[test]
fn a_card_that_fails_going_out_leaves_the_store_as_it_was() {
    let mut w = World::new();
    let objects = w.fill(600);
    let before = (w.zpool.stats(), w.frames.free_frames());
    w.card.fail_write = Some(1);
    assert_eq!(w.out(), Err(TierError::Card(1)));
    assert_eq!((w.zpool.stats(), w.frames.free_frames()), before);
    assert!(!w.map.out);
    w.check(&objects);
    w.card.fail_write = None;
    w.out().unwrap();
    w.game();
    w.back().unwrap();
    w.check(&objects);
}

#[test]
fn a_damaged_run_is_read_again_then_refused_with_nothing_lost() {
    let mut w = World::new();
    let objects = w.fill(600);
    w.out().unwrap();
    w.game();
    let free0 = w.frames.free_frames();
    // Damaged once: read again, and it agrees.
    w.card.damage = Some((1, 1));
    let back = w.back().unwrap();
    assert_eq!(back.retried, 1);
    w.check(&objects);
    // Damaged every time, or refused: the return is refused, every frame
    // given back, the map still out; once the card reads, it comes back.
    w.out().unwrap();
    w.game();
    for (fault, err) in [
        ((Some((1, 9)), None), TierError::Damaged(1)),
        ((None, Some(0)), TierError::Card(0)),
    ] {
        (w.card.damage, w.card.fail_read) = fault;
        assert_eq!(w.back(), Err(err));
        assert_eq!(w.frames.free_frames(), free0);
        assert!(w.map.out);
        assert_eq!(w.zpool.stats(), (0, 0, 0));
    }
    (w.card.damage, w.card.fail_read) = (None, None);
    w.back().unwrap();
    w.check(&objects);
}

#[test]
fn a_return_is_refused_before_anything_changes() {
    let mut w = World::new();
    let objects = w.fill(500);
    w.out().unwrap();
    let set: Vec<usize> = w.map.parked.set.iter().collect();
    // More of the store's frames held than may move.
    for &f in &set[..=MAX_MOVES] {
        assert!(w.frames.take(f));
    }
    let free0 = w.frames.free_frames();
    let reads = w.card.reads;
    assert_eq!(w.back(), Err(TierError::Occupied(MAX_MOVES as u32 + 1)));
    assert_eq!((w.frames.free_frames(), w.card.reads), (free0, reads));
    for &f in &set[..=MAX_MOVES] {
        assert!(w.frames.free(f).is_ok());
    }
    // Exactly as many as may move: they come back elsewhere.
    for &f in &set[..MAX_MOVES] {
        assert!(w.frames.take(f));
    }
    assert_eq!(w.back().map(|b| b.moved), Ok(MAX_MOVES as u32));
    w.check(&objects);
    for &f in &set[..MAX_MOVES] {
        assert!(w.frames.free(f).is_ok());
    }
    w.moves = Moves::new();
    w.out().unwrap();
    // No room at all: every free frame held.
    let mut held = Vec::new();
    while let Some(f) = w.frames.alloc(0) {
        held.push(f);
    }
    for &f in &held[..set.len() - 1] {
        assert!(w.frames.free(f).is_ok());
    }
    assert!(matches!(w.back(), Err(TierError::Occupied(_))));
    assert_eq!(w.frames.free_frames(), set.len() - 1);
    for &f in &held[set.len() - 1..] {
        assert!(w.frames.free(f).is_ok());
    }
    // The game's own compressed pages still held.
    let g = w
        .zpool
        .store(&mut *w.frames, &mut w.mem, &[1; 64])
        .ok()
        .unwrap();
    assert_eq!(w.back(), Err(TierError::Busy));
    w.zpool.free(&mut *w.frames, &mut w.mem, g);
    w.back().unwrap();
    w.check(&objects);
}

#[test]
fn an_empty_store_goes_out_and_back_with_no_runs() {
    let mut w = World::new();
    assert_eq!(w.out().unwrap(), Moved::default());
    assert!(w.card.writes.is_empty());
    assert_eq!(w.back().unwrap(), Moved::default());
    assert_eq!(w.card.reads, 0);
}

#[test]
fn the_map_goes_into_a_record_section_and_back() {
    let mut w = World::new();
    let objects = w.fill(700);
    w.out().unwrap();
    let mut buf = vec![0u8; w.map.encoded_len()];
    w.map.encode(&mut buf);
    // 2 KiB is the most the set can take; this one stops at its last frame.
    assert!(buf.len() <= FIXED + 4 * MAX_RUNS + 2048);
    let mut m = Box::new(TierMap::new());
    m.decode(&buf).unwrap();
    assert!(*m == *w.map);
    // Through a real record: the kernel's reserve after a reset.
    let mut reserve = vec![0u8; crate::RESERVE];
    crate::commit(&mut reserve, &mut |wr| {
        wr.section(crate::Kind::TierMap, &buf)
    })
    .unwrap();
    let (_, rec) = crate::newest(&reserve).unwrap();
    let (kind, body) = rec.sections().next().unwrap();
    assert_eq!(kind, crate::Kind::TierMap);
    let mut after = Box::new(TierMap::new());
    after.decode(body).unwrap();
    w.map = after;
    w.game();
    w.back().unwrap();
    w.check(&objects);
}

#[test]
fn a_map_that_does_not_hold_together_is_refused() {
    let mut w = World::new();
    w.fill(300);
    w.out().unwrap();
    let mut good = vec![0u8; w.map.encoded_len()];
    w.map.encode(&mut good);
    let mut m = Box::new(TierMap::new());
    let put = |b: &mut Vec<u8>, at: usize, v: u32| b[at..at + 4].copy_from_slice(&v.to_le_bytes());
    let mut cases: Vec<Vec<u8>> = Vec::new();
    cases.push(good[..FIXED - 1].to_vec()); // too short
    cases.push(good[..good.len() - 1].to_vec()); // a byte short
    let mut b = good.clone();
    b.push(0); // a byte long
    cases.push(b);
    let mut b = good.clone();
    put(&mut b, 0, 2); // out neither 0 nor 1
    cases.push(b);
    let mut b = good.clone();
    put(&mut b, 4, MAX_RUNS as u32 + 1); // too many runs
    cases.push(b);
    let mut b = good.clone();
    put(&mut b, 8, FrameSet::WORDS as u32 + 1); // too many words
    cases.push(b);
    let mut b = good.clone();
    put(&mut b, 12, w.map.parked.counts[0] + 1); // frames disagree with the set
    cases.push(b);
    let mut b = good.clone();
    let last = b.len() - 4;
    put(&mut b, last, 0); // the set's last word gone: frames disagree
    cases.push(b);
    let mut b = good.clone();
    let c = (1..=CLASSES)
        .find(|&c| w.map.parked.partial[c] != 0)
        .unwrap();
    let free = (0..N).find(|&f| !w.map.parked.set.contains(f)).unwrap();
    b[24 + 2 * c..26 + 2 * c].copy_from_slice(&(free as u16 + 1).to_le_bytes()); // a head off the set
    cases.push(b);
    let mut b = good.clone();
    b[24..26].copy_from_slice(&1u16.to_le_bytes()); // class 0 has no list
    cases.push(b);
    for (i, b) in cases.iter().enumerate() {
        assert_eq!(m.decode(b), Err(TierError::BadMap), "case {i}");
        assert!(
            *m == TierMap::new(),
            "case {i}: nothing taken from a bad map"
        );
    }
    m.decode(&good).unwrap();
    // The runs must agree with the set: one run fewer than its frames need.
    let mut fewer = good.clone();
    let runs = w.map.runs as usize;
    put(&mut fewer, 4, runs as u32 - 1);
    fewer.drain(FIXED + 4 * (runs - 1)..FIXED + 4 * runs);
    assert_eq!(m.decode(&fewer), Err(TierError::BadMap));
}

#[test]
fn a_run_hash_sees_order_place_and_every_byte() {
    let mut mem = Mem::default();
    for f in 0..3 {
        mem.page(f).fill(f as u8);
    }
    let h = run_hash(&mut mem, 0, &[0, 1, 2]);
    assert_ne!(h, run_hash(&mut mem, 1, &[0, 1, 2]), "another run's place");
    assert_ne!(h, run_hash(&mut mem, 0, &[0, 2, 1]), "another order");
    assert_ne!(h, run_hash(&mut mem, 0, &[0, 1]), "a frame short");
    mem.page(2)[4095] ^= 1;
    assert_ne!(h, run_hash(&mut mem, 0, &[0, 1, 2]), "the last byte");
}
