//! App programs loaded from storage (the owner, 2026-11-14: every app its
//! own program, loaded when opened and released when left;
//! docs/design/app-scope.md).
//!
//! An app is a PE user image (`hal/user/user.x`, `xtask/src/userimg.rs`)
//! kept as a file: in internal storage once RetroPane is installed, on the
//! SD card in this test stage (the same file, only the storage under it
//! differs: bare metal first). Opening one reads the file, lays its view out
//! in the program slot with every absolute address relocated to where the
//! slot is (`rp-kernel-pe`, the kernel's own loader for processes), and
//! returns the entry the shell calls (`rt::call_program`). Leaving the app
//! frees nothing: the next opening loads over the slot.
//!
//! Iterate, don't imitate (rule 0): Windows Mobile ran each app as a
//! process from ROM or RAM; a shell with its apps compiled in (our version
//! 1) had to fit them all in one image. Ours keeps one slot and loads only
//! the app in use, from the same file format the kernel will start as a
//! process, so a new app costs storage, not image space or RAM.

use rp_kernel_pe::{Image, MACHINE_ARM, PAGE, Relocations, dir};

/// The programs, in the Start menu's order: the name and the card file's
/// stem (`<stem>A.PAK`, ... as `cargo xtask load` puts them).
pub const PROGRAMS: [(&str, [u8; 5]); 5] = [
    ("Calendar", *b"APCAL"),
    ("Contacts", *b"APCON"),
    ("Tasks", *b"APTSK"),
    ("Notes", *b"APNOT"),
    ("Calculator", *b"APCLC"),
];

/// The program slot: the largest app's view and its zeroed RAM (the five
/// measured 16-49 KB of file on 2026-11-14; the view is a little more).
pub const SLOT: usize = 256 * 1024;
/// The largest program file read.
pub const FILE: usize = 128 * 1024;
/// The largest relocation table read.
pub const RELOCS: usize = 16 * 1024;
/// The scratch [`load`] needs: the relocation table and a page.
pub const SCRATCH: usize = RELOCS + PAGE as usize;

/// Why a program didn't load.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Refused {
    /// Not a PE file, or not ARM, or its layout is wrong.
    NotProgram,
    /// Bigger than the slot.
    TooBig,
    /// Code that could also be written (W^X).
    WritableCode,
    /// No entry point inside the image.
    NoEntry,
}

/// Lays `file` out in `slot` (whose first byte is at address `at`),
/// relocated to it: the entry's address, or why not. `scratch` holds the
/// relocation table and a page ([`SCRATCH`] bytes; never on the stack: the
/// image's 24 KiB stack overflowed with them there, emu-apps 2026-11-14).
pub fn load(file: &[u8], slot: &mut [u8], at: u32, scratch: &mut [u8]) -> Result<u32, Refused> {
    if scratch.len() < SCRATCH {
        return Err(Refused::TooBig);
    }
    let (table, page) = scratch.split_at_mut(RELOCS);
    let page: &mut [u8; PAGE as usize] = (&mut page[..PAGE as usize])
        .try_into()
        .map_err(|_| Refused::TooBig)?;
    let image = Image::parse(file).map_err(|_| Refused::NotProgram)?;
    if image.machine != MACHINE_ARM {
        return Err(Refused::NotProgram);
    }
    if image.image_size as usize > slot.len() {
        return Err(Refused::TooBig);
    }
    if image.writable_code() {
        return Err(Refused::WritableCode);
    }
    if image.entry == 0 || image.entry >= image.image_size {
        return Err(Refused::NoEntry);
    }
    let read = |off: u32, buf: &mut [u8]| -> Result<(), ()> {
        let src = file.get(off as usize..off as usize + buf.len()).ok_or(())?;
        buf.copy_from_slice(src);
        Ok(())
    };
    // The relocation table, read from the image's view.
    let rd = image.dirs[dir::BASERELOC];
    let n = (rd.size as usize).min(RELOCS);
    if rd.size as usize > RELOCS {
        return Err(Refused::TooBig);
    }
    image
        .read_view(rd.rva, &mut table[..n], read)
        .map_err(|_| Refused::NotProgram)?;
    let relocs = Relocations::new(&table[..n]).map_err(|_| Refused::NotProgram)?;
    let delta = at.wrapping_sub(image.image_base);
    // The whole slot cleared first: a program's zeroed RAM must read zero
    // (its one-shot state cells), and the pool it comes from holds the last
    // program's bytes (emu-apps 2026-11-14: Calendar's slot read "taken").
    slot.fill(0);
    for p in 0..image.pages() {
        image
            .load_page(p, page, &relocs, delta, read)
            .map_err(|_| Refused::NotProgram)?;
        let from = (p * PAGE) as usize;
        let to = (from + PAGE as usize).min(slot.len());
        slot[from..to].copy_from_slice(&page[..to - from]);
    }
    Ok(at + image.entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn junk_and_empty_files_are_refused() {
        let mut slot = vec![0u8; SLOT];
        let mut sc = vec![0u8; SCRATCH];
        assert_eq!(
            load(&[], &mut slot, 0xA900_0000, &mut sc),
            Err(Refused::NotProgram)
        );
        assert_eq!(
            load(&[0x4D; 512], &mut slot, 0xA900_0000, &mut sc),
            Err(Refused::NotProgram)
        );
        // An MZ header with nothing behind it.
        let mut f = vec![0u8; 512];
        f[..2].copy_from_slice(b"MZ");
        assert_eq!(
            load(&f, &mut slot, 0xA900_0000, &mut sc),
            Err(Refused::NotProgram)
        );
    }

    #[test]
    fn every_program_has_a_card_file_of_its_own() {
        for (i, (_, a)) in PROGRAMS.iter().enumerate() {
            for (_, b) in &PROGRAMS[i + 1..] {
                assert_ne!(a, b);
            }
            assert!(a.iter().all(u8::is_ascii_uppercase));
        }
    }
}
