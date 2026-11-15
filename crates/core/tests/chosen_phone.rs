//! Which Phone Desktop Calls follows when two are paired: the one the user
//! chose last in the Setup Window, whatever order BlueZ lists them in, and
//! the Setup Window rather than another Phone when that one is gone
//! (ticket #121).

use desktop_calls_core::messages;
use desktop_calls_core::{Effect, Input, Phone, SetupNotice, SetupStep, State, Timestamp, step};

fn setup_step(state: &State) -> Option<SetupStep> {
    state.surfaces().setup_window.map(|window| window.step)
}

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

/// The Pixel's address, as kept, written as `address`.
fn chosen(address: &str) -> Input {
    Input::ChosenPhoneKept {
        address: address.to_owned(),
        name: Some("My Pixel".to_owned()),
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
        setup_step(&state),
        Some(SetupStep::Choose {
            phones: vec![old()]
        }),
        "the Phone that is paired is offered, so the user is not stuck"
    );
    // And it says why it opened, naming the Phone gone (ticket #150).
    let window = state.surfaces().setup_window.expect("the window is open");
    let notice = SetupNotice::ChosenPhoneUnpaired {
        name: Some("My Pixel".to_owned()),
    };
    assert_eq!(window.notice, Some(notice));
    assert_eq!(
        messages::setup_window(&window, Timestamp(0), None)
            .notice
            .as_deref(),
        Some(
            "My Pixel isn't paired with this computer any more; pair it again, or choose another phone."
        )
    );
}

/// A first start, with no Phone chosen yet: the window opens without a
/// notice, as there is nothing gone to explain.
#[test]
fn with_no_phone_chosen_the_setup_window_opens_without_a_notice() {
    let (state, _) = run(State::default(), [Input::PairedPhonesListed]);

    let window = state.surfaces().setup_window.expect("the window is open");
    assert_eq!(window.notice, None);
}

/// A choice kept without a name, whose Phone is gone: the window says so
/// without one, and pairing that Phone again keeps it with its name, so a
/// later start can say which Phone is gone.
#[test]
fn a_choice_kept_without_a_name_is_kept_again_with_it_once_paired_again() {
    let (state, _) = run(
        State::default(),
        [
            Input::ChosenPhoneKept {
                address: PIXEL.to_owned(),
                name: None,
            },
            Input::PairedPhonesListed,
        ],
    );
    let window = state.surfaces().setup_window.expect("the window is open");
    assert_eq!(
        window.notice,
        Some(SetupNotice::ChosenPhoneUnpaired { name: None })
    );
    let (state, _) = run(
        state,
        [
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
    let (_, effects) = run(state, [Input::ConfirmPairing, Input::PhonePaired(pixel())]);

    assert!(effects.contains(&Effect::KeepChosenPhone { phone: pixel() }));
}

/// A choice kept without a name, or under an old one, whose Phone is
/// followed at startup: kept again with the name it has now.
#[test]
fn the_chosen_phone_followed_at_startup_is_kept_again_with_its_name_now() {
    for name in [None, Some("Old name".to_owned())] {
        let (_, effects) = run(
            State::default(),
            [
                Input::ChosenPhoneKept {
                    address: PIXEL.to_owned(),
                    name,
                },
                Input::PhonePaired(pixel()),
            ],
        );

        assert!(effects.contains(&Effect::KeepChosenPhone { phone: pixel() }));
    }
    let (_, effects) = run(
        State::default(),
        [chosen(PIXEL), Input::PhonePaired(pixel())],
    );
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::KeepChosenPhone { .. })),
        "nothing new to keep"
    );
}

/// BlueZ names a Phone with no name by its address, with dashes: that
/// isn't kept as a name, so a later notice doesn't show an address.
#[test]
fn a_phone_named_only_by_its_address_is_kept_without_a_name() {
    let nameless = phone("AA-BB-CC-DD-EE-FF", PIXEL);
    let (state, _) = run(
        State::default(),
        [
            Input::ChosenPhoneKept {
                address: PIXEL.to_owned(),
                name: None,
            },
            Input::PhonePaired(nameless),
            Input::PhoneUnpaired {
                address: PIXEL.to_owned(),
            },
            Input::PairedPhonesListed,
        ],
    );

    let window = state.surfaces().setup_window.expect("the window is open");
    assert_eq!(
        window.notice,
        Some(SetupNotice::ChosenPhoneUnpaired { name: None })
    );
}

#[test]
fn choosing_a_phone_that_is_paired_already_follows_and_keeps_it_without_pairing() {
    let (state, _) = run(
        State::default(),
        [
            chosen(PIXEL),
            Input::PhonePaired(old()),
            Input::PairedPhonesListed,
        ],
    );

    let (state, effects) = run(
        state,
        [Input::ChoosePhone {
            address: OLD.to_owned(),
        }],
    );

    assert_eq!(state.phone(), Some(&old()));
    assert!(effects.contains(&Effect::KeepChosenPhone { phone: old() }));
    assert!(effects.contains(&Effect::AttachPhone {
        address: OLD.to_owned()
    }));
    assert!(effects.contains(&Effect::ConnectPhone {
        address: OLD.to_owned(),
        attempt: 1,
        away: false,
    }));
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::PairPhone { .. })),
        "nothing is paired again"
    );
}

#[test]
fn paired_phones_are_offered_only_while_none_is_followed() {
    let (state, _) = run(
        State::default(),
        [
            Input::PhonePaired(old()),
            Input::PhonePaired(pixel()),
            Input::PairedPhonesListed,
            Input::OpenSetupWindow,
        ],
    );
    assert_eq!(state.phone(), Some(&old()));
    assert_eq!(
        setup_step(&state),
        Some(SetupStep::Choose { phones: vec![] })
    );

    // A Phone unpaired is no longer offered.
    let (state, _) = run(
        State::default(),
        [
            chosen(PIXEL),
            Input::PhonePaired(old()),
            Input::PhonePaired(phone("Third", "11:22:33:44:55:66")),
            Input::PairedPhonesListed,
            Input::PhoneUnpaired {
                address: OLD.to_owned(),
            },
        ],
    );
    assert_eq!(
        setup_step(&state),
        Some(SetupStep::Choose {
            phones: vec![phone("Third", "11:22:33:44:55:66")]
        })
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

/// The chosen Phone paired again outside the window, which stays open:
/// it no longer says the Phone is gone (ticket #150).
#[test]
fn a_chosen_phone_paired_again_with_the_window_open_takes_its_notice_away() {
    let (state, _) = run(
        State::default(),
        [
            chosen(PIXEL),
            Input::PairedPhonesListed,
            Input::PhonePaired(pixel()),
        ],
    );

    assert_eq!(state.phone(), Some(&pixel()));
    let window = state.surfaces().setup_window.expect("the window is open");
    assert_eq!(window.notice, None);
}

/// The window opened by hand before BlueZ listed the Phones keeps what it
/// says: it didn't open because the chosen Phone is gone.
#[test]
fn a_window_opened_by_hand_before_the_listing_gets_no_notice() {
    let (state, _) = run(
        State::default(),
        [
            chosen(PIXEL),
            Input::OpenSetupWindow,
            Input::SetupUnavailable,
            Input::PairedPhonesListed,
        ],
    );

    let window = state.surfaces().setup_window.expect("the window is open");
    assert_eq!(window.notice, Some(SetupNotice::BluetoothUnavailable));
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
        [&Effect::KeepChosenPhone { phone: pixel() }]
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
