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
pub(crate) fn open_port_script() -> String {
    open_port()
}

fn open_port() -> String {
    format!(
        r#"$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$dev = Get-CimInstance Win32_PnPEntity | Where-Object {{ $_.PNPDeviceID -like 'USB\{VID_PID}*' -and $_.Name -match '\(COM\d+\)' }} | Select-Object -First 1
if (-not $dev) {{ 'NOPORT'; exit 0 }}
$port = [regex]::Match($dev.Name, 'COM\d+').Value
"PORT $port"
$sp = New-Object System.IO.Ports.SerialPort $port, 115200
# Windows can take a moment to release the port after the previous
# command closed it (seen as a script dying right after PORT): retry.
function Open-Port {{
    for ($i = 0; $i -lt 20; $i++) {{
        try {{ $sp.Open(); return }} catch {{ Start-Sleep -Milliseconds 250 }}
    }}
    $sp.Open()
}}
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
Open-Port
try {{ "BANNER " + $sp.ReadLine() }} catch {{ 'NOBANNER' }}
$sp.WriteLine('{PROBE}')
try {{ "ECHO " + $sp.ReadLine() }} catch {{ 'NOECHO' }}
$sp.Close()
"#
        )
}

#[cfg(test)]
fn load_script(frame: &Path) -> String {
    load_script_opts(frame, true, 1)
}

/// [`load_script`], or with `wait_log` false only the send and the
/// loader's reply (for [`start`]). `frames`: control frames before the
/// image in the same send (the card log's `name`) each get their own
/// reply first (`EARLY`); the image's is the last.
fn load_script_opts(frame: &Path, wait_log: bool, frames: usize) -> String {
    let log_wait = log_wait_secs();
    let path = frame.display().to_string().replace('\'', "''");
    let stop = if wait_log { "" } else { "exit 0\n" };
    let early = frames.saturating_sub(1);
    open_port()
        + &format!(
            r#"$bytes = [System.IO.File]::ReadAllBytes('{path}')
$sp.DtrEnable = $true
$sp.RtsEnable = $true
$sp.ReadTimeout = 5000
$sp.WriteTimeout = 20000
$sp.NewLine = "`r`n"
Open-Port
$sp.DiscardInBuffer()
$sp.Write($bytes, 0, $bytes.Length)
"SENT " + $bytes.Length
$deadline = (Get-Date).AddSeconds(10)
$reply = ''
$early = {early}
while ((Get-Date) -lt $deadline) {{
    try {{ $l = $sp.ReadLine() }} catch {{ break }}
    if ($l -match '^RPL (OK|ERR)') {{
        if ($early -gt 0) {{ $early -= 1; "EARLY " + $l; continue }}
        $reply = $l; "REPLY " + $l; break
    }}
}}
try {{ $sp.Close() }} catch {{ }}
if ($reply -like 'RPL ERR*') {{ exit 0 }}
{stop}# No reply read is normal too: the device disconnects to run the image
# within half a second of saying OK, and the port may go away first.
# The device disconnects to run the image. If the image returns to the
# loader, it reconnects and sends the run log: reopen the port and read it.
    $deadline = (Get-Date).AddSeconds({log_wait})
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
Open-Port
# A loader holding a run log sends it by itself when the port opens: pass
# it on (EARLY/PUSHED lines, kept by the PC side), never discard it.
Start-Sleep -Milliseconds 300
foreach ($e in ($sp.ReadExisting() -split "`r`n")) {{ if ($e) {{ "EARLY " + $e }} }}
$sp.Write($bytes, 0, $bytes.Length)
"SENT " + $bytes.Length
$deadline = (Get-Date).AddSeconds({PUT_WAIT_SECS})
$pushed = $false
while ((Get-Date) -lt $deadline) {{
    try {{ $l = $sp.ReadLine() }} catch [System.TimeoutException] {{ continue }} catch {{ break }}
    if ($l -eq 'RPL LOG') {{ $pushed = $true; "PUSHED RPL LOG"; continue }}
    if ($pushed) {{ "PUSHED " + $l; if ($l -eq 'RPL END') {{ $pushed = $false }}; continue }}
    if ($l -match '^RPL (OK|ERR)') {{ "REPLY " + $l; break }}
}}
try {{ $sp.Close() }} catch {{ }}
"#
        )
}

/// Like [`put_script`], but also passes on every `RPD ` data line before
/// the reply (listings, files).
fn query_script(frame: &Path) -> String {
    put_script(frame).replace(
        "    if ($l -match '^RPL (OK|ERR)') {",
        "    if ($l -like 'RPD *') { $l; continue }\n    if ($l -match '^RPL (OK|ERR)') {",
    )
}

/// `command` as a control frame, refused here when it is longer than the
/// loader takes (`COMMAND_MAX`): the loader would answer only "bad
/// command" (2026-10-23: `get rp-wmprobe/rp-wm-baseline.txt`, 33 bytes).
fn control_frame_checked(command: &str) -> Result<Vec<u8>, String> {
    if command.len() > rp_loadproto::COMMAND_MAX {
        return Err(format!(
            "`{command}` is {} bytes; the loader takes commands of at most {} (a shorter path, or the file's 8.3 short name)",
            command.len(),
            rp_loadproto::COMMAND_MAX
        ));
    }
    Ok(rp_loadproto::control_frame(command))
}

/// Sends a control command; returns its `RPD` data lines (without the
/// prefix) and the reply after `RPL OK `.
pub fn query(root: &Path, command: &str) -> Result<(Vec<String>, String), String> {
    let out = root.join("target/load");
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let frame = out.join("query.frame");
    std::fs::write(&frame, control_frame_checked(command)?)
        .map_err(|e| format!("{}: {e}", frame.display()))?;
    let text = run_refinding(&query_script(&frame))?;
    if text.contains("NOPORT") {
        return Err(format!("no RetroPane USB port ({VID_PID}) found"));
    }
    keep_unsent_log(root, &text);
    parse_query(&text, command)
}

/// A run log the loader pushed while a command ran (it sends the last
/// run's log once, when the PC next opens the port: after an image ran with
/// the Axim out of its cradle, or after `load` stopped waiting). Saved to
/// `target/load/pushed-log-<seconds>.txt` and named on the console; before
/// 2026-10-14 such logs were thrown away (rp-today's first GPU run).
fn keep_unsent_log(root: &Path, text: &str) {
    let Some(body) = pushed_log(text) else {
        return;
    };
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let path = root.join(format!("target/load/pushed-log-{secs}.txt"));
    if std::fs::write(&path, body.join("\n") + "\n").is_ok() {
        eprintln!(
            "    the loader sent a waiting run log ({} lines): saved to {}",
            body.len(),
            path.display()
        );
    }
}

/// The lines of a run log pushed into a command's output (`EARLY`/`PUSHED`
/// lines from `RPL LOG` up to `RPL END`).
pub fn pushed_log(text: &str) -> Option<Vec<&str>> {
    let lines: Vec<&str> = text
        .lines()
        .filter_map(|l| {
            let l = l.trim_end();
            l.strip_prefix("EARLY ")
                .or_else(|| l.strip_prefix("PUSHED "))
        })
        .collect();
    let start = lines.iter().position(|l| *l == "RPL LOG")?;
    Some(
        lines[start + 1..]
            .iter()
            .take_while(|l| **l != "RPL END")
            .copied()
            .collect(),
    )
}

#[cfg(test)]
mod pushed_tests {
    #[test]
    fn a_pushed_log_is_kept_and_the_reply_still_found() {
        let text = "EARLY RPL LOG\nEARLY touch to flush: 3 answered\nSENT 20\nPUSHED stack: deepest 1 of 2\nPUSHED RPL END\nREPLY RPL ERR bad command\n";
        assert_eq!(
            super::pushed_log(text),
            Some(vec!["touch to flush: 3 answered", "stack: deepest 1 of 2"])
        );
        assert!(super::parse_query(text, "log").is_err_and(|e| e.contains("bad command")));
        assert_eq!(super::pushed_log("REPLY RPL OK cpld"), None);
    }
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
                .or_else(|| (l.starts_with("RPL OK") || l.starts_with("RPL ERR")).then_some(l))
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

/// The name a run's card log carries: the image's, without `rp-` (lowercase
/// letters, digits, `-`, `_`; `run` otherwise, as the loader would).
pub fn card_log_name(image: &str) -> String {
    let base = image.rsplit(['/', '\\']).next().unwrap_or(image);
    let base = base.strip_suffix(".bin").unwrap_or(base);
    let name = base
        .strip_prefix("rp-")
        .unwrap_or(base)
        .to_ascii_lowercase();
    if rp_hal_x51v::cardlog::name_ok(name.as_bytes()) {
        name
    } else {
        "run".to_string()
    }
}

/// The card logs listed in `entries` (`ls` lines): (number, file name), of
/// `image` (or any) and `number` (or any).
fn card_logs(entries: &[String], image: Option<&str>, number: Option<u32>) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    for e in entries {
        let Some((size, name)) = e.split_once(' ') else {
            continue;
        };
        if size == "d" {
            continue;
        }
        let Some((who, rest)) = name.rsplit_once('-') else {
            continue;
        };
        let Some(n) = rest
            .strip_suffix(".txt")
            .and_then(|d| d.parse::<u32>().ok())
        else {
            continue;
        };
        if image.is_some_and(|i| i != who) || number.is_some_and(|k| k != n) {
            continue;
        }
        out.push((n, name.to_string()));
    }
    out
}

/// `cargo xtask cardlog [image] [number]`: a run's log kept on the cards
/// as the run went (`rp_hal_x51v::cardlog`: a slot file `rplog/log-SS.txt`
/// found through `rplog/index.txt`, or an old-style `rplog/<image>-NNNN.txt`,
/// on the SD card, and on the CF card for what a program logged while it
/// had the SD card), the newest (of `image`, or of any image) or the one
/// numbered; both halves joined, saved under `local/cardlog/`, its text
/// printed. A log without the closing line is a run that didn't come
/// back: said so.
pub fn cardlog(root: &Path, image: Option<&str>, number: Option<u32>) -> Result<(), String> {
    let folder = rp_hal_x51v::cardlog::FOLDER;
    let image = image.map(card_log_name);
    let image = image.as_deref();
    // The slots first (R22): each card's index names the slot holding a run.
    if let Some(found) = cardlog_from_slots(root, image, number)? {
        return show_card_log(root, &found.file, found.sd.as_deref(), found.cf.as_deref());
    }
    let sd = query(root, &format!("ls {folder}"))
        .map(|(e, _)| card_logs(&e, image, number))
        .unwrap_or_default();
    let cf = query(root, &format!("ls cf:{folder}"))
        .map(|(e, _)| card_logs(&e, image, number))
        .unwrap_or_default();
    // Newest: the highest number (per image the numbers rise; across
    // images, the listing's last of equal numbers).
    let Some((n, name)) = sd
        .iter()
        .chain(&cf)
        .fold(None::<&(u32, String)>, |best, e| match best {
            Some(b) if b.0 > e.0 => Some(b),
            _ => Some(e),
        })
        .cloned()
    else {
        return Err(format!(
            "no card log{} in {folder}/ on either card",
            image.map(|i| format!(" of {i}")).unwrap_or_default(),
        ));
    };
    let fetch = |unit: &str, list: &[(u32, String)]| -> Result<Option<Vec<u8>>, String> {
        if !list.iter().any(|(k, f)| *k == n && *f == name) {
            return Ok(None);
        }
        let (lines, reply) = query(root, &format!("get {unit}{folder}/{name}"))?;
        decode_file(&lines, &reply).map(Some)
    };
    let sd_part = fetch("", &sd)?;
    let cf_part = fetch("cf:", &cf)?;
    show_card_log(root, &name, sd_part.as_deref(), cf_part.as_deref())
}

/// A run's log found through the cards' `rplog/index.txt` (the slots,
/// R22): the run's name for the saved file and each card's half; `None`
/// when neither index names such a run (the old-style files are then
/// looked for).
fn cardlog_from_slots(
    root: &Path,
    image: Option<&str>,
    number: Option<u32>,
) -> Result<Option<FoundLog>, String> {
    use rp_hal_x51v::cardlog;
    let index_of = |unit: &str| -> Option<Vec<u8>> {
        let (lines, reply) = query(root, &format!("get {unit}{}", cardlog::INDEX)).ok()?;
        decode_file(&lines, &reply).ok()
    };
    let sd = index_of("");
    let cf = index_of("cf:");
    // The run: by image and number, or the newest (by the index's order).
    let pick = |ix: &Option<Vec<u8>>| {
        ix.as_deref().and_then(|ix| {
            cardlog::find(ix, image.map(str::as_bytes), number).map(|(slot, n)| {
                let e = cardlog::index_entry(ix, slot);
                (
                    slot,
                    n,
                    e.map_or(0, |e| e.order),
                    e.map(|e| e.name.to_vec()),
                )
            })
        })
    };
    let (a, b) = (pick(&sd), pick(&cf));
    let Some((_, n, _, name)) = [&a, &b]
        .into_iter()
        .flatten()
        .max_by_key(|(_, _, order, _)| *order)
        .cloned()
    else {
        return Ok(None);
    };
    let name = name.unwrap_or_default();
    let fetch = |unit: &str,
                 hit: &Option<(usize, u32, u32, Option<Vec<u8>>)>|
     -> Result<Option<Vec<u8>>, String> {
        let Some((slot, k, _, who)) = hit else {
            return Ok(None);
        };
        if *k != n || who.as_deref() != Some(&name[..]) {
            return Ok(None);
        }
        let mut path = [0u8; 32];
        let len = cardlog::slot_path(*slot, &mut path);
        let path = String::from_utf8_lossy(&path[..len]).into_owned();
        let (lines, reply) = query(root, &format!("get {unit}{path}"))?;
        decode_file(&lines, &reply).map(Some)
    };
    let sd_part = fetch("", &a)?;
    let cf_part = fetch("cf:", &b)?;
    let file = format!("{}-{n:04}.txt", String::from_utf8_lossy(&name));
    Ok(Some(FoundLog {
        file,
        sd: sd_part,
        cf: cf_part,
    }))
}

/// A run's log found through the index: its file name, each card's half.
struct FoundLog {
    file: String,
    sd: Option<Vec<u8>>,
    cf: Option<Vec<u8>>,
}

/// Joins a run's two halves, saves it under `local/cardlog/` as `name`
/// and prints it.
fn show_card_log(
    root: &Path,
    name: &str,
    sd_part: Option<&[u8]>,
    cf_part: Option<&[u8]>,
) -> Result<(), String> {
    let folder = rp_hal_x51v::cardlog::FOLDER;
    let joined: Vec<u8> = rp_hal_x51v::cardlog::join(sd_part, cf_part).collect();
    let text = String::from_utf8_lossy(&joined).into_owned();
    let dir = root.join("local/cardlog");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let dest = dir.join(name);
    std::fs::write(&dest, text.as_bytes()).map_err(|e| format!("{}: {e}", dest.display()))?;
    println!("{text}");
    eprintln!(
        "    {folder}/{name}: {} bytes of log ({}), saved as {}",
        text.len(),
        match (sd_part.is_some(), cf_part.is_some()) {
            (true, true) => "the SD and CF cards' parts joined",
            (true, false) => "from the SD card",
            _ => "from the CF card",
        },
        dest.display()
    );
    if !text.contains("[card log closed: ") {
        eprintln!(
            "    no closing line: this run didn't come back and the loader hasn't closed it (a watchdog reset clears what it needs); the log ends at its last line written"
        );
    }
    Ok(())
}

/// How long `cargo xtask put` waits for the loader's verdict on a file.
const PUT_WAIT_SECS: u32 = 90;

/// How long `cargo xtask load` waits for the image to return to the loader
/// with its run log: the loader's 30 s run limit, then a watchdog reset and
/// Windows Mobile's boot back into the loader.
const LOG_WAIT_SECS: u32 = 120;

/// [`LOG_WAIT_SECS`], or `RETROPANE_LOG_WAIT_SECS` for a run known to take
/// longer (`rp-gpubench` run G: up to ~20 minutes; run G2's log was given
/// up on at 120 s while the image still ran).
fn log_wait_secs() -> u32 {
    std::env::var("RETROPANE_LOG_WAIT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(LOG_WAIT_SECS)
}

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

/// Runs a script on the device's port. The one way xtask reaches the Axim,
/// so it takes the machine-wide lock first and keeps it until the process
/// ends (crate::devlock; a test keeps this the only way).
pub(crate) fn powershell(script: &str) -> Result<std::process::Output, String> {
    crate::devlock::ensure_held()?;
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
/// Whether an image is a resident loader (by name or as a `.bin`, e.g. the
/// card's loader put back from RAM): it carries the READY line it sends.
fn is_loader(image: &[u8]) -> bool {
    let ready = rp_loadproto::READY.trim_end().as_bytes();
    image.windows(ready.len()).any(|w| w == ready)
}

/// The image `what` (a name, built first, or a `.bin` file) as bytes, and
/// its load frame written next to it.
/// The image's raw bytes, and a frame file to send: for any image but a
/// loader, the card log's `name` first, in the same send (so it can't be
/// lost on its own); and how many frames the file holds.
fn image_frame(root: &Path, what: &str) -> Result<(Vec<u8>, std::path::PathBuf, usize), String> {
    let bin = if what.ends_with(".bin") {
        Path::new(what).to_path_buf()
    } else {
        crate::cross()?;
        let out = root.join("target/load");
        std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
        let bin = out.join(format!("{what}.bin"));
        let elf = crate::arm_release(root).join(what);
        if !elf.exists() {
            return Err(format!("no image `{what}` (looked for {})", elf.display()));
        }
        crate::sdkit::raw_binary(&elf, &bin)?;
        bin
    };
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let frame = bin.with_extension("frame");
    let mut bytes = Vec::new();
    let mut frames = 1;
    if !is_loader(&payload) {
        bytes = rp_loadproto::control_frame(&format!("name {}", card_log_name(what)));
        frames = 2;
    }
    bytes.extend(rp_loadproto::frame(&payload));
    std::fs::write(&frame, bytes).map_err(|e| format!("{}: {e}", frame.display()))?;
    Ok((payload, frame, frames))
}

/// Sends image `what` and returns as soon as the loader has said it starts
/// it, with that moment: for `cargo xtask measure`, whose schedule counts
/// from the image's start. The run log is left for `cargo xtask log`.
pub fn start(root: &Path, what: &str) -> Result<std::time::Instant, String> {
    let (payload, frame, frames) = image_frame(root, what)?;
    let out = powershell(&load_script_opts(&frame, false, frames))?;
    let at = std::time::Instant::now();
    let text = String::from_utf8_lossy(&out.stdout);
    if text.contains("NOPORT") || !text.contains("PORT ") {
        return Err(format!(
            "no RetroPane USB port ({VID_PID}) found. Is the loader running with the cable plugged in?{}",
            stderr_note(&out.stderr)
        ));
    }
    let reply = text
        .lines()
        .find_map(|l| l.trim_end().strip_prefix("REPLY "))
        .ok_or("the loader didn't confirm the start (no reply): try again")?;
    let theirs = reply
        .strip_prefix("RPL OK sha256 ")
        .ok_or_else(|| format!("the loader refused the image: {reply}"))?;
    check_digest(theirs, &payload, what)?;
    Ok(at)
}

pub fn load(root: &Path, what: &str, force: bool) -> Result<(), String> {
    // An image that never returns would run into the watchdog on the
    // device (found 2026-10-12: rp-bench reset the Axim into Dell's
    // factory-restore prompt). The device gets only images that come back.
    if crate::size::on_device(what) == Some(crate::size::OnDevice::NeverReturns) && !force {
        return Err(format!(
            "`{what}` never returns to the loader (it runs in the emulator, or HaRET starts it \
             directly): on the device the watchdog would reset the Axim. `rp-probe` runs the \
             benchmarks on the device. Add --force to send it anyway."
        ));
    }
    let (payload, frame, frames) = image_frame(root, what)?;
    let mut out = powershell(&load_script_opts(&frame, true, frames))?;
    if String::from_utf8_lossy(&out.stdout).contains("NOPORT") {
        // Windows still holds a stale entry for the port (Windows Mobile's
        // device after a fresh start, or ours): restart it and look again.
        eprintln!("    no port yet: asking Windows to re-find the Axim");
        let _ = powershell(&format!(
            "schtasks /Run /TN '{RESTART_TASK}' | Out-Null; Start-Sleep 6"
        ));
        out = powershell(&load_script_opts(&frame, true, frames))?;
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
        } else if let Some(r) = trimmed.strip_prefix("EARLY ") {
            // The card log's name, sent with the image (2026-10-20: sent
            // on its own first, it was lost while Windows re-found the
            // port after the previous run, and the log was named `run`).
            if r.starts_with("RPL OK") {
                eprintln!(
                    "    card log: {}'s next run in rplog/ on the SD card (and the CF card when one is in); `cargo xtask cardlog` reads it",
                    card_log_name(what)
                );
            } else {
                eprintln!(
                    "    card log: the loader refused the name ({r}); an older loader keeps no card log for every run"
                );
            }
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
            if is_loader(&payload) && (restarted || returned) {
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
                // READY before any run log says only that the log didn't come
                // first: it may have gone out before the PC reopened the port.
                // The loader knows what happened; ask it (2026-10-15: a
                // deliberate crash was reported here as a watchdog reset).
                eprintln!("    the loader is back with no run log first; asking it for the run");
                let lines = after_restart(query(root, "log").map(|(lines, _)| lines))?;
                eprintln!("    the run log, from the loader:");
                for l in &lines {
                    println!("{l}");
                }
                return run_verdict(&lines);
            }
            if returned {
                eprintln!("    the image returned to the loader; its run log:");
                for l in &log {
                    println!("{l}");
                }
                run_verdict(&log)?;
            } else {
                if log.is_empty() {
                    eprintln!(
                        "    no run log within {} s; asking the loader for it",
                        log_wait_secs()
                    );
                } else {
                    eprintln!(
                        "    the run log arrived cut short ({} lines); asking for it again",
                        log.len()
                    );
                }
                if last_log(root).is_err() {
                    eprintln!(
                        "    the loader isn't answering: the image is still running (or hung), \
                         or doesn't return to the loader. Once it's back: cargo xtask log"
                    );
                }
            }
            Ok(())
        }
        Some(r) => Err(format!("the loader refused the image: {r}")),
    }
}

/// What a run log says about the run: `Err` for a hang the watchdog reset
/// (the loader's note), a crash, or a probe report with bad lines.
fn run_verdict(log: &[String]) -> Result<(), String> {
    if log.iter().any(|l| l.contains("did not return")) {
        return Err(
            "the image hung: the watchdog reset the device (log above; an image that keeps a card log has the rest: `cargo xtask cardlog`)"
                .into(),
        );
    }
    if log.iter().any(|l| l.contains("[the image crashed")) {
        return Err("the image crashed (log above; also on the card as crashlog.txt)".into());
    }
    // A probe report: every line's checksum, checked here.
    if log.iter().any(|l| l.contains("P1|")) {
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
    Ok(())
}

/// The run's log after the loader came back with READY first, from the
/// loader's answer to `log`: its lines, or what the answer says happened.
/// Only a loader with no record of any run means the device was reset; a
/// loader that can't be asked proves nothing either way.
fn after_restart(answer: Result<Vec<String>, String>) -> Result<Vec<String>, String> {
    match answer {
        Ok(lines) => Ok(lines),
        Err(e) if e.contains("no image has run") => Err(
            "the loader has no record of the run: the device was reset (a hang the \
             watchdog caught, or power) and the reset cleared the log"
                .into(),
        ),
        Err(e) => Err(format!(
            "no run log, and the loader couldn't be asked for one ({e}): the device's \
             screen says what happened; once the PC sees the loader again: cargo xtask log"
        )),
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
    let text = run_put(&frame)?;
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

/// Prints the last run's log again (the loader's `log` command): for a log
/// lost on the way while Windows re-found the device.
pub fn last_log(root: &Path) -> Result<(), String> {
    let (lines, _) = query(root, "log")?;
    eprintln!("    the last run's log, from the loader:");
    for l in lines {
        println!("{l}");
    }
    Ok(())
}

/// Sends a control command and returns the loader's reply (after `RPL OK `),
/// or why it failed.
pub fn command_reply(root: &Path, command: &str) -> Result<String, String> {
    let out = root.join("target/load");
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let frame = out.join("command.frame");
    std::fs::write(&frame, control_frame_checked(command)?)
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
/// `with_owner` (someone at the device: it touches the CF socket): the CF
/// card. The audio check is an image of its own (`cargo xtask tone`).
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
            "(CF skipped: run with --with-owner when someone is at the device; the tone: cargo xtask tone)\n",
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
    run_refinding(&put_script(frame))
}

/// Runs a script that talks to the loader; when Windows has no port for the
/// Axim (it lost the device when an image handed back to the loader), runs
/// the restart task and tries once more. Every command goes through here,
/// so none of them needs a cable replug after a run.
fn run_refinding(script: &str) -> Result<String, String> {
    let out = powershell(script)?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    if !text.contains("NOPORT") {
        return Ok(text);
    }
    eprintln!("    no port yet: asking Windows to re-find the Axim");
    let _ = powershell(&format!(
        "schtasks /Run /TN '{RESTART_TASK}' | Out-Null; Start-Sleep 6"
    ));
    let out = powershell(script)?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// What `cargo xtask install` puts on the card: (file in the kit, name on
/// the card). The loader is what HaRET starts (the kit's loader folder);
/// the icon pair is the card's drive icon on the PC.
const KIT_FILES: [(&str, &str); 6] = [
    ("loader/retropane.bin", "retropane.bin"),
    ("loader/startup.txt", "startup.txt"),
    ("default.txt", "default.txt"),
    ("haret.exe", "haret.exe"),
    ("cards/sd/card-sd.ico", "card-sd.ico"),
    ("cards/sd/autorun.inf", "autorun.inf"),
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

/// The CF card's folder for HaRET and the loader (reminder R27): Windows
/// Mobile starts `haret.exe` there, and HaRET boots `startup.txt` from its
/// own folder.
pub const CF_KIT_DIR: &str = "RetroPane";

/// The kit's boot files for the CF card: [`KIT_FILES`] less the SD card's
/// drive icon pair, from the same list, so the CF card can't get another
/// file under a boot file's name (2026-10-24: a hand copy put rp-probe
/// there as `retropane.bin`, and left `startup.txt` out).
pub fn cf_kit_files() -> Vec<(&'static str, &'static str)> {
    KIT_FILES
        .iter()
        .copied()
        .filter(|(local, _)| !local.starts_with("cards/"))
        .collect()
}

/// Puts the kit's boot files on the CF card, in [`CF_KIT_DIR`], through the
/// loader (`cf:` names: the PC itself may not change them, the guard keeps
/// them, ADR-0014), each checked by its SHA-256 as the SD card's are, then
/// every file read back through the loader and compared byte for byte with
/// the kit's.
pub fn install_cf(root: &Path) -> Result<(), String> {
    let kit = root.join("target/sdkit");
    let files = cf_kit_files();
    for (local, _) in &files {
        if !kit.join(local).exists() {
            return Err(format!(
                "{} is missing: run cargo xtask sdkit first",
                kit.join(local).display()
            ));
        }
    }
    for (local, name) in &files {
        let on_card = format!("cf:{CF_KIT_DIR}/{name}");
        put(root, &kit.join(local).display().to_string(), Some(&on_card))?;
    }
    for (local, name) in &files {
        let want = std::fs::read(kit.join(local)).map_err(|e| e.to_string())?;
        let (lines, reply) = query(root, &format!("get cf:{CF_KIT_DIR}/{name}"))?;
        let got = decode_file(&lines, &reply).map_err(|e| format!("{name} on the CF card: {e}"))?;
        if got != want {
            return Err(format!(
                "{name} on the CF card differs from the kit's {local}"
            ));
        }
        eprintln!(
            "    {name}: {} bytes, read back from the card the same as {local}",
            got.len()
        );
    }
    eprintln!(
        "    the CF card is up to date: in Windows Mobile, start haret.exe from the CF card's {CF_KIT_DIR} folder"
    );
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

    /// The CF card gets the loader under the loader's name, HaRET's two
    /// scripts and HaRET, all from the SD list, never the kit's top-level
    /// image (the probe) and never the SD card's drive icon.
    #[test]
    fn the_cf_kit_is_the_sd_kit_less_its_icon() {
        let cf = cf_kit_files();
        assert!(cf.contains(&("loader/retropane.bin", "retropane.bin")));
        assert!(cf.contains(&("loader/startup.txt", "startup.txt")));
        assert!(cf.contains(&("default.txt", "default.txt")));
        assert!(cf.contains(&("haret.exe", "haret.exe")));
        assert_eq!(cf.len(), 4);
        assert!(cf.iter().all(|(local, _)| *local != "retropane.bin"));
    }

    #[test]
    fn encodes_like_powershell_expects() {
        // "dir" as UTF-16LE is 64 00 69 00 72 00.
        assert_eq!(encode_command("dir"), "ZABpAHIA");
        assert_eq!(encode_command("ab"), "YQBiAA==");
    }

    /// READY before the run log is not a hang (2026-10-15: a deliberate
    /// crash was reported as a watchdog reset): only the loader's own
    /// record decides.
    #[test]
    fn a_restart_is_judged_by_the_loaders_record_not_by_a_missing_log() {
        let crashed = vec![
            "PANIC deliberate test panic".to_string(),
            "[the image crashed: report kept on the card as crashlog.txt]".to_string(),
        ];
        let lines = after_restart(Ok(crashed)).unwrap();
        let verdict = run_verdict(&lines).unwrap_err();
        assert!(verdict.contains("crashed"), "{verdict}");
        assert!(!verdict.contains("watchdog"), "{verdict}");
        assert!(run_verdict(&after_restart(Ok(vec!["all steps OK".into()])).unwrap()).is_ok());
        let hung = vec!["[the image did not return: it hung and the device was reset]".into()];
        assert!(run_verdict(&hung).unwrap_err().contains("watchdog"));
        let reset = after_restart(Err(
            "the loader refused `log`: RPL ERR log: no image has run since the loader started"
                .into(),
        ))
        .unwrap_err();
        assert!(reset.contains("device was reset"), "{reset}");
        let unknown = after_restart(Err("no reply from the loader".into())).unwrap_err();
        assert!(
            !unknown.contains("watchdog") && !unknown.contains("hung"),
            "{unknown}"
        );
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

    /// The card log's name goes in the same send as the image: its reply
    /// comes first and is set apart, the image's is the one acted on.
    #[test]
    fn a_name_sent_with_the_image_has_its_reply_set_apart() {
        let s = load_script_opts(Path::new("x.frame"), true, 2);
        assert!(s.contains("$early = 1"));
        assert!(s.contains("\"EARLY \" + $l"));
        assert!(load_script(Path::new("x.frame")).contains("$early = 0"));
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
        assert!(s.contains("-match '^RPL (OK|ERR)'"));
        assert!(!s.contains("schtasks"), "no reconnect dance for a put");
    }

    #[test]
    fn a_pushed_run_log_is_not_a_reply() {
        // The loader sends the last run's log (RPL LOG .. RPL END) by itself
        // when the port opens; its end marker once passed for `log`'s reply,
        // twice: the second time its start marker was lost. Only OK and ERR
        // lines are replies.
        for s in [
            put_script(Path::new("p.frame")),
            query_script(Path::new("q.frame")),
        ] {
            let skip = s.find("if ($pushed)").expect("pushed log skipped");
            assert!(skip < s.find("-match '^RPL (OK|ERR)'").unwrap());
        }
    }

    #[test]
    fn a_loader_is_known_by_its_ready_line() {
        assert!(is_loader(b"..\x00RPLOAD1 READY\r\n.."));
        assert!(!is_loader(b"RPLOAD1 READ"));
    }

    #[test]
    fn load_paths_are_quoted_for_powershell() {
        let s = load_script(Path::new("C:/it's/x.frame"));
        assert!(s.contains("ReadAllBytes('C:/it''s/x.frame')"));
    }
}

#[cfg(test)]
mod command_length_tests {
    use super::control_frame_checked;

    #[test]
    fn a_command_the_loader_cannot_take_is_refused_on_the_pc() {
        let max = rp_loadproto::COMMAND_MAX;
        let at = "g".repeat(max);
        assert_eq!(
            control_frame_checked(&at),
            Ok(rp_loadproto::control_frame(&at))
        );
        let over = "g".repeat(max + 1);
        let err = control_frame_checked(&over).unwrap_err();
        assert!(err.contains(&format!("{} bytes", max + 1)), "{err}");
        // The command that met it (2026-10-23).
        assert!(control_frame_checked("get rp-wmprobe/rp-wm-baseline.txt").is_err());
        assert!(control_frame_checked("get rp-wmprobe/rp-wmprobe.txt").is_ok());
    }
}
