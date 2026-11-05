//! Missed Calls: deciding whether a Call that rang out was missed, and calling
//! back.

use super::State;
use super::dialling::{dial, open_dialer};
use super::input::{Effect, Input};
use super::phone::contacts_sync;
use crate::call::{CallId, Caller, Timestamp};
use crate::contacts::{CallHistory, CallHistoryKind};
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
                call_back(state, &notified, effects);
            }
        }
        Input::MissedCallDismissed(notification) => {
            state.missed.take(notification);
        }
        _ => {}
    }
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
