// What `cargo test -p test-harness --test <file>` runs, for the names the
// harness's test files had when each was a binary of its own: a pointer to
// `just test-file`, and a failure, rather than a run of nothing.
//
// These targets are `test = false` in `Cargo.toml`, so a whole `cargo test`
// never runs them; only naming one with `--test` does.

use std::process::ExitCode;

fn main() -> ExitCode {
    let file = env!("CARGO_CRATE_NAME");
    eprintln!(
        "The harness's test files are one test binary now, `scenarios` (#118).\n\
         Run this file with:  just test-file {file}\n\
         One scenario:        just test-file {file} <part of its name>\n\
         Under CI's limits:   just test-file {file} --ci-like"
    );
    ExitCode::FAILURE
}
