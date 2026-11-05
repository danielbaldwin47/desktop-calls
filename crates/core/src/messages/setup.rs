//! What the Setup Window says at each step: a heading, the steps to take
//! on the Phone, and its buttons.

use crate::call::Timestamp;
use crate::connection::Phone;
use crate::keybinding::KeybindingCheck;
use crate::machine::{
    ContactsOutcome, KeybindingStage, ReadingContacts, SetupNotice, SetupStep, SetupWindowView,
};

/// The Setup Window's text for one step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetupWindowText {
    /// The window's title.
    pub title: &'static str,
    pub heading: String,
    /// What the step means, before what to do, if it needs saying.
    pub summary: Option<String>,
    /// What to do, in order, each one sentence.
    pub instructions: Vec<String>,
    /// The six digits both screens show, while confirming.
    pub code: Option<String>,
    /// The seconds left to answer the Phone, while it asks.
    pub countdown: Option<String>,
    /// How far the sync has got, one line for each part.
    pub progress: Vec<String>,
    /// The exact line the keybinding adds, while the user confirms it.
    pub line: Option<String>,
    /// A tip, on the finish page.
    pub tip: Option<&'static str>,
    /// Why the last attempt didn't work.
    pub notice: Option<&'static str>,
    /// What the list says while no Phone is nearby, while choosing.
    pub empty_list: Option<&'static str>,
    /// The buttons, the default one first.
    pub buttons: Vec<&'static str>,
    /// The keys that work, shown at the foot.
    pub keys: String,
}

/// What the Setup Window says for `view` at `now`: the countdown and the
/// time left reading Contacts change by the second.
#[must_use]
pub fn setup_window(view: &SetupWindowView, now: Timestamp) -> SetupWindowText {
    let desktop = view
        .desktop_name
        .as_deref()
        .map_or_else(|| "this computer".to_owned(), |name| format!("“{name}”"));
    let mut text = SetupWindowText {
        title: "Connect your phone",
        heading: String::new(),
        summary: None,
        instructions: Vec::new(),
        code: None,
        countdown: None,
        progress: Vec::new(),
        line: None,
        tip: None,
        notice: view.notice.map(notice),
        empty_list: None,
        buttons: Vec::new(),
        keys: String::new(),
    };
    let (heading, instructions, buttons): Page =
        match &view.step {
            SetupStep::Choose { .. } => {
                text.empty_list = Some("Looking for phones nearby…");
                (
                    "Connect your phone".to_owned(),
                    vec![
                        "On your phone: open Settings, then Connected devices.".to_owned(),
                        "On your phone: tap Pair new device, and keep that screen open.".to_owned(),
                        format!(
                            "Choose your phone below, or on your phone tap {desktop} when it shows."
                        ),
                    ],
                    vec!["Cancel"],
                )
            }
            SetupStep::Pairing { phone } => (
                format!("Pairing with {}…", phone.name),
                vec!["On your phone: keep the Pair new device screen open.".to_owned()],
                vec!["Cancel"],
            ),
            SetupStep::Confirm { phone, code: digits } => {
                text.code = Some(format!("{digits:06}"));
                (
                    "Check the code".to_owned(),
                    vec![
                        format!("Check that {} shows this code too.", phone.name),
                        "On your phone: tap Pair.".to_owned(),
                        "Then choose Confirm here.".to_owned(),
                    ],
                    vec!["Confirm", "Cancel"],
                )
            }
            SetupStep::Confirmed { phone } => (
                format!("Pairing with {}…", phone.name),
                vec!["On your phone: tap Pair, if you haven't yet.".to_owned()],
                vec!["Cancel"],
            ),
            SetupStep::Connecting { phone } => (
                format!("Connecting to {}…", phone.name),
                vec![
                    "On your phone: if it asks to share contacts and call history, tap Allow, so Calls show names."
                        .to_owned(),
                ],
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
            } => done(&mut text, phone, *connected, *contacts, keybinding, &desktop),
        };
    text.keys = if view.problem_has_focus {
        "Enter presses the highlighted button; Tab moves between buttons; Escape closes".to_owned()
    } else {
        keys(&view.step, &buttons)
    };
    text.heading = heading;
    text.instructions = instructions;
    text.buttons = buttons;
    text
}

/// A page's heading, instructions and buttons, the default one first.
type Page = (String, Vec<String>, Vec<&'static str>);

/// The keys that work, for the foot of the window.
fn keys(step: &SetupStep, buttons: &[&'static str]) -> String {
    match (step, buttons) {
        (SetupStep::Choose { .. }, _) => {
            "Up and Down choose a phone, Enter pairs it; Escape closes".to_owned()
        }
        (_, [only]) => format!("Enter: {only}; Escape closes"),
        (_, buttons) => format!(
            "Enter: {}; Tab moves between buttons; Escape closes",
            buttons.first().copied().unwrap_or_default()
        ),
    }
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
            "On your phone: turn Bluetooth on, and keep it close to this computer.".to_owned(),
            "Then choose Try again here.".to_owned(),
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
            "On your phone: open Settings, then Connected devices.".to_owned(),
            format!("On your phone: tap the settings icon next to {desktop}."),
            "On your phone: switch Phone calls on.".to_owned(),
            "Then choose Try again here.".to_owned(),
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
        vec![
            "On your phone: when it asks to allow access to contacts and call history, tap Allow."
                .to_owned(),
        ],
        vec!["Close"],
    )
}

fn contacts_prompt(text: &mut SetupWindowText, deadline: Timestamp, now: Timestamp) -> Page {
    text.countdown = Some(seconds_until(deadline, now).to_string());
    text.summary = Some("Seconds left before your phone stops asking.".to_owned());
    (
        "Tap Allow on your phone".to_owned(),
        vec!["On your phone: tap Allow, to share contacts and call history.".to_owned()],
        vec!["Close"],
    )
}

fn contacts_prompt_timed_out(text: &mut SetupWindowText) -> Page {
    // The countdown ran out.
    text.countdown = Some("0".to_owned());
    text.summary =
        Some("Nobody answered on your phone in time, so it isn't sharing contacts yet.".to_owned());
    (
        "Your phone stopped asking".to_owned(),
        vec![
            "Choose Try again, and your phone asks again.".to_owned(),
            "On your phone: tap Allow.".to_owned(),
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
) -> Page {
    text.summary = Some(contacts_outcome(contacts, desktop));
    text.tip = Some(
        "Tip: in Omarchy, to keep Desktop Calls in the bar, right-click the arrow at the bar's tray and pin Desktop Calls.",
    );
    let (instructions, buttons) = finish(keybinding, &mut text.line);
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
    text.progress = vec![
        if call_history {
            "✓ Recent Calls".to_owned()
        } else {
            "Recent Calls…".to_owned()
        },
        contacts_progress(contacts, contacts_read, now),
    ];
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
            vec![
                "Choose Continue, and your phone asks again whether to share contacts and call history."
                    .to_owned(),
            ],
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
            "On your phone: check that Bluetooth is on, and keep it near this computer.".to_owned(),
            "Once it connects again, choose Continue here.".to_owned(),
        ],
        vec!["Close"],
    )
}

/// Whole seconds from `now` until `then`, rounded up; 0 once past.
fn seconds_until(then: Timestamp, now: Timestamp) -> u64 {
    then.0.saturating_sub(now.0).div_ceil(1_000)
}

fn contacts_progress(reading: Option<ReadingContacts>, read: bool, now: Timestamp) -> String {
    if read {
        return "✓ Contacts".to_owned();
    }
    let Some(ReadingContacts {
        count: Some(count),
        done_by,
    }) = reading
    else {
        return "Contacts…".to_owned();
    };
    let contacts = plural(count, "Contact", "Contacts");
    match done_by.map(|done_by| seconds_until(done_by, now)) {
        Some(0) | None => format!("{contacts}…"),
        Some(1) => format!("{contacts}… about 1 second left"),
        Some(seconds) => format!("{contacts}… about {seconds} seconds left"),
    }
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

/// The finish page's invitation to a first Call, and the keybinding.
fn finish(
    keybinding: &KeybindingStage,
    line: &mut Option<String>,
) -> (Vec<String>, Vec<&'static str>) {
    let from_the_bar = "Place your first Call: choose Open Dialer from Desktop Calls in the bar, type a name or number, then press Enter.".to_owned();
    let check = match keybinding {
        KeybindingStage::Checking => return (vec![from_the_bar], vec!["Done"]),
        KeybindingStage::Confirming(KeybindingCheck::Available {
            file,
            line: shown,
            before_return,
            ..
        }) => {
            *line = Some(shown.clone());
            let place = if *before_return {
                format!("This line goes into {file}, just before its final return:")
            } else {
                format!("This line goes at the end of {file}:")
            };
            return (
                vec![
                    place,
                    "Choose Confirm to add it, or Cancel to leave the file as it is.".to_owned(),
                ],
                vec!["Confirm", "Cancel"],
            );
        }
        KeybindingStage::Checked(check) | KeybindingStage::Confirming(check) => check,
    };
    match check {
        KeybindingCheck::Available { keys, file, .. } => (
            vec![
                format!(
                    "Add a keybinding, and {} opens the Dialer from anywhere: {file} doesn't use those keys yet.",
                    shown_keys(keys)
                ),
                from_the_bar,
            ],
            vec!["Add keybinding", "Done"],
        ),
        KeybindingCheck::Added { keys, .. } => (
            vec![format!(
                "Place your first Call: press {} to open the Dialer, type a name or number, then press Enter.",
                shown_keys(keys)
            )],
            vec!["Done"],
        ),
        KeybindingCheck::Taken {
            keys,
            file,
            command,
        } => (
            vec![
                format!(
                    "{} already does something else in {file}; to open the Dialer from a key, bind another one to run {command}",
                    shown_keys(keys)
                ),
                from_the_bar,
            ],
            vec!["Done"],
        ),
        KeybindingCheck::Unreadable { file, command } => (
            vec![
                format!(
                    "{file} couldn't be read, so no keybinding can be added to it; to open the Dialer from a key, bind one to run {command}"
                ),
                from_the_bar,
            ],
            vec!["Done"],
        ),
        KeybindingCheck::NoConfig { command } => (
            vec![
                format!("To open the Dialer from a key, bind one to run {command}"),
                from_the_bar,
            ],
            vec!["Done"],
        ),
    }
}

/// Keys as Hyprland names them, `SUPER + ALT + C`, as people read them:
/// `Super + Alt + C`.
fn shown_keys(keys: &str) -> String {
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

/// Why the last attempt didn't work, and what to do about it.
fn notice(notice: SetupNotice) -> &'static str {
    match notice {
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
    }
}
