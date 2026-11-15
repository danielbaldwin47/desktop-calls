//! The Setup Window's parts in its Messages: the four named steps across
//! the top, instructions grouped by where each is done, and the countdown
//! that says what it counts.

use desktop_calls_core::messages::{
    self, Instruction, InstructionGroup, Place, SETUP_STAGES, StageMark,
};
use desktop_calls_core::{
    ContactsOutcome, KeybindingStage, Phone, SetupStep, SetupWindowView, Timestamp,
};

fn pixel() -> Phone {
    Phone {
        name: "My Pixel".to_owned(),
        address: "AA:BB:CC:DD:EE:FF".to_owned(),
    }
}

fn view(step: SetupStep) -> SetupWindowView {
    SetupWindowView {
        step,
        desktop_name: Some("desktop".to_owned()),
        notice: None,
        opened: 1,
        calls_off: None,
        problem_has_focus: None,
        focus: None,
    }
}

fn text(step: SetupStep) -> messages::SetupWindowText {
    messages::setup_window(&view(step), Timestamp(0), None)
}

fn done(contacts: ContactsOutcome) -> SetupStep {
    SetupStep::Done {
        phone: pixel(),
        connected: true,
        contacts,
        keybinding: KeybindingStage::Checking,
    }
}

#[test]
fn the_four_steps_mark_where_setup_is_and_tick_what_is_done() {
    use StageMark::{Ahead, Now, Passed, Ticked};
    assert_eq!(SETUP_STAGES, ["Pair", "Share contacts", "Sync", "Shortcut"]);
    let marks = |step| text(step).stages;
    assert_eq!(
        marks(SetupStep::Choose { phones: Vec::new() }),
        [Now, Ahead, Ahead, Ahead]
    );
    assert_eq!(
        marks(SetupStep::Confirm {
            phone: pixel(),
            code: 509_015
        }),
        [Now, Ahead, Ahead, Ahead]
    );
    assert_eq!(
        marks(SetupStep::ContactsWarning { phone: pixel() }),
        [Ticked, Now, Ahead, Ahead]
    );
    assert_eq!(
        marks(SetupStep::ContactsSyncing {
            phone: pixel(),
            call_history: false,
            contacts: None,
            contacts_read: false,
        }),
        [Ticked, Ticked, Now, Ahead]
    );
    // A Phone that leaves is asked again: that is sharing.
    assert_eq!(
        marks(SetupStep::ContactsPhoneAway {
            phone: pixel(),
            back: false
        }),
        [Ticked, Now, Ahead, Ahead]
    );
    assert_eq!(
        marks(done(ContactsOutcome::Synced {
            contacts: 1,
            calls: 1
        })),
        [Ticked, Ticked, Ticked, Now]
    );
    // Contacts not shared: sharing and syncing were passed, not done.
    assert_eq!(
        marks(done(ContactsOutcome::Denied)),
        [Ticked, Passed, Passed, Now]
    );
}

#[test]
fn instructions_are_grouped_on_your_phone_and_here_in_the_order_they_come() {
    let choose = text(SetupStep::Choose { phones: Vec::new() });
    assert_eq!(
        messages::grouped(&choose.instructions),
        [
            InstructionGroup {
                place: Place::OnYourPhone,
                label: Some("On your phone"),
                lines: vec![
                    "Open Settings, then Connected devices.".to_owned(),
                    "Tap Pair new device, and keep that screen open.".to_owned(),
                ],
            },
            InstructionGroup {
                place: Place::Here,
                label: Some("Here"),
                lines: vec![
                    "Choose your phone below, or tap “desktop” on your phone when it shows."
                        .to_owned()
                ],
            },
        ]
    );
    // Reported on one line each, saying where when it is on the Phone.
    assert_eq!(
        choose.instructions[0].line(),
        "On your phone: Open Settings, then Connected devices."
    );

    // What is done here first comes first.
    let timed_out = text(SetupStep::ContactsPromptTimedOut { phone: pixel() });
    let places: Vec<Place> = messages::grouped(&timed_out.instructions)
        .iter()
        .map(|group| group.place)
        .collect();
    assert_eq!(places, [Place::Here, Place::OnYourPhone]);

    // With nothing to do on the Phone, "Here" needs no label.
    let alone = [Instruction {
        place: Place::Here,
        text: "Choose Confirm.".to_owned(),
    }];
    assert_eq!(messages::grouped(&alone)[0].label, None);
}

#[test]
fn the_countdown_says_what_it_counts() {
    let asking = messages::setup_window(
        &view(SetupStep::ContactsPrompt {
            phone: pixel(),
            deadline: Timestamp(28_000),
        }),
        Timestamp(0),
        None,
    );
    assert_eq!(asking.countdown.as_deref(), Some("28"));
    assert_eq!(asking.countdown_label, Some("left to tap Allow"));
    // The label says it, so no summary says it again.
    assert_eq!(asking.summary, None);
}
