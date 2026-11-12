# Decisions for the owner, 2026-11-12

From the owner's list of 2026-11-12 (branch `fix/deep-dive-2026-11-12`):
his rulings on the three open boxes of
[decisions-2026-11-11.md](decisions-2026-11-11.md), applied one commit
each, each box ticked there as applied. Any box still open below is his to
rule on; nothing in it is decided here.

## Item 1: R191, Today's last picture kept (ruled: (a), applied)

- [x] **Ruled by the owner, 2026-11-12: (a), keep Today's last picture so
  the return is a copy.** Applied and proven on the X51v (runs today-0138
  to -0140, [the run report](runs/2026-11-12-today-0140.md)): the picture
  kept in the 2700G's spare 600 KiB at `0x200000`, composed ahead while
  another screen is up (the Reveal pattern); the return to Today
  **1,172 us** of CPU (was 55.5 ms), under the 45 ms allowance; look
  check 0 wrong, `cargo xtask look-judge` verdict OK. ADR-0023 rules 2
  and 4 amended. The first Light look switch on Settings, 45,068 us
  (68 over), is a one-time first-frame atlas build per look (the switch
  back took 41,756 us with no atlas built): recorded, not redesigned.

- [ ] **The Start menu over Today is still over its 33 ms.** **Why:**
  re-measured in today-0140, frame 191: **38,552 us** (610 blits, 278,516
  blended px, 154,944 px: half the screen; today-0137 38,452). The menu's
  glass is drawn over Today's commands under it, each blended again in
  the menu's area. Not redesigned (the ruling: no redesign of Today's list
  without your word). **Options:** (a) leave it over the rule, logged;
  (b) cut the blits under the menu: draw the menu over the screen as
  shown (Today's pixels copied, not its commands re-run under the glass);
  (c) prebuild the menu's picture while Today is still, as the app is
  composed ahead, so opening it is a copy. Each changes ADR-0023's
  renderer or the menu's drawing; nothing is built until you pick.
