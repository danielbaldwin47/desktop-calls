//! Missed Calls: deciding whether a Call that rang out was missed, calling
//! back, and the lasting cue that counts them until the Dialer opens.

use super::State;
use super::dialling::{dial, open_dialer};
use super::input::{Effect, Input};
use super::phone::contacts_sync;
use crate::call::{CallId, Caller, Timestamp};
use crate::contacts::{CallHistory, CallHistoryKind};
use crate::matching::{DialerRow, DialerRowKind};
use crate::messages::Failure;
use crate::missed::{MissedCallNotice, Notified};

/// The Call History refresh that decides whether a Call that rang out was
/// missed, and what the user does with a Missed Call notification.
pub(super) fn missed_call_input(state: &mut State, input: Input, effects: &mut Vec<Effect>) {
    match input {
        Input::CallHistoryRefreshed(call_history) => {
            let refreshed = call_history.clone();
            contacts_sync(state, Input::CallHistoryRefreshed(call_history), effects);
            refresh_answered(state, Some(&refreshed), effects);
        }
        Input::CallHistoryRefreshFailed => refresh_answered(state, None, effects),
        Input::OpenMissedCall(notification) => {
            if let Some(notified) = state.missed.take(notification) {
                open_dialer(state, String::new(), Some(notified.call));
            }
        }
        Input::CallBack(notification) => {
            if let Some(notified) = state.missed.take(notification) {
                state.missed.unseen = 0;
                call_back(state, &notified, effects);
            }
        }
        Input::MissedCallDismissed(notification) => {
            state.missed.take(notification);
        }
        Input::MissedCallsKept(count) => {
            state.missed.unseen = state.missed.unseen.max(count);
        }
        _ => {}
    }
}

impl State {
    /// How many Missed Calls the lasting cue counts: those since the user
    /// last opened the Dialer or called back.
    #[must_use]
    pub fn unseen_missed_calls(&self) -> u32 {
        self.missed.unseen
    }
}

/// Keeps the Missed Call cue for the next run whenever this step changed
/// it from `before`.
pub(super) fn keep_cue(before: u32, state: &State, effects: &mut Vec<Effect>) {
    if state.missed.unseen != before {
        effects.push(Effect::KeepMissedCalls(state.missed.unseen));
    }
}

/// The Dialer opens: it marks the Missed Calls the cue counted, which the
/// user now sees, so the cue clears.
pub(super) fn seen_in_dialer(state: &mut State) {
    state.missed.marked = std::mem::take(&mut state.missed.unseen);
}

/// With nothing typed, the Dialer lists the Call History newest first: the
/// newest `marked` Missed Calls in it go to the top, in their order, the
/// rest after them in theirs.
pub(super) fn missed_first(rows: Vec<DialerRow>, marked: u32) -> Vec<DialerRow> {
    let mut left = marked;
    let (mut first, rest): (Vec<DialerRow>, Vec<DialerRow>) = rows.into_iter().partition(|row| {
        let missed = left > 0 && row.kind == DialerRowKind::CallHistory(CallHistoryKind::Missed);
        if missed {
            left -= 1;
        }
        missed
    });
    first.extend(rest);
    first
}

/// A Call History refresh finished with `refreshed`, or failed. A Call
/// waiting on it is missed if the refresh lists a Missed Call from its
/// caller that wasn't there before, or if there is no refresh to go by.
/// Otherwise it was declined on the Phone.
fn refresh_answered(state: &mut State, refreshed: Option<&CallHistory>, effects: &mut Vec<Effect>) {
    for unconfirmed in state.missed.answered() {
        let listed = refreshed.is_none_or(|refreshed| {
            refreshed.entries().iter().any(|entry| {
                entry.kind == CallHistoryKind::Missed
                    && state.same_caller(&entry.caller, &unconfirmed.caller)
                    && !unconfirmed.before.entries().contains(entry)
            })
        });
        if listed {
            missed(
                state,
                unconfirmed.call,
                unconfirmed.caller,
                unconfirmed.at,
                effects,
            );
        }
    }
}

/// `call` from `caller` is a Missed Call: notify, adding it to the
/// notification already shown for the caller.
pub(super) fn missed(
    state: &mut State,
    call: CallId,
    caller: Caller,
    at: Timestamp,
    effects: &mut Vec<Effect>,
) {
    let shown = state
        .missed
        .notified
        .iter()
        .position(|notified| state.same_caller(&notified.caller, &caller));
    let notified = if let Some(index) = shown {
        let notified = &mut state.missed.notified[index];
        notified.call = call;
        notified.count = notified.count.saturating_add(1);
        notified.at = at;
        notified.clone()
    } else {
        let notified = Notified {
            notification: call,
            call,
            caller,
            count: 1,
            at,
        };
        state.missed.notified.push(notified.clone());
        notified
    };
    state.missed.unseen = state.missed.unseen.saturating_add(1);
    effects.push(Effect::NotifyMissedCall(MissedCallNotice {
        notification: notified.notification,
        call: notified.call,
        contact: state.contact(&notified.caller),
        caller: notified.caller,
        count: notified.count,
        at: notified.at,
    }));
}

/// Calls back the caller of a Missed Call notification.
pub(super) fn call_back(state: &mut State, notified: &Notified, effects: &mut Vec<Effect>) {
    match &notified.caller {
        Caller::Number(number) => dial(state, number, effects),
        Caller::Withheld => effects.push(Effect::ReportError(Failure::WithheldNumber)),
    }
}
