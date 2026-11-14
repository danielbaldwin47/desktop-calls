//! Turning calls off never cuts a live Call, at the Call state machine's
//! public interface (ticket #204): during a Call the turn-off waits, and
//! is applied once no Call is left; Keep calls on cancels it; a timed
//! choice keeps its deadline, and if the Call outlasts it calls stay on;
//! and a waiting turn-off is kept for the next run.

use desktop_calls_core::{
    CallId, CallState, Caller, CallsOffRecord, Deadline, Effect, IndicatorEntry, IndicatorLook,
    IndicatorStatus, Input, Phone, PhoneCommand, State, TimeOfDay, Timer, Timestamp, WallClock,
    messages, step,
};

const ADDRESS: &str = "AA:BB:CC:DD:EE:FF";
/// 20:30 local time, as the shell's clock read it.
const NOW: u64 = 1_800_000_000_000;
const MINUTE: u64 = 60_000;
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

/// In an hour: 21:30.
fn in_an_hour() -> Deadline {
    Deadline {
        at: Timestamp(NOW + 60 * MINUTE),
        shown: TimeOfDay {
            hour: 21,
            minute: 30,
        },
    }
}

/// The wall clock `minutes` after 20:30.
fn clock(minutes: u64) -> Input {
    let now = NOW + minutes * MINUTE;
    let shown = TimeOfDay { hour: 7, minute: 0 };
    Input::ClockTicked(WallClock {
        now: Timestamp(now),
        for_an_hour: Deadline {
            at: Timestamp(now + 60 * MINUTE),
            shown,
        },
        until_morning: Deadline {
            at: Timestamp(NOW + 630 * MINUTE),
            shown,
        },
        quiet_hours: None,
    })
}

fn paired() -> Input {
    Input::PhonePaired(Phone {
        name: "My Pixel".to_owned(),
        address: ADDRESS.to_owned(),
    })
}

fn connected_inputs() -> [Input; 5] {
    [
        clock(0),
        paired(),
        Input::PhonePresent {
            address: ADDRESS.to_owned(),
        },
        Input::PhoneConnected {
            address: ADDRESS.to_owned(),
        },
        Input::PhoneCommandDone(PhoneCommand::KeepAudioOnPhone),
    ]
}

fn connected() -> State {
    run(State::default(), connected_inputs()).0
}

fn call_added(call_state: CallState) -> Input {
    Input::CallAdded {
        call: CALL,
        state: call_state,
        caller: Caller::Number("+447700900123".to_owned()),
        at: Timestamp(NOW),
    }
}

/// The Phone with one Call in `call_state`.
fn with_call(call_state: CallState) -> State {
    run(connected(), [call_added(call_state)]).0
}

fn ended() -> Input {
    Input::CallEnded {
        call: CALL,
        at: Timestamp(NOW + MINUTE),
    }
}

fn block_phone(blocked: bool) -> Effect {
    Effect::BlockPhone {
        address: ADDRESS.to_owned(),
        blocked,
    }
}

fn record_pending(until: Option<Deadline>) -> CallsOffRecord {
    CallsOffRecord::Pending {
        address: ADDRESS.to_owned(),
        until,
        quiet_hours: false,
    }
}

fn record_off(until: Option<Deadline>) -> CallsOffRecord {
    CallsOffRecord::Off {
        address: ADDRESS.to_owned(),
        until,
        quiet_hours: false,
    }
}

/// Whether any effect touches the Phone's block.
fn blocks(effects: &[Effect]) -> bool {
    effects
        .iter()
        .any(|effect| matches!(effect, Effect::BlockPhone { .. }))
}

/// Calls are on, a turn-off waits: the Indicator says so and offers Keep
/// calls on, and nothing has touched the Phone.
fn waits(state: &State, effects: &[Effect]) {
    assert!(!blocks(effects), "{effects:?}");
    assert!(!state.calls_off());
    assert!(state.calls_off_pending());
    let view = state.indicator(&state.surfaces());
    assert!(view.menu.contains(&IndicatorEntry::KeepCallsOn), "{view:?}");
    assert!(
        !view
            .menu
            .iter()
            .any(|entry| matches!(entry, IndicatorEntry::TurnCallsOff { .. })),
        "{view:?}"
    );
    assert_ne!(view.look, IndicatorLook::Off);
}

/// A turn-off during `state`'s Call waits, then `end` leaves no Call and
/// calls go off.
fn waits_for_the_call(state: State, end: &[Input]) {
    let (state, effects) = run(state, [Input::TurnCallsOff { until: None }]);
    waits(&state, &effects);
    assert_eq!(
        effects,
        [
            Effect::KeepCallsOff(Some(record_pending(None))),
            Effect::CallsOffPending {
                address: ADDRESS.to_owned()
            }
        ]
    );

    let (state, effects) = run(state, end.iter().cloned());
    assert!(state.calls_off());
    assert!(!state.calls_off_pending());
    let kept = effects
        .iter()
        .position(|effect| *effect == Effect::KeepCallsOff(Some(record_off(None))));
    let block = effects
        .iter()
        .position(|effect| *effect == block_phone(true));
    assert!(
        kept.is_some() && kept < block,
        "the record, then the block: {effects:?}"
    );
}

#[test]
fn a_turn_off_during_a_call_waits_and_applies_when_it_ends() {
    for call_state in [
        CallState::Ringing,
        CallState::Active,
        CallState::Held,
        CallState::Dialing,
        CallState::Alerting,
    ] {
        waits_for_the_call(with_call(call_state), &[ended()]);
    }
}

#[test]
fn a_turn_off_waits_for_the_last_call_not_the_first() {
    let (state, _) = run(
        with_call(CallState::Active),
        [Input::CallAdded {
            call: CallId(2),
            state: CallState::Waiting,
            caller: Caller::Number("+447700900456".to_owned()),
            at: Timestamp(NOW),
        }],
    );
    let (state, _) = run(state, [Input::TurnCallsOff { until: None }]);
    let (state, effects) = run(state, [ended()]);
    waits(&state, &effects);

    let (state, _) = run(
        state,
        [Input::CallEnded {
            call: CallId(2),
            at: Timestamp(NOW + MINUTE),
        }],
    );
    assert!(state.calls_off());
}

#[test]
fn a_turn_off_while_placing_a_call_waits_for_it() {
    let (state, _) = run(connected(), [Input::Dial("07700900123".to_owned())]);
    // The Phone reports the Call, which then ends.
    waits_for_the_call(state, &[call_added(CallState::Dialing), ended()]);
}

#[test]
fn a_turn_off_while_a_lost_call_carries_on_on_the_phone_waits_for_it() {
    // The Phone goes out of range mid-Call.
    let (state, _) = run(
        with_call(CallState::Active),
        [
            Input::PhoneDisconnected {
                address: ADDRESS.to_owned(),
            },
            Input::PhoneAbsent {
                address: ADDRESS.to_owned(),
            },
        ],
    );
    let (state, effects) = run(state, [Input::TurnCallsOff { until: None }]);
    waits(&state, &effects);

    // The Phone link gives up on the Call: it carries on on the Phone, and
    // the Call Window still says so.
    let (state, effects) = run(state, [ended()]);
    waits(&state, &effects);

    // The Call Window has said it for long enough.
    let (state, effects) = run(state, [Input::TimerFired(Timer::ConnectionLost(1))]);
    assert!(state.calls_off(), "{effects:?}");
}

#[test]
fn the_indicator_says_calls_turn_off_when_this_call_ends() {
    let (state, _) = run(
        with_call(CallState::Active),
        [Input::TurnCallsOff { until: None }],
    );
    let view = state.indicator(&state.surfaces());

    assert_eq!(view.status, IndicatorStatus::CallsOffPending);
    assert_eq!(view.look, IndicatorLook::InCall);
    assert_eq!(
        messages::indicator_status(&view.status),
        "Calls turn off when this Call ends"
    );
    assert_eq!(
        messages::indicator_entry(&IndicatorEntry::KeepCallsOn, &view),
        "Keep calls on"
    );
    assert_eq!(
        IndicatorEntry::KeepCallsOn.intent(),
        Some(Input::KeepCallsOn)
    );
}

#[test]
fn a_ringing_call_keeps_the_indicator_on_the_caller() {
    let (state, _) = run(
        with_call(CallState::Ringing),
        [Input::TurnCallsOff { until: None }],
    );
    let view = state.indicator(&state.surfaces());

    assert!(matches!(view.status, IndicatorStatus::Ringing(_)));
    assert!(view.menu.contains(&IndicatorEntry::KeepCallsOn));
}

#[test]
fn keep_calls_on_cancels_the_waiting_turn_off() {
    let (state, _) = run(
        with_call(CallState::Active),
        [Input::TurnCallsOff { until: None }],
    );
    let (state, effects) = run(state, [Input::KeepCallsOn]);

    assert_eq!(
        effects,
        [
            Effect::KeepCallsOff(None),
            Effect::CallsKeptOn {
                address: ADDRESS.to_owned()
            }
        ]
    );
    assert!(!state.calls_off_pending());
    let view = state.indicator(&state.surfaces());
    assert!(
        view.menu
            .iter()
            .any(|entry| matches!(entry, IndicatorEntry::TurnCallsOff { enabled: true, .. })),
        "{view:?}"
    );

    let (state, effects) = run(state, [ended()]);
    assert!(!blocks(&effects), "{effects:?}");
    assert!(!state.calls_off());
}

#[test]
fn turn_calls_on_also_cancels_the_waiting_turn_off() {
    let (state, _) = run(
        with_call(CallState::Active),
        [Input::TurnCallsOff { until: None }],
    );
    let (state, _) = run(state, [Input::TurnCallsOn, ended()]);
    assert!(!state.calls_off());
    assert!(!state.calls_off_pending());
}

#[test]
fn a_timed_choice_keeps_the_deadline_it_was_given() {
    let (state, _) = run(
        with_call(CallState::Active),
        [Input::TurnCallsOff {
            until: Some(in_an_hour()),
        }],
    );
    assert_eq!(state.calls_off_until(), Some(in_an_hour()));

    let (state, effects) = run(state, [clock(20), ended()]);
    assert!(state.calls_off());
    assert_eq!(state.calls_off_until(), Some(in_an_hour()));
    assert!(
        effects.contains(&Effect::KeepCallsOff(Some(record_off(Some(in_an_hour()))))),
        "{effects:?}"
    );
    assert!(effects.contains(&Effect::ReadClockAt(in_an_hour().at)));
}

#[test]
fn a_deadline_passing_during_the_call_leaves_calls_on() {
    let (state, _) = run(
        with_call(CallState::Active),
        [Input::TurnCallsOff {
            until: Some(in_an_hour()),
        }],
    );
    let (state, effects) = run(state, [clock(60)]);
    assert!(!state.calls_off_pending());
    assert!(effects.contains(&Effect::KeepCallsOff(None)), "{effects:?}");

    let (state, effects) = run(state, [ended()]);
    assert!(!blocks(&effects), "{effects:?}");
    assert!(!state.calls_off());
}

#[test]
fn turning_calls_off_again_while_waiting_replaces_the_choice() {
    let (state, _) = run(
        with_call(CallState::Active),
        [Input::TurnCallsOff {
            until: Some(in_an_hour()),
        }],
    );
    let (state, effects) = run(state, [Input::TurnCallsOff { until: None }]);
    assert_eq!(effects, [Effect::KeepCallsOff(Some(record_pending(None)))]);

    let (state, _) = run(state, [clock(60), ended()]);
    assert!(state.calls_off());
    assert_eq!(state.calls_off_until(), None);
}

#[test]
fn a_waiting_turn_off_kept_by_an_earlier_run_waits_for_its_calls() {
    // The earlier run stopped mid-Call: its Call shows again once the
    // Phone is back, and the turn-off waits for it to end.
    let earlier = desktop_calls_core::EarlierRun {
        calls: vec![desktop_calls_core::EarlierCall {
            caller: Caller::Number("+447700900123".to_owned()),
            direction: desktop_calls_core::Direction::Incoming,
            active_since: Timestamp(NOW - MINUTE),
        }],
        restore_microphone_volume: None,
    };
    let (state, effects) = run(
        State::default(),
        [
            Input::EarlierRun(earlier),
            clock(0),
            Input::CallsOffKept(record_pending(None)),
        ],
    );
    assert!(state.calls_off_pending());
    let (state, more) = run(state, connected_inputs());
    let (state, more_still) = run(state, [call_added(CallState::Active)]);
    assert!(!blocks(&effects) && !blocks(&more) && !blocks(&more_still));
    assert!(state.calls_off_pending());

    let (state, _) = run(state, [ended()]);
    assert!(state.calls_off());
}

#[test]
fn a_waiting_turn_off_kept_with_no_call_recorded_waits_for_the_earlier_run_window() {
    // A Call that was still ringing isn't recorded: the turn-off waits
    // until the earlier run's Calls would have shown again.
    let (state, _) = run(
        State::default(),
        [clock(0), Input::CallsOffKept(record_pending(None))],
    );
    let (state, effects) = run(state, connected_inputs());
    assert!(!blocks(&effects), "{effects:?}");
    assert!(state.calls_off_pending());

    let (state, effects) = run(state, [Input::TimerFired(Timer::ForgetEarlierRun)]);
    assert!(state.calls_off(), "{effects:?}");
}

#[test]
fn a_waiting_turn_off_kept_past_its_deadline_is_dropped() {
    let (state, effects) = run(
        State::default(),
        [
            clock(61),
            Input::CallsOffKept(record_pending(Some(in_an_hour()))),
        ],
    );
    assert!(!state.calls_off_pending());
    assert!(effects.contains(&Effect::KeepCallsOff(None)), "{effects:?}");
}

#[test]
fn unpairing_the_phone_drops_the_waiting_turn_off() {
    let (state, _) = run(
        with_call(CallState::Active),
        [Input::TurnCallsOff { until: None }],
    );
    let (state, effects) = run(
        state,
        [Input::PhoneUnpaired {
            address: ADDRESS.to_owned(),
        }],
    );
    assert!(!state.calls_off_pending());
    assert!(effects.contains(&Effect::KeepCallsOff(None)), "{effects:?}");
    assert!(!blocks(&effects), "nothing was blocked: {effects:?}");
}
