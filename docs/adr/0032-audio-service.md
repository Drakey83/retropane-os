# ADR-0032: The audio service: one mixer, one owner of the codec

- **Status:** **Accepted (owner, 2026-10-18): A1 A, A2 A, A3 B with the
  hearing warning, A4 B, A5 A**, and R1-R10 with the owner's thirteen
  conditions, all landing before S1 (listed under "The owner's
  conditions" below; written into R2-R7, A2-A5 and the steps). Proposed
  and revised after a review the same day.
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
  late) and C (the codec on whenever the screen is on: 4-12% more drain). Because of the 645 ms
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
3. **S3 volumes and routing** (A2, A3, A4).
4. **S4 the CE personality's sound calls** (R7) with an old game and X50Mix,
   including the `WOM_DONE` latency comparison with Windows Mobile.
5. **S5 recording** (A5), the microphone path measured first (read-only,
   under Windows Mobile).
6. **S6 Bluetooth outputs**, with the peripherals design.

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
