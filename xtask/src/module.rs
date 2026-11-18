//! The module packer (ADR-0039): a big program's libraries linked at the
//! addresses they run at, against the runner's own addresses, and written
//! as module files (`rp_port::module` is the format; the runner's loader
//! reads what this writes).
//!
//! Iterate, don't imitate (rule 0). Every other system finishes a
//! library's link on the device: Linux and the consoles' ports resolve
//! names and fill call tables at each start (position-independent code, a
//! PLT, a GOT), Windows relocates and walks imports, and the Vita and
//! Switch ports of this very engine keep the list of C library functions
//! the main program must export by hand. Ours finishes it here, where the
//! runner and the game are built together:
//!
//! - what a module asks of the runner is **found**: a trial link reports
//!   the names it leaves undefined (only those its kept code reaches);
//! - the runner is **told to keep exactly those**, by one table of their
//!   addresses the build links into it, which is also the runner's
//!   identity (a module built against other addresses is refused);
//! - the module is then linked for good with each of those names given
//!   its address in the runner (`name = 0x...;`), so a call into the C
//!   library is one plain branch;
//! - the names a module exports are two tables this file sorts (by name
//!   in the assembly it generates, by address after the link), from the
//!   linked module's own symbols.
//!
//! A module asking for a name the runner has not, a call that would need
//! a veneer (out of a branch's reach), or an import of a Thumb function
//! fails the build, not the device.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use rp_port::module::{self as format, EXPORT, HEADER, Header, Names};

use crate::cc::{self, Job, Toolchain};

/// Where modules are placed from: the RAM pool's first byte.
pub const POOL: u32 = format::POOL;
/// The pool's end.
pub const POOL_END: u32 = POOL + format::POOL_LEN;
/// A module's file name on the card and in flash: `<name>.rpm`.
pub const EXTENSION: &str = "rpm";
/// The table of the runner's exported addresses, as the runner's own
/// code names it (`hal/cabi`), and the library that holds it.
pub const RUNNER_TABLE: &str = "rp_runner_table";
/// See [`RUNNER_TABLE`].
pub const RUNNER_TABLE_LIB: &str = "rprunnertable";

/// One module to build.
pub struct Spec {
    /// Its name: the file is `<name>.rpm`.
    pub name: &'static str,
    /// Its objects, as [`localised`] left them.
    pub objects: Vec<PathBuf>,
    /// Libraries linked into it: what has no state and is hot (ADR-0039
    /// rule 6: the arithmetic, the memory routines, the C++ run-time).
    pub archives: Vec<PathBuf>,
    /// Its entry's name, if it is a program and not only a library.
    pub entry: Option<&'static str>,
    /// The names it exports (functions looked up by name at run time).
    pub exports: Vec<String>,
    /// Its hottest functions, gathered by the link into `.rp_hot`, whole
    /// 64 KiB pages from `__rp_hot` to `__rp_hot_end` (ADR-0040 step 7b:
    /// the runner maps those pages onto the on-chip SRAM). Empty: none.
    pub hot: Vec<String>,
}

/// A symbol as `llvm-nm -P` prints it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    /// `T` code, `D`/`B`/`R` data, `U` undefined, `W`/`V` weak; lower case
    /// for a local one.
    pub kind: char,
    pub value: u32,
}

/// `llvm-nm -P`'s lines: `name kind [value [size]]`.
pub fn parse_nm(out: &str) -> Vec<Symbol> {
    out.lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let name = f.next()?.to_string();
            let kind = f.next()?.chars().next()?;
            let value = f
                .next()
                .and_then(|v| u32::from_str_radix(v, 16).ok())
                .unwrap_or(0);
            Some(Symbol { name, kind, value })
        })
        .collect()
}

/// The names the linker could not find (its `undefined symbol:` lines),
/// once each, sorted.
pub fn parse_undefined(stderr: &str) -> Vec<String> {
    let mut out: Vec<String> = stderr
        .lines()
        .filter_map(|l| l.split("undefined symbol: ").nth(1))
        .map(|s| s.trim().to_string())
        .collect();
    out.sort();
    out.dedup();
    out
}

fn tool_output(cmd: &mut Command) -> Result<String, String> {
    let out = cmd.output().map_err(|e| format!("{cmd:?}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{cmd:?} failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The symbols of `file` that are defined and global.
///
/// # Errors
/// `llvm-nm` could not read it.
pub fn globals(tc: &Toolchain, file: &Path) -> Result<Vec<Symbol>, String> {
    tool_output(
        Command::new(tc.tool("llvm-nm"))
            .args(["-P", "--defined-only", "--extern-only", "--no-demangle"])
            .arg(file),
    )
    .map(|out| parse_nm(&out))
}

/// The functions among `symbols`, by name, sorted as bytes: the order of
/// a module's by-name table.
pub fn functions(symbols: &[Symbol]) -> Vec<String> {
    let mut out: Vec<String> = symbols
        .iter()
        .filter(|s| matches!(s.kind, 'T' | 'W'))
        .map(|s| s.name.clone())
        .collect();
    out.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    out.dedup();
    out
}

/// `object` with every symbol its sources marked hidden made local (a
/// part compiled with `-fvisibility=hidden` then exports exactly what its
/// source marks for export), written to `out`; with a `prefix`, what is
/// left global and defined is renamed `<prefix><name>` (a part linked
/// inside another: its entry points must not clash with the other parts'
/// of the same name). The functions left global, by their names in the
/// source.
///
/// # Errors
/// A tool failed.
pub fn localised(
    tc: &Toolchain,
    object: &Path,
    out: &Path,
    prefix: Option<&str>,
) -> Result<Vec<String>, String> {
    let objcopy = tc.tool("llvm-objcopy");
    let Some(prefix) = prefix else {
        tool_output(
            Command::new(&objcopy)
                .arg("--localize-hidden")
                .arg(object)
                .arg(out),
        )?;
        return Ok(functions(&globals(tc, out)?));
    };
    let step = out.with_extension("hidden.o");
    tool_output(
        Command::new(&objcopy)
            .arg("--localize-hidden")
            .arg(object)
            .arg(&step),
    )?;
    let left = globals(tc, &step)?;
    let renames: String = left
        .iter()
        .map(|s| format!("{} {prefix}{}\n", s.name, s.name))
        .collect();
    let list = out.with_extension("renames");
    std::fs::write(&list, renames).map_err(|e| format!("{}: {e}", list.display()))?;
    tool_output(
        Command::new(&objcopy)
            .arg(format!("--redefine-syms={}", list.display()))
            .arg(&step)
            .arg(out),
    )?;
    Ok(functions(&left))
}

/// The assembly of a module's own record: room for the header, the
/// by-name table (`exports`, which must be sorted as bytes: each entry
/// its name's offset in the names and the function's address, which also
/// keeps the function when the linker drops what nothing reaches), room
/// for the by-address table, and the names.
pub fn tables_asm(exports: &[String]) -> String {
    let mut s = format!(
        "\t.section .rp_header,\"a\",%progbits\n\t.space {HEADER}\n\t.section .rp_exports,\"a\",%progbits\n\t.balign 4\n"
    );
    let mut offset = 0;
    for name in exports {
        s.push_str(&format!("\t.long {offset}\n\t.long \"{name}\"\n"));
        offset += name.len() + 1;
    }
    s.push_str(&format!(
        "\t.section .rp_by_address,\"a\",%progbits\n\t.balign 4\n\t.space {}\n\t.section .rp_names,\"a\",%progbits\n",
        exports.len() * 4
    ));
    for name in exports {
        s.push_str(&format!("\t.asciz \"{name}\"\n"));
    }
    s
}

/// The assembly of the runner's table: the address of every name the
/// modules import, in the names' own order (`names` sorted). Linked into
/// the runner it keeps each of them there, and its bytes are the runner
/// build's identity.
pub fn runner_table_asm(names: &[String]) -> String {
    let mut s = format!(
        "\t.section .rodata.{RUNNER_TABLE},\"a\",%progbits\n\t.balign 4\n\t.globl {RUNNER_TABLE}\n{RUNNER_TABLE}:\n"
    );
    for name in names {
        s.push_str(&format!("\t.long \"{name}\"\n"));
    }
    s.push_str(&format!(
        "\t.globl {RUNNER_TABLE}_end\n{RUNNER_TABLE}_end:\n"
    ));
    s
}

/// The linker script of a module placed at `base`: the header first, then
/// what is loaded, then its zeroed RAM.
pub fn link_script(base: u32, hot: &[String]) -> String {
    format!(
        "SECTIONS\n{{\n    . = 0x{base:08X};\n    .rp_header : {{ KEEP(*(.rp_header)) }}\n{}    .text : {{ *(.text .text.*) }}\n    .rodata : ALIGN(4) {{ *(.rodata .rodata.*) }}\n    .rp_tables : ALIGN(4) {{\n        __rp_exports = .;\n        KEEP(*(.rp_exports))\n        __rp_by_address = .;\n        KEEP(*(.rp_by_address))\n        __rp_names = .;\n        KEEP(*(.rp_names))\n        __rp_names_end = .;\n    }}\n    .init_array : ALIGN(4) {{\n        __rp_init = .;\n        KEEP(*(SORT_BY_INIT_PRIORITY(.init_array.*)))\n        KEEP(*(.init_array))\n        __rp_init_end = .;\n    }}\n    .data : ALIGN(4) {{ *(.data .data.*) *(.got .got.*) }}\n    . = ALIGN(4);\n    __rp_file_end = .;\n    .bss (NOLOAD) : ALIGN(16) {{ *(.bss .bss.* COMMON) }}\n    . = ALIGN(16);\n    __rp_mem_end = .;\n    /DISCARD/ : {{ *(.ARM.exidx .ARM.exidx.* .ARM.extab.* .fini_array .fini_array.* .preinit_array) }}\n}}\n",
        hot_section(hot)
    )
}

/// The `.rp_hot` section of [`link_script`]: `hot`'s functions (each
/// compiled into a section of its own, `.text.<name>`) from a 64 KiB
/// boundary, the section padded to the next one, so the runner can map
/// its pages whole onto the on-chip SRAM (ADR-0040 step 7b). Nothing for
/// an empty list.
fn hot_section(hot: &[String]) -> String {
    if hot.is_empty() {
        return String::new();
    }
    let mut s = String::from("    . = ALIGN(0x10000);\n    __rp_hot = .;\n    .rp_hot : {\n");
    for name in hot {
        s.push_str(&format!("        *(.text.{name})\n"));
    }
    s.push_str("    }\n    . = ALIGN(0x10000);\n    __rp_hot_end = .;\n");
    s
}

/// The linker script giving each imported name its address in the runner.
pub fn imports_script(imports: &[(String, u32)]) -> String {
    imports
        .iter()
        .map(|(name, at)| format!("\"{name}\" = 0x{at:08X};\n"))
        .collect()
}

/// Where the next module goes after one that ends at `end`: the next
/// 4 KiB.
#[must_use]
pub const fn next_base(end: u32) -> u32 {
    end.next_multiple_of(4096)
}

/// `text` assembled for the X51v: `<dir>/<name>.o`.
///
/// # Errors
/// The assembler refused it.
pub fn assemble(tc: &Toolchain, dir: &Path, name: &str, text: &str) -> Result<PathBuf, String> {
    let source = dir.join(format!("{name}.S"));
    let object = dir.join(format!("{name}.o"));
    std::fs::write(&source, text).map_err(|e| format!("{}: {e}", source.display()))?;
    cc::compile(
        tc,
        &cc::flag_strings(&["--target=armv5te-none-eabi", "-mcpu=xscale", "-marm"]),
        &[Job {
            source,
            object: object.clone(),
            flags: Vec::new(),
        }],
    )?;
    Ok(object)
}

/// The runner's table as a library in `lib_dir`, for the runner's link.
///
/// # Errors
/// The assembler or the archiver failed.
pub fn runner_table(tc: &Toolchain, lib_dir: &Path, names: &[String]) -> Result<(), String> {
    let object = assemble(tc, lib_dir, RUNNER_TABLE, &runner_table_asm(names))?;
    let lib = lib_dir.join(format!("lib{RUNNER_TABLE_LIB}.a"));
    let _ = std::fs::remove_file(&lib);
    tool_output(
        Command::new(tc.tool("llvm-ar"))
            .arg("rcs")
            .arg(&lib)
            .arg(&object),
    )
    .map(|_| ())
}

/// The linker's command for `spec` at `base`, its output `elf`.
fn linker(
    tc: &Toolchain,
    spec: &Spec,
    dir: &Path,
    base: u32,
    elf: &Path,
) -> Result<Command, String> {
    let tables = assemble(
        tc,
        dir,
        &format!("{}-tables", spec.name),
        &tables_asm(&spec.exports),
    )?;
    let script = dir.join(format!("{}.ld", spec.name));
    std::fs::write(&script, link_script(base, &spec.hot)).map_err(|e| format!("{}: {e}", script.display()))?;
    let mut cmd = Command::new(tc.tool("ld.lld"));
    cmd.arg("-o")
        .arg(elf)
        .args(["--gc-sections", "--no-demangle", "-z", "norelro", "-T"])
        .arg(&script)
        .arg("-e")
        .arg(spec.entry.unwrap_or("0"))
        .arg(&tables)
        .args(&spec.objects)
        .args(&spec.archives);
    Ok(cmd)
}

/// The names `spec` leaves undefined once its own objects and libraries
/// are linked and what nothing reaches is dropped: what it imports from
/// the runner.
///
/// # Errors
/// The link failed for another reason than undefined names.
pub fn imports(tc: &Toolchain, spec: &Spec, dir: &Path) -> Result<Vec<String>, String> {
    if spec.entry.is_none() && spec.exports.is_empty() {
        return Err(format!(
            "module {}: no entry and no exports: nothing of it would be kept",
            spec.name
        ));
    }
    let elf = dir.join(format!("{}-trial.elf", spec.name));
    let mut cmd = linker(tc, spec, dir, POOL, &elf)?;
    let out = cmd
        .arg("--warn-unresolved-symbols")
        .output()
        .map_err(|e| format!("ld.lld: {e}"))?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        return Err(format!(
            "module {}: the trial link failed:\n{stderr}",
            spec.name
        ));
    }
    Ok(parse_undefined(&stderr))
}

/// A module linked at its address.
pub struct Linked {
    pub name: &'static str,
    pub base: u32,
    /// The first address after its zeroed RAM.
    pub end: u32,
    /// What is loaded: the bytes from `base`, the header's room still
    /// empty.
    image: Vec<u8>,
    symbols: BTreeMap<String, u32>,
    entry: Option<&'static str>,
}

/// What stops a link's result from being a module: a veneer between a
/// call and its target (the names lld gives the ones it inserts).
fn veneers(symbols: &[Symbol]) -> Vec<&str> {
    symbols
        .iter()
        .filter(|s| s.name.contains("Thunk_") || s.name.starts_with("__ARMv"))
        .map(|s| s.name.as_str())
        .collect()
}

/// The addresses in `runner` of the names in `wanted`, in `wanted`'s
/// order.
///
/// # Errors
/// The runner has not one of them, or has it as a Thumb function (an
/// address alone cannot say so to the caller).
pub fn resolve(
    wanted: &[String],
    runner: &BTreeMap<String, u32>,
) -> Result<Vec<(String, u32)>, String> {
    wanted
        .iter()
        .map(|name| match runner.get(name) {
            None => Err(format!("the runner has no `{name}`")),
            Some(at) if at % 2 == 1 => Err(format!(
                "the runner's `{name}` is Thumb code: it cannot be imported by address"
            )),
            Some(at) => Ok((name.clone(), *at)),
        })
        .collect()
}

/// The runner image's global symbols by name.
///
/// # Errors
/// `llvm-nm` could not read it.
pub fn runner_symbols(tc: &Toolchain, elf: &Path) -> Result<BTreeMap<String, u32>, String> {
    Ok(globals(tc, elf)?
        .into_iter()
        .map(|s| (s.name, s.value))
        .collect())
}

/// The bytes of the runner's table for `names` (sorted, as the table has
/// them): what [`format::runner_identity`] is taken over.
///
/// # Errors
/// As [`resolve`].
pub fn runner_table_bytes(
    names: &[String],
    runner: &BTreeMap<String, u32>,
) -> Result<Vec<u8>, String> {
    Ok(resolve(names, runner)?
        .iter()
        .flat_map(|(_, at)| at.to_le_bytes())
        .collect())
}

/// Links `spec` for good at `base`, its imports at their addresses in
/// `runner`.
///
/// # Errors
/// A name the runner has not, a veneer, or the linker's own error.
pub fn link(
    tc: &Toolchain,
    spec: &Spec,
    dir: &Path,
    base: u32,
    wanted: &[String],
    runner: &BTreeMap<String, u32>,
) -> Result<Linked, String> {
    let resolved = resolve(wanted, runner).map_err(|e| format!("module {}: {e}", spec.name))?;
    let script = dir.join(format!("{}-imports.ld", spec.name));
    std::fs::write(&script, imports_script(&resolved))
        .map_err(|e| format!("{}: {e}", script.display()))?;
    let elf = dir.join(format!("{}.elf", spec.name));
    let mut cmd = linker(tc, spec, dir, base, &elf)?;
    let out = cmd
        .arg("-T")
        .arg(&script)
        .arg(format!(
            "-Map={}",
            dir.join(format!("{}.map", spec.name)).display()
        ))
        .output()
        .map_err(|e| format!("ld.lld: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "module {}: the link failed:\n{}",
            spec.name,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let all = parse_nm(&tool_output(
        Command::new(tc.tool("llvm-nm"))
            .args(["-P", "--no-demangle"])
            .arg(&elf),
    )?);
    let thunks = veneers(&all);
    if !thunks.is_empty() {
        return Err(format!(
            "module {}: {} calls need a veneer (their target is out of a branch's reach from 0x{base:08X}): the first is {}",
            spec.name,
            thunks.len(),
            thunks[0]
        ));
    }
    let bin = dir.join(format!("{}.bin", spec.name));
    tool_output(
        Command::new(tc.tool("llvm-objcopy"))
            .args(["-O", "binary"])
            .arg(&elf)
            .arg(&bin),
    )?;
    let image = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let symbols: BTreeMap<String, u32> = all.into_iter().map(|s| (s.name, s.value)).collect();
    let end = *symbols
        .get("__rp_mem_end")
        .ok_or_else(|| format!("module {}: the link has no __rp_mem_end", spec.name))?;
    if end > POOL_END {
        return Err(format!(
            "module {}: ends at 0x{end:08X}, past the RAM pool's end 0x{POOL_END:08X}",
            spec.name
        ));
    }
    Ok(Linked {
        name: spec.name,
        base,
        end,
        image,
        symbols,
        entry: spec.entry,
    })
}

/// The module file of what was linked: the header filled in, the
/// by-address table sorted, the check sum over all of it. `runner` is the
/// identity of the runner build it was linked against and `set_end` where
/// the set of modules built with it ends.
///
/// # Errors
/// The link's symbols or tables are not a module's.
pub fn seal(linked: &Linked, runner: u64, set_end: u32) -> Result<Vec<u8>, String> {
    seal_image(
        linked.image.clone(),
        linked.base,
        &linked.symbols,
        linked.entry,
        runner,
        set_end,
    )
    .map_err(|e| format!("module {}: {e}", linked.name))
}

fn seal_image(
    mut file: Vec<u8>,
    base: u32,
    symbols: &BTreeMap<String, u32>,
    entry: Option<&str>,
    runner: u64,
    set_end: u32,
) -> Result<Vec<u8>, String> {
    let at = |name: &str| {
        symbols
            .get(name)
            .copied()
            .ok_or_else(|| format!("the link has no `{name}`"))
    };
    let file_end = at("__rp_file_end")?;
    let file_len = file_end
        .checked_sub(base)
        .filter(|l| *l as usize >= file.len())
        .ok_or_else(|| {
            format!(
                "{} bytes were loaded but the file ends at 0x{file_end:08X}",
                file.len()
            )
        })?;
    file.resize(file_len as usize, 0);
    let (exports, by_address) = (at("__rp_exports")?, at("__rp_by_address")?);
    let count = by_address
        .checked_sub(exports)
        .filter(|b| b % EXPORT as u32 == 0)
        .ok_or("the by-name table is not whole entries")?
        / EXPORT as u32;
    let header = Header {
        runner,
        base,
        file_len,
        mem_len: at("__rp_mem_end")?
            .checked_sub(base)
            .ok_or("the module ends before it starts")?,
        set_end,
        entry: match entry {
            Some(name) => at(name)?,
            None => 0,
        },
        init: (at("__rp_init")?, at("__rp_init_end")?),
        exports: (exports, count),
        by_address,
        names: (
            at("__rp_names")?,
            at("__rp_names_end")?
                .checked_sub(at("__rp_names")?)
                .ok_or("the names end before they start")?,
        ),
    };
    if !header.holds() {
        return Err(format!("its header does not hold: {header:?}"));
    }
    // The by-address table: the by-name entries' numbers, sorted by the
    // address the linker gave each function.
    let word = |file: &[u8], address: u32| {
        let i = (address - base) as usize;
        u32::from_le_bytes([file[i], file[i + 1], file[i + 2], file[i + 3]])
    };
    let mut order: Vec<u32> = (0..count).collect();
    order.sort_by_key(|n| (word(&file, exports + n * EXPORT as u32 + 4), *n));
    for (k, n) in order.iter().enumerate() {
        let i = (by_address - base) as usize + k * 4;
        file[i..i + 4].copy_from_slice(&n.to_le_bytes());
    }
    file[..HEADER].copy_from_slice(&header.to_bytes(0));
    if !Names::new(&file, header).in_order() {
        return Err("its name tables are not in order".into());
    }
    let sum = format::check_sum(&file).ok_or("too short for a header")?;
    file[..HEADER].copy_from_slice(&header.to_bytes(sum));
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_symbol_tool_s_lines_are_read() {
        let out = "GetFSAPI T 1c 40\nerrno U\n_ZN5CBase5ThinkEv W a9001000 8\nlocal_thing t 20 4\nfs_globals B 0 100\n\nbroken\n";
        let s = parse_nm(out);
        assert_eq!(s.len(), 5);
        assert_eq!(
            s[0],
            Symbol {
                name: "GetFSAPI".into(),
                kind: 'T',
                value: 0x1C
            }
        );
        assert_eq!((s[1].kind, s[1].value), ('U', 0));
        assert_eq!(s[2].value, 0xA900_1000);
        // Functions only, global or weak, sorted as bytes (capitals first).
        assert_eq!(functions(&s), ["GetFSAPI", "_ZN5CBase5ThinkEv"]);
        let mixed = parse_nm("b T 0 0\nB T 0 0\na T 0 0\n_x T 0 0\nb T 0 0\n");
        assert_eq!(functions(&mixed), ["B", "_x", "a", "b"]);
    }

    #[test]
    fn the_linker_s_undefined_names_are_listed_once_each_and_sorted() {
        let err = "ld.lld: warning: undefined symbol: stat\n>>> referenced by sys.c\n\nld.lld: warning: undefined symbol: opendir\nld.lld: warning: undefined symbol: stat\nld.lld: warning: x.rlib: archive member 'lib.rmeta' is neither ET_REL nor LLVM bitcode\n";
        assert_eq!(parse_undefined(err), ["opendir", "stat"]);
        assert!(parse_undefined("ld.lld: error: cannot open x.o\n").is_empty());
    }

    #[test]
    fn the_tables_assembly_holds_each_name_once_at_its_offset() {
        let names = vec!["GetEntityAPI".to_string(), "worldspawn".to_string()];
        let s = tables_asm(&names);
        assert!(s.starts_with("\t.section .rp_header,\"a\",%progbits\n\t.space 72\n"));
        // Each entry: the name's offset in the names, then the function.
        assert!(
            s.contains("\t.long 0\n\t.long \"GetEntityAPI\"\n\t.long 13\n\t.long \"worldspawn\"\n")
        );
        assert!(s.contains(".rp_by_address,\"a\",%progbits\n\t.balign 4\n\t.space 8\n"));
        assert!(s.ends_with("\t.asciz \"GetEntityAPI\"\n\t.asciz \"worldspawn\"\n"));
        // No exports: the header's room and empty tables.
        let none = tables_asm(&[]);
        assert!(none.contains(".space 72") && none.contains(".space 0\n"));
        assert!(!none.contains(".long"));
    }

    #[test]
    fn the_scripts_place_the_header_first_and_name_each_import() {
        let s = link_script(0xA912_3000, &[]);
        assert!(!s.contains(".rp_hot"));
        let h = link_script(0xA912_3000, &["R_Foo".into(), "_Z3barv".into()]);
        let hot = h.find(".rp_hot").expect("the hot code");
        assert!(h.find(".rp_header").expect("header") < hot && hot < h.find(".text :").expect("code"));
        assert!(h.contains("*(.text.R_Foo)
        *(.text._Z3barv)"));
        assert_eq!(h.matches("ALIGN(0x10000)").count(), 2);
        let header = s.find(".rp_header").expect("the header");
        assert!(s.contains(". = 0xA9123000;"));
        assert!(header < s.find(".text").expect("code"));
        // The zeroed RAM after everything loaded.
        assert!(s.find("__rp_file_end").expect("end") < s.find(".bss").expect("bss"));
        assert!(s.find(".bss").expect("bss") < s.find("__rp_mem_end").expect("end"));
        assert_eq!(
            imports_script(&[
                ("malloc".into(), 0xA801_2340),
                ("_Znwj".into(), 0xA800_8004)
            ]),
            "\"malloc\" = 0xA8012340;\n\"_Znwj\" = 0xA8008004;\n"
        );
        let t = runner_table_asm(&["free".into(), "malloc".into()]);
        assert!(t.contains(
            "rp_runner_table:\n\t.long \"free\"\n\t.long \"malloc\"\n\t.globl rp_runner_table_end\n"
        ));
        assert_eq!(next_base(0xA900_0001), 0xA900_1000);
        assert_eq!(next_base(0xA900_1000), 0xA900_1000);
    }

    #[test]
    fn an_import_the_runner_has_not_or_has_as_thumb_fails_the_build() {
        let runner: BTreeMap<String, u32> = [
            ("malloc".to_string(), 0xA801_0000),
            ("free".to_string(), 0xA801_0040),
            ("thumb_thing".to_string(), 0xA801_0081),
        ]
        .into();
        let names = |n: &[&str]| n.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        assert_eq!(
            resolve(&names(&["free", "malloc"]), &runner),
            Ok(vec![
                ("free".to_string(), 0xA801_0040),
                ("malloc".to_string(), 0xA801_0000)
            ])
        );
        let err = resolve(&names(&["free", "dlopen"]), &runner).expect_err("not there");
        assert!(err.contains("`dlopen`"));
        let err = resolve(&names(&["thumb_thing"]), &runner).expect_err("thumb");
        assert!(err.contains("Thumb"));
        // The identity is over the addresses in the names' order.
        let bytes = runner_table_bytes(&names(&["free", "malloc"]), &runner).expect("both");
        assert_eq!(bytes, [0x40, 0, 0x01, 0xA8, 0, 0, 0x01, 0xA8]);
        assert_ne!(
            format::runner_identity(&bytes),
            format::runner_identity(
                &runner_table_bytes(&names(&["malloc", "free"]), &runner).expect("both")
            )
        );
    }

    #[test]
    fn a_veneer_is_seen_by_its_name() {
        let s = parse_nm(
            "__ARMv5LongLdrPcThunk_printf t a9000100 8\n__ARMV5PILongThunk_x t 0 0\nThunk t 0 0\nmain T 0 0\n__LongThunk_memcpy t 0 0\n",
        );
        assert_eq!(veneers(&s).len(), 3);
        assert!(veneers(&parse_nm("main T 0 0\nR_Thunker T 0 0\n")).is_empty());
    }

    /// A linked image by hand: the header's room, two functions' worth of
    /// bytes, the tables as [`tables_asm`] would lay them out for `names`
    /// with the functions at `at`, one constructor, some data.
    fn image(base: u32, names: &[&str], at: &[u32]) -> (Vec<u8>, BTreeMap<String, u32>) {
        let mut file = vec![0u8; HEADER];
        file.extend([0xE1u8; 24]);
        let here = |f: &Vec<u8>| base + f.len() as u32;
        let mut symbols = BTreeMap::new();
        symbols.insert("rp_module_main".to_string(), base + HEADER as u32);
        symbols.insert("__rp_exports".to_string(), here(&file));
        let mut offset = 0u32;
        for (name, a) in names.iter().zip(at) {
            file.extend(offset.to_le_bytes());
            file.extend(a.to_le_bytes());
            offset += name.len() as u32 + 1;
        }
        symbols.insert("__rp_by_address".to_string(), here(&file));
        file.extend(vec![0u8; names.len() * 4]);
        symbols.insert("__rp_names".to_string(), here(&file));
        for name in names {
            file.extend(name.as_bytes());
            file.push(0);
        }
        symbols.insert("__rp_names_end".to_string(), here(&file));
        file.resize(file.len().next_multiple_of(4), 0);
        symbols.insert("__rp_init".to_string(), here(&file));
        file.extend((base + HEADER as u32 + 8).to_le_bytes());
        symbols.insert("__rp_init_end".to_string(), here(&file));
        file.extend([7u8; 6]);
        // The tool leaves off the padding after the last section.
        symbols.insert("__rp_file_end".to_string(), here(&file) + 2);
        symbols.insert("__rp_mem_end".to_string(), here(&file) + 2 + 0x100);
        (file, symbols)
    }

    #[test]
    fn a_sealed_module_reads_back_with_both_tables_in_order() {
        let base = 0xA900_0000;
        let f = base + HEADER as u32;
        // Sorted by name; their addresses are not in that order.
        let (bytes, symbols) = image(base, &["Alpha", "beta", "gamma"], &[f + 16, f, f + 8]);
        let file = seal_image(
            bytes,
            base,
            &symbols,
            Some("rp_module_main"),
            0xABCD,
            base + 0x1000,
        )
        .expect("sealed");
        assert_eq!(file.len() % 4, 0);
        let (header, sum) = Header::parse(&file).expect("a header");
        assert!(header.holds());
        assert_eq!(format::check_sum(&file), Some(sum));
        assert_eq!(header.runner, 0xABCD);
        assert_eq!(header.entry, f);
        assert_eq!(header.exports.1, 3);
        assert_eq!(header.mem_len, header.file_len + 0x100);
        assert_eq!(header.set_end, base + 0x1000);
        assert_eq!(header.init.1 - header.init.0, 4);
        let names = Names::new(&file, header);
        assert!(names.in_order());
        assert_eq!(names.find(b"beta"), Some(f));
        assert_eq!(names.name_of(f + 8).map(|n| n.1), Some(&b"gamma"[..]));
        assert_eq!(names.name_of(f + 16).map(|n| n.1), Some(&b"Alpha"[..]));
        // A library: no entry.
        let (bytes, symbols) = image(base, &["Alpha"], &[f]);
        let file = seal_image(bytes, base, &symbols, None, 1, base + 0x1000).expect("sealed");
        assert_eq!(Header::parse(&file).expect("a header").0.entry, 0);
    }

    #[test]
    fn a_link_that_is_not_a_module_is_refused() {
        let base = 0xA900_0000;
        let f = base + HEADER as u32;
        let (bytes, symbols) = image(base, &["a", "b"], &[f, f + 8]);
        // A symbol the script should have made is missing.
        let mut fewer = symbols.clone();
        fewer.remove("__rp_init");
        let err =
            seal_image(bytes.clone(), base, &fewer, None, 1, base + 0x1000).expect_err("no symbol");
        assert!(err.contains("__rp_init"));
        // The entry asked for is not in the link.
        let err = seal_image(
            bytes.clone(),
            base,
            &symbols,
            Some("main"),
            1,
            base + 0x1000,
        )
        .expect_err("no entry");
        assert!(err.contains("`main`"));
        // More bytes loaded than the file is said to hold.
        let mut short = symbols.clone();
        short.insert("__rp_file_end".into(), base + 8);
        assert!(seal_image(bytes.clone(), base, &short, None, 1, base + 0x1000).is_err());
        // The set ends before this module does.
        assert!(seal_image(bytes.clone(), base, &symbols, None, 1, base + 0x40).is_err());
        // Names not sorted: the tables do not hold.
        let (bytes, symbols) = image(base, &["b", "a"], &[f, f + 8]);
        let err = seal_image(bytes, base, &symbols, None, 1, base + 0x1000).expect_err("unsorted");
        assert!(err.contains("not in order"));
    }
}
