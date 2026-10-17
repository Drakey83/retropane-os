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
    // Down to the quick row, across it, down to the softkeys.
    s.input(Input::Key(Key::Down));
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
    assert_eq!(
        hh_mm(
            &mut b,
            Time {
                hour: 7,
                minute: 5,
                ..Time::default()
            }
        ),
        b"07:05"
    );
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
    assert_eq!(hh_mm(&mut b, bad), b"23:59");
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
    for step in 0..300 {
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
        assert!(
            inc.plane() == full.plane(),
            "step {step}: incremental differs from full"
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

#[test]
fn temperatures_and_day_names_format() {
    let mut b = [0; 32];
    let n = high_low(&mut b, 75, 52);
    assert_eq!(&b[..n], b"High 75\x7f   Low 52\x7f");
    let n = high_low_short(&mut b, 73, 5);
    assert_eq!(&b[..n], b"73\x7f 05\x7f");
    assert_eq!(two(250), *b"99");
    assert_eq!(day_name(6, 1), b"MON");
    assert_eq!(day_name(4, 5), b"WED");
    assert_eq!(day_name(9, 1), b"MON"); // a weekday out of range is clamped to Sunday
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
            forecast: [Some(&PIC); 5],
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
    assert_eq!(s.notice(), Some(SHOWING_32));
    let f = drawn(&mut s);
    assert!(f.dirty().contains(&Rect::new(0, 0, W, H)));
    assert!(f.cmds().count() > 0, "the depth's notice");
    s.tick(NOTICE_MS + 1);
    assert_eq!(drawn(&mut s).cmds().count(), 0);
    // A tap anywhere switches the depth, and back; nothing else acts.
    assert_eq!(tap(&mut s, START), Action::WallpaperDepth { sixteen: true });
    assert_eq!(s.notice(), Some(SHOWING_16));
    assert!(drawn(&mut s).dirty().contains(&Rect::new(0, 0, W, H)));
    assert_eq!(
        tap(&mut s, SOFT_RIGHT),
        Action::WallpaperDepth { sixteen: false }
    );
    assert_eq!(
        tap(&mut s, SOFT_RIGHT),
        Action::WallpaperDepth { sixteen: true }
    );
    assert!(s.wallpaper_only());
    assert_eq!(s.input(Input::Key(Key::Down)), Action::Nothing);
    // Home goes back to the Today screen, at the depth chosen (named).
    assert_eq!(s.input(Input::Key(Key::Home)), Action::Redraw);
    assert!(!s.wallpaper_only());
    assert_eq!(s.notice(), Some(SHOWING_16));
    assert!(drawn(&mut s).cmds().count() > 20);
}
