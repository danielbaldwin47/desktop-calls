//! `automation real-phone-checklist` against a Desktop where Desktop Calls
//! isn't running says how to start it, without pointing at `cargo run`,
//! which would start a second app beside the installed service (#144,
//! ADR-0006).

use test_harness::Harness;

#[test]
fn with_the_app_not_running_the_checklist_says_to_start_the_service_or_use_just_run() {
    let mut harness = Harness::start();
    harness.kill_app_by_hand();

    // The runner needs a terminal for the real Phone, so it runs under
    // `script`, which gives it a pseudo-terminal.
    let runner = format!(
        "{} real-phone-checklist 7",
        env!("CARGO_BIN_EXE_automation")
    );
    let output = harness
        .external_command("script")
        .args(["--quiet", "--return", "--command", &runner, "/dev/null"])
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run the checklist under a pseudo-terminal");
    let said = String::from_utf8_lossy(&output.stdout).replace('\r', "");

    assert!(!output.status.success(), "{output:?}");
    assert!(
        said.contains("Desktop Calls isn't running"),
        "the runner got as far as looking for the app: {said:?}"
    );
    assert!(
        said.contains("`systemctl --user start desktop-calls`") && said.contains("`just run`"),
        "{said:?}"
    );
    assert!(!said.contains("cargo run"), "{said:?}");
}
