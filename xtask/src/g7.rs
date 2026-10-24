//! `cargo xtask g7-frame`: Dell's divide-1 frame taken out of `rp-wmprobe`
//! v4's capture of the 2700G's local memory, for `rp-glesfeat` test G7 to
//! replay word for word (docs/hw/2700g.md "Test G7"). The frame stays
//! local (docs/CLEANROOM.md: captures never committed); the image embeds it
//! at build time (`RP_GLESFEAT_G7_FRAME`).
//!
//! What is taken: the MBX registers of the probe's run-2 state as Dell left
//! them (`frame::NEVER` aside), and the 4 KiB pages of local memory his
//! render reads: the region array (`0x608`), every object list it points
//! to, and every object those lists point to (from `0x60C`), each object's
//! page and the next (an object may cross a page); then the hash of his own
//! picture (`0x65C`, his `0x650` format, `0x654`/`0x658` size), which G7's
//! replay at Windows Mobile's clock must equal.

use std::collections::BTreeSet;
use std::path::Path;

// The image reads the parts xtask doesn't (parse's accessors).
#[allow(dead_code)]
#[path = "../../boot/glesfeat/src/frame.rs"]
pub mod frame;

/// The MBX's end-of-list word and the address mask (`rp_drv_mbx3d`).
use rp_drv_mbx3d::{ADDR_MASK, END, LAST_REGION};

const PAGE: usize = 4096;

/// Run 2's state lines from the probe's log: `(block, offset, value)`.
pub fn state(log: &str) -> Result<Vec<(String, usize, u32)>, String> {
    let from = log
        .rfind("run 2 after: state follows")
        .ok_or("the log has no run 2 state (rp-wmprobe v3 or later, run 2)")?;
    let mut out = Vec::new();
    for l in log[from..].lines().skip(1) {
        if l.ends_with("state end") {
            return Ok(out);
        }
        let rest = l
            .strip_prefix("run 2 after: ")
            .ok_or(format!("not a state line: {l}"))?;
        let w: Vec<&str> = rest.split_whitespace().collect();
        let hex = |s: &str| {
            u32::from_str_radix(
                s.trim_matches(|c| c == '(' || c == ')')
                    .trim_start_matches("0x"),
                16,
            )
            .map_err(|_| format!("not hex in: {l}"))
        };
        out.push((
            rest[..3].to_string(),
            hex(w[w.len() - 2])? as usize,
            hex(w[w.len() - 1])?,
        ));
    }
    Err("run 2's state never ended".into())
}

/// The frame file from a 16 MiB capture and run 2's state.
pub fn extract(vram: &[u8], state: &[(String, usize, u32)]) -> Result<Vec<u8>, String> {
    if vram.len() != frame::LOCAL {
        return Err(format!("the capture is {} bytes, not 16 MiB", vram.len()));
    }
    let rd = |a: usize| -> Result<u32, String> {
        vram.get(a..a + 4)
            .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
            .ok_or(format!("0x{a:x} outside local memory"))
    };
    let regs: Vec<(usize, u32)> = state
        .iter()
        .filter(|(b, off, _)| {
            b == "MBX"
                && !frame::NEVER.contains(off)
                && (*off == 0x130 || (0x600..=0x6FC).contains(off) || (0x820..=0x87C).contains(off))
        })
        .map(|(_, o, v)| (*o, *v))
        .collect();
    let reg = |off: usize| {
        regs.iter()
            .find(|p| p.0 == off)
            .map(|p| p.1)
            .ok_or(format!("run 2's state has no MBX 0x{off:x}"))
    };
    let (r, p, t) = (
        (reg(0x608)? & ADDR_MASK) as usize,
        (reg(0x60C)? & 0x01FF_F000) as usize,
        (reg(0x65C)? & ADDR_MASK) as usize,
    );
    let mut pages = BTreeSet::new();
    let page = |a: usize, pages: &mut BTreeSet<usize>| {
        pages.insert(a / PAGE * PAGE);
    };
    // The region array: 8-byte entries to the last (bit 31 of its first
    // word), every list pointer followed.
    let mut lists = BTreeSet::new();
    let mut k = 0;
    loop {
        let (w0, w1) = (rd(r + 8 * k)?, rd(r + 8 * k + 4)?);
        page(r + 8 * k, &mut pages);
        page(r + 8 * k + 4, &mut pages);
        let list = (w1 & ADDR_MASK) as usize;
        if list != 0 {
            lists.insert(list);
        }
        k += 1;
        if w0 & LAST_REGION != 0 {
            break;
        }
        if k > 8192 {
            return Err("no last region within 8,192 entries".into());
        }
    }
    let mut objects = BTreeSet::new();
    for &l in &lists {
        for j in 0..256 {
            let a = l + 4 * j;
            page(a, &mut pages);
            let w = rd(a)?;
            if w == END {
                break;
            }
            objects.insert(p + 4 * (w & 0x001F_FFFF) as usize);
        }
    }
    for &o in &objects {
        page(o, &mut pages);
        if o + PAGE < frame::LOCAL {
            page(o + PAGE, &mut pages);
        }
    }
    // Pages run together become one segment.
    let mut segs: Vec<(usize, Vec<u32>)> = Vec::new();
    for &pg in &pages {
        let words: Vec<u32> = (0..PAGE / 4)
            .map(|i| rd(pg + 4 * i))
            .collect::<Result<_, _>>()?;
        match segs.last_mut() {
            Some((a, d)) if *a + 4 * d.len() == pg => d.extend(words),
            _ => segs.push((pg, words)),
        }
    }
    let (w, h) = ((reg(0x654)? >> 16) + 1, (reg(0x658)? >> 16) + 1);
    let bpp = if reg(0x650)? == 1 { 2 } else { 4 };
    let words = (w * h * bpp / 4) as usize;
    let picture = frame::hash((0..words).map(|i| rd(t + 4 * i).unwrap_or(0)));
    eprintln!(
        "    Dell's frame: region array 0x{r:x} ({k} entries), {} lists, {} objects from 0x{p:x}, {} pages in {} segments; his picture at 0x{t:x}, {w} x {h} at {bpp} bytes, hash 0x{picture:08x}",
        lists.len(),
        objects.len(),
        pages.len(),
        segs.len()
    );
    Ok(frame::write(&regs, &segs, (picture, words as u32)))
}

/// `cargo xtask g7-frame [capture] [log] [out]`.
pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let arg = |k: usize, d: &str| root.join(args.get(k).map_or(d, String::as_str));
    let (cap, log, out) = (
        arg(0, "local/rp-wmprobe-div1-vram.bin"),
        arg(1, "local/rp-wmprobe-7.txt"),
        arg(2, "local/g7-dell-frame.bin"),
    );
    let vram = std::fs::read(&cap).map_err(|e| format!("{}: {e}", cap.display()))?;
    let text = std::fs::read_to_string(&log).map_err(|e| format!("{}: {e}", log.display()))?;
    let b = extract(&vram, &state(&text)?)?;
    frame::parse(&b).map_err(|e| format!("the frame written doesn't parse: {e}"))?;
    std::fs::write(&out, &b).map_err(|e| format!("{}: {e}", out.display()))?;
    eprintln!("    {} ({} bytes)", out.display(), b.len());
    Ok(())
}

/// A frame of our own scene (GC2's quad over our flat background, our
/// places, as `rp-glesfeat`'s way "ours" draws it), for the emulator's
/// check of G7's replay: its replay at Windows Mobile's clock must draw
/// the very picture way 1 draws directly. No picture hash (0).
pub fn synthetic() -> Vec<u8> {
    use rp_drv_mbx3d::{self as m3, ISP_ALWAYS, TSP_DELL, Vertex};
    // `rp-glesfeat`'s QUAD, TRIS and TSP_SMOOTH (src/main.rs).
    const QUAD: [(f32, f32, u32); 4] = [
        (0.0, 0.0, 0xFF00_0000),
        (480.0, 0.0, 0xFFFF_FFFF),
        (480.0, 640.0, 0xFF10_20A0),
        (0.0, 640.0, 0xFFFF_E000),
    ];
    const TRIS: [[usize; 3]; 2] = [[0, 1, 3], [1, 2, 3]];
    const TSP_SMOOTH: u32 = 0x2260_0E80;
    let (target, pm, table, regions) = (
        0x0020_0000u32,
        0x0060_0000u32,
        0x0070_0000u32,
        0x00D1_0000u32,
    );
    let bg = m3::triangle(
        ISP_ALWAYS,
        TSP_DELL,
        &[(0.0, 0.0), (2048.0, 0.0), (0.0, 2048.0)].map(|(x, y)| Vertex {
            x,
            y,
            z: 0.0,
            rhw: 1.0,
            argb: 0xFF00_0000,
        }),
    );
    let tri = |t: [usize; 3]| {
        m3::triangle(
            ISP_ALWAYS,
            TSP_SMOOTH,
            &t.map(|k| Vertex {
                x: QUAD[k].0,
                y: QUAD[k].1,
                z: 0.0,
                rhw: 1.0,
                argb: QUAD[k].2,
            }),
        )
    };
    let objs = [bg, tri(TRIS[0]), tri(TRIS[1])];
    // The list at the parameter memory's start, the objects after 64 bytes.
    let mut params = vec![m3::END; 16];
    let mut words: Vec<u32> = Vec::new();
    for (k, o) in objs.iter().enumerate() {
        params[k] = m3::STRIP_ONE_TRIANGLE | ((64 / 4 + words.len()) as u32);
        words.extend_from_slice(o);
    }
    params.extend(words);
    let (cols, rows) = (480 / m3::TILE_W, 640 / m3::TILE_H);
    let mut ra = vec![0u32; 2 * (cols * rows) as usize];
    let n = m3::region_array(&mut ra, cols, rows, pm).expect("the grid fits");
    ra.truncate(n);
    let mut regs: Vec<(usize, u32)> = m3::start_table(regions, pm, 480, 640, 480 * 4)
        .into_iter()
        .filter(|p| p.0 != m3::START)
        .collect();
    regs.push((0x65C, target));
    regs.extend(
        m3::pm_init_table(pm, 0x0010_0000, table)
            .into_iter()
            .filter(|p| p.0 != 0x6D8 && p.0 != 0x838),
    );
    frame::write(
        &regs,
        &[(regions as usize, ra), (pm as usize, params)],
        (0, 0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A capture holding a frame laid as Dell's TA lays one: a region array
    /// of three tiles, two lists, objects on two pages.
    fn capture() -> (Vec<u8>, Vec<(String, usize, u32)>) {
        let mut v = vec![0u8; frame::LOCAL];
        let mut put = |a: usize, w: u32| v[a..a + 4].copy_from_slice(&w.to_le_bytes());
        let (r, p, t) = (0x00F3_5000usize, 0x0022_B000usize, 0x00E9_D000usize);
        // Tiles 0 and 1 share a list, tile 2 (the last) has its own.
        put(r, 0);
        put(r + 4, 0x0050_0000);
        put(r + 8, 1);
        put(r + 12, 0x0050_0000);
        put(r + 16, 2 | LAST_REGION);
        put(r + 20, 0x0050_1000);
        // List A: an object at 0x100 words into the parameters; list B:
        // one 0x2000 words in (another page) and a second at the first.
        put(0x0050_0000, 0x4080_0100);
        put(0x0050_0004, END);
        put(0x0050_1000, 0x4080_2000);
        put(0x0050_1004, 0x4080_0100);
        put(0x0050_1008, END);
        put(p + 0x400, 0xAAAA_AAAA);
        put(p + 0x8000, 0xBBBB_BBBB);
        put(t, 0x1234_5678);
        let mut state: Vec<(String, usize, u32)> = vec![
            ("MBX".into(), 0x608, r as u32),
            ("MBX".into(), 0x60C, p as u32),
            ("MBX".into(), 0x65C, t as u32),
            ("MBX".into(), 0x650, 1),
            ("MBX".into(), 0x654, 1 << 16),
            ("MBX".into(), 0x658, 1 << 16),
            ("MBX".into(), 0x130, 0x87F5),
            ("MBX".into(), 0x12C, 0x4),
            ("MBX".into(), 0x080, 0),
            ("MBX".into(), 0x800, 0),
            ("MBX".into(), 0x824, 0x001F_8000),
            ("SYS".into(), 0x014, 0x15),
        ];
        state.push(("MBX".into(), 0x680, 0));
        (v, state)
    }

    #[test]
    fn every_page_the_render_reads_and_nothing_it_never_does() {
        let (v, s) = capture();
        let b = extract(&v, &s).expect("extracts");
        let f = frame::parse(&b).expect("parses");
        let segs: Vec<(usize, usize)> = f.segs().map(|(a, d)| (a, d.len())).collect();
        // The region array's page; the lists' two pages, run together; the
        // objects' pages each with the next: 0x22B000 (+0x22C000) for the
        // one 0x400 in, 0x233000 (+0x234000) for the one 0x8000 in.
        assert_eq!(
            segs,
            [
                (0x0022_B000, 2 * PAGE),
                (0x0023_3000, 2 * PAGE),
                (0x0050_0000, 2 * PAGE),
                (0x00F3_5000, PAGE),
            ]
        );
        // The words came along.
        let at = |a: usize| {
            f.segs()
                .find(|(s, d)| *s <= a && a < s + d.len())
                .map(|(s, d)| u32::from_le_bytes(d[a - s..a - s + 4].try_into().unwrap()))
        };
        assert_eq!(at(0x0022_B400), Some(0xAAAA_AAAA));
        assert_eq!(at(0x0023_3000), Some(0xBBBB_BBBB));
        assert_eq!(at(0x00F3_5014), Some(0x0050_1000));
        // MBX registers only, never the reset, status, starts; the system
        // block's never.
        let regs: Vec<usize> = f.regs().map(|p| p.0).collect();
        assert_eq!(
            regs,
            [0x608, 0x60C, 0x65C, 0x650, 0x654, 0x658, 0x130, 0x824]
        );
        // His picture: 2 x 2 RGB565 = 2 words, hashed as the image does.
        assert_eq!(f.picture_words, 2);
        assert_eq!(f.picture_hash, frame::hash([0x1234_5678, 0]));
    }

    #[test]
    fn a_region_array_with_no_end_is_refused() {
        let (mut v, s) = capture();
        v[0x00F3_5010..0x00F3_5014].copy_from_slice(&2u32.to_le_bytes());
        // Every later entry is zero, never last: the walk must stop.
        assert!(extract(&v, &s).is_err());
    }

    #[test]
    fn the_wrong_size_or_a_missing_register_is_refused() {
        let (v, s) = capture();
        assert!(extract(&v[..frame::LOCAL - 4], &s).is_err());
        let no_target: Vec<_> = s.iter().filter(|e| e.1 != 0x65C).cloned().collect();
        assert!(extract(&v, &no_target).is_err());
    }

    #[test]
    fn the_logs_state_is_read() {
        let log = "x\nrun 2 after: state follows (SYS = ...)\nrun 2 after: SYS NINTPW (0x0014) 0x00000015\nrun 2 after: MBX 0x608 0x00f35000\nrun 2 after: state end\n";
        assert_eq!(
            state(log).unwrap(),
            [
                ("SYS".to_string(), 0x14, 0x15),
                ("MBX".to_string(), 0x608, 0x00F3_5000)
            ]
        );
        assert!(state("run 2 after: state follows\nrun 2 after: MBX 0x608 zz\n").is_err());
        assert!(state("nothing").is_err());
    }

    #[test]
    fn the_synthetic_frame_parses_and_reaches_its_objects() {
        let b = synthetic();
        let f = frame::parse(&b).expect("parses");
        assert_eq!(f.reg(0x60C), Some(0x0060_0000));
        assert_eq!(f.reg(0x65C), Some(0x0020_0000));
        assert!(f.reg(0x680).is_none());
        // The list's first word points past the 16-word list.
        let (_, d) = f.segs().find(|s| s.0 == 0x0060_0000).expect("parameters");
        assert_eq!(
            u32::from_le_bytes(d[..4].try_into().unwrap()) & 0x1F_FFFF,
            16
        );
    }
}
