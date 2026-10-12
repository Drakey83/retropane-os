# Memory budget: the whole device, top-down

The per-image budgets in `xtask/src/size.rs` stop any one file growing
unseen. This page is the budget above them: how the X51v's **64 MB of RAM**
is shared, so the OS as a whole can't creep up on the apps. `cargo xtask
cross` (and so CI) checks it on every build.

## The rule

**The OS leaves apps at least 52 MB**: what Windows Mobile leaves free on
the owner's X51v (ADR-0005). So the OS's parts together get at most 12 MB.
Framebuffers, window surfaces and textures live in the 2700G's own 16 MB
(ADR-0005 rule 5) and don't count here.

| Part | Share | Built so far (budget) |
|---|---|---|
| Kernel image: code, data, static tables, boot stacks | 1 MiB | `rp-kernel`: 65 + 120 KiB |
| Kernel memory in use: page tables, compressed store, thread stacks | 1 MiB | measured at run time |
| Driver hosts: storage, display+input, audio, connectivity, power (ADR-0016) | 1 MiB | |
| Service manager and system services (files, registry) | 1 MiB | |
| Window manager (GWES), shell, Today screen | 3 MiB | |
| Windows Mobile libraries (coredll, aygshell, GDI, GAPI): code shared by all apps | 2 MiB | |
| Networking: TCP/IP, TLS 1.3, Wi-Fi | 2 MiB | |
| Headroom | 1 MiB | |
| **OS total** | **12 MiB** | |
| **Apps (at least)** | **52 MiB** | |

What the check enforces (`check_allotments` in `xtask/src/size.rs`):

- the parts plus the apps' 52 MiB fit in 64 MiB;
- every OS image belongs to one part, and its budget (loaded + zeroed)
  counts against that part in full;
- the images in a part fit inside its share.

A part's share changes only with the owner's approval, like any OS budget,
and only by taking from another part or the headroom: the apps' floor
doesn't move.

## What keeps it possible

- Code is demand-paged from its file and shared between processes (ADR-0009):
  an app's untouched code costs nothing, a library's code exists once.
- Reclaim (item 41): pages untouched for a sweep are compressed or let go.
- Compressed RAM (ADR-0005 rule 6) instead of swap: no flash wear.
- Fixed-size kernel tables sized for the real limits (31 processes, 128
  threads), all zero at start, so they cost nothing in the image.
- Cold code built for size; Thumb where it helps (ADR-0015).

## Today (2026-10-12)

During bring-up the kernel runs under the resident loader, which keeps the
low 16 MiB, so the kernel image manages the top 48 MiB (12,281 frames free
after its own setup). Once RetroPane boots on its own, the whole 64 MiB is
its. The kernel image takes about 176 KiB of its 1 MiB share.
