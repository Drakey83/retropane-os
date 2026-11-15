//! Notes driven as the stylus drives it, against a real store.

extern crate std;

use super::*;
use rp_app::{Fonts, Frame, Look, Now, Shapes};
use rp_draw::{Font, Image};
use rp_pim::store::Store;
use std::boxed::Box;
use std::vec;

/// Room for the tests' own unpacking ([`decode_ink`]).
const POINTS: usize = 900;
const STROKES: usize = 48;

static FONT: Font = Font {
    line: 20,
    ascent: 16,
    first: 32,
    glyphs: &[],
};
static CORNER: Image = Image {
    w: 0,
    h: 0,
    px: &[],
};
static ROWS: [Argb; 64] = [0xFF20_2020; 64];
static ART: PanelArt = PanelArt {
    radius: 0,
    rows: &ROWS,
    top: [0, 0],
    bottom: [0, 0],
    side: 0,
    corners: [&CORNER, &CORNER, &CORNER, &CORNER],
};
static FONTS: Fonts = Fonts {
    ui: &FONT,
    label: &FONT,
    big: &FONT,
    num: &FONT,
    title: &FONT,
};
static LOOK: Look = Look {
    light: false,
    panel: &ART,
    pressed: &ART,
    key: &ART,
    key_down: &ART,
    on: &ART,
};

use rp_app::PanelArt;

fn store() -> &'static core::cell::RefCell<Option<Store<'static>>> {
    let mem = vec![0u8; rp_pim::store::MEMORY].leak();
    let mut s = Store::new(mem);
    s.load_start(None);
    Box::leak(Box::new(core::cell::RefCell::new(Some(s))))
}

fn ctx(pim: &'static core::cell::RefCell<Option<Store<'static>>>) -> Ctx<'static> {
    Ctx {
        fonts: &FONTS,
        look: &LOOK,
        now: Now {
            day: 19_723,
            minute: 720,
        },
        pics: None,
        forecast: &[],
        us: true,
        utc_offset: 0,
        shapes: Box::leak(Box::new(Shapes::new())),
        clip: Box::leak(Box::new(core::cell::RefCell::new(rp_app::Text::EMPTY))),
        pim: Some(pim),
    }
}

fn draw(n: &mut Notes, cx: &Ctx) {
    let mut f = Box::new(Frame::new());
    n.draw(&mut f, cx);
}

fn note_text(pim: &core::cell::RefCell<Option<Store<'static>>>) -> (std::vec::Vec<u8>, usize) {
    let g = pim.borrow();
    let s = g.as_ref().unwrap();
    let r = s.all(Kind::Note).next().unwrap();
    (r.text(tag::TEXT).to_vec(), r.text(tag::INK).len())
}

#[test]
fn new_then_write_a_stroke_then_undo_it() {
    let pim = store();
    let cx = ctx(pim);
    let mut n = opened(19_723);
    draw(&mut n, &cx);
    // New: a note opens, its ink panel ready.
    assert_eq!(n.event(Event::Key(Key::SoftRight), &cx), Reply::Redraw);
    assert!(matches!(n.view, View::Note(0)));
    draw(&mut n, &cx);
    let (ix, iy) = n.ink_at.expect("an ink panel on screen");
    // A stroke across the panel.
    n.event(
        Event::PenDown {
            x: ix + 20,
            y: iy + 40,
        },
        &cx,
    );
    for k in 1..20u16 {
        n.event(
            Event::PenMove {
                x: ix + 20 + k * 6,
                y: iy + 40 + k % 5,
            },
            &cx,
        );
    }
    n.event(
        Event::PenUp {
            x: ix + 140,
            y: iy + 44,
        },
        &cx,
    );
    let (_, ink) = note_text(pim);
    assert!(ink > 6, "the stroke kept ({ink} bytes)");
    // UNDO: the stroke goes.
    draw(&mut n, &cx);
    n.event(
        Event::PenUp {
            x: tool_key(5).x + 10,
            y: STRIP.y + 20,
        },
        &cx,
    );
    let (_, ink) = note_text(pim);
    assert_eq!(ink, 0);
}

#[test]
fn a_checklist_item_ticks_and_unticks() {
    let pim = store();
    {
        let mut g = pim.borrow_mut();
        let s = g.as_mut().unwrap();
        s.put(
            Kind::Note,
            None,
            &[
                (tag::TITLE, b"Shopping"),
                (tag::TEXT, b"[ ] Milk\n[ ] Stamps"),
            ],
        )
        .unwrap();
    }
    let cx = ctx(pim);
    let mut n = opened(19_723);
    n.show(View::Note(0));
    draw(&mut n, &cx);
    let (r, _) = n.checks[1];
    n.event(
        Event::PenUp {
            x: r.x + 10,
            y: r.y + 10,
        },
        &cx,
    );
    assert_eq!(note_text(pim).0, b"[ ] Milk\n[x] Stamps");
    draw(&mut n, &cx);
    let (r, _) = n.checks[1];
    n.event(
        Event::PenUp {
            x: r.x + 10,
            y: r.y + 10,
        },
        &cx,
    );
    assert_eq!(note_text(pim).0, b"[ ] Milk\n[ ] Stamps");
}

#[test]
fn typing_a_list_adds_items_and_return_starts_the_next() {
    let pim = store();
    let cx = ctx(pim);
    let mut n = opened(19_723);
    n.event(Event::Key(Key::SoftRight), &cx);
    draw(&mut n, &cx);
    // LIST, then "Eggs", Return, "Tea".
    n.event(
        Event::PenUp {
            x: tool_key(4).x + 10,
            y: STRIP.y + 20,
        },
        &cx,
    );
    assert_eq!(n.keyboard(), Some(Field::Sentence));
    for &c in b"Eggs" {
        n.event(Event::Char(c), &cx);
    }
    n.event(Event::Done, &cx);
    for &c in b"Tea" {
        n.event(Event::Char(c), &cx);
    }
    assert_eq!(note_text(pim).0, b"\x01\n[ ] Eggs\n[ ] Tea");
}

#[test]
fn ink_round_trips_with_each_stroke_s_style() {
    let a: &[(i16, i16)] = &[(0, 0), (10, 5), (300, 5), (300, 190)];
    let b: &[(i16, i16)] = &[(40, 40)];
    let mut out = [0u8; 256];
    let n = encode_styled(&[a, b], &[3, MARKER], &mut out).unwrap();
    let mut pts = [(0i16, 0i16); 64];
    let mut ends = [0u16; 8];
    let mut styles = [0u8; 8];
    let ns = decode_ink(&out[..n], &mut pts, &mut ends, &mut styles);
    assert_eq!(ns, 2);
    assert_eq!(&styles[..2], &[3, MARKER]);
    // The long steps were split but every point given is there.
    let first = &pts[..usize::from(ends[0])];
    for p in a {
        assert!(first.contains(p), "{p:?}");
    }
    assert_eq!(&pts[usize::from(ends[0])..usize::from(ends[1])], b);
    // Every cut of the field reads without a panic.
    for cut in 0..n {
        let _ = decode_ink(&out[..cut], &mut pts, &mut ends, &mut styles);
    }
}

#[test]
fn search_finds_by_title_and_text_and_the_menu_pins_papers_and_deletes() {
    let pim = store();
    {
        let mut g = pim.borrow_mut();
        let s = g.as_mut().unwrap();
        s.put(
            Kind::Note,
            None,
            &[(tag::TITLE, b"Garden"), (tag::TEXT, b"Plant tomatoes")],
        )
        .unwrap();
        s.put(
            Kind::Note,
            None,
            &[(tag::TITLE, b"Shopping"), (tag::TEXT, b"[ ] Milk")],
        )
        .unwrap();
    }
    let cx = ctx(pim);
    let mut n = opened(19_723);
    draw(&mut n, &cx);
    assert_eq!(n.n_cards, 2);
    n.event(
        Event::PenUp {
            x: SEARCH.x + 10,
            y: SEARCH.y + 10,
        },
        &cx,
    );
    for &c in b"toma" {
        n.event(Event::Char(c), &cx);
    }
    draw(&mut n, &cx);
    assert_eq!(n.n_cards, 1);
    // Open it: Menu, then PIN and PAPER (ruled to plain).
    let (r, _) = n.cards[0];
    n.event(
        Event::PenUp {
            x: r.x + 20,
            y: r.y + 20,
        },
        &cx,
    );
    draw(&mut n, &cx);
    n.event(Event::Key(Key::SoftRight), &cx);
    n.event(
        Event::PenUp {
            x: STRIP.x + 20,
            y: STRIP.y + 20,
        },
        &cx,
    );
    n.event(
        Event::PenUp {
            x: STRIP.x + 6 + 2 * 114 + 20,
            y: STRIP.y + 20,
        },
        &cx,
    );
    {
        let g = pim.borrow();
        let r = g.as_ref().unwrap().all(Kind::Note).next().unwrap();
        assert_eq!(r.num(tag::PINNED), Some(1));
        assert_eq!(r.num(tag::PAPER), Some(1));
        assert_eq!(r.text(tag::TEXT), b"Plant tomatoes");
    }
    // DELETE asks once, then deletes.
    let del = STRIP.x + 6 + 3 * 114 + 20;
    draw(&mut n, &cx);
    n.event(
        Event::PenUp {
            x: del,
            y: STRIP.y + 20,
        },
        &cx,
    );
    assert_eq!(pim.borrow().as_ref().unwrap().all(Kind::Note).count(), 2);
    n.event(
        Event::PenUp {
            x: del,
            y: STRIP.y + 20,
        },
        &cx,
    );
    assert_eq!(pim.borrow().as_ref().unwrap().all(Kind::Note).count(), 1);
}

/// The class of 2026-11-14: an edit that rewrote the note whole dropped
/// the fields it didn't list (the paper was lost on the next stroke).
#[test]
fn typing_and_ink_keep_the_paper_colour_pin_and_unknown_fields() {
    let pim = store();
    let cx = ctx(pim);
    let paper = 1u32.to_le_bytes();
    let colour = 4u32.to_le_bytes();
    let one = 1u32.to_le_bytes();
    let id = pim
        .borrow_mut()
        .as_mut()
        .unwrap()
        .put(
            Kind::Note,
            None,
            &[
                (tag::TITLE, b"Kept"),
                (tag::TEXT, b"a"),
                (tag::PAPER, &paper),
                (tag::COLOUR, &colour),
                (tag::PINNED, &one),
                (99, b"later"),
            ],
        )
        .unwrap();
    let n = opened(19_723);
    text_edit(&cx, id, &mut |text, tn| {
        text[*tn] = b'b';
        *tn += 1;
        true
    });
    n.set_num(&cx, id, tag::COLOUR, Some(2));
    let s = pim.borrow();
    let r = s.as_ref().unwrap().get(id).unwrap();
    assert_eq!(r.text(tag::TEXT), b"ab");
    assert_eq!(r.text(tag::TITLE), b"Kept");
    assert_eq!(r.num(tag::PAPER), Some(1));
    assert_eq!(r.num(tag::COLOUR), Some(2));
    assert_eq!(r.num(tag::PINNED), Some(1));
    assert_eq!(r.text(99), b"later");
}

/// A stroke from (`x0`, `y0`) going right `n` samples, `dy(k)` its rise.
fn stroke(n: &mut Notes, cx: &Ctx, x0: u16, y0: u16, samples: u16, dy: impl Fn(u16) -> u16) {
    n.event(Event::PenDown { x: x0, y: y0 }, cx);
    for k in 1..samples {
        n.event(
            Event::PenMove {
                x: x0 + k * 6,
                y: y0 + dy(k),
            },
            cx,
        );
    }
    n.event(
        Event::PenUp {
            x: x0 + samples * 6,
            y: y0,
        },
        cx,
    );
}

#[test]
fn the_palette_picks_colour_and_size_and_a_stroke_keeps_both() {
    // The owner, 2026-11-14: "the pen and marker colors are hard to
    // select", "there also is no stroke size control".
    let pim = store();
    let cx = ctx(pim);
    let mut n = opened(19_723);
    draw(&mut n, &cx);
    n.event(Event::Key(Key::SoftRight), &cx);
    draw(&mut n, &cx);
    // The third well (red), the bold size: one tap each.
    let (w, k) = (well(2), size_key(2));
    n.event(
        Event::PenUp {
            x: w.x + 20,
            y: w.y + 20,
        },
        &cx,
    );
    n.event(
        Event::PenUp {
            x: k.x + 20,
            y: k.y + 20,
        },
        &cx,
    );
    assert_eq!((n.pen, n.size, n.tool), (2, 2, Tool::Pen));
    let (ix, iy) = n.ink_at.expect("an ink panel on screen");
    // A straight stroke of 30 samples: kept as its ends (thinned), in the
    // colour and size chosen.
    stroke(&mut n, &cx, ix + 20, iy + 40, 30, |_| 0);
    let g = pim.borrow();
    let rec = g
        .as_ref()
        .unwrap()
        .all(Kind::Note)
        .next()
        .expect("the note");
    let mut pts = [(0i16, 0i16); POINTS];
    let mut ends = [0u16; STROKES];
    let mut styles = [0u8; STROKES];
    let ns = decode_ink(rec.text(tag::INK), &mut pts, &mut ends, &mut styles);
    assert_eq!(ns, 1);
    assert_eq!(styles[0], 2 | SIZE_BOLD);
    assert!(ends[0] <= 4, "a line is its ends, not {} points", ends[0]);
    assert_eq!(pts[0], (20, 40));
}

#[test]
fn a_curve_keeps_its_shape_when_thinned() {
    let mut pts = [(0i16, 0i16); LIVE];
    // A wave: 60 samples, 6 px apart, 12 px high.
    for (k, p) in pts.iter_mut().enumerate().take(60) {
        let up = [0, 6, 10, 12, 10, 6, 0, -6, -10, -12, -10, -6];
        *p = (k as i16 * 6, up[k % 12]);
    }
    let before = pts;
    let n = simplify(&mut pts, 60);
    assert!(n < 45, "{n}");
    // Every point left out lies within a pixel and a half of the kept line.
    for q in &before[..60] {
        let near = pts[..n].windows(2).any(|w| {
            let (ax, ay) = (i32::from(w[0].0), i32::from(w[0].1));
            let (dx, dy) = (i32::from(w[1].0) - ax, i32::from(w[1].1) - ay);
            let (px, py) = (i32::from(q.0) - ax, i32::from(q.1) - ay);
            let t = ((px * dx + py * dy) * 256 / (dx * dx + dy * dy).max(1)).clamp(0, 256);
            let (ex, ey) = (px - dx * t / 256, py - dy * t / 256);
            ex * ex + ey * ey <= 3
        });
        assert!(near, "{q:?} is off the thinned stroke");
    }
}

/// How many strokes the note and its ink pages hold, and how many pages;
/// every field whole strokes.
fn strokes_kept(pim: &core::cell::RefCell<Option<Store<'static>>>) -> (usize, usize) {
    let g = pim.borrow();
    let s = g.as_ref().unwrap();
    let mut count = 0;
    let note = s.all(Kind::Note).next().unwrap();
    for r in core::iter::once(note).chain(s.all(Kind::Ink)) {
        for field in [r.text(tag::INK), r.text(tag::INK_MORE)] {
            assert_eq!(split(field), field.len(), "a field ends mid-stroke");
            let mut at = 0;
            while let Some(n) = stroke_len(&field[at..]) {
                at += n;
                count += 1;
            }
        }
    }
    (count, s.all(Kind::Ink).count())
}

#[test]
fn ink_goes_on_past_one_record_and_a_full_note_says_so() {
    // The owner, 2026-11-14: "gets to the point where they dont show up at
    // all" (the first version held 48 strokes, the second a record's room;
    // now the ink goes on to pages of its own, R206).
    let pim = store();
    let cx = ctx(pim);
    let mut n = opened(19_723);
    draw(&mut n, &cx);
    n.event(Event::Key(Key::SoftRight), &cx);
    draw(&mut n, &cx);
    let (ix, iy) = n.ink_at.expect("an ink panel on screen");
    // Zigzags: nothing for the thinning to drop, some 60 bytes a stroke.
    let zig = |n: &mut Notes, k: u16| {
        stroke(
            n,
            &cx,
            ix + 10 + (k % 5) * 30,
            iy + 40 + (k % 30) * 4,
            28,
            |s| {
                if s % 2 == 0 { 0 } else { 30 }
            },
        );
    };
    // Past one record: no notice, a page begun, every stroke kept.
    let mut kept = 0u16;
    while strokes_kept(pim).1 < 2 {
        zig(&mut n, kept);
        kept += 1;
        assert!(!n.ink_full && kept < 400, "stroke {kept}");
    }
    assert!(kept > 60, "a second page at {kept} strokes");
    assert_eq!(strokes_kept(pim).0, usize::from(kept));
    // Undo takes the last stroke (the last page's; the page with it once
    // it is empty), the eraser one of the note's own.
    n.undo(&cx, n.open_id().unwrap());
    assert_eq!(strokes_kept(pim), (usize::from(kept) - 1, 1));
    n.tool = Tool::Erase;
    stroke(&mut n, &cx, ix + 10, iy + 40, 4, |_| 0);
    assert_eq!(strokes_kept(pim).0, usize::from(kept) - 2);
    n.tool = Tool::Pen;
    // To the last page's end: then it says so, and drops nothing kept.
    let mut more = 0u16;
    while !n.ink_full {
        zig(&mut n, more);
        more += 1;
        assert!(more < 4000);
    }
    let (count, pages) = strokes_kept(pim);
    assert_eq!(pages, PAGES);
    assert_eq!(count, usize::from(kept) - 2 + usize::from(more) - 1);
    // The eraser makes room: the notice goes.
    n.tool = Tool::Erase;
    stroke(&mut n, &cx, ix + 10, iy + 40, 4, |_| 0);
    assert!(!n.ink_full);
    assert_eq!(strokes_kept(pim).0, count - 1);
    // Deleted, its pages go with it.
    n.event(Event::Key(Key::SoftRight), &cx);
    for _ in 0..2 {
        n.event(
            Event::PenUp {
                x: STRIP.x + 6 + 3 * 114 + 20,
                y: STRIP.y + 20,
            },
            &cx,
        );
    }
    let g = pim.borrow();
    let s = g.as_ref().unwrap();
    assert_eq!(s.all(Kind::Note).count() + s.all(Kind::Ink).count(), 0);
}

/// `n` notes "Note 00".., the third pinned, the second a checklist.
fn many(pim: &core::cell::RefCell<Option<Store<'static>>>, n: u8) {
    let mut g = pim.borrow_mut();
    let s = g.as_mut().unwrap();
    let one = 1u32.to_le_bytes();
    for k in 0..n {
        let title = [b'N', b'o', b't', b'e', b' ', b'0' + k / 10, b'0' + k % 10];
        let text: &[u8] = if k == 1 {
            b"[ ] Milk\n[x] Stamps"
        } else {
            b"A line\nAnother"
        };
        let mut fields = vec![(tag::TITLE, &title[..]), (tag::TEXT, text)];
        if k == 2 {
            fields.push((tag::PINNED, &one));
        }
        s.put(Kind::Note, None, &fields).unwrap();
    }
}

#[test]
fn the_home_leads_with_the_pinned_sheet_and_its_tabs_filter() {
    let pim = store();
    many(pim, 4);
    let cx = ctx(pim);
    let mut n = opened(19_723);
    draw(&mut n, &cx);
    // The pinned note first, the whole width; the rest two columns.
    assert_eq!(n.n_cards, 4);
    let (sheet, place) = n.cards[0];
    assert_eq!((sheet.w, place), (WIDE, 2));
    assert!(
        n.cards[1..4]
            .iter()
            .all(|(r, _)| r.w < WIDE / 2 && r.y > sheet.y + sheet.h)
    );
    // The foot's tabs: Pinned shows it alone, Lists the checklist, Ink none.
    for (tab, cards) in [(1, 1), (2, 1), (3, 0), (0, 4)] {
        let r = kit::foot_tab(FILTERS.len(), tab);
        n.event(
            Event::PenDown {
                x: r.x + 20,
                y: r.y + 10,
            },
            &cx,
        );
        n.event(
            Event::PenUp {
                x: r.x + 20,
                y: r.y + 10,
            },
            &cx,
        );
        draw(&mut n, &cx);
        assert_eq!(n.n_cards, cards, "tab {tab}");
    }
    // The D-pad: down shows the first note chosen, down again the next,
    // the centre opens it; left and right step the tabs.
    n.event(Event::Key(Key::Down), &cx);
    n.event(Event::Key(Key::Down), &cx);
    n.event(Event::Key(Key::Enter), &cx);
    assert_eq!(n.view, View::Note(0));
    n.event(Event::Key(Key::SoftLeft), &cx);
    n.event(Event::Key(Key::Right), &cx);
    assert_eq!(n.filter, 1);
}

#[test]
fn every_note_is_within_reach_a_page_at_a_time() {
    // The grid left out what did not fit, with no way to it but search.
    let pim = store();
    many(pim, 23);
    let cx = ctx(pim);
    let mut n = opened(19_723);
    let mut seen = [false; 23];
    let mut pages = 0;
    loop {
        draw(&mut n, &cx);
        assert!(n.n_cards > 0);
        for &(_, place) in &n.cards[..n.n_cards] {
            assert!(!seen[usize::from(place)], "note {place} on two pages");
            seen[usize::from(place)] = true;
        }
        pages += 1;
        // A drag up turns the page; the last page stays.
        n.event(Event::PenDown { x: 240, y: 400 }, &cx);
        if n.event(Event::PenUp { x: 240, y: 300 }, &cx) == Reply::Nothing {
            break;
        }
    }
    assert!(seen.iter().all(|&s| s), "{seen:?}");
    assert!(pages > 2);
    // And back to the first, a drag down a page; the keys turn pages too.
    for _ in 1..pages {
        n.event(Event::PenDown { x: 240, y: 300 }, &cx);
        assert_eq!(n.event(Event::PenUp { x: 240, y: 400 }, &cx), Reply::Redraw);
    }
    assert_eq!(n.first, 0);
    draw(&mut n, &cx);
    let page = n.next;
    for _ in 0..=page {
        n.event(Event::Key(Key::Down), &cx);
        draw(&mut n, &cx);
    }
    assert_eq!((n.first, n.sel), (page, page));
}

#[test]
fn the_dpad_reaches_an_open_note_s_tools_colours_sizes_and_menu() {
    // R203: the D-pad in every screen of every app.
    let pim = store();
    let cx = ctx(pim);
    let mut n = opened(19_723);
    draw(&mut n, &cx);
    n.event(Event::Key(Key::SoftRight), &cx);
    draw(&mut n, &cx);
    assert!(matches!(n.view, View::Note(_)));
    // Every target is within the arrows' reach of the first.
    let reach = |n: &Notes| {
        let (t, k) = n.note_targets();
        let mut seen = std::vec![t[0]];
        let mut at = 0;
        while at < seen.len() {
            for key in [Key::Up, Key::Down, Key::Left, Key::Right] {
                let mut f = kit::Focus::default();
                f.set(seen[at]);
                if f.step(key, t[..k].iter().copied())
                    && let Some(r) = f.rect()
                    && !seen.contains(&r)
                {
                    seen.push(r);
                }
            }
            at += 1;
        }
        (seen.len(), k)
    };
    let (seen, all) = reach(&n);
    assert_eq!((seen, all), (16, 16));
    // The first arrow lands on the first colour well; right, and the
    // centre: the pen takes the second colour.
    n.event(Event::Key(Key::Down), &cx);
    assert_eq!(n.focus.rect(), Some(well(0)));
    n.event(Event::Key(Key::Right), &cx);
    n.event(Event::Key(Key::Enter), &cx);
    assert_eq!(n.pen, 1);
    // Down to the tools; the eraser by the centre.
    n.event(Event::Key(Key::Down), &cx);
    n.event(Event::Key(Key::Right), &cx);
    n.event(Event::Key(Key::Enter), &cx);
    assert!(
        n.tool == Tool::Erase || n.tool == Tool::Marker,
        "{:?}",
        n.tool
    );
    // The menu's keys take the arrows while it is up: PIN by the centre.
    n.event(Event::Key(Key::SoftRight), &cx);
    assert_eq!(reach(&n), (4, 4));
    n.event(Event::Key(Key::Down), &cx);
    n.event(Event::Key(Key::Enter), &cx);
    let g = pim.borrow();
    let r = g.as_ref().unwrap().all(Kind::Note).next().unwrap();
    assert_eq!(r.num(tag::PINNED), Some(1));
}

#[test]
fn a_search_s_match_is_found_for_the_highlighter() {
    assert_eq!(match_at(b"Plant tomatoes", b"TOMA"), Some(6));
    assert_eq!(match_at(b"Plant tomatoes", b"plant"), Some(0));
    assert_eq!(match_at(b"Plant", b"plants"), None);
    assert_eq!(match_at(b"Plant", b""), None);
}

#[test]
fn redo_puts_back_what_undo_took_strokes_and_typing() {
    let pim = store();
    let cx = ctx(pim);
    let mut n = opened(19_723);
    draw(&mut n, &cx);
    n.event(Event::Key(Key::SoftRight), &cx);
    draw(&mut n, &cx);
    let id = n.open_id().unwrap();
    let (ix, iy) = n.ink_at.expect("an ink panel on screen");
    let tap = |n: &mut Notes, k: usize| {
        let r = tool_key(k);
        n.event(
            Event::PenDown {
                x: r.x + 9,
                y: r.y + 9,
            },
            &cx,
        );
        n.event(
            Event::PenUp {
                x: r.x + 9,
                y: r.y + 9,
            },
            &cx,
        );
    };
    // The seven keys lie inside the strip, apart.
    for k in 0..7 {
        let r = tool_key(k);
        assert!(r.x >= STRIP.x && r.right() <= STRIP.right(), "{k}");
        assert!(k == 0 || tool_key(k - 1).right() < r.x);
    }
    // Three strokes; two undone, one put back, in order.
    for k in 0..3 {
        stroke(&mut n, &cx, ix + 10, iy + 20 + k * 30, 12, |s| {
            if s % 2 == 0 { 0 } else { 9 }
        });
    }
    let ink = |pim: &core::cell::RefCell<Option<Store<'static>>>| {
        let g = pim.borrow();
        g.as_ref().unwrap().get(id).unwrap().text(tag::INK).to_vec()
    };
    let three = ink(pim);
    tap(&mut n, 5);
    let two = ink(pim);
    tap(&mut n, 5);
    assert!(ink(pim).len() < two.len() && two.len() < three.len());
    tap(&mut n, 6);
    assert_eq!(ink(pim), two);
    tap(&mut n, 6);
    assert_eq!(ink(pim), three);
    // Nothing more to put back: REDO does nothing.
    tap(&mut n, 6);
    assert_eq!(ink(pim), three);
    // A new stroke forgets what was undone.
    tap(&mut n, 5);
    stroke(&mut n, &cx, ix + 200, iy + 20, 12, |_| 0);
    let now = ink(pim);
    tap(&mut n, 6);
    assert_eq!(ink(pim), now);
    // Typing: letters undone and put back.
    tap(&mut n, 3);
    for &c in b"Milk" {
        n.event(Event::Char(c), &cx);
    }
    let text = |pim: &core::cell::RefCell<Option<Store<'static>>>| {
        let g = pim.borrow();
        g.as_ref()
            .unwrap()
            .get(id)
            .unwrap()
            .text(tag::TEXT)
            .to_vec()
    };
    let typed = text(pim);
    assert!(typed.ends_with(b"Milk"));
    n.event(Event::Delete, &cx);
    n.event(Event::Delete, &cx);
    assert!(text(pim).ends_with(b"Mi"));
    assert_eq!(ink(pim), now, "backspace takes letters, not strokes");
    tap(&mut n, 6);
    tap(&mut n, 6);
    assert_eq!(text(pim), typed);
}
