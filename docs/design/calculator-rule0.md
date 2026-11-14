# Calculator, Rule 0 (version 2, 2026-11-14)

Version 1 (2026-11-13) was a four-function keypad, less than Windows
Mobile 5's own calculator (no memory keys), with every advanced feature
under "Later"; the owner rejected it on 2026-11-14 ("the calculator app
has no advanced features"). This version builds what the survey found,
under the rules gate's rule 6 ([app-scope.md](app-scope.md)). Look:
[design-language.md](design-language.md) version 2. Plans it answers to:
[apps-and-packs.md](apps-and-packs.md) (the Calculator row: basic,
scientific, unit conversion, history). The arithmetic stays version 1's
exact decimal engine (`rp-pim::calc`), which already beats binary floats
(0.1 + 0.2 is 0.3).

## The core job

Get a right answer fast, see how you got it, and do the everyday sums
(tips, units, money, dates) without thinking about the maths.

## What others do (findings, not the answer)

| System | What it does well | What it costs |
|---|---|---|
| Windows Mobile 5 | MC, MR, M+; big keys | Immediate execution (2 + 3 x 4 = 20); no history; floats |
| iPhone Calculator (iOS 18) | Scientific when turned; a history list; unit and currency conversion inside the calculator; the expression shown while typing | Conversion hidden behind a button |
| Google Calculator | History by pulling down; brackets; the result previewed live | Plain |
| PCalc | RPN option, constants, conversions, a paper tape | Dense |
| Soulver / Numi | Sums in words ("$40 + 15% tip split 3 ways"; "days until 25 Dec") with live answers | A text editor, not keys |
| Lean Linux (bc, qalc) | qalc understands units and currencies typed in a line | Typing only |

## Borrow and beat

| Problem | Borrowed | How ours goes further on the X51v |
|---|---|---|
| Seeing the sum | Google's live preview | The display shows the **whole expression** (wrapping to two lines) and the **answer live** in secondary text while typing; = moves it up |
| Order | school maths (version 1) | Precedence with **brackets**; exact decimals to 12 digits |
| What happened before | iOS's history; PCalc's tape | A **history tape** above the display, scrolled by the pen; tap a line to reuse its answer, hold to copy |
| Memory | WM5's MC, MR, M+ | **MC, MR, M+, M-** with an "M" mark when memory holds a value |
| Scientific | iOS's rotated keypad | A **second keypad page** slid sideways in portrait (sin, cos, tan, their inverses, ln, log, x², √, xʸ, π, e, !, deg/rad), exact where decimal allows |
| Units | iOS 18's conversion | **Unit conversion** mode: length, weight, volume, temperature, area, speed, time, data; US units first (the owner's rule), metric one tap |
| Money | qalc's currencies | **Currency conversion** with rates from the European Central Bank's daily reference file over any connection, kept with their date, shown as "rates of 14 Nov" offline |
| Everyday sums | Soulver's tip and split | **Tip and split:** bill, tip percent chips (15, 18, 20, custom), people; **days between dates** and "date plus N days" with the calendar's date picker |
| Moving numbers | phones' copy and paste | Copy the answer, paste a number into the display (the shell's clipboard) |
| Looks | modern calculators' coloured operators | **Graphite digit keys, orange operator keys, a tinted = key**, 106 x 70 targets; the mode chips (Basic, Scientific, Units, Money, Tip, Dates) across the top |

## The design

- **Basic:** history tape (up to four lines visible), the display (the
  expression in `ui`, the answer in `num`), MC MR M+ M- row, then the
  keypad: C, brackets, %, ÷ / 7 8 9 × / 4 5 6 − / 1 2 3 + / ± 0 . =.
- **Scientific:** the same display; the keypad page slides to the
  scientific keys.
- **Units, Money:** two value rows (from and to) with their unit chips,
  the number pad under them; swap button between.
- **Tip:** bill amount, tip chips, people stepper, then the results
  (tip, total, each) in large numerals.
- **Dates:** two date rows (or a date and a number of days) and the result
  ("41 days", "Thu 25 Dec").

## Build list

| Item | From | Status | Code |
|---|---|---|---|
| The expression shown whole with the answer previewed live | sheet | built | shell/pim/src/expr.rs, apps/calculator/src/lib.rs |
| Brackets with precedence | apps-and-packs: brackets | built | shell/pim/src/expr.rs |
| History tape: scroll, tap to reuse, hold to copy | apps-and-packs: history | built | apps/calculator/src/lib.rs, apps/calculator/src/tests.rs |
| Memory keys MC, MR, M+, M- with the M mark | apps-and-packs: memory keys | built | apps/calculator/src/lib.rs |
| Scientific keypad page and unit conversion (length, weight, volume, temperature, area, speed, time, data) | apps-and-packs: Basic + scientific + unit conversion | built | shell/pim/src/expr.rs, apps/calculator/src/lib.rs |
| Currency conversion with the ECB's daily rates over any connection, kept with their date | apps-and-packs: currency conversion | built | shell/pim/src/net.rs, apps/calculator/src/lib.rs, xtask/src/usb.rs |
| Tip and split | apps-and-packs: tip and split | built | apps/calculator/src/lib.rs |
| Days between dates and a date plus N days | apps-and-packs: days between dates | built | apps/calculator/src/lib.rs |
| Copy the answer and paste a number | sheet | built | shell/core/src/lib.rs, apps/calculator/src/lib.rs |
| Aero glass keys (the owner, 2026-11-14: not Metro): operators' signs in the tint, = the blue glow, the mode chips | sheet | built | apps/calculator/src/lib.rs, shell/core/src/panel_art.rs |
| The region's digit grouping and decimal mark | sheet | built | apps/calculator/src/lib.rs |
| The light look: the app's own light palette (its content is drawn after the shell's look pass, so rounded shapes keep one colour edge to edge) | sheet | pending | |
| Chips and cards as rounded shapes of one colour, no glass art inside (the owner, 2026-11-14: "a box on top of the button") | owner 2026-11-14 | built | apps/api/src/kit.rs, shell/core/src/panel_art.rs |
| Its own program, loaded when opened and released when left | owner 2026-11-14 | built | apps/calculator/src/main.rs, shell/today/src/program.rs |

## What would overturn it

- The owner preferring immediate execution as on WM: a setting, with
  precedence the default.
- The ECB's file not covering a currency he needs: a second free source
  for the rest, named then.
