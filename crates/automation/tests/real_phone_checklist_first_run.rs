//! Step 1.2 of `automation real-phone-checklist --demo`, where it used to
//! fail with a message that didn't say what was wrong: the Pixel still
//! paired, no keybinding offered, and the keybinding's page passing through
//! other states on its way to "added".

mod support;

use support::{Runner, result_file, says, step};

#[test]
fn step_1_2_with_the_pixel_still_paired_says_to_start_over_at_1_1_without_waiting_for_it_to_be_listed()
 {
    let results = tempfile::tempdir().expect("a results directory");

    // The user stops as 1.2 starts. 1.1 passed, and its forgetting the
    // pairing is not kept: the next run's Phone is paired.
    let mut first = Runner::start(&["1"], results.path());
    first.quit_at = Some("1.2");
    first.play(|_| "y".to_owned());
    assert_eq!(first.steps_started(), ["1.1", "1.2"]);

    let mut second = Runner::start(&["1"], results.path());
    second.quit_at = Some("1.3");
    second.play(|_| "y".to_owned());
    let result = result_file(results.path());

    let pairing = step(&result, "1.2");
    assert_eq!(pairing["result"], "fail", "{pairing:#}");
    assert!(
        says(pairing, "still paired") && says(pairing, "real-phone-checklist 1 --fresh"),
        "{pairing:#}"
    );
    assert!(
        !second.printed("tap Pair new device"),
        "it had the user pair a Pixel that was paired already: {}",
        second.transcript.join("\n")
    );
}

#[test]
fn step_1_2_fails_at_once_when_the_finish_page_offers_no_keybinding() {
    let results = tempfile::tempdir().expect("a results directory");
    let mut runner = Runner::start(&["1"], results.path());
    // The user's config already uses Super + Alt + C for another program:
    // set up as the user chooses the Pixel, before the finish page checks.
    runner.slips = vec![("enter", "bind-super-alt-c-elsewhere;enter")];
    runner.quit_at = Some("1.3");
    runner.play(|_| "y".to_owned());
    let result = result_file(results.path());

    let pairing = step(&result, "1.2");
    assert_eq!(pairing["result"], "fail", "{pairing:#}");
    assert!(
        says(pairing, "offered no keybinding to add")
            && says(pairing, "Super + Alt + C already does something else"),
        "{pairing:#}"
    );
    assert!(
        !runner.printed("Press Super + Alt + C"),
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
            "Once confirmed, the finish page says: Place your first Call: press"
        ),
        "{pairing:#}"
    );
}
