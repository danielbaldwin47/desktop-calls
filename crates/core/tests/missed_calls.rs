//! Missed Calls in the Call state machine, at its public interface: which
//! Calls are missed, how the Phone's Call History confirms them, the
//! notification that tells of them, and what its actions do.

use desktop_calls_core::matching::DialerRowKind;
use desktop_calls_core::{
    CallHistory, CallHistoryEntry, CallHistoryKind, CallId, CallState, Caller, Contact,
    ContactMatch, ContactNumber, Contacts, Directory, Effect, Input, MissedCallNotice, NumberLabel,
    Phone, PhoneCommand, RingingCaller, State, SyncFailure, Timestamp, Unavailable,
    messages::Failure, step,
};

const ADDRESS: &str = "AA:BB:CC:DD:EE:FF";
const ALICE: &str = "+447700900123";
const BOB: &str = "+447700900456";

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

fn number(number: &str) -> Caller {
    Caller::Number(number.to_owned())
}

fn directory() -> Directory {
    Directory::new(
        vec![Contact {
            name: "Alice Smith".to_owned(),
            numbers: vec![ContactNumber {
                number: ALICE.to_owned(),
                label: NumberLabel::Mobile,
            }],
        }],
        Some("GB"),
    )
}

fn alice() -> ContactMatch {
    ContactMatch {
        name: "Alice Smith".to_owned(),
        label: NumberLabel::Mobile,
    }
}

fn entry(kind: CallHistoryKind, caller: Caller, at: u64) -> CallHistoryEntry {
    CallHistoryEntry {
        kind,
        caller,
        at: Some(Timestamp(at)),
        name: String::new(),
    }
}

/// An older Call, already in the Call History when the Desktop synced.
fn older() -> CallHistoryEntry {
    entry(CallHistoryKind::Outgoing, number(ALICE), 1_000)
}

/// The Call History after a refresh: `newest`, newest first, then what was
/// synced.
fn history(newest: Vec<CallHistoryEntry>) -> CallHistory {
    let mut entries = newest;
    entries.push(older());
    CallHistory::new(entries, &directory())
}

/// Connected, with Contacts and Call History synced.
fn synced() -> State {
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
            Input::ContactsSynced {
                contacts: Contacts::new(directory()),
                call_history: history(Vec::new()),
            },
        ],
    )
    .0
}

/// `call` rings from `caller` at `at`, then stops ringing unanswered.
fn rings_out(state: State, call: u32, caller: Caller, at: u64) -> (State, Vec<Effect>) {
    run(
        state,
        [
            Input::CallAdded {
                call: CallId(call),
                state: CallState::Ringing,
                caller,
                at: Timestamp(at),
            },
            Input::CallEnded { call: CallId(call) },
        ],
    )
}

fn notices(effects: &[Effect]) -> Vec<&MissedCallNotice> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::NotifyMissedCall(notice) => Some(notice),
            _ => None,
        })
        .collect()
}

#[test]
fn a_call_that_rings_out_and_the_phone_lists_as_missed_is_notified() {
    let (state, effects) = rings_out(synced(), 1, number(ALICE), 5_000);
    assert!(effects.contains(&Effect::RefreshCallHistory));
    assert_eq!(notices(&effects), Vec::<&MissedCallNotice>::new());

    let (_, effects) = run(
        state,
        [Input::CallHistoryRefreshed(history(vec![entry(
            CallHistoryKind::Missed,
            number(ALICE),
            5_000,
        )]))],
    );

    assert_eq!(
        notices(&effects),
        [&MissedCallNotice {
            notification: CallId(1),
            call: CallId(1),
            caller: number(ALICE),
            contact: Some(alice()),
            count: 1,
            at: Timestamp(5_000),
        }]
    );
}

fn missed(caller: Caller, at: u64) -> CallHistoryEntry {
    entry(CallHistoryKind::Missed, caller, at)
}

/// `call` rings out from Alice and the Phone lists it as missed.
fn missed_from_alice(state: State, call: u32, at: u64) -> (State, Vec<Effect>) {
    let before: Vec<CallHistoryEntry> = state.call_history().entries().to_vec();
    let (state, _) = rings_out(state, call, number(ALICE), at);
    let mut entries = vec![missed(number(ALICE), at)];
    entries.extend(before);
    run(
        state,
        [Input::CallHistoryRefreshed(CallHistory::new(
            entries,
            &directory(),
        ))],
    )
}

fn denied() -> State {
    run(synced(), [Input::ContactsSyncFailed(SyncFailure::Denied)]).0
}

fn calls_of(notices: &[&MissedCallNotice]) -> Vec<CallId> {
    notices.iter().map(|notice| notice.call).collect()
}

#[test]
fn a_call_declined_on_the_phone_is_not_in_its_missed_calls_and_is_not_notified() {
    let (state, _) = rings_out(synced(), 1, number(ALICE), 5_000);

    let (_, effects) = run(
        state,
        [Input::CallHistoryRefreshed(history(vec![entry(
            CallHistoryKind::Incoming,
            number(ALICE),
            5_000,
        )]))],
    );

    assert_eq!(notices(&effects), Vec::<&MissedCallNotice>::new());
}

#[test]
fn an_older_missed_call_from_the_same_caller_does_not_confirm_a_new_one() {
    let (state, _) = missed_from_alice(synced(), 1, 5_000);
    let (state, _) = rings_out(state, 2, number(ALICE), 9_000);

    let unchanged = state.call_history().clone();
    let (_, effects) = run(state, [Input::CallHistoryRefreshed(unchanged)]);

    assert_eq!(notices(&effects), Vec::<&MissedCallNotice>::new());
}

#[test]
fn a_call_declined_on_the_desktop_is_never_missed() {
    let (_, effects) = run(
        synced(),
        [
            Input::CallAdded {
                call: CallId(1),
                state: CallState::Ringing,
                caller: number(ALICE),
                at: Timestamp(5_000),
            },
            Input::Decline(CallId(1)),
            Input::CallEnded { call: CallId(1) },
            // Some Phones list a Call declined from a hands-free device as
            // missed.
            Input::CallHistoryRefreshed(history(vec![missed(number(ALICE), 5_000)])),
        ],
    );

    assert_eq!(notices(&effects), Vec::<&MissedCallNotice>::new());
}

#[test]
fn a_call_answered_on_the_phone_is_never_missed() {
    let (_, effects) = run(
        synced(),
        [
            Input::CallAdded {
                call: CallId(1),
                state: CallState::Ringing,
                caller: number(ALICE),
                at: Timestamp(5_000),
            },
            Input::CallChanged {
                call: CallId(1),
                state: CallState::Active,
                at: Timestamp(6_000),
            },
            Input::CallEnded { call: CallId(1) },
            Input::CallHistoryRefreshed(history(vec![missed(number(ALICE), 5_000)])),
        ],
    );

    assert_eq!(notices(&effects), Vec::<&MissedCallNotice>::new());
}

#[test]
fn if_the_call_history_refresh_fails_a_call_that_rang_out_is_missed() {
    let (state, _) = rings_out(synced(), 1, number(ALICE), 5_000);

    let (_, effects) = run(
        state,
        [
            Input::ContactsSyncFailed(SyncFailure::Unavailable(Unavailable::Failed)),
            Input::CallHistoryRefreshFailed,
        ],
    );

    assert_eq!(calls_of(&notices(&effects)), [CallId(1)]);
}

#[test]
fn with_contacts_denied_a_call_that_rang_out_is_missed_at_once() {
    let (_, effects) = rings_out(denied(), 1, number(ALICE), 5_000);

    assert!(!effects.contains(&Effect::RefreshCallHistory));
    assert_eq!(
        notices(&effects),
        [&MissedCallNotice {
            notification: CallId(1),
            call: CallId(1),
            caller: number(ALICE),
            // Denied keeps nothing of the person's Contacts.
            contact: None,
            count: 1,
            at: Timestamp(5_000),
        }]
    );
}

#[test]
fn a_sync_that_lands_before_the_refresh_does_not_decide() {
    let (state, _) = rings_out(synced(), 1, number(ALICE), 5_000);

    let (state, effects) = run(
        state,
        [Input::ContactsSynced {
            contacts: Contacts::new(directory()),
            call_history: history(Vec::new()),
        }],
    );
    assert_eq!(notices(&effects), Vec::<&MissedCallNotice>::new());

    let (_, effects) = run(
        state,
        [Input::CallHistoryRefreshed(history(vec![missed(
            number(ALICE),
            5_000,
        )]))],
    );
    assert_eq!(calls_of(&notices(&effects)), [CallId(1)]);
}

#[test]
fn each_call_waits_for_the_refresh_asked_for_when_it_ended() {
    let (state, _) = rings_out(synced(), 1, number(ALICE), 5_000);
    let (state, _) = rings_out(state, 2, number(BOB), 6_000);

    // The first refresh ran before Bob's Call was in the Phone's Call
    // History.
    let (state, effects) = run(
        state,
        [Input::CallHistoryRefreshed(history(vec![missed(
            number(ALICE),
            5_000,
        )]))],
    );
    assert_eq!(calls_of(&notices(&effects)), [CallId(1)]);

    let (_, effects) = run(
        state,
        [Input::CallHistoryRefreshed(history(vec![
            missed(number(BOB), 6_000),
            missed(number(ALICE), 5_000),
        ]))],
    );
    assert_eq!(calls_of(&notices(&effects)), [CallId(2)]);
}

#[test]
fn a_waiting_call_nobody_answers_is_missed() {
    let (_, effects) = run(
        synced(),
        [
            Input::CallAdded {
                call: CallId(1),
                state: CallState::Active,
                caller: number(BOB),
                at: Timestamp(4_000),
            },
            Input::CallAdded {
                call: CallId(2),
                state: CallState::Waiting,
                caller: number(ALICE),
                at: Timestamp(5_000),
            },
            Input::CallEnded { call: CallId(2) },
            Input::CallHistoryRefreshed(history(vec![missed(number(ALICE), 5_000)])),
        ],
    );

    assert_eq!(calls_of(&notices(&effects)), [CallId(2)]);
}

#[test]
fn three_missed_calls_from_one_caller_update_one_notification() {
    let (state, _) = missed_from_alice(synced(), 1, 5_000);
    let (state, _) = missed_from_alice(state, 2, 6_000);
    let (_, effects) = missed_from_alice(state, 3, 7_000);

    assert_eq!(
        notices(&effects),
        [&MissedCallNotice {
            notification: CallId(1),
            call: CallId(3),
            caller: number(ALICE),
            contact: Some(alice()),
            count: 3,
            at: Timestamp(7_000),
        }]
    );
}

#[test]
fn missed_calls_from_different_callers_have_a_notification_each() {
    let (state, _) = missed_from_alice(synced(), 1, 5_000);
    let (state, _) = rings_out(state, 2, number(BOB), 6_000);

    let (_, effects) = run(
        state,
        [Input::CallHistoryRefreshed(history(vec![
            missed(number(BOB), 6_000),
            missed(number(ALICE), 5_000),
        ]))],
    );

    assert_eq!(notices(&effects).len(), 1);
    assert_eq!(notices(&effects)[0].notification, CallId(2));
    assert_eq!(notices(&effects)[0].count, 1);
}

#[test]
fn a_withheld_call_that_rang_out_is_notified_when_the_phone_lists_a_withheld_missed_call() {
    let (state, _) = rings_out(synced(), 1, Caller::Withheld, 5_000);

    let (_, effects) = run(
        state,
        [Input::CallHistoryRefreshed(history(vec![missed(
            Caller::Withheld,
            5_000,
        )]))],
    );

    assert_eq!(calls_of(&notices(&effects)), [CallId(1)]);
    assert_eq!(notices(&effects)[0].caller, Caller::Withheld);
    assert_eq!(notices(&effects)[0].contact, None);
}

#[test]
fn clicking_the_notification_opens_the_dialer_with_that_call_selected_and_enter_calls_back() {
    let (state, _) = missed_from_alice(synced(), 1, 5_000);

    let (state, _) = run(state, [Input::OpenMissedCall(CallId(1))]);

    let dialer = state.surfaces().dialer.expect("the Dialer is open");
    let selected = dialer.selected.expect("a row is selected");
    assert_eq!(dialer.rows[selected].caller, number(ALICE));
    assert_eq!(
        dialer.rows[selected].kind,
        DialerRowKind::CallHistory(CallHistoryKind::Missed)
    );
    let (_, effects) = run(state, [Input::DialerEnter]);
    assert!(effects.contains(&Effect::Phone(PhoneCommand::Dial(ALICE.to_owned()))));
}

#[test]
fn clicking_the_notification_selects_the_caller_even_without_a_call_history() {
    let (state, _) = rings_out(denied(), 1, number(ALICE), 5_000);

    let (state, _) = run(state, [Input::OpenMissedCall(CallId(1))]);
    let (_, effects) = run(state, [Input::DialerEnter]);

    assert!(effects.contains(&Effect::Phone(PhoneCommand::Dial(ALICE.to_owned()))));
}

#[test]
fn clicking_a_withheld_notification_selects_its_row_and_enter_explains() {
    let (state, _) = rings_out(synced(), 1, Caller::Withheld, 5_000);
    let (state, _) = run(
        state,
        [
            Input::CallHistoryRefreshed(history(vec![
                missed(number(BOB), 6_000),
                missed(Caller::Withheld, 5_000),
            ])),
            Input::OpenMissedCall(CallId(1)),
        ],
    );

    let (state, effects) = run(state, [Input::DialerEnter]);

    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::Phone(PhoneCommand::Dial(_))))
    );
    assert_eq!(
        state.surfaces().dialer.and_then(|dialer| dialer.notice),
        Some(Failure::WithheldNumber)
    );
}

#[test]
fn call_back_places_the_call() {
    let (state, _) = missed_from_alice(synced(), 1, 5_000);

    let (_, effects) = run(state, [Input::CallBack(CallId(1))]);

    assert!(effects.contains(&Effect::Phone(PhoneCommand::Dial(ALICE.to_owned()))));
}

#[test]
fn once_the_user_has_acted_on_a_notification_the_next_missed_call_starts_a_new_one() {
    for acted in [
        Input::OpenMissedCall(CallId(1)),
        Input::MissedCallDismissed(CallId(1)),
    ] {
        let (state, _) = missed_from_alice(synced(), 1, 5_000);
        let (state, _) = run(state, [acted, Input::CloseDialer]);

        let (_, effects) = missed_from_alice(state, 2, 6_000);

        assert_eq!(notices(&effects)[0].notification, CallId(2));
        assert_eq!(notices(&effects)[0].count, 1);
    }
}

/// Alice's Call rings, reported first with no number, then with hers, as
/// a Phone without `+CLCC` reports it.
fn rings_and_names_alice_late(state: State) -> State {
    run(
        state,
        [
            Input::CallAdded {
                call: CallId(1),
                state: CallState::Ringing,
                caller: Caller::Withheld,
                at: Timestamp(5_000),
            },
            Input::CallerChanged {
                call: CallId(1),
                caller: number(ALICE),
            },
        ],
    )
    .0
}

#[test]
fn the_banner_names_a_caller_the_phone_reports_late() {
    let state = rings_and_names_alice_late(synced());

    let banner = state.surfaces().banner.expect("the Banner shows");
    assert_eq!(
        banner.caller,
        RingingCaller::Known {
            caller: number(ALICE),
            contact: Some(alice()),
        }
    );
}

#[test]
fn a_missed_call_whose_caller_came_late_names_them_and_opens_on_them() {
    let state = rings_and_names_alice_late(synced());

    let (state, effects) = run(
        state,
        [
            Input::CallEnded { call: CallId(1) },
            Input::CallHistoryRefreshed(history(vec![missed(number(ALICE), 5_000)])),
        ],
    );
    assert_eq!(
        notices(&effects)
            .iter()
            .map(|notice| (&notice.caller, &notice.contact))
            .collect::<Vec<_>>(),
        [(&number(ALICE), &Some(alice()))]
    );

    let (state, _) = run(state, [Input::OpenMissedCall(CallId(1))]);
    let dialer = state.surfaces().dialer.expect("the Dialer is open");
    let selected = dialer.selected.expect("a row is selected");
    assert_eq!(dialer.rows[selected].caller, number(ALICE));
}

#[test]
fn a_caller_reported_for_a_call_already_over_changes_nothing() {
    let (state, _) = rings_out(synced(), 1, Caller::Withheld, 5_000);

    let (after, effects) = run(
        state.clone(),
        [Input::CallerChanged {
            call: CallId(1),
            caller: number(ALICE),
        }],
    );

    assert_eq!((after, effects), (state, Vec::new()));
}
