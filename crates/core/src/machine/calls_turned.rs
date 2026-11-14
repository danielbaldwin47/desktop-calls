//! The confirmation that the user turned calls off or on by hand (ticket
//! #231): the Indicator may be hidden in the tray drawer, so one short
//! notice says it happened. It is raised for the user's own `TurnCallsOff`,
//! `TurnCallsOn` and `KeepCallsOn`, whichever surface sent them (the
//! Indicator, the command line, the control interface, the Setup Window),
//! and never for Quiet Hours beginning or ending, a deadline passing, a
//! waiting turn-off applying after the Call, or the exception for a Call
//! placed from the Dialer: those don't go through here.

use super::State;
use super::calls_off::CallsOff;
use super::input::Effect;
use crate::clock::Deadline;
use crate::connection::ConnectionState;

/// How far from an hour on a turn-off's end may be and still be "for 1
/// hour", in milliseconds.
const FOR_AN_HOUR_TOLERANCE_MS: u64 = 90_000;

/// What the user turned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallsTurned {
    /// Calls are off until `until`, or until the user turns them on.
    /// `for_an_hour` when `until` is the Indicator's "For 1 hour".
    Off {
        until: Option<Deadline>,
        for_an_hour: bool,
    },
    /// Calls go off once the last Call ends, until `until`, or until the
    /// user turns them on.
    OffAfterCall { until: Option<Deadline> },
    /// Calls are on; the Phone is `reconnecting`, or never left.
    On { reconnecting: bool },
}

/// What the confirmation tells: what was turned, and the name of the Phone
/// it is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallsTurnedNotice {
    pub phone: String,
    pub turned: CallsTurned,
}

impl State {
    /// What the user's turn-off turned, if it changed the state `before`
    /// it. `None` if nothing changed: a refusal, or the choice already
    /// made.
    pub(super) fn calls_turned_off(&self, before: &CallsOff) -> Option<CallsTurned> {
        if &self.calls_off == before {
            return None;
        }
        match &self.calls_off {
            CallsOff::Off { until, .. } => Some(CallsTurned::Off {
                until: *until,
                for_an_hour: self.is_for_an_hour(*until),
            }),
            CallsOff::Pending { until, .. } => Some(CallsTurned::OffAfterCall { until: *until }),
            _ => None,
        }
    }

    /// Whether `until` is the Indicator's "For 1 hour", or the command
    /// line's `--for 1h`: an hour on from the wall clock as last read. The
    /// clock is read on the minute and the command line rounds down to the
    /// second, so a minute and a half either way is the same choice.
    fn is_for_an_hour(&self, until: Option<Deadline>) -> bool {
        let (Some(until), Some(clock)) = (until, self.clock) else {
            return false;
        };
        until.at.0.abs_diff(clock.for_an_hour.at.0) <= FOR_AN_HOUR_TOLERANCE_MS
    }

    /// What the user's turn-on turns if asked now: something only while
    /// calls are off or a turn-off waits. The Phone reconnects unless it is
    /// connected already.
    pub(super) fn calls_turned_on(&self) -> Option<CallsTurned> {
        (self.calls_off() || self.calls_off_pending()).then(|| self.on())
    }

    /// What the user's Keep calls on turns if asked now: something only
    /// while a turn-off waits. While calls are off it does nothing.
    pub(super) fn calls_kept_on(&self) -> Option<CallsTurned> {
        self.calls_off_pending().then(|| self.on())
    }

    fn on(&self) -> CallsTurned {
        CallsTurned::On {
            reconnecting: *self.connection_state() != ConnectionState::Connected,
        }
    }
}

/// Raises the confirmation that `turned` happened, for the Phone, if
/// there is one.
pub(super) fn confirm(state: &State, turned: Option<CallsTurned>, effects: &mut Vec<Effect>) {
    let (Some(turned), Some(phone)) = (turned, &state.phone) else {
        return;
    };
    effects.push(Effect::NotifyCallsTurned(CallsTurnedNotice {
        phone: phone.name.clone(),
        turned,
    }));
}
