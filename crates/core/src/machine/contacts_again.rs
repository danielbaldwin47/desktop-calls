//! Asking a Phone that refused to share Contacts again, while it stays
//! connected, in case sharing has been turned on since (ticket #394).
//!
//! Turning on Contacts and call history in the Phone's Bluetooth settings,
//! as the "Contacts not shared" Problem says, changes nothing on the link:
//! the Desktop is never told. So it asks again: 60 times 10 s apart (about
//! the first 10 minutes after the Phone connected), then every 5 minutes. A
//! Phone that refuses outright, as the Pixel 9a does, shows nothing each
//! time, and so does an Android Phone whose prompt the person answered no:
//! it remembers the answer. One that shows its prompt when asked again
//! isn't asked again until it next connects, so the prompt doesn't come
//! back every few seconds.

use super::State;
use super::input::{Effect, Timer};
use crate::contacts::ContactsState;

/// How long to wait between asks, at first.
const SOON_MS: u32 = 10_000;
/// How many asks [`SOON_MS`] apart: 10 minutes' worth.
const SOON_ASKS: u32 = 60;
/// How long to wait between asks after those.
const LATER_MS: u32 = 300_000;

/// How far asking the Phone again has got.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct AskingAgain {
    /// The wait under way, by number: only its timer asks.
    wait: u32,
    /// How many times the Phone has been asked again since it connected.
    asked: u32,
    /// A sync asking again is under way.
    under_way: bool,
    /// Asking again made the Phone show its prompt.
    prompted: bool,
}

impl AskingAgain {
    /// Asking again for a Phone that has just connected, or for the next
    /// Phone: any wait under way is over.
    pub(super) fn afresh(&self) -> Self {
        Self {
            wait: self.wait + 1,
            ..Self::default()
        }
    }
}

/// A sync ended, whatever its end: whether it was one asking again.
pub(super) fn sync_ended(state: &mut State) -> bool {
    std::mem::take(&mut state.asking_again.under_way)
}

/// The Phone is showing its prompt: if asking again made it, it isn't
/// asked again while it stays connected.
pub(super) fn prompt_shown(state: &mut State) {
    if state.asking_again.under_way {
        state.asking_again.prompted = true;
    }
}

/// The Phone refused, or asking it again failed some other way: waits,
/// then asks again, while it stays connected.
pub(super) fn ask_later(state: &mut State, effects: &mut Vec<Effect>) {
    if state.asking_again.prompted || !state.phone_is_here() {
        return;
    }
    let asking = &mut state.asking_again;
    asking.wait += 1;
    effects.push(Effect::StartTimer {
        timer: Timer::AskForContactsAgain(asking.wait),
        after_ms: if asking.asked < SOON_ASKS {
            SOON_MS
        } else {
            LATER_MS
        },
    });
}

/// The wait numbered `wait` is over: asks the Phone again, if it still
/// refuses and is here. During a Call it waits again instead, so nothing
/// new happens on the link mid-Call.
pub(super) fn wait_over(state: &mut State, wait: u32, effects: &mut Vec<Effect>) {
    if wait != state.asking_again.wait
        || state.contacts_sync != ContactsState::Denied
        || !state.phone_is_here()
    {
        return;
    }
    if !state.calls.is_empty() {
        ask_later(state, effects);
        return;
    }
    state.asking_again.asked += 1;
    state.asking_again.under_way = true;
    effects.push(Effect::SyncContactsAgain);
}
