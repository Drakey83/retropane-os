//! The glass panels: every box on every screen, in both looks (the owner's
//! pick, 2026-11-01: design B, "B but make the white closer to the grey
//! from this picture", then "looks good"; the mock-ups are
//! docs/design/mockups/glass-b-grey-*.png). Frosted grey glass in the light
//! look, frosted dark glass in the dark, 96% and 90% opaque, rounded
//! corners, a crisp bright rim, a sheen at the top.
//!
//! Iterate (rule 0): the web draws glass with a CSS backdrop, an inset box
//! shadow and a gradient, composited per pixel each frame by a GPU; Windows
//! Mobile drew flat boxes. The X51v's 2D engine has no shader, so the
//! panel is computed once off the device (`tools/shell-assets/panel_art.py`,
//! a supersampled signed distance to a rounded rectangle, the mock-up's own
//! model) into [`rp_draw::PanelArt`]: a colour per row, the rim's rows and
//! columns, four 12 x 12 corner images. The engine draws a box of any size
//! from it with a blended fill per run of like rows and four small images:
//! the mock-up's look, nothing computed per frame.

use rp_draw::{Argb, Cmd, Frame, PanelArt, Rect};

use crate::panel_art;

/// The panel art for the look (`light` the light look).
#[must_use]
pub fn art(light: bool) -> &'static PanelArt {
    if light {
        &panel_art::LIGHT
    } else {
        &panel_art::DARK
    }
}

/// The body's colour at its middle: a box too small for the corners (a
/// thin row, a chip under 26 px) is one fill of it.
#[must_use]
pub fn body(light: bool) -> Argb {
    art(light).rows[32]
}

/// Whether `c` is the panels' own (the light look keeps it).
#[must_use]
pub fn is_panel(c: Argb) -> bool {
    c == body(true)
}

/// Draws box `r` as glass in the look.
pub fn draw(f: &mut Frame, r: Rect, light: bool) {
    let art = art(light);
    let k = 2 * art.radius + 2;
    if r.w < k || r.h < k {
        f.push(Cmd::Fill {
            r,
            color: body(light),
        });
        return;
    }
    f.push(Cmd::Panel { r, art });
}
