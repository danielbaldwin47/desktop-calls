//! `automation real-phone-checklist` against the real Phone asks the user
//! questions, so it needs an interactive terminal. Run without one (from
//! Claude Code's `!` prefix, a pipe or a service), it says so in one line
//! before it touches anything (ticket #120).

#![expect(
    clippy::expect_used,
    reason = "test code: a failed step fails the test with its message"
)]

use std::process::{Command, Stdio};

#[test]
fn without_a_terminal_the_checklist_says_why_in_one_line_and_writes_no_results() {
    let home = tempfile::tempdir().expect("a temporary home");
    // A bus nobody listens on, so the runner can't reach the user's real
    // Desktop Calls even if it tried.
    let output = Command::new(env!("CARGO_BIN_EXE_automation"))
        .args(["real-phone-checklist", "7"])
        .current_dir(home.path())
        .env("HOME", home.path())
        .env("XDG_STATE_HOME", home.path().join("state"))
        .env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/nonexistent/bus")
        .stdin(Stdio::null())
        .output()
        .expect("run the checklist");

    assert!(!output.status.success(), "{output:?}");
    let said = String::from_utf8_lossy(&output.stderr);
    let lines: Vec<&str> = said.lines().collect();
    assert_eq!(lines.len(), 1, "one line: {said:?}");
    assert!(
        lines[0].contains("needs an interactive terminal")
            && lines[0].contains("just real-phone-checklist 7"),
        "{said:?}"
    );
    assert!(output.stdout.is_empty(), "{output:?}");
    let left: Vec<_> = walk(home.path());
    assert!(left.is_empty(), "no results anywhere: {left:?}");
}

/// Every file under `directory`.
fn walk(directory: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(directory).expect("list the directory") {
        let path = entry.expect("an entry").path();
        if path.is_dir() {
            files.extend(walk(&path));
        } else {
            files.push(path);
        }
    }
    files
}
