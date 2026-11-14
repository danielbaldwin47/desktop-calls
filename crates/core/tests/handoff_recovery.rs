//! The Call state machine at its public interface when hands-free doesn't
//! come back after a Handoff to the Phone (ticket #101): the Call carries on
//! on the Phone, so it is never shown as ended. The Call Window says the
//! connection to the Phone is lost, as it does for a Phone out of range
//! (ticket #19). The Desktop keeps asking BlueZ for hands-free, and picks
//! the Call back up when hands-free returns with it.

use desktop_calls_core::{
    AudioLocation, CallAudio, CallId, CallState, CallWindowPhase, Caller, ConnectionState, Effect,
    Input, Phone, PhoneCommand, State, Timer, Timestamp, Transport, messages::Failure, step,
};

const ADDRESS: &str = "AA:BB:CC:DD:EE:FF";
const CALL: CallId = CallId(1);
const ACTIVE_SINCE: Timestamp = Timestamp(2_000);

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

fn number() -> Caller {
    Caller::Number("07700900123".to_owned())
}

fn hands_free_up() -> Input {
    Input::PhoneConnected {
        address: ADDRESS.to_owned(),
    }
}

/// Hands-free drops for the Handoff to the Phone (ADR-0005): the
/// AudioGateway and the Call go, without the Call ending.
fn dropped() -> Input {
    Input::PhoneDisconnected {
        address: ADDRESS.to_owned(),
    }
}

/// A live Call placed on the Phone, with its audio brought to the Desktop.
fn live_here() -> State {
    run(
        State::default(),
        [
            Input::PhonePaired(Phone {
                name: "My Pixel".to_owned(),
                address: ADDRESS.to_owned(),
            }),
            Input::PhonePresent {
                address: ADDRESS.to_owned(),
            },
            hands_free_up(),
            Input::PhoneCommandDone(PhoneCommand::KeepAudioOnPhone),
            Input::CallAdded {
                call: CALL,
                state: CallState::Dialing,
                caller: number(),
                at: Timestamp(1_000),
            },
            Input::CallChanged {
                call: CALL,
                state: CallState::Active,
                at: ACTIVE_SINCE,
            },
            Input::MoveAudio(AudioLocation::Desktop),
            Input::PhoneCommandDone(PhoneCommand::MoveAudioToDesktop),
            Input::TransportChanged(Transport::Active),
            Input::PhoneCommandDone(PhoneCommand::KeepAudioOnPhone),
        ],
    )
    .0
}

/// The timer the effects started that `is`, with how long it runs.
fn started(effects: &[Effect], is: impl Fn(&Timer) -> bool) -> Option<(Timer, u32)> {
    effects.iter().find_map(|effect| match effect {
        Effect::StartTimer { timer, after_ms } if is(timer) => Some((*timer, *after_ms)),
        _ => None,
    })
}

fn phone_commands(effects: &[Effect]) -> Vec<PhoneCommand> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Phone(command) => Some(command.clone()),
            _ => None,
        })
        .collect()
}

/// The user moved the audio to the Phone, and hands-free dropped: returns
/// the state and the Handoff's 5 s limit.
fn dropped_for_handoff() -> (State, Timer) {
    let (state, effects) = run(live_here(), [Input::MoveAudio(AudioLocation::Phone)]);
    let (limit, _) = started(&effects, |timer| matches!(timer, Timer::HandoffTimedOut(_)))
        .unwrap_or_else(|| panic!("no limit for the Handoff: {effects:?}"));
    (run(state, [dropped()]).0, limit)
}

/// The Handoff to the Phone dropped hands-free and BlueZ couldn't connect it
/// again.
fn not_reconnected() -> (State, Vec<Effect>) {
    let (state, _) = dropped_for_handoff();
    run(
        state,
        [Input::PhoneCommandFailed(PhoneCommand::MoveAudioToPhone)],
    )
}

/// Asserts the Call Window shows the Call as live on the Phone, out of the
/// Desktop's reach, with its timer from when it connected.
fn assert_shown_lost(state: &State) {
    let window = state
        .surfaces()
        .call_window
        .unwrap_or_else(|| panic!("the Call Window closed"));
    assert_eq!(window.notice, Some(Failure::ConnectionLost));
    assert_eq!(window.audio, None);
    assert!(
        matches!(
            window.phase,
            CallWindowPhase::Live {
                active_since: ACTIVE_SINCE,
                ..
            }
        ),
        "{:?}",
        window.phase
    );
}

#[test]
fn if_hands_free_doesnt_reconnect_the_window_says_the_call_continues_on_the_phone() {
    let (state, _) = not_reconnected();

    assert_shown_lost(&state);
    assert_eq!(state.connection_state(), &ConnectionState::Disconnected);
    assert_eq!(
        state.refusal(&Input::MoveAudio(AudioLocation::Desktop)),
        Some(Failure::ConnectionLost)
    );
    let (_, effects) = run(state, [Input::HangUp(CALL)]);
    assert_eq!(effects, [Effect::ReportError(Failure::ConnectionLost)]);
}

#[test]
fn the_phone_link_giving_up_on_the_call_doesnt_end_it() {
    let (state, _) = not_reconnected();

    let (state, effects) = run(
        state,
        [Input::CallEnded {
            call: CALL,
            at: Timestamp(90_000),
        }],
    );

    assert_shown_lost(&state);
    assert!(
        !effects.contains(&Effect::RefreshCallHistory),
        "{effects:?}"
    );
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::NotifyMissedCall(_))),
        "{effects:?}"
    );
}

/// The wait before the next connect the effects started.
fn reconnect_wait(effects: &[Effect]) -> (Timer, u32) {
    started(effects, |timer| matches!(timer, Timer::Reconnect(_)))
        .unwrap_or_else(|| panic!("no wait before the next connect: {effects:?}"))
}

#[test]
fn the_desktop_keeps_asking_for_hands_free_waiting_longer_each_time() {
    let (mut state, effects) = not_reconnected();
    let mut wait = reconnect_wait(&effects);

    for next in [2_000, 3_000, 3_000] {
        let (timer, _) = wait;
        let (asked, effects) = run(state, [Input::TimerFired(timer)]);
        // The Phone is still present: only hands-free is missing, so the
        // Desktop doesn't ask BlueZ to connect the Phone itself.
        assert_eq!(phone_commands(&effects), [PhoneCommand::ConnectHandsFree]);
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, Effect::ConnectPhone { .. })),
            "{effects:?}"
        );
        let (failed, effects) = run(
            asked,
            [Input::PhoneCommandFailed(PhoneCommand::ConnectHandsFree)],
        );
        wait = reconnect_wait(&effects);
        assert_eq!(wait.1, next);
        state = failed;
    }
    assert_shown_lost(&state);
}

#[test]
fn the_first_try_again_comes_a_second_after_the_drop_failed() {
    let (_, effects) = not_reconnected();

    assert_eq!(reconnect_wait(&effects).1, 1_000);
}

/// Hands-free is back after the Desktop asked again, and so is the Call.
fn reconnected(state: State, effects: &[Effect], call_back: Input) -> State {
    let (timer, _) = reconnect_wait(effects);
    run(
        state,
        [
            Input::TimerFired(timer),
            Input::PhoneCommandDone(PhoneCommand::ConnectHandsFree),
            hands_free_up(),
            Input::PhoneCommandDone(PhoneCommand::KeepAudioOnPhone),
            call_back,
        ],
    )
    .0
}

const ON_PHONE: CallAudio = CallAudio {
    location: AudioLocation::Phone,
    moving: None,
};

/// Asserts the Call Window shows `call` live again, with its audio on the
/// Phone, its timer from when it first connected, and no notice.
fn assert_picked_up(state: &State, call: CallId) {
    let window = state
        .surfaces()
        .call_window
        .unwrap_or_else(|| panic!("no Call Window"));
    assert_eq!(window.call, Some(call));
    assert_eq!(window.notice, None);
    assert_eq!(window.audio, Some(ON_PHONE));
    assert!(
        matches!(
            window.phase,
            CallWindowPhase::Live {
                active_since: ACTIVE_SINCE,
                ..
            }
        ),
        "{:?}",
        window.phase
    );
    assert_eq!(state.connection_state(), &ConnectionState::Connected);
    assert_eq!(
        state.refusal(&Input::MoveAudio(AudioLocation::Desktop)),
        None
    );
}

#[test]
fn hands_free_back_before_the_phone_link_gives_up_picks_the_call_back_up() {
    let (state, effects) = not_reconnected();

    let state = reconnected(
        state,
        &effects,
        Input::CallChanged {
            call: CALL,
            state: CallState::Active,
            at: Timestamp(9_000),
        },
    );

    assert_picked_up(&state, CALL);
}

#[test]
fn hands_free_back_after_the_phone_link_gave_up_picks_the_call_back_up() {
    let (state, effects) = not_reconnected();
    let (state, _) = run(
        state,
        [Input::CallEnded {
            call: CALL,
            at: Timestamp(90_000),
        }],
    );

    let state = reconnected(
        state,
        &effects,
        Input::CallAdded {
            call: CallId(2),
            state: CallState::Active,
            caller: number(),
            at: Timestamp(9_000),
        },
    );

    assert_picked_up(&state, CallId(2));
}

#[test]
fn once_hands_free_is_back_the_desktop_stops_asking() {
    let (state, effects) = not_reconnected();
    let state = reconnected(
        state,
        &effects,
        Input::CallChanged {
            call: CALL,
            state: CallState::Active,
            at: Timestamp(9_000),
        },
    );
    let (timer, _) = reconnect_wait(&effects);

    let (_, effects) = run(state, [Input::TimerFired(timer)]);

    assert_eq!(effects, []);
}

#[test]
fn hands_free_not_back_by_the_limit_is_lost_the_same_way() {
    let (state, limit) = dropped_for_handoff();

    let (state, effects) = run(state, [Input::TimerFired(limit)]);

    assert_shown_lost(&state);
    assert_eq!(state.connection_state(), &ConnectionState::Disconnected);
    assert_eq!(reconnect_wait(&effects).1, 1_000);
    let (state, _) = run(
        state,
        [Input::CallEnded {
            call: CALL,
            at: Timestamp(90_000),
        }],
    );
    assert_shown_lost(&state);
}

#[test]
fn the_phone_link_giving_up_on_the_call_mid_handoff_doesnt_end_it() {
    let (state, limit) = dropped_for_handoff();

    let (state, effects) = run(
        state,
        [Input::CallEnded {
            call: CALL,
            at: Timestamp(90_000),
        }],
    );

    assert_shown_lost(&state);
    assert!(
        !effects.contains(&Effect::RefreshCallHistory),
        "{effects:?}"
    );
    assert_eq!(reconnect_wait(&effects).1, 1_000);
    // The limit has nothing left to do.
    let (_, effects) = run(state, [Input::TimerFired(limit)]);
    assert_eq!(effects, []);
}

#[test]
fn hands_free_back_late_from_the_drop_itself_picks_the_call_back_up() {
    let (state, _) = dropped_for_handoff();
    let (state, _) = run(
        state,
        [Input::CallEnded {
            call: CALL,
            at: Timestamp(90_000),
        }],
    );

    // The drop's own `ConnectProfile` got there in the end.
    let (state, _) = run(
        state,
        [
            hands_free_up(),
            Input::PhoneCommandDone(PhoneCommand::MoveAudioToPhone),
            Input::PhoneCommandDone(PhoneCommand::KeepAudioOnPhone),
            Input::CallAdded {
                call: CallId(2),
                state: CallState::Active,
                caller: number(),
                at: Timestamp(9_000),
            },
        ],
    );

    assert_picked_up(&state, CallId(2));
}

#[test]
fn the_phone_leaving_meanwhile_is_connected_as_any_phone_out_of_range() {
    let (state, effects) = not_reconnected();
    let (timer, _) = reconnect_wait(&effects);

    let (state, effects) = run(
        state,
        [Input::PhoneAbsent {
            address: ADDRESS.to_owned(),
        }],
    );

    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::ConnectPhone { .. })),
        "{effects:?}"
    );
    assert_eq!(phone_commands(&effects), []);
    assert_shown_lost(&state);
    // The wait for hands-free is over.
    let (_, effects) = run(state, [Input::TimerFired(timer)]);
    assert_eq!(effects, []);
}

#[test]
fn the_desktop_stops_asking_while_suspended_and_asks_at_once_on_resume() {
    let (state, effects) = not_reconnected();
    let (timer, _) = reconnect_wait(&effects);

    let (state, effects) = run(state, [Input::Suspending, Input::TimerFired(timer)]);
    assert_eq!(effects, []);

    let (_, effects) = run(state, [Input::Resumed]);
    assert_eq!(phone_commands(&effects), [PhoneCommand::ConnectHandsFree]);
}

#[test]
fn a_call_that_ended_while_hands_free_was_lost_closes_the_window_once_hands_free_is_back() {
    let (state, effects) = not_reconnected();
    let (state, _) = run(
        state,
        [Input::CallEnded {
            call: CALL,
            at: Timestamp(90_000),
        }],
    );
    let (timer, _) = reconnect_wait(&effects);

    let (state, effects) = run(
        state,
        [
            Input::TimerFired(timer),
            Input::PhoneCommandDone(PhoneCommand::ConnectHandsFree),
            hands_free_up(),
        ],
    );
    let (lost, after_ms) = started(&effects, |timer| matches!(timer, Timer::ConnectionLost(_)))
        .unwrap_or_else(|| panic!("no wait for the Call to come back: {effects:?}"));
    assert_eq!(after_ms, 2_000);

    let (state, _) = run(state, [Input::TimerFired(lost)]);
    assert_eq!(state.surfaces().call_window, None);
}
