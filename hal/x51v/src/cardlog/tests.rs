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
    clock: u32,
}

impl Run {
    fn new() -> Self {
        Self {
            stream: Vec::new(),
            cards: [vec![0; 4096 * SECTOR], vec![0; 4096 * SECTOR]],
            taken: [false; 2],
            refuse: [false; 2],
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
        if self.refuse[unit] {
            return false;
        }
        let at = lba as usize * SECTOR;
        self.cards[unit][at..at + SECTOR].copy_from_slice(sector);
        true
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
        clock: 0,
    };
    assert_eq!(keep_after_reset(&mut s, &cards, &mut reader(&cards)), 0);
    assert_eq!(s.on as usize, CF + 1);
}
