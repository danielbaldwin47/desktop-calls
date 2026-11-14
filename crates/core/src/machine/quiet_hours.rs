//! Quiet Hours (ticket #205): the daily times, set by the user, during
//! which calls are off.
//!
//! They are configuration, kept by the shell for the next run. The shell's
//! clock reports when they next begin and end with every reading of the
//! wall clock; while they are under way, they end before they next begin.
//! The core decides the rest, after every input:
//!
//! - Under way, with calls on, they turn calls off until they end, as a
//!   turn-off from the user would: waiting for the last Call to end.
//! - Calls off for them come back on when they end, at their deadline.
//! - Calls turned on, or kept on, during them stay on until they next
//!   begin, across a restart too.
//! - A choice by hand replaces them; "until I turn them on" outlasts them.
//! - Changed, calls off for them follow the new times; turned off, calls
//!   off for them come back on.

use super::State;
use super::calls_off::{CallsOff, drop_pending, turn_off, turn_on};
use super::input::{Effect, Input};
use crate::clock::{QuietHours, QuietHoursClock};

impl State {
    /// The Quiet Hours the user set, if any.
    #[must_use]
    pub fn quiet_hours(&self) -> Option<QuietHours> {
        self.quiet_hours
    }

    /// When the Quiet Hours set next begin and end, once the clock has
    /// read them: just after they change, it still has the old ones.
    pub(super) fn quiet_hours_clock(&self) -> Option<QuietHoursClock> {
        let quiet = self.clock?.quiet_hours?;
        (Some(quiet.hours) == self.quiet_hours).then_some(quiet)
    }

    /// When the Quiet Hours under way end and next begin, while they are.
    pub(super) fn quiet_hours_under_way(&self) -> Option<QuietHoursClock> {
        self.quiet_hours_clock().filter(QuietHoursClock::under_way)
    }
}

/// The user set Quiet Hours or turned them off, or an earlier run kept
/// them. The clock reads them once the shell has them.
pub(super) fn quiet_hours_input(state: &mut State, input: &Input, effects: &mut Vec<Effect>) {
    let hours = match input {
        &Input::QuietHoursKept(hours) => {
            state.quiet_hours = Some(hours);
            return;
        }
        &Input::SetQuietHours(hours) => Some(hours),
        Input::ClearQuietHours => None,
        _ => return,
    };
    if state.quiet_hours != hours {
        state.quiet_hours = hours;
        effects.push(Effect::KeepQuietHours(hours));
    }
}

/// After every input: calls follow the Quiet Hours, as the clock last read
/// them. Not while the Desktop suspends: the resume reads the clock again.
pub(super) fn apply(state: &mut State, effects: &mut Vec<Effect>) {
    if state.suspended() {
        return;
    }
    if state.quiet_hours.is_none() {
        gone(state, effects);
        return;
    }
    let (Some(quiet), Some(clock)) = (state.quiet_hours_clock(), state.clock) else {
        return;
    };
    let under_way = quiet.under_way();
    // Kept on through Quiet Hours that are over, or that began again since.
    if let Some(kept) = state.kept_on_until
        && (!under_way || kept.at <= clock.now)
    {
        state.kept_on_until = None;
        if state.calls_off == CallsOff::On {
            effects.push(Effect::KeepCallsOff(None));
        }
    }
    let moved = state
        .calls_off_until()
        .is_none_or(|until| until.at != quiet.next_end.at);
    match &state.calls_off {
        CallsOff::Off {
            quiet_hours: true, ..
        }
        | CallsOff::Reblocking {
            quiet_hours: true, ..
        } if !under_way => turn_on(state, effects),
        CallsOff::Pending {
            quiet_hours: true, ..
        } if !under_way => drop_pending(state, effects),
        CallsOff::Off {
            quiet_hours: true, ..
        }
        | CallsOff::Pending {
            quiet_hours: true, ..
        } if moved => turn_off(state, Some(quiet.next_end), true, effects),
        CallsOff::On if under_way && state.kept_on_until.is_none() && state.phone.is_some() => {
            turn_off(state, Some(quiet.next_end), true, effects);
        }
        _ => {}
    }
}

/// The user turned Quiet Hours off: calls off for them come back on, a
/// turn-off for them waiting for a Call is dropped, and nothing keeps
/// calls on through them any longer.
fn gone(state: &mut State, effects: &mut Vec<Effect>) {
    let kept_on = state.kept_on_until.take().is_some();
    if state.calls_off_for_quiet_hours() {
        if state.calls_off_pending() {
            drop_pending(state, effects);
        } else {
            turn_on(state, effects);
        }
    } else if kept_on && state.calls_off == CallsOff::On {
        effects.push(Effect::KeepCallsOff(None));
    }
}
