# ADR-0032: The audio service: one mixer, one owner of the codec

- **Status:** **Accepted (owner, 2026-10-18): A1 A, A2 A, A3 B with the
  hearing warning, A4 B, A5 A**, and R1-R10 with the owner's thirteen
  conditions, all landing before S1 (listed under "The owner's
  conditions" below; written into R2-R7, A2-A5 and the steps). Proposed
  and revised after a review the same day.
- **A7 (virtual bass on the speaker): B, overtone virtual bass (owner,
  2026-10-19)**, on the speaker only, switchable; a speaker sweep and the
  owner's blind A/B by ear before it ships.
- **A6 (quality audit E3, the DMA chain that stops at the mix's end):
  the owner chose B on 2026-10-19, conditional on the two device
  measurements; both passed on the X51v the same evening; B built the
  same night (below, "A6 as built"), emulator-checked (`emu-audio-e3`),
  and **PASSED on the X51v by the owner's ears (2026-10-19, run 2:
  "a little gap" in A, "no gap" in B).** Follow-ups: the stream after a
  stall (dry once, counted interactive), the test's forced wait, and
  quality audit C17.**
- **Date:** 2026-10-18

## In plain words

Sound works on the X51v (docs/hw/audio.md, 2026-10-18): the WM8750 codec
is found, powered and fed by DMA, and plays on the headphones and the
speaker. What's missing is the part every program talks to: **the audio
service**. It takes sound from any number of programs at once, mixes it
into one stream, sets the volume, picks the output (speaker, headphones,
later a Bluetooth headset), switches the hardware on when sound starts and
off when it's quiet, and answers old Windows Mobile programs through the
same `waveOut` calls they always used.

Five questions change what you hear or see, so they are yours (A1-A5,
each with pluses and minuses and a recommendation). The rest is
engineering with a recommendation (R1-R10).

## The owner's decision (2026-10-18)

| Vote | Chosen | The owner's reason |
|---|---|---|
| A1 the rate | **A: 44.1 kHz fixed** | Music and the old games live in the 44.1 family, the old games convert in exact whole steps; the 48 kHz conversion cost is measured before it's final. No rate switching, no gaps |
| A2 headphones out | **A: pause** | As phones do; programs too old to pause are muted instead, unmuted when the headphones go back in |
| A3 volumes | **B: per output, per kind**, with the hearing warning | First time past the level in a headphone session, never a hard cap |
| A4 tone | **B: the codec's bass and treble in Settings, plus the speaker tuning** | No crackle at full volume; no software equaliser |
| A5 microphone | **A: ask once per program, a mark while anything records** | Old programs are asked like everything else, never refused for being old |

### The owner's conditions (all before S1)

1. Stream roles (Media, Notification, Alarm, System, Voice) declared by
   native programs, a documented legacy mapping, the unplug policy per
   role (R3, A2).
2. The signal chain's order fixed; speaker protection in the mixer per
   output; the codec's bass and treble for the user and X50Mix, last
   writer wins, Settings shows the live value (R4, A4).
3. Start times get a feedback path: actual start reported back (R3).
4. Stream rings sized for the DMA ring plus the margin; the CE library
   owns a shadow ring that meets it (R2, R7).
5. Idle I2S power-down must beat the baseline (Windows Mobile's: on while
   awake); S1 measures idle codec power first (R5, S1).
6. A jack edge while the codec is down sets a flag only, no power-up (R6).
7. The limiter: peak detection, ~1 ms attack, ~100 ms release, hard clip
   as the backstop (R4).
8. The microphone's program identity is the executable's hash; a changed
   binary is asked again (A5).
9. S4 measures `WOM_DONE` latency against stock Windows Mobile on the same
   game (R7, S4).
10. The hearing warning is a codec-gain threshold (~-10 dB), once per
    plug-in (A3).
11. The day-to-day slider drives the active output's dominant kind (A3).
12. Bluetooth tone is done in software (the codec can't reach A2DP); A4's
    "no CPU" holds for the codec path only, and X50Mix's codes don't
    affect Bluetooth (A4).
13. The mixer's wake interval adapts (~15 ms with interactive streams,
    ~200 ms for background media only); rings stay sized for the maximum;
    switching between the two is glitch-free; a stream is interactive by
    its low-latency hint or by a write-pattern heuristic (R2).

## Context: what the hardware dictates (measured)

| Fact | Consequence for the service | Evidence |
|---|---|---|
| **A switch-on of I2S within ~645 ms of a switch-off is ignored** (`SACR0` reads 0) | I2S stays on for a whole session; power-down only after an idle time; a sound asked for inside the 645 ms after a power-down must wait for it (a guard, not a silent failure) | audio.md "How long I2S ignores a switch-on" |
| **One I2S link, one clock** for playback and recording | Playback and the microphone run at one shared rate; changing rate means stopping I2S (the 645 ms rule) | PXA27x manual; `SADIV` sets both |
| **The headphone jack (GPIO83) reads true only while the codec is powered and set up**; idle it reads 0 either way | After power-up: set the codec up, read the jack, **then** decide whether the speaker amplifier goes on (never the amplifier first: a sound meant for headphones could start on the speaker) | audio.md (Windows Mobile measured, `rp-tone` run 9) |
| Jack changes raise an interrupt (GPIO83 both edges) | No polling: the service waits on the jack's event | interrupts.md (SYSINTR 35) |
| Speaker amplifier: CPLD `0x14` bit 1, **70 ms to settle** | Speaker sound starts 70 ms after the amplifier; the service starts the amplifier early (on the first "about to play") | cpld.md, audio.md |
| Codec power: CPLD `0x1C` bit 1; on power-down Windows Mobile drives the I2C pins low | The power-down order matters (no signal into an unpowered chip) | audio.md "Power, from the same driver" |
| The codec's own controls: volume in 1 dB steps at zero crossings, bass and treble, 3D widening, anti-pop ramps | Volume and tone cost no CPU when done in the codec; Dell's `WAV1:` codes `0x1D2000`/`0x1D2004` reach the bass/treble | WM8750 datasheet (public); audio.md (X50Mix) |
| Windows Mobile runs the link at **44.1 kHz** (`SADIV` `0x0D`) and never switches I2S off between sounds | A finding, not a rule (A1 decides the rate on its merits) | audio.md (`rp-wmaudio`) |
| No FPU; iWMMXt has 16-bit multiply-accumulate in parallel | Mixer and resampler in fixed point, hot loops in iWMMXt | DESIGN.md |
| DMA streams the ring without the CPU (0% busy measured at 48 kHz) | The CPU only fills the ring; how often it wakes to do so is the power cost (R2) | audio.md run 6 |
| Standby (`PWRMODE` 2) is refused while audio streams; a clock relock is refused while the I2S FIFO margin is short | The service tells the power service when it streams and how much margin it has | ADR-0031 D3, D5 |

## How modern systems do it (the survey)

Four designs, read for what they get right on a small battery device:

| System | The idea worth having | What doesn't fit the X51v |
|---|---|---|
| **Android AudioFlinger** | One mixer thread at a fixed rate; a separate *fast* path for games with small buffers; per-category volume (media, alarms, calls, notifications); "becoming noisy" pauses music when headphones come out | Several threads and copies per stream; built for multi-core |
| **PulseAudio "glitch-free" (timer-based scheduling)** | **A large hardware ring for low power, rewritten ahead of the DMA position when a new sound arrives**: long sleeps while music plays, yet a click starts within milliseconds | The rewind logic was famously complex; we need it small and tested |
| **PipeWire** | One graph and one *quantum* (period) for every client; clients write into shared memory, no copies; latency chosen per client | A graph engine is more than one codec needs |
| **Apple Core Audio** | Pull model with precise timestamps: the mixer asks each stream for exactly the frames it needs at a known hardware time; sample-accurate start for games and A/V sync | Callbacks into client code at interrupt priority need trust we don't give programs |

Windows Mobile's own `waveapi` (one software mixer feeding Dell's driver,
44.1 kHz, I2S always on) is the baseline to beat, not the model.

## The owner's votes

### A1. The rate the hardware runs at

Every sound is converted to one rate before mixing. Which rate?

| | **A: 44.1 kHz fixed (recommended)** | B: 48 kHz fixed | C: follow what's playing |
|---|---|---|---|
| How | The link always runs at 44.1 kHz | The link always runs at 48 kHz | The link switches to the rate of the main sound (44.1 for an MP3, 48 for a video) |
| Music (MP3, CD rips, most podcasts: 44.1 kHz) | **Bit-exact**, no conversion | Converted (fractional 147:160) | Bit-exact |
| Old Windows Mobile games (11.025 / 22.05 kHz almost always) | **Whole-number steps** (x4, x2): simple, clean | Fractional conversion | Converted to whatever runs |
| Modern 48 kHz sound (Opus, many videos, Bluetooth HFP's 16 kHz) | Converted (fractional) | Bit-exact | Bit-exact while it's the main sound |
| Bluetooth headsets later (A2DP's SBC takes 44.1 or 48) | Either works | Either works | Either works |
| Cost | Conversion only for 48 kHz content (estimated under 1% CPU with iWMMXt; measured before it's final) | Conversion for nearly all content | **Every switch: I2S off ≥ 645 ms (a gap, perhaps a pop)**; a second sound at another rate still needs conversion |
| Unknowns | 44.1 kHz from `SADIV` `0x0D` is approximate on the PXA; the pitch error is tiny and inaudible (measure it) | 48 kHz proven on the device | Whether `SADIV` can change while I2S runs (untested) |

Why A: the content this device plays most (music, old games) is in the
44.1 kHz family, so A converts the least and converts the old games
exactly. It happens to match Windows Mobile, but for that reason, not by
imitation. C sounds best on paper but buys gaps and pops for little.

### A2. Headphones pulled out while music plays

Every stream has a role (R3: Media, Voice, Notification, Alarm, System),
so the answer can differ by role. The table is the policy per role.

| Role | **A: modern (recommended)** | B: as Windows Mobile |
|---|---|---|
| Media (music, video, games' sound) | **Pause** (native players get a "paused by unplug" event); legacy media, which can't be paused, is **muted** until the headphones go back in or you unmute it | Moves to the speaker |
| Voice (calls, later) | Moves to the speaker (or stays on a Bluetooth headset) | Moves to the speaker |
| Notification, System | Moves to the speaker | Moves to the speaker |
| Alarm | Moves to the speaker (alarms must be heard) | Moves to the speaker |
| Plus | What phones have done for a decade; no surprise blast in a quiet room; a cable yanked by accident costs nothing | Nothing to resume |
| Minus | One tap to resume (or unmute an old game) | Loud surprises; not what anyone expects in 2026 |

### A3. The volume controls

| | A: one volume (as Windows Mobile) | **B: per output, per kind (recommended)** |
|---|---|---|
| How | One master volume, plus "system sounds on/off" | Separate remembered volumes for **speaker, headphones and (later) each Bluetooth headset**; inside each, a level per role: **Media**, **Notification and System**, **Alarm** (never fully silent), **Voice** (once Bluetooth calls exist) |
| Plus | Simple; one slider | Plugging in headphones never starts at the speaker's loud level; alarms still wake you with media muted; matches modern phones |
| Minus | Headphones inherit the speaker's level (too loud or too quiet each switch) | More settings (one slider day to day, below; the rest under Settings) |

**The day-to-day slider under B** (as Android does): it sets the **active
output's level for the dominant role**: the role of the loudest stream
playing now (Voice over Media over Alarm over Notification), or Media when
nothing plays. The slider names what it's setting ("Media, headphones").
Levels are mixer gains per role on top of one codec output volume per
output, so changing a role never changes another.

Part of B, also yours to say yes or no: **a gentle hearing warning**, never
a cap. Numbers: shown once per **session**, a session being one plug-in of
the headphones, the first time the headphone output's codec gain plus the
Media level goes above **-10 dB of full scale** (approximate: headphone
impedance and sensitivity are unknown, so this is a level, not a sound
pressure; the threshold is tuned by ear with the owner in S3).

### A4. Tone controls (bass, treble)

| | A: none (only what X50Mix asks for) | **B: the codec's own bass/treble in Settings, plus a speaker tuning (recommended)** | C: a software equaliser (several bands) |
|---|---|---|---|
| CPU | none | **none on the wired outputs: the WM8750 does it** (software only for Bluetooth, below) | some per sample (measure) |
| Plus | Nothing to design | Free, works for every program; a tuned preset for the small speaker (less boom it can't play, a little more presence) and a limiter so it doesn't crackle at full volume; X50Mix's settings go to the same control | Most flexible |
| Minus | The small speaker sounds as Dell left it | Two bands only | Battery and CPU; more to get wrong |

Under B, who owns what (one owner per control, R4 has the chain):

- **Speaker protection** (the tuning for the small speaker: cutting bass it
  can't play, keeping it from crackling) is **not** the codec's tone
  control: it's one or two filters in the mixer, on the speaker output
  only, always on, not a user setting.
- **The codec's bass and treble** belong to you and to X50Mix (Dell's
  `WAV1:` codes): **the last writer wins**, and Settings always shows the
  codec's live value, so an X50Mix change appears there.
- **Bluetooth headsets** don't pass through the codec (A2DP carries the
  mix, not the codec's output), so the Settings bass and treble are applied
  by the mixer in software for a Bluetooth output (a little CPU, only
  then). **X50Mix's `WAV1:` codes act on the codec only and don't affect
  Bluetooth** (condition 12).

### A5. Who may use the microphone

The X51v's built-in microphone (the Record button's voice notes) goes
through the same codec. A program listening in the background is the
classic spy risk on a connected device (docs/security.md).

| | **A: ask once per program, show when it's on (recommended)** | B: always allowed |
|---|---|---|
| How | The first time a program opens the microphone, RetroPane asks; the answer is remembered and changeable in Settings; a small mark in the title bar while any program records. **A program is known by a hash of its executable** taken at its first microphone use, not by its path: a different file put in the same place on the SD card (or an update) is asked about again. (Signed packs could later be known by their signer, so updates keep the answer.) | Any program records any time |
| Plus | Modern privacy; old programs still work (they're asked about, never refused for being old: legacy is not locked out) | No prompt, like Windows Mobile |
| Minus | One prompt per program | Any program can listen without you knowing |

## Recommendations (engineering)

### R1. Where it runs
The service lives in the **audio driver host** (ADR-0016 D5) together with
the codec, I2S, I2C-gate and DMA drivers it owns: one hop from a program to
the mixer, a fault in audio can't reach anything else. Until IPC moves
pages it's a library (`services/audio`, `rp-audio`) used in one process,
with the same interface (the graphics service's path, ADR-0030 G4).

### R2. Low power and low latency at once: a long ring, rewritten ahead
From PulseAudio's glitch-free design, kept small. The DMA ring is long
(target 250 ms; ~44 KiB at 44.1 kHz stereo 16-bit), so with music playing
and the screen off the CPU wakes about every 200 ms (5 times a second) instead of ~50 times.
When a new sound arrives (a key click, a game's shot), the mixer reads the
DMA engine's position and **re-mixes from just ahead of it** (a safety
margin, measured, target ≤ 5 ms), so it starts within milliseconds. To
allow that, each stream keeps two positions: "mixed" and "played"; data
stays in a stream's ring until played. Games and the shell get the low
latency automatically; nothing asks for a "fast mode".

**The wake interval adapts (condition 13).** The mixer wakes about
every **~15 ms while any interactive stream is open** and about every
**~200 ms when only background media plays** (the case where wake-ups cost
battery); both are tuned in S2. A stream is interactive when its program
says so (**a low-latency hint** in the control block, R3: games, the
shell's sounds, Voice) or when **its write pattern** shows it (small
writes at short intervals, as old `waveOut` games do: a heuristic, so a
program that never sets the hint still gets low latency). **Switching
between the two intervals is glitch-free**: going to 15 ms only shortens
the next wake; going to 200 ms first mixes ahead the longer stretch while
every stream's data is there, so no sample is played that wasn't mixed.

**Ring sizing rule (condition 4).** Because data stays until played,
**every stream's ring is sized for the maximum: the DMA ring plus the
margin plus one refill of its own** (at 44.1 kHz stereo 16-bit about
**50 KiB**), whatever its current interval, so a switch of interval never
finds a ring too short. The library sizes rings by this rule when a stream
opens, and the service refuses a smaller one with a clear error.

### R3. Programs write into shared rings; no copies
Each stream is a ring in the global shared region (ADR-0016: buffers two
parties use at once), owned by its program and read by the mixer; "room
for more" and "drained" are waitable events (ADR-0010), not polling.

Each stream has a small **control block** beside its ring:

- **Role**, declared by a native program when it opens the stream:
  **Media, Voice, Notification, Alarm, System**. A2's unplug policy and
  A3's levels are per role, so the role is required (no default guessing
  for native code). Old programs get roles by a fixed mapping in the CE
  library: `waveOut` → Media; `PlaySound`, `sndPlaySound`, `MessageBeep`
  → System; the clock and calendar alarms (Windows Mobile's notification
  sounds, through the CE notification API) → Alarm.
- **Requested start** (program → service): a sample time on the output's
  clock, or "as soon as possible".
- **Actual start** (service → program): with mix-ahead, the real start is
  the later of the request and the DMA position plus the margin, which a
  program can't predict; the service writes the start it actually got, and
  games read it for picture-to-sound sync.
- **Played position** (service → program): frames the output has really
  played, the base for sync and for `waveOutGetPosition`.

The
mixer reads PCM 16-bit (8-bit and 24/32-bit converted on the client side by
the library). Decoders (MP3, AAC, Opus, FLAC) are libraries in the
programs, not in the service: a damaged file crashes the player, never the
audio host (docs/security.md).

### R4. The mixer
The chain, in order, per output:

1. **Rate conversion** of each stream to the output rate (below).
2. **Per-stream gain**: the stream's own volume times its role's level
   for this output (A3), with short ramps (no zipper noise on changes).
   Legacy media muted on unplug (A2) is a gain of 0 here.
3. **Ducking (optional, measured):** when the sum would overload, the
   loudest stream that isn't an Alarm is turned down first, so an alarm
   stays clear over loud music.
4. **Sum** in 32 bits.
5. **Output filters:** speaker protection on the speaker output (A4,
   always on); the user's two tone bands on a Bluetooth output (the codec
   does them for wired outputs).
6. **Limiter:** a peak detector driving a gain, **~1 ms attack, ~100 ms
   release** (values tuned in S2), so loud peaks are ridden down instead
   of clipped; a **hard clip only as the backstop** for what the limiter
   can't catch in time.
7. **TPDF dither** back to 16 bits, into the DMA ring.
8. In the codec: the output volume (1 dB steps, zero-cross latched) and,
   on wired outputs, the user's bass and treble (A4).
- Rate conversion by a windowed-sinc polyphase filter (whole-number steps
  for 11.025/22.05 kHz; 147:160 for 48 kHz under A1-A), fixed point, the
  hot loops in iWMMXt. Quality is measured, not assumed: host tests feed
  sine sweeps and check THD+N and aliasing against a target.
- Volume goes to the codec (1 dB steps, zero-cross latched) where it
  applies to everything on an output; per-stream gain in the mixer.

### R5. Power: sequences, idle power-down, no pops
- **Power-up:** codec power (CPLD `0x1C` bit 1) → I2S pins and clock →
  I2S on (once per session) → codec reset and set-up with its anti-pop
  ramp (WM8750 datasheet order, not Dell's timings: those are copied
  values to work, per the imitation audit) → read the jack → the speaker
  amplifier only if needed (70 ms) → unmute.
- **When audio powers down: the baseline is Windows Mobile's behaviour**
  (I2S and the codec stay up while the device is awake; down on suspend
  or when the screen goes off with nothing playing). **Corrected
  2026-10-19 (`rp-wmpower`, docs/hw/audio.md):** Windows Mobile powers the
  *codec* down about 2 s after the last sound and keeps *I2S* running, so
  it saves the codec's 10-35 mA without meeting the 645 ms rule. The
  choice that follows is the owner's (codec-only idle power-down, and
  its timeout and latency). **Decided by the owner, 2026-10-19 (option
  A):** after **5 s** of quiet the service powers the **codec** down (the
  amplifier off, the codec's insides down, its supply off) and **keeps I2S
  running**, so the 645 ms rule never applies; it **wakes the codec
  early**, at a touch on the screen or a key press (a click sound usually
  follows), so the codec's start-up (75 ms of VMID charge, 70 ms for the
  amplifier) is hidden behind the tap. A sound with no touch before it (an
  alarm, a notification) waits ~0.15 s. Full power-down (I2S off too) only
  on suspend. The 5 s is a tunable, measured later. Weighed and not
  chosen: B (no early wake: the first click after a quiet spell ~0.15 s
  late) and C (the codec on whenever the screen is on: 4-12% more drain).
  **Built and passed (S1b, the owner, 2026-10-19):** on the device the
  codec slept 5.0 s after each sound with I2S running, woke in 152 ms, and
  the sound after a tap started at the lift. Because of the 645 ms
  rule, powering down between sounds turns into a latency trap: a click
  arriving inside the window waits. **Idle power-down between sounds is
  built only if S1 measures that an idle, powered codec (with I2S running)
  costs enough battery to be worth it**, and then with a timeout long
  enough that the trap is rare. The power-down order is the reverse of
  power-up, I2C pins low as Windows Mobile does, the I2S restart guard
  armed.
- **Pops** are judged by ear with the owner (no loopback exists to record
  the output); each sequence change is a device run.

### R6. Jack and routing by interrupt
While audio is powered: GPIO83's edge event, debounced (~50 ms), re-checked
against the codec's set-up state; a change re-routes per A2 (per role) and
swaps to that output's remembered volumes (A3). **While audio is powered
down, a jack edge does not power it up**: the level can't be read then,
and where sound goes matters only when sound plays. The edge sets a "jack
changed" flag; the next power-up reads the jack (it always does, R5) and
routes from that.

**Found while building S1 (2026-10-18, emulator; device run pending):**
GPIO83 reads high only while the codec is powered, so (a) **switching the
codec off with headphones in makes a falling edge of our own**, which the
latch caught and took for a jack change (fixed: the service clears the
latch at power-down; test and mutant); and (b) **a plug or unplug while
audio is off most likely makes no edge at all** (under Windows Mobile the
line reads 0 idle, plugged or not), so condition 6's flag will rarely or
never be set on this board. Nothing depends on it: every power-up reads
the jack, after a settle (75 ms, Dell's, copied until S1 measures it).
Every output is an `Output` (speaker, headphones;
later Bluetooth A2DP and HFP), so Bluetooth headsets (docs/design/
peripherals.md) plug into the same routing without changing the mixer.

### R7. Old programs (the CE personality)
`waveOutOpen/Write/Reset/GetPosition`, `waveIn*`, `PlaySound`,
`sndPlaySound`, `MessageBeep`, and the `WAV1:` device with Dell's private
equalizer codes (X50Mix), mapped onto streams. Their buffer-done callbacks
(`WOM_DONE`) keep the timing old games expect (buffers complete when
played, not when copied). Their rates (8-22.05 kHz, 8-bit, mono) are
converted by the library.

Old programs often queue small buffers, smaller than R2's sizing rule.
**The CE library owns a shadow ring per `waveOut` handle** that meets the
rule: it copies each buffer in (and converts it), and returns the
program's buffer with `WOM_DONE` when those samples have been **played**
(from the played position, R3), not when copied. A legacy stream that writes small buffers often is
taken as interactive by the write-pattern heuristic (R2), so the mixer
wakes often while it plays.

**Parity is measured, not assumed:** S4 records the distribution of the
time from `waveOutWrite` to `WOM_DONE` on the same game under stock
Windows Mobile and under RetroPane; games that pace or sync animation by
`WOM_DONE` must see no more delay than on Windows Mobile.

### R8. Security
Programs never touch the codec, I2S or DMA; they hold stream handles only.
Ring indices are checked on every read (a program can make its own sound
wrong, never the mixer read outside its ring). Stream count and ring sizes
per program under a quota. The microphone per A5. No parser in the service
(R3).

### R9. Testing and the emulator
Host tests for the mixer, limiter, dither and resampler (golden signals,
THD+N), with mutants; the fork's codec model records its output, so
`emu-` tests check what would come out of the headphones bit for bit; the
645 ms rule, the jack and the amplifier are already modelled (rp-v0.24.0).
Device runs (owner listening) for pops, latency (a key-click loop timed by
the DMA position) and battery.

### R10. Memory and size
Within the audio host's share of the 1 MiB for driver hosts
(memory-budget.md): the DMA ring ~44 KiB, the mixer's state per stream a
few hundred bytes, program rings in the programs' own budgets (R2's
sizing rule: ~50 KiB per stream at 44.1 kHz stereo, so ~400 KiB with eight
streams open across all programs; mono and 22 kHz streams are smaller;
the CE library's shadow rings in the old program's budget). S2 records the
streams real use opens at once, so the cost is a measurement, not a
guess. Code budget
set when the first crate exists (ratcheted per the budget rule, asking if a
step grows it by more than 8 KiB).

## Steps (each proven on the device and modelled before the next)

1. **S1 one stream, the power sequence and the jack interrupt** (R1, R5,
   R6): the shell's click on the device; the restart guard; **the
   measurement R5 waits on: the battery current with audio idle but
   powered (codec set up, I2S running) against audio off**, which decides
   whether idle power-down between sounds is built at all. RetroPane has
   no battery gauge yet (ADR-0031 step c), so it's measured **under
   Windows Mobile**, which reports the battery's current:
   `rp-wmpower` (tools/wmbench), read-only, on battery, phases with the
   sound device closed, open and playing silence, and open idle.
   **Built 2026-10-18:** the core (`services/audio`), the guided device
   run `rp-audio-s1` (`cargo xtask audio-s1`; its emulator check
   `emu-audio-s1` passes) and `rp-wmpower`. **Passed by the owner
   2026-10-19** (docs/hw/audio.md, runs 1-3 and the diagnostics); the
   idle-power measurement (`rp-wmpower`) is carried forward, so R5's
   baseline (audio up while awake) stands until it runs.
2. **S2 the mixer** (R2-R4): roles, several streams, the rewrite-ahead
   and its adaptive wake interval (15 / 200 ms), the actual-start field; latency and wake-ups
   measured with music and clicks; limiter times tuned.
   **Built 2026-10-19** (`services/audio`: `mixer`, `chain`, `resample`,
   `role`; 63 host tests, 19 mutants all killed; the image `rp-audio-s2`
   (`cargo xtask audio-s2`) on the X51v wiring now shared as
   `rp-audio-x51v`; its emulator check `emu-audio-s2` passes). What was
   built and the engineering it settled are under "S2 as built" below; the
   numbers are in docs/hw/audio.md. **Passed by the owner on the device
   (2026-10-19, second listening run):** every test heard as meant; the
   one-ear sound in the headphones was the earbuds (they play one side
   under Windows Mobile too).
3. **S3 volumes and routing** (A2, A3, A4).
4. **S4 the CE personality's sound calls** (R7) with an old game and X50Mix,
   including the `WOM_DONE` latency comparison with Windows Mobile.
5. **S5 recording** (A5), the microphone path measured first (read-only,
   under Windows Mobile).
6. **S6 Bluetooth outputs**, with the peripherals design.

## S2 as built (2026-10-19)

Engineering inside R2-R4, settled by tests and the emulator; nothing here
changes a vote or a condition.

- **The mix is a function of output time** (the clever part, nothing given
  up): the resampler keeps no history (a stream's source position for any
  output frame is exact rational arithmetic from its anchor), volume ramps
  are functions of time, and the chain's whole state (limiter, speaker
  filter, dither generator: 42 bytes) is kept at every rewrite point (every
  128 frames, 2.9 ms). So a rewrite gives, bit for bit, what mixing once
  would have (a host test compares the two with the limiter working).
  PulseAudio's rewind bookkeeping isn't needed.
- **Rewrites** start at the next 128-frame point at least 5 ms past the DMA
  position, and each 32-frame block goes to the DMA engine as soon as it is
  mixed (nearest first), so the margin covers one block's mixing however
  long the stretch.
- **How far ahead** (R2's "while every stream's data is there"): the mixer
  mixes the mode's lead (245 ms background, 35 ms interactive) but never
  past where a playing stream's data ends, except that it always mixes the
  short lead, so one program that stops writing runs dry alone.
- **When it wakes:** at the mode's interval (15 / 200 ms), timed from the
  DMA position after mixing, and at the latest at a **low-water mark** (the
  lead less one interval: ~45 ms of sound left with music, ~20 ms with
  clicks): a program gets its room at a wake, so it has that long to refill
  (a decoder's work). Found by the emulator: waking only as the mix ran out
  left a program ~5 ms.
- **Interactive (condition 13):** the low-latency hint (System and Voice by
  default), or the write pattern: three writes in a row, once playing, of
  at most 50 ms each, within 100 ms of each other, **leaving at most
  150 ms queued**. The queue is what tells a game from a music player that
  tops up its ring in small pieces (without it the music test woke 33
  times a second). The short interval holds for 1 s after the last
  interactive write, in the mixer, so a click's own short stream closing
  doesn't end it.
- **Rings are bytes** (16-bit little-endian samples): the shared region
  between two processes will be bytes, no alignment assumed. Sizes by
  condition 4's rule: 50.9 KiB at 44.1 kHz stereo (a 32 ms write), 12.8 KiB
  at 22.05 kHz mono, 55.7 KiB at 48 kHz stereo; a smaller ring is refused
  with the size it needs.
- **The chain (condition 2's order):** per-stream gain with 5.8 ms ramps,
  sum in 32 bits, the speaker's filter (second-order high-pass at 300 Hz,
  ours: below the 440 Hz the owner already finds weak on this speaker; A/B
  by ear in the device run), the limiter (block peak, attack within the
  0.7 ms block, a 20 ms hold so the gain doesn't ripple inside a low note,
  100 ms release, aiming 0.5 dB under full scale once an overload set it
  off, the hard clip only as the backstop), then TPDF dither only where the
  mix isn't already a 16-bit value: a lone 44.1 kHz stream at full volume
  is bit-exact (A1) and silence stays silent. **Ducking** (R4 step 3,
  optional) is not built; it waits for S3's volumes.
- **The resampler (A1):** one Kaiser-windowed sinc table (10 zero
  crossings, 64 points each, computed at compile time), Smith's method, the
  filter stretched for 48 kHz. 11.025 and 22.05 kHz land exactly on table
  points. Measured on the host: THD+N under -80 dB for every rate, images
  under -65 dB, 19 kHz from 48 kHz content at -3.6 dB, 23 kHz folding to
  -26 dB. On the device (2026-10-19, the cycle counter): 174 cycles a
  frame at 44.1 kHz, 431 from 22.05 / 11.025 kHz, 669 from 48 kHz
  (1.2-4.7% of the 624 MHz CPU), after a scalar rework the same night that
  stays bit-exact (strided table points for whole-number steps, a
  compile-time 48 kHz table of its 148 phases, 6.5 KB). iWMMXt is still
  open (quality audit E1).
- **Measured on the device, for A1:** the link runs at **44,317 Hz**, not
  44,100 (+0.49%, ~8.5 cents sharp), and **no `SADIV` setting is exact**:
  48 kHz's gives 47,795 (-0.43%), 22.05 kHz's 21,959 (-0.41%)
  (docs/hw/audio.md). Windows Mobile has the same offset. **Options, the
  owner's to choose:** (a) keep 44.1 kHz as it is (as Windows Mobile; a
  player syncing video to the audio clock doesn't drift; music 8.5 cents
  sharp and 0.5% short, below what most notice); (b) keep the 44.1 kHz
  setting but treat the link as 44,317 Hz and convert every stream to it
  (pitch right; no bit-exact 44.1 kHz; the converter's cost for all music,
  ~430-670 cycles a frame measured); (c) 48 kHz (A1-B: 7.4 cents flat
  instead, 44.1 kHz content converted). Nothing changes until the owner
  says.
- **Memory:** the mixer itself 2.3 KB; its memory block 48.3 KiB (the DMA
  ring 44 KiB and the kept states 4.3 KiB), given by its owner. The image
  `rp-audio-s2` is 70.1 KiB.

## A6. When the mixer is late by more than the ring (quality audit E3)

**The owner's decision (2026-10-19): B, conditional on the two device
measurements below meeting their pass criteria.** If either misses, the
numbers come back to the owner with the options; nothing is decided by a
result alone. The owner's review the same day added five points, folded in
below: what an empty I2S FIFO might send, the end marker's rule in one
sentence, a pass criterion for the restart time, the "mixed or cleared"
rule, and the build order against S2. **Built 2026-10-19** in that
order: see "A6 as built" at the end of this section.

### In plain words

The DMA engine plays the mixed sound from a ring 254 ms long, round and
round, without the CPU. The mixer keeps it filled ahead and clears what
has played. If the mixer is late by **less** than the ring, the engine
reaches the cleared part and you hear **silence** until the mixer catches
up (built that way, S2). If it is late by **more** than the ring, the
engine goes round again and **replays the last quarter-second of music,
over and over**, until the mixer wakes: a loud stutter, the classic sound
of a hung computer. The mixer can count the lost laps afterwards, but it
can't stop the replay.

The fix: make the chain of descriptors **end where the mixed sound ends**,
so the engine **stops by itself** instead of going round. Each time the
mixer mixes further, it moves the end further. Late by any amount, you
hear the music, then silence, then the music carrying on from where it
stopped. Never a replay.

When could the mixer be that late? On the device so far only in test
images, where one program both mixes and draws (S2 found the console
taking up to 98 ms to scroll). In the audio host (R1) the mixer has its own
thread, so a quarter-second stall means something is wrong: a runaway
higher-priority thread, a kernel bug, a debugger stop. This choice is
about **how the device fails** when that happens: quietly or loudly.

### How the engine works (PXA27x manual, public; no Dell value)

- Each descriptor (16 bytes: next, source, target, command) is **loaded
  into the channel's registers when the engine starts it** and never read
  again.
- Bit 0 of the "next" word is **STOP**: after that descriptor, the engine
  stops and says so in `DCSR` (`STOPINTR`). I2S keeps running; its FIFO
  simply empties.
- `hal/pxa27x/src/dmachain.rs` already streams SD and CF transfers this
  way (a moving STOP, a restart when the CPU falls behind), proven on the
  device (docs/research/storage-speed.md) and modelled in the fork
  (rp-v0.9.6). What is **not** proven yet is the same restart on the I2S
  channel while I2S runs (see "What must be measured first").

### The options

| | A: keep the loop (as built) | **B: a chain that stops at the mix's end (recommended)** | C: a chain that falls into a loop of silence |
|---|---|---|---|
| How | 11 descriptors of 4 KiB, the last linked to the first | The ring stays (same 44 KiB, same place); the descriptor holding the **first frame not yet mixed** carries STOP. Each wake the mixer writes STOP on the new end's descriptor, then clears the old one (two 16-byte writes, each written back from the cache) | As B, but the end descriptor links to a tiny descriptor of zeros that links to itself. The engine never stops; when the mixer catches up it re-links the zeros back into the ring |
| Late by less than the ring | Silence, then the music carries on | Same | Same |
| **Late by more than the ring** | **The last ~0.25 s replayed until the mixer wakes** | **Silence**, then the music carries on | **Silence**, then the music carries on |
| Nothing playing | The engine loops the silent ring; the mixer must keep waking at least once a ring, or old sound comes back | **The engine stops by itself after the last sound: no wakes at all while quiet** | The engine loops the zeros (a descriptor fetch every 8 frames) |
| The output clock | From the DMA position, plus a guess at lost laps from the OS timer (a heuristic: a wrong guess skips a whole ring) | **From the position alone, exactly**: the engine can't lap the mix, so no lap can be lost; the clock stands still while the engine is stopped (it counts frames that really went out) | The position leaves the ring while in the zeros: time there is a guess from the timer again |
| The race (below) | None | A short gap of silence, only on a wake that comes with under 7.9 ms of sound left | Same race, and getting out of the zeros needs a bounded wait while the engine leaves them |
| Cost per wake | Nothing | Two descriptor writes and two cache-line cleans, read `DCSR` and `DDADR`: well under a microsecond (measured when built) | Same as B, plus the exit wait when used |
| Memory | 44 KiB ring, 11 of the channel's 32 descriptor slots | **Same 44 KiB ring, all 32 slots** (already reserved in `dma_map`: no budget change) | As B, plus a small zero buffer |
| Unknown on the device | Nothing new | Restarting the I2S DMA channel while I2S runs | Re-linking a running self-loop |

**Why B.** It fails quietly, the engine itself enforces the end (no
software has to be on time for it to hold), the clock becomes exact and
loses its lap-guessing heuristic, and a quiet mixer can sleep as long as it
likes. C avoids the restart but brings the timer guess back and needs a
wait to leave the zeros. A costs nothing now and is fine while nothing
stalls; its failure is the loud one.

### The details of B

- **Smaller descriptors: 32 of 1,408 bytes** (352 frames, 7.9 ms) instead
  of 11 of 4 KiB. Two reasons, both measured from the mixer's own numbers:
  1. **The end must never be the descriptor the engine is in.** Otherwise
     the engine would finish this lap, start that descriptor again with
     STOP set, and play the stale tail of it before stopping. **The end
     marker's rule (the invariant the race argument and the mutants rest
     on): the STOP sits on the descriptor holding the first frame not yet
     mixed, and that descriptor ends no further than one ring past the
     start of the descriptor the engine is reading; so the mix may lead the
     engine's read position by at most the ring less one descriptor
     (*lead ≤ ring − one descriptor*: 11,264 − 352 = 10,912 frames).**
     Built, the rule turned out to need the strict form (*lead < ring −
     one descriptor*), measured from the piece holding the last frame the
     engine played: an end one whole ring on is the piece just behind a
     stopped engine, which can't be told from an end it has already played
     (found by the HAL's host test; "A6 as built").
     The background lead is 10,816 frames
     and the ring 11,264, leaving 448 frames: a 4 KiB descriptor (1,024
     frames) breaks the rule; 352 frames keeps it with room to spare.
  2. **The race gets rare** (next point).
  1,408 is a whole number of cache lines (44 × 32 bytes) and of frames,
  under the length field's limit, and 32 × 1,408 is exactly the 44 KiB
  ring.
- **The race.** The engine loads a descriptor when it starts it. If, at a
  wake, it has already started the end descriptor, clearing that STOP is
  too late: it will stop at that descriptor's end even though the mixer
  has just mixed past it. The mixer sees this straight after moving the
  end (`DDADR` still carries STOP while the mix goes further, or `DCSR`
  says stopped) and **restarts the channel at the next descriptor** as soon
  as it stops (the channel's stop as an event to the audio host; until the
  host exists, a bounded wait of at most one descriptor). What you'd hear:
  a gap of silence of about the restart time. **When it can happen:** only
  when a wake comes with under 7.9 ms of sound left. The mixer wakes with
  ~20 ms left with clicks playing and ~45 ms with music alone, so the race
  needs a wake 12 ms late (clicks) or 37 ms late (music): already the
  nearly-late case. Counted (`restarts`), never hidden.
- **Every frame the engine plays is mixed for its time, or cleared (zero)**
  (the rule the host model checks, not "mixed" alone: after the mix's end
  the engine plays cleared frames up to the end of the STOP descriptor).
  The mixer guarantees it: **after every wake's mixing it clears from the
  mix's end to the end of the STOP descriptor** (at most 351 frames, 1.4
  KiB, usually already zero). Found while writing this down: a rewrite can
  end *short* of the old mix (a click arriving during music shrinks the
  lead from 245 ms to 35 ms), leaving the old mix beyond the new end; today
  that tail would play in a stall, and under B without the clear the
  engine would play up to 7.9 ms of it before stopping. Frames past the
  STOP descriptor need no clearing: the engine can't reach them before the
  next wake re-mixes or clears them.
- **What I2S sends while its FIFO is empty is not known**: zeros, or the
  last sample held (the manual doesn't say). It matters at a stop:
  - **A stall or the end of sound:** the last frames before the stop are
    cleared ones (the rule above; the end is on the descriptor holding the
    first *unmixed* frame), so either way the gap is silence with no step
    into it. Coming out of it, the music restarts mid-waveform: a possible
    click, the same as today's late path.
  - **The race:** the engine stops at the end of a descriptor the mixer has
    just mixed into, so the last sample is music. With zeros the gap is a
    hard cut to silence and back (a click at each end); with the last
    sample held it is a held DC level, a step at each end (also a click
    each). **Either way the race can click**; the device test listens for
    exactly that (below).
- **After a stop** the mixer's existing "late" path does the rest: every
  playing stream carries on after the gap from where it stopped, and `late`
  counts it. That path is S2's, **already built and passed** (`tick`'s
  "too late" branch; host test
  `a_late_wake_plays_silence_never_an_old_lap_and_nothing_is_lost`; on the
  device the S2 image's six console-stall wakes went through it). B leans
  on it, not the other way round; the build order below makes the stop
  reach it through a test before anything else changes. Starting again
  after silence needs no margin: the engine is stopped, so the mixer mixes
  the first block and then starts it.
- **The interface:** the mixer tells the DMA ring where the mix ends
  (`DmaRing` gains "end at byte N" and "stopped?"); the HAL owns the
  descriptors, as it does now. The emulator and host fakes implement the
  same.
- **What goes away:** the lap estimate from the OS timer in
  `advance_clock`, and `laps_lost` (replaced by `stops` and `restarts`).
  A1's rate measurement (`clock_now`) reports any stop inside its window, so
  a window with a stop isn't used.

### What must be measured first (device, before B is built on)

Each its own short bench test, owner watching, only when the owner says go.
**Written 2026-10-19: `rp-tone` built with `RP_TONE_DMA_RESTART`** (both
measurements in one run, 44.1 kHz, no codec, no I2C, no board bit: a
transfer one chain plus eight descriptors long stops at the chain's STOP
while I2S runs; each of 0, 1, 10, 100 and 1,000 ms stopped, three tries;
each line says whether it met the criteria). **Passes in the emulator**
(`emu-dma-restart`, fork rp-v0.28.0, where the PXA's link clock drains I2S
with the codec off as the X51v does: every stop at the chain's end, every restart moving and
finishing; the emulator's timings say nothing about the device's).
**Run on the X51v (2026-10-19 evening): both pass, 15 of 15 tries**
(docs/hw/audio.md). (1) Every restart moved and finished after every stop
length (0-1,000 ms) with I2S on throughout; `TUR` (`SASR0` bit 5) sets when
the FIFO runs dry and stays set, and doesn't block the restart, so nothing
needs clearing. (2) The restart moved **8 us** after it once the FIFO had
emptied; restarted at once, 168-172 us, which is the FIFO still playing its
last frames down to its threshold (no gap). Both under 0.5 ms. **The
owner's condition for B is met**; building waits for the owner's go. The
forced race's click (what an empty FIFO sends) is heard in the listening
run after B is built.

1. **Restarting the I2S DMA channel while I2S keeps running**: the channel
   stopped at a STOP link, left stopped 0, 10, 100 and 1,000 ms, restarted;
   the DMA timed to its first move each time, `SACR0` and `SASR0` read
   before and after (does an empty FIFO set an error that needs clearing?).
   The 645 ms rule is about switching I2S on; this test is the proof that it
   doesn't reach a DMA-only restart (2026-10-18's stalls all followed an
   I2S switch-off).
   **Passes if** every restart, after every stop length, moves within
   criterion 2's time, and `SACR0` still reads I2S on afterwards (with any
   `SASR0` error bit cleared by a documented write, not a reset of I2S).
2. **How long a restart takes**, from the channel's stop to the DMA moving
   again (the race's gap, less the mixer's own reaction, which the audio
   host's event sets later). **Passes if** under **0.5 ms** on every try:
   below the 5 ms margin the mixer already keeps, and a gap short enough
   that the click (above) is the only thing heard, not a dropout. The
   device listening run then checks the whole race by ear: **the forced
   race heard as at most a single click, no audible gap**, its logged gap
   (stop to move, the mixer's wait included) under **1 ms**.

If (1) fails, B isn't possible as written and the write-up comes back with
the measurement (C, or A). If (2) misses, the numbers come back with the
options (larger descriptors are no answer: the end rule needs them small;
the stop as an interrupt straight to a restart is the likely one).

### Build order (after both measurements pass)

1. **The end marker in the HAL**, pure logic beside `dmachain`, host-tested
   with a model engine and mutants; the ring re-cut into 32 descriptors of
   1,408 bytes (still looping: nothing heard changes).
2. **A stop reaches S2's late path**: a host test in which the model engine
   stops at the end marker and the existing late-path test's checks hold
   (silence, never an old lap, nothing lost). Only then the mixer moves the
   end each wake, clears to the STOP descriptor's end, and handles the race.
3. **The lap estimate removed** from `advance_clock` (it is the last thing
   to go, once a stop is the only way the engine can fall behind).
4. `emu-audio-e3`, then the owner's listening run.

### Tests, emulator, size

- **Host:** the moving end as pure logic beside `dmachain` (which
  descriptor carries STOP for a given mix end; the race told apart from a
  stop from `DCSR` and `DDADR`), a model engine with random wake timing
  checking *every frame played was mixed for its time or cleared*, and
  mutants (the STOP set after clearing the old one instead of before; the
  rule *lead ≤ ring − descriptor* dropped; the clear to the STOP
  descriptor's end dropped, caught by a shortening rewrite followed by a
  stall).
- **Emulator:** the fork's DMA already stops at STOP. A new
  `emu-audio-e3`: the image stalls the mixer 400 ms mid-music; the codec
  model's recording must show the music, silence, and the music carrying on,
  never a repeat. Whether the fork's I2S model handles an empty FIFO is
  checked then; if it doesn't, it is modelled first (docs/hw/emulator.md).
- **Device (owner listening):** the same 400 ms stall, heard as a clean
  gap, not a stutter (listening for a click going in, which there should
  not be, and coming out, which is possible); a forced late wake to
  exercise the race, listening for clicks at both ends of its gap (what an
  empty FIFO sends decides whether they're there), its gap logged against
  the 1 ms criterion.
- **Size:** no memory added; code a few hundred bytes (measured when
  built: 3.7 KB in `rp-audio-s2`, 83,780 to 87,524 bytes with the ring's
  logic, the mixer's end, its counters and log lines; "A6 as built").

### A6 as built (2026-10-19)

Built in the build order above, each step committed on its own; nothing
heard on the device yet. Engineering inside B, settled by host tests,
mutants and the emulator; no vote changes.

- **The HAL** (`hal/pxa27x/src/dmaring.rs`, beside `dmachain`): the ring
  is cut into as many pieces as the channel's chain holds, whole cache
  lines (the mixer's 44 KiB: 32 of 1,408 bytes; an 8 KiB ring, as S1's
  and the link-rate bench's: 32 of 256), and loops until given an end.
  `DmaRing` gained `piece`, `stopped` and `end_at`. Moving the end: STOP
  on the new piece's link first, then off the old one (a CPU held up
  between the two writes then still has one STOP in the chain; the other
  order left none and the engine went round: the host model catches it).
  The race is told from `DDADR` (it carries STOP and names the piece after
  the one the engine is in), waited for (at most a piece) and restarted at
  once. An end in the engine's own piece is a failure, counted, and held
  one piece on (at most a piece of what's there plays, never a lap).
  `position()` now reads the ring's end as its start (it was clamped one
  frame short: a stop at the ring's last piece would have put the clock
  one frame off).
- **Device fact the HAL leans on, not yet proven:** `DDADR` keeps the
  link's address bits with STOP set while the engine runs a STOP piece
  (the manual's register layout; the fork does the same). The bench
  proved the stop and restart, not this reading: the listening run's
  forced races prove it (a wrong reading shows as `failed`).
- **The mixer:** after each wake's mixing it clears the rest of the
  piece its mix ends in (audit E4) and moves the end there. After a stop
  every stream carries on **exactly where the engine stopped** (the end of
  the end's piece), not past the margin: the engine is stopped, so no
  margin is needed, and mixing from there also writes over any old mix
  past it. Found while building:
  1. **The ring gets an end the moment it starts** (silence up to the
     first piece past the margin). A ring looping even for its first
     wake let an engine faster than that wake's mixing lap the first mix
     and replay it (host test with the service starting the ring for a
     first stream).
  2. **The end moves on during a long mix, but only to a stopped
     engine.** A race leaves under a piece (7.9 ms) of sound, while mixing
     a wake's 245 ms lead can take ~9 ms on the X51v (48 kHz content). So
     each fully mixed piece checks whether the engine has stopped (one
     register read) and, if so, moves the end to it, starting the engine
     again within about two pieces' mixing (under 1 ms on the device).
     Moving it under a *playing* engine every piece (the first version)
     found it in the old end's piece again and again after any restart
     and waited for it each time: the mixer paced by the engine, its lead
     never rebuilt, 63-99% of the emulator's CPU in busy waits (found by
     `emu-audio-e3`; host test: 58,316 races in 4 s that way, 4 now).
  3. **The mix is capped by the end's rule** and mixed on until the end's
     piece starts at least the margin past the engine (only on a
     nearly-late wake).
- **The clock** is the DMA position alone; the OS-timer lap estimate and
  `laps_lost` are gone. It counts only frames that went out, so it stands
  still while the engine is stopped; the S2 image's result lines say how
  many stops a window held (such a window's rate isn't the link's).
- **Counters:** `stops`, `restarts`, `races`, `ring_failed` (never
  expected, never hidden), in the S2 image's log.
- **Host tests:** the HAL's model engine with random wake timing, a CPU
  held up between the two link writes and stalls longer than the ring:
  every frame played is the one written for its time, or silence. The
  mixer's: a stop reaches S2's late path; a stall longer than the ring is
  silence, never a replay (E3); a shortening rewrite then a stall plays no
  stale tail (E4); a wake inside the end's piece (the race); a larger cut;
  a mixer slower than the sound fails quietly; the race on a 28% CPU
  costs about two pieces' mixing; a restart doesn't tie the mixer to the
  engine. **13 mutants**, all killed (6 HAL, 7 mixer).
- **Emulator (`emu-audio-e3`, `rp-audio-s2` built with `RP_AUDIO_E3`):**
  A, a rising scale (a semitone every 100 ms) with the mixer stalled
  400 ms after 1 s: one late wake, the engine stopped 162 ms, then the
  scale carrying on, all 24 notes rising, never a step back. B, five
  wakes held back until the engine is inside the end's piece: five races,
  the engine stopped 0-77 us each (criterion 1 ms), all 24 notes, no gap.
  The fork's I2S sends nothing while its FIFO is empty, so the recording
  closes the stall's gap up; the guest's log holds it instead
  (docs/hw/emulator.md). Mixer CPU 0.5-1.6%.
- **On the X51v (2026-10-19 night, the owner's listening run; docs/hw/
  audio.md):** A: one late wake, the engine stopped 325 ms and restarted,
  nothing failed; B: five races, each recognised from `DDADR` (now a proven
  device fact) and restarted with **0 us** stopped (criterion 1 ms). The
  owner's verdict by ear (a clean gap in A? clicks in B?) is pending: B is
  not passed until the owner says so. **Run 2 (speaker only), the owner:
  A "a little gap", B "no gap"**; the log shows the stream running dry
  once after A's gap and counted interactive after the stall, and one
  late wake in B from the test's forced wait (docs/hw/audio.md).
- **Open (quality audit C17):** `emu-audio-s2` reported one ring end that
  failed in 2 of its first 6 runs after B was built (none in the next 4),
  in an ordinary wake; not understood yet. The ring now logs why and what
  it read; contained meanwhile (at most a piece, never a lap).
- **For the owner's listening run** (`RP_AUDIO_E3=1`): A heard as the
  scale, a clean gap of about 0.16 s, the scale carrying on (a click
  going in shouldn't be there; one coming out is possible); B as the
  scale with at most a click at each of five moments, no audible gap; the
  log's worst race gap under 1 ms. What an empty FIFO sends (zeros or the
  last sample) shows in whether B clicks.

### The step questions

- **Clever or re-implementing?** Neither Windows Mobile nor Dell's driver
  is the model here. Linux (ALSA) catches an underrun in the sound card's
  interrupt and stops the stream there, so it needs that interrupt to come
  on time; here the engine enforces the end itself, with no interrupt per
  period: correctness that doesn't depend on any software being on time.
- **Smaller?** No memory added; the lap heuristic removed.
- **Modernized?** A stall fails quietly, as a modern system should.
- **Security:** a program that starves the audio thread gets silence, not
  a loud replay; nothing new is exposed to programs (the descriptors stay
  in the audio host).

## A7. Virtual bass on the speaker (owner's vote 2026-10-19: B)

**Voted B (owner, 2026-10-19); nothing is built yet.** It needs the
speaker sweep and the owner's blind A/B by ear before it ships.

### In plain words

The X51v's speaker is tiny: below a few hundred hertz it barely moves air,
and pushing bass into it only makes it rattle, which is why the speaker
output already cuts bass (A4's speaker protection). Music then sounds thin.
**Virtual bass** uses a trick of hearing: when you hear the overtones of a
low note (twice, three times, four times its frequency), your brain fills
in the low note itself, even if it isn't there (the "missing
fundamental"). Small speakers in phones and laptops use it: the bass
the speaker can't play is replaced by overtones it can, so music sounds
fuller without the rattle. It only applies to the speaker; headphones get
the real bass.

### How it would work (in the mixer's output stage, speaker only)

1. Split the mix: the band the speaker can't play (below its cut-off,
   **not yet measured**: a few hundred hertz) goes to the bass path; the
   rest passes untouched.
2. The bass path makes overtones: a smooth nonlinear curve (a low-order
   polynomial; a rectifier is the cruder form) on the low band, then a
   filter keeping only the overtones the speaker can play (roughly 1-4 x
   the cut-off), at a level that follows the bass's own loudness.
3. The overtones are added back, and the speaker protection (A4) removes
   what's left below the cut-off, as today. Then the limiter, as today.

Cost, estimated: a few filters and a polynomial on one mono bass signal at
44.1 kHz, about **50-80 CPU cycles a sample: under 1% of the CPU** at 624
MHz (measured before shipping, host-tested for quality first).

### The options

| | **A: none (today)** | **B: overtone virtual bass (recommended)** | C: pitch-shifted bass (vocoder) | D: just boost the bass |
|---|---|---|---|---|
| How | The speaker protection cuts the bass the speaker can't play | Overtones of the missing bass added, as above | The bass band shifted up an octave by a phase vocoder | More bass sent to the speaker |
| Sounds | Thin, but clean | Fuller; the low notes' pitch "heard" | Can sound smeared or phasey | Rattle and distortion: the speaker can't move that air |
| CPU | none | <1% (estimated) | several % (FFTs) | none (codec) |
| Risk | none | Overtones too strong sound fuzzy: tuned by ear, switchable | Quality hard to get right | Damages the sound, maybe the speaker |

**Recommendation: B**, on the speaker output only, with a switch in Settings
(on by default once the owner's A/B approves it), tuned by ear on the X51v.
C costs much more for a doubtful gain; D is what the protection exists to
prevent.

### What must be measured first (device, owner's go, speaker only)

1. **The speaker's cut-off:** a slow sine sweep (60 Hz to 2 kHz) on the
   speaker at a fixed level, the owner noting where it becomes clearly
   audible and where it starts to rattle (a phone's sound meter app can
   add numbers). This sets the crossover and the protection filter alike.
2. **The A/B:** the same music clips with B off and on (and two strengths),
   in a random order the image logs, the owner picking which sounds better
   without knowing which is which.

### Build order (after the vote and the sweep)

1. The bass path in the host mixer, host-tested (a 100 Hz tone in: 200,
   300, 400 Hz out at the designed levels; nothing below the cut-off; no
   overload at full scale; CPU cycles counted on the device).
2. The device A/B run (above), the owner's ears deciding.
3. The Settings switch, the emulator check (the mixer's output on the
   emulated speaker carries the overtones), size and CPU budgets.

## The four lenses

| Lens | Effect |
|---|---|
| 🔁 Iterate | Each step a device run; quality, latency and wake-ups are numbers in tests, so a later mixer or resampler is judged against them |
| ✨ Modernize | Rewrite-ahead scheduling, per-output and per-kind volumes, pause on unplug, a limiter and dither, microphone permission and indicator, Bluetooth outputs by design |
| 🗄️ Legacy support | `waveOut`/`PlaySound`/`WAV1:` with the timing old games expect; X50Mix's private codes answered; old rates converted exactly (A1-A) |
| ⚡ Better / faster | Long DMA ring: few CPU wake-ups with music; new sounds within milliseconds; volume and tone in the codec at no CPU cost |

## Hardware limit or choice?

| Kept from Windows Mobile | Silicon or choice? |
|---|---|
| I2S on for a whole session | **Silicon**: the 645 ms ignored switch-on (measured) |
| Audio up while the device is awake, down on suspend | Choice, kept **as the baseline to beat**: idle power-down between sounds only if S1's battery measurement justifies it (R5) |
| 44.1 kHz (if A1-A) | Choice, made on content (see A1), not imitation |
| Codec I2C gate GPIO82 around each transfer; I2C pins low when powered down | Board wiring (gate: shared line, measured); pins low: a good practice for an unpowered chip, kept |
| Dell's 70 ms amplifier settle, 75 ms after codec reset, power-up waits | **Copied values**: open questions in the imitation audit, measured on the device in S1 |
| Move to the speaker on unplug | Choice: replaced if A2-A |

## Alternatives considered

- **A short ring with frequent wake-ups** (Android's normal mixer, ~20 ms
  periods): simpler, but ~50 wake-ups a second with the screen off costs
  battery; R2 gives the same latency with a tenth of the wake-ups.
- **Each program on its own DMA channel, mixed in hardware**: the PXA's
  I2S has one transmit FIFO; there's no hardware mixer.
- **A graph engine (PipeWire-like)**: more than one codec and a few outputs
  need; its good idea (shared memory, one period) is kept in R3.
- **Callbacks into programs at the mixer's time (Core Audio)**: precise,
  but runs program code inside the audio host; RetroPane gives programs
  rings and events instead (R3, R8). Sample-accurate starts for games are
  still possible: a stream may carry a start time.

## Consequences

- The CE personality's sound calls and the Music app both sit on one
  service; Bluetooth headsets later are another `Output`.
- The power service gets a "streaming, margin N µs" signal from audio
  (ADR-0031 D3, D5).
- Device runs ahead (owner listening): S1's power sequence and pops,
  S2's latency and battery, S5's microphone path.
- Copied Dell values (the waits) are counted in the imitation audit until
  measured.
