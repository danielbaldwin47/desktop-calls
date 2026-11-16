//! "Phone setup…" with a Phone set up opens the Setup Window on the
//! finish page for it, with its keybinding card and Pair another phone to
//! choose a different one; a Phone that forgot the pairing still opens it
//! on choosing (ticket #389).

use desktop_calls_core::{
    ConnectFailure, Effect, Input, KeybindingStage, Phone, SetupStep, State, Timestamp, messages,
    step,
};

const PHONE: &str = "AA:BB:CC:DD:EE:FF";

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

fn pixel() -> Phone {
    Phone {
        name: "My Pixel".to_owned(),
        address: PHONE.to_owned(),
    }
}

fn setup_step(state: &State) -> Option<SetupStep> {
    state.surfaces().setup_window.map(|window| window.step)
}

/// With a Phone set up, the window opens on the finish page for it, with
/// its keybinding card, and looks for no Phone until Pair another phone
/// (#389).
#[test]
fn with_a_phone_set_up_the_window_opens_on_its_finish_page() {
    let (state, _) = run(
        State::default(),
        [Input::PhonePaired(pixel()), Input::PairedPhonesListed],
    );

    let (state, effects) = run(state, [Input::OpenSetupWindow]);
    assert_eq!(effects, [Effect::CheckKeybinding]);
    assert!(matches!(
        setup_step(&state),
        Some(SetupStep::Done {
            pair_another: true,
            keybinding: KeybindingStage::Checking,
            ..
        })
    ));
    let text = messages::setup_window(
        &state.surfaces().setup_window.expect("open"),
        Timestamp(0),
        None,
    );
    assert_eq!(text.buttons, ["Done", "Pair another phone"]);
    // Opened again, the next Call isn't the first.
    assert!(
        text.instructions
            .iter()
            .any(|line| line.text.starts_with("Place a call: choose New call")),
        "{text:?}"
    );

    // Closed from here, nothing was looked for, so nothing stops.
    let (_, effects) = run(state.clone(), [Input::CloseSetupWindow]);
    assert_eq!(effects, []);

    let (state, effects) = run(state, [Input::PairAnotherPhone]);
    assert_eq!(effects, [Effect::StartSetup]);
    assert_eq!(
        setup_step(&state),
        Some(SetupStep::Choose { phones: vec![] })
    );
    let (_, effects) = run(state, [Input::CloseSetupWindow]);
    assert_eq!(effects, [Effect::StopSetup]);
}

/// The finish page says what the Phone is doing with Contacts right now,
/// not that it isn't sharing them, while a sync is under way or the Phone
/// asks (#389).
#[test]
fn the_finish_page_opened_mid_sync_says_contacts_are_being_read() {
    let (state, _) = run(
        State::default(),
        [Input::PhonePaired(pixel()), Input::PairedPhonesListed],
    );
    let (state, _) = run(
        state,
        [
            Input::PhoneConnected {
                address: PHONE.to_owned(),
            },
            Input::OpenSetupWindow,
        ],
    );
    let summary = |state: &State| {
        messages::setup_window(
            &state.surfaces().setup_window.expect("open"),
            Timestamp(0),
            None,
        )
        .summary
    };
    assert_eq!(
        summary(&state).as_deref(),
        Some("Getting contacts and recent calls from your phone\u{2026}")
    );
    let (state, _) = run(
        state,
        [Input::ContactsPermissionNeeded {
            deadline: Timestamp(30_000),
        }],
    );
    assert_eq!(
        summary(&state).as_deref(),
        Some(
            "Your phone is asking whether to share contacts. Tap Allow on it so calls show names."
        )
    );
}

/// A Phone that forgot the pairing still opens the window on choosing a
/// Phone, to pair it again.
#[test]
fn a_phone_that_forgot_the_pairing_opens_the_window_on_choosing() {
    let (state, _) = run(
        State::default(),
        [
            Input::PhonePaired(pixel()),
            Input::PairedPhonesListed,
            Input::PhoneAbsent {
                address: PHONE.to_owned(),
            },
            Input::ConnectPhoneFailed {
                attempt: 1,
                failure: ConnectFailure::PairingLost,
            },
        ],
    );
    let (state, effects) = run(state, [Input::OpenSetupWindow]);
    assert_eq!(effects, [Effect::StartSetup]);
    assert!(matches!(setup_step(&state), Some(SetupStep::Choose { .. })));
}
