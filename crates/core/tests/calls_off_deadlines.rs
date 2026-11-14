//! Calls Off for 1 hour or until tomorrow morning, at the Call state
//! machine's public interface (ticket #203): the menu shows each choice's
//! end time, the deadline is kept for the next run, and calls come back on
//! by themselves once the wall clock passes it, at once if it passed while
//! the app was stopped, and only after a resume if it passed during a
//! suspend.

use desktop_calls_core::{
    CallsOffRecord, Deadline, Effect, IndicatorEntry, IndicatorStatus, Input, Phone, PhoneCommand,
    State, TimeOfDay, Timestamp, WallClock,
    messages::{self, Failure},
    step,
};

const ADDRESS: &str = "AA:BB:CC:DD:EE:FF";
/// 20:30 local time, as the shell's clock read it.
const NOW: u64 = 1_800_000_000_000;
const MINUTE: u64 = 60_000;

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

/// In an hour: 21:30.
fn in_an_hour() -> Deadline {
    Deadline {
        at: Timestamp(NOW + 60 * MINUTE),
        shown: time(21, 30),
    }
}

/// The next morning: 07:00, ten and a half hours on.
fn morning() -> Deadline {
    Deadline {
        at: Timestamp(NOW + 630 * MINUTE),
        shown: time(7, 0),
    }
}

/// The wall clock `minutes` after 20:30. The timed choices' deadlines move
/// on with it; their local times don't matter here.
fn clock(minutes: u64) -> Input {
    let now = NOW + minutes * MINUTE;
    Input::ClockTicked(WallClock {
        now: Timestamp(now),
        for_an_hour: Deadline {
            at: Timestamp(now + 60 * MINUTE),
            shown: time(21, 30),
        },
        until_morning: morning(),
        quiet_hours: None,
    })
}

fn paired() -> Input {
    Input::PhonePaired(Phone {
        name: "My Pixel".to_owned(),
        address: ADDRESS.to_owned(),
    })
}

fn connected() -> State {
    run(
        State::default(),
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
        ],
    )
    .0
}

fn blocked(blocked: bool) -> Input {
    Input::PhoneBlocked {
        address: ADDRESS.to_owned(),
        blocked,
    }
}

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

fn block_phone(blocked: bool) -> Effect {
    Effect::BlockPhone {
        address: ADDRESS.to_owned(),
        blocked,
    }
}

/// Calls off for an hour, the Phone blocked and dropped.
fn off_for_an_hour() -> State {
    let (state, _) = run(
        connected(),
        [
            Input::TurnCallsOff {
                until: Some(in_an_hour()),
            },
            blocked(true),
        ],
    );
    run(state, dropped()).0
}

fn record_off(until: Deadline) -> CallsOffRecord {
    CallsOffRecord::Off {
        address: ADDRESS.to_owned(),
        until: Some(until),
        quiet_hours: false,
    }
}

fn record_turning_on() -> CallsOffRecord {
    CallsOffRecord::TurningOn {
        address: ADDRESS.to_owned(),
        kept_on_until: None,
    }
}

fn connects(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|effect| matches!(effect, Effect::ConnectPhone { .. }))
        .count()
}

fn status(state: &State) -> String {
    let view = state.indicator(&state.surfaces());
    messages::indicator_status(&view.status)
}

/// The deadline passed: it is logged, the record says calls are turning
/// on, and the Phone is unblocked.
fn coming_back_on() -> Vec<Effect> {
    vec![
        Effect::CallsOffDeadlinePassed {
            address: ADDRESS.to_owned(),
        },
        Effect::KeepCallsOff(Some(record_turning_on())),
        block_phone(false),
    ]
}

#[test]
fn turn_calls_off_offers_an_hour_and_the_morning_with_their_end_times() {
    let state = connected();
    let view = state.indicator(&state.surfaces());
    let entry = view
        .menu
        .iter()
        .find(|entry| matches!(entry, IndicatorEntry::TurnCallsOff { .. }))
        .expect("Turn calls off");

    let choices = entry.choices();
    let labels: Vec<String> = choices
        .iter()
        .map(|choice| messages::indicator_entry(choice, &view))
        .collect();

    assert_eq!(
        labels,
        [
            "Until I turn them on",
            "For 1 hour (until 21:30)",
            "Until tomorrow morning (07:00)"
        ]
    );
    assert_eq!(
        choices[1].intent(),
        Some(Input::TurnCallsOff {
            until: Some(in_an_hour())
        })
    );
    assert_eq!(
        choices[2].intent(),
        Some(Input::TurnCallsOff {
            until: Some(morning())
        })
    );
}

#[test]
fn before_the_wall_clock_is_read_only_until_i_turn_them_on_is_offered() {
    let (state, _) = run(State::default(), [paired()]);
    let view = state.indicator(&state.surfaces());
    let entry = view
        .menu
        .iter()
        .find(|entry| matches!(entry, IndicatorEntry::TurnCallsOff { .. }))
        .expect("Turn calls off");

    assert_eq!(entry.choices(), [IndicatorEntry::CallsOffUntilTurnedOn]);
}

#[test]
fn turning_calls_off_for_an_hour_keeps_the_deadline_and_asks_for_the_clock_then() {
    let (state, effects) = run(
        connected(),
        [Input::TurnCallsOff {
            until: Some(in_an_hour()),
        }],
    );

    assert_eq!(
        effects,
        [
            Effect::KeepCallsOff(Some(record_off(in_an_hour()))),
            block_phone(true),
            Effect::ReadClockAt(Timestamp(NOW + 60 * MINUTE)),
        ]
    );
    assert_eq!(state.calls_off_until(), Some(in_an_hour()));
    assert_eq!(status(&state), "Calls off until 21:30");
    assert_eq!(
        state.indicator(&state.surfaces()).status,
        IndicatorStatus::CallsOff {
            until: Some(time(21, 30)),
            quiet_hours: false,
        }
    );
}

#[test]
fn calls_stay_off_until_the_deadline_then_come_back_on_and_connect_at_once() {
    let (state, effects) = run(off_for_an_hour(), [clock(1), clock(59)]);
    assert!(effects.is_empty(), "{effects:?}");
    assert!(state.calls_off());

    let (state, effects) = run(state, [clock(60)]);
    assert_eq!(effects, coming_back_on());
    assert!(!state.calls_off());

    let (state, effects) = run(state, [blocked(false)]);
    assert!(effects.contains(&Effect::KeepCallsOff(None)), "{effects:?}");
    assert_eq!(connects(&effects), 1, "{effects:?}");
    assert!(!state.calls_off());
}

#[test]
fn a_deadline_that_passes_during_a_suspend_takes_effect_on_the_resume() {
    let (state, effects) = run(off_for_an_hour(), [Input::Suspending, clock(90)]);
    assert!(effects.is_empty(), "{effects:?}");
    assert!(state.calls_off(), "nothing changes as the Desktop sleeps");

    // The resume itself connects nothing while calls are off: the clock,
    // read again on the resume, says the deadline passed.
    let (state, effects) = run(state, [Input::Resumed]);
    assert_eq!(connects(&effects), 0, "{effects:?}");
    assert!(state.calls_off());

    let (state, effects) = run(state, [clock(90)]);
    assert_eq!(effects, coming_back_on());
    let (_, effects) = run(state, [blocked(false)]);
    assert_eq!(connects(&effects), 1, "{effects:?}");
}

#[test]
fn a_deadline_that_passed_while_the_app_was_stopped_unblocks_the_phone_at_startup() {
    let (state, effects) = run(
        State::default(),
        [clock(61), Input::CallsOffKept(record_off(in_an_hour()))],
    );

    assert_eq!(effects, coming_back_on());
    assert!(!state.calls_off());
    let (state, effects) = run(
        state,
        [
            paired(),
            Input::PhoneAbsent {
                address: ADDRESS.to_owned(),
            },
            blocked(false),
        ],
    );
    assert_eq!(connects(&effects), 1, "{effects:?}");
    assert!(!state.calls_off());
}

#[test]
fn a_deadline_still_ahead_at_startup_keeps_calls_off_until_it_passes() {
    let (state, effects) = run(
        State::default(),
        [clock(30), Input::CallsOffKept(record_off(in_an_hour()))],
    );
    assert_eq!(
        effects,
        [
            block_phone(true),
            Effect::ReadClockAt(Timestamp(NOW + 60 * MINUTE))
        ]
    );
    let (state, effects) = run(state, [paired(), blocked(true)]);
    assert_eq!(connects(&effects), 0, "{effects:?}");
    assert_eq!(status(&state), "Calls off until 21:30");

    let (_, effects) = run(state, [clock(60)]);
    assert_eq!(effects, coming_back_on());
}

#[test]
fn a_record_read_before_the_clock_waits_for_the_clock_to_say_whether_it_passed() {
    let (state, effects) = run(
        State::default(),
        [Input::CallsOffKept(record_off(in_an_hour()))],
    );
    assert_eq!(effects[0], block_phone(true), "{effects:?}");

    let (_, effects) = run(state, [blocked(true), clock(75)]);
    assert_eq!(effects, coming_back_on());
}

#[test]
fn turning_calls_off_until_a_time_that_has_passed_is_refused_and_does_nothing() {
    let passed = Deadline {
        at: Timestamp(NOW - MINUTE),
        shown: time(20, 29),
    };
    let state = connected();

    assert_eq!(
        state.refusal(&Input::TurnCallsOff {
            until: Some(passed)
        }),
        Some(Failure::CallsOffUntilPassed)
    );
    let (state, effects) = run(
        state,
        [Input::TurnCallsOff {
            until: Some(passed),
        }],
    );
    assert!(effects.is_empty(), "{effects:?}");
    assert!(!state.calls_off());
}

#[test]
fn if_bluez_wont_unblock_at_the_deadline_the_next_clock_tries_again() {
    let (state, _) = run(off_for_an_hour(), [clock(60)]);
    let (state, effects) = run(
        state,
        [Input::PhoneBlockFailed {
            address: ADDRESS.to_owned(),
            blocked: false,
        }],
    );
    assert!(
        effects.contains(&Effect::KeepCallsOff(Some(record_off(in_an_hour())))),
        "{effects:?}"
    );
    assert!(state.calls_off());

    let (_, effects) = run(state, [clock(61)]);
    assert_eq!(effects, coming_back_on());
}

#[test]
fn turning_calls_on_by_hand_before_the_deadline_leaves_nothing_for_it_to_do() {
    let (state, _) = run(off_for_an_hour(), [Input::TurnCallsOn, blocked(false)]);
    assert!(!state.calls_off());

    let (state, effects) = run(state, [clock(60)]);
    assert!(effects.is_empty(), "{effects:?}");
    assert!(!state.calls_off());
}

#[test]
fn a_time_of_day_reads_as_hh_mm_on_the_24_hour_clock() {
    assert_eq!(TimeOfDay::parse("07:00"), Some(time(7, 0)));
    assert_eq!(TimeOfDay::parse("23:59"), Some(time(23, 59)));
    for unreadable in ["7:00", "24:00", "12:60", "ab:cd", "+1:00", "0700", ""] {
        assert_eq!(TimeOfDay::parse(unreadable), None, "{unreadable:?}");
    }
    assert_eq!(time(7, 5).to_string(), "07:05");
}
