# The design language: one page every app follows

**Version 3, 2026-11-14** (the owner: "establish our design language for
this entire os"; materials and what every screen must contain, below the
frame). Version 2 (the owner's verdict on batch 1) and what version 1 got
right are kept.

Distilled 2026-11-13 from what is built and liked: the Today shell
(`shell/core`, `shell/today`), the Weather app and its settings screens
(`shell/core/src/weather_app.rs`, `weather_settings.rs`), the glass panels
(`shell/core/src/panel.rs`), the keyboard (`keyboard.rs`),
[icon-brief.md](icon-brief.md) and the icon prompt. It records the look; it
doesn't re-decide it. An app that needs something this page lacks adds it
here first, so five apps never drift into five looks. How a design is
argued: [principles.md](principles.md), modelled in
[weather-rule0.md](weather-rule0.md).

## The frame every screen shares (480 x 640, portrait)

| Band | Where | What |
|---|---|---|
| Title bar | y 0-36 | Start (left, 104 px), the app's name, battery and clock (right). The shell's, never the app's |
| Header | y 36-94 | A back chevron (64 px wide), the screen's name in spaced capitals; at most two controls on the right |
| Content | y 98-594 | Panels on the wallpaper, 16 px side margins (448 px wide) |
| Softkey bar | y 600-640 | Two softkeys, 160 px each: left is the way back or the quiet action, right is the screen's main action |

- Keys keep their Windows Mobile jobs: Home goes to Today, the D-pad moves
  a focus ring, Enter acts, the title bar's ok and the left softkey go back.
- **Every target works by stylus and by D-pad.** Nothing is reachable by
  one only.

## Version 3 (2026-11-14): the design language of the whole OS

The owner on the X51v, after version 2: the apps are "as basic as it can
possibly be", "just a bunch of fields"; an app's window "needs to be more
fancy then just a plain white or grey background, look into gradience and
aero and glass, windows vista, XP, retro modern design"; "the weather app
should be looked at as a template as to the attention to care we need to
bring to all of these apps"; and: "establish our design language for this
entire os". Version 2's colour, type, spacing, keyboard and panel rules
stand. Version 3 adds what they lacked: **materials** and **what every
screen must contain**. Concept screens of all five apps, drawn by the
shell and renderer the X51v runs (`xtask/src/design_v3.rs`,
`RP_PREVIEW_ONLY=v3- cargo xtask ui-preview`), are in
docs/reports/design-v3-2026-11-14/.

### Found: why Weather reads as finished and the apps as forms

Read off Weather's own screens beside the apps' (both rendered):

| Weather has | The version 2 apps had |
|---|---|
| A **hero**: one big number and a picture that answer at a glance | A title and rows |
| **Instruments**: data drawn (a compass, a UV arc, the sun's path, range bars, a curve) | Data written as text |
| A **varied grid**: cards of different sizes for different jobs | One row height, repeated |
| **Light behind the glass**: a photo the glass tints | A flat sheet |

### Borrow and beat

| Problem | Vista / XP / Aero | Windows Mobile, TouchFLO 3D | Today's best (iOS, Material, Fantastical, Things, GoodNotes) | Ours |
|---|---|---|---|---|
| What is behind an app's content | Aurora: a painted picture; thin glass over the desktop | A flat system colour; TouchFLO: full-screen tabs with rich graphics | Tonal surfaces from one seed colour; Liquid Glass over content | A backdrop of light **computed from the app's tint** (`rp_app::aurora`): no file, any theme colour gets one, painted when an app opens |
| Glass | Thin, a bright rim, a sheen | Opaque boxes | Frosted layers, specular edges | Two glasses: the wallpaper's near-opaque one (words over any photo) and the **window's thin one** over the calm backdrop, so the light glows through |
| Buttons | Gel: a bright upper half, a hard line, a glow when chosen | Flat grey | Flat tints, pills | The gel key in both looks; the chosen one is the blue glow (XP and Vista's selection) |
| Numbers | Gadgets: dials and meters | Text | Rings (Activity, Things), heat maps (Timepage), range bars | **Drawn**, with the kit's ring, bars, ribbon and tiles: made once as pictures, no pixel work per frame |
| Where you are in an app | Tabs under the title | Softkeys and menus; TouchFLO: a tab strip along the foot | A tab bar at the foot | Tabs along the foot, as Weather's, under the thumb; softkeys keep their Windows Mobile jobs |

### Materials (back to front)

1. **The backdrop** (`rp_app::aurora::paint`). Light falling from the top
   (lit under the title bar, deep at the softkeys), three ribbons fanning
   up to the right with a bright core in a wide glow (Aurora's), a glow
   from the top left where the Aero light comes from; in the light look
   clear at the top, the tint gathering toward the foot. Worked out on a
   grid of one node per 8 x 8 pixels in integers, spread bilinearly and
   dithered to the panel's 16 bits (no banding: a test holds neighbours
   within two levels). The tint's own bits place the ribbons, so each app
   has its own sky. The accents are muted for marks on glass (version 2);
   the backdrop takes the colour back (each channel pushed from the tint's
   grey by three quarters): light, not a mark.
2. **The window's glass** (`panel::window`). Dark: 64% opaque, a brighter
   rim, a sheen. Light: white frost at 72% with a cool darker edge (a
   white rim on white glass has no edge). Cards on the wallpaper (Today)
   keep the near-opaque glass.
3. **Gel keys** (`kit::key`): graphite gel in the dark, white gel in the
   light, the chosen one the blue glow in both.
4. **Paper** (Notes): real paper colours, marked `kit::keep`, the same in
   both looks; a sheet has a binding or a fold of shade, never a rim.
5. **Pictures an app paints** (rings, discs, avatars) use
   `kit::painted(cx, colour)` and `kit::track(cx)`: the look's pass
   doesn't recolour pictures, so they are painted for the look.

A panel's corners are painted with the same gradient rows the body's top
and bottom bands use (`rp_draw::panel_cmds`: those bands take the art's
first and last rows as they are, the middle stretches), so a corner never
differs from the body beside it on a glass with a real gradient.

### Colour on the window's glass

The light look's accents are computed twice by `accents.py`: against the
wallpaper's grey glass (`LIGHT_TWINS`, version 2) and against the window's
white glass (`WINDOW_TWINS`): the same rule (hue and chroma kept in OKLCH,
darkened only to 3:1 for a fill and 4.5:1 for text), against a far
lighter glass, so coral stays coral where it had to become maroon.

### Every app's home has (the Weather rule)

1. **A hero**: the answer to the app's first question, large (the date
   and what is next; today's progress; who you talk to; the pinned note;
   the answer).
2. **Its numbers drawn**: a ring for a part of a whole
   (`kit::ring`), a ribbon for a day, bars for load, a progress bar for a
   list. A number is written large only once a screen.
3. **A varied grid**: a hero card, a row of small tiles, then lists in
   grouped cards (rows inside one glass card with hairlines between, not
   a stack of separate pills).
4. **Actions where the subject is**: gel keys on the card they act on
   (Directions on the next appointment, Call on the person).
5. **Tabs along the foot** for an app's views (Weather's Now / Hourly /
   10 Day); the softkeys keep back and the main action.
6. **Everything drawn answers the stylus** (`kit::Hits`: what is drawn as
   tappable records its own target) and the D-pad.

### The components version 3 adds to the kit

| Component | Kit | Use |
|---|---|---|
| Ring gauge | `kit::ring`, `paint_ring` | A part of a whole: progress, a countdown |
| Glossy disc | `kit::gloss_disc` | Big avatars, colour wells |
| Flat disc | `kit::flat_disc` | The corners of flat rounded shapes (paper, bars) |
| Day ribbon | fills over a faint track, a now mark | A day's appointments at a glance |
| Stat tile | a small card: label, accent bar, one `num` value | Two to four numbers side by side |
| List tile | `kit::tinted` with a progress bar | A list, a calendar, a group |
| Grouped card | one panel, rows parted by hairlines | Any list of like things |
| Foot tabs | words, the chosen underlined in the tint | An app's views |
| Palette tray | wells 40 px wide, sizes drawn at their size, tools as gel keys | Notes' pen (the owner: colours "hard to select", "no stroke size control") |

### What is built, what is next

Built and tested (2026-11-14): the backdrop's painter, the renderers'
picture floor (both renderers, the reference test), the window glass and
light gel keys, the window twins, the kit's ring and discs, the corner
fix, the concepts. **Not yet on the X51v:** the image doesn't paint the
backdrop into the renderer's picture yet (`Shell::use_backdrops` is the
model's so far), so the device still shows version 2's window. Next: the
device's side (a picture in the 2700G's pool, painted when an app opens),
then each app rebuilt on its concept with its sheet's full build list.

## Version 2 (2026-11-14): why the first version produced plain apps

The owner's test of batch 1: every app was the same grey row stacked on
the wallpaper, "as basic as can be". The cause was on this page: one accent
for everything, "no per-app box styles", "inside an app, controls are words
... never small pictures", no shadows, no depth. Those rules made every
screen a form. **Found** (how modern apps look attractive and stay usable,
measured against what the X51v's renderer draws: fills, gradients,
outlines, pre-rendered panel art, run-time pictures, text, clips):

| System | What makes it attractive | What it costs |
|---|---|---|
| iOS 17-18 (Calendar, Contacts, Notes, Reminders) | A large title that says where you are; grouped cards with clear hierarchy; colour per list and per calendar; avatars with initials; one tint per app | White space a 480 x 640 VGA panel can't always spare |
| Android Material 3 (Google Calendar, Contacts, Keep) | Tonal colour from one seed per app; coloured event chips sized to their time; a floating button for the main action; cards in a staggered grid | The floating button covers content; tonal palettes need many colours |
| Things 3, Todoist | Restraint: one strong colour per list, a progress ring, generous type; the tick animates | Few screens; depends on gestures |
| Fantastical | The parse shown as you type, highlighted words; the DayTicker strip | Busy when the day is full |
| SPB Mobile Shell (WM, 2008-10; ui.md) | Big photos and tiles, dark glass behind bright content, sheets over a dimmed screen, the WM bars kept | Heavier pictures |
| Windows Mobile 5 | The title bar, softkeys, Today's agenda, the Calendar button; everything a form | Plain, small targets |

**Ours** keeps the frame and the glass (the neo-retro look the owner chose,
ui.md "smoked glass") and takes from the modern ones exactly what a 16-bit
VGA panel and the 2700G draw cheaply: a large title with a live summary,
colour that carries identity (per app, per calendar, per list, per
person), pictures where they mean something (avatars, the weather on a
day, ink), cards and grids where a list of identical rows hid structure,
and motion that explains (a tick that fills, a sheet that rises, a month
that zooms into its day). Every item is drawn with the renderer's existing
commands: colours are panel art made once, circles are pictures made once
and kept, so nothing adds pixel work per frame.

## Colour

All colours are the dark look's, written once through `rp_draw::look::theme`;
the light look is derived from them (`light_twin`, `light_glass`,
`light_text`).

| Use | Colour (dark look) |
|---|---|
| Text | `0xFFF0F2F4` |
| Secondary text (labels, hints, times) | `0xFFCCD3DA` |
| Focus ring, the caret | `0xFF7FD0FF` (the system accent: where you are) |
| Under the pen (pressed) | `0xFF3C4656` |
| Done, held, a ticked box | `0xFF81C08B` |
| Warning, overdue, delete | the alert chip's amber and red (`weather_app`) |

**App tints** (version 2). Each app has one tint: its header's glow, its
selected chips, its main action, its empty-state mark. The focus ring
stays the system accent so "where am I" reads the same everywhere.

| App | Tint | Why |
|---|---|---|
| Calendar | coral `0xFFD9907E` | warm, the day; reads on the cool glass |
| Contacts | teal `0xFF74B1A7` | people; the icon set's teal |
| Tasks | blue `0xFF749DD3` | calm, done-ness |
| Notes | amber `0xFFD2B37B` | paper, the legal pad |
| Calculator | orange operators `0xFFD6A270` on graphite keys | the operators found by eye, the digits quiet |

**Muted, not neon** (the owner, 2026-11-14: the bright accents clashed
with the Aero / XP look; he chose 55% of four rendered strengths,
docs/reports/apps-v2-2026-11-14/accents-compare.jpg). Every accent above
and every identity colour keeps its hue with 55% of its chroma in OKLCH,
and its lightness is lowered toward the lowest that still reads at 4.5:1
on the glass cards: computed by `tools/shell-assets/accents.py`, never
picked by hand, so a new accent is muted the same way.

**An app's own window** (the owner, 2026-11-14, R201): an app runs full
screen in its own window, no wallpaper behind it, title bar and softkeys
included, the way a full-screen Windows Mobile program does. Its body is
opaque in both looks: Aero's slate gradient in the dark, frosted white
cooling to the right in the light, each tinted a little by the app's
accent (the owner's pick, C of three drawn, 2026-11-14: solid, Aero
gradient, Aero tinted by the app). The glass
cards blend over the window, not the photo (the renderers' floor,
ADR-0023). Today keeps the wallpaper; Weather's full screen has its own
window too (the owner, the same day), but not over the radar (its map is
the backdrop) or while it opens out of Today. Version 3 makes the body a
backdrop of light (Materials, above).

**Both looks, one system** (the owner, 2026-11-14: "make sure we are
designing for both light and dark mode", Weather and the apps "cohesive").
The shell's light-look rule (`look_color`) maps the hosted programs'
colours as it maps its own screens, so every app has a light look with
no palette of its own; their corner pictures are painted in the mapped
colour. Each accent's light twins (fill 3:1, text 4.5:1 on the light
glass at its darkest, measured from the rendered screens) keep its hue and
chroma in OKLCH and only darken: computed by `tools/shell-assets/accents.py`
into `rp_app::kit::LIGHT_TWINS`. Paper and ink (Notes) are marked
`kit::keep` and stay themselves in both looks. Weather's cards carry the
apps' accent bar, by what each measures (wind teal, humidity and rain sky,
pressure indigo, UV and lightning amber, sunset coral); its switches are
the Aero keys and its section labels the Weather tint (the sky). Every
mock-up is drawn in both looks (`<name>` and `<name>-dark`;
docs/reports/apps-v2-2026-11-14/both-looks.jpg).

**Identity colours** (calendars, task lists, note colours, avatars): eight,
chosen on the X51v's panel for contrast against the glass in both looks
and so no pair is told apart by red against green alone: coral, amber,
lime, teal, sky, indigo, violet, rose. A person's avatar colour is a hash
of their name, so it never changes.

- Colour never carries meaning alone: overdue is red *and* says "Late";
  a calendar's colour is also its name in the editor.

## Glass, cards and depth

- A **panel** is the glass of version 1 (`panel::draw`, 12 px corners, rim,
  sheen). Version 2 adds **tinted panels**: the same art with the body
  tinted by an identity colour at 25% (made once per colour, kept in the
  picture store), for event blocks, note cards and list headers.
- A **card** is a panel holding a small layout (an avatar, two lines and
  an action; a note's first lines; a month). Cards replace a run of
  identical rows wherever the rows had structure.
- A **colour bar** (4 px, the item's identity colour) on a row's left edge
  says which calendar or list it belongs to.
- **Depth:** a sheet (a picker, the add panel) rises over the screen with
  the screen behind dimmed to 50%; a pressed card sinks 2 px. One soft
  shadow under sheets (pre-rendered, like the panel art).
- Bars (title, header, softkeys, the keyboard) stay the left-to-right
  glass gradients with a one-pixel light edge.

## Type

Didact Gothic, four cuts (`shell/today/src/fonts.rs`), ASCII for now:

| Font | Use |
|---|---|
| `ui` (18 px, 24 px line) | Everything read as words: rows, fields, softkeys, menus |
| `label` (spaced capitals) | Section labels, screen names, weekdays, chips, key caps |
| `big` (light numerals) | One hero number a screen: the clock, a temperature |
| `num` (36 px numerals and signs) | Numbers that are typed and read at arm's length: the calculator's display and keys (digits and `. , - + x / = %` only, 5 KB) |

- Sentence case in `ui`; capitals only through `label`.
- Text is left-aligned; a row's value is right-aligned. Numbers in columns
  align right.
- Text that doesn't fit is cut at a word or with `..`, never shrunk.
- **The large title** (version 2): an app's home screen opens with its
  name in light 36 px type over a one-line live summary in secondary text
  ("3 today, next at 3:00 PM"; "4 due, 1 late"; "128 people"). It folds
  into the header as the list scrolls (iOS's large title, kept to one
  64 px band on our 640-line screen).

## Spacing and sizes

- The grid is 4 px. Side margins 16; a 6 px gap between panels; 12 px of
  padding inside a panel before text.
- **A row is 36 px** (text baseline 25 px down), a two-line row 52. A list
  is a stack of one-row panels under spaced-capital section labels (18 px).
- **The smallest stylus target is 30 x 30** (the keyboard's keys); anything
  a thumb should hit (softkeys, calculator keys, the header) is 40 or more.
- A text field is a 40 px panel with the focus ring while typing and a
  cyan caret. The keyboard (K1 compact, 208 px) docks above the softkey bar;
  the field and what it changes stay visible above it.

## The keyboard knows the field (version 2)

Every text field declares its kind; the keyboard follows it. Phones have
done this since about 2010; Windows Mobile did none of it.

| Kind | Layout | Capitals | Helpers |
|---|---|---|---|
| Name, title, place | letters | each word | the strip suggests recent and matching names |
| Address | letters, numbers row | each word | live address suggestions (below) |
| Sentence (notes, descriptions) | letters | each sentence; "i" alone | word suggestions; a double space is ". " |
| Phone | a 3 x 4 number pad with `+ * #` | n/a | formatted as typed for the region ((206) 555-0142 in the US; the region's own pattern elsewhere: the international rule) |
| E-mail | letters with `@` and `.com` keys | none | known domains suggested |
| Number | the number pad with `.` and `-` | n/a | the region's decimal mark |
| Link | letters with `/` `.` `:` keys | none | Paste offered first |

- **The strip above the keys** (where PASTE and FROM PC sat) shows three
  suggestions for the field's kind; Paste and From PC move to its ends.
- **Address suggestions** come through the network service over whatever
  connection there is (Wi-Fi when it works, the dock's PC relay now):
  Photon (OpenStreetMap data, built for search as you type) asked after a
  300 ms pause, five answers, kept by key. Nominatim's public server
  forbids as-you-type use, so it is not asked while typing. With no
  connection: the region's postal-code table in internal storage (a US ZIP
  fills city and state) and the addresses typed before. A tap fills every
  address field.
- Phone formats come from a compact per-region pattern table in internal
  storage, made offline from the public numbering plans (libphonenumber's
  metadata is the finding; ours keeps only the formatting, a few KB).


## Panel rules

- Lists scroll under the header; the header and softkeys never move.
- A choice of two to four is a segmented chip row inside the row (the
  chosen one in the app tint), not a drop-down. More than four opens a list screen.
- **Changes apply as they are made.** No Save buttons for settings-like
  rows. An editor screen (a new appointment, a contact) has Done on the
  right softkey and Cancel on the left, and nothing is kept until Done.
- Deleting asks once, in place, never in a pop-up box: the control that
  deletes changes its own words for the next tap (an editor's Delete row
  reads "Tap again to delete"; a note's Delete softkey reads "Delete?"),
  and any other tap takes the question away.
- An empty list says what it is for and how to add the first one, in
  secondary text, in a panel.
- Motion: pages slide with the pen and settle in under 200 ms. Version 2
  adds feedback motion, each 150-250 ms and started only by the user's
  own act: a tick fills and the row slides to the done group; a sheet
  rises over the dimmed screen; a month zooms into the day tapped and back
  out. Every frame meets the 33/45 ms rule.

## Icons and pictures

- The Frutiger Aero glass set of [icon-brief.md](icon-brief.md) for apps
  in the Start menu, Programs and the task switcher.
- **Inside an app, pictures are used where they mean something:** a
  person's photo or initials, the weather icon on a calendar day (the
  Weather app's own icons), a list's colour dot, handwriting. Controls are
  drawn marks (chevron, tick, plus, flag, pin, bell, repeat) made once as
  pictures at their size; a word is used where no mark is the convention.
- State is a badge over a base icon (a check, a warning).

## What an app must not do

- Draw its own title bar, softkey bar or keyboard.
- Add a colour, a font or a size that isn't on this page (add it here
  first, with its reason).
- Ship with less than its plan: the rules gate's rule 6 holds each app's
  build to its sheet's build list ([app-scope.md](app-scope.md)).
- Put work in the touch path (principles.md P1): lists are sorted and
  searched when the data changes, not while the pen moves.
