# Interaction foundations: the rules every screen follows

Part of the design area: [README.md](README.md). **A draft for the owner's
vote (2026-10-30), nothing decided yet.** The choices to vote on are
numbered I1-I12 at the end, each with a recommendation. Decided once here,
they hold for every app in both looks, so Weather ([weather-flows.md](weather-flows.md))
and every app after it is designed on them instead of deciding them again.

What is already decided and not asked again here ([ui.md](ui.md)): the two
looks, **Classic** (Windows Mobile's layouts, modernised, stylus scale) and
**Modern** (mostly SPB Mobile Shell, the owner's changes, the default);
stylus and D-pad equally, the finger for navigation only in Modern; the
title bar with Start at top left and status on every screen; two softkeys
at the bottom; buttons keep Windows Mobile's jobs; the smoked glass; one
font in the base (Didact Gothic); US units.

**How each rule is written (rule 0).** For every value: what Windows
Mobile, SPB or the textbooks do (a finding, tagged as a claim where it
isn't measured), what this exact hardware says (measured, or the run that
will measure it), and our choice with why it should beat the original.
Values marked *provisional* are a first draft to be replaced by the
measurement named beside them.

## 1. The screen as a physical thing

| Fact | Value | Source |
|---|---|---|
| Pixels | 480 x 640, portrait | measured (every run) |
| Size | 3.7 in diagonal (Dell's spec, a claim) | 800 px diagonal over 3.7 in |
| Density | **216 px per inch, 8.5 px per mm** | computed from the two above |
| Touch | resistive, single point, pressure read (TSC2046) | [hw/touch.md](../hw/touch.md) |
| Stylus accuracy | a landing slides 3-10 px before it settles; no tap pressed more than 20 px from where it lifted (drags aside); alignments agree within ~9 px | hw/touch.md (230+ recorded taps) |
| Tap to screen | median 16.2 ms, worst 43.6 ms (the ratchet's floors); promise 50 ms | R115, perf floors |
| Buttons | D-pad (4 ways + centre), Home, Calendar, Contacts, Mail, Record, Wi-Fi, power; **no softkey buttons, no Back key** | [hw/keypad.md](../hw/keypad.md) |
| D-pad repeat | 500 ms, then 20 a second (Windows Mobile's, a claim) | hw/keypad.md |

Everything below is sized in pixels with millimetres beside them, because a
finger and a stylus tip are physical sizes, not pixel counts.

## 2. Grid and spacing

**Found.** Windows Mobile 5 at VGA drew its QVGA layouts doubled (a
claim, to check: R124); SPB's demo shows a 3-across launcher, full-width
panels and edge-to-edge photo grids. Our shell today uses 16 px side
margins and an 8 px step between cards, by eye.

**Proposed: a 12-column grid, 30 px columns, 8 px gutters, 16 px side
margins.** 12 x 30 + 11 x 8 = 448 = 480 - 2 x 16, and it divides exactly:

| Across | Span | Width each | Used for |
|---|---|---|---|
| 1 | 12 columns | 448 px (52.7 mm) | panels, list rows, the hero |
| 2 | 6 | 220 px | paired panels, two buttons |
| 3 | 4 | 144 px (16.9 mm) | Modern's launcher, photo grids (SPB's 3 across) |
| 4 | 3 | 106 px | Classic's launcher, small tiles |
| 6 | 2 | 68 px | icon rows, tool bars |

Five across (the forecast row, the quick buttons) doesn't divide; it uses
the full 448 px with 8 px gutters (5 x 83 + 4 x 8 = 447, as built today).

**Vertical rhythm: an 8 px step** for gaps between blocks (8, 16, 24),
**4 px inside** a block (text to its panel's edge, icon to label). Every
rectangle's top and height a multiple of 4.

**Why this beats picking by eye:** one table every app uses means a theme
can change margins in one place (themes carry layout, ui.md), the D-pad's
spatial rules (§6) get clean rows and columns to work on, and partial
redraws snap to the 2D engine's and the 3D core's tiles (8 x 16 tiles: an
8 px grid never splits a tile edge sideways).

**The fixed bars (both looks keep them, ui.md):**

| Bar | Classic | Modern | Today |
|---|---|---|---|
| Title / status bar (top) | 36 px (4.2 mm) | 36 px | 36 |
| Softkey bar (bottom) | 40 px (4.7 mm) | 56 px (6.6 mm), *provisional* | 40 |
| Content between | 564 px | 548 px | 564 |

Modern's softkeys are navigation, so they are finger targets (§3); the
title bar stays thin in both, because a finger uses it only to pull the
notification shade down (a drag, which starts anywhere in the top 48 px:
§5).

## 3. Touch targets: stylus and finger

**Found.** The textbooks' finger sizes: Apple 44 pt (about 7 mm), Google
48 dp (about 7.6 mm), and a 2006 study of thumbs on a PDA-sized touch
screen (Parhi, Karlson and Bederson) about 9.2-9.6 mm (claims, not
measured here). Windows Mobile sized for the stylus. None of them was
measured on a resistive X51v panel, which reports one point for a whole
fingertip.

**Our way: measure the X51v, then size from the error.** A short device
run (R124, proposed) shows a grid of crosshairs; the owner taps each with
the stylus, then with a fingertip, then a thumb, held naturally. From the
recorded taps: the miss distance at the 99th percentile for each, and the
pressure reading (Rtouch) each produces. A target's **hit area** is then
the smallest size that keeps misses under 1 in 100, and its **drawn size**
can be smaller (below). Until then, provisional values:

| | Stylus (both looks) | Finger (Modern navigation) |
|---|---|---|
| Hit area, minimum | 32 x 32 px (3.8 mm) | 64 x 64 px (7.5 mm) |
| Standard row / button height | 40 px (4.7 mm) | 64 px |
| Gap between targets | 4 px min | 8 px min |

**Three ways we beat a fixed size, all cheap:**

1. **Hit areas larger than the art.** Each target's hit area grows into
   the empty space around it, up to half the gap to its neighbour (the
   space between two targets is split down the middle, so nothing is
   dead). A small-looking control keeps a finger-sized hit area wherever
   there is room. Computed from the layout's rectangles, no per-screen
   work.
2. **A tap between two targets goes to the nearer, or to neither.** If
   the tap's point is within a few pixels of the line halfway between two
   targets (the margin from R124's spread), nothing happens instead of the
   wrong thing (a stray tap costs nothing; a wrong action costs a lot).
3. **Finger or stylus, told apart by the panel itself (a candidate).**
   The TSC2046 reads contact resistance; a fingertip's wide, soft contact
   should read differently from a hard stylus tip. If R124 shows the two
   apart, a finger touch gets the larger hit areas and a stylus touch the
   precise ones, on the same screen, with no setting. No 2005 device did
   this. If they overlap, we drop it.

**Precise controls stay stylus-sized in both looks** (decided, ui.md): the
keyboard, text selection, small controls, old programs.

## 4. Softkeys and buttons

**Found (Windows Mobile 5, claims to check on the owner's X51v, R124):**
the left softkey is the screen's most likely action (often "New"), the
right one is "Menu", opening a menu upward; a softkey may be blank; a
detail screen returns with "ok" at the title bar's right; the X at the top
right hides a program instead of closing it; tap and hold opens a context
menu (a ring of red dots first). The X51v has no softkey buttons, so on
it the softkeys are stylus only. SPB's demo keeps the title bar and the
two softkeys as they are.

**Proposed rules (both looks):**

- **Left softkey: the one action this screen is for** (New, Refresh,
  Done, Send). **Right softkey: Menu**, everything else, opening upward
  from it. Never blank: a screen with no action shows **Back** on the
  left (§5).
- **Labels:** one or two words, at most about 12 characters (fits 160 px
  in Classic at the UI size), sentence case.
- **No "Are you sure?" for anything that can be undone.** Delete, remove,
  move act at once and show a short notice with **Undo** for 5 s (a
  modern pattern; WM asked first). Asked first only when it can't be
  undone (formatting a card, a factory reset).
- **Menus:** at most about 12 rows at a time, the most used at the
  bottom nearest the softkey (the shortest reach), separators between
  groups, a submenu one level deep at most.
- **Hardware buttons keep Windows Mobile's jobs** (decided): Home goes
  to the home screen, Calendar to Calendar, Mail to Inbox, Contacts to
  Contacts (not a phone, so its program is open: I10), Record records
  while held, Wi-Fi switches the radio. **No button gains a job** without
  the owner's yes (I9 asks one).
- **The D-pad reaches the softkeys:** down from the screen's last row
  focuses the softkey bar (it does today). Not a new button job: the
  softkeys are on screen, and WM's own answer (mapping hardware buttons to
  softkeys in Settings, Buttons) stays available as a setting.

## 5. Navigation

**Found.** Windows Mobile: Start menu at top left; programs kept running
when "closed" with the X; "ok" returns from a detail or dialog; no Back
key on the X51v; Home to the Today screen. SPB: Today, a launcher grid,
panels swiped sideways; the WM bars kept.

**Proposed model, the same in both looks (only sizes and motion
differ):**

- **Depth is short:** home, an app's main screen, a detail, at most one
  more. A deeper flow is a sign the screen needs redesigning.
- **Back is always in one place: the title bar's right end** (where WM
  put "ok"), shown as **ok** in Classic (WM's word) and a back arrow in
  Modern, on every screen below an app's main screen. The left softkey
  shows Back only when the screen has no other action (§4). The D-pad has
  no Back key, so Back is a focusable target like any other.
- **Programs are never closed by hand.** Leaving a program keeps its
  place; the system frees memory by saving a program's state and
  restoring it when reopened (the modern phone model; WM's X that "hid"
  rather than closed confused people for years). A program shows the
  same screen when you come back, until it chooses otherwise.
- **Home button:** the home screen from anywhere, scroll position reset
  to the top on a second press.
- **Moving between programs:** the task switcher (W5, the card stack),
  opened from the status bar (a tap on the clock, the shade's first row)
  in both looks; I9 asks whether Home held should open it too.
- **Modern's gestures (finger):** swipe sideways between home panels;
  pull down from the top 48 px for the notification shade (W9); flick to
  scroll with momentum. Nothing else is a gesture: every gesture also
  has a visible target or a D-pad way, because a gesture you can't see is
  one you can't find.
- **Classic:** no gestures beyond tap, drag (scroll bars, dragging an
  item) and tap and hold.
- **Tap and hold** (both): the context menu at 500 ms *provisional*
  (WM's red-dot ring, our own drawing: a ring filling around the stylus),
  cancelled if the point moves more than the hit slop.

## 6. D-pad focus

**Found.** Windows Mobile showed a selection highlight in lists and moved
by each program's own tab order; programs that forgot left items
unreachable. Our shell today already moves spatially: the nearest target
in the pressed direction, centre to centre, within a cone (`neighbour` in
`shell/core/src/lib.rs`).

**Proposed:**

- **Spatial, from the layout, for every screen:** a direction picks the
  target whose rectangle overlaps the current one across the movement
  (same column for up and down, same row for left and right) and is
  nearest; only if none overlaps, the nearest within the cone (today's
  rule). No program writes a tab order.
- **Focus memory:** going up then down returns to the item you came from
  (a row remembers which of its items had focus), so a grid feels
  predictable.
- **Proved for every screen by a host test:** every target reachable from
  every other, up then down returns where the geometry allows, and no
  target is a dead end. A layout that breaks it fails the build (the
  "left items unreachable" class fixed for good, not per screen).
- **The ring shows only when the D-pad is in use** (what browsers call
  focus-visible): a stylus or finger tap hides it; the first D-pad press
  shows it on the **screen's default target** (the hero on the home
  screen, the first list row) instead of moving. Today the first press
  goes to Start; the default target is nearer what you want.
- **Ring style:** 2 px light ring with a 1 px dark line outside it, so it
  shows on any wallpaper, light or dark (one colour alone vanishes on
  some photos).
- **Lists:** up and down move by row and scroll to keep one row of
  context beyond the focused one; held, the repeat (500 ms, 20 a second)
  speeds up to a page a step after 2 s *provisional*. Left and right in a
  list do the list's own sideways action (another place in Weather, a day
  in Calendar) or nothing; they never jump out of the list.
- **Centre (Enter):** acts on the focused target. Held 1 s: its context
  menu, as a stylus's tap and hold (I8: WM gave the centre no hold job,
  to check).
- **Edges:** no wrapping around (wrapping makes "where am I" harder);
  the D-pad stops, and the ring nudges to say so.

## 7. States

Every target and every screen shows which state it is in; none is left to
guess.

| State | Looks like | Rule |
|---|---|---|
| Resting | its art | |
| Pressed | a light wash over it (today's `PRESSED`), shown at the touch filter's verdict (median 7 ms after landing) | acts on **lift inside its hit area**; sliding off cancels, sliding in a scrolling list turns into a scroll |
| Focused | the ring (§6) | only while the D-pad is in use |
| Selected / on | filled or checked mark, never colour alone | |
| Disabled | dimmed (`DIM`), no press | skipped by the D-pad; a tap shows why in a short notice ("Needs a network") instead of nothing |
| Working | a small spinner in place of the control's icon, after 300 ms (shorter work shows nothing) | input never waits on it; Back always works |
| Empty | a line of plain words saying why and the one action that fills it ("No places yet. Add a place") | never a blank screen |
| Stale | the data shown with its age ("Updated 9:40 AM") | the age goes amber after a set time (per app) |
| Error | plain words, what happened and what to do, with the action as a button | never a code alone |
| Sample data | the words "sample data" on it | until the real source exists (decided practice, today-screen-plan.md) |

**Feedback timing:** the press shows within the 50 ms promise (measured
16.2 ms median); anything that takes longer than 100 ms after the press
shows that it started (the screen changes, or the spinner after 300 ms).

**Motion** (decided targets, workload-brief.md: 60 fps when the finger
drives it, 30 for the task switcher and the sky, W3 within ~250 ms):
transitions 150-250 ms, eased out; nothing animates when nothing changes;
every effect can be turned off in Settings (decided, ui.md), and then a
change is instant, not slower.

## 8. Type roles

**Found.** Our base font is Didact Gothic at three sizes: 18 px (menus,
softkeys, small text), 15 px spaced capitals (labels, dates), 76 px (the
big numerals). Didact Gothic has **one weight**, so roles differ by size,
case, spacing and colour, never by bold. On this screen 18 px is 2.1 mm,
**smaller than Windows Mobile's own body text** (Tahoma 9 pt at VGA's 192
dpi is about 24 px, a claim to check: R124) and than iOS and Android body
text at their sizes (about 2.6 mm).

**Proposed roles (six sizes, three of them new):**

| Role | Size | Case, spacing | Used for | Status |
|---|---|---|---|---|
| Display | 76 px, light numerals | | the clock, the temperature | built |
| Title | 30 px | sentence case | a screen's or a panel's name, sheet titles | new |
| Heading | 24 px | sentence case | group headings, the place name | new |
| Body | **22 px** (2.6 mm) | sentence case | list rows, readable text | new, replaces 18 for reading |
| UI | 18 px | sentence case | softkeys, menus, secondary lines | built |
| Label | 15 px, +2 tracking | SPACED CAPITALS | labels, dates, units | built |

Nothing smaller than 15 px. Colour: `TEXT` for what you read first,
`DIM` for the second line; contrast checked on the darkest and lightest
spot of every shipped wallpaper under its glass (a build check, not by
eye).

**Our way, beyond sizes:**
- **Glyphs made on the PC for this panel:** each size is drawn on the PC
  (as today), with its anti-aliasing's gamma fitted to the X51v panel's
  measured response ([display-calibration.md](display-calibration.md)),
  so thin strokes of a light font don't wash out. Free at run time.
- **Sub-pixel text (a candidate):** if the panel's sub-pixel order is
  known (an eye check on a test pattern), text can be drawn at three
  times the horizontal detail, as ClearType did on desktops. At 216 px
  per inch it may not be needed; one eye check decides.
- **Cost:** each new size is a few KB to ~15 KB of glyphs in flash
  (measured when built, against the shell's budget); three new sizes are
  the price of readable text, and the owner votes it (I11).

## 9. What this page doesn't decide

- Colours, the glass and the art: decided (ui.md, boards N-R).
- The keyboard's layout and size: its own page (stylus-sized, decided).
- Landscape: W7's rotation exists in the workload; layouts for it come
  per app once portrait is agreed.
- Accessibility beyond the above (a larger-text setting, high contrast):
  the type roles are built so one setting can scale them; its own vote
  later.

## The votes

Each with the recommendation first. Values marked *provisional* are
replaced by R124's measurement whatever is voted.

- **I1 Grid:** A 12 columns / 30 px / 8 px gutters / 16 px margins, 8 px
  rhythm (recommended); B keep sizing by eye per screen.
- **I2 Bars:** A title 36 both looks, softkeys 40 Classic and 56 Modern
  (recommended); B 40 in both; C WM's own heights from R124.
- **I3 Targets:** A measure on the X51v (R124), then set sizes from the
  miss rate; provisional stylus 32 / finger 64 until then (recommended);
  B take a textbook size now and keep it.
- **I4 Hit areas grow into empty space, ties do nothing:** A yes
  (recommended); B hit area = the art.
- **I5 Finger told from stylus by the panel's pressure reading:** A try
  it in R124 and adopt only if they separate (recommended); B don't try.
- **I6 Softkeys:** A left = the screen's action, right = Menu, never
  blank, Undo instead of "Are you sure?" (recommended); B WM's rules as
  they were (blank allowed, confirmations).
- **I7 Back:** A at the title bar's right end on every screen below an
  app's main one ("ok" in Classic, an arrow in Modern), left softkey only
  when there's no other action (recommended); B the left softkey always
  Back; C WM's way (ok on some screens, X on others).
- **I8 D-pad:** A spatial from the layout with focus memory, proved by a
  host test per screen, the ring only when the D-pad is used, the first
  press shows the default target, Enter held = context menu
  (recommended); B as built today (first press to Start, ring always).
- **I9 Task switcher's way in:** A from the status bar only (recommended:
  no new button job); B also Home held 1 s (a new job for Home: your
  call).
- **I10 The Contacts button:** A keeps opening Contacts (WM's job; a
  Contacts app is in the WM5 list, apps-and-packs.md) (recommended); B a
  setting picks its program, Contacts by default.
- **I11 Type:** A six roles, body 22 px, three new sizes (recommended);
  B keep the three sizes built.
- **I12 Programs never closed by hand:** A the system saves and restores
  a program's state; no close button anywhere (recommended); B a close
  button as well, for people who want one.

## Reminders opened by this page

- **R124** (device run, proposed): the target-acquisition run (stylus,
  fingertip, thumb on a crosshair grid; miss spread and pressure per
  input), plus reading Windows Mobile 5's own sizes on the X51v (bar
  heights, list row height, its body font size, tap and hold time, the
  button jobs in Settings, Buttons). Needs a sheet and the owner's go.
