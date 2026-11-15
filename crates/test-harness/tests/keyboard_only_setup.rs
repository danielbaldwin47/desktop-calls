//! The keyboard-only suite's Setup Window cases (ticket #25): its list of
//! actions, and the rest of its cases, are in `keyboard_only.rs`.

use std::time::Duration;

use test_harness::{AppState, Harness, Key, RecordedMethodCall, vcards};

use super::keyboard_only::{
    DEADLINE, FIRST_RUN, Performed, SETUP_COPY, SETUP_REFUSED, SYNC_DEADLINE, focused, shown,
};

/// How long the app may take to start and open the Setup Window.
const STARTUP: Duration = Duration::from_secs(10);
/// How long pairing may take to connect.
const CONNECT_DEADLINE: Duration = Duration::from_secs(15);
/// Longer than the Phone's 30 s contacts prompt.
const PROMPT_TIMEOUT_DEADLINE: Duration = Duration::from_secs(40);
const PHONE_NAME: &str = "My Pixel";

fn setup_window<'a>(state: &'a AppState, key: &str) -> Option<&'a str> {
    shown(state, "setup-window", key)
}

/// Waits until the Setup Window's footer says Enter presses `button`: the
/// button with the keyboard.
fn wait_for_enter_on(harness: &Harness, button: &str, deadline: Duration) -> AppState {
    let enter = format!("Enter {button} ·");
    harness.wait_for_app(
        &format!("the Setup Window has the keyboard, on {button}"),
        deadline,
        |state| {
            focused(state, "setup-window")
                && setup_window(state, "keys").is_some_and(|keys| keys.starts_with(&enter))
        },
    )
}

/// Waits until the Problem card's only button, `button`, has the keyboard.
/// The footer still names the step's button then (#296), so this reads the
/// card's focus, which its focus ring shows.
fn wait_for_problem_button(harness: &Harness, button: &str) -> AppState {
    harness.wait_for_app(
        &format!("the Problem card's {button} has the keyboard"),
        DEADLINE,
        |state| {
            focused(state, "setup-problems")
                && shown(state, "setup-problems", "buttons") == Some(button)
        },
    )
}

fn wait_for_step(harness: &Harness, step: &str, deadline: Duration) -> AppState {
    harness.wait_for_app(&format!("the Setup Window is at {step:?}"), deadline, |state| {
        setup_window(state, "step") == Some(step) && focused(state, "setup-window")
    })
}

/// Audio settings that leave out hands-free, so the Setup Window offers to
/// set up audio.
const ROLES_WITHOUT_HANDS_FREE: &str = "\
monitor.bluez.properties = {
  bluez5.roles = [ a2dp_sink a2dp_source ]
}
";

fn sets_powered(recorded: &[RecordedMethodCall]) -> bool {
    recorded.iter().any(|call| {
        call.member == "Set" && call.arguments == "(\"org.bluez.Adapter1\", \"Powered\", <true>)"
    })
}

fn restarts_wireplumber(recorded: &[RecordedMethodCall]) -> bool {
    recorded.iter().any(|call| {
        call.member == "RestartUnit" && call.arguments == "(\"wireplumber.service\", \"replace\")"
    })
}

#[test]
fn first_run_in_the_setup_window_by_keys_alone() {
    let mut performed = Performed::by(FIRST_RUN);
    let harness = Harness::builder()
        .headless_screen()
        .phone_name(PHONE_NAME)
        .phone_unpaired()
        .bluetooth_off()
        .wireplumber_config("51-music.conf", ROLES_WITHOUT_HANDS_FREE)
        .hyprland_bindings("-- Keep only your personal keybinding overrides here.\n")
        .start();
    let phone = harness.simulated_phone();
    phone.set_pbap_folder("pb", &vcards::contacts(12, 0));
    phone.set_pbap_folder("cch", &vcards::call_history(5));
    phone.set_contacts_prompt("ask");

    wait_for_enter_on(&harness, "Turn on", STARTUP);
    harness.keyboard().press(Key::Return);
    harness
        .simulated_phone()
        .wait_for_recorded("Bluetooth is turned on", DEADLINE, sets_powered);
    performed.did("Turn Bluetooth on: Enter");

    // Set up audio takes no focus by itself: it is the button above the
    // Phone listed.
    wait_for_enter_on(&harness, "Pair", DEADLINE);
    harness.keyboard().press_with_shift(Key::Tab);
    wait_for_problem_button(&harness, "Set up audio");
    harness.keyboard().press(Key::Return);
    harness.simulated_phone().wait_for_recorded(
        "WirePlumber is restarted",
        DEADLINE,
        restarts_wireplumber,
    );
    harness.wait_for_app("no Problem is left", DEADLINE, |state| state.problems.is_empty());
    performed.did("Set up audio: Shift+Tab, Enter");

    let state = wait_for_enter_on(&harness, "Pair", DEADLINE);
    assert_eq!(setup_window(&state, "phones"), Some(PHONE_NAME));
    harness.keyboard().press(Key::Return);
    wait_for_step(&harness, "confirm", CONNECT_DEADLINE);
    performed.did("Pair the Phone listed: Enter");
    wait_for_enter_on(&harness, "Confirm", DEADLINE);
    harness.keyboard().press(Key::Return);
    harness.wait_for_app("the Phone is connected", CONNECT_DEADLINE, |state| {
        state.connection_state == "connected"
    });
    performed.did("Confirm the pairing code: Enter");

    wait_for_step(&harness, "contacts-prompt", CONNECT_DEADLINE);
    wait_for_step(&harness, "contacts-prompt-timed-out", PROMPT_TIMEOUT_DEADLINE);
    wait_for_enter_on(&harness, "Try again", DEADLINE);
    harness.keyboard().press(Key::Return);
    wait_for_step(&harness, "contacts-prompt", CONNECT_DEADLINE);
    let prompts = harness.simulated_phone().contacts_prompts();
    assert_eq!(prompts.len(), 2, "the Phone asks again: {prompts:?}");
    performed.did("Ask for Contacts again once the prompt timed out: Enter on Try again");
    harness.simulated_phone().answer_contacts_prompt("grant");

    wait_for_step(&harness, "done", SYNC_DEADLINE);
    wait_for_enter_on(&harness, "Add keybinding", DEADLINE);
    harness.keyboard().press(Key::Return);
    performed.did("Add the keybinding: Enter");
    wait_for_enter_on(&harness, "Confirm", DEADLINE);
    // The footer still names Confirm for Enter after Tab (#296); the
    // outcome shows Cancel was pressed.
    harness.keyboard().press(Key::Tab);
    harness.keyboard().press(Key::Return);
    wait_for_enter_on(&harness, "Add keybinding", DEADLINE);
    let bindings = std::fs::read_to_string(harness.directory().join("config/hypr/bindings.lua"))
        .unwrap_or_default();
    assert!(!bindings.contains("desktop-calls"), "{bindings}");
    performed.did("Cancel the keybinding's line: Tab, Enter on Cancel");
    harness.keyboard().press(Key::Return);
    let state = wait_for_enter_on(&harness, "Confirm", DEADLINE);
    let line = setup_window(&state, "line").unwrap_or_default().to_owned();
    assert!(!line.is_empty(), "the window shows the line: {state:#?}");
    harness.keyboard().press(Key::Return);
    harness.wait_for_app("the keybinding is added", DEADLINE, |state| {
        setup_window(state, "buttons") == Some("Done")
    });
    let bindings = std::fs::read_to_string(harness.directory().join("config/hypr/bindings.lua"))
        .unwrap_or_default();
    assert!(bindings.contains(&line), "{bindings}");
    performed.did("Confirm the keybinding's line: Enter");
    wait_for_enter_on(&harness, "Done", DEADLINE);
    harness.keyboard().press(Key::Return);
    harness.wait_for_app("the Setup Window is closed", DEADLINE, |state| {
        state.surface("setup-window").is_none()
    });
    performed.did("Finish: Enter on Done");
    performed.all_done();
}

#[test]
fn a_refused_phone_then_skipped_contacts_by_keys_alone() {
    let mut performed = Performed::by(SETUP_REFUSED);
    let harness = Harness::builder()
        .headless_screen()
        .phone_name(PHONE_NAME)
        .phone_unpaired()
        .start();
    let phone = harness.simulated_phone();
    phone.set_hands_free("refused");
    phone.set_contacts_prompt("ask");

    wait_for_enter_on(&harness, "Pair", STARTUP);
    harness.keyboard().press(Key::Return);
    wait_for_enter_on(&harness, "Confirm", CONNECT_DEADLINE);
    // The footer still names Confirm for Enter after Tab (#296); the
    // outcome shows Reject was pressed.
    harness.keyboard().press(Key::Tab);
    harness.keyboard().press(Key::Return);
    wait_for_step(&harness, "choose", CONNECT_DEADLINE);
    let outcome = harness.simulated_phone().pairing_requests().pop().map(|(_, outcome)| outcome);
    assert_eq!(outcome.as_deref(), Some("rejected"));
    performed.did("Reject the pairing code: Tab, Enter on Reject");

    wait_for_enter_on(&harness, "Pair", CONNECT_DEADLINE);
    harness.keyboard().press(Key::Return);
    wait_for_enter_on(&harness, "Confirm", CONNECT_DEADLINE);
    harness.keyboard().press(Key::Return);
    wait_for_step(&harness, "phone-calls-off", CONNECT_DEADLINE);
    wait_for_enter_on(&harness, "Try again", DEADLINE);
    // The user switches Phone calls on, and tries again.
    harness.simulated_phone().set_hands_free("when-asked");
    harness.keyboard().press(Key::Return);
    harness.wait_for_app("the Phone is connected", CONNECT_DEADLINE, |state| {
        state.connection_state == "connected"
    });
    performed.did("Try again once the Phone refused hands-free: Enter");

    // The Phone leaves while it asks for Contacts, and comes back.
    wait_for_step(&harness, "contacts-prompt", CONNECT_DEADLINE);
    harness.simulated_phone().disconnect();
    wait_for_step(&harness, "contacts-phone-away", DEADLINE);
    harness.simulated_phone().connect();
    wait_for_enter_on(&harness, "Continue", CONNECT_DEADLINE);
    harness.keyboard().press(Key::Return);
    wait_for_step(&harness, "contacts-prompt", CONNECT_DEADLINE);
    let prompts = harness.simulated_phone().contacts_prompts();
    assert_eq!(prompts.len(), 2, "the Phone asks again: {prompts:?}");
    performed.did("Carry on once the Phone is back: Enter on Continue");

    wait_for_step(&harness, "contacts-prompt-timed-out", PROMPT_TIMEOUT_DEADLINE);
    let state = wait_for_enter_on(&harness, "Try again", DEADLINE);
    let keys = setup_window(&state, "keys").unwrap_or_default();
    assert!(keys.split(" · ").any(|shown| shown == "Tab Next button"), "{keys:?}");
    harness.keyboard().press(Key::Tab);
    // The footer still names Try again for Enter once Tab has moved to
    // Skip (#296), so the outcome shows which button Enter pressed.
    harness.keyboard().press(Key::Return);
    wait_for_step(&harness, "done", DEADLINE);
    let prompts = harness.simulated_phone().contacts_prompts();
    assert_eq!(prompts.len(), 2, "the Phone is not asked again: {prompts:?}");
    performed.did("Skip Contacts once the prompt timed out: Tab, Enter on Skip");
    performed.all_done();
}

#[test]
fn a_setup_window_fix_that_copies_a_command_by_keys_alone() {
    const INSTALL: &str = "sudo pacman -S --needed bluez-obex";
    let mut performed = Performed::by(SETUP_COPY);
    let harness = Harness::builder()
        .headless_screen()
        .phone_name(PHONE_NAME)
        .phone_unpaired()
        .without_contacts_support()
        .start();
    wait_for_enter_on(&harness, "Pair", STARTUP);
    harness.keyboard().press_with_shift(Key::Tab);
    wait_for_problem_button(&harness, "Copy command");
    harness.keyboard().press(Key::Return);
    harness.wait_for_app("the Setup Window says it copied", DEADLINE, |state| {
        shown(state, "setup-problems", "copied") == Some(INSTALL)
    });
    assert_eq!(harness.clipboard_text(), INSTALL);
    performed.did("Copy the install command: Shift+Tab, Enter");

    let state = harness.app_state();
    let keys = setup_window(&state, "keys").unwrap_or_default();
    assert!(keys.split(" · ").any(|shown| shown == "Esc Close"), "{keys:?}");
    harness.keyboard().press(Key::Escape);
    harness.wait_for_app("Escape closes the Setup Window", DEADLINE, |state| {
        state.surface("setup-window").is_none()
    });
    performed.did("Close the Setup Window: Escape");
    performed.all_done();
}
