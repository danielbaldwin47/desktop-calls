//! What the Indicator shows and what its menu asks for, decided by the Call
//! state machine, at its public interface.

use desktop_calls_core::{
    AudioLocation, CallId, CallState, Caller, IndicatorEntry, IndicatorLook, IndicatorView, Input,
    Mute, Phone, PhoneCommand, State, SyncFailure, Timer, Timestamp, Transport, messages, step,
};

const ADDRESS: &str = "AA:BB:CC:DD:EE:FF";
const CALL: CallId = CallId(1);

fn run(state: State, inputs: impl IntoIterator<Item = Input>) -> State {
    inputs
        .into_iter()
        .fold(state, |state, input| step(state, input).0)
}

fn paired() -> State {
    run(
        State::default(),
        [Input::PhonePaired(Phone {
            name: "My Pixel".to_owned(),
            address: ADDRESS.to_owned(),
        })],
    )
}

fn connected() -> State {
    run(
        paired(),
        [
            Input::PhoneConnected {
                address: ADDRESS.to_owned(),
            },
            Input::PhoneCommandDone(PhoneCommand::KeepAudioOnPhone),
            Input::MicrophoneVolumeChanged(9),
        ],
    )
}

fn denied() -> State {
    run(
        connected(),
        [Input::ContactsSyncFailed(SyncFailure::Denied)],
    )
}

fn ringing(state: State) -> State {
    run(
        state,
        [Input::CallAdded {
            call: CALL,
            state: CallState::Ringing,
            caller: Caller::Number("+447700900123".to_owned()),
            at: Timestamp(1_000),
        }],
    )
}

fn live_on_desktop() -> State {
    run(
        ringing(connected()),
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
}

/// Out of range: hands-free drops, then BlueZ reports the Phone's
/// Bluetooth connection down.
fn out_of_range() -> State {
    run(
        connected(),
        [
            Input::PhoneDisconnected {
                address: ADDRESS.to_owned(),
            },
            Input::PhoneAbsent {
                address: ADDRESS.to_owned(),
            },
        ],
    )
}

fn indicator(state: &State) -> IndicatorView {
    state.indicator(&state.surfaces())
}

fn tooltip(state: &State) -> String {
    messages::indicator_tooltip(&indicator(state))
}

#[test]
fn each_state_has_its_own_look() {
    let looks = [
        connected(),
        out_of_range(),
        ringing(connected()),
        live_on_desktop(),
        denied(),
    ]
    .map(|state| indicator(&state).look);

    assert_eq!(
        looks,
        [
            IndicatorLook::Connected,
            IndicatorLook::Disconnected,
            IndicatorLook::Ringing,
            IndicatorLook::InCall,
            IndicatorLook::Problem,
        ]
    );
}

#[test]
fn the_tooltip_says_the_state_in_words() {
    assert_eq!(tooltip(&State::default()), "No Phone set up");
    assert_eq!(tooltip(&paired()), "My Pixel not connected");
    assert_eq!(tooltip(&connected()), "My Pixel connected");
    assert_eq!(tooltip(&out_of_range()), "My Pixel out of range");
    assert_eq!(tooltip(&ringing(connected())), "Call from +44 7700 900123");
    assert_eq!(
        tooltip(&live_on_desktop()),
        "In a Call with +44 7700 900123"
    );
    assert_eq!(
        tooltip(&denied()),
        "My Pixel connected · Contacts not shared"
    );
}

/// A Call that rang with no number, while the Phone may still report one.
fn ringing_with_no_number_yet() -> State {
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
fn while_the_caller_is_identified_the_tooltip_says_what_the_banner_says() {
    let identifying = ringing_with_no_number_yet();
    let withheld = run(
        identifying.clone(),
        [Input::TimerFired(Timer::NumberLate(CALL))],
    );

    assert_eq!(tooltip(&identifying), "Identifying caller…");
    assert_eq!(tooltip(&withheld), "Call from Withheld number");
}

#[test]
fn ringing_and_problems_ask_for_attention_and_a_ringing_call_shows_over_a_problem() {
    assert!(!indicator(&connected()).needs_attention);
    assert!(!indicator(&live_on_desktop()).needs_attention);
    assert!(indicator(&ringing(connected())).needs_attention);
    assert!(indicator(&denied()).needs_attention);

    let both = indicator(&ringing(denied()));
    assert_eq!(both.look, IndicatorLook::Ringing);
    assert!(both.needs_attention);
}

#[test]
fn during_a_call_the_menu_asks_for_what_the_call_window_buttons_do() {
    let view = indicator(&live_on_desktop());
    let intents: Vec<Option<Input>> = view.menu.iter().map(IndicatorEntry::intent).collect();

    assert_eq!(
        intents,
        [
            None,
            Some(Input::OpenDialer {
                query: String::new(),
                select_call: None,
            }),
            None,
            Some(Input::OpenQuietHoursWindow),
            Some(Input::OpenSetupWindow),
            None,
            Some(Input::HangUp(CALL)),
            Some(Input::SetMuted(true)),
            Some(Input::MoveAudio(AudioLocation::Phone)),
        ]
    );
    let labels: Vec<String> = view
        .menu
        .iter()
        .map(|entry| messages::indicator_entry(entry, &view))
        .collect();
    assert_eq!(
        labels,
        [
            "In a Call with +44 7700 900123",
            "Open Dialer",
            "Turn calls off",
            "Quiet Hours…",
            "Connect your phone…",
            "",
            "Hang up",
            "Mute",
            "Move audio to phone",
        ]
    );
}

#[test]
fn muted_the_mute_entry_unmutes() {
    let view = indicator(&run(
        live_on_desktop(),
        [
            Input::SetMuted(true),
            Input::PhoneCommandDone(PhoneCommand::SetMicrophoneVolume(0)),
        ],
    ));

    assert!(
        view.menu.contains(&IndicatorEntry::Mute(Mute::On)),
        "{view:?}"
    );
    assert_eq!(
        IndicatorEntry::Mute(Mute::On).intent(),
        Some(Input::SetMuted(false))
    );
}

#[test]
fn a_problem_is_in_the_menu() {
    let view = indicator(&denied());

    assert_eq!(
        view.menu.last(),
        Some(&IndicatorEntry::Problem(
            messages::ProblemKind::ContactsDenied
        ))
    );
    assert_eq!(view.menu.last().and_then(IndicatorEntry::intent), None);
}
