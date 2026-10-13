//! `cargo xtask trace` (item 54): the last program's trace ring, fetched
//! from the resident loader (`trace`: one `RPD` line of four hex words per
//! record, the header first) and printed as a timeline with the kernel's
//! own decoder (`rp-kernel-trace`), so torn or garbled records are dropped,
//! never misread.

use std::fmt::Write as _;
use std::path::Path;

use rp_kernel_trace::{Event, Kind};

/// OS timer ticks per microsecond, times 100 (3.25 MHz).
const TICKS_PER_US_X100: u64 = 325;

/// The ring's words from the loader's data lines.
pub fn parse(lines: &[String]) -> Result<Vec<u32>, String> {
    let mut words = Vec::with_capacity(lines.len() * 4);
    for l in lines {
        for w in l.split_whitespace() {
            words.push(u32::from_str_radix(w, 16).map_err(|_| format!("not a trace word: {w:?}"))?);
        }
    }
    Ok(words)
}

fn describe(e: &Event) -> String {
    match e.kind {
        Kind::Switch => format!("switch   thread {} -> {}", e.a, e.b),
        Kind::Fault => format!("fault    outcome {} at {:#010x}", e.a, e.b),
        Kind::Call => format!("call     slot {} (depth {})", e.a, e.b),
        Kind::Return => format!("return   to slot {} = {:#x}", e.a, e.b),
        Kind::Restart => format!("restart  slot {}", e.a),
        Kind::Spawn => format!("spawn    slot {} at {:#010x}", e.a, e.b),
        Kind::Exit => format!("exit     slot {} code {:#x}", e.a, e.b),
        Kind::Alarm => format!("alarm    deadline {}", e.b),
        Kind::Mark => format!("mark     {} {:#x}", e.a, e.b),
    }
}

/// The timeline of a ring's words: one line per event, time from the
/// first one kept (microseconds) and since the one before.
pub fn timeline(words: &[u32]) -> Result<String, String> {
    let (total, events) =
        rp_kernel_trace::decode_words(words).ok_or("no trace ring in the loader's reply")?;
    let events: Vec<Event> = events.collect();
    let mut out = String::new();
    let lost = u64::from(total).saturating_sub(events.len() as u64);
    let _ = writeln!(
        out,
        "{total} events recorded, {} kept{}",
        events.len(),
        if lost > 0 {
            format!(" ({lost} older ones overwritten or torn)")
        } else {
            String::new()
        }
    );
    let Some(first) = events.first().map(|e| e.time) else {
        return Ok(out);
    };
    let us = |ticks: u32| u64::from(ticks) * 100 / TICKS_PER_US_X100;
    let mut prev = first;
    let _ = writeln!(out, "      seq   time (us)    +us  event");
    for e in &events {
        let _ = writeln!(
            out,
            "{:>9} {:>11} {:>6}  {}",
            e.seq,
            us(e.time.wrapping_sub(first)),
            us(e.time.wrapping_sub(prev)),
            describe(e)
        );
        prev = e.time;
    }
    Ok(out)
}

/// Fetches the last program's ring from the loader and prints it.
pub fn run(root: &Path) -> Result<(), String> {
    let (lines, reply) = crate::usb::query(root, "trace")?;
    eprintln!("    {reply}");
    print!("{}", timeline(&parse(&lines)?)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rp_kernel_trace::Persistent;

    #[test]
    fn the_loaders_lines_become_a_timeline() {
        let mut area = vec![0u32; 4 + 4 * 4];
        let mut r = Persistent::start(&mut area).unwrap();
        r.record(1000, Kind::Mark, 1, 0);
        r.record(1325, Kind::Switch, 2, 3);
        let lines: Vec<String> = area
            .chunks(4)
            .map(|c| {
                c.iter()
                    .map(|w| format!("{w:08x}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect();
        let words = parse(&lines).unwrap();
        assert_eq!(words, area);
        let t = timeline(&words).unwrap();
        assert!(t.starts_with("2 events recorded, 2 kept\n"), "{t}");
        // 325 ticks = 100 us after the first.
        assert!(
            t.contains("        2         100    100  switch   thread 2 -> 3"),
            "{t}"
        );
    }

    #[test]
    fn bad_replies_are_refused() {
        assert!(parse(&["12 zz".into()]).is_err());
        assert!(timeline(&[1, 2, 3]).is_err());
    }
}
