//! The game path's route for each frame (ADR-0038, R90): which of the three
//! routes a game's next frame takes, from the frames it has drawn.
//!
//! **Rule 0.** Consoles' dynamic resolution (the PS4 / Xbox One era) scales
//! the render target per frame from the GPU's time on a fixed ladder of
//! sizes, sharpest first. ADR-0038 G2 (the owner's ruling, 2026-10-29)
//! keeps the per-frame choice and changes the ladder: the routes are
//! ranked by **their own measured times for this game**, not by their
//! pixel counts, because on the X51v the lighter route is not always the
//! faster one (the smooth route was 22% *slower* than native on the flat
//! test frame at 74.75 / 1, 12.4 against 10.2 ms; ADR-0038 G1). So a step
//! down goes to the sharpest route measured faster than the frames it
//! leaves, never to one measured slower, and a step up goes only to a
//! sharper route whose own time fits the budget: no route is entered on a
//! guess its own numbers already refute. Each step costs no copy (the
//! display's doubling and scaler do the resizing).
//!
//! The thresholds are G2's provisional 8 and 60 frames, to be fitted on
//! the first game port; the budget is the port rule's 33.3 ms (30 fps). G3:
//! a game can be held to one route. G4: two frames in flight on the lines
//! route only; the smooth route only as (d'), its 2D placements apart from
//! the render. G1: the smooth route only after the owner's eye check of its
//! colour (R89); until then it is never chosen.

/// A route to the screen, sharpest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// 480 x 640, shown as drawn.
    Native = 0,
    /// 480 x 320, each line shown twice (`GDRCTRL` bit 29).
    Lines = 1,
    /// 240 x 320, UYVY, the video plane's scaler 2x.
    Smooth = 2,
}

impl Route {
    /// Every route, sharpest first.
    pub const ALL: [Self; 3] = [Self::Native, Self::Lines, Self::Smooth];

    /// Frames in flight on this route (G4: two on the lines route, way
    /// (e), 800 of 800 right; one elsewhere).
    #[must_use]
    pub const fn in_flight(self) -> u8 {
        match self {
            Self::Lines => 2,
            Self::Native | Self::Smooth => 1,
        }
    }

    /// Whether its 2D placements must wait for the render (G4: the smooth
    /// route as (d') only; (d), the 2D beside the render, left stray words,
    /// R30).
    #[must_use]
    pub const fn placements_apart(self) -> bool {
        matches!(self, Self::Smooth)
    }
}

/// A game's route setting (G3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hold {
    /// Chosen per frame.
    Auto,
    /// Held to one route.
    Route(Route),
}

/// Frames whose mean, over the budget, steps down (G2, provisional).
pub const DOWN_FRAMES: usize = 8;
/// Frames in a row under the budget before a step up (G2, provisional).
pub const UP_FRAMES: u32 = 60;
/// The frame budget: 30 fps (the port rule).
pub const BUDGET_US: u32 = 33_333;

/// The route chooser for one game.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Picker {
    budget_us: u32,
    hold: Hold,
    smooth_ok: bool,
    route: Route,
    /// Each route's time for this game: the mean of its last
    /// [`DOWN_FRAMES`] frames, once it has drawn that many.
    times: [Option<u32>; 3],
    last: [u32; DOWN_FRAMES],
    n: usize,
    under: u32,
}

impl Picker {
    /// A game's chooser, starting at the sharpest route it may take.
    /// `smooth_ok`: the owner has passed the smooth route's colour (R89).
    #[must_use]
    pub const fn new(budget_us: u32, hold: Hold, smooth_ok: bool) -> Self {
        let route = match hold {
            Hold::Route(Route::Smooth) if !smooth_ok => Route::Lines,
            Hold::Route(r) => r,
            Hold::Auto => Route::Native,
        };
        Self {
            budget_us,
            hold,
            smooth_ok,
            route,
            times: [None; 3],
            last: [0; DOWN_FRAMES],
            n: 0,
            under: 0,
        }
    }

    /// The route the next frame takes.
    #[must_use]
    pub const fn route(&self) -> Route {
        self.route
    }

    /// Each route's measured time for this game.
    #[must_use]
    pub const fn times(&self) -> [Option<u32>; 3] {
        self.times
    }

    fn allowed(&self, r: Route) -> bool {
        r != Route::Smooth || self.smooth_ok
    }

    fn mean(&self) -> u32 {
        (self.last.iter().map(|&t| u64::from(t)).sum::<u64>() / DOWN_FRAMES as u64) as u32
    }

    fn enter(&mut self, r: Route) -> Option<Route> {
        self.route = r;
        self.n = 0;
        self.under = 0;
        Some(r)
    }

    /// A frame drawn on [`Self::route`] in `us` (the whole frame: set-up,
    /// render, poll): the route to switch to before the next, if any (the
    /// caller switches in the blanking, way C').
    pub fn frame(&mut self, us: u32) -> Option<Route> {
        self.last[self.n % DOWN_FRAMES] = us;
        self.n += 1;
        self.under = if us <= self.budget_us {
            self.under + 1
        } else {
            0
        };
        if self.n >= DOWN_FRAMES {
            self.times[self.route as usize] = Some(self.mean());
        }
        if self.hold != Hold::Auto {
            return None;
        }
        let here = self.route as usize;
        if self.n >= DOWN_FRAMES && self.mean() > self.budget_us {
            // Down: the sharpest lighter route measured faster than these
            // frames, or not measured yet (tried, then known).
            let now = self.mean();
            let to = Route::ALL[here + 1..]
                .iter()
                .copied()
                .find(|&r| self.allowed(r) && self.times[r as usize].is_none_or(|t| t < now));
            return to.and_then(|r| self.enter(r));
        }
        if self.under >= UP_FRAMES {
            // Up: the sharpest sharper route whose own time fits the
            // budget, or not measured yet.
            let to = Route::ALL[..here]
                .iter()
                .copied()
                .find(|&r| self.times[r as usize].is_none_or(|t| t <= self.budget_us));
            self.under = 0;
            return to.and_then(|r| self.enter(r));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(p: &mut Picker, us: u32, frames: u32) -> Option<Route> {
        let mut last = None;
        for _ in 0..frames {
            if let Some(r) = p.frame(us) {
                last = Some(r);
            }
        }
        last
    }

    #[test]
    fn a_heavy_scene_steps_down_after_eight_frames_and_not_before() {
        let mut p = Picker::new(BUDGET_US, Hold::Auto, true);
        assert_eq!(p.route(), Route::Native);
        for _ in 0..DOWN_FRAMES - 1 {
            assert_eq!(p.frame(40_000), None);
        }
        assert_eq!(p.frame(40_000), Some(Route::Lines));
        assert_eq!(p.times()[0], Some(40_000));
    }

    #[test]
    fn one_slow_frame_among_fast_ones_steps_nothing() {
        let mut p = Picker::new(BUDGET_US, Hold::Auto, true);
        for i in 0..200 {
            assert_eq!(p.frame(if i % 8 == 3 { 60_000 } else { 20_000 }), None);
        }
    }

    #[test]
    fn the_ranking_is_by_measured_time_not_by_pixels() {
        // The smooth route measured slower than native (ADR-0038 G1's flat
        // frame at 74.75 / 1): a step down from native skips it.
        let mut p = Picker::new(BUDGET_US, Hold::Auto, true);
        p.times = [None, Some(50_000), Some(45_000)];
        assert_eq!(
            run(&mut p, 40_000, DOWN_FRAMES as u32),
            None,
            "both measured slower"
        );
        p.times[2] = Some(30_000);
        p.n = 0;
        assert_eq!(run(&mut p, 40_000, DOWN_FRAMES as u32), Some(Route::Smooth));
    }

    #[test]
    fn a_step_up_needs_sixty_frames_under_and_a_route_that_fits() {
        let mut p = Picker::new(BUDGET_US, Hold::Auto, true);
        assert_eq!(run(&mut p, 40_000, DOWN_FRAMES as u32), Some(Route::Lines));
        // Native is known at 40 ms: however long the lines route runs
        // fast, it doesn't go back to a route its numbers refute.
        assert_eq!(run(&mut p, 20_000, 200), None);
        assert_eq!(p.route(), Route::Lines);
        // The scene gets lighter on native's account (a new measurement
        // would be needed; a known fit is): it steps up after 60.
        p.times[0] = Some(30_000);
        p.under = 0;
        assert_eq!(run(&mut p, 20_000, UP_FRAMES - 1), None);
        assert_eq!(p.frame(20_000), Some(Route::Native));
    }

    #[test]
    fn the_smooth_route_waits_for_the_owners_colour_check() {
        let mut p = Picker::new(BUDGET_US, Hold::Auto, false);
        assert_eq!(run(&mut p, 50_000, DOWN_FRAMES as u32), Some(Route::Lines));
        assert_eq!(run(&mut p, 50_000, 100), None, "never smooth before R89");
        assert_eq!(
            Picker::new(BUDGET_US, Hold::Route(Route::Smooth), false).route(),
            Route::Lines
        );
        assert_eq!(
            Picker::new(BUDGET_US, Hold::Route(Route::Smooth), true).route(),
            Route::Smooth
        );
    }

    #[test]
    fn a_held_game_never_switches_but_still_measures() {
        let mut p = Picker::new(BUDGET_US, Hold::Route(Route::Native), true);
        assert_eq!(run(&mut p, 90_000, 100), None);
        assert_eq!(p.route(), Route::Native);
        assert_eq!(p.times()[0], Some(90_000));
    }

    #[test]
    fn in_flight_and_placements_follow_g4() {
        assert_eq!(Route::Lines.in_flight(), 2);
        assert_eq!(Route::Native.in_flight(), 1);
        assert_eq!(Route::Smooth.in_flight(), 1);
        assert!(Route::Smooth.placements_apart());
        assert!(!Route::Lines.placements_apart() && !Route::Native.placements_apart());
    }
}
