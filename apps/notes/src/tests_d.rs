//! Group D's rows (notes-rule0.md): notes of any length, blocks in any
//! order, smooth ink (filter and widths measured), export, import (outside
//! input: damaged, truncated and oversized files), word suggestions.

extern crate std;

use super::tests::{ctx, draw, store, stroke};
use super::*;
use rp_pim::store::Store;
use std::vec::Vec;

type Pim = core::cell::RefCell<Option<Store<'static>>>;

fn lines(pim: &Pim, id: u32) -> Vec<Vec<u8>> {
    let g = pim.borrow();
    let mut out = Vec::new();
    doc::for_lines(g.as_ref().unwrap(), id, &mut |_, l| {
        out.push(l.to_vec());
        true
    });
    out
}

fn new_note(pim: &Pim, text: &[u8]) -> u32 {
    let mut g = pim.borrow_mut();
    g.as_mut()
        .unwrap()
        .put(
            Kind::Note,
            None,
            &[(tag::TITLE, b"Trip"), (tag::TEXT, text)],
        )
        .unwrap()
}

// ---- Notes of any length ----------------------------------------------

#[test]
fn a_note_grows_past_one_record_into_pages_and_reads_back_whole() {
    let pim = store();
    let id = new_note(pim, b"");
    // 60 KB of lines: thirty times a record's text.
    let mut want = Vec::new();
    {
        let mut g = pim.borrow_mut();
        let s = g.as_mut().unwrap();
        for k in 0..1500u32 {
            let l = std::format!("Line {k:04}: the quick brown fox jumps over it");
            doc::edit(s, id, &doc::Edit::Push(l.as_bytes())).unwrap();
            want.push(l.into_bytes());
        }
    }
    assert_eq!(lines(pim, id), want);
    let (ids, n) = {
        let g = pim.borrow();
        doc::chunks(g.as_ref().unwrap(), id)
    };
    assert!(n > 25, "{n} chunks");
    let _ = ids;
    // An edit in the middle rewrites one chunk only, and keeps the order.
    {
        let mut g = pim.borrow_mut();
        let s = g.as_mut().unwrap();
        let before = s.changes();
        doc::edit(s, id, &doc::Edit::Append(700, b" (edited)")).unwrap();
        assert_eq!(s.changes(), before + 1, "one record written");
        doc::edit(s, id, &doc::Edit::Insert(3, b"[ ] new item")).unwrap();
        doc::edit(s, id, &doc::Edit::Remove(1000)).unwrap();
    }
    want[700].extend_from_slice(b" (edited)");
    want.insert(3, b"[ ] new item".to_vec());
    want.remove(1000);
    assert_eq!(lines(pim, id), want);
    // A chunk grown full in the middle splits, nothing lost.
    {
        let mut g = pim.borrow_mut();
        let s = g.as_mut().unwrap();
        for _ in 0..200 {
            doc::edit(s, id, &doc::Edit::Append(10, b"0123456789")).unwrap();
        }
    }
    for _ in 0..200 {
        want[10].extend_from_slice(b"0123456789");
    }
    // (One line of 2,046 bytes: past a chunk, it goes on as continuation
    // lines and reads back joined.)
    let got = lines(pim, id);
    let mut joined: Vec<Vec<u8>> = Vec::new();
    for l in got {
        match l.strip_prefix(&[doc::CONT]) {
            Some(rest) => joined.last_mut().unwrap().extend_from_slice(rest),
            None => joined.push(l),
        }
    }
    for (k, (a, b)) in joined.iter().zip(&want).enumerate() {
        assert_eq!(a, b, "line {k}: {} vs {}", a.len(), b.len());
    }
    assert_eq!(joined.len(), want.len());
    // Deleting the note takes every page.
    let cx = ctx(pim);
    let mut n = opened(19_723);
    n.delete_note(&cx, id);
    let g = pim.borrow();
    let s = g.as_ref().unwrap();
    assert_eq!(s.all(Kind::Note).count() + s.all(Kind::Ink).count(), 0);
}

#[test]
fn typing_goes_on_past_two_thousand_bytes_in_the_app() {
    let pim = store();
    let cx = ctx(pim);
    let mut n = opened(19_723);
    draw(&mut n, &cx);
    n.event(Event::Key(Key::SoftRight), &cx);
    draw(&mut n, &cx);
    let id = n.open_id().unwrap();
    n.event(
        Event::PenUp {
            x: tool_key(3).x + 9,
            y: tool_key(3).y + 9,
        },
        &cx,
    );
    for k in 0..5000u32 {
        n.event(Event::Char(b'a' + (k % 26) as u8), &cx);
        if k % 80 == 79 {
            n.event(Event::Done, &cx);
        }
        assert!(!n.text_full, "refused at {k}");
    }
    let total: usize = lines(pim, id)
        .iter()
        .filter(|l| doc::ink_block(l).is_none())
        .map(Vec::len)
        .sum();
    assert_eq!(total, 5000);
    // It scrolls: the caret's line is in view, its first lines not.
    draw(&mut n, &cx);
    assert!(n.scroll > 0 && n.total_rows > n.view_rows());
}

#[test]
fn a_note_kept_before_blocks_opens_as_it_was() {
    // Version 2's format: the ink line first, text after, strokes with no
    // extension byte.
    let pim = store();
    let mut ink = [0u8; 200];
    let li = encode_styled(&[&[(10, 300), (60, 310)]], &[2], &mut ink).unwrap();
    let id = {
        let mut g = pim.borrow_mut();
        g.as_mut()
            .unwrap()
            .put(
                Kind::Note,
                None,
                &[(tag::TEXT, b"\x01\nMilk\n[x] Eggs"), (tag::INK, &ink[..li])],
            )
            .unwrap()
    };
    let cx = ctx(pim);
    let mut n = opened(19_723);
    n.show(View::Note(0));
    draw(&mut n, &cx);
    // The ink block first, as tall as its lowest stroke needs (it was the
    // whole sheet), then the text and the item below it.
    assert_eq!(n.vis[0].item, Item::Ink(0));
    assert!(n.vis[0].rows * ROW >= 310 + 28);
    // (Scrolled down: the lines after the tall block.)
    n.scroll = n.vis[0].rows;
    draw(&mut n, &cx);
    assert_eq!(n.vis[0].item, Item::Text);
    assert_eq!(n.vis[1].item, Item::Check(true));
    assert_eq!(n.last, Some((Item::Check(true), n.vis[1].row)));
    let g = pim.borrow();
    assert_eq!(export::inked_blocks(g.as_ref().unwrap(), id), 1);
}

// ---- Blocks in any order ----------------------------------------------

#[test]
fn ink_text_and_lists_go_in_any_order_each_stroke_in_its_block() {
    let pim = store();
    let cx = ctx(pim);
    let mut n = opened(19_723);
    draw(&mut n, &cx);
    n.event(Event::Key(Key::SoftRight), &cx);
    draw(&mut n, &cx);
    let id = n.open_id().unwrap();
    let (ix, iy) = INK_AT;
    // Ink in the first block.
    stroke(&mut n, &cx, ix + 20, iy + 30, 10, |_| 0);
    // A paragraph after it.
    n.event(
        Event::PenUp {
            x: tool_key(3).x + 9,
            y: tool_key(3).y + 9,
        },
        &cx,
    );
    for &c in b"Then a list" {
        n.event(Event::Char(c), &cx);
    }
    // A checklist after the paragraph.
    n.event(
        Event::PenUp {
            x: tool_key(4).x + 9,
            y: tool_key(4).y + 9,
        },
        &cx,
    );
    for &c in b"Bread" {
        n.event(Event::Char(c), &cx);
    }
    n.event(Event::Key(Key::SoftLeft), &cx); // keyboard down
    n.tool = Tool::Pen;
    draw(&mut n, &cx);
    // The pen on the blank paper under the list: a second ink block there.
    let below = n.screen_y(n.total_rows) as u16 + 20;
    stroke(&mut n, &cx, ix + 40, below, 10, |_| 0);
    draw(&mut n, &cx);
    let l = lines(pim, id);
    assert_eq!(l.len(), 4, "{l:?}");
    assert_eq!(doc::ink_block(&l[0]), Some(0));
    assert_eq!(l[1], b"Then a list");
    assert_eq!(l[2], b"[ ] Bread");
    assert_eq!(doc::ink_block(&l[3]), Some(1));
    // Each block holds its own stroke, in its own coordinates.
    let g = pim.borrow();
    let s = g.as_ref().unwrap();
    let mut blocks = Vec::new();
    let mut pts = [(0i16, 0i16); 255];
    let mut ws = [0u8; 255];
    each_stroke(s, id, &mut |p| {
        let u = unpack_all(p, &mut pts, &mut ws);
        blocks.push((u.block, pts[0].1, u.widths));
    });
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].0, 0);
    assert_eq!(blocks[1].0, 1);
    assert!(blocks[1].1 < 40, "local to its block: {:?}", blocks[1]);
    assert!(blocks[0].2 && blocks[1].2, "pen strokes keep their widths");
    drop(g);
    // The eraser takes a stroke of its own block only: on block 1 at the
    // place block 0's stroke has in block 0, nothing goes.
    n.tool = Tool::Erase;
    let v1 = n.vis[..n.n_vis]
        .iter()
        .find(|v| v.item == Item::Ink(1))
        .copied()
        .unwrap();
    let top1 = n.screen_y(v1.row) as u16;
    stroke(&mut n, &cx, ix + 20, top1 + 30, 4, |_| 0);
    let count = |pim: &Pim| {
        let g = pim.borrow();
        let mut k = 0;
        each_stroke(g.as_ref().unwrap(), id, &mut |_| k += 1);
        k
    };
    assert_eq!(count(pim), 2);
    stroke(&mut n, &cx, ix + 45, below, 4, |_| 0);
    assert_eq!(count(pim), 1);
}

#[test]
fn tapping_a_line_moves_the_caret_and_return_and_backspace_edit_there() {
    let pim = store();
    let id = new_note(pim, b"First\nSecond\n[ ] Item");
    let cx = ctx(pim);
    let mut n = opened(19_723);
    n.show(View::Note(0));
    draw(&mut n, &cx);
    let y = n.screen_y(0) as u16 + 10;
    n.event(Event::PenDown { x: 100, y }, &cx);
    n.event(Event::PenUp { x: 100, y }, &cx);
    assert!(n.typing && n.cursor == Some(0));
    n.event(Event::Char(b'!'), &cx);
    n.event(Event::Done, &cx);
    n.event(Event::Char(b'x'), &cx);
    assert_eq!(
        lines(pim, id),
        [&b"First!"[..], b"x", b"Second", b"[ ] Item"]
    );
    n.event(Event::Delete, &cx);
    n.event(Event::Delete, &cx);
    assert_eq!(lines(pim, id), [&b"First!"[..], b"Second", b"[ ] Item"]);
    // Return on a checklist item starts the next; on an empty one, the
    // list ends.
    n.cursor = Some(2);
    n.event(Event::Done, &cx);
    assert_eq!(lines(pim, id)[3], b"[ ] ");
    n.event(Event::Done, &cx);
    assert_eq!(lines(pim, id)[3], b"");
}

// ---- Smooth ink: the filter measured ----------------------------------

/// A pseudo-random source (the tests' noise, the same every run).
fn lcg(s: &mut u32) -> i32 {
    *s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    ((*s >> 16) % 5) as i32 - 2
}

/// A circle of radius 80 written at `step` px a sample, with the
/// resistive panel's +-2 px noise: the raw and the filtered points' mean
/// distance off the true circle (tenths of a pixel), and the filter's lag
/// (tenths of a pixel, along the stroke).
fn circle(step: f32) -> (i32, i32, i32) {
    let mut f = pen::Filter::default();
    let mut seed = 7u32;
    let (mut raw_err, mut filt_err, mut lag, mut k) = (0f32, 0f32, 0f32, 0f32);
    let n = (2.0 * core::f32::consts::PI * 80.0 / step) as usize;
    for i in 0..n {
        let a = i as f32 * step / 80.0;
        let (tx, ty) = (200.0 + 80.0 * a.cos(), 150.0 + 80.0 * a.sin());
        let (rx, ry) = (
            (tx.round() as i32 + lcg(&mut seed)) as i16,
            (ty.round() as i32 + lcg(&mut seed)) as i16,
        );
        let ((fx, fy), _) = if i == 0 {
            f.start(rx, ry)
        } else {
            f.step(rx, ry)
        };
        if i > 5 {
            let off =
                |x: f32, y: f32| (((x - 200.0).powi(2) + (y - 150.0).powi(2)).sqrt() - 80.0).abs();
            raw_err += off(f32::from(rx), f32::from(ry));
            filt_err += off(f32::from(fx), f32::from(fy));
            // Behind the pen along its way (the noise across it cancels).
            lag -= (f32::from(fx) - tx) * -a.sin() + (f32::from(fy) - ty) * a.cos();
            k += 1.0;
        }
    }
    (
        (raw_err / k * 10.0) as i32,
        (filt_err / k * 10.0) as i32,
        (lag / k * 10.0) as i32,
    )
}

#[test]
fn the_pen_filter_halves_the_jitter_and_keeps_up_with_a_fast_pen() {
    // Slow: the jitter is what shows; it is more than halved.
    let (raw, filt, lag) = circle(1.5);
    std::println!(
        "slow (1.5 px a sample): raw {raw}, filtered {filt} (tenths of a px off the circle), lag {lag}"
    );
    assert!(filt * 2 <= raw, "raw {raw} filtered {filt}");
    assert!(lag <= 30, "lag {lag}");
    // Writing speed: the filter lets the pen through.
    for step in [6.0, 12.0] {
        let (raw, filt, lag) = circle(step);
        std::println!("{step} px a sample: raw {raw}, filtered {filt}, lag {lag}");
        assert!(lag.abs() <= 10, "lag {lag} at {step}");
        assert!(filt <= raw, "{filt} {raw}");
    }
}

#[test]
fn width_comes_from_speed_wide_slow_thin_fast_and_never_jumps() {
    let mut f = pen::Filter::default();
    f.start(0, 0);
    let mut x = 0i16;
    let mut last = 11u8;
    for _ in 0..30 {
        x += 1;
        let (_, w) = f.step(x, 0);
        assert!(w.abs_diff(last) <= 2);
        last = w;
    }
    let slow = last;
    for _ in 0..30 {
        x += 30;
        let (_, w) = f.step(x, 0);
        assert!(w.abs_diff(last) <= 2, "{last} -> {w}");
        last = w;
    }
    assert!(slow >= 10 && last <= 5, "slow {slow} fast {last}");
    // The lift lands where the pen left the paper.
    assert_eq!(f.lift(x + 3, 4).0, (x + 3, 4));
    // A wide point paints wider than a thin one.
    let mut wide = std::vec![0u32; 56 * 352];
    let mut thin = std::vec![0u32; 56 * 352];
    let line: &[(i16, i16)] = &[(40, 100), (400, 100)];
    paint_cover_at(&mut wide, &[line], &[0], &[&[14, 14]], 0, PEN[0]);
    paint_cover_at(&mut thin, &[line], &[0], &[&[2, 2]], 0, PEN[0]);
    let ink = |w: &[u32]| w.iter().map(|x| x.count_ones()).sum::<u32>();
    assert!(ink(&wide) > ink(&thin) * 2);
}

#[test]
fn version_two_strokes_round_trip_their_block_and_widths() {
    let pts: Vec<(i16, i16)> = (0..40).map(|k| (k * 9, (k * 7) % 50)).collect();
    let ws: Vec<u8> = (0..40).map(|k| (k % 16) as u8).collect();
    let mut out = [0u8; STROKE_MAX];
    let n = encode_stroke(&pts, Some(&ws), 3 | SIZE_BOLD, 17, &mut out).unwrap();
    assert_eq!(stroke_len(&out[..n]), Some(n));
    let mut p = [(0i16, 0i16); 255];
    let mut w = [0u8; 255];
    let u = unpack_all(&out[..n], &mut p, &mut w);
    assert_eq!(
        (u.style, u.block, u.n, u.widths),
        (3 | SIZE_BOLD, 17, 40, true)
    );
    assert_eq!(&p[..40], &pts[..]);
    assert_eq!(&w[..40], &ws[..]);
    // A long step is split; a cut stroke is no stroke.
    let n2 = encode_stroke(&[(0, 0), (400, 0)], Some(&[3, 9]), 0, 1, &mut out).unwrap();
    assert_eq!(out[0], 5);
    for cut in 0..n2 {
        assert_eq!(stroke_len(&out[..cut]), None, "{cut}");
    }
}

// ---- Export -----------------------------------------------------------

fn exported(pim: &Pim, id: u32, fmt: export::Format, block: u8) -> Vec<u8> {
    let g = pim.borrow();
    let s = g.as_ref().unwrap();
    let mut v = Vec::new();
    let mut work = export::PngWork::new();
    match fmt {
        export::Format::Svg => export::svg(s, id, block, &mut v),
        export::Format::Png => export::png(s, id, block, &mut work, &mut v),
        _ => export::text(s, id, b"Trip", fmt, &mut v),
    }
    v
}

#[test]
fn text_and_markdown_say_the_note_as_written() {
    let pim = store();
    let id = new_note(
        pim,
        b"Pack light\n\x02 and early\n[ ] Charger\n[x] Ticket\n\x01\nDone",
    );
    let md = exported(pim, id, export::Format::Markdown, 0);
    assert_eq!(
        std::str::from_utf8(&md).unwrap(),
        "# Trip\n\nPack light and early\n\n- [ ] Charger\n- [x] Ticket\n\n![Ink 1](Trip-1.svg)\n\nDone\n"
    );
    let txt = exported(pim, id, export::Format::Text, 0);
    assert_eq!(
        std::str::from_utf8(&txt).unwrap(),
        "Trip\r\n\r\nPack light and early\r\n[ ] Charger\r\n[x] Ticket\r\n[Ink 1: Trip-1.svg]\r\nDone\r\n"
    );
    let mut name = [0u8; 64];
    let k = export::file_name(b"a/b:c*..\\con", 2, export::Format::Png, &mut name);
    assert_eq!(&name[..k], b"a_b_c____con-2.png");
    let k = export::file_name(b"   ", 0, export::Format::Text, &mut name);
    assert_eq!(&name[..k], b"Note.txt");
}

fn inked(pim: &Pim) -> u32 {
    let id = new_note(pim, b"\x01");
    let mut out = [0u8; STROKE_MAX];
    let mut ink = Vec::new();
    let a: Vec<(i16, i16)> = (0..30)
        .map(|k| (20 + k * 12, 60 + ((k * 13) % 40)))
        .collect();
    let n = encode_stroke(&a, Some(&[8; 30]), 2, 0, &mut out).unwrap();
    ink.extend_from_slice(&out[..n]);
    let n = encode_stroke(&[(30, 150), (300, 160)], None, MARKER, 0, &mut out).unwrap();
    ink.extend_from_slice(&out[..n]);
    let mut g = pim.borrow_mut();
    g.as_mut().unwrap().change(id, &[(tag::INK, &ink)]).unwrap();
    id
}

#[test]
fn svg_is_the_screen_s_own_curve() {
    let pim = store();
    let id = inked(pim);
    let svg = std::string::String::from_utf8(exported(pim, id, export::Format::Svg, 0)).unwrap();
    assert!(svg.starts_with("<?xml") && svg.trim_end().ends_with("</svg>"));
    assert!(
        svg.contains("width=\"448\" height=\"224\""),
        "the marker at y 160 needs 8 rows"
    );
    assert!(svg.contains("stroke=\"#c03030\""), "the red pen");
    assert!(
        svg.contains("stroke=\"#ffe040\" stroke-opacity=\"0.4"),
        "the marker see-through: {svg}"
    );
    assert!(
        svg.contains("stroke=\"#c03030\" stroke-width"),
        "a pen opaque"
    );
    // The first segment's Bezier: p1 (20,60), its controls p1 + (p2 - p0)/6
    // and p2 - (p3 - p1)/6, as the brush's Catmull-Rom.
    let p = |k: i32| (20 + k * 12, 60 + ((k * 13) % 40));
    let (p0, p1, p2, p3) = (p(0), p(0), p(1), p(2));
    let c1 = (
        (p1.0 * 10 + (p2.0 - p0.0) * 10 / 6),
        (p1.1 * 10 + (p2.1 - p0.1) * 10 / 6),
    );
    let c2 = (
        (p2.0 * 10 - (p3.0 - p1.0) * 10 / 6),
        (p2.1 * 10 - (p3.1 - p1.1) * 10 / 6),
    );
    let t = |v: i32| {
        if v % 10 == 0 {
            std::format!("{}", v / 10)
        } else {
            std::format!("{}.{}", v / 10, v % 10)
        }
    };
    let want = std::format!(
        "M20 60C{} {} {} {} 32 73",
        t(c1.0),
        t(c1.1),
        t(c2.0),
        t(c2.1)
    );
    assert!(svg.contains(&want), "{want} in {svg}");
}

/// A PNG's IDAT data inflated (fixed codes only: what ours writes), its
/// header's size, and each pixel's palette index.
fn inflate_png(png: &[u8]) -> (u32, u32, Vec<u8>, Vec<u8>) {
    assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 13, 10, 26, 10]);
    let (mut at, mut z, mut size, mut trns) = (8, Vec::new(), (0, 0), Vec::new());
    while at < png.len() {
        let len = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
        let ty = &png[at + 4..at + 8];
        let data = &png[at + 8..at + 8 + len];
        let crc = u32::from_be_bytes(png[at + 8 + len..at + 12 + len].try_into().unwrap());
        assert_eq!(
            crc,
            rp_pim::store::crc32(&png[at + 4..at + 8 + len]),
            "chunk CRC"
        );
        match ty {
            b"IHDR" => {
                size = (
                    u32::from_be_bytes(data[..4].try_into().unwrap()),
                    u32::from_be_bytes(data[4..8].try_into().unwrap()),
                )
            }
            b"IDAT" => z.extend_from_slice(data),
            b"tRNS" => trns = data.to_vec(),
            _ => {}
        }
        at += 12 + len;
    }
    assert_eq!(z[0] & 15, 8);
    assert_eq!(
        (u16::from(z[0]) << 8 | u16::from(z[1])) % 31,
        0,
        "zlib header check"
    );
    let mut bit = 16usize;
    let mut get = |n: u32| -> u32 {
        let mut v = 0;
        for k in 0..n {
            v |= u32::from((z[bit / 8] >> (bit % 8)) & 1) << k;
            bit += 1;
        }
        v
    };
    assert_eq!(get(1), 1);
    assert_eq!(get(2), 1, "fixed codes");
    let mut out: Vec<u8> = Vec::new();
    let rev = |get: &mut dyn FnMut(u32) -> u32, n: u32| (0..n).fold(0, |v, _| (v << 1) | get(1));
    let lb = [
        3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115,
        131, 163, 195, 227, 258,
    ];
    let le = [
        0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
    ];
    let db = [
        1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
        2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
    ];
    let de = [
        0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12,
        13, 13,
    ];
    loop {
        let mut c = rev(&mut get, 7);
        let sym = if c <= 23 {
            c + 256
        } else {
            c = (c << 1) | get(1);
            if (0x30..=0xBF).contains(&c) {
                c - 0x30
            } else if (0xC0..=0xC7).contains(&c) {
                c - 0xC0 + 280
            } else {
                c = (c << 1) | get(1);
                c - 0x190 + 144
            }
        };
        if sym < 256 {
            out.push(sym as u8);
        } else if sym == 256 {
            break;
        } else {
            let i = (sym - 257) as usize;
            let len = lb[i] + get(le[i]);
            let di = rev(&mut get, 5) as usize;
            let dist = (db[di] + get(de[di])) as usize;
            for _ in 0..len {
                out.push(out[out.len() - dist]);
            }
        }
    }
    let (w, h) = size;
    let mut px = Vec::new();
    for row in out.chunks(w as usize + 1) {
        assert_eq!(row[0], 0, "filter none");
        px.extend_from_slice(&row[1..]);
    }
    assert_eq!(px.len(), (w * h) as usize);
    (w, h, px, trns)
}

#[test]
fn png_decodes_to_the_brush_s_own_coverage_and_is_small() {
    let pim = store();
    let id = inked(pim);
    let png = exported(pim, id, export::Format::Png, 0);
    let (w, h, px, trns) = inflate_png(&png);
    assert_eq!((w, h), (448, 224));
    std::println!(
        "PNG {} bytes for a {w} x {h} block ({} raw)",
        png.len(),
        w * h
    );
    assert!(png.len() * 8 < (w * h) as usize, "{}", png.len());
    // Where the red stroke starts: red, nearly opaque; empty paper clear.
    let at = |x: usize, y: usize| px[y * w as usize + x];
    let e = usize::from(at(20, 60));
    assert!(e > 0 && (e - 1) / 15 == 2 && trns[e] > 180, "{e}");
    assert_eq!(at(440, 10), 0);
    // The marker's band is see-through yellow.
    // (The long step is split at (157, 160): at x 100 the band is at 155.)
    let m = usize::from(at(100, 155));
    assert!(m > 0);
    assert_eq!((m - 1) / 15, 6);
    assert!(trns[m] < 130);
    // Streamed in pieces, the bytes are the same.
    let g = pim.borrow();
    let s = g.as_ref().unwrap();
    let mut work = export::PngWork::new();
    let mut pieced = Vec::new();
    for off in (0..png.len()).step_by(700) {
        let mut buf = [0u8; 700];
        let mut win = export::Window::new(off, &mut buf);
        export::png(s, id, 0, &mut work, &mut win);
        let got = win.got;
        pieced.extend_from_slice(&buf[..got]);
    }
    assert_eq!(pieced, png);
}

#[test]
fn the_app_exports_through_the_shell_a_piece_at_a_time() {
    let pim = store();
    let id = inked(pim);
    {
        let mut g = pim.borrow_mut();
        g.as_mut()
            .unwrap()
            .change(id, &[(tag::TEXT, b"\x01\nHello"), (tag::TITLE, b"Trip")])
            .unwrap();
    }
    let cx = ctx(pim);
    let mut n = opened(19_723);
    n.show(View::Note(0));
    draw(&mut n, &cx);
    n.export(&cx, id, export::Format::Svg);
    // Group A's card service: `CARDPUT "<length> <name>"`, the body
    // taken a piece at a time (rp_today::cardfiles::put_from).
    let ask = n.card_ask().expect("a write asked");
    assert_eq!(ask.what, b"CARDPUT");
    let arg_b = ask.arg;
    let arg = std::str::from_utf8(arg_b.get()).unwrap();
    let (len, name) = arg.split_once(' ').unwrap();
    assert_eq!(name, "Trip-1.svg");
    let len: usize = len.parse().unwrap();
    let mut file = Vec::new();
    while file.len() < len {
        let mut buf = [0u8; 512];
        let k = n.net_body(file.len(), &mut buf, &cx);
        assert!(k > 0);
        file.extend_from_slice(&buf[..k]);
    }
    assert_eq!(file, exported(pim, id, export::Format::Svg, 0));
    n.net_answer(true, &[], &cx);
    assert!(n.card_ask().is_none(), "one ink block: one file");
    assert_eq!(n.said, b"SAVED IN RETROPANE ON THE CARD");
    // A card that refuses: its reason said, not dropped.
    n.export(&cx, id, export::Format::Markdown);
    assert_eq!(n.card_ask().map(|a| a.what), Some(&b"CARDPUT"[..]));
    n.net_answer(false, b"no card", &cx);
    assert_eq!(n.said, b"NOT SAVED: NO CARD");
    n.export(&cx, id, export::Format::Text);
    assert!(n.card_ask().is_some());
    n.net_answer(false, b"the card is full", &cx);
    assert_eq!(n.said, b"NOT SAVED: THE CARD IS FULL");
}

// ---- Import: outside input --------------------------------------------

fn read_all(bytes: &[u8], piece: usize) -> Result<Vec<Vec<u8>>, import::Damage> {
    let mut r = import::Reader::new();
    let mut out = Vec::new();
    for p in bytes.chunks(piece.max(1)) {
        r.feed(p, &mut |l| {
            out.push(l.to_vec());
            Ok(())
        })?;
    }
    r.finish(&mut |l| {
        out.push(l.to_vec());
        Ok(())
    })?;
    Ok(out)
}

#[test]
fn text_files_come_in_as_lines_tasks_as_items() {
    let file = "\u{FEFF}# Plan\r\n- [ ] Call Sam\r\n* [X] Book\r\nCaf\u{e9} at 9\tsharp\rlast";
    let got = read_all(file.as_bytes(), 3).unwrap();
    assert_eq!(
        got,
        [
            &b"# Plan"[..],
            b"[ ] Call Sam",
            b"[x] Book",
            "Caf\u{e9} at 9 sharp".as_bytes(),
            b"last"
        ]
    );
    // A line longer than a chunk goes on as continuations, whole.
    let long = "word ".repeat(1000);
    let got = read_all(long.as_bytes(), 4096).unwrap();
    assert!(got.len() > 4 && got[1..].iter().all(|l| l[0] == doc::CONT));
    let joined: Vec<u8> = got
        .iter()
        .flat_map(|l| l.strip_prefix(&[doc::CONT]).unwrap_or(l).to_vec())
        .collect();
    assert_eq!(joined, long.as_bytes());
    // Every piece size reads the same.
    let file = "a\u{1F600}b\u{e9}\nsecond line\n".repeat(50);
    let whole = read_all(file.as_bytes(), file.len()).unwrap();
    for piece in 1..9 {
        assert_eq!(read_all(file.as_bytes(), piece).unwrap(), whole);
    }
}

#[test]
fn damaged_files_are_refused_by_name_and_never_panic() {
    use import::Damage::*;
    let cases: &[(&[u8], import::Damage)] = &[
        (b"ok\x00", Control(2)),
        (b"forge \x01 ink", Control(6)),
        (b"\x1b[31m", Control(0)),
        (b"bad \xff", NotUtf8(4)),
        (b"\xc0\xaf overlong", NotUtf8(0)),
        (b"\xe0\x80\xaf overlong", NotUtf8(1)),
        (b"\xed\xa0\x80 surrogate", NotUtf8(1)),
        (b"\xf4\x90\x80\x80 past U+10FFFF", NotUtf8(1)),
        (b"cut \xe2\x82", Truncated(6)),
        (b"\xef\xbb", Truncated(2)),
        (b"x\xc3", Truncated(2)),
    ];
    for (bytes, want) in cases {
        for piece in [1, 2, 64] {
            assert_eq!(read_all(bytes, piece), Err(*want), "{bytes:?}");
        }
    }
    // Random bytes: refused or read, never a panic, every line within a
    // chunk's room and free of control bytes.
    let mut seed = 1u32;
    for _ in 0..3000 {
        let n = (lcg(&mut seed) + 2) as usize * 40;
        let bytes: Vec<u8> = (0..n)
            .map(|_| {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                (seed >> 16) as u8
            })
            .collect();
        if let Ok(lines) = read_all(&bytes, 7) {
            for l in lines {
                assert!(l.len() <= import::LINE_MAX + 1);
                assert!(l.iter().skip(1).all(|&c| c >= 0x20 || c == doc::CONT));
            }
        }
    }
}

#[test]
fn the_app_imports_into_the_note_and_takes_a_damaged_file_back_out() {
    let pim = store();
    let id = new_note(pim, b"Kept");
    let cx = ctx(pim);
    let mut n = opened(19_723);
    n.show(View::Note(0));
    draw(&mut n, &cx);
    // The menu's IMPORT lists the folder; a .txt among others is offered.
    n.event(Event::Key(Key::SoftRight), &cx);
    let r = menu_key(4, 6);
    n.event(
        Event::PenUp {
            x: r.x + 9,
            y: r.y + 9,
        },
        &cx,
    );
    let ask = n.card_ask().expect("a list asked");
    assert_eq!((ask.what, ask.arg.get()), (&b"CARDFILES"[..], &b"txt"[..]));
    // The card's text files by number (a name never asked for by path);
    // a damaged line skipped.
    n.net_answer(true, b"0|plan.TXT\nbad line\n3|readme.md\n", &cx);
    assert_eq!(n.n_files, 2);
    n.event(Event::PenUp { x: 100, y: 210 }, &cx);
    let ask = n.card_ask().expect("a read asked");
    assert_eq!(
        (ask.what, ask.arg.get()),
        (&b"CARDFILE"[..], &b"3 txt 0"[..])
    );
    // Each answer: the file's size, then its bytes from the offset asked.
    let file = b"Pack\n- [ ] Charger\nShoes";
    let piece = |at: usize, k: usize| {
        let mut b = 24u32.to_le_bytes().to_vec();
        b.extend_from_slice(&file[at..at + k]);
        b
    };
    n.net_answer(true, &piece(0, 10), &cx);
    let ask = n.card_ask().expect("the next piece asked");
    assert_eq!(ask.arg.get(), b"3 txt 10");
    n.net_answer(true, &piece(10, 14), &cx);
    assert!(n.card_ask().is_none());
    assert_eq!(
        lines(pim, id),
        [&b"Kept"[..], b"Pack", b"[ ] Charger", b"Shoes"]
    );
    assert_eq!(n.said, b"IMPORTED");
    // A damaged one: what it put in goes back out; the note is as it was.
    n.job = Job::Read {
        id,
        put: 0,
        file: 0,
        at: 0,
    };
    let bad = b"one\ntwo\nthr\xffee";
    let mut body = (bad.len() as u32).to_le_bytes().to_vec();
    body.extend_from_slice(bad);
    n.net_answer(true, &body, &cx);
    assert_eq!(
        lines(pim, id),
        [&b"Kept"[..], b"Pack", b"[ ] Charger", b"Shoes"]
    );
    assert_eq!(n.said, b"NOT IMPORTED: NOT UTF-8 TEXT");
    // Too big: refused before a byte is read.
    n.job = Job::Read {
        id,
        put: 0,
        file: 0,
        at: 0,
    };
    let mut body = 300_000u32.to_le_bytes().to_vec();
    body.push(b'a');
    n.net_answer(true, &body, &cx);
    assert_eq!(n.said, b"NOT IMPORTED: OVER 256 KB");
    assert_eq!(lines(pim, id).len(), 4);
    // A file cut short under the read, and a card that fails mid-file:
    // what came in taken back out.
    for fail in [None, Some(&b"not readable"[..])] {
        n.job = Job::Read {
            id,
            put: 0,
            file: 0,
            at: 0,
        };
        let mut body = 20u32.to_le_bytes().to_vec();
        body.extend_from_slice(b"one\ntwo\n");
        n.net_answer(true, &body, &cx);
        assert_eq!(lines(pim, id).len(), 6, "two lines in so far");
        assert!(n.card_ask().is_some());
        match fail {
            None => n.net_answer(true, &20u32.to_le_bytes(), &cx),
            Some(why) => n.net_answer(false, why, &cx),
        };
        assert_eq!(n.said, b"NOT IMPORTED: UNREADABLE");
        assert_eq!(lines(pim, id).len(), 4);
    }
    // Short or empty answers never panic.
    n.job = Job::Read {
        id,
        put: 0,
        file: 0,
        at: 0,
    };
    n.net_answer(true, b"ab", &cx);
    assert_eq!(n.said, b"NOT IMPORTED: UNREADABLE");
}

// ---- Words -------------------------------------------------------------

#[test]
fn the_dictionary_is_small_and_finds_words_by_prefix() {
    let d = words::Dict::default_lang().expect("US English packed");
    std::println!("en-US: {} words in {} bytes", d.len(), d.bytes());
    assert!(d.len() > 900);
    assert!(d.bytes() < 16 * 1024, "{} bytes", d.bytes());
    assert!(d.has(b"tomorrow") && d.has(b"groceries") && !d.has(b"zzz"));
    let l = words::Learned::new();
    let mut out = [[0u8; words::WORD_MAX]; 3];
    let n = words::suggest(b"tom", Some(&d), &l, &mut out);
    assert_eq!(&out[0][..n[0]], b"tomorrow");
    // Capitals as typed.
    let n = words::suggest(b"Tom", Some(&d), &l, &mut out);
    assert_eq!(&out[0][..n[0]], b"Tomorrow");
    let n = words::suggest(b"TOM", Some(&d), &l, &mut out);
    assert_eq!(&out[0][..n[0]], b"TOMORROW");
    // Nothing for a word not begun, or a number.
    assert_eq!(words::suggest(b"", Some(&d), &l, &mut out), [0; 3]);
    assert_eq!(words::suggest(b"12", Some(&d), &l, &mut out), [0; 3]);
}

#[test]
fn words_the_user_wrote_are_learned_and_offered_first() {
    let d = words::Dict::default_lang();
    let mut l = words::Learned::new();
    l.count_in(
        b"Meet Priyanka at Powell's. Priyanka has the tickets.",
        d.as_ref(),
    );
    let mut out = [[0u8; words::WORD_MAX]; 3];
    let n = words::suggest(b"Pri", d.as_ref(), &l, &mut out);
    assert_eq!(&out[0][..n[0]], b"Priyanka");
    let n = words::suggest(b"pow", d.as_ref(), &l, &mut out);
    assert!(out[..3].iter().zip(n).any(|(w, k)| &w[..k] == b"powell's"));
}

#[test]
fn a_sentence_starts_at_a_line_s_start_and_after_its_stop() {
    assert!(words::sentence_starts(b""));
    assert!(words::sentence_starts(b"[ ] "));
    assert!(words::sentence_starts(b"Done. "));
    assert!(words::sentence_starts(b"Why? "));
    assert!(!words::sentence_starts(b"Done."));
    assert!(!words::sentence_starts(b"e.g mid "));
    assert!(!words::sentence_starts(b"Milk and "));
}

#[test]
fn typing_in_the_app_offers_words_and_capitals() {
    let pim = store();
    let id = new_note(pim, b"Visit Grandma Ruthie. Ruthie likes tulips");
    let cx = ctx(pim);
    let mut n = opened(19_723);
    n.show(View::Note(0));
    draw(&mut n, &cx);
    n.event(
        Event::PenUp {
            x: tool_key(3).x + 9,
            y: tool_key(3).y + 9,
        },
        &cx,
    );
    assert_eq!(n.caps(), Some(true), "a new paragraph starts a sentence");
    for &c in b"Ru" {
        n.event(Event::Char(c), &cx);
    }
    assert_eq!(n.caps(), Some(false));
    assert_eq!(n.suggestions()[0], b"Ruthie");
    n.event(Event::Suggestion(0), &cx);
    assert_eq!(lines(pim, id)[1], b"Ruthie ");
    for &c in b"is here. " {
        n.event(Event::Char(c), &cx);
    }
    assert_eq!(n.caps(), Some(true));
}

#[test]
fn every_tray_message_fits_between_color_and_the_tray_edge() {
    use import::Damage;
    let pim = store();
    let label = ctx(pim).fonts.label;
    let room = 456 - (24 + label.width(b"COLOR") + 12);
    let mut all: Vec<&[u8]> = [
        &b"NO INK TO EXPORT"[..],
        b"SAVING...",
        b"IMPORTING...",
        b"IMPORTED",
        b"SAVED IN RETROPANE ON THE CARD",
        b"NOT IMPORTED: NO CARD",
        b"NOT IMPORTED: UNREADABLE",
        b"THIS NOTE'S TEXT IS FULL",
        b"THIS NOTE'S INK IS FULL",
    ]
    .to_vec();
    for why in [
        &b"no card"[..],
        b"the card is full",
        b"too big to write",
        b"the app's file was short",
        b"anything else",
    ] {
        all.push(super::not_saved(why));
    }
    for d in [
        Damage::TooBig(0),
        Damage::Control(0),
        Damage::NotUtf8(0),
        Damage::Truncated(0),
        Damage::NoRoom,
    ] {
        all.push(d.says());
    }
    for m in all {
        assert!(
            label.width(m) <= room,
            "{}: {} px of {room}",
            std::string::String::from_utf8_lossy(m),
            label.width(m)
        );
    }
}
