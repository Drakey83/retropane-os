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
/// The variable for the map a run starts on (R309: the sky and water of
/// c2a5 in the emulator, not reached by the walk's tour there: R311).
pub const MAP_VAR: &str = "RETROPANE_XASH_MAP";

/// The run's first map: [`MAP_VAR`]'s if it is a map's name (letters,
/// digits and `_`, up to 31), else [`MAP`].
fn map() -> String {
    std::env::var(MAP_VAR)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| {
            (1..32).contains(&v.len()) && v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
        .unwrap_or_else(|| MAP.to_string())
}

/// The variable for a longer run's seconds of play (a soak: R295's
/// confirmation at 133.25 / 1); the game reads the first `-rpend` on its
/// line, so the kit's own number is what changes.
pub const PLAY: &str = "RETROPANE_XASH_PLAY";

/// Seconds of play: [`PLAY`]'s if it is a number from 1 to an hour, else
/// [`PLAY_SECS`].
pub fn play_secs() -> u32 {
    std::env::var(PLAY)
        .ok()
        .and_then(|v| v.trim().parse::<u32>().ok())
        .filter(|s| (1..=3600).contains(s))
        .unwrap_or(PLAY_SECS)
}

/// The run's limit: [`LIMIT_SECS`], or five times a longer play.
fn limit_secs() -> u32 {
    LIMIT_SECS.max(play_secs() * 5)
}
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
///
/// An experiment's words go after them from `RETROPANE_XASH_EXTRA` (one
/// measured change a run, such as `+r_norefresh 1` for the frame with no
/// 3D view: R282); `kit.txt` and the emulator's check carry the same line.
#[must_use]
pub fn args() -> String {
    with_extra(
        format!(
            "+map {} -rpfps -rpend {} -rplimit {} -rpendshot -rpsample -rpfloat -rpbase /{FOLDER}/game -rodir /Ports -rpclock {} -console +cl_nopred 0 +cl_lw 1 +rp_defaults {LAYOUT}",
            map(),
            play_secs(),
            limit_secs(),
            clock()
        ),
        std::env::var(EXTRA).ok().as_deref(),
    )
}

/// The owner's button layout (2026-11-15, the mock-up hl-controls-doomgles;
/// ports/xash/platform/in_rp.c turns each X51v button into an engine key):
/// the D-pad walks and turns, its centre fires, Calendar jumps, Contacts
/// is use, Mail crouches, Home reloads, Record the next weapon; the
/// wireless button is the menu (Escape) already. Run 35 (the owner): "the
/// hardware buttons ... didnt appear to do anything other then walk": the
/// layout had never been written. A file of the game's own `bind`s,
/// exec'd after the engine's saved config, so it wins; the command line
/// cannot carry `+jump` as a bind's argument (a `+` starts a command).
/// Runs 35-38 (the owner: "no button besides d pad"): the engine's key
/// codes `K_AUX1`-`K_AUX5` are named `A_BUTTON`, `B_BUTTON`, `X_BUTTON`,
/// `Y_BUTTON` and `L1_BUTTON` in its table (Xash `in_keys.c`; keydefs.h
/// `K_A_BUTTON K_AUX1` ...): `bind AUX1` named no key and was refused.
/// [`KEY_NAMES`] holds the names in_rp.c's keys carry; the test checks
/// every bind against it. The `echo` line shows in the log that the file
/// ran (the engine's console reaches the log with `-console`).
/// The owner, run 46: "i would like the flashlight key and the secondary
/// fire keys to be on the screen like i originally read, home to bring up
/// the menu, and wifi/record to switch through weapons": Home (Y_BUTTON)
/// `togglemenu`, Record (L1_BUTTON) the weapon before, the wireless button
/// (R1_BUTTON) the next (swapped by the owner, 2026-11-20, run 277); the two touch keys (`rp_hl_keys.h`) send
/// C_BUTTON (the flashlight, `impulse 100`) and Z_BUTTON (the second
/// fire). Every one an engine key, so the game's Controls menu remaps it.
/// `sensitivity 12.8` (the owner, run 46: the stylus look "not nearly
/// sensative enough"): the engine turns a mouse count by `sensitivity` x
/// `m_yaw` 0.022 degrees, a desk mouse's scale (3: 42 degrees across the
/// screen's 640 pixels); rule 0, as touch shooters do, one stroke across the
/// long side turns half a circle: 180 / 640 / 0.022 = 12.8.
pub const BINDS: &str = "bind UPARROW +forward
bind DOWNARROW +back
bind LEFTARROW +left
bind RIGHTARROW +right
bind ENTER +attack
bind A_BUTTON +jump
bind B_BUTTON +use
bind X_BUTTON +duck
bind Y_BUTTON togglemenu
bind L1_BUTTON invprev
bind R1_BUTTON invnext
bind C_BUTTON impulse 100
bind Z_BUTTON +attack2
unbind MOUSE1
sensitivity 12.8
rp_tk_alpha 0.18
echo rp_binds.cfg: the X51v's buttons bound
";
/// The engine's names (Xash `engine/client/input/in_keys.c` `keynames`)
/// of the keys `ports/xash/platform/in_rp.c` sends: the D-pad's arrows,
/// its centre, the four program buttons and Record (`K_AUX1`-`K_AUX5`)
/// (the pen's tap is not bound: the owner, 2026-11-16, only the D-pad's
/// centre fires; dragging looks around).
#[cfg(test)]
const KEY_NAMES: [&str; 13] = [
    "UPARROW",
    "DOWNARROW",
    "LEFTARROW",
    "RIGHTARROW",
    "ENTER",
    "A_BUTTON",
    "B_BUTTON",
    "X_BUTTON",
    "Y_BUTTON",
    "L1_BUTTON",
    "R1_BUTTON",
    "C_BUTTON",
    "Z_BUTTON",
];
/// [`BINDS`]' file in the game's folder on the card.
pub const BINDS_FILE: &str = "rp_binds.cfg";
/// The layout's number: `rp_defaults` execs [`BINDS_FILE`] only when the
/// saved config's `rp_layout` is older (the player's remaps kept; the
/// owner, run 46: "make sure you make the key remappable"). Raise it with
/// any change to [`BINDS`] (3: the touch keys at 25%, the owner after run
/// 48: "the on screen buttons need to be more transparent"; 4: Record the
/// weapon before, the wireless button the next, the owner's swap of run
/// 277, which 48051789 made in [`BINDS`] without raising this, so the saved
/// config's layout 3 kept the old order: the owner, run 279, "the record
/// and wifi swap got swapped back"; 5: the touch keys' glass at 0.18, the
/// owner 2026-11-22, which 0c35f47a made in [`BINDS`] without raising this,
/// so his saved layout 4 kept 0.25). [`LAYOUT_OF`] holds the binds' sum
/// each layout was raised for: a test fails a change to [`BINDS`] that
/// left the number as it was.
pub const LAYOUT: u32 = 5;
/// The CRC-32 of [`BINDS`] when [`LAYOUT`] was last raised.
pub const LAYOUT_OF: u32 = 0xFBEB6547;

/// The 2700G's clock while the game draws (the runner's `-rpclock`: PLL
/// M, N, P, the cores' divider): 125.67 / 1, the owner's ruling of
/// 2026-11-16 ("make 125 the default clock for halflife"; ADR-0035, "The
/// TA path's step"): the fastest rung drawn clean on Half-Life's TA path
/// on this X51v, ~34,000 frames without a hang in runs 32-35 (standing,
/// walking, the owner playing); 130.00 / 1 hung 1 in 1,200, 133.25 1 in
/// 53, 136.50 about 1 in 5 (R295). An experiment's own `-rpclock` in
/// `RETROPANE_XASH_EXTRA` comes later on the line and wins.
pub const CLOCK: &str = "29 3 0 1";

/// An experiment's clock in place of [`CLOCK`] (`RETROPANE_XASH_CLOCK`,
/// four numbers: PLL M, N, P, the cores' divider): the same four words,
/// where an `-rpclock` in [`EXTRA`] costs five of the runner's 31 (run
/// 279). Anything but four whole numbers is ignored (the default kept).
pub const CLOCK_VAR: &str = "RETROPANE_XASH_CLOCK";

fn clock() -> String {
    std::env::var(CLOCK_VAR)
        .ok()
        .filter(|v| {
            let n: Vec<&str> = v.split_whitespace().collect();
            n.len() == 4 && n.iter().all(|w| w.parse::<u32>().is_ok())
        })
        .map_or_else(
            || CLOCK.to_string(),
            |v| v.split_whitespace().collect::<Vec<_>>().join(" "),
        )
}

/// The variable naming an experiment's extra words for the command line.
pub const EXTRA: &str = "RETROPANE_XASH_EXTRA";

/// `base`, then `extra`'s words if it has any.
fn with_extra(base: String, extra: Option<&str>) -> String {
    match extra.map(str::trim) {
        Some(words) if !words.is_empty() => format!("{base} {words}"),
        _ => base,
    }
}

/// An experiment's I-cache lock (R324 run 92: the cascade's heavy frames
/// lose 44.5% of their cycles to the I-cache): the KiB of the modules'
/// code to lock, taken from [`LOCK_LIST`] in its order, into the card's
/// `lock.bin` (the runner's `lock_icache`).
pub const LOCK_KIB: &str = "RETROPANE_XASH_LOCK_KIB";

/// The lock's size when [`LOCK_KIB`] is not set: none (R324, the X51v).
/// It paid while the debris lit each piece between its draws (kit 88,
/// runs 94-96: the heavy half 9.6 -> 10.8 fps at 24 KiB), but once kit 89
/// ran the lighting as a stage of its own the path nearly fits the cache
/// and a lock only takes ways from it (runs 98-101: 11.8 fps at 24 KiB,
/// 14.4 at 8 KiB re-ranked, 14.7 with none).
pub const LOCK_KIB_DEFAULT: u32 = 0;

/// The functions to lock, most I-cache stalls a byte first: `module
/// symbol` a line (the symbol as the ELF names it), `#` a comment.
pub const LOCK_LIST: &str = "ports/xash/icache-lock.txt";

/// `lock.bin` for [`LOCK_LIST`]'s `list`, up to `kib` KiB of code (each
/// function whole, in 32-byte lines): each function's address and length
/// as two little-endian words, from the kit's ELFs in `elf` (linked at the
/// addresses they load at). A name no ELF has is an error: the list is
/// stale. Also what it came to, in words.
///
/// # Errors
/// `llvm-nm` failed, or a listed function is not in its module.
pub fn lock_ranges(list: &str, elf: &Path, kib: u32) -> Result<(Vec<u8>, String), String> {
    use std::collections::HashMap;
    let mut table: HashMap<(String, String), (u32, u32)> = HashMap::new();
    for m in ["engine", "client", "server"] {
        let out = std::process::Command::new(crate::symbols::llvm_tool("llvm-nm"))
            .args(["-S", "--defined-only"])
            .arg(elf.join(format!("{m}.elf")))
            .output()
            .map_err(|e| format!("llvm-nm: {e}"))?;
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let mut w = line.split_whitespace();
            if let (Some(a), Some(n), Some(t), Some(name)) =
                (w.next(), w.next(), w.next(), w.next())
                && (t == "t" || t == "T")
                && let (Ok(a), Ok(n)) = (u32::from_str_radix(a, 16), u32::from_str_radix(n, 16))
                && n > 0
            {
                table.insert((m.to_string(), name.to_string()), (a, n));
            }
        }
    }
    pick_ranges(list, &table, kib)
}

/// [`lock_ranges`]' choice, from `table` (each module's functions: name to
/// address and length): `list`'s functions in order, each whole in 32-byte
/// lines, while they fit in `kib` KiB (one that doesn't is passed over for
/// a smaller one after it).
///
/// # Errors
/// A line is not `module symbol`, or names a function `table` lacks.
fn pick_ranges(
    list: &str,
    table: &std::collections::HashMap<(String, String), (u32, u32)>,
    kib: u32,
) -> Result<(Vec<u8>, String), String> {
    let budget = kib * 1024;
    let (mut bytes, mut used, mut count) = (Vec::new(), 0u32, 0);
    for line in list
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let mut w = line.split_whitespace();
        let (Some(m), Some(name)) = (w.next(), w.next()) else {
            return Err(format!("{LOCK_LIST}: not `module symbol`: {line}"));
        };
        let &(at, len) = table
            .get(&(m.to_string(), name.to_string()))
            .ok_or_else(|| {
                format!("{LOCK_LIST}: {m} has no function {name} (the list is stale)")
            })?;
        let lines = (at + len).div_ceil(32) - at / 32;
        if used + lines * 32 > budget {
            continue;
        }
        used += lines * 32;
        count += 1;
        bytes.extend_from_slice(&at.to_le_bytes());
        bytes.extend_from_slice(&len.to_le_bytes());
    }
    Ok((
        bytes,
        format!(
            "I-cache lock: {count} functions, {} KiB of {kib} KiB, into lock.bin",
            used / 1024
        ),
    ))
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
    card.push((
        format!("{FOLDER}/game/valve/{BINDS_FILE}"),
        BINDS.as_bytes().to_vec(),
    ));
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
        limit_secs() + 300
    ));
    text.push_str(&args());
    text.push_str("\n\nbytes, SHA-256, file:\n");
    text.push_str(&line("rp-port-runner.bin", image));
    for (name, bytes) in card {
        text.push_str(&line(&format!("card/{name}"), bytes));
    }
    text
}

/// `sram.bin` (ADR-0040 step 7b): each module's `.rp_hot` pages, from
/// `__rp_hot` to `__rp_hot_end` in its ELF in `elf`, as the address and
/// length (little-endian words, both multiples of 64 KiB) the runner maps
/// onto the on-chip SRAM; `None` when no module has any. Also what it came
/// to, in words.
///
/// # Errors
/// `llvm-nm` failed, a module's hot pages are not whole 64 KiB pages, or
/// they come to more than the SRAM's 256 KiB.
pub fn sram_ranges(elf: &Path) -> Result<Option<(Vec<u8>, String)>, String> {
    let mut bytes = Vec::new();
    let mut said = Vec::new();
    let mut total = 0u32;
    for m in ["engine", "client", "server"] {
        let out = std::process::Command::new(crate::symbols::llvm_tool("llvm-nm"))
            .arg("--defined-only")
            .arg(elf.join(format!("{m}.elf")))
            .output()
            .map_err(|e| format!("llvm-nm: {e}"))?;
        let (mut from, mut to) = (None, None);
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let mut w = line.split_whitespace();
            if let (Some(a), Some(_), Some(name)) = (w.next(), w.next(), w.next())
                && let Ok(a) = u32::from_str_radix(a, 16)
            {
                match name {
                    "__rp_hot" => from = Some(a),
                    "__rp_hot_end" => to = Some(a),
                    _ => {}
                }
            }
        }
        let (Some(from), Some(to)) = (from, to) else {
            continue;
        };
        if to <= from {
            continue;
        }
        if from % 0x1_0000 != 0 || to % 0x1_0000 != 0 {
            return Err(format!(
                "{m}: its hot code {from:08x}-{to:08x} is not whole 64 KiB pages"
            ));
        }
        total += to - from;
        bytes.extend_from_slice(&from.to_le_bytes());
        bytes.extend_from_slice(&(to - from).to_le_bytes());
        said.push(format!("{m} {} KiB at {from:08x}", (to - from) / 1024));
    }
    if total > 256 * 1024 {
        return Err(format!(
            "the hot code comes to {} KiB, past the SRAM's 256",
            total / 1024
        ));
    }
    if bytes.is_empty() {
        return Ok(None);
    }
    Ok(Some((
        bytes,
        format!("on-chip SRAM: {} into sram.bin", said.join(", ")),
    )))
}

/// Lays the kit out in `target/xash-kit`: `runner` (the linked image) as
/// a raw binary, the card's files under `card/`, and `kit.txt`.
/// `game` is the folder that holds the owner's `valve` (read only: the
/// menu's background is made from it).
///
/// # Errors
/// A file could not be written, or the game's background is damaged.
/// The runner's command line: 32 words with the program's name, its file
/// at most 511 bytes (`rp_hal_cabi::ARGS`; the runner refuses a longer
/// `args.txt` at its step 1: run 279's extra words made 32 and the X51v
/// ran nothing). Checked when the kit is built, not on the device.
///
/// # Errors
///
/// The line, its word count and its bytes, when it does not fit.
pub fn args_fit(line: &str) -> Result<(), String> {
    let words = line.split_whitespace().count();
    if words < 32 && line.len() < 512 {
        Ok(())
    } else {
        Err(format!(
            "the kit's command line is {words} words and {} bytes; the runner holds 31 words and 511 bytes (drop words from {EXTRA}): {line}",
            line.len()
        ))
    }
}

pub fn lay_out(
    root: &Path,
    runner: &Path,
    modules: &[(String, Vec<u8>)],
    game: &Path,
) -> Result<Kit, String> {
    args_fit(&args())?;
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
    let mut card = card_files(modules, background);
    let kib = match std::env::var(LOCK_KIB) {
        Ok(v) => v
            .trim()
            .parse::<u32>()
            .map_err(|_| format!("{LOCK_KIB}: not a number of KiB: {v}"))?,
        Err(_) => LOCK_KIB_DEFAULT,
    };
    if kib > 0 {
        let list = std::fs::read_to_string(root.join(LOCK_LIST))
            .map_err(|e| format!("{LOCK_LIST}: {e}"))?;
        let (bytes, said) = lock_ranges(&list, &elf, kib)?;
        eprintln!("    {said}");
        card.push((format!("{FOLDER}/lock.bin"), bytes));
    }
    if let Some((bytes, said)) = sram_ranges(&elf)? {
        eprintln!("    {said}");
        card.push((format!("{FOLDER}/sram.bin"), bytes));
    }
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
    #[test]
    fn the_lock_list_takes_whole_functions_in_order_within_the_budget() {
        let mut t = std::collections::HashMap::new();
        let mut put = |m: &str, n: &str, at: u32, len: u32| {
            t.insert((m.to_string(), n.to_string()), (at, len));
        };
        put("engine", "a", 0xA8E0_0010, 600); // lines 0x..00-0x..260: 20 lines
        put("client", "b", 0xA8D1_0000, 1024); // 32 lines
        put("engine", "c", 0xA8E1_0000, 64); // 2 lines
        let list = "# hot first
engine a

client b
engine c
";
        let (bytes, said) = super::pick_ranges(list, &t, 1).unwrap();
        // a (640 bytes of lines) fits 1 KiB, b (1 KiB) doesn't after it, c does.
        let words: Vec<u32> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|w| u32::from_le_bytes(*w))
            .collect();
        assert_eq!(words, [0xA8E0_0010, 600, 0xA8E1_0000, 64]);
        assert!(said.contains("2 functions"), "{said}");
        // A stale list is an error, not a silent skip.
        assert!(
            super::pick_ranges(
                "server gone
",
                &t,
                1
            )
            .is_err()
        );
        assert!(
            super::pick_ranges(
                "engine
", &t, 1
            )
            .is_err()
        );
    }

    #[test]
    fn extra_words_go_last_or_not_at_all() {
        assert_eq!(super::with_extra("a b".into(), None), "a b");
        assert_eq!(super::with_extra("a b".into(), Some("  ")), "a b");
        assert_eq!(
            super::with_extra("a b".into(), Some(" +r_norefresh 1 ")),
            "a b +r_norefresh 1"
        );
    }

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
        assert_eq!(after("-rpclock"), Some("29"));
        assert_eq!(after("+rp_defaults"), Some(LAYOUT.to_string().as_str()));
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
        assert!(args_fit(&args).is_ok());
    }

    #[test]
    fn a_change_to_the_binds_raises_the_layout() {
        // CRC-32 (IEEE), as zlib's: the binds' sum when LAYOUT was raised
        let mut c = !0u32;
        for b in BINDS.bytes() {
            c ^= u32::from(b);
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    c >> 1 ^ 0xEDB8_8320
                } else {
                    c >> 1
                };
            }
        }
        assert_eq!(
            !c, LAYOUT_OF,
            "BINDS changed: raise LAYOUT (the saved config's layout keeps the old binds otherwise) and set LAYOUT_OF to 0x{:08X}",
            !c
        );
    }

    #[test]
    fn a_line_past_the_runners_31_words_is_refused_at_build() {
        let line = with_extra(
            base_for_test(),
            Some("-a -b -c -d -e -f -g -h -i -j -k -l -m -n -o -p"),
        );
        assert!(args_fit(&line).is_err());
        let long = format!("+map {}", "x".repeat(520));
        assert!(args_fit(&long).is_err());
        assert!(args_fit("+map c1a0 -rpfps").is_ok());
    }

    fn base_for_test() -> String {
        "+map c2a5f -rpfps -rpend 900 -rplimit 4500 -rpendshot -rpsample -rpfloat -rpbase /rp-xash/game -rodir /Ports -rpclock 29 3 0 1 -console +cl_nopred 0 +cl_lw 1 +rp_defaults 3".to_string()
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
                "rp-xash/game/valve/rp_binds.cfg",
                "rp-xash/game/valve/retropane/background.tga"
            ]
        );
        assert!(names.iter().all(|n| n.starts_with("rp-xash/")));
        assert_eq!(card[2].1, args().into_bytes());
        // A game with no background in pieces: the kit has none.
        assert_eq!(card_files(&modules, None).len(), 4);
        // Every button of the owner's layout is bound (in_rp.c's keys),
        // and every bind names a key the engine has (runs 35-38: `AUX1`
        // named none).
        for key in KEY_NAMES {
            assert!(
                BINDS
                    .lines()
                    .any(|l| l.starts_with(&format!("bind {key} "))),
                "{key}"
            );
        }
        for line in BINDS.lines().filter(|l| l.starts_with("bind ")) {
            let key = line.split_whitespace().nth(1).unwrap_or("");
            assert!(KEY_NAMES.contains(&key), "{line}");
        }
        // A tap never fires (the owner, 2026-11-16): only the D-pad's centre.
        assert!(BINDS.lines().any(|l| l == "unbind MOUSE1"));
        assert!(!BINDS.lines().any(|l| l.starts_with("bind MOUSE1")));
        // The stylus turns half a circle across the long side (run 46).
        assert!(BINDS.lines().any(|l| l == "sensitivity 12.8"));
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

    /// R338's class guard: every per-bone bit test our client patch adds
    /// (a 32-bit mask shifted by the bone's number) is bounded to bones
    /// 0-31 on the same line. Unbounded, a shift by 32 or more is no bit
    /// (ARM gives 0), and bone 32 on of every larger model was skipped:
    /// the snark, grenade and satchel hands (75 bones) drawn stretched.
    #[test]
    fn the_client_never_shifts_a_bone_mask_past_its_width() {
        let patch = include_str!("../../ports/xash/patches/hlsdk-portable.patch");
        let mut tests = 0;
        for line in patch.lines().filter(|l| l.starts_with('+')) {
            for var in ["i", "k"] {
                if line.contains(&format!(">> {var} & 1")) {
                    tests += 1;
                    assert!(
                        line.contains(&format!("{var} < 32")) || line.contains("numbones <= 32"),
                        "a bone mask shifted by {var} with no bound to 32 (R338): {line}"
                    );
                }
            }
        }
        assert!(tests > 0, "the guard found no bone mask tests to check");
    }

    /// R352's class guard: whatever our client patch keeps for a model by
    /// its slot's address (kit 81's bone matrices, kit 110's needed-bones
    /// list) is also checked against the model's name on the same test.
    /// The engine gives a freed slot to the next model loaded, so the
    /// address alone handed one model another's bones (parts drawn
    /// stretched to where they were).
    #[test]
    fn what_the_client_keeps_by_a_model_s_slot_is_checked_by_its_name_too() {
        let patch = include_str!("../../ports/xash/patches/hlsdk-portable.patch");
        let lines: Vec<&str> = patch.lines().filter(|l| l.starts_with('+')).collect();
        let mut tests = 0;
        for (i, line) in lines.iter().enumerate() {
            if line.contains("mdl == m_pRenderModel") || line.contains("mdl != m_pRenderModel") {
                tests += 1;
                // the test's own statement: this line and the two after
                let whole = lines[i..(i + 3).min(lines.len())].join(" ");
                assert!(
                    whole.contains("strncmp(") && whole.contains("m_pRenderModel->name"),
                    "a model's slot compared with no look at its name (R352): {line}"
                );
            }
        }
        assert!(
            tests >= 2,
            "the guard found {tests} slot tests, not kit 81's and kit 110's"
        );
    }

    /// R356's guard: the map loader clears every edge's kept place in the
    /// software renderer's edge list (`cachededgeoffset`: the 16-bit edges
    /// hold it; the 32-bit ones of the larger map format have no such
    /// field). Upstream leaves it as the allocation found it; the software
    /// pass's first frame followed an odd value into an unaligned load
    /// (run 315's fault at the end shot).
    #[test]
    fn the_map_loader_clears_every_edge_s_kept_place() {
        let patch = include_str!("../../ports/xash/patches/xash3d-fwgs.patch");
        let cleared = patch
            .lines()
            .filter(|l| l.starts_with('+') && l.contains("out->cachededgeoffset = 0;"))
            .count();
        assert_eq!(
            cleared, 1,
            "the 16-bit edges' kept places cleared at the load"
        );
    }

    /// R358's guard: the integer hull trace decides only sides, outside
    /// kit 109's 1/32-unit margin, and keeps the float code's own
    /// operations for every value a trace returns; `-rphullcheck` (the
    /// emulator's proof, every trace and point both ways) and
    /// `-rphullfloat` (the old way) stay in.
    #[test]
    fn the_integer_hull_trace_keeps_its_margin_and_its_check() {
        let patch = include_str!("../../ports/xash/patches/xash3d-fwgs.patch");
        let added: String = patch
            .lines()
            .filter(|l| l.starts_with('+'))
            .collect::<Vec<_>>()
            .join("\n");
        for needed in [
            "\"-rphullcheck\"",
            "\"-rphullfloat\"",
            "RP_HullCheckBoth(",
            "RP_PointCheckBoth(",
            "RP_PlaneGet(",
            "qmid = rp_int && RP_TracePoint( mid, qm ) ? qm : NULL;",
            "RP_BoxCheckBoth(",
            "if( v >= RP_TRACE_MARGIN )",
            "if( sum > ( 1u << 31 ))",
        ] {
            assert!(added.contains(needed), "R358's trace lost {needed}");
        }
        // the crossing's own values still the float code's
        assert!(
            patch.contains("frac = ( t1 + DIST_EPSILON ) / ( t1 - t2 );")
                && patch.contains("frac = ( t1 - DIST_EPSILON ) / ( t1 - t2 );"),
            "DIST_EPSILON's crossing must stay as upstream wrote it"
        );
    }

    /// R352's guard (after run 330): a face's lightmap or lit surface
    /// written again at its own place is held to the frame's end and
    /// copied in after the render before has ended, never written while a
    /// frame in flight may read it.
    #[test]
    fn rewrites_in_place_wait_for_the_render_before() {
        let src = include_str!("../../ports/xash/ref_rp/r_emit.c");
        assert_eq!(
            src.matches("rp_upload_stage = placed;").count(),
            2,
            "RP_LmPlace and RP_SurfacePlace both hold their rewrites in place"
        );
        let flush = src
            .find("static void RP_StageFlush( void )")
            .expect("RP_StageFlush");
        let body = &src[flush..flush + 400];
        assert!(
            body.find("rp_gfx_wait( );") < body.find("memcpy("),
            "the held squares copied in only after the render before has ended"
        );
        let submit = src.find("void RP_EmitSubmit( void )").expect("RP_EmitSubmit");
        let parts = src[submit..].find("rp_gfx_frame_parts(").expect("the submit");
        assert!(
            src[submit..submit + parts].contains("RP_StageFlush( );"),
            "the held squares in before the frame that draws them"
        );
    }

    /// R352's guard (after run 331): every world sky face is drawn at its
    /// own depth (it hides what the map keeps behind the sky, as GoldSrc),
    /// its corners carrying their own depth word apart from the texture's
    /// perspective word (the box's side distance over the depth).
    #[test]
    fn sky_faces_hide_what_is_behind_them() {
        let src = include_str!("../../ports/xash/ref_rp/r_emit.c");
        let at = src.find("if( fa->flags & SURF_DRAWSKY )").expect("the sky branch");
        assert!(
            src[at..at + 400].contains("RP_SkyFace( fa );"),
            "sky faces drawn from RP_ChipFace's sky branch"
        );
        let f = src.find("static void RP_SkyFace( msurface_t *fa )").expect("RP_SkyFace");
        let body = &src[f..f + 4000];
        assert!(
            body.contains("RP_Put( RP_Bits( zi ));") && body.contains("RP_Put( RP_Bits( dist * zi ));"),
            "the depth word and the texture's perspective word kept apart"
        );
    }

    /// R352's guard (after run 333): a still brush entity's face in a world
    /// face's plane and side (c2a5f's steel ribs against the wall's top
    /// band) is drawn just behind it, so the world wins the tie every frame
    /// as ref_soft draws it, and the faces are found once a map.
    #[test]
    fn brush_faces_in_a_world_plane_lose_the_tie() {
        let src = include_str!("../../ports/xash/ref_rp/r_emit.c");
        let tables = src.find("static int RP_WorldTables( void )").expect("RP_WorldTables");
        let cook = src[tables..].find("RP_CookRead( mod )").expect("the cooked path");
        assert!(
            src[tables..tables + cook].contains("RP_CoplanarMark( mod );"),
            "the ties marked before the cooked map's early return"
        );
        let face = src.find("static int RP_ChipFaceFixed(").expect("RP_ChipFaceFixed");
        let lm = src[face..].find("if( rp_lm_on && RP_ChipFaceLm(").expect("option B");
        let body = &src[face..face + lm];
        assert!(
            body.contains("rp_bview.still") && body.contains("zis[i] -= RP_COPLANAR_BIAS;"),
            "a marked face of an unmoved entity pushed back before either path draws it"
        );
        assert!(
            body.find("nearzi = zis[i];") < body.find("zis[i] -= RP_COPLANAR_BIAS;"),
            "the mip level taken from the face's own depth"
        );
    }

    /// R351's guard: the menus show no control that cannot act on the
    /// X51v (multiplayer, previews, Xash's touch editor, the gamepad, the
    /// update page, video modes, the GL renderer's boxes, vibration, raw
    /// input), each left out under `XASH_RETROPANE` and not deleted.
    #[test]
    fn the_menus_show_no_control_that_cannot_act() {
        let patch = include_str!("../../ports/xash/patches/mainui.patch");
        // the patched files' lines (context and added), not the removed ones
        let lines: Vec<&str> = patch.lines().filter(|l| !l.starts_with('-')).collect();
        for item in [
            "AddItem( multiPlayer );",
            "AddItem( previews );",
            "AddItem( vbo );",
            "AddItem( filtering );",
            "AddItem( hudscale );",
            "AddItem( vibration );",
            "AddItem( muteFocusLost );",
            "AddItem( rawinput );",
            "AddItem( inputDev );",
            "UI_VidModes_Menu",
            "UI_Touch_Menu",
            "UI_GamePad_Menu",
        ] {
            let at = lines
                .iter()
                .position(|l| l.contains(item))
                .unwrap_or_else(|| panic!("{item} not in the patch's context"));
            let before = lines[at.saturating_sub(8)..at].join(" ");
            assert!(
                before.contains("#if !XASH_RETROPANE"),
                "{item} shown on the X51v (R351)"
            );
        }
    }
}
