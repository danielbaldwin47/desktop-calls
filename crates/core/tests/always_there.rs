//! The Call state machine keeps the user's Call through restarts, at its
//! public interface: a Call whose hands-free link drops and returns (as when
//! `WirePlumber` restarts) gets its audio back where it was, and a run of the
//! app that starts mid-Call takes the Call's direction, timer and Mute from
//! the run before it.

use desktop_calls_core::{
    AudioLocation, CallId, CallState, CallWindowPhase, Caller, Direction, EarlierCall, EarlierRun,
    Effect, Input, Mute, Phone, PhoneCommand, State, Timer, Timestamp, Transport, step,
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

fn paired() -> Input {
    Input::PhonePaired(Phone {
        name: "My Pixel".to_owned(),
        address: ADDRESS.to_owned(),
    })
}

fn connected_input() -> Input {
    Input::PhoneConnected {
        address: ADDRESS.to_owned(),
    }
}

fn disconnected_input() -> Input {
    Input::PhoneDisconnected {
        address: ADDRESS.to_owned(),
    }
}

fn number() -> Caller {
    Caller::Number("07700900123".to_owned())
}

fn connected() -> State {
    run(
        State::default(),
        [
            paired(),
            connected_input(),
            Input::PhoneCommandDone(PhoneCommand::KeepAudioOnPhone),
        ],
    )
    .0
}

/// Answered on the Desktop at 2 s, with the audio here.
fn live_on_desktop() -> State {
    run(
        connected(),
        [
            Input::CallAdded {
                call: CALL,
                state: CallState::Ringing,
                caller: number(),
                at: Timestamp(1_000),
            },
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

/// Answered on the Phone at 2 s, with the audio there.
fn live_on_phone() -> State {
    run(
        connected(),
        [
            Input::CallAdded {
                call: CALL,
                state: CallState::Ringing,
                caller: number(),
                at: Timestamp(1_000),
            },
            Input::CallChanged {
                call: CALL,
                state: CallState::Active,
                at: Timestamp(2_000),
            },
        ],
    )
    .0
}

/// Hands-free drops and comes back, and the Call with it, as when
/// `WirePlumber` restarts mid-Call.
fn hands_free_returns() -> [Input; 4] {
    [
        disconnected_input(),
        connected_input(),
        Input::PhoneCommandDone(PhoneCommand::KeepAudioOnPhone),
        Input::CallChanged {
            call: CALL,
            state: CallState::Active,
            at: Timestamp(9_000),
        },
    ]
}

fn phone_commands(effects: &[Effect]) -> Vec<PhoneCommand> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Phone(command) => Some(command.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_call_whose_audio_was_here_gets_it_back_when_hands_free_returns() {
    let (state, effects) = run(live_on_desktop(), hands_free_returns());

    assert!(
        phone_commands(&effects).contains(&PhoneCommand::MoveAudioToDesktop),
        "{effects:?}"
    );
    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert_eq!(window.call, Some(CALL));
    assert_eq!(state.calls()[0].active_since(), Some(Timestamp(2_000)));
    let (state, _) = run(state, [Input::TransportChanged(Transport::Active)]);
    assert_eq!(state.audio_location(), Some(AudioLocation::Desktop));
}

#[test]
fn the_call_window_stays_while_hands_free_is_away() {
    let (state, _) = run(live_on_desktop(), [disconnected_input()]);

    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert_eq!(window.call, Some(CALL));
}

#[test]
fn a_call_whose_audio_was_on_the_phone_keeps_it_there_when_hands_free_returns() {
    let (state, effects) = run(live_on_phone(), hands_free_returns());

    assert!(
        !phone_commands(&effects).contains(&PhoneCommand::MoveAudioToDesktop),
        "{effects:?}"
    );
    assert_eq!(state.audio_location(), Some(AudioLocation::Phone));
}

#[test]
fn audio_that_came_back_by_itself_is_not_asked_for_again() {
    let (_, effects) = run(
        live_on_desktop(),
        [
            disconnected_input(),
            connected_input(),
            Input::TransportChanged(Transport::Active),
            Input::CallChanged {
                call: CALL,
                state: CallState::Active,
                at: Timestamp(9_000),
            },
        ],
    );

    assert!(
        !phone_commands(&effects).contains(&PhoneCommand::MoveAudioToDesktop),
        "{effects:?}"
    );
}

/// Muted on the Desktop, from a gain of 12, and the Phone confirmed.
fn muted_on_desktop() -> State {
    run(
        live_on_desktop(),
        [
            Input::MicrophoneVolumeChanged(12),
            Input::SetMuted(true),
            Input::PhoneCommandDone(PhoneCommand::SetMicrophoneVolume(0)),
        ],
    )
    .0
}

#[test]
fn a_microphone_still_silenced_when_hands_free_returns_shows_muted_and_unmutes_to_its_gain() {
    let (state, effects) = run(muted_on_desktop(), [disconnected_input()]);
    // The Desktop tries to give the microphone back as the audio leaves,
    // but hands-free has gone.
    assert!(phone_commands(&effects).contains(&PhoneCommand::SetMicrophoneVolume(12)));
    let (state, _) = run(
        state,
        [Input::PhoneCommandFailed(
            PhoneCommand::SetMicrophoneVolume(12),
        )],
    );
    let (state, _) = run(state, hands_free_returns());
    let (state, _) = run(
        state,
        [
            Input::MicrophoneVolumeChanged(0),
            Input::TransportChanged(Transport::Active),
        ],
    );

    assert!(state.muted());
    let (_, effects) = run(state, [Input::SetMuted(false)]);
    assert_eq!(
        phone_commands(&effects),
        [PhoneCommand::SetMicrophoneVolume(12)]
    );
}

#[test]
fn a_microphone_given_back_by_the_phone_shows_unmuted() {
    let (state, _) = run(
        muted_on_desktop(),
        [
            disconnected_input(),
            Input::PhoneCommandFailed(PhoneCommand::SetMicrophoneVolume(12)),
        ],
    );
    let (state, _) = run(state, hands_free_returns());
    let (state, _) = run(
        state,
        [
            Input::MicrophoneVolumeChanged(12),
            Input::TransportChanged(Transport::Active),
        ],
    );

    assert!(!state.muted());
}

fn earlier_run(restore_microphone_volume: Option<u8>) -> Input {
    Input::EarlierRun(EarlierRun {
        calls: vec![EarlierCall {
            caller: number(),
            direction: Direction::Incoming,
            active_since: Timestamp(2_000),
        }],
        restore_microphone_volume,
    })
}

/// The app starts afresh with `earlier`, and finds the Phone connected with
/// a live Call from [`number`], its audio on the Desktop, and the
/// microphone gain at `microphone_volume`.
fn started_mid_call(earlier: Input, microphone_volume: u8) -> (State, Vec<Effect>) {
    run(
        State::default(),
        [
            earlier,
            paired(),
            connected_input(),
            Input::MicrophoneVolumeChanged(microphone_volume),
            Input::TransportChanged(Transport::Active),
            Input::CallAdded {
                call: CALL,
                state: CallState::Active,
                caller: number(),
                at: Timestamp(60_000),
            },
        ],
    )
}

#[test]
fn a_call_from_an_earlier_run_keeps_its_direction_and_timer() {
    let (state, _) = started_mid_call(earlier_run(None), 15);

    let call = &state.calls()[0];
    assert_eq!(call.direction(), Direction::Incoming);
    assert_eq!(call.active_since(), Some(Timestamp(2_000)));
    assert_eq!(state.audio_location(), Some(AudioLocation::Desktop));
}

#[test]
fn a_call_the_app_never_saw_times_from_when_it_is_first_seen() {
    let (state, _) = started_mid_call(
        Input::EarlierRun(EarlierRun {
            calls: Vec::new(),
            restore_microphone_volume: None,
        }),
        15,
    );

    let call = &state.calls()[0];
    assert_eq!(call.direction(), Direction::Unknown);
    assert_eq!(call.active_since(), Some(Timestamp(60_000)));
}

#[test]
fn a_call_with_another_number_is_not_the_earlier_runs() {
    let (state, _) = run(
        State::default(),
        [
            earlier_run(None),
            paired(),
            connected_input(),
            Input::CallAdded {
                call: CALL,
                state: CallState::Active,
                caller: Caller::Number("07700900999".to_owned()),
                at: Timestamp(60_000),
            },
        ],
    );

    assert_eq!(state.calls()[0].active_since(), Some(Timestamp(60_000)));
}

#[test]
fn the_earlier_run_is_forgotten_once_its_calls_had_time_to_show() {
    let (state, effects) = run(State::default(), [earlier_run(None)]);
    let timer = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::StartTimer {
                timer: timer @ Timer::ForgetEarlierRun,
                ..
            } => Some(*timer),
            _ => None,
        })
        .expect("a timer to forget the earlier run");

    let (state, _) = run(
        state,
        [
            Input::TimerFired(timer),
            paired(),
            connected_input(),
            Input::CallAdded {
                call: CALL,
                state: CallState::Active,
                caller: number(),
                at: Timestamp(60_000),
            },
        ],
    );

    assert_eq!(state.calls()[0].active_since(), Some(Timestamp(60_000)));
}

fn forgotten(effects: &[Effect]) -> Vec<usize> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::EarlierCallsForgotten(calls) => Some(*calls),
            _ => None,
        })
        .collect()
}

#[test]
fn forgetting_earlier_calls_that_never_showed_again_says_how_many() {
    let (state, _) = run(State::default(), [earlier_run(None)]);

    let (_, effects) = run(state, [Input::TimerFired(Timer::ForgetEarlierRun)]);

    assert_eq!(forgotten(&effects), [1]);
}

#[test]
fn earlier_calls_that_all_showed_again_leave_nothing_to_forget() {
    let (state, _) = started_mid_call(earlier_run(None), 15);

    let (_, effects) = run(state, [Input::TimerFired(Timer::ForgetEarlierRun)]);

    assert_eq!(forgotten(&effects), [0usize; 0]);
}

#[test]
fn an_earlier_run_that_had_muted_the_desktop_shows_muted_and_unmutes_to_its_gain() {
    let (state, _) = started_mid_call(earlier_run(Some(12)), 0);

    assert!(state.muted());
    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert!(matches!(
        window.phase,
        CallWindowPhase::Live { mute: Mute::On, .. }
    ));
    let (_, effects) = run(state, [Input::SetMuted(false)]);
    assert_eq!(
        phone_commands(&effects),
        [PhoneCommand::SetMicrophoneVolume(12)]
    );
}

#[test]
fn a_microphone_unmuted_since_the_earlier_run_shows_unmuted() {
    let (state, _) = started_mid_call(earlier_run(Some(12)), 12);

    assert!(!state.muted());
}

#[test]
fn what_the_next_run_needs_is_the_live_calls_and_the_mute() {
    let (state, _) = run(live_on_desktop(), [Input::SetMuted(true)]);
    let (state, _) = run(
        state,
        [Input::PhoneCommandDone(PhoneCommand::SetMicrophoneVolume(
            0,
        ))],
    );

    assert_eq!(
        state.for_next_run(),
        EarlierRun {
            calls: vec![EarlierCall {
                caller: number(),
                direction: Direction::Incoming,
                active_since: Timestamp(2_000),
            }],
            restore_microphone_volume: Some(15),
        }
    );
}

#[test]
fn with_no_live_call_the_next_run_needs_nothing() {
    assert_eq!(
        connected().for_next_run(),
        EarlierRun {
            calls: Vec::new(),
            restore_microphone_volume: None,
        }
    );
}

#[test]
fn the_next_run_keeps_an_earlier_call_not_seen_again_yet() {
    let (state, _) = run(State::default(), [earlier_run(Some(12))]);

    assert_eq!(
        state.for_next_run(),
        EarlierRun {
            calls: vec![EarlierCall {
                caller: number(),
                direction: Direction::Incoming,
                active_since: Timestamp(2_000),
            }],
            restore_microphone_volume: Some(12),
        }
    );
}
