//! Calls Off: the user takes the Desktop out of their Calls, and the
//! Desktop keeps the Phone disconnected until they turn calls on again
//! (ADR-0008).
//!
//! Turning calls off keeps a record for the next run, then has BlueZ block
//! the Phone, which drops it at once and refuses its own connections. BlueZ doesn't refuse a connect from the Desktop to a blocked
//! Phone: it pages it and fails slowly. So while calls are off, or until the
//! Phone is unblocked again, nothing connects it. Turning calls on unblocks
//! the Phone, then connects it at once, with the reconnect waits of a Phone
//! that just came back.

use super::State;
use super::input::{Effect, Input};
use super::presence::connect_again_at_once;
use crate::messages::Failure;

/// Whether calls are on or off, and the Phone the Desktop blocked for it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum CallsOff {
    #[default]
    On,
    /// Off until the user turns them on: the Desktop has blocked, or is
    /// blocking, the Phone with `address`.
    Off { address: String },
    /// Off, as an earlier run kept it: the Desktop is blocking the Phone
    /// with `address` again, in case it was unblocked meanwhile. If that
    /// fails, calls stay off: the Phone may well still be blocked, and a
    /// connect would only hang.
    Reblocking { address: String },
    /// Turning on: the Desktop is unblocking the Phone with `address`, and
    /// connects it once that is done.
    TurningOn { address: String },
}

/// What the Desktop keeps of Calls Off for the next run, written before it
/// blocks or unblocks the Phone, so a run that stops halfway leaves enough
/// to put it right at the next start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallsOffRecord {
    /// Calls are off, and the Desktop blocked the Phone with `address`.
    Off { address: String },
    /// Calls are on, and the Desktop is unblocking the Phone with `address`.
    TurningOn { address: String },
}

impl State {
    /// Whether calls are off: the Desktop keeps the Phone disconnected
    /// until the user turns them on.
    #[must_use]
    pub fn calls_off(&self) -> bool {
        matches!(
            self.calls_off,
            CallsOff::Off { .. } | CallsOff::Reblocking { .. }
        )
    }

    /// Whether calls are being turned on: the Phone isn't unblocked yet.
    pub(crate) fn calls_turning_on(&self) -> bool {
        matches!(self.calls_off, CallsOff::TurningOn { .. })
    }

    /// Whether calls are on and the Phone isn't blocked by the Desktop, so
    /// it may be connected.
    pub(super) fn calls_on(&self) -> bool {
        self.calls_off == CallsOff::On
    }

    /// Whether a Call keeps calls from being turned off: one rings, is
    /// live, held, waiting or being placed, or the Desktop lost contact
    /// with it. Until a turn-off can wait for the last Call (ticket #204),
    /// it is refused meanwhile.
    pub(crate) fn call_keeps_calls_on(&self) -> bool {
        !self.calls.is_empty() || self.dialling.is_some() || self.connection_lost.is_some()
    }

    /// Why calls can't be turned off now, if they can't.
    pub(super) fn calls_off_refusal(&self) -> Option<Failure> {
        if self.calls_off() {
            None
        } else if self.phone.is_none() {
            Some(Failure::PhoneNotSetUp)
        } else if self.call_keeps_calls_on() {
            Some(Failure::CallsOffDuringCall)
        } else {
            None
        }
    }
}

/// What the user and BlueZ report about Calls Off.
pub(super) fn calls_off_input(state: &mut State, input: Input, effects: &mut Vec<Effect>) {
    match input {
        Input::CallsOffKept(record) => kept(state, record, effects),
        Input::TurnCallsOff => turn_off(state, effects),
        Input::TurnCallsOn => turn_on(state, effects),
        Input::PhoneBlocked { address, blocked } => {
            blocked_as_asked(state, &address, blocked, effects);
        }
        Input::PhoneBlockFailed { address, blocked } => {
            block_failed(state, &address, blocked, effects);
        }
        _ => {}
    }
}

/// What an earlier run kept, at startup: calls stay off, blocking the Phone
/// again in case it was unblocked meanwhile; or a turn-on the earlier run
/// didn't finish is finished. A Phone blocked with no record is the user's
/// own setting, which the Desktop never touches.
fn kept(state: &mut State, record: CallsOffRecord, effects: &mut Vec<Effect>) {
    match record {
        CallsOffRecord::Off { address } => {
            block(&address, true, effects);
            state.calls_off = CallsOff::Reblocking { address };
        }
        CallsOffRecord::TurningOn { address } => {
            block(&address, false, effects);
            state.calls_off = CallsOff::TurningOn { address };
        }
    }
}

/// The user turned calls off: the record is kept first, then the Phone is
/// blocked, which drops it. While calls are being turned on it waits: the
/// unblock is already on its way.
fn turn_off(state: &mut State, effects: &mut Vec<Effect>) {
    if state.calls_off_refusal().is_some() || !state.calls_on() {
        return;
    }
    let Some(phone) = &state.phone else {
        return;
    };
    let address = phone.address.clone();
    effects.push(Effect::KeepCallsOff(Some(CallsOffRecord::Off {
        address: address.clone(),
    })));
    block(&address, true, effects);
    state.calls_off = CallsOff::Off { address };
}

/// The user turned calls on: the record says so first, then the Phone is
/// unblocked, and connected once it is.
fn turn_on(state: &mut State, effects: &mut Vec<Effect>) {
    let (CallsOff::Off { address } | CallsOff::Reblocking { address }) = &state.calls_off else {
        return;
    };
    let address = address.clone();
    effects.push(Effect::KeepCallsOff(Some(CallsOffRecord::TurningOn {
        address: address.clone(),
    })));
    block(&address, false, effects);
    state.calls_off = CallsOff::TurningOn { address };
}

/// BlueZ blocked or unblocked the Phone as asked. Once unblocked for
/// calls on, the record goes and the Phone is connected at once.
fn blocked_as_asked(state: &mut State, address: &str, blocked: bool, effects: &mut Vec<Effect>) {
    if blocked {
        if matches!(&state.calls_off, CallsOff::Reblocking { address: off } if off == address) {
            state.calls_off = CallsOff::Off {
                address: address.to_owned(),
            };
        }
        return;
    }
    if !turning_on(state, address) {
        return;
    }
    state.calls_off = CallsOff::On;
    effects.push(Effect::KeepCallsOff(None));
    connect_again_at_once(state, effects);
}

/// BlueZ couldn't block or unblock the Phone. Calls stay as they were
/// before the user asked, and the user is told. Calls an earlier run kept
/// off stay off.
fn block_failed(state: &mut State, address: &str, blocked: bool, effects: &mut Vec<Effect>) {
    if blocked
        && matches!(&state.calls_off, CallsOff::Reblocking { address: off } if off == address)
    {
        state.calls_off = CallsOff::Off {
            address: address.to_owned(),
        };
    } else if blocked {
        let off_for = matches!(&state.calls_off, CallsOff::Off { address: off } if off == address);
        if !off_for {
            return;
        }
        state.calls_off = CallsOff::On;
        effects.push(Effect::KeepCallsOff(None));
        effects.push(Effect::ReportError(Failure::CallsOffFailed));
        connect_again_at_once(state, effects);
    } else if turning_on(state, address) {
        effects.push(Effect::KeepCallsOff(Some(CallsOffRecord::Off {
            address: address.to_owned(),
        })));
        state.calls_off = CallsOff::Off {
            address: address.to_owned(),
        };
        effects.push(Effect::ReportError(Failure::CallsOnFailed));
    }
}

/// The Desktop stops following the Phone, unpaired or switched away from
/// in the Setup Window: calls are on again, so the next Phone can connect,
/// and the Phone the Desktop blocked is unblocked, if BlueZ still has it.
pub(super) fn phone_forgotten(state: &State, effects: &mut Vec<Effect>) {
    let (CallsOff::Off { address }
    | CallsOff::Reblocking { address }
    | CallsOff::TurningOn { address }) = &state.calls_off
    else {
        return;
    };
    effects.push(Effect::KeepCallsOff(None));
    block(address, false, effects);
}

fn turning_on(state: &State, address: &str) -> bool {
    matches!(&state.calls_off, CallsOff::TurningOn { address: turning } if turning == address)
}

fn block(address: &str, blocked: bool, effects: &mut Vec<Effect>) {
    effects.push(Effect::BlockPhone {
        address: address.to_owned(),
        blocked,
    });
}

/// While calls are off, the Setup Window doesn't ask a Phone for
/// hands-free: that is a `ConnectProfile`, which a blocked Phone answers
/// as slowly as a connect. Pairing while calls are off waits for the Setup
/// Window to say calls are off (spec #198).
pub(super) fn hold_back_connects(state: &State, effects: &mut Vec<Effect>) {
    if !state.calls_on() {
        effects.retain(|effect| !matches!(effect, Effect::ConnectHandsFree { .. }));
    }
}
