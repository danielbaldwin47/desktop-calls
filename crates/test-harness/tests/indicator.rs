//! The Indicator in Omarchy's bar: what the tray watcher stand-in records of
//! it, its left click and its menu, against the Simulated Phone.
//!
//! The deadlines allow for CI's slower runners: see
//! docs/research/2026-11-04-ci-banner-latency.md.

use std::time::Duration;

use test_harness::{Harness, MenuEntry, RecordedMethodCall, TrayItem};

/// How long the app may take to notice a change on the Phone, or a click.
const DEADLINE: Duration = Duration::from_secs(5);

/// The user's Pixel's Bluetooth name.
const PHONE_NAME: &str = "My Pixel";

const NUMBER: &str = "+447700900123";
const AUDIO_GATEWAY: &str = "org.pipewire.Telephony.AudioGateway1";
const TRANSPORT: &str = "org.pipewire.Telephony.AudioGatewayTransport1";
/// What the Indicator says while a Call from [`NUMBER`] rings, and once it
/// is answered.
const RINGING: &str = "Call from +44 7700 900123";
const IN_CALL: &str = "In a Call with +44 7700 900123";

fn connected() -> Harness {
    let harness = Harness::builder().phone_name(PHONE_NAME).start();
    harness.simulated_phone().connect();
    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
    harness
}

/// Waits until the Indicator, the only tray item, reports `status` and
/// `tooltip`, and returns it.
fn indicator_shows(harness: &Harness, status: &str, tooltip: &str) -> TrayItem {
    let items = harness.tray().wait_for(
        &format!("the Indicator is {status} and says {tooltip:?}"),
        DEADLINE,
        |items| {
            items.iter().any(|item| {
                item.status.as_deref() == Some(status) && item.tooltip.as_deref() == Some(tooltip)
            })
        },
    );
    assert_eq!(
        items.len(),
        1,
        "the Indicator is the only tray item: {items:?}"
    );
    items.into_iter().next().unwrap_or_else(|| unreachable!())
}

#[test]
fn connected_and_idle_the_indicator_is_active_and_says_the_phone_is_connected() {
    let harness = connected();

    let indicator = indicator_shows(&harness, "Active", "My Pixel connected");

    assert!(
        indicator
            .icon_name
            .as_deref()
            .is_some_and(|icon| !icon.is_empty()),
        "{indicator:?}"
    );
    assert!(
        !harness
            .tray()
            .statuses_seen()
            .iter()
            .any(|status| status == "Passive"),
        "Omarchy hides a Passive item: {:?}",
        harness.tray().statuses_seen()
    );
}

#[test]
fn ringing_in_a_call_and_out_of_range_each_have_their_own_icon_and_words() {
    let harness = connected();
    let connected = indicator_shows(&harness, "Active", "My Pixel connected");

    let _call = harness.simulated_phone().ring(NUMBER);
    // Omarchy draws NeedsAttention like Active, so the icon must show it.
    let ringing = indicator_shows(&harness, "NeedsAttention", RINGING);
    let call = harness.app_state().calls[0].id;
    harness.answer(call);
    let in_call = indicator_shows(&harness, "Active", IN_CALL);
    harness.hang_up(call);
    indicator_shows(&harness, "Active", "My Pixel connected");
    harness.simulated_phone().disconnect();
    let out_of_range = indicator_shows(&harness, "Active", "My Pixel out of range");

    let icons = [&connected, &ringing, &in_call, &out_of_range].map(|item| item.icon_name.clone());
    for (index, icon) in icons.iter().enumerate() {
        assert!(
            icon.as_deref().is_some_and(|icon| !icon.is_empty()),
            "{icons:?}"
        );
        assert!(
            !icons[..index].contains(icon),
            "each state has its own icon: {icons:?}"
        );
    }
    assert!(
        connected
            .icon_theme_path
            .as_deref()
            .is_some_and(|path| !path.is_empty()),
        "the icons ship with the app: {connected:?}"
    );
    assert_eq!(
        harness.tray().statuses_seen(),
        ["Active", "NeedsAttention", "Active"],
        "never Passive, which Omarchy hides"
    );
}

#[test]
fn clicking_the_indicator_opens_the_dialer() {
    let harness = Harness::builder()
        .headless_screen()
        .omarchy_bar()
        .phone_name(PHONE_NAME)
        .start();
    harness.simulated_phone().connect();
    indicator_shows(&harness, "Active", "My Pixel connected");

    harness.tray().activate();

    harness.wait_for_app("the Dialer has keyboard focus", DEADLINE, |state| {
        state
            .surface("dialer")
            .and_then(|dialer| dialer.get("focused"))
            .is_some_and(|focused| focused == "true")
    });
}

#[test]
fn idle_the_menu_offers_the_status_and_the_dialer() {
    let harness = connected();
    indicator_shows(&harness, "Active", "My Pixel connected");

    assert_eq!(
        MenuEntry::labels(&harness.tray().menu()),
        ["My Pixel connected", "Open Dialer", "Connect your phone…"]
    );
}

#[test]
fn during_a_call_the_menu_mutes_and_hangs_up_as_the_call_window_does() {
    let harness = connected();
    harness.simulated_phone().set_microphone_volume(9);
    live_on_desktop(&harness);
    assert_eq!(
        MenuEntry::labels(&harness.tray().menu()),
        [
            IN_CALL,
            "Open Dialer",
            "Connect your phone…",
            "---",
            "Hang up",
            "Mute",
            "Move audio to phone"
        ]
    );

    harness.tray().click("Mute");
    harness.wait_for_app("Muted is true", DEADLINE, |state| state.muted);
    harness.simulated_phone().wait_for_recorded(
        "MicrophoneVolume is set to 0",
        DEADLINE,
        |recorded| microphone_volumes_set(recorded) == [set_to(0)],
    );
    // The Indicator shows each state before the control interface does.
    let menu = harness.tray().menu();
    assert_eq!(
        MenuEntry::find(&menu, "Mute").and_then(|mute| mute.ticked),
        Some(true),
        "the Mute entry is ticked: {menu:#?}"
    );

    harness.tray().click("Mute");
    harness.wait_for_app("Muted is false", DEADLINE, |state| !state.muted);
    harness.simulated_phone().wait_for_recorded(
        "MicrophoneVolume is set back to 9",
        DEADLINE,
        |recorded| microphone_volumes_set(recorded) == [set_to(0), set_to(9)],
    );

    harness.tray().click("Hang up");
    harness.wait_for_app("the Call is over", DEADLINE, |state| state.calls.is_empty());
    harness
        .simulated_phone()
        .wait_for_recorded("the Call is hung up", DEADLINE, |recorded| {
            recorded.iter().any(|recorded| recorded.member == "Hangup")
        });
    indicator_shows(&harness, "Active", "My Pixel connected");
}

#[test]
fn the_menu_moves_the_audio_here_as_the_call_window_does() {
    let harness = connected();
    let call = harness.simulated_phone().ring(NUMBER);
    harness.wait_for_app("the Call rings", DEADLINE, |state| state.ringing);
    harness.simulated_phone().answer_on_phone(&call);
    harness.wait_for_app("the Call is live on the Phone", DEADLINE, |state| {
        state
            .calls
            .first()
            .is_some_and(|call| call.state == "active")
            && state.audio_location == "phone"
    });
    indicator_shows(&harness, "Active", IN_CALL);
    let menu = harness.tray().menu();
    let mute = MenuEntry::find(&menu, "Mute").unwrap_or_else(|| panic!("{menu:#?}"));
    assert!(!mute.enabled, "the microphone is the Phone's: {mute:?}");

    harness.tray().click("Move audio to this computer");

    harness.wait_for_app("the audio is on the Desktop", DEADLINE, |state| {
        state.audio_location == "desktop"
    });
    let calls = harness.simulated_phone().wait_for_recorded(
        "RejectSCO is set again once the audio is here",
        DEADLINE,
        |calls| handoff_requests(calls).len() >= 3,
    );
    assert_eq!(
        handoff_requests(&calls),
        ["RejectSCO=false", "Activate", "RejectSCO=true"],
        "{calls:#?}"
    );
    assert!(
        MenuEntry::find(&harness.tray().menu(), "Move audio to phone").is_some(),
        "the menu offers to move it back"
    );
}

/// A Call from [`NUMBER`] rings and is answered here, its audio on the
/// Desktop.
fn live_on_desktop(harness: &Harness) {
    let _call = harness.simulated_phone().ring(NUMBER);
    let state = harness.wait_for_app("the Call rings", DEADLINE, |state| {
        state.ringing && state.calls.len() == 1
    });
    harness.answer(state.calls[0].id);
    harness.wait_for_app("the Call is live here", DEADLINE, |state| {
        state.audio_location == "desktop" && state.calls[0].state == "active"
    });
    indicator_shows(harness, "Active", IN_CALL);
}

fn microphone_volumes_set(recorded: &[RecordedMethodCall]) -> Vec<String> {
    recorded
        .iter()
        .filter(|call| {
            call.member == "Set"
                && call.arguments.contains(AUDIO_GATEWAY)
                && call.arguments.contains("MicrophoneVolume")
        })
        .map(|call| call.arguments.clone())
        .collect()
}

fn set_to(volume: u8) -> String {
    format!("(\"{AUDIO_GATEWAY}\", \"MicrophoneVolume\", <byte 0x{volume:02x}>)")
}

/// What the Desktop asked of the audio from its first Handoff on, in
/// order: each `RejectSCO` write and each `Activate`.
fn handoff_requests(calls: &[RecordedMethodCall]) -> Vec<&'static str> {
    calls
        .iter()
        .filter(|call| call.bus == "session" && call.path.starts_with("/org/pipewire/Telephony"))
        .filter_map(|call| {
            let reject_sco = |value: &str| {
                call.member == "Set"
                    && call.arguments.contains(TRANSPORT)
                    && call.arguments.contains("RejectSCO")
                    && call.arguments.contains(value)
            };
            if reject_sco("false") {
                Some("RejectSCO=false")
            } else if reject_sco("true") {
                Some("RejectSCO=true")
            } else if call.member == "Activate" {
                Some("Activate")
            } else {
                None
            }
        })
        .skip_while(|request| *request != "RejectSCO=false")
        .collect()
}

#[test]
fn a_problem_asks_for_attention_shows_its_own_icon_and_the_menu_explains_it() {
    let connected_icon = indicator_shows(&connected(), "Active", "My Pixel connected").icon_name;
    let harness = Harness::builder().phone_name(PHONE_NAME).start();
    harness.simulated_phone().set_contacts_prompt("deny");
    harness.simulated_phone().connect();

    let problem = indicator_shows(
        &harness,
        "NeedsAttention",
        "My Pixel connected · Contacts not shared",
    );

    assert_ne!(problem.icon_name, connected_icon);
    let menu = harness.tray().menu();
    let entry = MenuEntry::find(&menu, "Contacts not shared")
        .unwrap_or_else(|| panic!("the Problem is in the menu: {menu:#?}"));
    let explanation = MenuEntry::labels(&entry.children).join(" ");
    assert!(
        explanation.contains("Bluetooth settings"),
        "the menu says how to fix it: {explanation:?}"
    );
}
