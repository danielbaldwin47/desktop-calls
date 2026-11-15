//! `automation scorecard`: every polish-scorecard item with PASS, FAIL,
//! MANUAL or NEEDS-REVIEW, and its evidence (#27), scored from made-up
//! scenario reports, Real-Phone Checklist result files and review verdicts.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The repository's root: item 8 compares the review with its `HEAD`.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Runs `automation scorecard` from the repository's root and returns what
/// it printed.
fn scorecard(arguments: &[&str]) -> String {
    let printed = Command::new(env!("CARGO_BIN_EXE_automation"))
        .current_dir(root())
        .arg("scorecard")
        .args(arguments)
        .output()
        .expect("run automation scorecard");
    assert!(printed.status.success(), "scorecard failed: {printed:?}");
    String::from_utf8(printed.stdout).expect("the scorecard is text")
}

/// Every scenario the scorecard reads, from the filter it prints.
fn scenarios() -> Vec<String> {
    scorecard(&["--filter"])
        .trim()
        .split(" | ")
        .map(|test| {
            test.strip_prefix("test(=")
                .and_then(|name| name.strip_suffix(')'))
                .unwrap_or_else(|| panic!("a filter term per scenario: {test}"))
                .to_owned()
        })
        .collect()
}

/// A JUnit report, as cargo-nextest writes it, of `scenarios`, with
/// `failed` among them failing.
fn junit(directory: &Path, scenarios: &[String], failed: &[&str]) -> PathBuf {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuites name=\"nextest-run\">\n<testsuite name=\"test-harness::scenarios\">\n",
    );
    for name in scenarios {
        if failed.contains(&name.as_str()) {
            xml.push_str(&format!(
                "<testcase name=\"{name}\" classname=\"test-harness::scenarios\" time=\"1.0\"><failure type=\"test failure\">thread panicked</failure></testcase>\n"
            ));
        } else {
            xml.push_str(&format!(
                "<testcase name=\"{name}\" classname=\"test-harness::scenarios\" time=\"1.0\"/>\n"
            ));
        }
    }
    xml.push_str("</testsuite>\n</testsuites>\n");
    let path = directory.join("junit.xml");
    std::fs::write(&path, xml).expect("write the report");
    path
}

/// The row of item `number`.
fn row(text: &str, number: usize) -> &str {
    text.lines()
        .find(|line| line.trim_start().starts_with(&format!("{number} ")))
        .unwrap_or_else(|| panic!("no row for item {number}:\n{text}"))
}

fn status(text: &str, number: usize) -> &'static str {
    let row = row(text, number);
    ["NEEDS-REVIEW", "MANUAL", "PASS", "FAIL"]
        .into_iter()
        .find(|status| row.contains(&format!("  {status} ")))
        .unwrap_or_else(|| panic!("item {number} has no status: {row}"))
}

/// Item 7 fails while the keyboard-only suite lists an action with no key
/// path, and passes once it lists none.
fn keyboard_status() -> &'static str {
    let suite = std::fs::read_to_string(root().join("crates/test-harness/tests/keyboard_only.rs"))
        .expect("read the keyboard-only suite");
    if suite.contains("Missing { ticket:") {
        "FAIL"
    } else {
        "PASS"
    }
}

#[test]
fn the_scorecard_lists_all_eight_items_from_the_wiki_with_status_and_evidence() {
    let empty = tempfile::tempdir().expect("an empty results directory");
    let text = scorecard(&["--results", empty.path().to_str().unwrap()]);

    let wiki = std::fs::read_to_string(root().join("docs/wiki/polish-scorecard.md"))
        .expect("read the polish scorecard");
    let items: Vec<&str> = wiki
        .lines()
        .filter_map(|line| {
            let (number, item) = line.split_once(". ")?;
            number.parse::<u8>().ok().map(|_| item)
        })
        .collect();
    assert_eq!(items.len(), 8, "the wiki lists eight items: {items:#?}");
    let header = text
        .lines()
        .find(|line| line.contains("Item"))
        .expect("a header row");
    assert!(
        header.contains("Status") && header.contains("Evidence"),
        "{text}"
    );
    for (index, item) in items.iter().enumerate() {
        assert!(
            row(&text, index + 1).contains(item),
            "row {} should read {item:?}",
            index + 1
        );
    }
}

#[test]
fn with_no_evidence_simulated_items_fail_real_phone_items_are_manual() {
    let empty = tempfile::tempdir().expect("an empty directory");
    let missing = empty.path().join("no-review.md");
    let text = scorecard(&[
        "--results",
        empty.path().to_str().unwrap(),
        "--review",
        missing.to_str().unwrap(),
    ]);

    for item in [1, 2, 5, 6, 7] {
        assert_eq!(status(&text, item), "FAIL", "{text}");
        assert!(
            row(&text, item)
                .ends_with("no scenario report: `just scorecard` runs the scenarios first")
        );
    }
    assert_eq!(status(&text, 3), "MANUAL", "{text}");
    assert!(row(&text, 3).contains("group 2"), "{text}");
    assert_eq!(status(&text, 4), "MANUAL", "{text}");
    assert!(row(&text, 4).contains("group 1"), "{text}");
    assert_eq!(status(&text, 8), "NEEDS-REVIEW", "{text}");
    assert!(row(&text, 8).contains("no review verdict"), "{text}");
    assert!(
        text.contains("0 of 8 items pass (5 fail, 2 manual, 1 need review)"),
        "{text}"
    );
}

#[test]
fn with_every_scenario_passing_and_no_real_phone_the_items_that_need_it_are_manual() {
    let directory = tempfile::tempdir().expect("a directory");
    let scenarios = scenarios();
    let report = junit(directory.path(), &scenarios, &[]);
    let text = scorecard(&[
        "--results",
        directory.path().to_str().unwrap(),
        "--scenarios",
        report.to_str().unwrap(),
    ]);

    for item in [1, 2, 6] {
        assert_eq!(status(&text, item), "MANUAL", "{text}");
        assert!(row(&text, item).contains("real Phone"), "{text}");
    }
    assert_eq!(status(&text, 5), "MANUAL", "{text}");
    assert!(
        row(&text, 5).contains("20 of 20 canonical scenarios pass"),
        "{text}"
    );
    assert_eq!(status(&text, 7), keyboard_status(), "{text}");
    assert!(row(&text, 7).contains("9 of 9 scenarios pass"), "{text}");
}

#[test]
fn a_failing_or_missing_scenario_fails_its_item_and_is_named() {
    let directory = tempfile::tempdir().expect("a directory");
    let scenarios = scenarios();
    let declining = "incoming_call::declining_hangs_up_the_ringing_call";
    let reconnect = "out_of_range::after_resume_the_phone_is_connected_within_10_s";
    // A renamed Banner scenario: the report no longer lists the old name.
    let renamed = "incoming_call_surfaces::the_banner_shows_the_formatted_number_within_a_second";
    let listed: Vec<String> = scenarios
        .into_iter()
        .filter(|name| name != renamed)
        .collect();
    let report = junit(directory.path(), &listed, &[declining, reconnect]);
    let text = scorecard(&[
        "--results",
        directory.path().to_str().unwrap(),
        "--scenarios",
        report.to_str().unwrap(),
    ]);

    assert_eq!(status(&text, 1), "FAIL", "{text}");
    assert!(
        row(&text, 1).contains(&format!("{renamed} is not in the report")),
        "{text}"
    );
    assert_eq!(status(&text, 5), "FAIL", "{text}");
    assert!(
        row(&text, 5).contains("18 of 20 canonical scenarios pass"),
        "{text}"
    );
    assert!(
        row(&text, 5).contains(&format!(
            "not passing: Decline on the Desktop ({declining} failed)"
        )),
        "{text}"
    );
    assert_eq!(status(&text, 6), "FAIL", "{text}");
    assert!(
        row(&text, 6).contains(&format!("{reconnect} failed")),
        "{text}"
    );
}

#[test]
fn the_latest_real_checklist_decides_the_items_that_need_the_real_phone() {
    let directory = tempfile::tempdir().expect("a directory");
    let scenarios = scenarios();
    let report = junit(directory.path(), &scenarios, &[]);
    let step = |id: &str, measurements: &str| {
        format!(
            r#"{{ "id": "{id}", "title": "Step {id}", "result": "pass", "evidence": [], "measurements": {measurements}, "started_us": 0, "ended_us": 0, "journal": [] }}"#
        )
    };
    let steps = [
        step("1.2", r#"{ "pairing_to_first_call_ms": 150000 }"#),
        step("2.1", r#"{ "banner_latency_ms": 210 }"#),
        step(
            "2.2",
            r#"{ "audio_latency_ms": 2000, "codec": "msbc", "wideband": true }"#,
        ),
        step("6.1", "{}"),
        step("6.2", "{}"),
    ];
    let file = format!(
        r#"{{ "mode": "real", "started": "2026-11-20T10:00:00Z", "updated": "2026-11-20T10:40:00Z", "phone": "My Pixel", "steps": [{}] }}"#,
        steps.join(", ")
    );
    std::fs::write(directory.path().join("2026-11-20T10-00-00Z.json"), file)
        .expect("write the result file");
    let text = scorecard(&[
        "--results",
        directory.path().to_str().unwrap(),
        "--scenarios",
        report.to_str().unwrap(),
    ]);

    assert_eq!(status(&text, 1), "PASS", "{text}");
    assert!(
        row(&text, 1).contains("real Phone: 210 ms against 1000 ms"),
        "{text}"
    );
    assert_eq!(status(&text, 2), "FAIL", "{text}");
    assert!(
        row(&text, 2).contains("real Phone: 2000 ms against 1500 ms"),
        "{text}"
    );
    assert_eq!(status(&text, 3), "PASS", "{text}");
    assert!(
        row(&text, 3).contains("msbc (wideband) on My Pixel"),
        "{text}"
    );
    assert_eq!(status(&text, 4), "PASS", "{text}");
    assert!(
        row(&text, 4).contains("150 s from pairing to the first Call"),
        "{text}"
    );
    assert_eq!(status(&text, 5), "PASS", "{text}");
    assert!(
        row(&text, 5).contains("5 pass, 0 fail, 0 skipped"),
        "{text}"
    );
    assert_eq!(status(&text, 6), "PASS", "{text}");
    assert!(
        row(&text, 6).contains("2 of 2 group 6 steps pass"),
        "{text}"
    );
}

/// A verdict file reviewing the repository's `HEAD`, with `ids` replacing
/// the object id of each path named in it.
fn verdict(directory: &Path, verdict: &str, defects: &[&str], ids: &[(&str, &str)]) -> PathBuf {
    let mut text =
        format!("# Screenshot review\n\nReviewed commit: abc1234\nVerdict: {verdict}\n\n");
    for path in [
        "crates/desktop-calls/src",
        "crates/core/src/messages",
        "crates/core/src/messages.rs",
        "crates/automation/src/gallery",
        "crates/automation/src/gallery.rs",
    ] {
        let id = ids.iter().find(|(named, _)| *named == path).map_or_else(
            || {
                let output = Command::new("git")
                    .current_dir(root())
                    .args(["rev-parse", &format!("HEAD:{path}")])
                    .output()
                    .expect("run git");
                String::from_utf8(output.stdout).unwrap().trim().to_owned()
            },
            |(_, id)| (*id).to_owned(),
        );
        text.push_str(&format!("Reviewed tree: {path} {id}\n"));
    }
    for defect in defects {
        text.push_str(&format!("Defect: {defect}\n"));
    }
    let path = directory.join("screenshot-review.md");
    std::fs::write(&path, text).expect("write the verdict");
    path
}

fn item_8(review: &Path) -> String {
    let empty = tempfile::tempdir().expect("an empty results directory");
    let text = scorecard(&[
        "--results",
        empty.path().to_str().unwrap(),
        "--review",
        review.to_str().unwrap(),
        "--gallery",
        "target/screenshots/gallery/index.html",
    ]);
    row(&text, 8).to_owned()
}

#[test]
fn a_current_review_with_no_defects_passes_and_links_the_gallery() {
    let directory = tempfile::tempdir().expect("a directory");
    let row = item_8(&verdict(directory.path(), "no defects", &[], &[]));
    assert!(row.contains("  PASS "), "{row}");
    assert!(row.contains("no defects at abc1234"), "{row}");
    assert!(
        row.ends_with("gallery: target/screenshots/gallery/index.html"),
        "{row}"
    );
}

#[test]
fn a_current_review_with_defects_fails_and_names_their_issues() {
    let directory = tempfile::tempdir().expect("a directory");
    let row = item_8(&verdict(
        directory.path(),
        "defects found",
        &[
            "#901 menu labels cut off",
            "#902 Dialer grows with a notice",
        ],
        &[],
    ));
    assert!(row.contains("  FAIL "), "{row}");
    assert!(
        row.contains(
            "2 defects at abc1234: #901 menu labels cut off; #902 Dialer grows with a notice"
        ),
        "{row}"
    );
}

#[test]
fn a_review_of_surfaces_that_changed_since_needs_a_new_review() {
    let directory = tempfile::tempdir().expect("a directory");
    let row = item_8(&verdict(
        directory.path(),
        "no defects",
        &[],
        &[(
            "crates/desktop-calls/src",
            "0000000000000000000000000000000000000000",
        )],
    ));
    assert!(row.contains("  NEEDS-REVIEW "), "{row}");
    assert!(
        row.contains("crates/desktop-calls/src changed since the review of abc1234"),
        "{row}"
    );
}
