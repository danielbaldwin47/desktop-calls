//! Call waiting: a Waiting Call ringing while the user is on another Call,
//! and Hold & Answer, End & Answer and Swap. The Phone carries them out on
//! its AudioGateway, and may refuse them; then both Calls stay as they were
//! and the Call Window says so.
//!
//! While the user's own Call is still being placed, the Phone can't hold it,
//! and PipeWire's `ReleaseAndAnswer` needs an active Call, so End & Answer
//! hangs that Call up and answers the Waiting Call once the Phone presents
//! it as ringing.
//!
//! With a Call on hold already, the Phone holds no second one, so only End &
//! Answer is offered; and with that Call on hold alone, nothing is live to
//! hold or end, so Answer is: PipeWire's `SwapCalls` (`AT+CHLD=2`), which
//! accepts the Waiting Call and keeps the held one.

use super::audio::bring_audio_here;
use super::input::{Effect, Input, PhoneCommand, Timer};
use super::view::WaitingOptions;
use super::{NOTICE_MS, State};
use crate::call::{AudioLocation, Call, CallId, CallState, Request};
use crate::messages::Failure;

impl State {
    /// The Waiting Call: one ringing while the user is on another Call, on
    /// hold or live, or still placing one. The Banner offers it what
    /// [`State::waiting_options`] says, and Decline, never plain Answer, as
    /// the Phone answers a `waiting` Call only with `AT+CHLD`.
    pub(super) fn waiting_call(&self) -> Option<&Call> {
        self.calls.iter().find(|call| {
            call.rings_here()
                && self
                    .calls
                    .iter()
                    .any(|other| other.id != call.id && other.state.is_live())
        })
    }

    /// What the Banner offers for the Waiting Call: Hold & Answer too, unless
    /// the user's other Call is still being placed or a Call is on hold
    /// already; Answer alone if the only other Call is on hold.
    pub(super) fn waiting_options(&self) -> WaitingOptions {
        let live_beside_held = self.calls.iter().any(|call| {
            matches!(
                call.state,
                CallState::Active | CallState::Dialing | CallState::Alerting
            )
        });
        if self.has_held_call() {
            if live_beside_held {
                WaitingOptions::End
            } else {
                WaitingOptions::Answer
            }
        } else if self.unconnected_call().is_some() && !self.has_active_call() {
            WaitingOptions::End
        } else {
            WaitingOptions::HoldOrEnd
        }
    }

    fn has_active_call(&self) -> bool {
        self.calls
            .iter()
            .any(|call| call.state == CallState::Active)
    }

    /// Whether the Phone has a Call in state `waiting`, answered or not. Then
    /// `Hangup` on a Held Call is `AT+CHLD=0`, which rejects the Waiting
    /// Call first, and on the live Call beside a Held one `AT+CHLD=1`, which
    /// answers it.
    pub(super) fn has_waiting_call(&self) -> bool {
        self.calls
            .iter()
            .any(|call| call.state == CallState::Waiting)
    }

    /// The user's own Call still being placed: dialing, or ringing at the
    /// other end.
    fn unconnected_call(&self) -> Option<CallId> {
        self.calls
            .iter()
            .find(|call| matches!(call.state, CallState::Dialing | CallState::Alerting))
            .map(|call| call.id)
    }

    /// Whether a Call is on hold, so Swap has something to swap to.
    pub(super) fn has_held_call(&self) -> bool {
        self.calls.iter().any(|call| call.state == CallState::Held)
    }
}

/// Hold & Answer: the Phone puts the connected Call on hold and answers the
/// waiting one. Answered here, its audio comes here, as for any Call
/// answered on the Desktop.
pub(super) fn hold_and_answer(state: &mut State, effects: &mut Vec<Effect>) {
    let refusal = state.refusal(&Input::HoldAndAnswer);
    answer_waiting(state, refusal, PhoneCommand::HoldAndAnswer, effects);
}

/// End & Answer: the Phone ends the live Call and answers the Waiting Call,
/// its audio here; a Call on hold stays on hold. With the user's own Call
/// still being placed, that Call is hung up instead, and the Waiting Call
/// answered once it rings.
pub(super) fn end_and_answer(state: &mut State, effects: &mut Vec<Effect>) {
    if state.refusal(&Input::EndAndAnswer).is_none()
        && !state.has_active_call()
        && let (Some(waiting), Some(placing)) = (
            state.waiting_call().map(|call| call.id),
            state.unconnected_call(),
        )
    {
        state.waiting_notice = None;
        if let Some(call) = state.call_mut(waiting) {
            call.request = Request::Answer;
        }
        if let Some(call) = state.call_mut(placing) {
            call.request = Request::HangUp;
        }
        effects.push(Effect::Phone(PhoneCommand::HangUp(placing)));
        return;
    }
    let refusal = state.refusal(&Input::EndAndAnswer);
    answer_waiting(state, refusal, PhoneCommand::EndAndAnswer, effects);
}

/// Answer for the Waiting Call while the user's only other Call is on hold:
/// the Phone answers it and keeps that Call on hold.
pub(super) fn answer_beside_held(state: &mut State, effects: &mut Vec<Effect>) {
    answer_waiting(state, None, PhoneCommand::AnswerBesideHeld, effects);
}

/// The Phone presents a Call the user chose End & Answer for as ringing,
/// now that the Call being placed has ended: it is answered, as Answer
/// does.
pub(super) fn answer_once_ringing(state: &mut State, call: CallId, effects: &mut Vec<Effect>) {
    effects.push(Effect::Phone(PhoneCommand::Answer(call)));
    bring_audio_here(state, Some(call), effects);
}

/// The Phone didn't hang up the Call being placed for End & Answer: the
/// Waiting Call rings on, and the Banner offers it again.
pub(super) fn placing_hang_up_failed(state: &mut State) {
    for call in &mut state.calls {
        if call.state == CallState::Waiting && call.request == Request::Answer {
            call.request = Request::None;
        }
    }
}

fn answer_waiting(
    state: &mut State,
    refusal: Option<Failure>,
    command: fn(CallId) -> PhoneCommand,
    effects: &mut Vec<Effect>,
) {
    if let Some(failure) = refusal {
        effects.push(Effect::ReportError(failure));
        return;
    }
    let Some(waiting) = state.waiting_call().map(|call| call.id) else {
        return;
    };
    state.waiting_notice = None;
    if let Some(call) = state.call_mut(waiting) {
        call.request = Request::Answer;
    }
    effects.push(Effect::Phone(command(waiting)));
    if state.audio_location() == Some(AudioLocation::Phone)
        && state.moving_audio.is_none()
        && state.moving_to_phone.is_none()
    {
        bring_audio_here(state, Some(waiting), effects);
    }
}

/// Swap: the Phone puts the live Call on hold and makes the held one live
/// again; with only a Held Call, it is live again.
pub(super) fn swap(state: &mut State, effects: &mut Vec<Effect>) {
    if let Some(failure) = state.refusal(&Input::Swap) {
        effects.push(Effect::ReportError(failure));
        return;
    }
    state.waiting_notice = None;
    effects.push(Effect::Phone(PhoneCommand::Swap));
}

/// The Phone refused Hold & Answer, End & Answer or Swap, or never
/// answered. It changed nothing, so the Waiting Call rings on, and the Call
/// Window says what happened for a while.
pub(super) fn call_waiting_failed(
    state: &mut State,
    command: &PhoneCommand,
    effects: &mut Vec<Effect>,
) {
    let failure = match *command {
        PhoneCommand::HoldAndAnswer(call)
        | PhoneCommand::EndAndAnswer(call)
        | PhoneCommand::AnswerBesideHeld(call) => {
            if let Some(known) = state.call_mut(call)
                && known.request == Request::Answer
            {
                known.request = Request::None;
            }
            // The batch stopped here: the audio was never asked for.
            state.moving_audio = None;
            if !state.has_desktop_audio() {
                effects.push(Effect::Phone(PhoneCommand::KeepAudioOnPhone));
            }
            match command {
                PhoneCommand::HoldAndAnswer(_) => Failure::HoldAndAnswerFailed,
                PhoneCommand::EndAndAnswer(_) => Failure::EndAndAnswerFailed,
                _ => Failure::AnswerFailed,
            }
        }
        PhoneCommand::Swap => Failure::SwapFailed,
        _ => return,
    };
    state.waiting_notice = Some(failure);
    state.waiting_notices += 1;
    effects.push(Effect::ReportError(failure));
    effects.push(Effect::StartTimer {
        timer: Timer::DismissCallWaitingNotice(state.waiting_notices),
        after_ms: NOTICE_MS,
    });
}

/// The Call Window has said for long enough why the Phone refused.
pub(super) fn dismiss_notice(state: &mut State, notice: u32) {
    if notice == state.waiting_notices {
        state.waiting_notice = None;
    }
}
