//! Scenarios: the Setup Window's later steps, once the Phone is paired and
//! connected for Calls (ticket #22; polish scorecard items 4, 5, 7 and 8).
//! The window warns that the Phone is about to ask whether to share
//! Contacts, counts down with the Phone's 30 s prompt, offers Try again
//! when nobody answers, goes on without Contacts when the Phone refuses,
//! shows the sync's progress, waits for a Phone that leaves part way and
//! carries on once it is back, and ends on the finish page with the Dialer
//! keybinding, which it adds to the Hyprland config only once the user
//! confirms the exact line. Every step is taken with synthetic keys alone.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use test_harness::vcards;
use test_harness::{AppState, Harness, Key};

/// How long the app may take to start and show the Setup Window by itself.
const STARTUP: Duration = Duration::from_secs(10);
/// How long the app may take to notice a change.
const DEADLINE: Duration = Duration::from_secs(5);
/// How long pairing and connecting may take after Confirm.
const CONNECT_DEADLINE: Duration = Duration::from_secs(15);
/// How long the warning and the Phone's prompt may take to show: the
/// warning stays 3 s, and the app takes the Phone to be asking once
/// `CreateSession` has waited 2 s.
const PROMPT_DEADLINE: Duration = Duration::from_secs(10);
/// How long a sync of 312 Contacts and 200 Calls may take at the Pixel's
/// pace: 9.4 s for Contacts alone.
const SYNC_DEADLINE: Duration = Duration::from_secs(25);
/// How long an unanswered prompt takes to time out, and the app to say so:
/// the Phone's 30 s.
const PROMPT_TIMEOUT_DEADLINE: Duration = Duration::from_secs(40);
/// How far the window's deadline may be from the Phone's own: the app
/// takes the Phone to be asking once `CreateSession` has waited 2 s, and
/// counts 30 s from when the session was asked for.
const COUNTDOWN_TOLERANCE_MS: u64 = 1_500;
const PHONE_NAME: &str = "My Pixel";
const SYNCED: &str = "Synced 312 Contacts and 200 recent Calls.";

/// A fresh install, with the Phone nearby holding 312 Contacts and 200
/// Calls, which it gives at the Pixel's pace, and asking whether to share
/// them as Android does.
fn fresh_install() -> Harness {
    let harness = Harness::builder()
        .headless_screen()
        .phone_name(PHONE_NAME)
        .phone_unpaired()
        .start();
    let phone = harness.simulated_phone();
    phone.set_pbap_folder("pb", &vcards::contacts(312, 0));
    phone.set_pbap_folder("cch", &vcards::call_history(200));
    // The Pixel gave 319 Contacts in 9.7 s, and 200 Calls in under 2 s.
    phone.set_pull_delay_per_vcard(Duration::from_millis(30), Duration::from_millis(5));
    phone.set_contacts_prompt("ask");
    harness
}

fn setup_window(state: &AppState) -> Option<&BTreeMap<String, String>> {
    state.surface("setup-window")
}

fn field<'state>(state: &'state AppState, name: &str) -> Option<&'state str> {
    setup_window(state)?.get(name).map(String::as_str)
}

/// Waits until the Setup Window is at `step` with keyboard focus, and
/// returns the app's state.
fn wait_for_step(harness: &Harness, step: &str, deadline: Duration) -> AppState {
    harness.wait_for_app(
        &format!("the Setup Window is at step {step:?} with keyboard focus"),
        deadline,
        |state| field(state, "step") == Some(step) && field(state, "focused") == Some("true"),
    )
}

/// Pairs the Phone through the Setup Window, by keys alone.
fn pair_through_the_window(harness: &Harness) {
    harness.wait_for_app("the Setup Window lists the Phone", STARTUP, |state| {
        field(state, "phones") == Some(PHONE_NAME) && field(state, "focused") == Some("true")
    });
    harness.keyboard().press(Key::Return);
    wait_for_step(harness, "confirm", CONNECT_DEADLINE);
    harness.keyboard().press(Key::Return);
    harness.wait_for_app("ConnectionState is connected", CONNECT_DEADLINE, |state| {
        state.connection_state == "connected"
    });
}

/// Waits until the Phone's prompt shows, and the window counts down.
fn wait_for_prompt(harness: &Harness) -> AppState {
    wait_for_step(harness, "contacts-warning", CONNECT_DEADLINE);
    wait_for_step(harness, "contacts-prompt", PROMPT_DEADLINE)
}

fn create_sessions(harness: &Harness) -> usize {
    harness
        .simulated_phone()
        .wait_for_recorded("the record", DEADLINE, |_| true)
        .iter()
        .filter(|call| call.member == "CreateSession")
        .count()
}

/// The Hyprland config in the scenario's temporary home.
fn bindings_file(harness: &Harness) -> PathBuf {
    harness.directory().join("config/hypr/bindings.lua")
}

const BINDINGS: &str = "-- Keep only your personal keybinding overrides here.\n\
                        o.bind(\"SUPER + SHIFT + R\", \"SSH\", \"alacritty -e ssh your-server\")\n";

/// Omarchy's Hyprland config, with a binding of the user's own.
fn with_hyprland_config(harness: &Harness) {
    write_bindings(harness, BINDINGS);
}

fn write_bindings(harness: &Harness, bindings: &str) {
    let file = bindings_file(harness);
    let written = file
        .parent()
        .map(std::fs::create_dir_all)
        .and_then(Result::ok)
        .and_then(|()| std::fs::write(&file, bindings).ok());
    assert!(written.is_some(), "write {}", file.display());
}

fn bindings(harness: &Harness) -> String {
    let file = bindings_file(harness);
    match std::fs::read_to_string(&file) {
        Ok(bindings) => bindings,
        Err(error) => panic!("read {}: {error}", file.display()),
    }
}

/// Paired, granted, synced: the finish page, offering the keybinding.
fn at_the_finish_page_offering_the_keybinding(harness: &Harness) -> AppState {
    with_hyprland_config(harness);
    pair_through_the_window(harness);
    wait_for_prompt(harness);
    harness.simulated_phone().answer_contacts_prompt("grant");
    wait_for_step(harness, "done", SYNC_DEADLINE);
    harness.wait_for_app("the finish page offers the keybinding", DEADLINE, |state| {
        field(state, "buttons") == Some("Add keybinding\nDone")
            && field(state, "focused") == Some("true")
    })
}

/// Adds the keybinding from the finish page: Add keybinding, then Confirm.
/// Returns the line shown.
fn add_the_keybinding(harness: &Harness) -> String {
    harness.keyboard().press(Key::Return);
    let state = harness.wait_for_app("the window shows the line", DEADLINE, |state| {
        field(state, "line").is_some() && field(state, "buttons") == Some("Confirm\nCancel")
    });
    let line = field(&state, "line").unwrap_or_default().to_owned();
    harness.keyboard().press(Key::Return);
    harness.wait_for_app("the keybinding is added", DEADLINE, |state| {
        field(state, "buttons") == Some("Done")
            && field(state, "instructions").is_some_and(|text| {
                text.starts_with("Place your first Call: press Super + Alt + C")
            })
    });
    line
}

/// The command a binding line runs: its last Lua string.
fn command_in(line: &str) -> String {
    let Some((_, inner)) = line
        .strip_suffix("\")")
        .and_then(|rest| rest.rsplit_once(", \""))
    else {
        panic!("not a binding: {line}");
    };
    inner.replace("\\\"", "\"").replace("\\\\", "\\")
}

#[test]
fn granted_the_window_warns_counts_down_with_the_phone_and_reports_what_was_synced() {
    let harness = fresh_install();
    pair_through_the_window(&harness);

    let state = wait_for_step(&harness, "contacts-warning", CONNECT_DEADLINE);
    assert_eq!(field(&state, "heading"), Some("Your phone is about to ask"));
    assert!(
        harness.simulated_phone().contacts_prompts().is_empty(),
        "the warning comes before the prompt"
    );

    let state = wait_for_step(&harness, "contacts-prompt", PROMPT_DEADLINE);
    let countdown: u64 = field(&state, "countdown")
        .and_then(|countdown| countdown.parse().ok())
        .unwrap_or_else(|| panic!("no countdown: {state:?}"));
    let deadline: u64 = field(&state, "deadline")
        .and_then(|deadline| deadline.parse().ok())
        .unwrap_or_else(|| panic!("no deadline: {state:?}"));
    let prompts = harness.simulated_phone().contacts_prompts();
    assert_eq!(prompts.len(), 1, "{prompts:?}");
    assert_eq!(prompts[0].outcome, "waiting");
    assert!(
        deadline.abs_diff(prompts[0].deadline) <= COUNTDOWN_TOLERANCE_MS,
        "the countdown ends at {deadline}, the Phone's prompt at {}",
        prompts[0].deadline
    );
    harness.wait_for_app("the countdown runs", DEADLINE, |state| {
        field(state, "countdown")
            .and_then(|shown| shown.parse::<u64>().ok())
            .is_some_and(|shown| shown < countdown)
    });

    harness.simulated_phone().answer_contacts_prompt("grant");

    // The Call History lands in under 2 s; Contacts take about 9 s more.
    let state = harness.wait_for_app(
        "Recent Calls are done while Contacts are read",
        PROMPT_DEADLINE,
        |state| {
            field(state, "progress").is_some_and(|progress| {
                progress.starts_with("✓ Recent Calls\n312 Contacts… about ")
            })
        },
    );
    assert_eq!(field(&state, "step"), Some("contacts-syncing"));
    let state = wait_for_step(&harness, "done", SYNC_DEADLINE);
    assert_eq!(field(&state, "summary"), Some(SYNCED));
    assert_eq!(state.contacts.state, "ready");
    assert_eq!(
        harness.simulated_phone().contacts_prompts()[0].outcome,
        "granted"
    );
}

#[test]
fn an_ignored_prompt_runs_the_countdown_to_zero_and_try_again_asks_again() {
    let harness = fresh_install();
    pair_through_the_window(&harness);
    wait_for_prompt(&harness);

    let state = wait_for_step(
        &harness,
        "contacts-prompt-timed-out",
        PROMPT_TIMEOUT_DEADLINE,
    );
    assert_eq!(field(&state, "countdown"), Some("0"));
    assert_eq!(field(&state, "buttons"), Some("Try again\nSkip"));
    let prompts = harness.simulated_phone().contacts_prompts();
    assert_eq!(prompts.len(), 1, "{prompts:?}");
    assert_eq!(prompts[0].outcome, "timed-out");
    let sessions_before = create_sessions(&harness);

    // Try again has keyboard focus.
    harness.keyboard().press(Key::Return);

    wait_for_step(&harness, "contacts-prompt", PROMPT_DEADLINE);
    let prompts = harness.simulated_phone().contacts_prompts();
    assert_eq!(prompts.len(), 2, "the Phone asks again: {prompts:?}");
    assert_eq!(prompts[1].outcome, "waiting");
    assert!(create_sessions(&harness) > sessions_before);
    harness.simulated_phone().answer_contacts_prompt("grant");
    let state = wait_for_step(&harness, "done", SYNC_DEADLINE);
    assert_eq!(field(&state, "summary"), Some(SYNCED));
}

/// The Phone goes out of range: the window says so, never that anything
/// synced, and Contacts stay not synced. Returns the app's state.
fn phone_leaves(harness: &Harness) -> AppState {
    harness.simulated_phone().disconnect();
    let state = harness.wait_for_app(
        "the Setup Window says the Phone disconnected",
        DEADLINE,
        |state| field(state, "step") == Some("contacts-phone-away"),
    );
    assert_eq!(
        field(&state, "heading"),
        Some(format!("{PHONE_NAME} disconnected").as_str())
    );
    assert_eq!(field(&state, "buttons"), Some("Close"));
    let shown = format!("{:?}", setup_window(&state));
    assert!(!shown.contains("Synced"), "nothing synced: {shown}");
    assert_eq!(state.contacts.state, "unavailable");
    assert_eq!(state.contacts.reason, "not-synced");
    state
}

/// The Phone comes back: the window offers to carry on, and Continue has
/// keyboard focus.
fn phone_comes_back(harness: &Harness) {
    harness.simulated_phone().connect();
    let state = harness.wait_for_app(
        "the Setup Window offers to carry on",
        CONNECT_DEADLINE,
        |state| {
            field(state, "heading") == Some(format!("{PHONE_NAME} is back").as_str())
                && field(state, "focused") == Some("true")
        },
    );
    assert_eq!(field(&state, "buttons"), Some("Continue\nClose"));
}

#[test]
fn a_phone_leaving_while_it_asks_is_waited_for_and_continue_asks_again() {
    let harness = fresh_install();
    pair_through_the_window(&harness);
    wait_for_prompt(&harness);

    phone_leaves(&harness);
    phone_comes_back(&harness);
    let prompts_before = harness.simulated_phone().contacts_prompts().len();

    harness.keyboard().press(Key::Return);

    wait_for_step(&harness, "contacts-prompt", PROMPT_DEADLINE);
    assert!(harness.simulated_phone().contacts_prompts().len() > prompts_before);
    harness.simulated_phone().answer_contacts_prompt("grant");
    let state = wait_for_step(&harness, "done", SYNC_DEADLINE);
    assert_eq!(field(&state, "summary"), Some(SYNCED));
    assert_eq!(state.contacts.state, "ready");
}

#[test]
fn a_phone_leaving_after_the_recent_calls_land_leaves_contacts_not_synced() {
    let harness = fresh_install();
    pair_through_the_window(&harness);
    wait_for_prompt(&harness);
    harness.simulated_phone().answer_contacts_prompt("grant");
    harness.wait_for_app(
        "Recent Calls are done while Contacts are read",
        PROMPT_DEADLINE,
        |state| {
            field(state, "progress")
                .is_some_and(|progress| progress.starts_with("✓ Recent Calls\n312 Contacts…"))
        },
    );

    phone_leaves(&harness);
}

#[test]
fn denied_the_window_says_calls_still_work_and_goes_on_to_the_finish_page() {
    let harness = fresh_install();
    pair_through_the_window(&harness);
    wait_for_prompt(&harness);

    harness.simulated_phone().answer_contacts_prompt("deny");

    let state = wait_for_step(&harness, "done", DEADLINE);
    let summary = field(&state, "summary").unwrap_or_default();
    assert!(summary.contains("Calls still work"), "{summary}");
    assert!(
        summary.contains("turn on Contacts and call history"),
        "how to allow it later: {summary}"
    );
    assert!(
        field(&state, "instructions").is_some_and(|text| text.contains("Place your first Call")),
        "{state:?}"
    );
    assert_eq!(state.contacts.state, "denied");
}

#[test]
fn the_keybinding_is_written_exactly_as_shown_once_confirmed_and_not_at_all_when_declined() {
    let harness = fresh_install();
    let state = at_the_finish_page_offering_the_keybinding(&harness);
    assert!(
        field(&state, "instructions").is_some_and(|text| text.contains("Super + Alt + C")),
        "{state:?}"
    );
    assert!(
        field(&state, "tip").is_some_and(|tip| tip.contains("pin Desktop Calls")),
        "{state:?}"
    );

    // Add keybinding shows the exact line, and writes nothing yet.
    harness.keyboard().press(Key::Return);
    let state = harness.wait_for_app("the window shows the line", DEADLINE, |state| {
        field(state, "buttons") == Some("Confirm\nCancel")
            && field(state, "focused") == Some("true")
    });
    let line = field(&state, "line").unwrap_or_default().to_owned();
    assert!(
        line.starts_with(r#"o.bind("SUPER + ALT + C", "Desktop Calls", ""#),
        "{line}"
    );
    assert_eq!(bindings(&harness), BINDINGS, "nothing written yet");

    // Cancel declines, and writes nothing.
    harness.keyboard().press(Key::Tab);
    harness.keyboard().press(Key::Return);
    harness.wait_for_app("the finish page offers it again", DEADLINE, |state| {
        field(state, "buttons") == Some("Add keybinding\nDone")
            && field(state, "focused") == Some("true")
    });
    assert_eq!(bindings(&harness), BINDINGS, "declining writes nothing");

    let added = add_the_keybinding(&harness);
    assert_eq!(added, line, "the same line both times");
    assert_eq!(
        bindings(&harness),
        format!("{BINDINGS}{line}\n"),
        "exactly the line shown, at the end"
    );
}

#[test]
fn keys_the_user_already_bound_are_left_alone_and_the_finish_page_names_the_command() {
    let harness = fresh_install();
    let config =
        format!("{BINDINGS}o.bind(\"super + alt + c\", \"Calculator\", \"gnome-calculator\")\n");
    write_bindings(&harness, &config);
    pair_through_the_window(&harness);
    wait_for_prompt(&harness);
    harness.simulated_phone().answer_contacts_prompt("grant");
    wait_for_step(&harness, "done", SYNC_DEADLINE);

    let state = harness.wait_for_app("the finish page names the clash", DEADLINE, |state| {
        field(state, "instructions")
            .is_some_and(|text| text.contains("Super + Alt + C already does something else"))
    });
    assert_eq!(field(&state, "buttons"), Some("Done"), "{state:?}");
    assert!(
        field(&state, "instructions").is_some_and(|text| text.contains("desktop-calls")),
        "the command to bind by hand: {state:?}"
    );
    assert_eq!(bindings(&harness), config, "the config is left alone");
}

#[test]
fn scenario_1_pair_sync_open_the_dialer_with_the_keybinding_and_place_a_call() {
    let harness = fresh_install();
    let state = at_the_finish_page_offering_the_keybinding(&harness);
    assert_eq!(field(&state, "summary"), Some(SYNCED));
    let line = add_the_keybinding(&harness);
    harness.keyboard().press(Key::Return);
    harness.wait_for_app("the Setup Window is closed", DEADLINE, |state| {
        setup_window(state).is_none()
    });

    // The keybinding runs its command through the shell, as Hyprland does.
    let output = harness
        .external_command("sh")
        .arg("-c")
        .arg(command_in(&line))
        .output()
        .expect("run the keybinding's command");
    assert!(output.status.success(), "{output:?}");
    harness.wait_for_app("the Dialer has keyboard focus", DEADLINE, |state| {
        state
            .surface("dialer")
            .and_then(|dialer| dialer.get("focused"))
            .is_some_and(|focused| focused == "true")
    });
    harness.keyboard().type_text("07700 900888");
    harness.wait_for_app("the Dialer offers to Call it", DEADLINE, |state| {
        state
            .surface("dialer")
            .and_then(|dialer| dialer.get("row-0"))
            .is_some_and(|row| row == "Call 07700 900888")
    });
    harness.keyboard().press(Key::Return);
    harness.simulated_phone().wait_for_recorded(
        "the Phone is asked to dial the number",
        DEADLINE,
        |calls| {
            calls
                .iter()
                .any(|call| call.member == "Dial" && call.arguments == "(\"07700900888\",)")
        },
    );
}
