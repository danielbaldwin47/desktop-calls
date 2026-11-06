//! Scenario 19: the Telephony service restarts, as it does when
//! `WirePlumber` restarts, while idle and mid-Call. The service leaves the bus
//! and comes back under a new owner with the same objects at the same paths,
//! as on the real Pixel (`docs/wiki/telephony-api-quirks.md`). The real
//! restart is Real-Phone Checklist group 6.

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
