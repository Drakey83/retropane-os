# ADR-0029: The device's random number generator

- **Status:** Accepted (2026-10-15, by the owner: R1-R7 as recommended:
  "Accept ADR-0029 R1-R7 as recommended"). Nothing is built yet, and
  nothing runs on the Axim until the owner says go.
- **Date:** 2026-10-15
- **Gates:** the device side of LegacySync's link (docs/legacysync/device-link.md
  K2, accepted 2026-10-15), and later Wi-Fi (WPA3-SAE), TLS 1.3 and the
  device's own long-term keys.

## In plain words

Every key the Axim makes (the LegacySync pairing key, Wi-Fi handshakes,
TLS sessions) starts as random numbers. If an attacker can guess those
numbers, the cryptography around them is worthless however good it is.
Computers are bad at being random on purpose, so the random numbers have
to come from **physical noise**: tiny, unpredictable wobbles in clocks and
analogue signals. The PXA270 has no documented random-number hardware
(ADR-0017), so RetroPane has to find its own noise, **measure** how much
real unpredictability it holds, and stretch a small amount of it into as
many random bytes as the OS needs with a well-studied generator.

The plan, in one line each:

1. **Collect** noise from several sources on the device (clock jitter first).
2. **Check** every raw sample live, so a broken or stuck source is noticed
   (the "health tests").
3. **Count** only measured noise towards "ready"; everything else is mixed
   in but counts for nothing.
4. **Refuse to make any key** until 256 bits of measured noise are in.
   Boot waits for that (expected well under a second; measured later).
5. **Stretch** with a ChaCha20 generator that throws away its own key after
   every use, so even a later memory leak can't reveal earlier outputs.
6. **Keep a seed file** between boots as extra insurance, never counted, so
   a seed someone read or replaced can't hurt.

## Context

- docs/security.md principle 4: "a real random-number source seeded from
  hardware noise before any key is made". Not built: `rp-crypto`
  (`kernel/crypto`) has SHA-256, HMAC-SHA256 and a constant-time compare,
  and nothing else; no entropy code exists anywhere (checked on 49061cb).
- ADR-0017: no hardware RNG is documented for the PXA27x. The loader's
  pairing got around it by using a key the PC made. The device link can't:
  Noise `XX` needs the device's own ephemeral and long-term X25519 keys,
  and "the device's own must not be guessable" (device-link.md K2).
- Clocks on the X51v (from the HAL and the PXA27x manual):
  - the **core clock** (624 MHz turbo) and the **OS timer** (OSCR0,
    3.25 MHz, `hal/pxa27x/src/rt.rs`) are both made from the **13 MHz
    crystal** by the PLL, so they are not independent of each other;
  - the **32.768 kHz oscillator** has its own crystal and feeds the RTC
    and, if asked (`OMCR4` resolution 1/32768 s), OS timer channel 4;
  - the XScale **CP14 cycle counter** (CCNT) counts core cycles, kernel
    only; one OS-timer register read costs ~846 ns, a CCNT read a few
    cycles (docs/research/imitation-audit.md).
- Analogue parts: the **TSC2046** touch controller on SSP1 (docs/hw/touch.md)
  has 12-bit inputs besides X/Y/pressure: battery (`VBAT`), two
  temperature readings and `AUX` (TSC2046 datasheet; whether Dell wired
  `VBAT`/`AUX` to anything is unknown). The **WM8750** codec
  (docs/hw/audio.md) has a stereo ADC with a microphone preamp; RetroPane
  doesn't record yet.
- The card is a USB drive to the PC (ADR-0014): what's on it can be read,
  and in development written, by whatever is on the cable. The end state
  is RetroPane installed in flash (DESIGN §10c stage 3); the Dell
  bootloader region is never written.
- Owner's rules that shape this: security very seriously; legacy programs
  never refused for lacking modern features; design for 64 MB and the
  flash install; measure, don't assume; emulator tracks the device.

## Decisions (accepted 2026-10-15)

Each decision has a recommendation in bold. "Credit" means: counts towards
the 256 bits needed before keys can be made. "Mixed only" means: stirred
into the pool, can only add unpredictability, never counted.

### R1. Which noise sources, and which ones count

| Source | How it's sampled | Why it might be random | Available | Proposed role |
|---|---|---|---|---|
| **Two-crystal clock jitter** | Read CCNT each time the 32.768 kHz timer ticks; keep the low bits of the cycle-count difference | The 13 MHz and 32.768 kHz crystals drift against each other with thermal phase noise no program can predict | From the first instruction the kernel runs, no driver needed | **Credited, primary**, after measurement |
| One-clock execution jitter | Time a fixed memory/cache loop with CCNT (the "CPU jitter" method Linux's `jitterentropy` uses) | SDRAM refresh, bus arbitration, cache state | Early boot | Mixed only, unless measurement shows it holds up (an in-order XScale is less jittery than the desktop CPUs this method was tuned for) |
| TSC2046 battery, temperature, `AUX` | Low bits of repeated 12-bit conversions | Thermal and quantisation noise in the ADC | Once the touch driver host runs | **Credited, secondary**, after measurement |
| WM8750 microphone input at high gain | Low bits of samples with nothing plugged in | Thermal noise of the preamp, the textbook hardware noise source | Once recording exists | **Credited, secondary**, after measurement; used when the pool is low (it costs power) |
| Touch, key and interrupt timing | CCNT timestamp of every event | Human timing and device latencies | Whenever they happen | Low credit (at most 1 bit per touch or key event, 1 bit per 64 interrupts), capped. Note the pairing prompt itself is a tap |
| USB start-of-frame timing | CCNT at each 1 ms frame from the PC | The PC's crystal is independent | Cable in | **Mixed only**: the attacker we defend against is on that cable |
| Cold-boot RAM contents, device serial, Wi-Fi/Bluetooth MAC, RTC time | Hashed in once at boot | Makes every device's state different even if all else failed | Boot | **Mixed only** (the identity values are a "personalisation string", not secret) |
| Seed file from the last boot | R4 | | Boot | **Mixed only** |
| An undocumented PXA27x hardware RNG | See below | | Unknown | Mixed only, even if found |

**The 32 kHz check comes first.** The whole primary source depends on the
32.768 kHz clock having its own crystal. The PXA27x can also make its
32 kHz clock from the 13 MHz one; then both clocks are one crystal and the
"jitter" is a predictable beat pattern. The clocks unit's `OSCC` register
says whether the 32.768 kHz oscillator is on and stable; reading it is
the first device measurement (read-only).

**Why this counts and the rest doesn't:** a source gets credit only when
it is (a) measured on the device with NIST SP 800-90B's estimators, (b)
not under an attacker's control, and (c) watched by the health tests. A
measured source is credited at **half** the estimate, capped at 1 bit per
sample, because two-crystal sampling mixes real noise with a deterministic
beat that the estimators can mistake for randomness. Until a source is
measured on the device its credit is **zero**, so a build with no
measurements can't make keys (the emulator gets a development-only
exception, R7).

**The undocumented hardware RNG:** Intel's 2004 "Wireless Trusted
Platform" papers describe a trusted module with a random-number generator
for some PXA27x parts; it isn't in the public Developer's Manual and may
not be in the X51v's chip. Proposed: a time-boxed, read-only look (Dell's
ROM imports and register reads near the documented blocks) when convenient;
if found and it works, it is **mixed only** (an unauditable source never
carries the system alone).

### R2. The generator

| | **A. ChaCha20, fast key erasure (recommended)** | B. HMAC_DRBG (NIST SP 800-90A) | C. CTR_DRBG with AES (SP 800-90A) |
|---|---|---|---|
| How | Pool = running SHA-256; seed = 32-byte ChaCha20 key; each request makes ChaCha20 blocks, the first 32 bytes replace the key, the rest are the output, then wiped (Bernstein 2017; OpenBSD `arc4random`, Linux since 5.17) | Two HMAC-SHA256 per 32 bytes | AES in counter mode |
| New code | None beyond what the link needs: ChaCha20 comes with ChaCha20-Poly1305 (device-link order of work, step 1); SHA-256 exists | None (HMAC-SHA256 exists) | AES, which needs table-free code to be constant time on ARMv5: slow and large |
| Speed (estimate, measure) | ChaCha20 is the fastest of the three on ARM; DESIGN §11a already plans an iWMMXt ChaCha20 | ~4 SHA-256 blocks per 32 bytes; SHA-256 ~9 MB/s on this CPU | Slowest in constant time |
| A memory leak later reveals earlier outputs? | No (the key that made them is gone) | No, if updated after each request | No, if rekeyed |
| Standard | Widely used and analysed; not a NIST-named DRBG | NIST-approved | NIST-approved |

Recommended A. The pool hashing uses SHA-256, which SP 800-90B lists as a
vetted way to condense raw noise. If NIST naming ever matters (it doesn't
for this project), B is a small change with the same pool.

Reseeding: when at least 256 new bits have been credited and 60 s have
passed since the last reseed; also forced (if any new credit exists)
**after waking from sleep** and **before any long-term key** is made
(pairing keys, the device identity key). Requests are cut into chunks of
at most 4 KiB so one big request can't hold the generator long.

### R3. Early boot and the hard rule

The hard rule: **no key, and no random byte for RetroPane's own security
code, before 256 credited bits are in.** How boot meets it:

| | **A. Boot waits (recommended)** | B. A separate "weak, early" call | C. Never wait; warn instead |
|---|---|---|---|
| How | The kernel collects clock jitter right after the clocks are up, before starting any process, until 256 bits are credited | Early users get bytes from an unready pool through a differently named call | Bytes always flow; a flag says whether they were ready |
| Risk | Boot is slower by the collection time | Someone uses the weak call for a key; the bug class stays possible | The same, silently |
| Cost | Estimated tens to hundreds of ms at 32,768 samples a second (measured in R5's plan); the boot target is under 5 s | Code and a rule to police | None |

Recommended A: it removes "used before seeded" as a class of bug rather
than guarding against it. Details:

- Stack canaries, address-space randomisation slides and hash-table seeds
  are taken after the wait, so they need no early variant.
- If the sources fail their start-up health tests, or 256 bits aren't in
  within 2 s, boot **continues** without the generator being ready: the
  device works, every RetroPane key request fails with "not ready", the
  Today screen shows "Secure connections unavailable: random-number
  source failed" and LegacySync pairing and Wi-Fi with WPA refuse. Never
  a silent fallback. The pool keeps collecting and the generator becomes
  ready the moment enough credit arrives (a few touches can finish it).
- In the OS proper, user processes ask through one system call
  (`getrandom`-like: blocks until ready, or "not ready" with a no-wait
  flag). Because boot waits, the normal case never blocks.

### R4. Carrying a seed across boots

| | A. No seed file | **B. Seed file, mixed only (recommended)** | C. Seed file that counts |
|---|---|---|---|
| How | Every boot starts from fresh noise only | At boot: hash the file into the pool, then **immediately** overwrite it with fresh generator output (before anything else can read it). Again at clean shutdown and every few hours | As B, but the file earns credit, so the device is ready at once |
| If someone reads it (PC through the drive, card removed) | n/a | They learn one input; fresh noise still decides everything, because the file never counts | They can predict the next boot's keys |
| If someone writes it (planted value) | n/a | Hashing can't remove the other inputs' unpredictability: harmless | They choose the keys |
| If a crash means it's used twice | n/a | Harmless (not counted) | Two boots share a seed |
| Gain | Simplest | Defence in depth: if a noise source is ever weaker than measured, the history of every earlier boot is still in there | Fast ready |

Recommended B. C is ruled out by the card being a USB drive (ADR-0014).
The file is 64 bytes plus a version byte, written with the store's
copy-then-swap write so a power cut leaves the old or the new file, never
half of each.

### R5. Where the seed lives

Designed for the flash install, with the SD-card stage as a stand-in
using the same code and format (the owner's bare-metal-first rule):

| Stage | Where | Protection |
|---|---|---|
| 3, flash install (target) | A record in RetroPane's system store in flash, beside settings and partnerships; never the Dell bootloader region | The system store is not offered as a USB drive. Writes: two per boot plus one every few hours, 65 bytes each, through the store's wear-levelled writes; at a few boots a day that is negligible wear |
| 3, installing | The installer writes a **fresh seed made on the PC** into each device's system store | Every installed device starts different. The published OS image never contains a seed (one baked into the image would be the same for everyone) |
| 2, SD card (stand-in) | A file in RetroPane's system folder on the card | `rp_fat::Guard` refuses PC writes to it (as for boot files and partnerships, device-link.md K3). Reads by the PC are not blocked; acceptable because the file never counts (R4) |

The device's long-term private key (device-link.md K3) needs the same
place and is the more sensitive sibling: unlike the seed it is a real
secret. It is out of scope here, but the flash system store not being a
USB drive is what makes it safe at stage 3; at stage 2 it stays "made
fresh each boot" as K3 says, unless the owner decides otherwise.

### R6. Health tests and what a failure does

On every raw sample, per source, in the spirit of SP 800-90B §4.4:

- **Repetition count test**: too many identical samples in a row (cutoff
  from the source's measured entropy, false-alarm rate 2^-20) means the
  source is stuck.
- **Adaptive proportion test**: one value too common in a window of 512
  samples means the source has lost most of its noise.
- **Start-up test**: 1,024 samples per source through both tests before
  any of it is used.

| On a failure | **A. Drop the source's credit, keep going (recommended)** | B. Stop the system |
|---|---|---|
| What happens | That source's samples since the last pass earn nothing; after 3 failures in an hour it is off until reboot and the security log records it. If no credited source is left healthy, R3's "not ready" path takes over (the generator already seeded keeps working; new long-term keys wait) | Panic |
| Why | A phone that stops working because a thermometer misbehaved helps nobody; refusing keys is enough | Some certification schemes demand it; we aren't certifying |

The kernel runs the tests itself on the raw samples. A driver host
(touch, audio) hands in raw readings through a granted capability; the
**kernel** decides their credit from the measured table, never the host's
claim, so a compromised driver host can't fake readiness (and can't make
the pool worse, because mixing only adds).

### R7. Legacy programs and the emulator

- Windows Mobile's own calls (`CeGenRandom`, CryptoAPI's `CryptGenRandom`)
  are served from this generator when the CE personality exists. They
  **never fail and never block** in normal operation (boot already
  waited). In the rare failed-sources case they still return bytes from
  the pool (mixed, unready): old programs never checked for errors and
  must not break (the owner's legacy rule); the device-wide warning from
  R3 tells the user. Only RetroPane's own key-making refuses.
- The C runtime's `rand()`/`srand()` stay the old predictable formula:
  games rely on it (seeded replays, level generators), and nothing
  security-related should use it.
- No `fork()` in CE (processes start fresh), so the classic "two processes
  share one generator state" bug can't happen.
- **Emulator**: QEMU's timing comes from the host PC, so its "jitter" says
  nothing about the Axim. Not modellable as noise; a row goes in
  docs/hw/emulator.md. The fork gets a small noise device with three modes
  for tests: deterministic (repeatable runs), **stuck** and **biased**
  (each must trip the health tests: the failure path is tested, not
  hoped for). Emulator builds may credit the deterministic source only
  under a development-only build flag that `cargo xtask lint` refuses in
  release images, like the other development exposures in security.md.

**Accepted by the owner, 2026-10-15: R1 sources as in the table, R2 A,
R3 A, R4 B, R5 as in the table, R6 A, R7 as above.**

## How it will be measured

1. **Host tests (no device):** ChaCha20 against RFC 8439's vectors; the
   generator against its own known-answer tests (fixed pool in, fixed
   bytes out, key erased after each call); health tests against synthetic
   sources (stuck, biased, periodic, good), each with a mutant in
   `xtask/src/mutants.rs` that must make a test fail; random-sequence
   tests that the "no key before ready" rule holds whatever order sources
   arrive in.
2. **Emulator:** the boot-time wait, the not-ready path (stuck noise
   device), reseed after a simulated sleep, the seed file's rewrite before
   use and its copy-then-swap under a power cut at every write.
3. **On the device, only when the owner says go** (a test image,
   `rp-noise`, run from the loader; nothing written except a results file
   on the card):
   - read `OSCC` (is the 32.768 kHz crystal running on its own?);
   - collect 1,000,000 raw samples per source (clock jitter at several
     sampling spacings, one-clock jitter, TSC2046 `VBAT`/`TEMP`/`AUX`;
     audio once recording exists) to the card;
   - on the PC, run NIST's SP 800-90B estimators (`ea_non_iid`, NIST's
     public tool) for the min-entropy per sample;
   - the **restart test** that matters most for early boot: the first
     1,000 samples after each of 1,000 starts (loader re-runs), plus about
     20 real power-ups by the owner, to check that a fresh boot doesn't
     repeat itself;
   - the results go in a new `docs/hw/entropy.md`, with the credit table
     the kernel uses; measured time to "ready" goes in docs/perf.md.

## Size and memory

| Part | Where | Estimate | Budget rule |
|---|---|---|---|
| ChaCha20 core | `rp-crypto` | ~1-2 KiB code | Needed for the link anyway |
| Generator, pool, health tests | `rp-crypto` (pure, host-tested, no `unsafe`) | ~1.5 KiB code | |
| Collector (CCNT, timer, the boot wait), system call | kernel / HAL | ~1.5 KiB code | |
| State in RAM | kernel | < 512 bytes: SHA-256 pool 104, key 32, per-source test state ~24 each, counters | Fixed-size, no allocation |
| `rp-noise` | test image | whatever it needs | Test images ratchet freely |

About 4-5 KiB on `rp-kernel` in total, under the 8 KiB per-step limit of
the kernel's standing rule, from a planned subsystem; the kernel's 1 MiB
share in docs/memory-budget.md is unaffected in practice. Nothing here is
in an OS part outside the kernel, so no other budget needs approval. Not
Thumb (ADR-0015): the generator is warm code when TLS is busy.

## Security

- **Outside input:** none parsed. The seed file is fixed-size; any other
  size or version is ignored (and rewritten). Driver hosts' raw readings
  are bounded counts of fixed-size values through a granted capability.
- **What could go wrong, and the defence:**
  - a source weaker than measured → credit at half the estimate, several
    sources, the seed file's history mixed in;
  - a stuck or failing source → health tests, the not-ready path;
  - an attacker controlling an input (cable timing, the seed file, a
    compromised driver host) → those are mixed only; mixing can't lower
    unpredictability; credit is the kernel's decision;
  - memory read after the fact (a kernel bug, cold-boot attack) → fast
    key erasure: earlier outputs can't be recomputed; output buffers
    wiped;
  - two devices alike → the installer's per-device seed, the device's
    identity values in the pool, no seed in the published image;
  - timing side channels → ChaCha20 and SHA-256 have no secret-dependent
    branches or table lookups; constant-time tests as for HMAC.
- **Exposes:** random bytes to processes through one system call; nothing
  about the pool or the sources. The not-ready state is visible (on
  purpose).
- **Privileged:** the pool and generator live in the kernel (they must
  survive any single process crashing); everything that can run outside
  (touch and audio sampling) does, in driver hosts.
- **Development-only:** the emulator's credited deterministic source,
  refused in release builds; listed in docs/security.md's exposures table
  when built.

## The four lenses

| Lens | Effect |
|---|---|
| 🔁 Iterate | Every source is measured on the device with NIST's estimators before it earns anything; credit is a table updated from measurements; mutants prove each health test bites; time to "ready" is a benchmark |
| ✨ Modernize | The 2017+ design (fast key erasure, as Linux and OpenBSD now use), SP 800-90B health tests, a hard "no key before seeded" rule, per-device install seeds; WM had none of this visible to us |
| 🗄️ Legacy support | `CeGenRandom`/`CryptGenRandom` served and never failing; `rand()` untouched for games; no program is refused because the RNG is new |
| ⚡ Better / faster | Ready in (estimated) well under a second with no driver needed; ChaCha20 faster than the SHA-based alternatives and later iWMMXt; one generator for the whole OS (shared, ~0.5 KiB RAM) |

## Hardware limit or choice?

- **No hardware RNG:** a hardware limit as far as the public manual goes;
  the trusted-module question (R1) is the one unexplored corner, looked at
  read-only and never relied on.
- **What Windows Mobile did:** how WM5 seeds `CeGenRandom` isn't
  documented to us. It's a finding to look up later (Dell ROM imports),
  not a design to copy; this ADR doesn't depend on it.
- **Which clocks are independent:** set by the board's crystals; `OSCC`
  will say.

## Alternatives considered

- **Trust the PC's randomness** (the LegacySync PC sends a seed): rejected
  as the device's only source: whoever is on the cable would know the keys,
  and the device must work with no PC (Wi-Fi, TLS). Kept as an install-time
  seed (R5), mixed only.
- **Port `jitterentropy` whole** (a published, certified single-clock
  design): rejected as the primary source: tuned for out-of-order desktop
  CPUs, larger, and our two-crystal source is the cleverer fit for this
  board. Its one-clock method is still measured (R1) and its health-test
  practice is borrowed.
- **Fortuna** (many pools, no entropy estimates): robust against wrong
  estimates, but slow to its first seed and larger. The measured-credit
  design plus the seed file covers the same risk more cheaply; revisit if
  measurements turn out unstable.
- **Generators:** R2's table.

## Consequences

- LegacySync's device side (device-link.md order of work) can proceed once
  ChaCha20 and this generator are in `rp-crypto` and the device
  measurements give the clock source a credit. Before measurements, it
  runs in the emulator only (development flag).
- Boot gains a short wait; a broken source shows the user a warning
  instead of quietly making weak keys.
- New docs when built: `docs/hw/entropy.md` (measurements, credit table),
  an emulator.md row, security.md rows (finding, development-only
  exposure), perf.md entries.

## Open questions

1. Is the X51v's 32.768 kHz clock its own crystal (`OSCC`)? Everything in
   R1 leans on it. **Likely yes** (found 2026-10-15 in the probe's
   2026-10-11 run: `OSCC = 0x0F`, the oscillator on and stable;
   docs/hw/entropy.md); `rp-noise` reads it again.
2. Are the TSC2046's `VBAT` and `AUX` inputs wired to anything on the X51v?
3. The measured entropy per sample of each source, and so the real time to
   "ready".
4. Whether the PXA270 in the X51v has Intel's trusted module.
5. Where the device's long-term private key lives at stage 2 (device-link
   K3); shares R5's answer at stage 3.

## Order of work (after the owner's vote)

1. `rp-crypto`: ChaCha20 (with the link's ChaCha20-Poly1305), the pool and
   generator, the health tests; host tests and mutants.
2. Emulator noise device (deterministic, stuck, biased) and the kernel's
   collector and boot wait; emulator tests.
3. `rp-noise` test image. **Device runs only when the owner says go.**
   Built 2026-10-15 (the owner's go on the measurements): checked in the
   emulator (`cargo xtask emu-noise`, plumbing only); the device run waits
   for the owner.
4. The measured credit table, `docs/hw/entropy.md`; the generator becomes
   ready on the device; LegacySync's device side may use it.
5. Seed file (SD stand-in), then the flash record with the installer at
   stage 3.
