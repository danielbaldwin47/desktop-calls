//! The Call state machine's incoming-Call policy, at its public interface:
//! ringing, answer, decline, hang up, Audio Location, codec and surfaces.

use desktop_calls_core::{
    AudioLocation, CallId, CallState, CallWindowPhase, Caller, Codec, Effect, Input, Mute, Phone,
    PhoneCommand, State, Timer, Timestamp, Transport, messages::Failure, step,
};

const ADDRESS: &str = "AA:BB:CC:DD:EE:FF";
const CALL: CallId = CallId(1);

fn caller() -> Caller {
    Caller::Number("+447700900123".to_owned())
}

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
            caller: caller(),
            at: Timestamp(1_000),
        }],
    )
    .0
}

fn answered_on_desktop() -> State {
    run(
        ringing(),
        [
            Input::Answer(CALL),
            Input::CallChanged {
                call: CALL,
                state: CallState::Active,
                at: Timestamp(2_000),
            },
            Input::CodecChanged(Codec::from_id(2)),
            Input::TransportChanged(Transport::Pending),
            Input::TransportChanged(Transport::Active),
        ],
    )
    .0
}

#[test]
fn connecting_keeps_audio_on_the_phone() {
    let (_, effects) = run(
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
    );

    assert!(effects.contains(&Effect::Phone(PhoneCommand::KeepAudioOnPhone)));
}

#[test]
fn an_incoming_call_rings_and_shows_the_banner() {
    let (state, effects) = run(
        connected(),
        [Input::CallAdded {
            call: CALL,
            state: CallState::Ringing,
            caller: caller(),
            at: Timestamp(1_000),
        }],
    );

    assert!(state.ringing());
    assert_eq!(effects, [Effect::StartRinging]);
    let banner = state.surfaces().banner.expect("the Banner is shown");
    assert_eq!(banner.call, CALL);
    assert_eq!(banner.caller, caller());
    assert_eq!(state.surfaces().call_window, None);
    assert_eq!(state.audio_location(), Some(AudioLocation::Phone));
}

#[test]
fn answering_on_the_desktop_answers_then_brings_the_audio_here_in_one_step() {
    let (state, effects) = run(ringing(), [Input::Answer(CALL)]);

    assert_eq!(
        effects,
        [
            Effect::StopRinging,
            Effect::Phone(PhoneCommand::Answer(CALL)),
            Effect::Phone(PhoneCommand::MoveAudioToDesktop),
            Effect::StartTimer {
                timer: Timer::HandoffTimedOut(1),
                after_ms: 3_000,
            },
        ]
    );
    assert!(!state.ringing());
    assert_eq!(state.surfaces().banner, None);
    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert_eq!(window.phase, CallWindowPhase::Connecting);
}

#[test]
fn once_active_with_the_transport_up_the_audio_is_on_the_desktop_with_a_timer() {
    let state = answered_on_desktop();

    assert_eq!(state.audio_location(), Some(AudioLocation::Desktop));
    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert_eq!(
        window.phase,
        CallWindowPhase::Live {
            active_since: Timestamp(2_000),
            wideband: true,
            mute: Mute::Off,
        }
    );
    assert_eq!(
        window.audio.map(|audio| audio.location),
        Some(AudioLocation::Desktop)
    );
}

#[test]
fn an_answered_incoming_call_stays_incoming_though_it_reports_active() {
    let state = answered_on_desktop();

    let call = &state.calls()[0];
    assert_eq!(call.state(), CallState::Active);
    assert_eq!(call.direction().as_str(), "incoming");
}

#[test]
fn declining_hangs_up_the_ringing_call_and_closes_the_banner() {
    let (state, effects) = run(ringing(), [Input::Decline(CALL)]);

    assert_eq!(
        effects,
        [
            Effect::StopRinging,
            Effect::Phone(PhoneCommand::HangUp(CALL))
        ]
    );
    assert_eq!(state.surfaces().banner, None);
    assert_eq!(state.surfaces().call_window, None);
}

#[test]
fn a_declined_call_ending_keeps_audio_on_the_phone_and_shows_no_call_window() {
    let (state, effects) = run(
        ringing(),
        [Input::Decline(CALL), Input::CallEnded { call: CALL }],
    );

    assert!(effects.contains(&Effect::Phone(PhoneCommand::KeepAudioOnPhone)));
    assert_eq!(state.surfaces().call_window, None);
    assert!(state.calls().is_empty());
}

#[test]
fn hanging_up_shows_call_ended_then_closes_the_call_window() {
    let (state, effects) = run(
        answered_on_desktop(),
        [Input::HangUp(CALL), Input::CallEnded { call: CALL }],
    );

    assert_eq!(effects[0], Effect::Phone(PhoneCommand::HangUp(CALL)));
    assert!(effects.contains(&Effect::Phone(PhoneCommand::KeepAudioOnPhone)));
    assert!(effects.contains(&Effect::StartTimer {
        timer: Timer::CloseCallWindow(CALL),
        after_ms: 2_000,
    }));
    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert_eq!(window.phase, CallWindowPhase::Ended);
    assert_eq!(state.audio_location(), None);

    let (state, _) = run(state, [Input::TimerFired(Timer::CloseCallWindow(CALL))]);
    assert_eq!(state.surfaces().call_window, None);
}

#[test]
fn a_new_call_replaces_the_ended_call_window_and_the_old_timer_does_not_close_it() {
    let next = CallId(2);
    let (state, _) = run(
        answered_on_desktop(),
        [
            Input::HangUp(CALL),
            Input::CallEnded { call: CALL },
            Input::CallAdded {
                call: next,
                state: CallState::Ringing,
                caller: caller(),
                at: Timestamp(9_000),
            },
            Input::Answer(next),
            Input::TimerFired(Timer::CloseCallWindow(CALL)),
        ],
    );

    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert_eq!(window.call, Some(next));
}

#[test]
fn the_codec_is_only_what_the_desktop_audio_negotiated() {
    // While ringing, the Phone's refused audio attempts flip the codec and
    // the transport stays idle; idle reads Codec 0.
    let (state, effects) = run(
        ringing(),
        [
            Input::CodecChanged(Codec::from_id(3)),
            Input::CodecChanged(Codec::from_id(1)),
            Input::CodecChanged(Codec::from_id(0)),
        ],
    );
    assert_eq!(state.codec(), None);
    assert!(effects.is_empty());

    let state = answered_on_desktop();
    assert_eq!(state.codec(), Codec::from_id(2));
    assert!(state.codec().is_some_and(Codec::is_wideband));

    // Codec 0 while idle again is not a change.
    let (state, _) = run(
        state,
        [
            Input::TransportChanged(Transport::Idle),
            Input::CodecChanged(Codec::from_id(0)),
        ],
    );
    assert_eq!(state.codec(), Codec::from_id(2));
}

#[test]
fn cvsd_is_not_wideband() {
    assert!(!Codec::from_id(1).is_some_and(Codec::is_wideband));
    assert!(Codec::from_id(3).is_some_and(Codec::is_wideband));
    assert_eq!(Codec::from_id(0), None);
}

#[test]
fn a_failed_answer_reports_an_error_and_the_banner_comes_back() {
    let (state, effects) = run(
        ringing(),
        [
            Input::Answer(CALL),
            Input::PhoneCommandFailed(PhoneCommand::Answer(CALL)),
        ],
    );

    assert!(effects.contains(&Effect::ReportError(Failure::AnswerFailed)));
    assert!(state.surfaces().banner.is_some());
    assert!(state.ringing());
}

#[test]
fn answering_a_call_that_is_gone_reports_it() {
    let (_, effects) = run(connected(), [Input::Answer(CALL)]);

    assert_eq!(effects, [Effect::ReportError(Failure::NoSuchCall)]);
}

#[test]
fn answered_on_the_phone_the_desktop_leaves_the_audio_there() {
    let (state, effects) = run(
        ringing(),
        [Input::CallChanged {
            call: CALL,
            state: CallState::Active,
            at: Timestamp(2_000),
        }],
    );

    assert_eq!(effects, [Effect::StopRinging]);
    assert_eq!(state.surfaces().banner, None);
    assert_eq!(state.audio_location(), Some(AudioLocation::Phone));
    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert_eq!(
        window.phase,
        CallWindowPhase::Live {
            active_since: Timestamp(2_000),
            wideband: false,
            mute: Mute::Unavailable,
        }
    );
    assert_eq!(
        window.audio.map(|audio| audio.location),
        Some(AudioLocation::Phone)
    );
}

#[test]
fn the_caller_hanging_up_stops_the_ringing() {
    let (state, effects) = run(ringing(), [Input::CallEnded { call: CALL }]);

    assert!(effects.contains(&Effect::StopRinging));
    assert!(!state.ringing());
    assert_eq!(state.surfaces().banner, None);
}

/// A Call the Phone reports ringing with no number, as it does both for a
/// withheld number and for a number it sends a moment later.
fn ringing_with_no_number() -> (State, Vec<Effect>) {
    run(
        connected(),
        [Input::CallAdded {
            call: CALL,
            state: CallState::Ringing,
            caller: Caller::Withheld,
            at: Timestamp(1_000),
        }],
    )
}

#[test]
fn a_call_with_no_number_yet_shows_the_banner_awaiting_the_number() {
    let (state, effects) = ringing_with_no_number();

    let banner = state.surfaces().banner.expect("the Banner shows at once");
    assert!(banner.awaiting_number, "{banner:?}");
    assert!(
        effects.contains(&Effect::StartTimer {
            timer: Timer::NumberLate(CALL),
            after_ms: 2_000,
        }),
        "{effects:?}"
    );
}

#[test]
fn a_number_that_comes_late_ends_the_wait() {
    let (state, _) = ringing_with_no_number();

    let (state, _) = run(
        state,
        [Input::CallerChanged {
            call: CALL,
            caller: caller(),
        }],
    );

    let banner = state.surfaces().banner.expect("the Banner shows");
    assert_eq!((banner.caller, banner.awaiting_number), (caller(), false));
}

#[test]
fn with_no_number_by_the_end_of_the_wait_the_number_is_withheld() {
    let (state, _) = ringing_with_no_number();

    let (state, _) = run(state, [Input::TimerFired(Timer::NumberLate(CALL))]);

    let banner = state.surfaces().banner.expect("the Banner still shows");
    assert_eq!(
        (banner.caller, banner.awaiting_number),
        (Caller::Withheld, false)
    );
}

#[test]
fn a_call_with_its_number_starts_no_wait() {
    let (_, effects) = run(
        connected(),
        [Input::CallAdded {
            call: CALL,
            state: CallState::Ringing,
            caller: caller(),
            at: Timestamp(1_000),
        }],
    );

    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::StartTimer { .. })),
        "{effects:?}"
    );
}
