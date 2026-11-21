//! The module runner, as a test image (ADR-0039; `cargo xtask emu-xash`).
//!
//! **A test harness, not how the OS starts a game.** A port runs as Game
//! Mode's one program under the resident kernel (ADR-0039 rule 1,
//! docs/design/game-mode.md); until the kernel is what the X51v runs, the
//! same runner code is tried the way every part of RetroPane is tried
//! today, as an image the loader starts. This image is never shipped as a
//! way to start a game.
//!
//! What it is: the port layer (`rp_port`: the card's files, the clock,
//! sound, input), the C library (picolibc, once, here: rule 6) and the
//! module loader. Nothing of any game is linked into it. It loads the
//! program module from its folder on the card ([`FOLDER`], [`PROGRAM`]),
//! runs its constructors and its entry, and the program loads its own
//! library modules through `rp_module_load`.
//!
//! Iterate, don't imitate (rule 0). The Vita and Switch ports of the
//! engine this was first built for are a main program with a small ELF
//! loader in it and a hand-kept list of the C library functions the
//! libraries may call; Linux has the system's dynamic linker. Here the
//! build finds what the modules import, links a table of exactly those
//! addresses into this image (which keeps them in it), and links each
//! module against them at the address it runs at: this image hashes that
//! table at its start, a module carries the hash of the table it was
//! linked against, and one that does not match is refused. Loading is a
//! read, a check sum and a zeroing; a call from a module into the C
//! library is one branch.
//!
//! Memory (boot/link.x, `rp_hal_x51v::devices`): this image below
//! 0xA80F0000; its zeroed RAM and a 2 MiB stack in the 3 MiB after it;
//! the modules from the RAM pool's start (0xA9000000) where the build
//! placed them; the C heap is what the set of modules leaves of the
//! pool's 48 MiB (each module's header says where the set ends).
//!
//! On the host, and in builds without the C toolchain, there is no
//! program here (the `c` feature).

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// The runner's folder on the card: its module files are here (the base
/// image's flash once RetroPane is installed: ADR-0039 rule 8). The card
/// is shared: nothing is written outside it by the runner itself.
pub const FOLDER: &str = "rp-xash";
/// The program module the test image starts.
pub const PROGRAM: &str = "engine.rpm";
/// Printed first: what this image is.
pub const BANNER: &str = "RetroPane: the module runner (a test image: ADR-0039)";

#[cfg(target_os = "none")]
mod cpu;
#[cfg(target_os = "none")]
mod gfx;
#[cfg(target_os = "none")]
mod panel;

#[cfg(target_os = "none")]
mod image {
    use core::cell::RefCell;
    use core::fmt::Write;

    use rp_audio_x51v::{BoardCpld, SharedGpio, found, name, put_back, service};
    use rp_drv_pxa_gpio::{Gpio, GpioPin};
    use rp_drv_pxa_keypad::{Keypad, Pressed};
    use rp_drv_pxa_ssp::Ssp;
    use rp_drv_tsc2046::Tsc2046;
    use rp_drv_wm8750::Output;

    use rp_drv_cpld::Cpld;
    use rp_drv_pxa_mmc::PxaMmc;
    use rp_drv_pxa_uart::PxaUart;
    use rp_drv_sdcard::{SdCard, mbr_partitions};
    use rp_fat::AnyVolume;
    use rp_hal_api::{BlockDevice, BlockError, RegisterBus};
    use rp_hal_cabi::{Break, Sys};
    use rp_hal_pxa27x::dma::Dma;
    use rp_hal_pxa27x::{Mmio, rt};
    use rp_hal_x51v::devices;
    use rp_port::files::Files;
    use rp_port::input::{Input, Pad};
    use rp_port::module::{self, Modules};
    use rp_port::names::Names;
    use rp_port::progress::{Lines, Said, clock, number, said};
    use rp_port::recent::Recent;
    use rp_port::sound::{Sound, memory_for};
    use rp_port::sys::{Console, Library, Looks, System};
    use rp_touch_cal::{Calibration, Sample};

    use super::panel;
    use super::{BANNER, FOLDER, PROGRAM};

    rp_image::entry!(main);

    /// The touch chip's select and pen lines (docs/hw/touch.md).
    const TS_CS: u8 = 24;
    const TS_PENIRQ: u8 = 94;
    /// The codec's output level, 0-100: full (0 dB), the game's own volume
    /// setting is the player's control (Half-Life's sound was "cant hardly
    /// be heard" at 60, then -29 dB: the owner, 2026-11-16).
    const LEVEL: u32 = 100;
    /// The sound's memory is sized for the largest stream a game opens
    /// here: 44.1 kHz stereo.
    const SOUND_FOR: (u32, u8) = (44_100, 2);
    /// Of the scratch RAM: the file system's sector work.
    const FAT_WORK: usize = 64 * 1024;
    /// Of the scratch RAM: the names of the folders the program looks
    /// into (`rp_port::names`), so a look-up costs no card read (R276).
    /// Half-Life's whole `valve` folder, 4,262 files in 83 folders, is
    /// 192 KiB of names (32 bytes each and 56 KiB of their text); what a
    /// run used is in its log.
    const NAMES_ROOM: usize = 320 * 1024;
    /// Files a program may have open at once.
    const HANDLES: usize = 16;
    /// Of the scratch RAM: the program-counter samples of a run asked
    /// for with [`SAMPLE_WORD`] (`rt::start_sampling`): 64 Ki samples, a
    /// minute at [`SAMPLE_HZ`].
    const SAMPLE_ROOM: usize = 512 * 1024;
    /// Samples a second.
    const SAMPLE_HZ: u32 = 1000;
    /// R346: free play keeps one sample in this many (a 15-minute play's
    /// ~37,500 of [`SAMPLE_ROOM`]'s 65,536), and every sample of a frame
    /// this long or longer (the hitches, run 279: 259 frames over 50 ms in
    /// 11.5 min, ~15,000 samples at 1,000 a second).
    const SAMPLE_KEEP: u32 = 24;
    const SAMPLE_SLOW_US: u32 = 50_000;
    /// The screen's pump's tick when not sampling (kit 69).
    const PUMP_HZ: u32 = 1000;
    /// A test run's word for sampling where the program is ([`SAMPLE_ROOM`])
    /// from its start, written at its end to [`SAMPLE_FILE`] in the
    /// runner's folder (R285: where the X51v's time goes, which no
    /// emulator can say).
    const SAMPLE_WORD: &[u8] = b"-rpsample";

    /// A test run's word for sampling on a performance event instead of the
    /// clock (`rt::start_event_sampling`): `-rpsampleon <event>`, e.g.
    /// `-rpsampleon 2`, a sample every 200,000 cycles stalled on memory
    /// (R324: the frames are bus-bound); an event that counts occurrences,
    /// not cycles (a miss), every 2,000 of them (one number: the command
    /// line holds 31 words). Implies [`SAMPLE_WORD`].
    const SAMPLE_ON_WORD: &[u8] = b"-rpsampleon";
    /// The samples' file: `RPSAMP1\0`, then little-endian words: samples
    /// a second, samples, ticks that found the room full, the sample at
    /// each of the run's steps 1 to 5 (`u32::MAX`: not reached), then each
    /// sample's two words (where, and the return address).
    const SAMPLE_FILE: &str = "samples.bin";
    /// A test run's word for timing the float routines before the program
    /// starts: the compiler's generic ones (this image's) against ours
    /// (`rp_hal_mem::float`, what the modules link), on numbers of a
    /// game's range, every result compared (R282).
    const FLOAT_WORD: &[u8] = b"-rpfloat";
    /// With [`FLOAT_WORD`], how many times 2^20 vectors of each kind the
    /// fused dot product's check takes (R316: the emulator's long proof;
    /// one when not said).
    const DOT_WORD: &[u8] = b"-rpdot";
    /// A test run's word for write-allocate on the game modules' memory
    /// (R306 item 2): [`WALLOC_FROM`]..[`WALLOC_TO`], the server's,
    /// client's and engine's code, data and tables (kit 53's ELFs: the
    /// server's text at 0xA8C00048, the engine's bss ending 0xA99F6284).
    /// Off by default: run 53 (sound on in every run) measured it ~8%
    /// slower than read-allocate at the same load; run 52's "13% faster"
    /// was a run whose sound never started (R301).
    const WALLOC_WORD: &[u8] = b"-rpwalloc";
    const WALLOC_FROM: u32 = 0xA8C0_0000;
    /// A test run's word for step 4f's probe (`gfx::Probe`,
    /// docs/design/ref-rp.md "Step 4f's probe before the build"), with the
    /// seconds of frames before it: the game's frames fed by DMA, paced
    /// then unpaced, each against the CPU's feed of the same words. Its
    /// cache-missing walk takes `gfx::WALK_BYTES` from the end of
    /// [`SAMPLE_ROOM`] (the samples then cover a little less of the run),
    /// and its DMA channel is the CompactFlash reads' (the runner reads
    /// no CF card; `dma_map`'s chains are at their budget).
    const PROBE_WORD: &[u8] = b"-rp4f";
    /// Step 4f (`gfx::Chain`, docs/design/ref-rp.md "Step 4f"): each frame
    /// fed to the TA by one DMA chain while the CPU makes the next, the
    /// default since run 56 (the fixed-step walk: 6.6% less time than the
    /// CPU's feed, the same frames; no hang in runs 53-56). This word turns
    /// it off: the CPU's feed, the control. Its table and descriptors take
    /// `gfx::CHAIN_BYTES` of the scratch RAM, its DMA channel is the USB
    /// controller's (the runner has no USB device).
    const NO_CHAIN_WORD: &[u8] = b"-rpnochain";
    /// Kit 63: the first [`CHAIN_CHECK_FRAMES`] frames fed by the CPU and
    /// then by the chain, both pictures hashed and compared (the chain's
    /// own clear and scene start against the CPU's).
    const CHAIN_CHECK_WORD: &[u8] = b"-rpchaincheck";
    /// Kit 113: the TA after each render, as before (the TA and the 3D
    /// stage at work on two frames at once is the default).
    const NO_OVERLAP_WORD: &[u8] = b"-rpnooverlap";
    const CHAIN_CHECK_FRAMES: u32 = 32;
    const WALLOC_TO: u32 = 0xA9A0_0000;
    /// The program's command line as built in: its name alone. What a
    /// run asks of the program is in [`ARGS_FILE`], so a run on the X51v
    /// can be quiet (its run log holds 64 KiB) and the emulator's check
    /// can ask for everything (`cargo xtask emu-xash` writes the engine's
    /// `-dev 2` and the platform layer's `-rpmemlist -rpshot` there).
    const ARGS: [&core::ffi::CStr; 1] = [c"xash3d"];
    /// More of the command line, if this file is in the runner's folder:
    /// words with spaces between (`+map c1a0`). How a test run says what
    /// the program should do without the image being built again.
    const ARGS_FILE: &str = "args.txt";
    /// The most of that file that is read.
    const ARGS_ROOM: usize = 512;
    static ARGS_TEXT: rt::Slot<[u8; ARGS_ROOM]> = rt::Slot::new();

    /// A test run's two words for ending by itself, which the program's
    /// platform layer acts on (ports/xash/platform/in_rp.c): seconds of
    /// its step 4 (what the run measures), and seconds on the clock at
    /// the latest. The runner only says them on the screen.
    const END_WORD: &[u8] = b"-rpend";
    const LIMIT_WORD: &[u8] = b"-rplimit";

    /// R341 (the owner, 2026-11-20: "it drops down to like half brightness
    /// and it makes the dark parts of the game hard to see"): the
    /// backlight's level for the program's time, in percent of the
    /// perceptual scale (`rp_drv_pxa_pwm::Backlight`), `-rplight <0-100>`;
    /// without it [`LIGHT_DEFAULT`].
    const LIGHT_WORD: &[u8] = b"-rplight";
    /// The backlight while a game plays: full duty, the most the X51v's
    /// software can give (docs/hw/backlight.md: no LED-current control).
    /// Windows Mobile leaves whatever its power settings last chose (Dell's
    /// table tops out at 137 / 216 on the owner's panel type, and dims on
    /// battery), so a game played at a leftover level; ours is the
    /// program's own choice, the found level put back at its end.
    const LIGHT_DEFAULT: u32 = 100;

    /// R341: PWM0 as found (`CR`, `PCR`, `DCR`), put back at the end (a
    /// program that ends by `exit` or a crash leaves it to the loader,
    /// which puts the whole board back after every image: audit C39).
    /// R341: the backlight to `level` percent for the program's time (a
    /// quarter-second fade), with its registers as found: the screen's
    /// close puts them back (`gfx::Chip::with_light`; R343's looks set
    /// its level while the game plays).
    fn light_up(level: u32) -> Option<super::gfx::Light> {
        use rp_drv_pxa_pwm::reg;
        let Some(mut pwm) = rt::take_pwm0() else {
            let _ = writeln!(
                Out,
                "runner: R341: the backlight not taken (PWM0 already taken)"
            );
            return None;
        };
        let found = [reg::CR, reg::PCR, reg::DCR].map(|r| pwm.read32(r));
        let mut b = rp_drv_pxa_pwm::Backlight::new(pwm);
        let was = b.level();
        b.fade_to(level, &mut |us| rt::delay_us(us));
        let (on, counts) = b.duty();
        let _ = writeln!(
            Out,
            "runner: R341: the backlight {level}% for the program ({on} of {counts} counts); found {was}% (PWM0 CR 0x{:x} PCR 0x{:x} DCR 0x{:x}), put back at the screen's close",
            found[0], found[1], found[2]
        );
        Some((b, found))
    }

    /// A test run's word for the 2700G's clock while the program draws:
    /// four numbers, the core PLL's M, N and P (`13 MHz x M / (N x 2^P)`)
    /// and the cores' divider, as ADR-0035's ladder writes them
    /// (`clock::Speed::ladder`: the memory timing rated for that clock,
    /// CL2). The screen's open steps to it and its close puts Windows
    /// Mobile's back; a rung past the owner's game step is not taken
    /// (`gfx::rung_allowed`).
    const CLOCK_WORD: &[u8] = b"-rpclock";

    /// A test run's word for R295's diagnostic ladder (`gfx::CELLS`,
    /// docs/research/sheets/r295-ta-path-136.md): the 2700G's clock and
    /// the TA path's layout and render words cell by cell, each render
    /// waited for and counted.
    const LADDER_WORD: &[u8] = b"-rpladder";

    /// A test run's word for C2 (`cpu`): the core's turbo multiplier past
    /// 624 MHz while the program has the screen, `-rpcpu 7` (728 MHz),
    /// `-rpcpu 8` (832) or `-rpcpu 157` (L 15 x 2N 7, 682.5 MHz, the bus
    /// and memory 195 MHz: `cpu::asked`), every frame and card write checked.
    const CPU_WORD: &[u8] = b"-rpcpu";

    /// ADR-0040 step 7g6: the SDRAM's automatic power-down is off while
    /// the program runs (Windows Mobile leaves it on: the SDRAM's clock
    /// dropped between accesses, so a miss after a gap waits for it to
    /// wake; runs 139-141: the cascade 27.5 / 17.6 -> 28.1 / 18.6 fps, the
    /// fight 18.4 / 20.7 -> 19.5 / 21.8); put back as found at the end.
    /// A test run's `-rpapd` keeps Windows Mobile's setting (the control).
    const APD_WORD: &[u8] = b"-rpapd";

    /// A test run's word for the frame-rate bar along the screen's bottom
    /// (`gfx::BAR_ROWS`): frames a second, the 2700G's clock, the hangs.
    const BAR_WORD: &[u8] = b"-rpfpsbar";

    /// The rung after the last [`CLOCK_WORD`] in `args` (an experiment's
    /// words come after the kit's default), if all four numbers are there
    /// and in the PLL's fields.
    fn clock_rung(args: &[&core::ffi::CStr]) -> Option<rp_drv_mbx_display::clock::Rung> {
        let at = args.iter().rposition(|a| a.to_bytes() == CLOCK_WORD)?;
        let mut n = [0u32; 4];
        for (k, v) in n.iter_mut().enumerate() {
            *v = match number(args.get(at + 1 + k)?.to_bytes())? {
                (v, []) => v,
                _ => return None,
            };
        }
        let [m, nn, p, div] = n;
        if !(1..64).contains(&m) || !(1..8).contains(&nn) || p > 7 || !(1..=4).contains(&div) {
            return None;
        }
        Some(rp_drv_mbx_display::clock::Rung {
            speed: rp_drv_mbx_display::clock::Speed::ladder(m << 7 | nn << 4 | p << 1 | 1),
            div,
        })
    }

    /// The whole number of seconds that follows `word` in `args`.
    fn seconds_after(args: &[&core::ffi::CStr], word: &[u8]) -> Option<u32> {
        let at = args.iter().position(|a| a.to_bytes() == word)?;
        match number(args.get(at + 1)?.to_bytes())? {
            (seconds, []) => Some(seconds),
            _ => None,
        }
    }

    /// The words of `text` (NUL where its spaces and line ends were, by
    /// [`words`]) added to `args` after its first `have`; how many `args`
    /// holds then. `None` if there are more words than it has room for:
    /// a run with part of its command line is not the run asked for.
    fn add_words(
        text: &'static [u8],
        args: &mut [&'static core::ffi::CStr; rp_hal_cabi::ARGS],
        mut have: usize,
    ) -> Option<usize> {
        let mut at = 0;
        while at < text.len() {
            match core::ffi::CStr::from_bytes_until_nul(&text[at..]) {
                Ok(word) => {
                    if !word.is_empty() {
                        *args.get_mut(have)? = word;
                        have += 1;
                    }
                    at += word.count_bytes() + 1;
                }
                Err(_) => break,
            }
        }
        Some(have)
    }

    type Sd = SdCard<PxaMmc<Mmio, Dma>>;

    /// The card, checking in with the run limit at every transfer.
    struct Card(Sd);

    /// Single sectors the file system read last, kept (`rp_port::recent`):
    /// a folder's sector is read once for its sixteen entries, not once
    /// each (R281). Its hits are not card commands and are not counted.
    const RECENT: usize = 16;

    /// The card as the file system sees it.
    type Disk = Recent<Card, RECENT>;

    impl BlockDevice for Card {
        fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), BlockError> {
            // The screen's clock keeps going while files are read.
            let start = rt::ticks();
            alive(start, true);
            let got = self.0.read_blocks(lba, buf);
            // How long the card took, beside what was asked of it: a
            // start's time on the X51v splits into the card's and the
            // program's (R281).
            let took = rt::ticks().wrapping_sub(start);
            CARD.with(|c| {
                c.reads = c.reads.wrapping_add(1);
                c.sectors = c.sectors.wrapping_add((buf.len() / 512) as u32);
                c.busy = c.busy.wrapping_add(u64::from(took));
            });
            got
        }
        fn write_blocks(&mut self, lba: u32, buf: &[u8]) -> Result<(), BlockError> {
            alive(rt::ticks(), true);
            // C2: a core past 624 MHz writes to the shared card only while
            // its known answer is right this moment (`cpu`).
            if !super::cpu::may_write() {
                return Err(BlockError::Device);
            }
            self.0.write_blocks(lba, buf)
        }
    }

    /// What the card was asked for, and what the program's look-ups of
    /// paths cost of it: a run's log says where a slow start's card
    /// reads went (R276: run 1 on the X51v read the card for twenty
    /// minutes before its first frame).
    #[derive(Clone, Copy)]
    struct CardCount {
        /// Read commands, the sectors they asked for, and the timer's
        /// ticks they took.
        reads: u32,
        sectors: u32,
        busy: u64,
        /// Paths looked up and found, and the read commands that took.
        found: u32,
        found_reads: u32,
        /// Paths looked up that were not there, and their read commands.
        missing: u32,
        missing_reads: u32,
        /// Whether each look-up is a line of the log ([`LOOKS_WORD`]).
        each: bool,
        /// File reads, the bytes they asked, and their read commands.
        file_reads: u32,
        file_bytes: u64,
        file_cmds: u32,
        /// Whether each file read is a line of the log ([`READS_WORD`]).
        each_read: bool,
        /// Whether each file opened once the game plays is a line of the
        /// log ([`PLAY_OPENS_WORD`]), and whether it plays (step 4 on).
        play_opens: bool,
        playing: bool,
        /// Folder entries listed, and their read commands.
        listed: u32,
        listed_cmds: u32,
    }

    static CARD: rt::Global<CardCount> = rt::Global::new(CardCount {
        reads: 0,
        sectors: 0,
        busy: 0,
        found: 0,
        found_reads: 0,
        missing: 0,
        missing_reads: 0,
        each: false,
        file_reads: 0,
        file_bytes: 0,
        file_cmds: 0,
        each_read: false,
        play_opens: false,
        playing: false,
        listed: 0,
        listed_cmds: 0,
    });

    /// A test run's word for a line in the log for every path looked up
    /// (`look: 12 - gfx/shell/x.bmp`: its read commands, `+` found or `-`
    /// not there). Thousands of lines: for the emulator's serial line,
    /// not the X51v's 64 KiB run log.
    const LOOKS_WORD: &[u8] = b"-rplooks";

    /// A test run's word for a line in the log for every file opened
    /// (`open: 5 /Ports/valve/halflife.wad`) and every file read (`read:
    /// 5 1048576 1024 3`: its handle, where, the bytes asked, its read
    /// commands). Tens of thousands of lines: the emulator's only (R281).
    const READS_WORD: &[u8] = b"-rpreads";

    /// A test run's word for a line in the log for every file opened once
    /// the game plays (step 4 on): `open in play: 5 sound/x.wav at 41234
    /// ms` (the owner, 2026-11-19: "find what's loading from the card
    /// mid-fight"; R335's run 265 read 372 KiB in 32 reads during the
    /// fight's second half). Tens of lines, so the X51v's log holds them.
    const PLAY_OPENS_WORD: &[u8] = b"-rpplayopens";

    fn opened(fd: i32, path: &[u8]) {
        if CARD.with(|c| c.play_opens && c.playing) {
            let _ = writeln!(
                Out,
                "open in play: {fd} {} at {} ms",
                core::str::from_utf8(path).unwrap_or("?"),
                rt::ticks() / TICKS_1MS
            );
        }
        if CARD.with(|c| c.each_read) {
            let _ = writeln!(
                Out,
                "open: {fd} {}",
                core::str::from_utf8(path).unwrap_or("?")
            );
        }
    }

    fn listed(fd: i32, more: bool, reads: u32) {
        let each = CARD.with(|c| {
            c.listed = c.listed.wrapping_add(u32::from(more));
            c.listed_cmds = c.listed_cmds.wrapping_add(reads);
            c.each_read
        });
        if each {
            let _ = writeln!(Out, "list: {fd} {} {reads}", u8::from(more));
        }
    }

    fn read(fd: i32, at: u64, asked: usize, _got: usize, reads: u32) {
        let (each, in_play) = CARD.with(|c| {
            c.file_reads = c.file_reads.wrapping_add(1);
            c.file_bytes = c.file_bytes.wrapping_add(asked as u64);
            c.file_cmds = c.file_cmds.wrapping_add(reads);
            (c.each_read, c.play_opens && c.playing && reads > 0)
        });
        // -rpplayopens: a read that went to the card once the game plays
        // (run 268 opened nothing in the fight's second half, yet run 265
        // read 372 KiB then: from files already open)
        if in_play {
            let _ = writeln!(
                Out,
                "read in play: {fd} at {at} {asked} bytes, {reads} card commands, at {} ms",
                rt::ticks() / TICKS_1MS
            );
        }
        if each {
            let _ = writeln!(Out, "read: {fd} {at} {asked} {reads}");
        }
    }

    fn card_reads() -> u32 {
        CARD.with(|c| c.reads)
    }

    fn looked(path: &[u8], found: bool, reads: u32) {
        let each = CARD.with(|c| {
            if found {
                c.found += 1;
                c.found_reads = c.found_reads.wrapping_add(reads);
            } else {
                c.missing += 1;
                c.missing_reads = c.missing_reads.wrapping_add(reads);
            }
            c.each
        });
        if each {
            let mark = if found { '+' } else { '-' };
            let _ = writeln!(
                Out,
                "look: {reads} {mark} {}",
                core::str::from_utf8(path).unwrap_or("?")
            );
        }
    }

    /// The card's count so far, as a line of the log (`when`: which
    /// moment of the run).
    fn card_count(when: core::fmt::Arguments<'_>) {
        let c = CARD.with(|c| *c);
        let _ = writeln!(
            Out,
            "runner: card reads {when}: {} commands for {} sectors in {} ms; {} paths found took {} of them, {} paths not there took {}; {} file reads of {} KiB took {}; {} folder entries listed took {}",
            c.reads,
            c.sectors,
            c.busy / u64::from(TICKS_1MS),
            c.found,
            c.found_reads,
            c.missing,
            c.missing_reads,
            c.file_reads,
            c.file_bytes / 1024,
            c.file_cmds,
            c.listed,
            c.listed_cmds
        );
    }

    /// Where the runner's own lines and all the program prints go: the
    /// run log when the loader started this image (the X51v: the loader
    /// sends it to the PC, and every line goes onto a card as it ends,
    /// `rp_hal_x51v::cardsink`), the serial line when nothing did (the
    /// emulator booting this image directly, where there is no run log).
    /// Never both: the X51v has no connector on that serial line, and a
    /// byte sent down it is 87 us of waiting once its buffer is full, in
    /// the middle of the frame being timed.
    fn say(bytes: &[u8]) {
        if devices::loader_present() {
            devices::log_write(bytes);
            return;
        }
        let mut uart = PxaUart::new(rt::emergency_uart());
        for &b in bytes {
            if b == b'\n' {
                uart.write_byte(b'\r');
            }
            uart.write_byte(b);
        }
    }

    /// [`say`] as a writer, for a line with numbers in it.
    #[derive(Clone, Copy)]
    struct Out;

    impl Write for Out {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            say(s.as_bytes());
            Ok(())
        }
    }

    /// A C program's console: what it prints goes where the runner's own
    /// lines go.
    struct Text;

    impl Console for Text {
        fn put(&mut self, bytes: &[u8]) {
            say(bytes);
            // A program that prints is not hung: the run limit hears of
            // it as it does of a clock read (at most ten times a second).
            let _ = ticks();
            // What the program says of its own progress goes on the
            // screen (`rp_port::progress`), and a step's time into the log.
            SAID.with(|lines| {
                lines.feed(bytes, &mut |line| match said(line) {
                    Some(Said::Step { n, of, text }) => step(n, of, text),
                    Some(Said::Note(text)) => panel::note(text),
                    None => {}
                });
            });
        }
    }

    /// The program's output as lines, for [`Text`].
    static SAID: rt::Global<Lines<LINE>> = rt::Global::new(Lines::new());
    /// The most of a line that is looked at.
    const LINE: usize = 200;
    /// The steps of a run: the runner's two (its own start, the program's
    /// start) and the three a program under it says (`rp-step: 3/5` to
    /// `5/5`: it is up, it does what the run is for, it ends).
    const STEPS: u32 = 5;

    /// Step `n` of `of` on the screen, and when it began in the log (the
    /// run's timings are read from the log, never asked of who watches).
    /// The sample at each step ([`SAMPLE_FILE`]).
    static SAMPLE_AT: rt::Global<[u32; 5]> = rt::Global::new([u32::MAX; 5]);
    /// The event sampler's totals at the last step (`-rpsampleon`).
    static EVENTS_AT: rt::Global<[u64; 5]> = rt::Global::new([0; 5]);

    fn step(n: u32, of: u32, text: &[u8]) {
        if let Some(i) = (n as usize).checked_sub(1).filter(|&i| i < 5) {
            SAMPLE_AT.with(|a| a[i] = rt::samples_taken());
        }

        // -rpsampleon: the bus waits since the last step, per mille of its
        // cycles (R324: what the frames wait on).
        let t = rt::event_totals();
        if t[0] != 0 {
            let was = EVENTS_AT.with(|a| core::mem::replace(a, t));
            let d: [u64; 5] = core::array::from_fn(|k| t[k] - was[k]);
            let pm = |v: u64| v * 1000 / d[0].max(1);
            let _ = writeln!(
                Out,
                "runner: before step {n}: {} cycles; the sampled event {} per mille, buffers full {}, I-cache {} per mille; {} data-cache misses ({} a thousand cycles)",
                d[0],
                pm(d[1]),
                pm(d[2]),
                pm(d[3]),
                d[4],
                pm(d[4])
            );
        }
        panel::step(n, of, text);
        let at = panel::beat(rt::ticks());
        let _ = writeln!(
            Out,
            "runner: step {n} of {of} at {at} s on the screen's clock"
        );
        card_count(format_args!("at step {n}"));
        if n >= 4 {
            CARD.with(|c| c.playing = true);
        }
    }

    /// Pairs the float routines are timed on.
    const FLOAT_PAIRS: usize = 1024;

    /// The routines timed, called as the compiler calls its own: not
    /// inlined into the timing loop. Ours in Rust (the reference the PC
    /// checks against its own IEEE hardware) and in ARM assembly (what the
    /// game's modules link), and the compiler's generic ones (this image's
    /// `compiler_builtins`).
    #[inline(never)]
    fn rust_add(a: u32, b: u32) -> u32 {
        rp_hal_mem::float::add(a, b)
    }
    #[inline(never)]
    fn rust_sub(a: u32, b: u32) -> u32 {
        rp_hal_mem::float::sub(a, b)
    }
    #[inline(never)]
    fn rust_mul(a: u32, b: u32) -> u32 {
        rp_hal_mem::float::mul(a, b)
    }
    #[inline(never)]
    fn generic_add(a: u32, b: u32) -> u32 {
        (f32::from_bits(a) + f32::from_bits(b)).to_bits()
    }
    #[inline(never)]
    fn generic_sub(a: u32, b: u32) -> u32 {
        (f32::from_bits(a) - f32::from_bits(b)).to_bits()
    }
    #[inline(never)]
    fn generic_mul(a: u32, b: u32) -> u32 {
        (f32::from_bits(a) * f32::from_bits(b)).to_bits()
    }
    #[inline(never)]
    fn generic_div(a: u32, b: u32) -> u32 {
        (f32::from_bits(a) / f32::from_bits(b)).to_bits()
    }
    #[inline(never)]
    fn rust_div(a: u32, b: u32) -> u32 {
        rp_hal_mem::float::div(a, b)
    }
    #[inline(never)]
    fn nothing(a: u32, b: u32) -> u32 {
        a ^ b
    }

    /// Tenths of a core clock cycle for one call of `f`, the loop's own
    /// cost taken off: the least of five passes over the pairs.
    fn cycles_per_call(pairs: &[(u32, u32)], f: fn(u32, u32) -> u32) -> u32 {
        let pass = |f: fn(u32, u32) -> u32| {
            let t = rt::cycles();
            let mut sink = 0u32;
            for &(a, b) in pairs {
                sink ^= f(core::hint::black_box(a), core::hint::black_box(b));
            }
            core::hint::black_box(sink);
            rt::cycles().wrapping_sub(t)
        };
        let best = |f| (0..5).map(|_| pass(f)).min().unwrap_or(0);
        let base = best(nothing);
        best(f).saturating_sub(base) * 10 / pairs.len() as u32
    }

    /// Values at every edge (zeros, the subnormals' and normals' ends,
    /// one and its neighbours, infinities, NaNs), as the PC's tests.
    const FLOAT_EDGES: [u32; 20] = [
        0,
        1,
        2,
        3,
        0x007F_FFFF,
        0x0080_0000,
        0x0080_0001,
        0x00FF_FFFF,
        0x3F7F_FFFF,
        0x3F80_0000,
        0x3F80_0001,
        0x3FFF_FFFF,
        0x4B00_0000,
        0x4B7F_FFFF,
        0x7F7F_FFFE,
        0x7F7F_FFFF,
        0x7F80_0000,
        0x7F80_0001,
        0x7FC0_0000,
        0x7FFF_FFFF,
    ];

    /// The float routines timed and compared ([`FLOAT_WORD`]), said in
    /// the run's log and on the screen.
    fn float_bench() {
        panel::note(b"Timing the float|routines: the|generic and ours.");
        rt::cycle_counter_start();
        let mut seed = 0x9E37_79B9_u32;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        // A game's range: 1e-5 to 1e4 or so, both signs.
        let mut pairs = [(0u32, 0u32); FLOAT_PAIRS];
        for p in &mut pairs {
            let mut one = || (next() & 0x807F_FFFF) | ((110 + next() % 31) << 23);
            *p = (one(), one());
        }
        type Op = fn(u32, u32) -> u32;
        let ops: [(&str, Op, Op, Op); 4] = [
            ("add", generic_add, rust_add, rp_hal_mem::asm::add),
            ("sub", generic_sub, rust_sub, rp_hal_mem::asm::sub),
            ("mul", generic_mul, rust_mul, rp_hal_mem::asm::mul),
            ("div", generic_div, rust_div, rp_hal_mem::asm::div),
        ];
        for (name, generic, rust, asm) in ops {
            let g = cycles_per_call(&pairs, generic);
            let r = cycles_per_call(&pairs, rust);
            let a = cycles_per_call(&pairs, asm);
            // Every result of both of ours against the generic one: the
            // game's range, every edge with every edge in both signs, near
            // numbers (cancellation, carries, ties) and bits of any kind.
            let nan = |v: u32| v & 0x7FFF_FFFF > 0x7F80_0000;
            let mut checked = 0u32;
            let mut differ = 0u32;
            let mut first = None;
            let mut check = |x: u32, y: u32| {
                let want = generic(x, y);
                for got in [rust(x, y), asm(x, y)] {
                    checked += 1;
                    if got != want && !(nan(got) && nan(want)) {
                        differ += 1;
                        first.get_or_insert((x, y, got, want));
                    }
                }
            };
            for &(x, y) in &pairs {
                check(x, y);
            }
            for &x in &FLOAT_EDGES {
                for &y in &FLOAT_EDGES {
                    for (sx, sy) in [(0, 0), (1 << 31, 0), (0, 1 << 31), (1 << 31, 1 << 31)] {
                        check(x | sx, y | sy);
                    }
                }
            }
            for _ in 0..32_768 {
                let x = next();
                let e = (x >> 23) & 0xFF;
                let eb = e.saturating_sub(next() % 28).min(254);
                let y = (next() & 0x8000_0000) | (eb << 23) | ((x ^ (next() & 0xFF)) & 0x007F_FFFF);
                check(x, y);
                check(next(), next());
            }
            let _ = writeln!(
                Out,
                "float: {name}: the generic {}.{} cycles, ours in Rust {}.{}, in assembly {}.{}; {checked} results compared, {differ} differ",
                g / 10,
                g % 10,
                r / 10,
                r % 10,
                a / 10,
                a % 10
            );
            if let Some((x, y, got, want)) = first {
                let _ = writeln!(
                    Out,
                    "float: {name}: the first that differs: {x:#010x}, {y:#010x} gave {got:#010x}, the generic {want:#010x}"
                );
            }
        }
    }

    /// The fused dot product (`rp_hal_mem::asm::dot3`, R316) against C's
    /// expression made of the compiler's generic routines, on `rounds`
    /// times 2^20 vectors of each of six kinds, every result compared; and
    /// timed against the generic routines' five calls and the assembly
    /// routines' five calls. Said in the run's log.
    fn dot_bench(rounds: u32) {
        panel::note(b"Checking the fused|dot product against|the generic floats.");
        let mut seed = 0x6A09_E667_u32;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        let generic = |a: &[u32; 3], b: &[u32; 3]| {
            generic_add(
                generic_add(generic_mul(a[0], b[0]), generic_mul(a[1], b[1])),
                generic_mul(a[2], b[2]),
            )
        };
        let calls = |a: &[u32; 3], b: &[u32; 3]| {
            use rp_hal_mem::asm::{add, mul};
            add(add(mul(a[0], b[0]), mul(a[1], b[1])), mul(a[2], b[2]))
        };
        let nan = |v: u32| v & 0x7FFF_FFFF > 0x7F80_0000;
        // A game's range (1e-5 to 1e4 or so, both signs).
        let game = |r: u32, e: u32| (r & 0x807F_FFFF) | ((110 + e % 31) << 23);
        // Kinds: 0 the game's range; 1 the same with zeros (axial
        // vectors); 2 the second product near minus the first and the third
        // near minus their sum (cancellations, exact zeros); 3 any normal
        // exponent (overflows and underflows on the way); 4 any bits; 5
        // the edges in any order and sign.
        let mut checked = 0u64;
        let mut differ = 0u64;
        let mut first = None;
        for kind in 0..6u32 {
            for _ in 0..rounds.saturating_mul(1 << 20) {
                let mut a = [0u32; 3];
                let mut b = [0u32; 3];
                for i in 0..3 {
                    let (x, y) = match kind {
                        0 | 2 => (game(next(), next()), game(next(), next())),
                        1 => {
                            let z = next();
                            let x = if z & 3 == 0 {
                                z & 0x8000_0000
                            } else {
                                game(next(), next())
                            };
                            let y = if z & 12 == 0 {
                                (z << 1) & 0x8000_0000
                            } else {
                                game(next(), next())
                            };
                            (x, y)
                        }
                        3 => (
                            (next() & 0x807F_FFFF) | ((1 + next() % 254) << 23),
                            (next() & 0x807F_FFFF) | ((1 + next() % 254) << 23),
                        ),
                        4 => (next(), next()),
                        _ => (
                            FLOAT_EDGES[(next() % 20) as usize] | (next() & 0x8000_0000),
                            FLOAT_EDGES[(next() % 20) as usize] | (next() & 0x8000_0000),
                        ),
                    };
                    a[i] = x;
                    b[i] = y;
                }
                if kind == 2 {
                    // b[1] = -(a0 b0) / a1, a few low bits off; then b[2]
                    // likewise against the first two's sum.
                    let wobble = |v: u32, r: u32| v ^ (r & 0xF);
                    let p0 = generic_mul(a[0], b[0]);
                    b[1] = wobble(generic_div(p0 ^ 0x8000_0000, a[1]), next());
                    let s = generic_add(p0, generic_mul(a[1], b[1]));
                    if s & 0x7FFF_FFFF != 0 {
                        b[2] = wobble(generic_div(s ^ 0x8000_0000, a[2]), next() >> 8);
                    }
                }
                let want = generic(&a, &b);
                for got in [rp_hal_mem::asm::dot3(&a, &b), calls(&a, &b)] {
                    checked += 1;
                    if got != want && !(nan(got) && nan(want)) {
                        differ += 1;
                        first.get_or_insert((a, b, got, want));
                    }
                }
            }
        }
        // Timed on a game's vectors, a quarter of their parts zero.
        let mut vecs = [([0u32; 3], [0u32; 3]); FLOAT_PAIRS];
        for (a, b) in &mut vecs {
            for i in 0..3 {
                let z = next();
                a[i] = if z & 7 == 0 { 0 } else { game(next(), next()) };
                b[i] = if z & 0x38 == 0 {
                    0
                } else {
                    game(next(), next())
                };
            }
        }
        rt::cycle_counter_start();
        let time = |f: &dyn Fn(&[u32; 3], &[u32; 3]) -> u32| {
            (0..5)
                .map(|_| {
                    let t = rt::cycles();
                    let mut sink = 0u32;
                    for (a, b) in &vecs {
                        sink ^= f(core::hint::black_box(a), core::hint::black_box(b));
                    }
                    core::hint::black_box(sink);
                    rt::cycles().wrapping_sub(t)
                })
                .min()
                .unwrap_or(0)
                * 10
                / FLOAT_PAIRS as u32
        };
        let base = time(&|a, b| a[0] ^ b[2]);
        let g = time(&generic).saturating_sub(base);
        let c = time(&calls).saturating_sub(base);
        let d = time(&|a, b| rp_hal_mem::asm::dot3(a, b)).saturating_sub(base);
        let _ = writeln!(
            Out,
            "float: dot: the generic five calls {}.{} cycles, ours five calls {}.{}, ours fused {}.{}; {checked} results compared, {differ} differ",
            g / 10,
            g % 10,
            c / 10,
            c % 10,
            d / 10,
            d % 10
        );
        if let Some((a, b, got, want)) = first {
            let _ = writeln!(
                Out,
                "float: dot: the first that differs: {a:08x?} . {b:08x?} gave {got:#010x}, the generic {want:#010x}"
            );
        }
    }

    /// The compares (kit 70, `rp_hal_mem::asm::compares`, the modules'
    /// `__aeabi_fcmp*`) against the compiler's generic ones on every edge
    /// pair in every sign and `rounds` times 2^20 pairs of any bits and of
    /// near values; said in the run's log.
    fn cmp_bench(rounds: u32) {
        let mut seed = 0xBB67_AE85_u32;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        #[inline(never)]
        fn generic(a: u32, b: u32) -> [i32; 5] {
            let (x, y) = (f32::from_bits(a), f32::from_bits(b));
            [
                i32::from(x == y),
                i32::from(x < y),
                i32::from(x <= y),
                i32::from(x > y),
                i32::from(x >= y),
            ]
        }
        let mut checked = 0u64;
        let mut differ = 0u64;
        let mut first = None;
        let mut check = |a: u32, b: u32| {
            checked += 1;
            if rp_hal_mem::asm::compares(a, b) != generic(a, b) {
                differ += 1;
                first.get_or_insert((a, b));
            }
        };
        for &x in &FLOAT_EDGES {
            for &y in &FLOAT_EDGES {
                for (sx, sy) in [(0, 0), (1 << 31, 0), (0, 1 << 31), (1 << 31, 1 << 31)] {
                    check(x | sx, y | sy);
                }
            }
        }
        for _ in 0..rounds.saturating_mul(1 << 20) {
            let a = next();
            check(a, next());
            // near: the same, a neighbour, the sign flipped
            let k = next();
            let b = match k & 3 {
                0 => a,
                1 => a.wrapping_add(1),
                2 => a.wrapping_sub(1),
                _ => a ^ 0x8000_0000,
            };
            check(a, b);
        }
        let _ = writeln!(
            Out,
            "float: cmp: the generic and ours in assembly; {checked} results compared, {differ} differ"
        );
        if let Some((a, b)) = first {
            let _ = writeln!(
                Out,
                "float: cmp: the first that differs: {a:#010x}, {b:#010x}"
            );
        }
    }

    /// The game modules' integer division (`rp_hal_mem::asm::udiv`,
    /// `idiv`: R306 after run 51) against the compiler's own (this image
    /// is built without the `float` feature, so its `/` and `%` are
    /// `compiler_builtins`'): every quotient and remainder of random
    /// numbers of every length, every edge against every edge in both
    /// signs; and the cycles a call, both, on a game's mix of lengths.
    fn div_bench() {
        let mut seed = 0x2545_F491_u32;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        const EDGES: [u32; 16] = [
            0,
            1,
            2,
            3,
            7,
            255,
            256,
            0xFFFF,
            0x1_0000,
            0x7FFF_FFFE,
            0x7FFF_FFFF,
            0x8000_0000,
            0x8000_0001,
            0xFFFF_FFFE,
            0xFFFF_FFFF,
            0x1234_5678,
        ];
        let mut checked = 0u32;
        let mut differ = 0u32;
        let mut first = None;
        let mut check = |n: u32, d: u32| {
            if d == 0 {
                return;
            }
            checked += 1;
            let want = (n / d, n % d);
            let got = rp_hal_mem::asm::udiv(n, d);
            if got != want {
                differ += 1;
                first.get_or_insert((n, d, got.0, want.0, 'u'));
            }
            let (sn, sd) = (n as i32, d as i32);
            if !(sn == i32::MIN && sd == -1) {
                checked += 1;
                let want = (sn / sd, sn % sd);
                let got = rp_hal_mem::asm::idiv(sn, sd);
                if got != want {
                    differ += 1;
                    first.get_or_insert((n, d, got.0 as u32, want.0 as u32, 'i'));
                }
            }
        };
        for &n in &EDGES {
            for &d in &EDGES {
                check(n, d);
            }
        }
        for _ in 0..65_536 {
            let (a, b) = (next(), next());
            check(a >> (next() % 32), b >> (next() % 32));
            check(a, (b >> (next() % 32)) | 1);
        }
        let mut pairs = [(0u32, 0u32); FLOAT_PAIRS];
        for p in &mut pairs {
            *p = (next() >> (next() % 16), (next() >> (8 + next() % 16)) | 1);
        }
        #[inline(never)]
        fn generic(n: u32, d: u32) -> u32 {
            core::hint::black_box(n) / core::hint::black_box(d)
        }
        #[inline(never)]
        fn ours(n: u32, d: u32) -> u32 {
            rp_hal_mem::asm::udiv(n, d).0
        }
        let g = cycles_per_call(&pairs, generic);
        let a = cycles_per_call(&pairs, ours);
        let _ = writeln!(
            Out,
            "div: the generic {}.{} cycles, ours in assembly {}.{}; {checked} results compared, {differ} differ",
            g / 10,
            g % 10,
            a / 10,
            a % 10
        );
        if let Some((n, d, got, want, k)) = first {
            let _ = writeln!(
                Out,
                "div: the first that differs ({k}): {n:#010x} / {d:#010x} gave {got:#010x}, the generic {want:#010x}"
            );
        }
    }

    /// The samples onto the card ([`SAMPLE_FILE`]), and what they are in
    /// the run's log.
    fn write_samples(buf: &[u8], taken: u32, full: u32) {
        let at = SAMPLE_AT.with(|a| *a);
        // R346: RPSAMP2 adds the one-in-N and the slow frames' length
        // (`rt::sample_keep`; 0, 0: every sample, as RPSAMP1)
        let (keep, slow_us) = rt::sample_kept();
        let mut head = [0u8; 8 + 4 * 10];
        head[..8].copy_from_slice(b"RPSAMP2\0");
        for (k, w) in [SAMPLE_HZ, taken, full]
            .iter()
            .chain(at.iter())
            .chain([keep, slow_us].iter())
            .enumerate()
        {
            head[8 + 4 * k..12 + 4 * k].copy_from_slice(&w.to_le_bytes());
        }
        let body = &buf[..taken as usize * rt::SAMPLE_BYTES];
        let put = write_card(SAMPLE_FILE, &[&head, body]);
        let _ = writeln!(
            Out,
            "runner: {taken} samples ({full} more found the room full; one in {keep} kept but frames of {} ms or more whole), the steps at samples {:?}: {}",
            slow_us / 1000,
            at,
            put
        );
    }

    /// `parts` written one after another to `name` in the runner's folder
    /// on the card (made new): what the log says of it.
    fn write_card(name: &str, parts: &[&[u8]]) -> &'static str {
        // From the root: the program may have moved the current folder.
        let mut path = [0u8; 64];
        let len = 1 + FOLDER.len() + 1 + name.len();
        if len > path.len() {
            return "NOT written: the name is too long";
        }
        path[0] = b'/';
        path[1..=FOLDER.len()].copy_from_slice(FOLDER.as_bytes());
        path[1 + FOLDER.len()] = b'/';
        path[2 + FOLDER.len()..len].copy_from_slice(name.as_bytes());
        // Write only (1), made new.
        let how = 1 | rp_hal_cabi::flags::CREATE | rp_hal_cabi::flags::TRUNCATE;
        let put = rp_hal_cabi::with_system(|s| {
            let fd = s.open(&path[..len], how)?;
            let mut wrote = Ok(0);
            for p in parts {
                wrote = wrote.and_then(|_| s.write(fd, p));
            }
            let closed = s.close(fd);
            wrote.and(closed)
        });
        match put {
            Some(Ok(_)) => "written to the card",
            Some(Err(_)) => "NOT written to the card",
            None => "NOT written: no system",
        }
    }

    static SYSTEM: rt::Slot<System<'static, Disk, HANDLES, Text>> = rt::Slot::new();
    static SOUND: rt::Slot<Sound<'static, Dma>> = rt::Slot::new();
    /// The GPIO block: the audio service, the codec's gate and the touch
    /// chip's two lines share it for the whole run.
    static GPIO: rt::Slot<SharedGpio> = rt::Slot::new();
    static INPUT: rt::Slot<Input<X51vPad>> = rt::Slot::new();
    /// The 2700G for the program's 3D frames (`rp_gfx_*`).
    static CHIP: rt::Slot<super::gfx::Chip> = rt::Slot::new();

    /// A line of the 2700G's into the run's log.
    fn gfx_log(line: core::fmt::Arguments<'_>) {
        let _ = writeln!(Out, "{line}");
    }

    type Touch = Tsc2046<Ssp<Mmio>, GpioPin<'static, Mmio>, GpioPin<'static, Mmio>>;

    /// The X51v's buttons and touch chip, read when the program asks.
    struct X51vPad {
        keypad: Keypad<Mmio>,
        touch: Touch,
    }

    impl Pad for X51vPad {
        fn pressed(&mut self) -> Pressed {
            // R343's picker takes the Record button while it runs
            super::gfx::curve_key(self.keypad.scan().held)
        }
        fn pen(&mut self) -> Option<Sample> {
            self.touch.sample().map(|t| Sample {
                x: t.x,
                y: t.y,
                z1: t.z1,
            })
        }
    }

    /// The keypad scanning and the touch chip on SSP1, as `rp-hello` sets
    /// them up; `None` if a part is missing.
    fn pad(gpio: &'static SharedGpio) -> Option<X51vPad> {
        let (kp, ssp) = (rt::take_keypad()?, rt::take_ssp1()?);
        let mut keypad = Keypad::new(kp);
        keypad.enable();
        {
            // Windows Mobile leaves GPIO24 as SSP1's frame signal
            // (docs/hw/touch.md): a plain GPIO, or the chip is never
            // selected and every reading is 0.
            let mut g = gpio.borrow_mut();
            g.make_output(TS_CS, true);
            g.set_function(TS_CS, 0);
            g.make_input(TS_PENIRQ);
        }
        let mut ssp = Ssp::new(ssp);
        ssp.configure_spi(2_600_000);
        let mut touch = Tsc2046::new(
            ssp,
            GpioPin::new(gpio, TS_CS),
            GpioPin::new(gpio, TS_PENIRQ),
        );
        // One reading arms its pen line.
        let _ = touch.read(rp_drv_tsc2046::channel::X);
        Some(X51vPad { keypad, touch })
    }

    /// A C `exit()`: the program's own end, said on the serial line. A
    /// program that ends with an error has not made the runner fail: the
    /// loader gets the image back either way, and the log says which.
    fn exit(code: i32) -> ! {
        let _ = writeln!(Out, "\nrunner: the program called exit({code})");
        longest_quiet();
        panel::done(b"The program ended|itself. Back to the|loader.");
        rp_image::finish()
    }

    /// The SD card, as the loader starts it (power through the CPLD, then
    /// the card); `None` if there is none.
    fn open_card(cpld: &mut BoardCpld) -> Option<Sd> {
        let (mmc, dma) = (rt::take_mmc()?, rt::take_dma(rp_hal_pxa27x::dma_map::SD)?);
        cpld.set_sd_power(true);
        rt::delay_us(10_000);
        SdCard::try_init(PxaMmc::with_dma(mmc, dma)).ok()
    }

    /// The card's first partition (or the whole card), mounted.
    fn mount(mut sd: Sd) -> Option<AnyVolume<Disk>> {
        let mut block = [0u8; rp_drv_sdcard::BLOCK];
        sd.read(0, &mut block).ok()?;
        let start = mbr_partitions(&block)
            .and_then(|p| p[0])
            .map_or(0, |p| p.start);
        AnyVolume::mount(Recent::new(Card(sd)), start).ok()
    }

    /// The test image's own check of the library modules beside the
    /// program (a game's server and client libraries, which the program
    /// loads only once it has the game's files): each is loaded through
    /// the same loader, every name it exports is looked up and every
    /// function named, and it is let go again. Its constructors are not
    /// run. Not something a runner under Game Mode does.
    fn check_libraries(s: &mut System<'static, Disk, HANDLES, Text>) {
        let uart = &mut Out;
        // The names first: the listing holds a handle of its own.
        let mut found = [([0u8; 32], 0usize); 8];
        let mut count = 0;
        let Ok(dir) = s.dir_open(FOLDER.as_bytes()) else {
            return;
        };
        let mut name = [0u8; 64];
        while let Ok(Some((len, folder))) = s.dir_next(dir, &mut name) {
            let n = &name[..len];
            if !folder
                && len <= 32
                && count < found.len()
                && n.len() > 4
                && n[len - 4..].eq_ignore_ascii_case(b".rpm")
                && !n.eq_ignore_ascii_case(PROGRAM.as_bytes())
            {
                found[count].0[..len].copy_from_slice(n);
                found[count].1 = len;
                count += 1;
            }
        }
        let _ = s.close(dir);
        for (bytes, len) in &found[..count] {
            let name = &bytes[..*len];
            let shown = core::str::from_utf8(name).unwrap_or("?");
            let Ok(m) = s.module_load(name) else {
                let _ = writeln!(uart, "check: {shown}: FAILED to load");
                continue;
            };
            let checked = u32::try_from(m.handle).ok().and_then(|h| {
                s.modules()
                    .and_then(|l| l.names(h).map(|n| (n.len(), n.round_trips())))
            });
            let _ = s.module_free(m.handle);
            match checked {
                Some((names, true)) => {
                    let _ = writeln!(
                        uart,
                        "check: {shown}: {names} names, each found by name and named by address: ok"
                    );
                }
                _ => {
                    let _ = writeln!(uart, "check: {shown}: its names FAILED");
                }
            }
        }
    }

    /// When the loader's run limit was last checked in with (the timer's
    /// counter), and the longest stretch between two check-ins so far.
    static CHECKED_IN: rt::Global<(u32, u32)> = rt::Global::new((0, 0));
    /// A tenth of a second of the OS timer's 3.25 MHz.
    const CHECK_IN_TICKS: u32 = 325_000;
    /// The OS timer's ticks in a millisecond.
    const TICKS_1MS: u32 = 3_250;

    /// A sign of life from the program at `now` (the timer's counter): a
    /// card transfer (`every_time`), a clock read or a printed line (at
    /// most ten times a second). The loader's run limit and watchdog hear
    /// of it, the screen's clock is brought up to date, and the longest
    /// stretch without one is kept: the loader ends a run that gives none
    /// for its run limit (30 s), and how near a run came is in its log
    /// ([`longest_quiet`]).
    fn alive(now: u32, every_time: bool) {
        let due = CHECKED_IN.with(|(last, longest)| {
            let quiet = now.wrapping_sub(*last);
            if !every_time && quiet < CHECK_IN_TICKS {
                return false;
            }
            *longest = (*longest).max(quiet);
            *last = now;
            true
        });
        if due {
            rt::check_in();
            panel::beat(now);
        }
    }

    /// The longest stretch without a sign of life, said at the run's end.
    fn longest_quiet() {
        let ms = CHECKED_IN.with(|(_, longest)| *longest) / TICKS_1MS;
        let limit = rt::run_limit_ms().unwrap_or(0);
        let _ = writeln!(
            Out,
            "runner: the longest stretch without a sign of life (a card transfer, a clock read, a printed line) was {ms} ms; the loader's run limit is {limit} ms (0: none armed)"
        );
    }

    /// The program's clock source: the OS timer's counter. A program reads
    /// its clock in every frame, so this is also where a game that plays
    /// for minutes without touching the card checks in with the loader's
    /// run limit and watchdog (the card's transfers do it otherwise), at
    /// most ten times a second: a game that stops reading its clock is
    /// hung, and is ended by the limit as any image is.
    fn ticks() -> u32 {
        let now = rt::ticks();
        alive(now, false);
        now
    }

    /// Makes loaded bytes fit to run: the data cache's copy written to
    /// RAM and the instruction cache's old lines for those addresses
    /// dropped (a module loaded a second time lies where its first load
    /// was run from).
    fn fit(at: u32, len: u32) {
        // The SRAM's hot pages (ADR-0040 step 7b) back on the SDRAM first:
        // what was loaded over them went to the SRAM.
        sram_unmap();
        // Code locked in the I-cache ([`lock_icache`]) could hold the old
        // bytes of what is loaded now: every lock goes first.
        if LOCKED.with(|l| core::mem::replace(l, 0)) != 0 {
            rp_hal_pxa27x::cpu::unlock_icache();
            let _ = writeln!(
                Out,
                "runner: the I-cache's locked code unlocked: a module is loaded"
            );
        }
        rp_hal_pxa27x::cpu::clean_invalidate_range(at, len);
    }

    /// The I-cache lock list's file in the program's folder: the address
    /// and length (little-endian words) of each piece of the modules' code
    /// to lock, most stalls a byte first (`cargo xtask xash-kit` with
    /// `RETROPANE_XASH_LOCK_KIB`; R324).
    const LOCK_FILE: &str = "lock.bin";
    /// Ranges the lock list holds at most.
    const LOCK_RANGES: usize = 512;
    /// Code locked at most, and lines locked at most in one set of the
    /// I-cache's 32 (Intel's limit is 28 of 32 ways; 8 stay free for the
    /// rest of the code in every set).
    const LOCK_MAX: u32 = 24 * 1024;
    const LOCK_PER_SET: u8 = 24;
    /// The list read from the card before the program starts, and how many
    /// ranges it holds.
    static LOCK_LIST: rt::Global<([u32; 2 * LOCK_RANGES], usize)> =
        rt::Global::new(([0; 2 * LOCK_RANGES], 0));
    /// Lines locked now (0: none).
    static LOCKED: rt::Global<u32> = rt::Global::new(0);
    /// Times the code was locked, and unlocked again (`rp_code_hint`).
    static LOCK_TIMES: rt::Global<(u32, u32)> = rt::Global::new((0, 0));

    /// The program's code hint (`rp_code_hint`): its hottest code locked
    /// while its frames run much code, unlocked when that passes (the lock
    /// gives the rest of its code a smaller cache: R324 run 93, a 16 KiB
    /// lock held throughout made the cascade's heavy frames 10% faster and
    /// its calm ones 2.6% slower).
    fn code_hint(hot: bool) {
        sram_map();
        if hot {
            lock_icache();
        } else if LOCKED.with(|l| core::mem::replace(l, 0)) != 0 {
            rp_hal_pxa27x::cpu::unlock_icache();
            LOCK_TIMES.with(|t| t.1 += 1);
        }
    }

    /// Locks the list's code into the I-cache (R324 run 92, 2026-11-18:
    /// the cascade's heavy frames lose 44.5% of their cycles to the
    /// I-cache, a third of it in ~16 KiB of the model drawing's code).
    /// Iterate, don't imitate (rule 0): GoldSrc and Xash leave the cache to
    /// hardware replacement, which on this core is round robin over a
    /// 32 KiB cache that each debris piece's code overruns, so every piece
    /// fetches its whole path again from SDRAM; consoles of the time kept
    /// hot code in scratchpads by hand; ours locks the lines the X51v's
    /// own stall profile ranks highest per byte, measured, leaving every set
    /// 8 free ways. Once, when the program says it plays (step 4): every
    /// module is loaded by then; a later load unlocks it all ([`fit`]).
    fn lock_icache() {
        if LOCKED.with(|l| *l) != 0 {
            return;
        }
        let (list, n) = LOCK_LIST.with(|l| *l);
        if n == 0 {
            return;
        }
        let code = module::POOL..module::POOL + module::POOL_LEN;
        let mut per_set = [0u8; 32];
        let (mut lines, mut skipped, mut bytes) = (0u32, 0u32, 0u32);
        let was = rt::irqs_enabled_now();
        rt::irq_disable();
        for k in 0..n {
            let (at, len) = (list[2 * k], list[2 * k + 1]);
            let end = at.saturating_add(len);
            if len == 0 || !code.contains(&at) || !code.contains(&(end - 1)) {
                skipped += 1;
                continue;
            }
            let mut line = at & !31;
            while line < end && bytes < LOCK_MAX {
                let set = (line >> 5 & 31) as usize;
                if per_set[set] < LOCK_PER_SET {
                    rp_hal_pxa27x::cpu::lock_icache_line(line);
                    per_set[set] += 1;
                    lines += 1;
                    bytes += 32;
                }
                line += 32;
            }
        }
        if was {
            rt::irq_enable();
        }
        LOCKED.with(|l| *l = lines);
        let first = LOCK_TIMES.with(|t| {
            t.0 += 1;
            t.0 == 1
        });
        if !first {
            return;
        }
        let _ = writeln!(
            Out,
            "runner: {lines} lines ({} KiB) of the modules' code locked into the I-cache from {n} ranges ({skipped} outside the modules' code skipped); the fullest set {} of 32",
            lines * 32 / 1024,
            per_set.iter().max().copied().unwrap_or(0)
        );
    }

    /// Unlocks what [`lock_icache`] locked, if anything.
    fn unlock_icache_after() {
        if LOCKED.with(|l| core::mem::replace(l, 0)) != 0 {
            rp_hal_pxa27x::cpu::unlock_icache();
        }
        let (locks, unlocks) = LOCK_TIMES.with(|t| *t);
        if locks != 0 {
            let _ = writeln!(
                Out,
                "runner: the I-cache's code locked {locks} times on the program's hint, unlocked {unlocks} times on it"
            );
        }
    }

    /// ADR-0040 step 7b: the modules' hottest code on the on-chip SRAM.
    /// Iterate, don't imitate (rule 0): GoldSrc and Xash run every
    /// function from SDRAM and leave the 32 KiB I-cache to round-robin
    /// replacement; the consoles of the time hand-placed overlays in a
    /// scratchpad; today's tools lay code out by profile (hot and cold
    /// splitting). Ours takes the X51v's own profile (`sram-hot.txt`), lets
    /// the linker gather those functions into whole 64 KiB pages a module
    /// (`.rp_hot`), and points the same addresses at the PXA270's 256 KiB
    /// SRAM through the page table (large pages): no address changes, no
    /// veneers, and an I-cache miss there costs a fraction of SDRAM's
    /// (docs/hw/cpu.md: a table read ~94 ns there against ~544).
    /// `sram.bin` in the program's folder: each range's address and length
    /// (little-endian words, 64 KiB multiples); no file, no mapping.
    const SRAM_FILE: &str = "sram.bin";
    /// Ranges at most.
    const SRAM_RANGES: usize = 4;
    /// The ranges read from the card.
    static SRAM_LIST: rt::Global<([u32; 2 * SRAM_RANGES], usize)> =
        rt::Global::new(([0; 2 * SRAM_RANGES], 0));
    /// Whether the code is on the SRAM now, refused for good, and the times
    /// it was put on and taken off.
    static SRAM: rt::Global<(bool, bool, u32, u32)> = rt::Global::new((false, false, 0, 0));

    /// Puts the hot pages on the SRAM, if the kit has them and they are not
    /// there now (`hot_code_on_sram`). Called on the program's hint each
    /// frame (a check once mapped), so it happens once the program plays,
    /// and again after a module load took them off.
    fn sram_map() {
        let (list, n) = SRAM_LIST.with(|l| *l);
        if n == 0 || SRAM.with(|s| s.0 || s.1) {
            return;
        }
        let code = module::POOL..module::POOL + module::POOL_LEN;
        let mut ranges = [(0u32, 0u32); SRAM_RANGES];
        for (k, r) in ranges[..n].iter_mut().enumerate() {
            *r = (list[2 * k], list[2 * k + 1]);
            if r.1 == 0 || !code.contains(&r.0) || !code.contains(&(r.0 + r.1 - 1)) {
                SRAM.with(|s| s.1 = true);
                let _ = writeln!(
                    Out,
                    "runner: {SRAM_FILE}: a range outside the modules' code: no SRAM"
                );
                return;
            }
        }
        let was = rt::irqs_enabled_now();
        rt::irq_disable();
        let done = rp_hal_pxa27x::diag::hot_code_on_sram(&ranges[..n]);
        if was {
            rt::irq_enable();
        }
        match done {
            Ok((bytes, mibs)) => {
                let first = SRAM.with(|s| {
                    s.0 = true;
                    s.2 += 1;
                    s.2 == 1
                });
                if first {
                    let _ = writeln!(
                        Out,
                        "runner: {} KiB of the modules' hottest code on the on-chip SRAM ({n} ranges, {mibs} MiBs as 64 KiB pages)",
                        bytes / 1024
                    );
                }
            }
            Err(why) => {
                SRAM.with(|s| s.1 = true);
                let _ = writeln!(Out, "runner: no hot code on the SRAM: {why}");
            }
        }
    }

    /// Takes the hot pages off the SRAM, if they are on it
    /// (`hot_code_off_sram`): before a module is made ready to run
    /// ([`fit`]) and at the program's end.
    fn sram_unmap() {
        if !SRAM.with(|s| s.0) {
            return;
        }
        let was = rt::irqs_enabled_now();
        rt::irq_disable();
        rp_hal_pxa27x::diag::hot_code_off_sram();
        if was {
            rt::irq_enable();
        }
        SRAM.with(|s| {
            s.0 = false;
            s.3 += 1;
        });
    }

    fn main() -> ! {
        // The serial line is set up only where it is listened to ([`say`]).
        if !devices::loader_present() {
            let Some(bus) = rt::take_ffuart() else {
                rp_image::fail("no serial port")
            };
            PxaUart::new(bus).init(115_200);
        }
        let mut uart = Out;
        rt::init_mmu();
        // The stretches between signs of life are counted from here.
        CHECKED_IN.with(|(last, _)| *last = rt::ticks());
        let _ = writeln!(uart, "\n{BANNER}");
        // The screen from the first moment: what this is and its step.
        let mbx = panel::start("RetroPane test run", "module runner: rp-xash");
        if mbx.is_none() {
            let _ = writeln!(uart, "screen: none (the run goes on without it)");
        }
        step(
            1,
            STEPS,
            b"Starting: the card,|the sound, the|program's modules.",
        );
        let mut cpld = devices::take_cpld().map(|(b, t)| Cpld::adopt(b, t).0);
        let volume = cpld.as_mut().and_then(open_card).and_then(mount);
        // Sound (the audio service, ADR-0032): the codec brought up as the
        // service does it; its two board bits are put back at the end.
        let board_found = found(cpld.as_ref());
        let Some(gpio) =
            rt::take_gpio().and_then(|bus| GPIO.put_with(|| RefCell::new(Gpio::new(bus))))
        else {
            rp_image::fail("no GPIO block")
        };
        let gpio: &'static SharedGpio = gpio;
        let mut audio = match (rt::take_i2c(), rt::take_i2s()) {
            (Some(i2c), Some(i2s)) if board_found.is_some() => {
                Some(service(gpio, cpld, i2c, i2s, LEVEL))
            }
            _ => None,
        };
        let speaker = audio.as_mut().and_then(|a| match a.up() {
            Ok(p) => {
                let _ = writeln!(uart, "sound: the codec is up, to the {}", name(p.output));
                Some(p.output == Output::Speaker)
            }
            Err(e) => {
                let _ = writeln!(uart, "sound: none ({e:?})");
                None
            }
        });
        let mut scratch = devices::take_scratch();
        // The sound's memory first: the scratch RAM starts on a cache line,
        // as the DMA ring must.
        let voice = match (
            speaker,
            scratch.take(),
            memory_for(SOUND_FOR.0, SOUND_FOR.1),
        ) {
            (Some(speaker), Some(ram), Some(bytes)) => {
                let (mem, rest) = ram.split_at_mut(bytes.next_multiple_of(32));
                scratch = Some(rest);
                rt::take_dma(rp_hal_pxa27x::dma_map::AUDIO)
                    .and_then(|dma| Sound::new(mem, dma, speaker))
                    .map(|mut s| {
                        // The mixer's stages timed in core cycles (R306
                        // item 3: inside the masked calls the samples
                        // can't see).
                        rt::cycle_counter_start();
                        s.set_clock(rt::cycles);
                        s
                    })
                    .and_then(|s| SOUND.put_with(move || s))
            }
            (_, ram, _) => {
                scratch = ram;
                None
            }
        };
        let (Some(vol), Some(ram)) = (volume, scratch) else {
            rp_image::fail("no card: the program's modules are on the card")
        };
        let _ = writeln!(uart, "card: mounted");
        let (work, room) = ram.split_at_mut(FAT_WORK);
        let (names, room) = room.split_at_mut(NAMES_ROOM);
        let (samples, room) = room.split_at_mut(SAMPLE_ROOM);
        let (chain_mem, room) = room.split_at_mut(super::gfx::CHAIN_BYTES);
        // R340's measure: the parameter memory manager's snapshots
        let (pm_mem, room) = room.split_at_mut(super::gfx::PM_WATCH_ROOM);
        super::gfx::PM_WATCH.with(|w| *w = super::gfx::PmWatch::new(pm_mem));
        let mut files: Files<'static, Disk, HANDLES> = Files::new(vol, work, room);
        files.remember(Names::new(names));

        // The modules' memory and this runner's identity.
        let Some(pool) = devices::take_port_pool() else {
            rp_image::fail("the RAM pool was taken twice")
        };
        if pool.as_ptr().addr() != module::POOL as usize || pool.len() != module::POOL_LEN as usize
        {
            rp_image::fail("the RAM pool is not where modules are built for");
        }
        let table = rp_hal_cabi::runner_table();
        let identity = module::runner_identity(table);
        let _ = writeln!(
            uart,
            "runner: {} names kept for modules, identity {identity:016x}",
            table.len() / 4
        );
        // Where the set of modules ends, from the program's own header:
        // the C heap is the pool from there.
        let mut path = [0u8; 64];
        let len = FOLDER.len() + 1 + PROGRAM.len();
        path[..FOLDER.len()].copy_from_slice(FOLDER.as_bytes());
        path[FOLDER.len()] = b'/';
        path[FOLDER.len() + 1..len].copy_from_slice(PROGRAM.as_bytes());
        let Ok(fd) = files.open(&path[..len]) else {
            let _ = writeln!(uart, "runner: {FOLDER}/{PROGRAM} is not on the card");
            rp_image::fail("the program's module is not on the card")
        };
        // The rest of the command line, if the card has one.
        let mut args = [c""; rp_hal_cabi::ARGS];
        args[..ARGS.len()].copy_from_slice(&ARGS);
        let mut arg_count = ARGS.len();
        let alen = FOLDER.len() + 1 + ARGS_FILE.len();
        let mut apath = [0u8; 64];
        apath[..FOLDER.len()].copy_from_slice(FOLDER.as_bytes());
        apath[FOLDER.len()] = b'/';
        apath[FOLDER.len() + 1..alen].copy_from_slice(ARGS_FILE.as_bytes());
        // The I-cache lock list (`lock_icache`), if the kit has one.
        let llen = FOLDER.len() + 1 + LOCK_FILE.len();
        let mut lpath = [0u8; 64];
        lpath[..FOLDER.len()].copy_from_slice(FOLDER.as_bytes());
        lpath[FOLDER.len()] = b'/';
        lpath[FOLDER.len() + 1..llen].copy_from_slice(LOCK_FILE.as_bytes());
        if let Ok(lfd) = files.open(&lpath[..llen]) {
            let mut raw = [0u8; 8 * LOCK_RANGES];
            let got = files.read(lfd, &mut raw).unwrap_or(0);
            let _ = files.close(lfd);
            let n = got / 8;
            LOCK_LIST.with(|(list, count)| {
                for (k, w) in raw[..n * 8].chunks_exact(4).enumerate() {
                    list[k] = u32::from_le_bytes([w[0], w[1], w[2], w[3]]);
                }
                *count = n;
            });
            let _ = writeln!(
                uart,
                "runner: {LOCK_FILE}: {n} ranges of code to lock into the I-cache when the program plays"
            );
        }
        // The SRAM's ranges (`sram_map`), if the kit has them.
        let slen = FOLDER.len() + 1 + SRAM_FILE.len();
        let mut spath = [0u8; 64];
        spath[..FOLDER.len()].copy_from_slice(FOLDER.as_bytes());
        spath[FOLDER.len()] = b'/';
        spath[FOLDER.len() + 1..slen].copy_from_slice(SRAM_FILE.as_bytes());
        if let Ok(sfd) = files.open(&spath[..slen]) {
            let mut raw = [0u8; 8 * SRAM_RANGES];
            let got = files.read(sfd, &mut raw).unwrap_or(0);
            let _ = files.close(sfd);
            let n = got / 8;
            SRAM_LIST.with(|(list, count)| {
                for (k, w) in raw[..n * 8].chunks_exact(4).enumerate() {
                    list[k] = u32::from_le_bytes([w[0], w[1], w[2], w[3]]);
                }
                *count = n;
            });
            let _ = writeln!(
                uart,
                "runner: {SRAM_FILE}: {n} ranges of hot code for the on-chip SRAM when the program plays"
            );
        }
        if let Ok(afd) = files.open(&apath[..alen])
            && let Some(text) = ARGS_TEXT.put_with(|| [0; ARGS_ROOM])
        {
            // The last byte stays NUL: the last word ends.
            let n = files.read(afd, &mut text[..ARGS_ROOM - 1]).unwrap_or(0);
            for b in &mut text[..n] {
                if b.is_ascii_whitespace() {
                    *b = 0;
                }
            }
            let text: &'static [u8] = text;
            // More than is read, or more words than the command line
            // holds: said and stopped, never a run with half its words.
            let mut one = [0u8; 1];
            let longer = n == ARGS_ROOM - 1 && files.read(afd, &mut one).unwrap_or(0) != 0;
            let _ = files.close(afd);
            let Some(count) = (!longer)
                .then(|| add_words(&text[..=n], &mut args, arg_count))
                .flatten()
            else {
                let _ = writeln!(
                    uart,
                    "runner: {ARGS_FILE} is too long: at most {} bytes and {} words",
                    ARGS_ROOM - 1,
                    rp_hal_cabi::ARGS - ARGS.len()
                );
                rp_image::fail("args.txt is longer than the command line holds")
            };
            arg_count = count;
            let _ = write!(uart, "runner: the command line has");
            for a in &args[ARGS.len()..arg_count] {
                let _ = write!(uart, " {}", a.to_str().unwrap_or("?"));
            }
            let _ = writeln!(uart, " from {ARGS_FILE}");
        }
        panel::foot(|c| {
            let _ = write!(c, "Asked:");
            for a in &args[1..arg_count] {
                let _ = write!(c, " {}", a.to_str().unwrap_or("?"));
            }
            // Whether the run ends by itself, and when: said from the
            // first moment, in the words a program's platform layer acts
            // on ([`END_WORD`], [`LIMIT_WORD`]).
            let words = &args[..arg_count];
            let _ = match (
                seconds_after(words, END_WORD),
                seconds_after(words, LIMIT_WORD),
            ) {
                (None, None) => write!(c, "\n\nDoes not end by itself: soft reset to stop it."),
                (end, limit) => {
                    let _ = write!(c, "\n\nEnds by itself:");
                    if let Some(s) = end {
                        let _ = write!(c, " {s} s into step 4");
                    }
                    if let Some(s) = limit {
                        let (m, s) = clock(s);
                        let or = if end.is_some() { "," } else { "" };
                        let _ = write!(c, "{or} with the clock at {m}:{s:02} at the latest");
                    }
                    write!(c, ".")
                }
            };
        });
        let peeked = Modules::new(&mut *pool, module::POOL, identity)
            .peek(&mut |into| files.read(fd, into).ok());
        let _ = files.close(fd);
        let header = match peeked {
            Ok(h) => h,
            Err(r) => {
                let _ = writeln!(
                    uart,
                    "runner: {PROGRAM} not loaded: {}",
                    r.reason().to_str().unwrap_or("refused")
                );
                rp_image::fail("the program's module was refused")
            }
        };
        let (modules, heap) = pool.split_at_mut((header.set_end - module::POOL) as usize);
        let heap = Break::new(heap.as_ptr().addr(), heap.as_ptr().addr() + heap.len());
        let _ = writeln!(
            uart,
            "runner: modules to 0x{:08X}, the C heap is the {} KiB after",
            header.set_end,
            (module::POOL + module::POOL_LEN - header.set_end) / 1024
        );

        // Made straight into its slot: the file table is several KiB, and
        // a copy or two of it on the way would be most of a small stack.
        let Some(s) = SYSTEM.put_with(|| System::new(files, Text, heap, ticks)) else {
            rp_image::fail("the C system was set up twice")
        };
        CARD.with(|c| {
            c.each = args[..arg_count].iter().any(|a| a.to_bytes() == LOOKS_WORD);
            c.each_read = args[..arg_count].iter().any(|a| a.to_bytes() == READS_WORD);
            c.play_opens = args[..arg_count]
                .iter()
                .any(|a| a.to_bytes() == PLAY_OPENS_WORD);
        });
        s.set_looks(Looks {
            reads: card_reads,
            looked,
            opened,
            read,
            listed,
        });
        s.set_modules(Library {
            modules: Modules::new(modules, module::POOL, identity),
            folder: FOLDER.as_bytes(),
            fit,
            hint: code_hint,
        });
        // ADR-0040 step 7g1: the PXA270's media unit (iWMMXt) on for the
        // OS mixer's filter dot products (`rp_hal_simd::dot`, held equal to
        // the plain loop by the audio crate's tests), as `rp-audio-s2` does;
        // the game's code is built without it, so nothing else touches its
        // registers. Put back as found at the end.
        let simd_was = rp_hal_pxa27x::iwmmxt::single_access(true);
        rp_hal_simd::set_unit(true);
        let apd_was = if args[..arg_count].iter().any(|a| a.to_bytes() == APD_WORD) {
            None
        } else {
            let was = rp_hal_pxa27x::clocks::sdram_auto_power_down(false);
            let _ = writeln!(
                uart,
                "runner: the SDRAM's automatic power-down off while the program runs (it was {})",
                if was { "on" } else { "off" }
            );
            Some(was)
        };
        if let Some(v) = voice {
            s.set_sound(v);
        }
        match pad(gpio).and_then(|p| INPUT.put_with(|| Input::new(p, Calibration::X51V_DEFAULT))) {
            Some(i) => s.set_input(i),
            None => {
                let _ = writeln!(uart, "input: none (no keypad or SSP1)");
            }
        }
        // `-rpwmclock`: Windows Mobile's 2700G clock kept (R343's picker
        // run, where the chip never wedged), whatever `-rpclock` says
        let rung = if args[..arg_count]
            .iter()
            .any(|a| a.to_bytes() == b"-rpwmclock")
        {
            None
        } else {
            clock_rung(&args[..arg_count])
        };
        let ladder = args[..arg_count]
            .iter()
            .any(|a| a.to_bytes() == LADDER_WORD);
        let bar = args[..arg_count].iter().any(|a| a.to_bytes() == BAR_WORD);
        // R343: the tone curves stepped through for the owner's pick
        let curves = args[..arg_count]
            .iter()
            .any(|a| a.to_bytes() == b"-rpcurves");
        if curves {
            super::gfx::curve_key_on();
        }
        let cpu_n2 = args[..arg_count]
            .iter()
            .rposition(|a| a.to_bytes() == CPU_WORD)
            .and_then(|at| args[..arg_count].get(at + 1))
            .and_then(|v| match number(v.to_bytes()) {
                Some((n, [])) => u16::try_from(n).ok(),
                _ => None,
            });
        // Step 4f's probe: its walk from the samples' room, its channel.
        let probe_after = seconds_after(&args[..arg_count], PROBE_WORD);
        let (samples, probe) = match probe_after {
            Some(after) => {
                let (samples, walk) = samples.split_at_mut(SAMPLE_ROOM - super::gfx::WALK_BYTES);
                let walk: &'static [u8] = walk;
                let probe = rt::take_dma(rp_hal_pxa27x::dma_map::CF)
                    .map(|dma| super::gfx::Probe::new(dma, walk, after));
                let _ = writeln!(
                    uart,
                    "runner: -rp4f: step 4f's probe {} after {after} s of frames",
                    if probe.is_some() {
                        "set"
                    } else {
                        "NOT set (no DMA channel)"
                    }
                );
                (samples, probe)
            }
            None => (samples, None),
        };
        // ADR-0040 step 7g6's probe of the strip header's bits.
        for (word, mode) in [
            (&b"-rpta17"[..], 1u32),
            (&b"-rpta21"[..], 2),
            (&b"-rpta1721"[..], 3),
        ] {
            if args[..arg_count].iter().any(|a| a.to_bytes() == word) {
                super::gfx::TA_PROBE.with(|m| *m = mode);
                let _ = writeln!(
                    uart,
                    "runner: the TA probe: strip header bits {}",
                    ["", "17", "21", "17 and 21"][mode as usize]
                );
            }
        }
        // Step 4f: the DMA chain, if asked for.
        let chain = if args[..arg_count]
            .iter()
            .any(|a| a.to_bytes() == NO_CHAIN_WORD)
        {
            let _ = writeln!(
                uart,
                "runner: -rpnochain: the CPU feeds the TA (the control)"
            );
            None
        } else {
            let chain = rt::take_dma(rp_hal_pxa27x::dma_map::UDC)
                .and_then(|dma| super::gfx::Chain::new(dma, chain_mem));
            let _ = writeln!(
                uart,
                "runner: step 4f {}",
                if chain.is_some() {
                    "on: each frame fed to the TA by one DMA chain"
                } else {
                    "NOT on (no DMA channel or memory)"
                }
            );
            chain
        };
        // R341: the backlight for the game (the loader puts the board
        // back after every image, a crash's included: audit C39)
        if args[..arg_count]
            .iter()
            .any(|a| a.to_bytes() == b"-rptexhold")
        {
            // (the bare word: the whole run; the line holds 31 words)
            let s = seconds_after(&args[..arg_count], b"-rptexhold").unwrap_or(0);
            super::gfx::QUIET.with(|q| q.hold_s = Some(s));
            let _ = writeln!(
                uart,
                "runner: R340 -rptexhold {s}: the program's texture writes wait while the TA and a render are both at work (the first {s} s; 0: the whole run)"
            );
        }
        if args[..arg_count]
            .iter()
            .any(|a| a.to_bytes() == b"-rpmemback")
        {
            super::gfx::MEM_BACK.with(|m| m.0 = true);
            let _ = writeln!(
                uart,
                "runner: R340 -rpmemback: 0x804 = 1 after every render (the overlap on)"
            );
        }
        let light = light_up(
            seconds_after(&args[..arg_count], LIGHT_WORD)
                .unwrap_or(LIGHT_DEFAULT)
                .min(100),
        );
        match CHIP.put_with(|| {
            super::gfx::Chip::new(gfx_log, mbx)
                .with_light(light)
                .with_rung(rung)
                .with_ladder(ladder)
                .with_bar(bar)
                .with_curves(curves)
                .with_rebuild_at(seconds_after(&args[..arg_count], b"-rprebuild"))
                .with_overlap_at(seconds_after(&args[..arg_count], b"-rpoverlapat"))
                .with_report_every(
                    if args[..arg_count].iter().any(|a| a.to_bytes() == b"-rpplay") {
                        15_000_000
                    } else {
                        5_000_000
                    },
                )
                .with_cpu(cpu_n2)
                .with_probe(probe)
                .with_chain(chain)
                .with_overlap(
                    !args[..arg_count]
                        .iter()
                        .any(|a| a.to_bytes() == NO_OVERLAP_WORD),
                )
                .with_chain_check(
                    if args[..arg_count]
                        .iter()
                        .any(|a| a.to_bytes() == CHAIN_CHECK_WORD)
                    {
                        CHAIN_CHECK_FRAMES
                    } else {
                        0
                    },
                )
        }) {
            Some(c) => s.set_screen(c),
            None => {
                let _ = writeln!(uart, "rp-gfx: none (set up twice)");
            }
        }
        check_libraries(s);
        step(
            2,
            STEPS,
            b"The program starts|and reads its files.|This can take|minutes: the clock|below keeps going.",
        );
        if args[..arg_count].iter().any(|a| a.to_bytes() == FLOAT_WORD) {
            float_bench();
            dot_bench(seconds_after(&args[..arg_count], DOT_WORD).unwrap_or(1));
            cmp_bench(seconds_after(&args[..arg_count], DOT_WORD).unwrap_or(1));
            div_bench();
        }
        if args[..arg_count]
            .iter()
            .any(|a| a.to_bytes() == WALLOC_WORD)
        {
            let n = rt::write_allocate_sections(WALLOC_FROM, WALLOC_TO);
            let _ = writeln!(
                uart,
                "runner: -rpwalloc: {n} MiB of the game's memory write-allocate (0x{WALLOC_FROM:08x}..0x{WALLOC_TO:08x})"
            );
        }
        let sample_on = args[..arg_count]
            .iter()
            .position(|a| a.to_bytes() == SAMPLE_ON_WORD)
            .and_then(
                |at| match number(args[..arg_count].get(at + 1)?.to_bytes())? {
                    (v, []) => u8::try_from(v).ok(),
                    _ => None,
                },
            )
            .map(|event| {
                // cycles (I-cache, data and buffer stalls) or occurrences
                (
                    event,
                    if matches!(event, 0x01 | 0x02 | 0x08) {
                        200_000
                    } else {
                        2_000
                    },
                )
            });
        let sampling = sample_on.is_some()
            || args[..arg_count]
                .iter()
                .any(|a| a.to_bytes() == SAMPLE_WORD);
        let sys: &'static mut dyn Sys = s;
        rp_hal_cabi::start(sys, exit);
        if sampling {
            let _ = writeln!(
                uart,
                "runner: sampling where the program is, {SAMPLE_HZ} a second, {} at most",
                SAMPLE_ROOM / rt::SAMPLE_BYTES
            );
            // the callers of helpers that use lr as data named from the
            // stack (R314): all of SDRAM, the code and the stacks
            rt::set_sample_window(0xA800_8000, module::POOL + module::POOL_LEN);
            match sample_on {
                Some((event, every)) => {
                    let _ = writeln!(
                        uart,
                        "runner: -rpsampleon: a sample every {every} of event 0x{event:02x}, not the clock"
                    );
                    rt::start_event_sampling(samples, event, every);
                }
                None => rt::start_sampling(samples, SAMPLE_HZ),
            }
            if args[..arg_count].iter().any(|a| a.to_bytes() == b"-rpplay") {
                // R346: free play's minutes fit (one in 24), its slow
                // frames whole
                rt::sample_keep(SAMPLE_KEEP, SAMPLE_SLOW_US);
                let _ = writeln!(
                    uart,
                    "runner: free play: one sample in {SAMPLE_KEEP} kept, every sample of a frame of {} ms or more",
                    SAMPLE_SLOW_US / 1000
                );
            }
        }
        // Kit 69 (Half-Life run 66: the CPU waited 0.6-2.9 ms a frame for
        // the last frame's render, which the pump started only at the
        // program's next clock read): the tick runs the screen's pump every
        // millisecond (the sampler's tick when sampling, else its own).
        rt::set_tick_hook(Some(rp_hal_cabi::pump_screen));
        if !sampling || sample_on.is_some() {
            rt::start_tick(PUMP_HZ);
        }
        let ran = rp_hal_cabi::run_module(PROGRAM.as_bytes(), &args[..arg_count]);
        unlock_icache_after();
        sram_unmap();
        let (maps, unmaps) = SRAM.with(|s| (s.2, s.3));
        if maps != 0 {
            let _ = writeln!(
                Out,
                "runner: the hot code put on the SRAM {maps} times, taken off {unmaps} times"
            );
        }
        rt::set_tick_hook(None);
        if !sampling {
            rt::stop_tick();
        }
        // C2: Dell's 624 MHz back whatever the program left (its screen's
        // close puts it back first; this is the net under it).
        super::cpu::put_back(gfx_log);
        rp_hal_simd::set_unit(false);
        rp_hal_pxa27x::iwmmxt::single_access(simd_was);
        if let Some(was) = apd_was {
            rp_hal_pxa27x::clocks::sdram_auto_power_down(was);
        }
        // Stopped at once: what follows is the runner's, not the program's.
        if let Some((buf, taken, full)) = rt::stop_sampling() {
            write_samples(buf, taken, full);
        }
        // R340's measure: the manager's snapshots to the card
        if let Some(w) = super::gfx::PM_WATCH.with(Option::take) {
            let put = write_card(super::gfx::PM_WATCH_FILE, &[w.bytes()]);
            let _ = writeln!(
                Out,
                "runner: R340: the parameter memory manager's snapshots ({} KiB) {put}",
                w.bytes().len() / 1024
            );
        }
        // The program's end stopped the DMA engine (`Sys::end`): the codec
        // down, the board as it was found.
        if let Some(mut a) = audio {
            let down = a.down().is_ok();
            put_back(a.board_mut().cpld.take(), board_found);
            let _ = writeln!(
                uart,
                "sound: the codec is down ({down}), the board as found"
            );
        }
        longest_quiet();
        card_count(format_args!("at the end"));
        match ran {
            Ok(code) => {
                let _ = writeln!(uart, "runner: the program returned {code}");
                panel::done(b"The program ended.|Back to the loader.");
                rp_image::finish()
            }
            Err(e) => {
                let _ = writeln!(uart, "runner: {PROGRAM} was not started ({e:?})");
                rp_image::fail("the program's module was not started")
            }
        }
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
