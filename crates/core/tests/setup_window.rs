//! The Setup Window at the Call state machine's public interface: opening
//! by itself with no Phone set up, pairing a nearby Phone with the same
//! code on both screens, pairing started from the Phone, checking that the
//! Phone shares Calls, and switching to another Phone.

use desktop_calls_core::{
    ConnectFailure, ConnectionState, ContactsOutcome, Effect, IndicatorEntry, Input,
    KeybindingStage, PairingFailure, Phone, PhoneCommand, SetupNotice, SetupStep, SetupWindowView,
    State, Timer, Timestamp,
    messages::{self, ProblemKind},
    step,
};

const PHONE: &str = "AA:BB:CC:DD:EE:FF";
const OTHER: &str = "48:A4:93:00:00:7E";

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

fn phone(name: &str, address: &str) -> Phone {
    Phone {
        name: name.to_owned(),
        address: address.to_owned(),
    }
}

fn pixel() -> Phone {
    phone("My Pixel", PHONE)
}

fn setup_step(state: &State) -> Option<SetupStep> {
    state.surfaces().setup_window.map(|window| window.step)
}

/// The window's wait timers the effects started.
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

/// A fresh install: BlueZ listed no paired Phone, and the Pixel is nearby.
fn choosing() -> State {
    run(
        State::default(),
        [
            Input::PairedPhonesListed,
            Input::SetupReady {
                desktop_name: "desktop".to_owned(),
            },
            Input::PhoneNearby(pixel()),
        ],
    )
    .0
}

/// The Pixel paired through the window, and BlueZ said so.
fn paired() -> (State, Vec<Effect>) {
    run(
        choosing(),
        [
            Input::ChoosePhone {
                address: PHONE.to_owned(),
            },
            Input::PairingCode {
                phone: pixel(),
                code: 48_213,
            },
            Input::ConfirmPairing,
            Input::PhonePaired(pixel()),
        ],
    )
}

#[test]
fn with_no_phone_set_up_the_setup_window_opens_by_itself_and_starts_looking() {
    let (state, effects) = run(State::default(), [Input::PairedPhonesListed]);

    assert_eq!(effects, [Effect::StartSetup]);
    assert_eq!(
        setup_step(&state),
        Some(SetupStep::Choose { phones: vec![] })
    );
}

#[test]
fn with_a_phone_set_up_the_setup_window_stays_closed() {
    let (state, effects) = run(
        State::default(),
        [Input::PhonePaired(pixel()), Input::PairedPhonesListed],
    );

    assert!(!effects.contains(&Effect::StartSetup));
    assert_eq!(setup_step(&state), None);
}

#[test]
fn nearby_phones_are_listed_by_name_in_the_order_found_and_leave_when_gone() {
    let (state, _) = run(
        choosing(),
        [
            Input::PhoneNearby(phone("Work phone", OTHER)),
            Input::PhoneNearby(phone("Pixel 8", PHONE)),
        ],
    );
    assert_eq!(
        setup_step(&state),
        Some(SetupStep::Choose {
            phones: vec![phone("Pixel 8", PHONE), phone("Work phone", OTHER)]
        })
    );

    let (state, _) = run(
        state,
        [Input::PhoneNotNearby {
            address: PHONE.to_owned(),
        }],
    );
    assert_eq!(
        setup_step(&state),
        Some(SetupStep::Choose {
            phones: vec![phone("Work phone", OTHER)]
        })
    );
}

#[test]
fn choosing_a_phone_pairs_it_and_shows_its_code_until_confirmed() {
    let (state, effects) = run(
        choosing(),
        [Input::ChoosePhone {
            address: PHONE.to_owned(),
        }],
    );
    assert_eq!(
        effects,
        [Effect::PairPhone {
            address: PHONE.to_owned()
        }]
    );
    assert_eq!(
        setup_step(&state),
        Some(SetupStep::Pairing { phone: pixel() })
    );

    let (state, _) = run(
        state,
        [Input::PairingCode {
            phone: pixel(),
            code: 48_213,
        }],
    );
    assert_eq!(
        setup_step(&state),
        Some(SetupStep::Confirm {
            phone: pixel(),
            code: 48_213
        })
    );

    let (state, effects) = run(state, [Input::ConfirmPairing]);
    assert!(effects.contains(&Effect::AnswerPairingCode { confirm: true }));
    assert_eq!(
        setup_step(&state),
        Some(SetupStep::Confirmed { phone: pixel() })
    );
}

#[test]
fn once_paired_the_phone_is_followed_trusted_and_connected() {
    let (state, effects) = paired();

    assert_eq!(state.phone(), Some(&pixel()));
    assert_eq!(state.connection_state(), &ConnectionState::Connecting);
    for wanted in [
        Effect::AttachPhone {
            address: PHONE.to_owned(),
        },
        Effect::TrustPhone {
            address: PHONE.to_owned(),
        },
        Effect::ConnectPhone {
            address: PHONE.to_owned(),
            attempt: 1,
            away: false,
        },
    ] {
        assert!(effects.contains(&wanted), "{wanted:?} in {effects:?}");
    }
    assert_eq!(
        setup_step(&state),
        Some(SetupStep::Connecting { phone: pixel() })
    );
}

#[test]
fn once_the_phone_is_connected_for_calls_the_window_goes_on_to_contacts() {
    let (state, _) = paired();
    let (state, _) = run(
        state,
        [
            Input::ConnectPhoneDone { attempt: 1 },
            Input::PhonePresent {
                address: PHONE.to_owned(),
            },
            Input::PhoneConnected {
                address: PHONE.to_owned(),
            },
            Input::PhoneCommandDone(PhoneCommand::KeepAudioOnPhone),
        ],
    );

    assert_eq!(state.connection_state(), &ConnectionState::Connected);
    assert_eq!(
        setup_step(&state),
        Some(SetupStep::ContactsWarning { phone: pixel() })
    );
}

#[test]
fn a_phone_pairing_from_its_own_screen_shows_the_same_code() {
    let (state, _) = run(
        choosing(),
        [Input::PairingCode {
            phone: pixel(),
            code: 7,
        }],
    );
    assert_eq!(
        setup_step(&state),
        Some(SetupStep::Confirm {
            phone: pixel(),
            code: 7
        })
    );

    let (state, _) = run(state, [Input::ConfirmPairing, Input::PhonePaired(pixel())]);
    assert_eq!(state.phone(), Some(&pixel()));
    assert_eq!(
        setup_step(&state),
        Some(SetupStep::Connecting { phone: pixel() })
    );
}

#[test]
fn with_the_setup_window_closed_pairing_requests_are_turned_down() {
    let (state, effects) = run(
        choosing(),
        [
            Input::CloseSetupWindow,
            Input::PairingCode {
                phone: pixel(),
                code: 7,
            },
        ],
    );

    assert_eq!(
        effects,
        [
            Effect::StopSetup,
            Effect::AnswerPairingCode { confirm: false }
        ]
    );
    assert_eq!(setup_step(&state), None);
}

#[test]
fn a_rejected_pairing_goes_back_to_the_list_and_says_why() {
    let (state, _) = run(
        choosing(),
        [
            Input::ChoosePhone {
                address: PHONE.to_owned(),
            },
            Input::PairingFailed(PairingFailure::Rejected),
        ],
    );

    let window = state.surfaces().setup_window.expect("the Setup Window");
    assert_eq!(
        window.step,
        SetupStep::Choose {
            phones: vec![pixel()]
        }
    );
    assert_eq!(window.notice, Some(SetupNotice::PairingRejected));
}

#[test]
fn a_phone_connected_without_hands_free_is_asked_for_it_alone() {
    let (state, _) = paired();
    let (state, effects) = run(state, [Input::ConnectPhoneDone { attempt: 1 }]);
    let timers = setup_timers(&effects);
    assert_eq!(timers.len(), 1);

    let (state, effects) = run(state, [Input::TimerFired(timers[0])]);
    assert!(effects.contains(&Effect::ConnectHandsFree {
        address: PHONE.to_owned()
    }));
    assert_eq!(
        setup_step(&state),
        Some(SetupStep::Connecting { phone: pixel() })
    );

    let (state, _) = run(
        state,
        [
            Input::PhoneConnected {
                address: PHONE.to_owned(),
            },
            Input::PhoneCommandDone(PhoneCommand::KeepAudioOnPhone),
        ],
    );
    assert_eq!(
        setup_step(&state),
        Some(SetupStep::ContactsWarning { phone: pixel() })
    );
}

#[test]
fn a_phone_that_did_not_answer_for_hands_free_says_to_bring_it_close_and_connects_it_again() {
    let (state, _) = paired();
    let (state, effects) = run(state, [Input::ConnectPhoneDone { attempt: 1 }]);
    let (state, _) = run(state, [Input::TimerFired(setup_timers(&effects)[0])]);

    let (state, _) = run(state, [Input::HandsFreeUnreachable]);
    assert_eq!(
        setup_step(&state),
        Some(SetupStep::PhoneUnreachable { phone: pixel() })
    );

    // Try again connects the Phone afresh: it may be back.
    let (state, effects) = run(state, [Input::RetryHandsFree]);
    assert!(effects.contains(&Effect::ConnectPhone {
        address: PHONE.to_owned(),
        attempt: 2,
        away: false,
    }));
    assert_eq!(
        setup_step(&state),
        Some(SetupStep::Connecting { phone: pixel() })
    );
}

#[test]
fn a_phone_refusing_hands_free_says_where_to_switch_phone_calls_on_and_tries_again() {
    let (state, _) = paired();
    let (state, effects) = run(state, [Input::ConnectPhoneDone { attempt: 1 }]);
    let (state, _) = run(state, [Input::TimerFired(setup_timers(&effects)[0])]);

    let (state, _) = run(state, [Input::HandsFreeRefused]);
    assert_eq!(
        setup_step(&state),
        Some(SetupStep::PhoneCallsOff { phone: pixel() })
    );

    let (_, effects) = run(state, [Input::RetryHandsFree]);
    assert!(effects.contains(&Effect::ConnectHandsFree {
        address: PHONE.to_owned()
    }));
}

#[test]
fn pairing_a_second_phone_switches_to_it() {
    let (state, _) = run(
        State::default(),
        [
            Input::PhonePaired(phone("Old phone", OTHER)),
            Input::PairedPhonesListed,
            Input::OpenSetupWindow,
            Input::PhoneNearby(pixel()),
        ],
    );
    assert_eq!(state.phone(), Some(&phone("Old phone", OTHER)));

    let (state, effects) = run(
        state,
        [
            Input::ChoosePhone {
                address: PHONE.to_owned(),
            },
            Input::PairingCode {
                phone: pixel(),
                code: 1,
            },
            Input::ConfirmPairing,
            Input::PhonePaired(pixel()),
        ],
    );

    assert_eq!(state.phone(), Some(&pixel()));
    assert!(effects.contains(&Effect::DetachPhone));
    assert!(effects.contains(&Effect::AttachPhone {
        address: PHONE.to_owned()
    }));
}

#[test]
fn the_setup_window_opens_on_request_and_closing_stops_the_looking() {
    let (state, _) = run(
        State::default(),
        [Input::PhonePaired(pixel()), Input::PairedPhonesListed],
    );

    let (state, effects) = run(state, [Input::OpenSetupWindow]);
    assert_eq!(effects, [Effect::StartSetup]);
    assert!(state.surfaces().setup_window.is_some());

    let (state, effects) = run(state, [Input::CloseSetupWindow]);
    assert_eq!(effects, [Effect::StopSetup]);
    assert_eq!(state.surfaces().setup_window, None);
}

#[test]
fn pair_again_after_the_phone_forgot_the_pairing_pairs_from_the_phone() {
    let address = || PHONE.to_owned();
    let (state, _) = run(
        State::default(),
        [
            Input::PhonePaired(pixel()),
            Input::PairedPhonesListed,
            Input::PhoneAbsent { address: address() },
            Input::ConnectPhoneFailed {
                attempt: 1,
                failure: ConnectFailure::PairingLost,
            },
        ],
    );
    assert_eq!(state.connection_state(), &ConnectionState::PairingLost);
    let pair_again = IndicatorEntry::Problem(ProblemKind::PairingLost)
        .intent()
        .expect("the Problem offers Pair again");
    assert_eq!(pair_again, Input::OpenSetupWindow);

    // The person holding the Phone pairs from its Pair new device screen.
    let (state, effects) = run(
        state,
        [
            pair_again,
            Input::PhonePresent { address: address() },
            Input::PairingCode {
                phone: pixel(),
                code: 990_001,
            },
            Input::ConfirmPairing,
            Input::PhoneConnected { address: address() },
            Input::PhoneCommandDone(PhoneCommand::KeepAudioOnPhone),
        ],
    );

    assert!(effects.contains(&Effect::AnswerPairingCode { confirm: true }));
    assert!(state.problems().is_empty());
    assert_eq!(
        setup_step(&state),
        Some(SetupStep::ContactsWarning { phone: pixel() })
    );
}

#[test]
fn a_phone_paired_again_that_does_not_connect_by_itself_is_connected() {
    let address = || PHONE.to_owned();
    let (state, _) = run(
        State::default(),
        [
            Input::PhonePaired(pixel()),
            Input::PairedPhonesListed,
            Input::PhoneAbsent { address: address() },
            Input::ConnectPhoneFailed {
                attempt: 1,
                failure: ConnectFailure::PairingLost,
            },
            Input::OpenSetupWindow,
            Input::PairingCode {
                phone: pixel(),
                code: 990_001,
            },
        ],
    );
    let (state, effects) = run(state, [Input::ConfirmPairing]);
    let (state, effects) = run(state, [Input::TimerFired(setup_timers(&effects)[0])]);

    assert!(effects.contains(&Effect::ConnectPhone {
        address: address(),
        attempt: 2,
        away: true,
    }));
    assert_eq!(
        setup_step(&state),
        Some(SetupStep::Connecting { phone: pixel() })
    );
}

#[test]
fn each_step_says_which_keys_work_and_numbers_only_what_to_do() {
    let view = |step| SetupWindowView {
        step,
        desktop_name: Some("desktop".to_owned()),
        notice: None,
        opened: 1,
        problem_has_focus: None,
    };
    let choose = messages::setup_window(
        &view(SetupStep::Choose {
            phones: vec![pixel()],
        }),
        Timestamp(0),
        None,
    );
    assert_eq!(
        choose.keys,
        [(&["Enter"][..], "Pair"), (&["Esc"][..], "Close")],
        "one Phone has nothing to move between"
    );
    let two = messages::setup_window(
        &view(SetupStep::Choose {
            phones: vec![pixel(), phone("Other", OTHER)],
        }),
        Timestamp(0),
        None,
    );
    assert_eq!(
        two.keys,
        [
            (&["↑", "↓"][..], "Choose"),
            (&["Enter"][..], "Pair"),
            (&["Esc"][..], "Close")
        ]
    );
    let none = messages::setup_window(
        &view(SetupStep::Choose { phones: vec![] }),
        Timestamp(0),
        None,
    );
    assert_eq!(
        none.keys,
        [(&["Esc"][..], "Close")],
        "with no Phone listed, only Escape works"
    );

    let confirm = messages::setup_window(
        &view(SetupStep::Confirm {
            phone: pixel(),
            code: 42,
        }),
        Timestamp(0),
        None,
    );
    assert_eq!(confirm.code.as_deref(), Some("000042"));
    assert_eq!(
        confirm.keys,
        [
            (&["Enter"][..], "Confirm"),
            (&["Tab"][..], "Next button"),
            (&["Esc"][..], "Close")
        ]
    );
}

#[test]
fn the_summary_is_said_once_and_a_single_button_shares_its_key_with_escape() {
    let view = |step| SetupWindowView {
        step,
        desktop_name: Some("desktop".to_owned()),
        notice: None,
        opened: 1,
        problem_has_focus: None,
    };
    let off = messages::setup_window(
        &view(SetupStep::PhoneCallsOff { phone: pixel() }),
        Timestamp(0),
        None,
    );
    assert_eq!(
        off.summary.as_deref(),
        Some("My Pixel is paired, but isn't sharing Calls with this computer.")
    );
    assert!(
        off.instructions
            .iter()
            .all(|line| !line.contains("is paired"))
    );

    let done = messages::setup_window(
        &view(SetupStep::Done {
            connected: true,
            phone: pixel(),
            contacts: ContactsOutcome::Synced {
                contacts: 1,
                calls: 1,
            },
            keybinding: KeybindingStage::Checking,
        }),
        Timestamp(0),
        None,
    );
    assert_eq!(
        done.summary.as_deref(),
        Some("Synced 1 Contact and 1 recent Call.")
    );
    assert!(
        done.instructions
            .iter()
            .all(|line| !line.contains("Synced"))
    );
    assert_eq!(
        done.keys,
        [(&["Enter"][..], "Done"), (&["Esc"][..], "Close")]
    );
    let syncing = messages::setup_window(
        &view(SetupStep::Pairing { phone: pixel() }),
        Timestamp(0),
        None,
    );
    assert_eq!(
        syncing.keys,
        [(&["Enter"][..], "Cancel"), (&["Esc"][..], "Close")]
    );
    let close_only = messages::setup_window(
        &view(SetupStep::Connecting { phone: pixel() }),
        Timestamp(0),
        None,
    );
    assert_eq!(
        close_only.keys,
        [(&["Enter", "Esc"][..], "Close")],
        "Enter and Escape both close, so one entry says it"
    );
}

#[test]
fn a_problem_says_bluetooth_is_off_once_and_the_window_adds_no_second_sentence() {
    use desktop_calls_core::{BluetoothPower, DesktopCheck};
    let off = Input::DesktopChecked(DesktopCheck::Bluetooth(BluetoothPower::Off));
    let (state, _) = run(choosing(), [off, Input::SetupUnavailable]);
    let window = state.surfaces().setup_window.expect("the Setup Window");
    assert_eq!(window.notice, None, "the Problem card says it");
    let problems = state.surfaces().setup_problems.expect("the Problem");
    assert_eq!(problems.problems, [ProblemKind::BluetoothOff]);
    assert!(problems.blocks_step);
    assert_eq!(window.problem_has_focus, Some(ProblemKind::BluetoothOff));

    // With no Bluetooth adapter at all there is no Problem to say it, so
    // the window does.
    let (state, _) = run(choosing(), [Input::SetupUnavailable]);
    let window = state.surfaces().setup_window.expect("the Setup Window");
    assert_eq!(window.notice, Some(SetupNotice::BluetoothUnavailable));
    assert_eq!(state.surfaces().setup_problems, None);
    assert_eq!(window.problem_has_focus, None);
}

#[test]
fn a_file_in_the_home_directory_is_named_with_a_tilde_and_other_files_whole() {
    assert_eq!(
        messages::shown_path("/home/user/.config/hypr/bindings.lua", Some("/home/user")),
        "~/.config/hypr/bindings.lua"
    );
    assert_eq!(
        messages::shown_path("/home/user/.config/hypr/bindings.lua", Some("/home/user/")),
        "~/.config/hypr/bindings.lua",
        "a trailing slash on the home directory changes nothing"
    );
    assert_eq!(
        messages::shown_path("/home/user-b/.config", Some("/home/user")),
        "/home/user-b/.config",
        "a folder that only starts with the same letters is not the home"
    );
    assert_eq!(
        messages::shown_path("/etc/hypr/bindings.lua", Some("/home/user")),
        "/etc/hypr/bindings.lua"
    );
    assert_eq!(
        messages::shown_path("/home/user/.config", None),
        "/home/user/.config"
    );
    assert_eq!(messages::shown_path("/x/y", Some("")), "/x/y");
    assert_eq!(messages::shown_path("/x/y", Some("/")), "/x/y");
}

/// The text of `text` cut into the pieces a line may not break inside.
fn pieces(text: &str) -> Vec<&str> {
    messages::unbreakable_runs(text)
        .into_iter()
        .map(|run| &text[run])
        .collect()
}

#[test]
fn a_path_may_break_only_after_a_slash_never_inside_a_name() {
    assert_eq!(
        pieces("~/repos/desktop-calls/.claude/bindings.lua"),
        ["~/", "repos/", "desktop-calls/", ".claude/", "bindings.lua"]
    );
    // Spaces are break points too, so a sentence wraps between words.
    assert_eq!(
        pieces("goes into ~/.config/a-b.lua, then"),
        ["goes ", "into ", "~/", ".config/", "a-b.lua, ", "then"]
    );
    assert_eq!(
        pieces(r#"o.bind("SUPER + ALT + C", "Desktop Calls", "/usr/bin/desktop-calls")"#),
        [
            "o.bind(\"SUPER ",
            "+ ",
            "ALT ",
            "+ ",
            "C\", ",
            "\"Desktop ",
            "Calls\", ",
            "\"/",
            "usr/",
            "bin/",
            "desktop-calls\")"
        ]
    );
    // Together the pieces are the whole text, so nothing is left out.
    let text = "/home/user/ünï-code/x y/";
    assert_eq!(pieces(text).concat(), text);
    assert!(messages::unbreakable_runs("").is_empty());
}
