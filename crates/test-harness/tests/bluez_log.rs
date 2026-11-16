//! The app's debug lines of each method call it makes to BlueZ, BlueZ's
//! reply and its error, for the Real-Phone Checklist: without root the
//! system bus refuses an outside monitor, so nothing else can see them
//! (#422). They are on only when `RUST_LOG` asks for them, and never name
//! the Phone.

use std::time::Duration;

use test_harness::Harness;

/// The app pages the Phone, out of range, as it starts; the Simulated
/// Phone's BlueZ fails the page after its page timeout.
const DEADLINE: Duration = Duration::from_secs(15);
/// A Phone name with a number in it, which no line may carry.
const PHONE_NAME: &str = "Ann's Pixel 5551234567";

fn bluez_lines(log: &str) -> Vec<&str> {
    log.lines()
        .filter(|line| line.contains(r#""event_kind":"bluez-method-"#))
        .collect()
}

#[test]
fn each_bluez_method_call_reply_and_error_is_logged_without_the_phones_address_or_name() {
    let harness = Harness::builder()
        .phone_name(PHONE_NAME)
        .log_filter(Some("info,bluez_dbus=debug"))
        .start();
    let error = harness.wait_for_app_log(r#""event_kind":"bluez-method-failed""#, DEADLINE);
    for field in [
        r#""level":"DEBUG""#,
        r#""object":"device""#,
        r#""interface":"org.bluez.Device1""#,
        r#""method":"Connect""#,
        r#""detail":"org.bluez.Error.Failed: br-connection-page-timeout""#,
    ] {
        assert!(error.contains(field), "{field} isn't in {error}");
    }

    let log = harness.app_log();
    let lines = bluez_lines(&log);
    let called = |method: &str| {
        lines.iter().any(|line| {
            line.contains(r#""event_kind":"bluez-method-called""#)
                && line.contains(&format!(r#""method":"{method}""#))
        })
    };
    assert!(called("Connect"), "{lines:#?}");
    assert!(
        lines.iter().any(
            |line| line.contains(r#""event_kind":"bluez-method-replied""#)
                && line.contains(r#""method":"GetManagedObjects""#)
                && line.contains(r#""object":"root""#)
        ),
        "{lines:#?}"
    );

    let address = &harness.phone().address;
    let in_a_path = address.replace(':', "_");
    for line in &lines {
        assert!(
            !line.contains(address.as_str()) && !line.contains(&in_a_path),
            "the Phone's address is in {line}"
        );
        assert!(
            !line.contains("Ann") && !line.contains("5551234567"),
            "the Phone's name is in {line}"
        );
    }

    // The switch turns on these lines and no other debug line.
    let other_debug: Vec<&str> = log
        .lines()
        .filter(|line| line.contains(r#""level":"DEBUG""#) && !lines.contains(line))
        .collect();
    assert!(other_debug.is_empty(), "{other_debug:#?}");
}

#[test]
fn a_run_without_rust_log_logs_no_bluez_method_call() {
    let harness = Harness::builder().log_filter(None).start();
    // At info: the page BlueZ failed.
    harness.wait_for_app_log("br-connection-page-timeout", DEADLINE);
    let log = harness.app_log();
    assert!(bluez_lines(&log).is_empty(), "{log}");
}
