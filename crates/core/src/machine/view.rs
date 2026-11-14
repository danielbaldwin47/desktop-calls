//! What the Desktop surfaces show, worked out from the state.

use super::desktop::SetupProblemsView;
use super::setup::SetupWindowView;
use super::{Focus, State};
use crate::call::{AudioLocation, Call, CallId, CallState, Caller, Codec, Timestamp};
use crate::matching::{ContactMatch, DialerRow};
use crate::messages::Failure;

/// What the Desktop surfaces show, worked out from the state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Surfaces {
    pub banner: Option<BannerView>,
    pub call_window: Option<CallWindowView>,
    pub dialer: Option<DialerView>,
    pub setup_window: Option<SetupWindowView>,
    /// The Desktop's Problems, shown at the top of the open Setup Window.
    pub setup_problems: Option<SetupProblemsView>,
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
    pub caller: RingingCaller,
    /// The Banner has keyboard focus, so A answers and D declines.
    pub focused: bool,
    /// What the Banner offers for a Waiting Call in place of Answer, or
    /// `None` for the only Call ringing.
    pub waiting: Option<WaitingOptions>,
}

/// Who a ringing Call is from, as the Banner and the Indicator both say it.
/// Worked out once, here, so the two can't disagree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RingingCaller {
    /// The Call rang with no number, and the Phone may still report one,
    /// so nothing yet says the number is withheld.
    Identifying,
    Known {
        caller: Caller,
        /// The Contact the caller is, if Contacts say.
        contact: Option<ContactMatch>,
    },
}

/// The Call Window's content.
#[derive(Clone, Debug, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is an independent fact about what the Call Window shows, not a mode"
)]
pub struct CallWindowView {
    /// The Call shown, or `None` for a Call placed here that the Phone
    /// hasn't reported yet, or couldn't place.
    pub call: Option<CallId>,
    pub caller: Caller,
    /// The Contact the other party is, if Contacts say.
    pub contact: Option<ContactMatch>,
    pub phase: CallWindowPhase,
    /// The one line under who that says where the Call is heard, or how it
    /// went; `None` while a notice says it instead (contact lost, or the
    /// Call not placed).
    pub state_line: Option<StateLine>,
    /// Where the Call's audio is, while there is a Call to move it for.
    pub audio: Option<CallAudio>,
    /// A failure to explain beside the Call.
    pub notice: Option<Failure>,
    /// The Call Window has keyboard focus.
    pub focused: bool,
    /// The other connected Call, on hold, while there are two: listed below
    /// the live one, which Swap makes live again.
    pub held: Option<HeldCall>,
    /// A Call is on hold while a Waiting Call rings: Swap, Resume and Hang
    /// up aren't offered, as the Phone would answer or decline the Waiting
    /// Call instead (`AT+CHLD`).
    pub waiting_first: bool,
    /// The Desktop has lost contact with the Calls shown, which carry on on
    /// the Phone: nothing in the window acts on them, and Hang up says to
    /// hang up on the Phone.
    pub connection_lost: bool,
    /// The Call Window shows as the Call Pill: the caller, the timer, Mute
    /// and Hang up, in a strip in the bottom-right corner.
    pub collapsed: bool,
}

/// What the Banner offers for a Waiting Call, besides Decline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitingOptions {
    /// Hold & Answer and End & Answer: the user's other Call is connected.
    HoldOrEnd,
    /// End & Answer alone: the user's own Call is still being placed, and a
    /// Call nobody has answered can't be put on hold; or another Call is on
    /// hold already, and the Phone holds one Call at a time.
    End,
    /// Answer, which keeps the user's only other Call on hold: there is no
    /// live Call to hold or end.
    Answer,
}

/// A Call on hold behind the one the Call Window shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeldCall {
    pub call: CallId,
    pub caller: Caller,
    /// The Contact the other party is, if Contacts say.
    pub contact: Option<ContactMatch>,
}

/// What the Call Window says about the Call's audio, and so what its
/// Handoff button offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallAudio {
    /// Where the audio is now.
    pub location: AudioLocation,
    /// Where the Desktop has asked the audio to go, until that Handoff is
    /// over; `None` with no Handoff.
    pub moving: Option<AudioLocation>,
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
    /// Connected, and on hold, with no live Call beside it: Swap makes it
    /// live again.
    OnHold { active_since: Timestamp },
    /// Connected: the timer runs from `active_since`.
    Live {
        active_since: Timestamp,
        wideband: bool,
        mute: Mute,
    },
    /// Over; the window closes shortly.
    Ended,
}

/// What the Call Window's state line says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateLine {
    /// Answered here, and not yet connected.
    Connecting,
    /// Placed here, and the other end isn't ringing yet.
    Calling,
    /// Placed here, and ringing at the other end.
    Ringing,
    /// Live, with the audio here; `wideband` adds the HD mark.
    OnThisComputer { wideband: bool },
    /// Live, with the audio here and the microphone muted: the other party
    /// can't hear the user.
    Muted,
    /// Live, with the audio on the Phone: mute there, or move it here.
    OnYourPhone,
    /// The only Call, on hold.
    OnHold,
    /// Over, after `lasted` seconds connected; `None` if it never connected.
    Ended { lasted: Option<u64> },
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

    /// Where a Handoff under way is taking the audio, if one is.
    fn audio_moving_to(&self) -> Option<AudioLocation> {
        if self.moving_audio.is_some() {
            Some(AudioLocation::Desktop)
        } else if self.moving_to_phone.is_some() {
            Some(AudioLocation::Phone)
        } else {
            None
        }
    }

    /// The Call Window for the Call that lost the Bluetooth connection to
    /// the Phone, while it says so: the Call as it was, with nothing to do
    /// to it from here.
    fn lost_call_window(&self) -> Option<CallWindowView> {
        let connection_lost = self.connection_lost.as_ref()?;
        let lost = &connection_lost.call;
        Some(CallWindowView {
            call: Some(lost.id),
            caller: lost.caller.clone(),
            contact: self.contact(&lost.caller),
            phase: phase(lost, false, Mute::Unavailable),
            state_line: None,
            audio: None,
            notice: Some(connection_lost.notice),
            focused: self.focus == Focus::CallWindow,
            held: connection_lost.held.as_ref().map(|held| HeldCall {
                call: held.id,
                caller: held.caller.clone(),
                contact: self.contact(&held.caller),
            }),
            waiting_first: false,
            connection_lost: true,
            collapsed: self.form.collapsed(),
        })
    }

    /// Who the ringing `call` is from: while it has no number and the Phone
    /// may still report one, nobody yet.
    fn ringing_caller(&self, call: &Call) -> RingingCaller {
        if call.caller == Caller::Withheld && self.awaiting_number.contains(&call.id) {
            RingingCaller::Identifying
        } else {
            RingingCaller::Known {
                caller: call.caller.clone(),
                contact: self.contact(&call.caller),
            }
        }
    }

    /// The Call Window with `caller` and no Call in progress: one placed
    /// here on its way, ended, or not placed.
    fn callless_window(
        &self,
        call: Option<CallId>,
        caller: Caller,
        phase: CallWindowPhase,
        notice: Option<Failure>,
    ) -> CallWindowView {
        CallWindowView {
            call,
            contact: self.contact(&caller),
            caller,
            audio: None,
            state_line: state_line(phase, None),
            phase,
            notice,
            focused: self.focus == Focus::CallWindow,
            held: None,
            waiting_first: false,
            connection_lost: false,
            collapsed: self.form.collapsed(),
        }
    }

    /// What the Desktop surfaces show.
    #[must_use]
    pub fn surfaces(&self) -> Surfaces {
        // A Call ringing on a Phone out of reach can't be answered here.
        let banner = self
            .calls
            .iter()
            .filter(|_| !self.calls_out_of_reach())
            .find(|call| call.rings_here())
            .map(|call| BannerView {
                call: call.id,
                caller: self.ringing_caller(call),
                focused: self.focus == Focus::Banner,
                waiting: self
                    .waiting_call()
                    .filter(|waiting| waiting.id == call.id)
                    .map(|_| self.waiting_options()),
            });
        // A refusal made beside a Call annotates it; a dial that failed on
        // its own has a Call Window of its own.
        let notice = self
            .notice
            .as_ref()
            .filter(|notice| notice.not_placed.is_none())
            .map(|notice| notice.failure)
            .or(self.waiting_notice)
            .or(self.audio_notice);
        let call_window = self
            .lost_call_window()
            .or_else(|| self.calls_window(notice))
            .or_else(|| {
                let dialling = self
                    .dialling
                    .as_ref()
                    .filter(|dialling| !dialling.cancelled)?;
                Some(self.callless_window(
                    None,
                    Caller::Number(dialling.number.clone()),
                    CallWindowPhase::Calling,
                    notice,
                ))
            })
            .or_else(|| {
                self.hung_up_dial.as_ref().map(|hung_up| {
                    self.callless_window(None, hung_up.caller.clone(), CallWindowPhase::Ended, None)
                })
            })
            .or_else(|| {
                let notice = self.notice.as_ref()?;
                let caller = notice.not_placed.clone()?;
                Some(self.callless_window(
                    None,
                    caller,
                    CallWindowPhase::NotPlaced,
                    Some(notice.failure),
                ))
            })
            .or_else(|| {
                self.ended.as_ref().map(|ended| CallWindowView {
                    state_line: Some(StateLine::Ended {
                        lasted: ended.lasted,
                    }),
                    ..self.callless_window(
                        Some(ended.id),
                        ended.caller.clone(),
                        CallWindowPhase::Ended,
                        None,
                    )
                })
            });
        Surfaces {
            banner,
            call_window,
            dialer: self.dialer_view(),
            setup_window: self.setup_window_view(),
            setup_problems: self.setup_problems_view(),
        }
    }

    /// The Call Window for the Phone's Calls, with `notice` beside them.
    fn calls_window(&self, notice: Option<Failure>) -> Option<CallWindowView> {
        let (call, held) = self.call_window_calls()?;
        let phase = phase(
            call,
            self.codec.is_some_and(Codec::is_wideband),
            self.mute(),
        );
        let audio = call.state.is_live().then(|| CallAudio {
            location: self.audio_location().unwrap_or(AudioLocation::Phone),
            moving: self.audio_moving_to(),
        });
        Some(CallWindowView {
            call: Some(call.id),
            caller: call.caller.clone(),
            contact: self.contact(&call.caller),
            phase,
            state_line: state_line(phase, audio),
            audio,
            notice,
            focused: self.focus == Focus::CallWindow,
            held: held.map(|held| HeldCall {
                call: held.id,
                caller: held.caller.clone(),
                contact: self.contact(&held.caller),
            }),
            waiting_first: self.has_waiting_call() && self.has_held_call(),
            connection_lost: false,
            collapsed: self.form.collapsed(),
        })
    }

    /// The Calls the Call Window shows: the live one, or failing that the
    /// first it has, and the other Call on hold, if there is one.
    pub(super) fn call_window_calls(&self) -> Option<(&Call, Option<&Call>)> {
        let shown: Vec<&Call> = self
            .calls
            .iter()
            .filter(|call| call.has_call_window() && self.unwanted != Some(call.id))
            .collect();
        let call = shown
            .iter()
            .find(|call| call.state == CallState::Active)
            .or_else(|| shown.first())?;
        let held = shown
            .iter()
            .find(|other| other.id != call.id && other.state == CallState::Held);
        Some((call, held.copied()))
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

/// What the state line says of a Call the Phone has, in `phase`, with its
/// audio at `audio`.
fn state_line(phase: CallWindowPhase, audio: Option<CallAudio>) -> Option<StateLine> {
    Some(match phase {
        CallWindowPhase::Connecting => StateLine::Connecting,
        CallWindowPhase::Calling => StateLine::Calling,
        CallWindowPhase::Ringing => StateLine::Ringing,
        CallWindowPhase::OnHold { .. } => StateLine::OnHold,
        CallWindowPhase::Live { mute: Mute::On, .. } => StateLine::Muted,
        CallWindowPhase::Live { wideband, .. } => match audio.map(|audio| audio.location) {
            Some(AudioLocation::Desktop) => StateLine::OnThisComputer { wideband },
            Some(AudioLocation::Phone) | None => StateLine::OnYourPhone,
        },
        CallWindowPhase::NotPlaced => return None,
        CallWindowPhase::Ended => StateLine::Ended { lasted: None },
    })
}

/// Where `call` is, as the Call Window shows it.
fn phase(call: &Call, wideband: bool, mute: Mute) -> CallWindowPhase {
    match (call.active_since, call.state) {
        (Some(active_since), CallState::Held) => CallWindowPhase::OnHold { active_since },
        (Some(active_since), state) if state.is_connected() => CallWindowPhase::Live {
            active_since,
            wideband,
            mute,
        },
        (_, CallState::Dialing) => CallWindowPhase::Calling,
        (_, CallState::Alerting) => CallWindowPhase::Ringing,
        _ => CallWindowPhase::Connecting,
    }
}
