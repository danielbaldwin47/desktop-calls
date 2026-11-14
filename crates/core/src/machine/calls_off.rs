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
//!
//! A live Call is never cut (ticket #204). Turned off while a Call rings,
//! is live, held, waiting or being placed, or carries on on the Phone after
//! the Desktop lost contact with it, calls stay on and the turn-off waits,
//! kept for the next run, until no Call is left. Keep calls on cancels it.
//! A timed choice keeps the deadline it was given: if the Call outlasts
//! it, calls simply stay on.
//!
//! Quiet Hours (ticket #205, in `quiet_hours`) turn calls off the same
//! way, until they end. Calls turned on, or kept on, during Quiet Hours
//! stay on until they next begin; a timed break that ends inside them
//! stays off until they end.

use super::input::{Effect, Input, Timer};
use super::presence::connect_again_at_once;
use super::{FORGET_EARLIER_RUN_MS, State};
use crate::call::Timestamp;
use crate::clock::{Deadline, WallClock};
use crate::messages::Failure;

/// Whether calls are on or off, and the Phone the Desktop blocked for it.
/// `quiet_hours` while the Quiet Hours turned them off, until `until`,
/// when they end.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum CallsOff {
    #[default]
    On,
    /// On, with a turn-off waiting for the last Call to end: then calls go
    /// off until `until`, or until the user turns them on, for the Phone
    /// with `address`. `after_restart` while an earlier run kept it and the
    /// Phone hasn't listed a Call again yet: a Call still ringing then
    /// isn't recorded, so it waits as long as the earlier run's Calls may
    /// take to show again.
    Pending {
        address: String,
        until: Option<Deadline>,
        after_restart: bool,
        quiet_hours: bool,
    },
    /// Off until `until`, or until the user turns them on: the Desktop has
    /// blocked, or is blocking, the Phone with `address`.
    Off {
        address: String,
        until: Option<Deadline>,
        quiet_hours: bool,
    },
    /// Off, as an earlier run kept it: the Desktop is blocking the Phone
    /// with `address` again, in case it was unblocked meanwhile. If that
    /// fails, calls stay off: the Phone may well still be blocked, and a
    /// connect would only hang.
    Reblocking {
        address: String,
        until: Option<Deadline>,
        quiet_hours: bool,
    },
    /// Turning on: the Desktop is unblocking the Phone with `address`, and
    /// connects it once that is done. If BlueZ won't, calls stay off as
    /// they were: until `until`, or until turned on.
    TurningOn {
        address: String,
        until: Option<Deadline>,
        quiet_hours: bool,
    },
}

/// What the Desktop keeps of Calls Off for the next run, written before it
/// blocks or unblocks the Phone, so a run that stops halfway leaves enough
/// to put it right at the next start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallsOffRecord {
    /// Calls are off until `until`, or until the user turns them on, and
    /// the Desktop blocked the Phone with `address`. `quiet_hours` while
    /// Quiet Hours turned them off, until they end.
    Off {
        address: String,
        until: Option<Deadline>,
        quiet_hours: bool,
    },
    /// Calls are on, and the Desktop is unblocking the Phone with `address`.
    /// Turned on during Quiet Hours, they stay on until `kept_on_until`,
    /// when those next begin.
    TurningOn {
        address: String,
        kept_on_until: Option<Deadline>,
    },
    /// Calls are on, and go off until `until`, or until the user turns them
    /// on, for the Phone with `address` once no Call is left. Nothing is
    /// blocked yet. `quiet_hours` while Quiet Hours turn them off.
    Pending {
        address: String,
        until: Option<Deadline>,
        quiet_hours: bool,
    },
    /// Calls are on through Quiet Hours the user turned them on, or kept
    /// them on, in: until `until`, when those next begin. Nothing is
    /// blocked.
    KeptOn { until: Deadline },
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

    /// Whether calls are off, or will be once a waiting turn-off is
    /// applied, because Quiet Hours are under way: until they end.
    #[must_use]
    pub fn calls_off_for_quiet_hours(&self) -> bool {
        matches!(
            self.calls_off,
            CallsOff::Off {
                quiet_hours: true,
                ..
            } | CallsOff::Reblocking {
                quiet_hours: true,
                ..
            } | CallsOff::Pending {
                quiet_hours: true,
                ..
            }
        )
    }

    /// Whether a turn-off waits for the last Call to end: calls are on
    /// until then.
    #[must_use]
    pub fn calls_off_pending(&self) -> bool {
        matches!(self.calls_off, CallsOff::Pending { .. })
    }

    /// Whether a turn-off asked for now waits for the last Call to end.
    #[must_use]
    pub fn calls_off_would_wait(&self) -> bool {
        self.calls_on() && self.call_keeps_calls_on()
    }

    /// When calls come back on by themselves, while they are off until a
    /// deadline, or will be once a waiting turn-off is applied.
    #[must_use]
    pub fn calls_off_until(&self) -> Option<Deadline> {
        match &self.calls_off {
            CallsOff::Off { until, .. }
            | CallsOff::Reblocking { until, .. }
            | CallsOff::Pending { until, .. } => *until,
            CallsOff::On | CallsOff::TurningOn { .. } => None,
        }
    }

    /// The wall clock as last reported, once it has been.
    #[must_use]
    pub fn wall_clock(&self) -> Option<WallClock> {
        self.clock
    }

    /// When "Until tomorrow morning" ends: when Quiet Hours next end, once
    /// the clock has read them, or else the next morning (spec #198).
    #[must_use]
    pub fn until_morning(&self) -> Option<Deadline> {
        self.quiet_hours_clock()
            .map(|quiet| quiet.next_end)
            .or_else(|| self.clock.map(|clock| clock.until_morning))
    }

    /// Whether calls are being turned on: the Phone isn't unblocked yet.
    pub(crate) fn calls_turning_on(&self) -> bool {
        matches!(self.calls_off, CallsOff::TurningOn { .. })
    }

    /// Whether calls are on and the Phone isn't blocked by the Desktop, so
    /// it may be connected.
    pub(super) fn calls_on(&self) -> bool {
        matches!(self.calls_off, CallsOff::On | CallsOff::Pending { .. })
    }

    /// Whether a Call keeps calls on for now: one rings, is live, held,
    /// waiting or being placed, or the Desktop lost contact with it and it
    /// carries on on the Phone. A turn-off meanwhile waits for it to end.
    fn call_keeps_calls_on(&self) -> bool {
        !self.calls.is_empty() || self.dialling.is_some() || self.connection_lost.is_some()
    }

    /// Why calls can't be turned off now, until `until`, if they can't.
    pub(super) fn calls_off_refusal(&self, until: Option<Deadline>) -> Option<Failure> {
        let passed = until.is_some_and(|until| self.passed(until.at));
        if self.calls_off() {
            passed.then_some(Failure::CallsOffUntilPassed)
        } else if self.phone.is_none() {
            Some(Failure::PhoneNotSetUp)
        } else if passed {
            Some(Failure::CallsOffUntilPassed)
        } else {
            None
        }
    }

    /// Whether `at` has passed, by the wall clock as last reported.
    pub(super) fn passed(&self, at: Timestamp) -> bool {
        self.clock.is_some_and(|clock| at <= clock.now)
    }

    /// What is kept for the next run while calls are on: that they stay
    /// on through Quiet Hours, if the user turned them on in some.
    pub(super) fn calls_on_record(&self) -> Option<CallsOffRecord> {
        self.kept_on_until
            .map(|until| CallsOffRecord::KeptOn { until })
    }
}

/// What the user, the wall clock and BlueZ report about Calls Off.
pub(super) fn calls_off_input(state: &mut State, input: Input, effects: &mut Vec<Effect>) {
    match input {
        Input::CallsOffKept(record) => kept(state, record, effects),
        Input::TurnCallsOff { until } => turn_off(state, until, false, effects),
        Input::TurnCallsOn => {
            keep_on_through_quiet_hours(state);
            keep_on(state, effects);
            turn_on(state, effects);
        }
        Input::KeepCallsOn => {
            keep_on_through_quiet_hours(state);
            keep_on(state, effects);
        }
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

/// The user turned calls on, or kept them on, while they are off or a
/// turn-off waits: during Quiet Hours, calls then stay on until those next
/// begin (spec #198).
fn keep_on_through_quiet_hours(state: &mut State) {
    let off_or_waiting = matches!(
        state.calls_off,
        CallsOff::Off { .. } | CallsOff::Reblocking { .. } | CallsOff::Pending { .. }
    );
    if !off_or_waiting {
        return;
    }
    if let Some(quiet) = state.quiet_hours_under_way() {
        state.kept_on_until = Some(quiet.next_start);
    }
}

/// What an earlier run kept, at startup: calls stay off, blocking the Phone
/// again in case it was unblocked meanwhile, unless their deadline passed
/// while the app was stopped; or a turn-on the earlier run didn't finish is
/// finished. A Phone blocked with no record is the user's own setting,
/// which the Desktop never touches.
fn kept(state: &mut State, record: CallsOffRecord, effects: &mut Vec<Effect>) {
    match record {
        CallsOffRecord::Off {
            address,
            until,
            quiet_hours,
        } => {
            state.calls_off = CallsOff::Reblocking {
                address: address.clone(),
                until,
                quiet_hours,
            };
            if until.is_some_and(|until| state.passed(until.at)) {
                deadline_passed(state, effects);
            }
            // Still off, perhaps on for Quiet Hours now under way.
            if let CallsOff::Reblocking { until, .. } = state.calls_off {
                block(&address, true, effects);
                read_clock_at(until, effects);
            }
        }
        CallsOffRecord::TurningOn {
            address,
            kept_on_until,
        } => {
            block(&address, false, effects);
            state.kept_on_until = kept_on_until;
            state.calls_off = CallsOff::TurningOn {
                address,
                until: None,
                quiet_hours: false,
            };
        }
        CallsOffRecord::Pending {
            address,
            until,
            quiet_hours,
        } => {
            if until.is_some_and(|until| state.passed(until.at)) {
                effects.push(Effect::KeepCallsOff(None));
                effects.push(Effect::CallsOffDeadlinePassed { address });
                return;
            }
            // Its Calls may take as long to show again as the earlier
            // run's: it waits as long, even with none recorded.
            if state.earlier_calls.is_empty() {
                effects.push(Effect::StartTimer {
                    timer: Timer::ForgetEarlierRun,
                    after_ms: FORGET_EARLIER_RUN_MS,
                });
            }
            read_clock_at(until, effects);
            state.calls_off = CallsOff::Pending {
                address,
                until,
                after_restart: true,
                quiet_hours,
            };
        }
        CallsOffRecord::KeptOn { until } => state.kept_on_until = Some(until),
    }
}

/// The earlier run's Calls had their time to show again: a turn-off it
/// kept waits only for the Calls that did.
pub(super) fn earlier_run_over(state: &mut State) {
    if let CallsOff::Pending { after_restart, .. } = &mut state.calls_off {
        *after_restart = false;
    }
}

/// The user chose Keep calls on: the turn-off waiting for the last Call to
/// end is cancelled.
fn keep_on(state: &mut State, effects: &mut Vec<Effect>) {
    if let CallsOff::Pending { address, .. } = &state.calls_off {
        effects.push(Effect::KeepCallsOff(state.calls_on_record()));
        effects.push(Effect::CallsKeptOn {
            address: address.clone(),
        });
        state.calls_off = CallsOff::On;
    }
}

/// After every input: a waiting turn-off whose deadline has passed is
/// dropped, so calls stay on; one with no Call left is applied.
pub(super) fn apply_pending(state: &mut State, effects: &mut Vec<Effect>) {
    let CallsOff::Pending {
        address,
        until,
        after_restart,
        quiet_hours,
    } = &mut state.calls_off
    else {
        return;
    };
    if *after_restart && !state.calls.is_empty() {
        *after_restart = false;
    }
    let (address, until, after_restart, quiet_hours) =
        (address.clone(), *until, *after_restart, *quiet_hours);
    if until.is_some_and(|until| state.passed(until.at)) {
        effects.push(Effect::KeepCallsOff(state.calls_on_record()));
        effects.push(Effect::CallsOffDeadlinePassed { address });
        state.calls_off = CallsOff::On;
        return;
    }
    if after_restart || state.call_keeps_calls_on() {
        return;
    }
    let Some(phone) = &state.phone else {
        return;
    };
    let address = phone.address.clone();
    turn_off_now(state, address, until, quiet_hours, effects);
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
    if state.calls_off()
        && state
            .calls_off_until()
            .is_some_and(|until| until.at <= clock.now)
    {
        deadline_passed(state, effects);
    }
}

/// Calls were off until a deadline, which has passed: they come back on.
/// A timed break that ends inside Quiet Hours stays off until they end
/// instead.
fn deadline_passed(state: &mut State, effects: &mut Vec<Effect>) {
    let quiet = state.quiet_hours_under_way();
    if let CallsOff::Off {
        address,
        until,
        quiet_hours,
    }
    | CallsOff::Reblocking {
        address,
        until,
        quiet_hours,
    } = &mut state.calls_off
    {
        if let Some(quiet) = quiet {
            *until = Some(quiet.next_end);
            *quiet_hours = true;
            effects.push(Effect::KeepCallsOff(Some(CallsOffRecord::Off {
                address: address.clone(),
                until: *until,
                quiet_hours: true,
            })));
            read_clock_at(*until, effects);
            return;
        }
        effects.push(Effect::CallsOffDeadlinePassed {
            address: address.clone(),
        });
    }
    turn_on(state, effects);
}

/// Asks the shell to report the wall clock at the deadline, if there is one.
pub(super) fn read_clock_at(until: Option<Deadline>, effects: &mut Vec<Effect>) {
    if let Some(until) = until {
        effects.push(Effect::ReadClockAt(until.at));
    }
}

/// Calls are turned off, by the user or, with `quiet_hours`, for Quiet
/// Hours: until `until` or until the user turns them on. The record is
/// kept first, then the Phone is blocked, which drops it. During a Call it
/// waits for the last Call to end. While calls are being turned on it
/// waits too: the unblock is already on its way. Turned off again while
/// off or waiting, calls go off until the new choice.
pub(super) fn turn_off(
    state: &mut State,
    until: Option<Deadline>,
    quiet_hours: bool,
    effects: &mut Vec<Effect>,
) {
    if state.calls_off_refusal(until).is_some() {
        return;
    }
    match &mut state.calls_off {
        CallsOff::Off {
            address,
            until: old,
            quiet_hours: old_quiet,
        } => {
            if *old != until || *old_quiet != quiet_hours {
                (*old, *old_quiet) = (until, quiet_hours);
                effects.push(Effect::KeepCallsOff(Some(CallsOffRecord::Off {
                    address: address.clone(),
                    until,
                    quiet_hours,
                })));
                read_clock_at(until, effects);
            }
            return;
        }
        CallsOff::Pending {
            address,
            until: old,
            quiet_hours: old_quiet,
            ..
        } => {
            if *old != until || *old_quiet != quiet_hours {
                (*old, *old_quiet) = (until, quiet_hours);
                effects.push(Effect::KeepCallsOff(Some(CallsOffRecord::Pending {
                    address: address.clone(),
                    until,
                    quiet_hours,
                })));
                read_clock_at(until, effects);
            }
            return;
        }
        CallsOff::On => {}
        CallsOff::Reblocking { .. } | CallsOff::TurningOn { .. } => return,
    }
    let Some(phone) = &state.phone else {
        return;
    };
    let address = phone.address.clone();
    state.kept_on_until = None;
    if !state.call_keeps_calls_on() {
        turn_off_now(state, address, until, quiet_hours, effects);
        return;
    }
    effects.push(Effect::KeepCallsOff(Some(CallsOffRecord::Pending {
        address: address.clone(),
        until,
        quiet_hours,
    })));
    effects.push(Effect::CallsOffPending {
        address: address.clone(),
    });
    read_clock_at(until, effects);
    state.calls_off = CallsOff::Pending {
        address,
        until,
        after_restart: false,
        quiet_hours,
    };
}

/// Calls go off now, with no Call left: the record is kept first, then
/// the Phone with `address` is blocked.
fn turn_off_now(
    state: &mut State,
    address: String,
    until: Option<Deadline>,
    quiet_hours: bool,
    effects: &mut Vec<Effect>,
) {
    effects.push(Effect::KeepCallsOff(Some(CallsOffRecord::Off {
        address: address.clone(),
        until,
        quiet_hours,
    })));
    block(&address, true, effects);
    read_clock_at(until, effects);
    state.calls_off = CallsOff::Off {
        address,
        until,
        quiet_hours,
    };
}

/// Calls are turned on, by the user or at their deadline: the record says
/// so first, then the Phone is unblocked, and connected once it is.
pub(super) fn turn_on(state: &mut State, effects: &mut Vec<Effect>) {
    let (CallsOff::Off {
        address,
        until,
        quiet_hours,
    }
    | CallsOff::Reblocking {
        address,
        until,
        quiet_hours,
    }) = &state.calls_off
    else {
        return;
    };
    let (address, until, quiet_hours) = (address.clone(), *until, *quiet_hours);
    effects.push(Effect::KeepCallsOff(Some(CallsOffRecord::TurningOn {
        address: address.clone(),
        kept_on_until: state.kept_on_until,
    })));
    block(&address, false, effects);
    state.calls_off = CallsOff::TurningOn {
        address,
        until,
        quiet_hours,
    };
}

/// A turn-off waiting for the last Call to end is dropped, as its Quiet
/// Hours are over or gone: calls stay on.
pub(super) fn drop_pending(state: &mut State, effects: &mut Vec<Effect>) {
    if let CallsOff::Pending { address, .. } = &state.calls_off {
        effects.push(Effect::KeepCallsOff(state.calls_on_record()));
        effects.push(Effect::CallsKeptOn {
            address: address.clone(),
        });
        state.calls_off = CallsOff::On;
    }
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
    effects.push(Effect::KeepCallsOff(state.calls_on_record()));
    connect_again_at_once(state, effects);
}

/// Calls an earlier run kept off are off, with the Phone blocked again or
/// left as it was.
fn reblocked(state: &mut State, address: &str) {
    if let CallsOff::Reblocking {
        address: off,
        until,
        quiet_hours,
    } = &state.calls_off
        && off == address
    {
        state.calls_off = CallsOff::Off {
            address: off.clone(),
            until: *until,
            quiet_hours: *quiet_hours,
        };
    }
}

/// BlueZ couldn't block or unblock the Phone. Calls stay as they were
/// before the user asked, and the user is told. Calls an earlier run kept
/// off stay off. Calls left off past their deadline come back on at the
/// clock's next report, which tries again. Calls Quiet Hours couldn't turn
/// off stay on until those next begin, rather than trying again and again.
fn block_failed(state: &mut State, address: &str, blocked: bool, effects: &mut Vec<Effect>) {
    if blocked
        && matches!(&state.calls_off, CallsOff::Reblocking { address: off, .. } if off == address)
    {
        reblocked(state, address);
    } else if blocked {
        let CallsOff::Off {
            address: off,
            quiet_hours,
            ..
        } = &state.calls_off
        else {
            return;
        };
        if off != address {
            return;
        }
        if *quiet_hours {
            state.kept_on_until = state.quiet_hours_under_way().map(|quiet| quiet.next_start);
        }
        state.calls_off = CallsOff::On;
        effects.push(Effect::KeepCallsOff(state.calls_on_record()));
        effects.push(Effect::ReportError(Failure::CallsOffFailed));
        connect_again_at_once(state, effects);
    } else if let CallsOff::TurningOn {
        address: turning,
        until,
        quiet_hours,
    } = &state.calls_off
        && turning == address
    {
        let (until, quiet_hours) = (*until, *quiet_hours);
        state.kept_on_until = None;
        effects.push(Effect::KeepCallsOff(Some(CallsOffRecord::Off {
            address: address.to_owned(),
            until,
            quiet_hours,
        })));
        state.calls_off = CallsOff::Off {
            address: address.to_owned(),
            until,
            quiet_hours,
        };
        effects.push(Effect::ReportError(Failure::CallsOnFailed));
    }
}

/// The Desktop stops following the Phone, unpaired or switched away from
/// in the Setup Window: calls are on again, so the next Phone can connect,
/// and the Phone the Desktop blocked is unblocked, if BlueZ still has it.
pub(super) fn phone_forgotten(state: &State, effects: &mut Vec<Effect>) {
    if let CallsOff::Pending { address, .. } = &state.calls_off {
        effects.push(Effect::KeepCallsOff(state.calls_on_record()));
        effects.push(Effect::CallsKeptOn {
            address: address.clone(),
        });
        return;
    }
    let (CallsOff::Off { address, .. }
    | CallsOff::Reblocking { address, .. }
    | CallsOff::TurningOn { address, .. }) = &state.calls_off
    else {
        return;
    };
    effects.push(Effect::KeepCallsOff(state.calls_on_record()));
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
