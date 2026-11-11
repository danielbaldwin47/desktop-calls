//! The Call state machine while the Phone goes out of range and comes back,
//! at its public interface: connecting it again with waits capped short of
//! the 10 s promise, at once on resume, the pairing lost, and a Call that
//! loses the Bluetooth connection mid-Call.

use desktop_calls_core::{
    CallId, CallState, CallWindowPhase, Caller, ConnectFailure, ConnectionState, Effect, Input,
    Phone, PhoneCommand, State, Timer, Timestamp,
    messages::{Failure, ProblemKind, indicator_tooltip},
    step,
};

const ADDRESS: &str = "AA:BB:CC:DD:EE:FF";
const CALL: CallId = CallId(1);

fn run(state: State, inputs: impl IntoIterator<Item = Input>) -> (State, Vec<Effect>) {
    let mut state = state;
    let mut effects = Vec::new();
    for input in inputs {
        let (next, mut more) = step(state, input);
        state = next;
        effects.append(&mut more);
    }
    (state, effects)
}

fn paired() -> Input {
    Input::PhonePaired(Phone {
        name: "My Pixel".to_owned(),
        address: ADDRESS.to_owned(),
    })
}

fn present() -> Input {
    Input::PhonePresent {
        address: ADDRESS.to_owned(),
    }
}

fn absent() -> Input {
    Input::PhoneAbsent {
        address: ADDRESS.to_owned(),
    }
}

fn hands_free_up() -> [Input; 2] {
    [
        Input::PhoneConnected {
            address: ADDRESS.to_owned(),
        },
        Input::PhoneCommandDone(PhoneCommand::KeepAudioOnPhone),
    ]
}

fn hands_free_down() -> Input {
    Input::PhoneDisconnected {
        address: ADDRESS.to_owned(),
    }
}

fn connected() -> State {
    let (state, _) = run(State::default(), [paired(), present()]);
    run(state, hands_free_up()).0
}

/// The waits for connecting again the effects started, in milliseconds.
fn reconnect_waits(effects: &[Effect]) -> Vec<(Timer, u32)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::StartTimer {
                timer: timer @ Timer::Reconnect(_),
                after_ms,
            } => Some((*timer, *after_ms)),
            _ => None,
        })
        .collect()
}

fn connects(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(
            |effect| matches!(effect, Effect::ConnectPhone { address, .. } if address == ADDRESS),
        )
        .count()
}

fn tooltip(state: &State) -> String {
    indicator_tooltip(&state.indicator(&state.surfaces()))
}

#[test]
fn a_paired_phone_absent_at_startup_is_connected_at_once() {
    let (state, effects) = run(State::default(), [paired(), absent()]);

    assert_eq!(connects(&effects), 1);
    assert_eq!(state.connection_state(), &ConnectionState::Connecting);
}

#[test]
fn a_phone_present_at_startup_is_left_to_connect_by_itself() {
    let (_, effects) = run(State::default(), [paired(), present()]);

    assert_eq!(connects(&effects), 0);
}

#[test]
fn going_out_of_range_is_quiet_and_says_so_on_the_indicator() {
    let (state, effects) = run(connected(), [hands_free_down(), absent()]);

    assert_eq!(state.connection_state(), &ConnectionState::Disconnected);
    assert_eq!(tooltip(&state), "My Pixel out of range");
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::ReportError(_) | Effect::NotifyMissedCall(_))),
        "{effects:?}"
    );
    assert_eq!(connects(&effects), 1);
}

#[test]
fn a_phone_that_doesnt_answer_is_tried_again_after_waits_capped_at_3_s() {
    let (mut state, _) = run(connected(), [hands_free_down(), absent()]);
    let mut waits = Vec::new();
    for attempt in 1..=4 {
        let (next, effects) = step(
            state,
            Input::ConnectPhoneFailed {
                attempt,
                failure: ConnectFailure::Unreachable,
            },
        );
        let [(timer, after_ms)] = reconnect_waits(&effects)[..] else {
            panic!("one wait expected: {effects:?}");
        };
        waits.push(after_ms);
        let (next, effects) = step(next, Input::TimerFired(timer));
        assert_eq!(connects(&effects), 1);
        state = next;
    }

    assert_eq!(waits, [1_000, 2_000, 3_000, 3_000]);
}

#[test]
fn once_the_phone_is_back_the_waits_stop_and_start_short_again() {
    let (state, _) = run(connected(), [hands_free_down(), absent()]);
    let (state, effects) = run(
        state,
        [Input::ConnectPhoneFailed {
            attempt: 1,
            failure: ConnectFailure::Unreachable,
        }],
    );
    let (wait, _) = reconnect_waits(&effects)[0];
    let (state, _) = step(state, Input::TimerFired(wait));
    let (state, effects) = step(
        state,
        Input::ConnectPhoneFailed {
            attempt: 2,
            failure: ConnectFailure::Unreachable,
        },
    );
    let old_wait = reconnect_waits(&effects).last().map(|(timer, _)| *timer);

    let (state, effects) = run(state, [present()]);
    let (state, late) = run(state, old_wait.map(Input::TimerFired));

    assert_eq!(connects(&effects) + connects(&late), 0);
    let (state, effects) = run(state, [absent()]);
    assert_eq!(connects(&effects), 1);
    let (_, effects) = run(
        state,
        [Input::ConnectPhoneFailed {
            attempt: 3,
            failure: ConnectFailure::Unreachable,
        }],
    );
    assert_eq!(
        reconnect_waits(&effects)
            .iter()
            .map(|(_, after_ms)| *after_ms)
            .collect::<Vec<_>>(),
        [1_000]
    );
}

#[test]
fn coming_back_reconnects_hands_free_and_refreshes_contacts_and_call_history() {
    let (state, _) = run(connected(), [hands_free_down(), absent()]);
    let (state, effects) = run(state, [Input::ConnectPhoneDone { attempt: 1 }, present()]);
    let (state, more) = run(state, hands_free_up());

    assert_eq!(state.connection_state(), &ConnectionState::Connected);
    assert!(more.contains(&Effect::SyncContacts), "{more:?}");
    assert!(
        reconnect_waits(&effects).iter().all(|(timer, _)| {
            connects(&run(state.clone(), [Input::TimerFired(*timer)]).1) == 0
        })
    );
}

#[test]
fn nothing_is_connected_while_the_desktop_is_suspended() {
    let (state, _) = run(connected(), [hands_free_down(), absent()]);
    let (state, effects) = run(
        state,
        [
            Input::ConnectPhoneFailed {
                attempt: 1,
                failure: ConnectFailure::Unreachable,
            },
            Input::Suspending,
        ],
    );
    let [(timer, _)] = reconnect_waits(&effects)[..] else {
        panic!("one wait expected: {effects:?}");
    };

    let (_, effects) = run(state, [Input::TimerFired(timer)]);

    assert_eq!(connects(&effects), 0);
}

#[test]
fn resuming_connects_the_phone_at_once() {
    let (state, _) = run(
        connected(),
        [
            Input::Suspending,
            hands_free_down(),
            absent(),
            Input::ConnectPhoneFailed {
                attempt: 1,
                failure: ConnectFailure::Unreachable,
            },
        ],
    );

    let (_, effects) = run(state, [Input::Resumed]);

    assert_eq!(connects(&effects), 1);
}

#[test]
fn resuming_while_a_connect_is_under_way_connects_again_as_soon_as_it_fails() {
    let (state, effects) = run(connected(), [hands_free_down(), absent()]);
    assert_eq!(connects(&effects), 1);

    let (state, effects) = run(state, [Input::Suspending, Input::Resumed]);
    assert_eq!(connects(&effects), 0);
    let (_, effects) = run(
        state,
        [Input::ConnectPhoneFailed {
            attempt: 1,
            failure: ConnectFailure::Unreachable,
        }],
    );

    assert_eq!(connects(&effects), 1);
    assert!(reconnect_waits(&effects).is_empty(), "{effects:?}");
}

#[test]
fn a_phone_that_forgot_the_pairing_is_a_problem_not_out_of_range() {
    let (state, _) = run(connected(), [hands_free_down(), absent()]);

    let (state, effects) = run(
        state,
        [Input::ConnectPhoneFailed {
            attempt: 1,
            failure: ConnectFailure::PairingLost,
        }],
    );

    assert_eq!(state.connection_state(), &ConnectionState::PairingLost);
    assert_eq!(
        state
            .problems()
            .iter()
            .map(|problem| problem.kind)
            .collect::<Vec<_>>(),
        [ProblemKind::PairingLost]
    );
    assert_eq!(tooltip(&state), "My Pixel not connected · Pairing lost");
    // Each connect would ask whoever holds the Phone to pair.
    assert!(reconnect_waits(&effects).is_empty(), "{effects:?}");
    let (state, _) = run(state, [Input::Dial("07700900123".to_owned())]);
    assert_eq!(
        state
            .surfaces()
            .call_window
            .and_then(|window| window.notice),
        Some(Failure::PhonePairingLost),
        "a dial says why"
    );
}

#[test]
fn a_phone_that_connects_after_all_is_no_longer_a_pairing_problem() {
    let (state, _) = run(
        connected(),
        [
            hands_free_down(),
            absent(),
            Input::ConnectPhoneFailed {
                attempt: 1,
                failure: ConnectFailure::PairingLost,
            },
        ],
    );

    let (state, _) = run(state, [present()]);

    assert!(state.problems().is_empty());
    assert_eq!(state.connection_state(), &ConnectionState::Connecting);
}

fn caller() -> Caller {
    Caller::Number("07700900123".to_owned())
}

fn live_call() -> State {
    run(
        connected(),
        [
            Input::CallAdded {
                call: CALL,
                state: CallState::Ringing,
                caller: caller(),
                at: Timestamp(1_000),
            },
            Input::CallChanged {
                call: CALL,
                state: CallState::Active,
                at: Timestamp(2_000),
            },
        ],
    )
    .0
}

/// The Bluetooth connection drops mid-Call: hands-free goes with it, and
/// the Phone link gives up waiting for the Call.
fn link_lost() -> [Input; 3] {
    [absent(), hands_free_down(), Input::CallEnded { call: CALL }]
}

#[test]
fn a_call_that_loses_the_connection_carries_on_on_the_phone_and_the_window_says_so() {
    let (state, effects) = run(live_call(), link_lost());

    let window = state.surfaces().call_window.expect("the Call Window stays");
    assert_eq!(window.call, Some(CALL));
    assert_eq!(window.notice, Some(Failure::ConnectionLost));
    assert!(
        matches!(
            window.phase,
            CallWindowPhase::Live {
                active_since: Timestamp(2_000),
                ..
            }
        ),
        "{:?}",
        window.phase
    );
    assert_eq!(window.audio, None);
    // Not "Call ended": nobody knows it did.
    assert!(
        !effects.contains(&Effect::RefreshCallHistory),
        "{effects:?}"
    );
    assert_eq!(
        state.refusal(&Input::SetMuted(true)),
        Some(Failure::ConnectionLost)
    );
}

#[test]
fn hanging_up_a_call_that_lost_the_connection_says_it_cant() {
    let (state, _) = run(live_call(), link_lost());

    let (_, effects) = run(state, [Input::HangUp(CALL)]);

    assert_eq!(effects, [Effect::ReportError(Failure::ConnectionLost)]);
}

#[test]
fn the_call_back_with_the_phone_keeps_its_timer_and_loses_the_notice() {
    let (state, _) = run(live_call(), link_lost());

    let (state, _) = run(state, [Input::ConnectPhoneDone { attempt: 1 }, present()]);
    let (state, _) = run(state, hands_free_up());
    let (state, _) = run(
        state,
        [Input::CallAdded {
            call: CallId(7),
            state: CallState::Active,
            caller: caller(),
            at: Timestamp(30_000),
        }],
    );

    let window = state.surfaces().call_window.expect("the Call Window stays");
    assert_eq!(window.call, Some(CallId(7)));
    assert_eq!(window.notice, None);
    assert!(
        matches!(
            window.phase,
            CallWindowPhase::Live {
                active_since: Timestamp(2_000),
                ..
            }
        ),
        "{:?}",
        window.phase
    );
}

fn connection_lost_timers(effects: &[Effect]) -> Vec<(Timer, u32)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::StartTimer {
                timer: timer @ Timer::ConnectionLost(_),
                after_ms,
            } => Some((*timer, *after_ms)),
            _ => None,
        })
        .collect()
}

#[test]
fn the_window_closes_if_the_call_doesnt_come_back_with_the_phone() {
    let (state, _) = run(live_call(), link_lost());
    let (state, _) = run(state, [Input::ConnectPhoneDone { attempt: 1 }, present()]);
    let (state, effects) = run(state, hands_free_up());
    let [(timer, 2_000)] = connection_lost_timers(&effects)[..] else {
        panic!("a 2 s timer expected: {effects:?}");
    };

    let (state, _) = run(state, [Input::TimerFired(timer)]);

    assert_eq!(state.surfaces().call_window, None);
}

#[test]
fn the_window_closes_after_a_while_if_the_phone_stays_away() {
    let (state, effects) = run(live_call(), link_lost());
    let [(timer, 10_000)] = connection_lost_timers(&effects)[..] else {
        panic!("a 10 s timer expected: {effects:?}");
    };

    let (state, _) = run(state, [Input::TimerFired(timer)]);

    assert_eq!(state.surfaces().call_window, None);
    assert_eq!(tooltip(&state), "My Pixel out of range");
}

#[test]
fn a_call_ringing_on_a_phone_out_of_reach_shows_no_banner() {
    let (state, _) = run(
        connected(),
        [
            Input::CallAdded {
                call: CALL,
                state: CallState::Ringing,
                caller: caller(),
                at: Timestamp(1_000),
            },
            absent(),
        ],
    );

    assert_eq!(state.surfaces().banner, None);
    assert!(!state.ringing());
}
