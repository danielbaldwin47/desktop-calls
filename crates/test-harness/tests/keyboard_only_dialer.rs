//! The keyboard-only suite's cases for the Dialer, the Quiet Hours Window,
//! calling anyway while calls are off, and the Ringer Window. [`ACTIONS`](super::keyboard_only)
//! and the rest of the cases are in `keyboard_only.rs`.

use test_harness::vcards::{self, CallKind};
use test_harness::{AppState, Harness, Key};

use super::keyboard_only::*;

/// Alice and five more Contacts.
fn contacts() -> String {
    let mut all = vcards::contact("Alice", &[("CELL", "+44 7700 900456")]);
    all.push_str(&vcards::contacts(5, 1_000));
    all
}

const OLDER_CALLS: [&str; 2] = ["+447700900001", "+447700900002"];

/// How many Calls the Call History holds before Bob's Missed Call: more
/// than the Dialer's page of 8 rows, so Page Down shows it moves a page.
const HISTORY: u32 = 12;

/// The two older Calls, then ten more on earlier days.
fn older_calls() -> String {
    let mut calls = vcards::call(CallKind::Dialed, Some(OLDER_CALLS[0]), "20261104T100000")
        + &vcards::call(CallKind::Dialed, Some(OLDER_CALLS[1]), "20261104T090000");
    for day in 1..=HISTORY - 2 {
        calls.push_str(&vcards::call(
            CallKind::Dialed,
            Some(&format!("+4477009002{day:02}")),
            &format!("202610{day:02}T090000"),
        ));
    }
    calls
}

/// Waits until the Phone is asked to dial `number`, the person called
/// answers, and the keybinding and Ctrl+H hang up.
fn dialled_then_hung_up(harness: &Harness, number: &str, dials: usize) {
    let expected = format!("(\"{number}\",)");
    let recorded = harness.simulated_phone().wait_for_recorded(
        &format!("the Phone is asked to dial {number}"),
        DEADLINE,
        |recorded| asked(recorded, "Dial") >= dials,
    );
    let dialled: Vec<_> = telephony(&recorded)
        .into_iter()
        .filter(|call| call.member == "Dial")
        .map(|call| call.arguments.clone())
        .collect();
    assert_eq!(dialled.last(), Some(&expected), "{dialled:?}");
    let _ = harness.simulated_phone().alert();
    harness.simulated_phone().callee_answers();
    harness.wait_for_app("the Call is live", DEADLINE, |state| {
        state.calls.len() == 1 && state.calls[0].state == "active"
    });
    focus_the_call_window_showing(harness, "hang-up", "Ctrl+H");
    harness.keyboard().press_with_control('h');
    harness.wait_for_app("the Call is over", DEADLINE, |state| state.calls.is_empty());
}

fn open_the_dialer(harness: &Harness) -> AppState {
    press_the_keybinding(harness);
    harness.wait_for_app("the Dialer has the keyboard", DEADLINE, |state| {
        focused(state, "dialer")
    })
}

/// The Dialer's footer shows `key` doing `does`, such as `Enter Call`.
fn footer_shows(state: &AppState, keys: &str) -> bool {
    shown(state, "dialer", "keys")
        .is_some_and(|footer| footer.split(" · ").any(|shown| shown == keys))
}

fn selected_row(state: &AppState) -> String {
    shown(state, "dialer", "selected")
        .and_then(|selected| shown(state, "dialer", &format!("row-{selected}")))
        .unwrap_or_default()
        .to_owned()
}

fn quiet_hours_window<'a>(state: &'a AppState, key: &str) -> Option<&'a str> {
    shown(state, "quiet-hours-window", key)
}

/// The Quiet Hours Window's footer shows `keys`, such as `Esc Close`.
fn quiet_hours_keys(state: &AppState, keys: &str) -> bool {
    quiet_hours_window(state, "keys")
        .is_some_and(|footer| footer.lines().any(|shown| shown == keys))
}

fn wait_for_quiet_hours(harness: &Harness, why: &str, fields: &[(&str, &str)]) -> AppState {
    harness.wait_for_app(why, DEADLINE, |state| {
        fields
            .iter()
            .all(|(key, value)| quiet_hours_window(state, key) == Some(value))
    })
}

#[test]
fn the_dialer_and_the_quiet_hours_window_by_keys_alone() {
    let mut performed = Performed::by(DIALER);
    let harness = Harness::builder().headless_screen().omarchy_bar().start();
    let phone = harness.simulated_phone();
    phone.set_pbap_folder("pb", &contacts());
    phone.set_pbap_folder("cch", &older_calls());
    phone.connect();
    harness.wait_for_app(
        "Contacts and Call History are synced",
        SYNC_DEADLINE,
        |state| {
            state.connection_state == "connected"
                && state.contacts.state == "ready"
                && state.contacts.call_history == HISTORY
        },
    );

    let state = open_the_dialer(&harness);
    for keys in ["Enter Call", "↑ ↓ Move", "Alt Q Quiet hours", "Esc Close"] {
        assert!(
            footer_shows(&state, keys),
            "the footer shows {keys:?}: {state:#?}"
        );
    }
    performed.did("Open the Dialer: the keybinding");
    harness.keyboard().press(Key::Escape);
    harness.wait_for_app("Escape closes the Dialer", DEADLINE, |state| {
        state.surface("dialer").is_none()
    });
    harness
        .simulated_phone()
        .recorded_holds("nothing is dialled", HOLD, |recorded| {
            asked(recorded, "Dial") == 0
        });
    performed.did("Close the Dialer: Escape");

    open_the_dialer(&harness);
    for (key, row, action) in [
        (
            Key::PageDown,
            "8",
            "Move a page down the Call History: Page Down",
        ),
        (Key::PageUp, "0", "Move a page up the Call History: Page Up"),
        (Key::Down, "1", ""),
        (Key::Up, "0", "Move up the Call History: Up"),
    ] {
        harness.keyboard().press(key);
        harness.wait_for_app(&format!("{key:?} selects row {row}"), DEADLINE, |state| {
            shown(state, "dialer", "selected") == Some(row)
        });
        if !action.is_empty() {
            performed.did(action);
        }
    }
    harness.keyboard().press(Key::Down);
    let state = harness.wait_for_app("Down selects the second row", DEADLINE, |state| {
        shown(state, "dialer", "selected") == Some("1")
    });
    assert!(selected_row(&state).contains("0002"), "{state:#?}");
    harness.keyboard().press(Key::Return);
    dialled_then_hung_up(&harness, OLDER_CALLS[1], 1);
    performed.did("Call the next row of the Call History: Down, Enter");

    open_the_dialer(&harness);
    harness.keyboard().type_text("ali");
    let state = harness.wait_for_app("Alice is the first match", DEADLINE, |state| {
        shown(state, "dialer", "row-0").is_some_and(|row| row.starts_with("Alice"))
            && footer_shows(state, "Enter Call")
    });
    assert_eq!(shown(&state, "dialer", "selected"), Some("0"));
    harness.keyboard().press(Key::Return);
    dialled_then_hung_up(&harness, "+447700900456", 2);
    performed.did("Search Contacts and Call the match: type, Enter");

    // Bob rings out: a Missed Call, on top of the Call History.
    let path = harness.simulated_phone().ring(BOB);
    harness.wait_for_app("Bob's Call rings", DEADLINE, |state| state.ringing);
    harness.simulated_phone().set_pbap_folder(
        "cch",
        &(vcards::call(CallKind::Missed, Some(BOB), "20261105T100000") + &older_calls()),
    );
    harness.simulated_phone().caller_hangs_up(&path);
    harness.wait_for_app(
        "the Missed Call is in the Call History",
        SYNC_DEADLINE,
        |state| state.calls.is_empty() && state.contacts.call_history == HISTORY + 1,
    );
    open_the_dialer(&harness);
    let state = harness.wait_for_app("the Missed Call is selected", DEADLINE, |state| {
        selected_row(state).contains("Missed")
    });
    assert_eq!(shown(&state, "dialer", "selected"), Some("0"));
    harness.keyboard().press(Key::Return);
    dialled_then_hung_up(&harness, BOB, 3);
    performed.did("Call back a Missed Call: Enter");

    quiet_hours_by_keys_alone(&harness, &mut performed);
    performed.all_done();
}

/// From the Dialer: Alt+Q, and every key the Quiet Hours Window takes.
fn quiet_hours_by_keys_alone(harness: &Harness, performed: &mut Performed) {
    open_the_dialer(harness);
    harness.keyboard().press_with_alt('q');
    let state = wait_for_quiet_hours(
        harness,
        "Alt+Q opens the Quiet Hours Window with the switch focused and its Space keycap shown",
        &[
            ("switch", "off"),
            ("focus", "switch"),
            ("switch-key", "Space"),
        ],
    );
    assert!(state.surface("dialer").is_none(), "the Dialer closed");
    performed.did("Open the Quiet Hours Window: Alt+Q");
    assert!(quiet_hours_keys(&state, "Space Switch on"), "{state:#?}");
    harness.keyboard().type_text(" ");
    let state = harness.wait_for_app("Space switches Quiet hours on", DEADLINE, |state| {
        state.quiet_hours == Some((22 * 60, 7 * 60)) && quiet_hours_keys(state, "Tab Next field")
    });
    assert!(quiet_hours_keys(&state, "↑ ↓ 15 min"), "{state:#?}");
    performed.did("Switch Quiet hours on: Space");
    harness.keyboard().press(Key::Tab);
    let state = wait_for_quiet_hours(harness, "Tab focuses the start", &[("focus", "start")]);
    assert_eq!(
        quiet_hours_window(&state, "switch-key"),
        None,
        "the switch's keycap goes with its focus: {state:#?}"
    );
    harness.keyboard().press(Key::Up);
    harness.wait_for_app("Up steps the start", DEADLINE, |state| {
        state.quiet_hours == Some((22 * 60 + 15, 7 * 60))
    });
    performed.did("Step the start of Quiet Hours: Tab, Up");
    harness.keyboard().press(Key::Down);
    harness.wait_for_app("Down steps the start back", DEADLINE, |state| {
        state.quiet_hours == Some((22 * 60, 7 * 60))
    });
    performed.did("Step the start of Quiet Hours back: Down");
    harness.keyboard().press(Key::Tab);
    wait_for_quiet_hours(harness, "Tab focuses the end", &[("focus", "end")]);
    // Without Backspace, 07630 would not be 06:30.
    harness.keyboard().type_text("07");
    harness.keyboard().press(Key::Backspace);
    performed.did("Erase a digit typed in a time: Backspace");
    harness.keyboard().type_text("630");
    harness.wait_for_app("the end typed is saved", DEADLINE, |state| {
        state.quiet_hours == Some((22 * 60, 6 * 60 + 30))
    });
    performed.did("Type the end of Quiet Hours: Tab, digits");
    harness.keyboard().press_with_shift(Key::Tab);
    harness.keyboard().press_with_shift(Key::Tab);
    wait_for_quiet_hours(
        harness,
        "Shift+Tab focuses the switch, which shows its Space keycap",
        &[("focus", "switch"), ("switch-key", "Space")],
    );
    harness.keyboard().type_text(" ");
    harness.wait_for_app("Space switches Quiet Hours off", DEADLINE, |state| {
        state.quiet_hours.is_none()
    });
    performed.did("Switch Quiet Hours off: Shift+Tab, Space");
    let state = harness.app_state();
    assert!(quiet_hours_keys(&state, "Esc Close"), "{state:#?}");
    harness.keyboard().press(Key::Escape);
    harness.wait_for_app("Escape closes the Quiet Hours Window", DEADLINE, |state| {
        state.surface("quiet-hours-window").is_none()
    });
    performed.did("Close the Quiet Hours Window: Escape");
}

#[test]
fn while_calls_are_off_the_dialer_calls_anyway_by_keys_alone() {
    let mut performed = Performed::by(CALLS_OFF);
    let (harness, _window) = connected();
    // How calls go off has no key path yet (#292).
    let output = harness.cli(&["calls", "off"]);
    assert!(output.status.success(), "{output:?}");
    harness.wait_for_app("calls are off", DEADLINE, |state| {
        state.calls_off == "off" && state.connection_state == "disconnected"
    });

    open_the_dialer(&harness);
    harness.keyboard().type_text("07700 900888");
    let state = harness.wait_for_app("the Dialer says Enter calls anyway", DEADLINE, |state| {
        shown(state, "dialer", "query") == Some("07700 900888")
            && footer_shows(state, "Enter Call anyway")
    });
    assert!(shown(&state, "dialer", "calls-off").is_some(), "{state:#?}");
    harness.keyboard().press(Key::Return);
    harness.simulated_phone().wait_for_recorded(
        "the Phone connects and is asked to dial",
        PHONE_FOR_DIAL,
        |recorded| asked(recorded, "Dial") == 1,
    );
    dialled_then_hung_up(&harness, "07700900888", 1);
    harness.wait_for_app("calls go off again", DEADLINE, |state| {
        state.calls_off == "off"
            && !state.calls_off_pending
            && state.connection_state == "disconnected"
    });
    performed.did("Call a typed number while calls are off: Enter calls anyway");

    // Out of range, the Call waits for the Phone to connect: Ctrl+H
    // cancels it before it is placed.
    harness.simulated_phone().disconnect();
    open_the_dialer(&harness);
    harness.keyboard().type_text("07700 900888");
    harness.wait_for_app("the Dialer says Enter calls anyway", DEADLINE, |state| {
        footer_shows(state, "Enter Call anyway")
    });
    harness.keyboard().press(Key::Return);
    harness.wait_for_app("the Call Window says it is connecting", DEADLINE, |state| {
        shown(state, "call-window", "state") == Some("Connecting to your phone…")
    });
    focus_the_call_window_showing(&harness, "hang-up", "Ctrl+H");
    harness.keyboard().press_with_control('h');
    harness.wait_for_app("the Call ended before it was placed", DEADLINE, |state| {
        shown(state, "call-window", "state") == Some("Call ended")
    });
    harness.wait_for_app("calls are back off", DEADLINE, |state| {
        state.calls_off == "off" && !state.calls_off_pending && state.calls.is_empty()
    });
    harness
        .simulated_phone()
        .recorded_holds("nothing more is dialled", HOLD, |recorded| {
            asked(recorded, "Dial") == 1
        });
    performed.did("Hang up a Call placed before the Phone connects: Ctrl+H");
    performed.all_done();
}

/// Waits until the Ringer Window has the keyboard and shows each of
/// `fields`.
fn wait_for_ringer(harness: &Harness, what: &str, fields: &[(&str, &str)]) -> AppState {
    harness.wait_for_app(what, DEADLINE, |state| {
        focused(state, "ringer-window")
            && fields
                .iter()
                .all(|(name, value)| shown(state, "ringer-window", name) == Some(*value))
    })
}

#[test]
fn the_ringer_window_by_keys_alone() {
    let mut performed = Performed::by(RINGER);
    let (harness, _window) = connected();
    open_the_dialer(&harness);
    harness.keyboard().press_with_alt('r');
    let state = wait_for_ringer(
        &harness,
        "Alt+R opens the Ringer Window with the Chime focused",
        &[("focus", "chime"), ("keys", "Space Choose\nEsc Close")],
    );
    assert!(state.surface("dialer").is_none(), "the Dialer closed");
    performed.did("Open the Ringer Window: Alt+R");

    harness.keyboard().press(Key::Tab);
    wait_for_ringer(&harness, "Tab focuses Bells", &[("focus", "bells")]);
    harness.keyboard().type_text(" ");
    wait_for_ringer(
        &harness,
        "Space chooses Bells and plays it, and Esc now stops it",
        &[
            ("sound", "bells"),
            ("play", "Stop"),
            ("keys", "Space Choose\nEsc Stop"),
        ],
    );
    harness.wait_for_plays(1, DEADLINE);
    performed.did("Choose a Ring Sound: Tab, Space");
    harness.keyboard().press(Key::Escape);
    harness.wait_until_nothing_plays(DEADLINE);
    wait_for_ringer(
        &harness,
        "Escape stops it and leaves the window open",
        &[("play", "Play"), ("keys", "Space Choose\nEsc Close")],
    );
    performed.did("Stop the Ring Sound playing: Escape");

    harness.keyboard().press_with_shift(Key::Tab);
    wait_for_ringer(
        &harness,
        "Shift+Tab focuses the Chime",
        &[("focus", "chime")],
    );
    harness.keyboard().press_with_shift(Key::Tab);
    wait_for_ringer(
        &harness,
        "Shift+Tab focuses Play",
        &[("focus", "play"), ("play", "Play")],
    );
    harness.keyboard().type_text(" ");
    wait_for_ringer(&harness, "Space on Play plays Bells", &[("play", "Stop")]);
    harness.wait_for_plays(2, DEADLINE);
    performed.did("Play the Ring Sound again: Shift+Tab, Space on Play");
    harness.keyboard().press(Key::Escape);
    harness.wait_until_nothing_plays(DEADLINE);

    harness.keyboard().press_with_shift(Key::Tab);
    wait_for_ringer(
        &harness,
        "Shift+Tab goes round to the volume",
        &[("focus", "volume"), ("volume", "60 %")],
    );
    harness.keyboard().press(Key::Left);
    wait_for_ringer(
        &harness,
        "Left takes the volume down a step, and plays Bells at it",
        &[("volume", "55 %"), ("play", "Stop")],
    );
    let plays = harness.wait_for_plays(3, DEADLINE);
    assert_eq!(plays[2].volume, Some(0.55), "{plays:?}");
    performed.did("Set the Ringer's volume: Shift+Tab, Left");
    harness.keyboard().press(Key::Escape);
    harness.wait_until_nothing_plays(DEADLINE);
    wait_for_ringer(
        &harness,
        "nothing plays",
        &[("keys", "Space Choose\nEsc Close")],
    );

    harness.keyboard().press(Key::Escape);
    harness.wait_for_app("Escape closes the Ringer Window", DEADLINE, |state| {
        state.surface("ringer-window").is_none()
    });
    performed.did("Close the Ringer Window: Escape");
    performed.all_done();
}
