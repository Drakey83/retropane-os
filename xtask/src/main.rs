//! `cargo xtask <command>`: one entry point for every build, check and test,
//! used identically by developers and CI.

#![forbid(unsafe_code)]

mod calib;
mod devlock;
mod emu;
mod measure;
mod mutants;
mod policy;
mod profile;
mod screen;
mod sdkit;
mod serve;
mod size;
mod spot;
mod trace;
mod usb;
mod usbbench;
mod userimg;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use policy::{DepKind, Placement};
use serde_json::Value;

pub(crate) const TARGET_SPEC: &str = "targets/armv5te-x51v-none-eabi.json";
/// The Thumb variant of the target (ADR-0015: cold code as Thumb).
const THUMB_TARGET_SPEC: &str = "targets/thumbv5te-x51v-none-eabi.json";
/// Images built as Thumb (ADR-0015); empty it to go back to all-ARM.
const THUMB_IMAGES: &[&str] = &["rp-loader"];

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  ci        everything CI runs: fmt, clippy, lint, test, cross
  fmt       check formatting
  clippy    lint all crates (warnings are errors)
  lint      architecture policy: layering and unsafe rules (DESIGN.md §3.1)
  test      host unit tests
  cross     build every OS crate for the X51v (ARMv5TE)
  coverage  host test coverage (needs cargo-llvm-cov), 80% line floor
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
            (replaces the files already there; run cargo xtask sdkit first)
  drive     the SD card as a drive on the PC (ADR-0014): `drive rw` lets the
            PC write it, `drive ro` (the default at start) makes it read-only,
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
  log       the last image's run log again, from the loader (for a log
            lost on the way back; `load` asks for it by itself)
  cpld      the CPLD's registers as the loader found them at start-up
            (read-only diagnostics)
  handoff   which DMA channels the loader's start found running, how long
            it waited for it to stop, and its boot time
  trace    the last program's trace ring (thread switches, faults,
            calls...) from the loader, as a timeline (item 54)
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
  emu-storfast the storage speed-ups' device test (rp-storfast) under the
            loader with SD and CF cards: DMA sizes, the write soak, CF paths
  mutants   the deliberate bugs each test suite must catch (docs/testing.md);
            --emu adds the slow emulator ones
  bench     deterministic emulator benchmarks vs bench/baseline-emulator.json
            (--update records a new baseline)";

fn main() -> ExitCode {
    let cmd = args().nth(1).unwrap_or_default();
    let flag = args().nth(2).unwrap_or_default();
    let result = match cmd.as_str() {
        "ci" => ["fmt", "clippy", "lint", "test", "cross"]
            .iter()
            .try_for_each(|c| run_step(c)),
        "fmt" | "clippy" | "lint" | "test" | "cross" | "coverage" | "emu" | "emu-picture"
        | "emu-blend" | "emu-mbx" | "emu-mbx-blend" | "emu-mbx3d" | "emu-glesfeat"
        | "emu-mbx3d-api" | "emu-mved" | "emu-clocks" | "emu-noise" | "emu-today" | "emu-crash"
        | "emu-cf" | "emu-kernel" | "emu-quantum" | "emu-threads" | "emu-trace"
        | "emu-fullsize" | "emu-usb" | "emu-stale" | "emu-drive" | "emu-usb-suite"
        | "emu-storfast" | "emu-loader" | "emu-handover" | "emu-nightlight" | "emu-cfdetect"
        | "emu-display-cal" | "emu-looks" | "emu-tone" | "emu-audio-s1" | "emu-audio-sleep"
        | "emu-dma-restart" | "emu-audio-s2" | "emu-silent-pc" | "emu-refresh"
        | "emu-screen-measure" | "emu-unplug" | "emu-keepfed" | "mutants" | "qemu" => {
            run_step(&cmd)
        }
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
            let cmd = match args().nth(2).as_deref() {
                Some("soft") => "cf soft",
                Some("log") => "cf log",
                _ => "cf",
            };
            eprintln!("==> {cmd}");
            usb::command(&workspace_root(), cmd)
        }
        "log" => {
            eprintln!("==> log");
            usb::last_log(&workspace_root())
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
                         -o target/wmbench/rp-wm3d.exe tools/wmbench/rp-wm3d.c -ltoolhelp &&                          ~/cegcc/bin/arm-mingw32ce-gcc -O2 -mcpu=xscale -Wall -s                          -Wl,--major-subsystem-version,5,--minor-subsystem-version,1                          -o target/wmbench/rp-wmaudio.exe tools/wmbench/rp-wmaudio.c -lmmtimer &&                          ~/cegcc/bin/arm-mingw32ce-gcc -O2 -mcpu=xscale -Wall -s                          -Wl,--major-subsystem-version,5,--minor-subsystem-version,1                          -o target/wmbench/rp-wmpower.exe tools/wmbench/rp-wmpower.c &&                          ~/cegcc/bin/arm-mingw32ce-gcc -O2 -mcpu=xscale -Wall -s                          -Wl,--major-subsystem-version,5,--minor-subsystem-version,1                          -o target/wmbench/rp-wmtones.exe tools/wmbench/rp-wmtones.c",
                        emu::to_wsl_path(&workspace_root())
                    ))
                })
                .map(|()| {
                    eprintln!("    target/wmbench/rp-wmbench.exe: put it in the SD root (cargo xtask put target/wmbench/rp-wmbench.exe rp-wmbench.exe), run it under Windows Mobile, then fetch rp-wm-baseline.txt");
                    eprintln!("    target/wmbench/rp-wmgpu.exe (item 69, what draws WM's screen): the same way; its log is rp-wm-gpu.txt");
                    eprintln!("    target/wmbench/rp-wmtones.exe (the A/B notes through Dell's driver, docs/hw/audio.md): into its own folder (cargo xtask put target/wmbench/rp-wmtones.exe rp-wmtones/rp-wmtones.exe), run under Windows Mobile, headphones out; its log is rp-wmtones/rp-wm-tones.txt");
                    eprintln!("    target/wmbench/rp-wmpower.exe (ADR-0032 S1, the idle audio power): into its own folder (cargo xtask put target/wmbench/rp-wmpower.exe rp-wmpower/rp-wmpower.exe), run on battery under Windows Mobile; its log is rp-wmpower/rp-wm-power.txt");
                    eprintln!("    target/wmbench/rp-wmaudio.exe (docs/hw/audio.md): into its own folder (cargo xtask put target/wmbench/rp-wmaudio.exe rp-wmaudio/rp-wmaudio.exe), run under Windows Mobile; its log is rp-wmaudio/rp-wm-audio.txt");
                    eprintln!("    target/wmbench/rp-wm3d.exe (ADR-0024 M3): the same way, then start DoomGLES; its log is rp-wm3d.txt, its copies rp-wm3d-1.bin and -2.bin (16 MiB each)");
                })
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
                .and_then(|()| usb::install(&workspace_root()))
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
        "lint" => lint(),
        "test" => cargo(&["test", "--workspace"]),
        "cross" => cross(),
        "emu-picture" => cross().and_then(|()| emu::boot_loader_picture(&workspace_root())),
        "emu-blend" => cross().and_then(|()| emu::boot_loader_blend(&workspace_root())),
        "emu-mbx" => cross().and_then(|()| emu::boot_loader_mbx(&workspace_root())),
        "emu-mbx-blend" => cross().and_then(|()| emu::boot_loader_mbx_blend(&workspace_root())),
        "emu-mbx3d" => cross().and_then(|()| emu::boot_loader_mbx3d(&workspace_root())),
        "emu-glesfeat" => cross().and_then(|()| emu::boot_loader_glesfeat(&workspace_root())),
        "emu-unplug" => cross().and_then(|()| emu::boot_loader_unplug(&workspace_root())),
        "emu-mbx3d-api" => cross().and_then(|()| emu::boot_loader_mbx3d_api(&workspace_root())),
        "emu-mved" => cross().and_then(|()| emu::boot_loader_mved(&workspace_root())),
        "emu-clocks" => cross().and_then(|()| emu::boot_loader_clocks(&workspace_root())),
        "emu-noise" => cross().and_then(|()| emu::boot_loader_noise(&workspace_root())),
        "emu-today" => cross().and_then(|()| emu::boot_loader_today(&workspace_root())),
        "emu-crash" => cross().and_then(|()| emu::boot_loader_crash(&workspace_root())),
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
        "emu-silent-pc" => cross().and_then(|()| emu::boot_loader_silent_pc(&workspace_root())),
        "emu-refresh" => cross().and_then(|()| emu::boot_loader_refresh(&workspace_root())),
        "emu-screen-measure" => {
            cross().and_then(|()| emu::boot_loader_screen_measure(&workspace_root()))
        }
        "emu-storfast" => cross().and_then(|()| emu::boot_loader_storfast(&workspace_root())),
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
        "emu" => cross()
            .and_then(|()| emu::boot_hello(&workspace_root()))
            .and_then(|()| emu::boot_probe(&workspace_root()))
            .and_then(|()| emu::boot_kernel(&workspace_root()))
            .and_then(|()| emu::boot_threads(&workspace_root()))
            .and_then(|()| emu::boot_quantum(&workspace_root()))
            .and_then(|()| emu::boot_light(&workspace_root()))
            .and_then(|()| emu::boot_usb(&workspace_root()))
            .and_then(|()| emu::boot_loader(&workspace_root()))
            .and_then(|()| emu::boot_loader_hang(&workspace_root()))
            .and_then(|()| emu::boot_loader_run_limit(&workspace_root()))
            .and_then(|()| emu::boot_loader_picture(&workspace_root()))
            .and_then(|()| emu::boot_loader_blend(&workspace_root()))
            .and_then(|()| emu::boot_loader_mbx(&workspace_root()))
            .and_then(|()| emu::boot_loader_mbx_blend(&workspace_root()))
            .and_then(|()| emu::boot_loader_mbx3d(&workspace_root()))
            .and_then(|()| emu::boot_loader_glesfeat(&workspace_root()))
            .and_then(|()| emu::boot_loader_mbx3d_api(&workspace_root()))
            .and_then(|()| emu::boot_loader_mved(&workspace_root()))
            .and_then(|()| emu::boot_loader_clocks(&workspace_root()))
            .and_then(|()| emu::boot_loader_noise(&workspace_root()))
            .and_then(|()| emu::boot_loader_today(&workspace_root()))
            .and_then(|()| emu::boot_loader_crash(&workspace_root()))
            .and_then(|()| emu::boot_loader_handover(&workspace_root()))
            .and_then(|()| emu::boot_loader_nightlight(&workspace_root()))
            .and_then(|()| emu::boot_loader_cfdetect(&workspace_root()))
            .and_then(|()| emu::boot_loader_display_cal(&workspace_root()))
            .and_then(|()| emu::boot_loader_looks(&workspace_root()))
            .and_then(|()| emu::boot_loader_refresh(&workspace_root()))
            .and_then(|()| emu::boot_usb_link(&workspace_root()))
            .and_then(|()| emu::boot_loader_usb(&workspace_root()))
            .and_then(|()| emu::boot_loader_trace(&workspace_root()))
            .and_then(|()| emu::boot_loader_put(&workspace_root()))
            .and_then(|()| emu::boot_loader_put_exfat(&workspace_root()))
            .and_then(|()| emu::boot_loader_cf(&workspace_root()))
            .and_then(|()| emu::boot_loader_tone(&workspace_root()))
            .and_then(|()| emu::boot_loader_audio_s1(&workspace_root()))
            .and_then(|()| emu::boot_loader_audio_sleep(&workspace_root()))
            .and_then(|()| emu::boot_loader_dma_restart(&workspace_root()))
            .and_then(|()| emu::boot_loader_audio_s2(&workspace_root()))
            .and_then(|()| emu::boot_loader_silent_pc(&workspace_root()))
            .and_then(|()| emu::boot_loader_drive(&workspace_root()))
            .and_then(|()| emu::boot_loader_stale_view(&workspace_root()))
            .and_then(|()| emu::boot_loader_fallback(&workspace_root()))
            .and_then(|()| emu::boot_loader_locked(&workspace_root()))
            .and_then(|()| emu::boot_loader_backlight(&workspace_root()))
            .and_then(|()| emu::boot_loader_full_size(&workspace_root()))
            .and_then(|()| emu::boot_loader_keeps_feeding(&workspace_root()))
            // The benchmark image's on-target checks (pixel paths, memory,
            // crypto, SD) and no regression past the baseline.
            .and_then(|()| emu::bench(&workspace_root(), false)),
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
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = Command::new(cargo)
        .args(args)
        .env("RP_BUILD_ID", build_id())
        .current_dir(workspace_root())
        .status()
        .map_err(|e| format!("failed to run cargo: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`cargo {}` failed", args.join(" ")))
    }
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
    userimg::package_all(&workspace_root().join("target/armv5te-x51v-none-eabi/release"))?;
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
        let from = workspace_root()
            .join("target/thumbv5te-x51v-none-eabi/release")
            .join(image);
        let to = workspace_root()
            .join("target/armv5te-x51v-none-eabi/release")
            .join(image);
        std::fs::copy(&from, &to).map_err(|e| format!("{}: {e}", from.display()))?;
    }
    // Bare-metal images: OS crates that are binaries only.
    let images: Vec<&str> = crates
        .iter()
        .filter(|c| matches!(c.place, Placement::Os(_)) && c.lib_root.is_none())
        .map(|c| c.name.as_str())
        .collect();
    size::check(
        &workspace_root().join("target/armv5te-x51v-none-eabi/release"),
        &images,
    )
}
