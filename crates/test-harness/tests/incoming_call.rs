//! Scenarios: an incoming Call rings on the Desktop and is answered,
//! declined or hung up there, against the Simulated Phone, observed through
//! the control interface and what the Simulated Phone was asked to do.
//! The surfaces' own scenarios are in `incoming_call_surfaces.rs`.

use std::time::Duration;

use test_harness::{AppState, Harness, RecordedMethodCall};

/// How long the app may take to notice a change on the Phone.
const DEADLINE: Duration = Duration::from_secs(5);
/// How long the Call audio may take to reach the Desktop after Answer
/// reaches the app, by its log (polish scorecard item 2).
const TRANSPORT_DEADLINE: Duration = Duration::from_millis(1_500);
/// How long the tests' own round trip may take: the request reaching the
/// app, and the change coming back on the control interface. Looser than
/// the scorecard targets because a loaded CI runner stretches it even when
/// the app is fast (ticket #83); the targets are asserted on the app's log.
const ROUND_TRIP: Duration = Duration::from_secs(3);
/// How long the Desktop may lag behind a Call that stopped ringing on the
/// Phone, from the moment the Phone confirms the Call is gone.
const KEEP_UP_DEADLINE: Duration = Duration::from_millis(200);
/// How long something the Phone must not be asked is watched for, after the
/// app has settled.
const HOLD: Duration = Duration::from_millis(300);

const NUMBER: &str = "+447700900123";
const TRANSPORT: &str = "org.pipewire.Telephony.AudioGatewayTransport1";
/// A number the user calls from the Phone.
const OTHER_NUMBER: &str = "+447700900456";
/// Longer than a Handoff to the Desktop may take before the user is told
/// the audio didn't move (3 s).
const HANDOFF_LIMIT: Duration = Duration::from_secs(4);

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

fn answered(harness: &Harness) -> (String, u32) {
    let (path, call) = ring(harness);
    harness.answer(call);
    harness.wait_for_app(
        "the Call is active with its audio on the Desktop",
        ROUND_TRIP,
        |state| is_live_on_desktop(state, call),
    );
    harness.assert_answered_audio_within(call, TRANSPORT_DEADLINE);
    (path, call)
}

fn is_live_on_desktop(state: &AppState, call: u32) -> bool {
    state.audio_location == "desktop"
        && state
            .calls
            .iter()
            .any(|known| known.id == call && known.state == "active")
}

/// The method calls among `recorded` that went to the Telephony service.
fn telephony(recorded: &[RecordedMethodCall]) -> Vec<RecordedMethodCall> {
    recorded
        .iter()
        .filter(|call| call.bus == "session" && call.path.starts_with("/org/pipewire/Telephony"))
        .cloned()
        .collect()
}

/// Waits until the method calls the Phone received on the Telephony service
/// satisfy `condition`, and returns them.
fn wait_for_telephony(
    harness: &Harness,
    description: &str,
    condition: impl Fn(&[RecordedMethodCall]) -> bool,
) -> Vec<RecordedMethodCall> {
    let recorded = harness
        .simulated_phone()
        .wait_for_recorded(description, DEADLINE, |recorded| {
            condition(&telephony(recorded))
        });
    telephony(&recorded)
}

/// Checks that the method calls the Phone received on the Telephony service
/// satisfy `condition` for a while.
fn telephony_holds(
    harness: &Harness,
    description: &str,
    condition: impl Fn(&[RecordedMethodCall]) -> bool,
) {
    harness
        .simulated_phone()
        .recorded_holds(description, HOLD, |recorded| {
            condition(&telephony(recorded))
        });
}

fn is_reject_sco(call: &RecordedMethodCall, value: bool) -> bool {
    call.member == "Set"
        && call.arguments.contains(TRANSPORT)
        && call.arguments.contains("RejectSCO")
        && call
            .arguments
            .contains(if value { "true" } else { "false" })
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

#[test]
fn connecting_rejects_desktop_audio_until_a_call_is_answered_here() {
    let harness = connected();
    let audio_gateway = harness.simulated_phone().audio_gateway();

    wait_for_reject_sco_on(&harness, &audio_gateway);
}

#[test]
fn the_phone_shows_connected_only_once_desktop_audio_is_rejected() {
    let harness = Harness::start();
    harness
        .simulated_phone()
        .enable_quirk("slow-property-writes");

    harness.simulated_phone().connect();

    let audio_gateway = harness.simulated_phone().audio_gateway();
    wait_for_reject_sco_on(&harness, &audio_gateway);
    // The Phone holds its reply for a second. Until then a Call could still
    // open its audio on the Desktop, so the Phone isn't ready for Calls.
    let state = harness.app_state();
    assert_eq!(state.connection_state, "connecting", "{state:?}");
    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
}

#[test]
fn starting_while_the_phone_is_connected_rejects_desktop_audio() {
    let harness = Harness::builder().phone_connected().start();
    let audio_gateway = harness.simulated_phone().audio_gateway();

    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });

    wait_for_reject_sco_on(&harness, &audio_gateway);
}

#[test]
fn an_incoming_call_rings_on_the_desktop() {
    let harness = connected();

    let _call = harness.simulated_phone().ring(NUMBER);

    let state = harness.wait_for_app("the Call rings", DEADLINE, |state| state.ringing);
    assert_eq!(state.calls.len(), 1, "{state:?}");
    let call = &state.calls[0];
    assert_eq!(call.direction, "incoming");
    assert_eq!(call.state, "ringing");
    assert_eq!(call.number, NUMBER);
    assert_eq!(call.active_since_ms, 0);
    assert_eq!(state.audio_location, "phone");
    // The Phone's refused audio attempts while ringing are not a codec.
    assert_eq!(state.codec, "");
}

#[test]
fn answering_answers_then_clears_reject_sco_then_activates_in_order() {
    let harness = connected();
    let (path, call) = ring(&harness);

    harness.answer(call);

    let state = harness.wait_for_app(
        "the Call is active with its audio on the Desktop",
        ROUND_TRIP,
        |state| is_live_on_desktop(state, call),
    );
    harness.assert_answered_audio_within(call, TRANSPORT_DEADLINE);
    assert!(!state.ringing);
    // The Phone reports an answered incoming Call as active; it stays incoming.
    assert_eq!(state.calls[0].direction, "incoming");
    assert_ne!(state.calls[0].active_since_ms, 0);

    let calls = wait_for_telephony(&harness, "the transport is activated", |calls| {
        calls
            .iter()
            .any(|recorded| recorded.interface == TRANSPORT && recorded.member == "Activate")
    });
    let answer = position(&calls, "answer the Call", |recorded| {
        recorded.path == path && recorded.member == "Answer"
    });
    let reject = position(&calls[answer..], "set RejectSCO=false", |recorded| {
        is_reject_sco(recorded, false)
    }) + answer;
    let activate = position(&calls[reject..], "Activate the transport", |recorded| {
        recorded.interface == TRANSPORT && recorded.member == "Activate"
    }) + reject;
    assert!(answer < reject && reject < activate, "{calls:#?}");
    assert!(
        harness.app_errors().is_empty(),
        "{:?}",
        harness.app_errors()
    );
}

#[test]
fn declining_hangs_up_the_ringing_call() {
    let harness = connected();
    let (path, call) = ring(&harness);

    harness.decline(call);

    let state = harness.wait_for_app("the Call is gone", DEADLINE, |state| state.calls.is_empty());
    assert!(!state.ringing);
    assert_eq!(state.audio_location, "");
    let calls = wait_for_telephony(&harness, "the ringing Call is hung up", |calls| {
        calls
            .iter()
            .any(|recorded| recorded.path == path && recorded.member == "Hangup")
    });
    assert!(
        !calls.iter().any(|recorded| recorded.member == "Answer"),
        "{calls:#?}"
    );
}

#[test]
fn hanging_up_ends_the_call_and_rejects_desktop_audio_again() {
    let harness = connected();
    let (path, call) = answered(&harness);

    harness.hang_up(call);

    let state = harness.wait_for_app("the Call is gone", DEADLINE, |state| state.calls.is_empty());
    assert_eq!(state.audio_location, "");
    wait_for_telephony(&harness, "the Call is hung up", |calls| {
        calls
            .iter()
            .any(|recorded| recorded.path == path && recorded.member == "Hangup")
    });
    wait_for_reject_sco_after(&harness, "Hangup");
    assert!(
        harness.app_errors().is_empty(),
        "{:?}",
        harness.app_errors()
    );
}

#[test]
fn a_wideband_call_reports_wideband() {
    let harness = connected();
    harness.simulated_phone().set_codec(2);

    let (_, call) = answered(&harness);

    let state = harness.wait_for_app("Codec is mSBC", DEADLINE, |state| state.codec == "msbc");
    assert!(state.wideband);
    assert!(is_live_on_desktop(&state, call));
}

#[test]
fn a_narrowband_call_is_not_wideband() {
    let harness = connected();
    harness.simulated_phone().set_codec(1);

    answered(&harness);

    let state = harness.wait_for_app("Codec is CVSD", DEADLINE, |state| state.codec == "cvsd");
    assert!(!state.wideband);
}

#[test]
fn hanging_up_falls_back_to_ofono_when_call1_hangup_does_not_exist() {
    let harness = connected();
    harness
        .simulated_phone()
        .enable_quirk("call1-hangup-missing");
    let (path, call) = answered(&harness);

    harness.hang_up(call);

    harness.wait_for_app("the Call is gone", DEADLINE, |state| state.calls.is_empty());
    wait_for_telephony(
        &harness,
        "the Call is hung up through org.ofono.VoiceCall",
        |calls| {
            calls.iter().any(|recorded| {
                recorded.path == path
                    && recorded.interface == "org.ofono.VoiceCall"
                    && recorded.member == "Hangup"
            })
        },
    );
    assert!(
        harness.app_errors().is_empty(),
        "{:?}",
        harness.app_errors()
    );
}

#[test]
fn too_few_arguments_while_a_call_tears_down_means_it_ended() {
    let harness = connected();
    harness
        .simulated_phone()
        .enable_quirk("hangup-too-few-arguments");
    let (_, call) = answered(&harness);

    harness.hang_up(call);

    harness.wait_for_app("the Call is gone", DEADLINE, |state| state.calls.is_empty());
    assert!(
        harness.app_errors().is_empty(),
        "{:?}",
        harness.app_errors()
    );
}

#[test]
fn consecutive_calls_on_the_same_path_are_different_calls() {
    let harness = connected();
    let (first_path, first) = ring(&harness);
    harness.decline(first);
    harness.wait_for_app("the first Call is gone", DEADLINE, |state| {
        state.calls.is_empty()
    });

    let (second_path, second) = ring(&harness);

    // PipeWire reuses the lowest free path.
    assert_eq!(second_path, first_path);
    assert_ne!(second, first);
    let state = harness.app_state();
    assert_eq!(state.calls[0].state, "ringing");
    assert_eq!(state.calls[0].active_since_ms, 0);
    harness.answer(second);
    harness.wait_for_app(
        "the second Call is active on the Desktop",
        ROUND_TRIP,
        |state| is_live_on_desktop(state, second),
    );
    harness.assert_answered_audio_within(second, TRANSPORT_DEADLINE);
}

#[test]
fn a_call_that_briefly_vanishes_is_the_same_call() {
    let harness = connected();
    let (path, call) = answered(&harness);

    harness
        .simulated_phone()
        .reregister_call(&path, Duration::from_millis(800));

    let state = harness.app_holds("the Call is still there", Duration::from_secs(3), |state| {
        state.calls.iter().any(|known| known.id == call)
    });
    assert_eq!(state.calls.len(), 1, "{state:?}");
    assert_eq!(state.calls[0].state, "active");
}

#[test]
fn answering_a_call_that_ended_reports_an_error() {
    let harness = connected();

    harness.answer(42);

    let error = harness.wait_for_app_error(DEADLINE);
    assert_eq!(error.kind, "no-such-call");
    assert!(!error.message.contains("org."), "{error:?}");
}

#[test]
fn without_an_audio_server_ringing_fails_quietly_and_the_app_keeps_going() {
    // The scenario has no audio server: its runtime directory is private.
    let harness = connected();

    let (_, call) = ring(&harness);

    harness.wait_for_app_log("ringing-failed", DEADLINE);
    harness.answer(call);
    harness.wait_for_app("the Call is active on the Desktop", ROUND_TRIP, |state| {
        is_live_on_desktop(state, call)
    });
    harness.assert_answered_audio_within(call, TRANSPORT_DEADLINE);
}

#[test]
fn answered_on_the_phone_the_audio_stays_on_the_phone() {
    let harness = connected();
    let (path, call) = ring(&harness);

    harness.simulated_phone().answer_on_phone(&path);

    let state = harness.wait_for_app("the Call is active", DEADLINE, |state| {
        state
            .calls
            .iter()
            .any(|known| known.id == call && known.state == "active")
    });
    assert!(!state.ringing, "{state:?}");
    assert_eq!(state.audio_location, "phone");
    assert_eq!(state.calls[0].direction, "incoming");
    harness.app_holds(
        "the audio stays on the Phone",
        Duration::from_millis(500),
        |state| state.audio_location == "phone",
    );
    // The Desktop leaves the audio alone: RejectSCO=true keeps it on the Phone.
    telephony_holds(
        &harness,
        "the Desktop leaves the audio alone",
        leaves_audio_alone,
    );
}

#[test]
fn after_a_call_on_the_desktop_the_next_call_answered_on_the_phone_stays_on_the_phone() {
    let harness = connected();
    let (_, first) = answered(&harness);
    harness.hang_up(first);
    harness.wait_for_app("the first Call is gone", DEADLINE, |state| {
        state.calls.is_empty()
    });
    wait_for_reject_sco_after(&harness, "Hangup");
    let (path, second) = ring(&harness);

    harness.simulated_phone().answer_on_phone(&path);

    let state = harness.wait_for_app("the second Call is active", DEADLINE, |state| {
        state
            .calls
            .iter()
            .any(|known| known.id == second && known.state == "active")
    });
    assert_eq!(state.audio_location, "phone");
    harness.app_holds(
        "the audio stays on the Phone",
        Duration::from_millis(500),
        |state| state.audio_location == "phone",
    );
    // The first Call ended with the Desktop's Hangup; the second is after it.
    telephony_holds(
        &harness,
        "the Desktop leaves the second Call's audio alone",
        |calls| {
            let hung_up = calls
                .iter()
                .rposition(|recorded| recorded.member == "Hangup")
                .unwrap_or_else(|| panic!("the first Call was never hung up: {calls:#?}"));
            leaves_audio_alone(&calls[hung_up..])
        },
    );
}

#[test]
fn declined_on_the_phone_the_desktop_stops_ringing_at_once() {
    let harness = connected();
    let (path, _) = ring(&harness);

    harness.simulated_phone().decline_on_phone(&path);

    let state = harness.wait_for_app("the Desktop stops ringing", KEEP_UP_DEADLINE, |state| {
        !state.ringing && state.calls.is_empty()
    });
    assert_eq!(state.audio_location, "");
    telephony_holds(&harness, "the Desktop never hangs up", |calls| {
        !calls.iter().any(|recorded| recorded.member == "Hangup")
    });
}

#[test]
fn the_caller_hanging_up_while_ringing_stops_the_desktop_ringing_at_once() {
    let harness = connected();
    let (path, _) = ring(&harness);

    harness.simulated_phone().caller_hangs_up(&path);

    harness.wait_for_app("the Desktop stops ringing", KEEP_UP_DEADLINE, |state| {
        !state.ringing && state.calls.is_empty()
    });
    assert!(
        harness.app_errors().is_empty(),
        "{:?}",
        harness.app_errors()
    );
}

/// Answer waits behind slow Phone commands. Meanwhile the user places a
/// Call on the Phone, and the ringing caller gives up before Answer is
/// sent, so the Phone link drops it. Nothing was answered and no audio was
/// asked for: the placed Call's Call Window offers to move its audio here
/// at once, and no "audio not moved" error follows when the Handoff's limit
/// would have run out.
#[test]
fn an_answer_dropped_because_the_caller_gave_up_leaves_no_handoff_under_way() {
    let harness = Harness::builder().headless_screen().start();
    harness.simulated_phone().connect();
    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
    let (first_path, _) = answered(&harness);
    // Mute, the unmute when the Call ends and RejectSCO=true after it each
    // hold the serialized Phone commands for a second.
    harness.simulated_phone().enable_quirk("slow-property-writes");
    harness
        .set_muted(true)
        .unwrap_or_else(|error| panic!("Mute was refused: {error:?}"));
    wait_for_telephony(&harness, "the Phone is processing Mute", |calls| {
        calls
            .iter()
            .any(|call| call.member == "Set" && call.arguments.contains("MicrophoneVolume"))
    });
    harness.simulated_phone().caller_hangs_up(&first_path);
    harness.wait_for_app("the first Call is gone", DEADLINE, |state| {
        state.calls.is_empty()
    });
    let (path, call) = ring(&harness);
    harness.answer(call);
    harness.wait_for_app("the Desktop has accepted Answer", DEADLINE, |state| {
        state.surface("banner").is_none()
    });
    let _ = harness.simulated_phone().dial_on_phone(OTHER_NUMBER);
    let state = harness.wait_for_app("the Call placed on the Phone is listed", DEADLINE, |state| {
        state.calls.len() == 2
    });
    let placed = state
        .calls
        .iter()
        .find(|known| known.id != call)
        .unwrap_or_else(|| panic!("the placed Call is missing: {state:?}"))
        .id;

    harness.simulated_phone().caller_hangs_up(&path);

    harness.wait_for_app("only the placed Call is left", DEADLINE, |state| {
        state.calls.len() == 1 && state.calls[0].id == placed
    });
    harness.app_holds(
        "the placed Call offers to move its audio here, with no notice",
        HANDOFF_LIMIT,
        |state| {
            let window = state.surface("call-window");
            window.and_then(|window| window.get("handoff")).map(String::as_str)
                == Some("Move audio here")
                && window.is_some_and(|window| !window.contains_key("notice"))
        },
    );
    let calls = wait_for_telephony(&harness, "the record", |_| true);
    assert_eq!(
        calls
            .iter()
            .filter(|recorded| recorded.member == "Answer")
            .count(),
        1,
        "only the first Call was answered; a second Answer means the queue reached it before \
         the caller gave up, and this run proves nothing: {calls:#?}"
    );
    assert!(
        harness.app_errors().is_empty(),
        "{:?}",
        harness.app_errors()
    );
}

/// Waits until the Phone has been asked to set `RejectSCO=true` after the
/// last `member` call it received.
fn wait_for_reject_sco_after(harness: &Harness, member: &str) {
    wait_for_telephony(
        harness,
        &format!("RejectSCO=true follows {member}"),
        |calls| {
            calls
                .iter()
                .rposition(|recorded| recorded.member == member)
                .is_some_and(|after| {
                    calls[after..]
                        .iter()
                        .any(|recorded| is_reject_sco(recorded, true))
                })
        },
    );
}

/// Waits until the Phone has been asked to set `RejectSCO=true` on the
/// AudioGateway at `audio_gateway`.
fn wait_for_reject_sco_on(harness: &Harness, audio_gateway: &str) {
    wait_for_telephony(
        harness,
        &format!("RejectSCO=true is set on {audio_gateway}"),
        |calls| {
            calls
                .iter()
                .any(|recorded| recorded.path == audio_gateway && is_reject_sco(recorded, true))
        },
    );
}

/// Whether the Desktop neither answered nor asked for the Call's audio.
fn leaves_audio_alone(calls: &[RecordedMethodCall]) -> bool {
    !calls.iter().any(|recorded| {
        recorded.member == "Answer"
            || (recorded.interface == TRANSPORT && recorded.member == "Activate")
            || is_reject_sco(recorded, false)
    })
}
