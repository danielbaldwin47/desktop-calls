//! Call waiting, as PipeWire 1.6.9 carries it out with `AT+CHLD`
//! (`backend-native.c`, `docs/research/2026-11-05-call-waiting.md`):
//!
//! - `HoldAndAnswer` (`AT+CHLD=2`) needs an `active` Call and a `waiting`
//!   one: the active Call goes `held` and the waiting one `active`.
//! - `ReleaseAndAnswer` (`AT+CHLD=1`) needs the same: the active Call ends
//!   and the waiting one goes `active`.
//! - `SwapCalls` (`AT+CHLD=2`) needs a `held` Call: it goes `active`, and
//!   an active one `held`.
//! - Any of them in another state is `InvalidState`. With the
//!   `chld-unsupported` quirk, a Phone that didn't list `AT+CHLD` support,
//!   all three are `NotSupported` whatever the state, as PipeWire checks
//!   that first.
//! - `SwapCalls` while a Call is `waiting` sends `AT+CHLD=2` all the same,
//!   which in 3GPP TS 27.007 accepts the Waiting Call rather than the held
//!   one: the active Call goes `held` and the waiting one `active`.
//! - `Hangup` on an `active`, `dialing`, `alerting` or `incoming` Call while
//!   another is `held` is `AT+CHLD=1`: the Call ends, and the other Call
//!   goes `active`, the waiting one if there is one (27.007), else the held
//!   one.
//! - `Hangup` on a `held` Call is `AT+CHLD=0`, which in 27.007 rejects a
//!   Waiting Call before it releases held ones: with a Call `waiting`, that
//!   Call ends, and the held one stays.
//! - A Waiting Call left with no Call the user is on, because that Call
//!   ended, rings as an `incoming` one, as the network presents it again.

use super::phone::calls;
use super::tree::{call_state, end_call, set_call_state};
use super::{
    Behaviour, INVALID_STATE, INVALID_STATE_MESSAGE, NOT_SUPPORTED, NOT_SUPPORTED_MESSAGE, Quirk,
    Signal,
};
use crate::objects::{ObjectTree, Reply};
use zbus::OwnedObjectPath;

/// The first of the AudioGateway's Calls in `state`.
fn call_in(
    tree: &ObjectTree,
    audio_gateway: &OwnedObjectPath,
    state: &str,
) -> Option<OwnedObjectPath> {
    calls(tree, audio_gateway.as_str())
        .into_iter()
        .find(|(_, properties)| call_state(properties) == state)
        .map(|(path, _)| path)
}

/// Whether the AudioGateway has a Call the user is on, so a new one waits
/// instead of ringing.
pub(super) fn has_live_call(tree: &ObjectTree, audio_gateway: &OwnedObjectPath) -> bool {
    calls(tree, audio_gateway.as_str())
        .iter()
        .any(|(_, properties)| {
            matches!(
                call_state(properties).as_str(),
                "active" | "held" | "dialing" | "alerting"
            )
        })
}

pub(super) fn hold_and_answer(
    tree: &mut ObjectTree,
    audio_gateway: &OwnedObjectPath,
    behaviour: &Behaviour,
) -> (Reply, Vec<Signal>) {
    if let Some(refusal) = without_chld(behaviour) {
        return refusal;
    }
    let (Some(active), Some(waiting)) = (
        call_in(tree, audio_gateway, "active"),
        call_in(tree, audio_gateway, "waiting"),
    ) else {
        return refused();
    };
    let mut signals = set_call_state(tree, &active, "held");
    signals.extend(set_call_state(tree, &waiting, "active"));
    (Reply::body(()), signals)
}

pub(super) fn release_and_answer(
    tree: &mut ObjectTree,
    audio_gateway: &OwnedObjectPath,
    behaviour: &Behaviour,
) -> (Reply, Vec<Signal>) {
    if let Some(refusal) = without_chld(behaviour) {
        return refusal;
    }
    let (Some(active), Some(waiting)) = (
        call_in(tree, audio_gateway, "active"),
        call_in(tree, audio_gateway, "waiting"),
    ) else {
        return refused();
    };
    let mut signals = end_call(tree, audio_gateway, &active);
    signals.extend(set_call_state(tree, &waiting, "active"));
    (Reply::body(()), signals)
}

pub(super) fn swap_calls(
    tree: &mut ObjectTree,
    audio_gateway: &OwnedObjectPath,
    behaviour: &Behaviour,
) -> (Reply, Vec<Signal>) {
    if let Some(refusal) = without_chld(behaviour) {
        return refusal;
    }
    let Some(held) = call_in(tree, audio_gateway, "held") else {
        return refused();
    };
    // `AT+CHLD=2` accepts a Waiting Call before a held one.
    let accepted = call_in(tree, audio_gateway, "waiting").unwrap_or(held);
    let mut signals = Vec::new();
    if let Some(active) = call_in(tree, audio_gateway, "active") {
        signals.extend(set_call_state(tree, &active, "held"));
    }
    signals.extend(set_call_state(tree, &accepted, "active"));
    (Reply::body(()), signals)
}

/// `Hangup` on the Call at `path`, `state`, as PipeWire sends it: on a held
/// Call while one is waiting, `AT+CHLD=0` rejects the Waiting Call instead.
/// Returns the Call that ends.
pub(super) fn hung_up_call(
    tree: &ObjectTree,
    audio_gateway: &OwnedObjectPath,
    path: OwnedObjectPath,
    state: &str,
) -> OwnedObjectPath {
    if state == "held" {
        call_in(tree, audio_gateway, "waiting").unwrap_or(path)
    } else {
        path
    }
}

/// After `Hangup` ended a Call that was `state`: with `AT+CHLD=1`, sent
/// while a Call is held, the Waiting Call, or else the held one, takes its
/// place.
pub(super) fn after_hang_up(
    tree: &mut ObjectTree,
    audio_gateway: &OwnedObjectPath,
    state: &str,
) -> Vec<Signal> {
    let mut signals = Vec::new();
    if matches!(state, "active" | "dialing" | "alerting" | "incoming")
        && let Some(held) = call_in(tree, audio_gateway, "held")
    {
        let accepted = call_in(tree, audio_gateway, "waiting").unwrap_or(held);
        signals.extend(set_call_state(tree, &accepted, "active"));
    }
    signals.extend(waiting_rings(tree, audio_gateway));
    signals
}

/// Once no Call the user is on is left, a Waiting Call rings as `incoming`.
pub(super) fn waiting_rings(tree: &mut ObjectTree, audio_gateway: &OwnedObjectPath) -> Vec<Signal> {
    if has_live_call(tree, audio_gateway) {
        return Vec::new();
    }
    match call_in(tree, audio_gateway, "waiting") {
        Some(waiting) => set_call_state(tree, &waiting, "incoming"),
        None => Vec::new(),
    }
}

/// `NotSupported`, as PipeWire replies before it looks at the Calls, if the
/// Phone didn't list `AT+CHLD` support.
fn without_chld(behaviour: &Behaviour) -> Option<(Reply, Vec<Signal>)> {
    behaviour.quirks.contains(&Quirk::ChldUnsupported).then(|| {
        (
            Reply::Error(NOT_SUPPORTED, NOT_SUPPORTED_MESSAGE.to_owned()),
            Vec::new(),
        )
    })
}

fn refused() -> (Reply, Vec<Signal>) {
    (
        Reply::Error(INVALID_STATE, INVALID_STATE_MESSAGE.to_owned()),
        Vec::new(),
    )
}
