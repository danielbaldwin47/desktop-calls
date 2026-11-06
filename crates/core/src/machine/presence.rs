//! Whether the Phone is within reach: connecting it again while it is away,
//! or while hands-free hasn't come back after a Handoff to the Phone,
//! suspend and resume, the pairing lost, and a Call that loses the
//! connection to the Phone either way.

use super::audio::end_handoff_to_phone;
use super::input::{ConnectFailure, Effect, Input, PhoneCommand, Timer};
use super::{CONNECTION_LOST_MS, ConnectionLost, LOST_CALL_BACK_MS, RECONNECT_WAITS_MS, State};
use crate::call::{CallId, Request, Transport};
use crate::connection::ConnectionState;
use crate::earlier_run::EarlierCall;
use crate::messages::Failure;

/// Whether the Phone is within reach, and what the Desktop does about it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Presence {
    reach: Reach,
    /// The Desktop is suspending, or suspended.
    suspended: bool,
    /// The Phone answered a connect, and no longer has the Desktop paired.
    pub(super) pairing_lost: bool,
    reconnect: Reconnect,
}

/// How much of the Phone the Desktop reaches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Reach {
    /// The Phone has a Bluetooth connection to the Desktop, as far as BlueZ
    /// has said.
    #[default]
    Present,
    /// BlueZ reported the Phone's Bluetooth connection down, and not up
    /// again since.
    Absent,
    /// The Phone is present, and hands-free dropped for a Handoff to the
    /// Phone and didn't come back: the Desktop connects hands-free again
    /// (ticket #101).
    HandsFreeLost,
}

impl Presence {
    /// BlueZ reported the Phone's Bluetooth connection down, and not up
    /// again since.
    pub(super) fn absent(&self) -> bool {
        self.reach == Reach::Absent
    }
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
    /// away or without hands-free, the Desktop is awake, and the Phone still
    /// has it paired.
    fn wants_phone(&self) -> bool {
        let presence = &self.presence;
        self.phone.is_some()
            && presence.reach != Reach::Present
            && !presence.suspended
            && !presence.pairing_lost
    }

    /// Whether the Calls are out of the Desktop's reach: the Phone is away,
    /// or hands-free didn't come back after a Handoff to the Phone. The Phone
    /// link giving up on a Call then says nothing about how it ended.
    pub(super) fn calls_out_of_reach(&self) -> bool {
        self.presence.reach != Reach::Present
    }

    /// Whether the Phone is present, and hands-free didn't come back after
    /// a Handoff to the Phone: not out of range, though not connected.
    pub(crate) fn hands_free_lost(&self) -> bool {
        self.presence.reach == Reach::HandsFreeLost
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
    if state.presence.absent() {
        state.presence.reach = Reach::Present;
    }
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
    let was_absent = state.presence.absent();
    // Connecting the Phone brings hands-free back with it.
    state.presence.reach = Reach::Absent;
    if !was_absent {
        lose_calls(state, Failure::ConnectionLost, effects);
    }
    connect_now(state, effects);
}

/// Hands-free dropped for a Handoff to the Phone and isn't coming back by
/// itself, while the Phone is still present. The Call went to the Phone
/// with the audio and carries on there, out of the Desktop's reach, as for
/// a Phone out of range. The drop's own `ConnectProfile` counts as the
/// first failed connect, so the Desktop asks BlueZ again after the first
/// wait.
pub(super) fn hands_free_lost(state: &mut State, effects: &mut Vec<Effect>) {
    lose_calls(state, Failure::ConnectionLost, effects);
    if state.presence.absent() {
        return;
    }
    state.presence.reach = Reach::HandsFreeLost;
    state.presence.reconnect.failures = 1;
    if !state.presence.reconnect.connecting {
        connect_later(state, effects);
    }
}

/// Hands-free is up: any connect it needed is over.
pub(super) fn hands_free_back(state: &mut State) {
    if state.presence.reach == Reach::HandsFreeLost {
        state.presence.reach = Reach::Present;
        state.presence.reconnect.failures = 0;
        state.presence.reconnect.wait += 1;
        state.presence.reconnect.at_once = false;
    }
}

/// BlueZ connected hands-free as the Desktop asked: it shows up by itself.
pub(super) fn hands_free_connect_done(state: &mut State, effects: &mut Vec<Effect>) {
    connect_done(state, effects);
}

/// BlueZ couldn't connect hands-free: the Desktop asks again after a wait.
pub(super) fn hands_free_connect_failed(state: &mut State, effects: &mut Vec<Effect>) {
    connect_failed(state, ConnectFailure::Unreachable, effects);
}

/// The Calls can't be acted on from here any more. The live one in the
/// Call Window, and the one on hold behind it, carry on on the Phone: the
/// window says so for a while, with `notice`.
fn lose_calls(state: &mut State, notice: Failure, effects: &mut Vec<Effect>) {
    // The audio stays with the Call on the Phone once the Phone is back.
    state.restore_audio = None;
    let Some((call, held)) = state
        .call_window_calls()
        .filter(|(call, _)| call.state.is_live())
        .map(|(call, held)| (call.clone(), held.cloned()))
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
        held,
        notice,
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

/// The Phone link gave up on `call` while the Phone was away, before the
/// Hang up the user asked for reached the Phone: it may carry on there. The
/// user is told the Phone didn't hang it up, and the Call Window shows it as
/// lost for a while, saying so. A Call that comes back with the Phone keeps
/// its direction and timer, and isn't hung up by itself: by then the user
/// may have picked it up on the Phone.
pub(super) fn hang_up_undelivered(state: &mut State, call: CallId, effects: &mut Vec<Effect>) {
    let Some(known) = state.call_mut(call) else {
        return;
    };
    known.request = Request::None;
    effects.push(Effect::ReportError(Failure::HangUpFailed));
    let losses = state.losses + 1;
    match state.connection_lost.as_mut() {
        // Already lost: it says so now, for as long as a fresh loss would.
        Some(lost)
            if lost.call.id == call || lost.held.as_ref().is_some_and(|held| held.id == call) =>
        {
            lost.notice = Failure::HangUpFailed;
            lost.loss = losses;
            state.losses = losses;
            effects.push(Effect::StartTimer {
                timer: Timer::ConnectionLost(losses),
                after_ms: CONNECTION_LOST_MS,
            });
        }
        Some(_) => {}
        None => {
            let shown = state.call_window_calls().is_some_and(|(shown, held)| {
                shown.id == call || held.is_some_and(|held| held.id == call)
            });
            if shown {
                lose_calls(state, Failure::HangUpFailed, effects);
            }
        }
    }
    lost_track(state, call);
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
/// as the connect under way ends. A Phone that is present only needs
/// hands-free.
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
    if state.presence.absent() {
        effects.push(Effect::ConnectPhone {
            address: phone.address.clone(),
        });
    } else {
        effects.push(Effect::Phone(PhoneCommand::ConnectHandsFree));
    }
}

/// Asks BlueZ to connect the Phone now, wherever it is: it was just
/// paired. A connect already under way does instead.
pub(super) fn connect_phone_now(state: &mut State, effects: &mut Vec<Effect>) {
    let Some(phone) = &state.phone else {
        return;
    };
    if state.presence.reconnect.connecting {
        return;
    }
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
            if state.phone.is_some() && state.presence.absent() {
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
