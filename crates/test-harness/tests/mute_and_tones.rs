//! Scenario 13, through the control interface and the CLI: Mute and keypad
//! tones during a Call, against the Simulated Phone. What the Call Window
//! shows, and its keyboard shortcuts, are in `call_window_keyboard.rs`.

use std::time::Duration;

use test_harness::{AppState, Harness, RecordedMethodCall};

/// How long the app may take to notice a change on the Phone.
const DEADLINE: Duration = Duration::from_secs(5);
/// How long something the Phone must not be asked is watched for, after the
/// app has settled.
const HOLD: Duration = Duration::from_millis(300);

const NUMBER: &str = "+447700900123";
const AUDIO_GATEWAY: &str = "org.pipewire.Telephony.AudioGateway1";

fn connected() -> Harness {
    let harness = Harness::start();
    harness.simulated_phone().connect();
    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
    harness
}

/// Rings the Phone and waits until the app rings. Returns the Call's object
/// path on the Phone and its app-assigned id.
fn ring(harness: &Harness) -> (String, u32) {
    let path = harness.simulated_phone().ring(NUMBER);
    let state = harness.wait_for_app("the Call rings", DEADLINE, |state| {
        state.ringing && state.calls.len() == 1
    });
    (path, state.calls[0].id)
}

fn answered_on_desktop(harness: &Harness) {
    let (_, call) = ring(harness);
    harness.answer(call);
    harness.wait_for_app(
        "the Call is active with its audio on the Desktop",
        DEADLINE,
        |state| is_live(state) && state.audio_location == "desktop",
    );
}

fn is_live(state: &AppState) -> bool {
    state.calls.iter().any(|call| call.state == "active")
}

/// The `MicrophoneVolume` values among `recorded`, in order.
fn microphone_volumes_set(recorded: &[RecordedMethodCall]) -> Vec<String> {
    recorded
        .iter()
        .filter(|call| {
            call.member == "Set"
                && call.arguments.contains(AUDIO_GATEWAY)
                && call.arguments.contains("MicrophoneVolume")
        })
        .map(|call| call.arguments.clone())
        .collect()
}

/// Waits until the Phone has been asked to set as many `MicrophoneVolume`
/// values as `expected` has, and checks they are those, in order.
fn wait_for_microphone_volumes(harness: &Harness, expected: &[String]) {
    let recorded = harness.simulated_phone().wait_for_recorded(
        &format!("{} MicrophoneVolume values are set", expected.len()),
        DEADLINE,
        |recorded| microphone_volumes_set(recorded).len() >= expected.len(),
    );
    assert_eq!(microphone_volumes_set(&recorded), expected);
}

/// Checks for a while that the Phone is never asked to set a
/// `MicrophoneVolume`.
fn no_microphone_volume_is_set(harness: &Harness) {
    harness
        .simulated_phone()
        .recorded_holds("no MicrophoneVolume is set", HOLD, |recorded| {
            microphone_volumes_set(recorded).is_empty()
        });
}

/// The tones among `recorded`, one `SendTones` each, in order.
fn sent(recorded: &[RecordedMethodCall]) -> Vec<String> {
    recorded
        .iter()
        .filter(|call| call.member == "SendTones")
        .map(tones_argument)
        .collect()
}

fn tones_argument(call: &RecordedMethodCall) -> String {
    call.arguments
        .trim_start_matches("(\"")
        .trim_end_matches("\",)")
        .to_owned()
}

/// `Set(AudioGateway1, MicrophoneVolume, <byte>)` as the Simulated Phone
/// records it.
fn set_to(volume: u8) -> String {
    format!("(\"{AUDIO_GATEWAY}\", \"MicrophoneVolume\", <byte 0x{volume:02x}>)")
}

#[test]
fn set_muted_sets_the_microphone_volume_to_zero_and_back() {
    let harness = connected();
    harness.simulated_phone().set_microphone_volume(9);
    answered_on_desktop(&harness);

    harness.set_muted(true).expect("SetMuted(true) is accepted");

    harness.wait_for_app("Muted is true", DEADLINE, |state| state.muted);
    wait_for_microphone_volumes(&harness, &[set_to(0)]);

    harness
        .set_muted(false)
        .expect("SetMuted(false) is accepted");

    harness.wait_for_app("Muted is false", DEADLINE, |state| !state.muted);
    wait_for_microphone_volumes(&harness, &[set_to(0), set_to(9)]);
}

#[test]
fn muted_shows_only_what_the_phone_has_done() {
    let harness = connected();
    answered_on_desktop(&harness);
    harness
        .simulated_phone()
        .enable_quirk("slow-property-writes");

    harness.set_muted(true).expect("SetMuted(true) is accepted");

    wait_for_microphone_volumes(&harness, &[set_to(0)]);
    // The Phone holds its reply for a second: until then the other person
    // may still hear, so the Desktop must not say Muted.
    assert!(!harness.app_state().muted, "{:?}", harness.app_state());
    harness.wait_for_app("Muted is true", DEADLINE, |state| state.muted);

    harness
        .set_muted(false)
        .expect("SetMuted(false) is accepted");

    wait_for_microphone_volumes(&harness, &[set_to(0), set_to(15)]);
    // Until the Phone replies, the other person may not hear yet.
    assert!(harness.app_state().muted, "{:?}", harness.app_state());
    harness.wait_for_app("Muted is false", DEADLINE, |state| !state.muted);
}

#[test]
fn unmuting_after_a_microphone_volume_of_minus_one_restores_the_default_not_minus_one() {
    let harness = connected();
    // PipeWire's -1 ("the Phone hasn't said") goes out as the byte 255.
    harness.simulated_phone().set_microphone_volume(255);
    answered_on_desktop(&harness);

    harness.set_muted(true).expect("SetMuted(true) is accepted");
    harness.wait_for_app("Muted is true", DEADLINE, |state| state.muted);
    harness
        .set_muted(false)
        .expect("SetMuted(false) is accepted");
    harness.wait_for_app("Muted is false", DEADLINE, |state| !state.muted);

    wait_for_microphone_volumes(&harness, &[set_to(0), set_to(15)]);
}

#[test]
fn with_the_audio_on_the_phone_set_muted_returns_a_plain_error() {
    let harness = connected();
    let (path, _) = ring(&harness);
    harness.simulated_phone().answer_on_phone(&path);
    harness.wait_for_app("the Call is live on the Phone", DEADLINE, |state| {
        is_live(state) && state.audio_location == "phone"
    });

    let refused = harness
        .set_muted(true)
        .expect_err("SetMuted(true) is refused");

    assert_eq!(
        refused.message,
        "Mute works only while the Call's audio is on this computer; mute on your phone instead."
    );
    assert!(!harness.app_state().muted);
    no_microphone_volume_is_set(&harness);
}

#[test]
fn desktop_calls_mute_toggles_mute() {
    let harness = connected();
    answered_on_desktop(&harness);

    let muted = harness.cli(&["mute"]);

    assert!(muted.status.success(), "{muted:?}");
    harness.wait_for_app("Muted is true", DEADLINE, |state| state.muted);

    let unmuted = harness.cli(&["mute"]);

    assert!(unmuted.status.success(), "{unmuted:?}");
    harness.wait_for_app("Muted is false", DEADLINE, |state| !state.muted);
    wait_for_microphone_volumes(&harness, &[set_to(0), set_to(15)]);
}

#[test]
fn desktop_calls_mute_with_the_audio_on_the_phone_says_why_not() {
    let harness = connected();
    let (path, _) = ring(&harness);
    harness.simulated_phone().answer_on_phone(&path);
    harness.wait_for_app("the Call is live on the Phone", DEADLINE, is_live);

    let printed = harness.cli(&["mute"]);

    assert!(!printed.status.success());
    assert_eq!(
        String::from_utf8_lossy(&printed.stderr).trim(),
        "Mute works only while the Call's audio is on this computer; mute on your phone instead."
    );
}

#[test]
fn send_tones_plays_each_tone_on_the_phone_in_order() {
    let harness = connected();
    answered_on_desktop(&harness);

    harness.send_tones("1234#").expect("SendTones is accepted");

    // A Pixel takes one tone per AT+VTS, so each goes in a SendTones of its
    // own (docs/research/2026-11-04-mute-and-keypad.md).
    let recorded =
        harness
            .simulated_phone()
            .wait_for_recorded("five tones are sent", DEADLINE, |recorded| {
                sent(recorded).len() >= 5
            });
    assert_eq!(sent(&recorded), ["1", "2", "3", "4", "#"]);
    assert!(
        harness.app_errors().is_empty(),
        "{:?}",
        harness.app_errors()
    );
}

#[test]
fn send_tones_with_letters_returns_a_plain_error() {
    let harness = connected();
    answered_on_desktop(&harness);

    let refused = harness
        .send_tones("12ab")
        .expect_err("SendTones is refused");

    assert_eq!(
        refused.message,
        "Only digits, * and # can be sent as tones."
    );
    harness
        .simulated_phone()
        .recorded_holds("no tone is sent", HOLD, |recorded| {
            sent(recorded).is_empty()
        });
}
