use rp_draw::{Glyph, MAX_CMDS};

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

fn shell(dev: bool) -> Shell {
    Shell::new(
        Fonts {
            ui: &FONT,
            label: &FONT,
            big: &FONT,
        },
        Icons::NONE,
        dev,
    )
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
        let mut g = Frame::new();
        s.scene(&mut g);
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
    assert!(t.contains(&b"High 75\x7f   Low 52\x7f".to_vec()));
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
fn the_hourly_list_scrolls_both_ways_and_stays_within_the_day() {
    let mut s = weather(1);
    let max = i32::from(weather_app::max_scroll(1));
    assert!(max > 400, "24 rows don't fit: {max}");
    s.input(Input::Key(Key::Up));
    assert_eq!(s.weather_page().1, 0);
    for _ in 0..30 {
        s.input(Input::Key(Key::Down));
    }
    assert_eq!(s.weather_page().1, max);
    assert!(!drawn(&mut s).overflowed());
    // A quick stroke with no moves reported: the list follows it 1:1.
    s.input(Input::PenDown { x: 200, y: 150 });
    s.input(Input::PenUp { x: 205, y: 290 });
    assert_eq!(s.weather_page().1, max - 140);
    s.input(Input::PenDown { x: 200, y: 400 });
    s.input(Input::PenUp { x: 200, y: 260 });
    assert_eq!(s.weather_page().1, max);
    // The pages that fit don't scroll by the D-pad; up and down move the
    // focus there.
    s.set_tab(2);
    s.input(Input::Key(Key::Down));
    assert_eq!(s.weather_page().1, 0);
}

/// The pen's samples, `step_ms` apart from `t0`, from `y0` to `y1` at `x`.
fn stroke(s: &mut Shell, t0: u32, x: u16, ys: &[u16], step_ms: u32) -> u32 {
    let mut t = t0;
    s.tick(t);
    s.input(Input::PenDown { x, y: ys[0] });
    for &y in &ys[1..] {
        t += step_ms;
        s.tick(t);
        s.input(Input::PenMove { x, y });
    }
    s.input(Input::PenUp {
        x,
        y: *ys.last().unwrap_or(&ys[0]),
    });
    t
}

#[test]
fn a_dragged_list_follows_the_pen_one_to_one_and_redraws_only_the_page() {
    let mut s = weather(1);
    s.tick(1_000);
    drawn(&mut s);
    s.input(Input::PenDown { x: 240, y: 400 });
    // Inside the slop it may still be a tap: nothing moves.
    s.input(Input::PenMove { x: 241, y: 395 });
    assert_eq!(s.weather_page().1, 0);
    assert!(!s.needs_frame());
    // Past it, 1:1 from where the pen went down (no jump).
    for (k, y) in [380u16, 352, 300].into_iter().enumerate() {
        s.tick(1_000 + 7 * (k as u32 + 1));
        s.input(Input::PenMove { x: 241, y });
        assert_eq!(s.weather_page().1, i32::from(400 - y), "sample {k}");
        let f = drawn(&mut s);
        assert_eq!(f.dirty(), &[weather_app::PAGE][..], "sample {k}");
    }
    // Held still and lifted: no fling.
    s.tick(1_200);
    s.input(Input::PenUp { x: 241, y: 300 });
    s.tick(1_400);
    assert_eq!(s.weather_page().1, 100);
    assert!(!s.animating());
}

#[test]
fn a_flick_flings_on_slows_down_and_stops_inside() {
    let mut s = weather(1);
    // ~2 px/ms upwards: later hours.
    let t = stroke(&mut s, 1_000, 240, &[500, 486, 472, 458, 444, 430], 7);
    let at_lift = s.weather_page().1;
    assert_eq!(at_lift, 70);
    assert!(s.animating(), "it carries on");
    s.tick(t + 100);
    let a = s.weather_page().1;
    assert!(a > at_lift + 100, "fast at first: {a}");
    s.tick(t + 200);
    let b = s.weather_page().1;
    assert!(b - a < a - at_lift, "slowing: {at_lift} {a} {b}");
    for k in 1..100 {
        s.tick(t + 200 + 50 * k);
    }
    assert!(!s.animating(), "it stops");
    let end = s.weather_page().1;
    assert!(end > b && end <= i32::from(weather_app::max_scroll(1)));
    // A touch catches a moving list.
    let t = stroke(&mut s, 20_000, 240, &[100, 130, 160, 190, 220], 7);
    assert!(s.animating());
    s.tick(t + 30);
    let moving = s.weather_page().1;
    s.input(Input::PenDown { x: 240, y: 300 });
    s.tick(t + 200);
    assert_eq!(s.weather_page().1, moving, "held where it was caught");
    assert_eq!(s.screen(), Screen::Weather, "the catch was no tap");
    s.input(Input::PenUp { x: 240, y: 300 });
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
fn back_goes_up_a_screen_by_softkey_header_or_home() {
    let mut s = weather(1);
    // The left softkey and the header's control both go back.
    tap(&mut s, SOFT_LEFT);
    assert_eq!(s.screen(), Screen::Today);
    let mut s = weather(1);
    tap(&mut s, weather_settings::BACK);
    assert_eq!(s.screen(), Screen::Today);
    // Settings, then a place, then back up one at a time.
    let mut s = weather(0);
    tap(&mut s, SOFT_RIGHT);
    assert_eq!(s.screen(), Screen::WeatherSettings);
    tap_t(&mut s, Target::Row(2));
    assert_eq!(s.screen(), Screen::WeatherPlaces);
    tap(&mut s, weather_settings::BACK);
    assert_eq!(s.screen(), Screen::WeatherSettings);
    tap(&mut s, SOFT_LEFT);
    assert_eq!(s.screen(), Screen::Weather);
    // Home from anywhere.
    tap(&mut s, SOFT_RIGHT);
    s.input(Input::Key(Key::Home));
    assert_eq!(s.screen(), Screen::Today);
}

#[test]
fn settings_rows_change_what_every_screen_shows() {
    let mut s = weather(0);
    tap(&mut s, SOFT_RIGHT);
    // Units: US customary to Metric, every number redrawn.
    drawn(&mut s);
    tap_t(&mut s, Target::Row(1));
    assert_eq!(s.region.temp, rp_weather::Temp::C);
    assert!(drawn(&mut s).dirty().contains(&Rect::new(0, 0, W, H)));
    // A place: picked from the list, named in the app's header.
    tap_t(&mut s, Target::Row(2));
    tap_t(&mut s, Target::Row(6));
    assert_eq!(s.wx.place, "Oslo");
    assert_eq!(s.screen(), Screen::WeatherSettings);
    // The source: kept, the sample still shown and saying so.
    tap_t(&mut s, Target::Row(3));
    assert_eq!(s.wset.source, rp_weather::settings::Source::OpenMeteo);
    assert_eq!(s.wx.source, "Sample (no network yet)");
    // Region: the next one brings its own units.
    tap_t(&mut s, Target::Row(0));
    assert_eq!(s.wset.region_info().name, "Canada");
    assert!(s.wset.metric);
    tap(&mut s, SOFT_LEFT);
    assert!(texts(&drawn(&mut s)).contains(&b"OSLO".to_vec()));
}

/// Taps target `t`.
fn tap_t(s: &mut Shell, t: Target) -> Action {
    let r = s.rect_of(t);
    tap(s, r)
}

/// Taps the keys for `text` on the key-entry screen (shift before a
/// capital).
fn type_keys(s: &mut Shell, text: &[u8]) {
    for &c in text {
        let i = weather_settings::cap_of(c).expect("a key for every character");
        if c.is_ascii_uppercase() {
            tap_t(s, Target::Cap(weather_settings::cap_of(0x0E).unwrap_or(0)));
        }
        tap_t(s, Target::Cap(i));
    }
}

#[test]
fn the_tempest_key_is_typed_masked_kept_and_never_drawn_in_full() {
    let key = b"0a1B2c3d-4e5f-6789-abcd-ef0123456789";
    let mut s = weather(0);
    tap(&mut s, SOFT_RIGHT);
    tap_t(&mut s, Target::Row(4));
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
    // Done keeps it and zeroes the typed copy.
    tap(&mut s, SOFT_RIGHT);
    assert_eq!(s.screen(), Screen::WeatherSettings);
    assert_eq!(s.wset.tempest_key.expose(), key);
    assert!(s.typed.iter().all(|&b| b == 0) && s.typed_n == 0);
    assert!(texts(&drawn(&mut s)).contains(&b"**** 6789".to_vec()));
    // Typed again and cancelled: the kept key stays, the typed one goes.
    tap_t(&mut s, Target::Row(4));
    type_keys(&mut s, b"abc");
    tap(&mut s, SOFT_LEFT);
    assert_eq!(s.wset.tempest_key.expose(), key);
    assert!(s.typed.iter().all(|&b| b == 0));
    // Home half way through forgets it too.
    tap_t(&mut s, Target::Row(4));
    type_keys(&mut s, b"xyz");
    s.input(Input::Key(Key::Home));
    assert!(s.typed.iter().all(|&b| b == 0) && s.typed_n == 0);
}

#[test]
fn every_new_screen_fits_a_frame_and_the_d_pad_reaches_its_targets() {
    for (row, screen) in [
        (None, Screen::WeatherSettings),
        (Some(2), Screen::WeatherPlaces),
        (Some(4), Screen::KeyEntry),
    ] {
        let mut s = weather(0);
        tap(&mut s, SOFT_RIGHT);
        if let Some(r) = row {
            tap_t(&mut s, Target::Row(r));
        }
        assert_eq!(s.screen(), screen);
        let f = drawn(&mut s);
        assert!(!f.overflowed(), "{screen:?}: {}", f.cmds().count());
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

static PX: [u32; 4] = [0xFFFF_FFFF; 4];
static PIC: Image = Image {
    w: 2,
    h: 2,
    px: &PX,
};

#[test]
fn pictures_replace_the_outlines_and_still_fit_a_frame() {
    let mut s = Shell::new(
        Fonts {
            ui: &FONT,
            label: &FONT,
            big: &FONT,
        },
        Icons {
            hero: Some(&PIC),
            card: [Some(&PIC); 22],
            row: [Some(&PIC); 22],
            radar: [Some(&PIC); 3],
        },
        true,
    );
    let f = drawn(&mut s);
    let images = f.cmds().filter(|c| matches!(c, Cmd::Image { .. })).count();
    assert_eq!(images, 6);
    assert!(!f.overflowed());
    // Without pictures the same places draw outlines and no images.
    let mut none = shell(true);
    let f = drawn(&mut none);
    assert_eq!(
        f.cmds().filter(|c| matches!(c, Cmd::Image { .. })).count(),
        0
    );
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
    assert_eq!(HERO.y + shift, 104, "the big panel read where the card was");
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
    assert!(v.row > LAYER_MARGIN, "scrolled down: {}", v.row);
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
    // Into Hourly: its scroll bar on screen 0 is drawn whole at once, not
    // clipped to Cascade's window (the emulator found it missing).
    tap(&mut s, weather_app::tab_rect(1));
    let f = drawn(&mut s);
    let bar = f
        .cmds_clipped()
        .filter(|(c, clip)| {
            let b = c.bounds(&f).intersect(clip);
            b.x >= W - 9 && b.h > 100
        })
        .count();
    assert_eq!(bar, 2, "the bar's track and thumb, whole");
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
    assert_eq!(line.x, next.x + 40, "{line:?}: on 10 Day, not past it");
}
