//! The pure core of Desktop Calls.
//!
//! It holds the Call state machine, matching and messages: every policy
//! decision, as functions from inputs to outputs. It performs no I/O, reads no
//! clock or environment, and knows nothing of GTK, D-Bus or logging. Shell
//! adapters in the `desktop-calls` crate turn the outside world into inputs and
//! carry out the effects. See `CODING_STANDARDS.md`.
#![deny(clippy::print_stdout, clippy::print_stderr, clippy::dbg_macro)]

mod call;
mod connection;
mod contacts;
mod earlier_run;
mod indicator;
mod machine;
pub mod matching;
pub mod messages;
mod missed;
mod number;

pub use call::{
    AudioLocation, Call, CallId, CallState, Caller, Codec, Direction, Timestamp, Transport,
};
pub use connection::{ConnectionState, Phone, UnknownConnectionState};
pub use contacts::{
    CallHistory, CallHistoryEntry, CallHistoryKind, Contact, ContactNumber, Contacts,
    ContactsState, NumberLabel, SyncFailure, Unavailable,
};
pub use earlier_run::{EarlierCall, EarlierRun};
pub use indicator::{IndicatorEntry, IndicatorLook, IndicatorStatus, IndicatorView};
pub use machine::{
    BannerView, CallAudio, CallWindowPhase, CallWindowView, ConnectFailure, DialProgress,
    DialerView, Effect, HeldCall, Input, Mute, PhoneCommand, PhoneRefusal, Problem, State,
    Surfaces, Timer, keypad_tones, step,
};
pub use matching::{ContactMatch, Directory};
pub use missed::MissedCallNotice;
