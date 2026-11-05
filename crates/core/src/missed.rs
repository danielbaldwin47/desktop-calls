//! What the Call state machine keeps about Missed Calls: the Calls that rang
//! out here and wait for the Phone's Call History to say whether they were
//! missed, and the notifications shown, one per caller.

use crate::call::{CallId, Caller, Timestamp};
use crate::contacts::CallHistory;
use crate::matching::ContactMatch;

/// What a Missed Call notification tells. One notification stands for each
/// caller: a later Missed Call from the same caller replaces it, counting
/// them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissedCallNotice {
    /// Which notification: the first Missed Call it told of. The
    /// notification's actions name it.
    pub notification: CallId,
    /// The latest Missed Call from this caller.
    pub call: CallId,
    pub caller: Caller,
    /// The Contact the caller is, if Contacts say.
    pub contact: Option<ContactMatch>,
    /// How many Missed Calls from this caller the notification tells of.
    pub count: u32,
    /// When the latest one started ringing.
    pub at: Timestamp,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct MissedCalls {
    /// Calls that rang out here, oldest first, until the Call History
    /// refresh after each says whether the Phone lists it as missed.
    pub(crate) unconfirmed: Vec<Unconfirmed>,
    /// How many Call History refreshes have been asked for.
    pub(crate) refreshes_asked: u32,
    /// How many of them have finished, or failed. Refreshes finish in the
    /// order asked.
    pub(crate) refreshes_answered: u32,
    /// The notifications shown, one per caller, until the user acts on or
    /// dismisses one.
    pub(crate) notified: Vec<Notified>,
}

/// A Call that rang out here, before the Phone's Call History confirms it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Unconfirmed {
    pub(crate) call: CallId,
    pub(crate) caller: Caller,
    pub(crate) at: Timestamp,
    /// The Call History as it was when the Call ended: a Missed Call the
    /// refresh lists that isn't in it is this Call.
    pub(crate) before: CallHistory,
    /// The refresh asked for when the Call ended.
    pub(crate) refresh: u32,
}

/// A notification shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Notified {
    pub(crate) notification: CallId,
    pub(crate) call: CallId,
    pub(crate) caller: Caller,
    pub(crate) count: u32,
    pub(crate) at: Timestamp,
}

impl MissedCalls {
    /// The notification with this id, taken out: the user has acted on it.
    pub(crate) fn take(&mut self, notification: CallId) -> Option<Notified> {
        let index = self
            .notified
            .iter()
            .position(|notified| notified.notification == notification)?;
        Some(self.notified.remove(index))
    }

    /// One more refresh finished: the Calls waiting on it, and on any
    /// before it.
    pub(crate) fn answered(&mut self) -> Vec<Unconfirmed> {
        self.refreshes_answered = self.refreshes_answered.saturating_add(1);
        let answered = self.refreshes_answered;
        let (due, waiting) = std::mem::take(&mut self.unconfirmed)
            .into_iter()
            .partition(|unconfirmed| unconfirmed.refresh <= answered);
        self.unconfirmed = waiting;
        due
    }
}
