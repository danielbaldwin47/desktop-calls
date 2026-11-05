//! Call waiting: a second Call ringing during a connected one, and Hold &
//! answer, End & answer and Swap. The Phone carries them out on its
//! AudioGateway, and may refuse them; then both Calls stay as they were and
//! the Call Window says so.

use super::audio::bring_audio_here;
use super::input::{Effect, Input, PhoneCommand, Timer};
use super::{NOTICE_MS, State};
use crate::call::{AudioLocation, Call, CallId, CallState, Request};
use crate::messages::Failure;

impl State {
    /// The Call ringing while another is connected: the one the Banner offers
    /// Hold & answer, End & answer and Decline for.
    pub(super) fn waiting_call(&self) -> Option<&Call> {
        self.calls.iter().find(|call| {
            call.rings_here()
                && self
                    .calls
                    .iter()
                    .any(|other| other.id != call.id && other.state.is_connected())
        })
    }

    /// Whether a Call is on hold, so Swap has something to swap to.
    pub(super) fn has_held_call(&self) -> bool {
        self.calls.iter().any(|call| call.state == CallState::Held)
    }
}

/// Hold & answer: the Phone puts the connected Call on hold and answers the
/// waiting one. Answered here, its audio comes here, as for any Call
/// answered on the Desktop.
pub(super) fn hold_and_answer(state: &mut State, effects: &mut Vec<Effect>) {
    answer_waiting(
        state,
        &Input::HoldAndAnswer,
        PhoneCommand::HoldAndAnswer,
        effects,
    );
}

/// End & answer: the Phone ends the connected Call and answers the waiting
/// one, its audio here.
pub(super) fn end_and_answer(state: &mut State, effects: &mut Vec<Effect>) {
    answer_waiting(
        state,
        &Input::EndAndAnswer,
        PhoneCommand::EndAndAnswer,
        effects,
    );
}

fn answer_waiting(
    state: &mut State,
    input: &Input,
    command: fn(CallId) -> PhoneCommand,
    effects: &mut Vec<Effect>,
) {
    if let Some(failure) = state.refusal(input) {
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
        bring_audio_here(state, effects);
    }
}

/// Swap: the Phone puts the live Call on hold and makes the held one live
/// again; with only a held Call, it is live again.
pub(super) fn swap(state: &mut State, effects: &mut Vec<Effect>) {
    if let Some(failure) = state.refusal(&Input::Swap) {
        effects.push(Effect::ReportError(failure));
        return;
    }
    state.waiting_notice = None;
    effects.push(Effect::Phone(PhoneCommand::Swap));
}

/// The Phone refused Hold & answer, End & answer or Swap, or never
/// answered. It changed nothing, so the waiting Call rings on, and the Call
/// Window says what happened for a while.
pub(super) fn call_waiting_failed(
    state: &mut State,
    command: &PhoneCommand,
    effects: &mut Vec<Effect>,
) {
    let failure = match *command {
        PhoneCommand::HoldAndAnswer(call) | PhoneCommand::EndAndAnswer(call) => {
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
            if matches!(command, PhoneCommand::HoldAndAnswer(_)) {
                Failure::HoldAndAnswerFailed
            } else {
                Failure::EndAndAnswerFailed
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
