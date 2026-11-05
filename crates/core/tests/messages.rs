//! The Messages module: every sentence the user reads.

use desktop_calls_core::messages::{self, Failure, Label};
use desktop_calls_core::{
    CallId, Caller, ConnectionState, ContactMatch, MissedCallNotice, NumberLabel, Phone, Timestamp,
};

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

fn missed_call(caller: Caller, contact: Option<ContactMatch>, count: u32) -> MissedCallNotice {
    MissedCallNotice {
        notification: CallId(1),
        call: CallId(count),
        caller,
        contact,
        count,
        // 08:30 UTC today.
        at: Timestamp(1_793_867_400_000),
    }
}

fn alice() -> ContactMatch {
    ContactMatch {
        name: "Alice".to_owned(),
        label: NumberLabel::Mobile,
    }
}

fn alices_number() -> Caller {
    Caller::Number("+447700900123".to_owned())
}

#[test]
fn a_missed_call_notification_names_the_caller_and_says_when() {
    let text = messages::missed_call(&missed_call(alices_number(), Some(alice()), 1), NOW, 0);

    assert_eq!(text.summary, "Missed call from Alice (mobile)");
    assert_eq!(text.body, "08:30");
}

#[test]
fn a_missed_call_from_someone_not_in_contacts_shows_the_number() {
    let text = messages::missed_call(&missed_call(alices_number(), None, 1), NOW, 0);

    assert_eq!(text.summary, "Missed call from +44 7700 900123");
}

#[test]
fn a_withheld_missed_call_says_it_cant_be_called_back() {
    let text = messages::missed_call(&missed_call(Caller::Withheld, None, 1), NOW, 0);

    assert_eq!(text.summary, "Missed call from a withheld number");
    assert_eq!(text.body, "08:30 · can't be called back");
}

#[test]
fn repeat_missed_calls_are_counted_with_the_latest_time() {
    let text = messages::missed_call(&missed_call(alices_number(), Some(alice()), 3), NOW, 0);
    assert_eq!(text.summary, "3 missed calls from Alice");
    assert_eq!(text.body, "Latest 08:30");

    let text = messages::missed_call(&missed_call(Caller::Withheld, None, 2), NOW, 0);
    assert_eq!(text.summary, "2 missed calls from withheld numbers");
    assert_eq!(text.body, "Latest 08:30 · can't be called back");
}

#[test]
fn missed_call_notifications_never_look_like_a_verification_code() {
    // Omarchy 4.5 lets a notification through Do Not Disturb if it finds 4
    // to 8 digits near a word such as code, pin, verify or confirm.
    const CODE_WORDS: [&str; 12] = [
        "code",
        "otp",
        "one-time",
        "verification",
        "verify",
        "passcode",
        "pin",
        "2fa",
        "two-factor",
        "security key",
        "confirm",
        "confirmation",
    ];
    for notice in [
        missed_call(alices_number(), Some(alice()), 1),
        missed_call(alices_number(), None, 1),
        missed_call(alices_number(), None, 3),
        missed_call(Caller::Withheld, None, 1),
        missed_call(Caller::Withheld, None, 2),
    ] {
        let text = messages::missed_call(&notice, NOW, 0);
        let all = format!("{} {}", text.summary, text.body).to_lowercase();
        for word in CODE_WORDS {
            assert!(!all.contains(word), "{all:?} contains {word:?}");
        }
    }
    for label in [Label::CallBack, Label::Dismiss, Label::ShowInDialer] {
        let label = messages::label(label).to_lowercase();
        assert!(
            !CODE_WORDS.iter().any(|word| label.contains(word)),
            "{label}"
        );
    }
}
