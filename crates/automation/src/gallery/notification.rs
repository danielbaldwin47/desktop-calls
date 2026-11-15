//! The Missed Call notification in the gallery, light and dark: the
//! notification the app actually sends, drawn as Omarchy's notification
//! card draws it. Omarchy's shell draws the real card; the gallery runs
//! none, so this is a stand-in, and its caption says so.

use std::path::{Path, PathBuf};

use test_harness::vcards::{self, CallKind};
use test_harness::{CardColours, Harness, OmarchyTheme, draw_notification_card};

use super::indicator::INDICATOR_THEMES;
use super::quiet_hours_window::at_noon;
use super::{DEADLINE, Look, NUMBER, connect};

/// One state of the Missed Call notification the gallery shows.
pub(super) struct NotificationState {
    /// The file name stem, after `notification-`.
    pub(super) name: &'static str,
    /// The caption.
    pub(super) title: &'static str,
    /// How many Missed Calls from Alice the notification tells of.
    missed: u32,
}

/// The notification's states.
pub(super) const NOTIFICATION_STATES: &[NotificationState] = &[
    NotificationState {
        name: "missed-call",
        title: "The Missed Call notification: a missed-call glyph, who in the summary, \
                the number's label and the time in the body",
        missed: 1,
    },
    NotificationState {
        name: "missed-calls-grouped",
        title: "The Missed Call notification, three from one caller: one notification counting them",
        missed: 3,
    },
];

/// Where the notification in `state` under `theme` is drawn.
fn notification_path(state: &NotificationState, theme: OmarchyTheme, output: &Path) -> PathBuf {
    output.join(format!(
        "notification-{}-{}.png",
        state.name,
        theme.as_str()
    ))
}

/// Draws every state, in Omarchy's light and dark themes.
pub(super) fn draw_notifications(output: &Path) -> Result<(), String> {
    for state in NOTIFICATION_STATES {
        for theme in INDICATOR_THEMES {
            draw_notification(state, theme, output)
                .map_err(|error| format!("the notification {}: {error}", state.name))?;
        }
    }
    Ok(())
}

/// The states drawn into `output`, now or by an earlier run.
pub(super) fn drawn_notifications(
    output: &Path,
) -> Vec<(&'static NotificationState, OmarchyTheme, PathBuf)> {
    NOTIFICATION_STATES
        .iter()
        .flat_map(|state| INDICATOR_THEMES.map(|theme| (state, theme)))
        .map(|(state, theme)| (state, theme, notification_path(state, theme, output)))
        .filter(|(_, _, path)| path.exists())
        .collect()
}

/// Alice Smith in Contacts, her number's label mobile, and an older Call in
/// the Call History, synced.
fn synced(harness: &Harness) {
    let phone = harness.simulated_phone();
    let mut contacts = vcards::contact("Alice Smith", &[("CELL", NUMBER)]);
    contacts.push_str(&vcards::contacts(5, 1_000));
    phone.set_pbap_folder("pb", &contacts);
    phone.set_pbap_folder("cch", &older_call());
    connect(harness);
    harness.wait_for_app("Contacts and Call History are synced", DEADLINE, |state| {
        state.contacts.state == "ready" && state.contacts.call_history == 1
    });
}

fn older_call() -> String {
    vcards::call(CallKind::Dialed, Some("+447700900001"), "20261104T090000")
}

/// Brings a scenario under `theme` to the notification `state` shows, and
/// draws it at its `notification_path`, as Omarchy's card shows it.
fn draw_notification(
    state: &NotificationState,
    theme: OmarchyTheme,
    output: &Path,
) -> Result<(), String> {
    let harness = at_noon(Look::Omarchy(theme).builder()).start();
    synced(&harness);
    let mut history = Vec::new();
    let mut notification = None;
    for count in 1..=state.missed {
        let path = harness.simulated_phone().ring(NUMBER);
        harness.wait_for_app("the Call rings", DEADLINE, |state| state.ringing);
        // The Phone lists the Missed Call on top of its Call History.
        history.insert(
            0,
            vcards::call(
                CallKind::Missed,
                Some(NUMBER),
                &format!("20261105T10{count:02}00"),
            ),
        );
        let mut all = history.concat();
        all.push_str(&older_call());
        harness.simulated_phone().set_pbap_folder("cch", &all);
        harness.simulated_phone().caller_hangs_up(&path);
        harness.wait_for_app("the Call is over", DEADLINE, |state| state.calls.is_empty());
        let summary = if count == 1 {
            "Missed call from Alice Smith".to_owned()
        } else {
            format!("{count} missed calls from Alice Smith")
        };
        notification = Some(harness.notifications().wait_for(
            &format!("the notification {summary:?}"),
            DEADLINE,
            |notification| notification.summary == summary,
        ));
    }
    let notification = notification.ok_or("no Missed Call notification")?;
    let colours = CardColours {
        background: theme.colour("background"),
        text: theme.colour("foreground"),
        border: theme.colour("accent"),
    };
    draw_notification_card(
        &notification,
        colours,
        &notification_path(state, theme, output),
    )
}
