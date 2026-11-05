//! What one run of the app leaves for the next, so that a run started
//! mid-Call, after a crash, picks the Call up as it was.
//!
//! The Phone reports each live Call's number and state, but not when it
//! connected or which way it went, and only the Desktop knows that it
//! silenced the microphone for Mute. The shell keeps this record where the
//! next run finds it, and gives it back at startup.

use crate::call::{Caller, Direction, Timestamp};

/// What the next run needs if this one stops now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EarlierRun {
    /// Each connected Call.
    pub calls: Vec<EarlierCall>,
    /// The microphone gain to give back, if the Desktop had silenced the
    /// microphone for Mute.
    pub restore_microphone_volume: Option<u8>,
}

impl EarlierRun {
    /// Whether there is nothing to keep.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.calls.is_empty() && self.restore_microphone_volume.is_none()
    }
}

/// A connected Call, as the next run needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EarlierCall {
    pub caller: Caller,
    pub direction: Direction,
    pub active_since: Timestamp,
}
