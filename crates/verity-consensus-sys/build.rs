use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const VENDOR: &str = "vendor/ssz-specs-pr132/lean";

fn main() {
    println!("cargo::rustc-check-cfg=cfg(verity_has_lean_ssz)");
    emit_rerun_paths(Path::new(VENDOR));
    println!("cargo::rerun-if-changed=src/bridge.c");
    println!("cargo::rerun-if-env-changed=VERITY_LEAN_LAKE");

    if env::var_os("CARGO_FEATURE_LEAN_SSZ").is_none() {
        return;
    }
    let Some(lake) = find_lake() else {
        println!("cargo::warning=Lean SSZ disabled: install elan and Lean 4.33.1");
        return;
    };
    let prefix = build_lean(&lake);
    compile_bridge(&prefix);
    emit_link_args(&prefix);
    println!("cargo::rustc-cfg=verity_has_lean_ssz");
}

fn find_lake() -> Option<PathBuf> {
    if let Some(path) = env::var_os("VERITY_LEAN_LAKE") {
        return command_works(&path).then(|| PathBuf::from(path));
    }
    if command_works(&OsString::from("lake")) {
        return Some(PathBuf::from("lake"));
    }
    let fallback = PathBuf::from(env::var_os("HOME")?).join(".elan/bin/lake");
    command_works(fallback.as_os_str()).then_some(fallback)
}

fn command_works(command: &std::ffi::OsStr) -> bool {
    Command::new(command)
        .current_dir(VENDOR)
        .args(["env", "lean", "--version"])
        .output()
        .is_ok_and(|output| {
            output.status.success()
                && String::from_utf8_lossy(&output.stdout).contains("version 4.33.1")
        })
}

fn build_lean(lake: &Path) -> PathBuf {
    let source = Path::new(VENDOR);
    run(Command::new(lake)
        .current_dir(source)
        .args(["build", "Ssz:static", "VeritySsz:static"]));
    let output =
        run(Command::new(lake)
            .current_dir(source)
            .args(["env", "lean", "--print-prefix"]));
    let prefix = String::from_utf8(output.stdout).expect("Lean prefix should be UTF-8");
    PathBuf::from(prefix.trim())
}

fn compile_bridge(prefix: &Path) {
    let mut build = cc::Build::new();
    build
        .cargo_metadata(false)
        .file("src/bridge.c")
        .include(prefix.join("include"))
        .opt_level(3)
        .warnings(true)
        .compile("verity_ssz_bridge");
}

fn emit_link_args(prefix: &Path) {
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR"));
    let lean_build = fs::canonicalize(Path::new(VENDOR).join(".lake/build/lib"))
        .expect("Lake should create its native library directory");
    println!("cargo::rustc-link-search=native={}", out.display());
    println!("cargo::rustc-link-search=native={}", lean_build.display());
    println!(
        "cargo::rustc-link-search=native={}",
        prefix.join("lib/lean").display()
    );
    println!(
        "cargo::rustc-link-search=native={}",
        prefix.join("lib").display()
    );
    for library in [
        "verity_ssz_bridge",
        "ssz_VeritySsz",
        "ssz_Ssz",
        "Init",
        "leanrt",
        "gmp",
        "unwind",
        "uv",
        "c++",
        "c++abi",
    ] {
        println!("cargo::rustc-link-lib=static={library}");
    }
    for library in ["pthread", "dl", "rt", "m"] {
        println!("cargo::rustc-link-lib={library}");
    }
    println!("cargo::rustc-link-arg=-Wl,--gc-sections");
}

fn run(command: &mut Command) -> Output {
    let description = format!("{command:?}");
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("failed to run {description}: {error}"));
    if !output.status.success() {
        panic!(
            "{description} failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    output
}

fn emit_rerun_paths(root: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.file_name().is_some_and(|name| name == ".lake") {
            continue;
        }
        if path.is_dir() {
            emit_rerun_paths(&path);
        } else {
            println!("cargo::rerun-if-changed={}", path.display());
        }
    }
}
