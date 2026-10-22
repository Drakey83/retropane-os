//! `cargo xtask symbols <image> <addr...>`: a crash report's addresses (its
//! `CHAIN` line, the fault's address) as functions and source lines.
//!
//! The toolchain's `llvm-tools` component has no `llvm-symbolizer`, so this
//! uses the two it has: `llvm-nm` for the function each address is in (its
//! symbol table, sorted), `llvm-objdump --line-numbers` for the source line
//! (the release profile keeps debug information, `debug = true`).

use std::path::{Path, PathBuf};
use std::process::Command;

/// A function symbol: start, size, demangled name.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Symbol {
    start: u32,
    size: u32,
    name: String,
}

/// The code symbols in `llvm-nm -n -S -C --defined-only` output (lines
/// `<addr> <size> <type> <name>`, or `<addr> <type> <name>` with no size):
/// text symbols (`t`/`T`) only, by address.
fn code_symbols(nm: &str) -> Vec<Symbol> {
    let mut out: Vec<Symbol> = nm
        .lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let start = u32::from_str_radix(f.next()?, 16).ok()?;
            let second = f.next()?;
            let (size, kind) = if second.len() == 1 {
                (0, second)
            } else {
                (u32::from_str_radix(second, 16).ok()?, f.next()?)
            };
            if !kind.eq_ignore_ascii_case("t") {
                return None;
            }
            let name = f.collect::<Vec<_>>().join(" ");
            (!name.is_empty() && !name.starts_with("$")).then_some(Symbol { start, size, name })
        })
        .collect();
    out.sort_by_key(|s| s.start);
    out
}

/// The function `addr` is in, and how far in: the last symbol starting at or
/// below it, if `addr` is inside its size (a symbol with no size, as the
/// assembly entries have, owns everything up to the next one).
fn function_at(symbols: &[Symbol], addr: u32) -> Option<(&str, u32)> {
    let i = symbols
        .partition_point(|s| s.start <= addr)
        .checked_sub(1)?;
    let s = &symbols[i];
    let end = if s.size != 0 {
        s.start.checked_add(s.size)?
    } else {
        symbols.get(i + 1).map_or(u32::MAX, |n| n.start)
    };
    (addr < end).then(|| (s.name.as_str(), addr - s.start))
}

/// The source line in `llvm-objdump -d -l` output for one instruction: the
/// last `; <file>:<line>` before it.
fn source_line(objdump: &str) -> Option<&str> {
    objdump
        .lines()
        .filter_map(|l| l.strip_prefix("; "))
        .rfind(|l| {
            l.rsplit_once(':')
                .is_some_and(|(_, n)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        })
}

/// A number as a crash report writes it (`0x` and hex) or plain hex.
fn parse_addr(s: &str) -> Option<u32> {
    let h = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    u32::from_str_radix(h, 16).ok()
}

/// One of the toolchain's `llvm-tools`.
fn llvm_tool(name: &str) -> PathBuf {
    let mut p = crate::sdkit::objcopy();
    p.set_file_name(if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    });
    p
}

/// The image's ELF: a path, or a name in the release folder.
fn elf_for(root: &Path, image: &str) -> PathBuf {
    let p = Path::new(image);
    if p.is_file() {
        p.to_path_buf()
    } else {
        root.join("target/armv5te-x51v-none-eabi/release")
            .join(image)
    }
}

/// Each address's function (and offset) and source line, one per line.
/// The image is looked up as built: rebuild (`cargo xtask cross`) the same
/// source the report came from first.
pub fn symbols(root: &Path, image: &str, addrs: &[String]) -> Result<Vec<String>, String> {
    if image.is_empty() || addrs.is_empty() {
        return Err("usage: cargo xtask symbols <image name | elf> <addr...>".into());
    }
    let elf = elf_for(root, image);
    if !elf.is_file() {
        return Err(format!(
            "{}: no such image (build it with `cargo xtask cross` first)",
            elf.display()
        ));
    }
    let nm = Command::new(llvm_tool("llvm-nm"))
        .args(["-n", "-S", "-C", "--defined-only"])
        .arg(&elf)
        .output()
        .map_err(|e| format!("llvm-nm: {e}"))?;
    if !nm.status.success() {
        return Err(format!("llvm-nm: {}", String::from_utf8_lossy(&nm.stderr)));
    }
    let symbols = code_symbols(&String::from_utf8_lossy(&nm.stdout));
    let mut out = Vec::new();
    for a in addrs {
        let addr = parse_addr(a).ok_or_else(|| format!("`{a}`: not an address"))?;
        let function = match function_at(&symbols, addr) {
            Some((name, 0)) => name.to_string(),
            Some((name, off)) => format!("{name}+{off:#x}"),
            None => "(no function: not in the image's code)".into(),
        };
        let dump = Command::new(llvm_tool("llvm-objdump"))
            .args(["-d", "-l", "--no-show-raw-insn"])
            .arg(format!("--start-address={addr:#x}"))
            .arg(format!("--stop-address={:#x}", addr.saturating_add(4)))
            .arg(&elf)
            .output()
            .map_err(|e| format!("llvm-objdump: {e}"))?;
        let dump = String::from_utf8_lossy(&dump.stdout);
        let line = source_line(&dump).unwrap_or("(no line)");
        out.push(format!("{addr:#010x}  {function}  {line}"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NM: &str = "\
a8008000 00000000 T _start
a80080b0 T rp_vectors
a800816c 00000020 T rp_undef_entry
a8008a88 00000010 T rp_main
a8008aa0 00000040 t core::panicking::panic_fmt
a8008ae0 00000008 t $a.1
a8009000 00000100 D some_data
a8009100 00000010 t rp_crash::image::main
";

    #[test]
    fn code_symbols_keep_text_only_by_address() {
        let s = code_symbols(NM);
        let names: Vec<&str> = s.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "_start",
                "rp_vectors",
                "rp_undef_entry",
                "rp_main",
                "core::panicking::panic_fmt",
                "rp_crash::image::main"
            ]
        );
        assert_eq!(s[1].size, 0, "a symbol with no size column");
    }

    #[test]
    fn an_address_finds_its_function_and_offset() {
        let s = code_symbols(NM);
        assert_eq!(
            function_at(&s, 0xa800_8aa8),
            Some(("core::panicking::panic_fmt", 8))
        );
        assert_eq!(
            function_at(&s, 0xa800_8aa0),
            Some(("core::panicking::panic_fmt", 0))
        );
        // One past a sized function's end: none (data or padding follows).
        assert_eq!(function_at(&s, 0xa800_8ae0), None);
        // A sizeless symbol owns everything up to the next.
        assert_eq!(function_at(&s, 0xa800_8100), Some(("rp_vectors", 0x50)));
        assert_eq!(function_at(&s, 0xa800_7ffc), None, "below every symbol");
        assert_eq!(
            function_at(&s, 0xa800_910f),
            Some(("rp_crash::image::main", 0xf))
        );
        assert_eq!(function_at(&s, 0xa800_9110), None);
    }

    #[test]
    fn the_source_line_is_the_last_before_the_instruction() {
        let dump = "\
Disassembly of section .text:

a8008aa0 <core::panicking::panic_fmt>:
; core::panicking::panic_fmt():
; C:\\rust\\library\\core\\src\\panicking.rs:55
; C:\\rust\\library\\core\\src\\panicking.rs:60
a8008aa8:      \tsub\tsp, sp, #24
";
        assert_eq!(
            source_line(dump),
            Some("C:\\rust\\library\\core\\src\\panicking.rs:60")
        );
        assert_eq!(source_line("; fn():\na8008aa8: nop\n"), None);
    }

    #[test]
    fn addresses_parse_as_a_report_writes_them() {
        assert_eq!(parse_addr("0xa8008aa8"), Some(0xa800_8aa8));
        assert_eq!(parse_addr("A8008AA8"), Some(0xa800_8aa8));
        assert_eq!(parse_addr("0x"), None);
        assert_eq!(parse_addr("start"), None);
        assert_eq!(parse_addr("0x1_0000_0000"), None);
    }
}
