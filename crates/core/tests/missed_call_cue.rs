//! The lasting Missed Call cue (#236), at the Call state machine's public
//! interface: Missed Calls counted until the Dialer opens, on the
//! Indicator's look, tooltip and menu, marked at the top of the Dialer's
//! Call History, and kept for the next run.

use desktop_calls_core::matching::DialerRowKind;
use desktop_calls_core::{
    CallHistory, CallHistoryEntry, CallHistoryKind, CallId, CallState, Caller, Contact,
    ContactNumber, Contacts, Directory, Effect, IndicatorEntry, IndicatorLook, Input, NumberLabel,
    Phone, PhoneCommand, State, Timestamp, messages, step,
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

fn entry(kind: CallHistoryKind, caller: &str, at: u64) -> CallHistoryEntry {
    CallHistoryEntry {
        kind,
        caller: number(caller),
        at: Some(Timestamp(at)),
        name: String::new(),
    }
}

/// Connected, with Contacts and a Call History of one older Missed Call
/// and one older outgoing Call synced.
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
                call_history: CallHistory::new(older(), &directory()),
            },
        ],
    )
    .0
}

fn older() -> Vec<CallHistoryEntry> {
    vec![
        entry(CallHistoryKind::Missed, BOB, 2_000),
        entry(CallHistoryKind::Outgoing, ALICE, 1_000),
    ]
}

/// `call` rings out from `caller` at `at`, and the Phone's Call History,
/// refreshed, lists it as missed, `then` any newer entries.
fn missed(state: State, call: u32, caller: &str, at: u64) -> (State, Vec<Effect>) {
    let mut entries = vec![entry(CallHistoryKind::Missed, caller, at)];
    entries.extend(state.call_history().entries().iter().cloned());
    run(
        state,
        [
            Input::CallAdded {
                call: CallId(call),
                state: CallState::Ringing,
                caller: number(caller),
                at: Timestamp(at),
            },
            Input::CallEnded {
                call: CallId(call),
                at: Timestamp(at + 20_000),
            },
            Input::CallHistoryRefreshed(CallHistory::new(entries, &directory())),
        ],
    )
}

fn kept(effects: &[Effect]) -> Vec<u32> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::KeepMissedCalls(count) => Some(*count),
            _ => None,
        })
        .collect()
}

fn open_dialer() -> Input {
    Input::OpenDialer {
        query: String::new(),
        select_call: None,
    }
}

#[test]
fn each_missed_call_counts_on_the_indicator_and_is_kept_for_the_next_run() {
    let (state, effects) = missed(synced(), 1, ALICE, 10_000);
    assert_eq!(kept(&effects), [1]);
    let view = state.indicator(&state.surfaces());
    assert_eq!(view.look, IndicatorLook::ConnectedMissedCalls);
    assert_eq!(view.menu[1], IndicatorEntry::MissedCalls(1));
    assert_eq!(
        view.menu[1].intent(),
        Some(open_dialer()),
        "the line opens the Dialer"
    );
    assert_eq!(
        messages::indicator_entry(&view.menu[1], &view),
        "1 missed call…"
    );
    assert_eq!(
        messages::indicator_tooltip(&view),
        "My Pixel connected · 1 missed call"
    );

    let (state, effects) = missed(state, 2, BOB, 30_000);
    assert_eq!(kept(&effects), [2]);
    let view = state.indicator(&state.surfaces());
    assert_eq!(
        messages::indicator_entry(&view.menu[1], &view),
        "2 missed calls…"
    );
}

#[test]
fn opening_the_dialer_marks_the_missed_calls_at_the_top_and_clears_the_cue() {
    let (state, _) = missed(synced(), 1, ALICE, 10_000);
    // A Call answered since, newer in the Call History than the Missed
    // Call.
    let mut entries = vec![entry(CallHistoryKind::Incoming, BOB, 40_000)];
    entries.extend(state.call_history().entries().iter().cloned());
    let (state, _) = run(
        state,
        [Input::CallHistoryRefreshed(CallHistory::new(
            entries,
            &directory(),
        ))],
    );

    let (state, effects) = run(state, [open_dialer()]);

    assert_eq!(kept(&effects), [0]);
    let dialer = state.surfaces().dialer.unwrap_or_else(|| panic!("open"));
    assert_eq!(
        dialer.new_missed, 1,
        "only the Missed Call since: {dialer:?}"
    );
    assert_eq!(
        dialer.rows[0].kind,
        DialerRowKind::CallHistory(CallHistoryKind::Missed)
    );
    assert_eq!(dialer.rows[0].caller, number(ALICE));
    assert_eq!(
        dialer.rows[1].kind,
        DialerRowKind::CallHistory(CallHistoryKind::Incoming),
        "the rest in their order"
    );
    let view = state.indicator(&state.surfaces());
    assert_eq!(view.look, IndicatorLook::Connected);
    assert!(
        !view
            .menu
            .iter()
            .any(|entry| matches!(entry, IndicatorEntry::MissedCalls(_)))
    );

    // Opened again, it marks nothing: the user has seen them.
    let (state, _) = run(state, [Input::CloseDialer, open_dialer()]);
    let dialer = state.surfaces().dialer.unwrap_or_else(|| panic!("open"));
    assert_eq!(dialer.new_missed, 0);
}

#[test]
fn typing_in_the_dialer_drops_the_marks_and_clearing_it_brings_them_back() {
    let (state, _) = missed(synced(), 1, ALICE, 10_000);
    let (state, _) = run(state, [open_dialer(), Input::DialerQuery("al".to_owned())]);
    let dialer = state.surfaces().dialer.unwrap_or_else(|| panic!("open"));
    assert_eq!(dialer.new_missed, 0);

    let (state, _) = run(state, [Input::DialerQuery(String::new())]);
    let dialer = state.surfaces().dialer.unwrap_or_else(|| panic!("open"));
    assert_eq!(dialer.new_missed, 1);
}

#[test]
fn call_back_clears_the_cue_and_dismissing_the_notification_does_not() {
    let (state, _) = missed(synced(), 1, ALICE, 10_000);
    let (state, effects) = run(state, [Input::MissedCallDismissed(CallId(1))]);
    assert_eq!(kept(&effects), Vec::<u32>::new());
    assert_eq!(state.unseen_missed_calls(), 1);

    let (state, _) = missed(state, 2, ALICE, 30_000);
    let (state, effects) = run(state, [Input::CallBack(CallId(2))]);
    assert_eq!(kept(&effects), [0]);
    assert_eq!(state.unseen_missed_calls(), 0);
}

#[test]
fn a_count_kept_by_an_earlier_run_is_picked_up() {
    let (state, effects) = run(synced(), [Input::MissedCallsKept(3)]);
    assert_eq!(kept(&effects), [3]);
    let view = state.indicator(&state.surfaces());
    assert_eq!(view.menu[1], IndicatorEntry::MissedCalls(3));

    let (state, _) = run(
        state,
        [Input::PhoneDisconnected {
            address: ADDRESS.to_owned(),
        }],
    );
    assert_eq!(
        state.indicator(&state.surfaces()).look,
        IndicatorLook::DisconnectedMissedCalls
    );
}
