//! A Call placed from the Dialer while calls are off, at the Call state
//! machine's public interface (ticket #206): the Dialer says calls are off
//! and that Enter calls anyway; Enter unblocks and connects the Phone for
//! that one Call and places it once the Phone connects; when it ends,
//! calls go off again for whatever remains of the state they were in.
//! Hang up first, or no connection in 15 s, puts them back off at once;
//! Keep calls on cancels the return to off.

use desktop_calls_core::{
    CallId, CallState, CallWindowPhase, Caller, CallsOffRecord, Deadline, DialProgress,
    DialerCallsOff, DialerKey, Effect, IndicatorEntry, IndicatorLook, IndicatorStatus, Input,
    Phone, PhoneCommand, QuietHours, QuietHoursClock, State, StateLine, TimeOfDay, Timer,
    Timestamp, WallClock,
    messages::{self, Failure},
    step,
};

const ADDRESS: &str = "AA:BB:CC:DD:EE:FF";
const NUMBER: &str = "+447700900123";
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

/// 21:30, an hour from now.
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

fn phone_input(make: fn(String) -> Input) -> Input {
    make(ADDRESS.to_owned())
}

/// The Phone connects: present, hands-free up, and ready for Calls.
fn phone_connects() -> [Input; 3] {
    [
        phone_input(|address| Input::PhonePresent { address }),
        phone_input(|address| Input::PhoneConnected { address }),
        Input::PhoneCommandDone(PhoneCommand::KeepAudioOnPhone),
    ]
}

fn blocked(blocked: bool) -> Input {
    Input::PhoneBlocked {
        address: ADDRESS.to_owned(),
        blocked,
    }
}

fn block_phone(blocked: bool) -> Effect {
    Effect::BlockPhone {
        address: ADDRESS.to_owned(),
        blocked,
    }
}

/// Calls off until `until`, or until turned on, with the Phone dropped.
fn off(until: Option<Deadline>) -> State {
    let mut inputs = vec![
        clock(0),
        Input::PhonePaired(Phone {
            name: "My Pixel".to_owned(),
            address: ADDRESS.to_owned(),
        }),
    ];
    inputs.extend(phone_connects());
    inputs.extend([
        Input::TurnCallsOff { until },
        blocked(true),
        phone_input(|address| Input::PhoneDisconnected { address }),
        phone_input(|address| Input::PhoneAbsent { address }),
    ]);
    let (state, _) = run(State::default(), inputs);
    assert!(state.calls_off());
    state
}

/// The Dialer open on `NUMBER`, typed.
fn dialer_open(state: State) -> State {
    run(
        state,
        [Input::OpenDialer {
            query: NUMBER.to_owned(),
            select_call: None,
        }],
    )
    .0
}

fn record_pending(until: Option<Deadline>) -> CallsOffRecord {
    CallsOffRecord::Pending {
        address: ADDRESS.to_owned(),
        until,
        quiet_hours: false,
        exception: true,
    }
}

fn record_off(until: Option<Deadline>) -> CallsOffRecord {
    CallsOffRecord::Off {
        address: ADDRESS.to_owned(),
        until,
        quiet_hours: false,
    }
}

fn dials(effects: &[Effect]) -> bool {
    effects
        .iter()
        .any(|effect| matches!(effect, Effect::Phone(PhoneCommand::Dial(_))))
}

fn connects(effects: &[Effect]) -> bool {
    effects
        .iter()
        .any(|effect| matches!(effect, Effect::ConnectPhone { .. }))
}

/// Enter pressed in the Dialer while calls are off: the Phone is being
/// unblocked for the Call.
fn entered(until: Option<Deadline>) -> State {
    run(dialer_open(off(until)), [Input::DialerEnter]).0
}

/// The Phone unblocked and connected for the Call, which is placed.
fn placed(until: Option<Deadline>) -> State {
    let (state, _) = run(entered(until), [blocked(false)]);
    let (state, _) = run(state, phone_connects());
    run(
        state,
        [Input::CallAdded {
            call: CALL,
            state: CallState::Dialing,
            caller: Caller::Number(NUMBER.to_owned()),
            at: Timestamp(NOW),
        }],
    )
    .0
}

fn ended(minutes: u64) -> Input {
    Input::CallEnded {
        call: CALL,
        at: Timestamp(NOW + minutes * MINUTE),
    }
}

#[test]
fn while_calls_are_off_the_dialer_searches_and_says_enter_calls_anyway() {
    let state = dialer_open(off(Some(in_an_hour())));

    let Some(dialer) = state.surfaces().dialer else {
        panic!("the Dialer isn't open");
    };
    assert!(!dialer.rows.is_empty(), "{dialer:?}");
    assert_eq!(
        dialer.calls_off,
        Some(DialerCallsOff {
            until: Some(in_an_hour().shown)
        })
    );
    assert_eq!(dialer.keys[0], DialerKey::CallAnyway);
    assert_eq!(dialer.notice, None);
    assert_eq!(
        messages::dialer_calls_off(dialer.calls_off.and_then(|off| off.until)),
        (
            "Calls are off until 21:30.".to_owned(),
            "Enter calls anyway; calls go off again after."
        )
    );
    assert_eq!(messages::dialer_calls_off(None).0, "Calls are off.");
    assert_eq!(messages::dialer_key(DialerKey::CallAnyway).1, "Call anyway");
}

#[test]
fn enter_while_calls_are_off_unblocks_the_phone_and_says_connecting() {
    let (state, effects) = run(dialer_open(off(None)), [Input::DialerEnter]);

    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(effect, Effect::KeepCallsOff(_) | Effect::BlockPhone { .. }))
            .collect::<Vec<_>>(),
        [
            &Effect::KeepCallsOff(Some(record_pending(None))),
            &block_phone(false)
        ]
    );
    assert!(!dials(&effects) && !connects(&effects), "{effects:?}");
    assert!(effects.contains(&Effect::DialProgress(DialProgress::Asked(1))));
    let surfaces = state.surfaces();
    assert_eq!(surfaces.dialer, None);
    let Some(window) = &surfaces.call_window else {
        panic!("no Call Window");
    };
    assert_eq!(window.state_line, Some(StateLine::ConnectingToPhone));
    assert_eq!(
        messages::state_line(StateLine::ConnectingToPhone, &window.caller, None),
        "Connecting to your phone…"
    );
    let view = state.indicator(&surfaces);
    assert_eq!(view.status, IndicatorStatus::CallsOffAgain);
    assert_eq!(
        messages::indicator_status(&view.status),
        "Calls off again when this Call ends"
    );
    assert!(view.menu.contains(&IndicatorEntry::KeepCallsOn), "{view:?}");
    assert_ne!(view.look, IndicatorLook::Off);
}

#[test]
fn the_call_is_placed_once_the_phone_connects_and_calls_go_off_again_after() {
    let (state, effects) = run(entered(None), [blocked(false)]);
    assert!(connects(&effects), "{effects:?}");
    assert!(!dials(&effects));
    assert!(effects.contains(&Effect::StartTimer {
        timer: Timer::PhoneForDial(1),
        after_ms: 15_000
    }));

    let (state, effects) = run(state, phone_connects());
    assert!(
        effects.contains(&Effect::Phone(PhoneCommand::Dial(NUMBER.to_owned()))),
        "{effects:?}"
    );
    let state = run(
        state,
        [Input::CallAdded {
            call: CALL,
            state: CallState::Dialing,
            caller: Caller::Number(NUMBER.to_owned()),
            at: Timestamp(NOW),
        }],
    )
    .0;
    assert!(!state.calls_off());
    assert_eq!(
        state.indicator(&state.surfaces()).status,
        IndicatorStatus::CallsOffAgain
    );

    let (state, effects) = run(state, [ended(5)]);
    assert!(state.calls_off());
    assert_eq!(state.calls_off_until(), None);
    let kept = effects
        .iter()
        .position(|effect| *effect == Effect::KeepCallsOff(Some(record_off(None))));
    let block = effects
        .iter()
        .position(|effect| *effect == block_phone(true));
    assert!(kept.is_some() && kept < block, "{effects:?}");
}

#[test]
fn a_timed_break_goes_off_again_until_its_deadline() {
    let (state, _) = run(placed(Some(in_an_hour())), [clock(5), ended(5)]);

    assert!(state.calls_off());
    assert_eq!(state.calls_off_until(), Some(in_an_hour()));
}

#[test]
fn a_deadline_that_passed_during_the_call_leaves_calls_on() {
    let (state, effects) = run(placed(Some(in_an_hour())), [clock(61), ended(61)]);

    assert!(!state.calls_off());
    assert!(!state.calls_off_pending());
    assert!(!effects.contains(&block_phone(true)), "{effects:?}");
}

#[test]
fn hang_up_before_the_phone_connects_cancels_the_call_and_calls_go_back_off() {
    let (state, _) = run(entered(None), [blocked(false)]);

    let (state, effects) = run(state, [Input::CancelDial]);

    assert!(state.calls_off());
    assert!(effects.contains(&block_phone(true)), "{effects:?}");
    assert!(!dials(&effects));
    let window = state.surfaces().call_window;
    assert_eq!(
        window.map(|window| window.phase),
        Some(CallWindowPhase::Ended)
    );
    // The Phone connecting now places nothing.
    let (_, effects) = run(state, phone_connects());
    assert!(!dials(&effects), "{effects:?}");
}

#[test]
fn hang_up_while_the_phone_is_being_unblocked_puts_calls_back_off_without_a_connect() {
    let (state, _) = run(entered(None), [Input::CancelDial]);

    let (state, effects) = run(state, [blocked(false)]);

    assert!(state.calls_off());
    assert!(!connects(&effects), "{effects:?}");
    assert!(effects.contains(&block_phone(true)), "{effects:?}");
}

#[test]
fn no_connection_in_15_s_says_out_of_range_and_calls_go_back_off() {
    let (state, _) = run(entered(None), [blocked(false)]);

    let (state, effects) = run(state, [Input::TimerFired(Timer::PhoneForDial(1))]);

    assert!(state.calls_off());
    assert!(effects.contains(&block_phone(true)), "{effects:?}");
    assert!(
        effects.contains(&Effect::DialProgress(DialProgress::NotPlaced(
            1,
            Failure::PhoneOutOfRange
        )))
    );
    let Some(window) = state.surfaces().call_window else {
        panic!("no Call Window");
    };
    assert_eq!(window.phase, CallWindowPhase::NotPlaced);
    assert_eq!(window.notice, Some(Failure::PhoneOutOfRange));
}

#[test]
fn keep_calls_on_during_the_exception_leaves_calls_on_after_the_call() {
    let (state, _) = run(placed(None), [Input::KeepCallsOn]);
    assert!(!state.calls_off_pending());

    let (state, effects) = run(state, [ended(5)]);

    assert!(!state.calls_off());
    assert!(!effects.contains(&block_phone(true)), "{effects:?}");
}

#[test]
fn keep_calls_on_while_the_phone_is_being_unblocked_leaves_calls_on() {
    let (state, _) = run(entered(None), [Input::KeepCallsOn, blocked(false)]);
    let (state, _) = run(state, phone_connects());
    let (state, effects) = run(
        state,
        [
            Input::CallAdded {
                call: CALL,
                state: CallState::Dialing,
                caller: Caller::Number(NUMBER.to_owned()),
                at: Timestamp(NOW),
            },
            ended(5),
        ],
    );

    assert!(!state.calls_off());
    assert!(!effects.contains(&block_phone(true)), "{effects:?}");
}

#[test]
fn kept_on_while_the_phone_is_being_unblocked_the_call_still_waits_only_15_s() {
    let (state, effects) = run(entered(None), [Input::KeepCallsOn, blocked(false)]);
    assert!(connects(&effects), "{effects:?}");
    assert!(
        effects.contains(&Effect::StartTimer {
            timer: Timer::PhoneForDial(1),
            after_ms: 15_000
        }),
        "{effects:?}"
    );

    let (state, effects) = run(state, [Input::TimerFired(Timer::PhoneForDial(1))]);

    assert!(
        effects.contains(&Effect::DialProgress(DialProgress::NotPlaced(
            1,
            Failure::PhoneOutOfRange
        )))
    );
    let Some(window) = state.surfaces().call_window else {
        panic!("no Call Window");
    };
    assert_eq!(window.notice, Some(Failure::PhoneOutOfRange));
    assert!(!state.calls_off(), "calls were kept on");
}

#[test]
fn bluez_refusing_the_unblock_leaves_calls_off_and_the_call_window_says_why() {
    let (state, effects) = run(
        entered(Some(in_an_hour())),
        [Input::PhoneBlockFailed {
            address: ADDRESS.to_owned(),
            blocked: false,
        }],
    );

    assert!(state.calls_off());
    assert_eq!(state.calls_off_until(), Some(in_an_hour()));
    assert!(
        effects.contains(&Effect::KeepCallsOff(Some(record_off(Some(in_an_hour()))))),
        "{effects:?}"
    );
    assert!(
        effects.contains(&Effect::DialProgress(DialProgress::NotPlaced(
            1,
            Failure::CallsOnFailed
        )))
    );
    // Said once, as the Call not placed.
    assert_eq!(
        effects
            .iter()
            .filter(|effect| **effect == Effect::ReportError(Failure::CallsOnFailed))
            .count(),
        1,
        "{effects:?}"
    );
    let Some(window) = state.surfaces().call_window else {
        panic!("no Call Window");
    };
    assert_eq!(window.phase, CallWindowPhase::NotPlaced);
    assert_eq!(window.notice, Some(Failure::CallsOnFailed));
}

#[test]
fn a_call_that_rings_during_the_exception_keeps_calls_on_until_it_ends_too() {
    let (state, _) = run(
        placed(None),
        [Input::CallAdded {
            call: CallId(2),
            state: CallState::Waiting,
            caller: Caller::Number("+447700900456".to_owned()),
            at: Timestamp(NOW),
        }],
    );
    let (state, _) = run(state, [ended(5)]);
    assert!(state.calls_off_pending(), "the Waiting Call is still live");

    let (state, _) = run(
        state,
        [Input::CallEnded {
            call: CallId(2),
            at: Timestamp(NOW + 6 * MINUTE),
        }],
    );
    assert!(state.calls_off());
}

#[test]
fn the_exception_is_kept_for_the_next_run_and_goes_off_once_its_call_ends() {
    let (state, effects) = run(
        State::default(),
        [
            clock(0),
            Input::CallsOffKept(record_pending(None)),
            Input::PhonePaired(Phone {
                name: "My Pixel".to_owned(),
                address: ADDRESS.to_owned(),
            }),
        ],
    );
    assert!(state.calls_off_pending());
    assert!(!effects.contains(&block_phone(true)));

    let mut inputs = phone_connects().to_vec();
    inputs.push(Input::CallAdded {
        call: CALL,
        state: CallState::Active,
        caller: Caller::Number(NUMBER.to_owned()),
        at: Timestamp(NOW),
    });
    let (state, _) = run(state, inputs);
    assert_eq!(
        state.indicator(&state.surfaces()).status,
        IndicatorStatus::CallsOffAgain
    );
    let (state, effects) = run(state, [ended(5)]);
    assert!(state.calls_off());
    assert!(effects.contains(&block_phone(true)));
}

/// Quiet Hours from 20:00 to 21:00, and the wall clock `minutes` after
/// 20:30: under way until 21:00.
fn quiet_clock(minutes: u64) -> Input {
    let now = NOW + minutes * MINUTE;
    let at = |offset: u64, hour: u8| Deadline {
        at: Timestamp(NOW + offset * MINUTE),
        shown: TimeOfDay { hour, minute: 0 },
    };
    let (next_start, next_end) = if minutes < 30 {
        (at(1_410, 20), at(30, 21))
    } else {
        (at(1_410, 20), at(1_470, 21))
    };
    let Some(hours) = QuietHours::new(
        TimeOfDay {
            hour: 20,
            minute: 0,
        },
        TimeOfDay {
            hour: 21,
            minute: 0,
        },
    ) else {
        panic!("different times");
    };
    Input::ClockTicked(WallClock {
        now: Timestamp(now),
        for_an_hour: Deadline {
            at: Timestamp(now + 60 * MINUTE),
            shown: TimeOfDay {
                hour: 21,
                minute: 30,
            },
        },
        until_morning: next_end,
        quiet_hours: Some(QuietHoursClock {
            hours,
            next_start,
            next_end,
        }),
    })
}

/// Calls off for Quiet Hours until 21:00, and a Call placed from the
/// Dialer meanwhile.
fn placed_in_quiet_hours() -> State {
    let Some(hours) = QuietHours::new(
        TimeOfDay {
            hour: 20,
            minute: 0,
        },
        TimeOfDay {
            hour: 21,
            minute: 0,
        },
    ) else {
        panic!("different times");
    };
    let mut inputs = vec![
        Input::QuietHoursKept(hours),
        quiet_clock(0),
        Input::PhonePaired(Phone {
            name: "My Pixel".to_owned(),
            address: ADDRESS.to_owned(),
        }),
        blocked(true),
    ];
    inputs.extend([
        Input::OpenDialer {
            query: NUMBER.to_owned(),
            select_call: None,
        },
        Input::DialerEnter,
        blocked(false),
    ]);
    let (state, _) = run(State::default(), inputs);
    let (state, _) = run(state, phone_connects());
    let (state, _) = run(
        state,
        [Input::CallAdded {
            call: CALL,
            state: CallState::Dialing,
            caller: Caller::Number(NUMBER.to_owned()),
            at: Timestamp(NOW),
        }],
    );
    assert!(state.calls_off_pending(), "the Call is an exception");
    state
}

#[test]
fn quiet_hours_go_off_again_until_they_end() {
    let (state, _) = run(placed_in_quiet_hours(), [quiet_clock(5), ended(5)]);

    assert!(state.calls_off());
    assert!(state.calls_off_for_quiet_hours());
    assert_eq!(
        state.calls_off_until().map(|until| until.shown.hour),
        Some(21)
    );
}

#[test]
fn quiet_hours_that_ended_during_the_call_leave_calls_on() {
    let (state, effects) = run(placed_in_quiet_hours(), [quiet_clock(31), ended(31)]);

    assert!(!state.calls_off());
    assert!(!effects.contains(&block_phone(true)), "{effects:?}");
}

/// Ticket #230: whether the Call Window carries the "Calls off after" tag.
fn calls_off_after(state: &State) -> bool {
    let Some(window) = state.surfaces().call_window else {
        panic!("the Call Window shows");
    };
    window.calls_off_after
}

#[test]
fn the_call_window_says_calls_off_after_from_connecting_to_the_phone_to_the_end_of_the_call() {
    let connecting = entered(None);
    let Some(window) = connecting.surfaces().call_window else {
        panic!("the Call Window shows");
    };
    assert_eq!(window.state_line, Some(StateLine::ConnectingToPhone));
    assert!(window.calls_off_after, "tagged while the Phone connects");

    let placed = placed(None);
    let Some(window) = placed.surfaces().call_window else {
        panic!("the Call Window shows");
    };
    assert_eq!(window.state_line, Some(StateLine::Calling));
    assert!(
        window.calls_off_after,
        "tagged through the Call's own states"
    );

    let (ended_state, _) = run(placed, [ended(5)]);
    assert!(
        !calls_off_after(&ended_state),
        "the Call ended: no tag on its Ended window"
    );
}

#[test]
fn keep_calls_on_takes_the_tag_away_during_the_exception() {
    let (state, _) = run(entered(None), [Input::KeepCallsOn]);

    assert!(!calls_off_after(&state));
}

#[test]
fn hanging_up_before_the_phone_connects_leaves_no_tag_on_the_ended_window() {
    let (state, _) = run(entered(None), [Input::CancelDial]);

    assert!(!calls_off_after(&state));
}
