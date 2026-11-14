//! The Dialer in the Call state machine, at its public interface: what opens
//! and closes it, what it lists and selects, and what Enter does.

use desktop_calls_core::matching::DialerRowKind;
use desktop_calls_core::{
    CallHistory, CallHistoryEntry, CallHistoryKind, CallId, CallState, Caller, Contact,
    ContactNumber, Contacts, DialerView, Directory, Effect, IndicatorStatus, Input, NumberLabel,
    Phone, PhoneCommand, PhoneStatus, State, Timestamp, messages::Failure, step,
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

fn mobile(name: &str, number: &str) -> Contact {
    Contact {
        name: name.to_owned(),
        numbers: vec![ContactNumber {
            number: number.to_owned(),
            label: NumberLabel::Mobile,
        }],
    }
}

fn call(kind: CallHistoryKind, caller: Caller, at: u64) -> CallHistoryEntry {
    CallHistoryEntry {
        kind,
        caller,
        at: Some(Timestamp(at)),
        name: String::new(),
    }
}

/// Contacts and Call History as synced from the Phone.
fn synced() -> Input {
    let directory = Directory::new(
        vec![
            mobile("José Smith", "+447700900001"),
            mobile("Alice Smith", "+447700900123"),
            mobile("Joanna Smithers", "+447700900002"),
        ],
        Some("GB"),
    );
    let call_history = CallHistory::new(
        vec![
            call(
                CallHistoryKind::Missed,
                Caller::Number("+447700900999".to_owned()),
                3_000,
            ),
            call(CallHistoryKind::Missed, Caller::Withheld, 2_500),
            call(
                CallHistoryKind::Outgoing,
                Caller::Number("+447700900123".to_owned()),
                2_000,
            ),
        ],
        &directory,
    );
    Input::ContactsSynced {
        contacts: Contacts::new(directory),
        call_history,
    }
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
            synced(),
        ],
    )
    .0
}

/// Synced, then the Phone went out of range.
fn disconnected() -> State {
    run(
        connected(),
        [Input::PhoneDisconnected {
            address: ADDRESS.to_owned(),
        }],
    )
    .0
}

fn ringing(state: State) -> State {
    run(
        state,
        [Input::CallAdded {
            call: CALL,
            state: CallState::Ringing,
            caller: Caller::Number("+447700900123".to_owned()),
            at: Timestamp(5_000),
        }],
    )
    .0
}

fn dialer(state: &State) -> DialerView {
    let Some(view) = state.surfaces().dialer else {
        panic!("the Dialer isn't open");
    };
    view
}

fn open(state: State, query: &str) -> State {
    run(
        state,
        [Input::OpenDialer {
            query: query.to_owned(),
            select_call: None,
        }],
    )
    .0
}

fn selected_name(view: &DialerView) -> &str {
    let Some(selected) = view.selected else {
        panic!("no row is selected: {view:?}");
    };
    &view.rows[selected].name
}

#[test]
fn activating_with_nothing_ringing_or_live_opens_the_dialer_on_the_call_history() {
    let (state, effects) = run(connected(), [Input::Activate]);

    let view = dialer(&state);
    assert_eq!(view.query, "");
    let kinds: Vec<DialerRowKind> = view.rows.iter().map(|row| row.kind).collect();
    assert_eq!(
        kinds,
        [
            DialerRowKind::CallHistory(CallHistoryKind::Missed),
            DialerRowKind::CallHistory(CallHistoryKind::Missed),
            DialerRowKind::CallHistory(CallHistoryKind::Outgoing),
        ]
    );
    assert_eq!(view.selected, Some(0));
    assert_eq!(view.notice, None);
    assert!(effects.is_empty(), "{effects:?}");
}

#[test]
fn each_opening_is_told_apart() {
    let first = dialer(&open(connected(), "")).opened;
    let (state, _) = run(open(connected(), ""), [Input::CloseDialer]);
    let second = dialer(&open(state, "")).opened;

    assert_ne!(first, second);
}

#[test]
fn typing_lists_the_matches_and_selects_the_first() {
    let (state, _) = run(
        open(connected(), ""),
        [Input::DialerMove(2), Input::DialerQuery("jose".to_owned())],
    );

    let view = dialer(&state);
    assert_eq!(view.query, "jose");
    assert_eq!(selected_name(&view), "José Smith");
}

#[test]
fn the_selection_moves_and_stops_at_either_end() {
    let state = run(
        open(connected(), ""),
        [Input::DialerQuery("smith".to_owned())],
    )
    .0;
    let rows = dialer(&state).rows.len();

    let down = run(state.clone(), [Input::DialerMove(1), Input::DialerMove(1)]).0;
    assert_eq!(dialer(&down).selected, Some(2));
    let bottom = run(state.clone(), [Input::DialerMove(100)]).0;
    assert_eq!(dialer(&bottom).selected, Some(rows - 1));
    let top = run(down, [Input::DialerMove(-5)]).0;
    assert_eq!(dialer(&top).selected, Some(0));
}

#[test]
fn enter_calls_the_selected_row_and_the_dialer_closes_as_the_call_starts() {
    let state = run(open(connected(), "smith"), [Input::DialerMove(1)]).0;
    let Some(number) = dialer(&state).rows[1].dial() else {
        panic!("the second row has no number to dial");
    };

    let (state, effects) = run(state, [Input::DialerEnter]);

    assert!(
        effects.contains(&Effect::Phone(PhoneCommand::Dial(number))),
        "{effects:?}"
    );
    let surfaces = state.surfaces();
    assert_eq!(surfaces.dialer, None);
    assert!(surfaces.call_window.is_some());
}

#[test]
fn clicking_a_row_calls_it() {
    let (state, effects) = run(open(connected(), "jose"), [Input::DialerChoose(0)]);

    assert!(
        effects.contains(&Effect::Phone(PhoneCommand::Dial(
            "+447700900001".to_owned()
        ))),
        "{effects:?}"
    );
    assert_eq!(state.surfaces().dialer, None);
}

#[test]
fn with_the_phone_disconnected_the_dialer_searches_held_contacts_and_enter_says_why_not() {
    let state = open(disconnected(), "jose");
    assert_eq!(selected_name(&dialer(&state)), "José Smith");

    let (state, effects) = run(state, [Input::DialerEnter]);

    assert_eq!(effects, [Effect::ReportError(Failure::PhoneOutOfRange)]);
    let surfaces = state.surfaces();
    assert_eq!(
        surfaces.dialer.map(|view| view.notice),
        Some(Some(Failure::PhoneOutOfRange))
    );
    assert_eq!(surfaces.call_window, None);
}

#[test]
fn during_a_live_call_enter_says_hang_up_the_current_call_first() {
    let live = run(
        ringing(connected()),
        [Input::CallChanged {
            call: CALL,
            state: CallState::Active,
            at: Timestamp(6_000),
        }],
    )
    .0;

    let (state, effects) = run(open(live, "jose"), [Input::DialerEnter]);

    assert_eq!(effects, [Effect::ReportError(Failure::CallInProgress)]);
    assert_eq!(dialer(&state).notice, Some(Failure::CallInProgress));
}

#[test]
fn a_withheld_number_in_the_call_history_cant_be_called_back() {
    let (state, effects) = run(
        open(connected(), ""),
        [Input::DialerMove(1), Input::DialerEnter],
    );

    assert_eq!(effects, [Effect::ReportError(Failure::WithheldNumber)]);
    assert_eq!(dialer(&state).notice, Some(Failure::WithheldNumber));
}

#[test]
fn a_number_that_cant_be_dialled_is_flagged_as_it_is_typed() {
    let state = open(connected(), "07700 9x");

    let view = dialer(&state);
    assert!(view.rows.is_empty(), "{:?}", view.rows);
    assert_eq!(view.notice, Some(Failure::Undialable));
    let (state, _) = run(state, [Input::DialerQuery("07700 9".to_owned())]);
    assert_eq!(dialer(&state).notice, None);
}

#[test]
fn escape_closes_the_dialer() {
    let (state, effects) = run(open(connected(), "jose"), [Input::CloseDialer]);

    assert_eq!(state.surfaces().dialer, None);
    assert!(effects.is_empty(), "{effects:?}");
}

#[test]
fn activating_while_a_call_rings_closes_the_dialer_and_focuses_the_banner() {
    let state = ringing(open(connected(), "jose"));
    assert!(state.surfaces().dialer.is_some());

    let (state, _) = run(state, [Input::Activate]);

    let surfaces = state.surfaces();
    assert_eq!(surfaces.dialer, None);
    assert!(surfaces.banner.is_some_and(|banner| banner.focused));
}

#[test]
fn opening_for_a_call_selects_its_call_history_row() {
    let state = run(
        ringing(connected()),
        [Input::CallEnded {
            call: CALL,
            at: Timestamp(90_000),
        }],
    )
    .0;

    let (state, _) = run(
        state,
        [Input::OpenDialer {
            query: String::new(),
            select_call: Some(CALL),
        }],
    );

    let view = dialer(&state);
    assert_eq!(selected_name(&view), "Alice Smith");
}

#[test]
fn a_new_sync_while_open_updates_the_rows() {
    let state = open(disconnected(), "zed");
    assert!(dialer(&state).rows.is_empty());
    let directory = Directory::new(vec![mobile("Zed Jones", "+447700900007")], Some("GB"));
    let call_history = CallHistory::new(Vec::new(), &directory);

    let (state, _) = run(
        state,
        [Input::ContactsSynced {
            contacts: Contacts::new(directory),
            call_history,
        }],
    );

    assert_eq!(selected_name(&dialer(&state)), "Zed Jones");
}

#[test]
fn the_footer_names_only_the_keys_that_work_now() {
    use desktop_calls_core::DialerKey::{Call, Close, Move};
    // The Call History: Enter Calls, and Up and Down move.
    assert_eq!(dialer(&open(connected(), "")).keys, [Call, Move, Close]);
    // One match: nothing to move to.
    assert_eq!(dialer(&open(connected(), "alice")).keys, [Call, Close]);
    // Nothing matches and nothing can be dialled: only Escape does anything.
    assert_eq!(dialer(&open(connected(), "zed")).keys, [Close]);
}

fn my_pixel() -> Phone {
    Phone {
        name: "My Pixel".to_owned(),
        address: ADDRESS.to_owned(),
    }
}

#[test]
fn the_footer_shows_the_phone_as_the_indicator_does() {
    for (state, expected, indicator) in [
        (
            connected(),
            PhoneStatus::Connected(my_pixel()),
            IndicatorStatus::Connected(my_pixel()),
        ),
        (
            goes_out_of_range(connected()),
            PhoneStatus::OutOfRange(my_pixel()),
            IndicatorStatus::OutOfRange(my_pixel()),
        ),
        (
            State::default(),
            PhoneStatus::NoPhone,
            IndicatorStatus::NoPhone,
        ),
    ] {
        let state = open(state, "");
        let surfaces = state.surfaces();
        let phone = surfaces.dialer.as_ref().map(|view| view.phone.clone());
        assert_eq!(phone, Some(expected));
        assert_eq!(
            state.indicator(&surfaces).status,
            indicator,
            "the Indicator says the same"
        );
    }
}

#[test]
fn the_footer_names_the_phone_while_connected_and_otherwise_says_where_it_is() {
    use desktop_calls_core::messages::{dialer_key, dialer_phone};
    assert_eq!(
        dialer_phone(&PhoneStatus::Connected(my_pixel())),
        "My Pixel"
    );
    assert_eq!(
        dialer_phone(&PhoneStatus::OutOfRange(my_pixel())),
        "Out of range"
    );
    assert_eq!(dialer_phone(&PhoneStatus::NoPhone), "Not set up");
    assert_eq!(
        dialer_key(desktop_calls_core::DialerKey::Call),
        (&["Enter"][..], "Call")
    );
}

/// The Phone in `state` drops its connection and goes out of range.
fn goes_out_of_range(state: State) -> State {
    run(
        state,
        [
            Input::PhoneDisconnected {
                address: ADDRESS.to_owned(),
            },
            Input::PhoneAbsent {
                address: ADDRESS.to_owned(),
            },
        ],
    )
    .0
}

#[test]
fn the_footer_follows_the_phone_going_out_of_range_while_open() {
    let state = goes_out_of_range(open(connected(), ""));

    assert_eq!(dialer(&state).phone, PhoneStatus::OutOfRange(my_pixel()));
}
