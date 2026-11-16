//! Scenarios for ticket #152: while the Desktop has lost hands-free contact
//! with the Phone, `desktop-calls status` and a Call the user asks for say
//! why in the words the Call Window and the Indicator use, never that the
//! Phone is out of range or has Bluetooth off. Contact is lost with a live
//! Call both ways: hands-free not coming back after a Handoff to the Phone
//! (#101), and the Phone going out of range (#19); and with no Call, by
//! hands-free dropped from elsewhere with the Phone still connected (#216).

use std::time::Duration;

use test_harness::{AppState, Bus, Harness};

/// How long the app may take to notice a change on the Phone.
const DEADLINE: Duration = Duration::from_secs(5);
/// How long the lost Call Window stays before it closes, with some slack.
const LOST_WINDOW_DEADLINE: Duration = Duration::from_secs(14);

const PHONE_NAME: &str = "My Pixel";
const BOB: &str = "+447700900123";
/// The Phone's side of hands-free.
const HANDS_FREE_AUDIO_GATEWAY_UUID: &str = "0000111f-0000-1000-8000-00805f9b34fb";

/// The Call Window's notice while contact is lost.
const LOST_NOTICE: &str = "The call continues on your phone. Hang up there.";
/// What a Call asked for while contact is lost says.
const LOST: &str = "Connection to phone lost: the call continues on your phone.";
/// What a Call asked for while the Phone is in range without hands-free
/// says.
const RECONNECTING: &str = "Phone not connected: it's reconnecting.";

/// The app on a screen of its own, so the Call Window shows, with the
/// Phone connected.
fn connected() -> Harness {
    let harness = Harness::builder()
        .headless_screen()
        .phone_name(PHONE_NAME)
        .start();
    harness.simulated_phone().connect();
    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
    harness
}

/// Bob rings and the user answers on the Desktop.
fn on_a_call(harness: &Harness) {
    let _call = harness.simulated_phone().ring(BOB);
    let state = harness.wait_for_app("Bob's Call rings", DEADLINE, |state| {
        state.ringing && state.calls.len() == 1
    });
    let call = state.calls[0].id;
    harness.answer(call);
    harness.wait_for_app("Bob's Call is live here", DEADLINE, |state| {
        state.audio_location == "desktop"
            && state
                .calls
                .iter()
                .any(|known| known.id == call && known.state == "active")
    });
}

/// Hands-free drops for a Handoff to the Phone and never comes back.
fn lose_hands_free(harness: &Harness) {
    harness
        .simulated_phone()
        .enable_quirk("connect-profile-fails");
    if let Err(error) = harness.move_audio("phone") {
        panic!("MoveAudio(\"phone\") was refused: {error:?}");
    }
}

/// Waits until the Call Window says contact with the Call is lost.
fn wait_lost(harness: &Harness) -> AppState {
    harness.wait_for_app(
        "the Call Window says the connection is lost",
        DEADLINE,
        |state| {
            state
                .surface("call-window")
                .and_then(|window| window.get("notice"))
                .is_some_and(|notice| notice == LOST_NOTICE)
        },
    )
}

/// What `desktop-calls status` prints.
fn status(harness: &Harness) -> String {
    let printed = harness.cli(&["status"]);
    assert!(printed.status.success(), "{printed:?}");
    String::from_utf8_lossy(&printed.stdout).trim().to_owned()
}

/// Asks for a Call with `desktop-calls dial`, which must be refused:
/// returns what it printed, and checks the app's `Error` signal says the
/// same, with `kind`, and that the Phone was never asked to dial.
fn dial_refused(harness: &Harness, kind: &str) -> String {
    let errors = harness.app_errors().len();
    let output = harness.cli(&["dial", "07700 900456"]);
    assert!(!output.status.success(), "{output:?}");
    let printed = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    let deadline = std::time::Instant::now() + DEADLINE;
    let error = loop {
        if let Some(error) = harness.app_errors().into_iter().nth(errors) {
            break error;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no Error signal for the refused Call"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(
        (error.kind.as_str(), error.message.as_str()),
        (kind, printed.as_str())
    );
    // The refusal is decided before anything is asked of the Phone, and
    // the CLI has returned, so what the Phone recorded by now is final.
    let dials = harness
        .simulated_phone()
        .wait_for_recorded("what the Phone was asked", DEADLINE, |_| true)
        .iter()
        .filter(|call| call.member == "Dial")
        .count();
    assert_eq!(dials, 0, "the Phone was asked to dial");
    printed
}

/// What `desktop-calls status` says while contact with a live Call is
/// lost: the Call Window's words, naming the Phone.
fn assert_status_says_lost(harness: &Harness) {
    let address = harness.phone().address.clone();
    assert_eq!(
        status(harness),
        format!("Connection to {PHONE_NAME} ({address}) lost: the call continues on your phone.")
    );
}

#[test]
fn with_hands_free_lost_during_a_call_status_and_a_call_say_contact_is_lost() {
    let harness = connected();
    on_a_call(&harness);
    lose_hands_free(&harness);
    wait_lost(&harness);

    assert_status_says_lost(&harness);
    assert_eq!(dial_refused(&harness, "connection-lost"), LOST);
}

#[test]
fn with_the_phone_out_of_range_during_a_call_status_and_a_call_say_contact_is_lost() {
    let harness = connected();
    on_a_call(&harness);
    harness.simulated_phone().disconnect();
    wait_lost(&harness);

    assert_status_says_lost(&harness);
    assert_eq!(dial_refused(&harness, "connection-lost"), LOST);
}

/// Once the lost Call Window has closed with the Phone still in range, a
/// Call asked for says the Phone is reconnecting, as the Dialer's footer
/// does, not that it is out of range.
#[test]
fn after_the_lost_call_window_closes_with_the_phone_in_range_a_call_says_it_is_reconnecting() {
    let harness = connected();
    on_a_call(&harness);
    lose_hands_free(&harness);
    wait_lost(&harness);
    harness.wait_for_app("the Call Window closes", LOST_WINDOW_DEADLINE, |state| {
        state.surface("call-window").is_none()
    });

    let address = harness.phone().address.clone();
    assert_eq!(
        status(&harness),
        format!(
            "{PHONE_NAME} ({address}) is in range but not connected for calls yet. Reconnecting."
        )
    );
    assert_eq!(dial_refused(&harness, "phone-reconnecting"), RECONNECTING);
}

/// Hands-free dropped from elsewhere with no Call, the Phone still
/// connected: the Dialer's Enter says it is reconnecting, as its footer
/// does.
#[test]
fn with_hands_free_dropped_and_no_call_the_dialer_says_the_phone_is_reconnecting() {
    let harness = connected();
    harness
        .simulated_phone()
        .enable_quirk("connect-profile-fails");
    let device = format!(
        "/org/bluez/hci0/dev_{}",
        harness.phone().address.replace(':', "_")
    );
    harness.call_phone_method(
        Bus::System,
        "org.bluez",
        &device,
        "org.bluez.Device1",
        "DisconnectProfile",
        &(HANDS_FREE_AUDIO_GATEWAY_UUID,),
    );
    harness.wait_for_app("ConnectionState is disconnected", DEADLINE, |state| {
        state.connection_state == "disconnected"
    });
    let output = harness.cli(&["dialer", "07700 900456"]);
    assert!(output.status.success(), "{output:?}");
    harness.wait_for_app(
        "the Dialer has the number and the keyboard",
        DEADLINE,
        |state| {
            state.surface("dialer").is_some_and(|dialer| {
                dialer.get("focused").map(String::as_str) == Some("true")
                    && dialer.get("query").map(String::as_str) == Some("07700 900456")
            })
        },
    );

    harness.keyboard().press(test_harness::Key::Return);

    let state = harness.wait_for_app("the Dialer says why not", DEADLINE, |state| {
        state
            .surface("dialer")
            .and_then(|dialer| dialer.get("notice"))
            .is_some()
    });
    assert_eq!(
        state
            .surface("dialer")
            .and_then(|dialer| dialer.get("notice"))
            .map(String::as_str),
        Some(RECONNECTING)
    );
}
