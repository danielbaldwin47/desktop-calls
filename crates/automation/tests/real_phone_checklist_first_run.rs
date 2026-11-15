//! Group 1 of `automation real-phone-checklist --demo`, where it used to
//! fail with a message that didn't say what was wrong (the Pixel still
//! paired, no keybinding offered, the keybinding's page passing through
//! other states on its way to "added"), and where the owner's sitting of
//! 2026-11-15 went wrong (#380): runs blended in one file, a resumed run
//! failing 1.2 and pairing under 1.3, and 1.3 failing a Pixel that shares
//! Contacts as it pairs.

// Clippy exempts `#[test]` functions but not their helpers. Like a test, a
// helper whose step fails must fail the test at once with its message.
#![expect(
    clippy::expect_used,
    reason = "test code: a failed helper step fails the test with its message"
)]

mod support;

use support::{Runner, result_file, says, step};

/// The records of step `id` that a later record replaced, oldest first.
fn earlier<'a>(result: &'a serde_json::Value, id: &str) -> Vec<&'a serde_json::Value> {
    result["earlier"]
        .as_array()
        .expect("the earlier records")
        .iter()
        .filter(|record| record["id"] == id)
        .collect()
}

/// The lines of the summary that ends the run.
fn summary(runner: &Runner) -> Vec<String> {
    runner
        .transcript
        .iter()
        .skip_while(|line| *line != "Summary:")
        .cloned()
        .collect()
}

#[test]
fn a_resumed_run_that_finds_the_pixel_paired_at_1_2_goes_back_to_1_1_and_keeps_each_runs_results_apart()
 {
    let results = tempfile::tempdir().expect("a results directory");

    // The user stops as 1.2 starts. 1.1 passed, and its forgetting the
    // pairing is not kept: the next run's Phone is paired.
    let mut first = Runner::start(&["1"], results.path());
    first.quit_at = Some("1.2");
    first.play(|_| "y".to_owned());
    assert_eq!(first.steps_started(), ["1.1", "1.2"]);
    assert!(
        summary(&first).iter().any(|line| line == "  1.1 passed.")
            && summary(&first).iter().any(|line| line == "  1.2 not run.")
            && summary(&first).iter().any(|line| line
                == "Next: run `just real-phone-checklist --demo 1` to carry on from 1.2."),
        "{}",
        first.transcript.join("\n")
    );

    // 1.2 offers 1.1 there and then, rather than failing and walking on
    // into 1.3 with a pairing to do.
    let mut second = Runner::start(&["1"], results.path());
    second.quit_at = Some("1.3");
    second.play(|_| "y".to_owned());
    assert_eq!(
        second.steps_started(),
        ["1.2", "1.1", "1.2", "1.3"],
        "{}",
        second.transcript.join("\n")
    );
    assert!(
        second.printed("The Pixel is still paired") && second.printed("go back to step 1.1"),
        "{}",
        second.transcript.join("\n")
    );
    let result = result_file(results.path());
    let runs = result["runs"].as_array().expect("the runs");
    assert_eq!(runs.len(), 2, "{result:#}");
    assert_eq!(runs[1]["command"], "just real-phone-checklist --demo 1");
    for id in ["1.1", "1.2"] {
        assert_eq!(step(&result, id)["result"], "pass", "{result:#}");
        assert_eq!(step(&result, id)["run"], 2, "{result:#}");
    }
    // Run 1's 1.1 is kept, not overwritten; going back recorded no 1.2.
    let first_1_1 = earlier(&result, "1.1");
    assert_eq!(first_1_1.len(), 1, "{result:#}");
    assert_eq!(first_1_1[0]["run"], 1, "{result:#}");
    assert!(earlier(&result, "1.2").is_empty(), "{result:#}");
    let first_call = &step(&result, "1.2")["measurements"]["pairing_to_first_call_ms"];
    assert!(first_call.as_u64().is_some(), "{result:#}");
}

#[test]
fn step_1_1_fails_when_the_app_comes_back_from_the_install_with_the_pixel_still_paired_and_offers_itself_again()
 {
    let results = tempfile::tempdir().expect("a results directory");
    let mut runner = Runner::start(&["1"], results.path());
    // Once Desktop Calls has let go, the user pairs again through the
    // Setup Window before installing: the window opens at its first step
    // by hand, and the app comes back from the install with the Pixel.
    runner.slips = vec![("restart-app", "pair-through-setup-window;restart-app")];
    runner.quit_at = Some("1.2");
    runner.play(|_| "y".to_owned());
    assert_eq!(
        runner.steps_started(),
        ["1.1", "1.1", "1.2"],
        "{}",
        runner.transcript.join("\n")
    );
    let result = result_file(results.path());
    let failed = earlier(&result, "1.1");
    assert_eq!(failed.len(), 1, "{result:#}");
    assert_eq!(failed[0]["result"], "fail", "{result:#}");
    assert!(
        says(
            failed[0],
            "found the Pixel still paired when it started again"
        ),
        "{result:#}"
    );
    assert_eq!(step(&result, "1.1")["result"], "pass", "{result:#}");
}

#[test]
fn step_1_3_passes_a_pixel_that_shares_contacts_as_it_pairs_with_the_box_unticked_and_the_run_ends_with_a_summary()
 {
    let results = tempfile::tempdir().expect("a results directory");
    let mut runner = Runner::start(&["1"], results.path());
    // Unticking the box changes nothing on this Phone: it never asks.
    runner.slips = vec![("contacts-access-off", "")];
    let questions = runner.play(|_| "y".to_owned());
    assert_eq!(questions.len(), 1, "{questions:?}");
    assert!(questions[0].contains("untick"), "{questions:?}");
    let result = result_file(results.path());
    let prompt = step(&result, "1.3");
    assert_eq!(prompt["result"], "pass", "{prompt:#}");
    assert!(
        says(prompt, "shares Contacts as it pairs even with"),
        "{prompt:#}"
    );
    let summary = summary(&runner);
    for line in [
        "  1.1 passed.",
        "  1.2 passed.",
        "  1.3 passed.",
        "  1.4 passed.",
        "Next: run `just real-phone-checklist --demo 2` for group 2.",
    ] {
        assert!(
            summary.iter().any(|printed| printed == line),
            "{line:?} missing from:\n{}",
            summary.join("\n")
        );
    }
}

#[test]
fn step_1_3_passes_a_pixel_that_refuses_contacts_with_the_box_unticked_and_says_how_to_share_them_again()
 {
    let results = tempfile::tempdir().expect("a results directory");
    let mut runner = Runner::start(&["1"], results.path());
    runner.slips = vec![("contacts-access-off", "contacts-refused")];
    runner.quit_at = Some("1.4");
    let questions = runner.play(|_| "y".to_owned());
    assert!(questions.is_empty(), "{questions:?}");
    let result = result_file(results.path());
    let prompt = step(&result, "1.3");
    assert_eq!(prompt["result"], "pass", "{prompt:#}");
    assert!(says(prompt, "refuses Contacts"), "{prompt:#}");
    assert!(
        runner.printed("switch on Contacts and call history"),
        "{}",
        runner.transcript.join("\n")
    );
}

#[test]
fn step_1_3_fails_when_the_box_was_left_ticked_and_the_summary_says_why_and_what_to_run() {
    let results = tempfile::tempdir().expect("a results directory");
    let mut runner = Runner::start(&["1"], results.path());
    runner.slips = vec![("contacts-access-off", "")];
    runner.play(|question| {
        if question.contains("untick") {
            "n".to_owned()
        } else {
            "y".to_owned()
        }
    });
    let result = result_file(results.path());
    assert_eq!(step(&result, "1.3")["result"], "fail", "{result:#}");
    let summary = summary(&runner);
    assert!(
        summary
            .iter()
            .any(|line| line.starts_with("  1.3 failed: ") && line.contains("left ticked")),
        "{}",
        summary.join("\n")
    );
    assert!(
        summary
            .iter()
            .any(|line| line == "Next: run `just real-phone-checklist --demo 1` to retry 1.3."),
        "{}",
        summary.join("\n")
    );
}

#[test]
fn step_1_2_fails_when_the_finish_page_offers_no_keybinding() {
    let results = tempfile::tempdir().expect("a results directory");
    let mut runner = Runner::start(&["1"], results.path());
    // The user's config already uses Super+Alt+C for another program:
    // set up as the user chooses the Pixel, before the finish page checks.
    runner.slips = vec![("enter", "bind-super-alt-c-elsewhere;enter")];
    runner.quit_at = Some("1.3");
    runner.play(|_| "y".to_owned());
    let result = result_file(results.path());

    let pairing = step(&result, "1.2");
    assert_eq!(pairing["result"], "fail", "{pairing:#}");
    assert!(
        says(pairing, "offered no keybinding to add")
            && says(pairing, "Super+Alt+C is already used"),
        "{pairing:#}"
    );
    assert!(
        !runner.printed("Press Super+Alt+C"),
        "it had the user press keys that open nothing: {}",
        runner.transcript.join("\n")
    );
}

#[test]
fn step_1_2_waits_for_the_added_page_when_the_user_cancels_the_line_and_adds_it_after_all() {
    let results = tempfile::tempdir().expect("a results directory");
    let mut runner = Runner::start(&["1"], results.path());
    // Choose the Pixel, confirm the code, Add keybinding: the fourth Enter
    // is Confirm on the line, which the user changes their mind about.
    runner.slips = vec![("enter", "cancel-then-confirm")];
    runner.slip_after = 3;
    runner.quit_at = Some("1.3");
    runner.play(|_| "y".to_owned());
    let result = result_file(results.path());

    let pairing = step(&result, "1.2");
    assert_eq!(pairing["result"], "pass", "{pairing:#}");
    assert!(
        says(
            pairing,
            "Once confirmed, the finish page says: Place your first call: press"
        ),
        "{pairing:#}"
    );
}
