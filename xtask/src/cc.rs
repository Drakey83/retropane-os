//! The C and C++ compiler for the X51v, and the C library, in the build
//! (docs/design/half-life-readiness.md lines 2 and 3; `cargo xtask cc`).
//!
//! Iterate, don't imitate (rule 0). Windows Mobile programs were built with
//! eVC or Visual Studio; Linux handhelds with a GCC cross toolchain each
//! distribution packages its own way; picolibc's own build wants Meson and
//! Ninja (and Python) on the PC. Ours: one pinned Clang release, the same
//! LLVM version as the Rust compiler's back end (so C and Rust objects are
//! one code generator's, and link-time optimisation across them is open),
//! fetched once by its SHA-256 as the emulator is, and picolibc compiled by
//! this file straight from its pinned source with our own settings
//! (`ports/libc/picolibc.h`): no Meson, no CMake, no Python, the same on
//! every PC, and every source in parallel (824 files in about 5 s here).
//! The memory routines are left out: a C program calls `rp-hal-mem`'s, the
//! tuned ones Rust code uses.
//!
//! Nothing here runs in the everyday builds (`cross`, `ci`): the C images
//! are built only by the commands that need them, so a checkout without
//! the 467 MB toolchain builds everything else.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The pinned LLVM release: the Rust toolchain's LLVM (`rustc -vV`), so
/// both compilers are the same code generator. Bump with the Rust pin.
pub const LLVM: &str = "23.1.1";
const LLVM_URL: &str = "https://github.com/llvm/llvm-project/releases/download";

/// The pinned picolibc release.
pub const PICOLIBC: &str = "1.8.12";
const PICOLIBC_URL: &str = "https://github.com/picolibc/picolibc/releases/download";
const PICOLIBC_SHA256: &str = "64e8c412e1c40fa6eb1a72d2b5cdbcbfe6ceca4cbea454edbad54557ffc747fa";

/// A release archive of the toolchain for the PC this runs on.
struct Asset {
    file: &'static str,
    sha256: &'static str,
    /// The archive's top folder.
    dir: &'static str,
}

/// The LLVM project's own release files (GitHub's digests, 2026-11-15).
const fn asset() -> Asset {
    if cfg!(windows) {
        Asset {
            file: "clang+llvm-23.1.1-x86_64-pc-windows-msvc.tar.zst",
            sha256: "c8a12d754b5050c5668b56a5425c806792d46c70f7244b1216046164aa4b6462",
            dir: "clang+llvm-23.1.1-x86_64-pc-windows-msvc",
        }
    } else {
        // Not yet run on a Linux PC (docs/reminders.md R249).
        Asset {
            file: "LLVM-23.1.1-Linux-X64.tar.xz",
            sha256: "832aeb58d105de1cabc7b982dd2c65de0610f7377df48ae8fc2dd8e97420a15c",
            dir: "LLVM-23.1.1-Linux-X64",
        }
    }
}

/// What every C file for the X51v is compiled with: the Rust target's own
/// CPU and float model (`targets/armv5te-x51v-none-eabi.json`), ARM code,
/// each function and object in a section of its own so the linker drops
/// what an image does not call.
const TARGET_FLAGS: &[&str] = &[
    "--target=armv5te-none-eabi",
    "-mcpu=xscale",
    "-mfloat-abi=soft",
    "-marm",
    "-O2",
    "-ffreestanding",
    "-fno-common",
    "-fno-stack-protector",
    "-ffunction-sections",
    "-fdata-sections",
    "-nostdinc",
];

/// What C++ is compiled with: the language the Half-Life SDK is written
/// in (no exceptions, no run-time type information, no standard C++
/// library: `ports/libc/cxxrt.cpp` is the whole runtime), one thread.
const CXX_FLAGS: &[&str] = &[
    "-std=c++17",
    "-fno-exceptions",
    "-fno-rtti",
    "-fno-threadsafe-statics",
    "-nostdinc++",
];

/// picolibc's folders that are not built: character-set conversion, Sun
/// RPC, the sanitiser's runtime, long double and complex maths, stack
/// protection (none asked for by a port; each is a line to remove when one
/// is).
const SKIP_DIRS: &[&str] = &["iconv", "xdr", "ubsan", "ld", "complex", "include", "ssp"];

/// picolibc's sources left out because `rp-hal-mem` has the function: one
/// set of memory routines for Rust and C.
const OURS: &[&str] = &[
    "memcpy",
    "memmove",
    "memset",
    "memcmp",
    "bcmp",
    "aeabi_memset",
];

/// The pinned compiler.
pub struct Toolchain {
    bin: PathBuf,
}

impl Toolchain {
    fn tool(&self, name: &str) -> PathBuf {
        self.bin.join(if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_string()
        })
    }

    /// The compiler's own headers (`stddef.h`, `stdarg.h`, `float.h`).
    fn own_headers(&self) -> Result<PathBuf, String> {
        let out = Command::new(self.tool("clang"))
            .arg("-print-resource-dir")
            .output()
            .map_err(|e| format!("clang: {e}"))?;
        Ok(PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()).join("include"))
    }
}

fn home(root: &Path) -> PathBuf {
    root.join("target").join("toolchain")
}

/// `path`'s SHA-256 in hex.
fn sha256(path: &Path) -> Result<String, String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut h = rp_crypto::Sha256::default();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f
            .read(&mut buf)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(String::from_utf8_lossy(&rp_crypto::hex(&h.finish())).into_owned())
}

/// `url` in `to`, kept only if its SHA-256 is `want`: a file that is not
/// the pinned one is removed and refused, never unpacked.
fn fetch(url: &str, to: &Path, want: &str) -> Result<(), String> {
    if !to.is_file() {
        if let Some(dir) = to.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        eprintln!("    downloading {url} ...");
        crate::emu::run(Command::new("curl").args(["-fsSL", "-o"]).arg(to).arg(url))?;
    }
    let got = sha256(to)?;
    if got != want {
        let _ = std::fs::remove_file(to);
        return Err(format!(
            "{}: SHA-256 {got}, not the pinned {want}: removed (xtask/src/cc.rs)",
            to.display()
        ));
    }
    Ok(())
}

/// The tar that reads the archive: Windows' own (bsdtar: zstd and xz), not
/// Git's (no zstd), whichever shell this runs from.
fn tar() -> Command {
    if cfg!(windows) {
        let sys = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        Command::new(Path::new(&sys).join("System32").join("tar.exe"))
    } else {
        Command::new("tar")
    }
}

/// The pinned compiler, downloaded and unpacked on first use (the programs
/// the build runs and the compiler's headers: 475 MB of the archive's
/// several GB). `RETROPANE_LLVM=<folder>` uses an LLVM already on the PC
/// instead, if it is the pinned version.
///
/// # Errors
/// The download failed, its hash is not the pinned one, or the folder
/// given holds another version.
pub fn toolchain(root: &Path) -> Result<Toolchain, String> {
    if let Some(dir) = std::env::var_os("RETROPANE_LLVM").filter(|d| !d.is_empty()) {
        let tc = Toolchain {
            bin: PathBuf::from(dir).join("bin"),
        };
        let out = Command::new(tc.tool("clang"))
            .arg("--version")
            .output()
            .map_err(|e| format!("RETROPANE_LLVM: clang: {e}"))?;
        let text = String::from_utf8_lossy(&out.stdout);
        if !text.contains(&format!("version {LLVM}")) {
            return Err(format!(
                "RETROPANE_LLVM is not LLVM {LLVM}, the pinned version (xtask/src/cc.rs): {}",
                text.lines().next().unwrap_or("")
            ));
        }
        return Ok(tc);
    }
    let a = asset();
    let tc = Toolchain {
        bin: home(root).join(a.dir).join("bin"),
    };
    if tc.tool("clang").is_file() && tc.tool("llvm-ar").is_file() {
        return Ok(tc);
    }
    let archive = home(root).join("dl").join(a.file);
    fetch(
        &format!("{LLVM_URL}/llvmorg-{LLVM}/{}", a.file),
        &archive,
        a.sha256,
    )?;
    eprintln!("    unpacking LLVM {LLVM} ...");
    let mut cmd = tar();
    cmd.arg("-xf").arg(&archive).arg("-C").arg(home(root));
    if cfg!(windows) {
        for exe in [
            "clang",
            "clang++",
            "ld.lld",
            "lld",
            "llvm-ar",
            "llvm-nm",
            "llvm-objdump",
            "llvm-size",
        ] {
            cmd.arg(format!("{}/bin/{exe}.exe", a.dir));
        }
        cmd.arg(format!("{}/lib/clang", a.dir));
    } else {
        cmd.arg("--wildcards");
        for pat in [
            "bin/clang*",
            "bin/*lld*",
            "bin/llvm-ar",
            "bin/llvm-nm",
            "bin/llvm-objdump",
            "bin/llvm-size",
            "lib/clang/*",
        ] {
            cmd.arg(format!("{}/{pat}", a.dir));
        }
    }
    crate::emu::run(&mut cmd)?;
    if tc.tool("clang").is_file() {
        Ok(tc)
    } else {
        Err(format!(
            "the LLVM archive did not hold {}",
            tc.tool("clang").display()
        ))
    }
}

/// picolibc's pinned source, downloaded and unpacked on first use.
fn picolibc_source(root: &Path) -> Result<PathBuf, String> {
    let dir = home(root)
        .join("picolibc")
        .join(format!("picolibc-{PICOLIBC}"));
    if dir.join("libc").is_dir() {
        return Ok(dir);
    }
    let file = format!("picolibc-{PICOLIBC}.tar.xz");
    let archive = home(root).join("dl").join(&file);
    fetch(
        &format!("{PICOLIBC_URL}/{PICOLIBC}/{file}"),
        &archive,
        PICOLIBC_SHA256,
    )?;
    let into = home(root).join("picolibc");
    std::fs::create_dir_all(&into).map_err(|e| format!("{}: {e}", into.display()))?;
    crate::emu::run(tar().arg("-xf").arg(&archive).arg("-C").arg(&into))?;
    Ok(dir)
}

/// One C or assembly source of picolibc to build.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Source {
    path: PathBuf,
    /// Built with `-fno-builtin` (its own list says so: the string
    /// routines, which the compiler would otherwise turn into calls to
    /// themselves).
    no_builtin: bool,
}

/// The words between `name(` and its `)` for every call of `name` in a
/// CMake list file's `text` (comments removed).
fn cmake_calls(text: &str, name: &str) -> Vec<Vec<String>> {
    let clean: String = text
        .lines()
        .map(|l| l.split('#').next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    let mut out = Vec::new();
    let mut rest = clean.as_str();
    while let Some(at) = rest.find(name) {
        let before_ok = rest[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'));
        let after = &rest[at + name.len()..];
        let open = after.trim_start();
        if before_ok
            && open.starts_with('(')
            && let Some(end) = open.find(')')
        {
            out.push(
                open[1..end]
                    .split_whitespace()
                    .map(str::to_string)
                    .collect(),
            );
            rest = &open[end..];
            continue;
        }
        rest = after;
    }
    out
}

/// picolibc's sources for this build, from its own source lists (each
/// folder's `CMakeLists.txt`, which the pinned archive fixes): the ARM
/// versions first (a later file of the same name is the generic one, and
/// dropped, as picolibc's own build does), every option's branch taken
/// (each file guards itself), [`SKIP_DIRS`] and [`OURS`] left out.
fn sources(src: &Path) -> Result<Vec<Source>, String> {
    fn walk(dir: &Path, seen: &mut Vec<String>, out: &mut Vec<Source>) -> Result<(), String> {
        let list = dir.join("CMakeLists.txt");
        let Ok(text) = std::fs::read_to_string(&list) else {
            return Ok(());
        };
        let subs: Vec<String> = cmake_calls(&text, "add_subdirectory")
            .into_iter()
            .filter_map(|w| w.into_iter().next())
            .collect();
        if subs.iter().any(|s| s == "machine") {
            walk(&dir.join("machine").join("arm"), seen, out)?;
        }
        for (flagged, calls) in [
            (false, cmake_calls(&text, "picolibc_sources")),
            (true, cmake_calls(&text, "picolibc_sources_flags")),
        ] {
            for words in calls {
                let no_builtin =
                    flagged && words.first().is_some_and(|w| w.contains("-fno-builtin"));
                for w in words.iter().skip(usize::from(flagged)) {
                    let Some((base, ext)) = w.rsplit_once('.') else {
                        continue;
                    };
                    if !matches!(ext, "c" | "S") {
                        continue;
                    }
                    let base = base.rsplit('/').next().unwrap_or(base).to_string();
                    if seen.contains(&base) {
                        continue;
                    }
                    seen.push(base.clone());
                    if !OURS.contains(&base.as_str()) {
                        out.push(Source {
                            path: dir.join(w),
                            no_builtin,
                        });
                    }
                }
            }
        }
        for s in subs
            .iter()
            .filter(|s| *s != "machine" && !SKIP_DIRS.contains(&s.as_str()))
        {
            walk(&dir.join(s), seen, out)?;
        }
        Ok(())
    }
    let mut out = Vec::new();
    let mut seen = Vec::new();
    walk(&src.join("libc"), &mut seen, &mut out)?;
    walk(&src.join("libm"), &mut seen, &mut out)?;
    if out.len() < 500 {
        return Err(format!(
            "picolibc's source lists gave {} files: its layout is not the pinned one's",
            out.len()
        ));
    }
    Ok(out)
}

/// One compile: the source, its object, the flags after the common ones.
struct Job {
    source: PathBuf,
    object: PathBuf,
    flags: Vec<String>,
}

/// Compiles every job, as many at once as the PC has cores; the first
/// failures' own words if any failed.
fn compile(tc: &Toolchain, common: &[String], jobs: &[Job]) -> Result<(), String> {
    let next = AtomicUsize::new(0);
    let failed = Mutex::new(Vec::new());
    let clang = tc.tool("clang");
    let workers = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                while let Some(job) = jobs.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let out = Command::new(&clang)
                        .args(common)
                        .args(&job.flags)
                        .arg("-c")
                        .arg(&job.source)
                        .arg("-o")
                        .arg(&job.object)
                        .output();
                    let error = match out {
                        Ok(o) if o.status.success() => continue,
                        Ok(o) => String::from_utf8_lossy(&o.stderr).into_owned(),
                        Err(e) => e.to_string(),
                    };
                    if let Ok(mut f) = failed.lock() {
                        f.push(format!("{}:\n{error}", job.source.display()));
                    }
                }
            });
        }
    });
    let failed = failed.into_inner().unwrap_or_default();
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} of {} C files did not compile; the first:\n{}",
            failed.len(),
            jobs.len(),
            failed[0]
        ))
    }
}

/// Puts `objects` into the static library `lib` (through a response file:
/// 800 paths are more than a Windows command line holds).
fn archive(tc: &Toolchain, lib: &Path, objects: &[PathBuf]) -> Result<(), String> {
    let _ = std::fs::remove_file(lib);
    let list = lib.with_extension("rsp");
    let text: String = objects
        .iter()
        .map(|o| format!("\"{}\"\n", o.to_string_lossy().replace('\\', "/")))
        .collect();
    std::fs::write(&list, text).map_err(|e| format!("{}: {e}", list.display()))?;
    crate::emu::run(
        Command::new(tc.tool("llvm-ar"))
            .arg("rcs")
            .arg(lib)
            .arg(format!("@{}", list.to_string_lossy().replace('\\', "/"))),
    )
}

/// The C library built for the X51v.
pub struct Libc {
    /// The folder holding `libc.a` (and the C programs' own libraries).
    pub lib_dir: PathBuf,
    /// What a C program is compiled with to find its headers.
    pub include_flags: Vec<String>,
}

fn flag_strings(flags: &[&str]) -> Vec<String> {
    flags.iter().map(|f| (*f).to_string()).collect()
}

/// picolibc built for the X51v with our settings, once: built again only
/// when the settings, the flags, the source list's rules or a pinned
/// version change.
///
/// # Errors
/// A download failed or was not the pinned file, or a source did not
/// compile.
pub fn libc(root: &Path, tc: &Toolchain) -> Result<Libc, String> {
    let src = picolibc_source(root)?;
    let out = root.join("target").join("c");
    let inc = out.join("include");
    let obj = out.join("libc-obj");
    let lib = out.join("libc.a");
    let config_src = root.join("ports").join("libc").join("picolibc.h");
    let config = std::fs::read_to_string(&config_src)
        .map_err(|e| format!("{}: {e}", config_src.display()))?;
    let own = tc.own_headers()?;
    let s = |p: PathBuf| p.to_string_lossy().into_owned();
    let mut include_flags = Vec::new();
    for dir in [
        inc.clone(),
        src.join("libc/machine/arm"),
        src.join("libc/include"),
        own,
    ] {
        include_flags.push("-isystem".to_string());
        include_flags.push(s(dir));
    }
    let key = format!(
        "llvm {LLVM}\npicolibc {PICOLIBC}\nflags {TARGET_FLAGS:?}\nskip {SKIP_DIRS:?}\nours {OURS:?}\n{config}"
    );
    let stamp = out.join("libc.stamp");
    if lib.is_file() && std::fs::read_to_string(&stamp).is_ok_and(|k| k == key) {
        return Ok(Libc {
            lib_dir: out,
            include_flags,
        });
    }
    let _ = std::fs::remove_dir_all(&obj);
    for d in [&inc, &obj] {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    std::fs::write(inc.join("picolibc.h"), &config).map_err(|e| e.to_string())?;
    let mut common = flag_strings(TARGET_FLAGS);
    common.extend(flag_strings(&[
        "-nostdlib",
        "-D_LIBC",
        "-frounding-math",
        "-w",
    ]));
    common.extend(include_flags.iter().cloned());
    // A source's own folder is on its include path already; these hold the
    // private headers other folders' sources include.
    for dir in [
        "libc/stdio",
        "libc/locale",
        "libc/stdlib",
        "libc/string",
        "libc/ctype",
        "libc/time",
        "libc/posix",
        "libc/search",
        "libc/misc",
        "libm/common",
        "libm/math",
        "libm/fenv",
    ] {
        common.push("-I".into());
        common.push(s(src.join(dir)));
    }
    let list = sources(&src)?;
    let jobs: Vec<Job> = list
        .iter()
        .enumerate()
        .map(|(i, source)| Job {
            source: source.path.clone(),
            object: obj.join(format!(
                "{i:03}-{}.o",
                source
                    .path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
            )),
            flags: if source.no_builtin {
                vec!["-fno-builtin".into()]
            } else {
                vec![]
            },
        })
        .collect();
    eprintln!(
        "    building picolibc {PICOLIBC} for the X51v ({} files) ...",
        jobs.len()
    );
    compile(tc, &common, &jobs)?;
    let objects: Vec<PathBuf> = jobs.into_iter().map(|j| j.object).collect();
    archive(tc, &lib, &objects)?;
    std::fs::write(&stamp, key).map_err(|e| e.to_string())?;
    Ok(Libc {
        lib_dir: out,
        include_flags,
    })
}

/// The C and C++ sources in `dir` (not below it) compiled, warnings as
/// errors, into
/// `lib<name>.a` beside `libc.a`.
///
/// # Errors
/// A source did not compile, warnings included.
pub fn program(tc: &Toolchain, libc: &Libc, name: &str, dir: &Path) -> Result<(), String> {
    let obj = libc.lib_dir.join(format!("{name}-obj"));
    let _ = std::fs::remove_dir_all(&obj);
    std::fs::create_dir_all(&obj).map_err(|e| format!("{}: {e}", obj.display()))?;
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "c" || x == "cpp"))
        .collect();
    files.sort();
    if files.is_empty() {
        return Err(format!("{}: no C or C++ sources", dir.display()));
    }
    let mut common = flag_strings(TARGET_FLAGS);
    common.extend(flag_strings(&["-Wall", "-Wextra", "-Werror"]));
    common.extend(libc.include_flags.iter().cloned());
    let jobs: Vec<Job> = files
        .iter()
        .map(|f| Job {
            source: f.clone(),
            object: obj.join(format!(
                "{}.o",
                f.file_name().unwrap_or_default().to_string_lossy()
            )),
            flags: flag_strings(if f.extension().is_some_and(|x| x == "cpp") {
                CXX_FLAGS
            } else {
                &["-std=c11"]
            }),
        })
        .collect();
    compile(tc, &common, &jobs)?;
    let objects: Vec<PathBuf> = jobs.into_iter().map(|j| j.object).collect();
    archive(tc, &libc.lib_dir.join(format!("lib{name}.a")), &objects)
}

/// `cargo xtask cc`: the compiler and the C library ready, and what they
/// are.
///
/// # Errors
/// As [`toolchain`] and [`libc`].
pub fn ready(root: &Path) -> Result<(Toolchain, Libc), String> {
    let tc = toolchain(root)?;
    let libc = libc(root, &tc)?;
    let size = std::fs::metadata(libc.lib_dir.join("libc.a")).map_or(0, |m| m.len());
    eprintln!(
        "    C toolchain: Clang {LLVM}, picolibc {PICOLIBC} ({} KiB as a library; an image links only what it calls)",
        size / 1024
    );
    Ok((tc, libc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cmake_list_gives_its_calls_words() {
        let text = "# a comment picolibc_sources(no.c)\nadd_subdirectory(machine)\nadd_subdirectory( stdio )\npicolibc_sources(\n  a.c # why\n  b.S\n  c.h\n)\npicolibc_sources_flags(\"-fno-builtin\"\n  memchr.c\n)\nmy_picolibc_sources(x.c)\n";
        assert_eq!(
            cmake_calls(text, "add_subdirectory"),
            vec![vec!["machine".to_string()], vec!["stdio".to_string()]]
        );
        // The comment's call and the longer name's are not this call.
        assert_eq!(
            cmake_calls(text, "picolibc_sources"),
            vec![vec!["a.c".to_string(), "b.S".into(), "c.h".into()]]
        );
        assert_eq!(
            cmake_calls(text, "picolibc_sources_flags"),
            vec![vec!["\"-fno-builtin\"".to_string(), "memchr.c".into()]]
        );
        // A call never closed is not a call.
        assert!(cmake_calls("picolibc_sources(a.c", "picolibc_sources").is_empty());
    }

    #[test]
    fn the_source_list_takes_the_arm_file_first_and_leaves_ours_out() {
        let dir = std::env::temp_dir().join(format!("rp-cc-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let put = |rel: &str, text: &str| {
            let p = dir.join(rel);
            std::fs::create_dir_all(p.parent().expect("has a folder")).expect("folder");
            std::fs::write(p, text).expect("file");
        };
        put(
            "libc/CMakeLists.txt",
            "add_subdirectory(machine)\nadd_subdirectory(string)\nadd_subdirectory(iconv)\n",
        );
        put(
            "libc/machine/arm/CMakeLists.txt",
            "picolibc_sources_flags(\"-fno-builtin\"\n strlen.S\n memcpy.S\n)\n",
        );
        let many: String = (0..600).map(|i| format!(" f{i}.c\n")).collect();
        put(
            "libc/string/CMakeLists.txt",
            &format!("picolibc_sources(\n strlen.c\n memset.c\n{many})\n"),
        );
        put("libc/iconv/CMakeLists.txt", "picolibc_sources(iconv.c)\n");
        put("libm/CMakeLists.txt", "add_subdirectory(math)\n");
        put("libm/math/CMakeLists.txt", "picolibc_sources(s_sin.c)\n");
        let got = sources(&dir).expect("a list");
        let names: Vec<String> = got
            .iter()
            .map(|s| {
                s.path
                    .file_name()
                    .expect("a name")
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        // The ARM strlen, with its flag; the generic one dropped.
        assert_eq!(names[0], "strlen.S");
        assert!(got[0].no_builtin);
        assert!(!names.contains(&"strlen.c".to_string()));
        // rp-hal-mem's functions, in either folder.
        assert!(
            !names
                .iter()
                .any(|n| n.starts_with("memcpy") || n.starts_with("memset"))
        );
        // A skipped folder; the maths library after the C library.
        assert!(!names.contains(&"iconv.c".to_string()));
        assert_eq!(names.last().map(String::as_str), Some("s_sin.c"));
        assert_eq!(got.len(), 602);
        // A layout that is not picolibc's is refused, not built short.
        std::fs::remove_dir_all(dir.join("libc/string")).expect("removed");
        assert!(sources(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_is_not_the_pinned_one_is_removed_and_refused() {
        let f = std::env::temp_dir().join(format!("rp-cc-fetch-{}", std::process::id()));
        std::fs::write(&f, b"abc").expect("file");
        // SHA-256("abc"), FIPS 180-2's first example.
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(sha256(&f).as_deref(), Ok(abc));
        assert!(fetch("unused", &f, abc).is_ok());
        assert!(f.is_file());
        let err = fetch("unused", &f, PICOLIBC_SHA256).expect_err("another file");
        assert!(err.contains("not the pinned"));
        assert!(!f.exists());
    }
}
