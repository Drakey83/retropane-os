//! `cargo xtask sdkit --serve`: hands the SD kit to the device over Wi-Fi.
//!
//! Stock Windows Mobile's cradle sync needs Windows Mobile Device Center,
//! which modern Windows no longer supports well. A web page works with any PC
//! and any browser: this serves the kit's files (and nothing else) on the
//! local network, and Pocket Internet Explorer on the device downloads them to
//! the storage card. Plain HTML, no scripts (Pocket IE). Stops on Ctrl+C.
//!
//! RetroPane's own cradle link replaces this: standard USB classes (MTP for
//! files, CDC-ACM console) that Windows supports with no extra software
//! (DESIGN §10b).

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::path::Path;

/// The port (26, for RetroPane 26).
pub const PORT: u16 = 8026;

/// The files offered, in the order to fetch them.
pub const FILES: [&str; 4] = ["haret.exe", "default.txt", "retropane.bin", "README.txt"];

/// Serves `kit` until the process is stopped.
pub fn serve(kit: &Path) -> Result<(), String> {
    for f in FILES {
        if !kit.join(f).exists() {
            return Err(format!("{} is missing; run `cargo xtask sdkit` first", f));
        }
    }
    let listener = TcpListener::bind(("0.0.0.0", PORT)).map_err(|e| format!("port {PORT}: {e}"))?;
    let ip = local_ip().unwrap_or_else(|| "<this PC's IP>".into());
    eprintln!("    serving the SD kit at  http://{ip}:{PORT}/");
    eprintln!(
        "    on the Axim: Internet Explorer -> that address -> tap each file -> save to Storage Card"
    );
    eprintln!("    (allow access if Windows Firewall asks; Ctrl+C to stop)");
    for stream in listener.incoming().flatten() {
        if let Err(e) = handle(stream, kit) {
            eprintln!("    request failed: {e}");
        }
    }
    Ok(())
}

/// This PC's address on the local network: the source address the OS would
/// use for an outside destination (no packet is sent).
fn local_ip() -> Option<String> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:80").ok()?; // TEST-NET-1: never routed anywhere
    Some(s.local_addr().ok()?.ip().to_string())
}

fn handle(mut stream: TcpStream, kit: &Path) -> Result<(), String> {
    let peer = stream
        .peer_addr()
        .map(|a| a.to_string())
        .unwrap_or_default();
    let mut line = String::new();
    BufReader::new(&stream)
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    let path = request_path(&line).unwrap_or("/");
    let (status, ctype, extra, body) = match route(path) {
        Route::Index => (
            "200 OK",
            "text/html",
            String::new(),
            index_page().into_bytes(),
        ),
        Route::File(name) => match std::fs::read(kit.join(name)) {
            Ok(b) => {
                eprintln!("    {peer} <- {name} ({} bytes)", b.len());
                (
                    "200 OK",
                    "application/octet-stream",
                    format!("Content-Disposition: attachment; filename=\"{name}\"\r\n"),
                    b,
                )
            }
            Err(_) => (
                "404 Not Found",
                "text/plain",
                String::new(),
                b"not found".to_vec(),
            ),
        },
        Route::NotFound => (
            "404 Not Found",
            "text/plain",
            String::new(),
            b"not found".to_vec(),
        ),
    };
    let head = format!(
        "HTTP/1.0 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(head.as_bytes())
        .map_err(|e| e.to_string())?;
    stream.write_all(&body).map_err(|e| e.to_string())
}

/// The path of an HTTP request line (`GET /x HTTP/1.1`).
fn request_path(line: &str) -> Option<&str> {
    let mut parts = line.split_whitespace();
    (parts.next()? == "GET").then_some(())?;
    parts.next()
}

#[derive(Debug, PartialEq)]
enum Route {
    Index,
    File(&'static str),
    NotFound,
}

/// Only the index and the four kit files exist: no paths, no traversal.
fn route(path: &str) -> Route {
    if path == "/" || path == "/index.html" {
        return Route::Index;
    }
    let name = path.trim_start_matches('/');
    FILES
        .iter()
        .find(|f| f.eq_ignore_ascii_case(name))
        .map_or(Route::NotFound, |f| Route::File(f))
}

fn index_page() -> String {
    let mut links = String::new();
    for (i, f) in FILES.iter().enumerate() {
        links += &format!("<p>{}. <a href=\"/{f}\"><b>{f}</b></a></p>\n", i + 1);
    }
    format!(
        "<html><head><title>RetroPane 26 SD kit</title></head><body>\n\
         <h3>RetroPane 26 SD kit</h3>\n\
         <p>Tap each file and save it to the <b>Storage Card</b>, all in the same folder.</p>\n\
         {links}\
         <p>Then open File Explorer, Storage Card, and tap <b>haret.exe</b>.</p>\n\
         </body></html>\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_kit_files_are_served() {
        assert_eq!(route("/"), Route::Index);
        assert_eq!(route("/haret.exe"), Route::File("haret.exe"));
        assert_eq!(route("/HARET.EXE"), Route::File("haret.exe"));
        assert_eq!(route("/../Cargo.toml"), Route::NotFound);
        assert_eq!(route("/..%2fCargo.toml"), Route::NotFound);
        assert_eq!(route("/C:/Windows/win.ini"), Route::NotFound);
        assert_eq!(route("/sub/haret.exe"), Route::NotFound);
    }

    #[test]
    fn request_lines() {
        assert_eq!(
            request_path("GET /haret.exe HTTP/1.1\r\n"),
            Some("/haret.exe")
        );
        assert_eq!(request_path("POST / HTTP/1.1"), None);
        assert_eq!(request_path(""), None);
    }

    #[test]
    fn the_page_links_every_file() {
        let page = index_page();
        for f in FILES {
            assert!(page.contains(&format!("href=\"/{f}\"")), "{f}");
        }
    }
}
