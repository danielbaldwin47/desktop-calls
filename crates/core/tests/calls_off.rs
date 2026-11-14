//! Calls Off at the Call state machine's public interface: turning calls
//! off blocks the Phone and nothing connects it again, whatever happens,
//! until calls are turned on; turning on unblocks it and connects it at
//! once; a record an earlier run kept is put right at startup; and the
//! Indicator shows the Off look and the entries that turn calls off and on.

use desktop_calls_core::{
    CallsOffRecord, ConnectFailure, ConnectionState, Effect, IndicatorEntry, IndicatorLook,
    IndicatorStatus, Input, Phone, PhoneCommand, State, SyncFailure, Timer,
    messages::{self, Failure},
    step,
};

const ADDRESS: &str = "AA:BB:CC:DD:EE:FF";

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

fn connected() -> State {
    let (state, _) = run(
        State::default(),
        [
            paired(),
            Input::PhonePresent {
                address: ADDRESS.to_owned(),
            },
            Input::PhoneConnected {
                address: ADDRESS.to_owned(),
            },
            Input::PhoneCommandDone(PhoneCommand::KeepAudioOnPhone),
        ],
    );
    state
}

/// BlueZ drops a blocked Phone at once: hands-free first, then the link.
fn dropped() -> [Input; 2] {
    [
        Input::PhoneDisconnected {
            address: ADDRESS.to_owned(),
        },
        Input::PhoneAbsent {
            address: ADDRESS.to_owned(),
        },
    ]
}

fn blocked(blocked: bool) -> Input {
    Input::PhoneBlocked {
        address: ADDRESS.to_owned(),
        blocked,
    }
}

fn off() -> State {
    let (state, _) = run(
        connected(),
        [Input::TurnCallsOff { until: None }, blocked(true)],
    );
    run(state, dropped()).0
}

/// Every effect that would page or connect the Phone.
fn connects(effects: &[Effect]) -> Vec<&Effect> {
    effects
        .iter()
        .filter(|effect| {
            matches!(
                effect,
                Effect::ConnectPhone { .. }
                    | Effect::ConnectHandsFree { .. }
                    | Effect::Phone(PhoneCommand::ConnectHandsFree)
            )
        })
        .collect()
}

fn reconnect_timers(effects: &[Effect]) -> Vec<Timer> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::StartTimer {
                timer: timer @ Timer::Reconnect(_),
                ..
            } => Some(*timer),
            _ => None,
        })
        .collect()
}

fn record_off() -> CallsOffRecord {
    CallsOffRecord::Off {
        address: ADDRESS.to_owned(),
        until: None,
        quiet_hours: false,
    }
}

fn record_turning_on() -> CallsOffRecord {
    CallsOffRecord::TurningOn {
        address: ADDRESS.to_owned(),
        kept_on_until: None,
    }
}

fn block_phone(blocked: bool) -> Effect {
    Effect::BlockPhone {
        address: ADDRESS.to_owned(),
        blocked,
    }
}

#[test]
fn turning_calls_off_keeps_the_record_before_blocking_the_phone() {
    let (state, effects) = run(connected(), [Input::TurnCallsOff { until: None }]);

    assert_eq!(
        effects,
        [Effect::KeepCallsOff(Some(record_off())), block_phone(true)]
    );
    assert!(state.calls_off());
}

#[test]
fn while_calls_are_off_nothing_connects_the_phone() {
    let (state, effects) = run(
        connected(),
        [Input::TurnCallsOff { until: None }, blocked(true)],
    );
    let (state, mut more) = run(state, dropped());
    let mut effects = [effects, std::mem::take(&mut more)].concat();
    // Whatever might connect it otherwise: the Phone's link coming and going
    // as it pages the Desktop, a resume, Bluetooth back on, a stray timer.
    let (_, mut more) = run(
        state,
        [
            Input::PhonePresent {
                address: ADDRESS.to_owned(),
            },
            Input::PhoneAbsent {
                address: ADDRESS.to_owned(),
            },
            Input::Suspending,
            Input::Resumed,
            Input::TimerFired(Timer::Reconnect(1)),
            Input::TimerFired(Timer::Reconnect(100)),
        ],
    );
    effects.append(&mut more);

    assert_eq!(connects(&effects), Vec::<&Effect>::new());
    assert_eq!(reconnect_timers(&effects), []);
}

#[test]
fn while_calls_are_off_the_indicator_shows_the_off_look_and_turn_calls_on() {
    let state = off();
    let view = state.indicator(&state.surfaces());

    assert_eq!(view.look, IndicatorLook::Off);
    assert_eq!(
        view.status,
        IndicatorStatus::CallsOff {
            until: None,
            quiet_hours: false,
        }
    );
    assert!(!view.needs_attention);
    assert_eq!(messages::indicator_tooltip(&view), "Calls off");
    assert!(view.menu.contains(&IndicatorEntry::TurnCallsOn));
    assert_eq!(
        IndicatorEntry::TurnCallsOn.intent(),
        Some(Input::TurnCallsOn)
    );
    assert!(
        !view
            .menu
            .iter()
            .any(|entry| matches!(entry, IndicatorEntry::TurnCallsOff { .. }))
    );
}

#[test]
fn while_calls_are_on_turn_calls_off_offers_until_i_turn_them_on() {
    let state = connected();
    let view = state.indicator(&state.surfaces());
    let entry = IndicatorEntry::TurnCallsOff {
        enabled: true,
        for_an_hour: None,
        until_morning: None,
    };

    assert_eq!(view.menu[2], entry);
    assert_eq!(messages::indicator_entry(&entry, &view), "Turn calls off");
    assert_eq!(entry.intent(), None);
    let choices = entry.choices();
    assert_eq!(choices, [IndicatorEntry::CallsOffUntilTurnedOn]);
    assert_eq!(
        messages::indicator_entry(&choices[0], &view),
        "Until I turn them on"
    );
    assert_eq!(
        choices[0].intent(),
        Some(Input::TurnCallsOff { until: None })
    );
}

#[test]
fn with_no_phone_set_up_there_is_nothing_to_turn_off() {
    let state = State::default();
    let view = state.indicator(&state.surfaces());

    assert!(
        !view
            .menu
            .iter()
            .any(|entry| matches!(entry, IndicatorEntry::TurnCallsOff { .. }))
    );
    assert_eq!(
        state.refusal(&Input::TurnCallsOff { until: None }),
        Some(Failure::PhoneNotSetUp)
    );
}

#[test]
fn a_call_placed_while_calls_are_off_is_refused_saying_so() {
    let (_, effects) = run(off(), [Input::Dial("07700900123".to_owned())]);

    assert!(
        effects.contains(&Effect::ReportError(Failure::CallsOff)),
        "{effects:?}"
    );
    assert_eq!(connects(&effects), Vec::<&Effect>::new());
}

#[test]
fn turning_calls_on_unblocks_then_connects_at_once_and_clears_the_record() {
    let (state, effects) = run(off(), [Input::TurnCallsOn]);

    assert_eq!(
        effects,
        [
            Effect::KeepCallsOff(Some(record_turning_on())),
            block_phone(false)
        ]
    );
    assert!(!state.calls_off());

    let (_, effects) = run(state, [blocked(false)]);
    assert_eq!(effects[0], Effect::KeepCallsOff(None));
    assert!(
        matches!(&effects[1..], [Effect::ConnectPhone { away: true, .. }]),
        "{effects:?}"
    );
}

#[test]
fn a_busy_first_connect_after_turning_on_is_tried_again_after_the_first_wait() {
    let (state, effects) = run(off(), [Input::TurnCallsOn, blocked(false)]);
    let Some(Effect::ConnectPhone { attempt, .. }) = effects
        .iter()
        .find(|effect| matches!(effect, Effect::ConnectPhone { .. }))
    else {
        panic!("no connect in {effects:?}");
    };

    let (_, effects) = run(
        state,
        [Input::ConnectPhoneFailed {
            attempt: *attempt,
            failure: ConnectFailure::Unreachable,
        }],
    );

    assert!(
        matches!(
            effects[..],
            [Effect::StartTimer {
                timer: Timer::Reconnect(_),
                after_ms: 1_000
            }]
        ),
        "{effects:?}"
    );
}

#[test]
fn an_off_record_kept_by_an_earlier_run_keeps_the_phone_blocked_and_disconnected() {
    let (state, effects) = run(
        State::default(),
        [
            Input::CallsOffKept(record_off()),
            paired(),
            Input::PhoneAbsent {
                address: ADDRESS.to_owned(),
            },
            Input::PairedPhonesListed,
        ],
    );

    assert_eq!(effects[0], block_phone(true));
    assert_eq!(connects(&effects), Vec::<&Effect>::new());
    assert!(state.calls_off());
    // Nothing connects it, so it is never shown as connecting.
    assert_eq!(state.connection_state(), &ConnectionState::Disconnected);
    assert_eq!(state.indicator(&state.surfaces()).look, IndicatorLook::Off);
}

#[test]
fn a_turn_on_an_earlier_run_left_halfway_is_finished_at_startup() {
    let (state, effects) = run(
        State::default(),
        [
            Input::CallsOffKept(record_turning_on()),
            paired(),
            Input::PhoneAbsent {
                address: ADDRESS.to_owned(),
            },
        ],
    );
    assert_eq!(effects[0], block_phone(false));
    assert_eq!(connects(&effects), Vec::<&Effect>::new());

    let (state, effects) = run(state, [blocked(false)]);
    assert_eq!(effects[0], Effect::KeepCallsOff(None));
    assert_eq!(connects(&effects).len(), 1, "{effects:?}");
    assert!(!state.calls_off());
}

#[test]
fn with_no_record_the_desktop_never_blocks_or_unblocks_the_phone() {
    let (_, effects) = run(
        State::default(),
        [
            paired(),
            Input::PhoneAbsent {
                address: ADDRESS.to_owned(),
            },
            Input::PairedPhonesListed,
        ],
    );

    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::BlockPhone { .. })),
        "{effects:?}"
    );
}

#[test]
fn a_phone_that_bluez_wont_block_leaves_calls_on_and_says_so() {
    let (state, effects) = run(
        connected(),
        [
            Input::TurnCallsOff { until: None },
            Input::PhoneBlockFailed {
                address: ADDRESS.to_owned(),
                blocked: true,
            },
        ],
    );

    assert!(!state.calls_off());
    assert!(effects.contains(&Effect::KeepCallsOff(None)));
    assert!(effects.contains(&Effect::ReportError(Failure::CallsOffFailed)));
}

#[test]
fn a_phone_that_bluez_wont_unblock_leaves_calls_off_and_says_so() {
    let (state, effects) = run(
        off(),
        [
            Input::TurnCallsOn,
            Input::PhoneBlockFailed {
                address: ADDRESS.to_owned(),
                blocked: false,
            },
        ],
    );

    assert!(state.calls_off());
    assert_eq!(
        effects.last(),
        Some(&Effect::ReportError(Failure::CallsOnFailed))
    );
    assert!(effects.contains(&Effect::KeepCallsOff(Some(record_off()))));
    assert_eq!(connects(&effects), Vec::<&Effect>::new());
}

#[test]
fn unpairing_the_phone_while_calls_are_off_turns_them_on_and_unblocks_it() {
    let (state, effects) = run(
        off(),
        [Input::PhoneUnpaired {
            address: ADDRESS.to_owned(),
        }],
    );

    assert!(!state.calls_off());
    assert!(effects.contains(&Effect::KeepCallsOff(None)), "{effects:?}");
    assert!(effects.contains(&block_phone(false)), "{effects:?}");
    // BlueZ has forgotten it, so unblocking fails: calls stay on.
    let (state, effects) = run(
        state,
        [Input::PhoneBlockFailed {
            address: ADDRESS.to_owned(),
            blocked: false,
        }],
    );
    assert!(!state.calls_off());
    assert_eq!(effects, []);
    // The next Phone paired is followed as on a Desktop that never turned
    // calls off.
    let other = || {
        [
            Input::PhonePaired(Phone {
                name: "Other".to_owned(),
                address: "11:22:33:44:55:66".to_owned(),
            }),
            Input::PhoneAbsent {
                address: "11:22:33:44:55:66".to_owned(),
            },
            Input::PairedPhonesListed,
        ]
    };
    let (after, effects) = run(state, other());
    let (fresh, fresh_effects) = run(State::default(), other());
    assert_eq!(effects, fresh_effects);
    assert!(!connects(&effects).is_empty(), "{effects:?}");
    assert_eq!(
        after.indicator(&after.surfaces()),
        fresh.indicator(&fresh.surfaces())
    );
}

#[test]
fn calls_kept_off_stay_off_when_blocking_the_phone_again_fails_at_startup() {
    let (state, effects) = run(
        State::default(),
        [
            Input::CallsOffKept(record_off()),
            Input::PhoneBlockFailed {
                address: ADDRESS.to_owned(),
                blocked: true,
            },
            paired(),
            Input::PhonePresent {
                address: ADDRESS.to_owned(),
            },
            Input::PairedPhonesListed,
        ],
    );

    assert!(state.calls_off());
    assert!(
        !effects.contains(&Effect::KeepCallsOff(None)),
        "{effects:?}"
    );
    assert_eq!(connects(&effects), Vec::<&Effect>::new());
    // Turning on still works from there.
    let (_, effects) = run(state, [Input::TurnCallsOn]);
    assert!(effects.contains(&block_phone(false)), "{effects:?}");
}

#[test]
fn while_calls_are_being_turned_on_turn_calls_off_waits() {
    let (state, _) = run(off(), [Input::TurnCallsOn]);
    let view = state.indicator(&state.surfaces());

    assert!(
        view.menu.contains(&IndicatorEntry::TurnCallsOff {
            enabled: false,
            for_an_hour: None,
            until_morning: None,
        }),
        "{:?}",
        view.menu
    );
    let (state, effects) = run(state, [Input::TurnCallsOff { until: None }]);
    assert_eq!(effects, []);
    let (state, _) = run(state, [blocked(false)]);
    assert!(!state.calls_off());
}

#[test]
fn while_calls_are_off_a_problem_is_listed_but_asks_for_no_attention() {
    let (state, _) = run(
        connected(),
        [
            Input::ContactsSyncFailed(SyncFailure::Denied),
            Input::TurnCallsOff { until: None },
            blocked(true),
        ],
    );
    let (state, _) = run(state, dropped());
    let view = state.indicator(&state.surfaces());

    assert_eq!(view.look, IndicatorLook::Off);
    assert!(!view.problems.is_empty(), "{view:?}");
    assert!(!view.needs_attention);
}
