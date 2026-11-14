//! The Call state machine's contextual keybinding (`Activate`) and keyboard
//! focus, at its public interface: it focuses the Incoming Call Banner while
//! a Call rings and the Call Window while one is live, and releasing focus
//! never ends or declines a Call.

use desktop_calls_core::{
    CallId, CallState, Caller, Effect, Input, Phone, PhoneCommand, State, Timestamp, step,
};

const ADDRESS: &str = "AA:BB:CC:DD:EE:FF";
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

fn connected() -> State {
    run(
        State::default(),
        [
            Input::PhonePaired(Phone {
                name: "My Pixel".to_owned(),
                address: ADDRESS.to_owned(),
            }),
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

fn live() -> State {
    run(
        ringing(),
        [Input::CallChanged {
            call: CALL,
            state: CallState::Active,
            at: Timestamp(2_000),
        }],
    )
    .0
}

fn banner_focused(state: &State) -> bool {
    state.surfaces().banner.is_some_and(|banner| banner.focused)
}

fn call_window_focused(state: &State) -> bool {
    state
        .surfaces()
        .call_window
        .is_some_and(|window| window.focused)
}

#[test]
fn the_banner_is_not_focused_until_activated() {
    let state = ringing();

    assert!(!banner_focused(&state));
}

#[test]
fn activating_while_ringing_focuses_the_banner_and_leaves_the_call_ringing() {
    let (state, effects) = run(ringing(), [Input::Activate]);

    assert!(banner_focused(&state));
    assert!(state.ringing());
    assert!(effects.is_empty(), "{effects:?}");
}

#[test]
fn releasing_focus_leaves_the_call_ringing() {
    let (state, effects) = run(ringing(), [Input::Activate, Input::ReleaseFocus]);

    assert!(!banner_focused(&state));
    assert!(state.ringing());
    assert!(state.surfaces().banner.is_some());
    assert!(effects.is_empty(), "{effects:?}");
}

#[test]
fn answering_from_the_focused_banner_answers_on_the_desktop() {
    let (state, effects) = run(ringing(), [Input::Activate, Input::Answer(CALL)]);

    assert!(effects.contains(&Effect::Phone(PhoneCommand::Answer(CALL))));
    assert!(effects.contains(&Effect::Phone(PhoneCommand::MoveAudioToDesktop)));
    assert_eq!(state.surfaces().banner, None);
    // Focus went with the Banner; the Call Window doesn't take it.
    assert!(!call_window_focused(&state));
}

#[test]
fn focus_goes_when_the_call_stops_ringing_on_the_phone() {
    let (state, _) = run(
        ringing(),
        [
            Input::Activate,
            Input::CallEnded {
                call: CALL,
                at: Timestamp(90_000),
            },
        ],
    );

    assert_eq!(state.surfaces().banner, None);

    // A later Call's Banner starts unfocused.
    let (state, _) = run(
        state,
        [Input::CallAdded {
            call: CallId(2),
            state: CallState::Ringing,
            caller: Caller::Withheld,
            at: Timestamp(3_000),
        }],
    );
    assert!(!banner_focused(&state));
}

#[test]
fn activating_during_a_live_call_focuses_the_call_window() {
    let (state, effects) = run(live(), [Input::Activate]);

    assert!(call_window_focused(&state));
    assert!(effects.is_empty(), "{effects:?}");
}

#[test]
fn escape_on_the_call_window_leaves_the_call_live() {
    let (state, effects) = run(live(), [Input::Activate, Input::CollapseCallWindow]);

    assert!(!call_window_focused(&state));
    assert_eq!(state.calls()[0].state(), CallState::Active);
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::Phone(_))),
        "{effects:?}"
    );
}

#[test]
fn activating_with_nothing_ringing_or_live_opens_the_dialer() {
    let (state, effects) = run(connected(), [Input::Activate]);

    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(state.surfaces().banner, None);
    assert_eq!(state.surfaces().call_window, None);
    assert!(state.surfaces().dialer.is_some());
}
