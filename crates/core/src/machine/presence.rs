//! Whether the Phone is within reach: connecting it again while it is away,
//! suspend and resume, the pairing lost, and a Call that loses the Bluetooth
//! connection to the Phone.

use super::audio::end_handoff_to_phone;
use super::input::{ConnectFailure, Effect, Input, Timer};
use super::{CONNECTION_LOST_MS, ConnectionLost, LOST_CALL_BACK_MS, RECONNECT_WAITS_MS, State};
use crate::call::{CallId, Transport};
use crate::connection::ConnectionState;
use crate::earlier_run::EarlierCall;

/// Whether the Phone is within reach, and what the Desktop does about it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Presence {
    /// BlueZ reported the Phone's Bluetooth connection down, and not up
    /// again since.
    pub(super) absent: bool,
    /// The Desktop is suspending, or suspended.
    suspended: bool,
    /// The Phone answered a connect, and no longer has the Desktop paired.
    pub(super) pairing_lost: bool,
    reconnect: Reconnect,
}

/// How the Desktop is connecting the Phone again while it is away.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Reconnect {
    /// BlueZ is connecting the Phone now.
    connecting: bool,
    /// Connects that failed since the Phone was last present: the longer
    /// it has been away, the longer the wait before the next, up to the
    /// last of [`RECONNECT_WAITS_MS`].
    failures: u32,
    /// Which wait is current, so only its timer counts.
    wait: u32,
    /// Connect again as soon as the connect under way ends: the Desktop
    /// resumed meanwhile.
    at_once: bool,
}

impl State {
    /// Whether the Desktop should be connecting the Phone: it is paired,
    /// away, the Desktop is awake, and the Phone still has it paired.
    fn wants_phone(&self) -> bool {
        let presence = &self.presence;
        self.phone.is_some() && presence.absent && !presence.suspended && !presence.pairing_lost
    }
}

/// What BlueZ and logind report about reaching the Phone.
pub(super) fn presence_input(state: &mut State, input: Input, effects: &mut Vec<Effect>) {
    match input {
        Input::PhonePresent { address } if state.is_our_phone(&address) => phone_present(state),
        Input::PhoneAbsent { address } if state.is_our_phone(&address) => {
            phone_absent(state, effects);
        }
        Input::ConnectPhoneDone => connect_done(state, effects),
        Input::ConnectPhoneFailed(failure) => connect_failed(state, failure, effects),
        Input::Suspending => suspending(state),
        Input::Resumed => resumed(state, effects),
        _ => {}
    }
}

/// The state once the Phone is unpaired: nothing of it is left, and the
/// Desktop is still asleep or awake.
pub(super) fn forget_phone(state: &State) -> State {
    State {
        presence: Presence {
            suspended: state.presence.suspended,
            ..Presence::default()
        },
        ..State::default()
    }
}

/// The Phone has a Bluetooth connection to the Desktop again, or still.
fn phone_present(state: &mut State) {
    state.presence.absent = false;
    state.presence.reconnect.failures = 0;
    // Any wait under way is over.
    state.presence.reconnect.wait += 1;
    state.presence.reconnect.at_once = false;
    pairing_found(state);
}

/// The Phone connected after all, so it has the Desktop paired.
pub(super) fn pairing_found(state: &mut State) {
    state.presence.pairing_lost = false;
    if state.connection == ConnectionState::PairingLost {
        state.connection = ConnectionState::Connecting;
    }
}

/// The Phone's Bluetooth connection to the Desktop is gone: out of range,
/// or its Bluetooth is off. A connected Call carries on on the Phone, and
/// the Call Window says so. The Desktop starts connecting it again at once.
fn phone_absent(state: &mut State, effects: &mut Vec<Effect>) {
    let was_absent = state.presence.absent;
    state.presence.absent = true;
    if !was_absent {
        lose_calls(state, effects);
    }
    connect_now(state, effects);
}

/// The Calls can't be acted on from here any more. The live one in the
/// Call Window carries on on the Phone: the window says so for a while.
fn lose_calls(state: &mut State, effects: &mut Vec<Effect>) {
    // The audio stays with the Call on the Phone once the Phone is back.
    state.restore_audio = None;
    let Some(call) = state
        .calls
        .iter()
        .find(|call| call.state.is_live() && call.has_call_window())
        .cloned()
    else {
        return;
    };
    state.transport = Transport::Idle;
    state.moving_audio = None;
    end_handoff_to_phone(state);
    state.audio_notice = None;
    state.losses += 1;
    state.connection_lost = Some(ConnectionLost {
        call,
        loss: state.losses,
    });
    effects.push(Effect::StartTimer {
        timer: Timer::ConnectionLost(state.losses),
        after_ms: CONNECTION_LOST_MS,
    });
}

/// The Phone link gave up on `call`, which went with the connection to the
/// Phone. Nobody can say how it ended, or whether it did: it isn't a Missed
/// Call, nor "Call ended". A connected Call that comes back once the Phone
/// does keeps its direction and timer.
pub(super) fn lost_track(state: &mut State, call: CallId) {
    let Some(index) = state.calls.iter().position(|known| known.id == call) else {
        return;
    };
    let lost = state.calls.remove(index);
    if let Some(active_since) = lost.active_since {
        state.earlier_calls.push(EarlierCall {
            caller: lost.caller,
            direction: lost.direction,
            active_since,
        });
    }
    if state.unwanted == Some(call) {
        state.unwanted = None;
    }
    if state.calls.is_empty() {
        state.codec = None;
        state.moving_audio = None;
        end_handoff_to_phone(state);
        state.restore_audio = None;
        state.audio_notice = None;
    }
}

/// Hands-free is up again with the Call that lost the connection still to
/// come back, if it is still live: if the Phone doesn't report it soon, it
/// ended while the Phone was away.
pub(super) fn wait_for_lost_call(state: &mut State, effects: &mut Vec<Effect>) {
    let Some(lost) = &state.connection_lost else {
        return;
    };
    if state.calls.iter().any(|call| call.id == lost.call.id) {
        return;
    }
    state.losses += 1;
    let next = state.losses;
    if let Some(lost) = state.connection_lost.as_mut() {
        lost.loss = next;
    }
    effects.push(Effect::StartTimer {
        timer: Timer::ConnectionLost(next),
        after_ms: LOST_CALL_BACK_MS,
    });
}

/// A Call is back that may be the one that lost the connection: by its id
/// if the Phone link knew it again, or by its caller if it didn't.
pub(super) fn found_lost_call(state: &mut State, call: CallId) {
    let Some(lost) = &state.connection_lost else {
        return;
    };
    let same = lost.call.id == call
        || state
            .calls
            .iter()
            .find(|known| known.id == call)
            .is_some_and(|known| known.caller == lost.call.caller);
    if same {
        state.connection_lost = None;
    }
}

/// The Call Window has said for long enough that the connection was lost,
/// or the lost Call didn't come back with the Phone.
pub(super) fn connection_lost_over(state: &mut State, loss: u32) {
    if state
        .connection_lost
        .as_ref()
        .is_some_and(|lost| lost.loss == loss)
    {
        state.connection_lost = None;
    }
}

/// Asks BlueZ to connect the Phone now, if the Desktop wants it, or as soon
/// as the connect under way ends.
fn connect_now(state: &mut State, effects: &mut Vec<Effect>) {
    if !state.wants_phone() {
        return;
    }
    if state.presence.reconnect.connecting {
        state.presence.reconnect.at_once = true;
        return;
    }
    let Some(phone) = &state.phone else {
        return;
    };
    state.presence.reconnect.wait += 1;
    state.presence.reconnect.connecting = true;
    effects.push(Effect::ConnectPhone {
        address: phone.address.clone(),
    });
}

/// Waits a while, longer the more connects have failed, then connects.
fn connect_later(state: &mut State, effects: &mut Vec<Effect>) {
    if !state.wants_phone() {
        return;
    }
    let index = usize::try_from(state.presence.reconnect.failures.saturating_sub(1))
        .unwrap_or(usize::MAX)
        .min(RECONNECT_WAITS_MS.len() - 1);
    state.presence.reconnect.wait += 1;
    effects.push(Effect::StartTimer {
        timer: Timer::Reconnect(state.presence.reconnect.wait),
        after_ms: RECONNECT_WAITS_MS[index],
    });
}

/// BlueZ reports the connect done. The Phone shows as present by itself;
/// if it doesn't, the Desktop tries again shortly.
fn connect_done(state: &mut State, effects: &mut Vec<Effect>) {
    state.presence.reconnect.connecting = false;
    state.presence.reconnect.at_once = false;
    pairing_found(state);
    connect_later(state, effects);
}

/// BlueZ couldn't connect the Phone. One that didn't answer is tried again
/// after a wait; one that no longer has the Desktop paired isn't, as each
/// try would ask the person holding it to pair.
fn connect_failed(state: &mut State, failure: ConnectFailure, effects: &mut Vec<Effect>) {
    state.presence.reconnect.connecting = false;
    match failure {
        ConnectFailure::PairingLost => {
            state.presence.reconnect.at_once = false;
            if state.phone.is_some() && state.presence.absent {
                state.presence.pairing_lost = true;
                state.connection = ConnectionState::PairingLost;
            }
        }
        ConnectFailure::Unreachable => {
            state.presence.reconnect.failures = state.presence.reconnect.failures.saturating_add(1);
            if std::mem::take(&mut state.presence.reconnect.at_once) {
                connect_now(state, effects);
            } else {
                connect_later(state, effects);
            }
        }
    }
}

/// A wait between connects is over.
pub(super) fn reconnect_due(state: &mut State, wait: u32, effects: &mut Vec<Effect>) {
    if wait == state.presence.reconnect.wait {
        connect_now(state, effects);
    }
}

/// The Desktop is about to suspend: no connecting until it resumes.
fn suspending(state: &mut State) {
    state.presence.suspended = true;
    state.presence.reconnect.wait += 1;
    state.presence.reconnect.at_once = false;
}

/// The Desktop resumed: the Phone is connected again at once, without the
/// waits of before.
fn resumed(state: &mut State, effects: &mut Vec<Effect>) {
    state.presence.suspended = false;
    state.presence.reconnect.failures = 0;
    connect_now(state, effects);
}
