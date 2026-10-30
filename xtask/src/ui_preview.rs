//! `cargo xtask ui-preview`: the shell's screens drawn on the PC through the
//! same shell, set-up and 2D renderer code as the image (the model of the
//! 2D engine that `emu-today` holds the emulator to, pixel for pixel), saved
//! as PNGs with an index page. A UI change is seen in seconds, with no
//! emulator and no device run; the device still proves it (the
//! proven-on-hardware rule). A developer tool: nothing here ships.

use std::path::Path;

use rp_shell::{Input, Key};

/// One preview: a file name, what it shows, and the inputs after the
/// previous one that lead to it.
struct Shot {
    name: &'static str,
    what: &'static str,
    steps: Vec<Input>,
}

/// The screens previewed: the Today screen, the D-pad's focus moving, the
/// Start menu opened by a tap, Home back, then the Weather app's pages.
fn shots() -> Vec<Shot> {
    let start = rp_shell::START;
    let (sx, sy) = (start.x + start.w / 2, start.y + start.h / 2);
    // The weather hero's middle (it opens the Weather app).
    let (hx, hy) = (240, 150);
    let tap = |x, y| vec![Input::PenDown { x, y }, Input::PenUp { x, y }];
    vec![
        Shot {
            name: "01-today",
            what: "The Today screen as it starts",
            steps: vec![],
        },
        Shot {
            name: "02-focus-down",
            what: "D-pad down: the focus moves",
            steps: vec![Input::Key(Key::Down)],
        },
        Shot {
            name: "03-focus-down-right",
            what: "D-pad down then right",
            steps: vec![Input::Key(Key::Down), Input::Key(Key::Right)],
        },
        Shot {
            name: "04-start-menu",
            what: "A tap on Start: the Start menu",
            steps: vec![
                Input::PenDown { x: sx, y: sy },
                Input::PenUp { x: sx, y: sy },
            ],
        },
        Shot {
            name: "05-home",
            what: "Home: back to Today",
            steps: vec![Input::Key(Key::Home)],
        },
        Shot {
            name: "06-weather-now",
            what: "A tap on the weather hero: the Weather app, Now",
            steps: tap(hx, hy),
        },
        Shot {
            name: "07-weather-hourly",
            what: "D-pad right: Hourly",
            steps: vec![Input::Key(Key::Right)],
        },
        Shot {
            name: "08-weather-hourly-scrolled",
            what: "Hourly, dragged up: later hours",
            steps: vec![
                Input::PenDown { x: 240, y: 420 },
                Input::PenUp { x: 240, y: 150 },
            ],
        },
        Shot {
            name: "09-weather-10day",
            what: "A swipe left: 10 Day",
            steps: vec![
                Input::PenDown { x: 400, y: 300 },
                Input::PenUp { x: 260, y: 300 },
            ],
        },
        Shot {
            name: "10-weather-menu",
            what: "The Start menu over the Weather app",
            steps: tap(sx, sy),
        },
        Shot {
            name: "11-weather-settings",
            what: "Start closed, the right softkey: Weather settings",
            steps: [tap(sx, sy), tap(400, 620)].concat(),
        },
        Shot {
            name: "12-weather-metric",
            what: "Units tapped: Metric, every number with it",
            steps: tap(240, 104 + 58 + 26),
        },
        Shot {
            name: "13-weather-places",
            what: "Place: the list, the current one marked",
            steps: tap(240, 104 + 2 * 58 + 26),
        },
        Shot {
            name: "14-weather-key",
            what: "Oslo picked; then the Tempest key, typed (masked, last four shown)",
            steps: [
                tap(240, 104 + 6 * 58 + 26),
                tap(240, 104 + 4 * 58 + 26),
                // "ab12": keys a, b, 1, 2 on the on-screen keyboard.
                tap(24, 330 + 2 * 54 + 26),
                tap(5 * 48 - 24, 330 + 3 * 54 + 26),
                tap(24, 330 + 26),
                tap(72, 330 + 26),
                tap(312, 330 + 3 * 54 + 26),
                tap(360, 330 + 3 * 54 + 26),
            ]
            .concat(),
        },
        Shot {
            name: "15-weather-key-kept",
            what: "Done: the key kept, shown masked in the settings",
            steps: tap(400, 620),
        },
        Shot {
            name: "16-weather-back",
            what: "Back (left softkey): the app again, Metric and Oslo",
            steps: tap(80, 620),
        },
        Shot {
            name: "17-weather-radar",
            what: "Radar: the sample's rain over our Night map, the loop at its start",
            steps: tap(420, 578),
        },
        Shot {
            name: "18-weather-radar-now",
            what: "The timeline tapped at now: paused there",
            steps: tap(96 + 176, 512),
        },
    ]
}

/// Builds the images (for rp-today's wallpaper), draws every [`shots`]
/// screen and writes `target/ui-preview/` (PNGs and `index.html`).
pub fn run(root: &Path) -> Result<(), String> {
    let release = crate::arm_release(root);
    let wall = crate::emu::today_wallpaper(&release)?;
    let map = crate::emu::today_map(&release);
    let shots = shots();
    let steps: Vec<Input> = shots.iter().flat_map(|s| s.steps.iter().copied()).collect();
    let all = crate::emu::today_screens(&wall, map.as_deref(), &steps, true);
    let out = root.join("target").join("ui-preview");
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    // Screen k of `all` is after k inputs; each shot shows the screen after
    // its own inputs.
    let mut at = 0;
    let mut page = String::from(
        "<!doctype html><meta charset=utf-8><title>RetroPane UI preview</title>\
         <style>body{background:#111;color:#ddd;font:14px sans-serif;margin:16px}\
         figure{display:inline-block;margin:8px}img{width:240px;image-rendering:auto;border:1px solid #444}\
         img:hover{width:480px}</style><h1>RetroPane UI preview</h1>\
         <p>Drawn on the PC by the shell and 2D renderer code the X51v runs (the model emu-today checks). \
         Hover a screen for full size (480 x 640).</p>\n",
    );
    for s in &shots {
        at += s.steps.len();
        let rgb = all.get(at).ok_or("ui-preview: a screen missing")?;
        let file = format!("{}.png", s.name);
        std::fs::write(out.join(&file), png_rgb(480, 640, rgb))
            .map_err(|e| format!("{file}: {e}"))?;
        page.push_str(&format!(
            "<figure><img src=\"{file}\" alt=\"{w}\"><figcaption>{w}</figcaption></figure>\n",
            w = s.what
        ));
    }
    // The movement: each entrance as frames every 30 ms (motion-<name>-NN.png),
    // which a GIF tool can join; the device draws the same frames.
    let hero = vec![
        Input::PenDown { x: 240, y: 150 },
        Input::PenUp { x: 240, y: 150 },
    ];
    let swipe = vec![
        Input::PenDown { x: 400, y: 300 },
        Input::PenUp { x: 260, y: 300 },
    ];
    for (name, steps) in [
        ("open", hero.clone()),
        (
            "hourly",
            [hero.clone(), vec![Input::Key(Key::Right)]].concat(),
        ),
        (
            "10day",
            [hero, vec![Input::Key(Key::Right)], swipe].concat(),
        ),
    ] {
        let frames = crate::emu::today_motion(&wall, map.as_deref(), &steps, 30, 20);
        for (k, rgb) in frames.iter().enumerate() {
            let file = format!("motion-{name}-{k:02}.png");
            std::fs::write(out.join(&file), png_rgb(480, 640, rgb))
                .map_err(|e| format!("{file}: {e}"))?;
        }
    }
    std::fs::write(out.join("index.html"), page).map_err(|e| format!("index.html: {e}"))?;
    eprintln!(
        "    ui-preview: {} screens in {}",
        shots.len(),
        out.display()
    );
    Ok(())
}

/// A PNG of `w` x `h` 8-bit RGB pixels, zlib-stored (no compression: a
/// preview, never shipped, needs no encoder dependency).
fn png_rgb(w: u32, h: u32, rgb: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((w as usize * 3 + 1) * h as usize);
    for row in rgb.chunks(w as usize * 3).take(h as usize) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65_535).collect();
    for (i, b) in blocks.iter().enumerate() {
        z.push(u8::from(i + 1 == blocks.len()));
        let n = b.len() as u16;
        z.extend_from_slice(&n.to_le_bytes());
        z.extend_from_slice(&(!n).to_le_bytes());
        z.extend_from_slice(b);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = w.to_be_bytes().to_vec();
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"IDAT", &z);
    chunk(&mut png, b"IEND", &[]);
    png
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    !c
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + u32::from(x)) % 65_521;
        b = (b + a) % 65_521;
    }
    b << 16 | a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_and_adler_match_their_published_check_values() {
        // The standard check values for "123456789".
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn a_png_has_its_signature_header_and_every_pixel() {
        let rgb: Vec<u8> = (0..2 * 3 * 3).map(|i| i as u8).collect();
        let p = png_rgb(2, 3, &rgb);
        assert_eq!(&p[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&p[12..16], b"IHDR");
        assert_eq!(u32::from_be_bytes(p[16..20].try_into().unwrap()), 2);
        assert_eq!(u32::from_be_bytes(p[20..24].try_into().unwrap()), 3);
        // Stored: every pixel byte is in the file, each row after its filter byte.
        let idat = p.windows(4).position(|w| w == b"IDAT").unwrap();
        let raw = &p[idat + 4 + 2 + 5..];
        assert_eq!(&raw[..7], &[0, 0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn the_shots_open_the_start_menu_and_come_home() {
        let s = shots();
        assert!(s.iter().any(|s| s.name.contains("start-menu")));
        assert!(s.iter().any(|s| s.steps == vec![Input::Key(Key::Home)]));
        assert!(s.iter().any(|s| s.name.contains("weather")));
    }
}
