//! `cargo xtask usbbench mtp`: the SD card over MTP (ADR-0037 rule 13's
//! open point: MTP's speed against the drive's), with the loader in its
//! MTP layout (`cargo xtask drive mtp` first). A [`DRIVE_BYTES`] file of
//! random bytes goes to the card through Windows' own portable-device path
//! (the Shell, as Explorer copies), comes back to the PC, is compared byte
//! for byte, and is then removed with the loader's `rm` (Windows' Shell
//! asks on screen before deleting anything on a portable device; the
//! loader's `rm` also raises MTP's ObjectRemoved, rule 10).
//!
//! Timing: a copy to the device ends when the file appears (the device
//! names a file only once every byte is on the card, rule 6), a copy back
//! when the PC's file is whole; polled every 20 ms, so a 16 MiB copy is
//! timed to well under 1%.
//!
//! **The card holds other sessions' files:** only [`MTP_DIR`] is written,
//! a folder of that name holding anything else is refused, and only what
//! this run made is removed.

use std::path::Path;

use super::{DRIVE_BYTES, kbps, pattern};

/// The bench's folder on the SD card.
pub const MTP_DIR: &str = "rp-mtpbench";
/// Its file.
const MTP_FILE: &str = "bench.bin";
/// The device and store as Windows names them (ADR-0037 Q4, rule 12).
const DEVICE: &str = "Axim X51v";
const STORE: &str = "SD Card";

/// The PowerShell side: copy `src` into the store's bench folder and back
/// into `back`; prints `WRITE <ms>` and `READ <ms>`, or `ERROR <why>`.
fn script(src: &Path, back: &Path, len: usize) -> String {
    // Windows' Shell copies nothing, silently, from a path mixing `/` and
    // `\` (found on the X51v run, 2026-10-26: the bench's first copy never
    // reached the device): every separator as Windows writes it.
    let q = |p: &Path| {
        p.display()
            .to_string()
            .replace('/', "\\")
            .replace('\'', "''")
    };
    r#"
$ErrorActionPreference = 'Stop'
function Wait-Until([scriptblock]$done, [int]$secs, [string]$what) {
    $t = [Diagnostics.Stopwatch]::StartNew()
    while (-not (& $done)) {
        if ($t.Elapsed.TotalSeconds -gt $secs) { "ERROR timed out: $what"; exit }
        Start-Sleep -Milliseconds 20
    }
}
$sh = New-Object -ComObject Shell.Application
$dev = $sh.Namespace(17).Items() | Where-Object { $_.Name -eq '@DEVICE@' } | Select-Object -First 1
if (-not $dev) { 'ERROR no portable device named @DEVICE@ (the loader in its MTP layout? cargo xtask drive mtp)'; exit }
$store = $dev.GetFolder.Items() | Where-Object { $_.Name -eq '@STORE@' } | Select-Object -First 1
if (-not $store) { 'ERROR no store named @STORE@ on @DEVICE@'; exit }
$root = $store.GetFolder
$dir = $root.ParseName('@DIR@')
if ($dir) {
    $others = @($dir.GetFolder.Items() | Where-Object { $_.Name -ne '@FILE@' })
    if ($others.Count -gt 0) { 'ERROR @DIR@ holds files that are not the bench''s: not ours'; exit }
    if ($dir.GetFolder.ParseName('@FILE@')) { 'ERROR @DIR@/@FILE@ is still there from an earlier run: cargo xtask rm @DIR@/@FILE@'; exit }
} else {
    $root.NewFolder('@DIR@')
    Wait-Until { $root.ParseName('@DIR@') } 30 'the bench folder'
    $dir = $root.ParseName('@DIR@')
}
$folder = $dir.GetFolder
$t = [Diagnostics.Stopwatch]::StartNew()
$folder.CopyHere('@SRC@', 4 + 16 + 1024)
Wait-Until { $folder.ParseName('@FILE@') } 300 'the copy to the device'
"WRITE $($t.ElapsedMilliseconds)"
$item = $folder.ParseName('@FILE@')
$size = $item.ExtendedProperty('System.Size')
if ($size -ne @LEN@) { "ERROR the device holds $size bytes, not @LEN@"; exit }
$out = Join-Path '@BACK@' '@FILE@'
$t.Restart()
$sh.Namespace('@BACK@').CopyHere($item, 4 + 16 + 1024)
Wait-Until {
    if (-not (Test-Path $out)) { return $false }
    if ((Get-Item $out).Length -ne @LEN@) { return $false }
    try { [IO.File]::Open($out, 'Open', 'Read', 'None').Close(); $true } catch { $false }
} 300 'the copy back'
"READ $($t.ElapsedMilliseconds)"
"#
    .replace("@DEVICE@", DEVICE)
    .replace("@STORE@", STORE)
    .replace("@DIR@", MTP_DIR)
    .replace("@FILE@", MTP_FILE)
    .replace("@SRC@", &q(src))
    .replace("@BACK@", &q(back))
    .replace("@LEN@", &len.to_string())
}

/// `WRITE` and `READ` milliseconds from the script's output.
fn parse(text: &str) -> Result<(u128, u128), String> {
    if let Some(e) = text.lines().find_map(|l| l.trim().strip_prefix("ERROR ")) {
        return Err(format!("MTP bench: {e}"));
    }
    let ms = |key: &str| {
        text.lines()
            .find_map(|l| l.trim().strip_prefix(key)?.trim().parse::<u128>().ok())
            .ok_or_else(|| format!("MTP bench: no {key}line in {text:?}"))
    };
    Ok((ms("WRITE ")?, ms("READ ")?))
}

/// Runs the bench `runs` times; the loader removes the file after each.
pub fn run(root: &Path, runs: usize) -> Result<(), String> {
    crate::devlock::ensure_held()?;
    let status = crate::usb::command_reply(root, "drive mtp status")?;
    if !status.contains("layout mtp") || status.contains("session 0 ") {
        return Err(format!(
            "the loader isn't serving MTP to Windows yet ({status}): cargo xtask drive mtp, then wait for Explorer to show {DEVICE}"
        ));
    }
    println!("before: {status}");
    let work = root.join("target/load/mtpbench");
    let back = work.join("back");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&back).map_err(|e| format!("{}: {e}", back.display()))?;
    let src = work.join(MTP_FILE);
    let mut result = Ok(());
    for i in 0..runs.max(1) {
        let data = pattern(DRIVE_BYTES, 0x3770 + i as u32);
        std::fs::write(&src, &data).map_err(|e| format!("{}: {e}", src.display()))?;
        let _ = std::fs::remove_file(back.join(MTP_FILE));
        let out = crate::usb::powershell(&script(&src, &back, data.len()))?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned()
            + &String::from_utf8_lossy(&out.stderr);
        let timed = parse(&text);
        // Whatever happened, the device's copy goes (only ours).
        let removed = crate::usb::command_reply(root, &format!("rm {MTP_DIR}/{MTP_FILE}"));
        match timed {
            Ok((w, r)) => {
                let got = std::fs::read(back.join(MTP_FILE)).map_err(|e| e.to_string())?;
                if got != data {
                    result = Err(format!(
                        "run {}: the file read back over MTP differs",
                        i + 1
                    ));
                    break;
                }
                println!(
                    "mtp {}: {DRIVE_BYTES} B write {w} ms = {} KB/s, read {r} ms = {} KB/s, read-back identical",
                    i + 1,
                    kbps(DRIVE_BYTES, w),
                    kbps(DRIVE_BYTES, r)
                );
            }
            Err(e) => {
                result = Err(e);
                break;
            }
        }
        removed?;
    }
    let folder = crate::usb::command_reply(root, &format!("rm {MTP_DIR}"));
    let _ = std::fs::remove_dir_all(&work);
    println!(
        "after: {}",
        crate::usb::command_reply(root, "drive mtp status")?
    );
    result?;
    folder.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_script_names_only_the_bench_folder_and_quotes_paths() {
        let s = script(Path::new("C:/a b/it's/bench.bin"), Path::new("C:/back"), 10);
        assert!(s.contains("'C:\\a b\\it''s\\bench.bin'"));
        assert!(!s.contains("C:/"), "no forward slashes in a Shell path");
        assert!(s.contains("ParseName('rp-mtpbench')"));
        for p in ["@DEVICE@", "@STORE@", "@DIR@", "@FILE@", "@SRC@", "@BACK@", "@LEN@"] {
            assert!(!s.contains(p), "{p} left unfilled");
        }
        // The only thing it creates on the device is the bench folder.
        assert_eq!(s.matches("NewFolder(").count(), 1);
        assert!(
            !s.contains("InvokeVerb"),
            "no Shell deletes (they ask on screen)"
        );
    }

    #[test]
    fn results_and_failures_are_read() {
        assert_eq!(parse("WRITE 1500\r\nREAD 900\r\n"), Ok((1500, 900)));
        assert!(parse("ERROR not ours\n").unwrap_err().contains("not ours"));
        assert!(parse("WRITE 10\n").is_err(), "a missing READ is a failure");
    }
}
