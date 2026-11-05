//! Which Phone Desktop Calls follows when two are paired: the one the user
//! chose last in the Setup Window, whatever order BlueZ lists them in, and
//! the Setup Window rather than another Phone when that one is gone
//! (ticket #121).

use desktop_calls_core::{Effect, Input, Phone, SetupStep, State, step};

const PIXEL: &str = "AA:BB:CC:DD:EE:FF";
const OLD: &str = "48:A4:93:00:00:7E";

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

fn phone(name: &str, address: &str) -> Phone {
    Phone {
        name: name.to_owned(),
        address: address.to_owned(),
    }
}

fn pixel() -> Phone {
    phone("My Pixel", PIXEL)
}

fn old() -> Phone {
    phone("Old Pixel", OLD)
}

fn chosen(address: &str) -> Input {
    Input::ChosenPhoneKept {
        address: address.to_owned(),
    }
}

#[test]
fn the_chosen_phone_is_followed_whichever_phone_bluez_lists_first() {
    for listed in [
        [Input::PhonePaired(old()), Input::PhonePaired(pixel())],
        [Input::PhonePaired(pixel()), Input::PhonePaired(old())],
    ] {
        let (state, effects) = run(
            State::default(),
            [chosen(PIXEL)]
                .into_iter()
                .chain(listed)
                .chain([Input::PairedPhonesListed]),
        );

        assert_eq!(state.phone(), Some(&pixel()));
        assert_eq!(
            effects
                .iter()
                .filter(|effect| matches!(effect, Effect::AttachPhone { .. }))
                .collect::<Vec<_>>(),
            [&Effect::AttachPhone {
                address: PIXEL.to_owned()
            }],
            "only the chosen Phone is ever attached"
        );
        assert_eq!(state.surfaces().setup_window, None);
    }
}

#[test]
fn a_chosen_phone_is_matched_by_address_whatever_the_case() {
    let (state, _) = run(
        State::default(),
        [
            chosen(&PIXEL.to_lowercase()),
            Input::PhonePaired(old()),
            Input::PhonePaired(pixel()),
            Input::PairedPhonesListed,
        ],
    );

    assert_eq!(state.phone(), Some(&pixel()));
}

#[test]
fn with_nothing_chosen_the_first_paired_phone_is_followed_as_before() {
    let (state, _) = run(
        State::default(),
        [
            Input::PhonePaired(old()),
            Input::PhonePaired(pixel()),
            Input::PairedPhonesListed,
        ],
    );

    assert_eq!(state.phone(), Some(&old()));
    assert_eq!(state.surfaces().setup_window, None);
}

#[test]
fn when_the_chosen_phone_is_not_paired_the_setup_window_opens_and_no_other_phone_is_followed() {
    let (state, effects) = run(
        State::default(),
        [
            chosen(PIXEL),
            Input::PhonePaired(old()),
            Input::PairedPhonesListed,
        ],
    );

    assert_eq!(state.phone(), None);
    assert!(effects.contains(&Effect::StartSetup));
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::AttachPhone { .. }))
    );
    assert_eq!(
        state.surfaces().setup_window.map(|window| window.step),
        Some(SetupStep::Choose { phones: vec![] })
    );
}

#[test]
fn a_chosen_phone_paired_after_the_listing_is_followed() {
    let (state, _) = run(
        State::default(),
        [
            chosen(PIXEL),
            Input::PhonePaired(old()),
            Input::PairedPhonesListed,
            Input::CloseSetupWindow,
            Input::PhonePaired(pixel()),
        ],
    );

    assert_eq!(state.phone(), Some(&pixel()));
}

#[test]
fn pairing_a_phone_in_the_setup_window_keeps_it_as_the_chosen_phone() {
    let (state, _) = run(
        State::default(),
        [
            Input::PhonePaired(old()),
            Input::PairedPhonesListed,
            Input::OpenSetupWindow,
            Input::PhoneNearby(pixel()),
            Input::ChoosePhone {
                address: PIXEL.to_owned(),
            },
            Input::PairingCode {
                phone: pixel(),
                code: 1,
            },
        ],
    );
    let (state, effects) = run(state, [Input::ConfirmPairing, Input::PhonePaired(pixel())]);

    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(effect, Effect::KeepChosenPhone { .. }))
            .collect::<Vec<_>>(),
        [&Effect::KeepChosenPhone {
            address: PIXEL.to_owned()
        }]
    );
    assert_eq!(state.phone(), Some(&pixel()));
}

#[test]
fn pairing_the_followed_phone_again_keeps_nothing_new() {
    let (state, _) = run(
        State::default(),
        [
            chosen(PIXEL),
            Input::PhonePaired(pixel()),
            Input::PairedPhonesListed,
            Input::OpenSetupWindow,
            Input::PairingCode {
                phone: pixel(),
                code: 1,
            },
            Input::ConfirmPairing,
        ],
    );
    let (_, effects) = run(state, [Input::PhonePaired(pixel())]);

    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::KeepChosenPhone { .. }))
    );
}

#[test]
fn unpairing_the_followed_phone_keeps_the_choice_so_no_other_phone_is_picked_next_time() {
    let (state, _) = run(
        State::default(),
        [
            chosen(PIXEL),
            Input::PhonePaired(pixel()),
            Input::PairedPhonesListed,
            Input::PhoneUnpaired {
                address: PIXEL.to_owned(),
            },
        ],
    );
    assert_eq!(state.phone(), None);

    // Another Phone paired while the choice is gone is not followed.
    let (state, _) = run(state, [Input::PhonePaired(old())]);
    assert_eq!(state.phone(), None);

    // The chosen Phone paired again is.
    let (state, _) = run(state, [Input::PhonePaired(pixel())]);
    assert_eq!(state.phone(), Some(&pixel()));
}
