//! The Half-Life port's evaluation build (docs/design/half-life-readiness.md
//! line 1; `cargo xtask xash-eval`): Xash3D FWGS and the Half-Life SDK
//! compiled for the X51v with the pinned toolchain (`cc`), each of the
//! engine's parts and each of the game's libraries linked on its own, and
//! what it found said plainly: what compiled, how big each part is, and
//! which calls the C library and the port layer do not answer yet.
//!
//! Iterate, don't imitate (rule 0). The engine is built by Waf, which
//! probes the PC it runs on and wants Python; a port to a new system
//! usually starts as a fork with the build bent to fit. Ours leaves the
//! engine's source as fetched but for three small patches kept here
//! (`ports/xash/patches`: the new platform's name, and three places that
//! assumed POSIX), lists the sources in this file, and builds them with the
//! same compiler, flags and C library as every C program of RetroPane, in
//! parallel, the same on every PC. The game's logic is built as libraries
//! of its own and never linked into the engine (Valve's SDK terms and the
//! GPL do not combine: docs/research/half-life-licences.md).
//!
//! The sources are not fetched here (they are the owner's go, given
//! 2026-11-15): `target/ports-src/xash3d-fwgs` and `hlsdk-portable` at the
//! pinned commits, with the engine's `library_suffix` submodule.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cc::{self, Job, Libc, Toolchain};

/// The pinned sources: (folder under `target/ports-src`, commit, patch).
const SOURCES: [(&str, &str, &str); 3] = [
    (
        "xash3d-fwgs",
        "707596f4905689ec224e993fcaa4d4ad66aa67d6",
        "xash3d-fwgs.patch",
    ),
    (
        "xash3d-fwgs/3rdparty/library_suffix",
        "0d4aec027312308d60de0f8eb087a68ce09623d6",
        "library-suffix.patch",
    ),
    (
        "hlsdk-portable",
        "89e83b92d561f5ceb11594c2f15e2e1519bc6c36",
        "hlsdk-portable.patch",
    ),
];

/// What every file of the engine is compiled with, beside the target's
/// flags: the platform (`__retropane__` selects `XASH_RETROPANE` in the
/// patched `build.h`), the backends (numbers of our own: the engine only
/// checks that each is not its "none"), the single-player build's choices,
/// and what picolibc has (the engine's own build finds these by probing).
const ENGINE_DEFINES: &[&str] = &[
    "-D__retropane__=1",
    "-DXASH_VIDEO=100",
    "-DXASH_SOUND=100",
    "-DXASH_INPUT=100",
    "-DXASH_TIMER=100",
    "-DXASH_LIB=100",
    "-DXASH_ENABLE_MAIN=1",
    "-DENGINE_DLL=1",
    "-DXASH_GAMEDIR=\"valve\"",
    "-DXASH_REF_SOFT_ENABLED=1",
    "-DXASH_MINIZ=1",
    "-DMINIZ_NO_ARCHIVE_APIS=1",
    "-DXASH_BUILD_COMMIT=\"707596f4\"",
    "-DXASH_BUILD_BRANCH=\"master\"",
    "-DXASH_BUILD_COMMIT_DATE=\"2026-11-15\"",
    "-DALLOCA_H=<alloca.h>",
    "-D_GNU_SOURCE=1",
    "-DHAVE_STRNCASECMP=1",
    "-DHAVE_STRCASECMP=1",
    "-DHAVE_STRCASESTR=1",
    "-DHAVE_STRCHRNUL=1",
    "-DHAVE_STRLCPY=1",
    "-DHAVE_STRLCAT=1",
    "-DHAVE_STRNLEN=1",
    "-DHAVE_TGMATH_H=1",
];

/// The engine's sources left out, each answered in
/// `ports/xash/platform/absent.c` or `sys_rp.c` (single-player Half-Life
/// never needs them: the list and what adding each back takes are in
/// docs/research/half-life-evaluation.md).
const LEFT_OUT: &[&str] = &[
    "whereami.c",
    "snd_mp3.c",
    "snd_ogg_opus.c",
    "snd_ogg_vorbis.c",
    "ogg_filestream.c",
    "voice.c",
    "net_http_tls.c",
];

/// What the SDK is compiled with: its own build defines these on every
/// system that is not Windows.
const SDK_DEFINES: &[&str] = &[
    "-D__retropane__=1",
    "-Dstricmp=strcasecmp",
    "-Dstrnicmp=strncasecmp",
    "-D_snprintf=snprintf",
    "-D_vsnprintf=vsnprintf",
    "-D_LINUX=1",
    "-DLINUX=1",
    "-DHAVE_STRLCPY=1",
    "-DHAVE_STRLCAT=1",
    "-D_GNU_SOURCE=1",
];

/// The server library's sources its own build leaves out.
const SERVER_LEFT_OUT: &[&str] = &["mpstubb.cpp", "stats.cpp", "Wxdebug.cpp"];
/// The client library's: a sample in its own folder.
const CLIENT_LEFT_OUT: &[&str] = &["GameStudioModelRenderer_Sample.cpp"];
/// The shared folder's voice files: the client has its own.
const SHARED_LEFT_OUT: &[&str] = &[
    "voice_vgui_tweakdlg.cpp",
    "voice_gamemgr.cpp",
    "voice_status.cpp",
];
/// The weapons the client library shares with the server's.
const CLIENT_WEAPONS: &[&str] = &[
    "crossbow",
    "crowbar",
    "egon",
    "gauss",
    "glock",
    "handgrenade",
    "hornetgun",
    "mp5",
    "python",
    "rpg",
    "satchel",
    "shotgun",
    "squeakgrenade",
    "tripmine",
];

/// One part: a library the engine loads, or the engine itself.
struct Part {
    name: &'static str,
    sources: Vec<PathBuf>,
    /// Include folders, in order, and defines.
    flags: Vec<String>,
    /// Libraries beside the C library to link it against for the check.
    cxx: bool,
}

/// The C and C++ sources in `dir` (and below it if `deep`), sorted, without
/// the names in `skip`.
fn sources(dir: &Path, deep: bool, skip: &[&str]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for p in entries.filter_map(|e| e.ok().map(|e| e.path())) {
        if p.is_dir() {
            if deep {
                out.extend(sources(&p, true, skip));
            }
            continue;
        }
        let name = p.file_name().unwrap_or_default().to_string_lossy();
        if p.extension().is_some_and(|x| x == "c" || x == "cpp") && !skip.contains(&name.as_ref()) {
            out.push(p);
        }
    }
    out.sort();
    out
}

fn includes(base: &Path, dirs: &[&str]) -> Vec<String> {
    dirs.iter()
        .flat_map(|d| {
            [
                "-I".to_string(),
                base.join(d).to_string_lossy().into_owned(),
            ]
        })
        .collect()
}

/// Checks that `dir` is the pinned commit and has `patch` applied (applies
/// it if not).
fn prepare(root: &Path, dir: &str, commit: &str, patch: &str) -> Result<(), String> {
    let at = root.join("target").join("ports-src").join(dir);
    let git = |args: &[&str]| -> Result<(bool, String), String> {
        let out = Command::new("git")
            .arg("-C")
            .arg(&at)
            .args(args)
            .output()
            .map_err(|e| format!("git: {e}"))?;
        Ok((
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).trim().to_string(),
        ))
    };
    let (ok, head) = git(&["rev-parse", "HEAD"])?;
    if !ok || !at.join(".git").exists() {
        return Err(format!(
            "{} is not there: the port's sources are fetched by hand (the owner's go, 2026-11-15): commit {commit}",
            at.display()
        ));
    }
    if head != commit {
        return Err(format!(
            "{} is at {head}, not the pinned {commit} (xtask/src/xash.rs)",
            at.display()
        ));
    }
    let patch = root.join("ports").join("xash").join("patches").join(patch);
    let patch = patch.to_string_lossy().into_owned();
    if git(&["apply", "--reverse", "--check", &patch])?.0 {
        return Ok(());
    }
    if !git(&["apply", "--check", &patch])?.0 || !git(&["apply", &patch])?.0 {
        return Err(format!(
            "{patch} does not apply to {} (changed by hand?)",
            at.display()
        ));
    }
    Ok(())
}

/// The parts of the port, with their sources and flags.
fn parts(root: &Path, low_memory: u8) -> Vec<Part> {
    let src = root.join("target").join("ports-src");
    let x = src.join("xash3d-fwgs");
    let sdk = src.join("hlsdk-portable");
    let ours = root.join("ports").join("xash");
    let defines = |d: &[&str]| d.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
    // Ours first: the sockets and threads headers, the stand-ins for the
    // libraries left out, and the port layer's own header.
    let mut common = includes(&ours, &["include"]);
    common.extend(includes(root, &["ports/include"]));
    common.extend(includes(
        &x,
        &[
            "common",
            "public",
            "pm_shared",
            "filesystem",
            "engine",
            "engine/common",
            "engine/common/imagelib",
            "3rdparty/library_suffix/include",
            "3rdparty/miniz",
        ],
    ));
    common.extend(defines(ENGINE_DEFINES));
    common.push(format!("-DXASH_LOW_MEMORY={low_memory}"));
    let public = || {
        let mut s = sources(&x.join("public"), false, &[]);
        s.extend(sources(&x.join("3rdparty/library_suffix/src"), false, &[]));
        s
    };

    let mut engine = Part {
        name: "engine",
        sources: Vec::new(),
        flags: includes(
            &x,
            &[
                "engine/server",
                "engine/client",
                "engine/client/vgui",
                "engine/common/soundlib",
                "engine/platform",
            ],
        ),
        cxx: false,
    };
    engine.flags.extend(common.iter().cloned());
    for d in [
        "engine/common",
        "engine/common/imagelib",
        "engine/common/soundlib",
        "engine/common/http",
        "engine/server",
    ] {
        engine.sources.extend(sources(&x.join(d), false, LEFT_OUT));
    }
    engine
        .sources
        .extend(sources(&x.join("engine/client"), true, LEFT_OUT));
    engine.sources.extend(public());
    engine.sources.push(x.join("3rdparty/miniz/miniz.c"));
    engine
        .sources
        .extend(sources(&ours.join("platform"), false, &[]));

    let mut filesystem = Part {
        name: "filesystem",
        sources: sources(&x.join("filesystem"), false, &[]),
        flags: common.clone(),
        // Its one C++ file is the interface other engines' games ask for.
        cxx: true,
    };
    filesystem.sources.extend(public());
    filesystem.sources.push(x.join("3rdparty/miniz/miniz.c"));

    // The renderer's own folders first: its `ref_common.h` is not the
    // engine's of the same name.
    let mut ref_soft = Part {
        name: "ref_soft",
        sources: sources(&x.join("ref/soft"), false, &[]),
        flags: includes(&x, &["ref/soft", "ref/common"]),
        cxx: false,
    };
    ref_soft.flags.push("-DREF_DLL=1".into());
    ref_soft.flags.extend(common.iter().cloned());
    ref_soft
        .sources
        .extend(sources(&x.join("ref/common"), false, &[]));
    ref_soft.sources.extend(public());

    let sdk_flags = |dirs: &[&str], more: &[&str]| {
        let mut f = includes(&sdk, dirs);
        f.extend(defines(SDK_DEFINES));
        f.extend(defines(more));
        f
    };
    let mut server = Part {
        name: "server",
        sources: sources(&sdk.join("dlls"), true, SERVER_LEFT_OUT),
        flags: sdk_flags(
            &[
                "dlls",
                "common",
                "engine",
                "pm_shared",
                "game_shared",
                "public",
            ],
            &[],
        ),
        cxx: true,
    };
    server
        .sources
        .extend(sources(&sdk.join("pm_shared"), false, &[]));
    server.sources.push(sdk.join("public/safe_snprintf.c"));
    server
        .sources
        .push(sdk.join("game_shared/voice_gamemgr.cpp"));
    server.sources.push(sdk.join("game_shared/vcs_info.c"));

    let mut client = Part {
        name: "client",
        sources: sources(&sdk.join("cl_dll"), true, CLIENT_LEFT_OUT),
        flags: sdk_flags(
            &[
                "cl_dll",
                "dlls",
                "common",
                "engine",
                "pm_shared",
                "game_shared",
                "public",
                "utils/vgui/include",
            ],
            &["-DCLIENT_DLL=1"],
        ),
        cxx: true,
    };
    client
        .sources
        .extend(sources(&sdk.join("game_shared"), false, SHARED_LEFT_OUT));
    client
        .sources
        .extend(sources(&sdk.join("pm_shared"), false, &[]));
    client.sources.push(sdk.join("public/safe_snprintf.c"));
    client.sources.extend(
        CLIENT_WEAPONS
            .iter()
            .map(|w| sdk.join("dlls").join(format!("{w}.cpp"))),
    );

    vec![engine, filesystem, ref_soft, server, client]
}

/// A part's sizes in bytes: code and constants, initialised data, zeroed
/// RAM (`llvm-size`'s three columns).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Sizes {
    text: u64,
    data: u64,
    bss: u64,
}

/// `llvm-size`'s output for one file.
fn parse_sizes(out: &str) -> Option<Sizes> {
    let mut n = out.lines().nth(1)?.split_whitespace();
    Some(Sizes {
        text: n.next()?.parse().ok()?,
        data: n.next()?.parse().ok()?,
        bss: n.next()?.parse().ok()?,
    })
}

/// The names the linker could not find (its `undefined symbol:` lines),
/// once each, sorted.
fn parse_undefined(stderr: &str) -> Vec<String> {
    let mut out: Vec<String> = stderr
        .lines()
        .filter_map(|l| l.split("undefined symbol: ").nth(1))
        .map(|s| s.trim().to_string())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Who answers a name the C library left undefined.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Answer {
    /// The image's own (`hal/cabi`'s C boundary, `rp-hal-mem`'s memory
    /// routines, the compiler's arithmetic helpers Rust brings).
    Image,
    /// Nothing yet: a call the port layer must grow.
    Missing,
    /// A library not built (FreeVGUI, for the client).
    Library,
}

/// `hal/cabi`'s names and the port layer's (`ports/include/retropane.h`).
const BOUNDARY: &[&str] = &[
    "write",
    "read",
    "open",
    "close",
    "lseek",
    "unlink",
    "sbrk",
    "raise",
    "_exit",
    "gettimeofday",
    "stat",
    "opendir",
    "readdir",
    "closedir",
    "mkdir",
    "rename",
    "dup",
    "fsync",
    "getcwd",
    "execve",
];

fn answer(name: &str) -> Answer {
    if name.starts_with("__aeabi_")
        || name.starts_with("rp_")
        || BOUNDARY.contains(&name)
        || ["memcpy", "memmove", "memset", "memcmp"].contains(&name)
    {
        Answer::Image
    } else if name.contains("vgui::") || name.starts_with("vtable for vgui") {
        Answer::Library
    } else {
        Answer::Missing
    }
}

/// What one part came to.
struct Built {
    name: &'static str,
    files: usize,
    sizes: Sizes,
    missing: Vec<String>,
    library: usize,
}

fn build(tc: &Toolchain, libc: &Libc, out: &Path, part: &Part) -> Result<Built, String> {
    let dir = out.join(part.name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut common = cc::flag_strings(cc::TARGET_FLAGS);
    // The engine's own code is not ours to make warning-free.
    common.push("-w".into());
    common.extend(libc.include_flags.iter().cloned());
    common.extend(part.flags.iter().cloned());
    let jobs: Vec<Job> = part
        .sources
        .iter()
        .enumerate()
        .map(|(i, f)| Job {
            source: f.clone(),
            object: dir.join(format!(
                "{i:03}-{}.o",
                f.file_name().unwrap_or_default().to_string_lossy()
            )),
            flags: if f.extension().is_some_and(|x| x == "cpp") {
                let mut cxx = cc::flag_strings(cc::CXX_FLAGS);
                cxx[0] = "-std=gnu++17".into();
                cxx
            } else {
                vec!["-std=gnu11".into()]
            },
        })
        .collect();
    cc::compile(tc, &common, &jobs).map_err(|e| format!("{}: {e}", part.name))?;
    // The part as one object: what a library of its own would hold.
    let whole = out.join(format!("{}.o", part.name));
    let list = out.join(format!("{}.rsp", part.name));
    let text: String = jobs
        .iter()
        .map(|j| format!("\"{}\"\n", j.object.to_string_lossy().replace('\\', "/")))
        .collect();
    std::fs::write(&list, text).map_err(|e| format!("{}: {e}", list.display()))?;
    crate::emu::run(
        Command::new(tc.tool("ld.lld"))
            .arg("-r")
            .arg("-o")
            .arg(&whole)
            .arg(format!("@{}", list.to_string_lossy().replace('\\', "/"))),
    )?;
    let size = Command::new(tc.tool("llvm-size"))
        .arg(&whole)
        .output()
        .map_err(|e| format!("llvm-size: {e}"))?;
    let sizes = parse_sizes(&String::from_utf8_lossy(&size.stdout))
        .ok_or_else(|| format!("{}: llvm-size's output was not read", part.name))?;
    // Linked against the C library (and the C++ runtime), to find what
    // neither answers. The result is not a program: only the names matter.
    let mut link = Command::new(tc.tool("ld.lld"));
    link.arg("-o")
        .arg(out.join(format!("{}.elf", part.name)))
        .arg(&whole);
    if part.cxx {
        link.arg(libc.lib_dir.join("librpcxx.a"));
    }
    link.arg(libc.lib_dir.join("libc.a")).args([
        "--entry=0",
        "--warn-unresolved-symbols",
        "--demangle",
    ]);
    let linked = link.output().map_err(|e| format!("ld.lld: {e}"))?;
    let undefined = parse_undefined(&String::from_utf8_lossy(&linked.stderr));
    let missing: Vec<String> = undefined
        .iter()
        .filter(|n| answer(n) == Answer::Missing)
        .cloned()
        .collect();
    let library = undefined
        .iter()
        .filter(|n| answer(n) == Answer::Library)
        .count();
    Ok(Built {
        name: part.name,
        files: jobs.len(),
        sizes,
        missing,
        library,
    })
}

/// `cargo xtask xash-eval`: builds every part and says what it found. The
/// same lines go to `target/xash-eval/report.txt`.
///
/// # Errors
/// A source folder is missing or not the pinned commit, a patch does not
/// apply, or a file does not compile.
pub fn eval(root: &Path) -> Result<(), String> {
    for (dir, commit, patch) in SOURCES {
        prepare(root, dir, commit, patch)?;
    }
    let (tc, libc) = cc::ready(root)?;
    cc::program(root, &tc, &libc, "rpcxx", &root.join("ports/libc"))?;
    let out = root.join("target").join("xash-eval");
    let mut report = String::new();
    let mut say = |line: String| {
        eprintln!("    {line}");
        report.push_str(&line);
        report.push('\n');
    };
    let kib = |b: u64| b.div_ceil(1024);
    let mut all_missing: Vec<String> = Vec::new();
    for part in parts(root, 0) {
        let b = build(&tc, &libc, &out, &part)?;
        say(format!(
            "{}: {} files compiled; {} KiB of code and constants, {} KiB of data, {} KiB of zeroed RAM",
            b.name,
            b.files,
            kib(b.sizes.text),
            kib(b.sizes.data),
            kib(b.sizes.bss)
        ));
        if b.library > 0 {
            say(format!(
                "{}: {} names of FreeVGUI's (the SDK's submodule, not fetched) are not answered",
                b.name, b.library
            ));
        }
        if !b.missing.is_empty() {
            say(format!(
                "{}: not answered by the C library or the port layer: {}",
                b.name,
                b.missing.join(" ")
            ));
        }
        all_missing.extend(b.missing);
    }
    // The engine's own low-memory builds: how much static RAM each gives
    // back (mode 2 gives up network compatibility, which the port has not).
    for mode in [1, 2] {
        let Some(engine) = parts(root, mode).into_iter().next() else {
            continue;
        };
        let b = build(&tc, &libc, &out, &engine)?;
        say(format!(
            "engine with XASH_LOW_MEMORY={mode}: {} KiB of code and constants, {} KiB of zeroed RAM",
            kib(b.sizes.text),
            kib(b.sizes.bss)
        ));
    }
    all_missing.sort();
    all_missing.dedup();
    say(format!(
        "{} calls for the port layer to grow: {}",
        all_missing.len(),
        all_missing.join(" ")
    ));
    let file = out.join("report.txt");
    std::fs::write(&file, report).map_err(|e| format!("{}: {e}", file.display()))?;
    eprintln!("    written to {}", file.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_size_tool_s_three_columns_are_read() {
        let out = "   text\t   data\t    bss\t    dec\t    hex\tfilename\n1127851\t  19880\t13261156\t14408887\t dbdcb7\tengine.o\n";
        assert_eq!(
            parse_sizes(out),
            Some(Sizes {
                text: 1_127_851,
                data: 19_880,
                bss: 13_261_156
            })
        );
        // No second line, or one that is not numbers: not read as zero.
        assert_eq!(parse_sizes("   text\t   data\n"), None);
        assert_eq!(parse_sizes("header\nerror: no such file\n"), None);
        assert_eq!(parse_sizes(""), None);
    }

    #[test]
    fn the_linker_s_undefined_names_are_listed_once_each_and_sorted() {
        let err = "ld.lld: warning: undefined symbol: stat\n>>> referenced by sys.c\n>>>               012-sys.c.o:(FS_SysFileTime)\n\nld.lld: warning: undefined symbol: opendir\n>>> referenced by sys.c\nld.lld: warning: undefined symbol: stat\nld.lld: warning: undefined symbol: vgui::Panel::setSize(int, int)\n";
        assert_eq!(
            parse_undefined(err),
            ["opendir", "stat", "vgui::Panel::setSize(int, int)"]
        );
        assert!(parse_undefined("ld.lld: error: cannot open x.o\n").is_empty());
    }

    #[test]
    fn each_undefined_name_goes_to_who_answers_it() {
        // The image's own: the C boundary, the port layer, the memory
        // routines, the compiler's arithmetic.
        for n in [
            "write",
            "sbrk",
            "_exit",
            "gettimeofday",
            "rp_time_us",
            "rp_snd_write",
            "memcpy",
            "__aeabi_dadd",
            "__aeabi_uidivmod",
        ] {
            assert_eq!(answer(n), Answer::Image, "{n}");
        }
        // Calls nothing answers yet: the list the evaluation is after.
        for n in ["fstat", "chdir", "writev", "memchr2", "statvfs", "rmdir"] {
            assert_eq!(answer(n), Answer::Missing, "{n}");
        }
        // The file calls the first run found missing, now the boundary's.
        for n in [
            "stat", "opendir", "readdir", "closedir", "mkdir", "rename", "fsync", "dup", "getcwd",
            "execve",
        ] {
            assert_eq!(answer(n), Answer::Image, "{n}");
        }
        // The client's missing library, counted apart.
        for n in ["vgui::Panel::setSize(int, int)", "vtable for vgui::Label"] {
            assert_eq!(answer(n), Answer::Library, "{n}");
        }
        // A game class's own table is not FreeVGUI's.
        assert_eq!(answer("vtable for CCrowbar"), Answer::Missing);
    }

    #[test]
    fn sources_are_listed_sorted_without_what_is_left_out() {
        let dir = std::env::temp_dir().join(format!("rp-xash-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).expect("folder");
        for f in ["b.c", "a.cpp", "voice.c", "notes.txt", "c.h", "sub/d.c"] {
            std::fs::write(dir.join(f), "").expect("file");
        }
        let names = |deep: bool| -> Vec<String> {
            sources(&dir, deep, &["voice.c"])
                .iter()
                .map(|p| {
                    p.file_name()
                        .expect("a name")
                        .to_string_lossy()
                        .into_owned()
                })
                .collect()
        };
        assert_eq!(names(false), ["a.cpp", "b.c"]);
        assert_eq!(names(true), ["a.cpp", "b.c", "d.c"]);
        // A folder that is not there gives nothing, not an error: the
        // check that the source is there is `prepare`'s.
        assert!(sources(&dir.join("nowhere"), true, &[]).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
