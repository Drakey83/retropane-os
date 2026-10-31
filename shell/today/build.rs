//! Links the image with the shared `boot/link.x` when building for the X51v,
//! and supplies:
//!
//! - the wallpaper: `local/wallpapers/<name>/<name>.yuv`, `<name>` from
//!   `RP_TODAY_WALLPAPER` (default `sunset`; made by
//!   `tools/shell-assets/wallpaper.py` from the photo; `local/` is never
//!   committed) or, when that's absent, a generated dusk sky, so the image
//!   always builds. A test image holds one (950 KiB image area), so the
//!   16-bit comparison is one build per wallpaper;
//! - the weather pictures (`local/icons/icons.rs`, else empty);
//! - the clock's start: the build machine's local time (there is no RTC
//!   driver yet), as minutes since 1970 plus the UTC offset. The clock is
//!   right to within the time between building and running.
//! - the touch replay's pen samples (`RP_TODAY_REPLAY`, the ratchet's touch
//!   row, docs/design/perf-ratchet.md): the owner's taps of 2026-10-17
//!   (`drivers/touch-cal/tests/data/owner-2026-10-17-12k.txt`), packed as
//!   `rp_today::replay` reads them.

use std::path::Path;

include!("../../tools/build/local_input.rs");

const W: usize = 480;
const H: usize = 640;
const BYTES: usize = W * H * 3 / 2;

fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo");
    let out = std::env::var("OUT_DIR").expect("set by cargo");
    println!("cargo:rerun-if-env-changed=RP_TODAY_WALLPAPER");
    // Sunset by default: the owner wants colour in every build he tests
    // (2026-10-31; the pier photo is black and white).
    let name = std::env::var("RP_TODAY_WALLPAPER").unwrap_or_else(|_| "sunset".into());
    assert!(
        !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'),
        "RP_TODAY_WALLPAPER: a folder name under local/wallpapers"
    );
    let source = Path::new(&dir).join(format!("../../local/wallpapers/{name}/{name}.yuv"));
    watch_local(&source);
    let wall = match std::fs::read(&source) {
        Ok(p) if p.len() == BYTES => p,
        Ok(p) => panic!("{}: {} bytes, want {BYTES}", source.display(), p.len()),
        Err(_) => {
            // Asked for by name: say it isn't there (CI has none, quietly).
            if std::env::var("RP_TODAY_WALLPAPER").is_ok() {
                println!(
                    "cargo:warning=no {}: the generated sky instead",
                    source.display()
                );
            }
            sky()
        }
    };
    println!("cargo:rustc-env=RP_TODAY_WALLPAPER_NAME={name}");
    // Packed for the image (sheet today-image-room.md): each plane through
    // PNG's Paeth filter, then zlib; unpacked once at start into the RAM
    // pool (`rp_today::unpack_wallpaper`). The raw planes stay beside it
    // for the PC's checks.
    let packed = pack_wallpaper(&wall);
    println!(
        "cargo:warning=wallpaper ({name}) packed {} -> {} bytes",
        wall.len(),
        packed.len()
    );
    std::fs::write(Path::new(&out).join("wallpaper.zz"), packed).expect("OUT_DIR writable");
    std::fs::write(Path::new(&out).join("wallpaper.yuv"), wall).expect("OUT_DIR writable");

    // The radar's base map (the Night look; Ink is made from it on the
    // device): `local/maps/radar-night-480x462.rgb`, made by
    // `RADAR_DEVICE=1 tools/mockups/radar_basemaps.py` from Natural Earth
    // (`local/` is never committed), else a plain stand-in. Packed at the
    // screen's depth (5, 6, 5 bits a plane), Paeth, zlib.
    let map_src = Path::new(&dir).join("../../local/maps/radar-night-480x462.rgb");
    watch_local(&map_src);
    let map = match std::fs::read(&map_src) {
        Ok(m) if m.len() == W * MAP_H * 3 => m,
        _ => stand_in_map(),
    };
    let packed = pack_map(&map);
    println!(
        "cargo:warning=radar map packed {} -> {} bytes",
        map.len(),
        packed.len()
    );
    std::fs::write(Path::new(&out).join("radar.zz"), packed).expect("OUT_DIR writable");

    // The weather pictures: `local/icons/icons.rs` (made by
    // `tools/shell-assets/icons.py` from assets/weather/vclouds; `local/` is
    // never committed) or, when absent, empty pictures: the screen then
    // draws outlines where they go.
    let icons = Path::new(&dir).join("../../local/icons/icons.rs");
    watch_local(&icons);
    let data = std::fs::read_to_string(&icons).unwrap_or_else(|_| {
        "pub const COUNT: usize = 0;
pub const WORDS: usize = 0;
pub static PACKED: [(u8, u8, u16, u16, &[u32]); COUNT] = [];
"
        .to_string()
    });
    // The owner's default picture (`rp_draw::look`, 2026-10-22) on every
    // icon pixel (non-premultiplied ARGB: the colour changes, alpha not),
    // the same function the wallpaper goes through as it loads;
    // `RP_PICTURE_ORIGINAL` leaves them as made.
    println!("cargo:rerun-if-env-changed=RP_PICTURE_ORIGINAL");
    let data = if std::env::var("RP_PICTURE_ORIGINAL").is_ok() {
        data
    } else {
        looked(&data)
    };
    // Packed as the wallpaper is (Paeth, then zlib), not as runs: a 3D
    // picture has few runs (the hero's was 140 KB as runs, 2026-10-31).
    let (rs, packed) = pack_icons(&data);
    println!(
        "cargo:warning=pictures packed {} -> {} bytes",
        icon_words(&data) * 4,
        packed.len()
    );
    std::fs::write(Path::new(&out).join("icons.zz"), packed).expect("OUT_DIR writable");
    std::fs::write(Path::new(&out).join("icons.rs"), rs).expect("OUT_DIR writable");
    // The card's picture packs, every condition at every size (made by
    // `icons.py --all`): too big for the image area (1.6 MB packed each),
    // so `cargo xtask load` puts them on the SD card (`RPICONS<n>.PAK`) and
    // the image reads the one the settings name. Pack 0 is VClouds
    // (`local/icons/icons-all.rs`), packs 1-6 the owner's own, in
    // `rp_weather::settings::PACKS`' order (`icons-<name>.rs`, from his art
    // in local/, never committed).
    for (n, file) in [
        (0, "icons-all.rs"),
        (1, "icons-sunburst.rs"),
        (2, "icons-golden-hour.rs"),
        (3, "icons-frosted.rs"),
        (4, "icons-noir.rs"),
        (5, "icons-aurora.rs"),
        (6, "icons-chrome.rs"),
    ] {
        let src = Path::new(&dir).join("../../local/icons").join(file);
        watch_local(&src);
        if let Ok(t) = std::fs::read_to_string(&src) {
            let t = if std::env::var("RP_PICTURE_ORIGINAL").is_ok() {
                t
            } else {
                looked(&t)
            };
            let pak = pack_file(&t);
            println!(
                "cargo:warning=picture pack {n} ({file}): {} bytes",
                pak.len()
            );
            std::fs::write(Path::new(&out).join(format!("rp-icons{n}.pak")), pak)
                .expect("OUT_DIR writable");
        }
    }

    // Rebuilt each time, so the clock starts near the real time.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=RP_TODAY_FORCE");
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    println!("cargo:rustc-env=RP_TODAY_UTC_MIN={}", secs / 60);
    println!(
        "cargo:rustc-env=RP_TODAY_OFFSET_MIN={}",
        utc_offset_minutes()
    );

    // The replay's recording: the ratchet's (default), or another
    // (`RP_TODAY_REPLAY_FILE`, under the repo, replayed whole, with pauses up
    // to `RP_TODAY_REPLAY_GAP_MS`: the plane path's unattended device runs
    // replay the owner's Weather session, its openings finishing between
    // taps).
    println!("cargo:rerun-if-env-changed=RP_TODAY_REPLAY_FILE");
    println!("cargo:rerun-if-env-changed=RP_TODAY_REPLAY_GAP_MS");
    let (rec, end) = match std::env::var("RP_TODAY_REPLAY_FILE") {
        Ok(f) => (Path::new(&dir).join("../..").join(f), u32::MAX),
        Err(_) => (
            Path::new(&dir).join("../../drivers/touch-cal/tests/data/owner-2026-10-17-12k.txt"),
            REPLAY_END_MS,
        ),
    };
    let gap = std::env::var("RP_TODAY_REPLAY_GAP_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(REPLAY_GAP_MAX_MS);
    println!("cargo:rerun-if-changed={}", rec.display());
    let text = std::fs::read_to_string(&rec).expect("the touch replay's recording");
    std::fs::write(
        Path::new(&out).join("replay.bin"),
        pack_replay(&text, end, gap),
    )
    .expect("OUT_DIR writable");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("none") {
        println!("cargo:rustc-link-arg-bins=-T{dir}/../../boot/link.x");
        // The stack. It was 24, then 40, then 12: the image used 23.5 KiB (96% of
        // 24) and an older build froze the Axim (the watchdog error screen,
        // 2026-10-17). Of that, 22 KiB was `main`'s frame holding the
        // long-lived objects; they are in `rt::Slot`s in `.bss` now, where
        // the zeroed-RAM budget counts them. Every emulator image fails above
        // 80% stack use.
        // 20 KiB since 2026-10-31: the last run used 11,840 of 12,288 bytes
        // and reading an icon pack from the SD card (the FAT volume's two
        // sectors, a read buffer) adds ~3 KiB; the first build with it
        // overflowed at start.
        // 16 KiB since the same day: the X51v's deepest use with the card
        // read was 12,744 bytes (78%), and the image area needed the room.
        // 24 KiB since the same evening: the first run with the real radar
        // (today-0036) used all 16 KiB (the PNG reader's palette and the
        // inflater's tables, ~3.5 KiB, on top of the main loop's depth);
        // the radar's base map moved to the card, so the area has the room.
        println!("cargo:rustc-link-arg-bins=--defsym=__stack_size=24576");
    }
}

/// The build machine's offset from UTC in minutes (0 if it can't be read).
fn utc_offset_minutes() -> i64 {
    let run = |cmd: &str, args: &[&str]| {
        std::process::Command::new(cmd)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    // Windows: minutes directly. Elsewhere: `date +%z` gives +hhmm.
    if let Some(m) = run(
        "powershell",
        &[
            "-NoProfile",
            "-Command",
            "[DateTimeOffset]::Now.Offset.TotalMinutes",
        ],
    )
    .and_then(|s| s.parse::<f64>().ok())
    {
        return m as i64;
    }
    run("date", &["+%z"])
        .filter(|z| z.len() == 5)
        .and_then(|z| {
            let sign = if z.starts_with('-') { -1 } else { 1 };
            let h: i64 = z[1..3].parse().ok()?;
            let m: i64 = z[3..5].parse().ok()?;
            Some(sign * (h * 60 + m))
        })
        .unwrap_or(0)
}

/// The planes (Y, then U and V at half size) each Paeth-filtered, then
/// zlib: what `rp_today::unpack_wallpaper` undoes.
fn pack_wallpaper(wall: &[u8]) -> Vec<u8> {
    let (y, uv) = wall.split_at(W * H);
    let (u, v) = uv.split_at(W * H / 4);
    let mut f = rp_inflate::pack::paeth_filter(y, W, H);
    f.extend(rp_inflate::pack::paeth_filter(u, W / 2, H / 2));
    f.extend(rp_inflate::pack::paeth_filter(v, W / 2, H / 2));
    rp_inflate::pack::zlib(&f)
}

/// The radar map's rows (the Weather app's page).
const MAP_H: usize = 462;

/// The map's R, G and B planes at 5, 6 and 5 bits, each Paeth-filtered,
/// then zlib: what `rp_today::unpack_map` undoes.
fn pack_map(rgb: &[u8]) -> Vec<u8> {
    let mut f = Vec::new();
    for (c, shift) in [(0, 3), (1, 2), (2, 3)] {
        let plane: Vec<u8> = rgb.chunks(3).map(|p| p[c] >> shift).collect();
        f.extend(rp_inflate::pack::paeth_filter(&plane, W, MAP_H));
    }
    rp_inflate::pack::zlib(&f)
}

/// A plain map when the real one isn't built: water with a coast running
/// down the middle (CI, and anyone without the map data).
fn stand_in_map() -> Vec<u8> {
    let mut m = Vec::with_capacity(W * MAP_H * 3);
    for y in 0..MAP_H {
        for x in 0..W {
            let coast = 200 + (y * 7 % 60);
            let p = if x > coast { [34, 40, 46] } else { [6, 14, 24] };
            m.extend(p);
        }
    }
    m
}

/// A dusk sky as planar 4:2:0: deep blue at the top to warm low light.
fn sky() -> Vec<u8> {
    let mut y = Vec::with_capacity(W * H);
    for row in 0..H {
        let t = row * 255 / (H - 1);
        y.extend(std::iter::repeat_n((24 + t * 90 / 255) as u8, W));
    }
    let mut u = Vec::with_capacity(W * H / 4);
    let mut v = Vec::with_capacity(W * H / 4);
    for row in 0..H / 2 {
        let t = row * 255 / (H / 2 - 1);
        u.extend(std::iter::repeat_n((150 - t * 40 / 255) as u8, W / 2));
        v.extend(std::iter::repeat_n((118 + t * 30 / 255) as u8, W / 2));
    }
    [y, u, v].concat()
}

/// The pictures' words unpacked (the generator's `WORDS`).
fn icon_words(text: &str) -> usize {
    text.lines()
        .find_map(|l| l.strip_prefix("pub const WORDS: usize = "))
        .and_then(|v| v.trim_end_matches(';').trim().parse().ok())
        .unwrap_or(0)
}

/// The generator's run-length pictures (`P<n>` arrays of words: a head
/// `skip << 16 | count`, then `count` ARGB pixels) and their list, as one
/// zlib stream of every picture in order, each pixel's bytes B, G, R, A
/// (the ARGB word in memory on the X51v) Paeth-filtered per channel
/// against its neighbours (PNG's filter at 4 bytes a pixel); and the
/// module `rp_today::icon_data` includes (the list, `WORDS`, the stream).
fn pack_icons(text: &str) -> (String, Vec<u8>) {
    let (desc, words, packed) = pack_set(text);
    let d: Vec<String> = desc
        .iter()
        .map(|(k, i, w, h)| format!("({k}, {i}, {w}, {h})"))
        .collect();
    let rs = format!(
        "pub const COUNT: usize = {n};
pub const WORDS: usize = {words};
pub static DESC: [(u8, u8, u16, u16); COUNT] = [{d}];
pub static PACKED: &[u8] = include_bytes!(concat!(env!(\"OUT_DIR\"), \"/icons.zz\"));
",
        n = desc.len(),
        d = d.join(", ")
    );
    (rs, packed)
}

/// The SD card's pack (`rp_today::PackLoad`'s format): `RPIK`, version
/// 2, the count, two zero bytes, the words, the table (kind, id, width,
/// height, stream length), then each picture's own stream.
fn pack_file(text: &str) -> Vec<u8> {
    let (desc, words, streams) = pack_pictures(text);
    let mut f = b"RPIK".to_vec();
    f.extend([2, desc.len() as u8, 0, 0]);
    f.extend((words as u32).to_le_bytes());
    for ((k, i, w, h), s) in desc.iter().zip(&streams) {
        f.extend([*k, *i]);
        f.extend(w.to_le_bytes());
        f.extend(h.to_le_bytes());
        f.extend((s.len() as u32).to_le_bytes());
    }
    for s in streams {
        f.extend(s);
    }
    f
}

/// The pictures' list, their words and each one's zlib stream.
fn pack_pictures(text: &str) -> (Vec<Entry>, usize, Vec<Vec<u8>>) {
    let (desc, filtered) = filtered_set(text);
    let words = filtered.iter().map(Vec::len).sum::<usize>() / 4;
    let streams = filtered.iter().map(|b| rp_inflate::pack::zlib(b)).collect();
    (desc, words, streams)
}

/// A picture in a pack's list: its id, size, width and height.
type Entry = (u8, u8, u16, u16);

/// The pictures' list, their words and the zlib stream.
fn pack_set(text: &str) -> (Vec<Entry>, usize, Vec<u8>) {
    let (desc, filtered) = filtered_set(text);
    let stream: Vec<u8> = filtered.concat();
    let packed = if stream.is_empty() {
        Vec::new()
    } else {
        rp_inflate::pack::zlib(&stream)
    };
    (desc, stream.len() / 4, packed)
}

/// The pictures' list and each one's Paeth-filtered bytes.
fn filtered_set(text: &str) -> (Vec<Entry>, Vec<Vec<u8>>) {
    let ints = |s: &str| -> Vec<u32> {
        s.split(',')
            .filter_map(|v| {
                let v = v.trim();
                if let Some(h) = v.strip_prefix("0x") {
                    u32::from_str_radix(h, 16).ok()
                } else {
                    v.parse().ok()
                }
            })
            .collect()
    };
    let arrays: std::collections::HashMap<String, Vec<u32>> = text
        .lines()
        .filter_map(|l| {
            let l = l.strip_prefix("static P")?;
            let (name, rest) = l.split_once(':')?;
            let body = rest.split_once('[')?.1.split_once('[')?.1;
            Some((format!("P{name}"), ints(body.trim_end_matches("];"))))
        })
        .collect();
    let list = text
        .lines()
        .find_map(|l| l.strip_prefix("pub static PACKED: [(u8, u8, u16, u16, &[u32]); COUNT] = ["))
        .unwrap_or("];");
    let mut desc = Vec::new();
    let mut filtered = Vec::new();
    for t in list
        .trim_end_matches("];")
        .split("),")
        .filter(|t| t.contains('&'))
    {
        let f: Vec<&str> = t
            .trim_start_matches([' ', '('])
            .split(',')
            .map(str::trim)
            .collect();
        let (kind, id, w, h) = (
            f[0],
            f[1],
            f[2].parse::<usize>().unwrap(),
            f[3].parse::<usize>().unwrap(),
        );
        let runs = &arrays[f[4].trim_start_matches('&').trim_end_matches(')')];
        let mut px = vec![0u32; w * h];
        let (mut at, mut i) = (0usize, 0usize);
        while let Some(&head) = runs.get(i) {
            i += 1;
            at += (head >> 16) as usize;
            for _ in 0..(head & 0xFFFF) {
                px[at] = runs[i];
                i += 1;
                at += 1;
            }
        }
        let bytes: Vec<u8> = px.iter().flat_map(|p| p.to_le_bytes()).collect();
        filtered.push(paeth4(&bytes, w, h));
        desc.push((
            kind.parse().unwrap(),
            id.parse().unwrap(),
            w as u16,
            h as u16,
        ));
    }
    (desc, filtered)
}

/// PNG's Paeth filter at 4 bytes a pixel (each channel against the same
/// channel left, up and up-left): what `rp_today::unpaeth4` undoes.
fn paeth4(px: &[u8], w: usize, h: usize) -> Vec<u8> {
    let stride = w * 4;
    let at = |i: isize| if i < 0 { 0 } else { px[i as usize] };
    let mut f = vec![0u8; px.len()];
    for y in 0..h {
        for x in 0..stride {
            let i = (y * stride + x) as isize;
            let a = if x >= 4 { at(i - 4) } else { 0 };
            let b = if y > 0 { at(i - stride as isize) } else { 0 };
            let c = if x >= 4 && y > 0 {
                at(i - stride as isize - 4)
            } else {
                0
            };
            f[i as usize] = px[i as usize].wrapping_sub(rp_inflate::paeth(a, b, c));
        }
    }
    f
}

/// `text` with every `0xAARRGGBB` pixel (the generator's only hex: ten
/// characters) through `rp_draw::look::apply`, alpha kept.
fn looked(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("0x") {
        out.push_str(&rest[..i]);
        let hex = rest
            .get(i + 2..i + 10)
            .filter(|h| h.bytes().all(|b| b.is_ascii_hexdigit()));
        match hex.and_then(|h| u32::from_str_radix(h, 16).ok()) {
            Some(px) => {
                let [a, r, g, b] = px.to_be_bytes();
                let (r, g, b) = rp_draw::look::apply(r, g, b);
                out.push_str(&format!("0x{:08x}", u32::from_be_bytes([a, r, g, b])));
                rest = &rest[i + 10..];
            }
            None => {
                out.push_str("0x");
                rest = &rest[i + 2..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The replay's longest wait between two touches: the recording's pauses
/// (seconds, the owner deciding where to tap next) shortened so the run
/// is short; long enough for every answer's frame to be on the screen.
const REPLAY_GAP_MAX_MS: u32 = 500;

/// A gap between two recorded samples longer than this is the pen up: the
/// recorder logs only while a touch is in progress, and within one its
/// samples are at most 11 ms apart (2026-10-17: 7 ms typical), between two
/// at least 352 ms. A touch can end without a no-pressure line (the filter
/// lifts on a near-zero pressure sample, and the recorder stops there).
const REPLAY_LIFT_MS: u32 = 100;

/// Where the replay stops: the recording's last touch (at 73,534 ms) is
/// the owner ending the run from the developer menu; that menu has since
/// gained Align screen where the tap lands, so it isn't replayed (the
/// replay ends the run itself). `rp_today::replay`'s scenario test says
/// when a shell change moves another tap off the Today screen.
const REPLAY_END_MS: u32 = 73_534;

/// The recorder's `R <ms> <x> <y> <z1> ...` lines (`-` for no pressure) as
/// 8 bytes a sample, little-endian: the wait before it in ms (non-zero only
/// on a touch's first sample: the pause since the last one, at most
/// [`REPLAY_GAP_MAX_MS`]; within a touch the image's own loop sets the
/// pace), then x, y and z1 (z1 0: no pressure). Where the pen lifted with
/// no line saying so, a no-pressure sample is added, as the image's loop
/// would have read one; and at the end.
fn pack_replay(text: &str, end_ms: u32, gap_max_ms: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let mut push = |wait: u32, x: u16, y: u16, z1: u16| {
        for v in [wait as u16, x, y, z1] {
            out.extend_from_slice(&v.to_le_bytes());
        }
    };
    let mut prev: Option<(u32, bool)> = None; // (ms, pressure)
    for line in text.lines().filter(|l| l.starts_with("R ")) {
        let f: Vec<&str> = line.split_whitespace().collect();
        let ms: u32 = f[1].parse().expect("a recorder line's time");
        if ms >= end_ms {
            break;
        }
        let (x, y, z1) = if f[2] == "-" {
            (0, 0, 0)
        } else {
            let n = |i: usize| f[i].parse::<u16>().expect("a recorder line's value");
            assert!(n(4) != 0, "a sample with pressure has z1 > 0");
            (n(2), n(3), n(4))
        };
        let wait = match prev {
            None => gap_max_ms,
            Some((at, pressed)) if ms - at > REPLAY_LIFT_MS => {
                if pressed {
                    push(0, 0, 0, 0);
                }
                (ms - at).min(gap_max_ms)
            }
            Some(_) => 0,
        };
        push(wait, x, y, z1);
        prev = Some((ms, z1 != 0));
    }
    if prev.is_some_and(|(_, pressed)| pressed) {
        push(0, 0, 0, 0);
    }
    out
}
