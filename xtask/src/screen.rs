//! Emulator screenshots: capture via QEMU's monitor, and check them against
//! the bring-up boot screen (test pattern plus boot log).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::time::{Duration, Instant};

use rp_drv_mbx_display::{console, pattern};

/// A connection to QEMU's human monitor.
pub struct Monitor {
    stream: TcpStream,
}

impl Monitor {
    /// Connects to the monitor on local `port`, retrying while QEMU starts.
    pub fn connect(port: u16) -> Result<Self, String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut stream = loop {
            match TcpStream::connect(("127.0.0.1", port)) {
                Ok(s) => break s,
                Err(e) if Instant::now() > deadline => {
                    return Err(format!("can't reach QEMU monitor on port {port}: {e}"));
                }
                Err(_) => std::thread::sleep(Duration::from_millis(100)),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_millis(300)))
            .map_err(|e| e.to_string())?;
        let mut banner = [0u8; 512];
        let _ = stream.read(&mut banner);
        Ok(Self { stream })
    }

    /// Sends one monitor command.
    pub fn command(&mut self, cmd: &str) -> Result<(), String> {
        writeln!(self.stream, "{cmd}").map_err(|e| e.to_string())?;
        // Drain the echo and prompt so the socket buffer never fills.
        let mut sink = [0u8; 1024];
        let _ = self.stream.read(&mut sink);
        Ok(())
    }

    /// Screenshots the 2700G console to `guest_path` (as QEMU sees it) and
    /// waits for the file to appear completely at `host_path`.
    pub fn screendump(&mut self, guest_path: &str, host_path: &Path) -> Result<(), String> {
        self.command(&format!("screendump {guest_path} mbx2700g"))?;
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Ok(meta) = std::fs::metadata(host_path)
                && meta.len() > 0
            {
                // Give QEMU a moment to finish writing, then check it's complete.
                std::thread::sleep(Duration::from_millis(200));
                if std::fs::metadata(host_path).is_ok_and(|m| m.len() == meta.len()) {
                    return Ok(());
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Err(format!("no screenshot at {}", host_path.display()))
    }
}

/// A connection to QEMU's machine protocol (QMP), for absolute pointer input.
pub struct Qmp {
    stream: TcpStream,
}

impl Qmp {
    /// Connects to QMP on local `port` and leaves capabilities negotiation mode.
    pub fn connect(port: u16) -> Result<Self, String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let stream = loop {
            match TcpStream::connect(("127.0.0.1", port)) {
                Ok(s) => break s,
                Err(e) if Instant::now() > deadline => {
                    return Err(format!("can't reach QMP on port {port}: {e}"));
                }
                Err(_) => std::thread::sleep(Duration::from_millis(100)),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_millis(500)))
            .map_err(|e| e.to_string())?;
        let mut q = Self { stream };
        q.read_reply()?; // greeting
        q.execute(&serde_json::json!({ "execute": "qmp_capabilities" }))?;
        Ok(q)
    }

    fn read_reply(&mut self) -> Result<String, String> {
        let mut buf = [0u8; 4096];
        let n = self
            .stream
            .read(&mut buf)
            .map_err(|e| format!("QMP read: {e}"))?;
        Ok(String::from_utf8_lossy(&buf[..n]).into_owned())
    }

    /// Sends a command and checks QEMU didn't report an error.
    pub fn execute(&mut self, cmd: &serde_json::Value) -> Result<(), String> {
        writeln!(self.stream, "{cmd}").map_err(|e| e.to_string())?;
        let reply = self.read_reply()?;
        if reply.contains("\"error\"") {
            return Err(format!("QMP error: {reply}"));
        }
        Ok(())
    }

    /// Stylus down at absolute pointer position `(x, y)` (0..0x7FFF), or up.
    pub fn touch(&mut self, at: Option<(u32, u32)>) -> Result<(), String> {
        let mut events = Vec::new();
        if let Some((x, y)) = at {
            events.push(serde_json::json!({ "type": "abs", "data": { "axis": "x", "value": x } }));
            events.push(serde_json::json!({ "type": "abs", "data": { "axis": "y", "value": y } }));
        }
        events.push(serde_json::json!({
            "type": "btn",
            "data": { "down": at.is_some(), "button": "left" }
        }));
        self.execute(&serde_json::json!({
            "execute": "input-send-event",
            "arguments": { "events": events }
        }))
    }
}

/// A decoded binary PPM (P6, 8-bit).
#[derive(Debug)]
pub struct Image {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// RGB triples, row-major.
    pub rgb: Vec<u8>,
}

/// Parses a binary PPM.
pub fn parse_ppm(data: &[u8]) -> Result<Image, String> {
    let mut fields = Vec::new();
    let mut pos = 0;
    while fields.len() < 4 {
        while pos < data.len() && data[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if data.get(pos) == Some(&b'#') {
            while pos < data.len() && data[pos] != b'\n' {
                pos += 1;
            }
            continue;
        }
        let start = pos;
        while pos < data.len() && !data[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if start == pos {
            return Err("truncated PPM header".into());
        }
        fields.push(String::from_utf8_lossy(&data[start..pos]).into_owned());
    }
    pos += 1; // single whitespace before pixel data
    if fields[0] != "P6" || fields[3] != "255" {
        return Err(format!("unsupported PPM ({} max {})", fields[0], fields[3]));
    }
    let width: u32 = fields[1].parse().map_err(|_| "bad PPM width")?;
    let height: u32 = fields[2].parse().map_err(|_| "bad PPM height")?;
    let len = (width * height * 3) as usize;
    let rgb = data
        .get(pos..pos + len)
        .ok_or("PPM pixel data truncated")?
        .to_vec();
    Ok(Image { width, height, rgb })
}

/// Checks a bring-up screenshot at 480 × 640: the test pattern
/// ([`pattern::rgb`]) everywhere outside the boot log box
/// ([`console::BOOT_LOG`]), and inside it the log's background with some text
/// drawn on it. Allows for RGB565 quantisation (up to 8 per channel).
pub fn check_boot_screen(ppm: &[u8]) -> Result<(), String> {
    let img = parse_ppm(ppm)?;
    if (img.width, img.height) != (480, 640) {
        return Err(format!(
            "screen is {}x{}, expected 480x640",
            img.width, img.height
        ));
    }
    let (lx, ly, lw, lh) = console::BOOT_LOG;
    let in_log = |x: u32, y: u32| {
        (lx..lx + lw).contains(&(x as usize)) && (ly..ly + lh).contains(&(y as usize))
    };
    let close = |want: (u8, u8, u8), got: &[u8]| {
        want.0.abs_diff(got[0]) <= 8 && want.1.abs_diff(got[1]) <= 8 && want.2.abs_diff(got[2]) <= 8
    };
    let (fg, bg) = console::BOOT_LOG_COLORS;
    let (fg, bg) = (from_rgb565(fg), from_rgb565(bg));
    let mut bad = Vec::new();
    let (mut ink, mut paper) = (0usize, 0usize);
    for y in 0..img.height {
        for x in 0..img.width {
            let i = ((y * img.width + x) * 3) as usize;
            let got = &img.rgb[i..i + 3];
            if in_log(x, y) {
                if close(fg, got) {
                    ink += 1;
                } else if close(bg, got) {
                    paper += 1;
                } else {
                    bad.push((x, y, bg, (got[0], got[1], got[2])));
                }
                continue;
            }
            let want = pattern::rgb(x, y, img.width, img.height);
            if !close(want, got) {
                bad.push((x, y, want, (got[0], got[1], got[2])));
            }
        }
    }
    if let Some(&(x, y, want, got)) = bad.first() {
        return Err(format!(
            "{} pixels are wrong; first at ({x},{y}): expected {want:?}, got {got:?}",
            bad.len()
        ));
    }
    // A few lines of 5x7 text at scale 2 is thousands of ink pixels.
    if ink < 1000 || paper < ink {
        return Err(format!(
            "boot log looks wrong: {ink} text pixels, {paper} background pixels"
        ));
    }
    Ok(())
}

/// Expands RGB565 to 8 bits per channel (low bits zero, as QEMU's
/// screendump produces).
fn from_rgb565(c: u16) -> (u8, u8, u8) {
    (
        ((c >> 11) << 3) as u8,
        (((c >> 5) & 0x3F) << 2) as u8,
        ((c & 0x1F) << 3) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ppm(w: u32, h: u32, f: impl Fn(u32, u32) -> (u8, u8, u8)) -> Vec<u8> {
        let mut out = format!("P6\n# test\n{w} {h}\n255\n").into_bytes();
        for y in 0..h {
            for x in 0..w {
                let (r, g, b) = f(x, y);
                out.extend([r, g, b]);
            }
        }
        out
    }

    /// The screen a bring-up image draws, rendered on the host with the same
    /// drivers, then quantised the way QEMU's screendump reports it.
    fn boot_screen(text: &str) -> Vec<u8> {
        use core::fmt::Write;
        let stride = 480 * 2;
        let mut fb = vec![0u8; stride * 640];
        pattern::fill_rgb565(&mut fb, 480, 640, stride);
        let (x, y, w, h) = console::BOOT_LOG;
        let mut con = console::Console::new(
            &mut fb,
            stride,
            (x, y),
            (w, h),
            console::BOOT_LOG_SCALE,
            console::BOOT_LOG_COLORS,
        );
        con.write_str(text).unwrap();
        ppm(480, 640, |x, y| {
            let i = (y as usize) * stride + (x as usize) * 2;
            from_rgb565(u16::from_le_bytes([fb[i], fb[i + 1]]))
        })
    }

    #[test]
    fn accepts_the_boot_screen() {
        check_boot_screen(&boot_screen(
            "RetroPane 26\nDisplay: 480x640\nSD: SD card\n",
        ))
        .unwrap();
    }

    #[test]
    fn rejects_an_empty_boot_log() {
        let err = check_boot_screen(&boot_screen("")).unwrap_err();
        assert!(err.contains("boot log"), "{err}");
    }

    #[test]
    fn rejects_a_black_screen() {
        let err = check_boot_screen(&ppm(480, 640, |_, _| (0, 0, 0))).unwrap_err();
        assert!(err.contains("wrong"), "{err}");
    }

    #[test]
    fn rejects_wrong_size() {
        assert!(check_boot_screen(&ppm(640, 480, |_, _| (0, 0, 0))).is_err());
    }

    #[test]
    fn rejects_truncated_ppm() {
        assert!(parse_ppm(b"P6\n4 4\n255\n\x00\x00").is_err());
        assert!(parse_ppm(b"P5\n1 1\n255\n\x00").is_err());
    }
}
