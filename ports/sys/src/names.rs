//! A folder's names kept in RAM once it has been listed, so that asking
//! what a path names costs no card read.
//!
//! Why (R276, measured 2026-11-16): Half-Life's start asks what
//! `/Ports/valve/gfx/shell` is 4,002 times and `/Ports/valve/gfx` 2,016
//! times, and every answer walked each folder on the way from the card,
//! one read command an entry: 291,728 of the start's 321,517 read
//! commands answered look-ups, and on the X51v's card that was twenty
//! minutes.
//!
//! Iterate, don't imitate (rule 0). An operating system answers this
//! with a general cache of the card's sectors or of whole directory
//! entries (megabytes, and still a search through each folder's sectors
//! a look-up); a console port most often reads the game's whole file
//! list into a hash table at its start, the folders nobody asks about
//! included. Ours keeps exactly what a look-up needs and nothing else:
//! for each folder a program has looked into, its names in the order a
//! binary search wants, each with where it is, what it is, its size and
//! its date (32 bytes and the name's own bytes). A name that is not
//! there is answered without the card, a name that is there too, and the
//! room is the caller's to size: a folder that does not fit is walked
//! from the card as before, so nothing is ever wrong for want of room.
//!
//! Names compare as the card's own code compares them
//! ([`rp_fat::node::upcase`], unit by unit); a folder that holds one
//! name twice (a damaged card) answers with the first, as the walk does.

use core::cmp::Ordering;

use rp_fat::node::{Node, Stamp, upcase};

/// What a look-up is answered with: enough for `stat` and for walking on
/// into a folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Brief {
    /// Where its entry set starts ([`Node::loc`]).
    pub loc: u64,
    /// A folder.
    pub folder: bool,
    /// Its size in bytes.
    pub size: u64,
    /// When it was last changed, if the card says.
    pub modified: Option<Stamp>,
}

impl Brief {
    /// What `node` says of itself.
    #[must_use]
    pub fn of(node: &Node) -> Self {
        Self {
            loc: node.loc,
            folder: node.folder,
            size: node.size,
            modified: node.modified,
        }
    }
}

/// A name's record: where, how big, when, what, and its name's place.
const RECORD: usize = 32;
/// Folders remembered at once.
const FOLDERS: usize = 128;

const FOLDER: u8 = 1;
const STAMPED: u8 = 2;
/// The name is kept two bytes a unit (one has a unit above a byte).
const WIDE: u8 = 4;
const NO_CENTIS: u8 = 0xFF;
const NO_ZONE: i16 = i16::MIN;

#[derive(Clone, Copy)]
struct Folder {
    dir: u64,
    /// Its first record and how many.
    first: usize,
    count: usize,
}

/// How much is remembered, for a run's log.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Used {
    /// Folders listed and kept.
    pub folders: usize,
    /// Their names.
    pub names: usize,
    /// Bytes of the room in use.
    pub bytes: usize,
    /// Folders that did not fit and are walked from the card.
    pub refused: u32,
}

/// The names of the folders looked into, in `room`: the names' bytes
/// from its start, the records from its end.
pub struct Names<'a> {
    room: &'a mut [u8],
    folders: [Folder; FOLDERS],
    folder_count: usize,
    /// Records in use, and bytes of names (some of folders forgotten
    /// since: given back when every folder is).
    records: usize,
    text: usize,
    refused: u32,
    /// The folder being listed (the last), if one is.
    open: bool,
}

fn le<const K: usize>(b: &[u8], at: usize) -> [u8; K] {
    let mut out = [0; K];
    out.copy_from_slice(&b[at..at + K]);
    out
}

impl<'a> Names<'a> {
    /// Nothing remembered yet, in `room`.
    pub fn new(room: &'a mut [u8]) -> Self {
        Self {
            room,
            folders: [Folder {
                dir: 0,
                first: 0,
                count: 0,
            }; FOLDERS],
            folder_count: 0,
            records: 0,
            text: 0,
            refused: 0,
            open: false,
        }
    }

    /// How much is remembered.
    #[must_use]
    pub fn used(&self) -> Used {
        Used {
            folders: self.folder_count,
            names: self.records,
            bytes: self.text + self.records * RECORD,
            refused: self.refused,
        }
    }

    /// Where record `i` lies in the room.
    fn at(&self, i: usize) -> usize {
        self.room.len() - RECORD * (i + 1)
    }

    fn folder(&self, dir: u64) -> Option<usize> {
        let live = self.folder_count - usize::from(self.open);
        self.folders[..live].iter().position(|f| f.dir == dir)
    }

    /// Whether `dir`'s names are remembered.
    #[must_use]
    pub fn listed(&self, dir: u64) -> bool {
        self.folder(dir).is_some()
    }

    /// Forgets everything.
    pub fn clear(&mut self) {
        self.folder_count = 0;
        self.records = 0;
        self.text = 0;
        self.open = false;
    }

    /// Forgets `dir`'s names (something in it is about to change).
    pub fn forget(&mut self, dir: u64) {
        self.abandon();
        let Some(k) = self.folder(dir) else {
            return;
        };
        let Folder { first, count, .. } = self.folders[k];
        // The records after it move down over it.
        if count > 0 && first + count < self.records {
            let (from, to) = (self.at(self.records - 1), self.at(first + count) + RECORD);
            self.room.copy_within(from..to, from + RECORD * count);
        }
        self.records -= count;
        self.folders.copy_within(k + 1..self.folder_count, k);
        self.folder_count -= 1;
        for f in &mut self.folders[k..self.folder_count] {
            f.first -= count;
        }
        if self.folder_count == 0 {
            self.text = 0;
        }
    }

    /// Starts remembering `dir` (not yet [`Names::listed`]): its names
    /// follow by [`Names::add`], then [`Names::keep`]. `false` when no
    /// more folders are kept.
    pub fn begin(&mut self, dir: u64) -> bool {
        self.abandon();
        if self.folder_count == FOLDERS {
            self.refused += 1;
            return false;
        }
        self.folders[self.folder_count] = Folder {
            dir,
            first: self.records,
            count: 0,
        };
        self.folder_count += 1;
        self.open = true;
        true
    }

    /// Drops the folder being listed, if one is (it did not fit, or the
    /// card failed part way).
    pub fn abandon(&mut self) {
        if self.open {
            self.open = false;
            self.folder_count -= 1;
            let f = self.folders[self.folder_count];
            // Its names were the last written.
            if let Some(first) = (0..f.count)
                .map(|i| u32::from_le_bytes(le(self.room, self.at(f.first + i) + 28)) as usize)
                .min()
            {
                self.text = first;
            }
            self.records = f.first;
        }
    }

    /// The folder being listed is complete: its names answer from now.
    pub fn keep(&mut self) {
        self.open = false;
    }

    /// The units of record `i`'s name.
    fn name(&self, i: usize) -> impl Iterator<Item = u16> + '_ {
        let r = self.at(i);
        let wide = self.room[r + 26] & WIDE != 0;
        let len = usize::from(self.room[r + 27]);
        let at = u32::from_le_bytes(le(self.room, r + 28)) as usize;
        (0..len).map(move |k| {
            if wide {
                u16::from_le_bytes(le(self.room, at + 2 * k))
            } else {
                u16::from(self.room[at + k])
            }
        })
    }

    /// Record `i`'s name against `want`, any case.
    fn order(&self, i: usize, want: &[u16]) -> Ordering {
        self.name(i)
            .map(upcase)
            .cmp(want.iter().map(|&u| upcase(u)))
    }

    /// Where `want` is among the folder's records, or would go.
    fn search(&self, f: &Folder, want: &[u16]) -> Result<usize, usize> {
        let (mut lo, mut hi) = (0, f.count);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            match self.order(f.first + mid, want) {
                Ordering::Less => lo = mid + 1,
                Ordering::Greater => hi = mid,
                Ordering::Equal => return Ok(f.first + mid),
            }
        }
        Err(f.first + lo)
    }

    /// Adds `node` to the folder being listed. `false` when the room is
    /// full (the folder is dropped: [`Names::abandon`] is done).
    pub fn add(&mut self, node: &Node) -> bool {
        if !self.open {
            return false;
        }
        let f = self.folders[self.folder_count - 1];
        let name = node.name();
        let at = match self.search(&f, name) {
            // A second entry of one name: the first answers.
            Ok(_) => return true,
            Err(at) => at,
        };
        let wide = name.iter().any(|&u| u > 0xFF);
        let bytes = name.len() * if wide { 2 } else { 1 };
        let free = self
            .room
            .len()
            .checked_sub(RECORD * (self.records + 1))
            .and_then(|end| end.checked_sub(self.text));
        if name.len() > 255 || free.is_none_or(|f| f < bytes) || self.text > u32::MAX as usize {
            self.refused += 1;
            self.abandon();
            return false;
        }
        for (k, &u) in name.iter().enumerate() {
            if wide {
                self.room[self.text + 2 * k..self.text + 2 * k + 2]
                    .copy_from_slice(&u.to_le_bytes());
            } else {
                self.room[self.text + k] = u as u8;
            }
        }
        // The records from `at` on make room: one further along.
        if at < self.records {
            let (from, to) = (self.at(self.records - 1), self.at(at) + RECORD);
            self.room.copy_within(from..to, from - RECORD);
        }
        let r = self.at(at);
        let rec = &mut self.room[r..r + RECORD];
        rec[0..8].copy_from_slice(&node.loc.to_le_bytes());
        rec[8..16].copy_from_slice(&node.size.to_le_bytes());
        let s = node.modified.unwrap_or_default();
        rec[16..18].copy_from_slice(&s.year.to_le_bytes());
        rec[18..23].copy_from_slice(&[s.month, s.day, s.hour, s.minute, s.second]);
        rec[23] = s.centis.unwrap_or(NO_CENTIS);
        rec[24..26].copy_from_slice(&s.zone.unwrap_or(NO_ZONE).to_le_bytes());
        rec[26] = if node.folder { FOLDER } else { 0 }
            | if node.modified.is_some() { STAMPED } else { 0 }
            | if wide { WIDE } else { 0 };
        rec[27] = name.len() as u8;
        rec[28..32].copy_from_slice(&(self.text as u32).to_le_bytes());
        self.text += bytes;
        self.records += 1;
        self.folders[self.folder_count - 1].count += 1;
        true
    }

    /// What `name` is in `dir`: `None` when `dir` is not remembered (ask
    /// the card), `Some(None)` when it is and holds no such name.
    #[must_use]
    pub fn get(&self, dir: u64, name: &[u16]) -> Option<Option<Brief>> {
        let f = self.folders[self.folder(dir)?];
        let Ok(i) = self.search(&f, name) else {
            return Some(None);
        };
        let r = self.at(i);
        let rec = &self.room[r..r + RECORD];
        let flags = rec[26];
        let zone = i16::from_le_bytes(le(rec, 24));
        Some(Some(Brief {
            loc: u64::from_le_bytes(le(rec, 0)),
            folder: flags & FOLDER != 0,
            size: u64::from_le_bytes(le(rec, 8)),
            modified: (flags & STAMPED != 0).then(|| Stamp {
                year: u16::from_le_bytes(le(rec, 16)),
                month: rec[18],
                day: rec[19],
                hour: rec[20],
                minute: rec[21],
                second: rec[22],
                centis: (rec[23] != NO_CENTIS).then_some(rec[23]),
                zone: (zone != NO_ZONE).then_some(zone),
            }),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(name: &str, loc: u64, folder: bool, size: u64) -> Node {
        let mut n = Node {
            loc,
            folder,
            size,
            ..Node::default()
        };
        for (o, u) in n.name.iter_mut().zip(name.encode_utf16()) {
            *o = u;
            n.name_len += 1;
        }
        n
    }

    fn units(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn list(names: &mut Names<'_>, dir: u64, of: &[Node]) -> bool {
        if !names.begin(dir) {
            return false;
        }
        for n in of {
            if !names.add(n) {
                return false;
            }
        }
        names.keep();
        true
    }

    #[test]
    fn a_listed_folder_answers_there_and_not_there_in_any_case() {
        let mut room = vec![0u8; 4096];
        let mut names = Names::new(&mut room);
        assert_eq!(
            names.get(7, &units("gfx")),
            None,
            "not listed: ask the card"
        );
        let stamp = Stamp {
            year: 2004,
            month: 11,
            day: 16,
            hour: 9,
            minute: 30,
            second: 58,
            centis: Some(7),
            zone: Some(-300),
        };
        let mut wad = node("halflife.wad", 0x51, false, 6_000_000_000);
        wad.modified = Some(stamp);
        // Out of order, as a card holds them.
        let of = [
            node("sound", 0x40, true, 0),
            wad,
            node("gfx", 0x20, true, 0),
            node("Gfx.wad", 0x30, false, 12),
            node("a", 0x10, false, 1),
        ];
        assert!(list(&mut names, 7, &of));
        assert!(names.listed(7) && !names.listed(8));
        for n in &of {
            for name in [
                String::from_utf16_lossy(n.name()),
                String::from_utf16_lossy(n.name()).to_uppercase(),
            ] {
                assert_eq!(
                    names.get(7, &units(&name)),
                    Some(Some(Brief::of(n))),
                    "{name}"
                );
            }
        }
        assert_eq!(
            names
                .get(7, &units("HALFLIFE.WAD"))
                .unwrap()
                .unwrap()
                .modified,
            Some(stamp)
        );
        for gone in ["gf", "gfxx", "", "b", "zzz", "gfx.wa"] {
            assert_eq!(names.get(7, &units(gone)), Some(None), "{gone}");
        }
        assert_eq!(names.used().names, 5);
    }

    #[test]
    fn names_above_a_byte_compare_as_the_card_compares_them() {
        let mut room = vec![0u8; 4096];
        let mut names = Names::new(&mut room);
        // A name beyond Latin-1, one inside it, and one whose capital
        // lies beyond a byte (U+00FF's is U+0178).
        let of = [
            node("привет.txt", 1, false, 1),
            node("café", 2, false, 2),
            node("\u{178}", 3, false, 3),
        ];
        assert!(list(&mut names, 0, &of));
        assert_eq!(names.get(0, &units("ПРИВЕТ.TXT")).unwrap().unwrap().loc, 1);
        assert_eq!(names.get(0, &units("CAFÉ")).unwrap().unwrap().loc, 2);
        assert_eq!(names.get(0, &units("\u{ff}")).unwrap().unwrap().loc, 3);
        assert_eq!(names.get(0, &units("cafe")), Some(None));
        for n in &of {
            assert!(rp_fat::node::same_name(n.name(), n.name()));
        }
    }

    #[test]
    fn a_name_held_twice_answers_with_the_first() {
        let mut room = vec![0u8; 1024];
        let mut names = Names::new(&mut room);
        let of = [node("save", 1, false, 1), node("SAVE", 2, false, 2)];
        assert!(list(&mut names, 0, &of));
        assert_eq!(names.get(0, &units("Save")).unwrap().unwrap().loc, 1);
        assert_eq!(names.used().names, 1);
    }

    #[test]
    fn a_folder_that_does_not_fit_is_dropped_whole_and_the_rest_stays() {
        let mut room = vec![0u8; 3 * (RECORD + 4) + 10];
        let mut names = Names::new(&mut room);
        assert!(list(
            &mut names,
            1,
            &[node("aaaa", 1, false, 1), node("bbbb", 2, false, 2)]
        ));
        let before = names.used();
        let big = [node("cccc", 3, false, 3), node("dddd", 4, false, 4)];
        assert!(!list(&mut names, 2, &big));
        assert!(!names.listed(2));
        assert_eq!(names.get(2, &units("cccc")), None);
        assert_eq!(
            names.used(),
            Used {
                refused: 1,
                ..before
            }
        );
        assert_eq!(names.get(1, &units("BBBB")).unwrap().unwrap().loc, 2);
        // One that fits still goes in after.
        assert!(list(&mut names, 3, &[node("eeee", 5, false, 5)]));
        assert_eq!(names.get(3, &units("eeee")).unwrap().unwrap().loc, 5);
        // A folder half listed answers nothing, and starting another
        // drops it.
        assert!(names.begin(4));
        assert!(!names.listed(4));
        names.abandon();
        assert_eq!(names.used().names, 3);
    }

    #[test]
    fn a_forgotten_folder_is_gone_and_the_others_answer_as_before() {
        let mut room = vec![0u8; 8192];
        let mut names = Names::new(&mut room);
        let folders: Vec<(u64, Vec<Node>)> = (0..5u64)
            .map(|d| {
                (
                    d * 100,
                    (0..7 + d)
                        .map(|i| {
                            node(
                                &format!("f{}-{d}.dat", (i * 5) % 11),
                                d * 100 + i,
                                i % 3 == 0,
                                i,
                            )
                        })
                        .collect(),
                )
            })
            .collect();
        for (d, of) in &folders {
            assert!(list(&mut names, *d, of));
        }
        let check = |names: &Names<'_>, gone: &[u64]| {
            for (d, of) in &folders {
                for n in of {
                    let want = (!gone.contains(d)).then(|| Some(Brief::of(n)));
                    assert_eq!(names.get(*d, n.name()), want, "folder {d}");
                }
            }
        };
        check(&names, &[]);
        names.forget(200);
        check(&names, &[200]);
        names.forget(0);
        check(&names, &[200, 0]);
        names.forget(400);
        check(&names, &[200, 0, 400]);
        // Listed again, it answers again.
        assert!(list(&mut names, 200, &folders[2].1));
        check(&names, &[0, 400]);
        for d in [100, 200, 300] {
            names.forget(d);
        }
        assert_eq!(
            names.used(),
            Used::default(),
            "every folder forgotten: the names' bytes are free again"
        );
    }

    #[test]
    fn no_more_folders_than_the_table_holds() {
        let mut room = vec![0u8; 64 * 1024];
        let mut names = Names::new(&mut room);
        for d in 0..FOLDERS as u64 {
            assert!(list(&mut names, d, &[node("x", d, false, d)]));
        }
        assert!(!names.begin(1000));
        assert_eq!(names.used().refused, 1);
        assert_eq!(names.get(95, &units("X")).unwrap().unwrap().size, 95);
        names.clear();
        assert_eq!(names.used().names, 0);
        assert!(names.begin(1000));
    }
}
