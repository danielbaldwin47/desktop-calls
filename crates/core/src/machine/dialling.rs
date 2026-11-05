//! Placing a Call from the Desktop, and the Dialer.

use super::input::{DialProgress, Effect, Input, PhoneCommand, PhoneRefusal, Timer};
use super::{
    CALL_ENDED_MS, DIAL_TIMEOUT_MS, Dialer, Dialling, Focus, HungUpDial, NOTICE_MS, Notice, State,
};
use crate::call::{Call, CallId, Caller, Request};
use crate::matching::{self, DialerRow};
use crate::messages::Failure;

/// Places a Call to `text`, a number or a `tel:` link, unless it can't be
/// dialled or placed now. A Call placed here has its audio on the Desktop,
/// so the Phone may bring the audio here before it dials.
pub(super) fn dial(state: &mut State, text: &str, effects: &mut Vec<Effect>) {
    state.dial_attempts += 1;
    let attempt = state.dial_attempts;
    let Some(number) = matching::dialable(text) else {
        let caller = Caller::Number(text.trim().to_owned());
        refuse(state, Failure::Undialable, attempt, caller, effects);
        return;
    };
    if let Some(failure) = state.dial_refusal() {
        refuse(state, failure, attempt, Caller::Number(number), effects);
        return;
    }
    // The Call is starting, so the Dialer's work is done.
    state.dialer = None;
    state.ended = None;
    state.notice = None;
    state.hung_up_dial = None;
    state.dialling = Some(Dialling {
        number: number.clone(),
        attempt,
        cancelled: false,
    });
    effects.push(Effect::Phone(PhoneCommand::AllowAudioOnDesktop));
    effects.push(Effect::Phone(PhoneCommand::Dial(number)));
    effects.push(Effect::DialProgress(DialProgress::Asked(attempt)));
    effects.push(Effect::StartTimer {
        timer: Timer::DialTimedOut(attempt),
        after_ms: DIAL_TIMEOUT_MS,
    });
}

/// The first outgoing Call after a dial is the one dialled: it takes the
/// dialled number if the Phone withholds it, and is hung up at once if the
/// user hung up first.
pub(super) fn adopt_dialled(state: &mut State, call: &mut Call, effects: &mut Vec<Effect>) {
    let Some(dialling) = state.dialling.take() else {
        return;
    };
    effects.push(Effect::DialProgress(DialProgress::Placed(dialling.attempt)));
    if call.caller == Caller::Withheld {
        call.caller = Caller::Number(dialling.number);
    }
    if dialling.cancelled {
        call.request = Request::HangUp;
        state.unwanted = Some(call.id);
        effects.push(Effect::Phone(PhoneCommand::HangUp(call.id)));
    }
}

/// The user hung up the Call they placed before the Phone reported it: the
/// Call Window says "Call ended", and the Call is hung up when it appears.
pub(super) fn cancel_dial(state: &mut State, effects: &mut Vec<Effect>) {
    let Some(dialling) = state
        .dialling
        .as_mut()
        .filter(|dialling| !dialling.cancelled)
    else {
        return;
    };
    dialling.cancelled = true;
    let attempt = dialling.attempt;
    state.hung_up_dial = Some(HungUpDial {
        attempt,
        caller: Caller::Number(dialling.number.clone()),
    });
    effects.push(Effect::StartTimer {
        timer: Timer::DismissDial(attempt),
        after_ms: CALL_ENDED_MS,
    });
}

/// Refuses dial attempt `attempt` to `caller`. With no other Call in the
/// Call Window, the window shows the attempt as not placed; otherwise the
/// refusal is a note beside that Call.
fn refuse(
    state: &mut State,
    failure: Failure,
    attempt: u32,
    caller: Caller,
    effects: &mut Vec<Effect>,
) {
    let alone = state.calls.is_empty() && state.dialling.is_none();
    if alone {
        state.ended = None;
        state.hung_up_dial = None;
    }
    effects.push(Effect::DialProgress(DialProgress::NotPlaced(
        attempt, failure,
    )));
    notify(state, failure, attempt, alone.then_some(caller), effects);
}

/// The pending dial is over without a Call: say why, unless the user hung
/// up already, and keep the next Call's audio on the Phone.
pub(super) fn dial_failed(state: &mut State, failure: Failure, effects: &mut Vec<Effect>) {
    let Some(dialling) = state.dialling.take() else {
        return;
    };
    effects.push(Effect::DialProgress(DialProgress::NotPlaced(
        dialling.attempt,
        failure,
    )));
    if !dialling.cancelled {
        let caller = Caller::Number(dialling.number);
        notify(state, failure, dialling.attempt, Some(caller), effects);
    }
    if !state.has_desktop_audio() {
        effects.push(Effect::Phone(PhoneCommand::KeepAudioOnPhone));
    }
}

/// The Phone was unpaired, so a dial waiting for it places no Call.
pub(super) fn phone_unpaired(state: &State, effects: &mut Vec<Effect>) {
    if let Some(dialling) = &state.dialling {
        effects.push(Effect::DialProgress(DialProgress::NotPlaced(
            dialling.attempt,
            Failure::PhoneNotSetUp,
        )));
    }
}

fn notify(
    state: &mut State,
    failure: Failure,
    attempt: u32,
    not_placed: Option<Caller>,
    effects: &mut Vec<Effect>,
) {
    state.notice = Some(Notice {
        failure,
        attempt,
        not_placed,
    });
    effects.push(Effect::ReportError(failure));
    effects.push(Effect::StartTimer {
        timer: Timer::DismissDial(attempt),
        after_ms: NOTICE_MS,
    });
}

pub(super) fn refusal_failure(refusal: PhoneRefusal) -> Failure {
    match refusal {
        PhoneRefusal::InvalidFormat => Failure::NumberRefused,
        PhoneRefusal::InvalidState | PhoneRefusal::InProgress => Failure::PhoneBusy,
        PhoneRefusal::NotSupported => Failure::DialNotSupported,
        PhoneRefusal::NoNetwork => Failure::NoSignal,
        PhoneRefusal::NetworkTimeout => Failure::NetworkError,
        PhoneRefusal::Failed => Failure::DialFailed,
    }
}

/// What the user does in the Dialer.
pub(super) fn dialer_input(state: &mut State, input: Input, effects: &mut Vec<Effect>) {
    match input {
        Input::OpenDialer { query, select_call } => open_dialer(state, query, select_call),
        Input::DialerQuery(query) => dialer_query(state, query),
        Input::DialerMove(by) => dialer_move(state, by),
        Input::DialerEnter => dialer_enter(state, None, effects),
        Input::DialerChoose(index) => dialer_enter(state, Some(index), effects),
        Input::CloseDialer => state.dialer = None,
        _ => {}
    }
}

/// Opens the Dialer afresh with `query` typed. It takes the keyboard.
pub(super) fn open_dialer(state: &mut State, query: String, select_call: Option<CallId>) {
    state.focus = Focus::None;
    state.dialer_openings = state.dialer_openings.wrapping_add(1);
    let caller = select_call.and_then(|call| {
        state
            .recent_calls
            .iter()
            .rev()
            .find(|(id, _)| *id == call)
            .map(|(_, caller)| caller.clone())
    });
    let mut query = query;
    let mut rows = matching::search(&query, &state.contacts, &state.call_history);
    let find = |rows: &[DialerRow]| {
        caller.as_ref().and_then(|caller| {
            rows.iter()
                .position(|row| state.same_caller(&row.caller, caller))
        })
    };
    let mut found = find(&rows);
    if let (None, true, Some(Caller::Number(number))) =
        (found, query.trim().is_empty(), caller.as_ref())
    {
        // The Call isn't in the Call History the Desktop holds: look the
        // caller up by number instead, so Enter calls them and nobody else.
        query.clone_from(number);
        rows = matching::search(&query, &state.contacts, &state.call_history);
        found = find(&rows);
    }
    let selected = found.unwrap_or(0);
    state.dialer = Some(Dialer {
        notice: undialable(&query, &rows),
        query,
        rows,
        selected,
        opened: state.dialer_openings,
    });
}

/// What is typed can't be dialled and matches nothing: say so before Enter.
fn undialable(query: &str, rows: &[DialerRow]) -> Option<Failure> {
    (rows.is_empty() && matching::looks_undialable(query)).then_some(Failure::Undialable)
}

fn dialer_query(state: &mut State, query: String) {
    if state
        .dialer
        .as_ref()
        .is_none_or(|dialer| dialer.query == query)
    {
        return;
    }
    let rows = matching::search(&query, &state.contacts, &state.call_history);
    if let Some(dialer) = state.dialer.as_mut() {
        dialer.notice = undialable(&query, &rows);
        dialer.query = query;
        dialer.rows = rows;
        dialer.selected = 0;
    }
}

/// Contacts or the Call History changed: the open Dialer lists them anew,
/// keeping the selection where it was as far as it can.
pub(super) fn refresh_dialer(state: &mut State) {
    let Some(query) = state.dialer.as_ref().map(|dialer| dialer.query.clone()) else {
        return;
    };
    let rows = matching::search(&query, &state.contacts, &state.call_history);
    if let Some(dialer) = state.dialer.as_mut() {
        dialer.selected = dialer.selected.min(rows.len().saturating_sub(1));
        dialer.rows = rows;
    }
}

fn dialer_move(state: &mut State, by: i32) {
    let Some(dialer) = state.dialer.as_mut() else {
        return;
    };
    let last = dialer.rows.len().saturating_sub(1);
    let steps = usize::try_from(by.unsigned_abs()).unwrap_or(usize::MAX);
    dialer.selected = if by >= 0 {
        dialer.selected.saturating_add(steps).min(last)
    } else {
        dialer.selected.saturating_sub(steps)
    };
}

/// Calls the Dialer's row at `index`, or the selected row. If it can't be
/// called now, the Dialer stays open and says why.
fn dialer_enter(state: &mut State, index: Option<usize>, effects: &mut Vec<Effect>) {
    let Some(dialer) = state.dialer.as_mut() else {
        return;
    };
    if let Some(index) = index.filter(|index| *index < dialer.rows.len()) {
        dialer.selected = index;
    }
    let number = match dialer.rows.get(dialer.selected) {
        None => Err(undialable(&dialer.query, &dialer.rows)),
        Some(row) => row.dial().ok_or(Some(if row.caller == Caller::Withheld {
            Failure::WithheldNumber
        } else {
            Failure::Undialable
        })),
    };
    let failure = match number {
        Ok(number) => match state.dial_refusal() {
            None => {
                dial(state, &number, effects);
                return;
            }
            refusal => refusal,
        },
        Err(failure) => failure,
    };
    if let (Some(failure), Some(dialer)) = (failure, state.dialer.as_mut()) {
        dialer.notice = Some(failure);
        effects.push(Effect::ReportError(failure));
    }
}
