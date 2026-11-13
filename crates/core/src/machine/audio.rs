//! Where the Call's audio goes: Handoff between the Phone and the Desktop, and
//! Mute.

use super::collapse::expand_with_keyboard;
use super::dialling::{dial_failed, open_dialer};
use super::input::{Effect, Input, PhoneCommand, Timer};
use super::phone::{phone_connected, phone_disconnected};
use super::presence::hands_free_lost;
use super::view::CallWindowPhase;
use super::{
    DEFAULT_MICROPHONE_VOLUME, Focus, HANDOFF_NOT_STARTED_MS, HANDOFF_TIMEOUT_MS,
    HANDOFF_TO_PHONE_TIMEOUT_MS, MAX_MICROPHONE_VOLUME, MovingAudio, MovingToPhone, Muting, State,
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
/// Call Window while a Call is live, expanding it if it is collapsed,
/// otherwise open the Dialer. Neither
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
        expand_with_keyboard(state);
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
    if state.moving_audio.is_some() || state.moving_to_phone.is_some() {
        return;
    }
    match (location, state.audio_location()) {
        (AudioLocation::Desktop, Some(AudioLocation::Phone)) => {
            state.audio_notice = None;
            bring_audio_here(state, effects);
        }
        (AudioLocation::Phone, Some(AudioLocation::Desktop)) => {
            state.audio_notice = None;
            send_audio_to_phone(state, effects);
        }
        _ => {}
    }
}

/// Starts a Handoff to the Phone: asks the Phone link to drop hands-free
/// and connect it again (ADR-0005), and starts the Handoff's limit.
fn send_audio_to_phone(state: &mut State, effects: &mut Vec<Effect>) {
    effects.push(Effect::Phone(PhoneCommand::MoveAudioToPhone));
    state.handoffs += 1;
    let handoff = state.handoffs;
    state.moving_to_phone = Some(MovingToPhone {
        handoff,
        dropped: false,
        phone_back: false,
        call_back: false,
    });
    effects.push(Effect::StartTimer {
        timer: Timer::HandoffTimedOut(handoff),
        after_ms: HANDOFF_TO_PHONE_TIMEOUT_MS,
    });
}

/// Hands-free is up: back after dropping for a Handoff to the Phone, or
/// connecting as usual. Back from the Handoff's drop, the new AudioGateway
/// starts with `RejectSCO` set (ADR-0005); setting it again keeps the audio
/// on the Phone even where that default is missing. It is the same link as
/// before the drop, so the Contacts aren't synced again.
pub(super) fn hands_free_up(state: &mut State, effects: &mut Vec<Effect>) {
    match state.moving_to_phone.as_mut() {
        Some(moving) if moving.dropped => {
            moving.phone_back = true;
            effects.push(Effect::Phone(PhoneCommand::KeepAudioOnPhone));
            handoff_to_phone_over(state);
        }
        _ => phone_connected(state, effects),
    }
}

/// Hands-free dropped. During a Handoff to the Phone, that puts the audio
/// on the Phone; the Call goes with hands-free and the Phone link holds it
/// until it comes back, so it stays as it is, and so does the connection.
pub(super) fn hands_free_down(state: &mut State, effects: &mut Vec<Effect>) {
    if let Some(moving) = state.moving_to_phone.as_mut() {
        moving.dropped = true;
        moving.phone_back = false;
        moving.call_back = false;
        state.transport = Transport::Idle;
    } else {
        phone_disconnected(state);
        dial_failed(state, Failure::PhoneOutOfRange, effects);
    }
}

/// The Phone reported a Call: after hands-free dropped for a Handoff to the
/// Phone, that is the Call coming back with it.
pub(super) fn call_back(state: &mut State) {
    if let Some(moving) = state.moving_to_phone.as_mut()
        && moving.dropped
    {
        moving.call_back = true;
        handoff_to_phone_over(state);
    }
}

/// Ends the Handoff to the Phone once hands-free and the Call are both
/// back.
fn handoff_to_phone_over(state: &mut State) {
    if state
        .moving_to_phone
        .is_some_and(|moving| moving.phone_back && moving.call_back)
    {
        state.moving_to_phone = None;
    }
}

/// Forgets the Handoff to the Phone. If hands-free dropped and isn't back,
/// the Phone is now disconnected like any Phone whose hands-free dropped.
pub(super) fn end_handoff_to_phone(state: &mut State) {
    if let Some(moving) = state.moving_to_phone.take()
        && moving.dropped
        && !moving.phone_back
    {
        phone_disconnected(state);
    }
}

/// The Phone link couldn't drop hands-free, or couldn't connect it again.
/// Before the drop, the audio stayed here and the user is told how to move
/// it on the Phone; after it, hands-free is lost.
pub(super) fn audio_not_moved_to_phone(state: &mut State, effects: &mut Vec<Effect>) {
    match state.moving_to_phone {
        Some(moving) if !moving.dropped => {
            state.moving_to_phone = None;
            state.audio_notice = Some(Failure::AudioNotMovedToPhone);
            effects.push(Effect::ReportError(Failure::AudioNotMovedToPhone));
        }
        Some(moving) if !moving.phone_back => hands_free_not_back(state, effects),
        _ => {}
    }
}

/// The Phone link gave up on a Call while hands-free is away for a Handoff
/// to the Phone. The Call went to the Phone with hands-free, and nothing
/// says it ended there: hands-free is taking too long, so it is lost.
pub(super) fn call_gone_mid_handoff(state: &mut State, effects: &mut Vec<Effect>) {
    if state
        .moving_to_phone
        .is_some_and(|moving| moving.dropped && !moving.phone_back)
    {
        hands_free_not_back(state, effects);
    }
}

/// Hands-free dropped for a Handoff to the Phone and isn't coming back by
/// itself. The Call carries on on the Phone, out of the Desktop's reach,
/// and the Phone is disconnected until the Desktop has hands-free again.
fn hands_free_not_back(state: &mut State, effects: &mut Vec<Effect>) {
    state.moving_to_phone = None;
    phone_disconnected(state);
    hands_free_lost(state, effects);
}

/// The Handoff to the Phone's limit: whatever hasn't happened by now
/// isn't going to.
pub(super) fn to_phone_timed_out(state: &mut State, handoff: u32, effects: &mut Vec<Effect>) {
    if state
        .moving_to_phone
        .is_some_and(|moving| moving.handoff == handoff)
    {
        audio_not_moved_to_phone(state, effects);
        end_handoff_to_phone(state);
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
