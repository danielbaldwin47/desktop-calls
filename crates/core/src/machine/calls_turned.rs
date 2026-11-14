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
                for_an_hour: until.is_some() && *until == self.clock.map(|clock| clock.for_an_hour),
            }),
            CallsOff::Pending { until, .. } => Some(CallsTurned::OffAfterCall { until: *until }),
            _ => None,
        }
    }

    /// What the user's turn-on, or Keep calls on, turns if asked now:
    /// something only while calls are off or a turn-off waits. The Phone
    /// reconnects unless it is connected already.
    pub(super) fn calls_turned_on(&self) -> Option<CallsTurned> {
        (self.calls_off() || self.calls_off_pending()).then(|| CallsTurned::On {
            reconnecting: *self.connection_state() != ConnectionState::Connected,
        })
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
