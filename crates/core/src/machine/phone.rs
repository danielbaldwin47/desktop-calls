//! The Phone link: the Phone connecting and dropping, its Contacts, the Calls
//! an earlier run left, and how the Phone answers commands.

use super::audio::{activated, ask_once_more, audio_not_moved_to_phone};
use super::call_waiting::{call_waiting_failed, placing_hang_up_failed};
use super::dialling::{dial_failed, refresh_dialer};
use super::input::{Effect, Input, PhoneCommand, Timer};
use super::presence::{
    hands_free_back, hands_free_connect_done, hands_free_connect_failed, pairing_found,
    wait_for_lost_call,
};
use super::{AudioToRestore, FORGET_EARLIER_RUN_MS, Muting, State};
use crate::call::{AudioLocation, Request, Transport};
use crate::connection::{ConnectionState, Phone};
use crate::contacts::{CallHistory, Contacts, ContactsState, SyncFailure, Unavailable};
use crate::earlier_run::EarlierRun;
use crate::messages::Failure;

/// The first paired Phone: Desktop Calls follows it, and waits for it to
/// be reachable.
pub(super) fn phone_paired(state: &mut State, phone: Phone, effects: &mut Vec<Effect>) {
    effects.push(Effect::AttachPhone {
        address: phone.address.clone(),
    });
    state.phone = Some(phone);
    state.connection = ConnectionState::Connecting;
}

/// What the Contacts sync reports. The open Dialer lists what it brings.
pub(super) fn contacts_sync(state: &mut State, input: Input, effects: &mut Vec<Effect>) {
    held_contacts(state, input, effects);
    refresh_dialer(state);
}

fn held_contacts(state: &mut State, input: Input, effects: &mut Vec<Effect>) {
    match input {
        Input::ContactsLoaded {
            contacts,
            call_history,
        } => {
            if state.contacts_sync == ContactsState::Unavailable(Unavailable::NotSynced) {
                state.contacts_sync = ContactsState::Ready;
            }
            state.contacts = contacts;
            state.call_history = call_history;
        }
        Input::ContactsSynced {
            contacts,
            call_history,
        } => {
            state.contacts_sync = ContactsState::Ready;
            state.contacts = contacts;
            state.call_history = call_history;
        }
        Input::CallHistoryRefreshed(call_history) => state.call_history = call_history,
        Input::ContactsPermissionNeeded { deadline } => {
            state.contacts_sync = ContactsState::PermissionNeeded { deadline };
        }
        Input::ContactsSyncFailed(SyncFailure::Denied) => {
            // The person said no: keep nothing of theirs.
            state.contacts_sync = ContactsState::Denied;
            state.contacts = Contacts::default();
            state.call_history = CallHistory::default();
            effects.push(Effect::ForgetContacts);
        }
        // A sync cut short by the Phone leaving isn't a failure to report;
        // what was synced before still names callers.
        Input::ContactsSyncFailed(SyncFailure::Unavailable(reason)) if state.phone_is_here() => {
            state.contacts_sync = ContactsState::Unavailable(reason);
        }
        _ => {}
    }
}

pub(super) fn phone_connected(state: &mut State, effects: &mut Vec<Effect>) {
    pairing_found(state);
    hands_free_back(state);
    wait_for_lost_call(state, effects);
    // Every connect syncs, so the Desktop catches up with changes made on
    // the Phone while apart.
    state.contacts_sync = ContactsState::Syncing;
    effects.push(Effect::SyncContacts);
    if state.has_desktop_audio() {
        state.connection = ConnectionState::Connected;
    } else {
        // A crash or a dropped link mid-Call must never leave the next
        // Call's audio jumping to the Desktop. Connected means ready for
        // Calls, so it waits until the Phone has done this.
        state.keeping_audio_on_phone = true;
        effects.push(Effect::Phone(PhoneCommand::KeepAudioOnPhone));
    }
}

/// Hands-free dropped. A Phone that never showed as connected stays
/// Connecting, as before it came into range.
pub(super) fn phone_disconnected(state: &mut State) {
    if state.phone_is_here() {
        state.contacts_sync = state.held_contacts_state();
    }
    state.keeping_audio_on_phone = false;
    if state.audio_location() == Some(AudioLocation::Desktop) {
        state.restore_audio = Some(AudioToRestore { call_back: false });
    }
    if state.connection == ConnectionState::Connected {
        state.connection = ConnectionState::Disconnected;
        state.transport = Transport::Idle;
    }
}

/// Startup with what the run before left: its Calls take their direction
/// and timer from it once the Phone lists them again, and a microphone it
/// silenced for Mute still counts as muted.
pub(super) fn pick_up_earlier_run(
    state: &mut State,
    earlier: EarlierRun,
    effects: &mut Vec<Effect>,
) {
    if !earlier.calls.is_empty() {
        effects.push(Effect::StartTimer {
            timer: Timer::ForgetEarlierRun,
            after_ms: FORGET_EARLIER_RUN_MS,
        });
    }
    state.earlier_calls = earlier.calls;
    state.silenced = earlier.restore_microphone_volume;
}

pub(super) fn command_done(state: &mut State, command: &PhoneCommand, effects: &mut Vec<Effect>) {
    if let PhoneCommand::SetMicrophoneVolume(volume) = command
        && state.silenced == Some(*volume)
    {
        state.silenced = None;
    }
    match (command, state.muting) {
        (PhoneCommand::KeepAudioOnPhone, _) if state.keeping_audio_on_phone => {
            state.keeping_audio_on_phone = false;
            state.connection = ConnectionState::Connected;
        }
        (PhoneCommand::SetMicrophoneVolume(0), Muting::Silencing { restore }) => {
            state.muting = Muting::On { restore };
        }
        (PhoneCommand::SetMicrophoneVolume(volume), Muting::Restoring { restore })
            if *volume == restore =>
        {
            state.muting = Muting::Off;
        }
        (PhoneCommand::MoveAudioToDesktop, _) => activated(state, effects),
        (PhoneCommand::ConnectHandsFree, _) => hands_free_connect_done(state, effects),
        _ => {}
    }
}

pub(super) fn command_failed(state: &mut State, command: &PhoneCommand, effects: &mut Vec<Effect>) {
    match *command {
        PhoneCommand::Answer(call) => {
            if let Some(known) = state.call_mut(call)
                && known.request == Request::Answer
            {
                known.request = Request::None;
            }
            // The batch stopped here: the audio was never asked for.
            state.moving_audio = None;
            effects.push(Effect::ReportError(Failure::AnswerFailed));
            if !state.has_desktop_audio() {
                effects.push(Effect::Phone(PhoneCommand::KeepAudioOnPhone));
            }
        }
        PhoneCommand::HangUp(call) => {
            if let Some(known) = state.call_mut(call)
                && matches!(known.request, Request::HangUp | Request::Decline)
            {
                known.request = Request::None;
            }
            placing_hang_up_failed(state);
            effects.push(Effect::ReportError(Failure::HangUpFailed));
        }
        PhoneCommand::MoveAudioToDesktop => ask_once_more(state, effects),
        PhoneCommand::MoveAudioToPhone => audio_not_moved_to_phone(state, effects),
        PhoneCommand::ConnectHandsFree => hands_free_connect_failed(state, effects),
        // Without these the Phone never dialled.
        PhoneCommand::AllowAudioOnDesktop | PhoneCommand::Dial(_) => {
            dial_failed(state, Failure::DialFailed, effects);
        }
        // The shell has logged it; the next Call end or connect tries again.
        // The Phone is there all the same, so it shows as connected.
        PhoneCommand::KeepAudioOnPhone => {
            if state.keeping_audio_on_phone {
                state.keeping_audio_on_phone = false;
                state.connection = ConnectionState::Connected;
            }
        }
        PhoneCommand::SendTones(_) => effects.push(Effect::ReportError(Failure::TonesNotSent)),
        PhoneCommand::HoldAndAnswer(_)
        | PhoneCommand::EndAndAnswer(_)
        | PhoneCommand::AnswerBesideHeld(_)
        | PhoneCommand::Swap => {
            call_waiting_failed(state, command, effects);
        }
        PhoneCommand::SetMicrophoneVolume(0) => {
            if let Muting::Silencing { .. } = state.muting {
                state.muting = Muting::Off;
                effects.push(Effect::ReportError(Failure::MuteFailed));
            }
        }
        PhoneCommand::SetMicrophoneVolume(_) => {
            // Unmuting the user asked for failed: the microphone is still
            // silent, so say so. An unmute because the audio left the
            // Desktop or the Call ended has no Mute left to show.
            if let Muting::Restoring { restore } = state.muting
                && state.audio_location() == Some(AudioLocation::Desktop)
            {
                state.muting = Muting::On { restore };
                effects.push(Effect::ReportError(Failure::UnmuteFailed));
            }
        }
    }
}
