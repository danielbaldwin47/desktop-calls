//! The Call state machine's Audio Location policy at its public interface:
//! a Call placed on the Phone keeps its audio there, Handoff to the Desktop
//! with its one retry (a second after `Activate` if the audio link hasn't
//! started) and its 3 s limit, a Handoff that fails, and the fallback
//! instruction for a Handoff to the Phone.

use desktop_calls_core::{
    AudioLocation, CallAudio, CallId, CallState, Caller, Effect, Input, Phone, PhoneCommand, State,
    Timer, Timestamp, Transport, messages::Failure, step,
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
            Input::PhoneCommandDone(PhoneCommand::KeepAudioOnPhone),
        ],
    )
    .0
}

fn number() -> Caller {
    Caller::Number("07700900123".to_owned())
}

/// The Phone reports a Call the Desktop didn't dial: one placed on the
/// Phone.
fn placed_on_phone() -> State {
    run(
        connected(),
        [Input::CallAdded {
            call: CALL,
            state: CallState::Dialing,
            caller: number(),
            at: Timestamp(1_000),
        }],
    )
    .0
}

/// A Call placed on the Phone, now connected, with its audio on the Phone.
fn live_on_phone() -> State {
    run(
        placed_on_phone(),
        [Input::CallChanged {
            call: CALL,
            state: CallState::Active,
            at: Timestamp(2_000),
        }],
    )
    .0
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

fn reported(effects: &[Effect]) -> Vec<Failure> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::ReportError(failure) => Some(*failure),
            _ => None,
        })
        .collect()
}

/// The timer for the Handoff's 3 s limit the effects started, if any.
fn handoff_timer(effects: &[Effect]) -> Option<Timer> {
    started(
        effects,
        |timer| matches!(timer, Timer::HandoffTimedOut(_)),
        3_000,
    )
}

/// The timer the effects started to check, a second after `Activate`, that
/// the audio link has started, if any.
fn not_started_timer(effects: &[Effect]) -> Option<Timer> {
    started(
        effects,
        |timer| matches!(timer, Timer::HandoffNotStarted(_)),
        1_000,
    )
}

/// The timer the effects started that `is`, checking it runs `after_ms`.
fn started(effects: &[Effect], is: impl Fn(&Timer) -> bool, after_ms: u32) -> Option<Timer> {
    effects.iter().find_map(|effect| match effect {
        Effect::StartTimer {
            timer,
            after_ms: ms,
        } if is(timer) => {
            assert_eq!(*ms, after_ms, "{timer:?}");
            Some(*timer)
        }
        _ => None,
    })
}

/// The Desktop has asked for the audio and the Phone has carried out
/// `Activate`: returns the state, the 3 s limit's timer, and the timer that
/// checks the audio link has started.
fn activated(state: State) -> (State, Timer, Timer) {
    let (state, effects) = run(state, [Input::MoveAudio(AudioLocation::Desktop)]);
    let limit = handoff_timer(&effects)
        .unwrap_or_else(|| panic!("no timer for the 3 s limit: {effects:?}"));
    assert_eq!(not_started_timer(&effects), None, "{effects:?}");
    let (state, effects) = run(
        state,
        [Input::PhoneCommandDone(PhoneCommand::MoveAudioToDesktop)],
    );
    let not_started = not_started_timer(&effects)
        .unwrap_or_else(|| panic!("no timer a second after Activate: {effects:?}"));
    (state, limit, not_started)
}

/// What the Call Window says about the audio; `None` with no Call Window
/// too.
fn audio(state: &State) -> Option<CallAudio> {
    state.surfaces().call_window.and_then(|window| window.audio)
}

const ON_PHONE: CallAudio = CallAudio {
    location: AudioLocation::Phone,
    moving_here: false,
};
const MOVING_HERE: CallAudio = CallAudio {
    location: AudioLocation::Phone,
    moving_here: true,
};
const ON_DESKTOP: CallAudio = CallAudio {
    location: AudioLocation::Desktop,
    moving_here: false,
};

#[test]
fn a_call_placed_on_the_phone_keeps_its_audio_there_and_shows_calling() {
    let (state, effects) = run(
        connected(),
        [Input::CallAdded {
            call: CALL,
            state: CallState::Dialing,
            caller: number(),
            at: Timestamp(1_000),
        }],
    );

    assert_eq!(phone_commands(&effects), []);
    assert_eq!(state.audio_location(), Some(AudioLocation::Phone));
    assert_eq!(audio(&state), Some(ON_PHONE));
}

#[test]
fn moving_the_audio_here_clears_reject_sco_and_activates() {
    let (state, effects) = run(live_on_phone(), [Input::MoveAudio(AudioLocation::Desktop)]);

    assert_eq!(phone_commands(&effects), [PhoneCommand::MoveAudioToDesktop]);
    assert!(handoff_timer(&effects).is_some(), "{effects:?}");
    assert_eq!(audio(&state), Some(MOVING_HERE));
}

#[test]
fn once_the_audio_is_here_reject_sco_is_set_again_and_the_window_says_so() {
    let (state, _) = run(live_on_phone(), [Input::MoveAudio(AudioLocation::Desktop)]);

    let (state, effects) = run(
        state,
        [
            Input::TransportChanged(Transport::Pending),
            Input::TransportChanged(Transport::Active),
        ],
    );

    assert_eq!(phone_commands(&effects), [PhoneCommand::KeepAudioOnPhone]);
    assert_eq!(state.audio_location(), Some(AudioLocation::Desktop));
    assert_eq!(audio(&state), Some(ON_DESKTOP));
}

#[test]
fn a_call_still_dialling_can_move_its_audio_here() {
    let (_, effects) = run(
        placed_on_phone(),
        [Input::MoveAudio(AudioLocation::Desktop)],
    );

    assert_eq!(phone_commands(&effects), [PhoneCommand::MoveAudioToDesktop]);
}

#[test]
fn asking_twice_while_the_audio_moves_asks_the_phone_once() {
    let (_, effects) = run(
        live_on_phone(),
        [
            Input::MoveAudio(AudioLocation::Desktop),
            Input::MoveAudio(AudioLocation::Desktop),
        ],
    );

    assert_eq!(phone_commands(&effects), [PhoneCommand::MoveAudioToDesktop]);
}

#[test]
fn if_the_audio_link_hasnt_started_a_second_after_activate_the_desktop_asks_once_more() {
    let (state, limit, not_started) = activated(live_on_phone());

    let (state, effects) = run(state, [Input::TimerFired(not_started)]);

    assert_eq!(phone_commands(&effects), [PhoneCommand::MoveAudioToDesktop]);
    assert_eq!(reported(&effects), []);
    // The retry keeps the Handoff's 3 s limit: it starts no timer of its own.
    assert_eq!(handoff_timer(&effects), None, "{effects:?}");
    assert_eq!(audio(&state), Some(MOVING_HERE));

    // The retry's own `Activate` doesn't lead to a third.
    let (state, effects) = run(
        state,
        [Input::PhoneCommandDone(PhoneCommand::MoveAudioToDesktop)],
    );
    assert_eq!(not_started_timer(&effects), None, "{effects:?}");
    let (state, effects) = run(state, [Input::TimerFired(not_started)]);
    assert_eq!(effects, []);

    let (state, effects) = run(
        state,
        [
            Input::TransportChanged(Transport::Pending),
            Input::TransportChanged(Transport::Active),
        ],
    );
    assert_eq!(phone_commands(&effects), [PhoneCommand::KeepAudioOnPhone]);
    assert_eq!(audio(&state), Some(ON_DESKTOP));
    let (_, effects) = run(state, [Input::TimerFired(limit)]);
    assert_eq!(effects, []);
}

#[test]
fn an_audio_link_that_has_started_gets_until_the_limit_without_a_second_activate() {
    let (state, limit, not_started) = activated(live_on_phone());
    let (state, _) = run(state, [Input::TransportChanged(Transport::Pending)]);

    let (state, effects) = run(state, [Input::TimerFired(not_started)]);
    assert_eq!(effects, []);
    assert_eq!(audio(&state), Some(MOVING_HERE));

    // A link that started but never came up fails at the limit, unretried.
    let (state, effects) = run(state, [Input::TimerFired(limit)]);
    assert_eq!(phone_commands(&effects), []);
    assert_eq!(reported(&effects), [Failure::AudioNotMoved]);
    assert_eq!(audio(&state), Some(ON_PHONE));
}

#[test]
fn a_new_handoff_isnt_ended_by_the_last_ones_timers() {
    let (state, limit, not_started) = activated(live_on_phone());
    let (state, _) = run(state, [Input::TimerFired(not_started)]);
    let (state, _) = run(state, [Input::TimerFired(limit)]);

    let (state, again, again_not_started) = activated(state);
    assert_ne!(again, limit);
    assert_ne!(again_not_started, not_started);
    let (state, effects) = run(
        state,
        [Input::TimerFired(limit), Input::TimerFired(not_started)],
    );
    assert_eq!(effects, []);
    assert_eq!(audio(&state), Some(MOVING_HERE));
}

#[test]
fn a_handoff_the_phone_never_carries_out_says_so_at_the_limit_and_keeps_the_call() {
    let (state, limit, not_started) = activated(live_on_phone());
    let (state, effects) = run(state, [Input::TimerFired(not_started)]);
    assert_eq!(phone_commands(&effects), [PhoneCommand::MoveAudioToDesktop]);

    let (state, effects) = run(state, [Input::TimerFired(limit)]);

    assert_eq!(reported(&effects), [Failure::AudioNotMoved]);
    // RejectSCO stays false, so the fallback on the Phone works; no hang-up.
    assert_eq!(phone_commands(&effects), []);
    assert_eq!(state.calls().len(), 1);
    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert_eq!(window.notice, Some(Failure::AudioNotMoved));
    assert_eq!(window.audio, Some(ON_PHONE));
}

#[test]
fn the_failure_notice_goes_once_the_audio_arrives_after_all() {
    let (state, limit, not_started) = activated(live_on_phone());
    let (state, _) = run(state, [Input::TimerFired(not_started)]);
    let (state, _) = run(state, [Input::TimerFired(limit)]);

    let (state, _) = run(state, [Input::TransportChanged(Transport::Active)]);

    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert_eq!(window.notice, None);
    assert_eq!(window.audio, Some(ON_DESKTOP));
}

#[test]
fn a_phone_that_refuses_the_handoff_outright_gets_one_more_try_then_the_notice() {
    let (state, _) = run(live_on_phone(), [Input::MoveAudio(AudioLocation::Desktop)]);

    let (state, effects) = run(
        state,
        [Input::PhoneCommandFailed(PhoneCommand::MoveAudioToDesktop)],
    );
    assert_eq!(phone_commands(&effects), [PhoneCommand::MoveAudioToDesktop]);
    assert_eq!(reported(&effects), []);

    let (state, effects) = run(
        state,
        [Input::PhoneCommandFailed(PhoneCommand::MoveAudioToDesktop)],
    );
    assert_eq!(reported(&effects), [Failure::AudioNotMoved]);
    assert_eq!(state.calls().len(), 1);
}

#[test]
fn answering_here_retries_and_reports_like_a_handoff() {
    let ringing = run(
        connected(),
        [Input::CallAdded {
            call: CALL,
            state: CallState::Ringing,
            caller: number(),
            at: Timestamp(1_000),
        }],
    )
    .0;
    let (state, effects) = run(ringing, [Input::Answer(CALL)]);
    let limit = handoff_timer(&effects).expect("a timer for the 3 s limit");
    let (state, effects) = run(
        state,
        [
            Input::PhoneCommandDone(PhoneCommand::Answer(CALL)),
            Input::CallChanged {
                call: CALL,
                state: CallState::Active,
                at: Timestamp(2_000),
            },
            Input::PhoneCommandDone(PhoneCommand::MoveAudioToDesktop),
        ],
    );
    let not_started = not_started_timer(&effects).expect("a timer a second after Activate");
    assert_eq!(audio(&state), Some(MOVING_HERE));

    let (state, effects) = run(state, [Input::TimerFired(not_started)]);
    assert_eq!(phone_commands(&effects), [PhoneCommand::MoveAudioToDesktop]);
    let (state, effects) = run(state, [Input::TimerFired(limit)]);

    assert_eq!(reported(&effects), [Failure::AudioNotMoved]);
    assert_eq!(audio(&state), Some(ON_PHONE));
}

#[test]
fn a_failed_answer_stops_waiting_for_the_audio() {
    let ringing = run(
        connected(),
        [Input::CallAdded {
            call: CALL,
            state: CallState::Ringing,
            caller: number(),
            at: Timestamp(1_000),
        }],
    )
    .0;
    let (state, effects) = run(ringing, [Input::Answer(CALL)]);
    let first = handoff_timer(&effects).expect("a Handoff timer");
    let (state, _) = run(
        state,
        [Input::PhoneCommandFailed(PhoneCommand::Answer(CALL))],
    );

    let (_, effects) = run(state, [Input::TimerFired(first)]);

    assert_eq!(effects, []);
}

#[test]
fn moving_the_audio_with_no_live_call_is_refused() {
    let state = connected();
    let input = Input::MoveAudio(AudioLocation::Desktop);

    assert_eq!(state.refusal(&input), Some(Failure::NoLiveCall));
    let (_, effects) = run(state, [input]);
    assert_eq!(phone_commands(&effects), []);
    assert_eq!(reported(&effects), [Failure::NoLiveCall]);
}

#[test]
fn moving_the_audio_where_it_already_is_does_nothing() {
    let state = live_on_phone();

    assert_eq!(state.refusal(&Input::MoveAudio(AudioLocation::Phone)), None);
    let (_, effects) = run(state, [Input::MoveAudio(AudioLocation::Phone)]);
    assert_eq!(effects, []);
}

#[test]
fn moving_the_audio_to_the_phone_gives_the_instruction_until_the_audio_leaves() {
    let (state, _) = run(
        live_on_phone(),
        [
            Input::MoveAudio(AudioLocation::Desktop),
            Input::TransportChanged(Transport::Active),
        ],
    );
    let input = Input::MoveAudio(AudioLocation::Phone);
    assert_eq!(
        state.refusal(&input),
        Some(Failure::HandoffToPhoneUnavailable)
    );

    let (state, effects) = run(state, [input]);
    assert_eq!(phone_commands(&effects), []);
    assert_eq!(reported(&effects), [Failure::HandoffToPhoneUnavailable]);
    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert_eq!(window.notice, Some(Failure::HandoffToPhoneUnavailable));

    // The user taps the audio button on the Phone.
    let (state, _) = run(state, [Input::TransportChanged(Transport::Idle)]);
    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert_eq!(window.notice, None);
    assert_eq!(window.audio, Some(ON_PHONE));
}

#[test]
fn the_call_ending_mid_handoff_forgets_it() {
    let (state, effects) = run(live_on_phone(), [Input::MoveAudio(AudioLocation::Desktop)]);
    let first = handoff_timer(&effects).expect("a Handoff timer");

    let (state, _) = run(state, [Input::CallEnded { call: CALL }]);
    let (_, effects) = run(state, [Input::TimerFired(first)]);

    assert_eq!(effects, []);
}
