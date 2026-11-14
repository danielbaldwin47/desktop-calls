//! The Setup Window while calls are off, at the Call state machine's public
//! interface (ticket #207): it says so with Turn calls on, which has the
//! keyboard while a Phone is being chosen; pairing and switching Phone
//! wait until calls are on; and a Phone paired during Quiet Hours isn't
//! blocked the moment it pairs.

// Clippy exempts `#[test]` functions but not their helpers. Like a test, a
// helper whose step fails must fail the test at once with its message.
#![expect(
    clippy::expect_used,
    reason = "test code: a failed helper step fails the test with its message"
)]

use desktop_calls_core::{
    BluetoothPower, CallsOffRecord, Deadline, DesktopCheck, Effect, Input, Phone, QuietHours,
    QuietHoursClock, SetupStep, SetupWindowView, State, TimeOfDay, Timestamp, WallClock,
    messages::{self, ProblemKind},
    step,
};

const ADDRESS: &str = "AA:BB:CC:DD:EE:FF";
const OTHER: &str = "48:A4:93:00:00:7E";
/// 22:30 local time, as the shell's clock read it, with Quiet Hours from
/// 22:00 to 07:00 under way: they end 510 minutes on and begin again 1,350
/// minutes on.
const NOW: u64 = 1_800_000_000_000;
const MINUTE: u64 = 60_000;

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
    phone("My Pixel", ADDRESS)
}

fn time(hour: u8, minute: u8) -> TimeOfDay {
    TimeOfDay { hour, minute }
}

fn blocked(blocked: bool) -> Input {
    Input::PhoneBlocked {
        address: ADDRESS.to_owned(),
        blocked,
    }
}

/// The Pixel is followed and connected.
fn connected() -> State {
    run(
        State::default(),
        [
            Input::PhonePaired(pixel()),
            Input::PhonePresent {
                address: ADDRESS.to_owned(),
            },
            Input::PhoneConnected {
                address: ADDRESS.to_owned(),
            },
        ],
    )
    .0
}

/// Calls are off until turned on, the Phone blocked and dropped.
fn off() -> State {
    let (state, _) = run(
        connected(),
        [Input::TurnCallsOff { until: None }, blocked(true)],
    );
    run(
        state,
        [
            Input::PhoneDisconnected {
                address: ADDRESS.to_owned(),
            },
            Input::PhoneAbsent {
                address: ADDRESS.to_owned(),
            },
        ],
    )
    .0
}

/// The Setup Window opened, with the other Phone nearby.
fn opened(state: State) -> State {
    run(
        state,
        [
            Input::OpenSetupWindow,
            Input::SetupReady {
                desktop_name: "desktop".to_owned(),
            },
            Input::PhoneNearby(phone("Work phone", OTHER)),
        ],
    )
    .0
}

fn window(state: &State) -> SetupWindowView {
    state
        .surfaces()
        .setup_window
        .unwrap_or_else(|| panic!("the Setup Window isn't open"))
}

fn text(state: &State) -> messages::SetupWindowText {
    messages::setup_window(&window(state), Timestamp(0), None)
}

fn choose_other() -> Input {
    Input::ChoosePhone {
        address: OTHER.to_owned(),
    }
}

#[test]
fn while_calls_are_off_the_setup_window_says_so_and_offers_turn_calls_on() {
    let state = opened(off());

    let view = window(&state);
    let off = view.calls_off.expect("the window says calls are off");
    assert_eq!(off.until, None);
    assert!(!off.quiet_hours);
    let text = text(&state);
    let card = text.calls_off.expect("the window has the card");
    assert_eq!(card.heading, "Calls are off");
    assert_eq!(card.button, "Turn calls on");
    assert_eq!(
        card.text,
        "Your phone stays disconnected until you turn calls on. Pairing and switching phones wait until calls are on."
    );
}

#[test]
fn the_card_names_the_time_calls_come_back_and_whether_quiet_hours_turned_them_off() {
    let until = Some(time(7, 0));

    assert_eq!(
        messages::setup_calls_off(until, false).text,
        "Your phone stays disconnected until 07:00. Pairing and switching phones wait until calls are on."
    );
    assert_eq!(
        messages::setup_calls_off(until, true).text,
        "Quiet hours keep your phone disconnected until 07:00. Pairing and switching phones wait until calls are on."
    );
}

#[test]
fn while_choosing_turn_calls_on_has_enter_and_nothing_is_listed_to_choose() {
    let state = opened(off());

    let view = window(&state);
    assert!(view.calls_off.is_some_and(|off| off.blocks_step));
    let text = text(&state);
    assert_eq!(text.keys[0], (&["Enter"][..], "Turn calls on"));
    assert_eq!(text.empty_list, None, "no \"Looking for phones nearby…\"");
}

#[test]
fn with_calls_on_the_setup_window_says_nothing_of_calls_off() {
    let state = opened(connected());

    assert_eq!(window(&state).calls_off, None);
    let text = text(&state);
    assert_eq!(text.calls_off, None);
    assert_eq!(text.empty_list, Some("Looking for phones nearby…"));
}

#[test]
fn pairing_waits_until_calls_are_on() {
    let state = opened(off());

    let (state, effects) = run(state, [choose_other()]);

    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::PairPhone { .. })),
        "{effects:?}"
    );
    assert!(matches!(window(&state).step, SetupStep::Choose { .. }));
}

#[test]
fn a_phone_pairing_from_its_own_screen_is_turned_down_while_calls_are_off() {
    let state = opened(off());

    let (state, effects) = run(
        state,
        [Input::PairingCode {
            phone: phone("Work phone", OTHER),
            code: 123_456,
        }],
    );

    assert_eq!(effects, [Effect::AnswerPairingCode { confirm: false }]);
    assert!(matches!(window(&state).step, SetupStep::Choose { .. }));
}

#[test]
fn turning_calls_on_lets_the_window_carry_on_as_with_calls_on() {
    let state = opened(off());

    let (state, effects) = run(state, [Input::TurnCallsOn, blocked(false)]);

    assert!(effects.contains(&Effect::BlockPhone {
        address: ADDRESS.to_owned(),
        blocked: false,
    }));
    assert_eq!(window(&state).calls_off, None);
    assert_eq!(text(&state).calls_off, None);
    let (state, effects) = run(state, [choose_other()]);
    assert!(effects.contains(&Effect::PairPhone {
        address: OTHER.to_owned(),
    }));
    assert!(matches!(window(&state).step, SetupStep::Pairing { .. }));
}

#[test]
fn once_past_choosing_the_card_stays_but_the_step_keeps_enter() {
    // Calls turned off while the window pairs the other Phone.
    let (state, _) = run(
        opened(connected()),
        [
            choose_other(),
            Input::TurnCallsOff { until: None },
            blocked(true),
        ],
    );
    assert!(matches!(window(&state).step, SetupStep::Pairing { .. }));

    let view = window(&state);
    let off = view.calls_off.expect("the card shows at any step");
    assert!(!off.blocks_step);
    assert!(
        text(&state).keys[0].1 != "Turn calls on",
        "Enter belongs to the step"
    );
}

#[test]
fn turn_calls_on_comes_before_a_bluetooth_problem_for_the_keyboard() {
    let state = run(
        opened(off()),
        [Input::DesktopChecked(DesktopCheck::Bluetooth(
            BluetoothPower::Off,
        ))],
    )
    .0;

    let surfaces = state.surfaces();
    assert_eq!(
        surfaces
            .setup_problems
            .as_ref()
            .map(|problems| &problems.problems[..]),
        Some(&[ProblemKind::BluetoothOff][..]),
        "the Problem is still said"
    );
    assert!(
        surfaces
            .setup_problems
            .is_some_and(|problems| !problems.blocks_step)
    );
    assert_eq!(window(&state).problem_has_focus, None);
    assert_eq!(text(&state).keys[0].1, "Turn calls on");

    // Once calls are on, the Problem's button has the keyboard again.
    let (state, _) = run(state, [Input::TurnCallsOn, blocked(false)]);
    assert!(
        state
            .surfaces()
            .setup_problems
            .is_some_and(|problems| problems.blocks_step)
    );
    assert_eq!(
        window(&state).problem_has_focus,
        Some(ProblemKind::BluetoothOff)
    );
}

fn quiet_hours() -> QuietHours {
    QuietHours::new(time(22, 0), time(7, 0)).expect("different times")
}

/// The wall clock at 22:30, in the middle of Quiet Hours.
fn clock() -> Input {
    let at = |minutes: u64, shown: TimeOfDay| Deadline {
        at: Timestamp(NOW + minutes * MINUTE),
        shown,
    };
    Input::ClockTicked(WallClock {
        now: Timestamp(NOW),
        for_an_hour: at(60, time(23, 30)),
        until_morning: at(510, time(7, 0)),
        quiet_hours: Some(QuietHoursClock {
            hours: quiet_hours(),
            next_start: at(1_350, time(22, 0)),
            next_end: at(510, time(7, 0)),
        }),
    })
}

#[test]
fn a_phone_paired_during_quiet_hours_is_not_blocked_the_moment_it_pairs() {
    // A fresh install, Quiet Hours set and under way.
    let (state, effects) = run(
        State::default(),
        [
            Input::QuietHoursKept(quiet_hours()),
            clock(),
            Input::PairedPhonesListed,
            Input::SetupReady {
                desktop_name: "desktop".to_owned(),
            },
            Input::PhoneNearby(pixel()),
            Input::ChoosePhone {
                address: ADDRESS.to_owned(),
            },
            Input::PairingCode {
                phone: pixel(),
                code: 48_213,
            },
            Input::ConfirmPairing,
            Input::PhonePaired(pixel()),
        ],
    );

    assert!(!state.calls_off(), "calls stay on");
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::BlockPhone { blocked: true, .. })),
        "{effects:?}"
    );
    assert!(effects.contains(&Effect::ConnectPhone {
        address: ADDRESS.to_owned(),
        attempt: 1,
        away: false,
    }));
    // The choice is kept for the next run, until Quiet Hours next begin.
    let kept = effects
        .iter()
        .rev()
        .find_map(|effect| match effect {
            Effect::KeepCallsOff(record) => Some(record.clone()),
            _ => None,
        })
        .expect("the choice is kept");
    assert_eq!(
        kept,
        Some(CallsOffRecord::KeptOn {
            until: Deadline {
                at: Timestamp(NOW + 1_350 * MINUTE),
                shown: time(22, 0),
            },
        })
    );
}

#[test]
fn a_phone_paired_outside_quiet_hours_keeps_nothing() {
    let (state, effects) = run(
        State::default(),
        [
            Input::QuietHoursKept(quiet_hours()),
            Input::PairedPhonesListed,
            Input::PhonePaired(pixel()),
        ],
    );

    assert!(!state.calls_off());
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::KeepCallsOff(_))),
        "{effects:?}"
    );
}
