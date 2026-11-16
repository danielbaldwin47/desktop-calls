//! Asking a Phone that refused to share Contacts again, at the Call state
//! machine's public interface (ticket #394): while it stays connected, so
//! that sharing turned on later in its Bluetooth settings is picked up; not
//! during a Call; and not again once asking made it show its prompt.

use desktop_calls_core::{
    CallHistory, CallId, CallState, Caller, Contacts, ContactsState, Effect, Input, Phone,
    PhoneCommand, State, SyncFailure, SyncProgress, Timer, Timestamp, Unavailable, step,
};

const ADDRESS: &str = "AA:BB:CC:DD:EE:FF";

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

/// The waits to ask again that `effects` start: each one's timer and how
/// long it is.
fn waits(effects: &[Effect]) -> Vec<(Timer, u32)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::StartTimer {
                timer: timer @ Timer::AskForContactsAgain(_),
                after_ms,
            } => Some((*timer, *after_ms)),
            _ => None,
        })
        .collect()
}

fn asks_again(effects: &[Effect]) -> bool {
    effects.contains(&Effect::SyncContactsAgain)
}

/// Refused, and the wait that follows.
fn refused() -> (State, Timer) {
    let (state, effects) = run(
        connected(),
        [Input::ContactsSyncFailed(SyncFailure::Denied)],
    );
    let started = waits(&effects);
    assert_eq!(started.len(), 1, "{effects:?}");
    (state, started[0].0)
}

#[test]
fn a_refusal_while_connected_asks_again_after_10_s() {
    let (state, effects) = run(
        connected(),
        [Input::ContactsSyncFailed(SyncFailure::Denied)],
    );
    assert_eq!(state.contacts_state(), ContactsState::Denied);
    let started = waits(&effects);
    assert!(matches!(started.as_slice(), [(_, 10_000)]), "{started:?}");

    let (state, effects) = run(state, [Input::TimerFired(started[0].0)]);
    assert!(asks_again(&effects), "{effects:?}");
    // Still refused while it asks: the Problem stays.
    assert_eq!(state.contacts_state(), ContactsState::Denied);
}

#[test]
fn sharing_turned_on_since_syncs_and_asks_no_more() {
    let (state, wait) = refused();
    let (state, _) = run(state, [Input::TimerFired(wait)]);

    let (state, effects) = run(
        state,
        [Input::ContactsSynced {
            contacts: Contacts::default(),
            call_history: CallHistory::default(),
        }],
    );

    assert_eq!(state.contacts_state(), ContactsState::Ready);
    assert!(waits(&effects).is_empty(), "{effects:?}");
    assert!(state.problems().is_empty(), "{:?}", state.problems());
}

#[test]
fn still_refused_it_asks_again_later() {
    let (state, wait) = refused();
    let (state, _) = run(state, [Input::TimerFired(wait)]);

    let (state, effects) = run(state, [Input::ContactsSyncFailed(SyncFailure::Denied)]);

    assert_eq!(state.contacts_state(), ContactsState::Denied);
    assert_eq!(waits(&effects).len(), 1, "{effects:?}");
}

#[test]
fn after_10_minutes_of_asking_it_asks_every_5_minutes() {
    let (mut state, mut wait) = refused();
    for _ in 0..60 {
        let (asked, _) = run(state, [Input::TimerFired(wait)]);
        let (next, effects) = run(asked, [Input::ContactsSyncFailed(SyncFailure::Denied)]);
        let started = waits(&effects);
        state = next;
        wait = started[0].0;
        if started[0].1 == 300_000 {
            return;
        }
        assert_eq!(started[0].1, 10_000);
    }
    panic!("still asking every 10 s after 60 asks");
}

#[test]
fn a_phone_that_prompts_when_asked_again_isnt_asked_again_while_connected() {
    let (state, wait) = refused();
    let (state, _) = run(state, [Input::TimerFired(wait)]);

    let (state, effects) = run(
        state,
        [
            Input::ContactsPermissionNeeded {
                deadline: Timestamp(30_000),
            },
            Input::ContactsSyncFailed(SyncFailure::Denied),
        ],
    );

    assert_eq!(state.contacts_state(), ContactsState::Denied);
    assert!(waits(&effects).is_empty(), "{effects:?}");
}

/// Android remembers a no and refuses with no prompt, so asking again
/// shows nothing; a Phone that does prompt again stops it (above).
#[test]
fn a_no_on_the_prompt_is_asked_again_in_case_sharing_is_turned_on_later() {
    let (state, effects) = run(
        connected(),
        [
            Input::ContactsPermissionNeeded {
                deadline: Timestamp(30_000),
            },
            Input::ContactsSyncFailed(SyncFailure::Denied),
        ],
    );

    assert_eq!(state.contacts_state(), ContactsState::Denied);
    assert_eq!(waits(&effects).len(), 1, "{effects:?}");
}

#[test]
fn a_sync_that_fails_after_the_phone_shared_is_unavailable_and_not_asked_again() {
    let (state, wait) = refused();
    let (state, _) = run(state, [Input::TimerFired(wait)]);

    let (state, effects) = run(
        state,
        [
            Input::ContactsSyncProgress(SyncProgress::Shared),
            Input::ContactsSyncFailed(SyncFailure::Unavailable(Unavailable::Failed)),
        ],
    );

    assert_eq!(
        state.contacts_state(),
        ContactsState::Unavailable(Unavailable::Failed)
    );
    assert!(state.problems().is_empty(), "{:?}", state.problems());
    assert!(waits(&effects).is_empty(), "{effects:?}");
}

#[test]
fn still_refused_the_contacts_already_forgotten_arent_forgotten_again() {
    let (state, wait) = refused();
    let (state, _) = run(state, [Input::TimerFired(wait)]);

    let (_, effects) = run(state, [Input::ContactsSyncFailed(SyncFailure::Denied)]);

    assert!(!effects.contains(&Effect::ForgetContacts), "{effects:?}");
}

#[test]
fn the_link_closing_after_the_phone_left_doesnt_forget_its_contacts() {
    let (state, _) = run(
        connected(),
        [Input::ContactsSynced {
            contacts: Contacts::default(),
            call_history: CallHistory::default(),
        }],
    );
    let (state, effects) = run(
        state,
        [
            Input::PhoneDisconnected {
                address: ADDRESS.to_owned(),
            },
            Input::ContactsSyncFailed(SyncFailure::Denied),
        ],
    );

    assert!(!effects.contains(&Effect::ForgetContacts), "{effects:?}");
    assert_ne!(state.contacts_state(), ContactsState::Denied);
}

#[test]
fn a_call_puts_asking_again_off_until_it_ends() {
    let (state, wait) = refused();
    let (state, _) = run(
        state,
        [Input::CallAdded {
            call: CallId(1),
            state: CallState::Active,
            caller: Caller::Withheld,
            at: Timestamp(1_000),
        }],
    );

    let (_, effects) = run(state, [Input::TimerFired(wait)]);

    assert!(!asks_again(&effects), "{effects:?}");
    assert_eq!(waits(&effects).len(), 1, "{effects:?}");
}

#[test]
fn a_phone_that_left_isnt_asked_again_until_it_connects() {
    let (state, wait) = refused();
    let (state, _) = run(
        state,
        [Input::PhoneDisconnected {
            address: ADDRESS.to_owned(),
        }],
    );

    let (state, effects) = run(state, [Input::TimerFired(wait)]);
    assert!(!asks_again(&effects), "{effects:?}");
    assert!(waits(&effects).is_empty(), "{effects:?}");

    // Back: every connect syncs afresh.
    let (_, effects) = run(
        state,
        [Input::PhoneConnected {
            address: ADDRESS.to_owned(),
        }],
    );
    assert!(effects.contains(&Effect::SyncContacts), "{effects:?}");
}
