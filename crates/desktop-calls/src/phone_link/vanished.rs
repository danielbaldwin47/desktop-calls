//! Vanished Calls: what the Phone link keeps of a Call that went away
//! without ending, how it tells whether a Call the Phone lists is that one
//! come back, and the grace periods that bound the wait (ADR-0007).

use std::time::{Duration, Instant};

use desktop_calls_core::{CallId, CallState, Caller, Timestamp};

use super::{Outcome, PhoneLink, caller};

/// One grace period of a vanished Call. Only the latest one of each Call
/// counts: the one started when its whole service went replaces the shorter
/// one before it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Grace {
    pub(super) call: CallId,
    pub(super) number: u32,
}

/// A Call that went away without ending, during its grace period.
pub(super) struct Vanished {
    pub(super) id: CallId,
    pub(super) grace: Grace,
    /// When its latest grace runs out.
    pub(super) grace_ends: Instant,
    pub(super) number: String,
    /// Its state when it went, if known.
    pub(super) state: Option<CallState>,
    /// The Desktop asked to hang up meanwhile: the Call is hung up as soon
    /// as it is back.
    pub(super) hang_up: bool,
    /// A Call came back that may be this one, with no number to tell: it
    /// isn't hung up on a guess, and this Call may still be live.
    pub(super) doubt: bool,
    /// When it went: the Call ended then, if it never comes back.
    pub(super) went: Timestamp,
}

/// Whether a Call the Phone lists is a vanished Call come back.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kinship {
    Same,
    /// It may be, with no number to tell.
    Unsure,
    Other,
}

impl Vanished {
    /// Whether a Call listed as `state`, from `number`, is this Call come
    /// back. A Call never goes back to ringing, nor to dialling, so a
    /// ringing Call is never a vanished connected or outgoing Call. With a
    /// Hang up to send again, only the same number makes it the same Call:
    /// a withheld number, or none, may be another Call.
    pub(super) fn kinship(&self, state: CallState, number: &str) -> Kinship {
        let ringing = |state| matches!(state, CallState::Ringing | CallState::Waiting);
        let dialling = |state| matches!(state, CallState::Dialing | CallState::Alerting);
        let earlier = match self.state {
            Some(was) => (ringing(state) && !ringing(was)) || (dialling(state) && !dialling(was)),
            None => ringing(state) || dialling(state),
        };
        if earlier {
            return Kinship::Other;
        }
        let known = |number: &str| matches!(caller(number), Caller::Number(_));
        match (known(&self.number), known(number)) {
            (true, true) if self.number == number => Kinship::Same,
            (true, true) => Kinship::Other,
            _ if self.hang_up => Kinship::Unsure,
            // Nothing is sent on the strength of it: the Call only keeps its
            // timer (ADR-0007).
            _ if self.number == number => Kinship::Same,
            _ => Kinship::Other,
        }
    }
}

impl PhoneLink {
    /// Starts a grace period of `lasting` for the vanished Call `call`.
    pub(super) fn start_grace(
        &mut self,
        call: CallId,
        lasting: Duration,
        outcome: &mut Outcome,
    ) -> Grace {
        self.graces_started += 1;
        let grace = Grace {
            call,
            number: self.graces_started,
        };
        outcome.graces.push((grace, lasting));
        grace
    }

    /// Gives every vanished Call a grace of `lasting` from now, in place of
    /// the one it had.
    pub(super) fn restart_graces(&mut self, lasting: Duration, outcome: &mut Outcome) {
        self.restart_graces_ending_before(None, lasting, outcome);
    }

    /// Gives every vanished Call whose grace would run out sooner a grace
    /// of `lasting` from now. None is shortened.
    pub(super) fn lengthen_graces(&mut self, lasting: Duration, outcome: &mut Outcome) {
        self.restart_graces_ending_before(Some(Instant::now() + lasting), lasting, outcome);
    }

    /// Gives a grace of `lasting` from now to every vanished Call whose
    /// grace ends before `before`, or to all of them.
    fn restart_graces_ending_before(
        &mut self,
        before: Option<Instant>,
        lasting: Duration,
        outcome: &mut Outcome,
    ) {
        let ids: Vec<CallId> = self
            .vanished
            .iter()
            .filter(|call| before.is_none_or(|before| call.grace_ends < before))
            .map(|call| call.id)
            .collect();
        for id in ids {
            let grace = self.start_grace(id, lasting, outcome);
            if let Some(vanished) = self.vanished.iter_mut().find(|call| call.id == id) {
                vanished.grace = grace;
                vanished.grace_ends = Instant::now() + lasting;
            }
        }
    }
}
