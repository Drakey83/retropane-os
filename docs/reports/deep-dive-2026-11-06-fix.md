# Deep dive fix, 2026-11-06/07: what changed, what proves it, what's owed

Branch `fix/deep-dive-2026-11-06` from 4f60eebe; **not pushed**. Every item's
commit passed `cargo xtask ci` in a separate checkout of exactly that
commit. Decisions that are the owner's are tick-boxes in
[decisions-2026-11-06.md](decisions-2026-11-06.md). **No device run was made
while the owner was away**; every device run owed is named below.

| # | What changed | Commits | Proved by | Device evidence owed |
|---|---|---|---|---|
| 1 | Divide-1 cures (R119, R121) follow the clock: every render into screen 1 takes the underlay and alpha-0 overlay when `Rung::now` reads divide 1 above 74.75 MHz; never at divide 2, at 74.75 / 1, or into screen 0; R120's switch stays dropped. ADR-0035 amendment E drafted, **PROPOSED**. `emu-scene-d1` repaired (it was red before: a test budget, and the fork's known row-544 write) | cf5fbdb9, acad827e | host tests `the_cures_are_wanted_only_at_divide_1_above_74_75`, `the_divide_1_cures_follow_the_clock_for_screen_1_renders`; 2 mutants; `emu-scene-d1` (new P8: every screen-1 render cured by the clock) | rp-scene divide-1 run on the X51v, P8 judged by DSIG ([run sheet](../research/sheets/divide1-cures-by-clock.md)) |
| 2 | Device-run evidence committed: 16 reports in [runs/](runs/README.md) (gpubench-0237 to -0241, scene-0008/0009, today-0054/0055, r184s3a-d, today-0062, r185a/b), redacted and read; `cargo xtask lint` refuses a new sheet citing a `local/` log without a report | 1ebfc55b, d16701f1 | `policy::check_run_reports` + its host test; lint clean | none |
| 3 | C97: a rain frame not kept or not readable asked again once, then lost, with RAIN UNAVAILABLE, never dry. C98: every card read and write through `AnyVolume`; exFAT gains the staged write. C99: `shown_at` gives the whole screen only to a client the owner grants; others see only their own layer's pixels | 13613f4c, 2f921f1f | host tests (rp-shell, rp-fat, rp-today, rp-graphics); 3 mutants; `emu-today-radar-card` on FAT and exFAT card images; `emu-today-look` 0 wrong | C97: a run where the rain store fills or a frame is damaged; C98: an exFAT card in the X51v |
| 4 | Frame budget (R171) also holds the look check's replay: a still frame over 33 ms fails (worst 16.3 ms), the worst moving frame reported (42.7 ms, the Radar slide-in). Leaving Radar no longer rebuilds lent pages in one pass (465 ms held on the X51v, 212 ms of it Hourly's picture): only the page shown, the rest ahead while still. **Open:** the Radar slide-in over budget; a look switch re-uploading every atlas (per-colour atlases, the glyph memory flushed whole) | 7f0ab76a | `emu-today-look`'s budget line; `lookcheck::frame_budget` test | the lent-page rebuild (the emulator lends none without the dock) |
| 5 | A guard at every image's stack bottom (8 painted words) checked at every check-in, return and crash report, in `rp-usb` and the loader's report; the emulator fails on OVERFLOWED. The loader's deepest path 87% -> 67% (its LUNs in static slots, the guard built in place). `RP_STACK_PATHS=<image> cargo xtask stacks` lists the deepest chain through each of `main`'s callees | bc4a1b5a | `cargo xtask stacks`; host tests; `emu-cf`, `emu-mtp`, `emu-usb-suite` | none (an overflow isn't wanted); the loader's zeroed-RAM budget 36 -> 42 KiB is a tick-box |
| 6 | C18 fixed with a test that bites (the low-water mark feeds a 90 ms decoder; the mutant back). C12/C26/C63: `emu-drive` passes three times on rp-v0.31.0. C21, C25, C11, C49 written down as not fixable here (a device run or the fork) with their costs; C60, C67 triaged | 0ceab312 | `the_low_water_mark_feeds_a_decoder_slow_to_refill`; its mutant killed; three `emu-drive` passes | C21: `rp-mbx-blend` on the X51v at 60 Hz |
| 7 | R182: the clock from the PC's time at every load (`RPTIM` on the card), not a stale build time. R183: the hero's words name the sky drawn (the Sky test overrode only the sky), and a weather change redraws the whole card once. R181: no cause provable; its logging stays | 8f3c9a28 | `the_load_time_reads_back_and_bad_files_are_refused`; `the_heros_words_name_the_sky_drawn`; `a_change_of_the_skys_weather_redraws_the_whole_card_once`; `emu-today-radar-card` requires the card's clock | R182: one load, the date checked; R183: the sky under a real forecast; R181: a run with a blank card |
| 8 | `cargo xtask perf sessions`: the spread between sessions for every floor; run files stamped with their build; 14 drift-type tolerances widened to their measured spread (no floor value changed). Four slower floors and the re-measure are tick-boxes | 279bd611 | `sessions_group_by_time_and_their_spread_sets_the_tolerance` | the re-measure: `perf noise 5` in three sessions on the X51v |
| 9 | Game Mode sheet A, **DRAFT** for the vote. R90: no new draft, ADR-0038 already holds it (accepted 2026-10-29); a tick-box asks whether a fresh one is wanted | 50bb9b83 | docs only | none |
| 10 | `xtask/src/emu.rs` (20,178 lines) split into `emu/{mod,today,radar,look,stacks,perf}.rs`, no behaviour change | 225ffc6b, a6d8f294, c5dc9c1c | `cargo test -p xtask` (167), CI; the full emulator suite re-run (see below) | none |

## The full emulator suite after the split (2026-11-07)

`cargo xtask emu` ran to `emu-mbx3d-api` (97 checks OK), which is red on
main before this branch (R186); the checks after it were then run one by
one: all pass but two, after fixes:

- **`emu-today` failed, then fixed (c5dc9c1c):** the emulator's card
  carried a fixed time file (item 7's R182 check) while the reference is
  drawn at the build's time, so the date differed in 243 pixels; the card
  now carries the build's time. Passes: Today pixel-exact, the replay, the
  radar card on FAT and exFAT, the look check.
- **`emu-cf`:** one Windows serial error ("Overlapped I/O operation is in
  progress"), passed on its re-run.
- **Red on main before the branch, parked as R186:** `emu-mbx3d-api`
  (run 4 draws the background where its shapes go) and `emu-gpubench`
  (over its size budget on 4f60eebe; with the budget raised, run A's G4a
  draws rectangular textures from one texel row). Both fail the same way on
  4f60eebe, so no item caused them; their causes aren't proven yet.

## Notes

- **Stopped there.** No new feature started; no settled ruling reopened.
- **Pushing:** not done (the list said not to).
