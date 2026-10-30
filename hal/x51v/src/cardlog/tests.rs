extern crate std;

use std::format;
use std::vec;
use std::vec::Vec;

use super::*;

/// A run: the stream, two cards, which ones the program has taken.
struct Run {
    stream: Vec<u8>,
    cards: [Vec<u8>; 2],
    taken: [bool; 2],
    /// Writes each card refuses from now on.
    refuse: [bool; 2],
    /// The cards' programming, waited for apart from the write.
    prog: Prog,
    clock: u32,
}

/// The cards' programming in a [`Run`].
#[derive(Default)]
struct Prog {
    /// Each card's programming fails (found when it is waited for).
    fail: [bool; 2],
    /// Waits for programming, per card.
    settles: [u32; 2],
    /// A card sent a sector and not waited for since.
    unsettled: [bool; 2],
    /// Writes sent while the card still programmed the one before (must
    /// stay 0).
    overlapped: u32,
    /// The writer waits in its writes, as the loader's drivers do.
    waits: bool,
    /// Polls that find a card still programming after each write.
    busy_polls: u32,
    /// Polls left before the card's current programming ends.
    busy_left: [u32; 2],
    /// OS timer ticks each poll of a busy card takes (a real status read).
    busy_ticks: u32,
}

impl Run {
    fn new() -> Self {
        Self {
            stream: Vec::new(),
            cards: [vec![0; 4096 * SECTOR], vec![0; 4096 * SECTOR]],
            taken: [false; 2],
            refuse: [false; 2],
            prog: Prog::default(),
            clock: 0,
        }
    }

    /// A state with a file on each unit given (at different places).
    fn state(units: [bool; 2]) -> State {
        let sectors = (FILE_BYTES / SECTOR) as u32;
        let file = |on: bool, lba| File {
            lba,
            sectors: if on { sectors } else { 0 },
        };
        State::new(
            7,
            [file(units[0], 100), file(units[1], 2000)],
            SdCard::default(),
            0,
        )
    }

    /// Logs `text` and syncs when a line is due, as `log_write` does.
    fn log(&mut self, state: &mut State, text: &[u8]) {
        self.stream.extend_from_slice(text);
        if due(state, text, self.stream.len()) {
            sync(state, self, &mut [0; SECTOR]);
        }
    }

    /// A unit's file as the card holds it.
    fn file(&self, state: &State, unit: usize) -> Option<&[u8]> {
        let f = state.files[unit];
        (f.sectors > 0).then(|| {
            let at = f.lba as usize * SECTOR;
            &self.cards[unit][at..at + f.sectors as usize * SECTOR]
        })
    }

    /// Both files joined.
    fn joined(&self, state: &State) -> Vec<u8> {
        join(self.file(state, SD), self.file(state, CF)).collect()
    }
}

impl Io for Run {
    fn logged(&self) -> usize {
        self.stream.len()
    }
    fn copy(&self, at: usize, out: &mut [u8]) {
        out.copy_from_slice(&self.stream[at..at + out.len()]);
    }
    fn note(&mut self, text: &[u8]) {
        self.stream.extend_from_slice(text);
    }
    fn free(&mut self, unit: usize) -> bool {
        !self.taken[unit]
    }
    fn write(&mut self, unit: usize, lba: u32, sector: &[u8; SECTOR]) -> bool {
        self.clock += 100;
        if self.prog.unsettled[unit] {
            self.prog.overlapped += 1;
        }
        if self.refuse[unit] {
            return false;
        }
        self.prog.unsettled[unit] = true;
        self.prog.busy_left[unit] = self.prog.busy_polls;
        let at = lba as usize * SECTOR;
        self.cards[unit][at..at + SECTOR].copy_from_slice(sector);
        true
    }
    fn defers(&self) -> bool {
        !self.prog.waits
    }
    fn poll(&mut self, unit: usize) -> Option<bool> {
        if self.prog.busy_left[unit] > 0 {
            self.prog.busy_left[unit] -= 1;
            self.clock = self.clock.wrapping_add(self.prog.busy_ticks);
            return None;
        }
        self.clock += 1000;
        self.prog.settles[unit] += 1;
        self.prog.unsettled[unit] = false;
        Some(!self.prog.fail[unit])
    }
    fn ticks(&self) -> u32 {
        self.clock
    }
}

fn text(run: &Run) -> &str {
    core::str::from_utf8(&run.stream).unwrap()
}

#[test]
fn every_line_is_on_the_sd_card_as_logged() {
    let mut run = Run::new();
    let mut s = Run::state([true, true]);
    for i in 0..300 {
        run.log(&mut s, format!("line {i} of the run\n").as_bytes());
        // Every finished line is on the card at once.
        assert_eq!(run.joined(&s), run.stream);
    }
    assert_eq!(
        run.file(&s, SD).unwrap()[..run.stream.len()],
        run.stream[..]
    );
    assert!(run.file(&s, CF).unwrap().iter().all(|&b| b == 0));
    assert_eq!(s.failed, 0);
    assert_eq!(s.on as usize, SD + 1);
}

#[test]
fn a_part_line_waits_for_its_end() {
    let mut run = Run::new();
    let mut s = Run::state([true, false]);
    run.log(&mut s, b"half ");
    assert_eq!(s.writes, 0);
    run.log(&mut s, b"a line\n");
    assert_eq!(s.writes, 1);
    assert_eq!(run.joined(&s), b"half a line\n");
}

#[test]
fn a_long_text_without_line_ends_goes_a_sector_at_a_time() {
    let mut run = Run::new();
    let mut s = Run::state([true, false]);
    let long = vec![b'x'; 3 * SECTOR + 10];
    run.log(&mut s, &long);
    assert_eq!(
        s.synced as usize,
        long.len(),
        "due: a sector's worth waiting"
    );
}

#[test]
fn an_sd_test_moves_the_log_to_the_cf_card_and_the_halves_join() {
    let mut run = Run::new();
    let mut s = Run::state([true, true]);
    for i in 0..50 {
        run.log(&mut s, format!("before {i}\n").as_bytes());
    }
    run.taken[SD] = true;
    let sd_before: Vec<u8> = run.file(&s, SD).unwrap().to_vec();
    for i in 0..50 {
        run.log(&mut s, format!("after {i}\n").as_bytes());
    }
    // SD untouched once taken; the move said once, in the stream.
    assert_eq!(run.file(&s, SD).unwrap(), &sd_before[..]);
    assert_eq!(
        text(&run)
            .matches(
                "[card log: the SD card was taken by the program; continuing on the CF card]\n"
            )
            .count(),
        1
    );
    assert_eq!(run.joined(&s), run.stream);
    assert_eq!(s.on as usize, CF + 1);
}

#[test]
fn a_cf_test_keeps_the_log_on_sd() {
    let mut run = Run::new();
    let mut s = Run::state([true, true]);
    run.taken[CF] = true;
    for i in 0..20 {
        run.log(&mut s, format!("line {i}\n").as_bytes());
    }
    assert!(!text(&run).contains("[card log"));
    assert_eq!(run.joined(&s), run.stream);
}

#[test]
fn a_program_with_both_cards_pauses_the_log_and_the_loader_catches_up() {
    let mut run = Run::new();
    let mut s = Run::state([true, true]);
    run.log(&mut s, b"first\n");
    run.taken = [true, true];
    for i in 0..10 {
        run.log(&mut s, format!("held back {i}\n").as_bytes());
    }
    assert_eq!(
        text(&run).matches("[card log paused: the SD card was taken by the program; the CF card was taken by the program; the loader adds the rest when the program returns]\n").count(),
        1,
        "{}",
        text(&run)
    );
    let on_card = run.joined(&s);
    assert_eq!(on_card, b"first\n");
    // The program is back: the loader has both cards.
    run.taken = [false, false];
    s.reclaim();
    run.note(b"[card log closed]\n");
    sync(&mut s, &mut run, &mut [0; SECTOR]);
    assert_eq!(run.joined(&s), run.stream);
}

#[test]
fn a_refused_write_moves_to_the_other_card() {
    let mut run = Run::new();
    let mut s = Run::state([true, true]);
    run.log(&mut s, b"one\n");
    run.refuse[SD] = true;
    run.log(&mut s, b"two\n");
    assert!(
        text(&run).contains("[card log: the SD card refused a write; continuing on the CF card]\n")
    );
    assert_eq!(s.failed, 1);
    assert_eq!(run.joined(&s), run.stream);
}

#[test]
fn only_one_card_and_it_is_taken_pauses() {
    let mut run = Run::new();
    let mut s = Run::state([true, false]);
    run.taken[SD] = true;
    run.log(&mut s, b"x\n");
    assert_eq!(
        text(&run),
        "x\n[card log paused: the SD card was taken by the program; the loader adds the rest when the program returns]\n"
    );
    assert_eq!(s.writes, 0);
}

#[test]
fn a_full_file_says_so_once_and_keeps_the_rest_in_the_run_log() {
    let mut run = Run::new();
    let mut s = Run::state([true, false]);
    s.files[SD].sectors = 2;
    for _ in 0..40 {
        run.log(&mut s, b"0123456789012345678901234567890123456789\n");
    }
    assert_eq!(s.synced as usize, 2 * SECTOR);
    assert_eq!(text(&run).matches("[card log full").count(), 1);
    assert_eq!(run.joined(&s)[..], run.stream[..2 * SECTOR]);
}

#[test]
fn a_sync_under_way_is_not_entered_again() {
    let mut run = Run::new();
    let mut s = Run::state([true, false]);
    s.busy = 1;
    run.log(&mut s, b"x\n");
    assert_eq!(s.writes, 0);
}

#[test]
fn no_file_is_no_card_log() {
    let s = Run::state([false, false]);
    assert!(!s.valid());
    let mut bad = Run::state([true, false]);
    bad.check = 0;
    assert!(!bad.valid());
}

#[test]
fn the_slowest_write_is_kept() {
    let mut run = Run::new();
    let mut s = Run::state([true, false]);
    run.log(&mut s, b"x\n");
    assert_eq!(s.worst, 100);
}

#[test]
fn names_numbers_and_paths() {
    assert!(name_ok(b"gpubench"));
    assert!(name_ok(b"emu-usb_2"));
    for bad in [
        &b""[..],
        b"Gpu",
        b"a/b",
        b"..",
        b"a b",
        b"a.txt",
        &[b'a'; NAME_MAX + 1],
    ] {
        assert!(!name_ok(bad), "{bad:?}");
    }
    assert_eq!(number_of(b"gpubench", b"gpubench-0012.txt"), Some(12));
    assert_eq!(number_of(b"gpubench", b"GPUBENCH-0012.TXT"), None);
    assert_eq!(number_of(b"gpu", b"gpubench-0012.txt"), None);
    assert_eq!(number_of(b"gpubench", b"gpubench-.txt"), None);
    assert_eq!(number_of(b"gpubench", b"gpubench-12a.txt"), None);
    let mut out = [0u8; 64];
    let n = path(b"gpubench", 7, &mut out);
    assert_eq!(&out[..n], b"rplog/gpubench-0007.txt");
    let n = path(b"x", 123_456, &mut out);
    assert_eq!(&out[..n], b"rplog/x-123456.txt");
}

#[test]
fn the_state_fits_the_page_before_the_sector() {
    assert!(core::mem::size_of::<State>() <= 256);
}

/// After a reset: reads a unit's sector from the run's cards.
fn reader(run: &Run) -> impl FnMut(usize, u32, &mut [u8; SECTOR]) -> bool + '_ {
    |unit, lba, out| {
        let at = lba as usize * SECTOR;
        out.copy_from_slice(&run.cards[unit][at..at + SECTOR]);
        true
    }
}

#[test]
fn after_a_reset_the_same_cards_are_kept_and_caught_up() {
    let mut run = Run::new();
    let mut s = Run::state([true, true]);
    run.log(
        &mut s,
        b"loader header
",
    );
    run.taken[SD] = true;
    for i in 0..40 {
        run.log(
            &mut s,
            format!(
                "on cf {i}
"
            )
            .as_bytes(),
        );
    }
    // The program hangs with a part line held back; a soft reset keeps RAM.
    run.stream.extend_from_slice(
        b"held back
",
    );
    run.taken = [false, false];
    s.reclaim();
    let cards = Run {
        stream: run.stream.clone(),
        cards: run.cards.clone(),
        taken: [false; 2],
        refuse: [false; 2],
        prog: Prog::default(),
        clock: 0,
    };
    assert_eq!(keep_after_reset(&mut s, &cards, &mut reader(&cards)), 0);
    assert!(s.files[SD].sectors > 0 && s.files[CF].sectors > 0);
    sync(&mut s, &mut run, &mut [0; SECTOR]);
    assert_eq!(run.joined(&s), run.stream);
}

#[test]
fn after_a_reset_a_swapped_card_is_never_written() {
    let mut run = Run::new();
    let mut s = Run::state([true, true]);
    for i in 0..5 {
        run.log(
            &mut s,
            format!(
                "line {i}
"
            )
            .as_bytes(),
        );
    }
    // Another card in the slot: other data where the file was.
    let at = s.files[SD].lba as usize * SECTOR;
    run.cards[SD][at..at + 4 * SECTOR].fill(0x41);
    let before = run.cards[SD].clone();
    s.reclaim();
    let cards = Run {
        stream: run.stream.clone(),
        cards: run.cards.clone(),
        taken: [false; 2],
        refuse: [false; 2],
        prog: Prog::default(),
        clock: 0,
    };
    assert_eq!(
        keep_after_reset(&mut s, &cards, &mut reader(&cards)),
        1 << SD
    );
    assert_eq!(s.files[SD].sectors, 0);
    assert_eq!(s.files[CF].sectors, 0, "never written: not needed");
    run.note(
        b"[closing]
",
    );
    sync(&mut s, &mut run, &mut [0; SECTOR]);
    assert_eq!(run.cards[SD], before, "the other card untouched");
}

#[test]
fn after_a_reset_a_cleared_or_shorter_sector_is_not_taken_for_the_run() {
    for damage in [0u8, 1] {
        let mut run = Run::new();
        let mut s = Run::state([true, false]);
        run.log(
            &mut s,
            b"the run's first line
",
        );
        let at = s.files[SD].lba as usize * SECTOR;
        if damage == 0 {
            // Zeros where the file was: nothing to tell it by.
            run.cards[SD][at..at + SECTOR].fill(0);
        } else {
            // Text that differs from the stream's.
            run.cards[SD][at] = b'X';
        }
        s.reclaim();
        let cards = Run {
            stream: run.stream.clone(),
            cards: run.cards.clone(),
            taken: [false; 2],
            refuse: [false; 2],
            prog: Prog::default(),
            clock: 0,
        };
        assert_eq!(
            keep_after_reset(&mut s, &cards, &mut reader(&cards)),
            1 << SD
        );
    }
}

#[test]
fn the_cf_card_is_checked_where_the_log_moved_to_it() {
    let mut run = Run::new();
    let mut s = Run::state([true, true]);
    // Several sectors on SD first, then the move: CF's first sector written
    // is not its file's first.
    for i in 0..60 {
        run.log(
            &mut s,
            format!(
                "sd line {i:03}
"
            )
            .as_bytes(),
        );
    }
    run.taken[SD] = true;
    run.log(
        &mut s,
        b"cf line
",
    );
    assert!(s.first[CF] > 0);
    s.reclaim();
    run.taken[SD] = false;
    let cards = Run {
        stream: run.stream.clone(),
        cards: run.cards.clone(),
        taken: [false; 2],
        refuse: [false; 2],
        prog: Prog::default(),
        clock: 0,
    };
    assert_eq!(keep_after_reset(&mut s, &cards, &mut reader(&cards)), 0);
    assert_eq!(s.on as usize, CF + 1);
}

#[test]
fn a_sector_is_waited_for_only_before_the_cards_next_command() {
    let mut run = Run::new();
    let mut s = Run::state([true, true]);
    run.log(&mut s, b"first line\n");
    // Sent, not waited for: the line costs the transfer alone.
    assert_eq!(run.prog.settles, [0, 0]);
    assert_ne!(s.noted & PENDING, 0);
    for i in 0..40 {
        run.log(&mut s, format!("line {i}\n").as_bytes());
    }
    assert_eq!(
        run.prog.overlapped, 0,
        "a write sent while the card programmed"
    );
    assert_eq!(run.prog.settles[SD], 40);
    // The end of the run (or a program taking the card) waits for it.
    settle(&mut s, &mut run);
    assert_eq!(s.noted & (PENDING | PENDING << CF), 0);
    assert_eq!(run.prog.settles[SD], 41);
    settle(&mut s, &mut run);
    assert_eq!(run.prog.settles[SD], 41, "nothing pending, nothing waited");
    assert_eq!(run.joined(&s), run.stream);
}

#[test]
fn a_sector_that_fails_to_program_is_written_again_on_the_other_card() {
    let mut run = Run::new();
    let mut s = Run::state([true, true]);
    for i in 0..30 {
        run.log(
            &mut s,
            format!("line {i} before the card fails\n").as_bytes(),
        );
    }
    // A line that fills its sector exactly: the next write would start a
    // new sector, so only the rewind puts the failed one on the CF card
    // (a partly filled sector is written again anyway).
    let fill = SECTOR - run.stream.len() % SECTOR;
    let mut line = vec![b'x'; fill - 1];
    line.push(b'\n');
    run.log(&mut s, &line);
    let pending = s.synced;
    assert_eq!(pending as usize % SECTOR, 0);
    run.prog.fail[SD] = true;
    run.refuse[SD] = true;
    run.log(&mut s, b"the line after\n");
    assert_ne!(s.off & 0x100, 0, "the SD card given up on");
    assert_eq!(s.failed, 1);
    assert!(
        text(&run).contains("[card log: the SD card refused a write; continuing on the CF card]")
    );
    // The sector whose programming failed starts the CF card's part, and
    // everything after it is there.
    let k = (pending - 1) as usize / SECTOR;
    assert_eq!(s.first[CF] as usize, k);
    let at = k * SECTOR;
    assert_eq!(
        &run.file(&s, CF).unwrap()[at..run.stream.len()],
        &run.stream[at..]
    );
}

#[test]
fn a_writer_that_waits_leaves_nothing_for_the_next_to_wait_for() {
    // The loader writes the log's start with its own drivers (they wait
    // after every write); the program's first write must not wait for a
    // programming it never started (emu-cardlog: a 6 s timeout, 2026-10-22).
    let mut run = Run::new();
    run.prog.waits = true;
    let mut s = Run::state([true, true]);
    run.log(&mut s, b"the loader's line\n");
    assert_eq!(s.noted & (PENDING | PENDING << CF), 0);
    run.prog.waits = false;
    run.log(&mut s, b"the program's line\n");
    assert_eq!(run.prog.settles, [0, 0]);
    assert_ne!(s.noted & PENDING, 0);
}

#[test]
fn lines_logged_while_the_card_programs_go_on_with_the_next_write() {
    let mut run = Run::new();
    run.prog.busy_polls = 1;
    let mut s = Run::state([true, true]);
    run.log(&mut s, b"line one\n");
    let writes = s.writes;
    // The card still programs line one's sector: line two waits in RAM,
    // nothing waits for the card.
    run.log(&mut s, b"line two\n");
    assert_eq!(s.writes, writes);
    assert_ne!(run.joined(&s), run.stream);
    // Free by line three: one write carries both.
    run.log(&mut s, b"line three\n");
    assert_eq!(s.writes, writes + 1);
    assert_eq!(run.joined(&s), run.stream);
    assert_eq!(
        run.prog.overlapped, 0,
        "a write sent while the card programmed"
    );
    // A burst of lines: about one write for every two, none overlapped.
    for i in 0..100 {
        run.log(&mut s, format!("burst {i}\n").as_bytes());
    }
    assert!(s.writes <= writes + 52, "{} writes", s.writes - writes);
    assert_eq!(run.prog.overlapped, 0);
    // A settle point waits; a sync then puts the rest on (the check-in's
    // catch-up, or the loader at the run's end).
    settle(&mut s, &mut run);
    sync(&mut s, &mut run, &mut [0; SECTOR]);
    settle(&mut s, &mut run);
    assert_eq!(run.joined(&s), run.stream);
}

#[test]
fn a_stuck_card_is_given_up_on_within_a_second() {
    // rp-clocks C2b runs 1-3 on the X51v (2026-10-30): a card stuck busy
    // after the first sector held the program in these waits past the
    // loader's 30 s run limit. With each status read taking ~1 us (3
    // ticks), 20 M polls are a minute; the wait now ends at one second.
    let mut run = Run::new();
    let mut s = Run::state([true, true]);
    run.log(
        &mut s, b"a line
",
    );
    run.prog.busy_left[SD] = u32::MAX;
    run.prog.busy_ticks = 3;
    let t0 = run.clock;
    settle(&mut s, &mut run);
    let took = run.clock.wrapping_sub(t0);
    assert!(took <= SETTLE_TICKS + 3, "{took} ticks");
    assert!(took >= SETTLE_TICKS, "{took} ticks: not given up early");
    assert_ne!(s.off & 0x100, 0, "the SD card given up on");
    assert_eq!(s.noted & PENDING, 0);
}

#[test]
fn a_card_that_never_finishes_programming_is_given_up_on() {
    let mut run = Run::new();
    let mut s = Run::state([true, true]);
    run.log(
        &mut s, b"a line
",
    );
    run.prog.busy_left[SD] = u32::MAX;
    settle(&mut s, &mut run);
    assert_eq!(s.noted & PENDING, 0);
    assert_ne!(s.off & 0x100, 0, "the SD card given up on");
    assert_eq!(s.failed, 1);
    assert_eq!(s.synced, 0, "its sector goes again on the CF card");
}

#[test]
fn slot_paths_are_fixed_names() {
    let mut out = [0u8; 32];
    let n = slot_path(0, &mut out);
    assert_eq!(&out[..n], b"rplog/log-00.txt");
    let n = slot_path(31, &mut out);
    assert_eq!(&out[..n], b"rplog/log-31.txt");
}

/// An index line round-trips; an empty, damaged or misplaced line is
/// an empty slot; a full index gives its oldest run's slot next, an
/// image's highest number and the newest order.
#[test]
fn the_index_says_which_run_each_slot_holds() {
    let mut index = vec![0u8; INDEX_BYTES];
    assert_eq!(index_entry(&index, 3), None);
    let e = Entry {
        order: 7,
        number: 92,
        name: b"gpubench",
    };
    set_index_entry(&mut index, 3, e);
    assert_eq!(index_entry(&index, 3), Some(e));
    assert_eq!(
        &index[3 * INDEX_LINE..3 * INDEX_LINE + 25],
        b"03 00000007 0092 gpubench"
    );
    assert_eq!(index[4 * INDEX_LINE - 1], b'\n');
    // The first empty slot first.
    assert_eq!(choose(&index, b"gpubench"), (0, 7, 92));
    // Every slot full: the oldest run's.
    for s in 0..SLOTS {
        let name: &[u8] = if s % 2 == 0 { b"gpubench" } else { b"light" };
        set_index_entry(
            &mut index,
            s,
            Entry {
                order: 100 + (s as u32 * 7) % SLOTS as u32,
                number: s as u32,
                name,
            },
        );
    }
    let (slot, newest, highest) = choose(&index, b"light");
    assert_eq!(index_entry(&index, slot).unwrap().order, 100);
    assert_eq!(newest, 100 + SLOTS as u32 - 1);
    assert_eq!(highest, 31);
    // A line damaged, or naming another slot: empty.
    index[5 * INDEX_LINE] = b'x';
    assert_eq!(index_entry(&index, 5), None);
    let mut moved = index.clone();
    moved.copy_within(6 * INDEX_LINE..7 * INDEX_LINE, 8 * INDEX_LINE);
    assert_eq!(index_entry(&moved, 8), None);
    // Found by image and number, or an image's newest.
    assert_eq!(find(&index, Some(b"light"), Some(9)), Some((9, 9)));
    let (s, _) = find(&index, Some(b"gpubench"), None).unwrap();
    assert!(index_entry(&index, s).unwrap().name == b"gpubench");
    assert_eq!(find(&index, Some(b"nothing"), None), None);
}
