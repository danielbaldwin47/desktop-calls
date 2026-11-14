//! Scenarios for ticket #111: while the Desktop has lost contact with a live
//! Call, which carries on on the Phone, every surface says so and offers
//! only what works. The Call Window's Hang up says to hang up on the Phone
//! and does nothing, Mute and the keypad are greyed out without the "Mute
//! works only…" hint, and the Indicator says what the Call Window says,
//! never "out of range" while only hands-free is lost. Contact is lost both
//! ways: hands-free not coming back after a Handoff to the Phone (#101), and
//! the Phone going out of range (#19), with a Held Call and with a Waiting
//! Call. Checked through the Call Window's `Surfaces`, the keyboard, the
//! tray watcher stand-in, the app's `Error` signals, and what the Simulated
//! Phone was asked.

use std::time::Duration;

use test_harness::{AppState, Harness, Key, MenuEntry, RecordedMethodCall, Region, Screenshot};

/// How long the app may take to notice a change on the Phone.
const DEADLINE: Duration = Duration::from_secs(5);
/// How long the lost Call Window stays before it closes, with some slack.
const LOST_WINDOW_DEADLINE: Duration = Duration::from_secs(14);
/// How soon the Phone is connected again once it is back in range.
const RECONNECT_DEADLINE: Duration = Duration::from_secs(10);
/// How long something that must not happen is watched for.
const HOLD: Duration = Duration::from_millis(500);

/// A little longer than the Phone link waits for a vanished Call to come
/// back as the same Call (2 s), and well within the time the lost Call
/// Window stays.
const PHONE_LINK_GIVES_UP: Duration = Duration::from_secs(4);

const PHONE_NAME: &str = "My Pixel";
/// Bob, the first caller.
const BOB: &str = "+447700900123";
/// Alice, who calls while the user is on with Bob.
const ALICE: &str = "+447700900456";

const LOST: &str = "Connection to phone lost: the Call continues on your phone.";
const HANG_UP_ON_PHONE: &str = "Hang up on your phone";
/// What the Indicator says while the Call Window says [`LOST`] about Bob's
/// Call.
const INDICATOR_LOST_BOB: &str =
    "Connection to phone lost: the Call with +44 7700 900123 continues on your phone";
const MUTE_UNAVAILABLE: &str =
    "Mute works only while the Call's audio is on this computer; mute on your phone instead.";

/// The app on a screen of its own, so its surfaces show, with the Phone
/// connected.
fn connected() -> Harness {
    let harness = Harness::builder()
        .headless_screen()
        .phone_name(PHONE_NAME)
        .start();
    harness.simulated_phone().connect();
    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
    harness
}

fn call_window(state: &AppState, key: &str) -> Option<String> {
    state
        .surface("call-window")
        .and_then(|window| window.get(key).cloned())
}

fn state_of(state: &AppState, call: u32) -> Option<&str> {
    state
        .calls
        .iter()
        .find(|known| known.id == call)
        .map(|known| known.state.as_str())
}

/// Bob rings and the user answers on the Desktop. Returns the Call's object
/// path and id.
fn on_a_call(harness: &Harness) -> (String, u32) {
    let path = harness.simulated_phone().ring(BOB);
    let state = harness.wait_for_app("Bob's Call rings", DEADLINE, |state| {
        state.ringing && state.calls.len() == 1
    });
    let call = state.calls[0].id;
    harness.answer(call);
    harness.wait_for_app("Bob's Call is live here", DEADLINE, |state| {
        state.audio_location == "desktop" && state_of(state, call) == Some("active")
    });
    (path, call)
}

/// Alice rings during the Call: waits until the Banner offers the Waiting
/// Call's options. Returns her Call's object path and id.
fn alice_waits(harness: &Harness) -> (String, u32) {
    let path = harness.simulated_phone().ring(ALICE);
    let state = harness.wait_for_app("Alice's Call waits", DEADLINE, |state| {
        state.calls.len() == 2
            && state.calls[1].state == "waiting"
            && state
                .surface("banner")
                .is_some_and(|banner| banner.contains_key("options"))
    });
    (path, state.calls[1].id)
}

/// Hands-free drops for a Handoff to the Phone and never comes back.
fn lose_hands_free(harness: &Harness) {
    harness
        .simulated_phone()
        .enable_quirk("connect-profile-fails");
    if let Err(error) = harness.move_audio("phone") {
        panic!("MoveAudio(\"phone\") was refused: {error:?}");
    }
}

/// Waits until the Call Window says the connection is lost.
fn wait_lost(harness: &Harness) -> AppState {
    harness.wait_for_app(
        "the Call Window says the connection is lost",
        DEADLINE,
        |state| call_window(state, "notice").as_deref() == Some(LOST),
    )
}

/// What the Phone was asked that would act on a Call: hang up, answer,
/// swap, or a microphone gain for Mute.
fn acts_on_calls(recorded: &[RecordedMethodCall]) -> usize {
    recorded
        .iter()
        .filter(|call| {
            matches!(
                call.member.as_str(),
                "Hangup" | "Answer" | "HoldAndAnswer" | "ReleaseAndAnswer" | "SwapCalls"
            ) || (call.member == "Set" && call.arguments.contains("MicrophoneVolume"))
        })
        .count()
}

/// Checks the Call Window of a Call with contact intact offers the tone
/// entry and the keypad toggle.
fn assert_offers_tones(state: &AppState) {
    let window = state
        .surface("call-window")
        .unwrap_or_else(|| panic!("no Call Window: {state:#?}"));
    for key in ["tone-entry", "keypad-toggle"] {
        assert!(window.contains_key(key), "{key} is not offered: {window:#?}");
    }
}

/// Checks the lost Call Window offers nothing as if it worked: Hang up says
/// to hang up on the Phone, no control has a shortcut, Mute has no hint
/// beside the lost message, and neither the tone entry, the keypad toggle nor
/// the keypad shows, as no tone can be sent.
fn assert_offers_nothing(state: &AppState) {
    let window = state
        .surface("call-window")
        .unwrap_or_else(|| panic!("no Call Window: {state:#?}"));
    assert_eq!(
        window.get("hang-up").map(String::as_str),
        Some(HANG_UP_ON_PHONE),
        "{window:#?}"
    );
    for key in [
        "hang-up-shortcut",
        "mute-shortcut",
        "keypad-shortcut",
        "handoff-shortcut",
        "swap-shortcut",
        "hang-up-held-shortcut",
        "mute-unavailable",
        "tone-entry",
        "keypad-toggle",
        "keypad",
        "tones",
    ] {
        assert!(!window.contains_key(key), "{key} is offered: {window:#?}");
    }
}

/// With the Call Window's keyboard, presses each of its shortcuts and
/// checks that none of them reaches the app or the Phone: no `Error`
/// signal, and nothing asked of the Phone.
fn assert_keys_do_nothing(harness: &Harness) {
    harness.activate();
    harness.wait_for_app("the Call Window has the keyboard", DEADLINE, |state| {
        call_window(state, "focused").as_deref() == Some("true")
    });
    let before = acts_on_calls(&harness.simulated_phone().wait_for_recorded(
        "what the Phone was asked so far",
        DEADLINE,
        |_| true,
    ));
    let errors = harness.app_errors().len();
    harness.keyboard().press_with_control('h');
    harness.keyboard().press_with_control('H');
    harness.keyboard().type_text("maks1");
    // Nothing the app reports marks a key as handled when it does nothing,
    // so this watches for a while instead. Before #111, these keys reached
    // the app within it as refused intents, each an `Error` signal.
    harness.simulated_phone().recorded_holds(
        "no key asks the Phone anything",
        HOLD,
        |recorded| acts_on_calls(recorded) == before,
    );
    assert_eq!(
        harness.app_errors()[errors..],
        [],
        "a greyed-out control was pressed through"
    );
    let state = harness.app_state();
    assert_eq!(call_window(&state, "keypad"), None, "K showed the keypad");
}

/// Waits until the Indicator's tooltip says `tooltip`, and returns its menu.
fn indicator_says(harness: &Harness, tooltip: &str) -> Vec<MenuEntry> {
    harness.tray().wait_for(
        &format!("the Indicator says {tooltip:?}"),
        DEADLINE,
        |items| {
            items
                .iter()
                .any(|item| item.tooltip.as_deref() == Some(tooltip))
        },
    );
    harness.tray().menu()
}

/// Checks the Indicator's menu says what the lost Call Window says, and
/// offers Hang up only as a greyed-out pointer to the Phone, with no Mute.
fn assert_lost_menu(menu: &[MenuEntry], status: &str) {
    assert_eq!(
        MenuEntry::labels(menu),
        [
            status,
            "Open Dialer",
            "Turn calls off",
            "Connect your phone…",
            "---",
            HANG_UP_ON_PHONE
        ]
    );
    let hang_up = MenuEntry::find(menu, HANG_UP_ON_PHONE)
        .unwrap_or_else(|| panic!("no Hang up entry: {menu:#?}"));
    assert!(!hang_up.enabled, "{hang_up:#?}");
}

/// Gives the Call Window the keyboard and waits until it has it.
fn focus_call_window(harness: &Harness) {
    harness.activate();
    harness.wait_for_app("the Call Window has the keyboard", DEADLINE, |state| {
        call_window(state, "focused").as_deref() == Some("true")
    });
}

/// Types the tones "12" and opens the keypad, and waits until the Call
/// Window shows both.
fn open_keypad_with_tones(harness: &Harness) {
    assert_offers_tones(&harness.app_state());
    focus_call_window(harness);
    harness.keyboard().type_text("12");
    harness.keyboard().type_text("k");
    harness.wait_for_app("the keypad shows with the tones typed", DEADLINE, |state| {
        call_window(state, "keypad").as_deref() == Some("shown")
            && call_window(state, "tones").as_deref() == Some("12")
    });
}

/// Checks the tone entry works: with the keyboard, a digit typed lands in
/// it, after `before`, and Backspace takes it out again, which only the
/// focused tone entry does.
fn assert_tones_usable(harness: &Harness, before: Option<&str>) {
    focus_call_window(harness);
    let tones = |state: &AppState| call_window(state, "tones");
    let typed = format!("{}5", before.unwrap_or_default());
    harness.keyboard().type_text("5");
    harness.wait_for_app("the digit is in the tone entry", DEADLINE, |state| {
        tones(state).as_deref() == Some(typed.as_str())
    });
    harness.keyboard().press(Key::Backspace);
    harness.wait_for_app("Backspace takes it out again", DEADLINE, |state| {
        tones(state).as_deref() == before
    });
}

/// Waits until contact is back: the lost message has gone and the Call is
/// live again. Returns the Call's id as the Call Window shows it.
fn wait_contact_back(harness: &Harness) -> (AppState, u32) {
    let state = harness.wait_for_app("contact is back", RECONNECT_DEADLINE, |state| {
        state.connection_state == "connected"
            && call_window(state, "notice").is_none()
            && call_window(state, "hang-up-shortcut").is_some()
    });
    let call = call_window(&state, "call")
        .and_then(|call| call.parse().ok())
        .unwrap_or_else(|| panic!("no Call in the Call Window: {state:#?}"));
    (state, call)
}

#[test]
fn with_hands_free_lost_the_call_window_offers_neither_hang_up_mute_nor_tones() {
    let harness = connected();
    let (_, bob) = on_a_call(&harness);
    // With the keypad open and a tone typed, none of it shows once lost.
    open_keypad_with_tones(&harness);
    lose_hands_free(&harness);

    let state = wait_lost(&harness);
    assert_eq!(call_window(&state, "call"), Some(bob.to_string()));
    assert_offers_nothing(&state);
    assert_keys_do_nothing(&harness);
    let on_phone = harness.simulated_phone().calls_on_phone();
    assert_eq!(on_phone.len(), 1, "{on_phone:?}");
    assert_eq!(on_phone[0].1, "active", "the Call carries on on the Phone");
}

#[test]
fn with_hands_free_lost_the_indicator_says_what_the_call_window_says() {
    let harness = connected();
    let _bob = on_a_call(&harness);
    lose_hands_free(&harness);
    wait_lost(&harness);

    let menu = indicator_says(&harness, INDICATOR_LOST_BOB);
    assert_lost_menu(&menu, INDICATOR_LOST_BOB);

    // Once the Call Window has closed, the Phone is still in range: only
    // hands-free is lost, and the Indicator says so in the same words.
    harness.wait_for_app("the Call Window closes", LOST_WINDOW_DEADLINE, |state| {
        state.surface("call-window").is_none()
    });
    let menu = indicator_says(&harness, "Connection to My Pixel lost");
    assert_eq!(
        MenuEntry::labels(&menu),
        [
            "Connection to My Pixel lost",
            "Open Dialer",
            "Turn calls off",
            "Connect your phone…"
        ]
    );
}

#[test]
fn with_the_phone_out_of_range_the_call_window_and_indicator_say_the_same() {
    let harness = connected();
    let _bob = on_a_call(&harness);
    harness.simulated_phone().disconnect();

    let state = wait_lost(&harness);
    assert_offers_nothing(&state);
    let menu = indicator_says(&harness, INDICATOR_LOST_BOB);
    assert_lost_menu(&menu, INDICATOR_LOST_BOB);
    assert_keys_do_nothing(&harness);
}

/// Alice live and Bob on hold when the Phone goes out of range: the Call
/// Window still lists Bob on hold, with neither Swap nor his Hang up, and
/// both come back with the Phone, with everything offered again.
#[test]
fn with_a_held_call_both_calls_stay_listed_and_neither_can_be_acted_on() {
    let harness = connected();
    let (_, bob) = on_a_call(&harness);
    let (_, alice) = alice_waits(&harness);
    harness
        .hold_and_answer()
        .expect("Hold & Answer is passed on");
    harness.wait_for_app("Alice is live and Bob on hold", DEADLINE, |state| {
        state_of(state, alice) == Some("active")
            && state_of(state, bob) == Some("held")
            && call_window(state, "held-call") == Some(bob.to_string())
    });

    harness.simulated_phone().disconnect();

    let state = wait_lost(&harness);
    assert_eq!(call_window(&state, "call"), Some(alice.to_string()));
    assert_eq!(call_window(&state, "held-call"), Some(bob.to_string()));
    assert_eq!(call_window(&state, "swap"), None);
    assert_offers_nothing(&state);
    assert_keys_do_nothing(&harness);
    for refused in [harness.swap(), harness.hold_and_answer()] {
        let error = refused.expect_err("nothing acts on the Calls while contact is lost");
        assert_eq!(error.message, LOST, "{error:?}");
    }
    harness.hang_up(bob);
    let error = harness.wait_for_app_error(DEADLINE);
    assert_eq!(error.message, LOST, "{error:?}");

    harness.simulated_phone().return_to_range();
    let state = harness.wait_for_app(
        "both Calls are back, with Swap and both Hang ups offered",
        RECONNECT_DEADLINE,
        |state| {
            state.connection_state == "connected"
                && call_window(state, "notice").is_none()
                && call_window(state, "held-call") == Some(bob.to_string())
                && call_window(state, "swap").as_deref() == Some("Swap")
        },
    );
    assert_eq!(state_of(&state, alice), Some("active"));
    assert_eq!(state_of(&state, bob), Some("held"));
    assert!(call_window(&state, "hang-up-shortcut").is_some(), "{state:#?}");
    assert!(call_window(&state, "hang-up-held-shortcut").is_some(), "{state:#?}");
    assert_eq!(call_window(&state, "hang-up"), None);
    assert_offers_tones(&state);
    // The Hang up refused while contact was lost was not kept for later.
    // The Phone's Calls have new paths since hands-free came back.
    let mut on_phone: Vec<String> = harness
        .simulated_phone()
        .calls_on_phone()
        .into_iter()
        .map(|(_, state)| state)
        .collect();
    on_phone.sort();
    assert_eq!(on_phone, ["active", "held"]);
}

/// Alice's Call waits during Bob's when hands-free is lost: her Banner goes,
/// as it can't be answered or declined from here, and the Indicator doesn't
/// say she is calling.
#[test]
fn with_a_waiting_call_its_banner_goes_and_it_cannot_be_answered() {
    let harness = connected();
    let (_, _bob) = on_a_call(&harness);
    let (_, alice) = alice_waits(&harness);
    lose_hands_free(&harness);

    // The Banner goes in the same change that shows the lost notice, not
    // only once the Phone link gives up on Alice's Call.
    let state = wait_lost(&harness);
    assert!(state.surface("banner").is_none(), "{state:#?}");
    assert!(!state.ringing, "{state:#?}");
    let menu = indicator_says(&harness, INDICATOR_LOST_BOB);
    assert_lost_menu(&menu, INDICATOR_LOST_BOB);
    let before = acts_on_calls(&harness.simulated_phone().wait_for_recorded(
        "what the Phone was asked so far",
        DEADLINE,
        |_| true,
    ));
    let errors = harness.app_errors().len();
    harness.answer(alice);
    harness.decline(alice);
    harness.wait_for_app("both are refused", DEADLINE, |_| {
        harness.app_errors().len() >= errors + 2
    });
    for error in &harness.app_errors()[errors..] {
        assert_eq!(error.message, LOST, "{error:?}");
    }
    harness.simulated_phone().recorded_holds(
        "the Phone is never asked to answer or decline Alice",
        HOLD,
        |recorded| acts_on_calls(recorded) == before,
    );
    let on_phone = harness.simulated_phone().calls_on_phone();
    assert!(
        on_phone.iter().any(|(_, state)| state == "waiting"),
        "Alice still waits on the Phone: {on_phone:?}"
    );
}

/// Contact comes back while the lost Call Window shows, after a Hang up was
/// refused: the Hang up isn't carried out late, and the Call Window offers
/// Hang up and Mute's hint again, so Hang up now ends the Call.
#[test]
fn when_contact_comes_back_hang_up_works_again_and_the_refused_one_is_not_kept() {
    let harness = connected();
    let (_, bob) = on_a_call(&harness);
    harness
        .simulated_phone()
        .enable_quirk("connect-profile-fails-once");
    if let Err(error) = harness.move_audio("phone") {
        panic!("MoveAudio(\"phone\") was refused: {error:?}");
    }
    wait_lost(&harness);
    harness.hang_up(bob);
    let error = harness.wait_for_app_error(DEADLINE);
    assert_eq!(error.message, LOST, "{error:?}");

    // Bob's Call may come back under a new id and path, if the Phone takes
    // longer than the Phone link waits: take both from what is shown.
    let state = harness.wait_for_app(
        "the Call is back live, with Hang up and Mute's hint offered again",
        DEADLINE,
        |state| {
            state.connection_state == "connected"
                && call_window(state, "notice").is_none()
                && call_window(state, "hang-up-shortcut").is_some()
                && call_window(state, "call")
                    .and_then(|call| call.parse().ok())
                    .is_some_and(|call| state_of(state, call) == Some("active"))
        },
    );
    assert_eq!(state.calls.len(), 1, "{state:#?}");
    assert_eq!(state.calls[0].number, BOB);
    assert_eq!(call_window(&state, "hang-up"), None);
    assert_offers_tones(&state);
    assert_eq!(
        call_window(&state, "mute-unavailable").as_deref(),
        Some(MUTE_UNAVAILABLE),
        "the audio is on the Phone, so Mute says why it is greyed out"
    );
    let on_phone = harness.simulated_phone().calls_on_phone();
    let [(bob_path, bob_state)] = on_phone.as_slice() else {
        panic!("the refused Hang up wasn't kept, so Bob's Call is on the Phone: {on_phone:?}");
    };
    assert_eq!(bob_state, "active");

    harness.activate();
    harness.wait_for_app("the Call Window has the keyboard", DEADLINE, |state| {
        call_window(state, "focused").as_deref() == Some("true")
    });
    harness.keyboard().press_with_control('h');
    harness
        .simulated_phone()
        .wait_for_recorded("Bob's Call is hung up", DEADLINE, |recorded| {
            recorded
                .iter()
                .any(|call| call.member == "Hangup" && call.path == *bob_path)
        });
}

/// With the keypad open and "12" typed when the Phone goes out of range,
/// and the same Call back when it returns, the Call Window has the toggle,
/// the entry, the keypad and "12" again, and the entry takes tones.
#[test]
fn when_the_phone_comes_back_the_open_keypad_and_typed_tones_are_as_they_were() {
    let harness = connected();
    let (_, bob) = on_a_call(&harness);
    open_keypad_with_tones(&harness);
    harness.simulated_phone().disconnect();
    assert_offers_nothing(&wait_lost(&harness));

    harness.simulated_phone().return_to_range();

    let (state, call) = wait_contact_back(&harness);
    assert_eq!(call, bob, "the same Call came back");
    assert_offers_tones(&state);
    assert_eq!(call_window(&state, "keypad").as_deref(), Some("shown"));
    assert_eq!(call_window(&state, "tones").as_deref(), Some("12"));
    assert_tones_usable(&harness, Some("12"));
}

/// With the keypad open and "12" typed when the Phone goes out of range, and
/// the Call back under a new id because the Phone took longer than the Phone
/// link waits for it, the Call Window starts afresh as for any new Call: the
/// keypad closed and no tones, but the toggle and the entry offered, and the
/// entry takes tones.
#[test]
fn when_the_call_comes_back_under_a_new_id_the_keypad_and_tones_start_afresh() {
    let harness = connected();
    let (_, bob) = on_a_call(&harness);
    open_keypad_with_tones(&harness);
    harness.simulated_phone().disconnect();
    assert_offers_nothing(&wait_lost(&harness));
    harness
        .simulated_phone()
        .return_to_range_after(PHONE_LINK_GIVES_UP);

    let (state, call) = wait_contact_back(&harness);
    assert_ne!(call, bob, "the Call came back under a new id");
    assert_offers_tones(&state);
    assert_eq!(call_window(&state, "keypad"), None, "{state:#?}");
    assert_eq!(call_window(&state, "tones"), None, "{state:#?}");
    assert_tones_usable(&harness, None);
}

const SCREEN_DEADLINE: Duration = Duration::from_secs(10);
/// Hang up's colour while it works.
const RED: [u8; 3] = [0xc0, 0x1c, 0x28];
/// The gap between the Call Window's rows.
const ROW_GAP: f64 = 8.0;
/// The most a control may move or change size and still be where it was.
const SAME_PLACE: f64 = 2.0;
/// How far below a button's top the rows are read: below its rounded corner,
/// above its label.
const EDGE_INSET: f64 = 3.0;
/// The space between Mute and the keypad toggle.
const MUTE_ROW_GAP: f64 = 12.0;

/// Where Hang up is, if the screen shows it working.
fn working_hang_up(screenshot: &Screenshot) -> Option<Region> {
    screenshot.region_where(|pixel| {
        pixel
            .iter()
            .zip(RED)
            .all(|(channel, wanted)| channel.abs_diff(wanted) <= 16)
    })
}

/// Where the controls are along the rows of Mute and Hang up, as left and
/// right edges, left to right, inside the Call Window.
struct Rows {
    controls: Vec<(f64, f64)>,
    hang_up: Vec<(f64, f64)>,
}

impl Rows {
    /// Reads the rows from `screenshot`, given where Hang up was and the
    /// Call Window's background colour. The window is anchored at the right
    /// of the screen and narrows when its shortcut hints go, so its left
    /// edge moves; its right edge doesn't.
    fn of(screenshot: &Screenshot, hang_up: Region, background: [u8; 3]) -> Self {
        let height = hang_up.bottom - hang_up.top;
        // Near the top of each button, above its label.
        let controls_y = hang_up.top - ROW_GAP - height + EDGE_INSET;
        let hang_up_y = hang_up.top + EDGE_INSET;
        // Not the screen to the left of the window, nor to its right.
        let in_window = |runs: Vec<(f64, f64)>| {
            runs.into_iter()
                .filter(|(left, right)| *left > hang_up.left - 100.0 && *right <= hang_up.right + 1.0)
                .collect()
        };
        Self {
            controls: in_window(screenshot.runs_along_row(controls_y, background, 6.0)),
            hang_up: in_window(screenshot.runs_along_row(hang_up_y, background, 6.0)),
        }
    }

    /// Checks Hang up's button ends where it did, at the window's right
    /// edge, and Mute is still half of it, as the keypad toggle's place in
    /// the row is kept: Mute neither stretches nor moves.
    fn assert_mute_and_hang_up_in_place(&self, hang_up_right: f64, when: &str) {
        let [(hang_left, hang_right)] = self.hang_up[..] else {
            panic!("expected one button in Hang up's row {when}: {self:?}");
        };
        assert!(
            (hang_right - hang_up_right).abs() <= SAME_PLACE,
            "Hang up's right edge moved {when}, from {hang_up_right} to {hang_right}: {self:?}"
        );
        let Some((mute_left, mute_right)) = self.controls.first().copied() else {
            panic!("no Mute {when}: {self:?}");
        };
        let half = (hang_right - hang_left - MUTE_ROW_GAP) / 2.0;
        assert!(
            (mute_left - hang_left).abs() <= SAME_PLACE
                && ((mute_right - mute_left) - half).abs() <= SAME_PLACE,
            "Mute is not half of Hang up's width {when}: {self:?}"
        );
    }
}

impl std::fmt::Debug for Rows {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "controls {:?}, Hang up {:?}",
            self.controls, self.hang_up
        )
    }
}

/// Nothing above Hang up resizes or moves when contact is lost or comes
/// back: Hang up stays on its row at the window's right edge, and Mute
/// stays half the row, as the keypad toggle's place in Mute's row is kept
/// while the toggle is hidden.
#[test]
fn losing_contact_and_getting_it_back_moves_neither_mute_nor_hang_up() {
    let harness = connected();
    on_a_call(&harness);
    let live = harness.screenshot_when("call-window-live", SCREEN_DEADLINE, |screenshot| {
        working_hang_up(screenshot).is_some()
    });
    let hang_up = working_hang_up(&live).expect("Hang up on screen");
    let background = live.colour_at(hang_up.left - 3.0, hang_up.top - ROW_GAP / 2.0);
    let before = Rows::of(&live, hang_up, background);
    assert_eq!(before.controls.len(), 2, "Mute and the toggle: {before:?}");
    before.assert_mute_and_hang_up_in_place(hang_up.right, "with contact");

    harness.simulated_phone().disconnect();
    wait_lost(&harness);
    let lost = harness.screenshot_when("call-window-lost", SCREEN_DEADLINE, |screenshot| {
        working_hang_up(screenshot).is_none()
    });
    let lost = Rows::of(&lost, hang_up, background);
    lost.assert_mute_and_hang_up_in_place(hang_up.right, "while contact is lost");
    // Mute's row has only Mute on screen: the toggle's place is kept empty.
    assert_eq!(lost.controls.len(), 1, "the toggle shows while lost: {lost:?}");

    harness.simulated_phone().return_to_range();
    wait_contact_back(&harness);
    let back = harness.screenshot_when("call-window-back", SCREEN_DEADLINE, |screenshot| {
        working_hang_up(screenshot).is_some()
    });
    let after = Rows::of(&back, hang_up, background);
    assert_eq!(after.controls.len(), 2, "Mute and the toggle: {after:?}");
    after.assert_mute_and_hang_up_in_place(hang_up.right, "once contact is back");
}

/// How long the Call Window may take to collapse to the Call Pill by
/// itself: 5 s after the Call connects, and a margin.
const COLLAPSE_DEADLINE: Duration = Duration::from_secs(9);

fn call_pill(state: &AppState, key: &str) -> Option<String> {
    state
        .surface("call-pill")
        .and_then(|pill| pill.get(key).cloned())
}

/// Ticket #201: the Desktop losing contact with the Call expands the
/// collapsed Call Window by itself, so the user sees why. Collapsed again by
/// hand, the Call Pill says the Call is on the Phone in place of a timer
/// that would mislead.
#[test]
fn lost_contact_expands_the_call_pill_and_collapsed_again_it_says_on_your_phone() {
    let harness = connected();
    on_a_call(&harness);
    harness.wait_for_app(
        "the Call Window collapses to the Call Pill",
        COLLAPSE_DEADLINE,
        |state| state.surface("call-pill").is_some() && state.surface("call-window").is_none(),
    );

    lose_hands_free(&harness);

    let state = wait_lost(&harness);
    assert_eq!(state.surface("call-pill"), None, "it expands by itself");

    harness.activate();
    harness.wait_for_app("the keybinding gives it the keyboard", DEADLINE, |state| {
        call_window(state, "focused").as_deref() == Some("true")
    });
    harness.keyboard().press(Key::Escape);
    let state = harness.wait_for_app("Escape collapses it", DEADLINE, |state| {
        state.surface("call-pill").is_some()
    });
    assert_eq!(call_pill(&state, "status").as_deref(), Some("On your phone"));
    assert_eq!(call_pill(&state, "mute"), None, "nothing to mute here");
}

