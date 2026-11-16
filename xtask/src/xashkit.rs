//! `cargo xtask xash-kit`: the Half-Life port's test run for the X51v,
//! laid out as files (ADR-0039; docs/reminders.md R278): the runner's
//! image for `cargo xtask load`, and the folder that goes onto the SD
//! card, whole, as `rp-xash`.
//!
//! Everything the run needs on the card is inside that one folder, and
//! everything the run writes goes there too (`-rpbase /rp-xash/game`,
//! with the engine's `-rodir /Ports` for reading the owner's game): the
//! card is shared, and the owner's `Ports/valve` is neither written by
//! the PC nor by the run. The menu's prepared background
//! (`crate::hlprep`) is found by the engine in its own write folder
//! before the game's, so it is under `rp-xash/game/valve` and not beside
//! the owner's files.
//!
//! Iterate, don't imitate (rule 0). A port is usually installed by
//! copying a build's files into the game's folder by hand, with the
//! command line in a shortcut; what ran on the device is then whatever
//! the hand copied. Here one command builds the image and lays out
//! exactly the files with their SHA-256 in a list, and `cargo xtask
//! emu-xash-kit` runs those same bytes under the loader in the emulator
//! from start to their own end, so what the X51v gets is what was
//! checked.

use std::path::{Path, PathBuf};

/// The run's folder on the card (the runner's: `rp_port_runner::FOLDER`).
pub const FOLDER: &str = "rp-xash";
/// The map the run plays: Anomalous Materials' first, the one the
/// emulator has played.
pub const MAP: &str = "c1a0";
/// Seconds of play the run measures before it ends by itself.
pub const PLAY_SECS: u32 = 120;
/// Seconds on the run's clock at which it ends whatever the engine is
/// doing (a map that never loads). The emulator starts the map in about
/// three minutes; the X51v's card is not measured yet, so this leaves it
/// several times that.
pub const LIMIT_SECS: u32 = 1200;
// The limit leaves the play its time after a slow start.
const _: () = assert!(LIMIT_SECS >= PLAY_SECS * 5);
/// The engine's low-memory build the kit is made with: an experiment's
/// build, not the port's (docs/reminders.md R277), and said as such on
/// the run's screen.
pub const LOW_MEMORY: u8 = 1;

/// The run's command line after the program's name, as `args.txt` holds
/// it: quiet (no `-dev`, no memory lists: the run log holds 64 KiB), the
/// frame rate every five seconds, its own end, its last frame written
/// once at that end, where the program is sampled a thousand times a
/// second for its first minute (`-rpsample`: `samples.bin`, named on the
/// PC by `cargo xtask xash-profile`, R285), and its writes kept in its
/// own folder.
#[must_use]
pub fn args() -> String {
    format!(
        "+map {MAP} -rpfps -rpend {PLAY_SECS} -rplimit {LIMIT_SECS} -rpendshot -rpsample -rpfloat -rpbase /{FOLDER}/game -rodir /Ports"
    )
}

/// The kit, laid out.
pub struct Kit {
    /// Its folder (`target/xash-kit`).
    pub dir: PathBuf,
    /// The runner's image, for `cargo xtask load <file>`.
    pub image: PathBuf,
    /// The card's files: each one's path from the card's root and bytes.
    pub card: Vec<(String, Vec<u8>)>,
}

/// The card's files for the run: the `modules` (name and bytes, as
/// built), the command line and, if the game has one in pieces, the
/// menu's `background` as prepared. All under [`FOLDER`].
#[must_use]
pub fn card_files(
    modules: &[(String, Vec<u8>)],
    background: Option<Vec<u8>>,
) -> Vec<(String, Vec<u8>)> {
    let mut card: Vec<(String, Vec<u8>)> = modules
        .iter()
        .map(|(name, bytes)| (format!("{FOLDER}/{name}"), bytes.clone()))
        .collect();
    card.push((format!("{FOLDER}/args.txt"), args().into_bytes()));
    if let Some(file) = background {
        card.push((
            format!("{FOLDER}/game/valve/{}", crate::hlprep::BACKGROUND),
            file,
        ));
    }
    card
}

/// The list that goes with the kit: each file's size, SHA-256 and path,
/// and what to do with them.
fn list(image: &[u8], card: &[(String, Vec<u8>)]) -> String {
    let line = |name: &str, bytes: &[u8]| {
        let hex = rp_crypto::hex(&rp_crypto::Sha256::digest(bytes));
        format!(
            "{:>9}  {}  {name}\n",
            bytes.len(),
            String::from_utf8_lossy(&hex)
        )
    };
    let mut text = String::from(
        "The Half-Life port's test run for the X51v (cargo xtask xash-kit).\n\
         An experiment build: the engine's low-memory build 1, the software\n\
         renderer, the game's picture not shown. Not the port.\n\n\
         Everything goes into the folder rp-xash on the SD card; nothing else\n\
         on the card is written, by these commands or by the run. With the\n\
         loader up (each file is checked on the X51v by its SHA-256):\n\n",
    );
    let (small, large): (Vec<_>, Vec<_>) = card
        .iter()
        .partition(|(name, bytes)| crate::usb::put_fits(name, bytes.len()));
    for (name, _) in &small {
        text.push_str(&format!(
            "  cargo xtask put target/xash-kit/card/{name} {name}\n"
        ));
    }
    if !large.is_empty() {
        text.push_str(&format!(
            "\nThese are larger than one put takes (at most about {} KiB):\n\
             through the loader's drive mode, then each checked on the X51v:\n\n\
             \x20 cargo xtask drive rw\n",
            crate::usb::put_max("rp-xash/x") / 1024
        ));
        for (name, _) in &large {
            text.push_str(&format!(
                "  copy target/xash-kit/card/{name} to the X51V SD drive as {name}\n"
            ));
        }
        for (name, _) in &large {
            text.push_str(&format!("  cargo xtask hash {name}\n"));
        }
        text.push_str("  cargo xtask drive ro\n");
    }
    text.push_str(&format!(
        "\nThen the run (it ends by itself; the wait is for a slow start):\n\n\
         \x20 RETROPANE_LOG_WAIT_SECS={} cargo xtask load target/xash-kit/rp-port-runner.bin\n\n\
         Then its samples back from the card and named against the kit's own ELFs\n\
         (where the X51v spent its time, R285):\n\n\
         \x20 cargo xtask get rp-xash/samples.bin target/xash-kit/samples.bin\n\
         \x20 cargo xtask xash-profile target/xash-kit/samples.bin\n\n\
         The run's command line (rp-xash/args.txt):\n  ",
        LIMIT_SECS + 300
    ));
    text.push_str(&args());
    text.push_str("\n\nbytes, SHA-256, file:\n");
    text.push_str(&line("rp-port-runner.bin", image));
    for (name, bytes) in card {
        text.push_str(&line(&format!("card/{name}"), bytes));
    }
    text
}

/// Lays the kit out in `target/xash-kit`: `runner` (the linked image) as
/// a raw binary, the card's files under `card/`, and `kit.txt`.
/// `game` is the folder that holds the owner's `valve` (read only: the
/// menu's background is made from it).
///
/// # Errors
/// A file could not be written, or the game's background is damaged.
pub fn lay_out(
    root: &Path,
    runner: &Path,
    modules: &[(String, Vec<u8>)],
    game: &Path,
) -> Result<Kit, String> {
    let dir = root.join("target").join("xash-kit");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let image = dir.join("rp-port-runner.bin");
    crate::sdkit::raw_binary(runner, &image)?;
    // The ELFs of exactly what is sent, for naming a run's samples
    // (`cargo xtask xash-profile`): the modules' folder is rebuilt by
    // every emulator run.
    let elf = dir.join("elf");
    std::fs::create_dir_all(&elf).map_err(|e| format!("{}: {e}", elf.display()))?;
    let built = root.join("target").join("xash-modules");
    for (from, to) in [
        (runner.to_path_buf(), "rp-port-runner"),
        (built.join("engine.elf"), "engine.elf"),
        (built.join("client.elf"), "client.elf"),
        (built.join("server.elf"), "server.elf"),
    ] {
        std::fs::copy(&from, elf.join(to)).map_err(|e| format!("{}: {e}", from.display()))?;
    }
    let background = crate::hlprep::background(&game.join("valve"))?.map(|(file, _)| file);
    if background.is_none() {
        eprintln!("    the game has no menu background in pieces: none in the kit");
    }
    let card = card_files(modules, background);
    for (name, bytes) in &card {
        let path = dir.join("card").join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    let image_bytes = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let text = list(&image_bytes, &card);
    let path = dir.join("kit.txt");
    std::fs::write(&path, &text).map_err(|e| format!("{}: {e}", path.display()))?;
    eprint!("{text}");
    eprintln!("    the kit: {}", dir.display());
    Ok(Kit { dir, image, card })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_line_is_quiet_ends_by_itself_and_writes_in_its_own_folder() {
        let args = args();
        let words: Vec<&str> = args.split(' ').collect();
        let after = |w: &str| {
            words
                .iter()
                .position(|x| *x == w)
                .and_then(|i| words.get(i + 1).copied())
        };
        assert_eq!(after("+map"), Some(MAP));
        assert_eq!(after("-rpend"), Some("120"));
        assert_eq!(after("-rplimit"), Some("1200"));
        assert_eq!(after("-rpbase"), Some("/rp-xash/game"));
        assert_eq!(after("-rodir"), Some("/Ports"));
        assert!(words.contains(&"-rpfps") && words.contains(&"-rpendshot"));
        assert!(words.contains(&"-rpsample") && words.contains(&"-rpfloat"));
        // Nothing that fills the 64 KiB run log, nothing written while
        // frames are timed.
        for loud in ["-dev", "-rpmemlist", "-rpshot", "-log"] {
            assert!(!words.contains(&loud), "{loud}");
        }
        // The runner's command line holds 32 words with the program's
        // name, its file 511 bytes.
        assert!(words.len() < 32 && args.len() < 512);
    }

    #[test]
    fn every_file_of_the_kit_is_inside_the_runs_own_folder() {
        let modules = vec![
            ("server.rpm".to_string(), vec![1u8; 3]),
            ("engine.rpm".to_string(), vec![2u8; 4]),
        ];
        let card = card_files(&modules, Some(vec![9u8; 5]));
        let names: Vec<&str> = card.iter().map(|f| f.0.as_str()).collect();
        assert_eq!(
            names,
            [
                "rp-xash/server.rpm",
                "rp-xash/engine.rpm",
                "rp-xash/args.txt",
                "rp-xash/game/valve/retropane/background.tga"
            ]
        );
        assert!(names.iter().all(|n| n.starts_with("rp-xash/")));
        assert_eq!(card[2].1, args().into_bytes());
        // A game with no background in pieces: the kit has none.
        assert_eq!(card_files(&modules, None).len(), 3);
    }

    #[test]
    fn the_list_names_every_file_with_its_size_and_digest() {
        let card = card_files(&[("engine.rpm".to_string(), b"abc".to_vec())], None);
        let text = list(b"image", &card);
        // SHA-256 of "abc".
        assert!(text.contains(
            "        3  ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  card/rp-xash/engine.rpm"
        ));
        assert!(text.contains("  rp-port-runner.bin\n"));
        // The commands that put each file where the run looks for it.
        assert!(text.contains(
            "  cargo xtask put target/xash-kit/card/rp-xash/engine.rpm rp-xash/engine.rpm\n"
        ));
        assert!(text.contains(
            "RETROPANE_LOG_WAIT_SECS=1500 cargo xtask load target/xash-kit/rp-port-runner.bin"
        ));
        // A name the loader's `put` takes (its longest is 64 bytes).
        let longest = card_files(&[], Some(Vec::new()));
        assert!(longest.iter().all(|f| f.0.len() <= 64));
        assert!(text.contains(&args()));
    }

    #[test]
    fn a_file_larger_than_one_put_goes_through_drive_mode_and_is_checked() {
        // The loader's limit as found on the X51v (2026-11-15): 983 KiB
        // and larger were refused, 553 KiB taken.
        let name = "rp-xash/server.rpm";
        let max = crate::usb::put_max(name);
        assert!((553 * 1024..983 * 1024).contains(&max), "{max}");
        assert!(crate::usb::put_fits(name, max) && !crate::usb::put_fits(name, max + 1));
        let card = card_files(
            &[
                ("server.rpm".to_string(), vec![1u8; max + 1]),
                ("client.rpm".to_string(), vec![2u8; max]),
            ],
            None,
        );
        let text = list(b"image", &card);
        assert!(text.contains(
            "  cargo xtask put target/xash-kit/card/rp-xash/client.rpm rp-xash/client.rpm
"
        ));
        assert!(!text.contains("cargo xtask put target/xash-kit/card/rp-xash/server.rpm"));
        let drive = text
            .find(
                "  cargo xtask drive rw
",
            )
            .expect("drive mode");
        let copy = text
            .find("  copy target/xash-kit/card/rp-xash/server.rpm to the X51V SD drive as rp-xash/server.rpm
")
            .expect("the copy");
        let hash = text
            .find(
                "  cargo xtask hash rp-xash/server.rpm
",
            )
            .expect("the check");
        let ro = text
            .find(
                "  cargo xtask drive ro
",
            )
            .expect("read-only again");
        assert!(drive < copy && copy < hash && hash < ro);
        assert!(!text.contains("hash rp-xash/client.rpm"));
    }
}
