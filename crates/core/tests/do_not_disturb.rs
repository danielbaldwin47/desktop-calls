//! The Call state machine's record of the Desktop's Do Not Disturb, at its
//! public interface, and its rule: under Do Not Disturb the Incoming Call
//! Banner still shows, but the Desktop doesn't ring.

use desktop_calls_core::{CallId, CallState, Caller, Effect, Input, Phone, State, Timestamp, step};

const ADDRESS: &str = "AA:BB:CC:DD:EE:FF";
const CALL: CallId = CallId(1);

fn pixel() -> Phone {
    Phone {
        name: "My Pixel".to_owned(),
        address: ADDRESS.to_owned(),
    }
}

/// Feeds `inputs` to a fresh state machine and returns where it ends up.
fn run(inputs: impl IntoIterator<Item = Input>) -> State {
    run_from(State::default(), inputs).0
}

/// Feeds `inputs` to `state` and returns where it ends up, with every effect
/// it asked for along the way.
fn run_from(state: State, inputs: impl IntoIterator<Item = Input>) -> (State, Vec<Effect>) {
    inputs
        .into_iter()
        .fold((state, Vec::new()), |(state, mut effects), input| {
            let (next, mut more) = step(state, input);
            effects.append(&mut more);
            (next, effects)
        })
}

fn connected() -> State {
    run([
        Input::PhonePaired(pixel()),
        Input::PhoneConnected {
            address: ADDRESS.to_owned(),
        },
    ])
}

fn call_rings() -> Input {
    Input::CallAdded {
        call: CALL,
        state: CallState::Ringing,
        caller: Caller::Number("+447700900123".to_owned()),
        at: Timestamp(1_000),
    }
}

/// Whether the Banner shows, whether the Desktop rings, and every ringtone
/// effect among `effects`.
fn banner_and_ringtone(state: &State, effects: &[Effect]) -> (bool, bool, Vec<Effect>) {
    let ringtone = effects
        .iter()
        .filter(|effect| matches!(effect, Effect::StartRinging | Effect::StopRinging))
        .cloned()
        .collect();
    (state.surfaces().banner.is_some(), state.ringing(), ringtone)
}

#[test]
fn do_not_disturb_is_off_until_the_desktop_says_otherwise() {
    assert!(!run([]).do_not_disturb());
    assert!(run([Input::DoNotDisturbChanged(true)]).do_not_disturb());
    assert!(
        !run([
            Input::DoNotDisturbChanged(true),
            Input::DoNotDisturbChanged(false)
        ])
        .do_not_disturb()
    );
}

#[test]
fn do_not_disturb_stays_on_when_the_phone_is_unpaired() {
    let state = run([
        Input::PhonePaired(pixel()),
        Input::DoNotDisturbChanged(true),
        Input::PhoneUnpaired {
            address: ADDRESS.to_owned(),
        },
    ]);

    assert_eq!(state.phone(), None);
    assert!(state.do_not_disturb());
}

#[test]
fn under_do_not_disturb_a_call_shows_the_banner_without_the_ringtone() {
    let (state, effects) = run_from(
        connected(),
        [Input::DoNotDisturbChanged(true), call_rings()],
    );

    assert_eq!(banner_and_ringtone(&state, &effects), (true, false, vec![]));
}

#[test]
fn do_not_disturb_turned_on_mid_ring_stops_the_ringtone_and_keeps_the_banner() {
    let (ringing, _) = run_from(connected(), [call_rings()]);
    assert!(ringing.ringing());

    let (state, effects) = run_from(ringing, [Input::DoNotDisturbChanged(true)]);

    assert_eq!(
        banner_and_ringtone(&state, &effects),
        (true, false, vec![Effect::StopRinging])
    );
}

#[test]
fn do_not_disturb_turned_off_mid_ring_starts_the_ringtone() {
    let (silent, _) = run_from(
        connected(),
        [Input::DoNotDisturbChanged(true), call_rings()],
    );

    let (state, effects) = run_from(silent, [Input::DoNotDisturbChanged(false)]);

    assert_eq!(
        banner_and_ringtone(&state, &effects),
        (true, true, vec![Effect::StartRinging])
    );
}

#[test]
fn a_call_answered_under_do_not_disturb_never_rings_even_once_it_is_turned_off() {
    let (state, effects) = run_from(
        connected(),
        [
            Input::DoNotDisturbChanged(true),
            call_rings(),
            Input::Answer(CALL),
            Input::DoNotDisturbChanged(false),
        ],
    );

    assert_eq!(
        banner_and_ringtone(&state, &effects),
        (false, false, vec![])
    );
}
