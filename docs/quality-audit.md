# Quality audit: every known shortcut, and its proper fix

The owner's rule (2026-10-18): **no corners cut; fix everything properly.**
This page lists every shortcut we know of in the project, as of
2026-10-18, so none stays hidden. Each item says what the shortcut is, why
it matters, what the proper fix is, and its state. New shortcuts found
later are added here the day they are found, never left in a commit
message or a chat.

States: **open**, **in progress**, **fixed** (with the commit or date).

## A. Conclusions that were wrong, or never checked

| # | Shortcut | Why it matters | Proper fix | State |
|---|---|---|---|---|
| A1 | Depth "less" declared broken on the 3D core (2026-10-15) from a test that changed two things at once (the depth mode was also written into the alpha-test bits); the renderer and both models then refused it | A whole class of games (most use `GL_LESS`) would have needed a workaround that wasn't needed | Measured both ways (`rp-glesfeat` D), renderer and models corrected, mutant for the mistake, device-proven | **fixed** 2026-10-18 (ADR-0026 option a, `rp-mbx3d-api` 104/0) |
| A2 | The object's second word named "ISP" by guess and treated as a depth word for four days | Wrong names make wrong code; this one hid A1 | Read from the GLES driver, confirmed on the device; comments corrected | **fixed** 2026-10-18 |
| A3 | rp-mbx3d-api "less in a partial render hid an 'always' quad" (runs 1-3) | Likely the same alpha-bits mistake, never rechecked | Re-run the partial-render case with the fixed renderer; record either way | open |
| A4 | Audio: the codec probed before Windows Mobile's power-up order had been read (run 1, 2026-10-18) | A device run spent on a test that couldn't answer | `wavedev.dll`'s order read; `rp-tone` follows it (`0ea7387`); owner-watched run next | in progress |
| A5 | Any other "X doesn't work on this chip" verdict reached from one run | Same class as A1 | Sweep ADR-0024, ADR-0026, `2700g.md`, `cpld.md`, `udc.md` for negative verdicts; each must name the single variable it changed, or be re-tested | open |

## B. The emulator doesn't model what the device showed

The rule (emulator tracks device) says every device fact gets modelled,
with a check that fails without it, or is listed as not modellable. These
rows of `docs/hw/emulator.md` are still "not modelled yet" (a check that
depends on them proves less than it looks):

| # | Not modelled | Proper fix | State |
|---|---|---|---|
| B1 | A PC that goes silent after an unplug (the silent-PC retry has host tests only) | A host option in the fork that ignores the first N plug-ins; an `emu-usb` variant must get configured by the retry; a mutant without the retry must fail | open |
| B2 | Alpha test, all depth modes, code 10 grey | Fork rp-v0.22.0 (`mbx-3d-alpha`), `emu-glesfeat` checks the device's pixels | **fixed** 2026-10-18 |
| B3 | Fog (TSP bit 24, colour `0x63C`) and the second texture unit | Device test first (needs the TA's vertex and state formats read statically), then model | open |
| B4 | Codec power (CPLD `0x1C` bit 1) and speaker amp (`0x14` bit 1): the model's codec answers with power off | Model after the owner-watched run decides what the codec needs | open |
| B5 | The 60 Hz display PLL (`DISPPLL`) | Model the PLL so `emu-refresh` checks the real switch | open |
| B6 | GPIO pin functions (touch CS on GPIO24) | Model `GAFR`, start from WM's values | open |
| B7 | The CF socket in the X51v machine; MVED decoder registers; clocks' relock time and `MDCNFG`; timer channels' odd modes; 2D engine format code 4 | One fork change each, listed in `emulator.md` | open |

## C. Checks that prove less than they claim

| # | Shortcut | Proper fix | State |
|---|---|---|---|
| C1 | emu-today waited on the wall clock and masked the clock areas everywhere | Presses answered near the pen, Home held in guest time, screens awaited | **fixed** 2026-10-18 (`43e7e22`) |
| C2 | 27 fixed sleeps of 500 ms or more remain in `xtask/src/emu.rs` (e.g. "press a key, sleep 4 s" in emu-mbx3d, emu-mbx3d-api, emu-glesfeat) | Each sleep replaced by a wait on the image (a serial marker, the screen, a register) or a guest-time hold; a test that forbids new ones | open |
| C3 | emu-glesfeat checked no pixels | Checks the device's pixels with rp-v0.22.0 | **fixed** 2026-10-18 |
| C4 | emu-noise could only check plumbing; with the R6 health rule the emulator can't make a healthy run | The append is host-tested (`append_row`); the emulator proves the refusal and that the file stays untouched | **fixed** 2026-10-18 |
| C5 | Mutants exist for some guards, not all | Every guard added since 2026-10-10 gets a mutant; list the ones missing | open |

## D. Copied values and housekeeping

| # | Shortcut | Proper fix | State |
|---|---|---|---|
| D1 | Values copied from Dell/WM carry no `inherited:` tag (0 tags in the code), so the open-copied-values count the owner asked for can't be given | Tag every copied value (`// inherited: <audit item>, <source>`), an `xtask` lint counts them, every summary reports the count; work them down (imitation audit) | open |
| D2 | The card's loader is one change behind main (the drive's naps counter) | Install with the next loader change, backup first | open |
| D3 | 26 old branches and their worktrees, all merged | Deleted (29 local branches, 3 on GitHub; only `main` left) after a full bundle (`retropane-backups/2026-10-18/retropane-os-all-before-branch-cleanup.bundle`) and the worktrees' `local/` files saved and copied into `local/`; 15 empty worktree folders stay until the app releases them | **fixed** 2026-10-18 |
| D4 | The RNG restart-test health rule sat unfinished on a branch | Finished into main with tests and a mutant | **fixed** 2026-10-18 |

## Order

1. Finish what's in progress: A4 (owner-watched audio run).
2. The ones that can hide wrong conclusions: A3, A5, C2, C5.
3. Emulator debt by what the next work needs: B1, B3, B4, then B5-B7.
4. D1 (the copied-values count), D2 with the next loader change.
