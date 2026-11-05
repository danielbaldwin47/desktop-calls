//! What the Desktop surfaces show, worked out from the state.

use super::{Focus, State};
use crate::call::{AudioLocation, CallId, CallState, Caller, Codec, Timestamp};
use crate::matching::{ContactMatch, DialerRow};
use crate::messages::Failure;

/// What the Desktop surfaces show, worked out from the state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Surfaces {
    pub banner: Option<BannerView>,
    pub call_window: Option<CallWindowView>,
    pub dialer: Option<DialerView>,
}

/// The Dialer's content. While open it has keyboard focus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialerView {
    /// What is typed.
    pub query: String,
    /// Who to Call, best first; with nothing typed, the Call History.
    pub rows: Vec<DialerRow>,
    /// The row Enter Calls, while there are rows.
    pub selected: Option<usize>,
    /// Why Enter can't Call the selected row, or why what is typed can't be
    /// dialled.
    pub notice: Option<Failure>,
    /// Different each time the Dialer opens, so the shell knows to put
    /// `query` in the Dialer's text entry afresh, and leave it alone while
    /// the user types.
    pub opened: u32,
}

/// The Incoming Call Banner's content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BannerView {
    pub call: CallId,
    pub caller: Caller,
    /// The Banner has keyboard focus, so A answers and D declines.
    pub focused: bool,
    /// The Contact the caller is, if Contacts say.
    pub contact: Option<ContactMatch>,
}

/// The Call Window's content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallWindowView {
    /// The Call shown, or `None` for a Call placed here that the Phone
    /// hasn't reported yet, or couldn't place.
    pub call: Option<CallId>,
    pub caller: Caller,
    /// The Contact the other party is, if Contacts say.
    pub contact: Option<ContactMatch>,
    pub phase: CallWindowPhase,
    /// Where the Call's audio is, while there is a Call to move it for.
    pub audio: Option<CallAudio>,
    /// A failure to explain beside the Call.
    pub notice: Option<Failure>,
    /// The Call Window has keyboard focus.
    pub focused: bool,
}

/// What the Call Window says about the Call's audio, and so what its
/// Handoff button offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallAudio {
    /// Where the audio is now.
    pub location: AudioLocation,
    /// The Desktop has asked for the audio, and it hasn't come yet.
    pub moving_here: bool,
}

/// Where the Call in the Call Window is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallWindowPhase {
    /// Answered here, and not yet connected.
    Connecting,
    /// Placed here, and the other end isn't ringing yet.
    Calling,
    /// Placed here, and ringing at the other end.
    Ringing,
    /// Placing it failed; the notice says why, and the window closes shortly.
    NotPlaced,
    /// Connected: the timer runs from `active_since`.
    Live {
        active_since: Timestamp,
        wideband: bool,
        mute: Mute,
    },
    /// Over; the window closes shortly.
    Ended,
}

/// The Call Window's Mute control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mute {
    /// The microphone is on; Mute mutes it.
    Off,
    /// The microphone is muted; Mute unmutes it.
    On,
    /// Mute works only while the Audio Location is the Desktop.
    Unavailable,
}

impl State {
    /// What the Call Window's Mute control offers.
    fn mute(&self) -> Mute {
        if self.muted() {
            Mute::On
        } else if self.audio_location() == Some(AudioLocation::Desktop) {
            Mute::Off
        } else {
            Mute::Unavailable
        }
    }

    /// What the Desktop surfaces show.
    #[must_use]
    pub fn surfaces(&self) -> Surfaces {
        let banner = self
            .calls
            .iter()
            .find(|call| call.rings_here())
            .map(|call| BannerView {
                call: call.id,
                caller: call.caller.clone(),
                focused: self.focus == Focus::Banner,
                contact: self.contact(&call.caller),
            });
        // A refusal made beside a Call annotates it; a dial that failed on
        // its own has a Call Window of its own.
        let notice = self
            .notice
            .as_ref()
            .filter(|notice| notice.not_placed.is_none())
            .map(|notice| notice.failure)
            .or(self.audio_notice);
        let call_window = self
            .calls
            .iter()
            .find(|call| call.has_call_window() && self.unwanted != Some(call.id))
            .map(|call| CallWindowView {
                call: Some(call.id),
                caller: call.caller.clone(),
                contact: self.contact(&call.caller),
                phase: match (call.active_since, call.state) {
                    (Some(active_since), state) if state.is_connected() => CallWindowPhase::Live {
                        active_since,
                        wideband: self.codec.is_some_and(Codec::is_wideband),
                        mute: self.mute(),
                    },
                    (_, CallState::Dialing) => CallWindowPhase::Calling,
                    (_, CallState::Alerting) => CallWindowPhase::Ringing,
                    _ => CallWindowPhase::Connecting,
                },
                audio: call.state.is_live().then(|| CallAudio {
                    location: self.audio_location().unwrap_or(AudioLocation::Phone),
                    moving_here: self.moving_audio.is_some(),
                }),
                notice,
                focused: self.focus == Focus::CallWindow,
            })
            .or_else(|| {
                let dialling = self
                    .dialling
                    .as_ref()
                    .filter(|dialling| !dialling.cancelled)?;
                let caller = Caller::Number(dialling.number.clone());
                Some(CallWindowView {
                    call: None,
                    contact: self.contact(&caller),
                    caller,
                    audio: None,
                    phase: CallWindowPhase::Calling,
                    notice,
                    focused: self.focus == Focus::CallWindow,
                })
            })
            .or_else(|| {
                self.hung_up_dial.as_ref().map(|hung_up| CallWindowView {
                    call: None,
                    caller: hung_up.caller.clone(),
                    contact: self.contact(&hung_up.caller),
                    audio: None,
                    phase: CallWindowPhase::Ended,
                    notice: None,
                    focused: self.focus == Focus::CallWindow,
                })
            })
            .or_else(|| {
                let notice = self.notice.as_ref()?;
                let caller = notice.not_placed.clone()?;
                Some(CallWindowView {
                    call: None,
                    contact: self.contact(&caller),
                    caller,
                    audio: None,
                    phase: CallWindowPhase::NotPlaced,
                    notice: Some(notice.failure),
                    focused: self.focus == Focus::CallWindow,
                })
            })
            .or_else(|| {
                self.ended.as_ref().map(|ended| CallWindowView {
                    call: Some(ended.id),
                    caller: ended.caller.clone(),
                    contact: self.contact(&ended.caller),
                    audio: None,
                    phase: CallWindowPhase::Ended,
                    notice: None,
                    focused: self.focus == Focus::CallWindow,
                })
            });
        Surfaces {
            banner,
            call_window,
            dialer: self.dialer_view(),
        }
    }

    fn dialer_view(&self) -> Option<DialerView> {
        self.dialer.as_ref().map(|dialer| DialerView {
            query: dialer.query.clone(),
            rows: dialer.rows.clone(),
            selected: (!dialer.rows.is_empty()).then_some(dialer.selected),
            notice: dialer.notice,
            opened: dialer.opened,
        })
    }
}
