//! The Setup Window's later steps at the Call state machine's public
//! interface: once the Phone is connected for Calls, the warning that its
//! Contacts prompt is about to appear, the countdown to the prompt's
//! deadline, Try again, the sync's progress, the Phone leaving part way,
//! and the finish page with the Dialer keybinding (ticket #22).

use desktop_calls_core::{
    CallHistory, CallHistoryEntry, CallHistoryKind, Caller, ConfigFile, Contact, ContactNumber,
    Contacts, ContactsOutcome, ContactsState, Directory, Effect, HyprlandConfig, Input,
    KeybindingCheck, KeybindingStage, NumberLabel, Phone, PhoneCommand, ReadingContacts,
    SetupNotice, SetupStep, State, SyncFailure, SyncProgress, Timer, Timestamp, Unavailable,
    messages, step,
};

const PHONE: &str = "AA:BB:CC:DD:EE:FF";
const KEYS: &str = "SUPER + ALT + C";
const FILE: &str = "/home/user/.config/hypr/bindings.lua";
const LINE: &str = r#"o.bind("SUPER + ALT + C", "Desktop Calls", "/usr/bin/desktop-calls")"#;
const COMMAND: &str = "/usr/bin/desktop-calls";
const BINDINGS: &str = "-- Keep only your personal keybinding overrides here.\n";

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

fn pixel() -> Phone {
    Phone {
        name: "My Pixel".to_owned(),
        address: PHONE.to_owned(),
    }
}

fn setup_step(state: &State) -> Option<SetupStep> {
    state.surfaces().setup_window.map(|window| window.step)
}

fn setup_timers(effects: &[Effect]) -> Vec<Timer> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::StartTimer {
                timer: timer @ Timer::SetupStep(_),
                ..
            } => Some(*timer),
            _ => None,
        })
        .collect()
}

fn syncs(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|effect| **effect == Effect::SyncContacts)
        .count()
}

/// Paired through the window and connected for Calls: the window warns
/// that the Phone is about to ask to share Contacts. Returns the effects of
/// the connect.
fn warned() -> (State, Vec<Effect>) {
    let (state, _) = run(
        State::default(),
        [
            Input::PairedPhonesListed,
            Input::PhoneNearby(pixel()),
            Input::ChoosePhone {
                address: PHONE.to_owned(),
            },
            Input::PairingCode {
                phone: pixel(),
                code: 48_213,
            },
            Input::ConfirmPairing,
            Input::PhonePaired(pixel()),
            Input::ConnectPhoneDone { attempt: 1 },
            Input::PhonePresent {
                address: PHONE.to_owned(),
            },
        ],
    );
    run(
        state,
        [
            Input::PhoneConnected {
                address: PHONE.to_owned(),
            },
            Input::PhoneCommandDone(PhoneCommand::KeepAudioOnPhone),
        ],
    )
}

/// Warned, and the warning has been up long enough: the sync starts.
fn syncing() -> State {
    let (state, effects) = warned();
    let Some(&timer) = setup_timers(&effects).last() else {
        panic!("the warning has a timer");
    };
    run(state, [Input::TimerFired(timer)]).0
}

fn contacts(count: usize) -> Contacts {
    let contacts = (0..count)
        .map(|index| Contact {
            name: format!("Contact {index}"),
            numbers: vec![ContactNumber {
                number: format!("+44770090{index:04}"),
                label: NumberLabel::Mobile,
            }],
        })
        .collect();
    Contacts::new(Directory::new(contacts, Some("GB")))
}

fn call_history(count: usize) -> CallHistory {
    let entries = (0..count)
        .map(|index| CallHistoryEntry {
            kind: CallHistoryKind::Incoming,
            caller: Caller::Number(format!("+44770091{index:04}")),
            at: Some(Timestamp(1_000 * u64::try_from(index).unwrap_or(0))),
            name: String::new(),
        })
        .collect();
    CallHistory::new(entries, &Directory::default())
}

/// Syncing, and the Phone shared everything, in the sync's order: the
/// newest 50 Calls, then 312 Contacts, then the older Calls, 200 in all.
fn synced() -> State {
    run(
        syncing(),
        [
            Input::ContactsSyncProgress(SyncProgress::Shared),
            Input::ContactsSyncProgress(SyncProgress::CallHistory(call_history(50))),
            Input::ContactsSyncProgress(SyncProgress::ReadingContacts {
                count: Some(312),
                at: Timestamp(10_000),
            }),
            Input::ContactsSynced {
                contacts: contacts(312),
                call_history: call_history(50),
            },
            Input::ContactsSyncProgress(SyncProgress::CallHistory(call_history(200))),
            Input::ContactsSyncProgress(SyncProgress::Finished),
        ],
    )
    .0
}

/// What the shell finds: Omarchy's `bindings.lua`, holding `text`.
fn config(text: &str) -> HyprlandConfig {
    HyprlandConfig {
        command: COMMAND.to_owned(),
        bindings: Some(ConfigFile {
            path: FILE.to_owned(),
            text: Some(text.to_owned()),
            required: Vec::new(),
        }),
    }
}

fn available() -> KeybindingCheck {
    KeybindingCheck::Available {
        keys: KEYS.to_owned(),
        file: FILE.to_owned(),
        line: LINE.to_owned(),
        before_return: false,
        seen: BINDINGS.to_owned(),
        added: format!("{BINDINGS}{LINE}\n"),
    }
}

fn done(contacts: ContactsOutcome, keybinding: KeybindingStage) -> SetupStep {
    SetupStep::Done {
        phone: pixel(),
        connected: true,
        contacts,
        keybinding,
    }
}

fn disconnected() -> Input {
    Input::PhoneDisconnected {
        address: PHONE.to_owned(),
    }
}

/// The Phone connects for Calls again.
fn reconnected(state: State) -> (State, Vec<Effect>) {
    run(
        state,
        [
            Input::PhoneConnected {
                address: PHONE.to_owned(),
            },
            Input::PhoneCommandDone(PhoneCommand::KeepAudioOnPhone),
        ],
    )
}

fn text(state: &State) -> messages::SetupWindowText {
    let Some(view) = state.surfaces().setup_window else {
        panic!("the window is open");
    };
    messages::setup_window(&view, Timestamp(0))
}

fn synced_312_and_200() -> ContactsOutcome {
    ContactsOutcome::Synced {
        contacts: 312,
        calls: 200,
    }
}

#[test]
fn once_connected_the_window_warns_of_the_prompt_and_holds_the_sync_until_read() {
    let (state, effects) = warned();

    assert_eq!(
        setup_step(&state),
        Some(SetupStep::ContactsWarning { phone: pixel() })
    );
    assert_eq!(syncs(&effects), 0, "the prompt waits for the warning");
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::StartTimer {
                timer: Timer::SetupStep(_),
                after_ms: 3_000
            }
        )),
        "the warning stays up for 3 s: {effects:?}"
    );

    let timer = setup_timers(&effects)[setup_timers(&effects).len() - 1];
    let (state, effects) = run(state, [Input::TimerFired(timer)]);
    assert_eq!(syncs(&effects), 1);
    assert_eq!(state.contacts_state().as_str(), "syncing");
    // Until the Phone is known to ask, the warning stays.
    assert_eq!(
        setup_step(&state),
        Some(SetupStep::ContactsWarning { phone: pixel() })
    );
}

#[test]
fn closing_the_window_during_the_warning_starts_the_sync_at_once() {
    let (state, _) = warned();

    let (state, effects) = run(state, [Input::CloseSetupWindow]);

    assert_eq!(syncs(&effects), 1);
    assert_eq!(setup_step(&state), None);
}

#[test]
fn while_the_phone_asks_the_window_counts_down_to_its_deadline() {
    let deadline = Timestamp(1_800_000_030_000);
    let (state, _) = run(syncing(), [Input::ContactsPermissionNeeded { deadline }]);

    assert_eq!(
        setup_step(&state),
        Some(SetupStep::ContactsPrompt {
            phone: pixel(),
            deadline
        })
    );
    let view = state.surfaces().setup_window.expect("the window is open");
    let text = messages::setup_window(&view, Timestamp(1_800_000_001_500));
    assert_eq!(text.countdown.as_deref(), Some("29"));
    let text = messages::setup_window(&view, Timestamp(1_800_000_040_000));
    assert_eq!(text.countdown.as_deref(), Some("0"));
}

#[test]
fn an_unanswered_prompt_offers_try_again_which_asks_the_phone_again() {
    let (state, _) = run(
        syncing(),
        [
            Input::ContactsPermissionNeeded {
                deadline: Timestamp(30_000),
            },
            Input::ContactsSyncFailed(SyncFailure::Unavailable(Unavailable::PromptTimedOut)),
        ],
    );
    assert_eq!(
        setup_step(&state),
        Some(SetupStep::ContactsPromptTimedOut { phone: pixel() })
    );
    let view = state.surfaces().setup_window.expect("the window is open");
    let text = messages::setup_window(&view, Timestamp(0));
    assert_eq!(text.buttons, ["Try again", "Skip"]);
    assert_eq!(
        text.countdown.as_deref(),
        Some("0"),
        "the countdown ran out"
    );

    let (state, effects) = run(state, [Input::RetryContacts]);
    assert_eq!(syncs(&effects), 1);
    assert_eq!(state.contacts_state().as_str(), "syncing");
    assert_eq!(
        setup_step(&state),
        Some(SetupStep::ContactsWarning { phone: pixel() })
    );

    // Granted this time.
    let (state, _) = run(
        state,
        [
            Input::ContactsPermissionNeeded {
                deadline: Timestamp(60_000),
            },
            Input::ContactsSyncProgress(SyncProgress::Shared),
            Input::ContactsSynced {
                contacts: contacts(3),
                call_history: call_history(2),
            },
            Input::ContactsSyncProgress(SyncProgress::Finished),
        ],
    );
    assert!(matches!(
        setup_step(&state),
        Some(SetupStep::Done {
            contacts: ContactsOutcome::Synced {
                contacts: 3,
                calls: 2
            },
            ..
        })
    ));
}

#[test]
fn skipping_an_unanswered_prompt_goes_to_the_finish_page() {
    let (state, _) = run(
        syncing(),
        [
            Input::ContactsSyncFailed(SyncFailure::Unavailable(Unavailable::PromptTimedOut)),
            Input::SkipContacts,
        ],
    );

    assert_eq!(
        setup_step(&state),
        Some(done(ContactsOutcome::NotShared, KeybindingStage::Checking))
    );
}

#[test]
fn granted_the_window_shows_the_call_history_done_first_then_the_contacts_being_read() {
    let (state, _) = run(
        syncing(),
        [
            Input::ContactsPermissionNeeded {
                deadline: Timestamp(30_000),
            },
            Input::ContactsSyncProgress(SyncProgress::Shared),
        ],
    );
    assert_eq!(
        setup_step(&state),
        Some(SetupStep::ContactsSyncing {
            phone: pixel(),
            call_history: false,
            contacts: None,
            contacts_read: false,
        })
    );

    let (state, _) = run(
        state,
        [
            Input::ContactsSyncProgress(SyncProgress::CallHistory(call_history(50))),
            Input::ContactsSyncProgress(SyncProgress::ReadingContacts {
                count: Some(312),
                at: Timestamp(10_000),
            }),
        ],
    );
    let reading = ReadingContacts {
        count: Some(312),
        // The Pixel read 319 Contacts in 9.7 s: about 30 ms each.
        done_by: Some(Timestamp(10_000 + 312 * 30)),
    };
    assert_eq!(
        setup_step(&state),
        Some(SetupStep::ContactsSyncing {
            phone: pixel(),
            call_history: true,
            contacts: Some(reading),
            contacts_read: false,
        })
    );
    assert_eq!(state.call_history().entries().len(), 50, "for the Dialer");
    let view = state.surfaces().setup_window.expect("the window is open");
    let text = messages::setup_window(&view, Timestamp(12_000));
    assert_eq!(
        text.progress,
        ["✓ Recent Calls", "312 Contacts… about 8 seconds left"]
    );
}

#[test]
fn once_the_whole_sync_is_done_the_finish_page_reports_what_was_synced() {
    let state = run(
        syncing(),
        [
            Input::ContactsSyncProgress(SyncProgress::Shared),
            Input::ContactsSyncProgress(SyncProgress::CallHistory(call_history(50))),
            Input::ContactsSynced {
                contacts: contacts(312),
                call_history: call_history(50),
            },
        ],
    )
    .0;
    // The older Calls are still being read.
    assert!(matches!(
        setup_step(&state),
        Some(SetupStep::ContactsSyncing {
            contacts_read: true,
            ..
        })
    ));

    let (state, effects) = run(
        state,
        [
            Input::ContactsSyncProgress(SyncProgress::CallHistory(call_history(200))),
            Input::ContactsSyncProgress(SyncProgress::Finished),
        ],
    );

    assert_eq!(
        setup_step(&state),
        Some(done(synced_312_and_200(), KeybindingStage::Checking))
    );
    assert!(effects.contains(&Effect::CheckKeybinding), "{effects:?}");
    assert_eq!(
        text(&state).summary.as_deref(),
        Some("Synced 312 Contacts and 200 recent Calls.")
    );
}

/// Asserts the window says the Phone left, never that anything synced.
fn says_the_phone_left(state: &State) {
    assert_eq!(
        setup_step(state),
        Some(SetupStep::ContactsPhoneAway {
            phone: pixel(),
            back: false
        })
    );
    let shown = text(state);
    assert_eq!(shown.heading, "My Pixel disconnected");
    assert_eq!(shown.buttons, ["Close"]);
    assert!(
        !format!("{shown:?}").contains("Synced"),
        "nothing synced: {shown:?}"
    );
}

#[test]
fn a_phone_leaving_while_it_asks_is_waited_for_and_continuing_asks_again() {
    let (state, _) = run(
        syncing(),
        [
            Input::ContactsPermissionNeeded {
                deadline: Timestamp(30_000),
            },
            disconnected(),
        ],
    );
    says_the_phone_left(&state);
    assert_eq!(
        state.contacts_state(),
        ContactsState::Unavailable(Unavailable::NotSynced)
    );

    // Back: the window offers to carry on, and syncs nothing until asked.
    let (state, effects) = reconnected(state);
    assert_eq!(syncs(&effects), 0, "{effects:?}");
    assert_eq!(
        setup_step(&state),
        Some(SetupStep::ContactsPhoneAway {
            phone: pixel(),
            back: true
        })
    );
    assert_eq!(text(&state).heading, "My Pixel is back");
    assert_eq!(text(&state).buttons, ["Continue", "Close"]);

    let (state, effects) = run(state, [Input::ContinueContacts]);
    assert_eq!(syncs(&effects), 1);
    let (state, _) = run(
        state,
        [
            Input::ContactsSyncProgress(SyncProgress::Shared),
            Input::ContactsSynced {
                contacts: contacts(3),
                call_history: call_history(2),
            },
            Input::ContactsSyncProgress(SyncProgress::Finished),
        ],
    );
    assert_eq!(
        text(&state).summary.as_deref(),
        Some("Synced 3 Contacts and 2 recent Calls.")
    );
}

#[test]
fn a_first_sync_cut_short_after_the_recent_calls_leaves_contacts_not_synced() {
    let (state, _) = run(
        syncing(),
        [
            Input::ContactsSyncProgress(SyncProgress::Shared),
            Input::ContactsSyncProgress(SyncProgress::CallHistory(call_history(50))),
            Input::ContactsSyncProgress(SyncProgress::ReadingContacts {
                count: Some(312),
                at: Timestamp(10_000),
            }),
            disconnected(),
        ],
    );

    says_the_phone_left(&state);
    assert_eq!(
        state.contacts_state(),
        ContactsState::Unavailable(Unavailable::NotSynced)
    );
}

#[test]
fn a_phone_leaving_during_the_warning_is_waited_for_too() {
    let (state, _) = warned();
    let (state, _) = run(state, [disconnected()]);
    says_the_phone_left(&state);
}

#[test]
fn a_phone_leaving_once_contacts_are_read_finishes_without_saying_it_is_connected() {
    let (state, _) = run(
        syncing(),
        [
            Input::ContactsSyncProgress(SyncProgress::Shared),
            Input::ContactsSyncProgress(SyncProgress::CallHistory(call_history(50))),
            Input::ContactsSynced {
                contacts: contacts(312),
                call_history: call_history(50),
            },
            disconnected(),
        ],
    );

    assert!(matches!(
        setup_step(&state),
        Some(SetupStep::Done {
            connected: false,
            contacts: ContactsOutcome::Synced {
                contacts: 312,
                calls: 50
            },
            ..
        })
    ));
    assert_eq!(text(&state).heading, "My Pixel is set up");
}

#[test]
fn denied_the_finish_page_says_calls_still_work_and_how_to_allow_access_later() {
    let (state, _) = run(
        syncing(),
        [
            Input::ContactsPermissionNeeded {
                deadline: Timestamp(30_000),
            },
            Input::ContactsSyncFailed(SyncFailure::Denied),
        ],
    );

    assert_eq!(
        setup_step(&state),
        Some(done(ContactsOutcome::Denied, KeybindingStage::Checking))
    );
    let view = state.surfaces().setup_window.expect("the window is open");
    let summary = messages::setup_window(&view, Timestamp(0))
        .summary
        .unwrap_or_default();
    assert!(summary.contains("Calls still work"), "{summary}");
    assert!(
        summary.contains("turn on Contacts and call history"),
        "{summary}"
    );
}

#[test]
fn the_finish_page_offers_the_keybinding_and_writes_nothing_until_confirmed() {
    let (state, _) = run(synced(), [Input::KeybindingChecked(config(BINDINGS))]);
    assert_eq!(
        setup_step(&state),
        Some(done(
            synced_312_and_200(),
            KeybindingStage::Checked(available())
        ))
    );
    let view = state.surfaces().setup_window.expect("the window is open");
    let text = messages::setup_window(&view, Timestamp(0));
    assert_eq!(text.buttons, ["Add keybinding", "Done"]);
    assert!(
        text.instructions
            .iter()
            .any(|line| line.contains("Super + Alt + C")),
        "{:?}",
        text.instructions
    );
    assert!(text.tip.is_some_and(|tip| tip.contains("pin")));

    let (state, effects) = run(state, [Input::AddKeybinding]);
    assert!(effects.is_empty(), "nothing written yet: {effects:?}");
    let view = state.surfaces().setup_window.expect("the window is open");
    let text = messages::setup_window(&view, Timestamp(0));
    assert_eq!(text.line.as_deref(), Some(LINE));
    assert_eq!(text.buttons, ["Confirm", "Cancel"]);

    // Declining writes nothing.
    let (declined, effects) = run(state.clone(), [Input::CancelKeybinding]);
    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(
        setup_step(&declined),
        Some(done(
            synced_312_and_200(),
            KeybindingStage::Checked(available())
        ))
    );

    let (state, effects) = run(state, [Input::ConfirmKeybinding]);
    assert_eq!(
        effects,
        [Effect::AddKeybinding {
            file: FILE.to_owned(),
            seen: BINDINGS.to_owned(),
            added: format!("{BINDINGS}{LINE}\n"),
        }]
    );
    let (state, _) = run(state, [Input::KeybindingAdded]);
    assert_eq!(
        setup_step(&state),
        Some(done(
            synced_312_and_200(),
            KeybindingStage::Checked(KeybindingCheck::Added {
                keys: KEYS.to_owned(),
                file: FILE.to_owned()
            })
        ))
    );
    let view = state.surfaces().setup_window.expect("the window is open");
    let text = messages::setup_window(&view, Timestamp(0));
    assert_eq!(text.buttons, ["Done"]);
    assert!(
        text.instructions[0].starts_with("Place your first Call: press Super + Alt + C"),
        "{:?}",
        text.instructions
    );
}

#[test]
fn a_keybinding_that_could_not_be_written_is_offered_again_and_says_why() {
    let (state, _) = run(
        synced(),
        [
            Input::KeybindingChecked(config(BINDINGS)),
            Input::AddKeybinding,
            Input::ConfirmKeybinding,
            Input::KeybindingNotAdded,
        ],
    );

    assert_eq!(
        setup_step(&state),
        Some(done(
            synced_312_and_200(),
            KeybindingStage::Checked(available())
        ))
    );
    let view = state.surfaces().setup_window.expect("the window is open");
    assert_eq!(view.notice, Some(SetupNotice::KeybindingNotAdded));
}

#[test]
fn without_a_hyprland_config_to_add_to_the_finish_page_says_what_to_bind() {
    let (state, _) = run(
        synced(),
        [Input::KeybindingChecked(HyprlandConfig {
            command: COMMAND.to_owned(),
            bindings: None,
        })],
    );

    let view = state.surfaces().setup_window.expect("the window is open");
    let text = messages::setup_window(&view, Timestamp(0));
    assert_eq!(text.buttons, ["Done"]);
    assert!(
        text.instructions
            .iter()
            .any(|line| line.contains("/usr/bin/desktop-calls")),
        "{:?}",
        text.instructions
    );
}

#[test]
fn keys_taken_after_the_offer_refresh_the_offer_instead_of_asking_to_try_again() {
    let (state, effects) = run(
        synced(),
        [
            Input::KeybindingChecked(config(BINDINGS)),
            Input::AddKeybinding,
            Input::ConfirmKeybinding,
        ],
    );
    assert_eq!(effects.len(), 1, "{effects:?}");
    let taken =
        format!("{BINDINGS}o.bind(\"SUPER + ALT + C\", \"Calculator\", \"gnome-calculator\")\n");

    let (state, _) = run(state, [Input::KeybindingConfigChanged(config(&taken))]);

    assert_eq!(
        setup_step(&state),
        Some(done(
            synced_312_and_200(),
            KeybindingStage::Checked(KeybindingCheck::Taken {
                keys: KEYS.to_owned(),
                file: FILE.to_owned(),
                command: COMMAND.to_owned(),
            })
        ))
    );
    let view = state.surfaces().setup_window.expect("the window is open");
    assert_eq!(view.notice, Some(SetupNotice::KeybindingConfigChanged));
    let shown = text(&state);
    assert_eq!(shown.buttons, ["Done"]);
    assert!(
        shown
            .notice
            .is_some_and(|notice| !notice.contains("try again")),
        "{shown:?}"
    );
}

#[test]
fn a_phone_leaving_well_after_a_failed_sync_leaves_the_finish_page_as_it_is() {
    let (state, effects) = run(
        syncing(),
        [Input::ContactsSyncFailed(SyncFailure::Unavailable(
            Unavailable::Failed,
        ))],
    );
    let Some(&timer) = setup_timers(&effects).last() else {
        panic!("the moment after the failure has a timer: {effects:?}");
    };
    // Long after, on the keybinding's Confirm.
    let (state, _) = run(
        state,
        [
            Input::TimerFired(timer),
            Input::KeybindingChecked(config(BINDINGS)),
            Input::AddKeybinding,
            disconnected(),
        ],
    );

    assert!(
        matches!(
            setup_step(&state),
            Some(SetupStep::Done {
                connected: false,
                ..
            })
        ),
        "{:?}",
        setup_step(&state)
    );
}

#[test]
fn a_sync_failing_just_before_the_phone_leaves_still_says_the_phone_left() {
    // The link dropping fails the PBAP session before hands-free goes down.
    let (state, _) = run(
        syncing(),
        [
            Input::ContactsPermissionNeeded {
                deadline: Timestamp(30_000),
            },
            Input::ContactsSyncFailed(SyncFailure::Unavailable(Unavailable::Failed)),
            disconnected(),
        ],
    );
    says_the_phone_left(&state);
    assert_eq!(
        state.contacts_state(),
        ContactsState::Unavailable(Unavailable::NotSynced)
    );
    let (state, _) = reconnected(state);
    let (_, effects) = run(state, [Input::ContinueContacts]);
    assert_eq!(syncs(&effects), 1);
}

#[test]
fn a_later_sync_cut_short_keeps_the_contacts_a_finished_sync_read() {
    let (state, _) = run(synced(), [disconnected()]);
    let (state, effects) = reconnected(state);
    assert_eq!(syncs(&effects), 1, "every connect syncs");

    let (state, _) = run(state, [disconnected()]);

    assert_eq!(state.contacts_state(), ContactsState::Ready);
}

#[test]
fn a_sync_cut_short_keeps_the_cached_contacts_ready() {
    let (state, _) = run(
        State::default(),
        [
            Input::PhonePaired(pixel()),
            Input::ContactsLoaded {
                contacts: contacts(3),
                call_history: call_history(2),
            },
        ],
    );
    let (state, effects) = reconnected(state);
    assert_eq!(syncs(&effects), 1);

    let (state, _) = run(state, [disconnected()]);

    assert_eq!(state.contacts_state(), ContactsState::Ready);
}
