//! Lists every scenario file in `tests/` as a module of the one test binary,
//! `tests/scenarios.rs`, so a new file runs without anyone listing it.
//!
//! The harness's test files used to be one binary each. Linking 30 binaries
//! cost CI over a minute a build, and `cargo test` ran them one after
//! another, each waiting on its slowest scenario (#118). As one binary they
//! link once and share one pool of test threads.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

/// The one file in `tests/` that is not a scenario file: the binary's root.
/// The stubs for the old per-file names are in `tests/moved/`, which this
/// doesn't read (see `Cargo.toml`).
const NOT_SCENARIOS: &[&str] = &["scenarios.rs"];

fn main() {
    let manifest = PathBuf::from(env("CARGO_MANIFEST_DIR"));
    let tests = manifest.join("tests");
    // A directory's modification time changes when a file is added,
    // removed or renamed in it.
    println!("cargo::rerun-if-changed={}", tests.display());

    let mut names: Vec<String> = fs::read_dir(&tests)
        .unwrap_or_else(|error| panic!("read {}: {error}", tests.display()))
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| {
            Path::new(name)
                .extension()
                .is_some_and(|extension| extension == "rs")
                && !NOT_SCENARIOS.contains(&name.as_str())
        })
        .collect();
    names.sort();

    let mut modules = String::new();
    for name in &names {
        let module = name.trim_end_matches(".rs");
        let path = tests.join(name);
        let _ = writeln!(
            modules,
            "#[path = {:?}]\nmod {module};",
            path.display().to_string()
        );
    }
    let out = PathBuf::from(env("OUT_DIR")).join("scenario_files.rs");
    write(&out, &modules);
}

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("cargo sets {name} for build scripts"))
}

fn write(path: &Path, contents: &str) {
    fs::write(path, contents).unwrap_or_else(|error| panic!("write {}: {error}", path.display()));
}
