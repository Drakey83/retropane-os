# Deep dive fix, 2026-11-09: the owner's eight items

Branch `fix/deep-dive-2026-11-09`, from main at 070aa9bf. **Pushed**
(corrected 2026-11-10): GitHub `main` was fast-forwarded to 04d3d142, the
branch's last commit, on the owner's "push to live" of 2026-11-09, after
`cargo xtask ci` passed on that commit; the branch itself is on GitHub too.
The list had authorized only the fork's release, rp-v0.31.1, also pushed. One or
more commits per item, each item's last commit through `cargo xtask ci`
(in a clean second checkout, `../rp-ci`). Device runs only on the X51v
already on the loader, unattended. Gate: 30 of 30 architectural lines for
phases 2, 4 and 5 (`cargo xtask gate`). Decisions left to the owner:
[decisions-2026-11-09.md](decisions-2026-11-09.md).

| # | Item | Outcome | Commits (CI) | Proof |
|---|---|---|---|---|
| 1 | Rule the game step | **Done (docs).** 136.50 / 1, 133.25 / 1 at peak heat, 146.25 struck, the provisional 139.75 closed: R76, ADR-0035 amendment D "The game step", ADR-0038, divide1-149.md, the 2700G page and the datasheet | 72ff5443 (CI passed) | GC2DT, GC2DV, GC2DR, GC2DK, GC2DG (the 2026-11-08 runs) |
| 2 | Fork release rp-v0.31.1, QEMU_RELEASE | **Done.** rp-v0.31.1 tagged on the four-corner fix (cbba0bae4f, the commit your list called 7fad420638 put alone on rp-v0.31.0) and published with both assets; QEMU_RELEASE pinned; glesfeat_no_quad counts 0x60A00000 and 0x61200000 as modelled. R186 closed | bf6808aa (CI passed) | the full emulator suite on the release build, exit 0 (boot_quantum missed its deadline once on a loaded PC, passed on the resume); the no-quad host test |
| 3 | The device frame budget | **Fixed, proven on the X51v; one fix waits for you.** Causes found on the engine model replaying the route and fixed: glyph atlases per colour overflowed the glyph memory (now glyphs uploaded as first drawn, 279 KiB for every pair), and the look switch drew 1.84 screens (damage under a later rectangle kept). Look switch 88-95 -> 39.5-42.6 ms; no glyph uploaded again; return 60.9 + 36.2 -> 53.5 ms. Left: Today's 315 KB of pictures through a 256 KiB cache (tick-box), and whole-screen frames' ~40 ms against the 33 ms budget (yours, unchanged) | 03150420, 78ec0b4a, 897e1e53 (CI passed) | the route test (fails without: 564,288 px dithered, 11 atlases on the return); run today-0082 |
| 4 | Bisect the touch replay (R188) | **Cause proven for half, fix prepared as a tick-box.** The floors came from Oct 30, not Nov 7; bisected on the X51v to 4fa4dbad (the app composed ahead for Reveal: 16.5 -> 40.8 ms there, ~19-20 ms now). Compose off: 18.1-18.4 ms against 19.2-20.3. The other ~2 ms not attributed (a second bisect, prepared) | 525b3342 (CI passed) | runs today-0083, -0094 to -0109 |
| 5 | Guard the renderer's own memory (R189) | **Closed by its stop rule; two runs prepared for you.** GC2DX: in 2,000 renders at 149.50 / 1 no change in the renderer's region array, lists or memory manager's block, nor free memory nor textures (against the 149.50 / 2 control). The first build's baseline was confounded, recorded. Cold chip and the X50v: your steps written | 05ab6238, a42b1005 (CI passed) | runs gpubench-0126, -0127 |
| 6 | Two stale report lines | **Fixed.** deep-dive-2026-11-07-fix.md now says pushed (main holds ad4c7e67); decisions-2026-11-06.md says P8 passed (scene-0001) | c647ce8d (in bf6808aa's CI) | the repo |
| 7 | Look check coverage (R187) | **Fixed, proven on the X51v at 4 PM.** The check now logs why a pass is unknown; at 4 PM that showed Hourly's points all sat inside its graph, a picture the device paints from the live forecast (the host never draws it), and the afternoon curve passed under five of nine. Refitted off the graph over twelve forecasts in both looks. Also fixed on the way: my item 3 log lines overflowed the run log and dropped the verdict | dce4c246, 8d264462 and the fix-up below (CI passed) | `hourly_look_points_hold_off_the_graph_under_every_forecast` (fails on the old points); emu-today-look; runs today-0113 (4 PM, old points: 48 unknown, dark Hourly 0) and today-0114 (4:01 PM, new: 0 unknown, dark Hourly 51); daylight runs today-0082, -0111, -0112 |
| 8 | C71 and C53 | **Both closed.** C53: a reset doesn't clear a TA hang, cycling the 3D clock does (M2: 0 of 3 -> 3 of 3, twice); the drivers now cycle it, and the shipped recovery brought the core back 3 of 3. C71: closed on its R-B evidence (0 of 120 wrong), guard and mutant; R190 opened for a test gap found on the way | 7fdc858c (CI passed) | runs glesfeat-0028, -0029; CH C71 R-B (R99); the tests and mutants named in the audit |

Reminders that came due and what happened: R76, R171, R172, R186, R187,
R188, R189 (worked, above); device-run reminders R3, R4, R12, R22 and
others need other images or the owner watching, so none were folded into
these unattended runs. Opened: R190. Extra runs on your "do any runs now": GC2DR after ~1.25 h idle (gpubench-0128: the same line, 137.80 / 1 right, 138.67 first wrong) and the look route at 3:18 PM (today-0112: 0 unknown). All eight items: done or prepared with your steps; pushed to GitHub `main` (04d3d142) on your later "push to live".
