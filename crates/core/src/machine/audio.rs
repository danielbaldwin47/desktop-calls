//! Where the Call's audio goes: Handoff between the Phone and the Desktop, and
//! Mute.

use super::dialling::open_dialer;
use super::input::{Effect, Input, PhoneCommand, Timer};
use super::view::CallWindowPhase;
use super::{
    DEFAULT_MICROPHONE_VOLUME, Focus, HANDOFF_NOT_STARTED_MS, HANDOFF_TIMEOUT_MS,
    MAX_MICROPHONE_VOLUME, MovingAudio, Muting, State,
};
use crate::call::{AudioLocation, Transport};
use crate::connection::ConnectionState;
use crate::messages::Failure;

pub(super) fn mute(state: &mut State, effects: &mut Vec<Effect>) {
    if matches!(state.muting, Muting::Silencing { .. } | Muting::On { .. }) {
        return;
    }
    if let Some(failure) = state.refusal(&Input::SetMuted(true)) {
        effects.push(Effect::ReportError(failure));
        return;
    }
    let restore = match state.muting {
        Muting::Restoring { restore } => restore,
        // A gain of 0, or PipeWire's 255 for "the Phone hasn't said", would
        // leave the microphone silent after unmuting.
        _ => match state.microphone_volume {
            Some(volume @ 1..=MAX_MICROPHONE_VOLUME) => volume,
            _ => DEFAULT_MICROPHONE_VOLUME,
        },
    };
    state.muting = Muting::Silencing { restore };
    effects.push(Effect::Phone(PhoneCommand::SetMicrophoneVolume(0)));
}

/// The user asked to unmute.
pub(super) fn unmute(state: &mut State, effects: &mut Vec<Effect>) {
    if let Muting::Silencing { restore } | Muting::On { restore } = state.muting {
        state.muting = Muting::Restoring { restore };
        effects.push(Effect::Phone(PhoneCommand::SetMicrophoneVolume(restore)));
    }
}

/// The audio left the Desktop, or the Call ended: the gain goes back at
/// once, as there is no Mute left to show.
pub(super) fn release_mute(state: &mut State, effects: &mut Vec<Effect>) {
    if let Muting::Silencing { restore } | Muting::On { restore } = state.muting {
        effects.push(Effect::Phone(PhoneCommand::SetMicrophoneVolume(restore)));
        state.silenced = Some(restore);
    }
    state.muting = Muting::Off;
}

/// With the audio on the Desktop, a microphone the Desktop silenced and the
/// Phone still reports at 0 is muted: showing it unmuted would have the
/// user talk to no one.
pub(super) fn still_silenced(state: &mut State) {
    if state.muting == Muting::Off
        && state.microphone_volume == Some(0)
        && let Some(restore) = state.silenced.take()
    {
        state.muting = Muting::On { restore };
    }
}

/// The keybinding: focus the Banner while a Call rings here, otherwise the
/// Call Window while a Call is live, otherwise open the Dialer. Neither
/// touches the Call itself. Only one surface can have the keyboard, so the
/// Dialer closes when another takes it.
pub(super) fn activate(state: &mut State) {
    let surfaces = state.surfaces();
    if surfaces.banner.is_some() {
        state.focus = Focus::Banner;
        state.dialer = None;
    } else if surfaces.call_window.is_some_and(|window| {
        !matches!(
            window.phase,
            CallWindowPhase::Ended | CallWindowPhase::NotPlaced
        )
    }) {
        state.focus = Focus::CallWindow;
        state.dialer = None;
    } else if state.dialer.is_none() {
        open_dialer(state, String::new(), None);
    }
}

/// The Call audio link changed. Once the audio the Desktop asked for is
/// here, `RejectSCO` is set again (ADR-0005); it doesn't close the open link.
/// The audio moving either way answers any notice about where it is.
pub(super) fn transport_changed(
    state: &mut State,
    transport: Transport,
    effects: &mut Vec<Effect>,
) {
    let was_here = state.transport == Transport::Active;
    state.transport = transport;
    if transport.is_up() && state.reported_codec.is_some() {
        state.codec = state.reported_codec;
    }
    let is_here = transport == Transport::Active;
    if was_here != is_here {
        state.audio_notice = None;
    }
    if transport.is_up() {
        // It came back by itself.
        state.restore_audio = None;
    }
    if is_here && state.moving_audio.take().is_some() {
        effects.push(Effect::Phone(PhoneCommand::KeepAudioOnPhone));
    }
}

pub(super) fn microphone_volume_changed(state: &mut State, volume: u8) {
    state.microphone_volume = Some(volume);
    if volume != 0 {
        state.silenced = None;
    }
}

/// Brings the audio back here once hands-free and the Call are back after
/// a drop that took it from the Desktop.
pub(super) fn restore_audio(state: &mut State, effects: &mut Vec<Effect>) {
    let ready = state.restore_audio.is_some_and(|restore| restore.call_back)
        && state.connection == ConnectionState::Connected;
    if ready {
        state.restore_audio = None;
        move_audio(state, AudioLocation::Desktop, effects);
    }
}

/// The user asked to move the audio to `location`.
pub(super) fn move_audio(state: &mut State, location: AudioLocation, effects: &mut Vec<Effect>) {
    let input = Input::MoveAudio(location);
    if let Some(failure) = state.refusal(&input) {
        effects.push(Effect::ReportError(failure));
        if state.calls.iter().any(|call| call.state.is_live()) {
            state.audio_notice = Some(failure);
        }
        return;
    }
    if location == AudioLocation::Desktop
        && state.audio_location() == Some(AudioLocation::Phone)
        && state.moving_audio.is_none()
    {
        state.audio_notice = None;
        bring_audio_here(state, effects);
    }
}

/// Starts a Handoff to the Desktop: asks the Phone to bring the audio here,
/// and starts its overall limit. The check that the audio link has started
/// waits for the Phone to carry out `Activate()`.
pub(super) fn bring_audio_here(state: &mut State, effects: &mut Vec<Effect>) {
    effects.push(Effect::Phone(PhoneCommand::MoveAudioToDesktop));
    state.handoffs += 1;
    let handoff = state.handoffs;
    state.moving_audio = Some(MovingAudio {
        handoff,
        retried: false,
    });
    effects.push(Effect::StartTimer {
        timer: Timer::HandoffTimedOut(handoff),
        after_ms: HANDOFF_TIMEOUT_MS,
    });
}

/// The Phone carried out `Activate()` for the first time in this Handoff:
/// check a second later that the audio link has started.
pub(super) fn activated(state: &State, effects: &mut Vec<Effect>) {
    if let Some(MovingAudio {
        handoff,
        retried: false,
    }) = state.moving_audio
    {
        effects.push(Effect::StartTimer {
            timer: Timer::HandoffNotStarted(handoff),
            after_ms: HANDOFF_NOT_STARTED_MS,
        });
    }
}

/// The Phone refused to bring the audio here, or hadn't started opening the
/// audio link a second after `Activate()`. The Pixel sometimes ignores
/// `Activate()`, mostly just after the Call goes active (ticket #70), so it
/// is asked once more, within the same limit; a second refusal tells the
/// user at once.
pub(super) fn ask_once_more(state: &mut State, effects: &mut Vec<Effect>) {
    match state.moving_audio.as_mut() {
        Some(moving) if !moving.retried => {
            moving.retried = true;
            effects.push(Effect::Phone(PhoneCommand::MoveAudioToDesktop));
        }
        Some(_) => audio_not_moved(state, effects),
        None => {}
    }
}

/// The audio didn't come here: the user is told how to do it on the Phone.
/// `RejectSCO` stays false, so doing it on the Phone works.
pub(super) fn audio_not_moved(state: &mut State, effects: &mut Vec<Effect>) {
    state.moving_audio = None;
    state.audio_notice = Some(Failure::AudioNotMoved);
    effects.push(Effect::ReportError(Failure::AudioNotMoved));
}
