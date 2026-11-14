//! Setup checks: the Desktop's own problems (Bluetooth off or blocked,
//! Contacts support missing, audio not set up for Calls, BlueZ's own
//! hands-free competing), decided by the Call state machine, at its public
//! interface (ticket #23).

use desktop_calls_core::messages::{self, FooterKeys, NextAction, ProblemKind};
use desktop_calls_core::{
    BluetoothPower, DesktopCheck, DesktopFix, Effect, FixFailure, IndicatorEntry, IndicatorLook,
    Input, Phone, PhoneCommand, State, Timer, Timestamp, step,
};

const ADDRESS: &str = "AA:BB:CC:DD:EE:FF";

fn run(state: State, inputs: impl IntoIterator<Item = Input>) -> State {
    inputs
        .into_iter()
        .fold(state, |state, input| step(state, input).0)
}

fn checked(check: DesktopCheck) -> Input {
    Input::DesktopChecked(check)
}

fn connected() -> State {
    run(
        State::default(),
        [
            Input::PhonePaired(Phone {
                name: "My Pixel".to_owned(),
                address: ADDRESS.to_owned(),
            }),
            Input::PhoneConnected {
                address: ADDRESS.to_owned(),
            },
            Input::PhoneCommandDone(PhoneCommand::KeepAudioOnPhone),
        ],
    )
}

/// A Desktop where every check passed.
fn all_well(state: State) -> State {
    run(
        state,
        [
            checked(DesktopCheck::Bluetooth(BluetoothPower::On)),
            checked(DesktopCheck::ContactsSupport(true)),
            checked(DesktopCheck::AudioSetUp(true)),
            checked(DesktopCheck::BluezHandsFree(false)),
        ],
    )
}

fn kinds(state: &State) -> Vec<ProblemKind> {
    state
        .problems()
        .into_iter()
        .map(|problem| problem.kind)
        .collect()
}

fn setup_open(state: State) -> State {
    run(state, [Input::OpenSetupWindow])
}

/// A Phone nearby, for the window to list.
fn nearby() -> Input {
    Input::PhoneNearby(Phone {
        name: "Other Phone".to_owned(),
        address: "11:22:33:44:55:66".to_owned(),
    })
}

#[test]
fn a_desktop_where_every_check_passes_has_no_problems() {
    let state = all_well(connected());
    assert!(kinds(&state).is_empty());
    assert!(!state.indicator(&state.surfaces()).needs_attention);
}

#[test]
fn nothing_is_a_problem_before_it_has_been_checked() {
    assert!(kinds(&connected()).is_empty());
}

#[test]
fn each_failed_check_is_a_problem_in_order_of_what_to_fix_first() {
    let state = run(
        all_well(connected()),
        [
            checked(DesktopCheck::BluezHandsFree(true)),
            checked(DesktopCheck::AudioSetUp(false)),
            checked(DesktopCheck::ContactsSupport(false)),
            checked(DesktopCheck::Bluetooth(BluetoothPower::Off)),
        ],
    );
    assert_eq!(
        kinds(&state),
        [
            ProblemKind::BluetoothOff,
            ProblemKind::BluezHandsFree,
            ProblemKind::AudioNotSetUp,
            ProblemKind::ContactsSupportMissing,
        ]
    );
}

#[test]
fn bluetooth_blocked_is_its_own_problem_with_the_command_that_unblocks_it() {
    let state = run(
        all_well(connected()),
        [checked(DesktopCheck::Bluetooth(BluetoothPower::Blocked))],
    );
    assert_eq!(kinds(&state), [ProblemKind::BluetoothBlocked]);
    assert_eq!(
        messages::problem(ProblemKind::BluetoothBlocked).next_action,
        Some(NextAction::Command("rfkill unblock bluetooth"))
    );
    assert_eq!(messages::fix_label(ProblemKind::BluetoothBlocked), None);
}

#[test]
fn contacts_support_missing_gives_the_exact_install_command() {
    assert_eq!(
        messages::problem(ProblemKind::ContactsSupportMissing).next_action,
        Some(NextAction::Command("sudo pacman -S --needed bluez-obex"))
    );
}

#[test]
fn each_problem_asks_for_attention_and_its_menu_entry_opens_the_setup_window() {
    for check in [
        DesktopCheck::Bluetooth(BluetoothPower::Off),
        DesktopCheck::Bluetooth(BluetoothPower::Blocked),
        DesktopCheck::ContactsSupport(false),
        DesktopCheck::AudioSetUp(false),
        DesktopCheck::BluezHandsFree(true),
    ] {
        let state = run(all_well(connected()), [checked(check)]);
        let view = state.indicator(&state.surfaces());
        assert!(view.needs_attention, "{check:?}");
        let [kind] = kinds(&state)[..] else {
            panic!("one Problem for {check:?}");
        };
        let entry = IndicatorEntry::Problem(kind);
        assert!(view.menu.contains(&entry), "{check:?}");
        assert_eq!(entry.intent(), Some(Input::OpenSetupWindow), "{check:?}");
    }
}

#[test]
fn bluetooth_off_shows_the_problem_look_even_with_the_phone_away() {
    let state = run(
        all_well(connected()),
        [
            checked(DesktopCheck::Bluetooth(BluetoothPower::Off)),
            Input::PhoneDisconnected {
                address: ADDRESS.to_owned(),
            },
        ],
    );
    assert_eq!(
        state.indicator(&state.surfaces()).look,
        IndicatorLook::Problem
    );
}

#[test]
fn no_problem_text_carries_a_raw_dbus_name() {
    for kind in [
        ProblemKind::BluetoothOff,
        ProblemKind::BluetoothBlocked,
        ProblemKind::ContactsSupportMissing,
        ProblemKind::AudioNotSetUp,
        ProblemKind::BluezHandsFree,
    ] {
        let text = messages::problem(kind).text;
        for raw in ["org.", "Error", "Telephony1", "obex", "RejectSCO"] {
            assert!(!text.contains(raw), "{kind:?}: {text}");
        }
        assert!(!messages::problem_heading(kind).contains("org."));
    }
}

#[test]
fn the_setup_window_shows_the_desktop_problems_only_while_it_is_open() {
    let state = run(
        all_well(connected()),
        [checked(DesktopCheck::Bluetooth(BluetoothPower::Off))],
    );
    assert_eq!(state.surfaces().setup_problems, None);
    let state = setup_open(state);
    let shown = state
        .surfaces()
        .setup_problems
        .expect("shown in the window");
    assert_eq!(shown.problems, [ProblemKind::BluetoothOff]);
    assert_eq!(shown.fixing, None);
    assert_eq!(shown.notice, None);
}

/// The keys shown at the foot of the Setup Window, while it is open.
fn key_hint(state: &State) -> Option<FooterKeys> {
    let window = state.surfaces().setup_window?;
    Some(messages::setup_window(&window, Timestamp(0), None).keys)
}

#[test]
fn the_key_hint_does_not_promise_enter_pairs_while_a_problem_button_has_focus() {
    let state = run(
        setup_open(run(
            all_well(connected()),
            [checked(DesktopCheck::Bluetooth(BluetoothPower::Off))],
        )),
        [nearby()],
    );
    assert!(
        state
            .surfaces()
            .setup_problems
            .is_some_and(|problems| problems.blocks_step),
        "Turn on has keyboard focus"
    );
    assert_eq!(
        key_hint(&state),
        Some(vec![
            (&["Enter"][..], "Turn on"),
            (&["Tab"][..], "Next button"),
            (&["Esc"][..], "Close"),
        ]),
        "Enter presses the Problem's button"
    );

    let state = run(
        state,
        [checked(DesktopCheck::Bluetooth(BluetoothPower::On))],
    );
    assert_eq!(
        key_hint(&state),
        Some(vec![(&["Enter"][..], "Pair"), (&["Esc"][..], "Close")]),
        "with Bluetooth on, Enter pairs again"
    );
}

#[test]
fn a_blocked_bluetooth_problem_with_a_command_to_copy_names_that_button_for_enter() {
    let state = setup_open(run(
        all_well(connected()),
        [checked(DesktopCheck::Bluetooth(BluetoothPower::Blocked))],
    ));
    assert_eq!(
        key_hint(&state).map(|keys| keys.first().copied()),
        Some(Some((&["Enter"][..], "Copy command")))
    );
}

#[test]
fn the_key_hint_keeps_enter_pairs_when_a_problem_takes_no_focus() {
    let state = run(
        setup_open(run(
            all_well(connected()),
            [checked(DesktopCheck::ContactsSupport(false))],
        )),
        [nearby()],
    );
    assert_eq!(
        key_hint(&state),
        Some(vec![(&["Enter"][..], "Pair"), (&["Esc"][..], "Close")])
    );
    let shown = state.surfaces().setup_problems.expect("shown");
    assert!(
        !shown.blocks_step,
        "pairing still works, so nothing is dimmed"
    );
}

#[test]
fn turn_on_asks_for_bluetooth_once_and_shows_it_is_turning_on() {
    let state = setup_open(run(
        all_well(connected()),
        [checked(DesktopCheck::Bluetooth(BluetoothPower::Off))],
    ));
    let (state, effects) = step(state, Input::Fix(ProblemKind::BluetoothOff));
    assert_eq!(effects, [Effect::Fix(DesktopFix::TurnOnBluetooth)]);
    let shown = state.surfaces().setup_problems.expect("shown");
    assert_eq!(shown.fixing, Some(DesktopFix::TurnOnBluetooth));
    // A second press while it turns on asks nothing more.
    let (_, effects) = step(state, Input::Fix(ProblemKind::BluetoothOff));
    assert!(effects.is_empty(), "{effects:?}");
}

#[test]
fn bluetooth_coming_on_while_the_setup_window_is_open_starts_looking_for_phones_again() {
    let state = setup_open(run(
        all_well(State::default()),
        [checked(DesktopCheck::Bluetooth(BluetoothPower::Off))],
    ));
    let state = run(state, [Input::SetupUnavailable]);
    let (state, effects) = step(state, checked(DesktopCheck::Bluetooth(BluetoothPower::On)));
    assert!(effects.contains(&Effect::StartSetup), "{effects:?}");
    assert!(kinds(&state).is_empty());
    assert_eq!(state.surfaces().setup_problems, None);
}

#[test]
fn bluetooth_turned_on_elsewhere_with_the_window_closed_asks_nothing() {
    let state = run(
        all_well(connected()),
        [checked(DesktopCheck::Bluetooth(BluetoothPower::Off))],
    );
    let (_, effects) = step(state, checked(DesktopCheck::Bluetooth(BluetoothPower::On)));
    assert!(!effects.contains(&Effect::StartSetup), "{effects:?}");
}

#[test]
fn a_fix_that_fails_says_so_and_can_be_tried_again() {
    let state = setup_open(run(
        all_well(connected()),
        [checked(DesktopCheck::AudioSetUp(false))],
    ));
    let (state, effects) = step(state, Input::Fix(ProblemKind::AudioNotSetUp));
    assert_eq!(effects, [Effect::Fix(DesktopFix::SetUpAudio)]);
    let state = run(state, [Input::FixFailed(FixFailure::AudioNotSaved)]);
    let shown = state.surfaces().setup_problems.expect("shown");
    assert_eq!(shown.fixing, None);
    assert_eq!(shown.notice, Some(FixFailure::AudioNotSaved));
    assert_eq!(kinds(&state), [ProblemKind::AudioNotSetUp]);
    assert!(
        messages::fix_failed(FixFailure::AudioNotSaved, "/tmp/settings/wireplumber")
            .contains("/tmp/settings/wireplumber")
    );

    let (state, effects) = step(state, Input::Fix(ProblemKind::AudioNotSetUp));
    assert_eq!(effects, [Effect::Fix(DesktopFix::SetUpAudio)]);
    assert_eq!(state.surfaces().setup_problems.expect("shown").notice, None);
}

#[test]
fn audio_saved_but_not_restarted_stays_a_problem_with_its_notice_until_a_restart_works() {
    let state = setup_open(run(
        all_well(connected()),
        [checked(DesktopCheck::AudioSetUp(false))],
    ));
    let state = run(
        state,
        [
            Input::Fix(ProblemKind::AudioNotSetUp),
            Input::FixFailed(FixFailure::AudioNotRestarted),
            Input::TimerFired(Timer::CheckDesktop),
            // The settings are saved, so the files now read as set up.
            checked(DesktopCheck::AudioSetUp(true)),
        ],
    );
    let shown = state.surfaces().setup_problems.expect("still shown");
    assert_eq!(shown.problems, [ProblemKind::AudioNotSetUp]);
    assert_eq!(shown.notice, Some(FixFailure::AudioNotRestarted));

    let (state, effects) = step(state, Input::Fix(ProblemKind::AudioNotSetUp));
    assert_eq!(effects, [Effect::Fix(DesktopFix::SetUpAudio)]);
    let state = run(state, [Input::FixDone(DesktopFix::SetUpAudio)]);
    assert!(kinds(&state).is_empty(), "{:?}", kinds(&state));
}

#[test]
fn audio_set_up_is_checked_again_once_done() {
    let state = setup_open(run(
        all_well(connected()),
        [checked(DesktopCheck::AudioSetUp(false))],
    ));
    let state = run(state, [Input::Fix(ProblemKind::AudioNotSetUp)]);
    let (state, effects) = step(state, Input::FixDone(DesktopFix::SetUpAudio));
    assert!(effects.contains(&Effect::CheckDesktop), "{effects:?}");
    let state = run(state, [checked(DesktopCheck::AudioSetUp(true))]);
    assert!(kinds(&state).is_empty());
}

#[test]
fn problems_that_cant_be_watched_are_checked_again_every_few_seconds_until_fixed() {
    let (state, effects) = step(
        all_well(connected()),
        checked(DesktopCheck::ContactsSupport(false)),
    );
    let timer = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::StartTimer {
                timer: Timer::CheckDesktop,
                after_ms,
            } => Some(*after_ms),
            _ => None,
        })
        .expect("a timer to check again");
    assert!(timer <= 5_000);
    // The same finding again doesn't start a second timer.
    let (state, effects) = step(state, checked(DesktopCheck::ContactsSupport(false)));
    assert!(effects.is_empty(), "{effects:?}");
    let (state, effects) = step(state, Input::TimerFired(Timer::CheckDesktop));
    assert_eq!(effects, [Effect::CheckDesktop]);
    let (state, effects) = step(state, checked(DesktopCheck::ContactsSupport(false)));
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::StartTimer {
                timer: Timer::CheckDesktop,
                ..
            }
        )),
        "{effects:?}"
    );
    // Once fixed, the timer that fires checks nothing more.
    let state = run(state, [checked(DesktopCheck::ContactsSupport(true))]);
    let (_, effects) = step(state, Input::TimerFired(Timer::CheckDesktop));
    assert!(effects.is_empty(), "{effects:?}");
}

#[test]
fn audio_is_not_restarted_during_a_call() {
    let state = setup_open(run(
        all_well(connected()),
        [
            checked(DesktopCheck::AudioSetUp(false)),
            Input::CallAdded {
                call: desktop_calls_core::CallId(1),
                state: desktop_calls_core::CallState::Active,
                caller: desktop_calls_core::Caller::Number("+447700900123".to_owned()),
                at: desktop_calls_core::Timestamp(1_000),
            },
        ],
    ));
    let (state, effects) = step(state, Input::Fix(ProblemKind::AudioNotSetUp));
    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(
        state.surfaces().setup_problems.expect("shown").notice,
        Some(FixFailure::CallInProgress)
    );
}

#[test]
fn only_bluetooth_off_and_audio_have_a_fix_button() {
    assert_eq!(
        messages::fix_label(ProblemKind::BluetoothOff),
        Some("Turn on")
    );
    assert_eq!(
        messages::fix_label(ProblemKind::AudioNotSetUp),
        Some("Set up audio")
    );
    for kind in [
        ProblemKind::ContactsSupportMissing,
        ProblemKind::BluezHandsFree,
        ProblemKind::ContactsDenied,
    ] {
        assert_eq!(messages::fix_label(kind), None);
        let (_, effects) = step(setup_open(connected()), Input::Fix(kind));
        assert!(effects.is_empty(), "{kind:?}: {effects:?}");
    }
}
