//! The audio checks: rp-tone, the audio sleep and restart, rp-audio-s1 and -s2, their recordings judged.

#[allow(clippy::wildcard_imports)]
use super::*;

/// Audio: `rp-tone` under the loader over USB (WM8750 over I2C, I2S fed by
/// a DMA ring): 440 Hz for 500 ms to the speaker, then, with the headphones
/// plugged in while it waits, 1000 Hz for 50 ms to them. The emulator records
/// to a wav file, and the first tone's pitch is measured from its zero
/// crossings.
pub fn boot_loader_tone(root: &Path) -> Result<(), String> {
    if !has_usb_model(root) {
        eprintln!("    tone: skipped (this emulator has no USB controller model)");
        return Ok(());
    }
    let loader = raw_image(root, "rp-loader")?;
    let image = raw_image(root, "rp-tone")?;
    let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let wav = scratch(root, "tone.wav")?;
    let _ = std::fs::remove_file(&wav);
    let wav_path = match emulator(root)? {
        Emulator::Wsl => to_wsl_path(&wav),
        Emulator::Native(_) => wav.display().to_string(),
    };
    let port = monitor_port();
    let mut args: Vec<String> = usb_args(2).into();
    args.extend([
        "-audiodev".into(),
        format!("wav,id=snd,path={wav_path}{WAV_VOICE}"),
        "-machine".into(),
        "audiodev=snd".into(),
    ]);
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let check = || -> Result<(), String> {
        let mut s = usb_connect(2)?;
        usb_read_until(&mut s, "RPLOAD1 READY", 10)?;
        s.write_all(&frame).map_err(|e| e.to_string())?;
        usb_read_line(&mut s, "RPL OK sha256 ", 15)?;
        // Headphones in while the image waits for the jack (after its
        // first tone): the second tone goes to them.
        guest_hold(
            &mut crate::screen::Monitor::connect(monitor_port())?,
            4000,
            "tone",
        )?;
        let mut mon = crate::screen::Monitor::connect(port)?;
        // An emulator with the audio wiring as measured under Windows
        // Mobile (rp-v0.23.0, `codec-power`: the codec answers only while
        // CPLD 0x1C bit 1 powers it, the jack on GPIO83 high when plugged in)
        // checks the jack too; an older one modelled the jack on GPIO96,
        // which the image no longer reads, and only the first tone is
        // checked there.
        let wired = mon.query("qom-get /machine codec-power")?.contains("true");
        mon.query("qom-set /machine/peripheral/jack pressed true")?;
        let log = usb_read_until(&mut s, "RPL END", 40)?;
        let mut wants = vec![
            "probe standard, as run 2",
            "tone 440 Hz 1000 ms (48000 frames",
            "tone 660 Hz 1000 ms (48000 frames",
            "tone 880 Hz 1000 ms (48000 frames",
            "by DMA",
            "to speaker",
            "codec at 0x1a",
            "speaker amplifier on (CPLD 14 bit 1)",
            "codec power put back off",
            "TONE-OK",
        ];
        // A release with the measured I2S restart rule (rp-v0.24.0) drops
        // the switch-on straight after a stop, as the X51v does.
        if mon.query("qom-get /machine i2s-restart")?.contains("true") {
            wants.push("I2S switched on again at once after a stop: taken false");
        }
        if wired {
            wants.extend([
                "headphones out (GPIO83 0; GPIO96 1)",
                "headphones in (GPIO83 1)",
                "speaker amplifier off (headphones)",
                "PLUG IN THE HEADPHONES NOW (30 s)",
                "to headphones",
            ]);
        } else {
            eprintln!(
                "    tone: this emulator's jack is the old GPIO96 model (before rp-v0.23.0): the jack part is skipped"
            );
        }
        for want in wants {
            if !log.contains(want) {
                return Err(format!("tone: no {want:?} in the run log:{log}"));
            }
        }
        if let Some(line) = log.lines().find(|l| l.starts_with("tone 440")) {
            eprintln!("    rp-tone: {}", line.trim());
        }
        // Let the backend write out what it has.
        guest_hold(&mut mon, 500, "tone")?;
        Ok(())
    };
    boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &extra,
            input: None,
            during: Some(&check),
            then: None,
            timeout: Duration::from_secs(90),
        },
    )?;
    let (found, seconds) = first_notes(&wav, &[440.0, 660.0, 880.0], "tone")?;
    eprintln!(
        "    tone OK: WM8750 found on I2C, three notes through I2S, recorded at {:.1}, {:.1} and {:.1} Hz over {seconds:.3} s",
        found[0], found[1], found[2]
    );
    Ok(())
}

/// The codec's sleep (ADR-0032 R5, S1b): `rp-audio-s1` built with
/// `RP_AUDIO_SLEEP`. After a note the codec sleeps within the quiet time
/// with I2S still running; a sound with no wake first wakes it from sleep
/// (VMID and the amplifier: ~150 ms); three taps on the screen wake it as
/// the stylus lands, so each click starts right at the lift.
pub fn boot_loader_audio_sleep(root: &Path) -> Result<(), String> {
    if !has_usb_model(root) {
        eprintln!("    audio sleep: skipped (this emulator has no USB controller model)");
        return Ok(());
    }
    let loader = raw_image(root, "rp-loader")?;
    let image = variant_image(root, "rp-audio-s1", ("RP_AUDIO_SLEEP", "1"))?;
    let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let port = monitor_port();
    let args: Vec<String> = usb_args(2).into();
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let check = || -> Result<(), String> {
        let mut s = usb_connect(2)?;
        usb_read_until(&mut s, "RPLOAD1 READY", 10)?;
        let mut mon = crate::screen::Monitor::connect(port)?;
        mon.query("qom-set /machine/peripheral/jack pressed false")?;
        let mut qmp = crate::screen::Qmp::connect(port + 1)?;
        s.write_all(&frame).map_err(|e| e.to_string())?;
        usb_read_line(&mut s, "RPL OK sha256 ", 15)?;
        // Three taps, each held 4 s (the image waits up to 30 s for each,
        // so a slow emulator still finds the stylus down), 12 s apart.
        let centre = (0x4000, 0x4000);
        guest_hold(&mut mon, 20000, "audio-sleep")?;
        for _ in 0..3 {
            qmp.touch(Some(centre))?;
            guest_hold(&mut mon, 4000, "audio-sleep")?;
            qmp.touch(None)?;
            guest_hold(&mut mon, 8000, "audio-sleep")?;
        }
        let log = usb_read_until(&mut s, "RPL END", 60)?;
        for want in [
            "SLEEP TEST: HEADPHONES OUT",
            "TEST 1:",
            "TEST 2: A NOTE WITH NO TOUCH FIRST",
            "TEST 5:",
            "DONE: DID YOU HEAR 2 NOTES AND 3 CLICKS?",
            "board bits put back as found",
            "AUDIO-S1-OK",
        ] {
            if !log.contains(want) {
                return Err(format!("audio sleep: no {want:?} in the run log:{log}"));
            }
        }
        let asleep = log.matches("codec asleep").count();
        let running = log.matches("I2S still running: true").count();
        if asleep != 5 || running != 5 {
            return Err(format!(
                "audio sleep: {asleep} sleeps, {running} with I2S running (wanted 5 and 5):{log}"
            ));
        }
        let woke = log.matches("(from sleep: true)").count();
        if woke != 4 {
            return Err(format!(
                "audio sleep: {woke} wakes from sleep (wanted 4: the alarm-style note and three taps):{log}"
            ));
        }
        // Each click starts right at the lift: the codec was already up.
        for l in log.lines().filter(|l| l.contains("the click started")) {
            let ms: u32 = l
                .split("started ")
                .nth(1)
                .and_then(|r| r.split(' ').next())
                .and_then(|n| n.parse().ok())
                .ok_or(format!("audio sleep: unreadable click line {l:?}"))?;
            if ms > 30 {
                return Err(format!(
                    "audio sleep: a click started {ms} ms after the lift:{log}"
                ));
            }
        }
        if log.contains("didn't sleep") || log.contains("no tap in 30 s") {
            return Err(format!("audio sleep: a step didn't happen:{log}"));
        }
        if let Some(l) = log.lines().find(|l| l.contains("the chip woke in")) {
            eprintln!("    audio sleep: {}", l.trim());
        }
        Ok(())
    };
    boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &extra,
            input: None,
            during: Some(&check),
            then: None,
            timeout: Duration::from_secs(150),
        },
    )?;
    eprintln!(
        "    audio sleep OK: five sleeps with I2S running, four wakes from sleep, every click at the lift"
    );
    Ok(())
}

/// ADR-0032 A6's device bench (`rp-tone` built with `RP_TONE_DMA_RESTART`):
/// the audio DMA channel stops at its chain's STOP link while I2S runs,
/// stays stopped 0-1,000 ms, and is restarted as option B would restart
/// it. Here every try must stop at the chain's end, move again, finish and
/// leave I2S on; the timings are the device's to measure (the emulator's
/// DMA restarts in no time), so none is checked. The codec is off, so the
/// link must drain on its own clock as on the X51v (`i2s-free-run`,
/// rp-v0.27.0+); an older emulator never moves the DMA, and is skipped.
pub fn boot_loader_dma_restart(root: &Path) -> Result<(), String> {
    if !has_usb_model(root) {
        eprintln!("    dma restart: skipped (this emulator has no USB controller model)");
        return Ok(());
    }
    let loader = raw_image(root, "rp-loader")?;
    let image = variant_image(root, "rp-tone", ("RP_TONE_DMA_RESTART", "1"))?;
    let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let frame = rp_loadproto::frame(&payload);
    // The model's I2S drains its FIFO only into an audio backend (without
    // one the DMA moves a burst and waits): a recording nobody checks.
    let wav = scratch(root, "dma-restart.wav")?;
    let _ = std::fs::remove_file(&wav);
    let wav_path = match emulator(root)? {
        Emulator::Wsl => to_wsl_path(&wav),
        Emulator::Native(_) => wav.display().to_string(),
    };
    let mut args: Vec<String> = usb_args(2).into();
    args.extend([
        "-audiodev".into(),
        format!("wav,id=snd,path={wav_path}{WAV_VOICE}"),
        "-machine".into(),
        "audiodev=snd".into(),
    ]);
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let port = monitor_port();
    let skipped = std::cell::Cell::new(false);
    let check = || -> Result<(), String> {
        let mut s = usb_connect(2)?;
        usb_read_until(&mut s, "RPLOAD1 READY", 10)?;
        let mut mon = crate::screen::Monitor::connect(port)?;
        if !mon.query("qom-get /machine i2s-free-run")?.contains("true") {
            skipped.set(true);
            return Ok(());
        }
        s.write_all(&frame).map_err(|e| e.to_string())?;
        usb_read_line(&mut s, "RPL OK sha256 ", 15)?;
        let log = usb_read_until(&mut s, "RPL END", 90)?;
        for want in [
            "restart: I2S at 44.1 kHz",
            "restart: 15 tries",
            "RETROPANE-TONE-OK",
        ] {
            if !log.contains(want) {
                return Err(format!("dma restart: no {want:?} in the run log:{log}"));
            }
        }
        let tries = log.lines().filter(|l| l.contains(" try ")).count();
        let moved = log
            .lines()
            .filter(|l| l.contains(" try ") && l.contains("I2S on true; finished Some(true)"))
            .count();
        let chain_end = log
            .lines()
            .filter(|l| l.contains(" try ") && l.contains("stopped at byte 131072,"))
            .count();
        if tries != 15 || moved != 15 || chain_end != 15 {
            return Err(format!(
                "dma restart: {tries} tries, {moved} moved and finished with I2S on, {chain_end} stopped at the chain's end (wanted 15 each):{log}"
            ));
        }
        if log.contains("NO MOVE") || log.contains("never stopped") || log.contains("refused") {
            return Err(format!("dma restart: a try failed:{log}"));
        }
        Ok(())
    };
    boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &extra,
            input: None,
            during: Some(&check),
            then: None,
            timeout: Duration::from_secs(150),
        },
    )?;
    if skipped.get() {
        eprintln!(
            "    dma restart: skipped (this emulator's I2S drains only into a powered codec: before rp-v0.28.0)"
        );
        return Ok(());
    }
    eprintln!(
        "    dma restart OK: 15 stops at the chain's end with I2S running, every restart moved and finished"
    );
    Ok(())
}

/// Audio service step S1 (ADR-0032, `rp-audio-s1`): audio up at 44.1 kHz
/// through the service, three notes to the speaker (pitch checked in the
/// recording: a codec set to another rate than I2S would shift it), the
/// headphones plugged in while up (re-routed by the edge latch), down and
/// straight back up (the restart guard waits, then a note plays: the
/// emulator drops an early switch-on as the X51v does), the headphones
/// pulled while down (no edge: the jack line is low with the codec off, as
/// measured; the next power-up reads it), the board bits put back.
pub fn boot_loader_audio_s1(root: &Path) -> Result<(), String> {
    if !has_usb_model(root) {
        eprintln!("    audio S1: skipped (this emulator has no USB controller model)");
        return Ok(());
    }
    let loader = raw_image(root, "rp-loader")?;
    let image = raw_image(root, "rp-audio-s1")?;
    let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let wav = scratch(root, "audio-s1.wav")?;
    let _ = std::fs::remove_file(&wav);
    let wav_path = match emulator(root)? {
        Emulator::Wsl => to_wsl_path(&wav),
        Emulator::Native(_) => wav.display().to_string(),
    };
    let port = monitor_port();
    let mut args: Vec<String> = usb_args(2).into();
    args.extend([
        "-audiodev".into(),
        format!("wav,id=snd,path={wav_path}{WAV_VOICE}"),
        "-machine".into(),
        "audiodev=snd".into(),
    ]);
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let check = || -> Result<(), String> {
        let mut s = usb_connect(2)?;
        usb_read_until(&mut s, "RPLOAD1 READY", 10)?;
        let mut mon = crate::screen::Monitor::connect(port)?;
        // The audio wiring (rp-v0.23.0+), the I2S restart rule (rp-v0.24.0+)
        // and the jack line waiting for VMID (rp-v0.25.0+): what this check is about.
        for prop in ["codec-power", "i2s-restart", "jack-vmid"] {
            if !mon
                .query(&format!("qom-get /machine {prop}"))?
                .contains("true")
            {
                return Err(format!(
                    "audio S1: this emulator lacks the {prop} model (pin rp-v0.25.0 or later)"
                ));
            }
        }
        // The owner's steps: headphones in at the start, out at step 2. At
        // step 4 the check leaves them out (the owner puts them back): the
        // model records only the speaker's voice, and the recording keeps
        // only the last power-up's sound, so the last note must go to the
        // speaker to be checked.
        mon.query("qom-set /machine/peripheral/jack pressed true")?;
        s.write_all(&frame).map_err(|e| e.to_string())?;
        usb_read_line(&mut s, "RPL OK sha256 ", 15)?;
        guest_hold(&mut mon, 7000, "audio-s1")?;
        mon.query("qom-set /machine/peripheral/jack pressed false")?;
        let log = usb_read_until(&mut s, "RPL END", 60)?;
        for want in [
            "RetroPane audio S1 (44.1 kHz",
            "STEP 1: HEADPHONES IN",
            "up (first): codec at 0x1a, to headphones, waited 0 ms",
            "note 440 Hz: 44100 frames",
            "note 880 Hz: 44100 frames",
            "STEP 2: PULL THE HEADPHONES OUT NOW",
            "jack: rerouted to speaker (edge latch, debounced)",
            "board: speaker amplifier on",
            "STEP 3: LEAVE THEM OUT",
            "up (straight after down): codec at 0x1a, to speaker",
            "STEP 4: PUT THE HEADPHONES BACK IN NOW",
            "while down: a jack edge came: false",
            "up (after the change while down): codec at 0x1a, to speaker",
            "board: speaker amplifier off",
            "DONE: 6 NOTES IN ALL",
            "board bits put back as found",
            "AUDIO-S1-OK",
        ] {
            if !log.contains(want) {
                return Err(format!("audio S1: no {want:?} in the run log:{log}"));
            }
        }
        // The guard waited (most of 700 ms: down takes a few) and the note
        // after it played.
        let waited = log
            .lines()
            .find(|l| l.starts_with("up (straight after down)"))
            .and_then(|l| l.split("waited ").nth(1))
            .and_then(|r| r.split(' ').next())
            .and_then(|n| n.parse::<u32>().ok())
            .ok_or("audio S1: no guard wait in the log")?;
        if !(500..=700).contains(&waited) {
            return Err(format!("audio S1: the restart guard waited {waited} ms"));
        }
        let played = log.matches("frames in").count();
        if played != 6 {
            return Err(format!("audio S1: {played} notes played, wanted 6:{log}"));
        }
        if log.contains("failed") || log.contains("stream ") {
            return Err(format!("audio S1: a step failed:{log}"));
        }
        guest_hold(&mut mon, 500, "audio-s1")?;
        Ok(())
    };
    boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &extra,
            input: None,
            during: Some(&check),
            then: None,
            timeout: Duration::from_secs(120),
        },
    )?;
    // Each power-up re-opens the emulator codec's voice, and its wav
    // backend starts the file again: the recording holds the last session,
    // its one note (880 Hz). Its pitch proves both sides at 44.1 kHz (a
    // codec left at 48 kHz would play it ~9% sharp).
    let (found, seconds) = first_notes(&wav, &[880.0], "audio S1")?;
    eprintln!(
        "    audio S1 OK: three power-ups at 44.1 kHz through the service, the last note recorded at {:.1} Hz over {seconds:.3} s; jack re-routed while up, the restart guard waited, no jack change seen for our own power-down",
        found[0]
    );
    Ok(())
}

/// A PC that stays silent after a plug-in (quality audit B1, docs/hw/udc.md
/// "unplugged, but nobody came"): the emulated host ignores the loader's
/// first plug-in (`pxa27x-udc.ignore-plugs=1`, rp-v0.26.0+). The loader's
/// retry (`pxa-udc` `unplug::Patience`: no bus reset for 10 s, unplug and
/// present again) must get it configured and talking; without the retry it
/// waits forever (an emulator mutant).
pub fn boot_loader_silent_pc(root: &Path) -> Result<(), String> {
    if !has_usb_model(root) || !device_has(root, "pxa27x-udc", "ignore-plugs") {
        eprintln!(
            "    silent PC: skipped (this emulator's USB host can't stay silent: rp-v0.26.0)"
        );
        return Ok(());
    }
    let loader = raw_image(root, "rp-loader")?;
    let mut args: Vec<String> = usb_args(15).into();
    args.extend(["-global".into(), "pxa27x-udc.ignore-plugs=1".into()]);
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let check = || -> Result<(), String> {
        // The first plug-in passes in silence; the retry comes after 10 s
        // of the loader's time without a bus reset.
        let mut s = usb_connect(15)?;
        usb_read_until(&mut s, "RPLOAD1 READY", 90)?;
        Ok(())
    };
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &extra,
            input: None,
            during: Some(&check),
            then: None,
            timeout: Duration::from_secs(150),
        },
    )?;
    let _ = output;
    eprintln!("    silent PC OK: the first plug-in ignored, the loader's retry got it configured");
    Ok(())
}

/// ADR-0032 A6, audit E3 (`rp-audio-s2` built with `RP_AUDIO_E3`): the
/// ring that stops at the end of the mixed sound. A: a rising scale (a
/// semitone every 100 ms), the mixer stalled 400 ms after 1 s: the
/// recording must hold the scale, one gap of silence, and the scale
/// carrying on, its notes never going down (a replay of the ring's last
/// quarter second would), none missing. B: the scale again with five wakes
/// held back until the engine is inside the end's piece (the race): the
/// notes in order with no gap. The log: the stall late once and stopped,
/// five races forced, nothing failed. The headphones are out (the model
/// records the speaker's voice).
pub fn boot_loader_audio_e3(root: &Path) -> Result<(), String> {
    if !has_usb_model(root) {
        eprintln!("    audio E3: skipped (this emulator has no USB controller model)");
        return Ok(());
    }
    let loader = raw_image(root, "rp-loader")?;
    let image = variant_image(root, "rp-audio-s2", ("RP_AUDIO_E3", "1"))?;
    let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let wav = scratch(root, "audio-e3.wav")?;
    let _ = std::fs::remove_file(&wav);
    let wav_path = match emulator(root)? {
        Emulator::Wsl => to_wsl_path(&wav),
        Emulator::Native(_) => wav.display().to_string(),
    };
    let port = monitor_port();
    let mut args: Vec<String> = usb_args(2).into();
    args.extend([
        "-audiodev".into(),
        format!("wav,id=snd,path={wav_path}{WAV_VOICE}"),
        "-machine".into(),
        "audiodev=snd".into(),
    ]);
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let check = || -> Result<(), String> {
        let mut s = usb_connect(2)?;
        usb_read_until(&mut s, "RPLOAD1 READY", 10)?;
        let mut mon = crate::screen::Monitor::connect(port)?;
        mon.query("qom-set /machine/peripheral/jack pressed false")?;
        s.write_all(&frame).map_err(|e| e.to_string())?;
        usb_read_line(&mut s, "RPL OK sha256 ", 15)?;
        let log = usb_read_until(&mut s, "RPL END", 150)?;
        for want in [
            "E3 TEST A",
            "E3A RESULT",
            "E3A RING",
            "E3 TEST B",
            "E3B RESULT",
            "E3B RING",
            "E3 ALL",
            "E3 DONE",
            "board bits put back as found",
            "AUDIO-S2-OK",
        ] {
            if !log.contains(want) {
                return Err(format!("audio E3: no {want:?} in the run log:{log}"));
            }
        }
        let line = |key: &str| -> Result<&str, String> {
            log.lines()
                .find(|l| l.contains(key))
                .ok_or(format!("audio E3: no {key:?} line"))
        };
        let after = |l: &str, key: &str| -> Result<u64, String> {
            l.split(key)
                .nth(1)
                .and_then(|r| r.trim_start().split([' ', ',']).next())
                .and_then(|n| n.parse().ok())
                .ok_or(format!("audio E3: no {key:?} in {l:?}"))
        };
        let a = line("E3A RING")?;
        if after(a, "late")? != 1 || after(a, "stops")? < 1 || after(a, "restarts")? < 1 {
            return Err(format!(
                "audio E3: the stall wasn't one late wake with the engine stopped and started again: {a:?}"
            ));
        }
        let b = line("E3B RING")?;
        if after(b, "forced")? != 5 || after(b, "late")? != 0 {
            return Err(format!("audio E3: five races not forced cleanly: {b:?}"));
        }
        // No stream heard dry, in either part (A6 run 2: the scale ran dry
        // once after the stall; a dry stretch is now counted only when it
        // plays).
        for part in ["E3A RESULT", "E3B RESULT"] {
            let l = line(part)?;
            if after(l, "dry")? != 0 {
                return Err(format!("audio E3: a stream ran dry: {l:?}"));
            }
        }
        let all = line("E3 ALL")?;
        if after(a, "failed")? != 0 || after(b, "failed")? != 0 || after(all, "failed")? != 0 {
            return Err(format!("audio E3: the ring's end failed to move:{log}"));
        }
        // The engine stood stopped through the rest of the stall: 400 ms
        // less the mix it had (245 ms) and its piece's silence.
        let stall = line("  stall:")?;
        if after(stall, "end, stopped")? < 100 {
            return Err(format!(
                "audio E3: the engine stopped under 100 ms: {stall:?}"
            ));
        }
        if !line("  stall:")?.contains("had stopped") {
            return Err(format!(
                "audio E3: the engine still played at the stall's end (longer than the mix and its piece): {:?}",
                line("  stall:")?
            ));
        }
        for l in log.lines().filter(|l| {
            l.contains("E3A") || l.contains("E3B") || l.contains("stall:") || l.contains("race ")
        }) {
            eprintln!("    audio E3: {}", l.trim());
        }
        guest_hold(&mut mon, 500, "audio-e3")?;
        Ok(())
    };
    boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &extra,
            input: None,
            during: Some(&check),
            then: None,
            timeout: Duration::from_secs(240),
        },
    )?;
    let found = e3_recording(&wav)?;
    eprintln!("    audio E3 OK: {found}");
    Ok(())
}

/// The scale `rp-audio-s2`'s E3 run plays (a semitone every 100 ms).
pub(crate) const E3_SCALE: [f64; 24] = [
    440.0, 466.0, 494.0, 523.0, 554.0, 587.0, 622.0, 659.0, 698.0, 740.0, 784.0, 831.0, 880.0,
    932.0, 988.0, 1047.0, 1109.0, 1175.0, 1245.0, 1319.0, 1397.0, 1480.0, 1568.0, 1661.0,
];

/// The pitch of `x` from its first and last zero crossings, each placed
/// between its two samples (a fraction of a sample): accurate to well
/// under 1 Hz over 20 ms, where counting crossings resolves only 50 Hz
/// (coarser than a semitone).
pub(crate) fn fine_pitch(x: &[i16], rate: u32) -> f64 {
    let at: Vec<f64> = x
        .windows(2)
        .enumerate()
        .filter(|(_, p)| (p[0] < 0) != (p[1] < 0))
        .map(|(i, p)| {
            let (a, b) = (f64::from(p[0]), f64::from(p[1]));
            i as f64 + a / (a - b)
        })
        .collect();
    match (at.first(), at.last()) {
        (Some(f), Some(l)) if at.len() > 2 && l > f => {
            (at.len() - 1) as f64 / 2.0 / ((l - f) / f64::from(rate))
        }
        _ => 0.0,
    }
}

/// `emu-audio-e3`'s rule that a scale never goes down (quality audit
/// C22). A window raises the highest note only when the next window is the
/// same note: a window across a join (or a note's quiet onset after
/// silence) can read as any pitch, and one read too high made every window
/// of the true note after it "below" (C22's "note 1 after 3/4" at part B's
/// first window, 2026-10-22). Two windows in a row below the highest
/// confirmed note are a replay (a lap of the ring is a quarter second of
/// earlier notes, a dozen windows).
#[derive(Default)]
pub(crate) struct Rising {
    /// The highest note seen in two windows in a row.
    pub(crate) high: usize,
    /// The window before, if it was a note.
    pub(crate) prev: Option<usize>,
    /// Windows in a row below `high` by more than one note.
    pub(crate) below: usize,
}

impl Rising {
    /// A window holding note `n`: `Some(high)` if it is a replay.
    pub(crate) fn note(&mut self, n: usize) -> Option<usize> {
        self.below = if n + 1 < self.high { self.below + 1 } else { 0 };
        if self.below >= 2 {
            return Some(self.high);
        }
        if self.prev == Some(n) {
            self.high = self.high.max(n);
        }
        self.prev = Some(n);
        None
    }

    /// A window that is quiet or no note of the scale.
    pub(crate) fn other(&mut self) {
        self.prev = None;
    }
}

/// `emu-audio-e3`'s recording: 20 ms windows, each silent, a note of the
/// scale (its pitch within 2.5%, by [`fine_pitch`]), or neither (a join
/// between notes). The two scales (A, B) are the two runs of notes split
/// by the long pause; in each the notes never go down and all 24 are
/// there (one window out of order is a join, where a stop's cleared
/// frames skew the pitch: a replay of the old looping ring would be a
/// quarter second of earlier notes); B has no gap over 20 ms. A's stall
/// gap: the fork's I2S sends nothing while its FIFO is empty (the stopped
/// DMA), so the recording closes it up (docs/hw/emulator.md); the log's
/// "stopped N ms" holds it instead. Should the model send silence there,
/// A's one gap must be at least 100 ms.
pub(crate) fn e3_recording(wav: &Path) -> Result<String, String> {
    let (x, rate) = wav_left(wav, "audio E3")?;
    let w = rate as usize / 50;
    #[derive(Clone, Copy, PartialEq)]
    enum Win {
        Quiet,
        Note(usize),
        Other,
    }
    let wins: Vec<Win> = x
        .chunks_exact(w)
        .map(|c| {
            if rms(c) < 200.0 {
                return Win::Quiet;
            }
            let hz = fine_pitch(c, rate);
            E3_SCALE
                .iter()
                .position(|&n| (hz - n).abs() < n * 0.025)
                .map_or(Win::Other, Win::Note)
        })
        .collect();
    // Runs of notes, split where at least 600 ms is quiet (the pauses).
    let mut parts: Vec<Vec<(usize, Win)>> = Vec::new();
    let mut quiet = usize::MAX;
    for (k, &v) in wins.iter().enumerate() {
        match v {
            Win::Quiet => quiet = quiet.saturating_add(1),
            _ => {
                if quiet >= 30 {
                    parts.push(Vec::new());
                }
                quiet = 0;
            }
        }
        if let Some(p) = parts.last_mut() {
            p.push((k, v));
        }
    }
    for p in &mut parts {
        while p.last().is_some_and(|&(_, v)| v == Win::Quiet) {
            p.pop();
        }
    }
    let parts: Vec<_> = parts.into_iter().filter(|p| p.len() > 25).collect();
    if parts.len() != 2 {
        return Err(format!(
            "audio E3: the recording holds {} scales, wanted 2 (A, B)",
            parts.len()
        ));
    }
    let mut said = Vec::new();
    // In windows: A's gap at least 100 ms, B's at most 20 ms.
    for (name, p, gap_min, gap_max) in [("A", &parts[0], 5, usize::MAX), ("B", &parts[1], 0, 1)] {
        let mut rising = Rising::default();
        let mut seen = [0usize; 24];
        let (mut run, mut gaps, mut longest) = (0usize, 0usize, 0usize);
        for &(k, v) in p {
            match v {
                Win::Note(n) => {
                    if let Some(high) = rising.note(n) {
                        return Err(format!(
                            "audio E3: in {name} the scale went down at {:.2} s (note {} after {}): a replay",
                            k as f64 / 50.0,
                            n + 1,
                            high + 1
                        ));
                    }
                    seen[n] += 1;
                }
                Win::Quiet => {
                    rising.other();
                    run += 1;
                    continue;
                }
                Win::Other => rising.other(),
            }
            if run > 1 {
                gaps += usize::from(run >= gap_min.max(2));
            }
            longest = longest.max(run);
            run = 0;
        }
        let missing: Vec<usize> = (0..24).filter(|&n| seen[n] < 2).map(|n| n + 1).collect();
        if !missing.is_empty() {
            return Err(format!(
                "audio E3: in {name} notes {missing:?} are missing or short"
            ));
        }
        if name == "A" && (gaps > 1 || (gaps == 1 && longest < gap_min)) {
            return Err(format!(
                "audio E3: A holds {gaps} gaps of silence (the longest {} ms), wanted one of at least 100 ms",
                longest * 20
            ));
        }
        if longest > gap_max {
            return Err(format!(
                "audio E3: B has a gap of {} ms (the race must leave none)",
                longest * 20
            ));
        }
        said.push(format!(
            "{name} all 24 notes rising, longest silence {} ms",
            longest * 20
        ));
    }
    Ok(said.join("; "))
}

/// Audio service step S2 (ADR-0032, `rp-audio-s2`): the mixer on the
/// emulated X51v, the headphones out the whole run (the model records the
/// speaker's voice). The two notes of test 1 come from a 22.05 kHz and a
/// 48 kHz stream, the second placed to start where the first ends: both
/// must be recorded at 880 Hz, back to back (a wrong conversion ratio shifts
/// the pitch). Three taps through QMP each get a click within 10 ms; the
/// tune alone sleeps long (few wakes a second), with clicks the interval is
/// short; no wake is late and no stream runs dry anywhere; the limiter
/// works on the overload (6A) and not at half volume (6B); the board bits
/// are put back.
pub fn boot_loader_audio_s2(root: &Path) -> Result<(), String> {
    if !has_usb_model(root) {
        eprintln!("    audio S2: skipped (this emulator has no USB controller model)");
        return Ok(());
    }
    // The clicks the log says were played (taps and timed ones), for the
    // recording's check.
    let clicks_seen = std::cell::RefCell::new(0usize);
    let loader = raw_image(root, "rp-loader")?;
    let image = raw_image(root, "rp-audio-s2")?;
    let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let wav = scratch(root, "audio-s2.wav")?;
    let _ = std::fs::remove_file(&wav);
    let wav_path = match emulator(root)? {
        Emulator::Wsl => to_wsl_path(&wav),
        Emulator::Native(_) => wav.display().to_string(),
    };
    let port = monitor_port();
    let mut args: Vec<String> = usb_args(2).into();
    args.extend([
        "-audiodev".into(),
        format!("wav,id=snd,path={wav_path}{WAV_VOICE}"),
        "-machine".into(),
        "audiodev=snd".into(),
    ]);
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let check = || -> Result<(), String> {
        let mut s = usb_connect(2)?;
        usb_read_until(&mut s, "RPLOAD1 READY", 10)?;
        let mut mon = crate::screen::Monitor::connect(port)?;
        mon.query("qom-set /machine/peripheral/jack pressed false")?;
        // The link at SADIV's measured rate (rp-v0.26.0+): checked when the
        // model is there.
        let rates = mon.query("qom-get /machine i2s-rates")?.contains("true");
        let mut qmp = crate::screen::Qmp::connect(port + 1)?;
        s.write_all(&frame).map_err(|e| e.to_string())?;
        usb_read_line(&mut s, "RPL OK sha256 ", 15)?;
        // Test 2 waits up to 30 s for each tap from ~8 s into the run.
        let centre = (0x4000, 0x4000);
        guest_hold(&mut mon, 12000, "audio-s2")?;
        for _ in 0..3 {
            qmp.touch(Some(centre))?;
            guest_hold(&mut mon, 2000, "audio-s2")?;
            qmp.touch(None)?;
            guest_hold(&mut mon, 3000, "audio-s2")?;
        }
        let log = usb_read_until(&mut s, "RPL END", 180)?;
        for want in [
            "RetroPane audio S2: the mixer (44.1 kHz",
            "TEST 1: TWO NOTES",
            "TEST 1 RESULT",
            "TEST 2: TAP THE SCREEN",
            "TEST 2 RESULT",
            "TEST 3 RESULT",
            "TEST 4 RESULT",
            "5A 22 kHz RESULT",
            "5B 11 kHz RESULT",
            "5C 48 kHz RESULT",
            "TEST 6: PULL THE HEADPHONES OUT NOW",
            "sound to the speaker",
            "6A RESULT",
            "6B RESULT",
            "7A RESULT",
            "7B RESULT",
            "DONE: TELL ME TESTS 1-7",
            "board bits put back as found",
            "AUDIO-S2-OK",
        ] {
            if !log.contains(want) {
                return Err(format!("audio S2: no {want:?} in the run log:{log}"));
            }
        }
        // The second note got the start it asked for, to the frame.
        let placed = log
            .lines()
            .find(|l| l.contains("second asked for"))
            .ok_or("audio S2: no placement line")?;
        let asked = placed
            .split("asked for ")
            .nth(1)
            .and_then(|r| r.split(' ').next())
            .ok_or(format!("audio S2: unreadable {placed:?}"))?;
        if !placed.contains(&format!("got Some({asked})")) {
            return Err(format!(
                "audio S2: the second note didn't start where asked: {placed:?}"
            ));
        }
        // A number after `key` in `line` (`12.3`, `4`).
        let num = |line: &str, key: &str| -> Result<f64, String> {
            line.split(key)
                .next()
                .and_then(|l| l.rsplit(' ').next())
                .and_then(|n| n.trim_end_matches('%').parse().ok())
                .ok_or(format!("audio S2: no {key:?} in {line:?}"))
        };
        let after = |line: &str, key: &str| -> Result<f64, String> {
            line.split(key)
                .nth(1)
                .and_then(|r| r.trim_start().split([' ', ',']).next())
                .and_then(|n| n.parse().ok())
                .ok_or(format!("audio S2: no {key:?} in {line:?}"))
        };
        let result = |name: &str| -> Result<&str, String> {
            log.lines()
                .find(|l| l.starts_with(&format!("{name} RESULT")))
                .ok_or(format!("audio S2: no {name} result"))
        };
        for name in [
            "TEST 1",
            "TEST 2",
            "TEST 3",
            "TEST 4",
            "5A 22 kHz",
            "5B 11 kHz",
            "5C 48 kHz",
            "6A",
            "6B",
            "7A",
            "7B",
        ] {
            let l = result(name)?;
            if after(l, "late")? != 0.0 || after(l, "dry")? != 0.0 {
                return Err(format!("audio S2: a late wake or a stream run dry: {l:?}"));
            }
        }
        let taps: Vec<&str> = log
            .lines()
            .filter(|l| l.contains("after the pen landed"))
            .collect();
        if taps.len() != 3 {
            return Err(format!(
                "audio S2: {} taps clicked, wanted 3:{log}",
                taps.len()
            ));
        }
        for l in &taps {
            let ms = after(l, "first sample")?;
            if ms > 10.0 {
                return Err(format!("audio S2: a tap's click {ms} ms late: {l:?}"));
            }
        }
        let clicks = log
            .lines()
            .find(|l| l.contains(" clicks: first sample"))
            .ok_or("audio S2: no click summary")?;
        let worst = num(clicks, " ms at worst")?;
        *clicks_seen.borrow_mut() = clicks
            .trim()
            .split(' ')
            .next()
            .and_then(|n| n.parse::<usize>().ok())
            .ok_or(format!("audio S2: unreadable {clicks:?}"))?
            + taps.len();
        if worst > 10.0 {
            return Err(format!("audio S2: a click {worst} ms late: {clicks:?}"));
        }
        let alone = num(result("TEST 3")?, " wakes/s")?;
        let busy = num(result("TEST 4")?, " wakes/s")?;
        if alone > 12.0 || busy < 40.0 {
            return Err(format!(
                "audio S2: {alone} wakes/s with the tune alone (at most 12), {busy} with clicks (at least 40)"
            ));
        }
        if after(result("6A")?, "limited")? == 0.0 {
            return Err("audio S2: the overload in 6A wasn't limited".into());
        }
        let half = result("6B")?;
        if after(half, "limited")? != 0.0 || after(half, "clipped")? != 0.0 {
            return Err(format!(
                "audio S2: half volume was limited or clipped: {half:?}"
            ));
        }
        // Test 2's 16 s: the DMA's rate against the guest's OS timer. With
        // the model it's the X51v's measured 44,317; the emulator's pacing
        // keeps it within 0.3%.
        let dma = after(result("TEST 2")?, "DMA at")?;
        if rates && !(44_190.0..=44_450.0).contains(&dma) {
            return Err(format!(
                "audio S2: the link ran at {dma} frames/s in test 2 (the X51v's 44,317, within 0.3%)"
            ));
        }
        if !rates {
            eprintln!(
                "    audio S2: the link's rate not checked (the emulator lacks i2s-rates: rp-v0.26.0)"
            );
        }
        if !log.contains(" 0 late, 0 dry, 0 clipped") || !log.contains(" failed 0") {
            return Err(format!(
                "audio S2: a late wake, a stream run dry, a clip or a ring end that failed over the whole run:{log}"
            ));
        }
        for l in log.lines().filter(|l| {
            l.contains("RESULT") || l.contains("after the pen") || l.contains(" clicks:")
        }) {
            eprintln!("    audio S2: {}", l.trim());
        }
        guest_hold(&mut mon, 500, "audio-s2")?;
        Ok(())
    };
    boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &extra,
            input: None,
            during: Some(&check),
            then: None,
            timeout: Duration::from_secs(240),
        },
    )?;
    let clicks_played = clicks_seen.into_inner();
    // Test 1's notes, the first sound of the run: 880 Hz from 22.05 kHz,
    // then 880 Hz from 48 kHz.
    let (found, seconds) = first_notes(&wav, &[880.0, 880.0], "audio S2")?;
    // The rest of the recording (one power-up: it holds the whole run).
    let rest = s2_recording(&wav, clicks_played)?;
    eprintln!(
        "    audio S2 OK: the two-rate notes recorded at {:.1} and {:.1} Hz over {seconds:.3} s, back to back; {rest}; taps and clicks within 10 ms; long sleeps with music alone; nothing late or dry",
        found[0], found[1]
    );
    Ok(())
}

/// A wav's left channel and its rate.
pub(crate) fn wav_left(wav: &Path, what: &str) -> Result<(Vec<i16>, u32), String> {
    let data = std::fs::read(wav).map_err(|e| format!("{}: {e}", wav.display()))?;
    if data.len() < 44 || &data[..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err(format!("{what}: the emulator wrote no wav"));
    }
    let rate = u32::from_le_bytes([data[24], data[25], data[26], data[27]]);
    let channels = usize::from(u16::from_le_bytes([data[22], data[23]]));
    let left = data[44..]
        .chunks_exact(2 * channels)
        .map(|f| i16::from_le_bytes([f[0], f[1]]))
        .collect();
    Ok((left, rate))
}

/// The stretches of sound in `x` (loud samples with less than `gap`
/// samples of quiet inside), as ranges.
pub(crate) fn stretches(x: &[i16], gap: usize) -> Vec<std::ops::Range<usize>> {
    let loud = |s: i16| s.unsigned_abs() > 2000;
    let mut out = Vec::new();
    let mut start = None;
    let mut last = 0;
    for (k, &s) in x.iter().enumerate() {
        if loud(s) {
            if start.is_none() {
                start = Some(k);
            }
            last = k;
        } else if let Some(st) = start
            && k - last > gap
        {
            out.push(st..last + 1);
            start = None;
        }
    }
    if let Some(st) = start {
        out.push(st..last + 1);
    }
    out
}

/// The pitch of `x` by zero crossings.
pub(crate) fn pitch(x: &[i16], rate: u32) -> f64 {
    let crossings = x.windows(2).filter(|p| (p[0] < 0) != (p[1] < 0)).count();
    crossings as f64 / 2.0 / (x.len() as f64 / f64::from(rate))
}

pub(crate) fn rms(x: &[i16]) -> f64 {
    (x.iter().map(|&s| f64::from(s) * f64::from(s)).sum::<f64>() / x.len().max(1) as f64).sqrt()
}

/// `emu-audio-s2`'s recording past test 1 (quality audit C13): nine long
/// stretches of sound (tests 1, 3, 5A-5C, 6A, 6B, 7A, 7B; the quieter tunes
/// under tests 2 and 4 fall below the threshold) and one short one per
/// click the log says was played (`clicks`). The tune (659, 784, 880, 988 Hz... a
/// quarter second each) from 22.05, 11.025 and 48 kHz (5A-5C) at its
/// pitch, note by note; 6A (two loud parts, the limiter) with no flat tops
/// and no louder than 6B (the same at half volume) by more than the
/// limiter's headroom; 7A (the speaker filter on) quieter than 7B on the
/// low tune's first note (330 Hz, where the filter cuts about 2 dB) and the
/// same on its last (659 Hz).
pub(crate) fn s2_recording(wav: &Path, clicks: usize) -> Result<String, String> {
    let (x, rate) = wav_left(wav, "audio S2")?;
    let all = stretches(&x, rate as usize / 10);
    let (parts, short): (Vec<_>, Vec<_>) = all
        .iter()
        .cloned()
        .partition(|r| r.len() > rate as usize / 2);
    if parts.len() != 9 || short.len() != clicks {
        let secs: Vec<String> = all
            .iter()
            .map(|r| format!("{:.2}", r.len() as f64 / f64::from(rate)))
            .collect();
        return Err(format!(
            "audio S2: the recording has {} long stretches (wanted 9) and {} clicks (the log played {clicks}); seconds: {}",
            parts.len(),
            short.len(),
            secs.join(", ")
        ));
    }
    let note = rate as usize / 4;
    // The middle 60% of note `k` of a stretch.
    let at = |r: &std::ops::Range<usize>, k: usize| {
        let s = r.start + k * note;
        &x[s + note / 5..s + note * 4 / 5]
    };
    const TUNE: [f64; 8] = [659.0, 784.0, 880.0, 988.0, 1175.0, 988.0, 880.0, 784.0];
    for (name, r) in [("5A", &parts[2]), ("5B", &parts[3]), ("5C", &parts[4])] {
        for (k, &want) in TUNE.iter().enumerate() {
            let hz = pitch(at(r, k), rate);
            if (hz - want).abs() > want * 0.02 {
                return Err(format!(
                    "audio S2: {name} note {} recorded at {hz:.1} Hz, wanted {want} Hz",
                    k + 1
                ));
            }
        }
    }
    // 6A: limited, not clipped: no runs of equal samples at its peak.
    let (a, b) = (&x[parts[5].clone()], &x[parts[6].clone()]);
    let peak = |v: &[i16]| v.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
    let (pa, pb) = (peak(a), peak(b));
    let flat = a
        .windows(3)
        .filter(|w| w[0] == w[1] && w[1] == w[2] && w[0].unsigned_abs() >= pa - pa / 64)
        .count();
    if flat > 0 || f64::from(pa) > f64::from(pb) * 1.15 {
        return Err(format!(
            "audio S2: 6A peaks {pa} against 6B's {pb}, {flat} flat tops (clipped)"
        ));
    }
    // 7A against 7B: the filter's cut at 330 Hz, nothing at 659 Hz.
    let (fa, fb) = (&parts[7], &parts[8]);
    let low = 20.0 * (rms(at(fa, 0)) / rms(at(fb, 0))).log10();
    let high = 20.0 * (rms(at(fa, 5)) / rms(at(fb, 5))).log10();
    if !(-3.5..=-1.0).contains(&low) || high.abs() > 0.8 {
        return Err(format!(
            "audio S2: the speaker filter took {low:.1} dB at 330 Hz and {high:.1} dB at 659 Hz (wanted about -2.3 and 0)"
        ));
    }
    Ok(format!(
        "{clicks} clicks recorded; the tune at pitch from 22.05, 11.025 and 48 kHz; 6A limited without a flat top (peak {pa}, 6B {pb}); the speaker filter {low:.1} dB at 330 Hz, {high:.1} dB at 659 Hz"
    ))
}

/// The pause between two looks of a wait loop (a screen, a register, a
/// log polled until it shows what's waited for): never a wait by itself
/// (quality audit C2's guard counts sleeps outside this).
pub(crate) fn poll_pause(d: Duration) {
    std::thread::sleep(d);
}

/// Waits until the guest's own clock (OSCR0, 3.25 MHz, read through `mon`,
/// the check's monitor connection: QEMU's monitor takes one client at a
/// time, so a hold never opens its own while the check holds one) has
/// moved `ms` on: "let the image run this long" in
/// its time, not the PC's (a busy PC slows the guest; quality audit C2).
/// An error, naming `what`, if the guest's clock stops for a minute.
pub(crate) fn guest_hold(
    mon: &mut crate::screen::Monitor,
    ms: u32,
    what: &str,
) -> Result<(), String> {
    let mut clock = GuestClock::start(mon, what)?;
    while clock.ms()? < u64::from(ms) {
        poll_pause(Duration::from_millis(100));
    }
    Ok(())
}

/// The guest's time since [`GuestClock::start`], from its OS timer (OSCR0,
/// 3.25 MHz, read through the monitor), past the counter's wrap.
pub(crate) struct GuestClock<'m> {
    pub(crate) mon: &'m mut crate::screen::Monitor,
    pub(crate) what: String,
    pub(crate) last: u32,
    pub(crate) ticks: u64,
    pub(crate) stuck: Instant,
}

impl<'m> GuestClock<'m> {
    pub(crate) fn start(mon: &'m mut crate::screen::Monitor, what: &str) -> Result<Self, String> {
        let mut c = Self {
            mon,
            what: what.to_string(),
            last: 0,
            ticks: 0,
            // A guest whose clock hasn't moved by then has stopped (a hung
            // emulator, not slow guest work).
            stuck: Instant::now() + Duration::from_secs(60),
        };
        c.last = c.oscr()?;
        Ok(c)
    }

    pub(crate) fn oscr(&mut self) -> Result<u32, String> {
        // A busy emulator sometimes misses one answer: asked again before
        // it counts (a stopped guest is caught by `ms`'s minute).
        let mut tries = 0;
        let out = loop {
            match self.mon.query("xp /1wx 0x40a00010") {
                Ok(o) => break o,
                Err(e) if tries >= 3 => return Err(e),
                Err(_) => tries += 1,
            }
        };
        let hex = out
            .rsplit("0x")
            .next()
            .map(|h| h.trim().trim_end_matches(|c: char| !c.is_ascii_hexdigit()))
            .unwrap_or("");
        u32::from_str_radix(hex, 16).map_err(|_| format!("{}: OSCR0 read: {out}", self.what))
    }

    /// Milliseconds of guest time so far; an error if the guest's clock
    /// hasn't moved for a minute.
    pub(crate) fn ms(&mut self) -> Result<u64, String> {
        let now = self.oscr()?;
        let step = now.wrapping_sub(self.last);
        if step > 0 {
            self.stuck = Instant::now() + Duration::from_secs(60);
        } else if Instant::now() > self.stuck {
            return Err(format!("{}: the guest's clock stopped", self.what));
        }
        self.ticks += u64::from(step);
        self.last = now;
        Ok(self.ticks / 3_250)
    }
}

/// The first stretch of sound in an emulator recording (`wav`): `want`'s
/// notes back to back, a second each. Their measured frequencies (within
/// 4% of `want`, else an error naming `what`) and the stretch's length.
pub(crate) fn first_notes(wav: &Path, want: &[f64], what: &str) -> Result<(Vec<f64>, f64), String> {
    let data = std::fs::read(wav).map_err(|e| format!("{}: {e}", wav.display()))?;
    if data.len() < 44 || &data[..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err(format!("{what}: the emulator wrote no wav"));
    }
    let rate = u32::from_le_bytes([data[24], data[25], data[26], data[27]]);
    let channels = usize::from(u16::from_le_bytes([data[22], data[23]]));
    let left: Vec<i16> = data[44..]
        .chunks_exact(2 * channels)
        .map(|f| i16::from_le_bytes([f[0], f[1]]))
        .collect();
    // The notes: the stretch between the first and last loud samples.
    let loud = |s: &i16| s.unsigned_abs() > 2000;
    let Some(start) = left.iter().position(loud) else {
        return Err(format!("{what}: the recording is silent"));
    };
    // They end at the first tenth of a second of quiet (later sounds come
    // seconds after).
    let gap = (rate / 10) as usize;
    let mut end = start;
    let mut quiet = 0;
    for (k, s) in left.iter().enumerate().skip(start) {
        if loud(s) {
            end = k;
            quiet = 0;
        } else {
            quiet += 1;
            if quiet > gap {
                break;
            }
        }
    }
    let tones = &left[start..=end];
    let seconds = tones.len() as f64 / f64::from(rate);
    let n = want.len() as f64;
    if !((n - 0.2)..=(n + 0.4)).contains(&seconds) {
        return Err(format!(
            "{what}: recorded {seconds:.3} s of sound (wanted {} notes of 1 s)",
            want.len()
        ));
    }
    // Each note, its middle 80% (away from the joins).
    let part_len = tones.len() / want.len();
    let mut found = Vec::new();
    for (k, &w) in want.iter().enumerate() {
        let part = &tones[k * part_len + part_len / 10..(k + 1) * part_len - part_len / 10];
        let crossings = part.windows(2).filter(|p| (p[0] < 0) != (p[1] < 0)).count();
        let hz = crossings as f64 / 2.0 / (part.len() as f64 / f64::from(rate));
        if (hz - w).abs() > w * 0.04 {
            return Err(format!(
                "{what}: note {} recorded at {hz:.1} Hz, wanted {w} Hz",
                k + 1
            ));
        }
        found.push(hz);
    }
    Ok((found, seconds))
}
