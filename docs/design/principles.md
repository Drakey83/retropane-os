# How every screen is built: borrow the best, then beat it

The owner, 2026-10-31: "we don't have to reinvent the wheel, we have all of
the OS in the world to borrow from: how does Windows do these things, how
does Android, how does Apple, how do super efficient Linux distros made to
run on older hardware pull off their OS, and how can we borrow from those
ideas and make them even better?" and "This shouldn't be just something we
do for the Weather app design, it should be our design philosophy for the
entire UI/UX."

And: "rule 0 should also be a part of that design philosophy."

This page is that philosophy. It holds for every screen, app, widget and
effect in the shell, both looks (Modern and Classic). Weather is the first
app built this way ([weather-rule0.md](weather-rule0.md)).

## Rule 0, the foundation

The owner's number one rule (CLAUDE.md rule 0) is the base of everything
below: **iterate, don't imitate; be clever, modern and efficient.**

- What Windows Mobile, Dell, Android, Apple, Linux or a textbook does is a
  finding, never the answer and never a ceiling. A textbook default or a
  copied value is a first draft to beat, not a design.
- For every design, three things are written down before building: the
  vendor or textbook way; the best way today's tools allow (computed
  offline, measured, fitted to this exact hardware); and why ours wins.
- Modernize everything that can be, while keeping the neo-retro look.
- Nothing is impossible or a hardware limit without proof: every idea and
  every "did we do it wrong?" check is ruled out by a measurement first.
- Small gains count: every percent adds up on this hardware.

Borrowing (below) is how rule 0 starts, not an exception to it: we take
the best idea the world already proved, then iterate past it. Copying it
as it is would be imitation.

## The method, for every design (rule 0 applied)

1. **Survey before drawing.** For the problem at hand (not the feature
   list), write down how Windows (Mobile and desktop), Android, Apple and
   the lean Linux systems built for old hardware (Tiny Core, Puppy, antiX,
   framebuffer-only stacks) solve it. One line each. They are findings,
   never the answer, and claims until measured here.
2. **Borrow the idea that wins.** Usually it is a good idea already proven
   on millions of devices; there's no prize for inventing it again.
3. **Beat it on the X51v.** Say how ours goes further, using what those
   systems didn't have: one program owning the whole machine, the build
   done on a modern PC (anything computable there is not computed on the
   device), today's tools (fitted filters, offline search, AI), and this
   exact hardware (the 2700G's planes and 2D engine, 64 MB, the card's
   measured speed).
4. **Measure, then adopt.** The borrowed idea and ours are both measured on
   the X51v (or shown to the owner on its panel, for anything the eye
   judges). The winner stays; the other is recorded.

Every design sheet for a screen or app carries a "borrow and beat" table
(problem; what each system does; what we take; how ours beats it) next to
its mock-ups, before the owner's vote: rule 0's three lines (their way,
the best way today, why ours wins) for each problem on the screen.

## What the big systems do, and what we take (first survey, 2026-10-31)

Leads from memory, not verified on anything (each is a claim until it is
measured here).

| Need | Windows | Android | Apple | Lean Linux | We borrow | Ours beats it by |
|---|---|---|---|---|---|---|
| Stack safety | a probe (`__chkstk`) touches each page of a frame over 4 KB; a guard page under every stack | guard pages under thread stacks | guard pages | `CONFIG_FRAME_WARN` (a warning per frame over 1-2 KB), `checkstack.pl`, guard pages under kernel stacks | the frame limit as a build failure; a guard page under the stack, so an overflow is a named fault, not quiet damage | the compiler's exact frame sizes and the whole call path checked in CI before an image exists; the guard catches only what the check can't see (calls through a pointer) |
| Pictures in many sizes | `.ico`: one file, a table of sizes, each picture apart; icon and thumbnail caches on disk by key | density folders, decode at the size shown (`inSampleSize`), an LRU sized to the device's memory class | asset catalogs: one indexed file, read lazily, decoded at size | one file per icon per size, read on demand | one indexed file (pack v2), decode at size, a small LRU | fewer sizes stored, the rest made by a downscale fitted offline to those very pictures; nothing decoded that isn't on screen |
| RAM under pressure | memory-mapped files: clean pages dropped and read again for free | `onTrimMemory`: caches dropped on request; "low RAM device" shrinks every cache | purgeable memory; memory warnings | zram/zswap: cold pages kept LZ4-compressed in RAM | caches marked purgeable, given back to the pool when something needs it; a compressed tier | off-screen pictures kept LZ4-compressed in RAM (our own `rp_lz4`): faster back than the card, a third to a fifth the size; measured before adopting |
| Fast start | Fast Startup (the kernel hibernated), prelaunch | Zygote (a process made once and copied for every app); a starting window | the launch screen; each app's last screen saved | the system run from RAM, unpacked from squashfs | the last screen saved and shown first | Zygote's work done at build time: tables, fonts and layouts as constants in the image, so start does nothing the PC could have done |
| Smooth touch | DWM composes windows the GPU already has | RenderThread, display lists, the overdraw view | Core Animation: layers drawn once, the GPU only moves them | a plain framebuffer | layers drawn once and only moved; overdraw counted | the 2700G's planes and 2D engine move whole pages with the CPU idle; overdraw per frame is a logged number |
| Picture depth | 16-bit modes | RGB_565 for opaque pictures on low-RAM phones | a format per picture in the catalog | 8-bit palettes, 16-bit framebuffers | 16-bit where the eye can't tell; palettes for pictures of few colours | each depth chosen on the X51v's own panel, not from a rule |
| Network | WinINet's cache by URL | the HTTP cache; WorkManager batching work into one wake | Background App Refresh, URLCache | `wget -N` (only if newer) | keep by key with its time; ask "changed since?" (a 304 costs a few hundred bytes) | fetch when the source says it changed (expiry times), never on a blind timer |

New rows are added as each new problem is surveyed (lists, text input,
the keyboard, transitions, the task switcher, sleep and wake, fonts).

## The practices, for every screen

From the first survey; each screen's sheet says how it meets them.

- **P1. Work at refresh, never during touch.** Fetching, parsing, choosing
  pictures, composing and decoding happen once when the content changes,
  in idle slices. Touch only moves pictures already made.
- **P2. Read only what is on screen.** Assets live in indexed files on
  flash or the card, read by offset, decoded at the size shown, into a
  small cache. The default set needs no card; extras are on the card.
- **P3. Compose once, then drop the parts.** A page picture keeps; its
  pieces (icons, text, curves) are given back until the content changes.
- **P4. Start from a snapshot.** The last screens are saved when they
  change and shown at once at start; work the PC can do moves to the
  build.
- **P5. The right depth per picture**, each chosen on the panel.
- **P6. Budgets the build enforces.** Each app has a RAM budget; no
  function frame of 4 KB or more but `main`, the deepest call path under
  75% of the stack (`cargo xtask stacks`, in CI); big objects made in
  their slot.
- **P7. Cache what the network gives, by key**, and ask only for what
  changed.

The targets that prove a screen meets them are measured on the X51v and
logged by the image: its RAM, the time to show it from start, frames per
second during touch, and the stack's deepest use.
