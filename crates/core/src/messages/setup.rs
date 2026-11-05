//! What the Setup Window says at each step: a heading, the steps to take
//! on the Phone, and its buttons.

use crate::machine::{SetupNotice, SetupStep, SetupWindowView};

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
    /// Why the last attempt didn't work.
    pub notice: Option<&'static str>,
    /// What the list says while no Phone is nearby, while choosing.
    pub empty_list: Option<&'static str>,
    /// The buttons, the default one first.
    pub buttons: Vec<&'static str>,
    /// The keys that work, shown at the foot.
    pub keys: String,
}

/// What the Setup Window says for `view`.
#[must_use]
pub fn setup_window(view: &SetupWindowView) -> SetupWindowText {
    let desktop = view
        .desktop_name
        .as_deref()
        .map_or_else(|| "this computer".to_owned(), |name| format!("“{name}”"));
    let mut code = None;
    let mut empty_list = None;
    let mut summary = None;
    let (heading, instructions, buttons): (String, Vec<String>, Vec<&'static str>) =
        match &view.step {
            SetupStep::Choose { .. } => {
                empty_list = Some("Looking for phones nearby…");
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
                code = Some(format!("{digits:06}"));
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
            SetupStep::PhoneCallsOff { phone } => {
                summary = Some(format!(
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
            SetupStep::Done { phone } => {
                summary = Some(
                    "Calls to your phone ring here now, and you can Call from here.".to_owned(),
                );
                (format!("{} is connected", phone.name), Vec::new(), vec!["Done"])
            }
        };
    let keys = match (&view.step, buttons.as_slice()) {
        (SetupStep::Choose { .. }, _) => {
            "Up and Down choose a phone, Enter pairs it; Escape closes".to_owned()
        }
        (_, [only]) => format!("Enter: {only}; Escape closes"),
        (_, buttons) => format!(
            "Enter: {}; Tab moves between buttons; Escape closes",
            buttons.first().copied().unwrap_or_default()
        ),
    };
    SetupWindowText {
        title: "Connect your phone",
        heading,
        summary,
        instructions,
        code,
        notice: view.notice.map(notice),
        empty_list,
        buttons,
        keys,
    }
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
    }
}
