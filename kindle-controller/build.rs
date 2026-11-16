//! Builds libfbink (vendor/FBInk, a git submodule) plus our small C shim with `zig cc`, for
//! the Kindle target only. Host builds (`cargo test`, `cargo check`) skip it and don't need zig.
//!
//! zig is found through the `ZIG` environment variable, or `zig` on the PATH. rust-lld still
//! does the final link; zig only compiles the C files and makes the static archive.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

const KINDLE_TARGET: &str = "armv7-unknown-linux-musleabihf";
/// zig release series we build with. zig is pre-1.0 and its CLI still changes between
/// minor releases, so other series are refused rather than half-working.
const ZIG_SERIES: &str = "0.17.";
/// The FBInk release the submodule is pinned to (shown in its version string).
const FBINK_VERSION: &str = "v1.25.0";

const FBINK_SOURCES: &[&str] = &["fbink.c", "cutef8/utf8.c", "cutef8/dfa.c"];

fn main() {
    println!("cargo::rustc-check-cfg=cfg(fbink)");
    println!("cargo::rerun-if-env-changed=ZIG");
    println!("cargo::rerun-if-changed=csrc");
    println!("cargo::rerun-if-changed=vendor/FBInk");

    if env::var("TARGET").unwrap() != KINDLE_TARGET {
        return;
    }

    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let fbink = manifest.join("vendor/FBInk");
    if !fbink.join("fbink.h").is_file() {
        fail(
            "vendor/FBInk is empty. Fetch the submodule with\n    \
             git submodule update --init kindle-controller/vendor/FBInk",
        );
    }

    let zig = env::var_os("ZIG").map(PathBuf::from).unwrap_or_else(|| "zig".into());
    check_zig(&zig);

    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let mut objects = Vec::new();
    let sources = FBINK_SOURCES
        .iter()
        .map(|s| fbink.join(s))
        .chain([manifest.join("csrc/fbink_shim.c")]);
    for (i, src) in sources.enumerate() {
        let obj = out.join(format!("{i}-{}.o", src.file_stem().unwrap().to_string_lossy()));
        compile(&zig, &fbink, &src, &obj);
        objects.push(obj);
    }

    let lib = out.join("libfbink.a");
    let _ = std::fs::remove_file(&lib);
    let mut ar = Command::new(&zig);
    ar.arg("ar").arg("rcs").arg(&lib).args(&objects);
    run(&mut ar, "zig ar");

    println!("cargo::rustc-link-search=native={}", out.display());
    println!("cargo::rustc-link-lib=static=fbink");
    println!("cargo::rustc-cfg=fbink");
}

fn compile(zig: &Path, fbink: &Path, src: &Path, obj: &Path) {
    let mut cc = Command::new(zig);
    cc.arg("cc")
        // Same baseline as Rust's armv7-unknown-linux-musleabihf: ARMv7-A, Thumb-2,
        // VFPv3-D16, hard float, no NEON.
        .args(["-target", "arm-linux-musleabihf", "-mcpu=generic+v7a+vfp3d16+thumb2"])
        .args(["-O2", "-std=gnu11", "-fno-sanitize=all"])
        // FBInk's Makefile flags for a MINIMAL KINDLE build: refreshing only, no text,
        // images or drawing (we do all the drawing ourselves).
        .args(["-DNDEBUG", "-D_REENTRANT=1", "-D_GNU_SOURCE"])
        .args(["-DFBINK_FOR_KINDLE", "-DFBINK_MINIMAL"])
        .arg(format!("-DFBINK_VERSION=\"{FBINK_VERSION} for Kindle\""))
        .arg("-I")
        .arg(fbink)
        .arg("-c")
        .arg(src)
        .arg("-o")
        .arg(obj);
    run(&mut cc, &format!("zig cc {}", src.display()));
}

fn check_zig(zig: &Path) {
    let out = match Command::new(zig).arg("version").output() {
        Ok(out) if out.status.success() => out,
        _ => fail(&format!(
            "couldn't run `{} version`. Install zig {ZIG_SERIES}x (https://ziglang.org/download/, \
             `winget install zig.zig`, or `pip install ziglang`), and put it on the PATH or set ZIG \
             to its path",
            zig.display()
        )),
    };
    let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !version.starts_with(ZIG_SERIES) {
        fail(&format!("zig {version} found at {}, but this build needs zig {ZIG_SERIES}x", zig.display()));
    }
}

fn run(cmd: &mut Command, what: &str) {
    match cmd.status() {
        Ok(s) if s.success() => {}
        Ok(s) => fail(&format!("{what} failed ({s})")),
        Err(e) => fail(&format!("couldn't run {what}: {e}")),
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(1);
}
