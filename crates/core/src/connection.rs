//! The Call state machine: one function from (state, input) to (new state,
//! effects). So far it decides only whether the Phone is connected.

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

/// Something that happened, as the shell adapters report it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    /// A paired Phone that can carry Calls exists on the Desktop.
    PhonePaired(Phone),
    /// The paired Phone with this address is gone.
    PhoneUnpaired { address: String },
    /// The Phone with this address is connected for Calls.
    PhoneConnected { address: String },
    /// The Phone with this address is no longer connected for Calls.
    PhoneDisconnected { address: String },
}

/// Something the shell adapters must do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Follow the Phone with this address: report when it connects and
    /// disconnects for Calls.
    AttachPhone { address: String },
    /// Stop following the Phone.
    DetachPhone,
}

/// Everything the Call state machine knows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    phone: Option<Phone>,
    connection_state: ConnectionState,
}

impl State {
    /// The Phone the Desktop works with, once one is paired.
    #[must_use]
    pub fn phone(&self) -> Option<&Phone> {
        self.phone.as_ref()
    }

    #[must_use]
    pub fn connection_state(&self) -> &ConnectionState {
        &self.connection_state
    }

    fn is_our_phone(&self, address: &str) -> bool {
        self.phone
            .as_ref()
            .is_some_and(|phone| phone.address.eq_ignore_ascii_case(address))
    }
}

/// Works out the new state and the effects that follow from `input`.
#[must_use]
pub fn step(state: State, input: Input) -> (State, Vec<Effect>) {
    match input {
        // Desktop Calls works with one Phone: the first one paired.
        Input::PhonePaired(phone) if state.phone.is_none() => {
            let effects = vec![Effect::AttachPhone {
                address: phone.address.clone(),
            }];
            let state = State {
                phone: Some(phone),
                connection_state: ConnectionState::Connecting,
            };
            (state, effects)
        }
        Input::PhoneUnpaired { address } if state.is_our_phone(&address) => {
            (State::default(), vec![Effect::DetachPhone])
        }
        Input::PhoneConnected { address } if state.is_our_phone(&address) => (
            State {
                connection_state: ConnectionState::Connected,
                ..state
            },
            Vec::new(),
        ),
        Input::PhoneDisconnected { address }
            if state.is_our_phone(&address)
                && state.connection_state == ConnectionState::Connected =>
        {
            (
                State {
                    connection_state: ConnectionState::Disconnected,
                    ..state
                },
                Vec::new(),
            )
        }
        _ => (state, Vec::new()),
    }
}
