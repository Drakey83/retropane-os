//! An app's file steps on the card ([`rp_app::FileAsk`]): Notes' export
//! and import (notes-rule0.md), and any app after it.
//!
//! Iterate, don't imitate (rule 0): Windows Mobile handed every app the
//! whole file system through `CreateFile`; a phone's apps get a sandbox
//! folder and a system picker. Ours gives an app one folder of its own
//! under the card's `RetroPane` folder (where the PC sees it over MTP),
//! names it may use checked here again whatever the app sent (no path, no
//! device name, no hidden or dot file), and moves a file a piece at a time
//! through a buffer of [`BUF`] bytes: a write is staged in one free run and
//! linked whole at the end (rp-fat's copy-then-swap: the old file stays
//! until the new one is complete, so a pulled card never leaves half a
//! file), a read streams to the app in [`PIECE`]-byte pieces. One step a
//! still pass of the image's loop, nothing logged while the card is
//! written (C59).

use rp_app::{FileAnswer, FileAsk};
use rp_fat::{AnyVolume, Error};
use rp_hal_api::BlockDevice;

/// The buffer a step needs: rp-fat's scratch and one piece.
pub const BUF: usize = SCRATCH + PIECE;
/// rp-fat's scratch (its free-run search reads the FAT a run at a time).
const SCRATCH: usize = 4096;
/// The bytes moved at once: whole sectors.
pub const PIECE: usize = 4096;
/// The largest file an app may read or write (the organiser's store is
/// 256 KB; anything bigger is no note).
pub const MAX: u32 = 1024 * 1024;
/// Most names a listing gives.
const NAMES: usize = 64;

/// The app's side of a step: the open program, through the shell.
pub trait Host {
    /// The file being written, from `off` into `out`: how many bytes.
    fn piece(&mut self, off: u32, out: &mut [u8]) -> usize;
    /// The step's answer.
    fn answer(&mut self, a: FileAnswer<'_>);
}

impl Host for rp_shell::Shell {
    fn piece(&mut self, off: u32, out: &mut [u8]) -> usize {
        self.file_piece(off, out)
    }

    fn answer(&mut self, a: FileAnswer<'_>) {
        self.file_answer(a);
    }
}

/// Whether `n` is a name an app may use: 1-64 letters, digits, spaces,
/// `-`, `_` and dots, not starting with a dot or a space, not ending with
/// either, no two dots running (so no `..`, no path, no `CON:`).
#[must_use]
pub fn name_ok(n: &[u8]) -> bool {
    !n.is_empty()
        && n.len() <= 64
        && n.iter()
            .all(|&c| c.is_ascii_alphanumeric() || matches!(c, b' ' | b'-' | b'_' | b'.'))
        && !matches!(n[0], b'.' | b' ')
        && !matches!(n[n.len() - 1], b'.' | b' ')
        && !n.windows(2).any(|w| w == b"..")
}

/// `RetroPane/<dir>[/<name>]` into `out`: its length, `None` for a name
/// that isn't [`name_ok`].
fn path(dir: &[u8], name: Option<&[u8]>, out: &mut [u8; 160]) -> Option<usize> {
    if !name_ok(dir) || name.is_some_and(|n| !name_ok(n)) {
        return None;
    }
    let mut k = 0;
    for part in [&b"RetroPane/"[..], dir] {
        out[k..k + part.len()].copy_from_slice(part);
        k += part.len();
    }
    if let Some(n) = name {
        out[k] = b'/';
        k += 1;
        out[k..k + n.len()].copy_from_slice(n);
        k += n.len();
    }
    Some(k)
}

/// What the user is told when a step fails.
fn says(e: Error) -> &'static str {
    match e {
        Error::NotFound => "NOT FOUND ON THE CARD",
        Error::Fragmented | Error::NoRoom => "THE CARD IS FULL",
        _ => "THE CARD REFUSED IT",
    }
}

/// Does `ask` on the card at partition `start`, answering `host` (a piece
/// at a time for a read; once for a list or a write). `buf` holds
/// [`BUF`] bytes.
pub fn serve<D: BlockDevice>(card: D, start: u32, ask: FileAsk, host: &mut dyn Host, buf: &mut [u8]) {
    let Ok(mut vol) = AnyVolume::mount(card, start) else {
        host.answer(FileAnswer::Failed("NO CARD"));
        return;
    };
    let (scratch, piece) = buf.split_at_mut(SCRATCH);
    let piece = &mut piece[..PIECE];
    let mut p = [0u8; 160];
    match ask {
        FileAsk::List { dir } => {
            let Some(n) = path(dir, None, &mut p) else {
                host.answer(FileAnswer::Failed("NOT A NAME"));
                return;
            };
            let Ok(dir) = core::str::from_utf8(&p[..n]) else { return };
            // The names, a line each (a folder not there yet: none).
            let (mut k, mut count) = (0, 0);
            let r = vol.list(dir, &mut |name, is_dir, _| {
                if is_dir || !name_ok(name) || count == NAMES || k + name.len() + 1 > piece.len() {
                    return;
                }
                if k > 0 {
                    piece[k] = b'\n';
                    k += 1;
                }
                piece[k..k + name.len()].copy_from_slice(name);
                k += name.len();
                count += 1;
            });
            match r {
                Ok(()) | Err(Error::NotFound) => host.answer(FileAnswer::Listed(&piece[..k])),
                Err(e) => host.answer(FileAnswer::Failed(says(e))),
            }
        }
        FileAsk::Read { dir, name } => {
            let Some(n) = path(dir, Some(name.get()), &mut p) else {
                host.answer(FileAnswer::Failed("NOT A NAME"));
                return;
            };
            let Ok(file) = core::str::from_utf8(&p[..n]) else { return };
            let mut off = 0u64;
            loop {
                let mut got = 0usize;
                let r = vol.read_range(file, off, PIECE as u64, scratch, &mut |c| {
                    let room = piece.len() - got;
                    let c = &c[..c.len().min(room)];
                    piece[got..got + c.len()].copy_from_slice(c);
                    got += c.len();
                });
                match r {
                    Err(e) => {
                        host.answer(FileAnswer::Failed(says(e)));
                        return;
                    }
                    Ok((size, _)) if size > u64::from(MAX) => {
                        // Said as its size: the app refuses it by name.
                        host.answer(FileAnswer::Piece {
                            off: 0,
                            total: u32::try_from(size).unwrap_or(u32::MAX),
                            bytes: &[],
                        });
                        return;
                    }
                    Ok((size, _)) => {
                        host.answer(FileAnswer::Piece {
                            off: off as u32,
                            total: size as u32,
                            bytes: &piece[..got],
                        });
                        off += got as u64;
                        if off >= size {
                            return;
                        }
                        if got == 0 {
                            // The file ended early (a card changed under
                            // it): said, never waited on.
                            host.answer(FileAnswer::Failed("THE FILE WAS CUT SHORT"));
                            return;
                        }
                    }
                }
            }
        }
        FileAsk::Write { dir, name, len } => {
            let Some(n) = path(dir, Some(name.get()), &mut p) else {
                host.answer(FileAnswer::Failed("NOT A NAME"));
                return;
            };
            let Ok(file) = core::str::from_utf8(&p[..n]) else { return };
            if len > MAX {
                host.answer(FileAnswer::Failed("TOO LARGE TO SAVE"));
                return;
            }
            if len == 0 {
                // No run to stage: an empty file made whole.
                match vol.put(file, &[], scratch) {
                    Ok(_) => host.answer(FileAnswer::Wrote),
                    Err(e) => host.answer(FileAnswer::Failed(says(e))),
                }
                return;
            }
            let staged = match vol.stage(file, len as usize, scratch) {
                Ok(s) => s,
                Err(e) => {
                    host.answer(FileAnswer::Failed(says(e)));
                    return;
                }
            };
            let mut off = 0u32;
            while off < len {
                let want = (len - off).min(PIECE as u32) as usize;
                let got = host.piece(off, &mut piece[..want]);
                if got != want {
                    host.answer(FileAnswer::Failed("THE NOTE CHANGED: NOT SAVED"));
                    return;
                }
                // Whole sectors (the last one's tail zero).
                let whole = want.div_ceil(512) * 512;
                piece[want..whole].iter_mut().for_each(|b| *b = 0);
                if let Err(e) = vol.stage_write(&staged, off / 512, &piece[..whole]) {
                    host.answer(FileAnswer::Failed(says(e)));
                    return;
                }
                off += want as u32;
            }
            match vol.commit(file, staged, scratch) {
                Ok(_) => host.answer(FileAnswer::Wrote),
                Err(e) => host.answer(FileAnswer::Failed(says(e))),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use rp_hal_api::BlockError;
    use std::vec::Vec;

    struct Card(Vec<u8>);

    impl BlockDevice for Card {
        fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), BlockError> {
            let o = lba as usize * 512;
            buf.copy_from_slice(self.0.get(o..o + buf.len()).ok_or(BlockError::OutOfRange)?);
            Ok(())
        }
        fn write_blocks(&mut self, lba: u32, buf: &[u8]) -> Result<(), BlockError> {
            let o = lba as usize * 512;
            self.0
                .get_mut(o..o + buf.len())
                .ok_or(BlockError::OutOfRange)?
                .copy_from_slice(buf);
            Ok(())
        }
    }

    fn card() -> (Card, u32) {
        let img = rp_fat::format::build(&rp_fat::format::Image {
            total_sectors: 16 * 1024 * 1024 / 512,
            sectors_per_cluster: 8,
            files: &[rp_fat::format::Entry {
                name: "RPIC0A.PAK",
                data: b"the icon pack",
            }],
            blocked_clusters: &[],
            ballast: 0,
        });
        let start = u32::from_le_bytes([img[454], img[455], img[456], img[457]]);
        (Card(img), start)
    }

    /// An app that writes `file` and keeps every answer.
    struct App {
        file: Vec<u8>,
        got: Vec<u8>,
        said: Vec<std::string::String>,
    }

    impl Host for App {
        fn piece(&mut self, off: u32, out: &mut [u8]) -> usize {
            let o = off as usize;
            let n = out.len().min(self.file.len().saturating_sub(o));
            out[..n].copy_from_slice(&self.file[o..o + n]);
            n
        }
        fn answer(&mut self, a: FileAnswer<'_>) {
            match a {
                FileAnswer::Piece { bytes, off, total } => {
                    assert_eq!(off as usize, self.got.len());
                    self.got.extend_from_slice(bytes);
                    self.said.push(std::format!("piece {total}"));
                }
                FileAnswer::Listed(n) => self.said.push(std::string::String::from_utf8(n.to_vec()).unwrap()),
                FileAnswer::Wrote => self.said.push("wrote".into()),
                FileAnswer::Failed(w) => self.said.push(w.into()),
            }
        }
    }

    fn name(n: &str) -> rp_app::Text<64> {
        rp_app::Text::of(n.as_bytes())
    }

    #[test]
    fn a_file_written_lists_reads_back_and_others_stay() {
        let (mut c, start) = card();
        let file: Vec<u8> = (0..10_000u32).map(|k| (k * 7) as u8).collect();
        let mut app = App { file: file.clone(), got: Vec::new(), said: Vec::new() };
        let mut buf = std::vec![0u8; BUF];
        let w = FileAsk::Write { dir: b"Notes", name: name("Trip plan.md"), len: file.len() as u32 };
        serve(&mut c, start, w, &mut app, &mut buf);
        assert_eq!(app.said, ["wrote"]);
        serve(&mut c, start, FileAsk::List { dir: b"Notes" }, &mut app, &mut buf);
        assert_eq!(app.said[1], "Trip plan.md");
        serve(&mut c, start, FileAsk::Read { dir: b"Notes", name: name("Trip plan.md") }, &mut app, &mut buf);
        assert_eq!(app.got, file);
        assert_eq!(app.said.len(), 2 + 3, "three pieces");
        // Written again shorter: the new one whole.
        app.file.truncate(10);
        app.got.clear();
        let w = FileAsk::Write { dir: b"Notes", name: name("Trip plan.md"), len: 10 };
        serve(&mut c, start, w, &mut app, &mut buf);
        serve(&mut c, start, FileAsk::Read { dir: b"Notes", name: name("Trip plan.md") }, &mut app, &mut buf);
        assert_eq!(app.got, &file[..10]);
        // The card's other file is untouched.
        let mut v = AnyVolume::mount(&mut c, start).unwrap();
        let mut other = Vec::new();
        v.read_range("RPIC0A.PAK", 0, 100, &mut [0u8; 4096], &mut |b| other.extend_from_slice(b))
            .unwrap();
        assert_eq!(other, b"the icon pack");
    }

    #[test]
    fn names_that_could_reach_out_of_the_folder_are_refused() {
        for bad in ["", "..", "../x.txt", "a/b.txt", "a\\b.txt", ".hidden", "x.", " x", "CON:", "a..b"] {
            assert!(!name_ok(bad.as_bytes()), "{bad}");
        }
        for good in ["Trip plan.md", "Note-1.svg", "a_b.c.txt"] {
            assert!(name_ok(good.as_bytes()), "{good}");
        }
        let (mut c, start) = card();
        let mut app = App { file: Vec::new(), got: Vec::new(), said: Vec::new() };
        let mut buf = std::vec![0u8; BUF];
        serve(&mut c, start, FileAsk::Read { dir: b"Notes", name: name("../RPIC0A.PAK") }, &mut app, &mut buf);
        serve(&mut c, start, FileAsk::List { dir: b".." }, &mut app, &mut buf);
        assert_eq!(app.said, ["NOT A NAME", "NOT A NAME"]);
        // A file not there, a folder not made yet.
        serve(&mut c, start, FileAsk::Read { dir: b"Notes", name: name("none.txt") }, &mut app, &mut buf);
        serve(&mut c, start, FileAsk::List { dir: b"Notes" }, &mut app, &mut buf);
        assert_eq!(app.said[2..], ["NOT FOUND ON THE CARD", ""]);
        // An app whose bytes run short: nothing linked, said.
        app.file = std::vec![1; 100];
        let w = FileAsk::Write { dir: b"Notes", name: name("short.txt"), len: 5000 };
        serve(&mut c, start, w, &mut app, &mut buf);
        assert_eq!(app.said[4], "THE NOTE CHANGED: NOT SAVED");
        serve(&mut c, start, FileAsk::List { dir: b"Notes" }, &mut app, &mut buf);
        assert_eq!(app.said[5], "");
        // An empty file reads as one empty piece.
        app.file.clear();
        let w = FileAsk::Write { dir: b"Notes", name: name("empty.txt"), len: 0 };
        serve(&mut c, start, w, &mut app, &mut buf);
        app.got.clear();
        serve(&mut c, start, FileAsk::Read { dir: b"Notes", name: name("empty.txt") }, &mut app, &mut buf);
        assert_eq!(app.said[app.said.len() - 1], "piece 0");
    }
}
