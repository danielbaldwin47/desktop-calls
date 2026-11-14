//! What the Indicator in the Desktop's top bar shows, worked out from the
//! Call state machine's state: its look, whether it asks for attention, and
//! its menu, with what each menu entry asks for.

use crate::call::{AudioLocation, CallId, Caller};
use crate::clock::{Deadline, QuietHours, TimeOfDay};
use crate::connection::{ConnectionState, Phone};
use crate::machine::{CallWindowPhase, Input, Mute, RingingCaller, State, Surfaces};
use crate::matching::ContactMatch;
use crate::messages::ProblemKind;

/// The Indicator's content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndicatorView {
    pub look: IndicatorLook,
    /// A Call rings here, or a Problem needs fixing. Some trays draw this
    /// differently; Omarchy's doesn't, so the look shows it too.
    pub needs_attention: bool,
    /// What the status line and the tooltip say.
    pub status: IndicatorStatus,
    /// Every Problem, for the tooltip and the menu.
    pub problems: Vec<ProblemKind>,
    /// The menu's entries, in one order in every state: the status line,
    /// the main action, during a Call the Call's actions, calls on or off,
    /// Quiet Hours, a separator, then setup and Problems.
    pub menu: Vec<IndicatorEntry>,
}

/// Which icon the Indicator shows. Each looks different.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndicatorLook {
    /// The Phone is connected and idle.
    Connected,
    /// The Phone isn't connected: out of range, not yet connected, or none
    /// is set up.
    Disconnected,
    /// A Call rings here.
    Ringing,
    /// A Call is live, or being placed.
    InCall,
    /// A Problem needs fixing, with the Phone connected, or kept from
    /// connecting by it.
    Problem,
    /// Calls are off: the Desktop keeps the Phone disconnected. Problems
    /// don't change it.
    Off,
}

/// What the Indicator's status says, before Messages puts it in words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IndicatorStatus {
    NoPhone,
    /// Calls are off until this local time, or until the user turns them
    /// on; with `quiet_hours`, for Quiet Hours, until they end.
    CallsOff {
        until: Option<TimeOfDay>,
        quiet_hours: bool,
    },
    /// Calls turn off when the last Call ends. Said instead of the Call's
    /// other party, which the Call Window shows, but not over a ringing
    /// Call's caller.
    CallsOffPending,
    /// Paired, and not yet connected since the Desktop started.
    NotConnected(Phone),
    Connected(Phone),
    OutOfRange(Phone),
    /// In range, and hands-free didn't come back after a Handoff to the
    /// Phone: the Desktop is connecting it again.
    ConnectionLost(Phone),
    /// A Call rings here from this caller, as the Banner says it.
    Ringing(RingingCaller),
    /// A Call with this other party is live or being placed.
    InCall {
        caller: Caller,
        contact: Option<ContactMatch>,
    },
    /// The Desktop lost contact with the Call with this other party, which
    /// carries on on the Phone, as the Call Window says.
    CallOnPhone {
        caller: Caller,
        contact: Option<ContactMatch>,
    },
}

/// Where the Phone is, apart from any Call: what the Indicator says with no
/// Call, and the Dialer's footer at all times. Worked out once, here, so the
/// two can't disagree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PhoneStatus {
    NoPhone,
    /// Calls are off until this local time, or until the user turns them
    /// on; with `quiet_hours`, for Quiet Hours, until they end.
    CallsOff {
        until: Option<TimeOfDay>,
        quiet_hours: bool,
    },
    /// Paired, and not yet connected since the Desktop started, or kept
    /// from connecting until it is paired again.
    NotConnected(Phone),
    Connected(Phone),
    OutOfRange(Phone),
    /// In range, and hands-free didn't come back after a Handoff to the
    /// Phone: the Desktop is connecting it again.
    ConnectionLost(Phone),
}

impl From<PhoneStatus> for IndicatorStatus {
    fn from(status: PhoneStatus) -> Self {
        match status {
            PhoneStatus::NoPhone => Self::NoPhone,
            PhoneStatus::CallsOff { until, quiet_hours } => Self::CallsOff { until, quiet_hours },
            PhoneStatus::NotConnected(phone) => Self::NotConnected(phone),
            PhoneStatus::Connected(phone) => Self::Connected(phone),
            PhoneStatus::OutOfRange(phone) => Self::OutOfRange(phone),
            PhoneStatus::ConnectionLost(phone) => Self::ConnectionLost(phone),
        }
    }
}

/// One entry in the Indicator's menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IndicatorEntry {
    /// The status line, which does nothing.
    Status,
    OpenDialer,
    /// Opens the Setup Window, to pair a Phone, pair again or switch.
    ConnectPhone,
    /// A line between groups of entries.
    Separator,
    /// Hangs up the Call in the Call Window, or stops placing it (`None`).
    HangUp(Option<CallId>),
    /// Says where to hang up the Call the Desktop lost contact with: on the
    /// Phone. Disabled, as the Call Window's Hang up is.
    HangUpOnPhone,
    /// Mutes or unmutes the microphone, ticked while muted; unavailable while
    /// the audio is on the Phone.
    Mute(Mute),
    /// Moves the audio to `to`; disabled while the audio is on its way here.
    MoveAudio {
        to: AudioLocation,
        moving: bool,
    },
    /// A Problem, with its explanation beneath.
    Problem(ProblemKind),
    /// Turns calls off, with [`IndicatorEntry::choices`] for how long;
    /// disabled, with none, while calls are being turned on. The timed
    /// choices end at `for_an_hour` and `until_morning`, once the wall
    /// clock has been read. During a Call, the turn-off waits for the last
    /// Call to end.
    TurnCallsOff {
        enabled: bool,
        for_an_hour: Option<Deadline>,
        until_morning: Option<Deadline>,
    },
    /// The choice under Turn calls off: until the user turns them on.
    CallsOffUntilTurnedOn,
    /// The choice under Turn calls off: for an hour, until this deadline.
    CallsOffForAnHour(Deadline),
    /// The choice under Turn calls off: until the next morning, this
    /// deadline.
    CallsOffUntilMorning(Deadline),
    /// Turns calls on.
    TurnCallsOn,
    /// Cancels the turn-off waiting for the last Call to end.
    KeepCallsOn,
    /// Opens the Quiet Hours Window, or raises it. Always in the menu,
    /// with the Quiet Hours set, if any, as a hint.
    QuietHours(Option<QuietHours>),
}

impl IndicatorEntry {
    /// What choosing the entry asks for: the same as the Call Window's
    /// button for it. `None` for an entry that only informs, or is
    /// disabled.
    #[must_use]
    pub fn intent(&self) -> Option<Input> {
        match self {
            Self::OpenDialer => Some(Input::OpenDialer {
                query: String::new(),
                select_call: None,
            }),
            Self::ConnectPhone
            | Self::Problem(
                ProblemKind::PairingLost
                | ProblemKind::BluetoothOff
                | ProblemKind::BluetoothBlocked
                | ProblemKind::ContactsSupportMissing
                | ProblemKind::AudioNotSetUp
                | ProblemKind::BluezHandsFree,
            ) => Some(Input::OpenSetupWindow),
            Self::HangUp(call) => Some(call.map_or(Input::CancelDial, Input::HangUp)),
            Self::Mute(Mute::Off) => Some(Input::SetMuted(true)),
            Self::Mute(Mute::On) => Some(Input::SetMuted(false)),
            Self::MoveAudio { to, moving: false } => Some(Input::MoveAudio(*to)),
            Self::CallsOffUntilTurnedOn => Some(Input::TurnCallsOff { until: None }),
            Self::CallsOffForAnHour(until) | Self::CallsOffUntilMorning(until) => {
                Some(Input::TurnCallsOff {
                    until: Some(*until),
                })
            }
            Self::TurnCallsOn => Some(Input::TurnCallsOn),
            Self::KeepCallsOn => Some(Input::KeepCallsOn),
            Self::QuietHours(_) => Some(Input::OpenQuietHoursWindow),
            Self::Status
            | Self::Separator
            | Self::HangUpOnPhone
            | Self::Mute(Mute::Unavailable)
            | Self::MoveAudio { moving: true, .. }
            | Self::Problem(_)
            | Self::TurnCallsOff { .. } => None,
        }
    }

    /// Whether the entry can be chosen. The status line and a Problem only
    /// inform, but read as enabled.
    #[must_use]
    pub fn enabled(&self) -> bool {
        !matches!(
            self,
            Self::HangUpOnPhone
                | Self::Mute(Mute::Unavailable)
                | Self::MoveAudio { moving: true, .. }
                | Self::TurnCallsOff { enabled: false, .. }
        )
    }

    /// The choices beneath the entry, as a submenu: how long to turn calls
    /// off for. Empty for every other entry.
    #[must_use]
    pub fn choices(&self) -> Vec<Self> {
        match self {
            Self::TurnCallsOff {
                enabled: true,
                for_an_hour,
                until_morning,
            } => [Some(Self::CallsOffUntilTurnedOn)]
                .into_iter()
                .chain([
                    for_an_hour.map(Self::CallsOffForAnHour),
                    until_morning.map(Self::CallsOffUntilMorning),
                ])
                .flatten()
                .collect(),
            _ => Vec::new(),
        }
    }
}

impl State {
    /// The menu's entry that turns calls on or off: on while they are off,
    /// keep on while a turn-off waits for the last Call to end, else off
    /// once there is a Phone, disabled while calls are being turned on.
    fn calls_off_entry(&self, has_phone: bool) -> Option<IndicatorEntry> {
        if self.calls_off() {
            Some(IndicatorEntry::TurnCallsOn)
        } else if self.calls_off_pending() {
            Some(IndicatorEntry::KeepCallsOn)
        } else {
            let clock = self.wall_clock();
            has_phone.then(|| IndicatorEntry::TurnCallsOff {
                enabled: !self.calls_turning_on(),
                for_an_hour: clock.map(|clock| clock.for_an_hour),
                until_morning: self.until_morning(),
            })
        }
    }

    /// Where the Phone is, apart from any Call.
    #[must_use]
    pub fn phone_status(&self) -> PhoneStatus {
        if self.calls_off() {
            return PhoneStatus::CallsOff {
                until: self.calls_off_until().map(|until| until.shown),
                quiet_hours: self.calls_off_for_quiet_hours(),
            };
        }
        let Some(phone) = self.phone().cloned() else {
            return PhoneStatus::NoPhone;
        };
        match self.connection_state() {
            ConnectionState::Connected => PhoneStatus::Connected(phone),
            ConnectionState::Disconnected if self.hands_free_lost() => {
                PhoneStatus::ConnectionLost(phone)
            }
            ConnectionState::Disconnected => PhoneStatus::OutOfRange(phone),
            ConnectionState::PairingLost | ConnectionState::Connecting => {
                PhoneStatus::NotConnected(phone)
            }
            ConnectionState::NotSetUp => PhoneStatus::NoPhone,
        }
    }

    /// What the Indicator shows, given the `surfaces` this state shows.
    /// They are passed in, not worked out again: with thousands of Contacts
    /// the Dialer's search is too slow to run twice per keystroke.
    #[must_use]
    pub fn indicator(&self, surfaces: &Surfaces) -> IndicatorView {
        let problems: Vec<ProblemKind> = self
            .problems()
            .into_iter()
            .map(|problem| problem.kind)
            .collect();
        let call_window = surfaces.call_window.as_ref().filter(|window| {
            !matches!(
                window.phase,
                CallWindowPhase::Ended | CallWindowPhase::NotPlaced
            )
        });
        let has_phone = self.phone().is_some();
        let (mut look, status) = if let Some(banner) = &surfaces.banner {
            (
                IndicatorLook::Ringing,
                IndicatorStatus::Ringing(banner.caller.clone()),
            )
        } else if let Some(window) = call_window {
            let caller = window.caller.clone();
            let contact = window.contact.clone();
            (
                IndicatorLook::InCall,
                if self.calls_off_pending() {
                    IndicatorStatus::CallsOffPending
                } else if window.connection_lost {
                    IndicatorStatus::CallOnPhone { caller, contact }
                } else {
                    IndicatorStatus::InCall { caller, contact }
                },
            )
        } else {
            let status = self.phone_status();
            let look = match &status {
                PhoneStatus::CallsOff { .. } => IndicatorLook::Off,
                PhoneStatus::Connected(_) if problems.is_empty() => IndicatorLook::Connected,
                PhoneStatus::Connected(_) => IndicatorLook::Problem,
                // Not out of range: it needs pairing again, which the
                // Problem explains.
                PhoneStatus::NotConnected(_)
                    if matches!(self.connection_state(), ConnectionState::PairingLost) =>
                {
                    IndicatorLook::Problem
                }
                _ => IndicatorLook::Disconnected,
            };
            (look, status.into())
        };

        // The Phone can't connect for Calls until the Desktop's Problem is
        // put right, which the Problem explains.
        if look == IndicatorLook::Disconnected && self.desktop_keeps_phone_away() {
            look = IndicatorLook::Problem;
        }

        let mut menu = vec![IndicatorEntry::Status, IndicatorEntry::OpenDialer];
        if call_window.is_some_and(|window| window.connection_lost) {
            // Nothing acts on the Call from here: only where to hang up.
            menu.push(IndicatorEntry::HangUpOnPhone);
        } else if let Some(window) = call_window {
            if let CallWindowPhase::Live { mute, .. } = window.phase {
                menu.push(IndicatorEntry::Mute(mute));
            }
            if let Some(audio) = window.audio {
                menu.push(IndicatorEntry::MoveAudio {
                    to: audio.moving.unwrap_or(match audio.location {
                        AudioLocation::Phone => AudioLocation::Desktop,
                        AudioLocation::Desktop => AudioLocation::Phone,
                    }),
                    moving: audio.moving.is_some(),
                });
            }
            menu.push(IndicatorEntry::HangUp(window.call));
        }
        menu.extend(self.calls_off_entry(has_phone));
        menu.push(IndicatorEntry::QuietHours(self.quiet_hours()));
        menu.push(IndicatorEntry::Separator);
        menu.push(IndicatorEntry::ConnectPhone);
        menu.extend(problems.iter().copied().map(IndicatorEntry::Problem));

        IndicatorView {
            look,
            needs_attention: surfaces.banner.is_some()
                || (!problems.is_empty() && look != IndicatorLook::Off),
            status,
            problems,
            menu,
        }
    }
}
