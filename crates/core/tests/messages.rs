//! The Messages module: every sentence the user reads.

use desktop_calls_core::messages::{self, Failure};
use desktop_calls_core::{ConnectionState, Phone};

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
