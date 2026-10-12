//! The PC side of the USB link (ADR-0012), through Windows' built-in serial
//! driver:
//!
//! - `cargo xtask usb`: with the USB link test (`rp-usb`) running, finds the
//!   COM port by RetroPane's USB IDs, opens it with DTR set (which makes the
//!   device send its banner), sends a line and checks it comes back.
//! - `cargo xtask load <image>`: with the resident loader (`rp-loader`)
//!   running, sends an image (an image name such as `rp-probe`, built first,
//!   or a `.bin` file) and waits for the loader's reply.
//!
//! PowerShell's `System.IO.Ports.SerialPort` does the port handling, so no
//! extra crates are needed; the script is passed base64-encoded (UTF-16LE,
//! as `-EncodedCommand` expects) so no quoting can break it.

use std::path::Path;
use std::process::Command;

/// Must match `rp_drv_pxa_udc::usb::{VENDOR_ID, PRODUCT_ID}`.
const VID_PID: &str = "VID_1209&PID_0001";
/// Must match `rp_usb::BANNER` (without the line ending).
const BANNER: &str = "RetroPane 26 USB console OK";
/// The line sent for the echo check.
const PROBE: &str = "RetroPane echo test 26";
/// The scheduled task that restarts the Axim's USB device entry with admin
/// rights (`cargo xtask load --setup` creates it once). The X51v's D+
/// pull-up is a fixed resistor (docs/hw/udc.md), so the PC never notices
/// the device leaving and returning; restarting its device entry makes
/// Windows enumerate it afresh, which a cable replug otherwise did.
const RESTART_TASK: &str = "RetroPane USB restart";
/// The script the task runs: restarts every present USB entry that could be
/// the Axim (ours, Windows Mobile's, or a failed one), so the port is
/// enumerated afresh.
const RESTART_SCRIPT: &str = "xtask/scripts/usb-restart.ps1";

/// Finds and opens the port (prints `PORT COMn`, or `NOPORT` and exits).
fn open_port() -> String {
    format!(
        r#"$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$dev = Get-CimInstance Win32_PnPEntity | Where-Object {{ $_.PNPDeviceID -like 'USB\{VID_PID}*' -and $_.Name -match '\(COM\d+\)' }} | Select-Object -First 1
if (-not $dev) {{ 'NOPORT'; exit 0 }}
$port = [regex]::Match($dev.Name, 'COM\d+').Value
"PORT $port"
$sp = New-Object System.IO.Ports.SerialPort $port, 115200
"#
    )
}

fn script() -> String {
    open_port()
        + &format!(
            r#"$sp.DtrEnable = $true
$sp.RtsEnable = $true
$sp.ReadTimeout = 3000
$sp.NewLine = "`r`n"
$sp.Open()
try {{ "BANNER " + $sp.ReadLine() }} catch {{ 'NOBANNER' }}
$sp.WriteLine('{PROBE}')
try {{ "ECHO " + $sp.ReadLine() }} catch {{ 'NOECHO' }}
$sp.Close()
"#
        )
}

fn load_script(frame: &Path) -> String {
    let path = frame.display().to_string().replace('\'', "''");
    open_port()
        + &format!(
            r#"$bytes = [System.IO.File]::ReadAllBytes('{path}')
$sp.DtrEnable = $true
$sp.RtsEnable = $true
$sp.ReadTimeout = 5000
$sp.WriteTimeout = 20000
$sp.NewLine = "`r`n"
$sp.Open()
$sp.DiscardInBuffer()
$sp.Write($bytes, 0, $bytes.Length)
"SENT " + $bytes.Length
$deadline = (Get-Date).AddSeconds(10)
$reply = ''
while ((Get-Date) -lt $deadline) {{
    try {{ $l = $sp.ReadLine() }} catch {{ break }}
    if ($l -like 'RPL *') {{ $reply = $l; "REPLY " + $l; break }}
}}
try {{ $sp.Close() }} catch {{ }}
if ($reply -like 'RPL ERR*') {{ exit 0 }}
# No reply read is normal too: the device disconnects to run the image
# within half a second of saying OK, and the port may go away first.
# The device disconnects to run the image. If the image returns to the
# loader, it reconnects and sends the run log: reopen the port and read it.
    $deadline = (Get-Date).AddSeconds({LOG_WAIT_SECS})
$done = $false
# The PC doesn't notice the device leaving and returning by itself: every
# few seconds, restart its device entry (admin rights come from the
# scheduled task; without it this is a no-op and a cable replug is needed).
$lastKick = Get-Date
while (-not $done -and (Get-Date) -lt $deadline) {{
    Start-Sleep -Milliseconds 500
    if (((Get-Date) - $lastKick).TotalSeconds -ge 8) {{
        schtasks /Run /TN '{RESTART_TASK}' 2>&1 | Out-Null
        $lastKick = Get-Date
        Start-Sleep -Milliseconds 2500
    }}
    $dev = Get-CimInstance Win32_PnPEntity | Where-Object {{ $_.PNPDeviceID -like 'USB\{VID_PID}*' -and $_.Name -match '\(COM\d+\)' }} | Select-Object -First 1
    if (-not $dev) {{ continue }}
    $port = [regex]::Match($dev.Name, 'COM\d+').Value
    try {{
        $sp = New-Object System.IO.Ports.SerialPort $port, 115200
        $sp.DtrEnable = $true
        $sp.RtsEnable = $true
        $sp.ReadTimeout = 2000
        $sp.NewLine = "`r`n"
        $sp.Open()
    }} catch {{ continue }}
    $inlog = $false
    $sp.NewLine = "`n"
    while ((Get-Date) -lt $deadline) {{
        try {{ $l = $sp.ReadLine().TrimEnd("`r") }}
        catch [System.TimeoutException] {{ continue }}
        catch {{ break }}
        if ($l -eq 'RPL LOG') {{ $inlog = $true; continue }}
        if (-not $inlog -and $l -eq 'RPLOAD1 READY') {{ 'RESTARTED'; $done = $true; break }}
        if ($l -eq 'RPL END') {{ 'LOGEND'; $done = $true; break }}
        if ($inlog) {{ 'LOG ' + $l }}
    }}
    try {{ $sp.Close() }} catch {{ }}
}}
"#
        )
}

/// Sends a file frame and reads the loader's reply (the loader stays up, so
/// no reconnect dance; a card write and its read-back take a few seconds).
fn put_script(frame: &Path) -> String {
    let path = frame.display().to_string().replace('\'', "''");
    open_port()
        + &format!(
            r#"$bytes = [System.IO.File]::ReadAllBytes('{path}')
$sp.DtrEnable = $true
$sp.RtsEnable = $true
$sp.ReadTimeout = 5000
$sp.WriteTimeout = 30000
$sp.NewLine = "`r`n"
$sp.Open()
$sp.DiscardInBuffer()
$sp.Write($bytes, 0, $bytes.Length)
"SENT " + $bytes.Length
$deadline = (Get-Date).AddSeconds({PUT_WAIT_SECS})
while ((Get-Date) -lt $deadline) {{
    try {{ $l = $sp.ReadLine() }} catch [System.TimeoutException] {{ continue }} catch {{ break }}
    if ($l -like 'RPL *') {{ "REPLY " + $l; break }}
}}
try {{ $sp.Close() }} catch {{ }}
"#
        )
}

/// Like [`put_script`], but also passes on every `RPD ` data line before
/// the reply (listings, files).
fn query_script(frame: &Path) -> String {
    put_script(frame).replace(
        "    if ($l -like 'RPL *') {",
        "    if ($l -like 'RPD *') { $l; continue }\n    if ($l -like 'RPL *') {",
    )
}

/// Sends a control command; returns its `RPD` data lines (without the
/// prefix) and the reply after `RPL OK `.
pub fn query(root: &Path, command: &str) -> Result<(Vec<String>, String), String> {
    let out = root.join("target/load");
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let frame = out.join("query.frame");
    std::fs::write(&frame, rp_loadproto::control_frame(command))
        .map_err(|e| format!("{}: {e}", frame.display()))?;
    let out = powershell(&query_script(&frame))?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    if text.contains("NOPORT") {
        return Err(format!("no RetroPane USB port ({VID_PID}) found"));
    }
    parse_query(&text, command)
}

/// The data lines and the OK reply in a query's output.
pub fn parse_query(text: &str, command: &str) -> Result<(Vec<String>, String), String> {
    let data = text
        .lines()
        .filter_map(|l| l.trim_end().strip_prefix("RPD "))
        .map(str::to_string)
        .collect();
    let reply = text
        .lines()
        .find_map(|l| {
            let l = l.trim_end();
            l.strip_prefix("REPLY ")
                .or_else(|| l.starts_with("RPL ").then_some(l))
        })
        .ok_or("no reply from the loader")?;
    match reply.strip_prefix("RPL OK ") {
        Some(ok) => Ok((data, ok.to_string())),
        None => Err(format!("the loader refused `{command}`: {reply}")),
    }
}

/// A file's base64 lines decoded and checked against the reply's length
/// and CRC (`<n> bytes crc <hex>`).
pub fn decode_file(lines: &[String], reply: &str) -> Result<Vec<u8>, String> {
    let mut data = Vec::new();
    let mut buf = [0u8; rp_loadproto::b64::LINE_BYTES];
    for l in lines {
        let n = rp_loadproto::b64::decode(l.as_bytes(), &mut buf)
            .ok_or_else(|| format!("bad data line from the loader: {l:?}"))?;
        data.extend_from_slice(&buf[..n]);
    }
    let mut words = reply.split_whitespace();
    let len: usize = words
        .next()
        .and_then(|w| w.parse().ok())
        .ok_or("no length in the reply")?;
    let crc = words
        .nth(2)
        .and_then(|w| u32::from_str_radix(w, 16).ok())
        .ok_or("no CRC in the reply")?;
    if data.len() != len || rp_loadproto::crc32(&data) != crc {
        return Err(format!(
            "the file arrived damaged: {} bytes (sent {len}), CRC {:08x} (sent {crc:08x})",
            data.len(),
            rp_loadproto::crc32(&data)
        ));
    }
    Ok(data)
}

/// `cargo xtask ls [folder]`.
pub fn ls(root: &Path, folder: &str) -> Result<(), String> {
    let (entries, reply) = query(root, &format!("ls {folder}"))?;
    for e in &entries {
        let (size, name) = e.split_once(' ').unwrap_or(("?", e));
        if size == "d" {
            println!("{:>12}  {name}/", "");
        } else {
            println!("{size:>12}  {name}");
        }
    }
    eprintln!("    {reply}");
    Ok(())
}

/// `cargo xtask get <file> [local]`: never overwrites a local file.
pub fn get(root: &Path, file: &str, local: Option<&str>) -> Result<(), String> {
    let name = local.unwrap_or_else(|| file.rsplit(['/', '\\']).next().unwrap_or(file));
    let dest = Path::new(name);
    if dest.exists() {
        return Err(format!(
            "{} already exists: name another file to save to",
            dest.display()
        ));
    }
    let (lines, reply) = query(root, &format!("get {file}"))?;
    let data = decode_file(&lines, &reply)?;
    std::fs::write(dest, &data).map_err(|e| format!("{}: {e}", dest.display()))?;
    eprintln!(
        "    {} bytes, CRC checked, saved as {}",
        data.len(),
        dest.display()
    );
    Ok(())
}

/// How long `cargo xtask put` waits for the loader's verdict on a file.
const PUT_WAIT_SECS: u32 = 90;

/// How long `cargo xtask load` waits for the image to return to the loader
/// with its run log: the loader's 30 s run limit, then a watchdog reset and
/// Windows Mobile's boot back into the loader.
const LOG_WAIT_SECS: u32 = 120;

/// Base64 of `text` as UTF-16LE, for PowerShell's `-EncodedCommand`.
fn encode_command(text: &str) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |a, (i, &b)| a | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ABC[(n >> (18 - 6 * i)) as usize & 63]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// What the check found.
#[derive(Debug, PartialEq, Eq)]
struct Outcome {
    port: Option<String>,
    banner: Option<String>,
    echo: Option<String>,
}

fn parse(output: &str) -> Outcome {
    let mut o = Outcome {
        port: None,
        banner: None,
        echo: None,
    };
    for line in output.lines().map(str::trim) {
        if let Some(p) = line.strip_prefix("PORT ") {
            o.port = Some(p.to_string());
        } else if let Some(b) = line.strip_prefix("BANNER ") {
            o.banner = Some(b.to_string());
        } else if let Some(e) = line.strip_prefix("ECHO ") {
            o.echo = Some(e.to_string());
        }
    }
    o
}

fn powershell(script: &str) -> Result<std::process::Output, String> {
    Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-EncodedCommand"])
        .arg(encode_command(script))
        .output()
        .map_err(|e| format!("powershell: {e}"))
}

/// Creates the admin scheduled task that restarts the Axim's USB device
/// entry (one UAC prompt, once). Without it, every `load` needs a cable
/// replug for the PC to find the loader again.
pub fn setup_restart_task(root: &Path) -> Result<(), String> {
    let script = root.join(RESTART_SCRIPT);
    let script = script
        .to_str()
        .ok_or("repository path isn't valid text")?
        .replace('\'', "''");
    let register = format!(
        "Register-ScheduledTask -TaskName '{RESTART_TASK}' -Action (New-ScheduledTaskAction \
         -Execute 'powershell.exe' -Argument '-NoProfile -ExecutionPolicy Bypass -File \"{script}\"') \
         -RunLevel Highest -Force | Out-Null"
    );
    // Elevated through UAC (the user approves the prompt), then waited for.
    let script = format!(
        "$p = Start-Process powershell -Verb RunAs -Wait -PassThru -ArgumentList \
         '-NoProfile','-EncodedCommand','{}'; exit $p.ExitCode",
        encode_command(&register)
    );
    let out = powershell(&script)?;
    if !out.status.success() {
        return Err(format!(
            "creating the task failed (UAC declined?){}",
            stderr_note(&out.stderr)
        ));
    }
    let check = powershell(&format!(
        "if (Get-ScheduledTask -TaskName '{RESTART_TASK}' -ErrorAction SilentlyContinue) {{ 'OK' }}"
    ))?;
    if !String::from_utf8_lossy(&check.stdout).contains("OK") {
        return Err("the task was not created".into());
    }
    eprintln!(
        "    task '{RESTART_TASK}' created: cargo xtask load now re-finds the Axim by itself"
    );
    Ok(())
}

/// Sends the image `what` (an image name, built first, or a `.bin` file) to
/// the resident loader.
pub fn load(root: &Path, what: &str) -> Result<(), String> {
    let bin = if what.ends_with(".bin") {
        Path::new(what).to_path_buf()
    } else {
        crate::cross()?;
        let out = root.join("target/load");
        std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
        let bin = out.join(format!("{what}.bin"));
        let elf = root
            .join("target/armv5te-x51v-none-eabi/release")
            .join(what);
        if !elf.exists() {
            return Err(format!("no image `{what}` (looked for {})", elf.display()));
        }
        crate::sdkit::raw_binary(&elf, &bin)?;
        bin
    };
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let frame = bin.with_extension("frame");
    std::fs::write(&frame, rp_loadproto::frame(&payload))
        .map_err(|e| format!("{}: {e}", frame.display()))?;
    let mut out = powershell(&load_script(&frame))?;
    if String::from_utf8_lossy(&out.stdout).contains("NOPORT") {
        // Windows still holds a stale entry for the port (Windows Mobile's
        // device after a fresh start, or ours): restart it and look again.
        eprintln!("    no port yet: asking Windows to re-find the Axim");
        let _ = powershell(&format!(
            "schtasks /Run /TN '{RESTART_TASK}' | Out-Null; Start-Sleep 6"
        ));
        out = powershell(&load_script(&frame))?;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut port = None;
    let mut reply = None;
    let mut log = Vec::new();
    let mut returned = false;
    let mut restarted = false;
    for line in text.lines() {
        let trimmed = line.trim_end();
        if let Some(p) = trimmed.strip_prefix("PORT ") {
            port.get_or_insert_with(|| p.to_string());
        } else if let Some(r) = trimmed.strip_prefix("REPLY ") {
            reply = Some(r.to_string());
        } else if let Some(l) = trimmed.strip_prefix("LOG ") {
            log.push(l.to_string());
        } else if trimmed == "LOG" {
            log.push(String::new());
        } else if trimmed == "LOGEND" {
            returned = true;
        } else if trimmed == "RESTARTED" {
            restarted = true;
        }
    }
    let Some(port) = port else {
        return Err(format!(
            "no RetroPane USB port ({VID_PID}) found. Is the loader running with the cable plugged in?{}",
            stderr_note(&out.stderr)
        ));
    };
    // The loader's fingerprint of what it received must be ours.
    if let Some(theirs) = reply
        .as_deref()
        .and_then(|r| r.strip_prefix("RPL OK sha256 "))
    {
        check_digest(theirs, &payload, what)?;
    }
    let reply = reply.map(|r| {
        if r.starts_with("RPL OK") {
            "RPL OK".to_string()
        } else {
            r
        }
    });
    match reply.as_deref() {
        Some("RPL OK") | None => {
            if reply.is_none() {
                eprintln!(
                    "    {} bytes sent on {port}; no reply read (the device disconnects to run the image, so this is usual): waiting for the run log",
                    payload.len()
                );
            } else {
                eprintln!(
                    "    {} bytes sent on {port}: the loader started it",
                    payload.len()
                );
            }
            if what == "rp-loader" && (restarted || returned) {
                // A new loader sent as an image replaces the running one in
                // RAM (it moves itself home first) and announces itself; it
                // never "returns", so the run log says the image didn't,
                // and that is the success here.
                eprintln!(
                    "    the new loader is running from RAM; `cargo xtask install` puts it on the card"
                );
                return Ok(());
            }
            if restarted {
                return Err(
                    "the image hung: the watchdog reset the device and the loader came \
                     back without a run log (the reset cleared it)"
                        .into(),
                );
            }
            if returned {
                eprintln!("    the image returned to the loader; its run log:");
                for l in &log {
                    println!("{l}");
                }
                if log.iter().any(|l| l.contains("did not return")) {
                    return Err("the image hung: the watchdog reset the device (log above)".into());
                }
                if log.iter().any(|l| l.contains("[the image crashed")) {
                    return Err(
                        "the image crashed (log above; also on the card as crashlog.txt)".into(),
                    );
                }
                // A probe report: every line's checksum, checked here.
                let text = log.join("\n");
                if text.contains("P1|") {
                    let bad: Vec<&String> = log
                        .iter()
                        .filter(|l| l.starts_with('P') && l.contains('|'))
                        .filter(|l| crate::emu::check_report_line(l).is_err())
                        .collect();
                    if bad.is_empty() {
                        eprintln!("    probe report: every line's checksum is valid");
                    } else {
                        return Err(format!("probe report lines with bad checksums: {bad:?}"));
                    }
                }
            } else {
                eprintln!(
                    "    no run log within {LOG_WAIT_SECS} s: the image is still running (or hung), \
                     or doesn't return to the loader"
                );
            }
            Ok(())
        }
        Some(r) => Err(format!("the loader refused the image: {r}")),
    }
}

/// Writes `file` to the card's root directory as `name` (its own name by
/// default) through the resident loader, which replaces the file there and
/// reads it back before saying OK.
pub fn put(root: &Path, file: &str, name: Option<&str>) -> Result<(), String> {
    let path = Path::new(file);
    let name = match name {
        Some(n) => n.to_string(),
        None => path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("the file has no name")?
            .to_string(),
    };
    let contents = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let out = root.join("target/load");
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let frame = out.join("put.frame");
    std::fs::write(&frame, rp_loadproto::file_frame(&name, &contents))
        .map_err(|e| format!("{}: {e}", frame.display()))?;
    let mut text = run_put(&frame)?;
    if text.contains("NOPORT") {
        eprintln!("    no port yet: asking Windows to re-find the Axim");
        let _ = powershell(&format!(
            "schtasks /Run /TN '{RESTART_TASK}' | Out-Null; Start-Sleep 6"
        ));
        text = run_put(&frame)?;
    }
    if text.contains("NOPORT") {
        return Err(format!(
            "no RetroPane USB port ({VID_PID}) found. Is the loader running with the cable plugged in?"
        ));
    }
    let reply = text
        .lines()
        .find_map(|l| l.trim_end().strip_prefix("REPLY "))
        .ok_or_else(|| {
            format!(
                "{} bytes sent, but no reply within {PUT_WAIT_SECS} s",
                contents.len()
            )
        })?;
    if let Some(detail) = reply.strip_prefix("RPL OK") {
        match detail.rsplit_once("sha256 ") {
            Some((_, theirs)) => check_digest(theirs.trim(), &contents, &name)?,
            None => eprintln!(
                "    (this loader reports no SHA-256: update it with `cargo xtask install`)"
            ),
        }
        eprintln!("    {name}: {} bytes on the card:{detail}", contents.len());
        Ok(())
    } else {
        Err(format!("the loader refused {name}: {reply}"))
    }
}

/// Compares the loader's SHA-256 (hex) of what it holds with ours of what
/// was sent.
pub fn check_digest(theirs: &str, sent: &[u8], what: &str) -> Result<(), String> {
    let ours = rp_crypto::hex(&rp_crypto::Sha256::digest(sent));
    if theirs.as_bytes() == ours {
        eprintln!(
            "    {what}: SHA-256 matches ({})",
            &theirs[..16.min(theirs.len())]
        );
        Ok(())
    } else {
        Err(format!(
            "{what}: the loader holds something else (SHA-256 {theirs}, sent {})",
            String::from_utf8_lossy(&ours)
        ))
    }
}

/// Sends a command to the resident loader (`drive rw` / `drive ro`) and
/// prints its reply.
pub fn command(root: &Path, command: &str) -> Result<(), String> {
    let detail = command_reply(root, command)?;
    eprintln!("    {detail}");
    Ok(())
}

/// Sends a control command and returns the loader's reply (after `RPL OK `),
/// or why it failed.
pub fn command_reply(root: &Path, command: &str) -> Result<String, String> {
    let out = root.join("target/load");
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let frame = out.join("command.frame");
    std::fs::write(&frame, rp_loadproto::control_frame(command))
        .map_err(|e| format!("{}: {e}", frame.display()))?;
    let text = run_put(&frame)?;
    if text.contains("NOPORT") {
        return Err(format!("no RetroPane USB port ({VID_PID}) found"));
    }
    let reply = text
        .lines()
        .find_map(|l| l.trim_end().strip_prefix("REPLY "))
        .ok_or("no reply from the loader")?;
    if let Some(detail) = reply.strip_prefix("RPL OK ") {
        Ok(detail.to_string())
    } else {
        Err(format!("the loader refused `{command}`: {reply}"))
    }
}

/// Runs the checks waiting for the device, read-only and each well inside
/// the loader's run limit, and writes a report (target/device-check.txt):
/// the drive's counters, the SD DMA self-test verdict, the backlight level,
/// and a scan of the card's blocks not yet scanned, in short chunks. With
/// `with_owner` (someone at the device: it makes sound and touches the CF
/// socket): the CF card and a tone.
pub fn device_check(root: &Path, with_owner: bool) -> Result<(), String> {
    let mut steps: Vec<(String, String)> = vec![
        ("drive".into(), "drive status".into()),
        ("SD DMA".into(), "card dma".into()),
        ("backlight".into(), "backlight".into()),
        ("file system".into(), "card check".into()),
    ];
    // The rest of the first FAT (docs/hw/sd.md), 400 blocks a command.
    let (first, last) = (15_074u32, 16_975u32);
    let mut at = first;
    while at <= last {
        let n = (last + 1 - at).min(400);
        steps.push((format!("scan {at}+{n}"), format!("card scan {at} {n}")));
        at += n;
    }
    if with_owner {
        steps.push(("CF card".into(), "cf".into()));
        steps.push(("tone".into(), "tone 440 500".into()));
    }
    let mut report = String::new();
    let mut failed = 0;
    for (name, cmd) in &steps {
        eprintln!("    {name}: {cmd}");
        let line = match command_reply(root, cmd) {
            Ok(r) => format!("OK   {name}: {r}"),
            Err(e) if e.contains("no RetroPane USB port") => {
                return Err(format!("{e} (is the Axim docked and the loader running?)"));
            }
            Err(e) => {
                failed += 1;
                format!("FAIL {name}: {e}")
            }
        };
        eprintln!("    {line}");
        report.push_str(&line);
        report.push('\n');
    }
    if !with_owner {
        report.push_str(
            "(CF and tone skipped: run with --with-owner when someone is at the device)\n",
        );
    }
    let path = root.join("target/device-check.txt");
    std::fs::write(&path, &report).map_err(|e| format!("{}: {e}", path.display()))?;
    eprintln!("    report: {}", path.display());
    if failed > 0 {
        Err(format!("{failed} of {} checks failed", steps.len()))
    } else {
        Ok(())
    }
}

fn run_put(frame: &Path) -> Result<String, String> {
    let out = powershell(&put_script(frame))?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// What `cargo xtask install` puts on the card: (file in the kit, name on
/// the card). The loader is what HaRET starts (the kit's loader folder).
const KIT_FILES: [(&str, &str); 4] = [
    ("loader/retropane.bin", "retropane.bin"),
    ("loader/startup.txt", "startup.txt"),
    ("default.txt", "default.txt"),
    ("haret.exe", "haret.exe"),
];

/// Puts the kit's files on the card through the loader: each replaced,
/// or created if the card doesn't have it yet.
pub fn install(root: &Path) -> Result<(), String> {
    let kit = root.join("target/sdkit");
    for (local, name) in KIT_FILES {
        let path = kit.join(local);
        if !path.exists() {
            return Err(format!(
                "{} is missing: run cargo xtask sdkit first",
                path.display()
            ));
        }
        put(root, &path.display().to_string(), Some(name))?;
    }
    eprintln!("    the card is up to date; soft-reset the Axim to start the new loader");
    Ok(())
}

/// Runs the check.
pub fn run() -> Result<(), String> {
    let out = powershell(&script())?;
    let text = String::from_utf8_lossy(&out.stdout);
    let o = parse(&text);
    let Some(port) = o.port else {
        return Err(format!(
            "no RetroPane USB port ({VID_PID}) found. Is the Axim running rp-usb with its cable plugged in? \
             The screen's status lines show what the PC has done so far.{}",
            stderr_note(&out.stderr)
        ));
    };
    eprintln!("    found the Axim on {port}");
    match o.banner.as_deref() {
        Some(BANNER) => eprintln!("    banner: {BANNER}"),
        Some(other) => eprintln!("    unexpected banner: {other:?}"),
        None => eprintln!("    no banner (the device didn't see the port open)"),
    }
    match o.echo.as_deref() {
        Some(PROBE) => {
            eprintln!("    echo OK: the USB link works both ways");
            Ok(())
        }
        Some(other) => Err(format!("echo came back wrong: {other:?}")),
        None => Err(format!("no echo from {port}{}", stderr_note(&out.stderr))),
    }
}

fn stderr_note(stderr: &[u8]) -> String {
    let s = String::from_utf8_lossy(stderr);
    let s = s.trim();
    // Serialized progress records (not errors) when PowerShell has no console.
    if s.is_empty() || (s.starts_with("#< CLIXML") && !s.contains("S=\"Error\"")) {
        String::new()
    } else {
        format!("\n    powershell said: {s}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_like_powershell_expects() {
        // "dir" as UTF-16LE is 64 00 69 00 72 00.
        assert_eq!(encode_command("dir"), "ZABpAHIA");
        assert_eq!(encode_command("ab"), "YQBiAA==");
    }

    #[test]
    fn parses_a_good_run() {
        let o = parse(
            "PORT COM7\r\nBANNER RetroPane 26 USB console OK\r\nECHO RetroPane echo test 26\r\n",
        );
        assert_eq!(o.port.as_deref(), Some("COM7"));
        assert_eq!(o.banner.as_deref(), Some(BANNER));
        assert_eq!(o.echo.as_deref(), Some(PROBE));
    }

    #[test]
    fn parses_a_missing_port() {
        let o = parse("NOPORT\r\n");
        assert_eq!(o.port, None);
    }

    #[test]
    fn the_script_looks_for_our_ids() {
        assert!(script().contains("USB\\VID_1209&PID_0001*"));
        assert!(load_script(Path::new("x.frame")).contains("USB\\VID_1209&PID_0001*"));
    }

    #[test]
    fn the_log_wait_kicks_the_device_restart_task() {
        let s = load_script(Path::new("x.frame"));
        assert!(s.contains("schtasks /Run /TN 'RetroPane USB restart'"));
    }

    #[test]
    fn the_put_script_waits_for_one_reply_line() {
        let s = put_script(Path::new("p.frame"));
        assert!(s.contains("ReadAllBytes('p.frame')"));
        assert!(s.contains("-like 'RPL *'"));
        assert!(!s.contains("schtasks"), "no reconnect dance for a put");
    }

    #[test]
    fn load_paths_are_quoted_for_powershell() {
        let s = load_script(Path::new("C:/it's/x.frame"));
        assert!(s.contains("ReadAllBytes('C:/it''s/x.frame')"));
    }
}
