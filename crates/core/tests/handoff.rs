//! The Call state machine's Audio Location policy at its public interface:
//! a Call placed on the Phone keeps its audio there, Handoff to the Desktop
//! with its one retry (a second after `Activate` if the audio link hasn't
//! started) and its 3 s limit, a Handoff that fails, and the fallback
//! Handoff to the Phone, which bounces hands-free (ADR-0005), with what it
//! shows when it fails.

use desktop_calls_core::{
    AudioLocation, CallAudio, CallId, CallState, Caller, ConnectionState, Effect, Input, Phone,
    PhoneCommand, State, Timer, Timestamp, Transport, messages::Failure, step,
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
    moving: None,
};
const MOVING_HERE: CallAudio = CallAudio {
    location: AudioLocation::Phone,
    moving: Some(AudioLocation::Desktop),
};
const ON_DESKTOP: CallAudio = CallAudio {
    location: AudioLocation::Desktop,
    moving: None,
};
/// Asked to go to the Phone, and still on the Desktop.
const LEAVING_DESKTOP: CallAudio = CallAudio {
    location: AudioLocation::Desktop,
    moving: Some(AudioLocation::Phone),
};
/// On the Phone, while hands-free comes back from the Handoff's bounce.
const ARRIVED_ON_PHONE: CallAudio = CallAudio {
    location: AudioLocation::Phone,
    moving: Some(AudioLocation::Phone),
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

/// The live Call's audio on the Desktop, after a Handoff here.
fn live_here() -> State {
    run(
        live_on_phone(),
        [
            Input::MoveAudio(AudioLocation::Desktop),
            Input::PhoneCommandDone(PhoneCommand::MoveAudioToDesktop),
            Input::TransportChanged(Transport::Pending),
            Input::TransportChanged(Transport::Active),
            Input::PhoneCommandDone(PhoneCommand::KeepAudioOnPhone),
        ],
    )
    .0
}

/// The timer for a Handoff to the Phone's 5 s limit the effects started.
fn to_phone_timer(effects: &[Effect]) -> Option<Timer> {
    started(
        effects,
        |timer| matches!(timer, Timer::HandoffTimedOut(_)),
        5_000,
    )
}

/// The Desktop has asked to move the audio to the Phone: returns the state
/// and the Handoff's limit.
fn asked_for_phone(state: State) -> (State, Timer) {
    let (state, effects) = run(state, [Input::MoveAudio(AudioLocation::Phone)]);
    let limit =
        to_phone_timer(&effects).unwrap_or_else(|| panic!("no timer for the limit: {effects:?}"));
    (state, limit)
}

/// Hands-free drops for the Handoff to the Phone (ADR-0005): the
/// AudioGateway and the Call go, without the Call ending.
fn dropped() -> Input {
    Input::PhoneDisconnected {
        address: ADDRESS.to_owned(),
    }
}

/// Hands-free is back, and so is the Call, still active.
fn back() -> [Input; 4] {
    [
        Input::PhoneConnected {
            address: ADDRESS.to_owned(),
        },
        Input::PhoneCommandDone(PhoneCommand::MoveAudioToPhone),
        Input::TransportChanged(Transport::Idle),
        Input::CallChanged {
            call: CALL,
            state: CallState::Active,
            at: Timestamp(9_000),
        },
    ]
}

#[test]
fn moving_the_audio_to_the_phone_asks_the_phone_link_to_move_it() {
    let state = live_here();
    let input = Input::MoveAudio(AudioLocation::Phone);
    assert_eq!(state.refusal(&input), None);

    let (state, effects) = run(state, [input]);

    assert_eq!(phone_commands(&effects), [PhoneCommand::MoveAudioToPhone]);
    assert!(to_phone_timer(&effects).is_some(), "{effects:?}");
    assert_eq!(reported(&effects), []);
    assert_eq!(audio(&state), Some(LEAVING_DESKTOP));
}

#[test]
fn hands_free_dropping_for_the_handoff_puts_the_audio_on_the_phone_and_keeps_the_call() {
    let (state, _) = asked_for_phone(live_here());

    let (state, effects) = run(state, [dropped()]);

    assert_eq!(phone_commands(&effects), []);
    assert_eq!(state.audio_location(), Some(AudioLocation::Phone));
    assert_eq!(audio(&state), Some(ARRIVED_ON_PHONE));
    // The Phone is still in range: its link is coming straight back.
    assert_eq!(state.connection_state(), &ConnectionState::Connected);
    assert_eq!(state.calls().len(), 1);
}

#[test]
fn once_hands_free_and_the_call_are_back_the_handoff_to_the_phone_is_over() {
    let (state, limit) = asked_for_phone(live_here());
    let before = state.calls()[0].clone();
    let (state, _) = run(state, [dropped()]);

    let (state, effects) = run(state, back());

    // RejectSCO is set again on the new AudioGateway, the audio isn't
    // brought back here, and the same link needs no new Contacts sync.
    assert_eq!(phone_commands(&effects), [PhoneCommand::KeepAudioOnPhone]);
    assert!(!effects.contains(&Effect::SyncContacts), "{effects:?}");
    assert_eq!(audio(&state), Some(ON_PHONE));
    assert_eq!(state.connection_state(), &ConnectionState::Connected);
    let after = &state.calls()[0];
    assert_eq!(after.id(), before.id());
    assert_eq!(after.active_since(), before.active_since());
    let (state, effects) = run(state, [Input::TimerFired(limit)]);
    assert_eq!(effects, []);
    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert_eq!(window.notice, None);
}

#[test]
fn the_audio_can_go_here_and_to_the_phone_twice_in_one_call() {
    let (state, _) = asked_for_phone(live_here());
    let (state, _) = run(state, [dropped()]);
    let (state, _) = run(state, back());

    let (state, effects) = run(state, [Input::MoveAudio(AudioLocation::Desktop)]);
    assert_eq!(phone_commands(&effects), [PhoneCommand::MoveAudioToDesktop]);
    let (state, _) = run(
        state,
        [
            Input::PhoneCommandDone(PhoneCommand::MoveAudioToDesktop),
            Input::TransportChanged(Transport::Active),
        ],
    );
    assert_eq!(audio(&state), Some(ON_DESKTOP));

    let (state, _) = asked_for_phone(state);
    let (state, _) = run(state, [dropped()]);
    let (state, _) = run(state, back());

    assert_eq!(audio(&state), Some(ON_PHONE));
    assert_eq!(state.calls().len(), 1);
    assert_eq!(state.calls()[0].id(), CALL);
}

#[test]
fn while_the_audio_goes_to_the_phone_asking_again_does_nothing() {
    let (state, _) = asked_for_phone(live_here());
    let (state, effects) = run(
        state,
        [
            Input::MoveAudio(AudioLocation::Desktop),
            Input::MoveAudio(AudioLocation::Phone),
        ],
    );
    assert_eq!(effects, []);

    let (state, _) = run(state, [dropped()]);
    let (_, effects) = run(
        state,
        [
            Input::MoveAudio(AudioLocation::Desktop),
            Input::MoveAudio(AudioLocation::Phone),
        ],
    );
    assert_eq!(effects, []);
}

#[test]
fn a_handoff_to_the_phone_the_phone_link_cant_start_says_how_to_do_it_on_the_phone() {
    let (state, limit) = asked_for_phone(live_here());

    let (state, effects) = run(
        state,
        [Input::PhoneCommandFailed(PhoneCommand::MoveAudioToPhone)],
    );

    assert_eq!(reported(&effects), [Failure::AudioNotMovedToPhone]);
    assert_eq!(phone_commands(&effects), []);
    assert_eq!(state.calls().len(), 1);
    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert_eq!(window.notice, Some(Failure::AudioNotMovedToPhone));
    assert_eq!(window.audio, Some(ON_DESKTOP));
    let (state, effects) = run(state, [Input::TimerFired(limit)]);
    assert_eq!(effects, []);

    // The user taps the audio button on the Phone.
    let (state, _) = run(state, [Input::TransportChanged(Transport::Idle)]);
    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert_eq!(window.notice, None);
    assert_eq!(window.audio, Some(ON_PHONE));
}

#[test]
fn a_handoff_to_the_phone_that_never_starts_says_so_at_the_limit() {
    let (state, limit) = asked_for_phone(live_here());

    let (state, effects) = run(state, [Input::TimerFired(limit)]);

    assert_eq!(reported(&effects), [Failure::AudioNotMovedToPhone]);
    assert_eq!(audio(&state), Some(ON_DESKTOP));
    assert_eq!(state.calls().len(), 1);
}

// Hands-free not coming back after the drop is in `handoff_recovery.rs`.

#[test]
fn the_phone_link_giving_up_on_the_call_mid_handoff_to_the_phone_ends_the_handoff() {
    let (state, limit) = asked_for_phone(live_here());
    let (state, _) = run(state, [dropped(), Input::CallEnded { call: CALL }]);

    let (state, effects) = run(state, [Input::TimerFired(limit)]);
    assert_eq!(effects, []);
    // Hands-free coming back is then an ordinary connect.
    let (_, effects) = run(
        state,
        [Input::PhoneConnected {
            address: ADDRESS.to_owned(),
        }],
    );
    assert!(effects.contains(&Effect::SyncContacts), "{effects:?}");
}

#[test]
fn the_call_ending_mid_handoff_forgets_it() {
    let (state, effects) = run(live_on_phone(), [Input::MoveAudio(AudioLocation::Desktop)]);
    let first = handoff_timer(&effects).expect("a Handoff timer");

    let (state, _) = run(state, [Input::CallEnded { call: CALL }]);
    let (_, effects) = run(state, [Input::TimerFired(first)]);

    assert_eq!(effects, []);
}
