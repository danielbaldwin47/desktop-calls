//! Scenarios 11 and 12: a Call placed on the Phone keeps its audio there,
//! and a Handoff moves it to the Desktop and back to the Phone, by the Call
//! Window's button, the control interface and the CLI. To the Desktop, a
//! Phone that ignores `Activate` once gets one more try a second later
//! (ticket #70), and one that keeps ignoring it gets a plain message after
//! 3 s and keeps the Call. To the Phone, hands-free drops and comes back
//! (ADR-0005) with the Call carrying on, and a refusal gets a plain message
//! and keeps the Call. Checked through the control interface, the Call
//! Window's `Surfaces`, screenshots, and what the Simulated Phone was asked.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use test_harness::{AppCall, AppState, Bus, Harness, RecordedMethodCall, Region, Screenshot};
use zbus::OwnedVariant;

/// How long the app may take to notice a change on the Phone.
const DEADLINE: Duration = Duration::from_secs(5);
/// How long the screen may take to show a change. One capture takes up to a
/// second on a slow runner.
const SCREEN_DEADLINE: Duration = Duration::from_secs(10);
/// How long the audio may take to reach the Desktop once the app is asked
/// (ticket #13), as the app's log records it (ticket #80).
const HANDOFF_DEADLINE: Duration = Duration::from_millis(1_500);
/// How long `desktop-calls audio here` may take, from starting the CLI to the
/// audio on the Desktop. Its slack over [`HANDOFF_DEADLINE`] absorbs the
/// CLI's start-up on a loaded runner, which took the round trip to 1.536 s
/// while the audio arrived 1 ms after `Activate` (ticket #80).
const CLI_HANDOFF_DEADLINE: Duration = Duration::from_secs(3);
/// How long something the Phone must not be asked is watched for.
const QUIET: Duration = Duration::from_secs(1);
/// How long the Desktop waits for the audio before it says it didn't come
/// (ticket #70).
const HANDOFF_LIMIT: Duration = Duration::from_secs(3);
/// How long after [`HANDOFF_LIMIT`] the app may take to report it, by its log.
const GIVE_UP_SLACK: Duration = Duration::from_millis(1_500);
/// How long the Simulated Phone takes after `Activate` to open the audio
/// link: about the slowest the Pixel took (313 ms, ticket #13).
const PHONE_OPENS_LINK: Duration = Duration::from_millis(300);
/// How long the audio may take when the Phone ignores the first `Activate`:
/// the second comes a second after the first, and the Phone takes
/// [`PHONE_OPENS_LINK`], so about 1.3 s. Waiting the full limit before
/// asking again took about 3.3 s (ticket #70).
const RETRIED_HANDOFF_DEADLINE: Duration = Duration::from_millis(2_500);
/// How long a Handoff to the Phone may take once the app is asked, until
/// the audio is on the Phone. Hands-free drops at once, so this is mostly
/// the app's own part; the Call coming back takes the Pixel up to 1.01 s
/// more (ticket #14's prototype), which the Call Window shows as "Moving
/// audio to phone…".
const TO_PHONE_DEADLINE: Duration = Duration::from_millis(500);

const NUMBER: &str = "+447700900123";
const TRANSPORT: &str = "org.pipewire.Telephony.AudioGatewayTransport1";
const TELEPHONY: &str = "org.pipewire.Telephony";
const BLUEZ: &str = "org.bluez";
const DEVICE: &str = "org.bluez.Device1";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
/// The Hands-Free Audio Gateway profile, the only one the Desktop drops.
const HANDS_FREE_PROFILE: &str = "0000111f-0000-1000-8000-00805f9b34fb";

/// The Hang up button's red.
const RED: [u8; 3] = [0xc0, 0x1c, 0x28];
/// The space between the Call Window's rows.
const ROW_GAP: f64 = 8.0;

/// The app on a screen of its own, so its surfaces show, with the Phone
/// connected.
fn connected() -> Harness {
    let harness = Harness::builder().headless_screen().omarchy_bar().start();
    harness.simulated_phone().connect();
    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
    harness
}

fn call_window(state: &AppState) -> Option<&BTreeMap<String, String>> {
    state.surface("call-window")
}

fn shown(state: &AppState, key: &str) -> Option<String> {
    call_window(state).and_then(|window| window.get(key).cloned())
}

/// The method calls among `calls` made on the Telephony service.
fn telephony(calls: &[RecordedMethodCall]) -> impl Iterator<Item = &RecordedMethodCall> {
    calls
        .iter()
        .filter(|call| call.bus == "session" && call.path.starts_with("/org/pipewire/Telephony"))
}

fn is_reject_sco(call: &RecordedMethodCall, value: bool) -> bool {
    call.member == "Set"
        && call.arguments.contains(TRANSPORT)
        && call.arguments.contains("RejectSCO")
        && call
            .arguments
            .contains(if value { "true" } else { "false" })
}

fn is_activate(call: &RecordedMethodCall) -> bool {
    call.member == "Activate"
}

/// What the Desktop asked of the audio, in order: `false` and `true` for
/// each `RejectSCO` write, `"Activate"` for each `Activate`.
fn audio_requests(calls: &[RecordedMethodCall]) -> Vec<&'static str> {
    telephony(calls)
        .filter_map(|call| {
            if is_reject_sco(call, false) {
                Some("RejectSCO=false")
            } else if is_reject_sco(call, true) {
                Some("RejectSCO=true")
            } else if is_activate(call) {
                Some("Activate")
            } else {
                None
            }
        })
        .collect()
}

/// What the Desktop asked of the audio from its first Handoff on: nothing
/// before it clears `RejectSCO`, which it never does for a Call placed on
/// the Phone until asked to move the audio.
fn handoff_requests(calls: &[RecordedMethodCall]) -> Vec<&'static str> {
    audio_requests(calls)
        .into_iter()
        .skip_while(|request| *request != "RejectSCO=false")
        .collect()
}

/// The person holding the Phone places a Call on it, and the app shows it
/// with the audio on the Phone.
fn placed_on_phone(harness: &Harness) -> AppState {
    let _call = harness.simulated_phone().dial_on_phone(NUMBER);
    harness.wait_for_app(
        "the Call Window shows the Call placed on the Phone",
        DEADLINE,
        |state| {
            state.calls.len() == 1
                && shown(state, "audio-location").as_deref() == Some("Audio on phone")
        },
    )
}

/// A Call placed on the Phone, connected, with its audio on the Phone.
fn live_on_phone(harness: &Harness) {
    placed_on_phone(harness);
    harness.simulated_phone().callee_answers();
    harness.wait_for_app("the Call is live on the Phone", DEADLINE, |state| {
        state
            .calls
            .first()
            .is_some_and(|call| call.state == "active")
            && state.audio_location == "phone"
            && shown(state, "handoff").as_deref() == Some("Move audio here")
    });
}

/// Waits until the audio is on the Desktop and the Call Window offers to
/// move it back, and checks the app's own part took no longer than
/// [`HANDOFF_DEADLINE`].
fn wait_for_audio_here(harness: &Harness) -> AppState {
    let state = harness.wait_for_app(
        "the audio is on the Desktop and the Call Window offers to move it back",
        DEADLINE,
        |state| {
            state.audio_location == "desktop"
                && shown(state, "audio-location").as_deref() == Some("Audio on this computer")
                && shown(state, "handoff").as_deref() == Some("Move audio to phone")
        },
    );
    let took = since_handoff_requested(harness, "audio-moved");
    assert!(
        took <= HANDOFF_DEADLINE,
        "the audio took {took:?} to reach the Desktop once the app was asked"
    );
    state
}

/// How long the app took from the Handoff request reaching it to its first
/// log event of `kind`, by the app's log. The tests' own clock also counts
/// the time to get the request to the app and the change back, which on a
/// loaded runner overran the targets by a few milliseconds (ticket #80).
fn since_handoff_requested(harness: &Harness, kind: &str) -> Duration {
    let requested = logged(harness, "handoff-requested");
    let then = logged(harness, kind);
    for event in [&requested, &then] {
        if let Some(location) = event.get("audio_location") {
            assert_eq!(location, "desktop", "{event}");
        }
    }
    Duration::from_micros(
        timestamp_us(&then)
            .checked_sub(timestamp_us(&requested))
            .unwrap_or_else(|| {
                panic!("{kind} came before the Handoff request: {requested} {then}")
            }),
    )
}

/// The app's first log event of `kind`, parsed.
fn logged(harness: &Harness, kind: &str) -> serde_json::Value {
    let line = harness.wait_for_app_log(&format!("\"event_kind\":\"{kind}\""), DEADLINE);
    serde_json::from_str(&line).unwrap_or_else(|error| panic!("{line:?} isn't JSON: {error}"))
}

fn timestamp_us(event: &serde_json::Value) -> u64 {
    event["timestamp_us"]
        .as_u64()
        .unwrap_or_else(|| panic!("{event} has no timestamp_us"))
}

/// Checks the Phone was asked to clear `RejectSCO`, to `Activate`, and,
/// once the audio was here, to set `RejectSCO` again, and nothing more.
fn assert_handed_off(harness: &Harness) {
    let calls = harness.simulated_phone().wait_for_recorded(
        "RejectSCO is set again once the audio is here",
        DEADLINE,
        |calls| handoff_requests(calls).len() >= 3,
    );
    assert_eq!(
        handoff_requests(&calls),
        ["RejectSCO=false", "Activate", "RejectSCO=true"],
        "{calls:#?}"
    );
}

#[test]
fn a_call_placed_on_the_phone_shows_calling_and_leaves_the_audio_there() {
    let harness = connected();

    let state = placed_on_phone(&harness);

    assert_eq!(shown(&state, "status").as_deref(), Some("Calling…"));
    assert_eq!(state.calls[0].direction, "outgoing");
    assert_eq!(state.calls[0].number, NUMBER);
    assert_eq!(state.audio_location, "phone");
    let calls = harness.simulated_phone().recorded_holds(
        "the Desktop never clears RejectSCO or activates the audio",
        QUIET,
        |calls| handoff_requests(calls).is_empty() && !telephony(calls).any(is_activate),
    );
    assert_eq!(handoff_requests(&calls), Vec::<&str>::new());
}

#[test]
fn move_audio_desktop_brings_the_audio_here_and_sets_reject_sco_again() {
    let harness = connected();
    live_on_phone(&harness);

    harness
        .move_audio("desktop")
        .expect("MoveAudio(\"desktop\") is accepted");

    wait_for_audio_here(&harness);
    assert_handed_off(&harness);
    let state = harness.app_holds("the audio stays on the Desktop", QUIET, |state| {
        state.audio_location == "desktop"
    });
    assert_eq!(state.calls.len(), 1);
}

#[test]
fn desktop_calls_audio_here_brings_the_audio_here() {
    let harness = connected();
    live_on_phone(&harness);

    let asked = Instant::now();
    let output = harness.cli(&["audio", "here"]);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "Moving the audio to this computer."
    );
    wait_for_audio_here(&harness);
    let took = asked.elapsed();
    assert!(
        took <= CLI_HANDOFF_DEADLINE,
        "the audio took {took:?} to reach the Desktop from starting the CLI"
    );
    assert_handed_off(&harness);
}

#[test]
fn a_handoff_works_while_the_call_placed_on_the_phone_still_dials() {
    let harness = connected();
    placed_on_phone(&harness);

    harness
        .move_audio("desktop")
        .expect("MoveAudio(\"desktop\") is accepted");

    let state = wait_for_audio_here(&harness);
    assert_eq!(shown(&state, "status").as_deref(), Some("Calling…"));
}

#[test]
fn an_incoming_call_answered_on_the_phone_can_move_its_audio_here() {
    let harness = connected();
    let call = harness.simulated_phone().ring(NUMBER);
    harness.wait_for_app("the Call rings", DEADLINE, |state| state.ringing);
    harness.simulated_phone().answer_on_phone(&call);
    harness.wait_for_app(
        "the Call Window shows the audio on the Phone",
        DEADLINE,
        |state| {
            state.audio_location == "phone"
                && shown(state, "handoff").as_deref() == Some("Move audio here")
        },
    );

    harness
        .move_audio("desktop")
        .expect("MoveAudio(\"desktop\") is accepted");

    wait_for_audio_here(&harness);
    assert_handed_off(&harness);
}

#[test]
fn a_phone_that_ignores_the_first_activate_is_asked_again_after_a_second() {
    let harness = connected();
    live_on_phone(&harness);
    harness
        .simulated_phone()
        .set_activate_delay(PHONE_OPENS_LINK);
    harness
        .simulated_phone()
        .enable_quirk("activate-ignored-once");

    harness
        .move_audio("desktop")
        .expect("MoveAudio(\"desktop\") is accepted");

    // The audio can't arrive before the second `Activate` a second in, so
    // the Call Window says it is coming the whole time.
    harness.app_holds("the Call Window says Moving audio here…", QUIET, |state| {
        shown(state, "handoff").as_deref() == Some("Moving audio here…")
            && state.audio_location == "phone"
    });
    let state = harness.wait_for_app("the audio reaches the Desktop", DEADLINE, |state| {
        state.audio_location == "desktop"
    });
    let took = since_handoff_requested(&harness, "audio-moved");
    assert!(
        took < RETRIED_HANDOFF_DEADLINE,
        "the audio took {took:?} to reach the Desktop once the app was asked"
    );
    assert_eq!(shown(&state, "notice"), None);
    let calls = harness.simulated_phone().wait_for_recorded(
        "RejectSCO is set again once the audio is here",
        DEADLINE,
        |calls| handoff_requests(calls).len() >= 5,
    );
    assert_eq!(
        handoff_requests(&calls),
        [
            "RejectSCO=false",
            "Activate",
            "RejectSCO=false",
            "Activate",
            "RejectSCO=true"
        ],
        "{calls:#?}"
    );
}

#[test]
fn a_phone_that_opens_the_audio_link_in_300_ms_is_asked_to_activate_once() {
    let harness = connected();
    live_on_phone(&harness);
    harness
        .simulated_phone()
        .set_activate_delay(PHONE_OPENS_LINK);

    harness
        .move_audio("desktop")
        .expect("MoveAudio(\"desktop\") is accepted");

    wait_for_audio_here(&harness);
    assert_handed_off(&harness);
    // Past the second after `Activate` at which an unstarted link is retried.
    let calls = harness.simulated_phone().recorded_holds(
        "the Phone is asked to Activate only once",
        QUIET,
        |calls| telephony(calls).filter(|call| is_activate(call)).count() == 1,
    );
    assert_eq!(
        handoff_requests(&calls),
        ["RejectSCO=false", "Activate", "RejectSCO=true"],
        "{calls:#?}"
    );
}

#[test]
fn a_phone_that_keeps_the_audio_gets_a_plain_message_and_the_call_stays_live() {
    let harness = connected();
    live_on_phone(&harness);
    harness.simulated_phone().enable_quirk("activate-ignored");

    harness
        .move_audio("desktop")
        .expect("MoveAudio(\"desktop\") is accepted");

    let state = harness.wait_for_app(
        "the Call Window says the Phone kept the audio",
        HANDOFF_LIMIT + DEADLINE,
        |state| shown(state, "notice").is_some(),
    );
    let reported = logged(&harness, "error-reported");
    assert_eq!(reported["error_kind"], "audio-not-moved", "{reported}");
    let took = since_handoff_requested(&harness, "error-reported");
    assert!(
        (HANDOFF_LIMIT..=HANDOFF_LIMIT + GIVE_UP_SLACK).contains(&took),
        "it gave up after {took:?} once the app was asked"
    );
    // Asked twice, a second apart, and never a third time.
    let calls = harness.simulated_phone().wait_for_recorded(
        "the Phone was asked twice",
        DEADLINE,
        |calls| handoff_requests(calls).len() >= 4,
    );
    assert_eq!(
        handoff_requests(&calls),
        ["RejectSCO=false", "Activate", "RejectSCO=false", "Activate"],
        "{calls:#?}"
    );
    let notice = shown(&state, "notice").unwrap_or_default();
    assert!(
        notice.contains("tap the audio button on your phone's call screen"),
        "{notice}"
    );
    assert_eq!(state.audio_location, "phone");
    assert_eq!(shown(&state, "handoff").as_deref(), Some("Move audio here"));
    let error = harness.wait_for_app_error(DEADLINE);
    assert_eq!(error.kind, "audio-not-moved");
    // The Call is still live, and the Phone was never asked to hang up.
    let state = harness.app_holds("the Call stays live", QUIET, |state| {
        state
            .calls
            .first()
            .is_some_and(|call| call.state == "active")
    });
    assert_eq!(state.calls.len(), 1);
    harness.simulated_phone().recorded_holds(
        "the Phone is never asked to hang up",
        QUIET,
        |calls| !telephony(calls).any(|call| call.member == "Hangup"),
    );
}

#[test]
fn moving_the_audio_with_no_call_is_refused_plainly() {
    let harness = connected();

    let error = harness
        .move_audio("desktop")
        .expect_err("MoveAudio is refused with no Call");

    assert!(error.kind.ends_with("NoLiveCall"), "{error:?}");
    let output = harness.cli(&["audio", "here"]);
    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr).trim(),
        error.message,
        "the CLI prints the same sentence"
    );
}

/// A live Call with its audio brought to the Desktop.
fn live_here(harness: &Harness) -> AppCall {
    live_on_phone(harness);
    if let Err(error) = harness.move_audio("desktop") {
        panic!("MoveAudio(\"desktop\") was refused: {error:?}");
    }
    let state = wait_for_audio_here(harness);
    harness.simulated_phone().wait_for_recorded(
        "RejectSCO is set again once the audio is here",
        DEADLINE,
        |calls| audio_requests(calls).last() == Some(&"RejectSCO=true"),
    );
    state.calls[0].clone()
}

/// What the Desktop asked of BlueZ's `Device1`, in order.
fn device_requests(calls: &[RecordedMethodCall]) -> Vec<&str> {
    calls
        .iter()
        .filter(|call| call.bus == "system" && call.interface == DEVICE)
        .map(|call| call.member.as_str())
        .collect()
}

/// Waits until the audio is on the Phone, the Call Window says so and
/// offers to bring it back, and checks the Call carried on as the same Call:
/// its id, its timer and its controls. Then checks the app's own part took
/// no longer than [`TO_PHONE_DEADLINE`].
fn wait_for_audio_on_phone(harness: &Harness, before: &AppCall) -> AppState {
    let state = harness.wait_for_app(
        "the audio is on the Phone and the Call Window offers to bring it back",
        DEADLINE,
        |state| {
            state.audio_location == "phone"
                && shown(state, "audio-location").as_deref() == Some("Audio on phone")
                && shown(state, "handoff").as_deref() == Some("Move audio here")
        },
    );
    assert_eq!(state.calls.len(), 1, "{:?}", state.calls);
    let call = &state.calls[0];
    assert_eq!(call.id, before.id);
    assert_eq!(call.state, "active");
    assert_eq!(call.active_since_ms, before.active_since_ms);
    assert_eq!(state.connection_state, "connected");
    let status = shown(&state, "status").unwrap_or_default();
    assert!(
        is_timer(&status),
        "the Call Window shows {status:?}, not the Call's timer"
    );
    assert_eq!(shown(&state, "call"), Some(before.id.to_string()));
    assert_eq!(shown(&state, "notice"), None);
    let took = since_last_handoff_request(harness, "phone");
    assert!(
        took <= TO_PHONE_DEADLINE,
        "the Handoff to the Phone took {took:?} once the app was asked"
    );
    state
}

/// Whether `text` reads as the Call timer, such as `0:07`.
fn is_timer(text: &str) -> bool {
    text.contains(':')
        && text
            .split(':')
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
}

/// How long the app took from its latest Handoff request, to `location`,
/// to the audio reaching `location`, by its log.
fn since_last_handoff_request(harness: &Harness, location: &str) -> Duration {
    let events = log_events(harness);
    let event = |kind: &str| {
        events
            .iter()
            .rev()
            .find(|event| event["event_kind"] == kind && event["audio_location"] == location)
            .unwrap_or_else(|| panic!("no {kind} to the {location} in the log"))
    };
    let requested = timestamp_us(event("handoff-requested"));
    let moved = timestamp_us(event("audio-moved"));
    Duration::from_micros(
        moved
            .checked_sub(requested)
            .unwrap_or_else(|| panic!("the audio moved to the {location} before it was asked")),
    )
}

/// Every event in the app's log so far, parsed.
fn log_events(harness: &Harness) -> Vec<serde_json::Value> {
    harness
        .app_log()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// A property of the Simulated Phone's objects, read as any D-Bus client
/// could.
fn phone_property(
    harness: &Harness,
    bus: Bus,
    service: &str,
    path: &str,
    interface: &str,
    name: &str,
) -> OwnedVariant {
    harness
        .call_phone_method(bus, service, path, PROPERTIES, "Get", &(interface, name))
        .body()
        .deserialize::<OwnedVariant>()
        .unwrap_or_else(|error| panic!("{interface}.{name} isn't a variant: {error}"))
}

fn device_path(harness: &Harness) -> String {
    format!(
        "/org/bluez/hci0/dev_{}",
        harness.phone().address.replace(':', "_")
    )
}

/// Checks the Desktop moved the audio to the Phone `handoffs` times by
/// dropping hands-free and connecting it again, and nothing else of BlueZ's:
/// never `Disconnect` or `RemoveDevice`, so the Phone stays connected and
/// paired; that it never asked the Phone to hang up; and that the Phone's
/// next audio link stays on the Phone (`RejectSCO` true).
fn assert_moved_to_phone(harness: &Harness, handoffs: usize) {
    let calls = harness.simulated_phone().wait_for_recorded(
        "BlueZ was asked to drop hands-free and connect it again",
        DEADLINE,
        |calls| device_requests(calls).len() >= 2 * handoffs,
    );
    assert_eq!(
        device_requests(&calls),
        ["DisconnectProfile", "ConnectProfile"].repeat(handoffs),
        "{calls:#?}"
    );
    for call in calls.iter().filter(|call| call.interface == DEVICE) {
        assert_eq!(call.path, device_path(harness), "{call:?}");
        assert_eq!(
            call.arguments,
            format!("(\"{HANDS_FREE_PROFILE}\",)"),
            "{call:?}"
        );
    }
    assert!(
        !calls.iter().any(|call| matches!(
            call.member.as_str(),
            "Hangup" | "Disconnect" | "RemoveDevice" | "Deactivate"
        )),
        "{calls:#?}"
    );
    let gateway = harness.simulated_phone().audio_gateway();
    let reject_sco = phone_property(
        harness,
        Bus::Session,
        TELEPHONY,
        &gateway,
        TRANSPORT,
        "RejectSCO",
    );
    assert_eq!(bool::try_from(reject_sco), Ok(true));
    let device = device_path(harness);
    for name in ["Paired", "Connected"] {
        let value = phone_property(harness, Bus::System, BLUEZ, &device, DEVICE, name);
        assert_eq!(bool::try_from(value), Ok(true), "Device1.{name}");
    }
}

#[test]
fn move_audio_phone_moves_the_audio_to_the_phone_and_keeps_the_call() {
    let harness = connected();
    let before = live_here(&harness);

    harness
        .move_audio("phone")
        .expect("MoveAudio(\"phone\") is accepted");

    wait_for_audio_on_phone(&harness, &before);
    assert_moved_to_phone(&harness, 1);
    let state = harness.app_holds("the audio stays on the Phone", QUIET, |state| {
        state.audio_location == "phone" && state.calls.len() == 1
    });
    assert_eq!(state.calls[0].state, "active");
}

#[test]
fn desktop_calls_audio_phone_moves_the_audio_to_the_phone() {
    let harness = connected();
    let before = live_here(&harness);

    let output = harness.cli(&["audio", "phone"]);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "Moving the audio to your phone."
    );
    wait_for_audio_on_phone(&harness, &before);
    assert_moved_to_phone(&harness, 1);
}

/// Scenario 12.
#[test]
fn the_audio_goes_here_to_the_phone_here_and_to_the_phone_again_in_one_call() {
    let harness = connected();
    let before = live_here(&harness);

    harness
        .move_audio("phone")
        .expect("MoveAudio(\"phone\") is accepted");
    wait_for_audio_on_phone(&harness, &before);
    harness
        .move_audio("desktop")
        .expect("MoveAudio(\"desktop\") is accepted on the AudioGateway that came back");
    harness.wait_for_app("the audio is back on the Desktop", DEADLINE, |state| {
        state.audio_location == "desktop"
            && shown(state, "handoff").as_deref() == Some("Move audio to phone")
    });
    let took = since_last_handoff_request(&harness, "desktop");
    assert!(
        took <= HANDOFF_DEADLINE,
        "the second Handoff here took {took:?} once the app was asked"
    );
    harness
        .move_audio("phone")
        .expect("MoveAudio(\"phone\") is accepted again");

    wait_for_audio_on_phone(&harness, &before);
    assert_moved_to_phone(&harness, 2);
    let calls = harness
        .simulated_phone()
        .wait_for_recorded("anything", DEADLINE, |_| true);
    assert_eq!(
        handoff_requests(&calls)
            .into_iter()
            .filter(|request| *request != "RejectSCO=true")
            .collect::<Vec<_>>(),
        ["RejectSCO=false", "Activate", "RejectSCO=false", "Activate"],
        "two Handoffs here, each asked once: {calls:#?}"
    );
}

/// The "no `Deactivate()`" quirk: PipeWire's transport has no way to close
/// the audio link, so the app never looks for one.
#[test]
fn the_transport_has_no_deactivate_and_the_handoff_to_the_phone_needs_none() {
    let harness = connected();
    let before = live_here(&harness);
    let gateway = harness.simulated_phone().audio_gateway();
    let introspection: String = harness
        .call_phone_method(
            Bus::Session,
            TELEPHONY,
            &gateway,
            "org.freedesktop.DBus.Introspectable",
            "Introspect",
            &(),
        )
        .body()
        .deserialize()
        .expect("Introspect replies with XML");
    assert!(introspection.contains(TRANSPORT), "{introspection}");
    assert!(!introspection.contains("Deactivate"), "{introspection}");

    harness
        .move_audio("phone")
        .expect("MoveAudio(\"phone\") is accepted");

    wait_for_audio_on_phone(&harness, &before);
    assert_moved_to_phone(&harness, 1);
}

#[test]
fn a_handoff_to_the_phone_bluez_refuses_says_how_to_do_it_on_the_phone() {
    let harness = connected();
    let before = live_here(&harness);
    harness
        .simulated_phone()
        .enable_quirk("disconnect-profile-refused");

    harness
        .move_audio("phone")
        .expect("MoveAudio(\"phone\") is accepted");

    let state = harness.wait_for_app(
        "the Call Window says the audio stayed here",
        DEADLINE,
        |state| shown(state, "notice").is_some(),
    );
    assert_eq!(
        shown(&state, "notice").as_deref(),
        Some(
            "The audio stayed here; to move it to your phone, tap the audio button on your phone's call screen."
        )
    );
    assert_eq!(state.audio_location, "desktop");
    assert_eq!(
        shown(&state, "handoff").as_deref(),
        Some("Move audio to phone")
    );
    let error = harness.wait_for_app_error(DEADLINE);
    assert_eq!(error.kind, "audio-not-moved-to-phone");
    let state = harness.app_holds("the Call stays live", QUIET, |state| {
        state.calls.len() == 1 && state.calls[0].state == "active"
    });
    assert_eq!(state.calls[0].id, before.id);
    let calls = harness
        .simulated_phone()
        .wait_for_recorded("anything", DEADLINE, |_| true);
    assert_eq!(device_requests(&calls), ["DisconnectProfile"], "{calls:#?}");
    assert!(
        !telephony(&calls).any(|call| call.member == "Hangup"),
        "{calls:#?}"
    );
}

#[test]
fn hanging_up_while_hands_free_is_away_ends_the_call_once_it_is_back() {
    let harness = connected();
    let before = live_here(&harness);
    harness
        .move_audio("phone")
        .expect("MoveAudio(\"phone\") is accepted");
    harness
        .simulated_phone()
        .wait_for_recorded("hands-free has dropped", DEADLINE, |calls| {
            device_requests(calls).contains(&"DisconnectProfile")
        });

    harness.hang_up(before.id);

    harness.simulated_phone().wait_for_recorded(
        "the Phone is asked to hang up the Call once it is back",
        DEADLINE,
        |calls| telephony(calls).any(|call| call.member == "Hangup"),
    );
    let state = harness.wait_for_app("the Call has ended", DEADLINE, |state| {
        state.calls.is_empty()
    });
    assert_eq!(state.connection_state, "connected");
}

fn close_to(colour: [u8; 3], target: [u8; 3]) -> bool {
    colour
        .iter()
        .zip(target)
        .all(|(channel, wanted)| channel.abs_diff(wanted) <= 16)
}

fn find(screenshot: &Screenshot, colour: [u8; 3]) -> Option<Region> {
    screenshot.region_where(|pixel| close_to(pixel, colour))
}

/// Where the Hang up button is, once the screen shows it.
fn hang_up_button(harness: &Harness, name: &str) -> Region {
    let screenshot = harness.screenshot_when(name, SCREEN_DEADLINE, |screenshot| {
        find(screenshot, RED).is_some()
    });
    find(&screenshot, RED).unwrap_or_else(|| panic!("Hang up left the screen"))
}

/// The Handoff button: at the right of the audio row, two rows above Hang
/// up, as tall as it.
fn handoff_button_centre(hang_up: Region) -> (f64, f64) {
    let height = hang_up.bottom - hang_up.top;
    (
        hang_up.right - 24.0,
        hang_up.top - 2.0 * (ROW_GAP + height) + height / 2.0,
    )
}

#[test]
fn clicking_the_handoff_button_moves_the_audio_here_and_back_to_the_phone() {
    let harness = connected();
    live_on_phone(&harness);
    let before = harness.app_state().calls[0].clone();
    let hang_up = hang_up_button(&harness, "call-window-audio-on-phone");
    let (x, y) = handoff_button_centre(hang_up);

    harness.pointer().click(x, y);

    wait_for_audio_here(&harness);
    assert_handed_off(&harness);
    // Hang up stayed on its row, and the window on the right edge, so the
    // Handoff button is still under the pointer. The window narrows as the
    // note on why Mute is disabled goes.
    let moved = hang_up_button(&harness, "call-window-audio-here");
    assert!(
        (moved.top - hang_up.top).abs() < 1.0 && (moved.right - hang_up.right).abs() < 1.0,
        "Hang up moved from {hang_up:?} to {moved:?}"
    );

    harness.pointer().click(x, y);

    wait_for_audio_on_phone(&harness, &before);
    assert_moved_to_phone(&harness, 1);
    let back = hang_up_button(&harness, "call-window-audio-back-on-phone");
    assert!(
        (back.top - hang_up.top).abs() < 1.0 && (back.right - hang_up.right).abs() < 1.0,
        "Hang up moved from {hang_up:?} to {back:?}"
    );
}
