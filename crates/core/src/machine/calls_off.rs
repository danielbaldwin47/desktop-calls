//! Calls Off: the user takes the Desktop out of their Calls, and the
//! Desktop keeps the Phone disconnected until they turn calls on again
//! (ADR-0008).
//!
//! Turning calls off keeps a record for the next run, then has BlueZ block
//! the Phone, which drops it at once and refuses its own connections.
//! BlueZ doesn't refuse a connect from the Desktop to a blocked Phone: it
//! pages it and fails slowly. So while calls are off, or until the
//! Phone is unblocked again, nothing connects it. Turning calls on unblocks
//! the Phone, then connects it at once, with the reconnect waits of a Phone
//! that just came back.
//!
//! Calls turned off for an hour, or until the morning, come back on by
//! themselves at their deadline: an instant by the wall clock, which the
//! shell reports on the minute, at the deadline and on resume. A deadline
//! that passed while the app was stopped or the Desktop suspended takes
//! effect at the next start or resume.

use super::State;
use super::input::{Effect, Input};
use super::presence::connect_again_at_once;
use crate::call::Timestamp;
use crate::clock::{Deadline, WallClock};
use crate::messages::Failure;

/// Whether calls are on or off, and the Phone the Desktop blocked for it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum CallsOff {
    #[default]
    On,
    /// Off until `until`, or until the user turns them on: the Desktop has
    /// blocked, or is blocking, the Phone with `address`.
    Off {
        address: String,
        until: Option<Deadline>,
    },
    /// Off, as an earlier run kept it: the Desktop is blocking the Phone
    /// with `address` again, in case it was unblocked meanwhile. If that
    /// fails, calls stay off: the Phone may well still be blocked, and a
    /// connect would only hang.
    Reblocking {
        address: String,
        until: Option<Deadline>,
    },
    /// Turning on: the Desktop is unblocking the Phone with `address`, and
    /// connects it once that is done. If BlueZ won't, calls stay off as
    /// they were: until `until`, or until turned on.
    TurningOn {
        address: String,
        until: Option<Deadline>,
    },
}

/// What the Desktop keeps of Calls Off for the next run, written before it
/// blocks or unblocks the Phone, so a run that stops halfway leaves enough
/// to put it right at the next start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallsOffRecord {
    /// Calls are off until `until`, or until the user turns them on, and
    /// the Desktop blocked the Phone with `address`.
    Off {
        address: String,
        until: Option<Deadline>,
    },
    /// Calls are on, and the Desktop is unblocking the Phone with `address`.
    TurningOn { address: String },
}

impl State {
    /// Whether calls are off: the Desktop keeps the Phone disconnected
    /// until the user turns them on, or their deadline passes.
    #[must_use]
    pub fn calls_off(&self) -> bool {
        matches!(
            self.calls_off,
            CallsOff::Off { .. } | CallsOff::Reblocking { .. }
        )
    }

    /// When calls come back on by themselves, while they are off until a
    /// deadline.
    #[must_use]
    pub fn calls_off_until(&self) -> Option<Deadline> {
        match &self.calls_off {
            CallsOff::Off { until, .. } | CallsOff::Reblocking { until, .. } => *until,
            CallsOff::On | CallsOff::TurningOn { .. } => None,
        }
    }

    /// The wall clock as last reported, once it has been.
    #[must_use]
    pub fn wall_clock(&self) -> Option<WallClock> {
        self.clock
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

    /// Why calls can't be turned off now, until `until`, if they can't.
    pub(super) fn calls_off_refusal(&self, until: Option<Deadline>) -> Option<Failure> {
        let passed = until.is_some_and(|until| self.passed(until.at));
        if self.calls_off() {
            passed.then_some(Failure::CallsOffUntilPassed)
        } else if self.phone.is_none() {
            Some(Failure::PhoneNotSetUp)
        } else if self.call_keeps_calls_on() {
            Some(Failure::CallsOffDuringCall)
        } else if passed {
            Some(Failure::CallsOffUntilPassed)
        } else {
            None
        }
    }

    /// Whether `at` has passed, by the wall clock as last reported.
    fn passed(&self, at: Timestamp) -> bool {
        self.clock.is_some_and(|clock| at <= clock.now)
    }
}

/// What the user, the wall clock and BlueZ report about Calls Off.
pub(super) fn calls_off_input(state: &mut State, input: Input, effects: &mut Vec<Effect>) {
    match input {
        Input::CallsOffKept(record) => kept(state, record, effects),
        Input::TurnCallsOff { until } => turn_off(state, until, effects),
        Input::TurnCallsOn => turn_on(state, effects),
        Input::ClockTicked(clock) => clock_ticked(state, clock, effects),
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
/// again in case it was unblocked meanwhile, unless their deadline passed
/// while the app was stopped; or a turn-on the earlier run didn't finish is
/// finished. A Phone blocked with no record is the user's own setting,
/// which the Desktop never touches.
fn kept(state: &mut State, record: CallsOffRecord, effects: &mut Vec<Effect>) {
    match record {
        CallsOffRecord::Off { address, until } => {
            state.calls_off = CallsOff::Reblocking {
                address: address.clone(),
                until,
            };
            if until.is_some_and(|until| state.passed(until.at)) {
                deadline_passed(state, effects);
                return;
            }
            block(&address, true, effects);
            read_clock_at(until, effects);
        }
        CallsOffRecord::TurningOn { address } => {
            block(&address, false, effects);
            state.calls_off = CallsOff::TurningOn {
                address,
                until: None,
            };
        }
    }
}

/// The wall clock, as the shell read it. Calls off until a deadline that
/// has passed come back on. While the Desktop suspends it isn't heeded: a
/// deadline then waits for the resume, rather than connecting the Phone as
/// the Desktop goes to sleep.
fn clock_ticked(state: &mut State, clock: WallClock, effects: &mut Vec<Effect>) {
    if state.suspended() {
        return;
    }
    state.clock = Some(clock);
    if state
        .calls_off_until()
        .is_some_and(|until| until.at <= clock.now)
    {
        deadline_passed(state, effects);
    }
}

/// Calls were off until a deadline, which has passed: they come back on.
fn deadline_passed(state: &mut State, effects: &mut Vec<Effect>) {
    if let CallsOff::Off { address, .. } | CallsOff::Reblocking { address, .. } = &state.calls_off {
        effects.push(Effect::CallsOffDeadlinePassed {
            address: address.clone(),
        });
    }
    turn_on(state, effects);
}

/// Asks the shell to report the wall clock at the deadline, if there is one.
fn read_clock_at(until: Option<Deadline>, effects: &mut Vec<Effect>) {
    if let Some(until) = until {
        effects.push(Effect::ReadClockAt(until.at));
    }
}

/// The user turned calls off, until `until` or until they turn them on:
/// the record is kept first, then the Phone is blocked, which drops it.
/// While calls are being turned on it waits: the unblock is already on its
/// way. Turned off again while off, calls stay off until the new choice.
fn turn_off(state: &mut State, until: Option<Deadline>, effects: &mut Vec<Effect>) {
    if state.calls_off_refusal(until).is_some() {
        return;
    }
    if let CallsOff::Off {
        address,
        until: old,
    } = &mut state.calls_off
    {
        if *old != until {
            *old = until;
            effects.push(Effect::KeepCallsOff(Some(CallsOffRecord::Off {
                address: address.clone(),
                until,
            })));
            read_clock_at(until, effects);
        }
        return;
    }
    if !state.calls_on() {
        return;
    }
    let Some(phone) = &state.phone else {
        return;
    };
    let address = phone.address.clone();
    effects.push(Effect::KeepCallsOff(Some(CallsOffRecord::Off {
        address: address.clone(),
        until,
    })));
    block(&address, true, effects);
    read_clock_at(until, effects);
    state.calls_off = CallsOff::Off { address, until };
}

/// Calls are turned on, by the user or at their deadline: the record says
/// so first, then the Phone is unblocked, and connected once it is.
fn turn_on(state: &mut State, effects: &mut Vec<Effect>) {
    let (CallsOff::Off { address, until } | CallsOff::Reblocking { address, until }) =
        &state.calls_off
    else {
        return;
    };
    let (address, until) = (address.clone(), *until);
    effects.push(Effect::KeepCallsOff(Some(CallsOffRecord::TurningOn {
        address: address.clone(),
    })));
    block(&address, false, effects);
    state.calls_off = CallsOff::TurningOn { address, until };
}

/// BlueZ blocked or unblocked the Phone as asked. Once unblocked for
/// calls on, the record goes and the Phone is connected at once.
fn blocked_as_asked(state: &mut State, address: &str, blocked: bool, effects: &mut Vec<Effect>) {
    if blocked {
        reblocked(state, address);
        return;
    }
    if !turning_on(state, address) {
        return;
    }
    state.calls_off = CallsOff::On;
    effects.push(Effect::KeepCallsOff(None));
    connect_again_at_once(state, effects);
}

/// Calls an earlier run kept off are off, with the Phone blocked again or
/// left as it was.
fn reblocked(state: &mut State, address: &str) {
    if let CallsOff::Reblocking {
        address: off,
        until,
    } = &state.calls_off
        && off == address
    {
        state.calls_off = CallsOff::Off {
            address: off.clone(),
            until: *until,
        };
    }
}

/// BlueZ couldn't block or unblock the Phone. Calls stay as they were
/// before the user asked, and the user is told. Calls an earlier run kept
/// off stay off. Calls left off past their deadline come back on at the
/// clock's next report, which tries again.
fn block_failed(state: &mut State, address: &str, blocked: bool, effects: &mut Vec<Effect>) {
    if blocked
        && matches!(&state.calls_off, CallsOff::Reblocking { address: off, .. } if off == address)
    {
        reblocked(state, address);
    } else if blocked {
        let off_for =
            matches!(&state.calls_off, CallsOff::Off { address: off, .. } if off == address);
        if !off_for {
            return;
        }
        state.calls_off = CallsOff::On;
        effects.push(Effect::KeepCallsOff(None));
        effects.push(Effect::ReportError(Failure::CallsOffFailed));
        connect_again_at_once(state, effects);
    } else if let CallsOff::TurningOn {
        address: turning,
        until,
    } = &state.calls_off
        && turning == address
    {
        let until = *until;
        effects.push(Effect::KeepCallsOff(Some(CallsOffRecord::Off {
            address: address.to_owned(),
            until,
        })));
        state.calls_off = CallsOff::Off {
            address: address.to_owned(),
            until,
        };
        effects.push(Effect::ReportError(Failure::CallsOnFailed));
    }
}

/// The Desktop stops following the Phone, unpaired or switched away from
/// in the Setup Window: calls are on again, so the next Phone can connect,
/// and the Phone the Desktop blocked is unblocked, if BlueZ still has it.
pub(super) fn phone_forgotten(state: &State, effects: &mut Vec<Effect>) {
    let (CallsOff::Off { address, .. }
    | CallsOff::Reblocking { address, .. }
    | CallsOff::TurningOn { address, .. }) = &state.calls_off
    else {
        return;
    };
    effects.push(Effect::KeepCallsOff(None));
    block(address, false, effects);
}

fn turning_on(state: &State, address: &str) -> bool {
    matches!(&state.calls_off, CallsOff::TurningOn { address: turning, .. } if turning == address)
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
