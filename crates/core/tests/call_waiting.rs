//! The Call state machine's call-waiting policy, at its public interface: a
//! a Waiting Call during a live one, Hold & Answer, End & Answer, Decline,
//! Swap, and the Phone refusing them.

use desktop_calls_core::{
    AudioLocation, CallId, CallState, CallWindowPhase, Caller, Codec, Effect, Input, Phone,
    PhoneCommand, RingingCaller, State, Timestamp, Transport, WaitingOptions, messages::Failure,
    step,
};

const ADDRESS: &str = "AA:BB:CC:DD:EE:FF";
/// The Call the user is on.
const FIRST: CallId = CallId(1);
/// The Call that rings during it.
const SECOND: CallId = CallId(2);

fn bob() -> Caller {
    Caller::Number("+447700900123".to_owned())
}

fn alice() -> Caller {
    Caller::Number("+447700900456".to_owned())
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

/// On a Call with Bob, answered on the Desktop, its audio here.
fn on_a_call() -> State {
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
            Input::CallAdded {
                call: FIRST,
                state: CallState::Ringing,
                caller: bob(),
                at: Timestamp(1_000),
            },
            Input::Answer(FIRST),
            Input::CallChanged {
                call: FIRST,
                state: CallState::Active,
                at: Timestamp(2_000),
            },
            Input::CodecChanged(Codec::from_id(2)),
            Input::TransportChanged(Transport::Active),
        ],
    )
    .0
}

/// [`on_a_call`], and Alice calls: her Call waits.
fn waiting() -> State {
    run(
        on_a_call(),
        [Input::CallAdded {
            call: SECOND,
            state: CallState::Waiting,
            caller: alice(),
            at: Timestamp(3_000),
        }],
    )
    .0
}

/// [`waiting`], then Hold & Answer, carried out by the Phone.
fn held() -> State {
    run(
        waiting(),
        [
            Input::HoldAndAnswer,
            Input::CallChanged {
                call: FIRST,
                state: CallState::Held,
                at: Timestamp(4_000),
            },
            Input::CallChanged {
                call: SECOND,
                state: CallState::Active,
                at: Timestamp(4_000),
            },
        ],
    )
    .0
}

#[test]
fn a_waiting_call_shows_the_banner_with_the_call_waiting_options_and_never_rings() {
    let (state, effects) = run(
        on_a_call(),
        [Input::CallAdded {
            call: SECOND,
            state: CallState::Waiting,
            caller: alice(),
            at: Timestamp(3_000),
        }],
    );

    assert!(!state.ringing());
    assert!(!effects.contains(&Effect::StartRinging));
    let banner = state.surfaces().banner.expect("the Banner shows");
    assert_eq!(banner.call, SECOND);
    assert_eq!(
        banner.caller,
        RingingCaller::Known {
            caller: alice(),
            contact: None
        }
    );
    assert_eq!(banner.waiting, Some(WaitingOptions::HoldOrEnd));
    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert_eq!(window.call, Some(FIRST));
}

#[test]
fn hold_and_answer_asks_the_phone_and_closes_the_banner() {
    let (state, effects) = run(waiting(), [Input::HoldAndAnswer]);

    assert_eq!(
        effects,
        [Effect::Phone(PhoneCommand::HoldAndAnswer(SECOND))]
    );
    assert_eq!(state.surfaces().banner, None);
}

#[test]
fn answering_a_waiting_call_holds_the_live_one_rather_than_ending_it() {
    let (_, effects) = run(waiting(), [Input::Answer(SECOND)]);

    assert_eq!(
        effects,
        [Effect::Phone(PhoneCommand::HoldAndAnswer(SECOND))]
    );
}

#[test]
fn once_held_the_call_window_shows_the_answered_call_live_and_the_first_on_hold() {
    let window = held()
        .surfaces()
        .call_window
        .expect("the Call Window shows");

    assert_eq!(window.call, Some(SECOND));
    assert_eq!(window.caller, alice());
    assert!(matches!(window.phase, CallWindowPhase::Live { .. }));
    let on_hold = window.held.expect("the Call on hold is listed");
    assert_eq!(on_hold.call, FIRST);
    assert_eq!(on_hold.caller, bob());
}

#[test]
fn swap_asks_the_phone_and_the_marks_follow_what_it_reports() {
    let (state, effects) = run(held(), [Input::Swap]);
    assert_eq!(effects, [Effect::Phone(PhoneCommand::Swap)]);

    let (state, _) = run(
        state,
        [
            Input::CallChanged {
                call: SECOND,
                state: CallState::Held,
                at: Timestamp(5_000),
            },
            Input::CallChanged {
                call: FIRST,
                state: CallState::Active,
                at: Timestamp(5_000),
            },
        ],
    );

    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert_eq!(window.call, Some(FIRST));
    assert_eq!(window.held.map(|held| held.call), Some(SECOND));
}

#[test]
fn hanging_up_the_live_call_leaves_the_held_one_which_the_phone_makes_live() {
    let (state, effects) = run(held(), [Input::HangUp(SECOND)]);
    assert_eq!(effects, [Effect::Phone(PhoneCommand::HangUp(SECOND))]);

    let (state, effects) = run(
        state,
        [
            Input::CallEnded { call: SECOND },
            Input::CallChanged {
                call: FIRST,
                state: CallState::Active,
                at: Timestamp(5_000),
            },
        ],
    );

    assert!(!effects.contains(&Effect::Phone(PhoneCommand::KeepAudioOnPhone)));
    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert_eq!(window.call, Some(FIRST));
    assert!(matches!(window.phase, CallWindowPhase::Live { .. }));
    assert_eq!(window.held, None);
}

#[test]
fn hanging_up_the_held_call_leaves_the_live_one() {
    let (state, effects) = run(held(), [Input::HangUp(FIRST)]);
    assert_eq!(effects, [Effect::Phone(PhoneCommand::HangUp(FIRST))]);

    let (state, _) = run(state, [Input::CallEnded { call: FIRST }]);

    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert_eq!(window.call, Some(SECOND));
    assert_eq!(window.held, None);
}

#[test]
fn a_lone_call_on_hold_shows_as_on_hold_and_swap_resumes_it() {
    let (state, _) = run(
        held(),
        [Input::HangUp(SECOND), Input::CallEnded { call: SECOND }],
    );

    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert_eq!(window.call, Some(FIRST));
    assert_eq!(
        window.phase,
        CallWindowPhase::OnHold {
            active_since: Timestamp(2_000)
        }
    );
    let (_, effects) = run(state, [Input::Swap]);
    assert_eq!(effects, [Effect::Phone(PhoneCommand::Swap)]);
}

#[test]
fn end_and_answer_asks_the_phone_and_the_answered_call_takes_the_call_window() {
    let (state, effects) = run(waiting(), [Input::EndAndAnswer]);
    assert_eq!(effects, [Effect::Phone(PhoneCommand::EndAndAnswer(SECOND))]);
    assert_eq!(state.surfaces().banner, None);

    let (state, effects) = run(
        state,
        [
            Input::CallEnded { call: FIRST },
            Input::CallChanged {
                call: SECOND,
                state: CallState::Active,
                at: Timestamp(4_000),
            },
        ],
    );

    assert!(!effects.contains(&Effect::Phone(PhoneCommand::KeepAudioOnPhone)));
    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert_eq!(window.call, Some(SECOND));
    assert!(matches!(window.phase, CallWindowPhase::Live { .. }));
}

#[test]
fn declining_a_waiting_call_hangs_up_only_that_call() {
    let (state, effects) = run(waiting(), [Input::Decline(SECOND)]);
    assert_eq!(effects, [Effect::Phone(PhoneCommand::HangUp(SECOND))]);

    let (state, effects) = run(state, [Input::CallEnded { call: SECOND }]);

    assert!(!effects.contains(&Effect::Phone(PhoneCommand::KeepAudioOnPhone)));
    assert_eq!(state.surfaces().banner, None);
    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert_eq!(window.call, Some(FIRST));
    assert!(matches!(window.phase, CallWindowPhase::Live { .. }));
}

#[test]
fn the_phone_refusing_hold_says_so_and_leaves_both_calls_as_they_were() {
    let (state, effects) = run(
        waiting(),
        [
            Input::HoldAndAnswer,
            Input::PhoneCommandFailed(PhoneCommand::HoldAndAnswer(SECOND)),
        ],
    );

    assert!(effects.contains(&Effect::ReportError(Failure::HoldAndAnswerFailed)));
    let banner = state.surfaces().banner.expect("the Waiting Call rings on");
    assert_eq!(banner.call, SECOND);
    assert_eq!(banner.waiting, Some(WaitingOptions::HoldOrEnd));
    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert_eq!(window.call, Some(FIRST));
    assert_eq!(window.notice, Some(Failure::HoldAndAnswerFailed));
    assert_eq!(state.calls()[0].state(), CallState::Active);
    assert_eq!(state.calls()[1].state(), CallState::Waiting);
}

#[test]
fn the_phone_refusing_swap_says_so_until_the_notice_times_out() {
    let (state, effects) = run(
        held(),
        [Input::Swap, Input::PhoneCommandFailed(PhoneCommand::Swap)],
    );

    assert!(effects.contains(&Effect::ReportError(Failure::SwapFailed)));
    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert_eq!(window.notice, Some(Failure::SwapFailed));
    assert_eq!(window.call, Some(SECOND));

    let timer = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::StartTimer {
                timer,
                after_ms: 6_000,
            } => Some(*timer),
            _ => None,
        })
        .expect("the notice times out");
    let (state, _) = run(state, [Input::TimerFired(timer)]);
    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert_eq!(window.notice, None);
}

#[test]
fn hold_and_answer_without_a_waiting_call_is_refused() {
    let (_, effects) = run(on_a_call(), [Input::HoldAndAnswer, Input::EndAndAnswer]);

    assert_eq!(
        effects,
        [
            Effect::ReportError(Failure::NoWaitingCall),
            Effect::ReportError(Failure::NoWaitingCall)
        ]
    );
}

#[test]
fn swap_without_a_call_on_hold_is_refused() {
    let (state, effects) = run(waiting(), [Input::Swap]);

    assert_eq!(effects, [Effect::ReportError(Failure::NoHeldCall)]);
    assert_eq!(state.refusal(&Input::Swap), Some(Failure::NoHeldCall));
}

/// The Call a third caller makes while Alice is live and Bob on hold.
const THIRD: CallId = CallId(3);

/// [`held`], and Carol calls: her Call waits too.
fn three_calls() -> State {
    run(
        held(),
        [Input::CallAdded {
            call: THIRD,
            state: CallState::Waiting,
            caller: Caller::Number("+447700900789".to_owned()),
            at: Timestamp(5_000),
        }],
    )
    .0
}

#[test]
fn while_a_waiting_call_rings_the_held_call_offers_neither_swap_nor_hang_up() {
    let state = three_calls();

    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert_eq!(window.held.expect("Bob is listed on hold").call, FIRST);
    assert!(
        window.waiting_first,
        "AT+CHLD=0 would decline Carol, and =2 answer her"
    );
    assert_eq!(state.refusal(&Input::Swap), Some(Failure::WaitingCallFirst));
    assert_eq!(
        state.surfaces().banner.map(|banner| banner.call),
        Some(THIRD)
    );
}

#[test]
fn hanging_up_the_held_call_while_a_waiting_call_rings_is_refused_and_asks_nothing() {
    let (state, effects) = run(three_calls(), [Input::HangUp(FIRST)]);

    assert_eq!(effects, [Effect::ReportError(Failure::WaitingCallFirst)]);
    assert_eq!(
        state.surfaces().banner.map(|banner| banner.call),
        Some(THIRD)
    );
}

#[test]
fn once_the_waiting_call_is_gone_the_held_call_offers_swap_and_hang_up_again() {
    let (state, _) = run(
        three_calls(),
        [Input::Decline(THIRD), Input::CallEnded { call: THIRD }],
    );

    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert_eq!(window.held.expect("Bob is still on hold").call, FIRST);
    assert!(!window.waiting_first);
    assert_eq!(state.refusal(&Input::Swap), None);
    let (_, effects) = run(state, [Input::HangUp(FIRST)]);
    assert_eq!(effects, [Effect::Phone(PhoneCommand::HangUp(FIRST))]);
}

/// Connected, and the user is placing a Call that rings at the other end,
/// when Alice calls: hers is `waiting`.
fn waiting_while_placing() -> State {
    let connected = run(
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
    .0;
    run(
        connected,
        [
            Input::CallAdded {
                call: FIRST,
                state: CallState::Dialing,
                caller: bob(),
                at: Timestamp(1_000),
            },
            Input::CallChanged {
                call: FIRST,
                state: CallState::Alerting,
                at: Timestamp(1_500),
            },
            Input::CallAdded {
                call: SECOND,
                state: CallState::Waiting,
                caller: alice(),
                at: Timestamp(2_000),
            },
        ],
    )
    .0
}

#[test]
fn a_call_ringing_while_one_is_still_being_placed_waits_with_end_and_answer_only() {
    let state = waiting_while_placing();

    assert!(!state.ringing());
    let banner = state.surfaces().banner.expect("the Banner shows");
    assert_eq!(banner.call, SECOND);
    assert_eq!(banner.waiting, Some(WaitingOptions::End));
    assert_eq!(
        state.refusal(&Input::HoldAndAnswer),
        Some(Failure::NotConnectedYet)
    );
}

#[test]
fn end_and_answer_while_placing_hangs_that_call_up_then_answers_once_the_phone_rings() {
    let (state, effects) = run(waiting_while_placing(), [Input::EndAndAnswer]);

    assert_eq!(effects, [Effect::Phone(PhoneCommand::HangUp(FIRST))]);
    assert_eq!(state.surfaces().banner, None);

    let (_, effects) = run(
        state,
        [
            Input::CallEnded { call: FIRST },
            Input::CallChanged {
                call: SECOND,
                state: CallState::Ringing,
                at: Timestamp(3_000),
            },
        ],
    );
    assert!(
        effects.contains(&Effect::Phone(PhoneCommand::Answer(SECOND))),
        "{effects:?}"
    );
    assert!(!effects.contains(&Effect::StartRinging), "{effects:?}");
}

#[test]
fn answer_on_a_waiting_call_while_placing_is_end_and_answer() {
    let (_, effects) = run(waiting_while_placing(), [Input::Answer(SECOND)]);

    assert_eq!(effects, [Effect::Phone(PhoneCommand::HangUp(FIRST))]);
}

#[test]
fn if_the_phone_keeps_the_placed_call_the_waiting_call_is_offered_again() {
    let (state, _) = run(
        waiting_while_placing(),
        [
            Input::EndAndAnswer,
            Input::PhoneCommandFailed(PhoneCommand::HangUp(FIRST)),
        ],
    );

    let banner = state.surfaces().banner.expect("the Banner is back");
    assert_eq!(banner.waiting, Some(WaitingOptions::End));
}

#[test]
fn with_a_call_on_hold_too_the_waiting_call_offers_only_end_and_answer_and_decline() {
    let state = three_calls();

    let banner = state.surfaces().banner.expect("the Banner shows");
    assert_eq!(banner.call, THIRD);
    assert_eq!(banner.waiting, Some(WaitingOptions::End));
    assert_eq!(
        state.refusal(&Input::HoldAndAnswer),
        Some(Failure::AlreadyHolding)
    );
    let (_, effects) = run(state, [Input::HoldAndAnswer, Input::Answer(THIRD)]);
    assert_eq!(
        effects,
        [
            Effect::ReportError(Failure::AlreadyHolding),
            Effect::ReportError(Failure::AlreadyHolding)
        ],
        "neither holds Alice beside Bob, nor ends her unasked"
    );
}

#[test]
fn hanging_up_the_live_call_while_one_is_held_and_one_waits_is_refused_and_asks_nothing() {
    let (state, effects) = run(three_calls(), [Input::HangUp(SECOND)]);

    assert_eq!(
        effects,
        [Effect::ReportError(Failure::WaitingCallFirst)],
        "AT+CHLD=1 would answer Carol"
    );
    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert_eq!(window.call, Some(SECOND));
    assert!(window.waiting_first);
}

#[test]
fn end_and_answer_with_a_call_on_hold_ends_only_the_live_call() {
    let (state, effects) = run(three_calls(), [Input::EndAndAnswer]);

    assert_eq!(effects, [Effect::Phone(PhoneCommand::EndAndAnswer(THIRD))]);
    let (state, _) = run(
        state,
        [
            Input::CallEnded { call: SECOND },
            Input::CallChanged {
                call: THIRD,
                state: CallState::Active,
                at: Timestamp(6_000),
            },
        ],
    );
    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert_eq!(window.call, Some(THIRD));
    assert_eq!(window.held.map(|held| held.call), Some(FIRST));
}

/// [`held`], then Alice hangs up, leaving Bob on hold alone, and Carol
/// calls: her Call waits.
fn held_alone_while_one_waits() -> State {
    run(
        held(),
        [
            Input::CallEnded { call: SECOND },
            Input::CallAdded {
                call: THIRD,
                state: CallState::Waiting,
                caller: Caller::Number("+447700900789".to_owned()),
                at: Timestamp(5_000),
            },
        ],
    )
    .0
}

#[test]
fn with_only_a_held_call_the_waiting_call_offers_answer_and_decline() {
    let state = held_alone_while_one_waits();

    let banner = state.surfaces().banner.expect("the Banner shows");
    assert_eq!(banner.call, THIRD);
    assert_eq!(banner.waiting, Some(WaitingOptions::Answer));
    for input in [Input::HoldAndAnswer, Input::EndAndAnswer] {
        assert_eq!(
            state.refusal(&input),
            Some(Failure::NothingToHoldOrEnd),
            "{input:?}"
        );
    }
    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert_eq!(window.call, Some(FIRST));
    assert!(window.waiting_first, "Resume would answer Carol");
    let (_, effects) = run(state, [Input::Decline(THIRD)]);
    assert_eq!(effects, [Effect::Phone(PhoneCommand::HangUp(THIRD))]);
}

#[test]
fn answer_with_only_a_held_call_answers_the_waiting_call_and_keeps_the_held_one() {
    let (state, effects) = run(held_alone_while_one_waits(), [Input::Answer(THIRD)]);

    assert_eq!(
        effects.first(),
        Some(&Effect::Phone(PhoneCommand::AnswerBesideHeld(THIRD)))
    );
    assert_eq!(state.surfaces().banner, None);
    let (state, _) = run(
        state,
        [Input::CallChanged {
            call: THIRD,
            state: CallState::Active,
            at: Timestamp(6_000),
        }],
    );
    let window = state.surfaces().call_window.expect("the Call Window shows");
    assert_eq!(window.call, Some(THIRD));
    assert_eq!(window.held.map(|held| held.call), Some(FIRST));
}

#[test]
fn if_the_phone_refuses_answer_beside_a_held_call_the_waiting_call_is_offered_again() {
    let (state, effects) = run(
        held_alone_while_one_waits(),
        [
            Input::Answer(THIRD),
            Input::PhoneCommandFailed(PhoneCommand::AnswerBesideHeld(THIRD)),
        ],
    );

    assert!(
        effects.contains(&Effect::ReportError(Failure::AnswerFailed)),
        "{effects:?}"
    );
    let banner = state.surfaces().banner.expect("the Banner is back");
    assert_eq!(banner.waiting, Some(WaitingOptions::Answer));
}

/// On a Call with Bob, answered on the Phone, so its audio is there, and
/// Alice's Call waiting.
fn waiting_with_audio_on_the_phone() -> State {
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
            Input::CallAdded {
                call: FIRST,
                state: CallState::Active,
                caller: bob(),
                at: Timestamp(1_000),
            },
            Input::CallAdded {
                call: SECOND,
                state: CallState::Waiting,
                caller: alice(),
                at: Timestamp(3_000),
            },
        ],
    )
    .0
}

fn audio_moving(state: &State) -> Option<AudioLocation> {
    state
        .surfaces()
        .call_window
        .and_then(|window| window.audio)
        .and_then(|audio| audio.moving)
}

#[test]
fn a_waiting_call_answered_that_ends_unconnected_ends_the_handoff_its_answer_asked_for() {
    let (state, _) = run(waiting_with_audio_on_the_phone(), [Input::HoldAndAnswer]);
    assert_eq!(audio_moving(&state), Some(AudioLocation::Desktop));

    let (state, effects) = run(state, [Input::CallEnded { call: SECOND }]);

    assert_eq!(audio_moving(&state), None);
    assert!(
        effects.contains(&Effect::Phone(PhoneCommand::KeepAudioOnPhone)),
        "{effects:?}"
    );
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::ReportError(_))),
        "{effects:?}"
    );
}

#[test]
fn a_waiting_call_answered_that_ends_unconnected_leaves_the_users_own_move_audio_here() {
    let (state, _) = run(
        waiting_with_audio_on_the_phone(),
        [
            Input::MoveAudio(AudioLocation::Desktop),
            Input::HoldAndAnswer,
        ],
    );

    let (state, effects) = run(state, [Input::CallEnded { call: SECOND }]);

    assert_eq!(audio_moving(&state), Some(AudioLocation::Desktop));
    assert!(
        !effects.contains(&Effect::Phone(PhoneCommand::KeepAudioOnPhone)),
        "{effects:?}"
    );
}
