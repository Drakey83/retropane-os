//! `cargo xtask usbbench`: USB throughput through the resident loader
//! (audit item 74, docs/perf.md). Two paths, measured the same way before
//! and after any driver change:
//!
//! - **Serial (image loading):** an image frame of [`SERIAL_BYTES`] random
//!   bytes with a deliberately wrong CRC. The loader takes every byte, finds
//!   the CRC wrong and answers `RPL ERR bad crc` without running anything,
//!   so the time from the first byte written to that reply is the transfer
//!   (plus one CRC pass on the device, ~2% of it), and the loader stays up.
//! - **Drive (mass storage):** a [`DRIVE_BYTES`] file of random bytes
//!   written to the SD card's drive with Windows' cache bypassed
//!   (`FILE_FLAG_NO_BUFFERING | FILE_FLAG_WRITE_THROUGH`), read back the
//!   same way, compared byte for byte, then deleted. The card's own speed is
//!   in these numbers (docs/perf.md: ~1.4-1.9 MB/s through the file layer).
//!
//! The Axim lock is held for the whole run: another session's load would
//! take the drive away mid-file.

use std::path::{Path, PathBuf};
use std::time::Instant;

/// Bytes in the serial frame: just under the loader's image area.
pub const SERIAL_BYTES: usize = 900 * 1024;
/// Bytes in the drive file.
pub const DRIVE_BYTES: usize = 16 * 1024 * 1024;
/// The drive file's name in the card's root.
const DRIVE_FILE: &str = "rp-usbbench.tmp";

/// Deterministic random bytes (xorshift32), so a mismatch can be located.
#[must_use]
pub fn pattern(len: usize, seed: u32) -> Vec<u8> {
    let mut x = seed | 1;
    let mut v = Vec::with_capacity(len);
    while v.len() < len {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        v.extend_from_slice(&x.to_le_bytes());
    }
    v.truncate(len);
    v
}

/// An image frame whose CRC is wrong on purpose.
#[must_use]
pub fn bad_crc_frame(payload: &[u8]) -> Vec<u8> {
    let mut f = rp_loadproto::frame(payload);
    f[8] ^= 0xFF;
    f
}

/// `bytes` in `ms` as KB/s (1000 bytes), integers only.
#[must_use]
pub fn kbps(bytes: usize, ms: u128) -> u128 {
    (bytes as u128) * 1000 / ms.max(1) / 1000
}

fn serial_script(frame: &Path) -> String {
    let path = frame.display().to_string().replace('\'', "''");
    super::usb::open_port_script()
        + &format!(
            r#"$bytes = [System.IO.File]::ReadAllBytes('{path}')
$sp.DtrEnable = $true
$sp.RtsEnable = $true
$sp.ReadTimeout = 5000
$sp.WriteTimeout = 60000
$sp.WriteBufferSize = 1048576
$sp.NewLine = "`r`n"
Open-Port
Start-Sleep -Milliseconds 300
$null = $sp.ReadExisting()
$sw = [System.Diagnostics.Stopwatch]::StartNew()
$sp.Write($bytes, 0, $bytes.Length)
$sent = $sw.ElapsedMilliseconds
"SENT " + $sent
$deadline = (Get-Date).AddSeconds(60)
while ((Get-Date) -lt $deadline) {{
    try {{ $l = $sp.ReadLine() }} catch [System.TimeoutException] {{ continue }} catch {{ break }}
    if ($l -match '^RPL (OK|ERR)') {{ "REPLY " + $sw.ElapsedMilliseconds + " " + $sent + " " + $l; break }}
    "LINE " + $l
}}
try {{ $sp.Close() }} catch {{ }}
"#
        )
}

/// One serial run: (ms to the reply, the reply line).
fn serial_once(frame: &Path) -> Result<(u128, String), String> {
    let out = super::usb::powershell(&serial_script(frame))?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    if text.contains("NOPORT") {
        return Err("no loader port (is the Axim docked and in the loader?)".into());
    }
    let line = text
        .lines()
        .find_map(|l| l.strip_prefix("REPLY "))
        .ok_or_else(|| {
            format!(
                "no reply from the loader:\n{text}{}",
                String::from_utf8_lossy(&out.stderr)
            )
        })?;
    let mut parts = line.splitn(3, ' ');
    let ms: u128 = parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or("bad REPLY line")?;
    let _sent = parts.next();
    let reply = parts.next().unwrap_or("").to_string();
    Ok((ms, reply))
}

/// The SD card drive's root (`X:\`), found by the disk's name. Waits up
/// to 30 s: after any loader frame the card is away from the PC for a few
/// seconds (the medium-change window).
fn drive_root() -> Result<PathBuf, String> {
    let deadline = Instant::now() + std::time::Duration::from_secs(30);
    loop {
        // Through the one locked way to PowerShell (the lock is held already).
        let out = super::usb::powershell(
            "Get-Disk | Where-Object FriendlyName -eq 'RetroPan SD card' | Get-Partition              | Get-Volume | Where-Object DriveLetter | Select-Object -First 1              -ExpandProperty DriveLetter",
        )?;
        let s = String::from_utf8_lossy(&out.stdout);
        let letter = s.trim();
        if letter.len() == 1 {
            return Ok(PathBuf::from(format!("{letter}:\\")));
        }
        if Instant::now() > deadline {
            return Err("the SD card's drive isn't mounted on the PC".into());
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}

/// A buffer whose bytes start on a 4 KiB boundary (unbuffered I/O needs
/// sector-aligned memory).
struct Aligned {
    raw: Vec<u8>,
    off: usize,
    len: usize,
}

impl Aligned {
    fn new(len: usize) -> Self {
        let raw = vec![0u8; len + 4096];
        let off = (4096 - (raw.as_ptr() as usize & 4095)) & 4095;
        Self { raw, off, len }
    }
    fn get(&mut self) -> &mut [u8] {
        &mut self.raw[self.off..self.off + self.len]
    }
}

#[cfg(windows)]
fn drive_once(root: &Path, data: &[u8]) -> Result<(u128, u128), String> {
    use std::io::{Read, Write};
    use std::os::windows::fs::OpenOptionsExt;
    const NO_BUFFERING: u32 = 0x2000_0000;
    const WRITE_THROUGH: u32 = 0x8000_0000;
    const CHUNK: usize = 1024 * 1024;
    let path = root.join(DRIVE_FILE);
    let mut buf = Aligned::new(CHUNK);
    let t = Instant::now();
    {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .custom_flags(NO_BUFFERING | WRITE_THROUGH)
            .open(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        for c in data.chunks(CHUNK) {
            buf.get()[..c.len()].copy_from_slice(c);
            f.write_all(&buf.get()[..c.len()])
                .map_err(|e| format!("write {}: {e}", path.display()))?;
        }
        f.sync_all().map_err(|e| format!("flush: {e}"))?;
    }
    let write_ms = t.elapsed().as_millis();
    let t = Instant::now();
    let mut back = Vec::with_capacity(data.len());
    {
        let mut f = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(NO_BUFFERING)
            .open(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        while back.len() < data.len() {
            let n = f
                .read(buf.get())
                .map_err(|e| format!("read {}: {e}", path.display()))?;
            if n == 0 {
                break;
            }
            back.extend_from_slice(&buf.get()[..n]);
        }
    }
    let read_ms = t.elapsed().as_millis();
    std::fs::remove_file(&path).map_err(|e| format!("remove {}: {e}", path.display()))?;
    if back.len() != data.len() {
        return Err(format!("read back {} bytes of {}", back.len(), data.len()));
    }
    if let Some(i) = back.iter().zip(data).position(|(a, b)| a != b) {
        return Err(format!(
            "read-back differs at byte {i} (sector {})",
            i / 512
        ));
    }
    Ok((write_ms, read_ms))
}

#[cfg(not(windows))]
fn drive_once(_root: &Path, _data: &[u8]) -> Result<(u128, u128), String> {
    Err("the drive bench needs Windows".into())
}

/// `cargo xtask usbbench trial <image> [runs]`: a new loader tried from
/// RAM, the old one still on the card, in one hold of the Axim lock: the
/// card's loader is copied aside, `image` loaded, the latency and both
/// throughput benches run, `udc stats` asked, and the card's loader loaded
/// back from RAM, so no other
/// session's run lands on the build under test (even if a step fails).
pub fn trial(root: &Path, image: &str, runs: usize) -> Result<(), String> {
    crate::devlock::ensure_held()?;
    let card = drive_root()?.join("retropane.bin");
    let saved = root.join("target/load/card-loader.bin");
    std::fs::create_dir_all(root.join("target/load")).map_err(|e| e.to_string())?;
    std::fs::copy(&card, &saved).map_err(|e| format!("{}: {e}", card.display()))?;
    println!("trial: card loader saved ({})", saved.display());
    super::usb::load(root, image, false)?;
    let result = trial_steps(root, runs);
    println!("trial: putting the card's loader back (from RAM)");
    let back = super::usb::load(root, &saved.display().to_string(), false);
    result.and(back)
}

/// The trial's steps on the loader under test (the caller puts the card's
/// loader back whatever they return).
fn trial_steps(root: &Path, runs: usize) -> Result<(), String> {
    if let Err(e) = drivers() {
        println!("trial: {e}");
    }
    if let Err(e) = latency(root) {
        println!("trial: {e}");
    }
    let r = run(root, "all", runs);
    match super::usb::command_reply(root, "udc stats") {
        Ok(p) => println!("trial: {p}"),
        Err(e) => println!("trial: udc stats: {e}"),
    }
    r
}

fn latency_script(frame: &Path, rounds: usize) -> String {
    let path = frame.display().to_string().replace('\'', "''");
    super::usb::open_port_script()
        + &format!(
            r#"$cmd = [System.IO.File]::ReadAllBytes('{path}')
$sp.DtrEnable = $true
$sp.RtsEnable = $true
$sp.ReadTimeout = 3000
$sp.NewLine = "`r`n"
Open-Port
Start-Sleep -Milliseconds 300
$null = $sp.ReadExisting()
for ($i = 0; $i -lt {rounds}; $i++) {{
    Start-Sleep -Milliseconds 300
    $null = $sp.ReadExisting()
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $sp.Write($cmd, 0, $cmd.Length)
    try {{
        while ($true) {{
            $l = $sp.ReadLine()
            if ($l -match '^RPL (OK|ERR)') {{ break }}
        }}
        'LAT ' + [int]($sw.Elapsed.TotalMilliseconds * 1000)
    }} catch {{ 'LOST' }}
}}
try {{ $sp.Close() }} catch {{ }}
"#
        )
}

/// `cargo xtask usbbench latency`: 20 round trips of a one-packet command
/// (one the loader refuses, so any loader answers it at once), each after
/// 300 ms of quiet so the loader is asleep: how fast it wakes for the PC.
/// Prints the minimum, median and maximum in microseconds.
pub fn latency(root: &Path) -> Result<(), String> {
    crate::devlock::ensure_held()?;
    let out = root.join("target/load");
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let frame = out.join("latency.frame");
    std::fs::write(&frame, rp_loadproto::control_frame("ping")).map_err(|e| e.to_string())?;
    let o = super::usb::powershell(&latency_script(&frame, 20))?;
    let text = String::from_utf8_lossy(&o.stdout);
    let mut us: Vec<u32> = text
        .lines()
        .filter_map(|l| l.strip_prefix("LAT ")?.trim().parse().ok())
        .collect();
    let lost = text.lines().filter(|l| *l == "LOST").count();
    if us.is_empty() {
        return Err(format!("no round trips:\n{text}"));
    }
    us.sort_unstable();
    println!(
        "latency: {} round trips, min {} us, median {} us, max {} us, {lost} lost",
        us.len(),
        us[0],
        us[us.len() / 2],
        us[us.len() - 1]
    );
    Ok(())
}

/// Lists every present device under RetroPane's USB IDs (and the disks
/// behind them) with its driver service and INF: `INF name service device`.
const DRIVERS_SCRIPT: &str = r#"$ErrorActionPreference = 'SilentlyContinue'
$ids = Get-PnpDevice -PresentOnly | Where-Object { $_.InstanceId -like 'USB\VID_1209&PID_0001*' -or $_.FriendlyName -like 'RetroPan*' }
foreach ($d in $ids) {
    $inf = (Get-PnpDeviceProperty -InstanceId $d.InstanceId -KeyName DEVPKEY_Device_DriverInfPath).Data
    $svc = (Get-PnpDeviceProperty -InstanceId $d.InstanceId -KeyName DEVPKEY_Device_Service).Data
    "INF $inf $svc $($d.FriendlyName)"
}
"#;

/// Whether every driver is one Windows ships (no `oem*.inf`): the device
/// must stay driverless (ADR-0012). Returns the lines for the report.
pub fn parse_drivers(text: &str) -> Result<Vec<String>, String> {
    let lines: Vec<String> = text
        .lines()
        .filter_map(|l| l.strip_prefix("INF "))
        .map(str::to_string)
        .collect();
    if lines.is_empty() {
        return Err("no RetroPane USB devices found".into());
    }
    if let Some(bad) = lines.iter().find(|l| {
        l.split(' ')
            .next()
            .is_none_or(|inf| inf.is_empty() || inf.starts_with("oem"))
    }) {
        return Err(format!("not an inbox driver: {bad}"));
    }
    Ok(lines)
}

/// `cargo xtask usbbench drivers`: the device's functions on Windows' own
/// drivers.
pub fn drivers() -> Result<(), String> {
    let o = super::usb::powershell(DRIVERS_SCRIPT)?;
    for l in parse_drivers(&String::from_utf8_lossy(&o.stdout))? {
        println!("driver: {l}");
    }
    println!("driverless: every function on Windows' own drivers");
    Ok(())
}

/// `cargo xtask usbbench [serial|drive] [runs]`.
pub fn run(root: &Path, which: &str, runs: usize) -> Result<(), String> {
    crate::devlock::ensure_held()?;
    let runs = runs.max(1);
    if which != "drive" {
        let out = root.join("target/load");
        std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
        let frame = out.join("usbbench.frame");
        // RP_USBBENCH_BYTES: a smaller frame (diagnostics).
        let bytes = std::env::var("RP_USBBENCH_BYTES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(SERIAL_BYTES)
            .min(SERIAL_BYTES);
        let payload = pattern(bytes, 0x5EED);
        std::fs::write(&frame, bad_crc_frame(&payload))
            .map_err(|e| format!("{}: {e}", frame.display()))?;
        for i in 0..runs {
            let (ms, reply) = serial_once(&frame)?;
            if !reply.contains("bad crc") {
                return Err(format!("expected `RPL ERR bad crc`, got `{reply}`"));
            }
            println!(
                "serial {}: {bytes} B in {ms} ms = {} KB/s",
                i + 1,
                kbps(bytes, ms)
            );
        }
    }
    if which != "serial" {
        let root = drive_root()?;
        for i in 0..runs {
            let data = pattern(DRIVE_BYTES, 0xD15C + i as u32);
            let (w, r) = drive_once(&root, &data)?;
            println!(
                "drive {}: {DRIVE_BYTES} B write {w} ms = {} KB/s, read {r} ms = {} KB/s, read-back identical",
                i + 1,
                kbps(DRIVE_BYTES, w),
                kbps(DRIVE_BYTES, r)
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_frame_is_refused_for_its_crc_only() {
        let p = pattern(1000, 7);
        let f = bad_crc_frame(&p);
        let mut rx = rp_loadproto::Receiver::new();
        let mut dest = vec![0u8; 2000];
        let (_, r) = rp_loadproto::feed_slice(&mut rx, &f, &mut dest);
        assert_eq!(r, Some(Err(rp_loadproto::LoadError::BadCrc)));
        assert_eq!(&dest[..1000], &p[..]);
    }

    #[test]
    fn third_party_drivers_are_caught() {
        let ok = "INF usb.inf usbccgp USB Composite Device\nINF usbser.inf usbser USB Serial Device (COM4)\nINF disk.inf disk RetroPan SD card USB Device\n";
        assert_eq!(parse_drivers(ok).unwrap().len(), 3);
        assert!(parse_drivers("INF oem42.inf foo X\n").is_err());
        assert!(parse_drivers("INF  usbser X\n").is_err());
        assert!(parse_drivers("").is_err());
    }

    #[test]
    fn the_pattern_is_repeatable_and_not_flat() {
        assert_eq!(pattern(64, 3), pattern(64, 3));
        assert_ne!(pattern(64, 3), pattern(64, 4));
        assert_eq!(kbps(1_000_000, 1000), 1000);
    }
}
