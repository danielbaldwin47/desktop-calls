//! Scenarios: with two Phones paired, Desktop Calls follows the one the
//! user chose last in the Setup Window, across restarts, and opens the Setup
//! Window rather than follow another Phone when that one is gone (ticket
//! #121).
//!
//! The Simulated Phone's other Phone (`old_phone_paired`) lists before the
//! Phone in BlueZ's objects and in the app's reading of them, so without a
//! remembered choice a restart follows it: every scenario here fails if the
//! choice is not kept and obeyed.

use std::collections::BTreeMap;
use std::fs;
use std::time::Duration;

use test_harness::{AppState, Bus, Harness, Key};

/// How long the app may take to start and notice a change.
const STARTUP: Duration = Duration::from_secs(10);
/// How long pairing and connecting may take after Confirm.
const CONNECT_DEADLINE: Duration = Duration::from_secs(15);
const PIXEL: &str = "My Pixel";
const OLD_PIXEL: &str = "Old Pixel";
const OLD_PIXEL_ADDRESS: &str = "00:A4:93:00:00:03";
const NUMBER: &str = "+447700900123";

/// The old Phone is paired and out of range, and the Pixel is nearby, not
/// paired. The app follows the old Phone, the only one paired.
fn old_phone_followed() -> Harness {
    let harness = Harness::builder()
        .headless_screen()
        .phone_name(PIXEL)
        .phone_unpaired()
        .old_phone_paired(OLD_PIXEL)
        .start();
    harness.wait_for_app("the old Phone is followed", STARTUP, |state| {
        phone_is(state, OLD_PIXEL)
    });
    harness
}

/// The user pairs the Pixel in the Setup Window, with keys alone, and the
/// app switches to it: two Phones are paired, and the Pixel was chosen last.
fn pixel_chosen_over_the_old_phone() -> Harness {
    let harness = old_phone_followed();
    choose_the_pixel(&harness);
    harness
}

/// Pairs the Pixel in the Setup Window, with keys alone.
fn choose_the_pixel(harness: &Harness) {
    let output = harness.cli(&["connect"]);
    assert!(output.status.success(), "{output:?}");
    harness.wait_for_app(
        "the Setup Window lists the Pixel and has keyboard focus",
        STARTUP,
        |state| field(state, "phones") == Some(PIXEL) && field(state, "focused") == Some("true"),
    );
    harness.keyboard().press(Key::Return);
    harness.wait_for_app("the code is shown", CONNECT_DEADLINE, |state| {
        field(state, "step") == Some("confirm")
    });
    harness.keyboard().press(Key::Return);
    harness.wait_for_app(
        "the Pixel is followed and connected",
        CONNECT_DEADLINE,
        |state| phone_is(state, PIXEL) && state.connection_state == "connected",
    );
}

fn phone_is(state: &AppState, name: &str) -> bool {
    state.phone.as_ref().is_some_and(|phone| phone.name == name)
}

fn setup_window(state: &AppState) -> Option<&BTreeMap<String, String>> {
    state.surface("setup-window")
}

fn field<'state>(state: &'state AppState, name: &str) -> Option<&'state str> {
    setup_window(state)?.get(name).map(String::as_str)
}

fn chosen_phone_file(harness: &Harness) -> std::path::PathBuf {
    harness.directory().join("state/desktop-calls/chosen-phone.json")
}

/// The user unpairs the device at `address` in Bluetooth settings. The
/// Simulated BlueZ removes only a device that isn't connected, so the Phone
/// goes out of range first, as `bluetoothctl remove` finds it after BlueZ
/// has dropped it.
fn unpair(harness: &Harness, address: &str) {
    let path = format!("/org/bluez/hci0/dev_{}", address.replace(':', "_"));
    let path = zbus::ObjectPath::try_from(path.as_str()).unwrap_or_else(|error| panic!("{path} isn't a device path: {error}"));
    harness.call_phone_method(
        Bus::System,
        "org.bluez",
        "/org/bluez/hci0",
        "org.bluez.Adapter1",
        "RemoveDevice",
        &(path,),
    );
}

#[test]
fn after_a_restart_the_app_follows_the_phone_chosen_last() {
    let mut harness = pixel_chosen_over_the_old_phone();

    harness.restart_app();

    let state = harness.wait_for_app(
        "the Pixel, chosen last, is followed and connected, not the Phone listed first",
        STARTUP,
        |state| phone_is(state, PIXEL) && state.connection_state == "connected",
    );
    assert!(setup_window(&state).is_none(), "the Setup Window stays closed");
}

#[test]
fn the_choice_is_kept_across_more_than_one_restart() {
    let mut harness = pixel_chosen_over_the_old_phone();

    for _ in 0..2 {
        harness.restart_app();
        harness.wait_for_app("the Pixel is followed", STARTUP, |state| {
            phone_is(state, PIXEL) && state.connection_state == "connected"
        });
    }
}

#[test]
fn with_both_phones_connected_at_once_the_chosen_one_is_followed() {
    let mut harness = pixel_chosen_over_the_old_phone();
    harness.simulated_phone().connect_other_phone();
    harness.restart_app();

    harness.wait_for_app(
        "the Pixel is followed and connected while the old Phone is too",
        STARTUP,
        |state| phone_is(state, PIXEL) && state.connection_state == "connected",
    );
    // The Pixel's Calls are the Desktop's Calls, not the other Phone's.
    let _call = harness.simulated_phone().ring(NUMBER);
    harness.wait_for_app("the Pixel's Call rings", STARTUP, |state| {
        state.ringing && state.calls.len() == 1
    });
}

#[test]
fn with_the_chosen_phone_out_of_range_at_start_it_is_still_the_one_followed() {
    let mut harness = pixel_chosen_over_the_old_phone();
    harness.simulated_phone().disconnect();
    harness.restart_app();

    let state = harness.wait_for_app(
        "the Pixel is followed, though it is away",
        STARTUP,
        |state| phone_is(state, PIXEL) && state.connection_state != "connected",
    );
    assert!(setup_window(&state).is_none(), "the Setup Window stays closed");

    harness.simulated_phone().return_to_range();
    harness.wait_for_app(
        "the Pixel connects once it is back in range",
        CONNECT_DEADLINE,
        |state| phone_is(state, PIXEL) && state.connection_state == "connected",
    );
}

#[test]
fn when_the_chosen_phone_was_unpaired_the_setup_window_opens_and_no_other_phone_is_followed() {
    let mut harness = pixel_chosen_over_the_old_phone();
    harness.simulated_phone().disconnect();
    unpair(&harness, &harness.phone().address.clone());
    harness.wait_for_app("the Pixel is forgotten", STARTUP, |state| {
        state.phone.is_none()
    });

    harness.restart_app();

    let state = harness.wait_for_app(
        "the Setup Window opens, at its first step",
        STARTUP,
        |state| field(state, "step") == Some("choose"),
    );
    // The window opens once BlueZ has listed every Phone, the old one among
    // them: it was left alone.
    assert!(state.phone.is_none(), "no other Phone is followed: {state:?}");
    assert_eq!(state.connection_state, "not-set-up");
}

#[test]
fn the_chosen_phone_unpaired_while_the_app_runs_is_not_replaced_by_another() {
    let harness = pixel_chosen_over_the_old_phone();
    harness.simulated_phone().disconnect();
    unpair(&harness, &harness.phone().address.clone());

    let state = harness.wait_for_app("the Pixel is forgotten", STARTUP, |state| {
        state.phone.is_none()
    });
    assert_eq!(state.connection_state, "not-set-up");
}

#[test]
fn a_saved_choice_that_cannot_be_read_counts_as_no_choice() {
    let mut harness = pixel_chosen_over_the_old_phone();
    let file = chosen_phone_file(&harness);
    assert!(file.exists(), "the choice was kept in {}", file.display());

    let corruptions: [(&str, &[u8]); 4] = [
        ("not JSON", b"\x00\xffnot json at all"),
        ("empty", b""),
        (
            "another version",
            br#"{"version":2,"address":"FC:41:16:00:00:01"}"#,
        ),
        ("not an address", br#"{"version":1,"address":"the Pixel"}"#),
    ];
    for (round, (what, bytes)) in corruptions.into_iter().enumerate() {
        fs::write(&file, bytes).expect("corrupt the saved choice");

        harness.restart_app();

        // No choice: the first Phone BlueZ lists, as before there was one.
        let state = harness.wait_for_app(
            &format!("the app starts with a saved choice that is {what}"),
            STARTUP,
            |state| phone_is(state, OLD_PIXEL),
        );
        assert!(setup_window(&state).is_none(), "{what}: no Setup Window");
        let failures = harness
            .app_log()
            .matches(r#""error_kind":"chosen-phone-unreadable""#)
            .count();
        assert_eq!(failures, round + 1, "{what}: the app logged it once");
    }
}

#[test]
fn the_next_choice_replaces_a_saved_choice_that_cannot_be_read() {
    let mut harness = old_phone_followed();
    let file = chosen_phone_file(&harness);
    fs::create_dir_all(file.parent().expect("a state directory")).expect("make it");
    fs::write(&file, b"garbage").expect("corrupt the saved choice");
    harness.restart_app();
    harness.wait_for_app("the old Phone is followed", STARTUP, |state| {
        phone_is(state, OLD_PIXEL)
    });

    choose_the_pixel(&harness);
    harness.restart_app();

    harness.wait_for_app("the Pixel is followed", STARTUP, |state| {
        phone_is(state, PIXEL) && state.connection_state == "connected"
    });
}

#[test]
fn an_old_phone_never_chosen_is_followed_when_it_is_the_only_one_paired() {
    let mut harness = old_phone_followed();
    assert!(!chosen_phone_file(&harness).exists(), "nothing was chosen");

    harness.restart_app();

    harness.wait_for_app("the old Phone is followed again", STARTUP, |state| {
        phone_is(state, OLD_PIXEL)
    });
    let address = harness.app_state().phone.map(|phone| phone.address);
    assert_eq!(address.as_deref(), Some(OLD_PIXEL_ADDRESS));
}
