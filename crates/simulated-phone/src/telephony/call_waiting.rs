//! Call waiting, as PipeWire 1.6.9 carries it out with `AT+CHLD`
//! (`backend-native.c`, `docs/research/2026-11-05-call-waiting.md`):
//!
//! - `HoldAndAnswer` (`AT+CHLD=2`) needs an `active` Call and a `waiting`
//!   one: the active Call goes `held` and the waiting one `active`.
//! - `ReleaseAndAnswer` (`AT+CHLD=1`) needs the same: the active Call ends
//!   and the waiting one goes `active`.
//! - `SwapCalls` (`AT+CHLD=2`) needs a `held` Call: it goes `active`, and
//!   an active one `held`.
//! - Any of them in another state is `InvalidState`, as with the
//!   `hold-refused` quirk, which refuses `HoldAndAnswer` and `SwapCalls`
//!   always, as a network without call hold does.
//! - `Hangup` on an `active`, `dialing`, `alerting` or `incoming` Call while
//!   another is `held` is `AT+CHLD=1`: the Call ends and the held one goes
//!   `active`.
//! - A waiting Call left with no Call the user is on, because that Call
//!   ended, rings as an `incoming` one, as the network presents it again.

use super::phone::calls;
use super::tree::{call_state, end_call, set_call_state};
use super::{Behaviour, INVALID_STATE, INVALID_STATE_MESSAGE, Quirk, Signal};
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
    let (Some(active), Some(waiting)) = (
        call_in(tree, audio_gateway, "active"),
        call_in(tree, audio_gateway, "waiting"),
    ) else {
        return refused();
    };
    if behaviour.quirks.contains(&Quirk::HoldRefused) {
        return refused();
    }
    let mut signals = set_call_state(tree, &active, "held");
    signals.extend(set_call_state(tree, &waiting, "active"));
    (Reply::body(()), signals)
}

pub(super) fn release_and_answer(
    tree: &mut ObjectTree,
    audio_gateway: &OwnedObjectPath,
) -> (Reply, Vec<Signal>) {
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
    let Some(held) = call_in(tree, audio_gateway, "held") else {
        return refused();
    };
    if behaviour.quirks.contains(&Quirk::HoldRefused) {
        return refused();
    }
    let mut signals = Vec::new();
    if let Some(active) = call_in(tree, audio_gateway, "active") {
        signals.extend(set_call_state(tree, &active, "held"));
    }
    signals.extend(set_call_state(tree, &held, "active"));
    (Reply::body(()), signals)
}

/// After `Hangup` ended a Call that was `state`: with `AT+CHLD=1`, the held
/// Call takes its place.
pub(super) fn after_hang_up(
    tree: &mut ObjectTree,
    audio_gateway: &OwnedObjectPath,
    state: &str,
) -> Vec<Signal> {
    let mut signals = Vec::new();
    if matches!(state, "active" | "dialing" | "alerting" | "incoming")
        && let Some(held) = call_in(tree, audio_gateway, "held")
    {
        signals.extend(set_call_state(tree, &held, "active"));
    }
    signals.extend(waiting_rings(tree, audio_gateway));
    signals
}

/// Once no Call the user is on is left, a waiting Call rings as `incoming`.
pub(super) fn waiting_rings(tree: &mut ObjectTree, audio_gateway: &OwnedObjectPath) -> Vec<Signal> {
    if has_live_call(tree, audio_gateway) {
        return Vec::new();
    }
    match call_in(tree, audio_gateway, "waiting") {
        Some(waiting) => set_call_state(tree, &waiting, "incoming"),
        None => Vec::new(),
    }
}

fn refused() -> (Reply, Vec<Signal>) {
    (
        Reply::Error(INVALID_STATE, INVALID_STATE_MESSAGE.to_owned()),
        Vec::new(),
    )
}
