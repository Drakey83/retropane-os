# RetroPane UI: direction (in progress)

Nothing here is approved for code yet (ROADMAP Phase 5: spec and mock-ups
approved before any UI code). This page records the owner's choices as they
are made, and links the concept mock-ups.

## Decided by the owner (2026-10-12)

- **Overall feel: hybrid.** Windows Mobile's proven structure (Today screen,
  Start menu, two softkeys at the bottom, stylus-precise) with modern
  touches: a notification shade, a smooth task switcher, live widgets on the
  Today screen, a first-class dark theme. Old WM programs keep their own
  title bars and softkeys and fit in naturally.
- **Input: stylus and D-pad equally.** Everything works with the stylus,
  and everything also works one-handed with the D-pad and buttons (a
  visible focus ring, keyboard-style navigation). Buttons keep exactly the
  jobs Windows Mobile gave them (owner's rule).
- **Always visible, like a phone:** time, battery and Wi-Fi, at least in a
  status bar on every screen, apps included.
- **Today screen:** a good weather display with the clock and calendar by
  default, fully customisable (add, remove, reorder widgets) if the owner
  doesn't like the defaults.
- **Visual style: not chosen yet.** Three looks of the same Today screen
  were mocked for comparison: A clean flat, dark-first; B glassy accents;
  C retro-modern (concept canvas, private to the owner).

## Constraints every concept respects

480 x 640 at 3.7 inches; resistive single-touch stylus plus D-pad and the WM
buttons; software rendering until the 2700G 2D blitter is supported, so
the framebuffer is write-only, translucency is used sparingly and chrome is
pre-rendered (docs/design-brief.md); 64 MB of RAM (docs/memory-budget.md).
