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
//! engine's source as fetched but for four small patches kept here
//! (`ports/xash/patches`: the new platform's name, and three places that
//! assumed POSIX), lists the sources in this file, and builds them with the
//! same compiler, flags and C library as every C program of RetroPane, in
//! parallel, the same on every PC. The game's logic is built as libraries
//! of its own and never linked into the engine (Valve's SDK terms and the
//! GPL do not combine: docs/research/half-life-licences.md).
//!
//! The sources are not fetched here (they are the owner's go, given
//! 2026-11-15): `target/ports-src/xash3d-fwgs` and `hlsdk-portable` at the
//! pinned commits, with the engine's `library_suffix`, `mainui` (the menu,
//! with its `miniutl`) and `mpg123` submodules and the SDK's `freevgui`.
//!
//! The MP3 decoder (the Steam copy's music) is mpg123 as its own
//! `arm_nofpu` build has it, not as the engine's build has it: the engine
//! builds the generic float decoder (`REAL_IS_FLOAT`), which on a CPU with
//! no FPU is a helper call for every multiply of every sample; ours is the
//! fixed-point decoder (`REAL_IS_FIXED`) with mpg123's ARM multiply and
//! synthesis (`OPT_ARM`, `synth_arm.S`: one `smull` a multiply) and its
//! tables computed beforehand, so decoding touches no float at all.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cc::{self, Job, Libc, Toolchain};
use crate::module::{self, parse_undefined};

/// The pinned sources: (folder under `target/ports-src`, commit, patch; no
/// patch where the source builds as fetched).
const SOURCES: [(&str, &str, &str); 7] = [
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
    (
        "hlsdk-portable/freevgui",
        "73bfb4659f3d9eb28b79e5b26baea8c6b888d5eb",
        "",
    ),
    (
        "xash3d-fwgs/3rdparty/mainui",
        "8981066ab977758653751e54e9083f289c0c3503",
        "mainui.patch",
    ),
    (
        "xash3d-fwgs/3rdparty/mainui/miniutl",
        "048a416f4c54c501dfd728fd792bfdc9f2883f51",
        "",
    ),
    (
        "xash3d-fwgs/3rdparty/mpg123/mpg123",
        "20c59b07437ea6908a5b7bcb9dc43cb2833c0dcd",
        "",
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
    // The engine's last words on a fatal error are ours to show
    // (sys_rp.c: the serial line today, the screen when the port has one).
    "-DXASH_MESSAGEBOX=100",
    // The libraries are asked for by their plain names ("server",
    // "menu"): the loader knows where each is (lib_rp.c, ADR-0039).
    "-DXASH_INTERNAL_GAMELIBS=1",
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

/// mpg123's sources for Layer III at the file's own rate, 16-bit out (the
/// engine's own choice of what to build, `3rdparty/mpg123/wscript`), with
/// the ARM synthesis its `arm_nofpu` build adds.
const MPG123_SOURCES: &[&str] = &[
    "mpg123/src/compat/compat_str.c",
    "mpg123/src/libmpg123/dct64.c",
    "mpg123/src/libmpg123/equalizer.c",
    "mpg123/src/libmpg123/feature.c",
    "mpg123/src/libmpg123/format.c",
    "mpg123/src/libmpg123/frame.c",
    "mpg123/src/libmpg123/id3.c",
    "mpg123/src/libmpg123/index.c",
    "mpg123/src/libmpg123/layer3.c",
    "mpg123/src/libmpg123/libmpg123.c",
    "mpg123/src/libmpg123/optimize.c",
    "mpg123/src/libmpg123/parse.c",
    "mpg123/src/libmpg123/readers.c",
    "mpg123/src/libmpg123/synth.c",
    "mpg123/src/libmpg123/tabinit.c",
    "mpg123/src/libmpg123/synth_arm.S",
    "stub.c",
];

/// The menu's and FreeVGUI's sources that are another system's.
const MENU_LEFT_OUT: &[&str] = &["FreeTypeFont.cpp", "WinAPIFont.cpp"];

/// One part: a library the engine loads, the engine itself, or a library
/// built to be linked inside one of those (`inside`).
struct Part {
    name: &'static str,
    sources: Vec<PathBuf>,
    /// Include folders, in order, and defines.
    flags: Vec<String>,
    /// Libraries beside the C library to link it against for the check.
    cxx: bool,
    /// Built with run-time type information: FreeVGUI asks what kind of
    /// panel it holds (`dynamic_cast`), so it and the client library whose
    /// panels it holds are, as the SDK's own Waf build has them; the rest
    /// are not (ports/libc/cxxrt.cpp).
    rtti: bool,
    /// Earlier parts linked into this one (mpg123 into the engine,
    /// FreeVGUI into the client library: BSD, its own static option).
    with: &'static [&'static str],
    /// Compiled with `-fvisibility=hidden`, as the part's own build does:
    /// what it exports is then exactly what its source marks for export,
    /// which is how the module packer finds a library's names (every part
    /// that is a library, or is linked into one).
    hidden: bool,
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
    if patch.is_empty() {
        return Ok(());
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
        rtti: false,
        with: &["mpg123"],
        hidden: false,
    };
    engine
        .flags
        .extend(includes(&x, &["3rdparty/mpg123/mpg123/src/include"]));
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
        rtti: false,
        with: &[],
        hidden: true,
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
        rtti: false,
        with: &[],
        hidden: true,
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
        rtti: false,
        with: &[],
        hidden: true,
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
            // FreeVGUI inside the library, as the SDK's own static option.
            &["-DCLIENT_DLL=1", "-DINTERNAL_VGUI_SUPPORT=1"],
        ),
        cxx: true,
        rtti: true,
        with: &["freevgui"],
        hidden: true,
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

    // The MP3 decoder, fixed point (the module's notes): its settings are
    // ours (`ports/xash/mpg123/config.h`), its sources as fetched.
    let m = x.join("3rdparty/mpg123");
    let mut mpg123 = Part {
        name: "mpg123",
        sources: MPG123_SOURCES.iter().map(|f| m.join(f)).collect(),
        flags: includes(&ours, &["mpg123"]),
        cxx: false,
        rtti: false,
        with: &[],
        hidden: false,
    };
    mpg123.flags.extend(includes(
        &m,
        &["mpg123/src", "mpg123/src/include", "mpg123/src/libmpg123"],
    ));
    mpg123
        .flags
        .extend(defines(&["-DOPT_ARM=1", "-DREAL_IS_FIXED=1"]));

    // FreeVGUI: the panels the client library is written against, with its
    // glue to this engine.
    let v = sdk.join("freevgui");
    let mut freevgui = Part {
        name: "freevgui",
        sources: sources(&v, false, &[]),
        flags: includes(&v, &["."]),
        cxx: true,
        rtti: true,
        with: &[],
        hidden: true,
    };
    // Its platform folder as its own build takes it for a system that is
    // not Windows (`platform/posix`: the clock through `clock_gettime`,
    // no system fonts), as fetched: nothing of RetroPane's is compiled
    // into the client library (the SDK's licence).
    for d in [
        "controls",
        "platform/common",
        "platform/posix",
        "platform/xash3d-fwgs",
    ] {
        freevgui.sources.extend(sources(&v.join(d), false, &[]));
    }
    // The engine's side of the glue is the SDK's copy of its headers.
    freevgui.flags.extend(includes(&sdk, &["engine", "common"]));
    freevgui.flags.extend(defines(SDK_DEFINES));
    freevgui.flags.push("-DINTERNAL_VGUI_SUPPORT=1".into());

    // The menu, with its own text drawing from the game's bitmap fonts
    // and stb_truetype (no FreeType: another library, another download).
    let u = x.join("3rdparty/mainui");
    let mut menu = Part {
        name: "menu",
        sources: sources(&u, false, &[]),
        flags: includes(
            &u,
            &[
                ".",
                "miniutl",
                "font",
                "controls",
                "menus",
                "model",
                "sdk_includes/common",
                "sdk_includes/engine",
                "sdk_includes/public",
                "sdk_includes/pm_shared",
            ],
        ),
        cxx: true,
        rtti: false,
        with: &[],
        hidden: true,
    };
    for d in [
        "miniutl",
        "font",
        "menus",
        "menus/dynamic",
        "model",
        "controls",
    ] {
        menu.sources
            .extend(sources(&u.join(d), false, MENU_LEFT_OUT));
    }
    menu.flags.extend(defines(&[
        "-D__retropane__=1",
        "-DSTDINT_H=<cstdint>",
        "-DMAINUI_USE_CUSTOM_FONT_RENDER=1",
        "-DMAINUI_USE_STB=1",
        "-D_GNU_SOURCE=1",
    ]));

    vec![
        mpg123, engine, filesystem, ref_soft, menu, server, freevgui, client,
    ]
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
    "clock_gettime",
    "chdir",
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

/// Compiles `part` and links its objects (and the parts inside it) into
/// one object, `<out>/<name>.o`: what a library of its own would hold.
/// The object and how many files went into it.
fn compile(
    root: &Path,
    tc: &Toolchain,
    libc: &Libc,
    out: &Path,
    part: &Part,
) -> Result<(PathBuf, usize), String> {
    let cxx_headers = root.join("ports").join("libc").join("cxx");
    let dir = out.join(part.name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut common = cc::flag_strings(cc::TARGET_FLAGS);
    // The engine's own code is not ours to make warning-free.
    common.push("-w".into());
    // The engine names its source files in its messages (`__FILE__`):
    // from the source's folder, not this PC's (a module carried the
    // builder's user folder until 2026-11-15), and the same on every PC.
    for (dir, to) in [
        (root.join("target").join("ports-src"), ""),
        (root.to_path_buf(), ""),
    ] {
        common.push(format!(
            "-ffile-prefix-map={}{}={to}",
            dir.to_string_lossy(),
            std::path::MAIN_SEPARATOR
        ));
    }
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
                // The few C++ headers a port may ask for (ports/libc/cxx).
                cxx.push("-I".into());
                cxx.push(cxx_headers.to_string_lossy().into_owned());
                if part.rtti {
                    cxx.push("-frtti".into());
                }
                if part.hidden {
                    cxx.push("-fvisibility=hidden".into());
                }
                cxx
            } else if f.extension().is_some_and(|x| x == "S") {
                Vec::new()
            } else if part.hidden {
                vec!["-std=gnu11".into(), "-fvisibility=hidden".into()]
            } else {
                vec!["-std=gnu11".into()]
            },
        })
        .collect();
    cc::compile(tc, &common, &jobs).map_err(|e| format!("{}: {e}", part.name))?;
    // The part as one object: what a library of its own would hold.
    let whole = out.join(format!("{}.o", part.name));
    let list = out.join(format!("{}.rsp", part.name));
    let mut text: String = jobs
        .iter()
        .map(|j| format!("\"{}\"\n", j.object.to_string_lossy().replace('\\', "/")))
        .collect();
    // The parts linked inside this one, as built before it.
    for inner in part.with {
        let o = out.join(format!("{inner}.o"));
        text.push_str(&format!("\"{}\"\n", o.to_string_lossy().replace('\\', "/")));
    }
    std::fs::write(&list, text).map_err(|e| format!("{}: {e}", list.display()))?;
    crate::emu::run(
        Command::new(tc.tool("ld.lld"))
            .arg("-r")
            .arg("-o")
            .arg(&whole)
            .arg(format!("@{}", list.to_string_lossy().replace('\\', "/"))),
    )?;
    Ok((whole, jobs.len()))
}

fn build(
    root: &Path,
    tc: &Toolchain,
    libc: &Libc,
    out: &Path,
    part: &Part,
) -> Result<Built, String> {
    let (whole, files) = compile(root, tc, libc, out, part)?;
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
    // A link that stops for another reason (a name defined twice) has not
    // looked for every undefined name: its count would be too low, and
    // was, for the client library, until 2026-11-15.
    if !linked.status.success() {
        return Err(format!(
            "{}: the check link failed:\n{}",
            part.name,
            String::from_utf8_lossy(&linked.stderr)
        ));
    }
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
        files,
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
        let b = build(root, &tc, &libc, &out, &part)?;
        let inside = if part.with.is_empty() {
            String::new()
        } else {
            format!(" (with {} inside)", part.with.join(", "))
        };
        say(format!(
            "{}{inside}: {} files compiled; {} KiB of code and constants, {} KiB of data, {} KiB of zeroed RAM",
            b.name,
            b.files,
            kib(b.sizes.text),
            kib(b.sizes.data),
            kib(b.sizes.bss)
        ));
        if b.library > 0 {
            say(format!(
                "{}: {} names of FreeVGUI's are not answered (it is linked into the client library)",
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
        let Some(engine) = parts(root, mode).into_iter().find(|p| p.name == "engine") else {
            continue;
        };
        let b = build(root, &tc, &libc, &out, &engine)?;
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

/// One module of the port (ADR-0039 rule 5: what is linked together
/// follows the licence line, and nothing else splits).
struct Plan {
    /// The module's name: its file is `<name>.rpm`, and the engine asks
    /// for a library by this name.
    name: &'static str,
    /// The part that is the module's own.
    part: &'static str,
    /// The parts linked inside it, each with the name the engine asks for
    /// it by (`lib_rp.c` answers those loads from a table in the module).
    inner: &'static [(&'static str, &'static str)],
    /// Its entry, if it is the program and not a library.
    entry: Option<&'static str>,
}

/// The three modules, in the order they are placed from the RAM pool's
/// start. The engine is last: its zeroed RAM (21 MiB with the renderer's)
/// would otherwise sit between the runner and the other modules' code,
/// out of a plain branch's reach (32 MiB), and every call into the C
/// library from them would need a veneer.
const PLAN: [Plan; 3] = [
    Plan {
        name: "server",
        part: "server",
        inner: &[],
        entry: None,
    },
    Plan {
        name: "client",
        part: "client",
        inner: &[],
        entry: None,
    },
    Plan {
        name: "engine",
        part: "engine",
        inner: &[
            ("filesystem", "filesystem_stdio"),
            ("ref_soft", "ref_soft"),
            ("menu", "menu"),
        ],
        entry: Some("rp_module_main"),
    },
];

/// The assembly of the table `lib_rp.c` answers the engine's loads of its
/// own parts from (`rp_inner_libraries`): for each part its name as the
/// engine asks for it, then each function its source marks for export, by
/// that name, at the symbol the build renamed it to (`prefix` + name).
fn inner_asm(libs: &[(&str, String, Vec<String>)]) -> String {
    let mut s = String::from(
        "\t.section .rodata.rp_inner_libraries,\"a\",%progbits\n\t.balign 4\n\t.globl rp_inner_libraries\nrp_inner_libraries:\n",
    );
    for (i, (_, _, names)) in libs.iter().enumerate() {
        s.push_str(&format!(
            "\t.long .Llib{i}, .Lexports{i}, {}\n",
            names.len()
        ));
    }
    s.push_str("\t.long 0, 0, 0\n");
    for (i, (_, prefix, names)) in libs.iter().enumerate() {
        s.push_str(&format!(".Lexports{i}:\n"));
        for (k, name) in names.iter().enumerate() {
            s.push_str(&format!("\t.long .Lname{i}_{k}, \"{prefix}{name}\"\n"));
        }
    }
    for (i, (asked, _, names)) in libs.iter().enumerate() {
        s.push_str(&format!(".Llib{i}:\n\t.asciz \"{asked}\"\n"));
        for (k, name) in names.iter().enumerate() {
            s.push_str(&format!(".Lname{i}_{k}:\n\t.asciz \"{name}\"\n"));
        }
    }
    s
}

/// The port's modules, built.
pub struct Set {
    /// Each module's file name and bytes, in the order they are placed.
    pub files: Vec<(String, Vec<u8>)>,
    /// What the build found, in plain lines (sizes, counts, addresses).
    pub lines: Vec<String>,
}

/// Builds the port's three modules (ADR-0039) against the runner:
/// compiles every part, finds what each module asks of the runner, has
/// `build_runner` build the runner keeping exactly that (it is given the
/// folder of the C libraries, the runner's table among them, and returns
/// the runner's linked image), then links each module at its address and
/// packs it. `arithmetic` is the libraries linked into every module (rule
/// 6: the compiler's arithmetic helpers and the memory routines).
///
/// # Errors
/// As [`eval`]; a name the runner has not; a link that is not a module.
pub fn modules(
    root: &Path,
    arithmetic: &[PathBuf],
    build_runner: &dyn Fn(&Path) -> Result<PathBuf, String>,
) -> Result<Set, String> {
    // An experiment's switch, not a setting of the port: the engine's own
    // low-memory builds (1 or 2), to measure what each gives and costs
    // (docs/reminders.md R277). The port's own build is 0.
    let low_memory = std::env::var("RETROPANE_XASH_LOW_MEMORY")
        .ok()
        .and_then(|v| v.parse::<u8>().ok())
        .filter(|m| *m <= 2)
        .unwrap_or(0);
    modules_as(root, arithmetic, build_runner, low_memory)
}

/// [`modules`] with the engine's low-memory build given (0: the port's
/// own build; 1 or 2: an experiment's), whatever the environment says.
///
/// # Errors
/// As [`modules`].
pub fn modules_as(
    root: &Path,
    arithmetic: &[PathBuf],
    build_runner: &dyn Fn(&Path) -> Result<PathBuf, String>,
    low_memory: u8,
) -> Result<Set, String> {
    for (dir, commit, patch) in SOURCES {
        prepare(root, dir, commit, patch)?;
    }
    let (tc, libc) = cc::ready(root)?;
    cc::program(root, &tc, &libc, "rpcxx", &root.join("ports/libc"))?;
    let out = root.join("target").join("xash-eval");
    let dir = root.join("target").join("xash-modules");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    if low_memory != 0 {
        eprintln!("    an experiment: the engine built with XASH_LOW_MEMORY={low_memory}");
    }
    let mut files = 0;
    for part in parts(root, low_memory) {
        files += compile(root, &tc, &libc, &out, &part)?.1;
    }
    let object = |part: &str| out.join(format!("{part}.o"));
    let mut specs = Vec::new();
    for plan in &PLAN {
        let mut spec = module::Spec {
            name: plan.name,
            objects: Vec::new(),
            archives: vec![libc.lib_dir.join("librpcxx.a")],
            entry: plan.entry,
            exports: Vec::new(),
        };
        spec.archives.extend(arithmetic.iter().cloned());
        if plan.entry.is_some() {
            // The program: its own part as compiled.
            spec.objects.push(object(plan.part));
        } else {
            let local = dir.join(format!("{}-local.o", plan.part));
            spec.exports = module::localised(&tc, &object(plan.part), &local, None)?;
            spec.objects.push(local);
        }
        let mut libs = Vec::new();
        for (part, asked) in plan.inner {
            let prefix = format!("rp_{part}_");
            let inner = dir.join(format!("{part}-inner.o"));
            let names = module::localised(&tc, &object(part), &inner, Some(&prefix))?;
            if names.is_empty() {
                return Err(format!(
                    "{part}: its source marks no function for export: the engine could ask it for nothing"
                ));
            }
            spec.objects.push(inner);
            libs.push((*asked, prefix, names));
        }
        if !libs.is_empty() {
            spec.objects.push(module::assemble(
                &tc,
                &dir,
                &format!("{}-inner", plan.name),
                &inner_asm(&libs),
            )?);
        }
        specs.push(spec);
    }
    // What each module asks of the runner, and the runner built to keep
    // exactly that.
    let mut wanted = Vec::new();
    for spec in &specs {
        wanted.push(module::imports(&tc, spec, &dir)?);
    }
    let mut all: Vec<String> = wanted.iter().flatten().cloned().collect();
    all.sort();
    all.dedup();
    module::runner_table(&tc, &libc.lib_dir, &all)?;
    let runner = module::runner_symbols(&tc, &build_runner(&libc.lib_dir)?)?;
    let identity = rp_port::module::runner_identity(&module::runner_table_bytes(&all, &runner)?);
    let mut base = module::POOL;
    let mut linked = Vec::new();
    for (spec, wanted) in specs.iter().zip(&wanted) {
        let l = module::link(&tc, spec, &dir, base, wanted, &runner)?;
        base = module::next_base(l.end);
        linked.push(l);
    }
    let set_end = base;
    let mut set = Set {
        files: Vec::new(),
        lines: vec![format!(
            "{files} files compiled; the runner keeps {} names for the modules (its identity {identity:016x})",
            all.len()
        )],
    };
    let kib = |b: u32| b.div_ceil(1024);
    for ((l, spec), wanted) in linked.iter().zip(&specs).zip(&wanted) {
        let file = module::seal(l, identity, set_end)?;
        let name = format!("{}.{}", l.name, module::EXTENSION);
        let path = dir.join(&name);
        std::fs::write(&path, &file).map_err(|e| format!("{}: {e}", path.display()))?;
        set.lines.push(format!(
            "{name}: {} KiB of file at 0x{:08X}, {} KiB of zeroed RAM after it; {} names exported, {} imported from the runner",
            kib(file.len() as u32),
            l.base,
            kib(l.end - l.base - file.len() as u32),
            spec.exports.len(),
            wanted.len()
        ));
        set.files.push((name, file));
    }
    set.lines.push(format!(
        "the set ends at 0x{set_end:08X}: {} KiB of the pool's {} are the modules', {} KiB are left for the game's heap",
        kib(set_end - module::POOL),
        kib(module::POOL_END - module::POOL),
        kib(module::POOL_END - set_end)
    ));
    Ok(set)
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
        for n in ["fstat", "fchdir", "writev", "memchr2", "statvfs", "rmdir"] {
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
    fn a_part_linked_inside_another_is_built_before_it() {
        // The lists alone (no sources fetched): the order and the flags.
        let all = parts(Path::new("nowhere"), 0);
        let names: Vec<&str> = all.iter().map(|p| p.name).collect();
        let mut once = names.clone();
        once.sort_unstable();
        once.dedup();
        assert_eq!(once.len(), names.len(), "a name twice: {names:?}");
        for (i, p) in all.iter().enumerate() {
            for inner in p.with {
                assert!(
                    names[..i].contains(inner),
                    "{} holds {inner}, which is not built before it",
                    p.name
                );
            }
        }
        // The game's logic never holds the engine's parts, nor the engine
        // the game's (docs/research/half-life-licences.md).
        let with = |n: &str| all.iter().find(|p| p.name == n).map(|p| p.with);
        assert_eq!(with("engine"), Some(&["mpg123"][..]));
        assert_eq!(with("client"), Some(&["freevgui"][..]));
        assert_eq!(with("server"), Some(&[][..]));
        // Run-time type information: FreeVGUI and the library whose panels
        // it holds, nothing else.
        let rtti: Vec<&str> = all.iter().filter(|p| p.rtti).map(|p| p.name).collect();
        assert_eq!(rtti, ["freevgui", "client"]);
        // The MP3 decoder is the fixed-point one with the ARM synthesis.
        let mpg = all.iter().find(|p| p.name == "mpg123").expect("mpg123");
        assert!(mpg.flags.iter().any(|f| f == "-DREAL_IS_FIXED=1"));
        assert!(mpg.flags.iter().any(|f| f == "-DOPT_ARM=1"));
        assert!(!mpg.flags.iter().any(|f| f.contains("REAL_IS_FLOAT")));
        assert!(mpg.sources.iter().any(|f| f.ends_with("synth_arm.S")));
        // Every pinned source is a whole commit; no patch is a plain "".
        for (dir, commit, patch) in SOURCES {
            assert_eq!(commit.len(), 40, "{dir}");
            assert!(patch.is_empty() || patch.ends_with(".patch"), "{dir}");
        }
    }

    #[test]
    fn the_modules_follow_the_licence_line_and_the_engine_is_placed_last() {
        let all = parts(Path::new("nowhere"), 0);
        let part = |n: &str| all.iter().find(|p| p.name == n);
        // Three modules: the game's two libraries alone, then the program
        // with the engine's own parts inside (ADR-0039 rule 5).
        assert_eq!(PLAN.map(|p| p.name), ["server", "client", "engine"]);
        assert_eq!(PLAN.map(|p| p.inner.len()), [0, 0, 3]);
        assert_eq!(PLAN.map(|p| p.entry.is_some()), [false, false, true]);
        // Every part is in exactly one module, on its own or inside
        // another part that is.
        let mut used: Vec<&str> = Vec::new();
        for plan in &PLAN {
            used.push(plan.part);
            used.extend(plan.inner.iter().map(|i| i.0));
        }
        for name in used.clone() {
            let p = part(name).unwrap_or_else(|| panic!("{name} is not a part"));
            used.extend(p.with);
        }
        let mut names: Vec<&str> = all.iter().map(|p| p.name).collect();
        names.sort_unstable();
        used.sort_unstable();
        assert_eq!(used, names);
        // The SDK's parts never share a module with the engine's.
        let sdk = ["server", "client", "freevgui"];
        for plan in &PLAN {
            let mut inside = vec![plan.part];
            inside.extend(plan.inner.iter().map(|i| i.0));
            let of_sdk = inside.iter().filter(|n| sdk.contains(n)).count();
            assert!(of_sdk == 0 || of_sdk == inside.len(), "{}", plan.name);
        }
        // A part whose exports the packer reads is compiled hidden, and so
        // is what is linked inside it; the program's own part is not.
        for plan in &PLAN {
            for (name, _) in plan.inner {
                assert!(part(name).is_some_and(|p| p.hidden), "{name}");
            }
            let own = part(plan.part).expect("a part");
            assert_eq!(own.hidden, plan.entry.is_none(), "{}", plan.part);
            for inner in own.with {
                assert_eq!(part(inner).map(|p| p.hidden), Some(own.hidden), "{inner}");
            }
        }
        // The engine asks for its libraries by their plain names and has
        // no main of its own: a module's entry is the runner's to call.
        assert!(ENGINE_DEFINES.contains(&"-DXASH_INTERNAL_GAMELIBS=1"));
        assert!(
            !ENGINE_DEFINES
                .iter()
                .any(|d| d.contains("XASH_ENABLE_MAIN"))
        );
        // FreeVGUI's platform files are its own (BSD), as fetched: nothing
        // of RetroPane's is compiled into the client library.
        let vgui = part("freevgui").expect("freevgui");
        assert!(
            vgui.flags
                .iter()
                .all(|f| !f.contains("ports/xash") && !f.contains("ports\\xash"))
        );
    }

    #[test]
    fn the_inner_table_names_each_part_as_the_engine_asks_for_it() {
        let s = inner_asm(&[
            (
                "filesystem_stdio",
                "rp_filesystem_".into(),
                vec!["CreateInterface".into(), "GetFSAPI".into()],
            ),
            ("menu", "rp_menu_".into(), vec!["GetMenuAPI".into()]),
        ]);
        // The table lib_rp.c walks: a name, its exports, how many; a null
        // name ends it.
        assert!(s.contains(
            "rp_inner_libraries:\n\t.long .Llib0, .Lexports0, 2\n\t.long .Llib1, .Lexports1, 1\n\t.long 0, 0, 0\n"
        ));
        // Each export by its name in the source, at its renamed symbol.
        assert!(s.contains(".Lexports0:\n\t.long .Lname0_0, \"rp_filesystem_CreateInterface\"\n\t.long .Lname0_1, \"rp_filesystem_GetFSAPI\"\n"));
        assert!(s.contains(".Lexports1:\n\t.long .Lname1_0, \"rp_menu_GetMenuAPI\"\n"));
        assert!(s.contains(
            ".Llib0:\n\t.asciz \"filesystem_stdio\"\n.Lname0_0:\n\t.asciz \"CreateInterface\"\n"
        ));
        assert!(s.ends_with(".Llib1:\n\t.asciz \"menu\"\n.Lname1_0:\n\t.asciz \"GetMenuAPI\"\n"));
        // No parts: only the end mark.
        assert!(inner_asm(&[]).ends_with("rp_inner_libraries:\n\t.long 0, 0, 0\n"));
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
