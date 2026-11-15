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
            x: STRIP.x + 6 + 5 * 76 + 10,
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
            x: STRIP.x + 6 + 4 * 76 + 10,
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
    n.rewrite(&cx, id, |text, tn, _, _| {
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

#[test]
fn a_full_note_says_so_instead_of_dropping_strokes_in_silence() {
    // The owner, 2026-11-14: "gets to the point where they dont show up at
    // all" (the first version held 48 strokes; now a note's ink is its
    // record's room, a few hundred strokes).
    let pim = store();
    let cx = ctx(pim);
    let mut n = opened(19_723);
    draw(&mut n, &cx);
    n.event(Event::Key(Key::SoftRight), &cx);
    draw(&mut n, &cx);
    let (ix, iy) = n.ink_at.expect("an ink panel on screen");
    let mut kept = 0u16;
    while !n.ink_full {
        stroke(
            &mut n,
            &cx,
            ix + 10 + (kept % 7) * 40,
            iy + 4 + (kept % 40) * 4,
            4,
            |_| 0,
        );
        kept += 1;
        assert!(kept < 2000);
    }
    assert!(kept > 150, "full at {kept} strokes");
    // Both fields hold whole strokes, the first as much as it can.
    {
        let g = pim.borrow();
        let rec = g.as_ref().unwrap().all(Kind::Note).next().unwrap();
        let (a, b) = (rec.text(tag::INK), rec.text(tag::INK_MORE));
        assert_eq!(split(a), a.len());
        assert_eq!(split(b), b.len());
        assert!(a.len() > rp_pim::store::TEXT_MAX - 8 && !b.is_empty());
    }
    // The eraser makes room: the notice goes.
    n.tool = Tool::Erase;
    stroke(&mut n, &cx, ix + 10, iy + 4, 4, |_| 0);
    assert!(!n.ink_full);
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
