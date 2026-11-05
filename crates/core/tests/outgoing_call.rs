//! The Call state machine's outgoing-Call policy, at its public interface:
//! what may be dialled, refusals, Desktop audio for a Call placed here, the
//! Call Window while the Call connects, and failures the Phone reports.

use desktop_calls_core::{
    AudioLocation, CallId, CallState, CallWindowPhase, Caller, DialProgress, Effect, Input, Phone,
    PhoneCommand, PhoneRefusal, State, Timer, Timestamp, Transport, matching, messages::Failure,
    step,
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

fn paired() -> State {
    run(
        State::default(),
        [Input::PhonePaired(Phone {
            name: "My Pixel".to_owned(),
            address: ADDRESS.to_owned(),
        })],
    )
    .0
}

fn connected() -> State {
    run(
        paired(),
        [
            Input::PhoneConnected {
                address: ADDRESS.to_owned(),
            },
            Input::PhoneCommandDone(PhoneCommand::KeepAudioOnPhone),
        ],
    )
    .0
}

fn dialled() -> State {
    run(connected(), [Input::Dial("07700 900123".to_owned())]).0
}

fn appeared(state: State, call_state: CallState, caller: Caller) -> State {
    run(
        state,
        [Input::CallAdded {
            call: CALL,
            state: call_state,
            caller,
            at: Timestamp(1_000),
        }],
    )
    .0
}

fn number() -> Caller {
    Caller::Number("07700900123".to_owned())
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

/// What the effects say became of dial attempts, for whoever asked.
fn progress(effects: &[Effect]) -> Vec<DialProgress> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::DialProgress(progress) => Some(*progress),
            _ => None,
        })
        .collect()
}

/// The dial attempt `effects` say the Phone was asked to place.
fn asked(effects: &[Effect]) -> u32 {
    match progress(effects).as_slice() {
        [DialProgress::Asked(attempt)] => *attempt,
        other => panic!("not one dial asked of the Phone: {other:?}"),
    }
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

#[test]
fn dialable_numbers_lose_their_spacing_and_tel_links_their_scheme() {
    assert_eq!(
        matching::dialable("07700 900123").as_deref(),
        Some("07700900123")
    );
    assert_eq!(
        matching::dialable("tel:+447700900123").as_deref(),
        Some("+447700900123")
    );
    assert_eq!(
        matching::dialable(" (0)20-7946.0018 ").as_deref(),
        Some("02079460018")
    );
    assert_eq!(
        matching::dialable("TEL:+44-7700-900123;phone-context=example.com").as_deref(),
        Some("+447700900123")
    );
    assert_eq!(
        matching::dialable("tel:%2B447700900123").as_deref(),
        Some("+447700900123")
    );
    assert_eq!(matching::dialable("*#06#").as_deref(), Some("*#06#"));
    assert_eq!(
        matching::dialable("07700900123,,1234").as_deref(),
        Some("07700900123,,1234")
    );
}

#[test]
fn undialable_text_is_refused_before_it_reaches_the_phone() {
    for text in [
        "",
        "   ",
        "tel:",
        "call mum",
        "0770090012x",
        "07700 9001 23!",
    ] {
        assert_eq!(matching::dialable(text), None, "{text:?}");
    }
    let eighty = "1".repeat(80);
    assert_eq!(
        matching::dialable(&eighty).as_deref(),
        Some(eighty.as_str())
    );
    assert_eq!(matching::dialable(&"1".repeat(81)), None);
}

#[test]
fn dialling_lets_the_phone_bring_the_audio_here_then_dials() {
    let (state, effects) = run(connected(), [Input::Dial("07700 900123".to_owned())]);

    assert_eq!(
        phone_commands(&effects),
        vec![
            PhoneCommand::AllowAudioOnDesktop,
            PhoneCommand::Dial("07700900123".to_owned()),
        ]
    );
    assert!(reported(&effects).is_empty());
    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert_eq!(window.call, None);
    assert_eq!(window.caller, number());
    assert_eq!(window.phase, CallWindowPhase::Calling);
    assert_eq!(window.notice, None);
    // Nothing is on the Phone yet, so there is no Call to list.
    assert!(state.calls().is_empty());
}

#[test]
fn the_dialled_call_shows_calling_then_ringing_then_its_timer() {
    let state = appeared(dialled(), CallState::Dialing, number());
    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert_eq!(window.call, Some(CALL));
    assert_eq!(window.phase, CallWindowPhase::Calling);
    assert_eq!(state.calls()[0].direction().as_str(), "outgoing");

    let (state, _) = run(
        state,
        [Input::CallChanged {
            call: CALL,
            state: CallState::Alerting,
            at: Timestamp(1_500),
        }],
    );
    assert_eq!(
        state.surfaces().call_window.map(|window| window.phase),
        Some(CallWindowPhase::Ringing)
    );

    let (state, _) = run(
        state,
        [
            Input::TransportChanged(Transport::Pending),
            Input::TransportChanged(Transport::Active),
            Input::CallChanged {
                call: CALL,
                state: CallState::Active,
                at: Timestamp(2_000),
            },
        ],
    );
    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert!(
        matches!(window.phase, CallWindowPhase::Live { active_since, .. } if active_since == Timestamp(2_000)),
        "{window:?}"
    );
    assert_eq!(
        state.audio_location().map(AudioLocation::as_str),
        Some("desktop")
    );
}

#[test]
fn a_call_that_appears_alerting_shows_ringing() {
    // The Pixel reports a dialled Call straight away as alerting.
    let state = appeared(dialled(), CallState::Alerting, number());

    assert_eq!(
        state.surfaces().call_window.map(|window| window.phase),
        Some(CallWindowPhase::Ringing)
    );
}

#[test]
fn a_dialled_call_the_phone_reports_without_a_number_shows_the_number_dialled() {
    let state = appeared(dialled(), CallState::Alerting, Caller::Withheld);

    assert_eq!(state.calls()[0].caller(), &number());
}

#[test]
fn dialling_while_disconnected_is_refused_with_the_reason() {
    let disconnected = run(
        connected(),
        [Input::PhoneDisconnected {
            address: ADDRESS.to_owned(),
        }],
    )
    .0;
    for (state, failure) in [
        (State::default(), Failure::PhoneNotSetUp),
        (paired(), Failure::PhoneConnecting),
        (disconnected, Failure::PhoneOutOfRange),
    ] {
        let (state, effects) = run(state, [Input::Dial("07700 900123".to_owned())]);

        assert!(phone_commands(&effects).is_empty(), "{effects:?}");
        assert_eq!(reported(&effects), vec![failure]);
        let window = state.surfaces().call_window.expect("the Call Window shows");
        assert_eq!(window.phase, CallWindowPhase::NotPlaced);
        assert_eq!(window.caller, number());
        assert_eq!(window.notice, Some(failure));
    }
}

#[test]
fn dialling_undialable_text_is_refused() {
    let (state, effects) = run(connected(), [Input::Dial("call mum".to_owned())]);

    assert!(phone_commands(&effects).is_empty(), "{effects:?}");
    assert_eq!(reported(&effects), vec![Failure::Undialable]);
    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert_eq!(window.phase, CallWindowPhase::NotPlaced);
    assert_eq!(window.caller, Caller::Number("call mum".to_owned()));
}

#[test]
fn dialling_while_a_call_is_live_is_refused_and_noted_on_the_live_call() {
    let live = run(
        appeared(dialled(), CallState::Alerting, number()),
        [Input::CallChanged {
            call: CALL,
            state: CallState::Active,
            at: Timestamp(2_000),
        }],
    )
    .0;

    let (state, effects) = run(live, [Input::Dial("01632 960001".to_owned())]);

    assert!(phone_commands(&effects).is_empty(), "{effects:?}");
    assert_eq!(reported(&effects), vec![Failure::CallInProgress]);
    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert_eq!(window.call, Some(CALL));
    assert!(matches!(window.phase, CallWindowPhase::Live { .. }));
    assert_eq!(window.notice, Some(Failure::CallInProgress));
    let Some(Effect::StartTimer { timer, .. }) = effects
        .iter()
        .find(|effect| matches!(effect, Effect::StartTimer { .. }))
    else {
        panic!("no timer clears the notice: {effects:?}");
    };

    let (state, _) = run(state, [Input::TimerFired(*timer)]);
    assert_eq!(
        state
            .surfaces()
            .call_window
            .and_then(|window| window.notice),
        None
    );
}

#[test]
fn dialling_again_while_the_first_dial_is_pending_is_refused() {
    let (_, effects) = run(dialled(), [Input::Dial("01632 960001".to_owned())]);

    assert!(phone_commands(&effects).is_empty(), "{effects:?}");
    assert_eq!(reported(&effects), vec![Failure::CallInProgress]);
}

#[test]
fn each_phone_refusal_is_explained_and_the_audio_stays_on_the_phone_again() {
    for (refusal, failure) in [
        (PhoneRefusal::InvalidFormat, Failure::NumberRefused),
        (PhoneRefusal::InvalidState, Failure::PhoneBusy),
        (PhoneRefusal::InProgress, Failure::PhoneBusy),
        (PhoneRefusal::NotSupported, Failure::DialNotSupported),
        (PhoneRefusal::NoNetwork, Failure::NoSignal),
        (PhoneRefusal::NetworkTimeout, Failure::NetworkError),
        (PhoneRefusal::Failed, Failure::DialFailed),
    ] {
        let (state, effects) = run(dialled(), [Input::DialFailed(refusal)]);

        assert_eq!(reported(&effects), vec![failure], "{refusal:?}");
        assert_eq!(
            phone_commands(&effects),
            vec![PhoneCommand::KeepAudioOnPhone],
            "{refusal:?}"
        );
        let window = state.surfaces().call_window.expect("the Call Window shows");
        assert_eq!(window.phase, CallWindowPhase::NotPlaced, "{refusal:?}");
        assert_eq!(window.notice, Some(failure), "{refusal:?}");
    }
}

#[test]
fn the_not_placed_call_window_closes_after_a_while() {
    let (state, effects) = run(dialled(), [Input::DialFailed(PhoneRefusal::NoNetwork)]);
    let timer = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::StartTimer { timer, .. } => Some(*timer),
            _ => None,
        })
        .expect("a timer closes the Call Window");

    let (state, _) = run(state, [Input::TimerFired(timer)]);

    assert_eq!(state.surfaces().call_window, None);
}

#[test]
fn a_dial_the_phone_never_starts_times_out() {
    let (state, effects) = run(connected(), [Input::Dial("07700 900123".to_owned())]);
    let timeout = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::StartTimer {
                timer: timer @ Timer::DialTimedOut(_),
                ..
            } => Some(*timer),
            _ => None,
        })
        .expect("a timer limits the wait for the Phone");

    let (state, effects) = run(state, [Input::TimerFired(timeout)]);

    assert_eq!(reported(&effects), vec![Failure::DialFailed]);
    assert_eq!(
        phone_commands(&effects),
        vec![PhoneCommand::KeepAudioOnPhone]
    );
    assert_eq!(
        state.surfaces().call_window.map(|window| window.phase),
        Some(CallWindowPhase::NotPlaced)
    );
}

#[test]
fn the_timeout_of_a_dial_that_connected_does_nothing() {
    let (state, effects) = run(connected(), [Input::Dial("07700 900123".to_owned())]);
    let timeout = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::StartTimer { timer, .. } => Some(*timer),
            _ => None,
        })
        .expect("a timer limits the wait for the Phone");
    let state = appeared(state, CallState::Alerting, number());

    let (state, effects) = run(state, [Input::TimerFired(timeout)]);

    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(
        state.surfaces().call_window.map(|window| window.phase),
        Some(CallWindowPhase::Ringing)
    );
}

#[test]
fn hanging_up_before_the_phone_reports_the_call_hangs_it_up_when_it_appears() {
    let (state, effects) = run(dialled(), [Input::CancelDial]);
    assert!(phone_commands(&effects).is_empty(), "{effects:?}");

    let (_, effects) = run(
        state,
        [Input::CallAdded {
            call: CALL,
            state: CallState::Alerting,
            caller: number(),
            at: Timestamp(1_000),
        }],
    );

    assert_eq!(phone_commands(&effects), vec![PhoneCommand::HangUp(CALL)]);
}

#[test]
fn hanging_up_before_the_phone_reports_the_call_shows_call_ended_once_and_never_the_call() {
    let phase = |state: &State| state.surfaces().call_window.map(|window| window.phase);
    let (state, effects) = run(dialled(), [Input::CancelDial]);
    assert_eq!(phase(&state), Some(CallWindowPhase::Ended));
    let close = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::StartTimer { timer, .. } => Some(*timer),
            _ => None,
        })
        .expect("a timer closes the Call Window");

    let state = appeared(state, CallState::Alerting, number());
    assert_eq!(phase(&state), Some(CallWindowPhase::Ended));

    let (state, effects) = run(state, [Input::CallEnded { call: CALL }]);
    assert_eq!(phase(&state), Some(CallWindowPhase::Ended));
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::StartTimer { .. })),
        "a second Call ended: {effects:?}"
    );

    let (state, _) = run(state, [Input::TimerFired(close)]);
    assert_eq!(phase(&state), None);
}

#[test]
fn the_phone_disconnecting_while_dialling_ends_the_dial_with_the_reason() {
    let (state, effects) = run(
        dialled(),
        [Input::PhoneDisconnected {
            address: ADDRESS.to_owned(),
        }],
    );

    assert_eq!(reported(&effects), vec![Failure::PhoneOutOfRange]);
    assert_eq!(
        state.surfaces().call_window.map(|window| window.phase),
        Some(CallWindowPhase::NotPlaced)
    );
}

#[test]
fn a_call_placed_here_ends_with_call_ended_and_the_audio_kept_on_the_phone_again() {
    let state = appeared(dialled(), CallState::Alerting, number());

    let (state, effects) = run(state, [Input::CallEnded { call: CALL }]);

    assert_eq!(
        phone_commands(&effects),
        vec![PhoneCommand::KeepAudioOnPhone]
    );
    assert_eq!(
        state.surfaces().call_window.map(|window| window.phase),
        Some(CallWindowPhase::Ended)
    );
}

#[test]
fn a_failed_dial_never_annotates_a_call_that_comes_after_it() {
    let (state, _) = run(dialled(), [Input::DialFailed(PhoneRefusal::NoNetwork)]);

    let state = appeared(state, CallState::Active, Caller::Withheld);

    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert_eq!(window.call, Some(CALL));
    assert_eq!(window.notice, None);
}

#[test]
fn a_dial_is_placed_once_the_phone_reports_its_call() {
    let (state, effects) = run(connected(), [Input::Dial("07700 900123".to_owned())]);
    let attempt = asked(&effects);

    let (_, effects) = run(
        state,
        [Input::CallAdded {
            call: CALL,
            state: CallState::Dialing,
            caller: number(),
            at: Timestamp(1_000),
        }],
    );

    assert_eq!(progress(&effects), vec![DialProgress::Placed(attempt)]);
}

#[test]
fn a_dial_refused_at_once_is_not_placed_with_the_reason_in_the_same_step() {
    let (state, _) = run(
        connected(),
        [Input::PhoneDisconnected {
            address: ADDRESS.to_owned(),
        }],
    );
    let (_, first) = run(state.clone(), [Input::Dial("07700 900123".to_owned())]);
    let (_, second) = run(state, [Input::Dial("call mum".to_owned())]);

    assert!(
        matches!(
            progress(&first).as_slice(),
            [DialProgress::NotPlaced(_, Failure::PhoneOutOfRange)]
        ),
        "{first:?}"
    );
    assert!(
        matches!(
            progress(&second).as_slice(),
            [DialProgress::NotPlaced(_, Failure::Undialable)]
        ),
        "{second:?}"
    );
}

#[test]
fn a_dial_the_phone_refuses_is_not_placed_with_the_call_windows_reason() {
    let (state, effects) = run(connected(), [Input::Dial("07700 900123".to_owned())]);
    let attempt = asked(&effects);

    let (state, effects) = run(state, [Input::DialFailed(PhoneRefusal::NoNetwork)]);

    assert_eq!(
        progress(&effects),
        vec![DialProgress::NotPlaced(attempt, Failure::NoSignal)]
    );
    assert_eq!(
        state
            .surfaces()
            .call_window
            .and_then(|window| window.notice),
        Some(Failure::NoSignal)
    );
}

#[test]
fn a_dial_that_times_out_or_loses_the_phone_is_not_placed() {
    let (state, effects) = run(connected(), [Input::Dial("07700 900123".to_owned())]);
    let attempt = asked(&effects);

    let (_, timed_out) = run(
        state.clone(),
        [Input::TimerFired(Timer::DialTimedOut(attempt))],
    );
    let (_, unpaired) = run(
        state,
        [Input::PhoneUnpaired {
            address: ADDRESS.to_owned(),
        }],
    );

    assert_eq!(
        progress(&timed_out),
        vec![DialProgress::NotPlaced(attempt, Failure::DialFailed)]
    );
    assert_eq!(
        progress(&unpaired),
        vec![DialProgress::NotPlaced(attempt, Failure::PhoneNotSetUp)]
    );
}
