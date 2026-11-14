//! The confirmation raised when the user turns calls off or on by hand
//! (ticket #231), at the Call state machine's public interface: one
//! notice per turn-off or turn-on the user asked for, naming the Phone,
//! and none when Quiet Hours begin or end, a deadline passes, a waiting
//! turn-off applies after the Call, or a Call is placed from the Dialer
//! while calls are off.
//!
//! The clock here starts at 20:30 local time, with Quiet Hours from 22:00
//! to 07:00.

use desktop_calls_core::{
    CallId, CallState, Caller, CallsTurned, CallsTurnedNotice, Deadline, Effect, Input, Phone,
    PhoneCommand, QuietHours, QuietHoursClock, State, TimeOfDay, Timestamp, WallClock, messages,
    step,
};

const ADDRESS: &str = "AA:BB:CC:DD:EE:FF";
const NUMBER: &str = "+447700900123";
const NOW: u64 = 1_800_000_000_000;
const MINUTE: u64 = 60_000;
const CALL: CallId = CallId(1);
/// When the Quiet Hours begin and end, in minutes after 20:30.
const START: u64 = 90;
const END: u64 = 630;

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

fn time(hour: u8, minute: u8) -> TimeOfDay {
    TimeOfDay { hour, minute }
}

/// 21:30, an hour from now.
fn in_an_hour() -> Deadline {
    Deadline {
        at: Timestamp(NOW + 60 * MINUTE),
        shown: time(21, 30),
    }
}

/// 07:00 the next morning.
fn morning() -> Deadline {
    Deadline {
        at: Timestamp(NOW + END * MINUTE),
        shown: time(7, 0),
    }
}

fn quiet_hours() -> QuietHours {
    QuietHours::new(time(22, 0), time(7, 0)).unwrap_or_else(|| unreachable!())
}

/// The wall clock `minutes` after 20:30, with or without Quiet Hours.
fn clock_with(minutes: u64, hours: Option<QuietHours>) -> Input {
    let now = NOW + minutes * MINUTE;
    let day = 24 * 60;
    let next = |first: u64| {
        if minutes < first {
            first
        } else {
            first + ((minutes - first) / day + 1) * day
        }
    };
    Input::ClockTicked(WallClock {
        now: Timestamp(now),
        for_an_hour: Deadline {
            at: Timestamp(now + 60 * MINUTE),
            shown: time(21, 30),
        },
        until_morning: morning(),
        quiet_hours: hours.map(|hours| QuietHoursClock {
            hours,
            next_start: Deadline {
                at: Timestamp(NOW + next(START) * MINUTE),
                shown: time(22, 0),
            },
            next_end: Deadline {
                at: Timestamp(NOW + next(END) * MINUTE),
                shown: time(7, 0),
            },
        }),
    })
}

fn clock(minutes: u64) -> Input {
    clock_with(minutes, None)
}

fn phone_input(make: fn(String) -> Input) -> Input {
    make(ADDRESS.to_owned())
}

fn connected_with(first: Vec<Input>) -> State {
    let mut inputs = first;
    inputs.extend([
        Input::PhonePaired(Phone {
            name: "My Pixel".to_owned(),
            address: ADDRESS.to_owned(),
        }),
        phone_input(|address| Input::PhonePresent { address }),
        phone_input(|address| Input::PhoneConnected { address }),
        Input::PhoneCommandDone(PhoneCommand::KeepAudioOnPhone),
    ]);
    run(State::default(), inputs).0
}

fn connected() -> State {
    connected_with(vec![clock(0)])
}

fn blocked(blocked: bool) -> Input {
    Input::PhoneBlocked {
        address: ADDRESS.to_owned(),
        blocked,
    }
}

/// BlueZ drops a blocked Phone at once.
fn dropped() -> [Input; 2] {
    [
        phone_input(|address| Input::PhoneDisconnected { address }),
        phone_input(|address| Input::PhoneAbsent { address }),
    ]
}

/// Calls off until `until`, with the Phone blocked and dropped.
fn off(until: Option<Deadline>) -> State {
    let (state, _) = run(connected(), [Input::TurnCallsOff { until }, blocked(true)]);
    run(state, dropped()).0
}

fn call_added() -> Input {
    Input::CallAdded {
        call: CALL,
        state: CallState::Active,
        caller: Caller::Number(NUMBER.to_owned()),
        at: Timestamp(NOW),
    }
}

fn call_ended(minutes: u64) -> Input {
    Input::CallEnded {
        call: CALL,
        at: Timestamp(NOW + minutes * MINUTE),
    }
}

/// Every confirmation among `effects`.
fn notices(effects: &[Effect]) -> Vec<&CallsTurnedNotice> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::NotifyCallsTurned(notice) => Some(notice),
            _ => None,
        })
        .collect()
}

/// The one confirmation among `effects`, with what it says.
fn said(effects: &[Effect]) -> (messages::NotificationText, CallsTurned) {
    let notices = notices(effects);
    assert_eq!(notices.len(), 1, "{effects:?}");
    (messages::calls_turned_notice(notices[0]), notices[0].turned)
}

#[test]
fn turning_calls_off_until_turned_on_says_so_and_names_the_phone() {
    let (_, effects) = run(connected(), [Input::TurnCallsOff { until: None }]);

    let (text, turned) = said(&effects);

    assert_eq!(
        turned,
        CallsTurned::Off {
            until: None,
            for_an_hour: false
        }
    );
    assert_eq!(text.summary, "Calls off");
    assert_eq!(
        text.body,
        "My Pixel is disconnected. Turn calls on from the bar."
    );
}

#[test]
fn turning_calls_off_until_the_morning_gives_the_time() {
    let (_, effects) = run(
        connected(),
        [Input::TurnCallsOff {
            until: Some(morning()),
        }],
    );

    let (text, _) = said(&effects);

    assert_eq!(text.summary, "Calls off until 07:00");
    assert_eq!(
        text.body,
        "My Pixel is disconnected. Turn calls on from the bar."
    );
}

#[test]
fn turning_calls_off_for_an_hour_says_for_1_hour_with_the_end_time() {
    let (_, effects) = run(
        connected(),
        [Input::TurnCallsOff {
            until: Some(in_an_hour()),
        }],
    );

    let (text, turned) = said(&effects);

    assert_eq!(
        turned,
        CallsTurned::Off {
            until: Some(in_an_hour()),
            for_an_hour: true
        }
    );
    assert_eq!(text.summary, "Calls off for 1 hour (until 21:30)");
}

#[test]
fn a_length_of_the_users_own_choosing_gives_only_its_end_time() {
    let two_hours = Deadline {
        at: Timestamp(NOW + 120 * MINUTE),
        shown: time(22, 30),
    };
    let (_, effects) = run(
        connected(),
        [Input::TurnCallsOff {
            until: Some(two_hours),
        }],
    );

    assert_eq!(said(&effects).0.summary, "Calls off until 22:30");
}

#[test]
fn turning_calls_on_says_the_phone_will_reconnect() {
    let (_, effects) = run(off(None), [Input::TurnCallsOn]);

    let (text, turned) = said(&effects);

    assert_eq!(turned, CallsTurned::On { reconnecting: true });
    assert_eq!(text.summary, "Calls on");
    assert_eq!(text.body, "My Pixel will reconnect.");
}

#[test]
fn a_turn_off_during_a_call_says_calls_go_off_after_it() {
    let (state, effects) = run(connected(), [call_added()]);
    assert!(notices(&effects).is_empty());

    let (_, effects) = run(
        state,
        [Input::TurnCallsOff {
            until: Some(morning()),
        }],
    );

    let (text, turned) = said(&effects);
    assert_eq!(
        turned,
        CallsTurned::OffAfterCall {
            until: Some(morning())
        }
    );
    assert_eq!(text.summary, "Calls off after this Call (until 07:00)");
    assert_eq!(
        text.body,
        "My Pixel stays connected until then. Keep calls on from the bar to cancel."
    );
}

#[test]
fn a_waiting_turn_off_applying_after_the_call_says_nothing() {
    let (state, _) = run(
        connected(),
        [call_added(), Input::TurnCallsOff { until: None }],
    );

    let (state, effects) = run(state, [call_ended(1)]);

    assert!(state.calls_off_pending() || state.calls_off(), "{state:?}");
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::BlockPhone { blocked: true, .. })),
        "the turn-off applied: {effects:?}"
    );
    assert!(notices(&effects).is_empty(), "{effects:?}");
}

#[test]
fn keeping_calls_on_while_a_turn_off_waits_says_the_phone_stays_connected() {
    let (state, _) = run(
        connected(),
        [call_added(), Input::TurnCallsOff { until: None }],
    );

    let (_, effects) = run(state, [Input::KeepCallsOn]);

    let (text, turned) = said(&effects);
    assert_eq!(
        turned,
        CallsTurned::On {
            reconnecting: false
        }
    );
    assert_eq!(text.summary, "Calls on");
    assert_eq!(text.body, "My Pixel stays connected.");
}

#[test]
fn turning_calls_on_while_a_turn_off_waits_says_the_same() {
    let (state, _) = run(
        connected(),
        [call_added(), Input::TurnCallsOff { until: None }],
    );

    let (_, effects) = run(state, [Input::TurnCallsOn]);

    assert_eq!(
        said(&effects).1,
        CallsTurned::On {
            reconnecting: false
        }
    );
}

#[test]
fn asks_that_change_nothing_say_nothing() {
    // Calls on already.
    let (_, effects) = run(connected(), [Input::TurnCallsOn, Input::KeepCallsOn]);
    assert!(notices(&effects).is_empty(), "{effects:?}");

    // Keep calls on does nothing while calls are off, as while on.
    let (state, effects) = run(off(None), [Input::KeepCallsOn]);
    assert!(state.calls_off(), "calls stay off");
    assert!(notices(&effects).is_empty(), "{effects:?}");

    // A time that has passed is refused.
    let passed = Deadline {
        at: Timestamp(NOW - MINUTE),
        shown: time(20, 29),
    };
    let (_, effects) = run(
        connected(),
        [Input::TurnCallsOff {
            until: Some(passed),
        }],
    );
    assert!(notices(&effects).is_empty(), "{effects:?}");

    // The same choice again, while off.
    let (_, effects) = run(off(None), [Input::TurnCallsOff { until: None }]);
    assert!(notices(&effects).is_empty(), "{effects:?}");

    // While the Phone is still being unblocked.
    let (state, _) = run(off(None), [Input::TurnCallsOn]);
    let (_, effects) = run(state, [Input::TurnCallsOn]);
    assert!(notices(&effects).is_empty(), "{effects:?}");
}

#[test]
fn a_new_choice_while_off_is_said() {
    let (_, effects) = run(
        off(None),
        [Input::TurnCallsOff {
            until: Some(morning()),
        }],
    );

    assert_eq!(said(&effects).0.summary, "Calls off until 07:00");
}

#[test]
fn quiet_hours_beginning_and_ending_say_nothing() {
    let state = connected_with(vec![
        Input::QuietHoursKept(quiet_hours()),
        clock_with(0, Some(quiet_hours())),
    ]);

    let (state, begins) = run(state, [clock_with(START, Some(quiet_hours()))]);
    assert!(state.calls_off(), "Quiet Hours turned calls off");
    let (state, _) = run(state, [blocked(true)]);
    let (state, _) = run(state, dropped());
    let (state, ends) = run(state, [clock_with(END, Some(quiet_hours()))]);

    assert!(!state.calls_off());
    assert!(notices(&begins).is_empty(), "{begins:?}");
    assert!(notices(&ends).is_empty(), "{ends:?}");
}

#[test]
fn quiet_hours_beginning_during_a_call_and_applying_after_it_say_nothing() {
    let state = connected_with(vec![
        Input::QuietHoursKept(quiet_hours()),
        clock_with(0, Some(quiet_hours())),
    ]);
    let (state, effects) = run(state, [call_added()]);
    assert!(notices(&effects).is_empty());

    let (state, begins) = run(state, [clock_with(START, Some(quiet_hours()))]);
    let (_, after) = run(state, [call_ended(START + 1)]);

    assert!(notices(&begins).is_empty(), "{begins:?}");
    assert!(notices(&after).is_empty(), "{after:?}");
}

#[test]
fn a_deadline_passing_says_nothing() {
    let state = off(Some(in_an_hour()));

    let (state, effects) = run(state, [clock(61)]);

    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::BlockPhone { blocked: false, .. })),
        "calls came back on: {effects:?}"
    );
    assert!(!state.calls_off());
    assert!(notices(&effects).is_empty(), "{effects:?}");
}

#[test]
fn a_call_placed_from_the_dialer_while_calls_are_off_says_nothing_going_or_returning() {
    let (state, effects) = run(
        off(None),
        [
            Input::OpenDialer {
                query: NUMBER.to_owned(),
                select_call: None,
            },
            Input::Dial(NUMBER.to_owned()),
        ],
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::BlockPhone { blocked: false, .. })),
        "the Phone was unblocked for the Call: {effects:?}"
    );
    assert!(notices(&effects).is_empty(), "{effects:?}");

    let (state, unblocked) = run(state, [blocked(false)]);
    let (state, hung_up) = run(state, [Input::CancelDial]);

    assert!(notices(&unblocked).is_empty(), "{unblocked:?}");
    assert!(notices(&hung_up).is_empty(), "{hung_up:?}");
    assert!(
        state.calls_off() || state.calls_off_pending(),
        "calls go back off: {state:?}"
    );
}

#[test]
fn an_hour_asked_for_by_the_command_line_or_just_after_a_clock_tick_is_still_for_1_hour() {
    // The command line's `--for 1h`: now by its own clock, to the second.
    let by_cli = Deadline {
        at: Timestamp(NOW + 60 * MINUTE + 17_000),
        shown: time(21, 30),
    };
    // A menu entry built before the clock ticked a minute on.
    let before_tick = Deadline {
        at: Timestamp(NOW + 60 * MINUTE),
        shown: time(21, 30),
    };
    for until in [by_cli, before_tick] {
        let (_, effects) = run(
            connected_with(vec![clock(1)]),
            [Input::TurnCallsOff { until: Some(until) }],
        );

        let (text, _) = said(&effects);

        assert_eq!(text.summary, "Calls off for 1 hour (until 21:30)");
    }
}
