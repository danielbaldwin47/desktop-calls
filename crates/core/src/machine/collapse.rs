//! When the Call Window collapses to the Call Pill, and when it expands
//! again.
//!
//! A few seconds after a Call connects, the Call Window collapses by itself,
//! unless the user is using it: while it has the keyboard, the pointer is
//! over it or the keypad is open, or while it shows a notice, lost contact or
//! a Call on hold, or a Call that hasn't connected. It collapses
//! [`COLLAPSE_MS`] after the last of those clears. Escape collapses it at
//! once; a click on the Call Pill, the keybinding or a click on the Indicator
//! expands it with the keyboard.

use super::dialling::open_dialer;
use super::input::{Effect, Input, Timer};
use super::view::{CallWindowPhase, CallWindowView};
use super::{Focus, State};

/// How long after a Call connects, or after the user stops using the Call
/// Window, it collapses by itself.
const COLLAPSE_MS: u32 = 5_000;

/// Which form the Call Window takes, and what the shell has reported of the
/// user using it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is an independent fact the shell reports, not a mode"
)]
pub(super) struct CallWindowForm {
    /// The Call Window shows as the Call Pill.
    collapsed: bool,
    /// The pointer is over the Call Window.
    pointer_over: bool,
    /// The Call Window has the keyboard, however it got it.
    keyboard: bool,
    /// The Call Window's keypad is open.
    keypad_open: bool,
    /// The collapse timer that counts, by number, while one runs.
    pending: Option<u32>,
    /// How many collapse timers have started.
    timers: u32,
}

impl CallWindowForm {
    pub(super) fn collapsed(&self) -> bool {
        self.collapsed
    }
}

/// Handles the inputs about the Call Window's form: `input` is one of
/// [`Input::CollapseCallWindow`], [`Input::ExpandCallWindow`],
/// [`Input::IndicatorActivated`], [`Input::PointerOverCallWindow`],
/// [`Input::CallWindowKeyboard`] and [`Input::KeypadShown`].
pub(super) fn form_input(state: &mut State, input: &Input) {
    match input {
        Input::CollapseCallWindow => collapse_by_hand(state),
        Input::ExpandCallWindow => {
            if state.form.collapsed {
                expand_with_keyboard(state);
            }
        }
        Input::IndicatorActivated => {
            if state.form.collapsed && state.surfaces().call_window.is_some() {
                expand_with_keyboard(state);
            } else {
                open_dialer(state, String::new(), None);
            }
        }
        Input::PointerOverCallWindow(over) => {
            state.form.pointer_over = *over && !state.form.collapsed;
        }
        Input::CallWindowKeyboard(keyboard) => {
            state.form.keyboard = *keyboard && !state.form.collapsed;
        }
        Input::KeypadShown(open) => state.form.keypad_open = *open,
        _ => {}
    }
}

/// Escape on the Call Window: it collapses and gives the keyboard back, once
/// its Call has connected. Before that, it only gives the keyboard back, so
/// the user can still cancel a Call being placed.
fn collapse_by_hand(state: &mut State) {
    state.focus = Focus::None;
    if state
        .surfaces()
        .call_window
        .is_some_and(|window| collapsible(&window))
    {
        collapse(state);
    }
}

/// Whether the Call Window shows a Call that has connected, so the Call
/// Pill has a Call to show.
fn collapsible(window: &CallWindowView) -> bool {
    window.connection_lost
        || matches!(
            window.phase,
            CallWindowPhase::Live { .. } | CallWindowPhase::OnHold { .. }
        )
}

fn collapse(state: &mut State) {
    state.focus = Focus::None;
    state.form.collapsed = true;
    state.form.pointer_over = false;
    state.form.keyboard = false;
    state.form.pending = None;
}

/// The Call Window expands, in its usual place, with the keyboard, as the
/// keybinding gives it. Only one surface can have the keyboard, so the Dialer
/// closes.
pub(super) fn expand_with_keyboard(state: &mut State) {
    state.form.collapsed = false;
    state.focus = Focus::CallWindow;
    state.dialer = None;
}

/// The collapse timer `timer` ran out: the Call Window collapses, if that
/// timer still counts.
pub(super) fn timer_ran_out(state: &mut State, timer: u32) {
    if state.form.pending == Some(timer) {
        collapse(state);
    }
}

/// After each step: forgets the form once the Call Window closes, and keeps
/// the collapse timer running exactly while the Call Window is expanded and
/// nothing keeps it so. `window` is the Call Window as the step left it. A
/// keystroke or a click in the window comes with the keyboard or the pointer
/// on it, which keep it expanded, so the time starts again once they leave.
pub(super) fn follow_call_window(
    state: &mut State,
    window: Option<&CallWindowView>,
    effects: &mut Vec<Effect>,
) {
    let Some(window) = window else {
        state.form = CallWindowForm {
            timers: state.form.timers,
            ..CallWindowForm::default()
        };
        return;
    };
    if state.form.collapsed || in_use(state, window) {
        state.form.pending = None;
        return;
    }
    if state.form.pending.is_none() {
        state.form.timers += 1;
        let timer = state.form.timers;
        state.form.pending = Some(timer);
        effects.push(Effect::StartTimer {
            timer: Timer::CollapseCallWindow(timer),
            after_ms: COLLAPSE_MS,
        });
    }
}

/// Whether something keeps the expanded Call Window from collapsing by
/// itself.
fn in_use(state: &State, window: &CallWindowView) -> bool {
    let form = &state.form;
    state.focus == Focus::CallWindow
        || form.keyboard
        || form.pointer_over
        || form.keypad_open
        || window.notice.is_some()
        || window.connection_lost
        || window.held.is_some()
        || !matches!(window.phase, CallWindowPhase::Live { .. })
}
