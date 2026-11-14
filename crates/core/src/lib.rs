//! The pure core of Desktop Calls.
//!
//! It holds the Call state machine, matching and messages: every policy
//! decision, as functions from inputs to outputs. It performs no I/O, reads no
//! clock or environment, and knows nothing of GTK, D-Bus or logging. Shell
//! adapters in the `desktop-calls` crate turn the outside world into inputs and
//! carry out the effects. See `CODING_STANDARDS.md`.
#![deny(clippy::print_stdout, clippy::print_stderr, clippy::dbg_macro)]

mod call;
pub mod clock;
mod connection;
mod contacts;
mod earlier_run;
mod indicator;
pub mod keybinding;
mod machine;
pub mod matching;
pub mod messages;
mod missed;
mod number;

pub use call::{
    AudioLocation, Call, CallId, CallState, Caller, Codec, Direction, Timestamp, Transport,
};
pub use clock::{Deadline, TimeOfDay, WallClock};
pub use connection::{ConnectionState, Phone, UnknownConnectionState};
pub use contacts::{
    CallHistory, CallHistoryEntry, CallHistoryKind, Contact, ContactNumber, Contacts,
    ContactsState, NumberLabel, SyncFailure, SyncProgress, Unavailable,
};
pub use earlier_run::{EarlierCall, EarlierRun};
pub use indicator::{IndicatorEntry, IndicatorLook, IndicatorStatus, IndicatorView};
pub use keybinding::{ConfigFile, HyprlandConfig, KeybindingCheck, RequiredFile};
pub use machine::{
    BannerView, BluetoothPower, CallAudio, CallPillLead, CallPillTag, CallWindowPhase,
    CallWindowView, CallsOffRecord, ConnectFailure, ContactsOutcome, DesktopCheck, DesktopFix,
    DialProgress, DialerView, Effect, FixFailure, HeldCall, Input, KeybindingStage, Mute,
    PairingFailure, PhoneCommand, PhoneRefusal, Problem, ReadingContacts, RingingCaller,
    SetupNotice, SetupProblemsView, SetupStep, SetupWindowView, State, StateLine, Surfaces, Timer,
    WaitingOptions, keypad_tones, step,
};
pub use matching::{ContactMatch, Directory};
pub use missed::MissedCallNotice;
