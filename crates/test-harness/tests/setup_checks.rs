//! Scenarios: the setup checks find what the Desktop lacks for Calls
//! (Bluetooth off or blocked, Contacts support not installed, audio not set
//! up, BlueZ's own experimental hands-free on), say so in plain words in
//! the Setup Window, in `Problems` and in the Indicator, and put right what
//! the Desktop can, once the user asks (ticket #23; polish scorecard items
//! 4, 5 and 7). Every button is pressed with synthetic keys alone.
//!
//! Whatever the audio settings say, the Phone link keeps asking each
//! AudioGateway to keep Call audio on the Phone (`RejectSCO`) while no
//! Call's audio is on the Desktop: `telephony_restart`'s
//! `a_restart_while_idle_comes_back_connected_and_keeps_audio_on_the_phone`
//! holds that after a restart, and the last scenario here after the restart
//! that setting up audio does.

use std::collections::BTreeMap;
use std::time::Duration;

use test_harness::{AppState, Harness, Key, MenuEntry, RecordedMethodCall};

/// How long the app may take to start and show the Setup Window by itself.
const STARTUP: Duration = Duration::from_secs(10);
/// How long the app may take to notice a change.
const DEADLINE: Duration = Duration::from_secs(5);
/// How long nothing may be written before the user asks.
const BEFORE_CONSENT: Duration = Duration::from_secs(1);
const PHONE_NAME: &str = "My Pixel";
const UNBLOCK: &str = "rfkill unblock bluetooth";
const INSTALL: &str = "sudo pacman -S --needed bluez-obex";
/// The drop-in setting up audio writes.
const DROP_IN: &str = "60-desktop-calls.conf";
/// Audio settings that leave out hands-free: the roles lack `hfp_hf`, and
/// nothing turns auto-connect or keeping audio on the Phone on.
const ROLES_WITHOUT_HANDS_FREE: &str = "\
# Only music, please.
monitor.bluez.properties = {
  bluez5.roles = [ a2dp_sink a2dp_source ]
  bluez5.enable-msbc = true
}
";
/// What setting up audio leaves in [`ROLES_WITHOUT_HANDS_FREE`]'s file.
const ROLES_MOVED: &str = "\
# Only music, please.
monitor.bluez.properties = {
  # Moved to 60-desktop-calls.conf by Desktop Calls: bluez5.roles = [ a2dp_sink a2dp_source ]
  bluez5.enable-msbc = true
}
";
/// What setting up audio writes after [`ROLES_WITHOUT_HANDS_FREE`]: each
/// property it needed to change, once.
const SET_UP: &str = "\
# Written by Desktop Calls when you chose Set up audio: hands-free with
# your Phone connects by itself, and a Call's audio stays on the Phone
# until you move it here. Each of these is set in this file alone.
monitor.bluez.properties = {
  bluez5.roles = [ a2dp_sink a2dp_source hfp_hf ]
  bluez5.telephony.default-reject-sco = true
}
monitor.bluez.rules = [
  {
    matches = [ { device.name = \"~bluez_card.*\" } ]
    actions = {
      update-props = {
        bluez5.auto-connect = [ hfp_hf ]
      }
    }
  }
]
";

fn problems_surface(state: &AppState) -> Option<&BTreeMap<String, String>> {
    state.surface("setup-problems")
}

fn field<'state>(state: &'state AppState, name: &str) -> Option<&'state str> {
    problems_surface(state)?.get(name).map(String::as_str)
}

fn kinds(state: &AppState) -> Vec<&str> {
    state
        .problems
        .iter()
        .map(|problem| problem.kind.as_str())
        .collect()
}

/// Waits until the Setup Window shows exactly `kinds` as the Desktop's
/// Problems, the first one's button with keyboard focus.
fn wait_for_problems(harness: &Harness, kinds: &str, deadline: Duration) -> AppState {
    harness.wait_for_app(
        &format!("the Setup Window shows the Problems {kinds:?}, with focus"),
        deadline,
        |state| {
            field(state, "problems") == Some(kinds)
                && state
                    .surface("setup-window")
                    .and_then(|window| window.get("focused"))
                    .is_some_and(|focused| focused == "true")
        },
    )
}

fn no_problems_shown(harness: &Harness) -> AppState {
    harness.wait_for_app(
        "the Setup Window shows no Problem, and Problems is empty",
        DEADLINE,
        |state| {
            state.surface("setup-window").is_some()
                && problems_surface(state).is_none()
                && state.problems.is_empty()
        },
    )
}

fn sets_powered(recorded: &[RecordedMethodCall]) -> bool {
    recorded.iter().any(|call| {
        call.interface == "org.freedesktop.DBus.Properties"
            && call.member == "Set"
            && call.path == "/org/bluez/hci0"
            && call.arguments == "(\"org.bluez.Adapter1\", \"Powered\", <true>)"
    })
}

fn restarts_wireplumber(recorded: &[RecordedMethodCall]) -> bool {
    recorded.iter().any(|call| {
        call.bus == "session"
            && call.interface == "org.freedesktop.systemd1.Manager"
            && call.member == "RestartUnit"
            && call.arguments == "(\"wireplumber.service\", \"replace\")"
    })
}

fn rejects_audio(recorded: &[RecordedMethodCall]) -> usize {
    recorded
        .iter()
        .filter(|call| {
            call.member == "Set"
                && call.arguments.contains("RejectSCO")
                && call.arguments.contains("true")
        })
        .count()
}

/// No text the user can read carries a D-Bus name or error.
fn no_raw_dbus_text(texts: &[&str]) {
    for text in texts {
        for raw in ["org.", "Error", "Telephony1", "Adapter1", "obex", "RejectSCO"] {
            assert!(!text.contains(raw), "{raw:?} shows in {text:?}");
        }
    }
}

#[test]
fn bluetooth_off_shows_turn_on_and_pressing_it_turns_bluetooth_on() {
    let harness = Harness::builder()
        .headless_screen()
        .phone_name(PHONE_NAME)
        .phone_unpaired()
        .bluetooth_off()
        .start();

    let state = wait_for_problems(&harness, "bluetooth-off", STARTUP);
    assert_eq!(field(&state, "buttons"), Some("Turn on"));
    assert_eq!(kinds(&state), ["bluetooth-off"]);
    harness.simulated_phone().recorded_holds(
        "nothing turns Bluetooth on before the user asks",
        BEFORE_CONSENT,
        |recorded| !sets_powered(recorded),
    );
    // Enter presses Turn on, so the key hint no longer says Enter pairs.
    harness.wait_for_app(
        "the key hint says Enter presses the highlighted button",
        DEADLINE,
        |state| {
            window_field(state, "keys")
                == Some(
                    "Enter presses the highlighted button; Tab moves between buttons; Escape closes",
                )
        },
    );

    harness.keyboard().press(Key::Return);

    harness
        .simulated_phone()
        .wait_for_recorded("Bluetooth is turned on", DEADLINE, sets_powered);
    no_problems_shown(&harness);
    // With Bluetooth on, the window looks for Phones after all.
    harness.wait_for_app("the Setup Window lists the Phone", DEADLINE, |state| {
        state
            .surface("setup-window")
            .and_then(|window| window.get("phones"))
            .is_some_and(|phones| phones == PHONE_NAME)
    });
}

#[test]
fn bluetooth_blocked_explains_how_to_unblock_it() {
    let harness = Harness::builder()
        .headless_screen()
        .phone_unpaired()
        .bluetooth_blocked()
        .start();

    let state = wait_for_problems(&harness, "bluetooth-blocked", STARTUP);
    assert_eq!(field(&state, "buttons"), Some("Copy command"));
    assert_eq!(field(&state, "commands"), Some(UNBLOCK));
    let problem = &state.problems[0];
    assert_eq!(problem.action, UNBLOCK);
    assert!(
        problem.message.contains("airplane mode"),
        "{}",
        problem.message
    );
}

#[test]
fn bluetooth_turned_on_elsewhere_while_the_window_is_open_clears_the_problem() {
    let harness = Harness::builder()
        .headless_screen()
        .phone_name(PHONE_NAME)
        .phone_unpaired()
        .bluetooth_off()
        .start();
    wait_for_problems(&harness, "bluetooth-off", STARTUP);

    harness.set_bluetooth_power("on");

    no_problems_shown(&harness);
    harness.wait_for_app("the Setup Window lists the Phone", DEADLINE, |state| {
        state
            .surface("setup-window")
            .and_then(|window| window.get("phones"))
            .is_some_and(|phones| phones == PHONE_NAME)
    });
    harness.simulated_phone().recorded_holds(
        "the app never turned Bluetooth on itself",
        Duration::ZERO,
        |recorded| !sets_powered(recorded),
    );
}

/// Waits until the Setup Window lists the Phone nearby, with focus.
fn wait_for_phone_listed(harness: &Harness) -> AppState {
    harness.wait_for_app("the Phone is listed, with focus", STARTUP, |state| {
        state.surface("setup-window").is_some_and(|window| {
            window.get("step").is_some_and(|step| step == "choose")
                && window
                    .get("phones")
                    .is_some_and(|phones| !phones.is_empty())
                && window.get("focused").is_some_and(|focused| focused == "true")
        })
    })
}

/// Moves keyboard focus back from the step to Set up audio, which takes
/// none by itself, and presses it.
fn press_set_up_audio(harness: &Harness) {
    harness.keyboard().press_with_shift(Key::Tab);
    harness.wait_for_app("Set up audio has focus", DEADLINE, |state| {
        field(state, "focused") == Some("true")
    });
    harness.keyboard().press(Key::Return);
}

fn window_field<'state>(state: &'state AppState, name: &str) -> Option<&'state str> {
    state.surface("setup-window")?.get(name).map(String::as_str)
}

#[test]
fn with_audio_not_set_up_enter_on_the_code_confirms_it_and_leaves_audio_alone() {
    let harness = Harness::builder()
        .headless_screen()
        .phone_unpaired()
        .wireplumber_config("51-music.conf", ROLES_WITHOUT_HANDS_FREE)
        .start();
    wait_for_problems(&harness, "audio-not-set-up", STARTUP);
    harness.wait_for_app("the Desktop is discoverable", DEADLINE, |state| {
        window_field(state, "desktop").is_some()
    });

    harness
        .simulated_phone()
        .pair_from_phone()
        .expect("the Phone finds the Desktop");
    harness.wait_for_app("the code shows, with focus", DEADLINE, |state| {
        window_field(state, "step") == Some("confirm")
            && window_field(state, "focused") == Some("true")
    });
    harness.keyboard().press(Key::Return);

    let give_up = std::time::Instant::now() + DEADLINE;
    loop {
        let requests = harness.simulated_phone().pairing_requests();
        if requests.last().is_some_and(|(_, outcome)| outcome == "confirmed") {
            break;
        }
        assert!(
            std::time::Instant::now() < give_up,
            "the code isn't confirmed after {DEADLINE:?}: {requests:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    harness.simulated_phone().recorded_holds(
        "WirePlumber not restarted",
        BEFORE_CONSENT,
        |recorded| !restarts_wireplumber(recorded),
    );
    assert!(
        !harness.wireplumber_drop_ins().contains_key(DROP_IN),
        "no drop-in written"
    );
}

#[test]
fn contacts_support_missing_takes_no_focus_so_enter_still_pairs() {
    let harness = Harness::builder()
        .headless_screen()
        .phone_unpaired()
        .without_contacts_support()
        .start();
    harness.wait_for_app("the Contacts support Problem shows", STARTUP, |state| {
        field(state, "problems") == Some("contacts-support-missing")
    });
    let state = wait_for_phone_listed(&harness);
    assert_eq!(
        window_field(&state, "keys"),
        Some("Up and Down choose a phone, Enter pairs it; Escape closes")
    );

    harness.keyboard().press(Key::Return);

    let state = harness.wait_for_app("pairing with the Phone", DEADLINE, |state| {
        state
            .surface("setup-window")
            .and_then(|window| window.get("step"))
            .is_some_and(|step| step == "pairing" || step == "confirm")
    });
    assert_eq!(field(&state, "copied"), None, "Enter didn't copy");
}

#[test]
fn contacts_support_missing_shows_the_install_command_and_copy_puts_it_on_the_clipboard() {
    let harness = Harness::builder()
        .headless_screen()
        .phone_unpaired()
        .without_contacts_support()
        .start();
    let state = harness.wait_for_app("the Contacts support Problem shows", STARTUP, |state| {
        field(state, "problems") == Some("contacts-support-missing")
    });
    assert_eq!(field(&state, "commands"), Some(INSTALL));
    assert_eq!(state.problems[0].action, INSTALL);
    wait_for_phone_listed(&harness);

    // Back from the Phone to the button above it.
    harness.keyboard().press_with_shift(Key::Tab);
    harness.wait_for_app("Copy command has focus", DEADLINE, |state| {
        field(state, "focused") == Some("true")
    });
    harness.keyboard().press(Key::Return);

    harness.wait_for_app("the Setup Window says it copied", DEADLINE, |state| {
        field(state, "copied") == Some(INSTALL)
    });
    assert_eq!(harness.clipboard_text(), INSTALL);
}

#[test]
fn audio_not_set_up_writes_one_drop_in_only_after_consent_and_restarts_wireplumber() {
    let harness = Harness::builder()
        .headless_screen()
        .phone_unpaired()
        .wireplumber_config("51-music.conf", ROLES_WITHOUT_HANDS_FREE)
        .start();

    let state = wait_for_problems(&harness, "audio-not-set-up", STARTUP);
    assert_eq!(field(&state, "buttons"), Some("Set up audio"));
    let before = BTreeMap::from([(
        "51-music.conf".to_owned(),
        ROLES_WITHOUT_HANDS_FREE.to_owned(),
    )]);
    harness.simulated_phone().recorded_holds(
        "nothing written or restarted before the user asks",
        BEFORE_CONSENT,
        |recorded| !restarts_wireplumber(recorded) && harness.wireplumber_drop_ins() == before,
    );

    wait_for_phone_listed(&harness);
    press_set_up_audio(&harness);

    harness.simulated_phone().wait_for_recorded(
        "WirePlumber is restarted",
        DEADLINE,
        restarts_wireplumber,
    );
    let after = harness.wireplumber_drop_ins();
    assert_eq!(after.get(DROP_IN).map(String::as_str), Some(SET_UP));
    assert_eq!(
        after.get("51-music.conf").map(String::as_str),
        Some(ROLES_MOVED)
    );
    assert_eq!(after.len(), 2, "{after:#?}");
    no_problems_shown(&harness);
}

/// Presses Set up audio once the Problem shows, and waits until setting up
/// is done: `WirePlumber` restarted and no Problem left.
fn set_up_audio_and_wait_until_done(harness: &Harness) {
    wait_for_problems(harness, "audio-not-set-up", STARTUP);
    wait_for_phone_listed(harness);
    press_set_up_audio(harness);
    harness.simulated_phone().wait_for_recorded(
        "WirePlumber is restarted",
        DEADLINE,
        restarts_wireplumber,
    );
    no_problems_shown(harness);
}

/// A permission mode the default umask can't produce: files made with a
/// umask of 022 get 0644, and 077 gives 0600.
const GROUP_READABLE: u32 = 0o640;

#[test]
fn setting_up_audio_keeps_the_permissions_of_a_drop_in_it_rewrites() {
    let harness = Harness::builder()
        .headless_screen()
        .phone_unpaired()
        .wireplumber_config_with_mode("51-music.conf", ROLES_WITHOUT_HANDS_FREE, GROUP_READABLE)
        .start();

    set_up_audio_and_wait_until_done(&harness);

    assert_eq!(
        harness.wireplumber_drop_in_text("51-music.conf"),
        ROLES_MOVED,
        "the drop-in was rewritten"
    );
    assert_eq!(
        harness.wireplumber_drop_in_mode("51-music.conf"),
        GROUP_READABLE
    );
}

#[test]
fn setting_up_audio_keeps_the_permissions_of_its_own_drop_in_it_rewrites() {
    // What an earlier Set up audio wrote, since changed by hand.
    const EARLIER: &str = "\
# Written by Desktop Calls when you chose Set up audio: hands-free with
monitor.bluez.properties = {
  bluez5.roles = [ a2dp_sink a2dp_source ]
}
";
    let harness = Harness::builder()
        .headless_screen()
        .phone_unpaired()
        .wireplumber_config_with_mode(DROP_IN, EARLIER, 0o660)
        .start();

    set_up_audio_and_wait_until_done(&harness);

    assert_ne!(
        harness.wireplumber_drop_in_text(DROP_IN),
        EARLIER,
        "the drop-in was rewritten"
    );
    assert_eq!(harness.wireplumber_drop_in_mode(DROP_IN), 0o660);
}

#[test]
fn setting_up_audio_rewrites_the_file_a_linked_drop_in_points_to_and_keeps_its_permissions() {
    let harness = Harness::builder()
        .headless_screen()
        .phone_unpaired()
        .wireplumber_config_linked("51-music.conf", ROLES_WITHOUT_HANDS_FREE, GROUP_READABLE)
        .start();

    set_up_audio_and_wait_until_done(&harness);

    assert!(
        harness.wireplumber_drop_in_is_link("51-music.conf"),
        "the drop-in is still a link"
    );
    assert_eq!(
        harness.wireplumber_drop_in_text("51-music.conf"),
        ROLES_MOVED,
        "the file it points to was rewritten"
    );
    assert_eq!(
        harness.wireplumber_drop_in_mode("51-music.conf"),
        GROUP_READABLE
    );
}

#[test]
fn keeping_audio_off_the_phone_turned_off_alone_is_a_problem_even_in_an_odd_drop_in() {
    // One line, a non-breaking space, and the other two settings right.
    const ODD: &str = "monitor.bluez.properties = {\u{a0}bluez5.telephony.default-reject-sco = false }\n\
        monitor.bluez.rules = [ { matches = [ { device.name = \"~bluez_card.*\" } ] \
        actions = { update-props = { bluez5.auto-connect = [ hfp_hf ] } } } ]\n";
    let harness = Harness::builder()
        .headless_screen()
        .phone_unpaired()
        .wireplumber_config("51-odd.conf", ODD)
        .start();
    wait_for_problems(&harness, "audio-not-set-up", STARTUP);
    wait_for_phone_listed(&harness);

    press_set_up_audio(&harness);

    harness.simulated_phone().wait_for_recorded(
        "WirePlumber is restarted",
        DEADLINE,
        restarts_wireplumber,
    );
    let after = harness.wireplumber_drop_ins();
    let ours = after.get(DROP_IN).map_or("", String::as_str);
    assert!(
        ours.contains("  bluez5.telephony.default-reject-sco = true\n"),
        "{ours}"
    );
    assert!(!ours.contains("bluez5.roles"), "roles were fine: {ours}");
    assert!(!ours.contains("auto-connect"), "auto-connect was fine: {ours}");
    assert_eq!(
        after.get("51-odd.conf").map(String::as_str),
        Some(
            ODD.replace(
                "bluez5.telephony.default-reject-sco = false",
                "# Moved to 60-desktop-calls.conf by Desktop Calls: \
                 bluez5.telephony.default-reject-sco = false\n"
            )
            .as_str()
        ),
        "the rest of the line goes on a line of its own"
    );
    no_problems_shown(&harness);
}

#[test]
fn auto_connect_without_hands_free_alone_is_a_problem_and_gains_it() {
    const MUSIC_ONLY: &str = "\
monitor.bluez.properties = {
  bluez5.telephony.default-reject-sco = true
}
monitor.bluez.rules = [
  {
    matches = [ { device.name = \"~bluez_card.*\" } ]
    actions = { update-props = { bluez5.auto-connect = [ a2dp_sink ] } }
  }
]
";
    let harness = Harness::builder()
        .headless_screen()
        .phone_unpaired()
        .wireplumber_config("51-music.conf", MUSIC_ONLY)
        .start();
    wait_for_problems(&harness, "audio-not-set-up", STARTUP);
    wait_for_phone_listed(&harness);

    press_set_up_audio(&harness);

    harness.simulated_phone().wait_for_recorded(
        "WirePlumber is restarted",
        DEADLINE,
        restarts_wireplumber,
    );
    let after = harness.wireplumber_drop_ins();
    let ours = after.get(DROP_IN).map_or("", String::as_str);
    assert!(
        ours.contains("        bluez5.auto-connect = [ a2dp_sink hfp_hf ]\n"),
        "{ours}"
    );
    assert!(!ours.contains("default-reject-sco"), "it was fine: {ours}");
    no_problems_shown(&harness);
}

#[test]
fn a_drop_in_of_the_users_own_by_the_same_name_keeps_everything_else_in_it() {
    const OTHER_DROP_IN: &str = "60-desktop-calls-audio.conf";
    let harness = Harness::builder()
        .headless_screen()
        .phone_unpaired()
        .wireplumber_config(DROP_IN, ROLES_WITHOUT_HANDS_FREE)
        .start();
    wait_for_problems(&harness, "audio-not-set-up", STARTUP);
    wait_for_phone_listed(&harness);

    press_set_up_audio(&harness);

    harness.simulated_phone().wait_for_recorded(
        "WirePlumber is restarted",
        DEADLINE,
        restarts_wireplumber,
    );
    let after = harness.wireplumber_drop_ins();
    assert_eq!(
        after.get(DROP_IN).map(String::as_str),
        Some(ROLES_MOVED.replace(DROP_IN, OTHER_DROP_IN).as_str()),
        "the user's own file keeps all but the moved setting"
    );
    assert_eq!(after.get(OTHER_DROP_IN).map(String::as_str), Some(SET_UP));
    assert_eq!(after.len(), 2, "{after:#?}");
    no_problems_shown(&harness);
}

#[test]
fn audio_settings_that_cannot_be_saved_say_so_and_the_problem_stays() {
    let harness = Harness::builder()
        .headless_screen()
        .phone_unpaired()
        .wireplumber_config("51-music.conf", ROLES_WITHOUT_HANDS_FREE)
        // A folder where the drop-in goes: it can't be written, even by root.
        .wireplumber_config_folder(DROP_IN)
        .start();
    wait_for_problems(&harness, "audio-not-set-up", STARTUP);
    wait_for_phone_listed(&harness);

    press_set_up_audio(&harness);

    let folder = harness
        .directory()
        .join("config/wireplumber/wireplumber.conf.d")
        .display()
        .to_string();
    let state = harness.wait_for_app("the Setup Window says why", DEADLINE, |state| {
        field(state, "notice")
            .is_some_and(|notice| notice.contains("couldn't be saved") && notice.contains(&folder))
    });
    assert_eq!(kinds(&state), ["audio-not-set-up"]);
    assert_eq!(field(&state, "buttons"), Some("Set up audio"), "it can be tried again");
    harness.simulated_phone().recorded_holds(
        "WirePlumber isn't restarted for settings that weren't saved",
        BEFORE_CONSENT,
        |recorded| !restarts_wireplumber(recorded),
    );
    assert_eq!(
        harness.wireplumber_drop_ins().get("51-music.conf").map(String::as_str),
        Some(ROLES_WITHOUT_HANDS_FREE),
        "the other file is left as it was"
    );
}

/// Sets up audio against a Desktop whose systemd accepts the restart of
/// `WirePlumber` and then ends the job as `result` says, and checks the
/// Setup Window says the audio didn't restart and keeps the Problem, with
/// its button, so the user can try again. `wait` is how long the app may
/// take to give up on the job.
fn setting_up_audio_with_a_restart_job_ending(result: &str, wait: Duration) {
    let harness = Harness::builder()
        .headless_screen()
        .phone_unpaired()
        .wireplumber_config("51-music.conf", ROLES_WITHOUT_HANDS_FREE)
        .wireplumber_restart_job(result)
        .start();
    wait_for_problems(&harness, "audio-not-set-up", STARTUP);
    wait_for_phone_listed(&harness);

    press_set_up_audio(&harness);

    harness.simulated_phone().wait_for_recorded(
        "WirePlumber's restart is accepted",
        DEADLINE,
        restarts_wireplumber,
    );
    let state = harness.wait_for_app("the Setup Window says the audio didn't restart", wait, |state| {
        field(state, "notice").is_some_and(|notice| notice.contains("didn't restart"))
    });
    assert_eq!(kinds(&state), ["audio-not-set-up"]);
    assert_eq!(field(&state, "buttons"), Some("Set up audio"), "it can be tried again");
    assert!(
        harness.wireplumber_drop_ins().contains_key(DROP_IN),
        "the audio setting is saved"
    );
    no_raw_dbus_text(&[
        field(&state, "notice").unwrap_or_default(),
        field(&state, "buttons").unwrap_or_default(),
    ]);
}

#[test]
fn a_restart_job_that_fails_keeps_the_problem_and_says_the_audio_didnt_restart() {
    setting_up_audio_with_a_restart_job_ending("failed", DEADLINE);
}

#[test]
fn a_restart_job_that_times_out_keeps_the_problem_and_says_the_audio_didnt_restart() {
    setting_up_audio_with_a_restart_job_ending("timeout", DEADLINE);
}

#[test]
fn a_restart_job_that_never_ends_keeps_the_problem_once_the_app_gives_up_waiting() {
    setting_up_audio_with_a_restart_job_ending("never", Duration::from_secs(15));
}

#[test]
fn bluez_hands_free_on_shows_the_conflict_and_how_to_turn_it_off() {
    let harness = Harness::builder()
        .headless_screen()
        .phone_unpaired()
        .bluez_hands_free_on()
        .start();

    let state = wait_for_problems(&harness, "bluez-hands-free", STARTUP);
    let problem = &state.problems[0];
    assert!(
        problem.message.contains("Experimental = false")
            && problem.message.contains("/etc/bluetooth/main.conf"),
        "{}",
        problem.message
    );
    assert_eq!(problem.action, "sudo systemctl restart bluetooth");
}

#[test]
fn every_problem_asks_for_attention_and_the_menu_explains_it_in_plain_words() {
    let harness = Harness::builder()
        .headless_screen()
        .phone_name(PHONE_NAME)
        .bluetooth_off()
        .without_contacts_support()
        .wireplumber_config("51-music.conf", ROLES_WITHOUT_HANDS_FREE)
        .bluez_hands_free_on()
        .start();
    let headings = [
        "Bluetooth is off",
        "Bluetooth's own calling is on",
        "Audio not set up for Calls",
        "Contacts support not installed",
    ];
    let tooltip = format!("My Pixel not connected · {}", headings.join(" · "));
    harness.tray().wait_for("the Indicator asks for attention", DEADLINE, |items| {
        items.iter().any(|item| {
            item.status.as_deref() == Some("NeedsAttention")
                && item.tooltip.as_deref() == Some(tooltip.as_str())
        })
    });
    let state = harness.app_state();
    assert_eq!(
        kinds(&state),
        [
            "bluetooth-off",
            "bluez-hands-free",
            "audio-not-set-up",
            "contacts-support-missing"
        ]
    );

    let menu = harness.tray().menu();
    let mut texts: Vec<String> = MenuEntry::labels(&menu);
    for (heading, problem) in headings.iter().zip(&state.problems) {
        let entry = MenuEntry::find(&menu, &format!("{heading}: put it right…"))
            .unwrap_or_else(|| panic!("{heading:?} is in the menu: {menu:#?}"));
        let explanation = MenuEntry::labels(&entry.children).join(" ");
        assert_eq!(explanation, problem.message, "the menu explains {heading:?}");
        texts.push(explanation);
        texts.push(problem.message.clone());
    }

    // Choosing one opens the Setup Window, which shows them all.
    harness.tray().click("Bluetooth is off: put it right…");
    let state = harness.wait_for_app("the Setup Window shows the Problems", DEADLINE, |state| {
        field(state, "problems")
            == Some("bluetooth-off\nbluez-hands-free\naudio-not-set-up\ncontacts-support-missing")
    });
    let shown = problems_surface(&state).cloned().unwrap_or_default();
    texts.extend(
        shown
            .iter()
            .filter(|(name, _)| *name != "problems" && *name != "commands")
            .map(|(_, text)| text.clone()),
    );
    no_raw_dbus_text(&texts.iter().map(String::as_str).collect::<Vec<_>>());
}

#[test]
fn a_problem_stays_while_the_phone_comes_and_goes_and_setting_up_audio_keeps_audio_on_the_phone() {
    let harness = Harness::builder()
        .headless_screen()
        .phone_name(PHONE_NAME)
        .wireplumber_config("51-music.conf", ROLES_WITHOUT_HANDS_FREE)
        .start();
    harness.simulated_phone().connect();
    harness.wait_for_app("connected, with the Problem", DEADLINE, |state| {
        state.connection_state == "connected" && kinds(state) == ["audio-not-set-up"]
    });

    harness.simulated_phone().disconnect();
    harness.wait_for_app("out of range, the Problem still there", DEADLINE, |state| {
        state.connection_state == "disconnected" && kinds(state) == ["audio-not-set-up"]
    });
    harness.tray().wait_for("the Indicator still asks for attention", DEADLINE, |items| {
        items
            .iter()
            .any(|item| item.status.as_deref() == Some("NeedsAttention"))
    });
    harness.simulated_phone().return_to_range();
    harness.wait_for_app("back, the Problem still there", DEADLINE, |state| {
        state.connection_state == "connected" && kinds(state) == ["audio-not-set-up"]
    });

    let rejected_before = rejects_audio(&harness.simulated_phone().wait_for_recorded(
        "audio kept on the Phone",
        DEADLINE,
        |recorded| rejects_audio(recorded) > 0,
    ));
    let output = harness.cli(&["connect"]);
    assert!(output.status.success(), "{output:?}");
    wait_for_problems(&harness, "audio-not-set-up", DEADLINE);
    press_set_up_audio(&harness);

    harness.simulated_phone().wait_for_recorded(
        "WirePlumber restarted, and audio kept on the Phone again after it",
        DEADLINE,
        |recorded| restarts_wireplumber(recorded) && rejects_audio(recorded) > rejected_before,
    );
    harness.wait_for_app("connected again, with no Problem", DEADLINE, |state| {
        state.connection_state == "connected" && state.problems.is_empty()
    });
}
