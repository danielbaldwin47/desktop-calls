//! Quiet Hours, at the Call state machine's public interface (ticket
//! #205): daily times during which calls are off. They turn calls off when
//! they begin, waiting for a live Call, and on when they end, and the
//! precedence rules of spec #198 hold against the user's own choices.
//!
//! The clock here starts at 20:30 local time, with Quiet Hours from 22:00
//! to 07:00: they begin 90 minutes on and end 630 minutes on, and again a
//! day later.

// Clippy exempts `#[test]` functions but not their helpers. Like a test, a
// helper whose step fails must fail the test at once with its message.
#![expect(
    clippy::expect_used,
    reason = "test code: a failed helper step fails the test with its message"
)]

use desktop_calls_core::{
    CallId, CallState, Caller, CallsOffRecord, Deadline, Effect, IndicatorEntry, IndicatorLook,
    IndicatorStatus, Input, Phone, PhoneCommand, QuietHours, QuietHoursClock, State, TimeOfDay,
    Timestamp, WallClock, messages, step,
};

const ADDRESS: &str = "AA:BB:CC:DD:EE:FF";
/// 20:30 local time, as the shell's clock read it.
const NOW: u64 = 1_800_000_000_000;
const MINUTE: u64 = 60_000;
const DAY: u64 = 24 * 60;
/// When the Quiet Hours begin and end, in minutes after 20:30.
const START: u64 = 90;
const END: u64 = 630;
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

fn time(hour: u8, minute: u8) -> TimeOfDay {
    TimeOfDay { hour, minute }
}

fn quiet_hours() -> QuietHours {
    QuietHours::new(time(22, 0), time(7, 0)).expect("different times")
}

/// The instant `minutes` after 20:30, showing `shown`.
fn at(minutes: u64, shown: TimeOfDay) -> Deadline {
    Deadline {
        at: Timestamp(NOW + minutes * MINUTE),
        shown,
    }
}

/// The first of `first`, `first` + a day, ... after `minutes`.
fn next(first: u64, minutes: u64) -> u64 {
    if minutes < first {
        first
    } else {
        first + ((minutes - first) / DAY + 1) * DAY
    }
}

/// When the Quiet Hours next end, read at `minutes` after 20:30.
fn end_after(minutes: u64) -> Deadline {
    at(next(END, minutes), time(7, 0))
}

/// When the Quiet Hours next begin, read at `minutes` after 20:30.
fn start_after(minutes: u64) -> Deadline {
    at(next(START, minutes), time(22, 0))
}

/// The wall clock `minutes` after 20:30, with `hours` as the shell's clock
/// read them.
fn clock_with(minutes: u64, hours: Option<QuietHours>) -> Input {
    let now = NOW + minutes * MINUTE;
    Input::ClockTicked(WallClock {
        now: Timestamp(now),
        for_an_hour: Deadline {
            at: Timestamp(now + 60 * MINUTE),
            shown: time(21, 30),
        },
        until_morning: at(next(END, minutes), time(7, 0)),
        quiet_hours: hours.map(|hours| QuietHoursClock {
            hours,
            next_start: start_after(minutes),
            next_end: end_after(minutes),
        }),
    })
}

/// The wall clock `minutes` after 20:30, with Quiet Hours from 22:00 to
/// 07:00.
fn clock(minutes: u64) -> Input {
    clock_with(minutes, Some(quiet_hours()))
}

fn paired() -> Input {
    Input::PhonePaired(Phone {
        name: "My Pixel".to_owned(),
        address: ADDRESS.to_owned(),
    })
}

/// The Phone connected at `minutes` after 20:30, Quiet Hours set from
/// 22:00 to 07:00.
fn connected_at(minutes: u64) -> State {
    run(
        State::default(),
        [
            Input::QuietHoursKept(quiet_hours()),
            clock(minutes),
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

fn block_phone(blocked: bool) -> Effect {
    Effect::BlockPhone {
        address: ADDRESS.to_owned(),
        blocked,
    }
}

fn blocks(effects: &[Effect]) -> bool {
    effects
        .iter()
        .any(|effect| matches!(effect, Effect::BlockPhone { .. }))
}

fn kept(effects: &[Effect]) -> Vec<Option<CallsOffRecord>> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::KeepCallsOff(record) => Some(record.clone()),
            _ => None,
        })
        .collect()
}

fn off_for_quiet_hours(until: Deadline) -> CallsOffRecord {
    CallsOffRecord::Off {
        address: ADDRESS.to_owned(),
        until: Some(until),
        quiet_hours: true,
    }
}

fn call_added(call_state: CallState) -> Input {
    Input::CallAdded {
        call: CALL,
        state: call_state,
        caller: Caller::Number("+447700900123".to_owned()),
        at: Timestamp(NOW),
    }
}

fn ended(minutes: u64) -> Input {
    Input::CallEnded {
        call: CALL,
        at: Timestamp(NOW + minutes * MINUTE),
    }
}

/// Calls off for the Quiet Hours, from 22:00 until 07:00: the Phone is
/// blocked, and BlueZ dropped it.
fn off_in_quiet_hours() -> State {
    run(
        connected_at(0),
        [
            clock(START),
            blocked(true),
            Input::PhoneDisconnected {
                address: ADDRESS.to_owned(),
            },
            Input::PhoneAbsent {
                address: ADDRESS.to_owned(),
            },
        ],
    )
    .0
}

fn status(state: &State) -> String {
    messages::indicator_status(&state.indicator(&state.surfaces()).status)
}

#[test]
fn quiet_hours_beginning_turn_calls_off_until_they_end() {
    let state = connected_at(0);
    let (state, effects) = run(state, [clock(START - 1)]);
    assert!(!blocks(&effects), "not yet: {effects:?}");

    let (state, effects) = run(state, [clock(START)]);

    assert_eq!(
        kept(&effects),
        [Some(off_for_quiet_hours(end_after(START)))],
        "kept before blocking"
    );
    assert!(effects.contains(&block_phone(true)), "{effects:?}");
    assert!(effects.contains(&Effect::ReadClockAt(end_after(START).at)));
    assert!(state.calls_off());
    assert_eq!(state.calls_off_until(), Some(end_after(START)));
    let view = state.indicator(&state.surfaces());
    assert_eq!(view.look, IndicatorLook::Off);
    assert_eq!(
        view.status,
        IndicatorStatus::CallsOff {
            until: Some(time(7, 0)),
            quiet_hours: true,
        }
    );
    assert_eq!(status(&state), "Calls off until 07:00 (quiet hours)");
    assert!(view.menu.contains(&IndicatorEntry::TurnCallsOn));
}

#[test]
fn quiet_hours_ending_turn_calls_on_and_connect_the_phone() {
    let (state, effects) = run(off_in_quiet_hours(), [clock(END - 1)]);
    assert!(!blocks(&effects), "not yet: {effects:?}");

    let (state, effects) = run(state, [clock(END), blocked(false)]);

    assert!(effects.contains(&block_phone(false)), "{effects:?}");
    assert!(effects.contains(&Effect::CallsOffDeadlinePassed {
        address: ADDRESS.to_owned()
    }));
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::ConnectPhone { .. })),
        "connected at once: {effects:?}"
    );
    assert_eq!(kept(&effects).last(), Some(&None), "the record goes");
    assert!(!state.calls_off());
}

#[test]
fn quiet_hours_under_way_when_the_phone_is_found_turn_calls_off_at_once() {
    let state = connected_at(START + 60);

    assert!(state.calls_off(), "off from the start");
    assert_eq!(state.calls_off_until(), Some(end_after(START + 60)));
}

#[test]
fn turning_calls_on_during_quiet_hours_keeps_them_on_until_they_next_begin() {
    let (state, effects) = run(
        off_in_quiet_hours(),
        [clock(START + 30), Input::TurnCallsOn, blocked(false)],
    );
    assert!(effects.contains(&block_phone(false)), "{effects:?}");
    assert_eq!(
        kept(&effects).last(),
        Some(&Some(CallsOffRecord::KeptOn {
            until: start_after(START + 30)
        })),
        "kept on until they next begin, for the next run"
    );

    // Through the rest of tonight's Quiet Hours and the day after.
    let (state, effects) = run(state, [clock(START + 31), clock(END), clock(END + 60)]);
    assert!(!blocks(&effects), "{effects:?}");
    assert!(!state.calls_off());

    // Tomorrow night's begin again.
    let (state, effects) = run(state, [clock(START + DAY)]);
    assert!(effects.contains(&block_phone(true)), "{effects:?}");
    assert_eq!(state.calls_off_until(), Some(end_after(START + DAY)));
}

#[test]
fn calls_kept_on_through_quiet_hours_stay_on_across_a_restart() {
    let state = run(
        State::default(),
        [
            Input::QuietHoursKept(quiet_hours()),
            clock(START + 60),
            Input::CallsOffKept(CallsOffRecord::KeptOn {
                until: start_after(START),
            }),
            paired(),
        ],
    )
    .0;
    assert!(!state.calls_off());

    let (state, effects) = run(state, [clock(START + 61)]);
    assert!(!blocks(&effects), "{effects:?}");
    assert!(!state.calls_off());
}

#[test]
fn calls_kept_on_past_the_next_start_go_off_after_a_restart() {
    let (state, effects) = run(
        State::default(),
        [
            Input::QuietHoursKept(quiet_hours()),
            clock(START + DAY + 5),
            Input::CallsOffKept(CallsOffRecord::KeptOn {
                until: start_after(START),
            }),
            paired(),
        ],
    );
    assert!(effects.contains(&block_phone(true)), "{effects:?}");
    assert!(state.calls_off());
}

#[test]
fn calls_off_until_turned_on_by_hand_outlast_quiet_hours() {
    let (state, _) = run(
        connected_at(0),
        [Input::TurnCallsOff { until: None }, blocked(true)],
    );

    let (state, effects) = run(state, [clock(START), clock(END), clock(END + 1)]);

    assert!(!blocks(&effects), "{effects:?}");
    assert!(state.calls_off());
    assert_eq!(state.calls_off_until(), None);
    assert_eq!(status(&state), "Calls off");
}

#[test]
fn a_timed_break_ending_inside_quiet_hours_stays_off_until_they_end() {
    let until = at(START + 60, time(23, 0));
    let (state, _) = run(
        connected_at(0),
        [Input::TurnCallsOff { until: Some(until) }, blocked(true)],
    );
    assert_eq!(status(&state), "Calls off until 23:00");

    let (state, effects) = run(state, [clock(START + 60)]);

    assert!(!blocks(&effects), "the Phone stays blocked: {effects:?}");
    assert_eq!(
        kept(&effects),
        [Some(off_for_quiet_hours(end_after(START + 60)))]
    );
    assert!(state.calls_off());
    assert_eq!(status(&state), "Calls off until 07:00 (quiet hours)");

    let (state, effects) = run(state, [clock(END)]);
    assert!(effects.contains(&block_phone(false)), "{effects:?}");
    assert!(!state.calls_off());
}

#[test]
fn a_timed_break_that_ended_inside_quiet_hours_while_stopped_stays_off_at_start() {
    let (state, effects) = run(
        State::default(),
        [
            Input::QuietHoursKept(quiet_hours()),
            clock(START + 120),
            Input::CallsOffKept(CallsOffRecord::Off {
                address: ADDRESS.to_owned(),
                until: Some(at(START + 60, time(23, 0))),
                quiet_hours: false,
            }),
        ],
    );

    assert!(
        effects.contains(&block_phone(true)),
        "blocked again: {effects:?}"
    );
    assert!(!effects.contains(&block_phone(false)), "{effects:?}");
    assert!(state.calls_off());
    assert_eq!(state.calls_off_until(), Some(end_after(START + 120)));
}

#[test]
fn until_tomorrow_morning_ends_when_quiet_hours_end() {
    let state = connected_at(0);
    let view = state.indicator(&state.surfaces());
    let entry = view
        .menu
        .iter()
        .find(|entry| matches!(entry, IndicatorEntry::TurnCallsOff { .. }))
        .expect("Turn calls off");
    assert!(
        entry
            .choices()
            .contains(&IndicatorEntry::CallsOffUntilMorning(end_after(0))),
        "{entry:?}"
    );

    let other = QuietHours::new(time(23, 0), time(6, 15)).expect("different times");
    let shown_end = at(585, time(6, 15));
    let (state, _) = run(
        state,
        [
            Input::SetQuietHours(other),
            Input::ClockTicked(WallClock {
                now: Timestamp(NOW),
                for_an_hour: at(60, time(21, 30)),
                until_morning: at(END, time(7, 0)),
                quiet_hours: Some(QuietHoursClock {
                    hours: other,
                    next_start: at(150, time(23, 0)),
                    next_end: shown_end,
                }),
            }),
        ],
    );
    let view = state.indicator(&state.surfaces());
    let entry = view
        .menu
        .iter()
        .find(|entry| matches!(entry, IndicatorEntry::TurnCallsOff { .. }))
        .expect("Turn calls off");
    assert!(
        entry
            .choices()
            .contains(&IndicatorEntry::CallsOffUntilMorning(shown_end)),
        "{entry:?}"
    );
}

#[test]
fn quiet_hours_beginning_during_a_call_wait_for_it_to_end() {
    let state = run(connected_at(0), [call_added(CallState::Active)]).0;

    let (state, effects) = run(state, [clock(START)]);
    assert!(!blocks(&effects), "the Call carries on: {effects:?}");
    assert!(state.calls_off_pending());
    assert_eq!(
        kept(&effects),
        [Some(CallsOffRecord::Pending {
            address: ADDRESS.to_owned(),
            until: Some(end_after(START)),
            quiet_hours: true,
            exception: false,
        })]
    );

    let (state, effects) = run(state, [ended(START + 5)]);
    assert!(effects.contains(&block_phone(true)), "{effects:?}");
    assert_eq!(status(&state), "Calls off until 07:00 (quiet hours)");
}

#[test]
fn keep_calls_on_while_a_quiet_hours_turn_off_waits_keeps_them_on_until_the_next_quiet_hours() {
    let state = run(
        connected_at(0),
        [call_added(CallState::Active), clock(START)],
    )
    .0;

    let (state, effects) = run(state, [Input::KeepCallsOn, ended(START + 5)]);
    assert!(!blocks(&effects), "{effects:?}");
    assert_eq!(
        kept(&effects),
        [Some(CallsOffRecord::KeptOn {
            until: start_after(START)
        })]
    );

    let (state, effects) = run(state, [clock(START + 10), clock(END + 10)]);
    assert!(!blocks(&effects), "{effects:?}");
    assert!(!state.calls_off());

    let (_, effects) = run(state, [clock(START + DAY)]);
    assert!(effects.contains(&block_phone(true)), "{effects:?}");
}

#[test]
fn a_hand_choice_during_quiet_hours_replaces_them() {
    let (state, effects) = run(off_in_quiet_hours(), [Input::TurnCallsOff { until: None }]);
    assert!(!blocks(&effects), "already blocked: {effects:?}");
    assert_eq!(
        kept(&effects),
        [Some(CallsOffRecord::Off {
            address: ADDRESS.to_owned(),
            until: None,
            quiet_hours: false,
        })]
    );
    assert_eq!(status(&state), "Calls off");

    let (state, effects) = run(state, [clock(END)]);
    assert!(!blocks(&effects), "{effects:?}");
    assert!(state.calls_off());
}

#[test]
fn setting_quiet_hours_keeps_them_for_the_next_run_and_clearing_turns_calls_on() {
    let state = run(
        State::default(),
        [
            clock_with(0, None),
            paired(),
            Input::PhonePresent {
                address: ADDRESS.to_owned(),
            },
        ],
    )
    .0;
    let (state, effects) = run(state, [Input::SetQuietHours(quiet_hours())]);
    assert_eq!(state.quiet_hours(), Some(quiet_hours()));
    assert!(effects.contains(&Effect::KeepQuietHours(Some(quiet_hours()))));

    // Inside them, once the clock has read them, calls go off.
    let (state, effects) = run(state, [clock(START + 1)]);
    assert!(effects.contains(&block_phone(true)), "{effects:?}");
    let (state, _) = run(state, [blocked(true)]);

    let (state, effects) = run(state, [Input::ClearQuietHours]);
    assert_eq!(state.quiet_hours(), None);
    assert!(effects.contains(&Effect::KeepQuietHours(None)));
    assert!(effects.contains(&block_phone(false)), "{effects:?}");
    let (state, _) = run(state, [blocked(false)]);
    assert!(!state.calls_off());
}

#[test]
fn quiet_hours_an_earlier_run_kept_are_not_written_again() {
    let (state, effects) = run(State::default(), [Input::QuietHoursKept(quiet_hours())]);
    assert_eq!(state.quiet_hours(), Some(quiet_hours()));
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::KeepQuietHours(_))),
        "{effects:?}"
    );
}

#[test]
fn quiet_hours_changed_while_under_way_move_when_calls_come_back_on() {
    let later = QuietHours::new(time(22, 0), time(8, 0)).expect("different times");
    let (state, effects) = run(
        off_in_quiet_hours(),
        [Input::SetQuietHours(later), clock_with(START + 10, None)],
    );
    assert!(!blocks(&effects), "{effects:?}");
    assert_eq!(
        state.calls_off_until(),
        Some(end_after(START)),
        "unchanged until the clock reads the new times"
    );

    let new_end = at(END + 60, time(8, 0));
    let (state, effects) = run(
        state,
        [Input::ClockTicked(WallClock {
            now: Timestamp(NOW + (START + 10) * MINUTE),
            for_an_hour: at(START + 70, time(23, 10)),
            until_morning: new_end,
            quiet_hours: Some(QuietHoursClock {
                hours: later,
                next_start: start_after(START + 10),
                next_end: new_end,
            }),
        })],
    );
    assert!(!blocks(&effects), "{effects:?}");
    assert_eq!(state.calls_off_until(), Some(new_end));
    assert_eq!(kept(&effects), [Some(off_for_quiet_hours(new_end))]);
}

#[test]
fn quiet_hours_that_bluez_wont_block_for_leave_calls_on_until_they_next_begin() {
    let (state, effects) = run(
        connected_at(0),
        [
            clock(START),
            Input::PhoneBlockFailed {
                address: ADDRESS.to_owned(),
                blocked: true,
            },
        ],
    );
    let tries = effects
        .iter()
        .filter(|effect| **effect == block_phone(true))
        .count();
    assert_eq!(tries, 1, "no loop of retries: {effects:?}");
    assert!(!state.calls_off());

    let (_, effects) = run(state, [clock(START + 1)]);
    assert!(!blocks(&effects), "{effects:?}");
}

#[test]
fn quiet_hours_are_read_and_written_as_hh_mm_hh_mm() {
    assert_eq!(QuietHours::parse("22:00-07:00"), Some(quiet_hours()));
    assert_eq!(quiet_hours().to_string(), "22:00-07:00");
    assert_eq!(QuietHours::parse("07:00-07:00"), None, "no length");
    assert_eq!(QuietHours::parse("22:00"), None);
    assert_eq!(QuietHours::parse("25:00-07:00"), None);
    assert_eq!(TimeOfDay::from_minutes(1_320), Some(time(22, 0)));
    assert_eq!(TimeOfDay::from_minutes(1_440), None);
}
