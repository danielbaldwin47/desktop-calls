//! `automation real-phone-checklist 4 --demo`: group 4, call waiting (#148),
//! against the Simulated Phone, with this test playing the user, the two
//! helpers and the third phone.

mod support;

use support::{Runner, result_file, says, step};

const MISSED_CALL_SHOWN: &str = "Does a notification say you missed a call from the second helper, by name or number, with the time?";

/// Every step of group 4 passes, each saying what it saw, and 4.6 carries
/// on from the Calls 4.5 left up instead of placing them again.
#[test]
fn group_4_checks_each_waiting_call_choice_and_4_6_carries_on_from_4_5() {
    let results = tempfile::tempdir().expect("a results directory");
    let mut runner = Runner::start(&["4"], results.path());
    let questions = runner.play(|_| "y".to_owned());
    let result = result_file(results.path());

    assert_eq!(
        questions,
        [MISSED_CALL_SHOWN],
        "{}",
        runner.transcript.join("\n")
    );
    for (id, saw) in [
        (
            "4.1",
            &[
                "The Banner offered Hold & answer, End & answer, Decline.",
                "The Desktop stayed silent for the Waiting Call.",
                "Hold & answer put the Test Number's Call on hold and made the second helper's live",
                "Swap put the Test Number's Call live and the second helper's on hold",
                "Hanging up one Call left the other, the second helper's, live alone.",
            ][..],
        ),
        (
            "4.2",
            &["End & answer ended the Test Number's Call and connected the second helper's"][..],
        ),
        (
            "4.3",
            &["Decline ended only the second helper's Call: the Test Number's is still live"][..],
        ),
        (
            "4.4",
            &[
                "A Missed Call was notified",
                "The Test Number's Call carried on, live, while the second helper's rang out.",
            ][..],
        ),
        (
            "4.5",
            &[
                "The Banner offered End & answer, Decline.",
                "End & answer ended only the second helper's first Call",
            ][..],
        ),
        (
            "4.6",
            &[
                "Carrying on from 4.5",
                "The Test Number's Call stayed on hold, alone, when the third phone hung up.",
                "The Banner offered Answer, Decline.",
                "Answer connected the second helper's new Call, and the Test Number's stayed on hold.",
            ][..],
        ),
    ] {
        let recorded = step(&result, id);
        assert_eq!(recorded["result"], "pass", "{id}: {recorded:#}");
        for line in saw {
            assert!(says(recorded, line), "{id}: {line:?} in {recorded:#}");
        }
        assert!(
            recorded["measurements"]["banner_latency_ms"].is_u64(),
            "{id}: {recorded:#}"
        );
    }
    assert_eq!(
        runner.steps_started(),
        ["4.1", "4.2", "4.3", "4.4", "4.5", "4.6"]
    );
    assert!(runner.printed("Leave both Calls as they are"));
    let placed_again = runner
        .transcript
        .iter()
        .filter(|line| line.contains("call you again, from the third phone"))
        .count();
    assert_eq!(placed_again, 1, "{}", runner.transcript.join("\n"));
}

/// A Phone that doesn't hold, as one that didn't list `AT+CHLD` support:
/// the step says what the app showed and the Calls are ended, so the next
/// step starts clean.
#[test]
fn a_phone_that_refuses_to_hold_fails_4_1_with_what_the_call_window_said_and_ends_the_calls() {
    let results = tempfile::tempdir().expect("a results directory");
    let mut runner = Runner::start(&["4"], results.path());
    runner.slips = vec![("call", "quirk chld-unsupported;call")];
    runner.play(|_| "y".to_owned());
    let result = result_file(results.path());

    let held = step(&result, "4.1");
    assert_eq!(held["result"], "fail", "{held:#}");
    assert!(
        says(
            held,
            "Hold & answer didn't leave the second helper's Call live beside the Test Number's on hold"
        ),
        "{held:#}"
    );
    assert!(
        says(held, "the Call Window says: Couldn't hold your call"),
        "{held:#}"
    );
    assert!(
        !says(held, "Swap put"),
        "the step went on after the refusal: {held:#}"
    );
    let transcript = runner.transcript.join("\n");
    assert!(
        !transcript.contains("A Call is still going"),
        "{transcript}"
    );
}

/// The Test Number guards the first Call: a slipped digit is hung up at
/// once and no helper is asked to call in.
#[test]
fn a_first_call_to_another_number_is_hung_up_and_step_4_1_goes_no_further() {
    let results = tempfile::tempdir().expect("a results directory");
    let mut runner = Runner::start(&["4"], results.path());
    runner.slips = vec![("type-test-number", "type-wrong-number")];
    runner.play(|_| "y".to_owned());
    let result = result_file(results.path());

    let wrong = step(&result, "4.1");
    assert_eq!(wrong["result"], "fail", "{wrong:#}");
    assert!(
        says(
            wrong,
            "isn't the Test Number, so the runner hung it up, and it ended"
        ),
        "{wrong:#}"
    );
    assert!(!says(wrong, "Banner"), "{wrong:#}");
    assert_eq!(step(&result, "4.2")["result"], "pass");
}

/// When 4.5 fails, 4.6 has nothing to carry on from, and brings the Calls
/// up again itself.
#[test]
fn step_4_6_brings_up_its_own_calls_when_4_5_left_none() {
    let results = tempfile::tempdir().expect("a results directory");
    let mut runner = Runner::start(&["4"], results.path());
    // 4.1 to 4.4 place their Call as asked; 4.5's goes to a wrong number.
    runner.slips = vec![("type-test-number", "type-wrong-number")];
    runner.slip_after = 4;
    runner.play(|_| "y".to_owned());
    let result = result_file(results.path());

    let five = step(&result, "4.5");
    assert_eq!(five["result"], "fail", "{five:#}");
    let six = step(&result, "4.6");
    assert_eq!(six["result"], "pass", "{six:#}");
    assert!(!says(six, "Carrying on from 4.5"), "{six:#}");
    assert!(
        says(six, "Answer connected the second helper's new Call"),
        "{six:#}"
    );
}
