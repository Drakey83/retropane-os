# Calculator, Rule 0

Batch 1 of [core-apps-list.md](core-apps-list.md). Look:
[design-language.md](design-language.md). Method:
[principles.md](principles.md). No store: a calculator keeps nothing.
Built without a stop for the owner's go; the notes for his morning test
are at the end.

## The core job

Add up a bill, split it, work out a percentage: right, at a glance. v1: a
basic calculator with correct arithmetic and behaviour that is clear at
every key. Scientific mode, unit conversion and a history are on the later
list.

## What others do (findings, not the answer)

| System | Arithmetic | The order of a sum | The display | After equals |
|---|---|---|---|---|
| Windows Mobile 5 Calculator | Binary floating point (doubles), rounded for show | **Immediate:** each key works on the running total, so `2 + 3 x 4` is 20 | One line: the number only, so the sum typed can't be seen | A digit starts a new number; what the pending operation was isn't shown |
| A pocket four-function calculator | Decimal (BCD), 8-12 digits | Immediate | One line | `=` again repeats the last operation; `%` does different things by maker |
| iOS Calculator (to 2023) | Decimal | Immediate in portrait, precedence in the scientific layout: two answers for one sum | One line | As a pocket one |
| Google Calculator (Android) | Decimal, arbitrary precision | **Precedence** (`2 + 3 x 4` is 14) | The whole expression, with the answer previewed as you type | The answer becomes the start of the next sum |
| Windows 10 Calculator | Decimal-safe rational arithmetic | Standard mode immediate, scientific precedence | The pending sum on a line above | - |
| `bc`, `dc` | Arbitrary-precision decimal | Precedence | Text | - |

## Borrow and beat

| Problem | Borrowed | How ours goes further on the X51v |
|---|---|---|
| Wrong pennies | The pocket calculator's decimal arithmetic | **Numbers are decimal, not binary:** a whole-number mantissa and a count of places, so 0.1 + 0.2 is 0.3, a hundred dimes are ten dollars, and 19.99 x 3 is 59.97 exactly. No FPU is needed or used (the PXA270 has none): it is integer arithmetic, 12 significant digits, rounded half away from zero only where a result can't be exact (division, very long products). WM5's doubles on an FPU-less ARM were both slower and inexact |
| Which answer is "right" | Google's: school precedence | `2 + 3 x 4` is 14, in every layout: one calculator, one answer |
| Not seeing the sum | Google's and Windows 10's expression line | **Two lines:** the sum so far above ("12 + 3 x"), the number being typed or the answer below in large numerals. After `=` the line above reads "12 + 3 x 4 =", so the answer is never without its question |
| After equals | The pocket calculator, made visible | A digit starts a new sum (the old line clears); an operation goes on from the answer; `=` again does nothing (no hidden repeat); Delete after an answer clears. Each is stated here and tested |
| Percent | The phone calculators' rule | After plus or minus it is a share of what came before (`200 + 10 %` is 220); otherwise a hundredth (`50 %` is 0.5). The number it becomes is shown before `=` |
| Errors | "E" or "Error" | Words: "Cannot divide by zero", "Too large to show". Clear or a new number starts again; no other key does anything, so an error can't be built on |
| Keys for a stylus *and* a thumb | The four-column pad | 20 keys in 4 x 5, each 106 x 62 px: the largest targets in the shell, since a calculator is used in a hurry and often with a thumb. Digits in the reading font, operations in the accent's panel, `=` filled |

## The design (v1 as built)

- **The display:** a glass panel: the expression line right-aligned in
  secondary text, and under it the number right-aligned in the numeral
  font (36 px), grouped in threes ("1,234,567.5").
- **The keys:**

  | | | | |
  |---|---|---|---|
  | C | DEL | % | / |
  | 7 | 8 | 9 | x |
  | 4 | 5 | 6 | - |
  | 1 | 2 | 3 | + |
  | +/- | 0 | . | = |

- **C** clears everything; **DEL** takes off the last digit typed (after
  an answer it clears); **+/-** changes the sign of the number being
  typed, or takes an answer negated as the new number.
- Twelve digits may be typed; a second point is ignored; `007` is 7.
- **The D-pad** moves a focus ring over the keys and Enter presses.
- Softkeys: **Back** (left); the right one is empty in v1.
- **Nothing is kept:** leaving and coming back finds the sum as it was
  (the shell holds it while it runs); a restart clears it.

## How the design was iterated

1. *Draft:* WM's immediate execution (it is what "basic" calculators
   do). **Lost:** it gives 20 for `2 + 3 x 4`, and with no line above the
   user can't even see why. Precedence plus the visible sum is the design
   that can be defended to anyone who checks it by hand.
2. *Draft:* binary floating point through the soft-float library (the
   textbook default). **Lost on its own results:** 0.1 + 0.2 shows
   0.30000000000000004 or needs display rounding that hides other errors;
   and soft-float is slow here. Decimal integers are exact and faster.
3. *Draft:* a live preview of the answer under the number (Google's).
   **Lost on space and clarity:** three lines of changing numbers on a
   3.7-inch screen; the two lines say everything, and `=` is one key.
4. *Draft:* `=` repeating the last operation (pocket calculators).
   **Lost:** a hidden state the display doesn't show; a doubled tap on a
   resistive screen would silently change the answer. `=` on an answer
   does nothing.
5. *Draft:* the 76 px numerals of the clock. **Lost:** nine digits fill
   the width and the font has no point or minus. A 36 px numeral cut
   (digits, point, comma, minus) was added to the design language for
   this: twelve digits with their commas fit.

## The practices (principles.md)

- **P1:** nothing is computed while the pen moves; a key press is a few
  integer operations.
- **P6:** the calculator's whole state is about 400 bytes in the shell's
  own memory; no frame over 4 KB.
- **Frame rule:** the screen is about 60 draw commands; a key press
  redraws the display and that key only.

## Later (not in v1)

Scientific mode (landscape); unit and currency conversion; a history
tape; memory keys (M+, MR, MC); parentheses; copying the answer to the
clipboard and pasting a number; the region's digit grouping and decimal
mark (the international rule: today the comma groups and the point is the
decimal mark, the US default); a tip and split helper.

## Notes for the owner's morning test

- **`2 + 3 x 4` is 14** (precedence), not WM's 20.
- **`=` twice does nothing.** Alternative: repeat the last operation.
- **No memory keys in v1** (WM5 had MC, MR, M+); they are on the later
  list.
- **Percent:** `200 + 10 %` shows 20, then `=` gives 220.

## What would overturn it

- The owner preferring immediate execution because WM did it: a setting,
  with precedence the default.
