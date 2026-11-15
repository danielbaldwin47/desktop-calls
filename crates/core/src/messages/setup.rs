//! What the Setup Window says at each step: a heading, the steps to take
//! on the Phone, and its buttons.

use crate::call::Timestamp;
use crate::connection::Phone;
use crate::keybinding::KeybindingCheck;
use crate::machine::{
    ContactsOutcome, KeybindingStage, ReadingContacts, SetupNotice, SetupStep, SetupWindowView,
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
        SetupStep::Choose { .. } => {
            // Nothing to look for while pairing waits for calls.
            let waits = view.calls_off.is_some_and(|off| off.blocks_step);
            text.empty_list = (!waits).then_some("Looking for phones nearby…");
            choose(&desktop)
        }
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
                    // One line per place, so grouping keeps the order.
                    Instruction::here(format!(
                        "Check that {} shows this code too, then choose Confirm.",
                        phone.name
                    )),
                    Instruction::phone("Tap Pair."),
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
                "If it asks to share contacts and call history, tap Allow, so Calls show names.",
            )],
            vec!["Close"],
        ),
        SetupStep::PhoneCallsOff { phone } => phone_calls_off(&mut text, phone, &desktop),
        SetupStep::PhoneUnreachable { phone } => phone_unreachable(&mut text, phone),
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
        } => done(
            &mut text, phone, *connected, *contacts, keybinding, &desktop, home,
        ),
    };
    let turns_on = text
        .calls_off
        .as_ref()
        .filter(|_| view.calls_off.is_some_and(|off| off.blocks_step));
    text.keys = keys(&view.step, &buttons, view.problem_has_focus, turns_on);
    text.heading = heading;
    text.instructions = instructions;
    text.buttons = buttons;
    text
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

/// The keys that work now, for the foot of the window: the Problem's button
/// when it has the keyboard, otherwise the step's. While calls are off and
/// the step waits for them, Turn calls on has the keyboard.
fn keys(
    step: &SetupStep,
    buttons: &[&'static str],
    problem: Option<ProblemKind>,
    turns_on: Option<&SetupCallsOffText>,
) -> Keys {
    const ENTER: &[&str] = &["Enter"];
    const CLOSE: (&[&str], &str) = (&["Esc"], "Close");
    const NEXT: (&[&str], &str) = (&["Tab"], "Next button");
    if let Some(off) = turns_on {
        return vec![(ENTER, off.button), NEXT, CLOSE];
    }
    if let Some(problem) = problem {
        let does = fix_label(problem).unwrap_or_else(copy_command_label);
        return vec![(ENTER, does), NEXT, CLOSE];
    }
    match (step, buttons) {
        (SetupStep::Choose { phones }, _) => {
            let mut keys = Keys::new();
            if phones.len() > 1 {
                keys.push((&["↑", "↓"], "Choose"));
            }
            if !phones.is_empty() {
                keys.push((ENTER, "Pair"));
            }
            keys.push(CLOSE);
            keys
        }
        // Enter and Escape do one thing: one entry says it.
        (_, ["Close"]) => vec![(&["Enter", "Esc"], "Close")],
        (_, [only]) => vec![(ENTER, only), CLOSE],
        (_, buttons) => vec![
            (ENTER, buttons.first().copied().unwrap_or_default()),
            NEXT,
            CLOSE,
        ],
    }
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

fn phone_calls_off(text: &mut SetupWindowText, phone: &Phone, desktop: &str) -> Page {
    text.summary = Some(format!(
        "{} is paired, but isn't sharing Calls with this computer.",
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
        "{} is connected. Next it asks whether to share contacts and call history with this computer, so Calls show names.",
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
            "Tap Allow, to share contacts and call history.",
        )],
        vec!["Close"],
    )
}

fn contacts_prompt_timed_out(text: &mut SetupWindowText) -> Page {
    // The countdown ran out.
    text.countdown = Some("0".to_owned());
    text.countdown_label = Some(COUNTDOWN_LABEL);
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
        "Reading Contacts and recent Calls…".to_owned(),
        Vec::new(),
        vec!["Close"],
    )
}

/// The Phone left before Contacts were read: wait for it, then carry on.
fn phone_away(text: &mut SetupWindowText, phone: &Phone, back: bool) -> Page {
    if back {
        text.summary = Some("Contacts and recent Calls aren't read yet.".to_owned());
        return (
            format!("{} is back", phone.name),
            vec![Instruction::here(
                "Choose Continue, and your phone asks again whether to share contacts and call history.",
            )],
            vec!["Continue", "Close"],
        );
    }
    text.summary = Some(format!(
        "{} disconnected before contacts and recent Calls were read.",
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
            plural(count(contacts), "Contact", "Contacts"),
            plural(count(calls), "recent Call", "recent Calls")
        ),
        ContactsOutcome::Denied => format!(
            "Your phone isn't sharing contacts. Calls still work, and show numbers instead of names. To allow it later: on your phone open Settings, then Connected devices, tap the settings icon next to {desktop} and turn on Contacts and call history."
        ),
        ContactsOutcome::NotShared => {
            "Your phone isn't sharing contacts yet. Calls still work, and show numbers instead of names."
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
            title: "Open the Dialer from anywhere",
            text,
        })
    };
    let check = match keybinding {
        KeybindingStage::Checking => {
            let text = "Checking the Hyprland config for a free key…".to_owned();
            return (Vec::new(), card(text), vec!["Done"]);
        }
        KeybindingStage::Confirming(KeybindingCheck::Available {
            file,
            line: shown,
            before_return,
            ..
        }) => {
            *line = Some(shown.clone());
            let file = shown_path(file, home);
            let place = if *before_return {
                format!("This line goes into {file}, just before its final return:")
            } else {
                format!("This line goes at the end of {file}:")
            };
            return (
                vec![
                    Instruction::here(place),
                    Instruction::here(
                        "Choose Confirm to add it, or Cancel to leave the file as it is.",
                    ),
                ],
                None,
                vec!["Confirm", "Cancel"],
            );
        }
        KeybindingStage::Checked(check) | KeybindingStage::Confirming(check) => check,
    };
    let (text, buttons) = match check {
        KeybindingCheck::Available { keys, file, .. } => (
            format!(
                "Add keybinding puts {} in {}, which doesn't use those keys yet. You see the exact line first.",
                shown_keys(keys),
                shown_path(file, home)
            ),
            vec!["Add keybinding", "Done"],
        ),
        KeybindingCheck::Added { keys, .. } => (
            format!(
                "Place your first Call: press {} to open the Dialer, type a name or number, then press Enter.",
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
                "{} already does something else in {}; to open the Dialer from a key, bind another one to run {command}",
                shown_keys(keys),
                shown_path(file, home)
            ),
            vec!["Done"],
        ),
        KeybindingCheck::Unreadable { file, command } => (
            format!(
                "{} couldn't be read, so no keybinding can be added to it; to open the Dialer from a key, bind one to run {command}",
                shown_path(file, home)
            ),
            vec!["Done"],
        ),
        KeybindingCheck::NoConfig { command } => (
            format!("To open the Dialer from a key, bind one to run {command}"),
            vec!["Done"],
        ),
    };
    (Vec::new(), card(text), buttons)
}

/// Keys as Hyprland names them, `SUPER + ALT + C`, as people read them:
/// `Super + Alt + C`.
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
        .join(" + ")
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
        SetupNotice::KeybindingNotAdded => {
            "The keybinding couldn't be added to the Hyprland config; choose Add keybinding to try again."
        }
        SetupNotice::KeybindingConfigChanged => {
            "The Hyprland config changed after the line was shown, so nothing was written; this is what it can take now."
        }
    };
    text.to_owned()
}
