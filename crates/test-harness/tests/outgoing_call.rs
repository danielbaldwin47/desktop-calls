//! Scenarios: the user places a Call from the Desktop, by the CLI, the
//! control interface or a `tel:` link, against the Simulated Phone, observed
//! through the control interface, the app's log and what the Simulated Phone
//! was asked to do. The Call Window's own scenarios are in
//! `outgoing_call_surfaces.rs`.

use std::thread;
use std::time::{Duration, Instant};

use test_harness::{AppState, Harness, RecordedMethodCall};

/// How long the app may take to notice a change on the Phone.
const DEADLINE: Duration = Duration::from_secs(5);

/// How long a refused dial is watched for any sign of reaching the Phone.
/// The refusal is decided before the reply, so a Dial sent anyway would
/// already be on its way.
const QUIET: Duration = Duration::from_secs(1);

const TRANSPORT: &str = "org.pipewire.Telephony.AudioGatewayTransport1";

fn connected() -> Harness {
    let harness = Harness::start();
    harness.simulated_phone().connect();
    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
    harness
}

/// The method calls among `calls` made on the Telephony service.
fn telephony(calls: &[RecordedMethodCall]) -> impl Iterator<Item = &RecordedMethodCall> {
    calls
        .iter()
        .filter(|call| call.bus == "session" && call.path.starts_with("/org/pipewire/Telephony"))
}

fn allowed_audio_on_desktop(calls: &[RecordedMethodCall]) -> usize {
    telephony(calls)
        .filter(|call| is_reject_sco(call, false))
        .count()
}

fn is_reject_sco(call: &RecordedMethodCall, value: bool) -> bool {
    call.member == "Set"
        && call.arguments.contains(TRANSPORT)
        && call.arguments.contains("RejectSCO")
        && call
            .arguments
            .contains(if value { "true" } else { "false" })
}

fn is_dial(call: &RecordedMethodCall) -> bool {
    call.member == "Dial"
}

fn position(
    calls: &[RecordedMethodCall],
    what: &str,
    matches: impl Fn(&RecordedMethodCall) -> bool,
) -> usize {
    calls
        .iter()
        .position(matches)
        .unwrap_or_else(|| panic!("the Phone was never asked to {what}: {calls:#?}"))
}

/// Waits until the Phone has been asked to dial, and returns its method
/// calls on the Telephony service then.
fn wait_for_dial(harness: &Harness) -> Vec<RecordedMethodCall> {
    let calls = harness.simulated_phone().wait_for_recorded(
        "the Phone is asked to dial",
        DEADLINE,
        |calls| telephony(calls).any(is_dial),
    );
    telephony(&calls).cloned().collect()
}

/// Checks that the Phone was asked to clear `RejectSCO` and then to dial
/// exactly `number`, on the current AudioGateway.
fn assert_dialled_with_desktop_audio(harness: &Harness, number: &str) {
    let calls = wait_for_dial(harness);
    let audio_gateway = harness.simulated_phone().audio_gateway();
    let reject = position(&calls, "set RejectSCO=false", |call| {
        call.path == audio_gateway && is_reject_sco(call, false)
    });
    let dial = position(&calls, "dial", is_dial);
    assert!(reject < dial, "RejectSCO=false came after Dial: {calls:#?}");
    assert_eq!(calls[dial].path, audio_gateway);
    assert_eq!(calls[dial].arguments, format!("(\"{number}\",)"));
}

/// Checks that the Phone is never asked to dial, nor to let the audio
/// come to the Desktop more than the `allowed` times it already was.
fn assert_never_dialled(harness: &Harness, allowed: usize) {
    harness
        .simulated_phone()
        .recorded_holds("the Phone is never asked to dial", QUIET, |calls| {
            !telephony(calls).any(is_dial) && allowed_audio_on_desktop(calls) <= allowed
        });
}

fn is_outgoing(state: &AppState, call_state: &str, number: &str) -> bool {
    state.calls.len() == 1
        && state.calls[0].direction == "outgoing"
        && state.calls[0].state == call_state
        && state.calls[0].number == number
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).trim().to_owned()
}

#[test]
fn dialling_a_number_typed_with_spaces_clears_reject_sco_dials_and_brings_the_audio_here() {
    let harness = connected();

    let output = harness.cli(&["dial", "07700 900123"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stderr(&output), "");
    assert_dialled_with_desktop_audio(&harness, "07700900123");

    let _call = harness.simulated_phone().alert();
    let state = harness.wait_for_app(
        "the dialled Call rings at the other end",
        DEADLINE,
        |state| is_outgoing(state, "alerting", "07700900123"),
    );
    assert_eq!(state.audio_location, "desktop");

    harness.simulated_phone().callee_answers();
    let state = harness.wait_for_app("the dialled Call is answered", DEADLINE, |state| {
        is_outgoing(state, "active", "07700900123")
    });
    assert_eq!(state.audio_location, "desktop");
    assert_ne!(state.calls[0].active_since_ms, 0);
}

#[test]
fn a_tel_link_on_the_cli_dials_its_number() {
    let harness = connected();

    let output = harness.cli(&["dial", "tel:+447700900123"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_dialled_with_desktop_audio(&harness, "+447700900123");
}

#[test]
fn opening_a_tel_link_dials_it_through_the_desktop_file() {
    let harness = connected();

    harness.open_link("tel:+447700900123");

    assert_dialled_with_desktop_audio(&harness, "+447700900123");
}

#[test]
fn the_control_interface_dials_too() {
    let harness = connected();

    harness.dial("07700 900123").expect("the app dials");

    assert_dialled_with_desktop_audio(&harness, "07700900123");
}

#[test]
fn dialling_while_the_phone_is_out_of_range_says_so_and_never_dials() {
    let harness = connected();
    harness.simulated_phone().disconnect();
    harness.wait_for_app("ConnectionState is disconnected", DEADLINE, |state| {
        state.connection_state == "disconnected"
    });

    let output = harness.cli(&["dial", "07700 900123"]);

    assert!(!output.status.success());
    assert_eq!(
        stderr(&output),
        "Phone not connected: it's out of range or has Bluetooth off."
    );
    let error = harness.wait_for_app_error(DEADLINE);
    assert_eq!(error.kind, "phone-out-of-range");
    assert_never_dialled(&harness, 0);
}

#[test]
fn dialling_during_a_live_call_says_hang_up_first_and_never_dials() {
    let harness = connected();
    let _call = harness.simulated_phone().ring("+447700900123");
    let state = harness.wait_for_app("the Call rings", DEADLINE, |state| state.ringing);
    harness.answer(state.calls[0].id);
    harness.wait_for_app("the Call is active", DEADLINE, |state| {
        state
            .calls
            .first()
            .is_some_and(|call| call.state == "active")
    });

    let answered = harness.simulated_phone().wait_for_recorded(
        "answering lets the audio come here",
        DEADLINE,
        |calls| allowed_audio_on_desktop(calls) >= 1,
    );
    let allowed = allowed_audio_on_desktop(&answered);

    let output = harness.cli(&["dial", "01632 960001"]);

    assert!(!output.status.success());
    assert_eq!(stderr(&output), "Hang up the current Call first.");
    assert_eq!(
        harness.wait_for_app_error(DEADLINE).kind,
        "call-in-progress"
    );
    assert_never_dialled(&harness, allowed);
    assert_eq!(harness.app_state().calls.len(), 1);
}

#[test]
fn an_undialable_number_is_refused_in_plain_words_and_never_dialled() {
    let harness = connected();

    let refused = harness.dial("call mum").expect_err("the app refuses");

    assert_eq!(
        refused,
        "That isn't a number your phone can dial; use digits, spaces and + * # only."
    );
    assert_eq!(harness.wait_for_app_error(DEADLINE).kind, "undialable");
    assert_never_dialled(&harness, 0);
}

#[test]
fn each_phone_refusal_is_one_plain_sentence_and_the_raw_detail_is_only_in_the_log() {
    // The Simulated Phone's error, the kind the app signals, and the raw
    // detail the Phone gave, which PipeWire passes on as the message.
    let refusals = [
        (
            "invalid-format",
            "number-refused",
            "Invalid phone number or tones",
        ),
        (
            "invalid-state",
            "phone-busy",
            "The current state does not allow this method call",
        ),
        ("in-progress", "phone-busy", "Command already in progress"),
        (
            "not-supported",
            "dial-not-supported",
            "Method is not supported on this Audio Gateway",
        ),
        ("failed", "dial-failed", "Method call failed"),
        ("cme:30", "no-signal", "no network service"),
        (
            "cme:32",
            "no-signal",
            "network not allowed - Emergency calls only",
        ),
        ("cme:31", "network-error", "network Timeout"),
        (
            "cme:27",
            "number-refused",
            "invalid characters in dial string",
        ),
        ("cme:13", "dial-failed", "SIM failure"),
    ];
    let harness = connected();

    for (count, (error, kind, detail)) in refusals.into_iter().enumerate() {
        harness.simulated_phone().refuse_next_dial(error);

        let output = harness.cli(&["dial", "07700 900123"]);

        assert!(output.status.success(), "{error}: {}", stderr(&output));
        let started = Instant::now();
        let signalled = loop {
            if let Some(signalled) = harness.app_errors().get(count).cloned() {
                break signalled;
            }
            assert!(started.elapsed() < DEADLINE, "{error}: no Error signal");
            thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(signalled.kind, kind, "{error}");
        assert!(
            signalled.message.ends_with('.') && signalled.message.matches(". ").count() == 0,
            "{error}: not one sentence: {:?}",
            signalled.message
        );
        assert!(
            !signalled.message.contains(detail),
            "{error}: the raw detail reached the user: {:?}",
            signalled.message
        );
        let logged = harness.wait_for_app_log(detail, DEADLINE);
        assert!(logged.contains("phone-command-failed"), "{logged}");
    }
}
