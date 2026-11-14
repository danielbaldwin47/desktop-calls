//! The Quiet Hours Window, at the Call state machine's public interface
//! (ticket #209): opened from the Indicator's one Quiet Hours… entry, it
//! shows the Quiet Hours set, and each change of its switch or its times
//! is saved at once.

#![expect(
    clippy::expect_used,
    reason = "test code: a failed helper step fails the test with its message"
)]

use desktop_calls_core::{
    Deadline, Effect, IndicatorEntry, Input, Phone, QuietHours, QuietHoursClock, QuietHoursDay,
    QuietHoursField, QuietHoursSentence, QuietHoursWindowView, State, TimeOfDay, Timestamp,
    WallClock, messages, step,
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

fn hours(start: TimeOfDay, end: TimeOfDay) -> QuietHours {
    QuietHours::new(start, end).expect("different times")
}

fn at(minutes: u64, shown: TimeOfDay) -> Deadline {
    Deadline {
        at: Timestamp(NOW + minutes * MINUTE),
        shown,
    }
}

/// The wall clock at 20:30, with `quiet` beginning and ending
/// `start` and `end` minutes on.
fn clock(quiet: Option<(QuietHours, u64, u64)>) -> WallClock {
    WallClock {
        now: Timestamp(NOW),
        for_an_hour: at(60, time(21, 30)),
        until_morning: at(630, time(7, 0)),
        quiet_hours: quiet.map(|(hours, start, end)| QuietHoursClock {
            hours,
            next_start: at(start, hours.start()),
            next_end: at(end, hours.end()),
        }),
    }
}

/// The Phone connected, calls on.
fn connected() -> State {
    let phone = Phone {
        address: ADDRESS.to_owned(),
        name: "My Pixel".to_owned(),
    };
    run(
        State::default(),
        [
            Input::PhonePaired(phone),
            Input::PhoneConnected {
                address: ADDRESS.to_owned(),
            },
            Input::ClockTicked(clock(None)),
        ],
    )
    .0
}

fn window(state: &State) -> QuietHoursWindowView {
    state
        .surfaces()
        .quiet_hours_window
        .expect("the Quiet Hours Window is open")
}

/// Opens the window from the Indicator's menu.
fn open(state: State) -> State {
    let entry = state
        .indicator(&state.surfaces())
        .menu
        .into_iter()
        .find(|entry| *entry == IndicatorEntry::QuietHours)
        .expect("the menu has Quiet Hours…");
    run(state, [entry.intent().expect("it opens the window")]).0
}

fn saved(effects: &[Effect]) -> Vec<Option<QuietHours>> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::KeepQuietHours(hours) => Some(*hours),
            _ => None,
        })
        .collect()
}

fn typed(state: State, field: QuietHoursField, digits: &str) -> (State, Vec<Effect>) {
    run(
        state,
        digits.bytes().map(|digit| Input::QuietHoursDigit {
            field,
            digit: digit - b'0',
        }),
    )
}

#[test]
fn the_menu_has_one_quiet_hours_entry_whether_calls_are_on_off_or_pending() {
    let on = connected();
    let (off, _) = run(on.clone(), [Input::TurnCallsOff { until: None }]);
    let no_phone = State::default();
    for state in [on, off, no_phone] {
        let menu = state.indicator(&state.surfaces()).menu;
        let entries = menu
            .iter()
            .filter(|entry| **entry == IndicatorEntry::QuietHours)
            .count();
        assert_eq!(entries, 1, "{menu:?}");
        assert!(
            IndicatorEntry::QuietHours.choices().is_empty(),
            "no submenu"
        );
    }
    let state = connected();
    let view = state.indicator(&state.surfaces());
    assert_eq!(
        messages::indicator_entry(&IndicatorEntry::QuietHours, &view),
        "Quiet Hours…"
    );
}

#[test]
fn opened_with_no_quiet_hours_it_is_switched_off_and_offers_22_00_to_07_00() {
    let state = open(connected());
    let view = window(&state);
    assert!(!view.on);
    assert_eq!((view.start, view.end), (time(22, 0), time(7, 0)));
    assert_eq!(view.sentence, QuietHoursSentence::Off);
    assert!(view.off_spans.is_empty(), "no bar while switched off");
    let text = messages::quiet_hours_window(&view);
    assert_eq!(
        text.sentence,
        "Off. Calls stay on unless you turn them off from the Indicator."
    );
    assert_eq!(
        text.keys,
        [(&["Space"][..], "Switch on"), (&["Esc"][..], "Close")],
        "the times can't be changed while switched off"
    );
}

#[test]
fn switching_on_saves_the_offered_times_at_once_and_switching_off_clears_them_keeping_the_times() {
    let state = open(connected());
    let (state, effects) = run(state, [Input::SwitchQuietHours(true)]);
    assert_eq!(saved(&effects), [Some(hours(time(22, 0), time(7, 0)))]);
    assert_eq!(state.quiet_hours(), Some(hours(time(22, 0), time(7, 0))));
    assert!(window(&state).on);

    let (state, effects) = run(state, [Input::SwitchQuietHours(false)]);
    assert_eq!(saved(&effects), [None]);
    assert_eq!(state.quiet_hours(), None);
    let view = window(&state);
    assert!(!view.on);
    assert_eq!((view.start, view.end), (time(22, 0), time(7, 0)));
}

#[test]
fn switched_off_the_times_cant_be_changed_so_nothing_switches_quiet_hours_on() {
    let state = open(connected());
    let (state, effects) = run(
        state,
        [
            Input::QuietHoursStep {
                field: QuietHoursField::Start,
                later: true,
            },
            Input::QuietHoursDigit {
                field: QuietHoursField::End,
                digit: 6,
            },
            Input::QuietHoursFieldLeft(QuietHoursField::End),
        ],
    );
    assert!(saved(&effects).is_empty(), "{effects:?}");
    assert_eq!(state.quiet_hours(), None);
    let view = window(&state);
    assert_eq!(
        (view.start, view.end, view.typing),
        (time(22, 0), time(7, 0), None)
    );
}

#[test]
fn it_shows_quiet_hours_set_from_the_command_line_and_follows_them_while_open() {
    let (state, _) = run(
        connected(),
        [Input::SetQuietHours(hours(time(23, 0), time(6, 30)))],
    );
    let state = open(state);
    let view = window(&state);
    assert!(view.on);
    assert_eq!((view.start, view.end), (time(23, 0), time(6, 30)));

    let (state, _) = run(
        state,
        [Input::SetQuietHours(hours(time(21, 0), time(8, 0)))],
    );
    let view = window(&state);
    assert_eq!((view.start, view.end), (time(21, 0), time(8, 0)));
}

#[test]
fn up_and_down_step_a_time_by_15_minutes_round_midnight_saving_each_step() {
    let (state, _) = run(
        connected(),
        [Input::SetQuietHours(hours(time(23, 45), time(0, 0)))],
    );
    let state = open(state);
    let (state, effects) = run(
        state,
        [Input::QuietHoursStep {
            field: QuietHoursField::Start,
            later: true,
        }],
    );
    // 23:45 + 15 is 00:00, the end: refused, shown with the notice.
    assert!(saved(&effects).is_empty(), "{effects:?}");
    let view = window(&state);
    assert_eq!(view.start, time(0, 0));
    assert!(view.refused);
    assert_eq!(
        view.off_spans,
        [(23 * 60 + 45, 24 * 60)],
        "the bar shows what is saved"
    );
    assert_eq!(
        messages::quiet_hours_window(&view).notice,
        Some("Start and end must differ")
    );
    let (state, effects) = run(
        state,
        [Input::QuietHoursStep {
            field: QuietHoursField::Start,
            later: true,
        }],
    );
    assert_eq!(saved(&effects), [Some(hours(time(0, 15), time(0, 0)))]);
    assert!(!window(&state).refused);
    let (state, effects) = run(
        state,
        [Input::QuietHoursStep {
            field: QuietHoursField::End,
            later: false,
        }],
    );
    assert_eq!(saved(&effects), [Some(hours(time(0, 15), time(23, 45)))]);
    assert_eq!(window(&state).end, time(23, 45));
}

#[test]
fn four_digits_typed_save_the_time_and_fewer_save_it_when_the_field_is_left() {
    let (state, _) = run(
        connected(),
        [Input::SetQuietHours(hours(time(22, 0), time(7, 0)))],
    );
    let state = open(state);
    let (state, effects) = typed(state, QuietHoursField::Start, "2");
    assert!(saved(&effects).is_empty());
    assert_eq!(messages::quiet_hours_window(&window(&state)).start, "2_:__");
    let (state, effects) = typed(state, QuietHoursField::Start, "330");
    assert_eq!(saved(&effects), [Some(hours(time(23, 30), time(7, 0)))]);
    assert_eq!(messages::quiet_hours_window(&window(&state)).start, "23:30");

    // A first digit over 2 is the hour's second: 7 is 07.
    let (state, effects) = typed(state, QuietHoursField::End, "73");
    assert!(saved(&effects).is_empty());
    assert_eq!(messages::quiet_hours_window(&window(&state)).end, "07:3_");
    let (state, effects) = run(state, [Input::QuietHoursFieldLeft(QuietHoursField::End)]);
    assert_eq!(saved(&effects), [Some(hours(time(23, 30), time(7, 30)))]);

    let (state, _) = typed(state, QuietHoursField::End, "22");
    let (state, effects) = run(state, [Input::QuietHoursFieldLeft(QuietHoursField::End)]);
    assert_eq!(saved(&effects), [Some(hours(time(23, 30), time(22, 0)))]);
    assert_eq!(window(&state).end, time(22, 0));

    // One digit alone is an hour.
    let (state, _) = typed(state, QuietHoursField::End, "2");
    let (_, effects) = run(state, [Input::QuietHoursFieldLeft(QuietHoursField::End)]);
    assert_eq!(saved(&effects), [Some(hours(time(23, 30), time(2, 0)))]);
}

#[test]
fn a_time_that_cant_be_read_is_never_saved_and_the_field_shows_the_last_time_again() {
    let (state, _) = run(
        connected(),
        [Input::SetQuietHours(hours(time(22, 0), time(7, 0)))],
    );
    let state = open(state);
    let (state, effects) = typed(state, QuietHoursField::Start, "2960");
    assert!(saved(&effects).is_empty());
    assert_eq!(messages::quiet_hours_window(&window(&state)).start, "29:60");
    let (state, effects) = run(state, [Input::QuietHoursFieldLeft(QuietHoursField::Start)]);
    assert!(saved(&effects).is_empty());
    assert_eq!(messages::quiet_hours_window(&window(&state)).start, "22:00");

    // Erased to nothing, then left.
    let (state, _) = typed(state, QuietHoursField::End, "1");
    let (state, effects) = run(
        state,
        [
            Input::QuietHoursErase(QuietHoursField::End),
            Input::QuietHoursFieldLeft(QuietHoursField::End),
        ],
    );
    assert!(saved(&effects).is_empty());
    assert_eq!(messages::quiet_hours_window(&window(&state)).end, "07:00");
    assert_eq!(state.quiet_hours(), Some(hours(time(22, 0), time(7, 0))));
}

#[test]
fn the_sentence_says_when_calls_next_go_off_and_the_bar_shows_the_span_in_two_pieces_across_midnight()
 {
    let quiet = hours(time(22, 0), time(7, 0));
    let (state, _) = run(
        connected(),
        [
            Input::SetQuietHours(quiet),
            Input::ClockTicked(clock(Some((quiet, 90, 630)))),
        ],
    );
    let view = window(&open(state));
    assert_eq!(
        view.sentence,
        QuietHoursSentence::Saved {
            start: time(22, 0),
            end: time(7, 0),
            day: Some(QuietHoursDay::Tonight),
        }
    );
    assert_eq!(
        messages::quiet_hours_window(&view).sentence,
        "Saved. Calls go off tonight at 22:00 and come back at 07:00, every day."
    );
    assert_eq!(view.off_spans, [(0, 7 * 60), (22 * 60, 24 * 60)]);
    assert_eq!(
        messages::quiet_hours_window(&view).keys,
        [
            (&["Tab"][..], "Next"),
            (&["↑", "↓"][..], "15 min"),
            (&["Esc"][..], "Close")
        ]
    );

    // At 20:30, Quiet Hours from 20:00 to 21:00 next begin tomorrow; they
    // are under way, so calls are off now.
    let early = hours(time(20, 0), time(21, 0));
    let (state, _) = run(
        connected(),
        [
            Input::SetQuietHours(early),
            Input::ClockTicked(clock(Some((early, 1410, 30)))),
        ],
    );
    let view = window(&open(state));
    assert_eq!(
        messages::quiet_hours_window(&view).sentence,
        "Saved. Calls are off until 21:00, and every day from 20:00 to 21:00."
    );
    assert_eq!(view.off_spans, [(20 * 60, 21 * 60)]);

    // At 20:30, Quiet Hours from 08:00 to 09:00 begin tomorrow.
    let morning = hours(time(8, 0), time(9, 0));
    let (state, _) = run(
        connected(),
        [
            Input::SetQuietHours(morning),
            Input::ClockTicked(clock(Some((morning, 690, 750)))),
        ],
    );
    assert_eq!(
        messages::quiet_hours_window(&window(&open(state))).sentence,
        "Saved. Calls go off tomorrow at 08:00 and come back at 09:00, every day."
    );
}

#[test]
fn closing_the_window_keeps_what_was_saved_and_opening_it_again_shows_it() {
    let state = open(connected());
    let (state, _) = run(state, [Input::SwitchQuietHours(true)]);
    let opened = window(&state).opened;
    let (state, _) = run(state, [Input::CloseQuietHoursWindow]);
    assert!(state.surfaces().quiet_hours_window.is_none());
    assert_eq!(state.quiet_hours(), Some(hours(time(22, 0), time(7, 0))));
    let state = open(state);
    assert!(window(&state).on);
    assert_ne!(window(&state).opened, opened, "a fresh opening");
}
