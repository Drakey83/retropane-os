//! One long-running ArgyllCMS `spotread`, a reading per key press
//! (docs/design/display-calibration.md, C6 step 2).
//!
//! `spotread` reads its keys from the console itself, so piped keys never
//! reach it on Windows; each one-shot run (`spotread -O`) costs the
//! instrument's start-up again, about 10 s. Here it runs inside a Windows
//! pseudo-console hosted by `conpty.ps1`, which xtask drives line by line:
//! the instrument starts once and each reading costs only the reading.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use crate::measure::{Xyz, parse_result};

const HOST: &str = include_str!("conpty.ps1");
/// What spotread prints when it waits for the next key.
const PROMPT: &str = "to take a reading";

/// A running `spotread`.
pub struct Spot {
    child: Child,
    keys: ChildStdin,
    out: BufReader<ChildStdout>,
}

impl Spot {
    /// Starts `spotread` on the USB instrument at Argyll port `port` (with
    /// the spectral correction `ccss` if it exists) and waits for its
    /// first prompt.
    ///
    /// # Errors
    /// The host or the instrument didn't start.
    pub fn open(spotread: &Path, port: u32, ccss: &Path) -> Result<Self, String> {
        // The colorimeter, not the Axim, but it runs only inside a device
        // measurement: under the same lock (devlock's class guard).
        crate::devlock::ensure_held()?;
        let dir = std::env::temp_dir().join("retropane-spot");
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let host = dir.join("conpty.ps1");
        std::fs::write(&host, HOST).map_err(|e| format!("{}: {e}", host.display()))?;
        let mut cmd = format!("\"{}\" -e -c {port}", spotread.display());
        if ccss.exists() {
            cmd += &format!(" -X \"{}\"", ccss.display());
        }
        let mut child = Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(&host)
            .env("RP_PTY_CMD", &cmd)
            .env("RP_PTY_CWD", &dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("powershell: {e}"))?;
        let keys = child.stdin.take().ok_or("no stdin")?;
        let out = BufReader::new(child.stdout.take().ok_or("no stdout")?);
        let mut s = Self { child, keys, out };
        let first = s.line()?;
        if first != "STARTED" {
            return Err(format!("pseudo-console host: {first}"));
        }
        let got = s.wait(30_000, PROMPT)?;
        if !got.0 {
            return Err(format!("spotread didn't get ready: {}", got.1));
        }
        Ok(s)
    }

    fn line(&mut self) -> Result<String, String> {
        let mut l = String::new();
        let n = self
            .out
            .read_line(&mut l)
            .map_err(|e| format!("pseudo-console host: {e}"))?;
        if n == 0 {
            return Err("pseudo-console host ended".into());
        }
        Ok(l.trim_end().to_string())
    }

    fn send(&mut self, line: &str) -> Result<(), String> {
        writeln!(self.keys, "{line}")
            .and_then(|()| self.keys.flush())
            .map_err(|e| format!("pseudo-console host: {e}"))
    }

    /// Waits up to `ms` for `text`; (found, the output seen).
    fn wait(&mut self, ms: u32, text: &str) -> Result<(bool, String), String> {
        self.send(&format!("WAIT {ms} {text}"))?;
        let l = self.line()?;
        if let Some(rest) = l.strip_prefix("OUT ") {
            Ok((true, rest.to_string()))
        } else if let Some(rest) = l.strip_prefix("TIMEOUT") {
            Ok((false, rest.trim().to_string()))
        } else {
            Err(format!("pseudo-console host: {l}"))
        }
    }

    /// One reading, taking at most `ms`.
    ///
    /// # Errors
    /// No result in time, or spotread reported a failure.
    pub fn read(&mut self, ms: u32) -> Result<Xyz, String> {
        self.send("KEY  ")?;
        let (done, text) = self.wait(ms, PROMPT)?;
        match parse_result(&text) {
            Some(v) => Ok(v),
            None if !done => Err("no reading in time".into()),
            None => Err(format!("spotread said: {}", tail(&text))),
        }
    }
}

fn tail(s: &str) -> &str {
    let n = s.len();
    let mut at = n.saturating_sub(120);
    while !s.is_char_boundary(at) {
        at += 1;
    }
    &s[at..]
}

impl Drop for Spot {
    fn drop(&mut self) {
        let _ = self.send("QUIT");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let _ = self.child.kill();
    }
}
