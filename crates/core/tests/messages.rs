//! The Messages module: every sentence the user reads.

use desktop_calls_core::messages::{self, Failure};
use desktop_calls_core::{ConnectionState, Phone, Timestamp};

fn pixel() -> Phone {
    Phone {
        name: "My Pixel".to_owned(),
        address: "AA:BB:CC:DD:EE:FF".to_owned(),
    }
}

#[test]
fn status_says_the_connection_state_and_names_the_phone() {
    assert_eq!(
        messages::status(&ConnectionState::Connected, Some(&pixel())),
        "Connected to My Pixel (AA:BB:CC:DD:EE:FF)."
    );
    assert_eq!(
        messages::status(&ConnectionState::Connecting, Some(&pixel())),
        "Connecting to My Pixel (AA:BB:CC:DD:EE:FF)."
    );
    assert_eq!(
        messages::status(&ConnectionState::Disconnected, Some(&pixel())),
        "My Pixel (AA:BB:CC:DD:EE:FF) is out of range or has Bluetooth off."
    );
    assert_eq!(
        messages::status(&ConnectionState::NotSetUp, None),
        "No Phone is set up yet."
    );
}

#[test]
fn a_failure_is_one_sentence() {
    let message = messages::failure(Failure::ServiceNotRunning);

    assert_eq!(message.text, "Desktop Calls isn't running.");
    assert!(message.next_action.is_some());
}

#[test]
fn a_caller_is_their_formatted_number_or_withheld() {
    use desktop_calls_core::Caller;

    assert_eq!(
        messages::caller(&Caller::Number("+447700900123".to_owned())),
        "+44 7700 900123"
    );
    // A number the Phone sent without a country code is shown as it came.
    assert_eq!(
        messages::caller(&Caller::Number("07700900123".to_owned())),
        "07700900123"
    );
    assert_eq!(messages::caller(&Caller::Withheld), "Withheld number");
}

#[test]
fn the_call_timer_reads_minutes_and_seconds() {
    assert_eq!(messages::elapsed(0), "0:00");
    assert_eq!(messages::elapsed(65), "1:05");
    assert_eq!(messages::elapsed(3_725), "1:02:05");
}

#[test]
fn call_failures_have_a_kind_and_never_raw_dbus_text() {
    for failure in [
        Failure::AnswerFailed,
        Failure::HangUpFailed,
        Failure::AudioNotMoved,
        Failure::HandoffToPhoneUnavailable,
        Failure::NoSuchCall,
        Failure::CallNotRinging,
    ] {
        let message = messages::failure(failure);
        assert!(message.text.ends_with('.'), "{failure:?}: {}", message.text);
        assert!(
            !message.text.contains("org."),
            "{failure:?}: {}",
            message.text
        );
        assert!(!failure.as_str().is_empty());
    }
    assert_eq!(Failure::AnswerFailed.as_str(), "answer-failed");
}

/// 2026-11-05 09:30 UTC, in milliseconds since the Unix epoch.
const NOW: Timestamp = Timestamp(1_793_871_000_000);

#[test]
fn when_a_call_was_is_the_time_today_yesterday_then_the_date() {
    let at = |ms| messages::when(Timestamp(ms), NOW, 0);
    assert_eq!(at(1_793_871_000_000 - 3_600_000), "08:30");
    assert_eq!(at(1_793_790_000_000), "Yesterday 11:00");
    assert_eq!(at(1_793_747_700_000), "3 Nov");
    assert_eq!(at(1_767_168_000_000), "31 Dec 2025");
}

#[test]
fn when_a_call_was_is_in_local_time() {
    // 23:15 UTC on 3 November is 00:15 on the 4th an hour east.
    assert_eq!(
        messages::when(Timestamp(1_793_747_700_000), NOW, 3_600),
        "Yesterday 00:15"
    );
}
