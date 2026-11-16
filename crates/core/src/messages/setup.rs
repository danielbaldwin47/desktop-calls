//! What the Setup Window says at each step: a heading, the steps to take
//! on the Phone, and its buttons.

use crate::call::Timestamp;
use crate::connection::Phone;
use crate::keybinding::KeybindingCheck;
use crate::machine::{
    ContactsOutcome, KeybindingStage, ReadingContacts, SetupFocus, SetupNotice, SetupStep,
    SetupWindowView,
};
use crate::messages::setup_parts::{
    Instruction, SetupCard, StageMark, SyncText, pin_card, seconds_until, stage_marks, sync,
};
use crate::messages::{
    ProblemKind, SetupCallsOffText, copy_command_label, fix_label, setup_calls_off,
};
use std::ops::Range;

/// The keys that work, each as its keycaps and what pressing it does, for
/// the foot of the window: Up and Down are two keys doing one thing.
pub type Keys = Vec<(&'static [&'static str], &'static str)>;

/// The Setup Window's text for one step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetupWindowText {
    /// The window's title.
    pub title: &'static str,
    /// How each of the four named steps across the top is marked.
    pub stages: [StageMark; 4],
    pub heading: String,
    /// What the step means, before what to do, if it needs saying.
    pub summary: Option<String>,
    /// What to do, in order, each one sentence, with where it is done.
    pub instructions: Vec<Instruction>,
    /// The six digits both screens show, while confirming.
    pub code: Option<String>,
    /// The seconds left to answer the Phone, while it asks.
    pub countdown: Option<String>,
    /// What the countdown counts.
    pub countdown_label: Option<&'static str>,
    /// The sync under way.
    pub sync: Option<SyncText>,
    /// The exact line the keybinding adds, while the user confirms it.
    pub line: Option<String>,
    /// The finish page's card offering the keybinding, or saying how to
    /// open the Dialer from a key.
    pub keybinding: Option<SetupCard>,
    /// The finish page's card on keeping Desktop Calls in the bar.
    pub tip: Option<SetupCard>,
    /// Why the last attempt didn't work.
    pub notice: Option<String>,
    /// What the list says while no Phone is nearby, while choosing.
    pub empty_list: Option<&'static str>,
    /// The card that says calls are off, with its Turn calls on button,
    /// while they are.
    pub calls_off: Option<SetupCallsOffText>,
    /// The buttons, the default one first.
    pub buttons: Vec<&'static str>,
    /// The keys that work now, shown at the foot.
    pub keys: Keys,
}

/// What the Setup Window says for `view` at `now`: the countdown and the
/// time left reading Contacts change by the second. `home` is the Desktop's
/// home directory, which a file in it is shown relative to, as `~`.
#[must_use]
pub fn setup_window(view: &SetupWindowView, now: Timestamp, home: Option<&str>) -> SetupWindowText {
    let desktop = view
        .desktop_name
        .as_deref()
        .map_or_else(|| "this computer".to_owned(), |name| format!("“{name}”"));
    let mut text = SetupWindowText {
        title: "Connect your phone",
        stages: stage_marks(&view.step),
        heading: String::new(),
        summary: None,
        instructions: Vec::new(),
        code: None,
        countdown: None,
        countdown_label: None,
        sync: None,
        line: None,
        keybinding: None,
        tip: None,
        notice: view.notice.as_ref().map(notice),
        empty_list: None,
        calls_off: view
            .calls_off
            .map(|off| setup_calls_off(off.until, off.quiet_hours)),
        buttons: Vec::new(),
        keys: Vec::new(),
    };
    let (heading, instructions, buttons): Page = match &view.step {
        SetupStep::Choose { .. } => choosing(view, &desktop, &mut text.empty_list),
        SetupStep::Pairing { phone } => (
            format!("Pairing with {}…", phone.name),
            vec![Instruction::phone("Keep the Pair new device screen open.")],
            vec!["Cancel"],
        ),
        SetupStep::Confirm {
            phone,
            code: digits,
        } => {
            text.code = Some(format!("{digits:06}"));
            (
                "Check the code".to_owned(),
                vec![
                    // One line per place, so grouping keeps the order:
                    // check and tap Pair on the Phone, then confirm here.
                    Instruction::phone(format!(
                        "Check that {} shows this code too, then tap Pair.",
                        phone.name
                    )),
                    Instruction::here("Then choose Confirm."),
                ],
                vec!["Confirm", "Cancel"],
            )
        }
        SetupStep::Confirmed { phone } => (
            format!("Pairing with {}…", phone.name),
            vec![Instruction::phone("Tap Pair, if you haven't yet.")],
            vec!["Cancel"],
        ),
        SetupStep::Connecting { phone } => (
            format!("Connecting to {}…", phone.name),
            vec![Instruction::phone(
                "If it asks to share contacts and call history, tap Allow, so calls show names.",
            )],
            vec!["Close"],
        ),
        SetupStep::PhoneCallsOff { phone } => phone_calls_off(&mut text, phone, &desktop),
        SetupStep::PhoneUnreachable { phone } => phone_unreachable(&mut text, phone),
        SetupStep::PairingLost { phone } => pairing_lost(&mut text, phone),
        SetupStep::PairingAgain { phone } => pairing_again(phone),
        SetupStep::ContactsWarning { phone } => contacts_warning(&mut text, phone),
        SetupStep::ContactsPrompt { deadline, .. } => contacts_prompt(&mut text, *deadline, now),
        SetupStep::ContactsPromptTimedOut { .. } => contacts_prompt_timed_out(&mut text),
        SetupStep::ContactsSyncing {
            call_history,
            contacts,
            contacts_read,
            ..
        } => contacts_syncing(&mut text, *call_history, *contacts, *contacts_read, now),
        SetupStep::ContactsPhoneAway { phone, back } => phone_away(&mut text, phone, *back),
        SetupStep::Done {
            phone,
            connected,
            contacts,
            keybinding,
            ..
        } => done(
            &mut text, phone, *connected, *contacts, keybinding, &desktop, home,
        ),
    };
    let buttons = with_pair_another(&view.step, &text, buttons);
    text.keys = keys(view, &buttons, text.calls_off.as_ref());
    // Escape does what a lone Cancel does: the footer says it in the
    // button's word, not a second word beside it.
    if buttons == ["Cancel"] {
        for key in &mut text.keys {
            if *key == CLOSE {
                key.1 = "Cancel";
            }
        }
    }
    text.heading = heading;
    text.instructions = instructions;
    text.buttons = buttons;
    text
}

/// The finish page's button back to choosing a Phone, when the window
/// opened on it for the Phone already set up.
pub const PAIR_ANOTHER: &str = "Pair another phone";

/// `buttons`, and on the finish page the window opened on for the Phone
/// set up, the way back to choosing a Phone, last, after Done, so never
/// Enter's (#389). Not while the keybinding's line is confirmed: that page
/// is about the line alone.
fn with_pair_another(
    step: &SetupStep,
    text: &SetupWindowText,
    mut buttons: Vec<&'static str>,
) -> Vec<&'static str> {
    if matches!(
        step,
        SetupStep::Done {
            pair_another: true,
            ..
        }
    ) && text.line.is_none()
    {
        buttons.push(PAIR_ANOTHER);
    }
    buttons
}

/// A page's heading, instructions and buttons, the default one first.
type Page = (String, Vec<Instruction>, Vec<&'static str>);

/// The page that lists the Phones nearby to choose from.
fn choose(desktop: &str) -> Page {
    (
        "Connect your phone".to_owned(),
        vec![
            Instruction::phone("Open Settings, then Connected devices."),
            Instruction::phone("Tap Pair new device, and keep that screen open."),
            Instruction::here(format!(
                "Choose your phone below, or tap {desktop} on your phone when it shows."
            )),
        ],
        vec!["Cancel"],
    )
}

/// The first page, and in `empty_list` what shows while no Phone is listed.
fn choosing(view: &SetupWindowView, desktop: &str, empty_list: &mut Option<&'static str>) -> Page {
    // Nothing to look for while pairing waits for calls.
    let calls_off = view.calls_off.is_some_and(|off| off.blocks_step);
    // While Bluetooth is off or blocked nothing is looked for, and how this
    // computer shows on phones isn't known yet (#315).
    let bluetooth_off = view.bluetooth_off;
    *empty_list = (!calls_off && !bluetooth_off).then_some("Looking for phones nearby…");
    let (heading, mut instructions, buttons) = choose(desktop);
    // A Phone tapping this computer is turned down while calls are off, and
    // nothing is looked for, so its name isn't known either: here the page
    // says only what follows once what it waits for is on (#369).
    let once = match (bluetooth_off, calls_off) {
        (true, true) => "Once Bluetooth and calls are on, choose your phone below.",
        (true, false) => "Once Bluetooth is on, choose your phone below.",
        (false, true) => "Once calls are on, choose your phone below.",
        (false, false) => return (heading, instructions, buttons),
    };
    instructions.pop();
    instructions.push(Instruction::here(once));
    (heading, instructions, buttons)
}

const ENTER: &[&str] = &["Enter"];
const CLOSE: (&[&str], &str) = (&["Esc"], "Close");
const NEXT: (&[&str], &str) = (&["Tab"], "Next button");

/// The keys that work now, for the foot of the window, Enter naming the
/// button that has the keyboard: the one the window last reported, after
/// Tab or Shift+Tab, while it is still there. Otherwise the button that
/// takes the keyboard by itself: Turn calls on while calls are off and the
/// step waits for them, a Problem's button while it blocks the step, and
/// otherwise the step's default.
fn keys(
    view: &SetupWindowView,
    buttons: &[&'static str],
    calls_off: Option<&SetupCallsOffText>,
) -> Keys {
    let blocked = view.calls_off.is_some_and(|off| off.blocks_step);
    let reported = match (view.focus, &view.step) {
        (Some(SetupFocus::TurnCallsOn), _) => calls_off.map(turn_calls_on_keys),
        (Some(SetupFocus::Problem(problem)), _) => Some(problem_keys(problem)),
        (Some(SetupFocus::Button(index)), _) => buttons
            .get(index)
            .map(|button| button_keys(buttons, button)),
        (Some(SetupFocus::Phone), SetupStep::Choose { phones }) => {
            Some(choose_keys(phones, blocked))
        }
        (Some(SetupFocus::Phone) | None, _) => None,
    };
    if let Some(keys) = reported {
        return keys;
    }
    if let Some(off) = calls_off.filter(|_| blocked) {
        return turn_calls_on_keys(off);
    }
    if let Some(problem) = view.problem_has_focus {
        return problem_keys(problem);
    }
    match (&view.step, buttons.first()) {
        (SetupStep::Choose { phones }, _) => choose_keys(phones, blocked),
        (_, Some(default)) => button_keys(buttons, default),
        (_, None) => vec![CLOSE],
    }
}

/// While choosing, a Phone listed has the keyboard. Enter pairs it unless
/// calls are off and `blocked` pairing until they are on.
fn choose_keys(phones: &[Phone], blocked: bool) -> Keys {
    let mut keys = Keys::new();
    if phones.len() > 1 {
        keys.push((&["↑", "↓"], "Choose"));
    }
    if !phones.is_empty() && !blocked {
        keys.push((ENTER, "Pair"));
    }
    keys.push(CLOSE);
    keys
}

/// `button`, one of the step's `buttons`, has the keyboard.
fn button_keys(buttons: &[&'static str], button: &'static str) -> Keys {
    match buttons {
        // Enter and Escape do one thing, but two keycaps in one entry
        // read as a chord, as Alt Q does: an entry each.
        [_] => vec![(ENTER, button), CLOSE],
        _ => vec![(ENTER, button), NEXT, CLOSE],
    }
}

fn problem_keys(problem: ProblemKind) -> Keys {
    let does = fix_label(problem).unwrap_or_else(copy_command_label);
    vec![(ENTER, does), NEXT, CLOSE]
}

fn turn_calls_on_keys(off: &SetupCallsOffText) -> Keys {
    vec![(ENTER, off.button), NEXT, CLOSE]
}

/// `file` as the Setup Window names it: relative to `home` as `~` when it
/// is in it, otherwise whole.
#[must_use]
pub fn shown_path(file: &str, home: Option<&str>) -> String {
    let Some(home) = home.map(|home| home.trim_end_matches('/')) else {
        return file.to_owned();
    };
    match file.strip_prefix(home) {
        Some(rest) if !home.is_empty() && rest.starts_with('/') => format!("~{rest}"),
        _ => file.to_owned(),
    }
}

/// The parts of `text` a line may not break inside: a line may break only
/// at a space or after a `/`, so a path wraps between its folders and never
/// in the middle of a name, hyphens and dots included.
#[must_use]
pub fn unbreakable_runs(text: &str) -> Vec<Range<usize>> {
    let mut runs = Vec::new();
    let mut start = 0;
    for (at, character) in text.char_indices() {
        if character == ' ' || character == '/' {
            let end = at + character.len_utf8();
            runs.push(start..end);
            start = end;
        }
    }
    if start < text.len() {
        runs.push(start..text.len());
    }
    runs
}

/// The step for a Phone that didn't answer.
fn phone_unreachable(text: &mut SetupWindowText, phone: &Phone) -> Page {
    text.summary = Some(format!(
        "Can't reach {}; bring it close with Bluetooth on.",
        phone.name
    ));
    (
        format!("Can't reach {}", phone.name),
        vec![
            Instruction::phone("Turn Bluetooth on, and keep it close to this computer."),
            Instruction::here("Then choose Try again."),
        ],
        vec!["Try again", "Close"],
    )
}

fn pairing_lost(text: &mut SetupWindowText, phone: &Phone) -> Page {
    text.summary = Some(format!(
        "{} no longer has this computer paired, so it can't share calls.",
        phone.name
    ));
    (
        format!("Pair {} again", phone.name),
        vec![Instruction::here(
            "Choose Pair again: this computer removes its old pairing, then pairs with your phone.",
        )],
        vec!["Pair again", "Close"],
    )
}

/// The Phone is found only while its "Pair new device" screen is open
/// (inferred from Android's behaviour; not measured on the Pixel).
fn pairing_again(phone: &Phone) -> Page {
    (
        format!("Pairing with {} again…", phone.name),
        vec![
            Instruction::phone(
                "Open Settings, then Connected devices, then Pair new device, so this computer finds it.",
            ),
            Instruction::phone("When it asks, check the code and tap Pair."),
        ],
        vec!["Cancel"],
    )
}

fn phone_calls_off(text: &mut SetupWindowText, phone: &Phone, desktop: &str) -> Page {
    text.summary = Some(format!(
        "{} is paired, but isn't sharing calls with this computer.",
        phone.name
    ));
    (
        "Turn on Phone calls".to_owned(),
        vec![
            Instruction::phone("Open Settings, then Connected devices."),
            Instruction::phone(format!("Tap the settings icon next to {desktop}.")),
            Instruction::phone("Switch Phone calls on."),
            Instruction::here("Then choose Try again."),
        ],
        vec!["Try again", "Close"],
    )
}

fn contacts_warning(text: &mut SetupWindowText, phone: &Phone) -> Page {
    text.summary = Some(format!(
        "{} is connected. Next it asks whether to share contacts and call history with this computer, so calls show names.",
        phone.name
    ));
    (
        "Your phone is about to ask".to_owned(),
        vec![Instruction::phone(
            "When it asks to allow access to contacts and call history, tap Allow.",
        )],
        vec!["Close"],
    )
}

/// What the countdown counts, so it isn't a bare number.
const COUNTDOWN_LABEL: &str = "left to tap Allow";

fn contacts_prompt(text: &mut SetupWindowText, deadline: Timestamp, now: Timestamp) -> Page {
    text.countdown = Some(seconds_until(deadline, now).to_string());
    text.countdown_label = Some(COUNTDOWN_LABEL);
    (
        "Tap Allow on your phone".to_owned(),
        vec![Instruction::phone(
            "Tap Allow to share contacts and call history.",
        )],
        vec!["Close"],
    )
}

fn contacts_prompt_timed_out(text: &mut SetupWindowText) -> Page {
    // The countdown ran out.
    text.countdown = Some("0".to_owned());
    text.countdown_label = Some("Time ran out to tap Allow");
    text.summary =
        Some("Nobody answered on your phone in time, so it isn't sharing contacts yet.".to_owned());
    (
        "Your phone stopped asking".to_owned(),
        vec![
            Instruction::here("Choose Try again, and your phone asks again."),
            Instruction::phone("Tap Allow."),
        ],
        vec!["Try again", "Skip"],
    )
}

/// The finish page: what was synced, the first Call and the keybinding.
/// It says the Phone is connected only while it is.
fn done(
    text: &mut SetupWindowText,
    phone: &Phone,
    connected: bool,
    contacts: ContactsOutcome,
    keybinding: &KeybindingStage,
    desktop: &str,
    home: Option<&str>,
) -> Page {
    text.summary = Some(contacts_outcome(contacts, desktop));
    let (instructions, card, buttons) = finish(keybinding, &mut text.line, home);
    // While the line is confirmed, the page is about the line alone.
    if card.is_some() {
        text.tip = Some(pin_card());
    }
    text.keybinding = card;
    let heading = if connected {
        format!("{} is connected", phone.name)
    } else {
        format!("{} is set up", phone.name)
    };
    (heading, instructions, buttons)
}

/// The sync under way: recent Calls, then Contacts.
fn contacts_syncing(
    text: &mut SetupWindowText,
    call_history: bool,
    contacts: Option<ReadingContacts>,
    contacts_read: bool,
    now: Timestamp,
) -> Page {
    text.sync = Some(sync(call_history, contacts, contacts_read, now));
    (
        "Reading contacts and recent calls…".to_owned(),
        Vec::new(),
        vec!["Close"],
    )
}

/// The Phone left before Contacts were read: wait for it, then carry on.
fn phone_away(text: &mut SetupWindowText, phone: &Phone, back: bool) -> Page {
    if back {
        text.summary = Some("Contacts and recent calls aren't read yet.".to_owned());
        return (
            format!("{} is back", phone.name),
            vec![Instruction::here(
                "Choose Continue, and your phone asks again whether to share contacts and call history.",
            )],
            vec!["Continue", "Close"],
        );
    }
    text.summary = Some(format!(
        "{} disconnected before contacts and recent calls were read.",
        phone.name
    ));
    (
        format!("{} disconnected", phone.name),
        vec![
            Instruction::phone("Check that Bluetooth is on, and keep it near this computer."),
            Instruction::here("Once it connects again, choose Continue."),
        ],
        vec!["Close"],
    )
}

fn plural(count: impl Into<u64>, one: &str, many: &str) -> String {
    match count.into() {
        1 => format!("1 {one}"),
        count => format!("{count} {many}"),
    }
}

fn contacts_outcome(outcome: ContactsOutcome, desktop: &str) -> String {
    match outcome {
        ContactsOutcome::Synced { contacts, calls } => format!(
            "Synced {} and {}.",
            plural(count(contacts), "contact", "contacts"),
            plural(count(calls), "recent call", "recent calls")
        ),
        ContactsOutcome::Denied => format!(
            "Your phone isn't sharing contacts. Calls still work, and show numbers instead of names. To allow it later: on your phone open Settings, then Connected devices, tap the settings icon next to {desktop} and turn on Contacts and call history."
        ),
        ContactsOutcome::NotShared => {
            "Your phone isn't sharing contacts yet. Calls still work, and show numbers instead of names."
                .to_owned()
        }
        ContactsOutcome::Syncing => "Getting contacts and recent calls from your phone…".to_owned(),
        ContactsOutcome::Asking => {
            "Your phone is asking whether to share contacts. Tap Allow on it so calls show names."
                .to_owned()
        }
    }
}

fn count(count: usize) -> u64 {
    u64::try_from(count).unwrap_or(u64::MAX)
}

/// The finish page's keybinding: its card, or, while the user confirms it,
/// the exact line and where it goes. The Hyprland config's file is named
/// relative to `home`; the line the user confirms, and the command to bind,
/// are shown exactly as they are written.
fn finish(
    keybinding: &KeybindingStage,
    line: &mut Option<String>,
    home: Option<&str>,
) -> (Vec<Instruction>, Option<SetupCard>, Vec<&'static str>) {
    let card = |text: String| {
        Some(SetupCard {
            title: "Open the dialer from anywhere",
            text,
        })
    };
    // Until a key opens the Dialer, the first Call starts from the bar.
    let from_the_bar = || {
        vec![Instruction::here(
            "Place your first call: choose New call… from Desktop Calls in the bar, type a name or number, then press Enter.",
        )]
    };
    let check = match keybinding {
        KeybindingStage::Checking => {
            let text = "Checking the Hyprland config for a free key…".to_owned();
            return (from_the_bar(), card(text), vec!["Done"]);
        }
        KeybindingStage::Confirming(KeybindingCheck::Available {
            file,
            line: shown,
            before_return,
            ..
        }) => {
            *line = Some(shown.clone());
            let file = shown_path(file, home);
            // One instruction ahead of the line it is about, so nothing to
            // do sits between the line and its buttons (#326).
            let place = if *before_return {
                format!("{file}, just before its final return")
            } else {
                format!("the end of {file}")
            };
            return (
                vec![Instruction::here(format!(
                    "Choose Confirm to add this line to {place}, or Cancel to leave the file as it is:"
                ))],
                None,
                vec!["Confirm", "Cancel"],
            );
        }
        KeybindingStage::Checked(check) | KeybindingStage::Confirming(check) => check,
    };
    let (text, buttons) = match check {
        KeybindingCheck::Available { keys, file, .. } => (
            format!(
                "Add keybinding binds {} in {}, which doesn't use those keys yet. You'll see the exact line first.",
                shown_keys(keys),
                shown_path(file, home)
            ),
            vec!["Add keybinding", "Done"],
        ),
        KeybindingCheck::Added { keys, .. } => (
            format!(
                "Place your first call: press {} to open the dialer, type a name or number, then press Enter.",
                shown_keys(keys)
            ),
            vec!["Done"],
        ),
        KeybindingCheck::Taken {
            keys,
            file,
            command,
        } => (
            format!(
                "{} is already used in {}. To open the dialer from a key, bind another one to run {command}",
                shown_keys(keys),
                shown_path(file, home)
            ),
            vec!["Done"],
        ),
        KeybindingCheck::Unreadable { file, command } => (
            format!(
                "{} couldn't be read, so no keybinding can be added to it; to open the dialer from a key, bind one to run {command}",
                shown_path(file, home)
            ),
            vec!["Done"],
        ),
        KeybindingCheck::NoConfig { command } => (
            format!("To open the dialer from a key, bind one to run {command}"),
            vec!["Done"],
        ),
    };
    let instructions = if matches!(check, KeybindingCheck::Added { .. }) {
        // The keybinding card says how, with the keys.
        Vec::new()
    } else {
        from_the_bar()
    };
    (instructions, card(text), buttons)
}

/// Keys as Hyprland names them, `SUPER + ALT + C`, as people read them:
/// `Super+Alt+C`, written as tooltips write `Ctrl+H`.
pub(super) fn shown_keys(keys: &str) -> String {
    keys.split('+')
        .map(|key| {
            let key = key.trim();
            let mut letters = key.chars();
            letters.next().map_or_else(String::new, |first| {
                first
                    .to_uppercase()
                    .chain(letters.flat_map(char::to_lowercase))
                    .collect()
            })
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// Why the last attempt didn't work, or why the window opened, and what to
/// do about it.
fn notice(notice: &SetupNotice) -> String {
    let text = match notice {
        SetupNotice::ChosenPhoneUnpaired { name } => {
            return format!(
                "{} isn't paired with this computer any more; pair it again, or choose another phone.",
                name.as_deref().unwrap_or("The phone you chose last")
            );
        }
        SetupNotice::BluetoothUnavailable => {
            "This computer's Bluetooth is off or missing; turn it on to connect your phone."
        }
        SetupNotice::PairingRejected => "Pairing was turned down; choose your phone to try again.",
        SetupNotice::PairingUnreachable => {
            "Your phone didn't answer; check its Bluetooth is on and it's nearby, then try again."
        }
        SetupNotice::PairingFailed => "Pairing didn't finish; choose your phone to try again.",
        SetupNotice::PairingNotRemoved => {
            "This computer couldn't remove its old pairing with your phone; remove your phone in this computer's Bluetooth settings, then choose Pair again."
        }
        SetupNotice::KeybindingNotAdded => {
            "The keybinding couldn't be added to the Hyprland config; choose Add keybinding to try again."
        }
        SetupNotice::KeybindingConfigChanged => {
            "The Hyprland config changed after the line was shown, so nothing was written. Check the updated line."
        }
    };
    text.to_owned()
}
