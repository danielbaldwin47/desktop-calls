//! The Incoming Call Banner's view and words, at the core's public
//! interface (#226): each Waiting Call choice says what happens to the
//! user's other Call, naming the right one, and the footer names the keys
//! that work now.

use desktop_calls_core::messages::{self, FooterPart};
use desktop_calls_core::{
    BannerChoice, BannerFooter, BannerView, CallId, CallState, Caller, ConfigFile, Contact,
    ContactNumber, Contacts, Directory, IndicatorEntry, Input, NumberLabel, Phone, PhoneCommand,
    State, Timestamp, WaitingOptions, step,
};

const ADDRESS: &str = "AA:BB:CC:DD:EE:FF";
const FIRST: CallId = CallId(1);
const SECOND: CallId = CallId(2);
const THIRD: CallId = CallId(3);
const ALICE: &str = "+447700900123";
const JOSE: &str = "+447700900001";
const STRANGER: &str = "+447700900456";

fn run(state: State, inputs: impl IntoIterator<Item = Input>) -> State {
    inputs
        .into_iter()
        .fold(state, |state, input| step(state, input).0)
}

fn number(number: &str) -> Caller {
    Caller::Number(number.to_owned())
}

fn mobile(name: &str, number: &str) -> Contact {
    Contact {
        name: name.to_owned(),
        numbers: vec![ContactNumber {
            number: number.to_owned(),
            label: NumberLabel::Mobile,
        }],
    }
}

/// Connected, with Alice and José in Contacts.
fn connected() -> State {
    let directory = Directory::new(
        vec![mobile("Alice Smith", ALICE), mobile("José Smith", JOSE)],
        Some("GB"),
    );
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
            Input::ContactsSynced {
                contacts: Contacts::new(directory),
                call_history: desktop_calls_core::CallHistory::default(),
            },
        ],
    )
}

fn added(call: CallId, state: CallState, caller: &str, at: u64) -> Input {
    Input::CallAdded {
        call,
        state,
        caller: number(caller),
        at: Timestamp(at),
    }
}

fn changed(call: CallId, state: CallState, at: u64) -> Input {
    Input::CallChanged {
        call,
        state,
        at: Timestamp(at),
    }
}

/// On a Call with `first`, live, when `second` calls.
fn waiting_during(first: &str, second: &str) -> State {
    run(
        connected(),
        [
            added(FIRST, CallState::Ringing, first, 1_000),
            changed(FIRST, CallState::Active, 2_000),
            added(SECOND, CallState::Waiting, second, 3_000),
        ],
    )
}

fn banner(state: &State) -> BannerView {
    state
        .surfaces()
        .banner
        .unwrap_or_else(|| panic!("the Banner shows"))
}

fn consequence(view: &BannerView, choice: BannerChoice) -> Option<String> {
    messages::banner_consequence(view, choice)
}

#[test]
fn hold_and_answer_names_who_waits_on_hold_and_end_and_answer_whose_call_ends() {
    let view = banner(&waiting_during(ALICE, JOSE));

    assert_eq!(view.waiting, Some(WaitingOptions::HoldOrEnd));
    assert_eq!(
        view.choices(),
        [
            BannerChoice::HoldAndAnswer,
            BannerChoice::EndAndAnswer,
            BannerChoice::Decline
        ]
    );
    assert_eq!(
        consequence(&view, BannerChoice::HoldAndAnswer).as_deref(),
        Some("Alice Smith waits on hold")
    );
    assert_eq!(
        consequence(&view, BannerChoice::EndAndAnswer).as_deref(),
        Some("Ends the Call with Alice")
    );
    assert_eq!(consequence(&view, BannerChoice::Decline), None);
}

/// The number is held together by no-break spaces, so where the line
/// wraps it never splits the number.
#[test]
fn a_call_with_a_bare_number_is_named_by_its_number() {
    let view = banner(&waiting_during(STRANGER, JOSE));

    assert_eq!(
        consequence(&view, BannerChoice::HoldAndAnswer).as_deref(),
        Some("+44\u{a0}7700\u{a0}900456 waits on hold")
    );
    assert_eq!(
        consequence(&view, BannerChoice::EndAndAnswer).as_deref(),
        Some("Ends the Call with +44\u{a0}7700\u{a0}900456")
    );
}

#[test]
fn while_placing_a_call_end_and_answer_names_the_call_being_placed() {
    let state = run(
        connected(),
        [
            added(FIRST, CallState::Dialing, ALICE, 1_000),
            changed(FIRST, CallState::Alerting, 1_500),
            added(SECOND, CallState::Waiting, JOSE, 2_000),
        ],
    );
    let view = banner(&state);

    assert_eq!(view.waiting, Some(WaitingOptions::End));
    assert_eq!(
        view.choices(),
        [BannerChoice::EndAndAnswer, BannerChoice::Decline]
    );
    assert_eq!(
        consequence(&view, BannerChoice::EndAndAnswer).as_deref(),
        Some("Ends the Call with Alice")
    );
}

/// Alice on hold, José live, and a stranger calls.
fn three_calls() -> State {
    run(
        waiting_during(ALICE, JOSE),
        [
            Input::HoldAndAnswer,
            changed(FIRST, CallState::Held, 4_000),
            changed(SECOND, CallState::Active, 4_000),
            added(THIRD, CallState::Waiting, STRANGER, 5_000),
        ],
    )
}

#[test]
fn with_a_call_on_hold_end_and_answer_names_the_live_call_not_the_held_one() {
    let view = banner(&three_calls());

    assert_eq!(view.call, THIRD);
    assert_eq!(view.waiting, Some(WaitingOptions::End));
    assert_eq!(
        consequence(&view, BannerChoice::EndAndAnswer).as_deref(),
        Some("Ends the Call with José")
    );
}

#[test]
fn with_only_a_held_call_answer_says_that_call_stays_on_hold() {
    let state = run(
        three_calls(),
        [Input::CallEnded {
            call: SECOND,
            at: Timestamp(6_000),
        }],
    );
    let view = banner(&state);

    assert_eq!(view.waiting, Some(WaitingOptions::Answer));
    assert_eq!(
        view.choices(),
        [BannerChoice::Answer, BannerChoice::Decline]
    );
    assert_eq!(
        consequence(&view, BannerChoice::Answer).as_deref(),
        Some("Alice Smith stays on hold")
    );
}

#[test]
fn the_only_call_ringing_offers_answer_and_decline_with_no_consequence() {
    let state = run(
        connected(),
        [added(FIRST, CallState::Ringing, ALICE, 1_000)],
    );
    let view = banner(&state);

    assert_eq!(view.waiting, None);
    assert_eq!(
        view.choices(),
        [BannerChoice::Answer, BannerChoice::Decline]
    );
    assert_eq!(consequence(&view, BannerChoice::Answer), None);
}

fn with_bindings(text: &str) -> Input {
    Input::KeybindingRead(desktop_calls_core::HyprlandConfig {
        command: "/usr/bin/desktop-calls".to_owned(),
        bindings: Some(ConfigFile {
            path: "/home/user/.config/hypr/bindings.lua".to_owned(),
            text: Some(text.to_owned()),
            required: Vec::new(),
        }),
    })
}

fn footer(state: &State, letters: &[&str]) -> Option<String> {
    banner(state)
        .footer
        .map(|footer| messages::footer_text(&messages::banner_footer(&footer, letters)))
}

#[test]
fn the_footer_names_the_keybinding_set_then_the_letters_that_act() {
    let ringing = run(
        connected(),
        [added(FIRST, CallState::Ringing, ALICE, 1_000)],
    );
    assert_eq!(
        banner(&ringing).footer,
        None,
        "nothing said before the config is read"
    );

    let bound = run(
        ringing.clone(),
        [with_bindings(
            "o.bind(\"SUPER + ALT + C\", \"Desktop Calls\", \"/usr/bin/desktop-calls\")\n",
        )],
    );
    assert_eq!(
        banner(&bound).footer,
        Some(BannerFooter::Keybinding(vec![
            "SUPER".to_owned(),
            "ALT".to_owned(),
            "C".to_owned()
        ]))
    );
    assert_eq!(
        footer(&bound, &["A", "D"]).as_deref(),
        Some("Super Alt C then A or D")
    );
    assert_eq!(
        messages::banner_footer(&banner(&bound).footer.expect("a footer"), &["A", "D"]),
        [
            FooterPart::Key("Super".to_owned()),
            FooterPart::Key("Alt".to_owned()),
            FooterPart::Key("C".to_owned()),
            FooterPart::Words("then"),
            FooterPart::Key("A".to_owned()),
            FooterPart::Words("or"),
            FooterPart::Key("D".to_owned()),
        ]
    );
    assert_eq!(
        footer(&bound, &["H", "E", "D"]).as_deref(),
        Some("Super Alt C then H, E or D")
    );

    let unbound = run(
        ringing,
        [with_bindings("-- Keep only your overrides here.\n")],
    );
    assert_eq!(banner(&unbound).footer, Some(BannerFooter::NoKeybinding));
    assert_eq!(
        footer(&unbound, &["A", "D"]).as_deref(),
        Some("No keybinding set. Add one in Connect your phone…")
    );
    // It names the Indicator's entry in that entry's own words.
    let indicator = unbound.indicator(&unbound.surfaces());
    let entry = messages::indicator_entry(&IndicatorEntry::ConnectPhone, &indicator);
    assert!(
        footer(&unbound, &["A", "D"]).is_some_and(|footer| footer.ends_with(&entry)),
        "the footer names {entry:?}"
    );
}

#[test]
fn with_the_keyboard_the_footer_says_escape_gives_it_back() {
    let state = run(
        connected(),
        [
            with_bindings(
                "o.bind(\"SUPER + ALT + C\", \"Desktop Calls\", \"/usr/bin/desktop-calls\")\n",
            ),
            added(FIRST, CallState::Ringing, ALICE, 1_000),
            Input::Activate,
        ],
    );
    let view = banner(&state);

    assert!(view.focused);
    assert_eq!(view.footer, Some(BannerFooter::GiveKeyboardBack));
    assert_eq!(
        footer(&state, &["A", "D"]).as_deref(),
        Some("Esc gives the keyboard back; the Call keeps ringing")
    );
}
