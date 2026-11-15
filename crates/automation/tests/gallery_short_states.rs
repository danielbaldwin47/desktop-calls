//! The gallery's states that the app shows for only 2 s by design are
//! captured however slow the runner (#248): the Banner's "Identifying
//! caller…" and the Call Pill's "Call ended". On a loaded CI runner, frames
//! came about 2 s apart and missed them. `--late` waits longer than that
//! between reaching each state and capturing it, so a capture can't fall
//! inside the state by luck.

#![expect(
    clippy::expect_used,
    reason = "test code: a failed step fails the test with its message"
)]

use std::path::Path;
use std::process::Command;

/// Longer than either state lasts.
const LATE_MS: &str = "2500";

fn render_late(state: &str) {
    let binary = Path::new(env!("CARGO_BIN_EXE_automation"));
    let output = std::env::temp_dir().join(format!(
        "desktop-calls-short-states-{state}-{}",
        std::process::id()
    ));
    let rendered = Command::new(binary)
        .args(["screenshot", "--only", state, "--late", LATE_MS])
        .arg("--output")
        .arg(&output)
        .output()
        .expect("run automation screenshot");
    let _ = std::fs::remove_dir_all(&output);
    assert!(
        rendered.status.success(),
        "{state}, captured {LATE_MS} ms late, wasn't shown ({}). Its standard error:\n{}",
        rendered.status,
        String::from_utf8_lossy(&rendered.stderr)
    );
}

#[test]
fn the_banner_identifying_the_caller_is_captured_however_late_the_capture() {
    render_late("incoming-call-banner-identifying");
}

#[test]
fn the_call_pill_saying_call_ended_is_captured_however_late_the_capture() {
    render_late("call-pill-ended");
}
