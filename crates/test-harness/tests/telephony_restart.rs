//! Scenario 19: the Telephony service restarts, as it does when
//! `WirePlumber` restarts, while idle and mid-Call. The service leaves the bus
//! and comes back under a new owner with the same objects at the same paths,
//! as on the real Pixel (`docs/wiki/telephony-api-quirks.md`). The real
//! restart is Real-Phone Checklist group 6.

use std::collections::BTreeMap;
use std::time::Duration;

use test_harness::{AppState, Builder, Harness, RecordedMethodCall};

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
/// How long Telephony was off the bus when `WirePlumber` restarted on the
/// real Pixel.
const GAP: Duration = Duration::from_millis(730);
/// Long enough for the caller to hang up while the service is away.
const ENDS_MEANWHILE_GAP: Duration = Duration::from_secs(2);
/// Long enough for the restart and the app's recovery from it.
const RESTART: Duration = Duration::from_secs(3);
/// How long a Call that ended is watched for coming back: longer than the
/// 2 s the Simulated Phone's `calls-listed-late` quirk takes to list the
/// Calls after the AudioGateway.
const LISTED_LATE_WATCH: Duration = Duration::from_secs(3);

/// Longer than the 10 s the Desktop waits for the Telephony service to come
/// back with its Calls.
const OUTLASTS_THE_WAIT: Duration = Duration::from_secs(12);
/// How long the Desktop may take to give up on a service that hasn't come
/// back: the 10 s it waits, and then as long as any change may take.
const SERVICE_GIVEN_UP: Duration = Duration::from_secs(15);
/// What the Call Window says when the Hang up never reached the Phone.
const NOT_HUNG_UP: &str = "Your phone didn't hang up the Call; hang up on your phone instead.";

const NUMBER: &str = "+447700900123";
const TRANSPORT: &str = "org.pipewire.Telephony.AudioGatewayTransport1";

fn connected(builder: Builder) -> Harness {
    let harness = builder.start();
    harness.simulated_phone().connect();
    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
    harness
}

/// A Call answered on the Desktop, live with its audio here. Returns its
/// object path on the Phone and its app-assigned id.
fn answered_here(harness: &Harness) -> (String, u32) {
    let path = harness.simulated_phone().ring(NUMBER);
    let state = harness.wait_for_app("the Call rings", DEADLINE, |state| {
        state.ringing && state.calls.len() == 1
    });
    let call = state.calls[0].id;
    harness.answer(call);
    harness.wait_for_app(
        "the Call is live with its audio on the Desktop",
        ROUND_TRIP,
        |state| has_live_call(state, call) && state.audio_location == "desktop",
    );
    harness.assert_answered_audio_within(call, TRANSPORT_DEADLINE);
    (path, call)
}

fn has_live_call(state: &AppState, call: u32) -> bool {
    state
        .calls
        .iter()
        .any(|known| known.id == call && known.state == "active")
}

/// How many method calls in `recorded` match `member` and contain all of
/// `words` in their arguments.
fn count(recorded: &[RecordedMethodCall], member: &str, words: &[&str]) -> usize {
    recorded
        .iter()
        .filter(|call| {
            call.bus == "session"
                && call.member == member
                && words.iter().all(|word| call.arguments.contains(word))
        })
        .count()
}

/// How many times the Phone was asked to hang up the Call at `path`.
fn hang_ups(recorded: &[RecordedMethodCall], path: &str) -> usize {
    recorded
        .iter()
        .filter(|known| known.path == path && known.member == "Hangup")
        .count()
}

fn audio_moves_here(recorded: &[RecordedMethodCall]) -> usize {
    recorded
        .iter()
        .filter(|call| call.interface == TRANSPORT && call.member == "Activate")
        .count()
}

fn rejects_audio(recorded: &[RecordedMethodCall]) -> usize {
    count(recorded, "Set", &["RejectSCO", "true"])
}

#[test]
fn a_restart_while_idle_comes_back_connected_and_keeps_audio_on_the_phone() {
    let harness = connected(Harness::builder());

    harness.simulated_phone().restart_telephony(GAP);

    harness.wait_for_app("the Phone shows as disconnected", DEADLINE, |state| {
        state.connection_state == "disconnected"
    });
    harness.wait_for_app("the Phone is connected again", DEADLINE, |state| {
        state.connection_state == "connected"
    });
    // The new service starts with whatever its configuration says, so the
    // Desktop asks it again, as at the first connect, to keep the next
    // Call's audio on the Phone.
    harness.simulated_phone().wait_for_recorded(
        "RejectSCO set true again after the restart",
        DEADLINE,
        |recorded| rejects_audio(recorded) >= 2,
    );
}

#[test]
fn a_restart_mid_call_keeps_the_call_in_the_call_window_and_hang_up_still_works() {
    let harness = connected(Harness::builder().headless_screen());
    let (path, call) = answered_here(&harness);
    let active_since = harness.app_state().calls[0].active_since_ms;

    harness.simulated_phone().restart_telephony(GAP);

    let state = harness.app_holds("the Call Window shows the Call", RESTART, |state| {
        state.calls.iter().any(|known| known.id == call) && state.surface("call-window").is_some()
    });
    assert_eq!(state.calls.len(), 1, "{state:?}");
    assert_eq!(state.calls[0].active_since_ms, active_since);
    assert_eq!(state.connection_state, "connected");
    harness.hang_up(call);
    harness.wait_for_app("the Call ended", DEADLINE, |state| state.calls.is_empty());
    harness.simulated_phone().wait_for_recorded(
        "the Phone was asked to hang up",
        DEADLINE,
        |recorded| {
            recorded
                .iter()
                .any(|known| known.path == path && known.member == "Hangup")
        },
    );
    assert!(
        harness.app_errors().is_empty(),
        "{:?}",
        harness.app_errors()
    );
}

#[test]
fn a_call_whose_audio_was_here_gets_it_back_after_a_restart() {
    let harness = connected(Harness::builder());
    let (_, call) = answered_here(&harness);

    harness.simulated_phone().restart_telephony(GAP);

    harness.wait_for_app("the audio dropped with the restart", DEADLINE, |state| {
        state.audio_location == "phone"
    });
    harness.wait_for_app("the audio is back on the Desktop", DEADLINE, |state| {
        has_live_call(state, call) && state.audio_location == "desktop"
    });
}

#[test]
fn a_call_whose_audio_was_on_the_phone_keeps_it_there_after_a_restart() {
    let harness = connected(Harness::builder());
    let path = harness.simulated_phone().ring(NUMBER);
    harness.wait_for_app("the Call rings", DEADLINE, |state| state.ringing);
    harness.simulated_phone().answer_on_phone(&path);
    let state = harness.wait_for_app("the Call is live on the Phone", DEADLINE, |state| {
        state.calls.len() == 1 && state.calls[0].state == "active"
    });
    let call = state.calls[0].id;

    harness.simulated_phone().restart_telephony(GAP);

    let state = harness.app_holds("the Call stays, on the Phone", RESTART, |state| {
        has_live_call(state, call) && state.audio_location == "phone"
    });
    assert_eq!(state.connection_state, "connected");
    harness.simulated_phone().recorded_holds(
        "the Desktop never asked for the audio",
        Duration::ZERO,
        |recorded| audio_moves_here(recorded) == 0,
    );
}

#[test]
fn hang_up_while_the_service_is_away_ends_the_call_once_it_is_back() {
    let harness = connected(Harness::builder());
    let (path, call) = answered_here(&harness);

    harness.simulated_phone().restart_telephony(GAP);
    harness.wait_for_app("the Phone shows as disconnected", DEADLINE, |state| {
        state.connection_state == "disconnected"
    });
    harness.hang_up(call);

    harness.wait_for_app("the Call ended", DEADLINE, |state| state.calls.is_empty());
    harness.simulated_phone().wait_for_recorded(
        "the Phone was asked to hang up once it was back",
        DEADLINE,
        |recorded| {
            recorded
                .iter()
                .any(|known| known.path == path && known.member == "Hangup")
        },
    );
    assert!(
        harness.app_errors().is_empty(),
        "{:?}",
        harness.app_errors()
    );
}

/// Ticket #169: the restarted service lists its Calls 2 s after the
/// AudioGateway rather than 40 ms. The Hang up pressed while it was away
/// ends the Call once the Phone lists it, and the Call stays ended.
#[test]
fn hang_up_while_the_service_is_away_ends_the_call_even_when_it_is_listed_late() {
    let harness = connected(Harness::builder());
    let (path, call) = answered_here(&harness);
    harness.simulated_phone().enable_quirk("calls-listed-late");

    harness.simulated_phone().restart_telephony(GAP);
    harness.wait_for_app("the Phone shows as disconnected", DEADLINE, |state| {
        state.connection_state == "disconnected"
    });
    harness.hang_up(call);

    harness.wait_for_app("the Call ended", DEADLINE, |state| state.calls.is_empty());
    harness.simulated_phone().wait_for_recorded(
        "the Phone was asked to hang up once it was back",
        DEADLINE,
        |recorded| {
            recorded
                .iter()
                .any(|known| known.path == path && known.member == "Hangup")
        },
    );
    harness.app_holds("the Call stays ended", LISTED_LATE_WATCH, |state| {
        state.calls.is_empty()
    });
    assert_eq!(harness.simulated_phone().calls_on_phone(), []);
    assert!(
        harness.app_errors().is_empty(),
        "{:?}",
        harness.app_errors()
    );
}

/// Ticket #175: the Hang up reaches the Telephony service just as it leaves
/// for a restart, and never reaches the Phone. The Call goes without
/// `disconnected` and the new service lists it again, live. The Desktop
/// sends the Hang up again then, and the Call ends and stays ended.
#[test]
fn a_hang_up_lost_as_the_service_leaves_is_sent_again_once_the_call_is_back() {
    let harness = connected(Harness::builder());
    let (path, call) = answered_here(&harness);
    harness
        .simulated_phone()
        .enable_quirk("hangup-unanswered-once");

    harness.hang_up(call);
    harness.simulated_phone().wait_for_recorded(
        "the Hang up reached the service",
        DEADLINE,
        |recorded| hang_ups(recorded, &path) == 1,
    );
    harness.simulated_phone().restart_telephony(GAP);

    harness.simulated_phone().wait_for_recorded(
        "the Phone was asked to hang up again once the Call was back",
        DEADLINE,
        |recorded| hang_ups(recorded, &path) == 2,
    );
    harness.wait_for_app("the Call ended", DEADLINE, |state| state.calls.is_empty());
    harness.app_holds("the Call stays ended", LISTED_LATE_WATCH, |state| {
        state.calls.is_empty()
    });
    assert_eq!(harness.simulated_phone().calls_on_phone(), []);
    assert!(
        harness.app_errors().is_empty(),
        "{:?}",
        harness.app_errors()
    );
}

/// Ticket #175: the service stays away longer than the Desktop waits for
/// it, with the Hang up the user pressed meanwhile still to send. The
/// Desktop doesn't say the Call ended: the Call Window says the Phone
/// didn't hang it up. Once the service is back and lists the Call, it shows
/// again with its timer, and the Desktop doesn't hang it up by itself.
#[test]
fn a_hang_up_that_never_reaches_the_phone_says_the_call_may_still_be_live() {
    let harness = connected(Harness::builder().headless_screen());
    let (path, call) = answered_here(&harness);
    let active_since = harness.app_state().calls[0].active_since_ms;

    harness
        .simulated_phone()
        .restart_telephony(OUTLASTS_THE_WAIT);
    harness.wait_for_app("the Phone shows as disconnected", DEADLINE, |state| {
        state.connection_state == "disconnected"
    });
    harness.hang_up(call);

    let error = harness.wait_for_app_error(SERVICE_GIVEN_UP);
    assert_eq!(error.kind, "hang-up-failed", "{error:?}");
    let state = harness.wait_for_app("the Call shows again, live", DEADLINE, |state| {
        state.calls.len() == 1
            && state.calls[0].state == "active"
            && state.connection_state == "connected"
    });
    assert_eq!(state.calls[0].active_since_ms, active_since);
    let windows: Vec<BTreeMap<String, String>> = harness
        .surfaces_seen()
        .into_iter()
        .filter_map(|mut surfaces| surfaces.remove("call-window"))
        .collect();
    assert!(
        windows
            .iter()
            .any(|window| window.get("notice").map(String::as_str) == Some(NOT_HUNG_UP)),
        "{windows:#?}"
    );
    assert!(
        !windows
            .iter()
            .any(|window| window.get("status").map(String::as_str) == Some("Call ended")),
        "{windows:#?}"
    );
    harness.simulated_phone().recorded_holds(
        "the Desktop never hung up the Call by itself",
        LISTED_LATE_WATCH,
        |recorded| hang_ups(recorded, &path) == 0,
    );
}

/// The caller hangs up while the service is away, so the new service
/// doesn't list the Call. It ends on the Desktop within 3 s of hands-free
/// being back, rather than when the 10 s the service had to come back run
/// out (ADR-0007).
#[test]
fn a_call_that_ends_while_the_service_is_away_ends_here_soon_after_it_is_back() {
    let harness = connected(Harness::builder());
    let (path, call) = answered_here(&harness);

    harness.simulated_phone().restart_telephony(ENDS_MEANWHILE_GAP);
    harness.simulated_phone().caller_hangs_up(&path);
    harness.wait_for_app("the Phone is connected again", DEADLINE, |state| {
        state.connection_state == "connected"
    });

    harness.wait_for_app("the Call ended", DEADLINE, |state| {
        !state.calls.iter().any(|known| known.id == call)
    });
    harness.simulated_phone().recorded_holds(
        "the Desktop never asked to hang up",
        Duration::ZERO,
        |recorded| !recorded.iter().any(|known| known.member == "Hangup"),
    );
    assert!(
        harness.app_errors().is_empty(),
        "{:?}",
        harness.app_errors()
    );
}

#[test]
fn a_slow_restart_still_keeps_the_same_call() {
    let harness = connected(Harness::builder());
    let (_, call) = answered_here(&harness);

    // Longer than a hands-free profile bounce is allowed to take.
    harness
        .simulated_phone()
        .restart_telephony(Duration::from_secs(4));

    let state = harness.app_holds("the Call stays", Duration::from_secs(6), |state| {
        state.calls.iter().any(|known| known.id == call)
    });
    assert_eq!(state.calls.len(), 1, "{state:?}");
    assert_eq!(state.connection_state, "connected");
}

#[test]
fn a_call_muted_here_is_still_muted_after_a_restart_and_unmutes() {
    let harness = connected(Harness::builder());
    let (_, call) = answered_here(&harness);
    harness.set_muted(true).expect("mute");
    harness.wait_for_app("muted", DEADLINE, |state| state.muted);

    harness.simulated_phone().restart_telephony(GAP);

    harness.wait_for_app("the audio dropped with the restart", DEADLINE, |state| {
        state.audio_location == "phone"
    });
    let state = harness.wait_for_app("the audio is back on the Desktop", DEADLINE, |state| {
        has_live_call(state, call) && state.audio_location == "desktop"
    });
    // The Phone still has the microphone silenced.
    assert!(state.muted, "{state:?}");
    harness.set_muted(false).expect("unmute");
    harness.wait_for_app("unmuted", DEADLINE, |state| !state.muted);
}
