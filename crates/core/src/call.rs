//! Calls as the Call state machine knows them.

use std::fmt;

/// The app-assigned stable id of a Call. It stays the same for the whole
/// Call, even when the Phone's own object for it is replaced.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CallId(pub u32);

impl fmt::Display for CallId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A moment, in milliseconds since the Unix epoch. The shell reads the clock;
/// the core only compares and stores these.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp(pub u64);

/// Where a Call is in its life, as the Phone reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallState {
    /// Incoming, and ringing.
    Ringing,
    /// Incoming while another Call is live.
    Waiting,
    /// Outgoing, before the other end rings.
    Dialing,
    /// Outgoing, and ringing at the other end.
    Alerting,
    /// Connected.
    Active,
    /// Connected and on hold.
    Held,
}

impl CallState {
    /// The lower-case name the control interface uses.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ringing => "ringing",
            Self::Waiting => "waiting",
            Self::Dialing => "dialing",
            Self::Alerting => "alerting",
            Self::Active => "active",
            Self::Held => "held",
        }
    }

    fn is_live(self) -> bool {
        matches!(
            self,
            Self::Active | Self::Held | Self::Dialing | Self::Alerting
        )
    }

    pub(crate) fn is_connected(self) -> bool {
        matches!(self, Self::Active | Self::Held)
    }

    pub(crate) fn blocks_ringing(self) -> bool {
        self.is_live()
    }
}

/// Which way a Call goes. The Phone reports an answered incoming Call as
/// `active`, so this is kept from when the Call was first seen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Incoming,
    Outgoing,
    /// First seen already connected, such as after the app restarts mid-Call.
    Unknown,
}

impl Direction {
    /// The lower-case name the control interface uses.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Incoming => "incoming",
            Self::Outgoing => "outgoing",
            Self::Unknown => "unknown",
        }
    }

    pub(crate) fn from_first_state(state: CallState) -> Self {
        match state {
            CallState::Ringing | CallState::Waiting => Self::Incoming,
            CallState::Dialing | CallState::Alerting => Self::Outgoing,
            CallState::Active | CallState::Held => Self::Unknown,
        }
    }
}

/// Who is on the other end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Caller {
    /// A phone number, as the Phone reports it.
    Number(String),
    /// The caller withheld their number.
    Withheld,
}

/// The Call's audio link between Phone and Desktop.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Transport {
    /// No audio on the Desktop.
    #[default]
    Idle,
    /// The audio link is being set up.
    Pending,
    /// Call audio flows through the Desktop.
    Active,
}

impl Transport {
    /// Whether the audio link is up or coming up.
    pub(crate) fn is_up(self) -> bool {
        matches!(self, Self::Pending | Self::Active)
    }
}

/// The speech codec of the Call's audio on the Desktop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    /// Narrowband.
    Cvsd,
    /// Wideband.
    Msbc,
    /// Super-wideband.
    Lc3Swb,
    /// A codec this version doesn't know, by its hands-free id.
    Other(u8),
}

impl Codec {
    /// The codec with this hands-free id, or `None` for 0, which the Phone
    /// reports while no audio link exists.
    #[must_use]
    pub fn from_id(id: u8) -> Option<Self> {
        match id {
            0 => None,
            1 => Some(Self::Cvsd),
            2 => Some(Self::Msbc),
            3 => Some(Self::Lc3Swb),
            other => Some(Self::Other(other)),
        }
    }

    /// Whether the codec carries wideband speech or better ("HD").
    #[must_use]
    pub fn is_wideband(self) -> bool {
        matches!(self, Self::Msbc | Self::Lc3Swb)
    }

    /// The lower-case name the control interface uses.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cvsd => "cvsd",
            Self::Msbc => "msbc",
            Self::Lc3Swb => "lc3-swb",
            Self::Other(_) => "other",
        }
    }
}

/// Where a Call's audio is heard and spoken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioLocation {
    Phone,
    Desktop,
}

impl AudioLocation {
    /// The lower-case name the control interface uses.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Phone => "phone",
            Self::Desktop => "desktop",
        }
    }
}

/// What the user asked the Desktop to do with a Call, while the Phone has
/// not caught up yet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Request {
    #[default]
    None,
    Answer,
    Decline,
    HangUp,
}

/// One Call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Call {
    pub(crate) id: CallId,
    pub(crate) direction: Direction,
    pub(crate) state: CallState,
    pub(crate) caller: Caller,
    pub(crate) active_since: Option<Timestamp>,
    pub(crate) request: Request,
}

impl Call {
    #[must_use]
    pub fn id(&self) -> CallId {
        self.id
    }

    #[must_use]
    pub fn direction(&self) -> Direction {
        self.direction
    }

    #[must_use]
    pub fn state(&self) -> CallState {
        self.state
    }

    #[must_use]
    pub fn caller(&self) -> &Caller {
        &self.caller
    }

    /// When the Call connected, once it has.
    #[must_use]
    pub fn active_since(&self) -> Option<Timestamp> {
        self.active_since
    }

    /// Ringing on the Desktop: incoming, and neither answered nor declined
    /// here yet.
    pub(crate) fn rings_here(&self) -> bool {
        self.state == CallState::Ringing && self.request == Request::None
    }

    /// Shown in the Call Window: live, or answered here and connecting.
    pub(crate) fn has_call_window(&self) -> bool {
        self.state.is_live() || self.request == Request::Answer
    }
}
