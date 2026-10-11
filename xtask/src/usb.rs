//! `cargo xtask usb`: talks to an X51v running the USB link test (`rp-usb`,
//! ADR-0012) over its sync cable, through Windows' built-in serial driver.
//!
//! Finds the COM port by RetroPane's USB IDs, opens it with DTR set (which
//! makes the device send its banner), sends a line and checks it comes back.
//! PowerShell's `System.IO.Ports.SerialPort` does the port handling, so no
//! extra crates are needed; the script is passed base64-encoded (UTF-16LE,
//! as `-EncodedCommand` expects) so no quoting can break it.

use std::process::Command;

/// Must match `rp_drv_pxa_udc::usb::{VENDOR_ID, PRODUCT_ID}`.
const VID_PID: &str = "VID_1209&PID_0001";
/// Must match `rp_usb::BANNER` (without the line ending).
const BANNER: &str = "RetroPane 26 USB console OK";
/// The line sent for the echo check.
const PROBE: &str = "RetroPane echo test 26";

fn script() -> String {
    format!(
        r#"$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$dev = Get-CimInstance Win32_PnPEntity | Where-Object {{ $_.PNPDeviceID -like 'USB\{VID_PID}*' -and $_.Name -match '\(COM\d+\)' }} | Select-Object -First 1
if (-not $dev) {{ 'NOPORT'; exit 0 }}
$port = [regex]::Match($dev.Name, 'COM\d+').Value
"PORT $port"
$sp = New-Object System.IO.Ports.SerialPort $port, 115200
$sp.DtrEnable = $true
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

/// Runs the check.
pub fn run() -> Result<(), String> {
    let out = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-EncodedCommand"])
        .arg(encode_command(&script()))
        .output()
        .map_err(|e| format!("powershell: {e}"))?;
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
    }
}
