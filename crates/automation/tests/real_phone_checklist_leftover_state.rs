//! Group 1 of `automation real-phone-checklist --demo` started from the
//! states a failed attempt leaves behind, where a real sitting begins (retro
//! of 2026-11-15, candidate 4): each step, resumed with the steps before it
//! done, from the Phone still paired, paired on its side only, its pairing
//! forgotten with the link up, and Contacts shared, refused or prompting.
//! Whatever the step makes of it, no result lands under a step that didn't
//! run, nothing waits without a word, and the run ends with the command to
//! run next.

// Clippy exempts `#[test]` functions but not their helpers. Like a test, a
// helper whose step fails must fail the test at once with its message.
#![expect(
    clippy::expect_used,
    reason = "test code: a failed helper step fails the test with its message"
)]

mod support;

use std::path::Path;

use support::{Runner, result_file, says, step};

/// Group 1's runnable steps, in order.
const GROUP_1: [&str; 4] = ["1.1", "1.2", "1.3", "1.4"];

/// A state a failed attempt leaves, as the demo actions that make it.
struct Leftover {
    name: &'static str,
    actions: &'static [&'static str],
}

const STILL_PAIRED: Leftover = Leftover {
    name: "the Phone still paired",
    actions: &[],
};
const PHONE_SIDE_ONLY: Leftover = Leftover {
    name: "paired on the Phone's side only, unknown to the app",
    actions: &["desktop-removes-phone"],
};
const FORGOTTEN_LINK_UP: Leftover = Leftover {
    name: "the pairing forgotten on the Phone with the link up",
    actions: &["phone-forgets-keeping-link"],
};
const CONTACTS_SHARED: Leftover = Leftover {
    name: "Contacts shared",
    actions: &["contacts-access-on"],
};
const CONTACTS_REFUSED: Leftover = Leftover {
    name: "Contacts refused",
    actions: &["contacts-refused"],
};
const CONTACTS_PROMPTING: Leftover = Leftover {
    name: "Contacts prompting",
    actions: &["contacts-prompt"],
};

/// A demo result file in `results` with each step of group 1 before `step`
/// done in run 1, so the next run resumes at `step`.
fn done_before(results: &Path, step: &str) {
    let done: Vec<serde_json::Value> = GROUP_1
        .iter()
        .take_while(|id| **id != step)
        .map(|id| {
            serde_json::json!({
                "id": id, "title": "", "result": "pass", "run": 1,
                "evidence": [], "measurements": {},
                "started_us": 1, "ended_us": 2, "journal": []
            })
        })
        .collect();
    if done.is_empty() {
        return;
    }
    let file = serde_json::json!({
        "mode": "demo",
        "started": "2026-11-15T00:00:00Z",
        "updated": "2026-11-15T00:00:00Z",
        "phone": "Simulated Pixel",
        "runs": [{ "run": 1, "started": "2026-11-15T00:00:00Z",
                   "command": "just real-phone-checklist --demo 1" }],
        "steps": done,
        "earlier": []
    });
    std::fs::write(results.join("2026-11-15T00-00-00Z.json"), file.to_string())
        .expect("write the earlier run's results");
}

/// Runs `step` of group 1 from `leftover`, answering every question yes,
/// and stops as the next step starts.
fn from_leftover(id: &'static str, leftover: &Leftover) {
    let results = tempfile::tempdir().expect("a results directory");
    done_before(results.path(), id);
    let mut runner = Runner::start(&["1"], results.path());
    runner.before_start = leftover.actions.to_vec();
    runner.quit_at = GROUP_1
        .iter()
        .skip_while(|other| **other != id)
        .nth(1)
        .copied();
    runner.play(|_| "y".to_owned());
    let transcript = runner.transcript.join("\n");
    let context = format!("step {id} from {}:\n{transcript}", leftover.name);

    let started = runner.steps_started();
    assert!(runner.printed(&format!("Step {id}:")), "{context}");
    assert_eq!(started.first().map(String::as_str), Some(id), "{context}");
    assert!(
        runner
            .transcript
            .last()
            .is_some_and(|line| line.starts_with("Next: ")),
        "the run ends with what to run next; {context}"
    );

    // Each record from this run is a step that ran in it, and the steps
    // done before are kept as they were unless the run went back to them.
    let result = result_file(results.path());
    let this_run = result["runs"].as_array().map_or(0, Vec::len);
    let records = result["steps"]
        .as_array()
        .into_iter()
        .chain(result["earlier"].as_array())
        .flatten();
    for record in records {
        let other = record["id"].as_str().unwrap_or_default();
        if record["run"].as_u64() == u64::try_from(this_run).ok() {
            assert!(
                started.iter().any(|ran| ran == other),
                "{other} has a result from this run but didn't run; {context}"
            );
            assert!(
                record["result"] != "fail" || says(record, ""),
                "{other} failed without saying why; {context}"
            );
        } else {
            assert_eq!(record["result"], "pass", "{other}: {context}");
        }
    }
    // The step started from the leftover has its own result, unless it
    // went back to an earlier step, which then ran.
    if !runner.printed("Going back to step") {
        assert_eq!(
            step(&result, id)["run"].as_u64(),
            u64::try_from(this_run).ok(),
            "{context}"
        );
    }
}

macro_rules! leftover_tests {
    ($($(#[$attribute:meta])* $name:ident: $step:literal from $leftover:ident;)*) => {
        $(
            #[test]
            $(#[$attribute])*
            fn $name() {
                from_leftover($step, &$leftover);
            }
        )*
    };
}

leftover_tests! {
    step_1_1_from_the_phone_still_paired: "1.1" from STILL_PAIRED;
    step_1_1_paired_on_the_phones_side_only: "1.1" from PHONE_SIDE_ONLY;
    step_1_1_forgotten_with_the_link_up: "1.1" from FORGOTTEN_LINK_UP;
    step_1_1_with_contacts_shared: "1.1" from CONTACTS_SHARED;
    step_1_1_with_contacts_refused: "1.1" from CONTACTS_REFUSED;
    step_1_1_with_contacts_prompting: "1.1" from CONTACTS_PROMPTING;
    step_1_2_from_the_phone_still_paired: "1.2" from STILL_PAIRED;
    #[ignore = "#417: 1.2 waits 2 minutes in silence for a Setup Window nobody opened"]
    step_1_2_paired_on_the_phones_side_only: "1.2" from PHONE_SIDE_ONLY;
    step_1_2_forgotten_with_the_link_up: "1.2" from FORGOTTEN_LINK_UP;
    step_1_2_with_contacts_shared: "1.2" from CONTACTS_SHARED;
    step_1_2_with_contacts_refused: "1.2" from CONTACTS_REFUSED;
    step_1_2_with_contacts_prompting: "1.2" from CONTACTS_PROMPTING;
    step_1_3_from_the_phone_still_paired: "1.3" from STILL_PAIRED;
    step_1_3_paired_on_the_phones_side_only: "1.3" from PHONE_SIDE_ONLY;
    step_1_3_forgotten_with_the_link_up: "1.3" from FORGOTTEN_LINK_UP;
    step_1_3_with_contacts_shared: "1.3" from CONTACTS_SHARED;
    step_1_3_with_contacts_refused: "1.3" from CONTACTS_REFUSED;
    step_1_3_with_contacts_prompting: "1.3" from CONTACTS_PROMPTING;
    step_1_4_from_the_phone_still_paired: "1.4" from STILL_PAIRED;
    step_1_4_paired_on_the_phones_side_only: "1.4" from PHONE_SIDE_ONLY;
    step_1_4_forgotten_with_the_link_up: "1.4" from FORGOTTEN_LINK_UP;
    step_1_4_with_contacts_shared: "1.4" from CONTACTS_SHARED;
    step_1_4_with_contacts_refused: "1.4" from CONTACTS_REFUSED;
    step_1_4_with_contacts_prompting: "1.4" from CONTACTS_PROMPTING;
}
