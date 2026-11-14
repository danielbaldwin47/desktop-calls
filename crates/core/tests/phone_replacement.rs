//! Work started for a Phone cannot change a later attachment, including
//! unpairing and pairing the same Phone again. All observations use the
//! public Call state machine interface.

use desktop_calls_core::{
    AudioLocation, CallId, CallState, Caller, ConnectFailure, ConnectionState, DialProgress,
    Effect, Input, Phone, PhoneCommand, State, Timer, Timestamp, messages::Failure, step,
};

const FIRST: &str = "AA:BB:CC:DD:EE:FF";
const SECOND: &str = "11:22:33:44:55:66";
const NUMBER: &str = "07700900123";
const CALL: CallId = CallId(1);

fn run(mut state: State, inputs: impl IntoIterator<Item = Input>) -> State {
    for input in inputs {
        state = step(state, input).0;
    }
    state
}

fn connected(state: State, address: &str) -> State {
    run(
        state,
        [
            Input::PhonePaired(Phone {
                name: "Phone".to_owned(),
                address: address.to_owned(),
            }),
            Input::PhoneConnected {
                address: address.to_owned(),
            },
            Input::PhoneCommandDone(PhoneCommand::KeepAudioOnPhone),
        ],
    )
}

fn replace(state: State, address: &str) -> State {
    let state = run(
        state,
        [Input::PhoneUnpaired {
            address: FIRST.to_owned(),
        }],
    );
    connected(state, address)
}

fn timer(effects: &[Effect], is: impl Fn(Timer) -> bool) -> Timer {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::StartTimer { timer, .. } if is(*timer) => Some(*timer),
            _ => None,
        })
        .unwrap_or_else(|| panic!("the operation starts its timer: {effects:?}"))
}

fn connect_attempt(effects: &[Effect]) -> u32 {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::ConnectPhone { attempt, .. } => Some(*attempt),
            _ => None,
        })
        .unwrap_or_else(|| panic!("the absent Phone starts a connect: {effects:?}"))
}

fn absent(state: State, address: &str) -> (State, u32) {
    let (state, effects) = step(
        state,
        Input::PhoneAbsent {
            address: address.to_owned(),
        },
    );
    (state, connect_attempt(&effects))
}

fn live_call(state: State) -> State {
    run(
        state,
        [Input::CallAdded {
            call: CALL,
            state: CallState::Active,
            caller: Caller::Number(NUMBER.to_owned()),
            at: Timestamp(1_000),
        }],
    )
}

#[test]
fn an_earlier_phones_dial_timeout_leaves_the_current_dial_alone() {
    for address in [SECOND, FIRST] {
        let (state, effects) = step(
            connected(State::default(), FIRST),
            Input::Dial(NUMBER.to_owned()),
        );
        let old = timer(&effects, |timer| matches!(timer, Timer::DialTimedOut(_)));
        let (state, effects) = step(replace(state, address), Input::Dial(NUMBER.to_owned()));
        let current = timer(&effects, |timer| matches!(timer, Timer::DialTimedOut(_)));
        let before = state.surfaces();

        let (state, effects) = step(state, Input::TimerFired(old));
        assert!(
            effects.is_empty(),
            "the earlier dial must not fail or reset audio: {effects:?}"
        );
        assert_eq!(state.surfaces(), before);

        let (_, effects) = step(state, Input::TimerFired(current));
        assert!(
            effects.iter().any(|effect| matches!(
                effect,
                Effect::DialProgress(DialProgress::NotPlaced(_, Failure::DialFailed))
            )),
            "the current dial still times out: {effects:?}"
        );
    }
}

#[test]
fn an_earlier_connect_result_leaves_the_current_phones_recovery_alone() {
    for address in [SECOND, FIRST] {
        for succeeded in [false, true] {
            let (state, old) = absent(connected(State::default(), FIRST), FIRST);
            let (state, current) = absent(replace(state, address), address);
            let before = state.surfaces();
            let input = if succeeded {
                Input::ConnectPhoneDone { attempt: old }
            } else {
                Input::ConnectPhoneFailed {
                    attempt: old,
                    failure: ConnectFailure::PairingLost,
                }
            };

            let (state, effects) = step(state, input);
            assert!(
                effects.is_empty(),
                "an earlier connect has no effects: {effects:?}"
            );
            assert_eq!(state.surfaces(), before);
            assert_ne!(state.connection_state(), &ConnectionState::PairingLost);

            let (state, effects) = step(
                state,
                Input::ConnectPhoneFailed {
                    attempt: current,
                    failure: ConnectFailure::PairingLost,
                },
            );
            assert_eq!(state.connection_state(), &ConnectionState::PairingLost);
            assert!(!effects.iter().any(|effect| matches!(
                effect,
                Effect::StartTimer {
                    timer: Timer::Reconnect(_),
                    ..
                }
            )));
        }
    }
}

#[test]
fn a_duplicate_connect_result_cannot_finish_a_later_connect_for_the_same_phone() {
    let (state, old) = absent(connected(State::default(), FIRST), FIRST);
    let (state, effects) = step(
        state,
        Input::ConnectPhoneFailed {
            attempt: old,
            failure: ConnectFailure::Unreachable,
        },
    );
    let wait = timer(&effects, |timer| matches!(timer, Timer::Reconnect(_)));
    let (state, effects) = step(state, Input::TimerFired(wait));
    let current = connect_attempt(&effects);

    let (state, effects) = step(state, Input::ConnectPhoneDone { attempt: old });
    assert!(effects.is_empty());
    let (_, effects) = step(
        state,
        Input::ConnectPhoneFailed {
            attempt: current,
            failure: ConnectFailure::Unreachable,
        },
    );
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::StartTimer {
                timer: Timer::Reconnect(_),
                after_ms: 2_000
            }
        )),
        "the current failure retains retry backoff: {effects:?}"
    );
}

#[test]
fn an_earlier_phones_reconnect_wait_cannot_start_a_connect_for_its_replacement() {
    let (state, attempt) = absent(connected(State::default(), FIRST), FIRST);
    let (state, effects) = step(
        state,
        Input::ConnectPhoneFailed {
            attempt,
            failure: ConnectFailure::Unreachable,
        },
    );
    let old = timer(&effects, |timer| matches!(timer, Timer::Reconnect(_)));
    let (state, attempt) = absent(replace(state, SECOND), SECOND);
    let (state, effects) = step(
        state,
        Input::ConnectPhoneFailed {
            attempt,
            failure: ConnectFailure::Unreachable,
        },
    );
    let current = timer(&effects, |timer| matches!(timer, Timer::Reconnect(_)));

    let (state, effects) = step(state, Input::TimerFired(old));
    assert!(
        effects.is_empty(),
        "the new Phone keeps its own retry wait: {effects:?}"
    );
    let (_, effects) = step(state, Input::TimerFired(current));
    connect_attempt(&effects);
}

#[test]
fn an_earlier_phones_handoff_timeout_cannot_fail_the_current_handoff() {
    let (state, effects) = step(
        live_call(connected(State::default(), FIRST)),
        Input::MoveAudio(AudioLocation::Desktop),
    );
    let old = timer(&effects, |timer| matches!(timer, Timer::HandoffTimedOut(_)));
    let (state, effects) = step(
        live_call(replace(state, SECOND)),
        Input::MoveAudio(AudioLocation::Desktop),
    );
    let current = timer(&effects, |timer| matches!(timer, Timer::HandoffTimedOut(_)));
    let before = state.surfaces();

    let (state, effects) = step(state, Input::TimerFired(old));
    assert!(effects.is_empty());
    assert_eq!(state.surfaces(), before);
    let (_, effects) = step(state, Input::TimerFired(current));
    assert!(effects.contains(&Effect::ReportError(Failure::AudioNotMoved)));
}

#[test]
fn an_earlier_phones_handoff_retry_cannot_retry_the_current_handoff() {
    let state = run(
        live_call(connected(State::default(), FIRST)),
        [Input::MoveAudio(AudioLocation::Desktop)],
    );
    let (state, effects) = step(
        state,
        Input::PhoneCommandDone(PhoneCommand::MoveAudioToDesktop),
    );
    let old = timer(&effects, |timer| {
        matches!(timer, Timer::HandoffNotStarted(_))
    });
    let state = run(
        live_call(replace(state, SECOND)),
        [Input::MoveAudio(AudioLocation::Desktop)],
    );
    let (state, effects) = step(
        state,
        Input::PhoneCommandDone(PhoneCommand::MoveAudioToDesktop),
    );
    let current = timer(&effects, |timer| {
        matches!(timer, Timer::HandoffNotStarted(_))
    });

    let (state, effects) = step(state, Input::TimerFired(old));
    assert!(effects.is_empty());
    let (_, effects) = step(state, Input::TimerFired(current));
    assert!(effects.contains(&Effect::Phone(PhoneCommand::MoveAudioToDesktop)));
}

#[test]
fn an_earlier_connect_result_cannot_advance_the_replacement_phones_setup() {
    let (state, old) = absent(connected(State::default(), FIRST), FIRST);
    let phone = Phone {
        name: "New Phone".to_owned(),
        address: SECOND.to_owned(),
    };
    let state = run(
        state,
        [
            Input::OpenSetupWindow,
            Input::PhoneNearby(phone.clone()),
            Input::ChoosePhone {
                address: SECOND.to_owned(),
            },
            Input::PairingCode {
                phone: phone.clone(),
                code: 123_456,
            },
            Input::ConfirmPairing,
        ],
    );
    let (state, effects) = step(state, Input::PhonePaired(phone));
    let current = connect_attempt(&effects);
    let before = state.surfaces();

    let (state, effects) = step(state, Input::ConnectPhoneDone { attempt: old });
    assert!(
        effects.is_empty(),
        "an earlier result must not start the hands-free wait: {effects:?}"
    );
    assert_eq!(state.surfaces(), before);
    let (_, effects) = step(state, Input::ConnectPhoneDone { attempt: current });
    timer(&effects, |timer| matches!(timer, Timer::SetupStep(_)));
}

#[test]
fn an_earlier_phones_connection_loss_timer_cannot_dismiss_the_current_notice() {
    let (state, effects) = step(
        live_call(connected(State::default(), FIRST)),
        Input::PhoneAbsent {
            address: FIRST.to_owned(),
        },
    );
    let old = timer(&effects, |timer| matches!(timer, Timer::ConnectionLost(_)));
    let (state, effects) = step(
        live_call(replace(state, SECOND)),
        Input::PhoneAbsent {
            address: SECOND.to_owned(),
        },
    );
    let current = timer(&effects, |timer| matches!(timer, Timer::ConnectionLost(_)));
    let state = run(
        state,
        [Input::CallEnded {
            call: CALL,
            at: Timestamp(90_000),
        }],
    );
    let before = state.surfaces();

    let (state, _) = step(state, Input::TimerFired(old));
    assert_eq!(state.surfaces(), before);
    let (state, _) = step(state, Input::TimerFired(current));
    assert!(state.surfaces().call_window.is_none());
}

fn waiting_refused(state: State) -> (State, Timer) {
    let state = run(
        live_call(state),
        [
            Input::CallAdded {
                call: CallId(2),
                state: CallState::Waiting,
                caller: Caller::Number("07700900456".to_owned()),
                at: Timestamp(2_000),
            },
            Input::HoldAndAnswer,
        ],
    );
    let (state, effects) = step(
        state,
        Input::PhoneCommandFailed(PhoneCommand::HoldAndAnswer(CallId(2))),
    );
    (
        state,
        timer(&effects, |timer| {
            matches!(timer, Timer::DismissCallWaitingNotice(_))
        }),
    )
}

#[test]
fn an_earlier_phones_waiting_notice_timer_cannot_dismiss_the_current_notice() {
    let (state, old) = waiting_refused(connected(State::default(), FIRST));
    let (state, current) = waiting_refused(replace(state, SECOND));
    let before = state.surfaces();

    let (state, _) = step(state, Input::TimerFired(old));
    assert_eq!(state.surfaces(), before);
    let (state, _) = step(state, Input::TimerFired(current));
    assert_ne!(state.surfaces(), before);
    assert_eq!(
        state
            .surfaces()
            .call_window
            .expect("the live Call stays")
            .notice,
        None
    );
}

#[test]
fn an_earlier_phones_dial_notice_timer_cannot_dismiss_the_current_notice() {
    let state = run(
        connected(State::default(), FIRST),
        [Input::Dial(NUMBER.to_owned())],
    );
    let (state, effects) = step(state, Input::CancelDial);
    let old = timer(&effects, |timer| matches!(timer, Timer::DismissDial(_)));
    let state = run(replace(state, SECOND), [Input::Dial(NUMBER.to_owned())]);
    let (state, effects) = step(state, Input::CancelDial);
    let current = timer(&effects, |timer| matches!(timer, Timer::DismissDial(_)));
    let before = state.surfaces();

    let (state, _) = step(state, Input::TimerFired(old));
    assert_eq!(state.surfaces(), before);
    let (state, _) = step(state, Input::TimerFired(current));
    assert!(state.surfaces().call_window.is_none());
}
