//! The Real-Phone Checklist keeps the Bluetooth D-Bus conversation around
//! each step beside its result file (retro of 2026-11-15, candidate 6, in
//! the runner only): BlueZ's signals and objects, and the session bus's
//! messages to and from the Phone's services and the app, with Bluetooth
//! addresses hidden.

#[expect(
    dead_code,
    reason = "shared by the checklist's test files; this one uses a few"
)]
mod support;

use support::{Runner, result_file, step};

#[test]
fn a_step_keeps_its_bluetooth_conversation_with_addresses_hidden() {
    let results = tempfile::tempdir().expect("a results directory");
    let mut runner = Runner::start(&["1", "--fresh"], results.path());
    runner.quit_at = Some("1.2");
    runner.play(|_| "y".to_owned());

    let result = result_file(results.path());
    let record = step(&result, "1.1");
    let name = record["bus"].as_str().expect("1.1 names its capture");
    let captures: Vec<_> = std::fs::read_dir(results.path())
        .expect("list the results")
        .map(|entry| entry.expect("an entry").path())
        .filter(|path| path.is_dir())
        .collect();
    assert_eq!(captures.len(), 1, "one directory of captures: {captures:?}");
    let text = std::fs::read_to_string(captures[0].join(name)).expect("read the capture");

    // 1.1 forgets the pairing on both sides: BlueZ signals it, and on the
    // session bus the hands-free gateway goes and the app says so.
    assert!(text.contains("BlueZ at the step's start"), "{text}");
    assert!(text.contains("BlueZ at the step's end"), "{text}");
    assert!(text.contains("system: signal"), "{text}");
    assert!(text.contains("/org/pipewire/Telephony"), "{text}");
    assert!(
        text.contains(r#""ConnectionState": <"not-set-up">"#),
        "{text}"
    );
    assert!(!text.contains("Not captured"), "{text}");
    // The app's own BlueZ method calls, which no outside monitor hears.
    assert!(text.contains(" app: call "), "{text}");

    // 1.2, quit at its start, has no record but keeps its capture.
    assert!(
        runner.printed("The Bluetooth D-Bus capture of step 1.2, to where you quit: "),
        "{:#?}",
        runner.transcript
    );
    assert!(text.contains("dev-"), "{text}");
    assert!(!has_address(&text), "an address is left in:\n{text}");
}

/// Whether `text` holds a Bluetooth address, with `:` or `_` between its
/// bytes.
fn has_address(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.windows(17).any(|window| {
        let separator = window[2];
        (separator == b':' || separator == b'_')
            && window.iter().enumerate().all(|(index, byte)| {
                if index % 3 == 2 {
                    *byte == separator
                } else {
                    byte.is_ascii_hexdigit()
                }
            })
    })
}
