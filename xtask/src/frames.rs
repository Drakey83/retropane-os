//! Stack frames of 4 KiB or more (ADR-0001 "Why, and what was fixed"):
//! an ARM load reaches 4,095 bytes from its base, so a function whose frame
//! is that big reaches its locals beyond it from `sp` with extra adds and
//! a stall each time. With frame pointers off, `r11` no longer covers the
//! far side: LZ4's text compress lost 6.2% this way (its 8 KiB hash table
//! in the loop's frame). `cargo xtask cross` lists every such frame in every
//! image and fails on one not in [`REVIEWED`], so a new one is looked at.

use std::path::Path;
use std::process::Command;

/// Functions allowed a frame of 4 KiB or more, each reviewed: run once or
/// for setup, not a hot loop (2026-10-22). `<image>: <function>`.
pub const REVIEWED: &[&str] = &[
    // The benchmark that calls `rp_lz4::compress`: the 8 KiB table lives
    // here on purpose, out of the loop's frame.
    "rp-probe: rp_bench_suite::target::lz4::{closure#0}",
    "rp-bench: rp_bench_suite::target::lz4::{closure#0}",
    "rp-probe: rp_bench_suite::target::frames",
    "rp-bench: rp_bench_suite::target::frames",
    // The 3D renderer's set-up benchmark (R88): renderers (~5 KiB each)
    // made and held here; the timed frames run in the renderer's own
    // functions through `&mut self`, in their own small frames.
    "rp-bench: rp_bench_suite::render3d::one",
    "rp-bench: rp_bench_suite::render3d::button",
    "rp-probe: rp_probe::image::main",
    "rp-bench: rp_bench::image::main",
    "rp-mbx3d-api: rp_mbx3d_api::image::s1",
    "rp-mbx3d-api: rp_mbx3d_api::image::s2",
    "rp-mbx3d-api: rp_mbx3d_api::image::s3",
    "rp-mbx3d-api: rp_mbx3d_api::image::s4",
    "rp-mbx3d-api: rp_mbx3d_api::image::s5",
    "rp-mbx3d-api: rp_mbx3d_api::image::s6_s7",
    "rp-mbx3d-api: rp_mbx3d_api::image::s9",
    "rp-gpubench: rp_gpubench::image::g1",
    "rp-gpubench: rp_gpubench::image::g2",
    "rp-gpubench: rp_gpubench::image::g3",
    "rp-gpubench: rp_gpubench::image::g4a",
    "rp-gpubench: rp_gpubench::image::w2",
    "rp-gpubench: rp_gpubench::image::w3",
    "rp-cftime: rp_cftime::image::run",
    "rp-storfast: rp_storfast::image::run",
    "rp-noise: rp_noise::image::main",
    "rp-looks: rp_looks::image::read_photo",
    "rp-looks: rp_looks::image::main",
];

/// Hot loops kept out of their callers' big frames by `#[inline(never)]`:
/// each must stay a function of its own in the image named, or the loop is
/// back in the caller's frame (reviewed above for its setup, not the loop).
pub const APART: &[(&str, &str)] = &[("rp-probe", "rp_lz4::compress_k")];

/// The functions in `disassembly` (`llvm-objdump -d -C`) that make a frame
/// of 4 KiB or more (`sub sp, sp, #N`, N >= 4096): (function, bytes).
pub fn big_frames(disassembly: &str) -> Vec<(String, u32)> {
    let mut out: Vec<(String, u32)> = Vec::new();
    let mut function = String::new();
    for line in disassembly.lines() {
        let t = line.trim();
        if let Some(name) = t.strip_suffix(">:").and_then(|l| l.split_once(" <")) {
            function = name.1.to_string();
            continue;
        }
        let Some(rest) = t.split_once("sub\tsp, sp, #").map(|(_, r)| r) else {
            continue;
        };
        let n: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if let Ok(n) = n.parse::<u32>()
            && n >= 4096
            && !out.iter().any(|(f, _)| *f == function)
        {
            out.push((function.clone(), n));
        }
    }
    out
}

/// Checks every image in `release`: a frame of 4 KiB or more outside
/// [`REVIEWED`] fails.
pub fn check(release: &Path, images: &[&str]) -> Result<(), String> {
    let mut objdump = crate::sdkit::objcopy();
    objdump.set_file_name(if cfg!(windows) {
        "llvm-objdump.exe"
    } else {
        "llvm-objdump"
    });
    let mut new = Vec::new();
    for image in images {
        let elf = release.join(image);
        if !elf.is_file() {
            continue;
        }
        let out = Command::new(&objdump)
            .args(["-d", "-C", "--no-show-raw-insn"])
            .arg(&elf)
            .output()
            .map_err(|e| format!("llvm-objdump: {e}"))?;
        let text = String::from_utf8_lossy(&out.stdout);
        for (_, function) in APART.iter().filter(|(i, _)| i == image) {
            if !text.contains(&format!(" <{function}")) {
                new.push(format!(
                    "{image}: {function} is no longer a function of its own (inlined into a big frame)"
                ));
            }
        }
        for (function, bytes) in big_frames(&text) {
            let entry = format!("{image}: {function}");
            if !REVIEWED.contains(&entry.as_str()) {
                new.push(format!("{entry} ({bytes} B)"));
            }
        }
    }
    if new.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "stack frames of 4 KiB or more, not reviewed (xtask/src/frames.rs: a hot loop in such a \
             frame pays two adds and a stall a local; keep big arrays out of the loop's frame, or \
             review and list it):\n  {}",
            new.join("\n  ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_each_big_frame_once_by_function() {
        let dis = "\
a801ce94 <rp_lz4::compress_k::<82951117>>:
a801ce94:      \tpush\t{r4, r5, lr}
a801ce98:      \tsub\tsp, sp, #44
a800d000 <rp_bench_suite::target::lz4::{closure#0}>:
a800d004:      \tsub\tsp, sp, #4096
a800d008:      \tsub\tsp, sp, #8192
a800e000 <small>:
a800e004:      \tsub\tsp, sp, #4095
";
        assert_eq!(
            big_frames(dis),
            [("rp_bench_suite::target::lz4::{closure#0}".to_string(), 4096)]
        );
    }

    #[test]
    fn a_frame_of_exactly_4_kib_counts() {
        let dis = "a <f>:\n  b:\tsub\tsp, sp, #4096\n";
        assert_eq!(big_frames(dis), [("f".to_string(), 4096)]);
        assert!(big_frames("a <f>:\n  b:\tsub\tsp, sp, #4095\n").is_empty());
    }
}
