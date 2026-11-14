//! Calls coming and going: the Phone reporting them, answering, declining,
//! hanging up and keypad tones, and the timers that close them.

use super::audio::{
    ask_once_more, audio_not_moved, bring_audio_here, call_back, call_gone_mid_handoff,
    end_handoff_to_phone, restore_audio, to_phone_timed_out,
};
use super::call_waiting::{
    answer_beside_held, answer_once_ringing, dismiss_notice, end_and_answer, hold_and_answer,
};
use super::dialling::{adopt_dialled, dial_failed};
use super::input::{Effect, Input, PhoneCommand, Timer};
use super::missed_calls::missed;
use super::presence::{connection_lost_over, found_lost_call, lost_track, reconnect_due};
use super::setup::setup_timer;
use super::view::WaitingOptions;
use super::{CALL_ENDED_MS, EndedCall, RECENT_CALLS, State};
use crate::call::{Call, CallId, CallState, Caller, Direction, Request, Timestamp};
use crate::messages::Failure;
use crate::missed::Unconfirmed;

/// The Phone reported `call`, unless it already had.
pub(super) fn call_added(state: &mut State, mut call: Call, effects: &mut Vec<Effect>) {
    if state.calls.iter().any(|known| known.id == call.id) {
        return;
    }
    if state
        .ended
        .as_ref()
        .is_some_and(|ended| ended.id == call.id)
    {
        state.ended = None;
    }
    if call.direction == Direction::Outgoing {
        adopt_dialled(state, &mut call, effects);
    }
    if call.active_since.is_some()
        && let Some(index) = state
            .earlier_calls
            .iter()
            .position(|earlier| earlier.caller == call.caller)
    {
        // Connected before this run started: the Call it had then.
        let earlier = state.earlier_calls.remove(index);
        call.direction = earlier.direction;
        call.active_since = Some(earlier.active_since);
    }
    if state.recent_calls.len() == RECENT_CALLS {
        state.recent_calls.remove(0);
    }
    state.recent_calls.push((call.id, call.caller.clone()));
    let id = call.id;
    state.calls.push(call);
    found_lost_call(state, id);
}

/// The Phone reports the Call over: it leaves the Desktop, and if it rang
/// out, it may be a Missed Call. With the Calls out of reach, the Phone
/// link has only given up on it.
pub(super) fn phone_ended_call(state: &mut State, call: CallId, effects: &mut Vec<Effect>) {
    call_gone_mid_handoff(state, effects);
    if state.calls_out_of_reach() {
        lost_track(state, call);
        return;
    }
    let ended = state.calls.iter().find(|known| known.id == call).cloned();
    end_call(state, call, effects);
    if let Some(ended) = ended {
        call_ended(state, &ended, effects);
    }
}

/// The earlier run's Calls that haven't shown again by now had ended.
fn forget_earlier_run(state: &mut State, effects: &mut Vec<Effect>) {
    if !state.earlier_calls.is_empty() {
        effects.push(Effect::EarlierCallsForgotten(state.earlier_calls.len()));
        state.earlier_calls.clear();
    }
}

pub(super) fn timer_fired(state: &mut State, timer: Timer, effects: &mut Vec<Effect>) {
    match timer {
        Timer::ForgetEarlierRun => forget_earlier_run(state, effects),
        Timer::Reconnect(wait) => reconnect_due(state, wait, effects),
        Timer::ConnectionLost(loss) => connection_lost_over(state, loss),
        Timer::SetupStep(wait) => setup_timer(state, wait, effects),
        Timer::NumberLate(call) => state.awaiting_number.retain(|id| *id != call),
        Timer::CheckDesktop => super::desktop::check_again(state, effects),
        Timer::CollapseCallWindow(timer) => super::collapse::timer_ran_out(state, timer),
        Timer::DismissDial(attempt) => {
            if state
                .notice
                .as_ref()
                .is_some_and(|notice| notice.attempt == attempt)
            {
                state.notice = None;
            }
            if state
                .hung_up_dial
                .as_ref()
                .is_some_and(|hung_up| hung_up.attempt == attempt)
            {
                state.hung_up_dial = None;
            }
        }
        Timer::DialTimedOut(attempt) => {
            if state
                .dialling
                .as_ref()
                .is_some_and(|dialling| dialling.attempt == attempt)
            {
                dial_failed(state, Failure::DialFailed, effects);
            }
        }
        Timer::HandoffTimedOut(handoff) => {
            if state
                .moving_audio
                .is_some_and(|moving| moving.handoff == handoff)
            {
                audio_not_moved(state, effects);
            }
            to_phone_timed_out(state, handoff, effects);
        }
        Timer::HandoffNotStarted(handoff) => {
            if state
                .moving_audio
                .is_some_and(|moving| moving.handoff == handoff && !moving.retried)
                && !state.transport.is_up()
            {
                ask_once_more(state, effects);
            }
        }
        Timer::DismissCallWaitingNotice(notice) => dismiss_notice(state, notice),
        Timer::CloseCallWindow(call) => {
            if state.ended.as_ref().is_some_and(|ended| ended.id == call) {
                state.ended = None;
            }
        }
    }
}

/// The keypad tones in `typed`: digits, `*` and `#`, leaving out the
/// spaces, dashes, dots and brackets a pasted number may carry. `None` if
/// it has anything else, or no tone at all.
#[must_use]
pub fn keypad_tones(typed: &str) -> Option<String> {
    let mut tones = String::new();
    for character in typed.chars() {
        match character {
            '0'..='9' | '*' | '#' => tones.push(character),
            ' ' | '-' | '.' | '(' | ')' => {}
            _ => return None,
        }
    }
    (!tones.is_empty()).then_some(tones)
}

pub(super) fn send_tones(state: &State, typed: String, effects: &mut Vec<Effect>) {
    let input = Input::SendTones(typed);
    if let Some(failure) = state.refusal(&input) {
        effects.push(Effect::ReportError(failure));
    } else if let Input::SendTones(typed) = input
        && let Some(tones) = keypad_tones(&typed)
    {
        effects.push(Effect::Phone(PhoneCommand::SendTones(tones)));
    }
}

/// The Phone reported a Call's new state.
pub(super) fn call_changed(
    state: &mut State,
    call: CallId,
    call_state: CallState,
    at: Timestamp,
    effects: &mut Vec<Effect>,
) {
    state.waiting_notice = None;
    let Some(known) = state.call_mut(call) else {
        return;
    };
    known.state = call_state;
    if call_state.is_connected() && known.active_since.is_none() {
        known.active_since = Some(at);
    }
    // A Waiting Call the user chose End & Answer for, ringing now that their
    // own Call has ended.
    let answer_now = call_state == CallState::Ringing && known.request == Request::Answer;
    if let Some(restore) = &mut state.restore_audio {
        restore.call_back = true;
    }
    restore_audio(state, effects);
    call_back(state);
    found_lost_call(state, call);
    if answer_now {
        answer_once_ringing(state, call, effects);
    }
}

/// The Phone reported who `call` is with after reporting the Call. Every
/// surface showing it follows, and so does the Missed Call it may become,
/// since that takes the caller when the Call ends.
pub(super) fn caller_changed(state: &mut State, call: CallId, caller: Caller) {
    let Some(known) = state.call_mut(call) else {
        return;
    };
    known.caller = caller.clone();
    if let Some((_, recent)) = state
        .recent_calls
        .iter_mut()
        .rev()
        .find(|(id, _)| *id == call)
    {
        *recent = caller;
    }
}

pub(super) fn answer(state: &mut State, call: CallId, effects: &mut Vec<Effect>) {
    if lost_contact(state, effects) {
        return;
    }
    // Answering a Waiting Call keeps the connected one, on hold: the choice
    // that loses nothing. A Call still being placed can't be held, so it
    // ends; but a connected Call never ends unasked, so with one on hold
    // already, Hold & Answer's refusal says to choose.
    if state
        .waiting_call()
        .is_some_and(|waiting| waiting.id == call)
    {
        match state.waiting_options() {
            WaitingOptions::End if !state.has_held_call() => end_and_answer(state, effects),
            WaitingOptions::HoldOrEnd | WaitingOptions::End => hold_and_answer(state, effects),
            WaitingOptions::Answer => answer_beside_held(state, effects),
        }
        return;
    }
    match state.call_mut(call) {
        Some(known) if known.rings_here() => {
            known.request = Request::Answer;
            effects.push(Effect::Phone(PhoneCommand::Answer(call)));
            bring_audio_here(state, Some(call), effects);
        }
        Some(known) if known.request == Request::Answer => {}
        Some(_) => effects.push(Effect::ReportError(Failure::CallNotRinging)),
        None => effects.push(Effect::ReportError(Failure::NoSuchCall)),
    }
}

pub(super) fn decline(state: &mut State, call: CallId, effects: &mut Vec<Effect>) {
    if lost_contact(state, effects) {
        return;
    }
    match state.call_mut(call) {
        Some(known) if known.rings_here() => {
            known.request = Request::Decline;
            effects.push(Effect::Phone(PhoneCommand::HangUp(call)));
        }
        Some(known) if known.request == Request::Decline => {}
        Some(_) => effects.push(Effect::ReportError(Failure::CallNotRinging)),
        None => effects.push(Effect::ReportError(Failure::NoSuchCall)),
    }
}

/// While the Call Window says the connection to the Phone is lost and the
/// Calls are still out of reach, a ringing Call can't be answered or
/// declined from here: says so, and returns whether it did.
fn lost_contact(state: &State, effects: &mut Vec<Effect>) -> bool {
    let lost = state.connection_lost.is_some() && state.calls_out_of_reach();
    if lost {
        effects.push(Effect::ReportError(Failure::ConnectionLost));
    }
    lost
}

pub(super) fn hang_up(state: &mut State, call: CallId, effects: &mut Vec<Effect>) {
    if state.connection_lost.is_some() {
        // A hang-up kept until the Phone is back could end the Call the user
        // is now on, on the Phone.
        effects.push(Effect::ReportError(Failure::ConnectionLost));
        return;
    }
    if state.has_waiting_call()
        && state.has_held_call()
        && state
            .calls
            .iter()
            .any(|known| known.id == call && known.state != CallState::Waiting)
    {
        // On the Held Call, `AT+CHLD=0` would reject the Waiting Call before
        // it; on the live one, `AT+CHLD=1` would answer the Waiting Call.
        effects.push(Effect::ReportError(Failure::WaitingCallFirst));
        return;
    }
    match state.call_mut(call) {
        Some(known) if matches!(known.request, Request::HangUp | Request::Decline) => {}
        Some(known) => {
            known.request = if known.rings_here() {
                Request::Decline
            } else {
                Request::HangUp
            };
            effects.push(Effect::Phone(PhoneCommand::HangUp(call)));
        }
        None => effects.push(Effect::ReportError(Failure::NoSuchCall)),
    }
}

/// A Call the Phone had is over, and now in the Phone's Call History, so
/// the Desktop refreshes it. A Call that rang out here waits for that
/// refresh to say whether the Phone lists it as missed; if there can be no
/// refresh, it is missed.
fn call_ended(state: &mut State, ended: &Call, effects: &mut Vec<Effect>) {
    let refreshes = state.may_sync_contacts();
    if refreshes {
        effects.push(Effect::RefreshCallHistory);
        state.missed.refreshes_asked = state.missed.refreshes_asked.saturating_add(1);
    }
    if !ended.rang_out() {
        return;
    }
    if refreshes {
        state.missed.unconfirmed.push(Unconfirmed {
            call: ended.id,
            caller: ended.caller.clone(),
            at: ended.added_at,
            before: state.call_history.clone(),
            refresh: state.missed.refreshes_asked,
        });
    } else {
        missed(
            state,
            ended.id,
            ended.caller.clone(),
            ended.added_at,
            effects,
        );
    }
}

fn end_call(state: &mut State, call: CallId, effects: &mut Vec<Effect>) {
    let Some(index) = state.calls.iter().position(|known| known.id == call) else {
        return;
    };
    let ended = state.calls.remove(index);
    state.waiting_notice = None;
    if state.unwanted == Some(call) {
        state.unwanted = None;
    } else if ended.has_call_window() {
        state.ended = Some(EndedCall {
            id: ended.id,
            caller: ended.caller,
        });
        effects.push(Effect::StartTimer {
            timer: Timer::CloseCallWindow(call),
            after_ms: CALL_ENDED_MS,
        });
    }
    if state.calls.is_empty() {
        state.codec = None;
        state.moving_audio = None;
        end_handoff_to_phone(state);
        state.restore_audio = None;
        state.audio_notice = None;
        effects.push(Effect::Phone(PhoneCommand::KeepAudioOnPhone));
    } else if ended.active_since.is_none()
        && state
            .moving_audio
            .is_some_and(|moving| moving.answering == Some(call))
    {
        // The user answered a Call that ended before it connected, so the
        // Handoff that followed its Answer is moot. The Phone link drops a
        // plain Answer whose Call is gone, with the commands behind it, and
        // reports nothing; Hold & Answer and the like are still sent, and
        // their refusal comes later. The Call still live keeps its audio
        // where it is.
        state.moving_audio = None;
        if !state.has_desktop_audio() {
            effects.push(Effect::Phone(PhoneCommand::KeepAudioOnPhone));
        }
    }
}
