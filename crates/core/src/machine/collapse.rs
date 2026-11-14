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
//!
//! While collapsed, it expands by itself, without taking the keyboard, when
//! the Desktop loses contact with the Call, a notice appears, a Call goes on
//! hold or a new Call connects: each needs the user to see the Call Window.
//! A Waiting Call ringing doesn't expand it, as the Banner shows that, and
//! nor does a Problem.

use super::dialling::open_dialer;
use super::input::{Effect, Input, Timer};
use super::view::{CallWindowPhase, CallWindowView};
use super::{Focus, State};
use crate::CallId;

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
    /// The user has clicked in the Call Window or pressed a key in it since
    /// it last got the keyboard. A compositor may hand a newly shown window
    /// the keyboard by itself, as sway does with nothing else focused: that
    /// alone is no sign of the user using it.
    typed: bool,
    /// The Call Window's keypad is open.
    keypad_open: bool,
    /// The collapse timer that counts, by number, while one runs.
    pending: Option<u32>,
    /// How many collapse timers have started.
    timers: u32,
    /// What the Call Window showed after the last step, of what expands it
    /// by itself once it arises; `None` before it shows.
    shown: Option<Shown>,
}

/// What the Call Window shows that expands it by itself when it arises
/// while it is collapsed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Shown {
    /// The last Call the Call Window showed connected, live.
    live_call: Option<CallId>,
    /// The Desktop has lost contact with the Call.
    connection_lost: bool,
    /// A notice shows.
    notice: bool,
    /// A Call is on hold.
    held: bool,
}

impl Shown {
    fn of(window: &CallWindowView, before: Option<Self>) -> Self {
        let live = matches!(window.phase, CallWindowPhase::Live { .. });
        Self {
            live_call: if live {
                window.call
            } else {
                before.and_then(|before| before.live_call)
            },
            connection_lost: window.connection_lost,
            notice: window.notice.is_some(),
            held: window.held.is_some() || matches!(window.phase, CallWindowPhase::OnHold { .. }),
        }
    }

    /// Whether `self`, shown after `before`, needs the user to see the Call
    /// Window: lost contact, a notice or a Call on hold arose, or another
    /// Call connected.
    fn needs_seeing_after(self, before: Self) -> bool {
        (self.connection_lost && !before.connection_lost)
            || (self.notice && !before.notice)
            || (self.held && !before.held)
            || (before.live_call.is_some() && self.live_call != before.live_call)
    }
}

impl CallWindowForm {
    pub(super) fn collapsed(&self) -> bool {
        self.collapsed
    }
}

/// Handles the inputs about the Call Window's form: `input` is one of
/// [`Input::CollapseCallWindow`], [`Input::ExpandCallWindow`],
/// [`Input::IndicatorActivated`], [`Input::PointerOverCallWindow`],
/// [`Input::CallWindowKeyboard`], [`Input::CallWindowUsed`] and
/// [`Input::KeypadShown`].
pub(super) fn form_input(state: &mut State, input: &Input) {
    match input {
        Input::CollapseCallWindow => collapse_by_hand(state),
        Input::ExpandCallWindow => {
            if state.form.collapsed {
                expand_with_keyboard(state);
            }
        }
        Input::IndicatorActivated => {
            // Once the Call has ended, the Call Pill only says so, and the
            // Indicator opens the Dialer as it does with no Call.
            if state.form.collapsed
                && state
                    .surfaces()
                    .call_window
                    .is_some_and(|w| collapsible(&w))
            {
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
            if !*keyboard {
                state.form.typed = false;
            }
        }
        Input::CallWindowUsed => {
            state.form.typed = !state.form.collapsed;
            // The time to collapse starts again once the user stops.
            state.form.pending = None;
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
    state.form.typed = false;
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
/// nothing keeps it so. `window` is the Call Window as the step left it.
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
    let before = state.form.shown;
    let shown = Shown::of(window, before);
    state.form.shown = Some(shown);
    if state.form.collapsed && before.is_some_and(|before| shown.needs_seeing_after(before)) {
        // Expanded by itself, it leaves the keyboard where it is.
        state.form.collapsed = false;
    }
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
        || (form.keyboard && form.typed)
        || form.pointer_over
        || form.keypad_open
        || window.notice.is_some()
        || window.connection_lost
        || window.held.is_some()
        || !matches!(window.phase, CallWindowPhase::Live { .. })
}
