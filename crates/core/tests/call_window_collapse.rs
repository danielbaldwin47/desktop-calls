//! The Call state machine's rules for collapsing the Call Window to the Call
//! Pill and expanding it again, at its public interface (spec #197, ticket
//! #200).

use desktop_calls_core::{
    AudioLocation, CallId, CallState, Caller, Effect, Input, Phone, State, Timer, Timestamp,
    Transport, step,
};

const ADDRESS: &str = "AA:BB:CC:DD:EE:FF";
const CALL: CallId = CallId(1);
const SECOND: CallId = CallId(2);

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

fn connected() -> State {
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
            Input::PhoneConnected {
                address: ADDRESS.to_owned(),
            },
        ],
    )
    .0
}

fn ringing() -> State {
    run(
        connected(),
        [Input::CallAdded {
            call: CALL,
            state: CallState::Ringing,
            caller: Caller::Number("+447700900123".to_owned()),
            at: Timestamp(1_000),
        }],
    )
    .0
}

/// Answered on the Desktop and connected, with the audio here, and the
/// effects of the Call connecting.
fn connecting() -> (State, Vec<Effect>) {
    run(
        ringing(),
        [
            Input::Answer(CALL),
            Input::TransportChanged(Transport::Active),
            Input::CallChanged {
                call: CALL,
                state: CallState::Active,
                at: Timestamp(2_000),
            },
        ],
    )
}

fn live() -> State {
    connecting().0
}

/// The collapse timers `effects` started, with how long each runs.
fn collapse_timers(effects: &[Effect]) -> Vec<(Timer, u32)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::StartTimer {
                timer: timer @ Timer::CollapseCallWindow(_),
                after_ms,
            } => Some((*timer, *after_ms)),
            _ => None,
        })
        .collect()
}

/// The one collapse timer `effects` started.
fn collapse_timer(effects: &[Effect]) -> Timer {
    match collapse_timers(effects)[..] {
        [(timer, 5_000)] => timer,
        ref timers => panic!("one 5 s collapse timer expected: {timers:?}"),
    }
}

fn collapsed(state: &State) -> bool {
    let Some(window) = state.surfaces().call_window else {
        panic!("the Call Window shows");
    };
    window.collapsed
}

fn focused(state: &State) -> bool {
    let Some(window) = state.surfaces().call_window else {
        panic!("the Call Window shows");
    };
    window.focused
}

/// A live Call whose Call Window has collapsed by itself.
fn collapsed_by_itself() -> State {
    let (state, effects) = connecting();
    run(state, [Input::TimerFired(collapse_timer(&effects))]).0
}

#[test]
fn the_call_window_collapses_five_seconds_after_the_call_connects() {
    let (state, effects) = connecting();

    let timer = collapse_timer(&effects);
    assert!(!collapsed(&state), "expanded until the timer runs out");

    let (state, effects) = run(state, [Input::TimerFired(timer)]);

    assert!(collapsed(&state));
    assert!(!focused(&state));
    assert_eq!(collapse_timers(&effects), []);
}

#[test]
fn a_call_answered_on_the_phone_collapses_the_same_way() {
    let (state, effects) = run(
        ringing(),
        [Input::CallChanged {
            call: CALL,
            state: CallState::Active,
            at: Timestamp(2_000),
        }],
    );
    assert_eq!(state.audio_location(), Some(AudioLocation::Phone));

    let (state, _) = run(state, [Input::TimerFired(collapse_timer(&effects))]);

    assert!(collapsed(&state));
}

#[test]
fn a_call_being_placed_or_ringing_out_never_collapses() {
    let (state, effects) = run(
        connected(),
        [
            Input::Dial("+447700900123".to_owned()),
            Input::CallAdded {
                call: CALL,
                state: CallState::Dialing,
                caller: Caller::Number("+447700900123".to_owned()),
                at: Timestamp(1_000),
            },
            Input::CallChanged {
                call: CALL,
                state: CallState::Alerting,
                at: Timestamp(1_500),
            },
        ],
    );

    assert_eq!(collapse_timers(&effects), []);
    assert!(!collapsed(&state));

    // Escape only gives the keyboard back, so Hang up stays in reach.
    let (state, _) = run(state, [Input::Activate, Input::CollapseCallWindow]);

    assert!(!collapsed(&state));
    assert!(!focused(&state));
}

#[test]
fn the_keyboard_keeps_it_expanded_and_it_collapses_five_seconds_after_it_leaves() {
    let (state, effects) = connecting();
    let first = collapse_timer(&effects);

    let (state, _) = run(state, [Input::CallWindowKeyboard(true)]);
    let (state, effects) = run(state, [Input::TimerFired(first)]);
    assert!(!collapsed(&state), "the keyboard keeps it expanded");
    assert_eq!(collapse_timers(&effects), []);

    let (state, effects) = run(state, [Input::CallWindowKeyboard(false)]);
    let second = collapse_timer(&effects);
    assert_ne!(second, first);

    let (state, _) = run(state, [Input::TimerFired(second)]);
    assert!(collapsed(&state));
}

#[test]
fn the_pointer_keeps_it_expanded_and_it_collapses_five_seconds_after_it_leaves() {
    let (state, effects) = connecting();
    let first = collapse_timer(&effects);

    let (state, _) = run(
        state,
        [Input::PointerOverCallWindow(true), Input::TimerFired(first)],
    );
    assert!(!collapsed(&state), "the pointer keeps it expanded");

    let (state, effects) = run(state, [Input::PointerOverCallWindow(false)]);
    let (state, _) = run(state, [Input::TimerFired(collapse_timer(&effects))]);
    assert!(collapsed(&state));
}

#[test]
fn the_open_keypad_keeps_it_expanded_and_it_collapses_five_seconds_after_it_closes() {
    let (state, effects) = connecting();
    let first = collapse_timer(&effects);

    let (state, _) = run(state, [Input::KeypadShown(true), Input::TimerFired(first)]);
    assert!(!collapsed(&state), "the keypad keeps it expanded");

    let (state, effects) = run(state, [Input::KeypadShown(false)]);
    let (state, _) = run(state, [Input::TimerFired(collapse_timer(&effects))]);
    assert!(collapsed(&state));
}

#[test]
fn a_notice_keeps_it_expanded_and_it_collapses_five_seconds_after_it_goes() {
    let (state, effects) = connecting();
    let first = collapse_timer(&effects);

    // The Phone ignores a Handoff to the Phone: the window says so.
    let (state, handoff) = run(state, [Input::MoveAudio(AudioLocation::Phone)]);
    let Some(timed_out) = handoff.iter().find_map(|effect| match effect {
        Effect::StartTimer {
            timer: timer @ Timer::HandoffTimedOut(_),
            ..
        } => Some(*timer),
        _ => None,
    }) else {
        panic!("a Handoff limit expected: {handoff:?}");
    };
    let (state, _) = run(
        state,
        [Input::TimerFired(timed_out), Input::TimerFired(first)],
    );
    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert!(window.notice.is_some(), "{window:?}");
    assert!(!window.collapsed, "the notice keeps it expanded");

    // The audio moving answers the notice.
    let (state, effects) = run(state, [Input::TransportChanged(Transport::Idle)]);
    assert_eq!(state.surfaces().call_window.and_then(|w| w.notice), None);
    let (state, _) = run(state, [Input::TimerFired(collapse_timer(&effects))]);
    assert!(collapsed(&state));
}

#[test]
fn lost_contact_keeps_it_expanded() {
    let (state, effects) = connecting();
    let first = collapse_timer(&effects);

    let (state, effects) = run(
        state,
        [
            Input::PhoneAbsent {
                address: ADDRESS.to_owned(),
            },
            Input::PhoneDisconnected {
                address: ADDRESS.to_owned(),
            },
            Input::CallEnded { call: CALL },
            Input::TimerFired(first),
        ],
    );

    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert!(window.connection_lost);
    assert!(!window.collapsed);
    assert_eq!(collapse_timers(&effects), []);
}

#[test]
fn a_call_on_hold_keeps_it_expanded_and_it_collapses_five_seconds_after_the_swap_back() {
    let (state, effects) = connecting();
    let first = collapse_timer(&effects);

    let (state, effects) = run(
        state,
        [
            Input::CallAdded {
                call: SECOND,
                state: CallState::Waiting,
                caller: Caller::Number("+447700900456".to_owned()),
                at: Timestamp(3_000),
            },
            Input::HoldAndAnswer,
            Input::CallChanged {
                call: CALL,
                state: CallState::Held,
                at: Timestamp(4_000),
            },
            Input::CallChanged {
                call: SECOND,
                state: CallState::Active,
                at: Timestamp(4_000),
            },
            Input::TimerFired(first),
        ],
    );
    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert!(window.held.is_some());
    assert!(!window.collapsed, "the Call on hold keeps it expanded");
    assert_eq!(collapse_timers(&effects), []);

    let (state, effects) = run(state, [Input::CallEnded { call: CALL }]);
    let (state, _) = run(state, [Input::TimerFired(collapse_timer(&effects))]);
    assert!(collapsed(&state));
}

#[test]
fn escape_collapses_it_and_gives_the_keyboard_back_even_with_the_keypad_open() {
    let (state, _) = run(
        live(),
        [
            Input::Activate,
            Input::KeypadShown(true),
            Input::CollapseCallWindow,
        ],
    );

    assert!(collapsed(&state));
    assert!(!focused(&state));
    assert_eq!(state.calls()[0].state(), CallState::Active);
}

#[test]
fn clicking_the_call_pill_expands_it_with_the_keyboard() {
    let (state, effects) = run(collapsed_by_itself(), [Input::ExpandCallWindow]);

    assert!(!collapsed(&state));
    assert!(focused(&state));
    assert_eq!(collapse_timers(&effects), [], "the keyboard keeps it open");
}

#[test]
fn the_keybinding_expands_it_with_the_keyboard() {
    let (state, _) = run(collapsed_by_itself(), [Input::Activate]);

    assert!(!collapsed(&state));
    assert!(focused(&state));
}

#[test]
fn clicking_the_indicator_expands_it_with_the_keyboard_and_leaves_the_dialer_shut() {
    let (state, _) = run(collapsed_by_itself(), [Input::IndicatorActivated]);

    assert!(!collapsed(&state));
    assert!(focused(&state));
    assert_eq!(state.surfaces().dialer, None);
}

#[test]
fn clicking_the_indicator_with_no_call_window_collapsed_opens_the_dialer() {
    let (state, _) = run(connected(), [Input::IndicatorActivated]);
    assert!(state.surfaces().dialer.is_some());

    let (state, _) = run(live(), [Input::IndicatorActivated]);
    assert!(state.surfaces().dialer.is_some());
}

#[test]
fn an_expanded_window_collapses_again_five_seconds_after_escape_leaves_it_alone() {
    let (state, effects) = run(
        collapsed_by_itself(),
        [Input::ExpandCallWindow, Input::CollapseCallWindow],
    );
    assert!(collapsed(&state));
    assert_eq!(collapse_timers(&effects), []);

    // Expanded, then left: the keyboard goes as the pointer never came.
    let (state, effects) = run(
        state,
        [Input::ExpandCallWindow, Input::CallWindowKeyboard(true)],
    );
    assert_eq!(collapse_timers(&effects), []);
    let (state, effects) = run(
        state,
        [Input::ReleaseFocus, Input::CallWindowKeyboard(false)],
    );
    let (state, _) = run(state, [Input::TimerFired(collapse_timer(&effects))]);
    assert!(collapsed(&state));
}

#[test]
fn the_next_call_starts_expanded() {
    let (state, effects) = run(collapsed_by_itself(), [Input::CallEnded { call: CALL }]);
    assert!(collapsed(&state), "the Call Pill says the Call ended");
    let Some(close) = effects.iter().find_map(|effect| match effect {
        Effect::StartTimer {
            timer: timer @ Timer::CloseCallWindow(_),
            ..
        } => Some(*timer),
        _ => None,
    }) else {
        panic!("a timer to close the window expected: {effects:?}");
    };
    let (state, _) = run(state, [Input::TimerFired(close)]);
    assert_eq!(state.surfaces().call_window, None);

    let (state, _) = run(
        state,
        [Input::CallAdded {
            call: SECOND,
            state: CallState::Ringing,
            caller: Caller::Number("+447700900456".to_owned()),
            at: Timestamp(9_000),
        }],
    );
    let (state, _) = run(
        state,
        [Input::CallChanged {
            call: SECOND,
            state: CallState::Active,
            at: Timestamp(10_000),
        }],
    );
    assert!(!collapsed(&state));
}

#[test]
fn a_late_report_of_the_pointer_or_keyboard_on_the_collapsed_window_is_ignored() {
    let (state, effects) = run(
        collapsed_by_itself(),
        [
            Input::PointerOverCallWindow(true),
            Input::CallWindowKeyboard(true),
            Input::ExpandCallWindow,
            Input::ReleaseFocus,
        ],
    );

    // Neither stale report keeps the window expanded.
    let (state, _) = run(state, [Input::TimerFired(collapse_timer(&effects))]);
    assert!(collapsed(&state));
}
