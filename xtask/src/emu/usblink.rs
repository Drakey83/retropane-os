//! The USB checks: the link, the drive and the trace over the emulated cable.

#[allow(clippy::wildcard_imports)]
use super::*;

/// Port of the emulated USB serial port (the UDC model's chardev); each test
/// gets its own (`n`), so one can never reach another test's emulator.
pub(crate) fn usb_port(n: u16) -> u16 {
    assert!(n < USB_PORTS, "usb_port({n}): past this run's port block");
    monitor_port() + 2 + n
}

/// QEMU arguments that connect the UDC model's serial data to a TCP port,
/// with the model's protocol checks reported (guest errors).
pub(crate) fn usb_args(n: u16) -> [String; 6] {
    [
        "-d".into(),
        "guest_errors".into(),
        "-chardev".into(),
        format!(
            "socket,id=usb,host=127.0.0.1,port={},server=on,wait=off",
            usb_port(n)
        ),
        "-global".into(),
        "pxa27x-udc.chardev=usb".into(),
    ]
}

/// QEMU arguments for the UDC model's mass storage chardev (ADR-0014).
pub(crate) fn msc_args(n: u16) -> [String; 4] {
    [
        "-chardev".into(),
        format!(
            "socket,id=msc,host=127.0.0.1,port={},server=on,wait=off",
            usb_port(n)
        ),
        "-global".into(),
        "pxa27x-udc.msc=msc".into(),
    ]
}

/// Whether the emulator's `device` model has the property `prop`.
/// An emulator that can't be used (refused as the wrong release, quality
/// audit C41; missing; not downloaded) ends the run. The feature probes
/// (`device_has` and its siblings) once answered "not modelled" here, so a
/// refused emulator made checks print "skipped" and pass (found
/// 2026-10-21 with C41's refusal).
pub(crate) fn no_emulator(e: &str) -> ! {
    eprintln!("error: {e}");
    std::process::exit(1)
}

pub(crate) fn device_has(root: &Path, device: &str, prop: &str) -> bool {
    let out = match emulator(root) {
        Ok(Emulator::Native(exe)) => Command::new(exe)
            .args(["-M", "x51v", "-device", &format!("{device},help")])
            .output(),
        Ok(Emulator::Wsl) => {
            wsl_bash(&format!("{} -M x51v -device {device},help", wsl_qemu())).output()
        }
        Err(e) => no_emulator(&e),
    };
    out.is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains(&format!("{prop}=")))
}

/// Whether the emulator starts with `-global <setting>`: an emulator whose
/// device lacks the property refuses to start. For devices whose `,help`
/// can't be asked (the PC Card socket's instantiates it outside a machine,
/// which asserts in QEMU's memory code).
pub(crate) fn emulator_accepts_global(root: &Path, setting: &str) -> bool {
    let args = format!("-M x51v -display none -S -monitor stdio -serial null -global {setting}");
    let child = match emulator(root) {
        Ok(Emulator::Native(exe)) => {
            // Windows QEMU never reads a stdin pipe (item 61), so `quit`
            // can't end it: it waited forever (2026-10-16, emu-cf on the
            // rp-v0.17.0 Windows release). A rejected setting ends QEMU at
            // once with an error; one still paused after the wait was taken.
            let native = args.replace("-monitor stdio", "-monitor none");
            let Ok(mut child) = Command::new(exe)
                .args(native.split(' '))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            else {
                return false;
            };
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                if let Ok(Some(status)) = child.try_wait() {
                    return status.success();
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            let _ = child.kill();
            let _ = child.wait();
            return true;
        }
        Ok(Emulator::Wsl) => wsl_bash(&format!("echo quit | {} {args}", wsl_qemu()))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn(),
        Err(e) => no_emulator(&e),
    };
    let Ok(mut child) = child else {
        return false;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(b"quit\n");
    }
    child.wait().is_ok_and(|s| s.success())
}

/// Whether a device at `path` in the running emulator has `prop`.
pub(crate) fn device_has_runtime(mon: &mut crate::screen::Monitor, path: &str, prop: &str) -> bool {
    mon.query(&format!("qom-list {path}"))
        .is_ok_and(|out| out.contains(prop))
}

/// Whether the emulator's UDC model bridges a mass storage function
/// (retropane-qemu after rp-v0.8.0).
pub(crate) fn has_msc_model(root: &Path) -> bool {
    device_has(root, "pxa27x-udc", "msc")
}

/// The block the emulated card is too slow on for a multi-block read
/// (like the owner's card at block 10880): inside the drive test's raw
/// 1 MiB read, before the partition, so only that read meets it.
pub(crate) const SLOW_BLOCK: u32 = 1000;

/// Whether the emulator in use has the USB device controller model
/// (retropane-qemu after rp-v0.6.0). Older releases skip the USB tests.
pub(crate) fn has_usb_model(root: &Path) -> bool {
    let out = match emulator(root) {
        Ok(Emulator::Native(exe)) => Command::new(exe)
            .args(["-M", "x51v", "-device", "pxa27x-udc,help"])
            .output(),
        Ok(Emulator::Wsl) => {
            wsl_bash(&format!("{} -M x51v -device pxa27x-udc,help", wsl_qemu())).output()
        }
        Err(e) => no_emulator(&e),
    };
    out.is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("chardev"))
}

/// Connects to the emulated USB serial port.
pub(crate) fn usb_connect(n: u16) -> Result<std::net::TcpStream, String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match std::net::TcpStream::connect(("127.0.0.1", usb_port(n))) {
            Ok(s) => {
                s.set_read_timeout(Some(Duration::from_millis(200)))
                    .map_err(|e| e.to_string())?;
                return Ok(s);
            }
            Err(e) if Instant::now() > deadline => {
                return Err(format!("emulated USB serial port: {e}"));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

/// Reads from `s` until `want` arrives (or `secs` pass). Returns what came.
pub(crate) fn usb_read_until(
    s: &mut std::net::TcpStream,
    want: &str,
    secs: u64,
) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut got = Vec::new();
    let mut buf = [0u8; 256];
    while Instant::now() < deadline {
        match s.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                got.extend_from_slice(&buf[..n]);
                if String::from_utf8_lossy(&got).contains(want) {
                    let text = String::from_utf8_lossy(&got).into_owned();
                    check_stack(&text)?;
                    return Ok(text);
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Err(format!(
        "no `{want}` on the emulated USB port; got {:?}",
        String::from_utf8_lossy(&got)
    ))
}

/// Reads until `start` and the end of its line: the reply's whole line
/// (with whatever came before it), however the bytes were split into reads.
/// (Reading until `start` and then until a newline waited for a second
/// line whenever the reply arrived in one piece.)
pub(crate) fn usb_read_line(
    s: &mut std::net::TcpStream,
    start: &str,
    secs: u64,
) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut got = Vec::new();
    let mut buf = [0u8; 256];
    while Instant::now() < deadline {
        match s.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                got.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&got);
                if let Some(at) = text.find(start)
                    && text[at..].contains('\n')
                {
                    check_stack(&text)?;
                    return Ok(text.into_owned());
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Err(format!(
        "no whole `{start}` line on the emulated USB port; got {:?}",
        String::from_utf8_lossy(&got)
    ))
}

/// The USB link test end to end: the emulator's host enumerates rp-usb,
/// opens the port (the banner arrives) and gets its echo back.
pub fn boot_usb_link(root: &Path) -> Result<(), String> {
    if !has_usb_model(root) {
        eprintln!("    usb link: skipped (this emulator has no USB controller model)");
        return Ok(());
    }
    let image = crate::arm_release(root).join("rp-usb");
    let args = usb_args(0);
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let check = || -> Result<(), String> {
        let mut s = usb_connect(0)?;
        usb_read_until(&mut s, "RetroPane 26 USB console OK", 10)?;
        s.write_all(b"echo through USB\r\n")
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "echo through USB", 10)?;
        Ok(())
    };
    boot(
        root,
        &Boot {
            image: &image,
            marker: USB_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &extra,
            input: None,
            during: Some(&check),
            then: None,
            timeout: BOOT_TIMEOUT,
        },
    )?;
    eprintln!("    usb link OK: enumerated by the emulated host, banner and echo over USB");
    Ok(())
}

/// The loader over USB: first light sent through the emulated USB serial
/// port, its run log back the same way.
/// `cargo xtask emu-usb`: the loader-over-USB check on its own.
pub fn boot_loader_usb_alone(root: &Path) -> Result<(), String> {
    usb_images(root)?;
    boot_loader_usb(root)
}

/// `cargo xtask emu-drive`: the drive check alone (item 74: its blocks go
/// by the UDC's DMA both ways).
pub fn boot_loader_drive_alone(root: &Path) -> Result<(), String> {
    usb_images(root)?;
    boot_loader_drive(root)
}

/// The raw images the USB checks send.
pub(crate) fn usb_images(root: &Path) -> Result<(), String> {
    let dir = emu_dir(root)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let release = crate::arm_release(root);
    for image in ["rp-loader", "rp-light"] {
        crate::sdkit::raw_binary(&release.join(image), &dir.join(format!("{image}.bin")))?;
    }
    Ok(())
}

pub fn boot_loader_usb(root: &Path) -> Result<(), String> {
    if !has_usb_model(root) {
        eprintln!("    loader over USB: skipped (this emulator has no USB controller model)");
        return Ok(());
    }
    let loader = raw_image(root, "rp-loader")?;
    let light = raw_image(root, "rp-light")?;
    let payload = std::fs::read(&light).map_err(|e| format!("{}: {e}", light.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let args = usb_args(1);
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let check = || -> Result<(), String> {
        let mut s = usb_connect(1)?;
        usb_read_until(&mut s, "RPLOAD1 READY", 10)?;
        s.write_all(&frame).map_err(|e| e.to_string())?;
        let ok = usb_read_line(&mut s, "RPL OK sha256 ", 15)?;
        let theirs = ok.rsplit("RPL OK sha256 ").next().unwrap_or("").trim();
        crate::usb::check_digest(theirs, &payload, "image over USB")?;
        // The loader disconnects to run the image and reconnects when it
        // returns (the emulated host enumerates it again); the socket stays
        // up throughout, and the run log arrives once the port is reopened.
        let log = usb_read_until(&mut s, "RPL END", 30)?;
        if !log.contains("all steps OK") || !log.contains("slot: fresh") {
            return Err(format!("run log over USB incomplete: {log:?}"));
        }
        // The same image again under the same loader: it must start fresh
        // (rt::Slot: filled once per start, `.bss` zeroed by `_start`), not
        // find the first run's state. Sent with the card log's name in the
        // same transfer, as `cargo xtask load` sends it: the image's first
        // bytes share a packet with the name's end (2026-10-20, on the
        // X51v: the loader dropped them and the image never arrived).
        let mut both = rp_loadproto::control_frame("name light");
        both.extend_from_slice(&frame);
        s.write_all(&both).map_err(|e| e.to_string())?;
        usb_read_line(&mut s, "RPL OK name", 15)?;
        usb_read_line(&mut s, "RPL OK sha256 ", 15)?;
        let again = usb_read_until(&mut s, "RPL END", 30)?;
        if !again.contains("all steps OK") || !again.contains("slot: fresh") {
            return Err(format!("second run of the same image not fresh: {again:?}"));
        }
        // A new loader sent as an image takes over; its predecessor's run is
        // no hang (2026-10-16: the device saved a 45-byte "did not return"
        // report over crashlog.txt each time a loader was tried from RAM).
        let new_loader =
            std::fs::read(&loader).map_err(|e| format!("{}: {e}", loader.display()))?;
        s.write_all(&rp_loadproto::frame(&new_loader))
            .map_err(|e| e.to_string())?;
        usb_read_line(&mut s, "RPL OK sha256 ", 15)?;
        // The new loader sends the run's log by itself once the port is open.
        let run = usb_read_until(&mut s, "RPL END", 30)?;
        if !run.contains("[the image was a new loader: it took over]")
            || run.contains("did not return")
        {
            return Err(format!("loader hand-over reported as: {run:?}"));
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
            timeout: Duration::from_secs(60),
        },
    )?;
    eprintln!(
        "    loader over USB OK: image in, run log out, through the emulated USB port; the same image twice, fresh both times"
    );
    Ok(())
}

/// The trace over USB (item 54): the loader runs rp-threads (which carries
/// on the ring the loader started), and after it returned, `trace` hands
/// the ring over: the image's start and end marks and its thread switches
/// must decode, in order, none torn.
pub fn boot_loader_trace(root: &Path) -> Result<(), String> {
    if !has_usb_model(root) {
        eprintln!("    trace: skipped (this emulator has no USB controller model)");
        return Ok(());
    }
    let dir = emu_dir(root)?;
    let release = crate::arm_release(root);
    crate::sdkit::raw_binary(&release.join("rp-loader"), &dir.join("rp-loader.bin"))?;
    let threads = dir.join("rp-threads.bin");
    crate::sdkit::raw_binary(&release.join("rp-threads"), &threads)?;
    let payload = std::fs::read(&threads).map_err(|e| format!("{}: {e}", threads.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let loader = dir.join("rp-loader.bin");
    let args = usb_args(11);
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let check = || -> Result<(), String> {
        let mut s = usb_connect(11)?;
        usb_read_until(&mut s, "RPLOAD1 READY", 10)?;
        s.write_all(&frame).map_err(|e| e.to_string())?;
        let log = usb_read_until(&mut s, "RPL END", 60)?;
        if !log.contains(THREADS_MARKER) {
            return Err(format!("rp-threads didn't pass under the loader: {log:?}"));
        }
        s.write_all(&rp_loadproto::control_frame("trace"))
            .map_err(|e| e.to_string())?;
        let reply = usb_read_until(&mut s, " kept\r\n", 20)?;
        let lines: Vec<String> = reply
            .lines()
            .filter_map(|l| l.trim_end().strip_prefix("RPD "))
            .map(str::to_string)
            .collect();
        let words = crate::trace::parse(&lines)?;
        let (total, events) =
            rp_kernel_trace::decode_words(&words).ok_or("trace: no ring in the reply")?;
        let events: Vec<rp_kernel_trace::Event> = events.collect();
        use rp_kernel_trace::Kind;
        let marks: Vec<(u32, u32)> = events
            .iter()
            .filter(|e| e.kind == Kind::Mark)
            .map(|e| (e.a, e.b))
            .collect();
        let switches = events.iter().filter(|e| e.kind == Kind::Switch).count();
        let in_order = events.windows(2).all(|w| w[1].seq == w[0].seq + 1);
        // Marks: the image's start (1) and its end with every check passed
        // (2, 1); if the ring wrapped, the start may be gone.
        let ended = marks.last() == Some(&(2, 1));
        let started = marks.first() == Some(&(1, 0)) || u64::from(total) > events.len() as u64;
        if !(ended && started && switches > 10 && in_order) {
            return Err(format!(
                "trace: total {total}, {} kept, marks {marks:?}, {switches} switches, in order {in_order}",
                events.len()
            ));
        }
        eprintln!(
            "    trace OK: {total} events from rp-threads under the loader ({switches} switches kept), decoded on the PC"
        );
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
    Ok(())
}
