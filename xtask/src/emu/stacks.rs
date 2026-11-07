//! The stack report every image prints, judged (quality audit C27, C50, C90: the 80% line and the guard words).

/// The stack's headroom, for every image the emulator runs: each reports
/// "stack: deepest N of M bytes" when it returns to the loader, and use above
/// 80% fails. A silent overflow freezes the image (rp-today ran at 96% of its
/// stack and an older build froze the Axim into the watchdog error screen,
/// 2026-10-17). Returns the deepest report seen, as (used, size).
pub(crate) fn check_stack(text: &str) -> Result<Option<(u32, u32)>, String> {
    // The stack's guard words (quality audit C27, C90): written means an
    // overflow, whatever the depth read says.
    if let Some(l) = text.lines().find(|l| l.contains("OVERFLOWED")) {
        return Err(format!("the stack overflowed: {l:?}"));
    }
    let mut deepest: Option<(u32, u32)> = None;
    for rest in text.split("stack: deepest ").skip(1) {
        let mut n = rest.split(" of ").map(|p| {
            p.split(|c: char| !c.is_ascii_digit())
                .next()
                .and_then(|d| d.parse::<u32>().ok())
        });
        let (Some(Some(used)), Some(Some(size))) = (n.next(), n.next()) else {
            return Err(format!(
                "unreadable stack report: {:?}",
                rest.lines().next().unwrap_or("")
            ));
        };
        if u64::from(used) * 100 > u64::from(size) * 80 {
            return Err(format!(
                "the stack went {used} of {size} bytes deep (over 80%: an overflow freezes the image)"
            ));
        }
        if deepest
            .is_none_or(|(u, s)| u64::from(used) * u64::from(s) > u64::from(u) * u64::from(size))
        {
            deepest = Some((used, size));
        }
    }
    Ok(deepest)
}
