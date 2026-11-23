//! The files on the card an app reads and writes (contacts-rule0.md "vCard
//! 3 and 4 import and export" and "Photos from a picture file on the
//! card"; calendar-rule0.md ".ics import and export"; notes-rule0.md
//! "Export as text or Markdown, ink as SVG or PNG; import text files"): an
//! app's `CARDFILES`, `CARDFILE` and `CARDPUT` questions
//! ([`rp_app::Program::card_ask`]) answered by the device itself, no dock.
//!
//! Iterate, don't imitate (rule 0): Windows Mobile's Contacts and Calendar
//! read no file at all (ActiveSync only), and its File Explorer opened a
//! .vcf in Pocket Outlook one at a time. Ours lists the files of the kind
//! asked for from the card's root and its `RetroPane` folder (where a PC
//! or a camera puts them, and where an export goes), and hands the app
//! their names only by number: an app never names a path, so no app can
//! read or write outside the two folders (security.md). A write streams
//! from the app a piece at a time into a staged run of the card, linked
//! whole at the end (rp-fat's copy-then-swap: the old file stays until the
//! new one is complete, so a pulled card never leaves half a file, and no
//! buffer a file's size is needed); a text file or a picture is read a
//! piece at a time. Generic over the card so the host tests run them on
//! FAT and exFAT card images.

use rp_fat::{AnyVolume, Error};
use rp_hal_api::BlockDevice;

/// The folder an export goes to (made when missing).
pub const DIR: &str = "RetroPane";
/// The files a list names at most.
pub const LIST_MAX: usize = 40;
/// The longest name listed or written.
const NAME_MAX: usize = 64;
/// The largest file an app writes (Notes' longest export is the
/// organiser's whole 256 KB store as Markdown).
pub const WRITE_MAX: usize = 4 * 1024 * 1024;
/// rp-fat's scratch for a write (its free-run search reads the FAT a run
/// at a time).
pub const SCRATCH: usize = 4096;

/// The extensions of a kind of file (`vcf`, `ics`, `pic`, `txt`).
fn extensions(kind: &[u8]) -> &'static [&'static [u8]] {
    match kind {
        b"vcf" => &[b".vcf"],
        b"ics" => &[b".ics"],
        b"pic" => &[b".jpg", b".jpeg"],
        b"txt" => &[b".txt", b".md"],
        _ => &[],
    }
}

/// The extensions an app may write: its exports (a picture only as the
/// PNG a note's ink makes, never over a camera's JPEG).
const WRITABLE: [&[u8]; 6] = [b".vcf", b".ics", b".txt", b".md", b".svg", b".png"];

/// Whether a kind is read a piece at a time (its reader streams), not
/// whole.
fn in_pieces(kind: &[u8]) -> bool {
    matches!(kind, b"pic" | b"txt")
}

fn is_kind(name: &[u8], kind: &[u8]) -> bool {
    extensions(kind)
        .iter()
        .any(|e| name.len() > e.len() && name[name.len() - e.len()..].eq_ignore_ascii_case(e))
}

/// Each file of `kind` (its name and size), the root's first and then the
/// folder's, at most [`LIST_MAX`].
fn each<D: BlockDevice>(
    vol: &mut AnyVolume<D>,
    kind: &[u8],
    mut f: impl FnMut(usize, bool, &[u8], u64),
) {
    let mut n = 0;
    for in_dir in [false, true] {
        let mut take = |name: &[u8], size: u64, dir: bool| {
            if !dir && n < LIST_MAX && name.len() <= NAME_MAX && is_kind(name, kind) {
                f(n, in_dir, name, size);
                n += 1;
            }
        };
        // A missing folder lists nothing.
        let _ = match (&mut *vol, in_dir) {
            (AnyVolume::Fat(v), false) => {
                v.for_each_root(|e| take(e.name, u64::from(e.size), e.is_dir))
            }
            (AnyVolume::Fat(v), true) => {
                v.for_each_in(DIR, |e| take(e.name, u64::from(e.size), e.is_dir))
            }
            (AnyVolume::ExFat(v), false) => v.for_each_root(|n, e| take(n, e.size, e.is_dir)),
            (AnyVolume::ExFat(v), true) => v.for_each_in(DIR, |n, e| take(n, e.size, e.is_dir)),
        };
    }
}

/// The files of `kind` as "n|name" lines into `out`: the length.
///
/// # Errors
/// The card not readable.
pub fn list<D: BlockDevice>(
    card: D,
    start: u32,
    kind: &[u8],
    out: &mut [u8],
) -> Result<usize, &'static str> {
    let mut vol = AnyVolume::mount(card, start).map_err(|_| "the card not readable")?;
    let mut at = 0;
    each(&mut vol, kind, |n, _, name, _| {
        let mut line = [0u8; NAME_MAX + 4];
        let mut k = 0;
        if n >= 10 {
            line[k] = b'0' + (n / 10) as u8;
            k += 1;
        }
        line[k] = b'0' + (n % 10) as u8;
        line[k + 1] = b'|';
        k += 2;
        // Names as text: a byte that isn't printable ASCII shown as '?'.
        for &c in name {
            line[k] = if (b' '..=b'~').contains(&c) && c != b'|' {
                c
            } else {
                b'?'
            };
            k += 1;
        }
        line[k] = b'\n';
        k += 1;
        if let Some(o) = out.get_mut(at..at + k) {
            o.copy_from_slice(&line[..k]);
            at += k;
        }
    });
    Ok(at)
}

/// File `n` of `kind`'s list into `out`: its length. A vCard or a
/// calendar is read whole (a part of one would lose people); a picture or
/// a text file a piece at a time from `offset`, after four bytes of the
/// file's size (little-endian), so a camera's 4 MB file is read in the
/// pieces its reader asks for.
///
/// # Errors
/// The card not readable, no such file, or a whole file bigger than `out`.
pub fn read<D: BlockDevice>(
    card: D,
    start: u32,
    kind: &[u8],
    n: usize,
    offset: u64,
    out: &mut [u8],
) -> Result<usize, &'static str> {
    let mut vol = AnyVolume::mount(card, start).map_err(|_| "the card not readable")?;
    let mut path = [0u8; DIR.len() + 1 + NAME_MAX];
    let mut found = None;
    each(&mut vol, kind, |k, in_dir, name, size| {
        if k == n {
            let mut p = 0;
            if in_dir {
                path[..DIR.len()].copy_from_slice(DIR.as_bytes());
                path[DIR.len()] = b'/';
                p = DIR.len() + 1;
            }
            path[p..p + name.len()].copy_from_slice(name);
            found = Some((p + name.len(), size));
        }
    });
    let (len, size) = found.ok_or("no such file")?;
    let piece = in_pieces(kind);
    let (head, offset) = if piece {
        let s = u32::try_from(size).unwrap_or(u32::MAX).to_le_bytes();
        out.get_mut(..4).ok_or("no room")?.copy_from_slice(&s);
        (4, offset.min(size))
    } else if size > out.len() as u64 || offset != 0 {
        return Err("too big to read");
    } else {
        (0, 0)
    };
    let want = (size - offset).min((out.len() - head) as u64);
    let path = core::str::from_utf8(&path[..len]).map_err(|_| "a name that isn't text")?;
    let mut scratch = [0u8; 1024];
    let mut at = head;
    if want > 0 {
        vol.read_range(path, offset, want, &mut scratch, &mut |c| {
            if let Some(o) = out.get_mut(at..at + c.len()) {
                o.copy_from_slice(c);
            }
            at += c.len();
        })
        .map_err(|_| "not readable")?;
    }
    Ok(at.min(out.len()))
}

/// Whether `name` is one an app may write: letters, digits, spaces and
/// `_ . -`, one of [`WRITABLE`]'s kinds, no folder, no `..`.
#[must_use]
pub fn name_ok(name: &[u8]) -> bool {
    (4..=40).contains(&name.len())
        && name
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || b" _.-".contains(c))
        && !name.starts_with(b".")
        && !name.ends_with(b" ")
        && !name.windows(2).any(|w| w == b"..")
        && WRITABLE
            .iter()
            .any(|e| name.len() > e.len() && name[name.len() - e.len()..].eq_ignore_ascii_case(e))
}

/// `data` kept as the card's `RetroPane/name` (in place of one of that
/// name); `scratch` holds [`SCRATCH`] bytes and a piece of 512 or more.
///
/// # Errors
/// As [`put_from`].
pub fn put<D: BlockDevice>(
    card: D,
    start: u32,
    name: &[u8],
    data: &[u8],
    scratch: &mut [u8],
) -> Result<(), &'static str> {
    if scratch.len() < SCRATCH + 512 {
        return Err("no room");
    }
    let (scratch, piece) = scratch.split_at_mut(SCRATCH);
    put_from(
        card,
        start,
        name,
        data.len(),
        scratch,
        piece,
        &mut |at, out| {
            let n = out.len().min(data.len().saturating_sub(at));
            out[..n].copy_from_slice(&data[at..at + n]);
            n
        },
    )
}

/// A file of `len` bytes kept as the card's `RetroPane/name` (in place of
/// one of that name), its bytes taken from `fill` (from `at` into `out`:
/// how many) a `piece` at a time and staged on the card, linked only once
/// whole: an app that runs short or a pulled card leaves the old file as
/// it was. `scratch` holds [`SCRATCH`] bytes, `piece` 512 or more.
///
/// # Errors
/// A name [`name_ok`] refuses, over [`WRITE_MAX`], the card not readable
/// or full, or the app's bytes ending short.
pub fn put_from<D: BlockDevice>(
    card: D,
    start: u32,
    name: &[u8],
    len: usize,
    scratch: &mut [u8],
    piece: &mut [u8],
    fill: &mut dyn FnMut(usize, &mut [u8]) -> usize,
) -> Result<(), &'static str> {
    if !name_ok(name) {
        return Err("not a name an export may have");
    }
    if len > WRITE_MAX {
        return Err("too big to write");
    }
    // Whole sectors a piece (the last one's tail zero).
    let whole_len = piece.len() / 512 * 512;
    if whole_len == 0 {
        return Err("no room");
    }
    let piece = &mut piece[..whole_len];
    let mut vol = AnyVolume::mount(card, start).map_err(|_| "the card not readable")?;
    match &mut vol {
        AnyVolume::Fat(v) => match v.mkdir(DIR, scratch) {
            Ok(()) | Err(Error::Exists) => {}
            Err(_) => return Err("the folder not made"),
        },
        AnyVolume::ExFat(v) => match v.mkdir(DIR, scratch) {
            Ok(()) | Err(Error::Exists) => {}
            Err(_) => return Err("the folder not made"),
        },
    }
    let mut path = [0u8; DIR.len() + 1 + 40];
    path[..DIR.len()].copy_from_slice(DIR.as_bytes());
    path[DIR.len()] = b'/';
    path[DIR.len() + 1..DIR.len() + 1 + name.len()].copy_from_slice(name);
    let path = core::str::from_utf8(&path[..DIR.len() + 1 + name.len()]).map_err(|_| "no name")?;
    let says = |e| match e {
        Error::NoRoom | Error::Fragmented => "the card is full",
        _ => "not written",
    };
    if len == 0 {
        // No run to stage: an empty file made whole.
        return vol.put(path, &[], scratch).map(|_| ()).map_err(says);
    }
    let staged = vol.stage(path, len, scratch).map_err(says)?;
    let mut at = 0;
    while at < len {
        let want = (len - at).min(piece.len());
        let mut got = 0;
        while got < want {
            let k = fill(at + got, &mut piece[got..want]);
            if k == 0 {
                break;
            }
            got += k;
        }
        if got != want {
            // Never linked: the old file (if any) stays.
            return Err("the app's file was short");
        }
        let whole = want.div_ceil(512) * 512;
        piece[want..whole].fill(0);
        let sector = u32::try_from(at / 512).map_err(|_| "too big to write")?;
        vol.stage_write(&staged, sector, &piece[..whole])
            .map_err(says)?;
        at += want;
    }
    vol.commit(path, staged, scratch).map(|_| ()).map_err(says)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rp_hal_api::BlockError;

    struct Card(Vec<u8>);

    impl BlockDevice for Card {
        fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), BlockError> {
            let o = lba as usize * 512;
            let src = self.0.get(o..o + buf.len()).ok_or(BlockError::OutOfRange)?;
            buf.copy_from_slice(src);
            Ok(())
        }
        fn write_blocks(&mut self, lba: u32, buf: &[u8]) -> Result<(), BlockError> {
            let o = lba as usize * 512;
            let dst = self
                .0
                .get_mut(o..o + buf.len())
                .ok_or(BlockError::OutOfRange)?;
            dst.copy_from_slice(buf);
            Ok(())
        }
    }

    fn fat_card() -> (Card, u32) {
        let files = [
            rp_fat::format::Entry {
                name: "RPIC0A.PAK",
                data: b"the icon pack",
            },
            rp_fat::format::Entry {
                name: "PEOPLE.VCF",
                data: b"BEGIN:VCARD\r\nFN:Ada\r\nEND:VCARD\r\n",
            },
            rp_fat::format::Entry {
                name: "WORK.ICS",
                data: b"BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n",
            },
            rp_fat::format::Entry {
                name: "ADA.JPG",
                data: &[0xFF, 0xD8, 0xFF, 0xD9],
            },
        ];
        let img = rp_fat::format::build(&rp_fat::format::Image {
            total_sectors: 16 * 1024 * 1024 / 512,
            sectors_per_cluster: 8,
            files: &files,
            blocked_clusters: &[],
            ballast: 0,
        });
        let start = u32::from_le_bytes([img[454], img[455], img[456], img[457]]);
        (Card(img), start)
    }

    fn exfat_card() -> (Card, u32) {
        let sectors = 64 * 1024 * 1024 / 512;
        let o = rp_fat::mkfs::Options {
            fs: Some(rp_fat::mkfs::Fs::ExFat),
            cluster_bytes: Some(4096),
            align_sectors: None,
            label: "FILES",
            serial: 0x1234_5678,
        };
        let l = rp_fat::mkfs::plan(sectors, &o).expect("an exFAT layout");
        let mut c = Card(vec![0; sectors as usize * 512]);
        rp_fat::mkfs::write(&mut c, &l, &mut vec![0u8; 16 * 1024]).expect("formatted");
        (c, l.part_start)
    }

    fn text(b: &[u8]) -> String {
        String::from_utf8_lossy(b).into_owned()
    }

    #[test]
    fn files_of_a_kind_are_listed_read_and_written_on_fat_and_exfat() {
        for (mut card, start) in [fat_card(), exfat_card()] {
            let mut out = [0u8; 512];
            // An export first: the folder is made, the file listed after
            // the root's.
            let mut scratch = [0u8; SCRATCH + 4096];
            put(
                &mut card,
                start,
                b"contacts.vcf",
                b"BEGIN:VCARD\r\nEND:VCARD\r\n",
                &mut scratch,
            )
            .unwrap();
            put(
                &mut card,
                start,
                b"contacts.vcf",
                b"BEGIN:VCARD\r\nFN:Bo\r\nEND:VCARD\r\n",
                &mut scratch,
            )
            .unwrap();
            let n = list(&mut card, start, b"vcf", &mut out).unwrap();
            let l = text(&out[..n]);
            let names: Vec<&str> = l.lines().collect();
            assert!(names.last().unwrap().ends_with("|contacts.vcf"), "{l}");
            let k = names.len() - 1;
            let n = read(&mut card, start, b"vcf", k, 0, &mut out).unwrap();
            // Written in place of the first: the second's bytes.
            assert_eq!(&out[..n], b"BEGIN:VCARD\r\nFN:Bo\r\nEND:VCARD\r\n");
            assert_eq!(
                read(&mut card, start, b"vcf", 99, 0, &mut out),
                Err("no such file")
            );
            assert_eq!(
                read(&mut card, start, b"vcf", k, 0, &mut [0u8; 4]),
                Err("too big to read")
            );
            // Other kinds aren't listed as these.
            let n = list(&mut card, start, b"exe", &mut out).unwrap();
            assert_eq!(n, 0);
        }
        // The FAT card's own files, by kind.
        let (mut card, start) = fat_card();
        let mut out = [0u8; 256];
        let n = list(&mut card, start, b"ics", &mut out).unwrap();
        assert_eq!(text(&out[..n]), "0|WORK.ICS\n");
        let n = list(&mut card, start, b"pic", &mut out).unwrap();
        assert_eq!(text(&out[..n]), "0|ADA.JPG\n");
        // A picture: its size first, then from the offset asked.
        let n = read(&mut card, start, b"pic", 0, 0, &mut out).unwrap();
        assert_eq!(&out[..n], &[4, 0, 0, 0, 0xFF, 0xD8, 0xFF, 0xD9]);
        let n = read(&mut card, start, b"pic", 0, 2, &mut out).unwrap();
        assert_eq!(&out[..n], &[4, 0, 0, 0, 0xFF, 0xD9]);
        let n = read(&mut card, start, b"pic", 0, 9, &mut out).unwrap();
        assert_eq!(&out[..n], &[4, 0, 0, 0]);
        let n = read(&mut card, start, b"pic", 0, 0, &mut out[..6]).unwrap();
        assert_eq!(&out[..n], &[4, 0, 0, 0, 0xFF, 0xD8]);
    }

    #[test]
    fn an_export_never_names_a_path_or_another_kind() {
        for bad in [
            &b"../evil.vcf"[..],
            b"a/b.vcf",
            b"a\\b.ics",
            b"RPIC0A.PAK",
            b".vcf",
            b"x.exe",
            b"name:.ics",
            b"",
            b"a_very_long_name_that_goes_on_past_forty_bytes.vcf",
            b"a..b.md",
            b"photo.jpg",
            b"plan.txt.exe",
            b".md",
        ] {
            assert!(!name_ok(bad), "{}", text(bad));
        }
        assert!(name_ok(b"Contacts 2026-11-23.vcf"));
        assert!(name_ok(b"calendar.ics"));
        for good in [
            &b"Note.md"[..],
            b"a.MD",
            b"Plan 2.txt",
            b"Note-1.svg",
            b"Note-2.png",
        ] {
            assert!(name_ok(good), "{}", text(good));
        }
        let (mut card, start) = fat_card();
        let mut scratch = [0u8; SCRATCH + 4096];
        assert_eq!(
            put(&mut card, start, b"RPIC0A.PAK", b"x", &mut scratch),
            Err("not a name an export may have")
        );
    }

    #[test]
    fn a_long_file_streams_in_and_out_in_pieces_on_fat_and_exfat() {
        for (mut card, start) in [fat_card(), exfat_card()] {
            // 300 KB, past the image's 256 KB pool: given in odd pieces.
            let data: Vec<u8> = (0..300_000u32).map(|i| b'a' + (i % 26) as u8).collect();
            let mut scratch = [0u8; SCRATCH];
            let mut piece = [0u8; 4096 + 100];
            put_from(
                &mut card,
                start,
                b"Long note.md",
                data.len(),
                &mut scratch,
                &mut piece,
                &mut |at, out| {
                    let n = out.len().min(777).min(data.len() - at);
                    out[..n].copy_from_slice(&data[at..at + n]);
                    n
                },
            )
            .unwrap();
            // Listed as text, read back a piece at a time after its size.
            let mut out = vec![0u8; 512];
            let n = list(&mut card, start, b"txt", &mut out).unwrap();
            let l = text(&out[..n]);
            let k: usize = l
                .lines()
                .last()
                .unwrap()
                .split('|')
                .next()
                .unwrap()
                .parse()
                .unwrap();
            assert!(
                l.ends_with(
                    "|Long note.md
"
                ),
                "{l}"
            );
            let mut back = Vec::new();
            let mut buf = vec![0u8; 65_536];
            loop {
                let n = read(&mut card, start, b"txt", k, back.len() as u64, &mut buf).unwrap();
                assert_eq!(u32::from_le_bytes(buf[..4].try_into().unwrap()), 300_000);
                if n == 4 {
                    break;
                }
                back.extend_from_slice(&buf[4..n]);
            }
            assert_eq!(back, data);
            // An app that runs short: refused, the file as it was.
            assert_eq!(
                put_from(
                    &mut card,
                    start,
                    b"Long note.md",
                    10_000,
                    &mut scratch,
                    &mut piece,
                    &mut |at, out| {
                        let n = out.len().min(5_000usize.saturating_sub(at));
                        out[..n].fill(b'x');
                        n
                    },
                ),
                Err("the app's file was short")
            );
            let n = read(&mut card, start, b"txt", k, 0, &mut buf).unwrap();
            assert_eq!(&buf[4..n], &data[..n - 4]);
            // An empty file, and one past the cap.
            put(
                &mut card,
                start,
                b"empty.txt",
                b"",
                &mut [0u8; SCRATCH + 512],
            )
            .unwrap();
            assert_eq!(
                put_from(
                    &mut card,
                    start,
                    b"huge.md",
                    WRITE_MAX + 1,
                    &mut scratch,
                    &mut piece,
                    &mut |_, _| 0,
                ),
                Err("too big to write")
            );
        }
    }
}
