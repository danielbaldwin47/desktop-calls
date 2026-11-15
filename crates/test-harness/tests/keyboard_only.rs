//! The keyboard-only suite (ticket #25, polish scorecard item 7): every user
//! action, performed with synthetic keys alone.
//!
//! Each case starts from the Desktop Calls keybinding's command
//! (`desktop-calls` with no argument), or from a surface that opens by
//! itself (the Setup Window on first run), and then only presses keys: no
//! pointer, and no control-interface method that acts. The Simulated Phone
//! plays the other end of each Call, and the control interface only
//! observes. Before a key is pressed, the case checks that the surface
//! shows it: a keycap, the footer's keys, or the button that has focus.
//!
//! Each case is a journey through one surface, so the suite starts the app
//! eight times rather than once an action; the Setup Window's cases are in
//! `keyboard_only_setup.rs`, and the Dialer's in `keyboard_only_dialer.rs`.
//! [`ACTIONS`] names every action and the case that performs it; a case
//! fails unless it performed exactly its actions, and `every_action_is_performed_by_a_case_or_names_its_ticket`
//! fails if an action is left with neither a case nor a ticket. [`UNSHOWN`]
//! names the keys a case presses that its surface doesn't show yet, each
//! with its ticket.

use std::collections::BTreeSet;
use std::time::Duration;

use test_harness::{AppState, Harness, Key, RecordedMethodCall, TestClient};

use super::handoff::audio_requests;
use super::mute_and_tones::{sent, set_to, wait_for_microphone_volumes};

/// How long the app may take to show a change.
pub(super) const DEADLINE: Duration = Duration::from_secs(5);
/// How long Contacts and Call History may take to sync.
pub(super) const SYNC_DEADLINE: Duration = Duration::from_secs(25);
/// Longer than the 15 s the Phone has to connect for a Call placed while
/// calls are off.
pub(super) const PHONE_FOR_DIAL: Duration = Duration::from_secs(25);
/// How long something the Phone must not be asked is watched for.
pub(super) const HOLD: Duration = Duration::from_millis(500);

pub(super) const BOB: &str = "+447700900123";
const ALICE: &str = "+447700900456";
const CAROL: &str = "+447700900789";
const DAVE: &str = "+447700900321";
const ERIN: &str = "+447700900654";
const FRANK: &str = "+447700900987";

/// How keys alone reach an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeyPath {
    /// The case, a test in this file or `keyboard_only_setup.rs`, that
    /// performs it.
    Case(&'static str),
    /// No key path yet: the open ticket that adds one.
    Missing { ticket: u32 },
    /// Keys that something other than the app owns, and why.
    Elsewhere(&'static str),
}

use KeyPath::{Case, Elsewhere, Missing};

/// Every user action in the spec's stories (#1, #197 and #198), and how
/// keys alone reach it.
const ACTIONS: &[(&str, KeyPath)] = &[
    // The Incoming Call Banner.
    ("Answer a Call: A", Case(BANNER)),
    ("Decline a Call: D", Case(BANNER)),
    (
        "Give the Banner's keyboard back, the Call still ringing: Escape",
        Case(BANNER),
    ),
    // The Call Window and the Call Pill.
    ("Focus the Call Window: the keybinding", Case(CALL_WINDOW)),
    ("Mute: M", Case(CALL_WINDOW)),
    ("Unmute: M", Case(CALL_WINDOW)),
    ("Open the keypad: K", Case(CALL_WINDOW)),
    ("Send tones by typing them", Case(CALL_WINDOW)),
    ("Send tones by pasting them: Ctrl+V", Case(CALL_WINDOW)),
    ("Close the keypad: K", Case(CALL_WINDOW)),
    ("Move the audio to the Phone: A", Case(CALL_WINDOW)),
    ("Move the audio here: A", Case(CALL_WINDOW)),
    (
        "Collapse the Call Window to the Call Pill, the Call still live: Escape",
        Case(CALL_WINDOW),
    ),
    (
        "Expand the Call Pill with the keyboard in it: the keybinding",
        Case(CALL_WINDOW),
    ),
    ("Hang up: Ctrl+H", Case(CALL_WINDOW)),
    (
        "Hang up a Call placed before the Phone connects: Ctrl+H",
        Case(CALLS_OFF),
    ),
    // Call waiting.
    (
        "Give a Waiting Call's Banner the keyboard back, the Call still waiting: Escape",
        Case(CALL_WAITING),
    ),
    ("Hold & Answer: H", Case(CALL_WAITING)),
    ("Swap: S", Case(CALL_WAITING)),
    ("Hang up the live Call of two: Ctrl+H", Case(CALL_WAITING)),
    ("Resume the Call left on hold: S", Case(CALL_WAITING)),
    ("End & Answer: E", Case(CALL_WAITING)),
    ("Decline a Waiting Call: D", Case(CALL_WAITING)),
    ("Hang up the Call on hold: Shift+Ctrl+H", Case(CALL_WAITING)),
    // The Dialer and the Quiet Hours Window.
    ("Open the Dialer: the keybinding", Case(DIALER)),
    ("Close the Dialer: Escape", Case(DIALER)),
    ("Move a page down the Call History: Page Down", Case(DIALER)),
    ("Move a page up the Call History: Page Up", Case(DIALER)),
    ("Move up the Call History: Up", Case(DIALER)),
    (
        "Call the next row of the Call History: Down, Enter",
        Case(DIALER),
    ),
    (
        "Search Contacts and Call the match: type, Enter",
        Case(DIALER),
    ),
    ("Call back a Missed Call: Enter", Case(DIALER)),
    ("Open the Quiet Hours Window: Alt+Q", Case(DIALER)),
    ("Switch Quiet Hours on: Space", Case(DIALER)),
    ("Step the start of Quiet Hours: Tab, Up", Case(DIALER)),
    ("Step the start of Quiet Hours back: Down", Case(DIALER)),
    ("Type the end of Quiet Hours: Tab, digits", Case(DIALER)),
    ("Erase a digit typed in a time: Backspace", Case(DIALER)),
    ("Switch Quiet Hours off: Shift+Tab, Space", Case(DIALER)),
    ("Close the Quiet Hours Window: Escape", Case(DIALER)),
    // Calls Off.
    (
        "Call a typed number while calls are off: Enter calls anyway",
        Case(CALLS_OFF),
    ),
    ("Turn calls off until turned on", Missing { ticket: 292 }),
    ("Turn calls off for an hour", Missing { ticket: 292 }),
    ("Turn calls off until the morning", Missing { ticket: 292 }),
    ("Turn calls on", Missing { ticket: 292 }),
    (
        "Keep calls on, cancelling a turn-off that waits for the Call",
        Missing { ticket: 292 },
    ),
    // The Setup Window.
    ("Turn Bluetooth on: Enter", Case(FIRST_RUN)),
    ("Set up audio: Shift+Tab, Enter", Case(FIRST_RUN)),
    ("Pair the Phone listed: Enter", Case(FIRST_RUN)),
    (
        "Reject the pairing code: Tab, Enter on Reject",
        Case(SETUP_REFUSED),
    ),
    ("Confirm the pairing code: Enter", Case(FIRST_RUN)),
    (
        "Ask for Contacts again once the prompt timed out: Enter on Try again",
        Case(FIRST_RUN),
    ),
    ("Add the keybinding: Enter", Case(FIRST_RUN)),
    (
        "Cancel the keybinding's line: Tab, Enter on Cancel",
        Case(FIRST_RUN),
    ),
    ("Confirm the keybinding's line: Enter", Case(FIRST_RUN)),
    ("Finish: Enter on Done", Case(FIRST_RUN)),
    (
        "Try again once the Phone refused hands-free: Enter",
        Case(SETUP_REFUSED),
    ),
    (
        "Carry on once the Phone is back: Enter on Continue",
        Case(SETUP_REFUSED),
    ),
    (
        "Skip Contacts once the prompt timed out: Tab, Enter on Skip",
        Case(SETUP_REFUSED),
    ),
    (
        "Copy the install command: Shift+Tab, Enter",
        Case(SETUP_COPY),
    ),
    ("Close the Setup Window: Escape", Case(SETUP_COPY)),
    (
        "Open the Setup Window again: Connect your phone…",
        Missing { ticket: 292 },
    ),
    // A Missed Call's notification.
    (
        "Call back or dismiss a Missed Call from its notification",
        Elsewhere("Omarchy's notification daemon owns a notification's keys"),
    ),
];

/// Keys a case presses that its surface doesn't show yet: the action, and
/// the open ticket that shows them. Close on a Setup Window step is not
/// here: Escape closes it, and its footer says so on every step.
const UNSHOWN: &[(&str, u32)] = &[
    (
        "Collapse the Call Window to the Call Pill, the Call still live: Escape",
        300,
    ),
    ("Send tones by pasting them: Ctrl+V", 301),
    ("Move a page down the Call History: Page Down", 301),
    ("Move a page up the Call History: Page Up", 301),
    ("Erase a digit typed in a time: Backspace", 301),
    ("Switch Quiet Hours off: Shift+Tab, Space", 301),
];

const BANNER: &str = "the_banner_by_keys_alone";
const CALL_WINDOW: &str = "the_call_window_and_the_call_pill_by_keys_alone";
const CALL_WAITING: &str = "call_waiting_by_keys_alone";
pub(super) const DIALER: &str = "the_dialer_and_the_quiet_hours_window_by_keys_alone";
pub(super) const CALLS_OFF: &str = "while_calls_are_off_the_dialer_calls_anyway_by_keys_alone";
pub(super) const FIRST_RUN: &str =
    "keyboard_only_setup::first_run_in_the_setup_window_by_keys_alone";
pub(super) const SETUP_REFUSED: &str =
    "keyboard_only_setup::a_refused_phone_then_skipped_contacts_by_keys_alone";
pub(super) const SETUP_COPY: &str =
    "keyboard_only_setup::a_setup_window_fix_that_copies_a_command_by_keys_alone";

/// The tests that perform actions. A name here is not checked against the
/// tests that exist: deleting a test leaves its actions listed and passing
/// here, so a case is deleted with its entries in [`ACTIONS`].
const CASES: [&str; 8] = [
    BANNER,
    CALL_WINDOW,
    CALL_WAITING,
    DIALER,
    CALLS_OFF,
    FIRST_RUN,
    SETUP_REFUSED,
    SETUP_COPY,
];

/// Open tickets that add a missing key path, or show a key.
const OPEN_TICKETS: [u32; 3] = [292, 300, 301];

#[test]
fn every_action_is_performed_by_a_case_or_names_its_ticket() {
    let mut names = BTreeSet::new();
    for (action, path) in ACTIONS {
        assert!(names.insert(*action), "{action:?} is listed twice");
        match path {
            Case(case) => assert!(CASES.contains(case), "{action:?}: no case {case:?}"),
            Missing { ticket } => assert!(
                OPEN_TICKETS.contains(ticket),
                "{action:?}: #{ticket} is not a ticket this suite waits on"
            ),
            Elsewhere(why) => assert!(!why.is_empty(), "{action:?}: say whose keys"),
        }
    }
    for (action, ticket) in UNSHOWN {
        assert!(
            ACTIONS
                .iter()
                .any(|(listed, path)| listed == action && matches!(path, Case(_))),
            "{action:?} is unshown but no case performs it"
        );
        assert!(
            OPEN_TICKETS.contains(ticket),
            "{action:?}: #{ticket} is not a ticket this suite waits on"
        );
    }
    for case in CASES {
        assert!(
            !actions_of(case).is_empty(),
            "the case {case:?} performs no listed action"
        );
    }
}

fn actions_of(case: &str) -> Vec<&'static str> {
    ACTIONS
        .iter()
        .filter(|(_, path)| matches!(path, Case(listed) if *listed == case))
        .map(|(action, _)| *action)
        .collect()
}

/// The actions a case has performed, checked against [`ACTIONS`] at the end.
pub(super) struct Performed {
    case: &'static str,
    done: Vec<&'static str>,
}

impl Performed {
    pub(super) fn by(case: &'static str) -> Self {
        Self {
            case,
            done: Vec::new(),
        }
    }

    pub(super) fn did(&mut self, action: &'static str) {
        assert!(
            ACTIONS.iter().any(|(listed, _)| *listed == action),
            "{action:?} is not in ACTIONS"
        );
        self.done.push(action);
    }

    /// Fails unless the case performed exactly the actions it is listed
    /// for, in any order.
    pub(super) fn all_done(self) {
        let done: BTreeSet<_> = self.done.into_iter().collect();
        let listed: BTreeSet<_> = actions_of(self.case).into_iter().collect();
        assert_eq!(
            done, listed,
            "{} performed other actions than listed",
            self.case
        );
    }
}

/// Runs the keybinding's command, as Hyprland does on the key.
pub(super) fn press_the_keybinding(harness: &Harness) {
    let output = harness.cli(&[]);
    assert!(
        output.status.success(),
        "desktop-calls failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

pub(super) fn shown<'a>(state: &'a AppState, surface: &str, key: &str) -> Option<&'a str> {
    state
        .surface(surface)
        .and_then(|content| content.get(key))
        .map(String::as_str)
}

pub(super) fn focused(state: &AppState, surface: &str) -> bool {
    shown(state, surface, "focused") == Some("true")
}

fn state_of(state: &AppState, call: u32) -> Option<&str> {
    state
        .calls
        .iter()
        .find(|known| known.id == call)
        .map(|known| known.state.as_str())
}

/// Every method the Phone was asked on its Telephony service.
pub(super) fn telephony(recorded: &[RecordedMethodCall]) -> Vec<&RecordedMethodCall> {
    recorded
        .iter()
        .filter(|call| call.bus == "session" && call.path.starts_with("/org/pipewire/Telephony"))
        .collect()
}

pub(super) fn asked(recorded: &[RecordedMethodCall], member: &str) -> usize {
    telephony(recorded)
        .iter()
        .filter(|call| call.member == member)
        .count()
}

/// Waits until the Phone has been asked `member` `count` times in all.
fn wait_for_asked(harness: &Harness, member: &str, count: usize) -> Vec<RecordedMethodCall> {
    harness.simulated_phone().wait_for_recorded(
        &format!("the Phone is asked {member} ({count} in all)"),
        DEADLINE,
        |recorded| asked(recorded, member) >= count,
    )
}

/// What BlueZ was asked of the Phone's hands-free profile, in order.
fn profile_requests(recorded: &[RecordedMethodCall]) -> Vec<&str> {
    recorded
        .iter()
        .filter(|call| call.bus == "system" && call.member.ends_with("Profile"))
        .map(|call| call.member.as_str())
        .collect()
}

fn hung_up(recorded: &[RecordedMethodCall], path: &str) -> bool {
    recorded
        .iter()
        .any(|call| call.member == "Hangup" && call.path == path)
}

/// The Phone is asked neither to answer nor to hang up anything new while
/// [`HOLD`] passes: `answers` and `hang_ups` are the counts so far.
fn phone_left_alone(harness: &Harness, answers: usize, hang_ups: usize) {
    harness
        .simulated_phone()
        .recorded_holds("no Call is answered or hung up", HOLD, |recorded| {
            asked(recorded, "Answer") == answers && asked(recorded, "Hangup") == hang_ups
        });
}

/// A connected Phone, and the user's own window open with the keyboard, for
/// Escape to give it back to.
pub(super) fn connected() -> (Harness, TestClient) {
    let harness = Harness::builder().headless_screen().omarchy_bar().start();
    harness.simulated_phone().connect();
    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
    let client = harness.start_test_client();
    client.wait_for("the user's window has the keyboard", DEADLINE, |line| {
        line == "keyboard-enter"
    });
    (harness, client)
}

/// The keycaps a Banner's buttons show, such as `["D", "A"]`.
fn keycaps(state: &AppState) -> Vec<&str> {
    shown(state, "banner", "keycaps").map_or_else(Vec::new, |caps| caps.split(", ").collect())
}

/// Presses the keybinding while a Call rings or waits, and waits until the
/// Banner has the keyboard and shows `key` on a button.
fn focus_the_banner_showing(harness: &Harness, key: &str) {
    press_the_keybinding(harness);
    harness.wait_for_app(
        &format!("the Banner has the keyboard and shows {key}"),
        DEADLINE,
        |state| focused(state, "banner") && keycaps(state).contains(&key),
    );
}

/// Waits until the focused Banner's footer says what Escape does.
fn banner_shows_escape(harness: &Harness) {
    harness.wait_for_app("the Banner's footer shows Esc", DEADLINE, |state| {
        shown(state, "banner", "footer")
            .is_some_and(|footer| footer.starts_with("Esc gives the keyboard back"))
    });
}

/// Presses the keybinding during a Call, and waits until the Call Window
/// has the keyboard and shows `shortcut` for `action`.
pub(super) fn focus_the_call_window_showing(harness: &Harness, action: &str, shortcut: &str) {
    press_the_keybinding(harness);
    wait_for_call_window_showing(harness, action, shortcut);
}

fn wait_for_call_window_showing(harness: &Harness, action: &str, shortcut: &str) -> AppState {
    harness.wait_for_app(
        &format!("the Call Window has the keyboard and shows {shortcut} for {action}"),
        DEADLINE,
        |state| {
            focused(state, "call-window")
                && shown(state, "call-window", &format!("{action}-shortcut")) == Some(shortcut)
        },
    )
}

/// Rings the Phone from `number`; the keybinding and A answer on the
/// Desktop. Returns the Call's object path and id.
fn answered_by_keys(harness: &Harness, number: &str) -> (String, u32) {
    let path = harness.simulated_phone().ring(number);
    let state = harness.wait_for_app("the Call rings", DEADLINE, |state| {
        state.ringing && state.calls.len() == 1 && state.surface("banner").is_some()
    });
    let call = state.calls[0].id;
    focus_the_banner_showing(harness, "A");
    harness.keyboard().type_text("a");
    // Each case answers once by A, so this is its Answer.
    let recorded = wait_for_asked(harness, "Answer", 1);
    assert!(
        recorded
            .iter()
            .any(|asked| asked.member == "Answer" && asked.path == path),
        "{recorded:#?}"
    );
    harness.wait_for_app("the Call is live here", DEADLINE, |state| {
        state_of(state, call) == Some("active") && state.audio_location == "desktop"
    });
    (path, call)
}

#[test]
fn the_banner_by_keys_alone() {
    let mut performed = Performed::by(BANNER);
    let (harness, _window) = connected();
    let path = harness.simulated_phone().ring(BOB);
    let state = harness.wait_for_app("the Banner shows", DEADLINE, |state| {
        state.ringing && state.surface("banner").is_some()
    });
    let call = state.calls[0].id;

    // Escape gives the keyboard back; the Call rings on.
    focus_the_banner_showing(&harness, "D");
    banner_shows_escape(&harness);
    harness.keyboard().press(Key::Escape);
    harness.wait_for_app("the Banner gave the keyboard back", DEADLINE, |state| {
        state.surface("banner").is_some() && !focused(state, "banner")
    });
    harness.app_holds("the Call still rings", HOLD, |state| {
        state.ringing && state_of(state, call) == Some("ringing")
    });
    phone_left_alone(&harness, 0, 0);
    performed.did("Give the Banner's keyboard back, the Call still ringing: Escape");

    focus_the_banner_showing(&harness, "D");
    harness.keyboard().type_text("d");
    let recorded = wait_for_asked(&harness, "Hangup", 1);
    assert!(hung_up(&recorded, &path), "{recorded:#?}");
    harness.wait_for_app("the Banner is gone", DEADLINE, |state| {
        state.calls.is_empty() && state.surface("banner").is_none()
    });
    performed.did("Decline a Call: D");

    answered_by_keys(&harness, ALICE);
    performed.did("Answer a Call: A");
    performed.all_done();
}

#[test]
fn the_call_window_and_the_call_pill_by_keys_alone() {
    let mut performed = Performed::by(CALL_WINDOW);
    let (harness, _window) = connected();
    harness.simulated_phone().set_microphone_volume(9);
    let (path, call) = answered_by_keys(&harness, BOB);

    focus_the_call_window_showing(&harness, "mute", "M");
    performed.did("Focus the Call Window: the keybinding");
    harness.keyboard().type_text("m");
    wait_for_microphone_volumes(&harness, &[set_to(0)]);
    harness.wait_for_app("M mutes", DEADLINE, |state| state.muted);
    performed.did("Mute: M");
    harness.keyboard().type_text("m");
    wait_for_microphone_volumes(&harness, &[set_to(0), set_to(9)]);
    harness.wait_for_app("M unmutes", DEADLINE, |state| !state.muted);
    performed.did("Unmute: M");

    wait_for_call_window_showing(&harness, "keypad", "K");
    harness.keyboard().type_text("k");
    harness.wait_for_app("K opens the keypad", DEADLINE, |state| {
        shown(state, "call-window", "keypad") == Some("shown")
    });
    performed.did("Open the keypad: K");
    harness.keyboard().type_text("12#");
    let recorded = wait_for_asked(&harness, "SendTones", 3);
    assert_eq!(sent(&recorded), ["1", "2", "#"], "{recorded:#?}");
    performed.did("Send tones by typing them");
    let pasted = harness.set_clipboard("*0");
    harness.keyboard().press_with_control('v');
    let recorded = wait_for_asked(&harness, "SendTones", 5);
    assert_eq!(sent(&recorded), ["1", "2", "#", "*", "0"], "{recorded:#?}");
    drop(pasted);
    harness.wait_for_app("the keypad shows the tones sent", DEADLINE, |state| {
        shown(state, "call-window", "tones") == Some("12#*0")
    });
    performed.did("Send tones by pasting them: Ctrl+V");
    harness.keyboard().type_text("k");
    harness.wait_for_app("K closes the keypad", DEADLINE, |state| {
        shown(state, "call-window", "keypad").is_none()
    });
    performed.did("Close the keypad: K");

    wait_for_call_window_showing(&harness, "handoff", "A");
    harness.keyboard().type_text("a");
    harness.wait_for_app("A moves the audio to the Phone", DEADLINE, |state| {
        state.audio_location == "phone"
    });
    // The Desktop drops hands-free and connects it again, and hangs up
    // nothing.
    let recorded = harness.simulated_phone().wait_for_recorded(
        "BlueZ is asked to drop hands-free and connect it again",
        DEADLINE,
        |recorded| profile_requests(recorded).len() >= 2,
    );
    assert_eq!(
        profile_requests(&recorded),
        ["DisconnectProfile", "ConnectProfile"],
        "{recorded:#?}"
    );
    assert_eq!(asked(&recorded, "Hangup"), 0, "{recorded:#?}");
    performed.did("Move the audio to the Phone: A");
    wait_for_call_window_showing(&harness, "handoff", "A");
    let activations = |recorded: &[RecordedMethodCall]| {
        audio_requests(recorded)
            .into_iter()
            .filter(|request| *request == "Activate")
            .count()
    };
    let before = activations(&harness.simulated_phone().wait_for_recorded(
        "anything",
        DEADLINE,
        |_| true,
    ));
    harness.keyboard().type_text("a");
    harness.simulated_phone().wait_for_recorded(
        "the Phone is asked to bring the audio here",
        DEADLINE,
        |recorded| activations(recorded) > before,
    );
    harness.wait_for_app("A brings the audio here", DEADLINE, |state| {
        state.audio_location == "desktop"
    });
    performed.did("Move the audio here: A");

    // Escape collapses the Call Window; the Call stays live.
    let hang_ups = asked(&wait_for_asked(&harness, "Answer", 1), "Hangup");
    harness.keyboard().press(Key::Escape);
    harness.wait_for_app("Escape collapses the Call Window", DEADLINE, |state| {
        state.surface("call-pill").is_some() && state.surface("call-window").is_none()
    });
    harness.app_holds("the Call stays live", HOLD, |state| {
        state_of(state, call) == Some("active")
    });
    phone_left_alone(&harness, 1, hang_ups);
    performed.did("Collapse the Call Window to the Call Pill, the Call still live: Escape");

    focus_the_call_window_showing(&harness, "hang-up", "Ctrl+H");
    performed.did("Expand the Call Pill with the keyboard in it: the keybinding");
    harness.keyboard().press_with_control('h');
    let recorded = wait_for_asked(&harness, "Hangup", hang_ups + 1);
    assert!(hung_up(&recorded, &path), "{recorded:#?}");
    harness.wait_for_app("the Call is over", DEADLINE, |state| state.calls.is_empty());
    performed.did("Hang up: Ctrl+H");
    performed.all_done();
}

/// Waits until a Call from `number` waits during the live one, the Banner
/// offering its choices. Returns the Call's object path and id.
fn waits(harness: &Harness, number: &str, calls: usize) -> (String, u32) {
    let path = harness.simulated_phone().ring(number);
    let state = harness.wait_for_app("a Call waits", DEADLINE, |state| {
        state.calls.len() == calls
            && state
                .calls
                .last()
                .is_some_and(|call| call.state == "waiting")
            && shown(state, "banner", "options").is_some()
    });
    let call = state.calls.last().map_or(0, |call| call.id);
    (path, call)
}

#[test]
fn call_waiting_by_keys_alone() {
    let mut performed = Performed::by(CALL_WAITING);
    let (harness, _window) = connected();
    let (_, bob) = answered_by_keys(&harness, BOB);

    // Escape on a Waiting Call's Banner leaves both Calls as they were.
    let (alice_path, alice) = waits(&harness, ALICE, 2);
    focus_the_banner_showing(&harness, "H");
    banner_shows_escape(&harness);
    harness.keyboard().press(Key::Escape);
    harness.wait_for_app("the Banner gave the keyboard back", DEADLINE, |state| {
        state.surface("banner").is_some() && !focused(state, "banner")
    });
    harness.app_holds("Alice still waits and Bob is live", HOLD, |state| {
        state_of(state, alice) == Some("waiting") && state_of(state, bob) == Some("active")
    });
    phone_left_alone(&harness, 1, 0);
    performed.did("Give a Waiting Call's Banner the keyboard back, the Call still waiting: Escape");

    focus_the_banner_showing(&harness, "H");
    harness.keyboard().type_text("h");
    wait_for_asked(&harness, "HoldAndAnswer", 1);
    harness.wait_for_app("Alice is live and Bob on hold", DEADLINE, |state| {
        state_of(state, alice) == Some("active") && state_of(state, bob) == Some("held")
    });
    performed.did("Hold & Answer: H");

    focus_the_call_window_showing(&harness, "swap", "S");
    harness.keyboard().type_text("s");
    wait_for_asked(&harness, "SwapCalls", 1);
    harness.wait_for_app("Bob is live and Alice on hold", DEADLINE, |state| {
        state_of(state, bob) == Some("active") && state_of(state, alice) == Some("held")
    });
    performed.did("Swap: S");

    // Two Calls: Shift+Ctrl+H hangs up the one on hold, and only it.
    wait_for_call_window_showing(&harness, "hang-up-held", "Shift+Ctrl+H");
    harness.keyboard().press_with_control('H');
    let recorded = wait_for_asked(&harness, "Hangup", 1);
    assert!(hung_up(&recorded, &alice_path), "{recorded:#?}");
    harness.wait_for_app("only Bob is left, live", DEADLINE, |state| {
        state.calls.len() == 1 && state_of(state, bob) == Some("active")
    });
    performed.did("Hang up the Call on hold: Shift+Ctrl+H");

    let (_, carol) = waits(&harness, CAROL, 2);
    focus_the_banner_showing(&harness, "E");
    harness.keyboard().type_text("e");
    wait_for_asked(&harness, "ReleaseAndAnswer", 1);
    harness.wait_for_app("only Carol is left, live", DEADLINE, |state| {
        state.calls.len() == 1 && state_of(state, carol) == Some("active")
    });
    performed.did("End & Answer: E");

    let hang_ups = asked(&wait_for_asked(&harness, "Hangup", 1), "Hangup");
    let (dave_path, _) = waits(&harness, DAVE, 2);
    focus_the_banner_showing(&harness, "D");
    harness.keyboard().type_text("d");
    let recorded = wait_for_asked(&harness, "Hangup", hang_ups + 1);
    assert!(hung_up(&recorded, &dave_path), "{recorded:#?}");
    harness.wait_for_app("only Carol is left", DEADLINE, |state| {
        state.calls.len() == 1 && state_of(state, carol) == Some("active")
    });
    performed.did("Decline a Waiting Call: D");

    // Two Calls: Ctrl+H hangs up the live one, and only it.
    let (erin_path, erin) = held_for(&harness, ERIN, carol, 2);
    wait_for_call_window_showing(&harness, "hang-up", "Ctrl+H");
    harness.keyboard().press_with_control('h');
    let recorded = wait_for_asked(&harness, "Hangup", hang_ups + 2);
    assert!(hung_up(&recorded, &erin_path), "{recorded:#?}");
    harness.wait_for_app("only Carol is left", DEADLINE, |state| {
        state.calls.len() == 1
            && state_of(state, erin).is_none()
            && state_of(state, carol).is_some()
    });
    performed.did("Hang up the live Call of two: Ctrl+H");

    // Carol alone on hold, once the other caller hangs up: S resumes her.
    let (frank_path, _) = held_for(&harness, FRANK, carol, 3);
    harness.simulated_phone().caller_hangs_up(&frank_path);
    let state = harness.wait_for_app("the Call Window offers Resume on S", DEADLINE, |state| {
        focused(state, "call-window")
            && shown(state, "call-window", "swap") == Some("Resume")
            && shown(state, "call-window", "swap-shortcut") == Some("S")
    });
    assert_eq!(state_of(&state, carol), Some("held"), "{state:#?}");
    harness.keyboard().type_text("s");
    harness.wait_for_app("Carol is live again", DEADLINE, |state| {
        state.calls.len() == 1 && state_of(state, carol) == Some("active")
    });
    performed.did("Resume the Call left on hold: S");

    wait_for_call_window_showing(&harness, "hang-up", "Ctrl+H");
    harness.keyboard().press_with_control('h');
    wait_for_asked(&harness, "Hangup", hang_ups + 3);
    harness.wait_for_app("no Call is left", DEADLINE, |state| state.calls.is_empty());
    performed.all_done();
}

/// A Call from `number` waits during `live`'s, and H holds `live` and
/// answers it. Afterwards the Call Window has the keyboard. Returns the
/// new Call's object path and id. `answered` counts Hold & Answers so far,
/// this one too.
fn held_for(harness: &Harness, number: &str, live: u32, answered: usize) -> (String, u32) {
    let (path, call) = waits(harness, number, 2);
    focus_the_banner_showing(harness, "H");
    harness.keyboard().type_text("h");
    wait_for_asked(harness, "HoldAndAnswer", answered);
    harness.wait_for_app(
        "the new Call is live and the other on hold",
        DEADLINE,
        |state| state_of(state, call) == Some("active") && state_of(state, live) == Some("held"),
    );
    press_the_keybinding(harness);
    harness.wait_for_app("the Call Window has the keyboard", DEADLINE, |state| {
        focused(state, "call-window")
    });
    (path, call)
}
