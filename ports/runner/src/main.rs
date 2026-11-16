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
    use rp_hal_api::{BlockDevice, BlockError};
    use rp_hal_cabi::{Break, Sys};
    use rp_hal_pxa27x::dma::Dma;
    use rp_hal_pxa27x::{Mmio, rt};
    use rp_hal_x51v::devices;
    use rp_port::files::Files;
    use rp_port::input::{Input, Pad};
    use rp_port::module::{self, Modules};
    use rp_port::progress::{Lines, Said, said};
    use rp_port::sound::{Sound, memory_for};
    use rp_port::sys::{Console, Library, System};
    use rp_touch_cal::{Calibration, Sample};

    use super::panel;
    use super::{BANNER, FOLDER, PROGRAM};

    rp_image::entry!(main);

    /// The touch chip's select and pen lines (docs/hw/touch.md).
    const TS_CS: u8 = 24;
    const TS_PENIRQ: u8 = 94;
    /// The codec's output level, 0-100.
    const LEVEL: u32 = 60;
    /// The sound's memory is sized for the largest stream a game opens
    /// here: 44.1 kHz stereo.
    const SOUND_FOR: (u32, u8) = (44_100, 2);
    /// Of the scratch RAM: the file system's sector work.
    const FAT_WORK: usize = 64 * 1024;
    /// Files a program may have open at once.
    const HANDLES: usize = 16;
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
    const ARGS_ROOM: usize = 256;
    static ARGS_TEXT: rt::Slot<[u8; ARGS_ROOM]> = rt::Slot::new();

    /// The words of `text` (NUL where its spaces and line ends were, by
    /// [`words`]) added to `args` after its first `have`; how many `args`
    /// holds then. Words past its room are left off.
    fn add_words(
        text: &'static [u8],
        args: &mut [&'static core::ffi::CStr; rp_hal_cabi::ARGS],
        mut have: usize,
    ) -> usize {
        let mut at = 0;
        while at < text.len() && have < args.len() {
            match core::ffi::CStr::from_bytes_until_nul(&text[at..]) {
                Ok(word) => {
                    if !word.is_empty() {
                        args[have] = word;
                        have += 1;
                    }
                    at += word.count_bytes() + 1;
                }
                Err(_) => break,
            }
        }
        have
    }

    type Sd = SdCard<PxaMmc<Mmio, Dma>>;

    /// The card, checking in with the run limit at every transfer.
    struct Card(Sd);

    impl BlockDevice for Card {
        fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), BlockError> {
            // The screen's clock keeps going while files are read.
            panel::beat(rt::ticks());
            rt::check_in();
            self.0.read_blocks(lba, buf)
        }
        fn write_blocks(&mut self, lba: u32, buf: &[u8]) -> Result<(), BlockError> {
            rt::check_in();
            self.0.write_blocks(lba, buf)
        }
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
    fn step(n: u32, of: u32, text: &[u8]) {
        panel::step(n, of, text);
        let at = panel::beat(rt::ticks());
        let _ = writeln!(
            Out,
            "runner: step {n} of {of} at {at} s on the screen's clock"
        );
    }

    static SYSTEM: rt::Slot<System<'static, Card, HANDLES, Text>> = rt::Slot::new();
    static SOUND: rt::Slot<Sound<'static, Dma>> = rt::Slot::new();
    /// The GPIO block: the audio service, the codec's gate and the touch
    /// chip's two lines share it for the whole run.
    static GPIO: rt::Slot<SharedGpio> = rt::Slot::new();
    static INPUT: rt::Slot<Input<X51vPad>> = rt::Slot::new();

    type Touch = Tsc2046<Ssp<Mmio>, GpioPin<'static, Mmio>, GpioPin<'static, Mmio>>;

    /// The X51v's buttons and touch chip, read when the program asks.
    struct X51vPad {
        keypad: Keypad<Mmio>,
        touch: Touch,
    }

    impl Pad for X51vPad {
        fn pressed(&mut self) -> Pressed {
            self.keypad.scan().held
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
    fn mount(mut sd: Sd) -> Option<AnyVolume<Card>> {
        let mut block = [0u8; rp_drv_sdcard::BLOCK];
        sd.read(0, &mut block).ok()?;
        let start = mbr_partitions(&block)
            .and_then(|p| p[0])
            .map_or(0, |p| p.start);
        AnyVolume::mount(Card(sd), start).ok()
    }

    /// The test image's own check of the library modules beside the
    /// program (a game's server and client libraries, which the program
    /// loads only once it has the game's files): each is loaded through
    /// the same loader, every name it exports is looked up and every
    /// function named, and it is let go again. Its constructors are not
    /// run. Not something a runner under Game Mode does.
    fn check_libraries(s: &mut System<'static, Card, HANDLES, Text>) {
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
    /// counter).
    static CHECKED_IN: rt::Global<u32> = rt::Global::new(0);
    /// A tenth of a second of the OS timer's 3.25 MHz.
    const CHECK_IN_TICKS: u32 = 325_000;

    /// The program's clock source: the OS timer's counter. A program reads
    /// its clock in every frame, so this is also where a game that plays
    /// for minutes without touching the card checks in with the loader's
    /// run limit and watchdog (the card's transfers do it otherwise), at
    /// most ten times a second: a game that stops reading its clock is
    /// hung, and is ended by the limit as any image is.
    fn ticks() -> u32 {
        let now = rt::ticks();
        let due = CHECKED_IN.with(|last| {
            let due = now.wrapping_sub(*last) >= CHECK_IN_TICKS;
            if due {
                *last = now;
            }
            due
        });
        if due {
            rt::check_in();
            panel::beat(now);
        }
        now
    }

    /// Makes loaded bytes fit to run: the data cache's copy written to
    /// RAM and the instruction cache's old lines for those addresses
    /// dropped (a module loaded a second time lies where its first load
    /// was run from).
    fn fit(at: u32, len: u32) {
        rp_hal_pxa27x::cpu::clean_invalidate_range(at, len);
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
        let _ = writeln!(uart, "\n{BANNER}");
        // The screen from the first moment: what this is and its step.
        if !panel::start("RetroPane test run", "module runner: rp-xash") {
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
        let mut files: Files<'static, Card, HANDLES> = Files::new(vol, work, room);

        // The modules' memory and this runner's identity.
        let Some(pool) = devices::take_ram_pool_bytes() else {
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
        if let Ok(afd) = files.open(&apath[..alen])
            && let Some(text) = ARGS_TEXT.put_with(|| [0; ARGS_ROOM])
        {
            // The last byte stays NUL: the last word ends.
            let n = files.read(afd, &mut text[..ARGS_ROOM - 1]).unwrap_or(0);
            let _ = files.close(afd);
            for b in &mut text[..n] {
                if b.is_ascii_whitespace() {
                    *b = 0;
                }
            }
            let text: &'static [u8] = text;
            arg_count = add_words(&text[..=n], &mut args, arg_count);
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
        s.set_modules(Library {
            modules: Modules::new(modules, module::POOL, identity),
            folder: FOLDER.as_bytes(),
            fit,
        });
        if let Some(v) = voice {
            s.set_sound(v);
        }
        match pad(gpio).and_then(|p| INPUT.put_with(|| Input::new(p, Calibration::X51V_DEFAULT))) {
            Some(i) => s.set_input(i),
            None => {
                let _ = writeln!(uart, "input: none (no keypad or SSP1)");
            }
        }
        check_libraries(s);
        step(
            2,
            STEPS,
            b"The program starts|and reads its files.|This can take|minutes: the clock|below keeps going.",
        );
        let sys: &'static mut dyn Sys = s;
        rp_hal_cabi::start(sys, exit);
        let ran = rp_hal_cabi::run_module(PROGRAM.as_bytes(), &args[..arg_count]);
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
