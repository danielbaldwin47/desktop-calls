//! The Indicator's states in the gallery, drawn as Omarchy's bar draws
//! its icon, light and dark.

use std::path::PathBuf;

use test_harness::{Harness, OmarchyTheme, draw_tray_icon};

use super::{DEADLINE, Look, NUMBER, connect};

/// One state of the Indicator the gallery shows.
pub(super) struct IndicatorState {
    /// The file name stem, after `indicator-`.
    pub(super) name: &'static str,
    /// The caption.
    pub(super) title: &'static str,
    /// Brings the scenario into the state.
    reach: fn(&Harness),
    /// What the Indicator's tooltip says once it shows the state.
    tooltip: &'static str,
}

/// The Indicator is drawn in Omarchy's light and dark themes.
pub(super) const INDICATOR_THEMES: [OmarchyTheme; 2] = [OmarchyTheme::Light, OmarchyTheme::Dark];

pub(super) const INDICATOR_STATES: &[IndicatorState] = &[
    IndicatorState {
        name: "connected",
        title: "The Indicator, the Phone connected",
        reach: |harness| harness.simulated_phone().connect(),
        tooltip: "Simulated Pixel connected",
    },
    IndicatorState {
        name: "disconnected",
        title: "The Indicator, the Phone out of range",
        reach: |harness| {
            connect(harness);
            harness.simulated_phone().disconnect();
        },
        tooltip: "Simulated Pixel out of range",
    },
    IndicatorState {
        name: "ringing",
        title: "The Indicator, a Call ringing",
        reach: |harness| {
            connect(harness);
            let _path = harness.simulated_phone().ring(NUMBER);
        },
        tooltip: "Call from +44 7700 900123",
    },
    IndicatorState {
        name: "in-call",
        title: "The Indicator, in a Call",
        reach: |harness| {
            connect(harness);
            let _path = harness.simulated_phone().ring(NUMBER);
            let state = harness.wait_for_app("the Call rings", DEADLINE, |state| state.ringing);
            harness.answer(state.calls.first().map_or(0, |call| call.id));
        },
        tooltip: "In a Call with +44 7700 900123",
    },
    IndicatorState {
        name: "problem",
        title: "The Indicator, a Problem: Contacts not shared",
        reach: |harness| {
            harness.simulated_phone().set_contacts_prompt("deny");
            harness.simulated_phone().connect();
        },
        tooltip: "Simulated Pixel connected · Contacts not shared",
    },
];

/// Where the Indicator in `state` under `theme` is drawn.
pub(super) fn indicator_path(
    state: &IndicatorState,
    theme: OmarchyTheme,
    output: &std::path::Path,
) -> PathBuf {
    output.join(format!("indicator-{}-{}.png", state.name, theme.as_str()))
}

/// Brings a scenario under `theme` into `state` and draws the Indicator
/// as Omarchy's bar shows it, in the theme's background and foreground,
/// at its `indicator_path`.
pub(super) fn draw_indicator(
    state: &IndicatorState,
    theme: OmarchyTheme,
    output: &std::path::Path,
) -> Result<(), String> {
    let harness = Look::Omarchy(theme).builder().start();
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
    let item = items.first().ok_or("no tray item")?;
    draw_tray_icon(
        item,
        theme.colour("background"),
        theme.colour("foreground"),
        &path,
    )
}
