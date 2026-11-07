# Run sheet: re-measure the performance floors across sessions (R109)

**Status: READY, NOT RUN.** The owner approved the re-measure on
2026-11-07 ("yes, but not now"): it needs him and the X51v, and he is
recovering. Nothing here has been run. Run it only on his go.

## What it proves

The ratchet's tolerances so far come from runs inside one boot (five back
to back), so they can't see drift between boots and builds (R109: 48 of
115 floors spread past their tolerance over 11 sessions in `local/perf`,
`cargo xtask perf sessions`). Three sessions of the same build, a soft
reset between, give each floor its spread between boots; every run file now
records its build (`build <hash>`, item 8), so no session mixes builds.

## Before

- The X51v docked, Windows Mobile up, nothing else using it
  (`cargo xtask axim` names no holder; `local/DEVICE-IN-USE.txt` free).
- One commit for all three sessions: note `git rev-parse --short=12 HEAD`
  and don't commit, pull or build anything else between sessions.
- The loader started by the owner (HaRET), the screen saying the loader
  is up.

## The run (about 35 minutes a session, about 2 hours with the resets)

Session 1:

```bash
RETROPANE_LOG_WAIT_SECS=600 cargo xtask perf noise 5
```

Then a soft reset into Windows Mobile (Claude does it):

```bash
cargo xtask reboot-wm
```

The owner starts the loader by hand again. Sessions 2 and 3: the same two
commands (session 3 needs no reset after it).

Each `perf noise 5` rewrites `docs/perf-floors.md` from its own five runs;
that's expected and is replaced in the last step. All 15 run files stay in
`local/perf` (`run-<secs>-noise<k>.txt`, each starting `build <hash>`).

## After (no device)

```bash
cargo xtask perf refloor 15
```

```bash
cargo xtask perf sessions
```

`refloor 15` sets every floor from the 15 runs (keeping each
between-sessions tolerance as its least); `perf sessions` then lists any
floor whose three session medians still spread past its tolerance.

## Pass

- All three sessions' run files name one build.
- `git diff docs/perf-floors.md`: every floor that got faster is kept;
  every floor that got slower is listed for the owner (a slower floor is
  his, as on 2026-11-07); no row loses its approval note.
- `perf sessions` lists no drift-type floor (R109's list) past its
  tolerance; any other one listed is named with its sessions' medians.

## Fail, and what it means

- A session stops with "no numbers in its run log": that image's run went
  wrong; keep its card log (`local/`), report it, rerun only that session.
- The three sessions' medians differ by more than the tolerance of a floor
  set this way: that floor's tolerance is the between-sessions one (4 sd of
  its session medians), recorded as `BETWEEN_SESSIONS` in its row.
