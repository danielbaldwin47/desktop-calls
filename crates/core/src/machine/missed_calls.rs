//! Missed Calls: deciding whether a Call that rang out was missed, calling
//! back, and the lasting cue that counts them until the Dialer opens.

use super::State;
use super::dialling::{dial, open_dialer, refresh_dialer};
use super::input::{Effect, Input};
use super::phone::contacts_sync;
use crate::call::{CallId, Caller, Timestamp};
use crate::contacts::{CallHistory, CallHistoryKind};
use crate::matching::{DialerRow, DialerRowKind};
use crate::messages::Failure;
use crate::missed::{Cue, MissedCallNotice, Notified};

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
                state.missed.unseen = Cue::default();
                call_back(state, &notified, effects);
            }
        }
        Input::MissedCallDismissed(notification) => {
            state.missed.take(notification);
        }
        Input::MissedCallsKept(count) => {
            let unseen = &mut state.missed.unseen;
            unseen.count = unseen.count.max(count);
        }
        _ => {}
    }
}

impl State {
    /// How many Missed Calls the lasting cue counts: those since the user
    /// last opened the Dialer or called back.
    #[must_use]
    pub fn unseen_missed_calls(&self) -> u32 {
        self.missed.unseen.count
    }

    /// Which of `rows`, a Call History newest first, the open Dialer marks
    /// as Missed Calls the user hasn't seen: for each caller it marks, the
    /// newest Missed Call from them; for each it can't name, kept by an
    /// earlier run, the newest Missed Call left.
    pub(super) fn marks(&self, rows: &[DialerRow]) -> Vec<bool> {
        let marked = &self.missed.marked;
        let mut callers = marked.callers.clone();
        let named = u32::try_from(callers.len()).unwrap_or(u32::MAX);
        let mut earlier = marked.count.saturating_sub(named);
        rows.iter()
            .map(|row| {
                if row.kind != DialerRowKind::CallHistory(CallHistoryKind::Missed) {
                    return false;
                }
                if let Some(index) = callers
                    .iter()
                    .position(|caller| self.same_caller(caller, &row.caller))
                {
                    callers.remove(index);
                    true
                } else if earlier > 0 {
                    earlier -= 1;
                    true
                } else {
                    false
                }
            })
            .collect()
    }
}

/// Keeps the Missed Call cue for the next run whenever this step changed
/// its count from `before`.
pub(super) fn keep_cue(before: u32, state: &State, effects: &mut Vec<Effect>) {
    let count = state.missed.unseen.count;
    if count != before {
        effects.push(Effect::KeepMissedCalls(count));
    }
}

/// The Dialer opens: it marks the Missed Calls the cue counted, which the
/// user now sees, so the cue clears.
pub(super) fn seen_in_dialer(state: &mut State) {
    state.missed.marked = std::mem::take(&mut state.missed.unseen);
}

/// With nothing typed, the Dialer lists the Call History newest first: the
/// Missed Calls it marks go to the top, in their order, the rest after them
/// in theirs.
pub(super) fn missed_first(state: &State, rows: Vec<DialerRow>) -> Vec<DialerRow> {
    let marks = state.marks(&rows);
    let (first, rest): (Vec<_>, Vec<_>) =
        rows.into_iter().zip(marks).partition(|(_, marked)| *marked);
    first.into_iter().chain(rest).map(|(row, _)| row).collect()
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
    if state.dialer.is_some() {
        // The user is looking at the Call History: the open Dialer marks
        // it, and the Indicator raises no cue behind it.
        state.missed.marked.add(notified.caller.clone());
        refresh_dialer(state);
    } else {
        state.missed.unseen.add(notified.caller.clone());
    }
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
