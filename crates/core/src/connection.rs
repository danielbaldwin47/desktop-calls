//! The Phone and whether the Desktop is connected to it.

use std::fmt;
use std::str::FromStr;

/// The paired Phone, as the Desktop knows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Phone {
    /// The name to show the user.
    pub name: String,
    /// The Phone's Bluetooth address, such as `AA:BB:CC:DD:EE:FF`.
    pub address: String,
}

/// Whether the Desktop can take and place Calls through the Phone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ConnectionState {
    /// No Phone is paired.
    #[default]
    NotSetUp,
    /// A Phone is paired, and the Desktop is waiting for it to be reachable.
    Connecting,
    /// The Phone is connected for Calls.
    Connected,
    /// The Phone was connected and is not now: out of range, or Bluetooth is off.
    Disconnected,
}

impl ConnectionState {
    /// The lower-case name the control interface and the CLI use.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotSetUp => "not-set-up",
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Disconnected => "disconnected",
        }
    }
}

impl fmt::Display for ConnectionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A name that is not one of [`ConnectionState::as_str`]'s.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownConnectionState;

impl FromStr for ConnectionState {
    type Err = UnknownConnectionState;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        [
            Self::NotSetUp,
            Self::Connecting,
            Self::Connected,
            Self::Disconnected,
        ]
        .into_iter()
        .find(|state| state.as_str() == name)
        .ok_or(UnknownConnectionState)
    }
}
