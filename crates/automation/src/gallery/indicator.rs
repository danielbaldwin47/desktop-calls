//! The Indicator in the gallery, light and dark: its icon in each state,
//! drawn as Omarchy's bar draws it, and its menu in four states, drawn as
//! Omarchy's tray draws it from the layout the app publishes. Omarchy's
//! shell draws the real menu; the gallery runs none, so the menu is a
//! stand-in, and its caption says so.

use std::path::{Path, PathBuf};

use test_harness::{
    Builder, Harness, MenuColours, MenuEntry, OmarchyTheme, draw_tray_icon, draw_tray_menu,
};

use super::quiet_hours_window::at_noon;
use super::{DEADLINE, Look, NUMBER, connect};

/// The Indicator's title, which Omarchy's tray matches a menu's first
/// entry against.
const TITLE: &str = "Desktop Calls";

/// One state of the Indicator the gallery shows.
pub(super) struct IndicatorState {
    /// The file name stem, after `indicator-`.
    pub(super) name: &'static str,
    /// The caption.
    pub(super) title: &'static str,
    /// Sets the scenario up before it starts.
    prepare: fn(Builder) -> Builder,
    /// Brings the scenario into the state.
    reach: fn(&Harness),
    /// What the Indicator's tooltip says once it shows the state.
    tooltip: &'static str,
    /// For a menu state, the label of the entry whose submenu is open.
    submenu: Option<&'static str>,
}

/// The Indicator is drawn in Omarchy's light and dark themes.
pub(super) const INDICATOR_THEMES: [OmarchyTheme; 2] = [OmarchyTheme::Light, OmarchyTheme::Dark];

/// The scenario's own setup, for a state that needs none.
fn as_is(builder: Builder) -> Builder {
    builder
}

/// A live Call, answered here.
fn live(harness: &Harness) {
    connect(harness);
    let _path = harness.simulated_phone().ring(NUMBER);
    let state = harness.wait_for_app("the Call rings", DEADLINE, |state| state.ringing);
    harness.answer(state.calls.first().map_or(0, |call| call.id));
    harness.wait_for_app("the Call is live", DEADLINE, |state| {
        state
            .calls
            .first()
            .is_some_and(|call| call.state == "active")
    });
}

/// Contacts not shared: a Problem, with the Phone connected.
fn problem(harness: &Harness) {
    harness.simulated_phone().set_contacts_prompt("deny");
    harness.simulated_phone().connect();
}

/// Quiet Hours from 22:00 to 07:00, which the app at noon isn't in.
fn quiet_hours(harness: &Harness) {
    if let Err(error) = harness.set_quiet_hours(22 * 60, 7 * 60) {
        panic!("SetQuietHours: {error:?}");
    }
    // The control interface shows each state before the Indicator does, so
    // wait on the menu itself.
    harness
        .tray()
        .wait_for_menu("the menu shows the Quiet Hours", DEADLINE, |menu| {
            MenuEntry::find(menu, QUIET_HOURS_ENTRY).is_some()
        });
}

/// The Quiet Hours entry, with the times `quiet_hours` sets as its hint.
const QUIET_HOURS_ENTRY: &str = "Quiet Hours…  22:00–07:00";

/// The icon's states.
pub(super) const INDICATOR_STATES: &[IndicatorState] = &[
    IndicatorState {
        name: "connected",
        title: "The Indicator, the Phone connected",
        prepare: as_is,
        reach: |harness| harness.simulated_phone().connect(),
        tooltip: "Simulated Pixel connected",
        submenu: None,
    },
    IndicatorState {
        name: "disconnected",
        title: "The Indicator, the Phone out of range: a dashed outline",
        prepare: as_is,
        reach: |harness| {
            connect(harness);
            harness.simulated_phone().disconnect();
        },
        tooltip: "Simulated Pixel out of range",
        submenu: None,
    },
    IndicatorState {
        name: "ringing",
        title: "The Indicator, a Call ringing",
        prepare: as_is,
        reach: |harness| {
            connect(harness);
            let _path = harness.simulated_phone().ring(NUMBER);
        },
        tooltip: "Call from +44 7700 900123",
        submenu: None,
    },
    IndicatorState {
        name: "in-call",
        title: "The Indicator, in a Call",
        prepare: as_is,
        reach: live,
        tooltip: "In a Call with +44 7700 900123",
        submenu: None,
    },
    IndicatorState {
        name: "calls-off-pending",
        title: "The Indicator, in a Call, with calls turning off when it ends",
        prepare: as_is,
        reach: |harness| {
            live(harness);
            let _output = harness.cli(&["calls", "off"]);
        },
        tooltip: "Calls turn off when this Call ends",
        submenu: None,
    },
    IndicatorState {
        name: "problem",
        title: "The Indicator, a Problem (Contacts not shared): a filled badge",
        prepare: as_is,
        reach: problem,
        tooltip: "Simulated Pixel connected · Contacts not shared",
        submenu: None,
    },
    IndicatorState {
        name: "off",
        title: "The Indicator, calls off: a full diagonal",
        prepare: as_is,
        reach: |harness| {
            connect(harness);
            let _output = harness.cli(&["calls", "off"]);
        },
        tooltip: "Calls off",
        submenu: None,
    },
    IndicatorState {
        name: "off-until-morning",
        title: "The Indicator, calls off until tomorrow morning",
        prepare: as_is,
        reach: |harness| {
            connect(harness);
            let _output = harness.cli(&["calls", "off", "--until-morning"]);
        },
        tooltip: "Calls off until 07:00",
        submenu: None,
    },
    IndicatorState {
        name: "off-quiet-hours",
        title: "The Indicator, calls off for Quiet Hours",
        prepare: as_is,
        // Under way at any time but the day's last minute, in whichever
        // time zone the gallery runs.
        reach: |harness| {
            connect(harness);
            let _output = harness.cli(&["quiet-hours", "00:00-23:59"]);
        },
        tooltip: "Calls off until 23:59 (quiet hours)",
        submenu: None,
    },
];

/// The menu's states: the stand-in for the menu Omarchy's tray draws.
pub(super) const MENU_STATES: &[IndicatorState] = &[
    IndicatorState {
        name: "menu-calls-on",
        title: "The Indicator's menu, calls on, with Quiet Hours set",
        prepare: at_noon,
        reach: |harness| {
            connect(harness);
            quiet_hours(harness);
        },
        tooltip: "Simulated Pixel connected",
        submenu: None,
    },
    IndicatorState {
        name: "menu-turn-off-choices",
        title: "The Indicator's menu, Turn calls off opened: how long for",
        prepare: at_noon,
        reach: connect,
        tooltip: "Simulated Pixel connected",
        submenu: Some("Turn calls off"),
    },
    IndicatorState {
        name: "menu-off-problem",
        title: "The Indicator's menu, calls off, with a Problem (Contacts not shared)",
        prepare: at_noon,
        reach: |harness| {
            problem(harness);
            harness.wait_for_app("the Problem is reported", DEADLINE, |state| {
                !state.problems.is_empty()
            });
            let _output = harness.cli(&["calls", "off"]);
        },
        tooltip: "Calls off · Contacts not shared",
        submenu: None,
    },
    IndicatorState {
        name: "menu-calls-off-pending",
        title: "The Indicator's menu, in a Call, with calls turning off when it ends",
        prepare: at_noon,
        reach: |harness| {
            live(harness);
            let _output = harness.cli(&["calls", "off"]);
        },
        tooltip: "Calls turn off when this Call ends",
        submenu: None,
    },
];

/// Where the Indicator in `state` under `theme` is drawn.
pub(super) fn indicator_path(
    state: &IndicatorState,
    theme: OmarchyTheme,
    output: &Path,
) -> PathBuf {
    output.join(format!("indicator-{}-{}.png", state.name, theme.as_str()))
}

/// Brings a scenario under `theme` into `state` and draws, at its
/// `indicator_path`, the Indicator as Omarchy shows it: for a menu state
/// its menu, as the app publishes it, in the theme's colours; otherwise
/// its icon on a strip of bar.
pub(super) fn draw_indicator(
    state: &IndicatorState,
    theme: OmarchyTheme,
    menu: bool,
    output: &Path,
) -> Result<(), String> {
    let harness = (state.prepare)(Look::Omarchy(theme).builder()).start();
    (state.reach)(&harness);
    let items = harness.tray().wait_for(
        &format!("the Indicator says {:?}", state.tooltip),
        DEADLINE,
        |items| {
            items
                .iter()
                .any(|item| item.tooltip.as_deref() == Some(state.tooltip))
        },
    );
    let path = indicator_path(state, theme, output);
    if menu {
        let colours = MenuColours {
            background: theme.colour("background"),
            foreground: theme.colour("foreground"),
            separator: theme.colour("accent"),
        };
        draw_tray_menu(&harness.tray().menu(), TITLE, state.submenu, colours, &path)
    } else {
        let item = items.first().ok_or("no tray item")?;
        draw_tray_icon(
            item,
            theme.colour("background"),
            theme.colour("foreground"),
            &path,
        )
    }
}
