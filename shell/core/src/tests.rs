use rp_draw::{Glyph, MAX_CMDS};

use super::weather_settings::Item;
use super::*;

static G: Glyph = Glyph {
    w: 1,
    h: 1,
    left: 0,
    top: 1,
    advance: 8,
    cover: &[255],
};
static GLYPHS: [Glyph; 0xB1 - 0x20] = [G; 0xB1 - 0x20];
static FONT: Font = Font {
    line: 12,
    ascent: 10,
    first: 0x20,
    glyphs: &GLYPHS,
};

/// A picture for every slot: the frame holds as many commands as the
/// device's, whose pictures are all there (with none, a test counted fewer
/// commands than the X51v draws and missed Hourly's 2D frame overflowing:
/// 185 commands against 160).
static PIC: Image = Image {
    w: 4,
    h: 4,
    px: &[0; 16],
};

thread_local! {
    /// The store of the shell made last on this thread (the tests'
    /// renderers draw its pictures).
    static STORE: core::cell::Cell<Option<&'static rp_draw::pics::Pics<'static>>> =
        const { core::cell::Cell::new(None) };
}

/// The store of the shell made last.
fn store() -> &'static rp_draw::pics::Pics<'static> {
    STORE.with(core::cell::Cell::get).expect("a shell made")
}

/// A store with a 4 px picture for every condition at every size and room
/// for Hourly's graph (leaked: the shell's handles are checked against a
/// live store, as on the X51v).
fn every_pic() -> Icons {
    const CLASSES: [(u16, u16, u8); 2] = [
        (4, 4, 88),
        (weather_app::GRAPH_PIC.0, weather_app::GRAPH_PIC.1, 1),
    ];
    let store: &'static rp_draw::pics::Pics<'static> =
        Box::leak(Box::new(rp_draw::pics::Pics::new(
            Box::leak(vec![0; rp_draw::pics::Pics::words(&CLASSES)].into_boxed_slice()),
            &CLASSES,
        )));
    STORE.with(|s| s.set(Some(store)));
    let mut icons = Icons::NONE;
    for size in Size::ALL {
        for p in icons.of_mut(size) {
            *p = store.alloc(4, 4);
        }
    }
    icons.radar = [Some(&PIC); 3];
    icons
}

fn shell(dev: bool) -> Shell {
    let mut s = Shell::new(
        Fonts {
            ui: &FONT,
            label: &FONT,
            big: &FONT,
        },
        every_pic(),
        dev,
    );
    // Hourly's graph, painted as the device paints it.
    let pics = store();
    let (gw, gh) = weather_app::GRAPH_PIC;
    let graph = pics.alloc(gw, gh);
    if let Some(g) = graph {
        pics.write(g, |px| s.paint_graph(px));
    }
    s.set_graph(graph);
    s
}

fn tap(s: &mut Shell, r: Rect) -> Action {
    let (x, y) = (r.x + r.w / 2, r.y + r.h / 2);
    s.input(Input::PenDown { x, y });
    s.input(Input::PenUp { x, y })
}

fn drawn(s: &mut Shell) -> Frame {
    let mut f = Frame::new();
    s.frame(&mut f);
    f
}

#[test]
fn the_first_frame_is_the_whole_screen_and_fits() {
    let mut s = shell(true);
    assert!(s.needs_frame());
    let f = drawn(&mut s);
    assert_eq!(f.dirty(), &[Rect::new(0, 0, W, H)]);
    assert!(!f.overflowed());
    assert!(f.cmds().count() < MAX_CMDS);
    assert!(!s.needs_frame());
    // With the menu open, the busiest scene still fits.
    tap(&mut s, START);
    let f = drawn(&mut s);
    assert!(!f.overflowed(), "{} commands", f.cmds().count());
}

#[test]
fn start_opens_and_closes_the_menu() {
    let mut s = shell(false);
    drawn(&mut s);
    assert_eq!(tap(&mut s, START), Action::Redraw);
    assert!(s.menu_open());
    let f = drawn(&mut s);
    assert!(
        f.dirty()
            .iter()
            .any(|d| d.intersect(&s.menu_rect()) == s.menu_rect())
    );
    assert_eq!(tap(&mut s, START), Action::Redraw);
    assert!(!s.menu_open());
}

#[test]
fn a_tap_outside_the_menu_closes_it_and_does_nothing_else() {
    let mut s = shell(false);
    tap(&mut s, START);
    // Where a quick button is, under nothing of the menu? The menu covers
    // the left; the last quick button is at the right.
    let r = quick_rect(4);
    assert!(!s.menu_rect().contains_point(r.x + 10, r.y + 10));
    tap(&mut s, r);
    assert!(!s.menu_open());
    assert_eq!(s.notice(), None, "the tap only closed the menu");
}

#[test]
fn placeholders_say_not_yet_and_the_notice_goes() {
    let mut s = shell(false);
    s.tick(1000);
    tap(&mut s, quick_rect(0));
    assert_eq!(s.notice(), Some("Not yet: Notes"));
    drawn(&mut s);
    assert_eq!(s.tick(1000 + NOTICE_MS - 1), Action::Nothing);
    assert_eq!(s.tick(1000 + NOTICE_MS), Action::Redraw);
    assert_eq!(s.notice(), None);
    let f = drawn(&mut s);
    assert_eq!(f.dirty(), &[NOTICE]);
}

#[test]
fn a_tap_that_slides_off_its_target_does_nothing() {
    let mut s = shell(false);
    let r = quick_rect(1);
    s.input(Input::PenDown {
        x: r.x + 5,
        y: r.y + 5,
    });
    assert_eq!(s.input(Input::PenUp { x: 5, y: 300 }), Action::Redraw); // unpressed
    assert_eq!(s.notice(), None);
}

#[test]
fn back_to_loader_only_in_test_images() {
    let mut s = shell(false);
    tap(&mut s, START);
    assert_eq!(s.menu_len() as usize, MENU.len());
    let mut s = shell(true);
    tap(&mut s, START);
    let last = s.menu_len() - 1;
    assert_eq!(tap(&mut s, Shell::menu_entry(last)), Action::BackToLoader);
}

#[test]
fn menu_entries_close_the_menu_and_today_says_nothing() {
    let mut s = shell(false);
    tap(&mut s, START);
    tap(&mut s, Shell::menu_entry(0)); // Today
    assert!(!s.menu_open());
    assert_eq!(s.notice(), None);
    tap(&mut s, START);
    tap(&mut s, Shell::menu_entry(9)); // Settings
    assert_eq!(s.notice(), Some("Not yet: Settings"));
}

#[test]
fn the_d_pad_reaches_every_target() {
    let mut s = shell(false);
    // The first key press shows the focus on Start.
    s.input(Input::Key(Key::Down));
    assert_eq!(s.focus(), Some(Target::Start));
    // Down to the weather hero (straight below, though wide), down to the
    // quick row under its middle, across it, down to the softkeys.
    s.input(Input::Key(Key::Down));
    assert_eq!(s.focus(), Some(Target::Hero));
    s.input(Input::Key(Key::Down));
    assert_eq!(s.focus(), Some(Target::Quick(2)));
    s.input(Input::Key(Key::Left));
    s.input(Input::Key(Key::Left));
    assert_eq!(s.focus(), Some(Target::Quick(0)));
    for i in 1..5 {
        s.input(Input::Key(Key::Right));
        assert_eq!(s.focus(), Some(Target::Quick(i)));
    }
    // Past the edge: stays.
    s.input(Input::Key(Key::Right));
    assert_eq!(s.focus(), Some(Target::Quick(4)));
    s.input(Input::Key(Key::Down));
    assert_eq!(s.focus(), Some(Target::SoftRight));
    s.input(Input::Key(Key::Left));
    assert_eq!(s.focus(), Some(Target::SoftLeft));
    s.input(Input::Key(Key::Up));
    assert!(matches!(s.focus(), Some(Target::Quick(_))));
    // Enter acts on the focus.
    s.input(Input::Key(Key::Enter));
    assert!(s.notice().is_some());
}

#[test]
fn the_d_pad_walks_the_menu_and_home_goes_back() {
    let mut s = shell(true);
    s.input(Input::Key(Key::Enter)); // focus Start
    s.input(Input::Key(Key::Enter)); // open
    assert!(s.menu_open());
    assert_eq!(s.focus(), Some(Target::Menu(0)));
    for i in 1..s.menu_len() {
        s.input(Input::Key(Key::Down));
        assert_eq!(s.focus(), Some(Target::Menu(i)));
    }
    s.input(Input::Key(Key::Home));
    assert!(!s.menu_open());
    assert_eq!(s.focus(), None);
}

#[test]
fn a_new_minute_redraws_only_the_clocks() {
    let mut s = shell(false);
    let t = Time {
        hour: 9,
        minute: 41,
        weekday: 1,
        day: 13,
        month: 10,
    };
    s.set_time(t);
    drawn(&mut s);
    assert_eq!(s.set_time(t), Action::Nothing);
    s.set_time(Time { minute: 42, ..t });
    let f = drawn(&mut s);
    let area: u32 = f
        .dirty()
        .iter()
        .map(|r| u32::from(r.w) * u32::from(r.h))
        .sum();
    assert!(area < u32::from(W) * u32::from(H) / 8, "{:?}", f.dirty());
    assert!(f.dirty().contains(&TITLE_CLOCK));
}

#[test]
fn text_formats() {
    let mut b = [0; 32];
    let t = Time {
        hour: 0,
        minute: 0,
        weekday: 1,
        day: 13,
        month: 10,
    };
    assert_eq!(date_line(&mut b, t), b"TUE 13 OCTOBER");
    let t = Time {
        weekday: 6,
        day: 1,
        month: 1,
        ..t
    };
    assert_eq!(date_line(&mut b, t), b"SUN 1 JANUARY");
    // Out-of-range values are clamped, never a panic.
    let bad = Time {
        hour: 99,
        minute: 99,
        weekday: 99,
        day: 0,
        month: 0,
    };
    assert_eq!(date_line(&mut b, bad), b"SUN 1 JANUARY");
}

#[test]
fn many_changes_between_frames_merge() {
    let mut s = shell(false);
    drawn(&mut s);
    for i in 0..40u16 {
        s.input(Input::PenDown {
            x: i * 11,
            y: i * 15,
        });
        s.input(Input::PenUp {
            x: i * 11,
            y: i * 15,
        });
    }
    let f = drawn(&mut s);
    assert!(f.dirty().len() <= rp_draw::MAX_DIRTY);
}

/// Draws only what the shell marked dirty, frame after frame, and checks
/// the plane against a full redraw of the same state every time: a change
/// the shell forgets to mark shows up here.
#[test]
fn incremental_frames_match_full_redraws() {
    use rp_draw::Renderer;
    use rp_gfx::layer::{Layer, PlaneFormat};
    use rp_gfx::render::CpuRenderer;

    let (w, h) = (u32::from(W), u32::from(H));
    let mut px = vec![0; (w * h) as usize];
    let mut plane = vec![0u8; (w * h * 4) as usize];
    let mut inc = CpuRenderer::new(
        Layer::new(&mut px, w, h, w as usize).unwrap(),
        &mut plane,
        w as usize * 4,
        PlaneFormat::Argb8888,
    );
    let mut s = shell(true);
    inc.set_pics(store());
    let mut f = Frame::new();
    let mut seed = 0x1234_5678u32;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    let mut now = 0;
    // Every text run of the whole scene, placed: one that appears (new
    // words, or old ones moved) must lie inside what the frame marks
    // changed, even where the test font draws no pixels for it (the
    // key-entry count once didn't, 2026-10-30).
    let placed = |s: &Shell| {
        let mut g = Frame::new();
        s.scene(&mut g);
        g.cmds_clipped()
            .filter_map(|(c, clip)| match *c {
                Cmd::Text {
                    x, baseline, text, ..
                } => {
                    // The line box, without the bounds' 4 px overhang
                    // margin, inside its clip (what can show).
                    let b = c.bounds(&g);
                    let b = Rect::new(
                        b.x + 4,
                        b.y + 4,
                        b.w.saturating_sub(8),
                        b.h.saturating_sub(8),
                    )
                    .intersect(&clip);
                    Some((x, baseline, g.text_of(text).to_vec(), b))
                }
                _ => None,
            })
            .collect::<std::vec::Vec<_>>()
    };
    for step in 0..300 {
        let before = placed(&s);
        let r = next();
        let (x, y) = ((r % u32::from(W)) as u16, ((r >> 10) % u32::from(H)) as u16);
        match r % 7 {
            0 | 1 => {
                s.input(Input::PenDown { x, y });
                s.input(Input::PenUp { x, y });
            }
            2 => {
                s.input(Input::PenDown { x, y });
                s.input(Input::PenUp { x: y % W, y: x });
            }
            3 => {
                let k = [
                    Key::Up,
                    Key::Down,
                    Key::Left,
                    Key::Right,
                    Key::Enter,
                    Key::Home,
                ];
                s.input(Input::Key(k[(r >> 3) as usize % k.len()]));
            }
            4 => {
                now += 700;
                s.tick(now);
            }
            // Into the Weather app's lower screens and their keys (the
            // key-entry count once sat outside what it marked changed).
            6 if r & 0x100 == 0 => {
                s.input(Input::Key(Key::Home));
                let cap = |k: u32| Target::Cap(((r >> k) % 41) as u8);
                for t in [
                    Target::Hero,
                    Target::SoftRight,
                    Target::Row(if r & 0x1000 == 0 {
                        4
                    } else {
                        ((r >> 13) % 8) as u8
                    }),
                    cap(15),
                    cap(20),
                ] {
                    let rr = s.rect_of(t);
                    let (x, y) = (rr.x + rr.w / 2, rr.y + rr.h / 2);
                    s.input(Input::PenDown { x, y });
                    s.input(Input::PenUp { x, y });
                }
            }
            _ => {
                let t = Time {
                    hour: (r >> 4) as u8 % 24,
                    minute: (r >> 9) as u8 % 60,
                    weekday: (r >> 15) as u8 % 7,
                    day: 1 + (r >> 18) as u8 % 31,
                    month: 1 + (r >> 23) as u8 % 12,
                };
                s.set_time(t);
            }
        }
        s.frame(&mut f);
        for (x, y, t, b) in placed(&s) {
            if t.is_empty()
                || b.is_empty()
                || before
                    .iter()
                    .any(|(x0, y0, t0, _)| (*x0, *y0, t0) == (x, y, &t))
            {
                continue;
            }
            assert!(
                f.dirty().iter().any(|d| d.intersect(&b) == b),
                "step {step} ({:?}): {:?} at ({x}, {y}) is new but not marked changed",
                s.screen(),
                std::string::String::from_utf8_lossy(&t)
            );
        }
        inc.draw(&f);
        // The same state, drawn in full on a fresh plane.
        let mut px2 = vec![0; (w * h) as usize];
        let mut plane2 = vec![0u8; (w * h * 4) as usize];
        let mut full = CpuRenderer::new(
            Layer::new(&mut px2, w, h, w as usize).unwrap(),
            &mut plane2,
            w as usize * 4,
            PlaneFormat::Argb8888,
        );
        full.set_pics(store());
        let mut g = Frame::new();
        s.scene(&mut g);
        s.look(&mut g);
        g.damage(Rect::new(0, 0, W, H));
        full.draw(&g);
        let first = inc
            .plane()
            .iter()
            .zip(full.plane())
            .position(|(a, b)| a != b)
            .map(|i| ((i / 4) % W as usize, (i / 4) / W as usize));
        assert!(
            first.is_none(),
            "step {step} ({:?}, input {}): incremental differs from full at {first:?}",
            s.screen(),
            r % 7
        );
    }
}

/// A panel like the X51v's: raw X follows the vertical, raw Y the
/// horizontal mirrored. `bend` pushes the centre's raw X (a non-linear
/// panel).
fn raw_at(x: u16, y: u16, bend: bool) -> (u16, u16) {
    let mut rx = 99 + u32::from(y) * 4128 / 1000;
    let ry = 3719 - u32::from(x) * 7083 / 1000;
    if bend && (200..280).contains(&x) && (280..360).contains(&y) {
        rx += 120;
    }
    (rx as u16, ry as u16)
}

/// Holds the pen on screen point `(x, y)` for `n` readings, then lifts.
fn hold(s: &mut Shell, x: u16, y: u16, n: usize, bend: bool) -> Action {
    let (rx, ry) = raw_at(x, y, bend);
    for _ in 0..n {
        s.input(Input::Raw { rx, ry });
    }
    s.input(Input::RawUp)
}

fn open_align(s: &mut Shell) {
    tap(s, START);
    let entries = Shell::menu_entries(true);
    // "Align screen" is the second-to-last test-image entry.
    tap(s, Shell::menu_entry(entries - 2));
    assert!(s.aligning() && s.wants_raw());
}

#[test]
fn the_align_screen_finds_the_panel() {
    let mut s = shell(true);
    open_align(&mut s);
    for &(x, y) in &align::TARGETS {
        assert_eq!(hold(&mut s, x, y, align::HOLD, false), Action::Redraw);
    }
    let Action::Calibrated(cal) = hold(&mut s, align::CHECK.0, align::CHECK.1, align::HOLD, false)
    else {
        panic!("not calibrated");
    };
    assert!(!s.aligning());
    // Anywhere on screen now maps back within a pixel.
    for (x, y) in [(10, 10), (470, 630), (240, 500), (100, 200)] {
        let (rx, ry) = raw_at(x, y, false);
        let (mx, my) = cal.to_screen(rx, ry, (W, H));
        assert!(
            mx.abs_diff(x) <= 1 && my.abs_diff(y) <= 1,
            "{x},{y} -> {mx},{my}"
        );
    }
    // The frames while it ran fit.
    let f = drawn(&mut s);
    assert!(!f.overflowed());
}

#[test]
fn the_align_screen_wants_a_real_hold() {
    let mut s = shell(true);
    open_align(&mut s);
    let (x, y) = align::TARGETS[0];
    hold(&mut s, x, y, align::HOLD - 1, false);
    assert!(s.aligning());
    // Still on the first target, and it says why.
    let mut f = Frame::new();
    s.frame(&mut f);
    assert!(f.cmds().count() > 0);
    hold(&mut s, x, y, align::HOLD, false);
    let (x, y) = align::TARGETS[1];
    hold(&mut s, x, y, align::HOLD, false);
}

#[test]
fn a_bent_panel_or_a_missed_check_starts_again() {
    let mut s = shell(true);
    open_align(&mut s);
    for &(x, y) in &align::TARGETS {
        hold(&mut s, x, y, align::HOLD, true);
    }
    // The corners' fit misses the bent centre: back to the first target.
    assert!(s.aligning());
    for &(x, y) in &align::TARGETS {
        hold(&mut s, x, y, align::HOLD, false);
    }
    // The check tap far from the dot: once more.
    assert_eq!(hold(&mut s, 50, 600, align::HOLD, false), Action::Redraw);
    assert!(s.aligning());
}

#[test]
fn home_leaves_the_align_screen_unchanged() {
    let mut s = shell(true);
    open_align(&mut s);
    assert_eq!(s.input(Input::Key(Key::Home)), Action::Redraw);
    assert!(!s.aligning());
    // Raw readings outside it do nothing.
    assert_eq!(s.input(Input::Raw { rx: 1, ry: 1 }), Action::Nothing);
}

fn overlap(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

#[test]
fn the_today_layout_fits_the_screen_and_nothing_overlaps() {
    let mut boxes = vec![TITLE, HERO, FORECAST_ROW, CALENDAR_PANEL, SOFTKEYS];
    // The cards sit inside the row, below the icons' headroom.
    let card = Rect::new(FORECAST_ROW.x, FORECAST_TOP, FORECAST_ROW.w, FORECAST_H);
    assert!(FORECAST_ROW.union(&card) == FORECAST_ROW);
    boxes.extend((0..5).map(quick_rect));
    for r in &boxes {
        assert!(r.x + r.w <= W && r.y + r.h <= H, "{r:?} leaves the screen");
    }
    for (i, a) in boxes.iter().enumerate() {
        for b in &boxes[i + 1..] {
            assert!(!overlap(*a, *b), "{a:?} overlaps {b:?}");
        }
    }
    // The date sits inside the hero; the five forecast cards inside their row.
    assert!(HERO.union(&DATE_BOX) == HERO);
    const { assert!(4 * FORECAST_STEP + FORECAST_W <= FORECAST_ROW.w) };
    // The weather icon's box reaches over the hero's top right, as in the
    // mock-ups, but stays on the screen.
    const { assert!(HERO_ICON.x + HERO_ICON.w <= W) };
}

#[test]
fn a_new_day_redraws_the_date_and_forecast_names_not_the_screen() {
    let mut s = shell(false);
    drawn(&mut s);
    let monday = Time {
        hour: 23,
        minute: 59,
        weekday: 0,
        day: 12,
        month: 10,
    };
    s.set_time(monday);
    drawn(&mut s);
    s.set_time(Time {
        hour: 0,
        minute: 0,
        weekday: 1,
        day: 13,
        ..monday
    });
    let f = drawn(&mut s);
    for want in [TITLE_CLOCK, DATE_BOX, FORECAST_ROW] {
        assert!(
            f.dirty().contains(&want),
            "{want:?} not redrawn: {:?}",
            f.dirty()
        );
    }
    // A minute within the day redraws neither of the day's boxes.
    s.set_time(Time {
        minute: 1,
        weekday: 1,
        day: 13,
        hour: 0,
        ..monday
    });
    let f = drawn(&mut s);
    assert!(!f.dirty().contains(&DATE_BOX) && !f.dirty().contains(&FORECAST_ROW));
}

/// Every text the frame draws.
fn texts(f: &Frame) -> Vec<Vec<u8>> {
    f.cmds()
        .filter_map(|c| match *c {
            Cmd::Text { text, .. } => Some(f.text_of(text).to_vec()),
            _ => None,
        })
        .collect()
}

#[test]
fn the_title_clock_is_twelve_hour_with_am_and_pm() {
    let mut s = shell(false);
    s.set_time(Time {
        hour: 14,
        minute: 5,
        ..Time::default()
    });
    assert!(texts(&drawn(&mut s)).contains(&b"2:05 PM".to_vec()));
}

#[test]
fn the_hero_and_days_read_the_forecast_including_frost_and_heat() {
    let mut s = shell(false);
    let f = drawn(&mut s);
    let t = texts(&f);
    assert!(t.contains(&b"72\xb0".to_vec()), "{t:?}");
    assert!(
        t.iter()
            .any(|x| x.starts_with(b"High 75\x7f  Low 52\x7f  Feels ")),
        "{t:?}"
    );
    assert!(t.contains(&b"73\x7f 52\x7f".to_vec()));
    // A winter morning and a desert afternoon: no leading zeros, no caps.
    s.wx.now.temp = -70;
    s.wx.days[1].high = 463;
    s.wx.days[1].low = 50;
    s.dirty(Rect::new(0, 0, W, H));
    let t = texts(&drawn(&mut s));
    assert!(t.contains(&b"19\xb0".to_vec()), "{t:?}");
    assert!(t.contains(&b"115\x7f 41\x7f".to_vec()), "{t:?}");
    s.region = Region::METRIC;
    s.wx.now.temp = -70;
    s.dirty(Rect::new(0, 0, W, H));
    assert!(texts(&drawn(&mut s)).contains(&b"-7\xb0".to_vec()));
}

fn weather(page: u8) -> Shell {
    let mut s = shell(true);
    drawn(&mut s);
    tap(&mut s, HERO);
    assert_eq!(s.screen(), Screen::Weather);
    s.settle();
    if page > 0 {
        tap(&mut s, weather_app::tab_rect(page));
        s.settle();
    }
    s
}

#[test]
fn the_hero_opens_weather_and_home_comes_back() {
    let mut s = weather(0);
    assert_eq!(s.weather_page(), (0, 0));
    assert_eq!(s.input(Input::Key(Key::Home)), Action::Redraw);
    assert_eq!(s.screen(), Screen::Today);
    // The D-pad reaches the hero from Start, and Enter opens it.
    s.input(Input::Key(Key::Enter));
    s.input(Input::Key(Key::Down));
    assert_eq!(s.focus(), Some(Target::Hero));
    s.input(Input::Key(Key::Enter));
    assert_eq!(
        (s.screen(), s.focus()),
        (Screen::Weather, Some(Target::Tab(0)))
    );
    // The Start menu's Today entry also comes home.
    tap(&mut s, START);
    tap(&mut s, Shell::menu_entry(0));
    assert_eq!(s.screen(), Screen::Today);
}

#[test]
fn every_page_fits_a_frame_even_with_the_menu_open_and_while_moving() {
    for page in 0..3 {
        let mut s = weather(page);
        let f = drawn(&mut s);
        assert!(
            !f.overflowed(),
            "page {page}: {} commands",
            f.cmds().count()
        );
        assert!(f.cmds().count() < MAX_CMDS);
        tap(&mut s, START);
        let f = drawn(&mut s);
        assert!(
            !f.overflowed(),
            "page {page} with the menu: {}",
            f.cmds().count()
        );
        tap(&mut s, START);
        drawn(&mut s);
        // Mid-entrance, every command is still drawn.
        s.set_tab((page + 1) % 3);
        s.tick(s.now + 100);
        assert!(!drawn(&mut s).overflowed());
        // Pulled past the end and scrolled: still every command.
        s.input(Input::PenDown { x: 240, y: 200 });
        s.input(Input::PenMove { x: 240, y: 560 });
        assert!(!drawn(&mut s).overflowed());
        s.input(Input::PenUp { x: 240, y: 560 });
    }
}

#[test]
fn the_entrance_runs_redraws_the_page_and_ends_in_place() {
    let mut s = shell(false);
    s.tick(1_000);
    drawn(&mut s);
    tap(&mut s, HERO);
    assert!(s.animating());
    assert_eq!(s.ease(0), 0, "it starts from the side");
    drawn(&mut s);
    assert_eq!(s.tick(1_100), Action::Redraw);
    let f = drawn(&mut s);
    assert!(f.dirty().contains(&weather_app::PAGE));
    let mid = s.ease(0);
    assert!(mid > 0 && mid < 1024, "{mid}");
    // Later items start later.
    assert!(s.ease(5) < mid);
    s.tick(1_000 + weather_app::ANIM_MS);
    assert!(!s.animating());
    assert_eq!(s.ease(9), 1024);
    // The count-up ends on the reading.
    assert!(texts(&drawn(&mut s)).contains(&b"72\xb0".to_vec()));
    // The clock wrapping round mid-entrance doesn't stick it.
    s.now = u32::MAX - 50;
    s.set_tab(1);
    s.tick((u32::MAX - 50).wrapping_add(weather_app::ANIM_MS));
    assert!(!s.animating());
}

#[test]
fn tabs_turn_by_tap_d_pad_and_swipe_and_stop_at_the_ends() {
    let mut s = weather(0);
    s.input(Input::Key(Key::Right));
    assert_eq!(s.weather_page().0, 1);
    for _ in 0..3 {
        s.input(Input::Key(Key::Right));
    }
    assert_eq!(s.weather_page().0, weather_app::RADAR, "the last page");
    s.input(Input::Key(Key::Left));
    s.input(Input::Key(Key::Left));
    assert_eq!(s.weather_page().0, 1);
    // A swipe left on the page turns forward; a short one is a tap.
    s.input(Input::PenDown { x: 400, y: 300 });
    s.input(Input::PenUp { x: 320, y: 310 });
    assert_eq!(s.weather_page().0, 2);
    s.input(Input::PenDown { x: 300, y: 300 });
    s.input(Input::PenUp { x: 260, y: 300 });
    assert_eq!(s.weather_page().0, 2);
    s.input(Input::PenDown { x: 100, y: 300 });
    s.input(Input::PenUp { x: 200, y: 300 });
    assert_eq!(s.weather_page().0, 1);
    // A stroke starting on the tabs is not a swipe.
    s.input(Input::PenDown {
        x: 400,
        y: weather_app::TABS.y + 10,
    });
    s.input(Input::PenUp {
        x: 200,
        y: weather_app::TABS.y + 10,
    });
    assert_eq!(s.weather_page().0, 1);
}

#[test]
fn the_hourly_graph_fits_and_a_tap_on_it_picks_the_hour_for_the_card() {
    let mut s = weather(1);
    assert!(
        weather_app::max_scroll(1) > 300,
        "the 24-hour grid to scroll to"
    );
    assert_eq!(s.hour_pick, 0);
    let g = weather_app::HOURLY_GRAPH;
    // The seventh hour's column (12 hours, 35 px each from x 30).
    tap(&mut s, Rect::new(30 + 6 * 35, g.y + 60, 35, 10));
    assert_eq!(s.hour_pick, 6);
    let f = drawn(&mut s);
    assert!(!f.overflowed(), "{}", f.cmds().count());
    let rain = rp_weather::Line::new()
        .push(b"Humidity ")
        .num(i32::from(s.wx.hours[6].humidity))
        .push(b"%")
        .push(weather_app::SEP)
        .push(b"dew point ")
        .push(s.region.temp(s.wx.hours[6].dew).as_bytes());
    assert!(texts(&f).contains(&rain.as_bytes().to_vec()));
    // Off the graph, a tap changes nothing.
    tap(&mut s, Rect::new(240, 450, 2, 2));
    assert_eq!(s.hour_pick, 6);
}

#[test]
fn a_drag_on_a_page_redraws_only_the_page_and_springs_back() {
    // 10 Day fits its page: a drag gives way and springs back.
    let mut s = weather(2);
    s.tick(1_000);
    drawn(&mut s);
    s.input(Input::PenDown { x: 240, y: 400 });
    // Inside the slop it may still be a tap: nothing moves.
    s.input(Input::PenMove { x: 241, y: 395 });
    assert_eq!(s.weather_page().1, 0);
    assert!(!s.needs_frame());
    // Past it, the page gives way (it fits: a rubber band).
    for (k, y) in [380u16, 352, 300].into_iter().enumerate() {
        s.tick(1_000 + 7 * (k as u32 + 1));
        s.input(Input::PenMove { x: 241, y });
        assert!(s.weather_page().1 > 0, "sample {k}");
        let f = drawn(&mut s);
        assert_eq!(f.dirty(), &[weather_app::PAGE][..], "sample {k}");
    }
    s.tick(1_200);
    s.input(Input::PenUp { x: 241, y: 300 });
    for k in 1..60 {
        s.tick(1_200 + 50 * k);
    }
    assert_eq!(s.weather_page().1, 0, "back in place");
    assert!(!s.animating());
}

#[test]
fn pulled_past_either_end_it_gives_way_then_springs_back() {
    let mut s = weather(1);
    // Down at the top: a third as far, then back to 0.
    s.tick(1_000);
    s.input(Input::PenDown { x: 240, y: 150 });
    s.tick(1_010);
    s.input(Input::PenMove { x: 240, y: 300 });
    assert_eq!(s.weather_page().1, -50);
    s.tick(1_400);
    s.input(Input::PenUp { x: 240, y: 300 });
    assert!(s.animating());
    let mut last = s.weather_page().1;
    for k in 1..=60 {
        s.tick(1_400 + 10 * k);
        let now = s.weather_page().1;
        assert!(
            now >= last && now <= 0,
            "no wobble past the end: {last} -> {now}"
        );
        last = now;
    }
    assert_eq!(s.weather_page().1, 0);
    assert!(!s.animating());
    // A page that fits gives way too (the owner missed the rebound).
    s.set_tab(2);
    s.tick(5_000);
    s.input(Input::PenDown { x: 240, y: 400 });
    s.tick(5_010);
    s.input(Input::PenMove { x: 240, y: 250 });
    assert_eq!(s.weather_page().1, 50);
    s.tick(5_300);
    s.input(Input::PenUp { x: 240, y: 250 });
    for k in 1..=60 {
        s.tick(5_300 + 10 * k);
    }
    assert_eq!(s.weather_page().1, 0);
    // The band never shows more than 120 px however far it's pulled.
    s.tick(9_000);
    s.input(Input::PenDown { x: 240, y: 100 });
    s.tick(9_010);
    s.input(Input::PenMove { x: 240, y: 600 });
    assert!(s.weather_page().1 >= -120);
    s.input(Input::PenUp { x: 240, y: 600 });
}

#[test]
fn a_press_that_moves_is_no_tap_and_loses_its_press_state() {
    let mut s = weather(1);
    // Down on a tab, slid off it: unpressed, the lift does nothing.
    let tab = weather_app::tab_rect(2);
    s.input(Input::PenDown {
        x: tab.x + 20,
        y: tab.y + 10,
    });
    assert_eq!(s.pressed, Some(Target::Tab(2)));
    s.input(Input::PenMove {
        x: tab.x + 20,
        y: tab.y - 30,
    });
    assert_eq!(s.pressed, None);
    s.input(Input::PenUp {
        x: tab.x + 20,
        y: tab.y + 10,
    });
    assert_eq!(s.weather_page().0, 1);
}

#[test]
fn a_swipe_moves_the_underline_as_it_goes_and_turns_the_page_on_lift() {
    let mut s = weather(1);
    s.tick(1_000);
    drawn(&mut s);
    s.input(Input::PenDown { x: 400, y: 300 });
    s.input(Input::PenMove { x: 350, y: 302 });
    assert_eq!(s.swipe_dx(), -50);
    let f = drawn(&mut s);
    assert!(f.dirty().contains(&weather_app::TABS));
    // Short of the turn, back: nothing turns, nothing scrolled.
    s.input(Input::PenMove { x: 380, y: 300 });
    s.input(Input::PenUp { x: 380, y: 300 });
    assert_eq!(s.weather_page(), (1, 0));
    s.input(Input::PenDown { x: 400, y: 300 });
    s.input(Input::PenMove { x: 330, y: 305 });
    s.input(Input::PenUp { x: 300, y: 305 });
    assert_eq!(s.weather_page(), (2, 0));
}

#[test]
fn back_goes_up_a_screen_by_header_softkey_or_home() {
    // The Weather app: the header's control goes back; the left softkey
    // refreshes (T4).
    let mut s = weather(1);
    tap(&mut s, weather_settings::BACK);
    assert_eq!(s.screen(), Screen::Today);
    let mut s = weather(1);
    tap(&mut s, SOFT_LEFT);
    assert_eq!(s.screen(), Screen::Weather);
    assert!(s.wants(Ask::Forecast));
    // Menu, then Add a place, then back up one at a time.
    let mut s = weather(0);
    tap(&mut s, SOFT_RIGHT);
    assert_eq!(s.screen(), Screen::WeatherSettings);
    tap_item(&mut s, Item::AddPlace);
    assert_eq!(s.screen(), Screen::AddPlace);
    tap(&mut s, weather_settings::BACK);
    assert_eq!(s.screen(), Screen::WeatherSettings);
    tap_item(&mut s, Item::Forecast);
    assert_eq!(s.screen(), Screen::WeatherFrom);
    tap(&mut s, SOFT_LEFT);
    assert_eq!(s.screen(), Screen::WeatherSettings);
    tap(&mut s, SOFT_LEFT);
    assert_eq!(s.screen(), Screen::Weather);
    // Home from anywhere.
    tap(&mut s, SOFT_RIGHT);
    s.input(Input::Key(Key::Home));
    assert_eq!(s.screen(), Screen::Today);
}

/// Taps target `t`.
fn tap_t(s: &mut Shell, t: Target) -> Action {
    let r = s.rect_of(t);
    tap(s, r)
}

/// Taps the first row that is `it`.
fn tap_item(s: &mut Shell, it: Item) -> Action {
    let t = row_of(s, it);
    tap_t(s, t)
}

/// The target of the first row that is `it` on the screen up.
fn row_of(s: &Shell, it: Item) -> Target {
    let (items, n) = s.items();
    let k = items[..usize::from(n)]
        .iter()
        .position(|i| *i == it)
        .unwrap_or_else(|| panic!("{it:?} not on {:?}", s.screen()));
    Target::Row(k as u8)
}

#[test]
fn settings_rows_change_what_every_screen_shows_and_are_kept() {
    let mut s = weather(0);
    tap(&mut s, SOFT_RIGHT);
    drawn(&mut s);
    // Units: the next region and its units, every number redrawn, kept.
    tap_item(&mut s, Item::Units);
    assert_eq!(s.wset.region_info().name, "Canada");
    assert_eq!(s.region.temp, rp_weather::Temp::C);
    assert!(drawn(&mut s).dirty().contains(&Rect::new(0, 0, W, H)));
    assert!(s.wants(Ask::Save));
    // Appearance: Auto, Light, Dark.
    tap_item(&mut s, Item::Appearance);
    assert_eq!(s.wset.appearance, rp_weather::settings::Appearance::Light);
    // The provider: picked on T3, the forecast asked for again.
    tap_item(&mut s, Item::Forecast);
    tap_item(&mut s, Item::Provider(2));
    assert_eq!(s.wset.provider, rp_weather::settings::Provider::MetNo);
    assert!(s.wants(Ask::Forecast));
    // The save carries all of it, never the key.
    let mut q = None;
    while let Some(next) = s.next_ask() {
        let a = next.ask;
        if a == Ask::Save {
            q = Some(next);
            break;
        }
        s.net_answer(false, b"");
    }
    let q = q.expect("a save");
    assert_eq!(q.line(), b"RPNET SAVE 825\n");
    let back = rp_weather::store::decode(
        &q.body.expect("the record"),
        rp_weather::settings::Secret::NONE,
    );
    assert_eq!(back.provider, rp_weather::settings::Provider::MetNo);
    assert_eq!(back.region, 1);
}

/// Taps the keys for `text` (Shift before a capital; the symbols layer
/// for what only it has).
fn type_keys(s: &mut Shell, text: &[u8]) {
    let find = |s: &Shell, cap: keyboard::Cap| {
        (0..keyboard::CAPS)
            .find(|&i| s.kb.cap(i).is_some_and(|(c, _)| c == cap))
            .expect("the key")
    };
    for &c in text {
        let low = c.to_ascii_lowercase();
        if s.kb.cap_of(low).is_none() && s.kb.cap_of(c).is_none() {
            let sym = find(s, keyboard::Cap::Symbols);
            tap_t(s, Target::Cap(sym));
            let i = s.kb.cap_of(c).expect("a key for every character");
            tap_t(s, Target::Cap(i));
            let sym = find(s, keyboard::Cap::Symbols);
            tap_t(s, Target::Cap(sym));
            continue;
        }
        if c.is_ascii_uppercase() {
            let shift = find(s, keyboard::Cap::Shift);
            tap_t(s, Target::Cap(shift));
        }
        let i = s.kb.cap_of(c).expect("a key for every character");
        tap_t(s, Target::Cap(i));
    }
}

#[test]
fn the_tempest_key_is_typed_masked_kept_and_never_drawn_in_full() {
    let key = b"0a1B2c3d-4e5f-6789-abcd-ef0123456789";
    let mut s = weather(0);
    tap(&mut s, SOFT_RIGHT);
    tap_item(&mut s, Item::Station);
    tap_item(&mut s, Item::StationKind(0));
    assert_eq!(s.screen(), Screen::KeyEntry);
    type_keys(&mut s, key);
    assert_eq!(&s.typed[..s.typed_n], key);
    // Masked as typed: only the last four characters drawn.
    let shown = texts(&drawn(&mut s));
    assert!(
        shown
            .iter()
            .all(|t| !t.windows(8).any(|w| key.windows(8).any(|k| k == w)))
    );
    assert!(shown.iter().any(|t| t.ends_with(b"6789")));
    // Done keeps it, connects the station and zeroes the typed copy.
    tap(&mut s, SOFT_RIGHT);
    assert_eq!(s.screen(), Screen::WeatherFrom);
    assert_eq!(s.wset.tempest_key.expose(), key);
    assert_eq!(s.wset.station, rp_weather::settings::Station::Tempest);
    assert!(s.typed.iter().all(|&b| b == 0) && s.typed_n == 0);
    // Typed again and cancelled: the kept key stays, the typed one goes.
    tap_item(&mut s, Item::StationKind(0));
    type_keys(&mut s, b"abc");
    tap(&mut s, SOFT_LEFT);
    assert_eq!(s.wset.tempest_key.expose(), key);
    assert!(s.typed.iter().all(|&b| b == 0));
    // Home half way through forgets it too.
    tap_item(&mut s, Item::StationKind(0));
    type_keys(&mut s, b"xyz");
    s.input(Input::Key(Key::Home));
    assert!(s.typed.iter().all(|&b| b == 0) && s.typed_n == 0);
}

#[test]
fn every_new_screen_fits_a_frame_and_the_d_pad_reaches_its_targets() {
    for screen in [
        Screen::WeatherSettings,
        Screen::WeatherPlaces,
        Screen::AddPlace,
        Screen::WeatherFrom,
        Screen::KeyEntry,
    ] {
        let mut s = weather(0);
        tap(&mut s, SOFT_RIGHT);
        s.open(screen);
        assert_eq!(s.screen(), screen);
        let f = drawn(&mut s);
        assert!(!f.overflowed(), "{screen:?}: {}", f.cmds().count());
        // Every row on the screen, above the keyboard and the softkeys.
        for k in 0..s.rows() {
            let r = s.rect_of(Target::Row(k));
            let floor = if s.has_keyboard() {
                keyboard::PANEL.y
            } else {
                H - 40
            };
            assert!(r.h > 0 && r.y + r.h <= floor, "{screen:?} row {k}: {r:?}");
        }
        // The D-pad walks from Start to every row or key and back.
        s.input(Input::Key(Key::Enter));
        let mut seen = std::vec::Vec::new();
        for k in [Key::Down; 12].into_iter().chain([Key::Right; 10]) {
            s.input(Input::Key(k));
            seen.extend(s.focus());
        }
        assert!(
            seen.iter()
                .any(|t| matches!(t, Target::Row(_) | Target::Cap(_))),
            "{screen:?}: {seen:?}"
        );
    }
}

/// The recorded answers (shell/weather/tests/data).
const SEATTLE_JSON: &[u8] = include_bytes!("../../weather/tests/data/open-meteo-seattle.json");
const PERRYVILLE_JSON: &[u8] =
    include_bytes!("../../weather/tests/data/open-meteo-find-perryville.json");

#[test]
fn the_dock_flow_loads_fetches_searches_adds_and_refreshes() {
    let mut s = weather(0);
    // At start: the stored settings; none (no dock answer) gives the
    // default place's forecast, from Open-Meteo, by position only.
    let q = s.next_ask().expect("load");
    assert_eq!(q.line(), b"RPNET LOAD\n");
    assert!(s.next_ask().is_none(), "one question at a time");
    s.net_answer(false, b"");
    let q = s.next_ask().expect("the forecast");
    assert_eq!(q.line(), b"RPNET OPENMETEO 47.606 -122.332\n");
    s.net_answer(true, SEATTLE_JSON);
    assert_eq!((s.wx.now.temp, s.wx.source), (61, "Open-Meteo"));
    assert_eq!(s.read_at().as_bytes(), b"12:00 AM");
    s.tick(s.now + 5 * 60_000);
    assert_eq!(s.read_at().as_bytes(), b"11:55 PM");
    // Add a place: typed, searched once typing pauses, a match tapped.
    tap(&mut s, SOFT_RIGHT);
    tap_item(&mut s, Item::AddPlace);
    type_keys(&mut s, b"Perryv");
    assert!(!s.wants(Ask::Find), "not while typing");
    let now = s.now;
    s.tick(now + weather_settings::SEARCH_PAUSE_MS + 1);
    while let Some(q) = s.next_ask() {
        if q.ask == Ask::Find {
            assert_eq!(q.line(), b"RPNET FIND Perryv\n");
            s.net_answer(true, PERRYVILLE_JSON);
            break;
        }
        s.net_answer(true, b"");
    }
    assert!(s.n_found >= 3);
    assert!(texts(&drawn(&mut s)).contains(&b"Missouri, US".to_vec()));
    tap_item(&mut s, Item::Match(0));
    assert_eq!(s.screen(), Screen::WeatherSettings);
    assert_eq!(
        (s.wset.n_places, s.wset.place().name.as_str()),
        (2, "Perryville")
    );
    assert_eq!(s.wx.place.as_str(), "Perryville");
    let mut fetched = false;
    while let Some(q) = s.next_ask() {
        if q.ask == Ask::Forecast {
            assert_eq!(q.line(), b"RPNET OPENMETEO 37.724 -89.861\n");
            fetched = true;
        }
        s.net_answer(false, b"");
    }
    assert!(fetched);
    // A hostile search answer finds nothing and says so.
    s.open(Screen::AddPlace);
    type_keys(&mut s, b"zz");
    s.tick(s.now + 1000);
    while let Some(q) = s.next_ask() {
        let a = q.ask;
        s.net_answer(
            true,
            if a == Ask::Find {
                b"\xff{\"results\":[{"
            } else {
                b""
            },
        );
    }
    assert_eq!((s.n_found, s.find_status), (0, "Nothing found"));
}

#[test]
fn a_pull_past_the_top_refreshes_and_a_scroll_does_not() {
    let mut s = weather(0);
    while s.next_ask().is_some() {
        s.net_answer(false, b"");
    }
    // A scroll up the page: no refresh.
    s.input(Input::PenDown { x: 240, y: 500 });
    for y in (300..500).rev().step_by(20) {
        s.input(Input::PenMove { x: 240, y });
    }
    s.input(Input::PenUp { x: 240, y: 300 });
    assert!(!s.wants(Ask::Forecast));
    s.settle();
    s.scroll.set(0);
    // Pulled down from the top well past the rebound: refresh.
    s.input(Input::PenDown { x: 240, y: 200 });
    for y in (200..440).step_by(20) {
        s.input(Input::PenMove { x: 240, y });
    }
    s.input(Input::PenUp { x: 240, y: 440 });
    assert!(s.wants(Ask::Forecast));
}

#[test]
fn pictures_replace_the_outlines_and_still_fit_a_frame() {
    let mut s = shell(true);
    let f = drawn(&mut s);
    let images = f.cmds().filter(|c| matches!(c, Cmd::Pic { .. })).count();
    assert_eq!(images, 6);
    assert!(!f.overflowed());
    // Without pictures the same places draw outlines and no images.
    let mut none = shell(true);
    none.set_icons(Icons::NONE);
    let f = drawn(&mut none);
    assert_eq!(f.cmds().filter(|c| matches!(c, Cmd::Pic { .. })).count(), 0);
}

#[test]
fn the_wallpaper_comparison_switches_depth_and_shows_the_wallpaper_alone() {
    // Only in test images, and the test image's menu stays above the
    // softkeys.
    let s = shell(false);
    assert!((0..s.menu_len()).all(|i| s.menu_label(i) != WALLPAPER_COMPARE));
    let mut s = shell(true);
    assert!(s.menu_rect().bottom() <= SOFTKEYS.y, "{:?}", s.menu_rect());
    let entry = (0..s.menu_len())
        .find(|&i| s.menu_label(i) == WALLPAPER_COMPARE)
        .map(Shell::menu_entry)
        .expect("entry in the test image's menu");
    drawn(&mut s);

    // The wallpaper alone: nothing over it once the depth's notice has gone.
    tap(&mut s, START);
    assert_eq!(tap(&mut s, entry), Action::Redraw);
    assert!(s.wallpaper_only());
    assert_eq!(s.notice(), Some(SHOWING_DITHERED));
    let f = drawn(&mut s);
    assert!(f.dirty().contains(&Rect::new(0, 0, W, H)));
    assert!(f.cmds().count() > 0, "the depth's notice");
    s.tick(NOTICE_MS + 1);
    assert_eq!(drawn(&mut s).cmds().count(), 0);
    // A tap anywhere switches the depth, and back; nothing else acts.
    assert_eq!(
        tap(&mut s, START),
        Action::WallpaperDepth { dithered: false }
    );
    assert_eq!(s.notice(), Some(SHOWING_PLAIN));
    assert!(drawn(&mut s).dirty().contains(&Rect::new(0, 0, W, H)));
    assert_eq!(
        tap(&mut s, SOFT_RIGHT),
        Action::WallpaperDepth { dithered: true }
    );
    assert_eq!(
        tap(&mut s, SOFT_RIGHT),
        Action::WallpaperDepth { dithered: false }
    );
    assert!(s.wallpaper_only());
    assert_eq!(s.input(Input::Key(Key::Down)), Action::Nothing);
    // Home goes back to the Today screen, at the depth chosen (named).
    assert_eq!(s.input(Input::Key(Key::Home)), Action::Redraw);
    assert!(!s.wallpaper_only());
    assert_eq!(s.notice(), Some(SHOWING_PLAIN));
    assert!(drawn(&mut s).cmds().count() > 20);
}

/// Every notice the shell's code can show is in NOTICES (the width check
/// runs over that list, in rp-today, with the real font).
#[test]
fn every_notice_shown_is_listed() {
    let src = [include_str!("lib.rs"), include_str!("weather_settings.rs")];
    for text in src {
        for part in text.split("show_notice(\"").skip(1) {
            let n = part.split('"').next().unwrap_or("");
            assert!(NOTICES.contains(&n), "{n} not listed");
        }
        for part in text.split("=> \"Not yet: ").skip(1) {
            let n = std::format!("Not yet: {}", part.split('"').next().unwrap_or(""));
            assert!(NOTICES.contains(&n.as_str()), "{n} not listed");
        }
    }
}

#[test]
fn the_radar_loops_its_frames_and_holds_on_the_last() {
    use rp_weather::radar::FRAMES;
    let mut s = weather(weather_app::RADAR);
    assert_eq!(
        s.backdrop(),
        Some(false),
        "Night by default (the owner's pick)"
    );
    s.wset.radar = rp_weather::settings::RadarMap::Auto;
    s.set_time(Time {
        hour: 12,
        ..Time::default()
    });
    assert_eq!(s.backdrop(), Some(true), "noon: Ink under the page");
    s.set_time(Time {
        hour: 22,
        ..Time::default()
    });
    assert_eq!(s.backdrop(), Some(false), "night: Night");
    s.radar_frame = 0;
    s.radar_at = 10_000;
    s.tick(10_000);
    s.tick(10_399);
    assert_eq!(s.radar_frame, 0);
    assert_eq!(s.tick(10_400), Action::Redraw);
    assert_eq!(s.radar_frame, 1);
    let mut t = 10_400;
    while s.radar_frame < FRAMES - 1 {
        t += 400;
        s.tick(t);
    }
    // The last frame holds longer, then the loop starts again.
    s.tick(t + 1_000);
    assert_eq!(s.radar_frame, FRAMES - 1);
    s.tick(t + 1_200);
    assert_eq!(s.radar_frame, 0);
    // Night by the setting; nothing under any other page.
    s.wset.radar = rp_weather::settings::RadarMap::Night;
    assert_eq!(s.backdrop(), Some(false));
    s.set_tab(0);
    assert_eq!(s.backdrop(), None);
}

#[test]
fn the_radar_timeline_picks_a_frame_and_play_pauses() {
    use rp_weather::radar::{FRAMES, NOW_FRAME};
    let mut s = weather(weather_app::RADAR);
    let tl = weather_app::TIMELINE;
    // The track's middle is now; its ends the first and last frames.
    for (x, want) in [
        (96 + 176, NOW_FRAME),
        (96, 0),
        (tl.x + tl.w - 1, FRAMES - 1),
    ] {
        tap(&mut s, Rect::new(x, tl.y + 40, 1, 1));
        assert_eq!(s.radar_frame, want, "x {x}");
        assert!(!s.radar_play, "a pick pauses");
    }
    s.tick(50_000);
    s.tick(60_000);
    assert_eq!(s.radar_frame, FRAMES - 1, "paused: it stays");
    tap_t(&mut s, Target::Play);
    assert!(s.radar_play);
    // A vertical stroke on the map moves nothing.
    s.input(Input::PenDown { x: 240, y: 300 });
    s.input(Input::PenMove { x: 240, y: 380 });
    s.input(Input::PenUp { x: 240, y: 380 });
    assert_eq!(s.weather_page(), (weather_app::RADAR, 0));
    assert!(!drawn(&mut s).overflowed());
}

#[test]
fn every_entrance_ends_in_place_and_none_has_no_entrance() {
    use weather_app::Motion;
    let settled = {
        let mut s = weather(0);
        texts(&drawn(&mut s))
    };
    for m in [
        Motion::Stagger,
        Motion::Rise,
        Motion::Expand,
        Motion::None,
        Motion::Cascade,
        Motion::Reveal,
    ] {
        let mut s = shell(false);
        s.set_motion(m);
        s.tick(1_000);
        drawn(&mut s);
        tap(&mut s, HERO);
        assert_eq!(s.animating(), m != Motion::None, "{m:?}");
        for k in 1..=20 {
            s.tick(1_000 + 20 * k);
            let f = drawn(&mut s);
            assert!(!f.overflowed(), "{m:?} at {k}");
        }
        s.tick(1_000 + weather_app::ANIM_MS);
        assert!(!s.animating(), "{m:?}");
        assert_eq!(texts(&drawn(&mut s)), settled, "{m:?}: in place");
    }
}

#[test]
fn reveal_opens_from_the_card_and_cascade_grows_down_with_nothing_inside_moving() {
    use weather_app::{Motion, PAGE};
    // Reveal: the window starts on the Today card and grows to the screen;
    // the app is final inside it from the first frame.
    let mut s = shell(false);
    s.set_motion(Motion::Reveal);
    s.tick(1_000);
    drawn(&mut s);
    tap(&mut s, HERO);
    let (first, shift) = s.reveal().expect("revealing");
    assert_eq!(first, HERO, "it starts as the card");
    assert_eq!(HERO.y + shift, 80, "the big panel read where the card was");
    assert_eq!(s.ease(0), 1024, "nothing inside counts up");
    let mut last = first;
    for k in 1..=20 {
        s.tick(1_000 + 25 * k);
        if let Some((r, _)) = s.reveal() {
            assert!(r.w >= last.w && r.h >= last.h, "it only grows, at {k}");
            assert!(r.y >= TITLE.h, "never over the title bar, at {k}");
            last = r;
        }
        assert!(!drawn(&mut s).overflowed(), "at {k}");
    }
    s.tick(1_000 + weather_app::ANIM_MS);
    assert_eq!(s.reveal(), None, "open");

    // Its pages then cascade: the page's window grows down from its top.
    s.set_tab(1);
    let opened = s.now;
    let mut h = 0;
    for k in 0..=20 {
        s.tick(opened + 27 * k);
        let a = s.page_area();
        assert_eq!((a.x, a.y, a.w), (PAGE.x, PAGE.y, PAGE.w));
        assert!(a.h >= h, "it only grows, at {k}");
        h = a.h;
    }
    assert_eq!(s.page_area(), PAGE);
    assert_eq!(s.reveal(), None, "a page turn never reveals");
}

// ---- the plane path (sheet weather-gpu-movement.md; ADR-0034 C4 as changed, L2)

/// A layer of `w` x `h` drawn by the CPU renderer (ARGB8888, straight
/// alpha), cleared.
fn cpu_layer(w: u32, h: u32) -> (Vec<u32>, Vec<u8>) {
    (vec![0; (w * h) as usize], vec![0u8; (w * h * 4) as usize])
}

/// Page `tab`'s whole picture as the device draws it: band by band.
fn picture_of(s: &mut Shell, tab: u8) -> Vec<u32> {
    use rp_draw::Renderer;
    use rp_gfx::layer::{Layer, PlaneFormat};
    use rp_gfx::render::CpuRenderer;
    let rows = u32::from(picture_rows(tab));
    let (mut px, mut plane) = cpu_layer(480, rows);
    {
        let mut r = CpuRenderer::new(
            Layer::new(&mut px, 480, rows, 480).unwrap(),
            &mut plane,
            480 * 4,
            PlaneFormat::Argb8888,
        );
        r.set_pics(store());
        let mut f = Frame::new();
        let mut top = 0;
        while top < rows as u16 {
            let n = 100.min(rows as u16 - top);
            s.page_band(tab, &mut f, top, n);
            assert!(!f.overflowed(), "band {top}: {} commands", f.cmds().count());
            r.draw(&f);
            top += n;
        }
    }
    px
}

/// The page area of the screen as the 2D path draws it (the page's own
/// commands only: everything else drawn transparent is left out by
/// clearing the rest of the frame's commands' reach to the page).
fn page_2d(s: &mut Shell) -> Vec<u32> {
    use rp_draw::Renderer;
    use rp_gfx::layer::{Layer, PlaneFormat};
    use rp_gfx::render::CpuRenderer;
    let (w, h) = (u32::from(W), u32::from(H));
    let (mut px, mut plane) = cpu_layer(w, h);
    {
        let mut r = CpuRenderer::new(
            Layer::new(&mut px, w, h, w as usize).unwrap(),
            &mut plane,
            w as usize * 4,
            PlaneFormat::Argb8888,
        );
        r.set_pics(store());
        let mut f = Frame::new();
        f.damage(weather_app::PAGE);
        f.clip(Some(weather_app::PAGE));
        match s.weather_page().0 {
            0 => s.now_page(&mut f),
            1 => s.hourly_page(&mut f),
            _ => s.days_page(&mut f),
        }
        r.draw(&f);
    }
    px
}

#[test]
fn each_pages_picture_is_the_page_the_2d_path_draws() {
    // The page layer shows a picture drawn once in bands; at every scroll
    // its window must show exactly what the 2D path draws there (the
    // scroll bar aside: screen 0's on the planes).
    for tab in 0..3u8 {
        let mut s = weather(tab);
        let pic = picture_of(&mut s, tab);
        let max = i32::from(weather_app::max_scroll(tab));
        for scroll in [0, max / 3, max] {
            s.scroll.set(scroll);
            let flat = page_2d(&mut s);
            let p = weather_app::PAGE;
            for y in p.y..p.y + p.h {
                let row = u32::from(LAYER_MARGIN) + u32::from(y - p.y) + scroll as u32;
                // Hourly's graph: painted per pixel in the picture, pieces
                // on the 2D path (alike, not equal: checked by eye).
                let content = i32::from(y) + scroll;
                let g = weather_app::HOURLY_GRAPH;
                if tab == 1 && (i32::from(g.y)..i32::from(g.y + g.h)).contains(&content) {
                    continue;
                }
                for x in 0..W - 10 {
                    let a = flat[usize::from(y) * 480 + usize::from(x)];
                    let b = pic[row as usize * 480 + usize::from(x)];
                    assert_eq!(a, b, "page {tab} scroll {scroll} at ({x}, {y})");
                }
            }
        }
    }
}

#[test]
fn on_the_planes_screen_0_carries_only_the_chrome_and_a_drag_only_the_bar() {
    let mut s = weather(1);
    s.use_planes(true);
    let f = drawn(&mut s);
    // The page's area is the wallpaper's on screen 0: nothing of the page
    // but the scroll bar is drawn there.
    let page = weather_app::PAGE;
    let inside = f
        .cmds_clipped()
        .filter(|(c, clip)| !c.bounds(&f).intersect(clip).intersect(&page).is_empty())
        .count();
    assert_eq!(inside, 2, "the bar's track and thumb only");
    let v = s.page_layer().expect("the layer shows Hourly");
    assert_eq!((v.tab, v.win, v.row), (1, page, LAYER_MARGIN));
    // A drag moves the layer's row; screen 0 redraws only the bar.
    let (x, y) = (240, 400);
    s.input(Input::PenDown { x, y });
    s.input(Input::PenMove { x, y: y - 20 });
    s.input(Input::PenMove { x, y: y - 60 });
    let f = drawn(&mut s);
    assert!(
        f.dirty().iter().all(|d| d.x >= W - 9 && d.w <= 3),
        "{:?}",
        f.dirty()
    );
    let v = s.page_layer().unwrap();
    assert!(v.row > LAYER_MARGIN, "pulled up: {}", v.row);
    s.input(Input::PenUp { x, y: y - 60 });
}

#[test]
fn on_the_planes_the_layer_hands_back_for_menus_the_radar_and_settings() {
    let mut s = weather(0);
    s.use_planes(true);
    drawn(&mut s);
    assert!(s.page_layer().is_some());
    // The menu over the page: the page comes back to screen 0, whole.
    tap(&mut s, START);
    assert!(s.page_layer().is_none());
    let f = drawn(&mut s);
    assert!(
        f.dirty().iter().any(|d| d.union(&weather_app::PAGE) == *d),
        "{:?}",
        f.dirty()
    );
    tap(&mut s, START);
    assert!(s.page_layer().is_some());
    // The radar stays 2D.
    tap(&mut s, weather_app::tab_rect(weather_app::RADAR));
    assert!(s.page_layer().is_none());
}

#[test]
fn on_the_planes_entrances_move_the_window_and_draw_nothing() {
    let mut s = shell(true);
    s.use_planes(true);
    drawn(&mut s);
    // Reveal from the Today card: the app drawn whole once (aside), then
    // only the window grows.
    tap(&mut s, HERO);
    let (r0, _) = s.reveal_plane().expect("revealing");
    let f = drawn(&mut s);
    assert!(
        f.dirty().contains(&Rect::new(0, 0, W, H)),
        "the app, whole: {:?}",
        f.dirty()
    );
    assert!(s.page_layer().is_none(), "the page waits for the reveal");
    // Restarted when the window first shows: from the card again.
    s.tick(300);
    s.restart_entrance(300);
    assert_eq!(s.reveal_plane().unwrap().0, r0, "from the card");
    s.tick(400);
    assert!(!s.needs_frame(), "no redraw while it opens");
    let (r1, _) = s.reveal_plane().unwrap();
    assert!(r1.w > r0.w && r1.y < r0.y, "{r0:?} to {r1:?}");
    s.tick(10_000);
    assert!(s.reveal_plane().is_none());
    assert!(s.page_layer().is_some());
    // Cascade between pages: the window grows down from the tabs' foot.
    drawn(&mut s);
    tap(&mut s, weather_app::tab_rect(2));
    let v0 = s.page_layer().unwrap();
    assert!(v0.win.h < weather_app::PAGE.h && v0.win.h.is_multiple_of(2));
    drawn(&mut s);
    tap(&mut s, weather_app::tab_rect(1));
    drawn(&mut s);
    tap(&mut s, weather_app::tab_rect(2));
    let v0 = s.page_layer().unwrap();
    let now = s.now;
    s.tick(now + 100);
    let v1 = s.page_layer().unwrap();
    assert!(v1.win.h > v0.win.h, "{v0:?} to {v1:?}");
    let f = drawn(&mut s);
    // Nothing of the page on screen 0 but Hourly's scroll bar going away.
    assert!(
        f.dirty()
            .iter()
            .all(|d| d.intersect(&weather_app::PAGE).is_empty() || d.x >= W - 9 && d.w <= 3),
        "{:?}",
        f.dirty()
    );
}

#[test]
fn a_long_swipe_moves_the_tab_line_one_tab_at_most() {
    // A swipe turns one page; its line over the tabs follows the pen no
    // further (the owner, 2026-10-30: a long swipe slid it two or three).
    let mut s = weather(1);
    drawn(&mut s);
    s.input(Input::PenDown { x: 440, y: 300 });
    for x in [420, 300, 150, 40] {
        s.input(Input::PenMove { x, y: 300 });
    }
    let f = drawn(&mut s);
    let line = f
        .cmds()
        .find_map(|c| match *c {
            Cmd::Fill { r, color } if color == FOCUS && r.h == 3 => Some(r),
            _ => None,
        })
        .expect("the tab line");
    let next = weather_app::tab_rect(2);
    assert_eq!(line.x, next.x + 1, "{line:?}: on 10 Day, not past it");
}

#[test]
fn the_weather_preview_is_the_app_as_reveal_leaves_it_and_changes_nothing() {
    // Composed ahead on the device: it must be exactly the app's frame at
    // rest on Now (as the 2D path draws it), from the Today screen, and
    // leave the Today screen as it was.
    let mut s = shell(true);
    drawn(&mut s);
    let mut f = Frame::new();
    s.weather_preview(&mut f);
    assert_eq!(s.screen(), Screen::Today);
    assert!(!s.needs_frame(), "nothing marked to draw");
    let preview = texts(&f);
    let mut w = weather(0);
    w.dirty(Rect::new(0, 0, W, H));
    assert_eq!(preview, texts(&drawn(&mut w)), "the app's own frame");
}

#[test]
fn a_connected_station_becomes_the_place_shown_and_home() {
    let mut s = weather(0);
    while s.next_ask().is_some() {
        s.net_answer(false, b"");
    }
    s.wset.station = rp_weather::settings::Station::Tempest;
    s.want(Ask::Tempest);
    let q = s.next_ask().expect("the station");
    assert_eq!(q.ask, Ask::Tempest);
    s.net_answer(
        true,
        br#"{"location_name":"Liberty Hill Station","latitude":30.664,"longitude":-97.922,"timezone_offset_minutes":-300,"current_conditions":{"air_temperature":18.0}}"#,
    );
    let p = s.wset.place();
    assert_eq!(
        (p.name.as_str(), p.lat, p.lon),
        ("Liberty Hill Station", 30_664, -97_922)
    );
    assert_eq!(s.wset.home, s.wset.current);
    assert_eq!(s.wx.place.as_str(), "Liberty Hill Station");
    assert!(s.wants(Ask::Forecast) && s.wants(Ask::Save));
    // The Station button, then its screen: every instrument card drawn,
    // within one frame.
    assert!(s.station.is_some());
    s.open_weather();
    s.settle();
    tap(&mut s, weather_app::STATION_BTN);
    assert_eq!(s.screen(), Screen::Station);
    let f = drawn(&mut s);
    assert!(!f.overflowed());
    assert!(texts(&f).contains(&b"LIGHTNING".to_vec()));
    tap(&mut s, SOFT_LEFT);
    assert_eq!(s.screen(), Screen::Weather);
    // Seattle can go: Manage places, Remove, tap it.
    tap(&mut s, SOFT_RIGHT);
    s.open(Screen::WeatherSettings);
    tap_item(&mut s, Item::AllPlaces);
    assert_eq!(s.screen(), Screen::WeatherPlaces);
    tap(&mut s, SOFT_RIGHT);
    tap_item(&mut s, Item::Place(0));
    assert_eq!(s.wset.n_places, 1);
    assert_eq!(s.wset.place().name.as_str(), "Liberty Hill Station");
}

#[test]
fn the_radar_asks_its_frames_then_the_map_then_each_rain_frame_tile_by_tile() {
    let mut s = weather(0);
    while s.next_ask().is_some() {
        s.net_answer(false, b"");
    }
    // A place: Liberty Hill (the sample tiles' place).
    s.wset.places[0].lat = 30_664;
    s.wset.places[0].lon = -97_922;
    s.set_tab(weather_app::RADAR);
    s.tick(s.now + 10);
    let q = s.next_ask().expect("the frames");
    assert_eq!(q.line(), b"RPNET FRAMES\n");
    s.net_answer(true, b"1793457000 1793457600 1793458200");
    assert_eq!(s.radar_real(), None, "no map yet");
    let mut lines = std::vec::Vec::new();
    while let Some(q) = s.next_ask() {
        assert_eq!(q.ask, Ask::Tile);
        lines.push(q.line().to_vec());
        s.net_answer(true, b"");
    }
    assert_eq!(
        lines.len(),
        9 * 4,
        "the map and three frames, nine tiles each"
    );
    assert_eq!(lines[0], b"RPNET TILE MAP 7 28 51\n");
    assert_eq!(lines[4], b"RPNET TILE MAP 7 29 52\n");
    assert_eq!(lines[9], b"RPNET TILE RAIN 0 7 28 51\n");
    assert_eq!(lines[35], b"RPNET TILE RAIN 2 7 30 53\n");
    assert_eq!(s.radar_frames(), 3);
    assert_eq!(s.radar_real(), Some(Some(0)));
    let f = drawn(&mut s);
    assert!(!f.overflowed());
    assert!(texts(&f).contains(&b"(c) OpenStreetMap contributors, RainViewer".to_vec()));
}

#[test]
fn an_alert_in_force_shows_in_the_header_and_opens_in_full() {
    let mut s = weather(1);
    let f = drawn(&mut s);
    assert!(
        !texts(&f)
            .iter()
            .any(|t| t.as_slice() == b"Winter Weather Advisory")
    );
    s.set_alerts_from_nws(include_bytes!(
        "../../weather/tests/data/nws-alerts-two.json"
    ));
    let f = drawn(&mut s);
    assert!(!f.overflowed(), "{}", f.cmds().count());
    assert!(
        texts(&f)
            .iter()
            .any(|t| t.as_slice() == b"Winter Weather Advisory")
    );
    let chip = s.alert_rect();
    tap(&mut s, chip);
    assert_eq!(s.screen(), Screen::Alert);
    let f = drawn(&mut s);
    let meta = rp_weather::Line::new()
        .push(b"MODERATE  UNTIL 2 NOVEMBER ")
        .push(s.region.time(11, 0, true).as_bytes());
    assert!(texts(&f).contains(&meta.as_bytes().to_vec()));
    tap(&mut s, SOFT_LEFT);
    assert_eq!(s.screen(), Screen::Weather);
}

/// A Tempest answer: the station's place and its reading now.
const STATION_JSON: &[u8] = br#"{"latitude": 37.7, "longitude": -89.86,
  "location_name": "Perryville Loop Station",
  "current_conditions": {"time": 1761900000, "conditions": "Clear", "icon": "clear-night",
    "air_temperature": 17.4, "feels_like": 17.1, "relative_humidity": 93,
    "wind_avg": 5.0, "wind_direction": 360, "uv": 0, "station_pressure": 813.2}}"#;

#[test]
fn a_place_picked_keeps_its_own_forecast_once_a_station_is_home() {
    // The owner, 2026-10-31 (today-0036): with the station connected,
    // Seattle picked from the header's menu still showed the station.
    let mut s = weather(0);
    s.wset.provider = rp_weather::settings::Provider::Tempest;
    s.wset.station = rp_weather::settings::Station::Tempest;
    // No station place yet: the first question is the station's.
    let mut asked = Vec::new();
    for _ in 0..8 {
        let Some(q) = s.next_ask() else { break };
        asked.push(q.line().to_vec());
        let tempest = q.line().starts_with(b"RPNET TEMPEST");
        s.net_answer(tempest, if tempest { STATION_JSON } else { b"" });
    }
    assert!(
        asked.iter().any(|l| l.starts_with(b"RPNET TEMPEST")),
        "{asked:?}"
    );
    assert!(s.at_station(), "the station's place is home and shown");
    assert_eq!(s.wset.place().name.as_str(), "Perryville Loop Station");
    // Seattle picked: its forecast from the internet, and it stays.
    tap(&mut s, weather_app::PLACE_BTN);
    let seattle = (0..s.wset.n_places)
        .find(|&i| s.wset.places()[usize::from(i)].name.as_str() == "Seattle")
        .expect("Seattle");
    tap(&mut s, Shell::place_item(seattle));
    assert_eq!(s.wset.place().name.as_str(), "Seattle");
    let mut forecast = None;
    for _ in 0..8 {
        let Some(q) = s.next_ask() else { break };
        if q.ask == Ask::Forecast {
            forecast = Some(q.line().to_vec());
        }
        let tempest = q.line().starts_with(b"RPNET TEMPEST");
        s.net_answer(tempest, if tempest { STATION_JSON } else { b"" });
    }
    assert_eq!(
        forecast.as_deref(),
        Some(&b"RPNET OPENMETEO 47.606 -122.332\n"[..])
    );
    assert_eq!(
        s.wset.place().name.as_str(),
        "Seattle",
        "a station answer doesn't take it back"
    );
}

#[test]
fn hourlys_line_runs_on_past_its_ends_on_the_curves_slope() {
    // Past the first and last hours the painted line keeps the curve's
    // slope there: in whole tenths a slope over 1/16 hour rounded to
    // nothing and the ends lay flat (emu-today, 2026-10-31).
    let g = weather_app::HOURLY_GRAPH;
    for (first, second) in [(220i16, 237i16), (237, 220), (220, 226)] {
        let mut s = weather(1);
        s.wx.hours[0].temp = first;
        s.wx.hours[1].temp = second;
        let (stride, rows) = (
            usize::from(weather_app::GRAPH_PIC.0),
            usize::from(weather_app::GRAPH_PIC.1),
        );
        let mut px = vec![0u32; stride * rows];
        s.paint_graph(&mut px);
        // The line's top edge in a column (the first row painted; the
        // picture starts at the panel's first inner column).
        let top = |x: u16| {
            (0..rows)
                .find(|&y| px[y * stride + usize::from(x - g.x - 1)] != 0)
                .expect("the line in every column")
        };
        let (edge, inner) = (top(g.x + 1), top(g.x + 20));
        // Rising, the end is lower (a later row); falling, higher.
        assert_eq!(
            edge.cmp(&inner),
            second.cmp(&first),
            "{first} then {second}: the left end at row {edge}, inside at {inner}"
        );
    }
}

#[test]
fn a_tap_off_the_page_is_never_a_tap_on_it() {
    // Hourly scrolled, then Home: the Start menu's last entry lies where a
    // grid cell would be, and picked an hour instead of going back to the
    // loader (emu-today, 2026-10-31). Every page, dragged or not.
    for page in 0..3 {
        for drag in [false, true] {
            let mut s = weather(page);
            drawn(&mut s);
            if drag {
                s.input(Input::PenDown { x: 240, y: 500 });
                for y in [470, 430, 390, 350, 310] {
                    s.input(Input::PenMove { x: 240, y });
                }
                s.input(Input::PenUp { x: 240, y: 310 });
                s.settle();
            }
            s.input(Input::Key(Key::Home));
            s.settle();
            drawn(&mut s);
            let pick = s.hour_pick;
            tap(&mut s, START);
            let last = s.menu_len() - 1;
            assert_eq!(
                tap(&mut s, Shell::menu_entry(last)),
                Action::BackToLoader,
                "page {page}, dragged {drag}"
            );
            assert_eq!(s.hour_pick, pick, "page {page}, dragged {drag}");
        }
    }
}

/// Each picture a frame draws, as (size, pack id less one).
fn pics_drawn(s: &Shell, f: &Frame) -> Vec<(Size, usize)> {
    f.cmds()
        .filter_map(|c| match c {
            Cmd::Pic { pic, .. } if Some(*pic) != s.graph() => Some(*pic),
            _ => None,
        })
        .map(|pic| {
            Size::ALL
                .iter()
                .flat_map(|&z| (0..22).map(move |i| (z, i)))
                .find(|&(z, i)| s.pic(z, i) == Some(pic))
                .expect("a drawn picture is one the shell holds")
        })
        .collect()
}

#[test]
fn the_screens_draw_only_wanted_pictures_and_every_hour_s_card_is_wanted() {
    // Option C (docs/design/weather-rule0.md): the device decodes what
    // `wanted` lists and nothing else, so a picture drawn but not listed
    // would be missing on the X51v.
    let mut s = shell(true);
    let want = s.wanted();
    let f = drawn(&mut s);
    let today = pics_drawn(&s, &f);
    assert!(!today.is_empty());
    for (z, i) in today {
        assert!(want.has(z, i), "Today draws {z:?} {i} unlisted");
    }
    for page in 0..3 {
        let mut s = weather(page);
        let picks: Vec<u8> = if page == 1 {
            (0..24).collect()
        } else {
            vec![0]
        };
        for h in picks {
            s.hour_pick = h;
            s.dirty(Rect::new(0, 0, W, H));
            let f = drawn(&mut s);
            let shown = pics_drawn(&s, &f);
            for &(z, i) in &shown {
                assert!(want.has(z, i), "page {page} draws {z:?} {i} unlisted");
            }
            if page == 1 {
                let k = usize::from(Icon::of(s.wx.hours[usize::from(h)].sky).id() - 1);
                assert!(shown.contains(&(Size::Mid, k)), "hour {h}'s card picture");
            }
        }
    }
    // Given back, a picture is simply not drawn; the count is what the
    // device's store must hold (its classes: docs/design/weather-rule0.md).
    let mut s = shell(true);
    let k = usize::from(Icon::of(s.wx.now.sky).id() - 1);
    s.set_pic(Size::Hero, k, None);
    s.set_pic(Size::Card, k, None);
    let f = drawn(&mut s);
    assert!(pics_drawn(&s, &f).iter().all(|&(z, _)| z == Size::Card));
    assert!(want.count() <= 2 + 4 + 8 + 16, "{want:?}");
}

#[test]
fn the_default_pack_is_asked_by_offset_after_the_forecast_and_kept_by_the_device() {
    // Option C: an image with no card fills its copy of the default icon
    // pack from the PC a piece at a time; the shell only asks.
    let mut s = shell(true);
    while let Some(q) = s.next_ask() {
        s.net_answer(false, &[]);
        assert_ne!(q.ask, Ask::Pack);
    }
    s.want(Ask::Forecast);
    s.want_pack(196_608);
    assert!(s.wants(Ask::Pack));
    let q = s.next_ask().expect("a question");
    assert_eq!(q.ask, Ask::Forecast, "the forecast first");
    s.net_answer(false, &[]);
    let q = s.next_ask().expect("the pack's piece");
    assert_eq!(q.line(), b"RPNET PACK 196608\n");
    let before = *s.forecast();
    s.net_answer(true, &[1, 2, 3]);
    assert!(!s.wants(Ask::Pack));
    assert!(
        *s.forecast() == before,
        "the answer is the device's, not the shell's"
    );
}

#[test]
fn the_busiest_frame_leaves_a_quarter_of_the_frame_spare() {
    // MAX_CMDS is sized from this (P6: budgets the build enforces): every
    // screen, every page scrolled to its top, middle and end, with the
    // menu open and not, at most three quarters of a frame.
    let mut most = (0usize, String::new());
    let mut note = |f: &Frame, what: String| {
        assert!(!f.overflowed(), "{what}");
        let n = f.cmds().count();
        if n > most.0 {
            most = (n, what);
        }
    };
    let mut s = shell(true);
    note(&drawn(&mut s), "Today".into());
    tap(&mut s, START);
    note(&drawn(&mut s), "Today, menu".into());
    for page in 0..3u8 {
        let max = i32::from(weather_app::max_scroll(page));
        for scroll in [0, max / 2, max] {
            for menu in [false, true] {
                let mut s = weather(page);
                s.scroll.set(scroll);
                s.dirty(Rect::new(0, 0, W, H));
                if menu {
                    tap(&mut s, START);
                }
                note(
                    &drawn(&mut s),
                    format!("page {page} at {scroll}, menu {menu}"),
                );
            }
        }
    }
    for screen in [
        Screen::WeatherSettings,
        Screen::WeatherPlaces,
        Screen::AddPlace,
        Screen::WeatherFrom,
        Screen::KeyEntry,
    ] {
        let mut s = weather(0);
        tap(&mut s, SOFT_RIGHT);
        s.open(screen);
        note(&drawn(&mut s), format!("{screen:?}"));
    }
    // The living sky at its busiest (a storm: heavy rain, every cloud and
    // the flash), the menu open over it.
    let mut s = shell(true);
    let mut wx = s.wx;
    wx.now.sky.wmo = 95;
    wx.now.sky.cover = 100;
    s.set_forecast(wx);
    s.preview_sky(true);
    s.tick(5200 * 3 + 10);
    note(&drawn(&mut s), "Today, storm sky".into());
    tap(&mut s, START);
    note(&drawn(&mut s), "Today, storm sky, menu".into());
    // Your station on Now (N1, N2) with a chart's sheet open over it, and
    // the Station screen with one.
    let storm: &[u8] = br#"{"current_conditions": {"time": 1761900000,
      "air_temperature": 20.0, "relative_humidity": 61, "wind_avg": 12.9, "wind_gust": 33.8,
      "sea_level_pressure": 1016.4, "pressure_trend": "rising", "uv": 6,
      "lightning_strike_count_last_3hr": 4, "lightning_strike_last_distance": 10}}"#;
    let mut s = weather(0);
    s.set_station_from_tempest(storm);
    note(&drawn(&mut s), "Now, your station".into());
    let r = s.now_reading_rect(0);
    tap(&mut s, r);
    note(&drawn(&mut s), "Now, a chart".into());
    s.open(Screen::Station);
    tap(&mut s, station::reading_rect(1));
    note(&drawn(&mut s), "Station, a chart".into());
    println!("busiest frame: {} commands ({})", most.0, most.1);
    assert!(
        most.0 * 4 <= MAX_CMDS * 3,
        "{} commands ({})",
        most.0,
        most.1
    );
}

#[test]
fn a_radar_refresh_fetches_only_the_frames_it_has_not_got() {
    // P7, by key: ten minutes on, RainViewer's list has moved on by one
    // frame; the map and the frames still listed are kept (moved forward
    // by the device), so 9 tiles are asked, not 81.
    let mut s = weather(0);
    while s.next_ask().is_some() {
        s.net_answer(false, b"");
    }
    s.wset.places[0].lat = 30_664;
    s.wset.places[0].lon = -97_922;
    s.set_tab(weather_app::RADAR);
    s.tick(s.now + 10);
    let all = |s: &mut Shell| {
        let mut n = 0;
        while let Some(q) = s.next_ask() {
            assert_eq!(q.ask, Ask::Tile);
            n += 1;
            s.net_answer(true, b"");
        }
        n
    };
    let q = s.next_ask().expect("the frames");
    assert_eq!(q.ask, Ask::Frames);
    s.net_answer(true, b"100 200 300 400");
    assert_eq!(all(&mut s), 9 * 5);
    assert_eq!(s.take_radar_moves(), None, "nothing kept the first time");
    // Ten minutes on: the same place.
    s.tick(s.now + 600_001);
    let q = s.next_ask().expect("the frames again");
    assert_eq!(q.ask, Ask::Frames);
    assert!(s.radar_real().is_some(), "still shown while it asks");
    s.net_answer(true, b"200 300 400 500");
    let m = s.take_radar_moves().expect("frames kept");
    assert_eq!(&m[..4], &[1, 2, 3, u8::MAX]);
    assert!(
        matches!(s.radar_real(), Some(Some(f)) if f < 3),
        "the frame shown is a kept one: {:?}",
        s.radar_real()
    );
    let tiles: Vec<Vec<u8>> = core::iter::from_fn(|| {
        let q = s.next_ask()?;
        let l = q.line().to_vec();
        s.net_answer(true, b"");
        Some(l)
    })
    .collect();
    assert_eq!(tiles.len(), 9, "only the new frame's tiles");
    assert!(tiles[0].starts_with(b"RPNET TILE RAIN 3 "));
    assert_eq!(s.radar_frames(), 4);
    // Another place: nothing kept.
    s.wset.places[0].lat = 47_606;
    s.tick(s.now + 10);
    let q = s.next_ask().expect("the frames for the new place");
    assert_eq!(q.ask, Ask::Frames);
    s.net_answer(true, b"200 300 400 500");
    assert_eq!(s.take_radar_moves(), None);
    assert_eq!(all(&mut s), 9 * 5);
}

#[test]
fn the_place_menu_closes_on_a_tap_outside_home_and_any_screen_or_page_change() {
    // Left open, the place menu kept the page off its layer and stayed
    // drawn behind Today and the app (the owner's run, 2026-10-31;
    // emu-today-steps shot 09). Every way out closes it, and the layer
    // takes the page back.
    let open = |s: &mut Shell| {
        tap(s, weather_app::PLACE_BTN);
        assert!(s.place_menu, "opened");
    };
    let mut s = weather(0);
    s.use_planes(true);
    open(&mut s);
    assert_eq!(
        s.layer_tab(),
        None,
        "the page off its layer while it is open"
    );
    tap(&mut s, Rect::new(380, 380, 40, 40));
    assert!(!s.place_menu, "a tap outside closes it");
    assert_eq!(s.layer_tab(), Some(0), "the page back on its layer");
    assert_eq!(s.screen(), Screen::Weather);
    open(&mut s);
    s.input(Input::Key(Key::Home));
    assert!(!s.place_menu, "Home closes it");
    tap(&mut s, HERO);
    assert!(!s.place_menu && s.screen() == Screen::Weather);
    open(&mut s);
    s.set_tab(1);
    assert!(!s.place_menu, "a page turn closes it");
    open(&mut s);
    s.open(Screen::WeatherSettings);
    assert!(!s.place_menu, "another screen closes it");
    // Picking a place still works.
    s.open_weather();
    open(&mut s);
    tap(&mut s, Shell::place_item(0));
    assert!(!s.place_menu);
}

#[test]
fn a_pressed_button_is_solid_under_its_label() {
    // The owner, 2026-10-31: a see-through press, "I do NOT like that";
    // he picked solid slate (docs/design/mockups/pressed-look-choices.png).
    // Held, a button draws an opaque fill before its label, and nothing
    // see-through over it; the weather card still darkens.
    let mut s = weather(0);
    let r = weather_app::tab_rect(1);
    let (x, y) = (r.x + r.w / 2, r.y + r.h / 2);
    s.input(Input::PenDown { x, y });
    let f = drawn(&mut s);
    let cmds: Vec<&Cmd> = f.cmds().collect();
    let fill = cmds
        .iter()
        .position(|c| matches!(c, Cmd::Fill { r: fr, color } if *fr == r && color >> 24 == 0xFF))
        .expect("a solid fill over the pressed tab");
    let label = cmds
        .iter()
        .rposition(|c| matches!(c, Cmd::Text { x: tx, .. } if *tx >= r.x && *tx < r.x + r.w))
        .expect("the tab's label");
    assert!(fill < label, "the fill under the label");
    assert!(
        !cmds[fill + 1..]
            .iter()
            .any(|c| matches!(c, Cmd::Fill { r: fr, color } if *fr == r && color >> 24 != 0xFF)),
        "nothing see-through over it"
    );
}

#[test]
fn every_day_and_every_hour_shows_its_chance_of_rain() {
    // The owner, 2026-10-31 (the third time): rows without a percentage
    // look broken. Every 10 Day row and every Hourly cell shows one, 0%
    // too.
    let mut s = weather(2);
    for d in &mut s.wx.days {
        d.precip = 0;
    }
    s.dirty(Rect::new(0, 0, W, H));
    let f = drawn(&mut s);
    let zeros = texts(&f).iter().filter(|t| t.as_slice() == b"0%").count();
    assert_eq!(zeros, rp_weather::DAYS, "a 0% on each 10 Day row");
    let mut s = weather(1);
    for h in &mut s.wx.hours {
        h.precip = 0;
    }
    s.use_planes(true);
    let mut seen = 0;
    let rows = picture_rows(1);
    let mut top = 0;
    while top < rows {
        let n = 100.min(rows - top);
        let mut f = Frame::new();
        s.page_band(1, &mut f, top, n);
        seen += texts(&f).iter().filter(|t| t.as_slice() == b"0%").count();
        top += n;
    }
    assert!(
        seen >= rp_weather::HOURS,
        "a 0% in each hour's cell: {seen}"
    );
}

#[test]
fn the_weather_app_has_no_ok_in_its_title_bar() {
    // The owner's ruling, 2026-10-31 (R162): back and Home do its job.
    for screen in [
        Screen::Weather,
        Screen::WeatherSettings,
        Screen::WeatherPlaces,
        Screen::WeatherFrom,
    ] {
        let mut s = weather(0);
        if screen != Screen::Weather {
            s.open(screen);
        }
        let f = drawn(&mut s);
        assert!(
            !texts(&f).iter().any(|t| t.as_slice() == b"ok"),
            "{screen:?}"
        );
        assert_eq!(s.hit(OK_BOX.x + 20, OK_BOX.y + 14), None, "{screen:?}");
    }
}

#[test]
fn the_play_mark_is_a_smooth_triangle() {
    // R161: the stepped one looked broken. Solid at its left middle, see-
    // through at its right corners, and its edge shades between.
    let img = &weather_app::play_icon();
    let a = |x: usize, y: usize| img.px[y * usize::from(img.w) + x] >> 24;
    let (w, h) = (usize::from(img.w), usize::from(img.h));
    assert_eq!(a(1, h / 2), 255);
    assert_eq!(a(w - 1, 0), 0);
    assert_eq!(a(w - 1, h - 1), 0);
    assert!(
        img.px.iter().any(|p| (1..255).contains(&(p >> 24))),
        "anti-aliased"
    );
}

#[test]
fn light_and_dark_change_every_screen_and_auto_follows_the_sun() {
    // R160: the settings' Light and Dark changed nothing. Light turns the
    // neutral colours (glass, text) to their light twins and keeps accents;
    // Auto is light between sunrise and sunset.
    use rp_weather::settings::Appearance;
    let text_colors = |s: &mut Shell| -> Vec<Argb> {
        drawn(s)
            .cmds()
            .filter_map(|c| match c {
                Cmd::Text { color, .. } => Some(*color),
                _ => None,
            })
            .collect()
    };
    let mut s = weather(0);
    s.wset.appearance = Appearance::Dark;
    s.dirty(Rect::new(0, 0, W, H));
    let dark = text_colors(&mut s);
    s.wset.appearance = Appearance::Light;
    s.dirty(Rect::new(0, 0, W, H));
    let light = text_colors(&mut s);
    assert!(dark.contains(&TEXT) && !light.contains(&TEXT));
    assert!(light.contains(&light_twin(TEXT)));
    assert_eq!(light_twin(FOCUS), FOCUS, "an accent keeps its colour");
    // Mirrored short of white (the owner's run, 2026-11-01: too bright):
    // the glass a light grey, the text near black.
    let glass = light_twin(GLASS_LEFT);
    assert!(
        (glass & 0xFF) <= 196 && (glass & 0xFF) >= 176,
        "{glass:08X}"
    );
    assert!(light_twin(TEXT) & 0xFF <= 16);
    // Auto: by the sun at the place shown, and a change redraws it all.
    s.wset.appearance = Appearance::Auto;
    let (rise, set) = s.wx.sun;
    let t = |m: u16| Time {
        hour: (m / 60) as u8,
        minute: (m % 60) as u8,
        ..s.time
    };
    let (day, night) = (t(rise + 60), t(set + 60));
    s.set_time(day);
    assert!(s.light());
    drawn(&mut s);
    s.set_time(night);
    assert!(!s.light());
    assert!(drawn(&mut s).dirty().contains(&Rect::new(0, 0, W, H)));
}

#[test]
fn the_cards_settings_name_the_pack_and_place_from_the_start() {
    // R153's settings half: the record the relay keeps, read from the card
    // at start, so the chosen pack is known before any picture is made; a
    // damaged one gives the defaults.
    let mut s = shell(true);
    let mut set = s.wset;
    set.pack = 2;
    let rec = rp_weather::store::encode(&set);
    s.settings_from_card(&rec);
    assert_eq!(s.icon_pack(), 2);
    let mut bad = rec;
    bad[10] ^= 0xFF;
    s.settings_from_card(&bad);
    assert_eq!(s.icon_pack(), 0, "a damaged record: the defaults");
}

#[test]
fn with_a_station_now_shows_its_live_readings_and_what_matters_now() {
    // N1 and N2 (docs/design/station-everywhere.md): the big temperature
    // and the cards are the station's, marked live; the strip puts a
    // storm's lightning first.
    let storm: &[u8] = br#"{"current_conditions": {"time": 1761900000,
      "air_temperature": 20.0, "feels_like": 19.5, "relative_humidity": 61, "dew_point": 12.2,
      "wind_avg": 12.9, "wind_gust": 33.8, "wind_direction": 315, "uv": 3,
      "sea_level_pressure": 1016.4, "pressure_trend": "rising",
      "lightning_strike_count_last_3hr": 4, "lightning_strike_last_distance": 10}}"#;
    let mut s = weather(0);
    let before = texts(&drawn(&mut s));
    assert!(!before.iter().any(|t| t.as_slice() == b" LIVE"));
    s.set_station_from_tempest(storm);
    s.dirty(Rect::new(0, 0, W, H));
    let f = drawn(&mut s);
    assert!(!f.overflowed(), "{} commands", f.cmds().count());
    let t = texts(&f);
    let has = |b: &[u8]| t.iter().any(|x| x.as_slice() == b);
    assert!(has(b" LIVE"));
    assert!(has(b"68\xB0"), "the station's 20.0 C as 68 F: {t:?}");
    assert!(has(b"LIGHTNING") && has(b"GUSTS") && has(b"PRESSURE"));
    let order: Vec<&[u8]> = s
        .strip_readings(&s.station.unwrap())
        .iter()
        .flatten()
        .map(|r| r.0)
        .collect();
    assert_eq!(order, [&b"LIGHTNING"[..], b"GUSTS", b"PRESSURE"]);
    // Stale (an hour old): the forecast's again.
    s.now = s.station_at.wrapping_add(61 * 60_000);
    s.dirty(Rect::new(0, 0, W, H));
    assert!(
        !texts(&drawn(&mut s))
            .iter()
            .any(|x| x.as_slice() == b" LIVE")
    );
}

#[test]
fn a_reading_opens_its_chart_the_marker_scrubs_and_the_sheet_closes() {
    // N3, N4 (docs/design/station-everywhere.md): the wind card opens the
    // wind's chart, the station's day joined to the forecast at now; the
    // D-pad and a finger move the marker; up changes the reading; Close, a
    // tap above and a pull down shut it.
    let now: &[u8] = br#"{"current_conditions": {"time": 1761900000,
      "air_temperature": 20.0, "relative_humidity": 61, "wind_avg": 12.9,
      "sea_level_pressure": 1016.4, "pressure_trend": "rising"}}"#;
    let mut s = weather(0);
    s.set_station_from_tempest(now);
    // A day of half-hour rows, as Tempest's observations answer.
    let rows: Vec<String> = (0..48)
        .map(|i| {
            format!(
                "[{},0.5,{}.0,6.0,315,3,{}.0,{}.0,60]",
                1_761_900_000 - (47 - i) * 1800,
                1 + i % 5,
                990 + i / 8,
                10 + i / 4
            )
        })
        .collect();
    let json = format!("{{\"obs\":[{}]}}", rows.join(","));
    s.history = rp_weather::tempest::history(json.as_bytes());
    assert!(s.history.is_some());
    drawn(&mut s);
    assert!(s.reading_shown(1));
    let r = s.now_reading_rect(1);
    tap(&mut s, r);
    assert_eq!(s.chart.map(|c| c.reading), Some(chart::Reading::Wind));
    assert_eq!(s.softkeys(), ("Close", "2 days"));
    let f = drawn(&mut s);
    assert!(!f.overflowed(), "{} commands", f.cmds().count());
    let t = texts(&f);
    let has = |t: &[Vec<u8>], b: &[u8]| t.iter().any(|x| x.as_slice() == b);
    assert!(has(&t, b"WIND") && has(&t, b"NOW, YOUR STATION"), "{t:?}");
    // The line is painted into the graph's picture.
    let (gw, gh) = weather_app::GRAPH_PIC;
    let mut px = vec![0u32; usize::from(gw) * usize::from(gh)];
    s.paint_graph(&mut px);
    assert!(px.iter().filter(|p| **p >> 24 > 0x80).count() > 400);
    assert_ne!(s.graph_key(), {
        let c = s.chart.take();
        let k = s.graph_key();
        s.chart = c;
        k
    });
    // The D-pad: right into the forecast, left back into the station's.
    s.input(Input::Key(Key::Right));
    assert_eq!(s.chart.map(|c| c.at), Some(1));
    assert!(has(&texts(&drawn(&mut s)), b"THE FORECAST"));
    for _ in 0..3 {
        s.input(Input::Key(Key::Left));
    }
    assert_eq!(s.chart.map(|c| c.at), Some(-2));
    assert!(has(&texts(&drawn(&mut s)), b"YOUR STATION MEASURED"));
    // A finger at the line's left end: twelve hours ago.
    let (x, y) = (17 + 10, chart::SHEET.y + 156 + 80);
    s.input(Input::PenDown { x, y });
    s.input(Input::PenMove { x: x + 1, y });
    s.input(Input::PenUp { x: x + 1, y });
    assert_eq!(s.chart.map(|c| c.at), Some(-24));
    // Up: the next reading round (temperature); the right softkey: 2 days.
    s.input(Input::Key(Key::Up));
    assert_eq!(s.chart.map(|c| c.reading), Some(chart::Reading::Temp));
    tap(&mut s, SOFT_RIGHT);
    assert_eq!(s.chart.map(|c| c.span), Some(1));
    assert!(!drawn(&mut s).overflowed());
    // Close; then a tap above the sheet; then a pull down from its top.
    tap(&mut s, SOFT_LEFT);
    assert!(s.chart.is_none());
    assert_eq!(s.screen(), Screen::Weather);
    let r = s.now_reading_rect(0);
    tap(&mut s, r);
    assert!(s.chart.is_some());
    s.input(Input::PenDown { x: 240, y: 120 });
    s.input(Input::PenUp { x: 240, y: 120 });
    assert!(s.chart.is_none());
    let r = s.now_reading_rect(2);
    tap(&mut s, r);
    assert_eq!(s.chart.map(|c| c.reading), Some(chart::Reading::Pressure));
    let y0 = chart::SHEET.y + 20;
    s.input(Input::PenDown { x: 100, y: y0 });
    s.input(Input::PenMove { x: 100, y: y0 + 50 });
    s.input(Input::PenUp { x: 100, y: y0 + 50 });
    assert!(s.chart.is_none());
    // The Station screen's tiles open it too.
    s.open(Screen::Station);
    drawn(&mut s);
    tap(&mut s, station::reading_rect(1));
    assert_eq!(s.chart.map(|c| c.reading), Some(chart::Reading::Wind));
    assert!(!drawn(&mut s).overflowed());
}

#[test]
fn the_sky_moves_only_with_a_real_forecast_at_its_own_pace() {
    // W6: rain falls through drifting cloud behind Today's picture; the
    // sample forecast keeps it still (nothing pretends).
    let mut s = shell(false);
    drawn(&mut s);
    let mut wx = s.wx;
    wx.now.sky.wmo = 63;
    s.set_forecast(wx);
    drawn(&mut s);
    assert!(!s.sky_live(), "the sample forecast: still");
    assert_eq!(s.tick(1000), Action::Nothing);
    s.preview_sky(true);
    assert!(s.sky_live());
    drawn(&mut s);
    assert_eq!(s.tick(2000), Action::Redraw);
    let f = drawn(&mut s);
    assert!(
        f.dirty()
            .iter()
            .all(|d| sky::AREA.intersect(d) == *d || *d == sky::AREA)
    );
    let runs = f
        .cmds()
        .filter(|c| matches!(c, Cmd::Text { font, .. } if core::ptr::eq(*font, &sky::SKY)))
        .count();
    assert!(runs >= 10, "{runs} runs of the sky's glyphs");
    // Not again before its step; then again.
    assert_eq!(s.tick(2000 + sky::STEP_MS - 1), Action::Nothing);
    assert_eq!(s.tick(2000 + sky::STEP_MS), Action::Redraw);
    // Off the Today screen: still.
    tap(&mut s, HERO);
    assert!(!s.sky_live());
}

#[test]
fn reveal_starts_from_rest() {
    // The owner's 60 fps video (2026-11-01): the window's foot dropped
    // ~38 px in the first refresh. From rest now: a few px in the first
    // two refreshes, every refresh's step no more than twice the mean.
    let mut s = shell(false);
    s.use_planes(true);
    s.tick(1000);
    drawn(&mut s);
    tap(&mut s, HERO);
    let foot = |s: &Shell| s.reveal_window().map(|(r, _)| i32::from(r.y + r.h));
    let start = foot(&s).expect("revealing");
    s.tick(1017);
    let one = foot(&s).expect("revealing");
    s.tick(1034);
    let two = foot(&s).expect("revealing");
    assert!(one - start <= 3 && two - start <= 8, "{start} {one} {two}");
    let mean = (i32::from(H) - start) * 17 / weather_app::ANIM_MS as i32;
    let mut last = start;
    for k in 1..32 {
        s.tick(1000 + k * 17);
        let Some(f) = foot(&s) else { break };
        assert!(
            f - last <= mean * 2,
            "refresh {k}: {last} to {f}, mean {mean}"
        );
        last = f;
    }
}

#[test]
fn the_sky_test_cycles_through_the_weathers_and_back() {
    // The owner, 2026-11-01: a way to see rain, snow and a storm on demand.
    let mut s = shell(true);
    drawn(&mut s);
    assert!(!s.sky_live(), "the sample forecast: still");
    let entry = (0..s.menu_len())
        .find(|&i| s.menu_label(i) == "Sky test")
        .expect("the test image's menu has it");
    let pick = |s: &mut Shell| {
        tap(s, START);
        tap(s, Shell::menu_entry(entry));
        s.notice()
    };
    assert_eq!(pick(&mut s), Some("Sky test: rain"));
    assert!(s.sky_live() && weather_of_test(&s) == 63);
    for want in [
        "Sky test: heavy rain",
        "Sky test: snow",
        "Sky test: storm",
        "Sky test: cloudy",
    ] {
        assert_eq!(pick(&mut s), Some(want));
        assert!(!drawn(&mut s).overflowed());
    }
    assert_eq!(pick(&mut s), Some("Sky: the forecast's"));
    assert!(!s.sky_live());
    // Every menu row fits above the softkeys.
    let last = Shell::menu_entry(s.menu_len() - 1);
    assert!(last.y + last.h <= H - 40);
}

fn weather_of_test(s: &Shell) -> u8 {
    s.sky_now().wmo
}

#[test]
fn every_station_field_opens_its_own_chart() {
    // The owner, 2026-11-01: "what happened to everything you click on gets
    // a new barograph or open to a new page rule?" Every tile on the Station
    // screen, every card and strip item on Now, opens its reading's chart,
    // each drawn within the frame.
    use chart::Reading as R;
    let storm: &[u8] = br#"{"current_conditions": {"time": 1761900000,
      "air_temperature": 20.0, "relative_humidity": 61, "dew_point": 12.2, "wind_avg": 12.9,
      "wind_gust": 33.8, "sea_level_pressure": 1016.4, "pressure_trend": "rising", "uv": 6,
      "solar_radiation": 420, "precip_accum_local_day": 3.0,
      "lightning_strike_count_last_3hr": 4, "lightning_strike_last_distance": 10}}"#;
    let mut s = weather(0);
    s.set_station_from_tempest(storm);
    let rows: Vec<String> = (0..48)
        .map(|i| {
            format!(
                "[{},0.5,2.0,6.0,315,60,990.0,18.0,70,9000,{}.5,{},{}.{},0,8,{}]",
                1_761_900_000 - (47 - i) * 1800,
                i % 7,
                100 + i * 9,
                i % 3,
                i % 9,
                i % 4
            )
        })
        .collect();
    s.history =
        rp_weather::tempest::history(format!("{{\"obs\":[{}]}}", rows.join(",")).as_bytes());
    let h = s.history.expect("a history");
    assert_eq!(h.s[47].uv, Some(55), "UV 5.5 in tenths");
    assert_eq!(h.s[47].strikes, Some(3));
    assert_eq!(h.s[47].solar, Some(523));
    assert_eq!(h.s[47].rain, Some(22), "2.2 mm in tenths");
    drawn(&mut s);
    let mut seen = Vec::new();
    for i in 0..8u8 {
        if !s.reading_shown(i) {
            continue;
        }
        let r = s.now_reading_rect(i);
        tap(&mut s, r);
        let got = s
            .chart
            .map(|c| c.reading)
            .unwrap_or_else(|| panic!("slot {i}: no chart"));
        assert_eq!(got, s.reading_of(i));
        assert!(!drawn(&mut s).overflowed());
        seen.push(got);
        s.close_chart();
        drawn(&mut s);
    }
    assert_eq!(
        seen.len(),
        8,
        "the panel, four cards, three strip items: {seen:?}"
    );
    assert!(seen.contains(&R::Sun) && seen.contains(&R::Lightning) && seen.contains(&R::Gust));
    s.open(Screen::Station);
    drawn(&mut s);
    let mut seen = Vec::new();
    for i in 0..7u8 {
        assert!(s.reading_shown(i));
        tap(&mut s, station::reading_rect(i));
        let got = s.chart.map(|c| c.reading).expect("a chart");
        // Every reading has a value somewhere on its chart.
        assert!(
            (-24..=24).any(|o| s.chart_value(got, o).is_some()),
            "{got:?}"
        );
        let (gw, gh) = weather_app::GRAPH_PIC;
        let mut px = vec![0u32; usize::from(gw) * usize::from(gh)];
        s.paint_graph(&mut px);
        assert!(px.iter().any(|p| p >> 24 > 0x80), "{got:?} painted");
        seen.push(got);
        s.close_chart();
    }
    assert_eq!(
        seen,
        [
            R::Temp,
            R::Wind,
            R::Pressure,
            R::Uv,
            R::Rain,
            R::Lightning,
            R::Sun
        ]
    );
}

#[test]
fn todays_card_and_the_now_panel_are_one_card() {
    // The owner's 60 fps video (2026-11-01): Reveal's first refresh jumped
    // from Today's lines to the app's. Every text the Today card draws, the
    // Now panel draws at the same offset from its own top (Reveal reads the
    // app 28 rows lower, so the first refresh is the same picture), with
    // the station's temperature on both (S3).
    let storm: &[u8] = br#"{"current_conditions": {"time": 1761900000,
      "air_temperature": 25.6, "feels_like": 26.0, "relative_humidity": 61}}"#;
    let card_texts = |f: &Frame, top: u16| -> Vec<(u16, i32, Vec<u8>)> {
        f.cmds()
            .filter_map(|c| match *c {
                Cmd::Text {
                    x, baseline, text, ..
                } if (top..top + 196).contains(&baseline) && x < 470 => Some((
                    x,
                    i32::from(baseline) - i32::from(top),
                    f.text_of(text).to_vec(),
                )),
                _ => None,
            })
            .collect()
    };
    let mut s = shell(false);
    s.set_station_from_tempest(storm);
    s.dirty(Rect::new(0, 0, W, H));
    let today = card_texts(&drawn(&mut s), 52);
    assert!(
        today.iter().any(|t| t.2 == b"78\xb0"),
        "the station's 25.6 C: {today:?}"
    );
    let mut s2 = weather(0);
    s2.set_station_from_tempest(storm);
    s2.dirty(Rect::new(0, 0, W, H));
    let panel = card_texts(&drawn(&mut s2), 80);
    for t in &today {
        assert!(
            panel.contains(t),
            "{t:?} not on the panel at the same place: {panel:?}"
        );
    }
}
