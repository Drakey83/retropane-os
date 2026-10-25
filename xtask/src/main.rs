//! `cargo xtask <command>`: one entry point for every build, check and test,
//! used identically by developers and CI.

#![forbid(unsafe_code)]

mod adrs;
mod calib;
mod devlock;
mod emu;
mod eol;
mod frames;
mod g7;
mod gate;
mod measure;
mod mutants;
mod perf;
mod policy;
mod profile;
mod reminders;
mod screen;
mod sdkit;
mod serve;
mod size;
mod spot;
mod symbols;
mod trace;
mod usb;
mod usbbench;
mod userimg;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use policy::{DepKind, Placement};
use serde_json::Value;

pub(crate) const TARGET_SPEC: &str = "targets/armv5te-x51v-none-eabi.json";

/// Where cargo puts the X51v's release builds: under `CARGO_TARGET_DIR` when
/// it is set (relative to the workspace, as cargo takes it), else the
/// workspace's `target`. Every image path goes through here: one built
/// elsewhere and read from `target` is a stale image, measured and run
/// without a word (2026-10-24: `emu-storfast` ran an old `rp-storfast`).
pub(crate) fn arm_release(root: &Path) -> PathBuf {
    target_dir(root).join("armv5te-x51v-none-eabi/release")
}

/// Cargo's build folder: `CARGO_TARGET_DIR` when it is set (relative to the
/// workspace, as cargo takes it), else the workspace's `target`. Every
/// path to a build cargo made without `--target-dir` starts here (the Thumb
/// images were copied from `target` too: `emu-cf` ran an old loader,
/// 2026-10-24).
pub(crate) fn target_dir(root: &Path) -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .filter(|d| !d.is_empty())
        .map_or_else(|| root.join("target"), |d| root.join(d))
}
/// The Thumb variant of the target (ADR-0015: cold code as Thumb).
const THUMB_TARGET_SPEC: &str = "targets/thumbv5te-x51v-none-eabi.json";
/// Images built as Thumb (ADR-0015); empty it to go back to all-ARM.
const THUMB_IMAGES: &[&str] = &["rp-loader"];
/// Images built with frame pointers (ADR-0001, "The crash-screen backtrace
/// is mode-dependent"): their crash report walks the frame chain
/// (hal/pxa27x/src/backtrace.rs). Every other image is may-omit, has no
/// walker and says so. Emptying it puts every image back to may-omit. ARM
/// code only: a Thumb image's frame pointer is r7 and calls between ARM and
/// Thumb break the chain (`cross` refuses an image in both lists).
pub(crate) const FRAME_POINTER_IMAGES: &[&str] = &["rp-crash"];
/// What a frame-pointer image is built with: frame pointers in the whole
/// build (`core` and `compiler_builtins` too, `-Zbuild-std` builds them
/// with these flags, so the chain is unbroken) and the walker compiled in.
const FRAME_POINTER_FLAGS: &str = "-C force-frame-pointers=yes --cfg rp_frame_pointers";

/// Sets up a cargo build of `image` into target folder `dir`: for a
/// frame-pointer image ([`FRAME_POINTER_IMAGES`]) its flags, and a folder of
/// its own inside `dir` (the flags rebuild everything, `core` included, so
/// it never shares a folder with a may-omit build). Returns the target
/// folder to build into. Every image build goes through it (`cross`, the
/// emulator's variants, `perf`, `codegen-variant`). `RP_MAY_OMIT=1` builds
/// a listed image may-omit anyway: the X51v check of a may-omit crash
/// report (`RP_MAY_OMIT=1 cargo xtask codegen-variant rp-crash may-omit`).
pub(crate) fn frame_pointer_build(cmd: &mut Command, image: &str, dir: &str) -> String {
    let may_omit = std::env::var_os("RP_MAY_OMIT").is_some_and(|v| v == "1");
    if FRAME_POINTER_IMAGES.contains(&image) && !may_omit {
        cmd.env("RUSTFLAGS", FRAME_POINTER_FLAGS);
        format!("{dir}/frame-pointers")
    } else {
        dir.to_string()
    }
}

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  ci        everything CI runs: eol, fmt, clippy, lint, gate, reminders, adrs, test, cross
  eol       no tracked file with CRLF line endings in the working tree
  reminders the reminders list (docs/reminders.md) checked, the open count printed
  fmt       check formatting
  clippy    lint all crates (warnings are errors)
  lint      architecture policy: layering and unsafe rules (DESIGN.md §3.1)
  test      host unit tests
  cross     build every OS crate for the X51v (ARMv5TE)
  coverage  host test coverage (needs cargo-llvm-cov), 80% line floor
  gate      count the performance gate's checklist lines (docs/ROADMAP.md)
  adrs      the ADR index (docs/adr/README.md) must match the records
  adr-index write the ADR index from the records
  symbols   a crash report's addresses as functions and source lines:
            `symbols <image | elf> <addr...>` (its CHAIN line, the fault address)
  perf      the performance ratchet on the docked X51v: the device suite against
            docs/perf-floors.md (`--update` ratchets faster floors down;
            `perf noise N` sets every floor and tolerance from N runs;
            `perf refloor N` from the newest N saved ones)
  emu       boot the bring-up, probe, kernel, threads, first-light and USB images
            in the emulator
            (downloads it if needed)
  qemu      build the emulator locally in WSL from ../retropane-qemu (fast loop)
  sdkit     SD card kit for the real X51v: HaRET + raw image (target/sdkit)
            (--serve: also serve it over Wi-Fi for the device's browser)
  axim      who is using the docked Axim, if anyone: every command that
            talks to it holds a machine-wide lock for its whole run, and
            waits while another holds it (--no-wait, or RP_AXIM_NO_WAIT=1:
            fail at once instead; RP_AXIM_WHO=<label> names the holder)
  usb       talk to an X51v running the USB link test (target/sdkit/usb) over
            its sync cable: find the COM port, check the banner and echo
  codegen-variant <image> <label> <--config ...>...
            build a measurement variant of an image with other build
            settings (e.g. 'profile.release.package.rp-probe.opt-level=3')
            into target/variants/<label>/, its .bin beside it, for
            `load <file.bin>`; size reported against the budget, not
            enforced (never shipped: the gate's C9 comparisons)
  load      send an image (name, e.g. rp-probe, or a .bin) to an X51v running
            the resident loader (target/sdkit/loader) over USB
            (--setup: create the admin task that lets it re-find the Axim
            after each run without a cable replug; one UAC prompt, once)
  usbbench  USB throughput through the loader (docs/perf.md, item 74):
            `usbbench [serial|drive|all] [runs]`; a bad-CRC image frame and a
            16 MiB unbuffered file written, read back and compared on the card;
            `usbbench trial [loader image] [runs]` tries a new loader from RAM
            (probe + benches), then loads the card's loader back
  put       write a file to the X51v's SD card over USB through the resident
            loader (ADR-0013): cargo xtask put <file> [name on the card]
  install   put the kit's loader, HaRET and scripts on the card the same way
            (replaces the files already there; run cargo xtask sdkit first);
            `install cf` puts them in the CF card's RetroPane folder instead
            (through the loader, as the PC may not change them; each file
            read back and compared)
  drive     the SD and CF cards as drives on the PC (ADR-0014): writable by
            default except their boot files; `drive ro` makes them read-only,
            `drive rw` writable again,
            `drive off`/`drive on` withdraws/offers the card (an empty reader
            to the PC), `drive status` reports counters and the last card error
  backlight the X51v's backlight through the loader: `backlight` reports
            it, `backlight <0-100>` fades to that level (perceptual percent)
  card      card diagnostics through the loader: `card scan <lba> <count>`
            reads blocks one by one, reports failures and the slowest read;
            `card scrub <lba>` rewrites a weak first-FAT sector in place,
            verified against its mirror in the second FAT; `card repair
            <lba>` restores a damaged one from the mirror (only when the
            first copy's differing entries are invalid); `card check`
            checks the file system read-only (FAT32 or exFAT); `card dma` reports
            the SD DMA self-test, `card dma writes` lets writes use DMA
  ls        list a folder on the card through the loader (read-only)
  get       `get <file> [local name]`: copy a file from the card, CRC
            checked end to end; never overwrites a local file
  rm        delete a file or empty folder on the card through the loader
            (crash-safe; the files the device boots from are refused)
  cf        identify the CompactFlash card through the loader (read-only;
            then it is the drive's second unit); `cf log`: READY changes,
            attach and drop times; `cf soft`: no hardware reset (diagnostics)
  stack     the loader's deepest stack use so far, of its size (quality
            audit C79: the CF attach's timing searches overflowed it once)
  log       the last image's run log again, from the loader (for a log
            lost on the way back; `load` asks for it by itself)
  cardlog   `cardlog [image] [number]`: a run's log kept on the cards as
            the run went (one of 32 files rplog/log-SS.txt reused in turn,
            which run each holds in rplog/index.txt; every run the loader
            starts; the SD card's part and the CF card's joined): the
            newest, or the one numbered; it survives hangs, watchdog and
            soft resets; saved under local/cardlog/
  cpld      the CPLD's registers as the loader found them at start-up
            (read-only diagnostics)
  handoff   which DMA channels the loader's start found running, how long
            it waited for it to stop, and its boot time
  board     what the loader's start did with the board record (quality
            audit C39/C57): recorded, put back, or still differing
  trace    the last program's trace ring (thread switches, faults,
            calls...) from the loader, as a timeline (item 54)
  g7-frame  [capture] [log] [out]: Dell's divide-1 frame out of rp-wmprobe
            v4's capture, for rp-glesfeat G7 (defaults local/rp-wmprobe-div1-vram.bin,
            local/rp-wmprobe-7.txt, local/g7-dell-frame.bin)
  wmbench   build the stock Windows Mobile baseline program (tools/wmbench:
            run by hand under WM; it writes rp-wm-baseline.txt next to itself)
  reboot-wm reset the X51v back into Windows Mobile the way its own
            reboot does (GPIO99; item 64, not yet tried on the device)
  tone     the audio check: loads rp-tone (440 Hz to the speaker or the
            headphones, then waits 15 s for the jack to change; the run log
            names the output and the CPU's busy share)
  audio-s1  audio service step S1 (ADR-0032), a guided run with the
            owner listening: loads rp-audio-s1 (44.1 kHz, the jack while up
            and while down, the I2S restart guard)
  audio-s2  audio service step S2 (ADR-0032), the mixer, a guided run
            with the owner listening: loads rp-audio-s2 (two rates, taps,
            the tune with and without clicks, old-game rates, the limiter
            and the speaker filter A/B)
  device-check  the read-only checks waiting for the device, in one run
            (drive, SD DMA, backlight, the unscanned card blocks), report
            in target/device-check.txt; --with-owner adds CF (the tone:
            `cargo xtask tone`)
  emu-picture  only the rp-picture check (a UI concept on screen, then
            back to the loader): run it before `load rp-picture`
  emu-blend-scale rp-blend's P9 scaler steps (RP_BLEND_SCALE): each step up,
            the scaler registers read back as written, put back
  emu-blend only the rp-blend check (plane blending, three stages
            pixel-exact): run it before `load rp-blend`
  emu-mbx   only the GPU check (rp-mbx: fills by the emulated MBX 2D
            engine, edges exact): run it before `load rp-mbx`
  emu-mbx-blend  only the blend and display-event probe (rp-mbx-blend:
            every stage runs, state put back): run it before `load rp-mbx-blend`
  emu-glesfeat the alpha test, depth 'less' and grey textures (rp-glesfeat:
            every case runs and returns): run it before `load rp-glesfeat`
  emu-mbx3d the 3D core's first-triangle test (rp-mbx3d: runs and returns;
            the 3D core isn't modelled): run it before `load rp-mbx3d`
  emu-unplug only the USB disconnect hunt's safety check (rp-unplug: every
            candidate tried and undone, back at the loader): run it before
            `load rp-unplug`
  emu-mbx3d-api  the 3D renderer's device test (rp-mbx3d-api: every frame
            refused safely, the 3D core isn't modelled; back on a button):
            run it before `load rp-mbx3d-api`
  emu-cs3bus the 2700G bus-timing experiment (rp-cs3bus: every step and
            the soak run, MSC1 put back): run it before `load rp-cs3bus`
  emu-cpldbus the CPLD bus-timing test (rp-cpldbus: every read step run,
            MSC2 put back): run it before `load rp-cpldbus`
  emu-gpubench  the GPU measuring run for the 3D compositor proposal
            (rp-gpubench run A and every RP_GPUBENCH_* run, run P's target
            formats included: every stage runs, memory given back, each
            returns): run it before `load rp-gpubench`
  emu-mved  the video decoder's first test (rp-mved: runs, times out
            safely, returns; the decoder isn't modelled): before `load rp-mved`
  emu-noise the RNG noise-source measurement (rp-noise, ADR-0029: every
            source collected and saved to the card; plumbing only, the
            emulator's timing is the PC's): run it before `load rp-noise`
  emu-clocks the clocks test (rp-clocks: every operating point visited,
            the clocks and voltage put back): run it before `load rp-clocks`
  emu-today the shell's Today screen (ADR-0021): pixel-exact, a tap on
            Start opens the menu, Back to loader returns with latencies
  emu-crash only the crash path check (an image's crash box on the plane
            on screen, then the loader's crash screen, log and card report)
  emu-probe only the probe's check (rp-probe: the device report, its
            benchmarks and checks, every page's checksums)
  emu-kernel only the kernel image's check (VM, IPC, the driver host)
  emu-threads only the threads image's check (rp-threads: scheduler, user
            mode, KData, FCSE in a native slot): run it before `load rp-threads`
  emu-quantum the time-slice policies' touch, frame and switch-cost tests
            (rp-quantum: all run; numbers printed): run it before `load rp-quantum`
  emu-usb   only the loader-over-USB check (image in, run log out)
  emu-loader only the loader's boot check (boot time, an image's round trip)
  emu-keepfed after an image returns, the loader keeps the watchdog fed with no PC
            program (150 s; needs rp-v0.21.0's timer fix)
  emu-handover only the hand-over check (rp-handover: the 2700G with no fixed
            wait, the CPLD shadow adopted from Windows Mobile's table)
  emu-nightlight only the night-light experiment's check (rp-nightlight)
  emu-cfdetect only the CF detect diagnostic's check (rp-cfdetect)
  emu-screen-measure the colorimeter patches' check (rp-screen-measure driven
            by a measurement program: steps on time, the table switched, a
            damaged program refused; not part of `emu`): run it before `measure`
  measure   the screen measured with a colorimeter (ArgyllCMS + Spyder4):
            starts rp-screen-measure, reads each patch, writes a report and a
            correcting gamma table to local/screen-measure/
            --profile: a profiling set (Argyll targen + greys and ramps) at
            full backlight, one spotread kept open, missed patches read again;
            .csv + Argyll .ti3 to local/screen-measure/c6/. --ti1 <file>
            other patches, --table <file> through a 17-point table, --light
            found|<0-432>, --name <text>, --dry-run (plan only)
  calibrate <readings.ti3> 17-point tables (native, 9000 K, 7500 K, D65
            white; BT.1886 greys) from a profiling run, with a report.
            --verify native,9000,6504 [--passes n]: each table's greys read on
            the Axim, corrected, read again (n passes, default 2; hands-free)
  emu-refresh the refresh-rate test's check (rp-refresh: frames counted, no PLL
            switch without Windows Mobile's value): run it before `load rp-refresh`
  emu-display-cal the display calibration image's check (rp-display-cal: the
            picture pixel-exact through each gamma table its dials set)
  emu-tone  the audio test image (rp-tone): the codec behind its GPIO82 gate,
            440 Hz to the speaker, the GPIO96 jack, 1000 Hz to headphones
  emu-audio-s1  the S1 image's check (rp-audio-s1: three notes at
            44.1 kHz, the jack re-routes while up, the restart guard, no
            power-up for a jack change while down)
  emu-silent-pc  a PC silent after the loader's first plug-in (the fork's
            pxa27x-udc.ignore-plugs=1): the loader's retry must get it
            configured
  emu-audio-s2  the mixer's check (rp-audio-s2: two notes from 22.05 and
            48 kHz recorded at the same pitch back to back, taps and clicks
            within 10 ms, long sleeps with music alone, nothing late or dry,
            the limiter on the overload)
  emu-audio-e3  ADR-0032 A6, audit E3 (rp-audio-s2 built with
            RP_AUDIO_E3): a 400 ms stall mid-music recorded as silence then
            the music carrying on, never a repeat; five forced races with
            no gap
  emu-audio-sleep  the codec's sleep (ADR-0032 R5, S1b): rp-audio-s1 built
            with RP_AUDIO_SLEEP: the idle sleep with I2S kept on, a sound
            with no wake first, three taps that wake it early
  emu-dma-restart  ADR-0032 A6's bench (rp-tone built with
            RP_TONE_DMA_RESTART): the audio DMA stopped at a STOP link
            while I2S runs, restarted, every try moving and finishing
  emu-looks the C6 looks test's check (rp-looks: photos, chart and ramps
            pixel-exact through each measured table)
  emu-drive only the drive check (the card as a USB drive, its blocks by DMA)
  emu-usb-suite every emulator check that goes through the USB controller
  emu-fullsize the loader's card commands on 4 GB and 32 GB card images
            (sparse files, 90% full), as the owner's cards (item 57)
  emu-trace only the trace-over-USB check (rp-threads under the loader)
  emu-cf    only the CompactFlash check (slot power through the CPLD,
            the card as the PC's second drive)
  emu-stale only the stale-view check (a PC write from before an image's
            run is refused; the drive goes away; no file lost)
  emu-cardlog  the card log for every run: on the SD card line for line, moved
            to the CF card when a program takes the SD card, closed by the
            loader (returned, crashed, a watchdog reset), old files cleaned
  emu-storfast the storage speed-ups' device test (rp-storfast) under the
            loader with SD and CF cards: DMA sizes, the write soak, CF paths
  mutants   the deliberate bugs each test suite must catch (docs/testing.md);
            --emu adds the slow emulator ones; RETROPANE_MUTANT=<text> only those
            whose name holds it
  bench     deterministic emulator benchmarks vs bench/baseline-emulator.json
            (--update records a new baseline)";

fn main() -> ExitCode {
    let cmd = args().nth(1).unwrap_or_default();
    let flag = args().nth(2).unwrap_or_default();
    let result = match cmd.as_str() {
        "ci" => [
            "eol",
            "fmt",
            "clippy",
            "lint",
            "gate",
            "reminders",
            "adrs",
            "test",
            "cross",
        ]
        .iter()
        .try_for_each(|c| run_step(c)),
        "eol" | "reminders" | "fmt" | "clippy" | "lint" | "gate" | "adrs" | "test" | "cross"
        | "coverage" | "emu" | "emu-picture" | "emu-blend" | "emu-blend-scale" | "emu-mbx"
        | "emu-mbx-blend" | "emu-mbx3d" | "emu-glesfeat" | "emu-mbx3d-api" | "emu-gpubench"
        | "emu-cs3bus" | "emu-cpldbus" | "emu-mved" | "emu-clocks" | "emu-noise" | "emu-today"
        | "emu-crash" | "emu-probe" | "emu-cf" | "emu-kernel" | "emu-quantum" | "emu-threads"
        | "emu-trace" | "emu-fullsize" | "emu-usb" | "emu-stale" | "emu-drive"
        | "emu-usb-suite" | "emu-storfast" | "emu-loader" | "emu-handover" | "emu-nightlight"
        | "emu-cfdetect" | "emu-display-cal" | "emu-looks" | "emu-tone" | "emu-audio-s1"
        | "emu-audio-sleep" | "emu-dma-restart" | "emu-audio-s2" | "emu-audio-e3"
        | "emu-silent-pc" | "emu-refresh" | "emu-screen-measure" | "emu-unplug" | "emu-keepfed"
        | "emu-cardlog" | "mutants" | "qemu" => run_step(&cmd),
        "sdkit" => {
            eprintln!("==> sdkit");
            cross()
                .and_then(|()| sdkit::build(&workspace_root()))
                .and_then(|()| {
                    if flag == "--serve" {
                        serve::serve(&workspace_root().join("target").join("sdkit"))
                    } else {
                        Ok(())
                    }
                })
        }
        "axim" => devlock::status(&devlock::dir()).map(|s| println!("{s}")),
        "usb" => {
            eprintln!("==> usb");
            usb::run()
        }
        "adr-index" => adrs::write(&workspace_root()),
        "perf" => {
            eprintln!("==> perf {flag}");
            if flag == "noise" {
                match args().nth(3).map(|n| n.parse::<usize>()) {
                    Some(Ok(n)) => perf::noise(&workspace_root(), n),
                    _ => Err("usage: cargo xtask perf noise <runs>".into()),
                }
            } else if flag == "refloor" {
                match args().nth(3).map(|n| n.parse::<usize>()) {
                    Some(Ok(n)) => perf::refloor(&workspace_root(), n),
                    _ => Err("usage: cargo xtask perf refloor <saved noise runs>".into()),
                }
            } else if flag.is_empty() || flag == "--update" {
                perf::check(&workspace_root(), flag == "--update")
            } else {
                Err("usage: cargo xtask perf [--update] | perf noise <runs>".into())
            }
        }
        "codegen-variant" => {
            let rest: Vec<String> = args().skip(3).collect();
            codegen_variant(&flag, &rest)
        }
        "load" => {
            eprintln!("==> load {flag}");
            if flag == "--setup" {
                usb::setup_restart_task(&workspace_root())
            } else if flag.is_empty() {
                Err("usage: cargo xtask load <image name | file.bin> | --setup".into())
            } else {
                let force = args().skip(3).any(|a| a == "--force");
                usb::load(&workspace_root(), &flag, force)
            }
        }
        "usbbench" => {
            eprintln!("==> usbbench {flag}");
            if flag == "drivers" {
                usbbench::drivers()
            } else if flag == "latency" {
                usbbench::latency(&workspace_root())
            } else if flag == "trial" {
                let image = args().nth(3).unwrap_or_else(|| "rp-loader".into());
                let runs = args().nth(4).and_then(|n| n.parse().ok()).unwrap_or(3);
                usbbench::trial(&workspace_root(), &image, runs)
            } else {
                let runs = args().nth(3).and_then(|n| n.parse().ok()).unwrap_or(3);
                usbbench::run(&workspace_root(), &flag, runs)
            }
        }
        "put" => {
            eprintln!("==> put {flag}");
            if flag.is_empty() {
                Err("usage: cargo xtask put <file> [name on the card]".into())
            } else {
                usb::put(&workspace_root(), &flag, args().nth(3).as_deref())
            }
        }
        "drive" => {
            eprintln!("==> drive {flag}");
            match flag.as_str() {
                "rw" | "ro" | "on" | "off" | "status" => {
                    usb::command(&workspace_root(), &format!("drive {flag}"))
                }
                _ => Err("usage: cargo xtask drive rw|ro|on|off|status".into()),
            }
        }
        "card" => {
            // Diagnostics: `cargo xtask card scan <lba> <count>`.
            let rest: Vec<String> = args().skip(2).collect();
            eprintln!("==> card {}", rest.join(" "));
            usb::command(&workspace_root(), &format!("card {}", rest.join(" ")))
        }
        "backlight" => {
            eprintln!("==> backlight {flag}");
            usb::command(&workspace_root(), &format!("backlight {flag}"))
        }
        "ls" => {
            eprintln!("==> ls {flag}");
            usb::ls(&workspace_root(), &flag)
        }
        "get" => {
            eprintln!("==> get {flag}");
            if flag.is_empty() {
                Err("usage: cargo xtask get <file on the card> [local name]".into())
            } else {
                usb::get(&workspace_root(), &flag, args().nth(3).as_deref())
            }
        }
        "rm" => {
            let rest: Vec<String> = args().skip(2).collect();
            eprintln!("==> rm {}", rest.join(" "));
            if rest.is_empty() {
                Err("usage: cargo xtask rm <file or empty folder on the card>".into())
            } else {
                usb::command(&workspace_root(), &format!("rm {}", rest.join(" ")))
            }
        }
        "cf" => {
            // `cf soft`: without the hardware reset; `cf log`: the slot's
            // timestamped events (diagnostics only).
            // `cf trace`: the drive's commands since the last pull (R33).
            let cmd = match args().nth(2).as_deref() {
                Some("soft") => "cf soft",
                Some("log") => "cf log",
                Some("trace") => "cf trace",
                _ => "cf",
            };
            eprintln!("==> {cmd}");
            if cmd == "cf trace" {
                usb::cf_trace(&workspace_root())
            } else {
                usb::command(&workspace_root(), cmd)
            }
        }
        "stack" => {
            eprintln!("==> loader stack");
            usb::command(&workspace_root(), "loader stack")
        }
        "log" => {
            eprintln!("==> log");
            usb::last_log(&workspace_root())
        }
        "cardlog" => {
            let image = (!flag.is_empty()).then_some(flag.as_str());
            let number = args().nth(3).map(|n| n.parse::<u32>()).transpose();
            eprintln!("==> cardlog {flag}");
            match number {
                Ok(n) => usb::cardlog(&workspace_root(), image, n),
                Err(_) => Err("usage: cargo xtask cardlog [image] [number]".into()),
            }
        }
        "measure" => {
            eprintln!("==> measure");
            if flag == "--report" {
                match args().nth(3) {
                    Some(csv) => measure::report(Path::new(&csv)),
                    None => Err("usage: cargo xtask measure --report <readings.csv>".into()),
                }
            } else if matches!(flag.as_str(), "--profile" | "--ti1" | "--table" | "--light") {
                measure_options().and_then(|o| profile::run(&workspace_root(), &o))
            } else {
                measure::measure(&workspace_root())
            }
        }
        "calibrate" => {
            // C6 step 4 (docs/design/display-calibration.md): 17-point
            // tables from a profiling run's readings.
            eprintln!("==> calibrate");
            let a: Vec<String> = args().skip(2).collect();
            let usage = "usage: cargo xtask calibrate <readings.ti3> [--verify native,9000,6504 [--passes n]]";
            match a.as_slice() {
                [ti3] => calib::run(Path::new(ti3)),
                [ti3, v, whites, rest @ ..] if v == "--verify" => {
                    let passes = match rest {
                        [] => Ok(2),
                        [p, n] if p == "--passes" => n
                            .parse::<u32>()
                            .ok()
                            .filter(|n| (1..=5).contains(n))
                            .ok_or_else(|| "--passes: 1-5".to_string()),
                        _ => Err(usage.to_string()),
                    };
                    passes.and_then(|passes| {
                        let w: Vec<calib::White> = whites
                            .split(',')
                            .map(calib::parse_white)
                            .collect::<Result<_>>()?;
                        calib::verify(&workspace_root(), Path::new(ti3), &w, passes)
                    })
                }
                _ => Err(usage.into()),
            }
        }
        "udc" => {
            // Diagnostics: `cargo xtask udc stats` (item 74, docs/hw/udc.md).
            eprintln!("==> udc {flag}");
            usb::command(&workspace_root(), &format!("udc {flag}"))
        }
        "cpld" => {
            eprintln!("==> cpld");
            usb::command(&workspace_root(), "cpld")
        }
        "handoff" => {
            eprintln!("==> handoff");
            usb::command(&workspace_root(), "handoff")
        }
        "board" => {
            eprintln!("==> board");
            usb::command(&workspace_root(), "board")
        }
        "trace" => {
            eprintln!("==> trace");
            trace::run(&workspace_root())
        }
        "wmbench" => {
            // The stock Windows Mobile baseline program (tools/wmbench),
            // built with cegcc in WSL; run by hand under Windows Mobile.
            eprintln!("==> wmbench");
            std::fs::create_dir_all(workspace_root().join("target/wmbench"))
                .map_err(|e| e.to_string())
                .and_then(|()| {
                    emu::wsl_run(&format!(
                        "cd '{}' && ~/cegcc/bin/arm-mingw32ce-gcc -O2 -mcpu=xscale -Wall -s \
                         -Wl,--major-subsystem-version,5,--minor-subsystem-version,1 \
                         -o target/wmbench/rp-wmbench.exe tools/wmbench/rp-wmbench.c && \
                         ~/cegcc/bin/arm-mingw32ce-gcc -O2 -mcpu=xscale -Wall -s \
                         -Wl,--major-subsystem-version,5,--minor-subsystem-version,1 \
                         -o target/wmbench/rp-wmgpu.exe tools/wmbench/rp-wmgpu.c && \
                         ~/cegcc/bin/arm-mingw32ce-gcc -O2 -mcpu=xscale -Wall -s \
                         -Wl,--major-subsystem-version,5,--minor-subsystem-version,1 \
                         -o target/wmbench/rp-wm3d.exe tools/wmbench/rp-wm3d.c -ltoolhelp &&                          ~/cegcc/bin/arm-mingw32ce-gcc -O2 -mcpu=xscale -Wall -s                          -Wl,--major-subsystem-version,5,--minor-subsystem-version,1                          -o target/wmbench/rp-wmaudio.exe tools/wmbench/rp-wmaudio.c -lmmtimer &&                          ~/cegcc/bin/arm-mingw32ce-gcc -O2 -mcpu=xscale -Wall -s                          -Wl,--major-subsystem-version,5,--minor-subsystem-version,1                          -o target/wmbench/rp-wmpower.exe tools/wmbench/rp-wmpower.c &&                          ~/cegcc/bin/arm-mingw32ce-gcc -O2 -mcpu=xscale -Wall -s                          -Wl,--major-subsystem-version,5,--minor-subsystem-version,1                          -o target/wmbench/rp-wmtones.exe tools/wmbench/rp-wmtones.c &&                          ~/cegcc/bin/arm-mingw32ce-gcc -O2 -mcpu=xscale -Wall -s                          -Wl,--major-subsystem-version,5,--minor-subsystem-version,1                          -o target/wmbench/rp-wmprobe.exe tools/wmbench/rp-wmprobe.c",
                        emu::to_wsl_path(&workspace_root())
                    ))
                })
                .map(|()| {
                    eprintln!("    target/wmbench/rp-wmbench.exe: put it in the SD root (cargo xtask put target/wmbench/rp-wmbench.exe rp-wmbench.exe), run it under Windows Mobile, then fetch rp-wm-baseline.txt");
                    eprintln!("    target/wmbench/rp-wmgpu.exe (item 69, what draws WM's screen): the same way; its log is rp-wm-gpu.txt");
                    eprintln!("    target/wmbench/rp-wmtones.exe (the A/B notes through Dell's driver, docs/hw/audio.md): into its own folder (cargo xtask put target/wmbench/rp-wmtones.exe rp-wmtones/rp-wmtones.exe), run under Windows Mobile, headphones out; its log is rp-wmtones/rp-wm-tones.txt");
                    eprintln!("    target/wmbench/rp-wmpower.exe (ADR-0032 S1, the idle audio power): into its own folder (cargo xtask put target/wmbench/rp-wmpower.exe rp-wmpower/rp-wmpower.exe), run on battery under Windows Mobile; its log is rp-wmpower/rp-wm-power.txt");
                    eprintln!("    target/wmbench/rp-wmaudio.exe (docs/hw/audio.md): into its own folder (cargo xtask put target/wmbench/rp-wmaudio.exe rp-wmaudio/rp-wmaudio.exe), run under Windows Mobile; its log is rp-wmaudio/rp-wm-audio.txt");
                    eprintln!("    target/wmbench/rp-wmprobe.exe (the Windows Mobile probe and baseline suite): into its own folder with rp-wmbench.exe beside it (cargo xtask put target/wmbench/rp-wmprobe.exe rp-wmprobe/rp-wmprobe.exe, and rp-wmbench.exe to rp-wmprobe/rp-wmbench.exe), run under Windows Mobile, soft-reset when it says, run it again; its log is rp-wmprobe/rp-wmprobe.txt");
                    eprintln!("    target/wmbench/rp-wm3d.exe (ADR-0024 M3): the same way, then start DoomGLES; its log is rp-wm3d.txt, its copies rp-wm3d-1.bin and -2.bin (16 MiB each)");
                })
        }
        "g7-frame" => {
            // Test G7's frame out of rp-wmprobe v4's capture (xtask/src/g7.rs).
            eprintln!("==> g7-frame");
            let rest: Vec<String> = args().skip(2).collect();
            g7::run(&workspace_root(), &rest)
        }
        "reboot-wm" => {
            eprintln!("==> reboot wm");
            usb::command(&workspace_root(), "reboot wm")
        }
        "tone" => {
            eprintln!("==> tone (rp-tone)");
            usb::load(&workspace_root(), "rp-tone", false)
        }
        "audio-s1" => {
            eprintln!("==> audio-s1 (rp-audio-s1)");
            usb::load(&workspace_root(), "rp-audio-s1", false)
        }
        "audio-s2" => {
            eprintln!("==> audio-s2 (rp-audio-s2)");
            usb::load(&workspace_root(), "rp-audio-s2", false)
        }
        "device-check" => {
            eprintln!("==> device-check {flag}");
            usb::device_check(&workspace_root(), flag == "--with-owner")
        }
        "install" => {
            eprintln!("==> install");
            // Always from a fresh kit: an old target/sdkit once put a stale
            // loader on the card while reporting "up to date".
            cross()
                .and_then(|()| sdkit::build(&workspace_root()))
                .and_then(|()| match flag.as_str() {
                    "" | "sd" => usb::install(&workspace_root()),
                    "cf" => usb::install_cf(&workspace_root()),
                    _ => Err("usage: cargo xtask install [sd|cf]".into()),
                })
        }
        "symbols" => {
            let addrs: Vec<String> = args().skip(3).collect();
            symbols::symbols(&workspace_root(), &flag, &addrs).map(|lines| {
                for l in lines {
                    println!("{l}");
                }
            })
        }
        "bench" => {
            eprintln!("==> bench");
            cross().and_then(|()| emu::bench(&workspace_root(), flag == "--update"))
        }
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask: {e}");
            ExitCode::FAILURE
        }
    }
}

type Result<T = ()> = std::result::Result<T, String>;

/// One emulator check of the `emu` suite.
type EmuCheck = fn(&std::path::Path) -> Result;

/// The command line without `--no-wait` (devlock reads that one itself),
/// so it can go anywhere without shifting the positional arguments.
fn args() -> impl Iterator<Item = String> {
    std::env::args().filter(|a| a != devlock::NO_WAIT_FLAG)
}

fn run_step(step: &str) -> Result {
    eprintln!("==> {step}");
    match step {
        "fmt" => cargo(&["fmt", "--all", "--check"]),
        "clippy" => cargo(&[
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ]),
        "eol" => eol::check(&workspace_root()),
        "reminders" => reminders::run(&workspace_root()),
        "lint" => lint(),
        "gate" => gate::run(&workspace_root()),
        "adrs" => adrs::check(&workspace_root()),
        "test" => cargo(&["test", "--workspace"]),
        "cross" => cross(),
        "emu-picture" => cross().and_then(|()| emu::boot_loader_picture(&workspace_root())),
        "emu-blend" => cross().and_then(|()| emu::boot_loader_blend(&workspace_root())),
        "emu-blend-scale" => cross().and_then(|()| emu::boot_loader_blend_scale(&workspace_root())),
        "emu-mbx" => cross().and_then(|()| emu::boot_loader_mbx(&workspace_root())),
        "emu-mbx-blend" => cross().and_then(|()| emu::boot_loader_mbx_blend(&workspace_root())),
        "emu-mbx3d" => cross().and_then(|()| emu::boot_loader_mbx3d(&workspace_root())),
        "emu-glesfeat" => cross().and_then(|()| emu::boot_loader_glesfeat(&workspace_root())),
        "emu-unplug" => cross().and_then(|()| emu::boot_loader_unplug(&workspace_root())),
        "emu-mbx3d-api" => cross().and_then(|()| emu::boot_loader_mbx3d_api(&workspace_root())),
        "emu-gpubench" => cross().and_then(|()| emu::boot_loader_gpubench(&workspace_root())),
        "emu-cs3bus" => cross().and_then(|()| emu::boot_loader_cs3bus(&workspace_root())),
        "emu-cpldbus" => cross().and_then(|()| emu::boot_loader_cpldbus(&workspace_root())),
        "emu-mved" => cross().and_then(|()| emu::boot_loader_mved(&workspace_root())),
        "emu-clocks" => cross().and_then(|()| emu::boot_loader_clocks(&workspace_root())),
        "emu-noise" => cross().and_then(|()| emu::boot_loader_noise(&workspace_root())),
        "emu-today" => cross()
            .and_then(|()| emu::boot_loader_today(&workspace_root()))
            .and_then(|()| emu::today_replay(&workspace_root())),
        "emu-crash" => cross().and_then(|()| emu::boot_loader_crash(&workspace_root())),
        "emu-probe" => cross().and_then(|()| emu::boot_probe(&workspace_root())),
        "emu-cf" => cross().and_then(|()| emu::boot_loader_cf_alone(&workspace_root())),
        "emu-stale" => cross().and_then(|()| emu::boot_loader_stale_view_alone(&workspace_root())),
        "emu-kernel" => cross().and_then(|()| emu::boot_kernel(&workspace_root())),
        "emu-quantum" => cross().and_then(|()| emu::boot_quantum(&workspace_root())),
        "emu-threads" => cross().and_then(|()| emu::boot_threads(&workspace_root())),
        "emu-trace" => cross().and_then(|()| emu::boot_loader_trace(&workspace_root())),
        "emu-fullsize" => cross().and_then(|()| emu::boot_loader_full_size(&workspace_root())),
        "emu-usb" => cross().and_then(|()| emu::boot_loader_usb_alone(&workspace_root())),
        "emu-loader" => cross().and_then(|()| emu::boot_loader(&workspace_root())),
        "emu-keepfed" => cross().and_then(|()| emu::boot_loader_keeps_feeding(&workspace_root())),
        "emu-handover" => cross().and_then(|()| emu::boot_loader_handover(&workspace_root())),
        "emu-nightlight" => cross().and_then(|()| emu::boot_loader_nightlight(&workspace_root())),
        "emu-cfdetect" => cross().and_then(|()| emu::boot_loader_cfdetect(&workspace_root())),
        "emu-display-cal" => cross().and_then(|()| emu::boot_loader_display_cal(&workspace_root())),
        "emu-looks" => cross().and_then(|()| emu::boot_loader_looks(&workspace_root())),
        "emu-tone" => cross().and_then(|()| emu::boot_loader_tone(&workspace_root())),
        "emu-audio-s1" => cross().and_then(|()| emu::boot_loader_audio_s1(&workspace_root())),
        "emu-audio-sleep" => cross().and_then(|()| emu::boot_loader_audio_sleep(&workspace_root())),
        "emu-dma-restart" => cross().and_then(|()| emu::boot_loader_dma_restart(&workspace_root())),
        "emu-audio-s2" => cross().and_then(|()| emu::boot_loader_audio_s2(&workspace_root())),
        "emu-audio-e3" => cross().and_then(|()| emu::boot_loader_audio_e3(&workspace_root())),
        "emu-silent-pc" => cross().and_then(|()| emu::boot_loader_silent_pc(&workspace_root())),
        "emu-refresh" => cross().and_then(|()| emu::boot_loader_refresh(&workspace_root())),
        "emu-screen-measure" => {
            cross().and_then(|()| emu::boot_loader_screen_measure(&workspace_root()))
        }
        "emu-storfast" => cross().and_then(|()| emu::boot_loader_storfast(&workspace_root())),
        "emu-cardlog" => cross().and_then(|()| emu::boot_loader_cardlog(&workspace_root())),
        "emu-drive" => cross().and_then(|()| emu::boot_loader_drive_alone(&workspace_root())),
        // Every check that goes through the USB controller, in `emu`'s order
        // (UDC work: the rest of the suite doesn't touch it).
        "emu-usb-suite" => cross()
            .and_then(|()| emu::boot_loader(&workspace_root()))
            .and_then(|()| emu::boot_usb_link(&workspace_root()))
            .and_then(|()| emu::boot_loader_usb(&workspace_root()))
            .and_then(|()| emu::boot_loader_trace(&workspace_root()))
            .and_then(|()| emu::boot_loader_put(&workspace_root()))
            .and_then(|()| emu::boot_loader_put_exfat(&workspace_root()))
            .and_then(|()| emu::boot_loader_cf(&workspace_root()))
            .and_then(|()| emu::boot_loader_drive(&workspace_root()))
            .and_then(|()| emu::boot_loader_fallback(&workspace_root()))
            .and_then(|()| emu::boot_loader_locked(&workspace_root()))
            .and_then(|()| emu::boot_loader_backlight(&workspace_root()))
            .and_then(|()| emu::boot_loader_full_size(&workspace_root())),
        "mutants" => mutants::run(&workspace_root(), args().any(|a| a == "--emu")),
        "emu" => cross().and_then(|()| {
            // `RETROPANE_EMU_FROM=<check>` resumes the suite at that check
            // (its function's name), after a known flake stopped it
            // (docs/quality-audit.md C25).
            let checks: &[(&str, EmuCheck)] = &[
                ("boot_hello", emu::boot_hello),
                ("boot_probe", emu::boot_probe),
                ("boot_kernel", emu::boot_kernel),
                ("boot_threads", emu::boot_threads),
                ("boot_quantum", emu::boot_quantum),
                ("boot_light", emu::boot_light),
                ("boot_usb", emu::boot_usb),
                ("boot_loader", emu::boot_loader),
                ("boot_loader_hang", emu::boot_loader_hang),
                ("boot_loader_run_limit", emu::boot_loader_run_limit),
                ("boot_loader_picture", emu::boot_loader_picture),
                ("boot_loader_blend", emu::boot_loader_blend),
                ("boot_loader_mbx", emu::boot_loader_mbx),
                ("boot_loader_mbx_blend", emu::boot_loader_mbx_blend),
                ("boot_loader_mbx3d", emu::boot_loader_mbx3d),
                ("boot_loader_glesfeat", emu::boot_loader_glesfeat),
                ("boot_loader_mbx3d_api", emu::boot_loader_mbx3d_api),
                ("boot_loader_gpubench", emu::boot_loader_gpubench),
                ("boot_loader_cs3bus", emu::boot_loader_cs3bus),
                ("boot_loader_cpldbus", emu::boot_loader_cpldbus),
                ("boot_loader_mved", emu::boot_loader_mved),
                ("boot_loader_clocks", emu::boot_loader_clocks),
                ("boot_loader_noise", emu::boot_loader_noise),
                ("boot_loader_today", emu::boot_loader_today),
                ("boot_loader_crash", emu::boot_loader_crash),
                ("boot_loader_handover", emu::boot_loader_handover),
                ("boot_loader_nightlight", emu::boot_loader_nightlight),
                ("boot_loader_cfdetect", emu::boot_loader_cfdetect),
                ("boot_loader_display_cal", emu::boot_loader_display_cal),
                ("boot_loader_looks", emu::boot_loader_looks),
                ("boot_loader_refresh", emu::boot_loader_refresh),
                ("boot_usb_link", emu::boot_usb_link),
                ("boot_loader_usb", emu::boot_loader_usb),
                ("boot_loader_trace", emu::boot_loader_trace),
                ("boot_loader_put", emu::boot_loader_put),
                ("boot_loader_put_exfat", emu::boot_loader_put_exfat),
                ("boot_loader_cf", emu::boot_loader_cf),
                ("boot_loader_tone", emu::boot_loader_tone),
                ("boot_loader_audio_s1", emu::boot_loader_audio_s1),
                ("boot_loader_audio_sleep", emu::boot_loader_audio_sleep),
                ("boot_loader_dma_restart", emu::boot_loader_dma_restart),
                ("boot_loader_audio_s2", emu::boot_loader_audio_s2),
                ("boot_loader_audio_e3", emu::boot_loader_audio_e3),
                ("boot_loader_silent_pc", emu::boot_loader_silent_pc),
                ("boot_loader_drive", emu::boot_loader_drive),
                ("boot_loader_stale_view", emu::boot_loader_stale_view),
                ("boot_loader_fallback", emu::boot_loader_fallback),
                ("boot_loader_locked", emu::boot_loader_locked),
                ("boot_loader_backlight", emu::boot_loader_backlight),
                ("boot_loader_full_size", emu::boot_loader_full_size),
                ("boot_loader_keeps_feeding", emu::boot_loader_keeps_feeding),
                ("boot_loader_cardlog", emu::boot_loader_cardlog),
            ];
            let from = std::env::var("RETROPANE_EMU_FROM").ok();
            let start = match from.as_deref() {
                Some(name) => checks
                    .iter()
                    .position(|(n, _)| *n == name)
                    .ok_or_else(|| format!("RETROPANE_EMU_FROM: no check named {name}"))?,
                None => 0,
            };
            for (_, check) in &checks[start..] {
                check(&workspace_root())?;
            }
            // The benchmark image's on-target checks (pixel paths, memory,
            // crypto, SD) and no regression past the baseline.
            emu::bench(&workspace_root(), false)
        }),
        "qemu" => emu::build_wsl(&workspace_root()),
        "coverage" => cargo(&[
            "llvm-cov",
            "--workspace",
            "--exclude",
            "xtask",
            "--exclude",
            "rp-hello",
            "--exclude",
            "rp-bench",
            "--exclude",
            "rp-bench-suite",
            "--exclude",
            "rp-kernel",
            "--exclude",
            "rp-threads",
            "--exclude",
            "rp-quantum",
            // CLI glue; its logic lives in the tested romdump library.
            "--ignore-filename-regex",
            r"romdump.src.main\.rs",
            "--fail-under-lines",
            "80",
        ]),
        _ => unreachable!(),
    }
}

fn cargo(args: &[&str]) -> Result {
    cargo_with(&mut Command::new(""), args)
}

/// [`cargo`] with `cmd`'s environment (a [`frame_pointer_build`]'s flags):
/// only its environment is used.
fn cargo_with(cmd: &mut Command, args: &[&str]) -> Result {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut run = Command::new(cargo);
    for (k, v) in cmd.get_envs() {
        match v {
            Some(v) => run.env(k, v),
            None => run.env_remove(k),
        };
    }
    let status = run
        .args(args)
        .env("RP_BUILD_ID", build_id())
        .env(local_input::VAR, local_inputs(&workspace_root()))
        .current_dir(workspace_root())
        .status()
        .map_err(|e| format!("failed to run cargo: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`cargo {}` failed", args.join(" ")))
    }
}

/// The build scripts' `local/` helper (quality audit C40), for its folder
/// list and variable name.
mod local_input {
    include!("../../tools/build/local_input.rs");
    pub const DIRS: &[&str] = LOCAL_INPUT_DIRS;
    pub const VAR: &str = LOCAL_INPUTS_VAR;
}

/// Quality audit C40: a fingerprint of every file the build scripts read
/// from `local/` (its path, length and modification time), given to every
/// cargo build as `RP_LOCAL_INPUTS`. The scripts watch the variable, so a
/// picture overwritten in place by an older-dated copy, which cargo's time
/// comparison misses, still reruns them. Times alone would do for a copy;
/// a same-length file given its predecessor's exact time is the one change
/// it can't see (only by setting the time on purpose).
fn local_inputs(root: &Path) -> String {
    use std::hash::{Hash, Hasher};
    fn files(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            // Followed, not the entry's own type: `local/` may be a link.
            if p.is_dir() {
                files(&p, out);
            } else {
                out.push(p);
            }
        }
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for dir in local_input::DIRS {
        dir.hash(&mut h);
        let mut list = Vec::new();
        files(&root.join(dir), &mut list);
        list.sort();
        for f in list {
            f.strip_prefix(root).unwrap_or(&f).hash(&mut h);
            let meta = std::fs::metadata(&f).ok();
            meta.as_ref().map(std::fs::Metadata::len).hash(&mut h);
            meta.and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .hash(&mut h);
        }
    }
    format!("{:016x}", h.finish())
}

/// Gives a cargo command the `local/` inputs' fingerprint (C40).
fn with_local_inputs(cmd: &mut Command) -> &mut Command {
    cmd.env(local_input::VAR, local_inputs(&workspace_root()))
}

/// `cargo xtask codegen-variant <image> <label> <--config ...>...`: the gate's
/// C9 comparisons (code generation) built the same way every time. The
/// image is built with the extra cargo `--config` settings in
/// `target/variants/<label>/`, and its raw binary written there for
/// `cargo xtask load <file.bin>`. Its size is reported against the image's
/// budget but not held to it: a measurement variant is never shipped.
fn codegen_variant(image: &str, rest: &[String]) -> Result {
    let (Some(label), configs) = (rest.first(), &rest[rest.len().min(1)..]) else {
        return Err("usage: cargo xtask codegen-variant <image> <label> <--config ...>...".into());
    };
    if image.is_empty()
        || !label
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        || label.is_empty()
    {
        return Err("codegen-variant: an image name and a label of letters, digits and -".into());
    }
    let mut env = Command::new("");
    let dir = frame_pointer_build(&mut env, image, &format!("target/variants/{label}"));
    let mut args: Vec<&str> = vec![
        "build",
        "--release",
        "--target",
        TARGET_SPEC,
        "-Zbuild-std=core,compiler_builtins",
        "-Zjson-target-spec",
        "-p",
        image,
        "--target-dir",
        &dir,
    ];
    for c in configs {
        args.extend(["--config", c.as_str()]);
    }
    eprintln!("==> codegen-variant {image} {label}: {}", configs.join(" "));
    cargo_with(&mut env, &args)?;
    let root = workspace_root();
    let elf = root
        .join(&dir)
        .join("armv5te-x51v-none-eabi/release")
        .join(image);
    let bin = root.join(&dir).join(format!("{image}.bin"));
    sdkit::raw_binary(&elf, &bin)?;
    let bytes = std::fs::read(&elf).map_err(|e| format!("{}: {e}", elf.display()))?;
    let s = size::measure(&bytes)?;
    let budget = size::BUDGETS.iter().find(|b| b.image == image);
    eprintln!(
        "    {image} ({label}): {} B loaded{}, {} B zeroed RAM; load it with: cargo xtask load {}",
        s.load,
        budget.map_or(String::new(), |b| format!(
            " (the shipped image's budget {}, not held: a measurement variant)",
            b.max_load
        )),
        s.zero,
        bin.display()
    );
    Ok(())
}

/// The commit being built (short hash, `+` if there are uncommitted
/// changes), so a photo of the probe report names its exact build.
fn build_id() -> String {
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(workspace_root())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let hash = git(&["rev-parse", "--short=8", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let dirty = git(&["status", "--porcelain"]).is_some_and(|s| !s.is_empty());
    if dirty { format!("{hash}+") } else { hash }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives one level below the workspace root")
        .to_path_buf()
}

/// A workspace crate, as far as the policy checks care.
struct Crate {
    name: String,
    place: Placement,
    lib_root: Option<PathBuf>,
    deps: Vec<(String, DepKind)>,
}

fn workspace_crates() -> Result<Vec<Crate>> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(workspace_root())
        .output()
        .map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err("cargo metadata failed".into());
    }
    let meta: Value = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
    let root = workspace_root();
    let mut crates = Vec::new();
    for pkg in meta["packages"].as_array().into_iter().flatten() {
        let name = pkg["name"].as_str().unwrap_or_default().to_string();
        let manifest = PathBuf::from(pkg["manifest_path"].as_str().unwrap_or_default());
        let dir = manifest.parent().unwrap_or(&root);
        let rel = dir
            .strip_prefix(&root)
            .map_err(|_| format!("{name} is outside the workspace"))?
            .to_string_lossy()
            .replace('\\', "/");
        let place = policy::placement(&rel)
            .ok_or_else(|| format!("{name} ({rel}) is not in a known layer directory"))?;
        let lib_root = pkg["targets"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|t| {
                t["kind"]
                    .as_array()
                    .is_some_and(|k| k.iter().any(|k| k == "lib"))
            })
            .and_then(|t| t["src_path"].as_str())
            .map(PathBuf::from);
        let deps = pkg["dependencies"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|d| {
                let kind = match d["kind"].as_str() {
                    Some("dev") => DepKind::Dev,
                    Some("build") => DepKind::Build,
                    _ => DepKind::Normal,
                };
                (d["name"].as_str().unwrap_or_default().to_string(), kind)
            })
            .collect();
        crates.push(Crate {
            name,
            place,
            lib_root,
            deps,
        });
    }
    Ok(crates)
}

fn lint() -> Result {
    let crates = workspace_crates()?;
    let mut violations = Vec::new();
    for c in &crates {
        for (dep, kind) in &c.deps {
            // Only workspace-internal edges are layered.
            if let Some(target) = crates.iter().find(|t| &t.name == dep) {
                violations.extend(policy::check_edge(
                    (&c.name, c.place),
                    (&target.name, target.place),
                    *kind,
                ));
            }
        }
        if let Some(lib) = &c.lib_root {
            let src =
                std::fs::read_to_string(lib).map_err(|e| format!("{}: {e}", lib.display()))?;
            violations.extend(policy::check_unsafe_policy(&c.name, c.place, &src));
        }
    }
    // Every tracked text file: no merge-conflict markers.
    let mut inherited = Vec::new();
    let tracked = Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(workspace_root())
        .output()
        .map_err(|e| format!("git ls-files: {e}"))?;
    for path in String::from_utf8_lossy(&tracked.stdout).split('\0') {
        if path.is_empty() {
            continue;
        }
        // Binary or unreadable files are skipped: markers are text.
        if let Ok(text) = std::fs::read_to_string(workspace_root().join(path)) {
            violations.extend(policy::check_conflict_markers(path, &text));
            // One crash path for every image (quality audit C38).
            violations.extend(policy::check_crash_path(path, &text));
            violations.extend(policy::check_unchecked_2d(path, &text));
            violations.extend(policy::check_mbxclk_writes(path, &text));
            // Copied values (quality-audit D1), in Rust sources outside xtask
            // (whose policy code names the tag itself).
            if path.ends_with(".rs") && !path.starts_with("xtask/") {
                let (tags, bad) = policy::inherited_tags(path, &text);
                inherited.extend(tags);
                violations.extend(bad);
            }
        }
    }
    let mut items: Vec<(&str, usize)> = Vec::new();
    for t in &inherited {
        match items.iter_mut().find(|(i, _)| *i == t.item) {
            Some((_, n)) => *n += 1,
            None => items.push((&t.item, 1)),
        }
    }
    eprintln!(
        "    {} copied values tagged `inherited:` (docs/research/imitation-audit.md){}",
        inherited.len(),
        if items.is_empty() {
            String::new()
        } else {
            format!(
                ": {}",
                items
                    .iter()
                    .map(|(i, n)| format!("{i} {n}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    );
    if violations.is_empty() {
        eprintln!("    {} crates, no policy violations", crates.len());
        Ok(())
    } else {
        for v in &violations {
            eprintln!("    violation: {v}");
        }
        Err(format!("{} policy violation(s)", violations.len()))
    }
}

/// `measure --profile | --ti1 <file>`, `--table <file>`, `--light found|<counts>`,
/// `--name <text>`, `--dry-run` (C6, docs/design/display-calibration.md).
fn measure_options() -> Result<profile::Options> {
    let a: Vec<String> = args().skip(2).collect();
    let mut o = profile::Options {
        name: "profile".into(),
        ti1: None,
        table: None,
        light: rp_screen_measure::program::StepLight::Counts(432),
        dry: false,
    };
    let mut i = 0;
    let value = |i: usize| {
        a.get(i + 1)
            .cloned()
            .ok_or(format!("{} needs a value", a[i]))
    };
    while i < a.len() {
        match a[i].as_str() {
            "--profile" => i += 1,
            "--dry-run" => {
                o.dry = true;
                i += 1;
            }
            "--ti1" => {
                o.ti1 = Some(value(i)?.into());
                i += 2;
            }
            "--table" => {
                o.table = Some(value(i)?.into());
                i += 2;
            }
            "--name" => {
                o.name = value(i)?;
                i += 2;
            }
            "--light" => {
                let v = value(i)?;
                o.light = if v == "found" {
                    rp_screen_measure::program::StepLight::Found
                } else {
                    let c: u16 = v
                        .parse()
                        .map_err(|_| format!("--light {v}: found or 0-432"))?;
                    if c > 432 {
                        return Err(format!("--light {v}: at most 432"));
                    }
                    rp_screen_measure::program::StepLight::Counts(c)
                };
                i += 2;
            }
            other => return Err(format!("measure: unknown option {other}")),
        }
    }
    Ok(o)
}

fn cross() -> Result {
    let crates = workspace_crates()?;
    // User process images first: the kernel image embeds them (its build
    // script reads `<name>.pe` next to them).
    let mut user = vec![
        "build",
        "--release",
        "--target",
        TARGET_SPEC,
        "-Zbuild-std=core,compiler_builtins",
        "-Zjson-target-spec",
    ];
    for name in userimg::IMAGES {
        user.extend(["-p", name]);
    }
    cargo(&user)?;
    userimg::package_all(&crate::arm_release(&workspace_root()))?;
    let mut args = vec![
        "build".to_string(),
        "--release".into(),
        "--target".into(),
        TARGET_SPEC.into(),
        "-Zbuild-std=core,compiler_builtins".into(),
        "-Zjson-target-spec".into(),
    ];
    for c in crates
        .iter()
        .filter(|c| matches!(c.place, Placement::Os(_)) && c.name != policy::MOCK_CRATE)
        // Built on their own above: a build's features are unified, and a
        // program's runtime (its panic handler) mustn't reach the kernel.
        .filter(|c| !userimg::IMAGES.contains(&c.name.as_str()))
    {
        args.push("-p".into());
        args.push(c.name.clone());
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    cargo(&refs)?;
    // ADR-0015: these images are built as Thumb code (the HAL's assembly
    // stays ARM, marked per function), then put where the ARM build put
    // them, so everything downstream (sizes, the emulator, the SD kit) uses
    // them unchanged. Reverting to all-ARM is emptying this list.
    for image in THUMB_IMAGES {
        cargo(&[
            "build",
            "--release",
            "--target",
            THUMB_TARGET_SPEC,
            "-Zbuild-std=core,compiler_builtins",
            "-Zjson-target-spec",
            "-p",
            image,
        ])?;
        let from = target_dir(&workspace_root())
            .join("thumbv5te-x51v-none-eabi/release")
            .join(image);
        let to = crate::arm_release(&workspace_root()).join(image);
        std::fs::copy(&from, &to).map_err(|e| format!("{}: {e}", from.display()))?;
    }
    // ADR-0001: these images are built with frame pointers (their crash
    // report walks the chain), in a folder of their own, then put where the
    // build above put them, as the Thumb ones are.
    for image in FRAME_POINTER_IMAGES {
        if THUMB_IMAGES.contains(image) {
            return Err(format!(
                "{image}: in both the Thumb and the frame-pointer lists (the walker follows ARM \
                 code's chain only)"
            ));
        }
        let mut env = Command::new("");
        let dir = frame_pointer_build(&mut env, image, "target");
        if dir == "target" {
            // `RP_MAY_OMIT=1`: built may-omit above, where it stays (a copy
            // onto itself could empty it).
            continue;
        }
        cargo_with(
            &mut env,
            &[
                "build",
                "--release",
                "--target",
                TARGET_SPEC,
                "-Zbuild-std=core,compiler_builtins",
                "-Zjson-target-spec",
                "-p",
                image,
                "--target-dir",
                &dir,
            ],
        )?;
        let from = workspace_root()
            .join(&dir)
            .join("armv5te-x51v-none-eabi/release")
            .join(image);
        let to = crate::arm_release(&workspace_root()).join(image);
        std::fs::copy(&from, &to).map_err(|e| format!("{}: {e}", from.display()))?;
    }
    // Bare-metal images: OS crates that are binaries only.
    let images: Vec<&str> = crates
        .iter()
        .filter(|c| matches!(c.place, Placement::Os(_)) && c.lib_root.is_none())
        .map(|c| c.name.as_str())
        .collect();
    let release = crate::arm_release(&workspace_root());
    size::check(&release, &images)?;
    frames::check(&release, &images)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    /// Class guard for stale images: no xtask source names the X51v's build
    /// folder under `target` itself; every path comes from `arm_release`.
    #[test]
    fn image_paths_come_from_arm_release() {
        let folders = [
            ["target/", "armv5te-x51v-none-eabi"].concat(),
            ["target/", "thumbv5te-x51v-none-eabi"].concat(),
        ];
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut found = Vec::new();
        for entry in std::fs::read_dir(&src).unwrap() {
            let path = entry.unwrap().path();
            let text = std::fs::read_to_string(&path).unwrap();
            for (n, line) in text.lines().enumerate() {
                if folders.iter().any(|f| line.contains(f.as_str())) {
                    found.push(format!("{}:{}", path.display(), n + 1));
                }
            }
        }
        assert!(
            found.is_empty(),
            "image paths outside arm_release: {found:?}"
        );
    }

    #[test]
    fn arm_release_follows_cargo_target_dir() {
        let root = Path::new("/w");
        // The variable is the process's; this test only reads it.
        let want = match std::env::var_os("CARGO_TARGET_DIR").filter(|d| !d.is_empty()) {
            Some(d) => root.join(d),
            None => root.join("target"),
        };
        assert_eq!(
            super::arm_release(root),
            want.join("armv5te-x51v-none-eabi/release")
        );
    }
}
