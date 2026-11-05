//! What the Indicator in the Desktop's top bar shows, worked out from the
//! Call state machine's state: its look, whether it asks for attention, and
//! its menu, with what each menu entry asks for.

use crate::call::{AudioLocation, CallId, Caller};
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
    /// The menu's entries, in order.
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
}

/// What the Indicator's status says, before Messages puts it in words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IndicatorStatus {
    NoPhone,
    /// Paired, and not yet connected since the Desktop started.
    NotConnected(Phone),
    Connected(Phone),
    OutOfRange(Phone),
    /// A Call rings here from this caller, as the Banner says it.
    Ringing(RingingCaller),
    /// A Call with this other party is live or being placed.
    InCall {
        caller: Caller,
        contact: Option<ContactMatch>,
    },
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
            Self::ConnectPhone | Self::Problem(ProblemKind::PairingLost) => {
                Some(Input::OpenSetupWindow)
            }
            Self::HangUp(call) => Some(call.map_or(Input::CancelDial, Input::HangUp)),
            Self::Mute(Mute::Off) => Some(Input::SetMuted(true)),
            Self::Mute(Mute::On) => Some(Input::SetMuted(false)),
            Self::MoveAudio { to, moving: false } => Some(Input::MoveAudio(*to)),
            Self::Status
            | Self::Separator
            | Self::Mute(Mute::Unavailable)
            | Self::MoveAudio { moving: true, .. }
            | Self::Problem(_) => None,
        }
    }

    /// Whether the entry can be chosen. The status line and a Problem only
    /// inform, but read as enabled.
    #[must_use]
    pub fn enabled(&self) -> bool {
        !matches!(
            self,
            Self::Mute(Mute::Unavailable) | Self::MoveAudio { moving: true, .. }
        )
    }
}

impl State {
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
        let phone = self.phone().cloned();
        let (look, status) = if let Some(banner) = &surfaces.banner {
            (
                IndicatorLook::Ringing,
                IndicatorStatus::Ringing(banner.caller.clone()),
            )
        } else if let Some(window) = call_window {
            (
                IndicatorLook::InCall,
                IndicatorStatus::InCall {
                    caller: window.caller.clone(),
                    contact: window.contact.clone(),
                },
            )
        } else {
            match (self.connection_state(), phone) {
                (ConnectionState::Connected, Some(phone)) => (
                    if problems.is_empty() {
                        IndicatorLook::Connected
                    } else {
                        IndicatorLook::Problem
                    },
                    IndicatorStatus::Connected(phone),
                ),
                (ConnectionState::Disconnected, Some(phone)) => (
                    IndicatorLook::Disconnected,
                    IndicatorStatus::OutOfRange(phone),
                ),
                // Not out of range: it needs pairing again, which the
                // Problem explains.
                (ConnectionState::PairingLost, Some(phone)) => {
                    (IndicatorLook::Problem, IndicatorStatus::NotConnected(phone))
                }
                (ConnectionState::Connecting, Some(phone)) => (
                    IndicatorLook::Disconnected,
                    IndicatorStatus::NotConnected(phone),
                ),
                _ => (IndicatorLook::Disconnected, IndicatorStatus::NoPhone),
            }
        };

        let mut menu = vec![
            IndicatorEntry::Status,
            IndicatorEntry::OpenDialer,
            IndicatorEntry::ConnectPhone,
        ];
        if let Some(window) = call_window {
            menu.push(IndicatorEntry::Separator);
            menu.push(IndicatorEntry::HangUp(window.call));
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
        }
        if !problems.is_empty() {
            menu.push(IndicatorEntry::Separator);
            menu.extend(problems.iter().copied().map(IndicatorEntry::Problem));
        }

        IndicatorView {
            look,
            needs_attention: surfaces.banner.is_some() || !problems.is_empty(),
            status,
            problems,
            menu,
        }
    }
}
