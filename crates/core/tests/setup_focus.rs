//! The Setup Window's footer names the button that has the keyboard, as the
//! window reports it when Tab or Shift+Tab moves it, on every step and on a
//! Problem card (ticket #296), at the Call state machine's and Messages'
//! public interfaces.

use desktop_calls_core::messages::{self, FooterKeys, ProblemKind};
use desktop_calls_core::{
    BluetoothPower, DesktopCheck, Input, Phone, SetupFocus, SetupStep, SetupWindowView, State,
    Timestamp, step,
};

const PHONE: &str = "AA:BB:CC:DD:EE:FF";

fn run(state: State, inputs: impl IntoIterator<Item = Input>) -> State {
    inputs
        .into_iter()
        .fold(state, |state, input| step(state, input).0)
}

fn pixel() -> Phone {
    Phone {
        name: "My Pixel".to_owned(),
        address: PHONE.to_owned(),
    }
}

fn view(step: SetupStep, focus: Option<SetupFocus>) -> SetupWindowView {
    SetupWindowView {
        step,
        desktop_name: Some("desktop".to_owned()),
        notice: None,
        opened: 1,
        calls_off: None,
        problem_has_focus: None,
        focus,
    }
}

fn keys_of(view: &SetupWindowView) -> FooterKeys {
    messages::setup_window(view, Timestamp(0), None).keys
}

/// The footer's first entry: what Enter presses.
fn enter(view: &SetupWindowView) -> Option<&'static str> {
    keys_of(view)
        .first()
        .filter(|(keycaps, _)| keycaps.contains(&"Enter"))
        .map(|(_, does)| *does)
}

/// The keys shown at the foot of the open Setup Window.
fn footer(state: &State) -> FooterKeys {
    let Some(window) = state.surfaces().setup_window else {
        panic!("the Setup Window is open");
    };
    keys_of(&window)
}

fn focus(focus: SetupFocus) -> Input {
    Input::SetupWindowFocus(Some(focus))
}

fn checked(check: DesktopCheck) -> Input {
    Input::DesktopChecked(check)
}

/// A fresh install with every check passed: the window lists the Pixel.
fn choosing() -> State {
    run(
        State::default(),
        [
            checked(DesktopCheck::Bluetooth(BluetoothPower::On)),
            checked(DesktopCheck::ContactsSupport(true)),
            checked(DesktopCheck::AudioSetUp(true)),
            checked(DesktopCheck::BluezHandsFree(false)),
            Input::PairedPhonesListed,
            Input::SetupReady {
                desktop_name: "desktop".to_owned(),
            },
            Input::PhoneNearby(pixel()),
        ],
    )
}

fn confirming(state: State) -> State {
    run(
        state,
        [
            Input::ChoosePhone {
                address: PHONE.to_owned(),
            },
            Input::PairingCode {
                phone: pixel(),
                code: 48_213,
            },
        ],
    )
}

#[test]
fn a_focused_second_button_is_what_enter_presses_with_tab_still_shown() {
    let confirm = SetupStep::Confirm {
        phone: pixel(),
        code: 42,
    };
    assert_eq!(
        keys_of(&view(confirm.clone(), Some(SetupFocus::Button(1)))),
        [
            (&["Enter"][..], "Cancel"),
            (&["Tab"][..], "Next button"),
            (&["Esc"][..], "Close")
        ]
    );
    assert_eq!(
        enter(&view(confirm, Some(SetupFocus::Button(0)))),
        Some("Confirm"),
        "Shift+Tab back to the default names it again"
    );
    let timed_out = SetupStep::ContactsPromptTimedOut { phone: pixel() };
    assert_eq!(
        enter(&view(timed_out, Some(SetupFocus::Button(1)))),
        Some("Skip")
    );
}

#[test]
fn a_focused_problem_button_is_what_enter_presses_even_where_it_takes_no_focus_itself() {
    let choose = SetupStep::Choose {
        phones: vec![pixel()],
    };
    assert_eq!(
        keys_of(&view(
            choose.clone(),
            Some(SetupFocus::Problem(ProblemKind::AudioNotSetUp))
        )),
        [
            (&["Enter"][..], "Set up audio"),
            (&["Tab"][..], "Next button"),
            (&["Esc"][..], "Cancel")
        ]
    );
    assert_eq!(
        enter(&view(
            choose,
            Some(SetupFocus::Problem(ProblemKind::ContactsSupportMissing))
        )),
        Some("Copy command")
    );
}

#[test]
fn cancel_focused_while_choosing_is_what_enter_presses_and_a_phone_focused_pairs() {
    let choose = SetupStep::Choose {
        phones: vec![pixel()],
    };
    assert_eq!(
        keys_of(&view(choose.clone(), Some(SetupFocus::Button(0)))),
        [(&["Enter"][..], "Cancel"), (&["Esc"][..], "Cancel")]
    );
    assert_eq!(
        keys_of(&view(choose, Some(SetupFocus::Phone))),
        [(&["Enter"][..], "Pair"), (&["Esc"][..], "Cancel")]
    );
}

#[test]
fn a_focused_button_the_step_lacks_names_the_default() {
    let pairing = SetupStep::Pairing { phone: pixel() };
    assert_eq!(
        keys_of(&view(pairing, Some(SetupFocus::Button(1)))),
        [(&["Enter"][..], "Cancel"), (&["Esc"][..], "Cancel")]
    );
}

#[test]
fn the_window_reporting_shift_tab_to_a_problem_names_its_button_until_it_is_put_right() {
    let state = run(choosing(), [checked(DesktopCheck::AudioSetUp(false))]);
    assert_eq!(
        footer(&state).first(),
        Some(&(&["Enter"][..], "Pair")),
        "audio not set up takes no focus by itself"
    );
    let state = run(
        state,
        [focus(SetupFocus::Problem(ProblemKind::AudioNotSetUp))],
    );
    assert_eq!(
        footer(&state).first(),
        Some(&(&["Enter"][..], "Set up audio"))
    );
    let state = run(state, [checked(DesktopCheck::AudioSetUp(true))]);
    assert_eq!(
        state
            .surfaces()
            .setup_window
            .and_then(|window| window.focus),
        None,
        "its button is gone"
    );
    assert_eq!(footer(&state).first(), Some(&(&["Enter"][..], "Pair")));
}

#[test]
fn the_window_reporting_tab_away_from_a_blocking_problem_names_where_it_went() {
    let state = run(
        choosing(),
        [checked(DesktopCheck::Bluetooth(BluetoothPower::Off))],
    );
    assert_eq!(footer(&state).first(), Some(&(&["Enter"][..], "Turn on")));
    let state = run(state, [focus(SetupFocus::Button(0))]);
    assert_eq!(footer(&state).first(), Some(&(&["Enter"][..], "Cancel")));
    let state = run(state, [Input::SetupWindowFocus(None)]);
    assert_eq!(
        footer(&state).first(),
        Some(&(&["Enter"][..], "Turn on")),
        "with nothing reported, the button that takes the keyboard by itself"
    );
}

#[test]
fn a_reported_button_holds_only_on_the_step_it_was_reported_on() {
    let state = run(confirming(choosing()), [focus(SetupFocus::Button(1))]);
    assert_eq!(footer(&state).first(), Some(&(&["Enter"][..], "Cancel")));
    let state = run(state, [Input::RejectPairing]);
    assert_eq!(
        state
            .surfaces()
            .setup_window
            .and_then(|window| window.focus),
        None
    );
    assert_eq!(footer(&state).first(), Some(&(&["Enter"][..], "Pair")));
    // Back at the code, the default has the keyboard again.
    let state = confirming(state);
    assert_eq!(footer(&state).first(), Some(&(&["Enter"][..], "Confirm")));
}

#[test]
fn a_reported_focus_is_forgotten_when_the_window_closes() {
    let state = run(choosing(), [focus(SetupFocus::Button(0))]);
    let state = run(state, [Input::CloseSetupWindow, Input::OpenSetupWindow]);
    assert_eq!(
        state
            .surfaces()
            .setup_window
            .and_then(|window| window.focus),
        None
    );
}

#[test]
fn a_focus_reported_with_the_window_closed_changes_nothing() {
    let state = run(State::default(), [focus(SetupFocus::Button(1))]);
    assert_eq!(state.surfaces().setup_window, None);
}
