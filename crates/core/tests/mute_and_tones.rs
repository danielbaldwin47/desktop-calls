//! The Call state machine's Mute and keypad-tone policy, at its public
//! interface.

use desktop_calls_core::{
    CallId, CallState, CallWindowPhase, Caller, Effect, Input, Mute, Phone, PhoneCommand, State,
    StateLine, Timestamp, Transport, messages::Failure, step,
};

const ADDRESS: &str = "AA:BB:CC:DD:EE:FF";
const CALL: CallId = CallId(1);

/// Feeds `inputs` to `state` and returns where it ends up, with every effect
/// it asked for along the way.
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

/// A connected Phone whose `MicrophoneVolume` reads `microphone_volume`, with
/// a Call ringing.
fn ringing(microphone_volume: u8) -> State {
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
            Input::MicrophoneVolumeChanged(microphone_volume),
            Input::CallAdded {
                call: CALL,
                state: CallState::Ringing,
                caller: Caller::Number("+447700900123".to_owned()),
                at: Timestamp(1_000),
            },
        ],
    )
    .0
}

/// Answered on the Desktop, with the audio here.
fn live_on_desktop(microphone_volume: u8) -> State {
    run(
        ringing(microphone_volume),
        [
            Input::Answer(CALL),
            Input::CallChanged {
                call: CALL,
                state: CallState::Active,
                at: Timestamp(2_000),
            },
            Input::TransportChanged(Transport::Active),
        ],
    )
    .0
}

/// Answered on the Phone: connected, with the audio there.
fn live_on_phone() -> State {
    run(
        ringing(15),
        [Input::CallChanged {
            call: CALL,
            state: CallState::Active,
            at: Timestamp(2_000),
        }],
    )
    .0
}

fn mute_control(state: &State) -> Option<Mute> {
    match state.surfaces().call_window?.phase {
        CallWindowPhase::Live { mute, .. } => Some(mute),
        _ => None,
    }
}

#[test]
fn muting_sets_the_microphone_volume_to_zero_and_shows_muted_once_the_phone_has() {
    let (state, effects) = run(live_on_desktop(11), [Input::SetMuted(true)]);

    assert_eq!(
        effects,
        [Effect::Phone(PhoneCommand::SetMicrophoneVolume(0))]
    );
    // The other person may still hear until the Phone confirms.
    assert!(!state.muted());
    assert_eq!(mute_control(&state), Some(Mute::Off));

    let (state, _) = run(
        state,
        [Input::PhoneCommandDone(PhoneCommand::SetMicrophoneVolume(
            0,
        ))],
    );

    assert!(state.muted());
    assert_eq!(mute_control(&state), Some(Mute::On));
}

#[test]
fn unmuting_restores_the_volume_from_before_and_shows_unmuted_once_the_phone_has() {
    let (state, effects) = run(
        live_on_desktop(11),
        [
            Input::SetMuted(true),
            Input::PhoneCommandDone(PhoneCommand::SetMicrophoneVolume(0)),
            Input::SetMuted(false),
        ],
    );

    assert_eq!(
        effects,
        [
            Effect::Phone(PhoneCommand::SetMicrophoneVolume(0)),
            Effect::Phone(PhoneCommand::SetMicrophoneVolume(11)),
        ]
    );
    // The other person may not hear yet until the Phone confirms.
    assert!(state.muted());
    assert_eq!(mute_control(&state), Some(Mute::On));

    let (state, _) = run(
        state,
        [Input::PhoneCommandDone(PhoneCommand::SetMicrophoneVolume(
            11,
        ))],
    );

    assert!(!state.muted());
    assert_eq!(mute_control(&state), Some(Mute::Off));
}

#[test]
fn muting_again_while_an_unmute_is_on_its_way_keeps_the_gain_to_restore() {
    let (state, effects) = run(
        live_on_desktop(11),
        [
            Input::SetMuted(true),
            Input::PhoneCommandDone(PhoneCommand::SetMicrophoneVolume(0)),
            Input::SetMuted(false),
            Input::SetMuted(true),
            Input::PhoneCommandDone(PhoneCommand::SetMicrophoneVolume(11)),
            Input::PhoneCommandDone(PhoneCommand::SetMicrophoneVolume(0)),
            Input::SetMuted(false),
        ],
    );

    assert_eq!(
        effects.last(),
        Some(&Effect::Phone(PhoneCommand::SetMicrophoneVolume(11)))
    );
    assert!(state.muted(), "muted until the Phone confirms the unmute");
}

#[test]
fn keypad_tones_go_to_the_phone_as_typed() {
    let (_, effects) = run(
        live_on_phone(),
        [
            Input::SendTones("1".to_owned()),
            Input::SendTones("*".to_owned()),
            Input::SendTones("1234#".to_owned()),
        ],
    );

    assert_eq!(
        effects,
        [
            Effect::Phone(PhoneCommand::SendTones("1".to_owned())),
            Effect::Phone(PhoneCommand::SendTones("*".to_owned())),
            Effect::Phone(PhoneCommand::SendTones("1234#".to_owned())),
        ]
    );
}

#[test]
fn spaces_and_dashes_in_pasted_tones_are_left_out() {
    let (_, effects) = run(
        live_on_desktop(15),
        [Input::SendTones(" 12-34 #".to_owned())],
    );

    assert_eq!(
        effects,
        [Effect::Phone(PhoneCommand::SendTones("1234#".to_owned()))]
    );
}

#[test]
fn tones_with_anything_but_digits_star_and_hash_are_refused() {
    let state = live_on_desktop(15);

    for tones in ["12x", "", " - "] {
        assert_eq!(
            state.refusal(&Input::SendTones(tones.to_owned())),
            Some(Failure::InvalidTones),
            "{tones:?}"
        );
    }
    let (_, effects) = run(state, [Input::SendTones("12x".to_owned())]);
    assert_eq!(effects, [Effect::ReportError(Failure::InvalidTones)]);
}

#[test]
fn tones_need_a_connected_call() {
    let state = ringing(15);

    assert_eq!(
        state.refusal(&Input::SendTones("1".to_owned())),
        Some(Failure::NoLiveCall)
    );
    let (_, effects) = run(state, [Input::SendTones("1".to_owned())]);
    assert_eq!(effects, [Effect::ReportError(Failure::NoLiveCall)]);
}

#[test]
fn tones_the_phone_refuses_are_reported() {
    let (_, effects) = run(
        live_on_phone(),
        [Input::PhoneCommandFailed(PhoneCommand::SendTones(
            "1".to_owned(),
        ))],
    );

    assert_eq!(effects, [Effect::ReportError(Failure::TonesNotSent)]);
}

#[test]
fn unmuting_never_restores_pipewires_minus_one_but_the_phones_default() {
    // PipeWire's -1, "the Phone hasn't said", arrives as the byte 255.
    let (_, effects) = run(
        live_on_desktop(255),
        [Input::SetMuted(true), Input::SetMuted(false)],
    );

    assert_eq!(
        effects,
        [
            Effect::Phone(PhoneCommand::SetMicrophoneVolume(0)),
            Effect::Phone(PhoneCommand::SetMicrophoneVolume(15)),
        ]
    );
}

#[test]
fn unmuting_after_a_microphone_volume_of_zero_turns_the_microphone_back_on() {
    let (_, effects) = run(
        live_on_desktop(0),
        [Input::SetMuted(true), Input::SetMuted(false)],
    );

    assert_eq!(
        effects.last(),
        Some(&Effect::Phone(PhoneCommand::SetMicrophoneVolume(15)))
    );
}

#[test]
fn with_the_audio_on_the_phone_mute_is_unavailable_and_refused() {
    let state = live_on_phone();

    assert_eq!(mute_control(&state), Some(Mute::Unavailable));
    assert_eq!(
        state.refusal(&Input::SetMuted(true)),
        Some(Failure::MuteUnavailable)
    );
    let (state, effects) = run(state, [Input::SetMuted(true)]);
    assert_eq!(effects, [Effect::ReportError(Failure::MuteUnavailable)]);
    assert!(!state.muted());
}

#[test]
fn with_no_call_mute_is_refused() {
    let state = run(
        ringing(15),
        [Input::CallEnded {
            call: CALL,
            at: Timestamp(90_000),
        }],
    )
    .0;

    assert_eq!(
        state.refusal(&Input::SetMuted(true)),
        Some(Failure::NoLiveCall)
    );
}

#[test]
fn unmuting_is_never_refused() {
    assert_eq!(live_on_phone().refusal(&Input::SetMuted(false)), None);
}

#[test]
fn the_audio_leaving_the_desktop_unmutes() {
    let (state, effects) = run(
        live_on_desktop(11),
        [
            Input::SetMuted(true),
            Input::TransportChanged(Transport::Idle),
        ],
    );

    assert_eq!(
        effects.last(),
        Some(&Effect::Phone(PhoneCommand::SetMicrophoneVolume(11)))
    );
    assert!(!state.muted());
    assert_eq!(mute_control(&state), Some(Mute::Unavailable));
}

#[test]
fn the_call_ending_while_muted_turns_the_microphone_back_on() {
    let (state, effects) = run(
        live_on_desktop(11),
        [
            Input::SetMuted(true),
            Input::CallEnded {
                call: CALL,
                at: Timestamp(90_000),
            },
        ],
    );

    assert!(effects.contains(&Effect::Phone(PhoneCommand::SetMicrophoneVolume(11))));
    assert!(!state.muted());
}

#[test]
fn a_mute_the_phone_refuses_is_reported_and_shows_unmuted() {
    let (state, effects) = run(
        live_on_desktop(11),
        [
            Input::SetMuted(true),
            Input::PhoneCommandFailed(PhoneCommand::SetMicrophoneVolume(0)),
        ],
    );

    assert_eq!(
        effects.last(),
        Some(&Effect::ReportError(Failure::MuteFailed))
    );
    assert!(!state.muted());
}

#[test]
fn an_unmute_the_phone_refuses_is_reported_and_still_shows_muted() {
    let (state, effects) = run(
        live_on_desktop(11),
        [
            Input::SetMuted(true),
            Input::PhoneCommandDone(PhoneCommand::SetMicrophoneVolume(0)),
            Input::SetMuted(false),
            Input::PhoneCommandFailed(PhoneCommand::SetMicrophoneVolume(11)),
        ],
    );

    assert_eq!(
        effects.last(),
        Some(&Effect::ReportError(Failure::UnmuteFailed))
    );
    assert!(state.muted());
    let (_, effects) = run(state, [Input::SetMuted(false)]);
    assert_eq!(
        effects,
        [Effect::Phone(PhoneCommand::SetMicrophoneVolume(11))]
    );
}

#[test]
fn the_state_line_says_muted_while_muted_and_where_the_audio_is_otherwise() {
    let line = |state: &State| state.surfaces().call_window?.state_line;
    let state = live_on_desktop(11);
    assert_eq!(
        line(&state),
        Some(StateLine::OnThisComputer { wideband: false })
    );

    let (state, _) = run(
        state,
        [
            Input::SetMuted(true),
            Input::PhoneCommandDone(PhoneCommand::SetMicrophoneVolume(0)),
        ],
    );
    assert!(state.muted());
    assert_eq!(line(&state), Some(StateLine::Muted));

    assert_eq!(line(&live_on_phone()), Some(StateLine::OnYourPhone));
}
