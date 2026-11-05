//! The Call state machine's connection states, at its public interface.

use desktop_calls_core::{ConnectionState, Effect, Input, Phone, PhoneCommand, State, step};

fn pixel() -> Phone {
    Phone {
        name: "My Pixel".to_owned(),
        address: "AA:BB:CC:DD:EE:FF".to_owned(),
    }
}

/// Feeds `inputs` to a fresh state machine and returns where it ends up, with
/// every effect it asked for along the way.
fn run(inputs: impl IntoIterator<Item = Input>) -> (State, Vec<Effect>) {
    let mut state = State::default();
    let mut effects = Vec::new();
    for input in inputs {
        let (next, mut more) = step(state, input);
        state = next;
        effects.append(&mut more);
    }
    (state, effects)
}

#[test]
fn with_no_paired_phone_nothing_is_set_up() {
    let (state, effects) = run([]);

    assert_eq!(state.connection_state(), &ConnectionState::NotSetUp);
    assert_eq!(state.phone(), None);
    assert!(effects.is_empty());
}

#[test]
fn a_paired_phone_is_attached_and_connecting() {
    let (state, effects) = run([Input::PhonePaired(pixel())]);

    assert_eq!(state.connection_state(), &ConnectionState::Connecting);
    assert_eq!(state.phone(), Some(&pixel()));
    assert_eq!(
        effects,
        [Effect::AttachPhone {
            address: "AA:BB:CC:DD:EE:FF".to_owned()
        }]
    );
}

#[test]
fn hands_free_up_is_connected_once_the_phone_keeps_call_audio() {
    let (state, effects) = run([
        Input::PhonePaired(pixel()),
        Input::PhoneConnected {
            address: "AA:BB:CC:DD:EE:FF".to_owned(),
        },
    ]);

    // Until then a Call arriving could open its audio on the Desktop.
    assert_eq!(state.connection_state(), &ConnectionState::Connecting);
    assert_eq!(
        effects.last(),
        Some(&Effect::Phone(PhoneCommand::KeepAudioOnPhone))
    );

    let (state, _) = step(
        state,
        Input::PhoneCommandDone(PhoneCommand::KeepAudioOnPhone),
    );

    assert_eq!(state.connection_state(), &ConnectionState::Connected);
}

#[test]
fn hands_free_up_is_connected_even_if_the_phone_refuses_to_keep_call_audio() {
    let (state, _) = run([
        Input::PhonePaired(pixel()),
        Input::PhoneConnected {
            address: "AA:BB:CC:DD:EE:FF".to_owned(),
        },
        Input::PhoneCommandFailed(PhoneCommand::KeepAudioOnPhone),
    ]);

    assert_eq!(state.connection_state(), &ConnectionState::Connected);
}

#[test]
fn hands_free_dropping_before_the_phone_keeps_call_audio_is_not_connected() {
    let (state, _) = run([
        Input::PhonePaired(pixel()),
        Input::PhoneConnected {
            address: "AA:BB:CC:DD:EE:FF".to_owned(),
        },
        Input::PhoneDisconnected {
            address: "AA:BB:CC:DD:EE:FF".to_owned(),
        },
        Input::PhoneCommandFailed(PhoneCommand::KeepAudioOnPhone),
    ]);

    assert_eq!(state.connection_state(), &ConnectionState::Connecting);
}

#[test]
fn hands_free_down_means_disconnected_but_the_phone_is_remembered() {
    let (state, _) = run([
        Input::PhonePaired(pixel()),
        Input::PhoneConnected {
            address: "AA:BB:CC:DD:EE:FF".to_owned(),
        },
        Input::PhoneCommandDone(PhoneCommand::KeepAudioOnPhone),
        Input::PhoneDisconnected {
            address: "AA:BB:CC:DD:EE:FF".to_owned(),
        },
    ]);

    assert_eq!(state.connection_state(), &ConnectionState::Disconnected);
    assert_eq!(state.phone(), Some(&pixel()));
}

#[test]
fn a_second_paired_phone_is_ignored() {
    let other = Phone {
        name: "Other".to_owned(),
        address: "00:11:22:33:44:55".to_owned(),
    };

    let (state, effects) = run([Input::PhonePaired(pixel()), Input::PhonePaired(other)]);

    assert_eq!(state.phone(), Some(&pixel()));
    assert_eq!(effects.len(), 1, "only the first Phone is attached");
}

#[test]
fn another_phones_hands_free_is_ignored() {
    let (state, _) = run([
        Input::PhonePaired(pixel()),
        Input::PhoneConnected {
            address: "00:11:22:33:44:55".to_owned(),
        },
    ]);

    assert_eq!(state.connection_state(), &ConnectionState::Connecting);
}

#[test]
fn hands_free_before_pairing_is_known_is_ignored() {
    let (state, _) = run([Input::PhoneConnected {
        address: "AA:BB:CC:DD:EE:FF".to_owned(),
    }]);

    assert_eq!(state.connection_state(), &ConnectionState::NotSetUp);
}

#[test]
fn unpairing_the_phone_means_nothing_is_set_up() {
    let (state, effects) = run([
        Input::PhonePaired(pixel()),
        Input::PhoneConnected {
            address: "AA:BB:CC:DD:EE:FF".to_owned(),
        },
        Input::PhoneUnpaired {
            address: "AA:BB:CC:DD:EE:FF".to_owned(),
        },
    ]);

    assert_eq!(state.connection_state(), &ConnectionState::NotSetUp);
    assert_eq!(state.phone(), None);
    assert_eq!(effects.last(), Some(&Effect::DetachPhone));
}

#[test]
fn addresses_match_whatever_their_case() {
    let (state, _) = run([
        Input::PhonePaired(pixel()),
        Input::PhoneConnected {
            address: "aa:bb:cc:dd:ee:ff".to_owned(),
        },
        Input::PhoneCommandDone(PhoneCommand::KeepAudioOnPhone),
    ]);

    assert_eq!(state.connection_state(), &ConnectionState::Connected);
}

#[test]
fn connection_states_have_their_wire_names() {
    let names: Vec<&str> = [
        ConnectionState::NotSetUp,
        ConnectionState::Connecting,
        ConnectionState::Connected,
        ConnectionState::Disconnected,
        ConnectionState::PairingLost,
    ]
    .iter()
    .map(ConnectionState::as_str)
    .collect();

    assert_eq!(
        names,
        [
            "not-set-up",
            "connecting",
            "connected",
            "disconnected",
            "pairing-lost"
        ]
    );
    assert_eq!(
        "pairing-lost".parse::<ConnectionState>(),
        Ok(ConnectionState::PairingLost)
    );
    assert_eq!(
        "disconnected".parse::<ConnectionState>(),
        Ok(ConnectionState::Disconnected)
    );
    assert!("nonsense".parse::<ConnectionState>().is_err());
}
