//! The Call state machine: one function from (state, input) to (new state,
//! effects). Every policy decision lives here.

use crate::call::{
    AudioLocation, Call, CallId, CallState, Caller, Codec, Direction, Request, Timestamp, Transport,
};
use crate::connection::{ConnectionState, Phone};
use crate::messages::Failure;

/// How long the Call Window shows "Call ended" before it closes.
const CALL_ENDED_MS: u32 = 2_000;

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
    /// The Phone has a Call it didn't have before.
    CallAdded {
        call: CallId,
        state: CallState,
        caller: Caller,
        at: Timestamp,
    },
    /// A Call moved on in its life.
    CallChanged {
        call: CallId,
        state: CallState,
        at: Timestamp,
    },
    /// A Call is over, however it ended.
    CallEnded { call: CallId },
    /// The Call audio link between Phone and Desktop changed.
    TransportChanged(Transport),
    /// The audio link's codec changed; `None` is the Phone's 0, "no link".
    CodecChanged(Option<Codec>),
    /// The user asked to answer this Call on the Desktop.
    Answer(CallId),
    /// The user asked to decline this ringing Call.
    Decline(CallId),
    /// The user asked to hang up this Call.
    HangUp(CallId),
    /// The Phone refused, or never answered, this command.
    PhoneCommandFailed(PhoneCommand),
    /// A timer started by [`Effect::StartTimer`] ran out.
    TimerFired(Timer),
}

/// What the Phone link can be asked to do with Calls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PhoneCommand {
    /// Answer this ringing Call.
    Answer(CallId),
    /// Hang up this Call; for a ringing Call, that declines it.
    HangUp(CallId),
    /// Bring the live Call's audio to the Desktop.
    MoveAudioToDesktop,
    /// Keep the next Call's audio on the Phone unless the Desktop asks for it.
    KeepAudioOnPhone,
}

/// A timer the state machine can ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Timer {
    /// Close the Call Window that shows this ended Call.
    CloseCallWindow(CallId),
}

/// Something the shell adapters must do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Follow the Phone with this address: report when it connects and
    /// disconnects for Calls.
    AttachPhone { address: String },
    /// Stop following the Phone.
    DetachPhone,
    /// Ask the Phone to do something. The Phone commands of one step run in
    /// order, and stop at the first that fails.
    Phone(PhoneCommand),
    /// Play the Desktop ringtone until told to stop.
    StartRinging,
    /// Stop the Desktop ringtone.
    StopRinging,
    /// Report [`Input::TimerFired`] with `timer` after `after_ms`.
    StartTimer { timer: Timer, after_ms: u32 },
    /// Tell the user something went wrong.
    ReportError(Failure),
}

/// What the Desktop surfaces show, worked out from the state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Surfaces {
    pub banner: Option<BannerView>,
    pub call_window: Option<CallWindowView>,
}

/// The Incoming Call Banner's content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BannerView {
    pub call: CallId,
    pub caller: Caller,
}

/// The Call Window's content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallWindowView {
    pub call: CallId,
    pub caller: Caller,
    pub phase: CallWindowPhase,
}

/// Where the Call in the Call Window is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallWindowPhase {
    /// Answered or placed, and not yet connected.
    Connecting,
    /// Connected: the timer runs from `active_since`.
    Live {
        active_since: Timestamp,
        audio_location: AudioLocation,
        wideband: bool,
    },
    /// Over; the window closes shortly.
    Ended,
}

/// An ended Call the Call Window still shows.
#[derive(Clone, Debug, PartialEq, Eq)]
struct EndedCall {
    id: CallId,
    caller: Caller,
}

/// Everything the Call state machine knows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    phone: Option<Phone>,
    connection: ConnectionState,
    calls: Vec<Call>,
    transport: Transport,
    /// The Phone's latest codec report, whatever the transport is doing.
    reported_codec: Option<Codec>,
    /// The codec the Desktop's audio negotiated during the current Calls.
    codec: Option<Codec>,
    ended: Option<EndedCall>,
}

impl State {
    /// The Phone the Desktop works with, once one is paired.
    #[must_use]
    pub fn phone(&self) -> Option<&Phone> {
        self.phone.as_ref()
    }

    #[must_use]
    pub fn connection_state(&self) -> &ConnectionState {
        &self.connection
    }

    /// The Phone's Calls, oldest first.
    #[must_use]
    pub fn calls(&self) -> &[Call] {
        &self.calls
    }

    /// Whether the Desktop ringtone should be playing: a Call rings here and
    /// no other Call is live, so a waiting Call never rings over a live one.
    #[must_use]
    pub fn ringing(&self) -> bool {
        self.calls.iter().any(Call::rings_here)
            && !self.calls.iter().any(|call| call.state.blocks_ringing())
    }

    /// Where the Calls' audio is, while there are any.
    #[must_use]
    pub fn audio_location(&self) -> Option<AudioLocation> {
        if self.calls.is_empty() {
            None
        } else if self.transport == Transport::Active {
            Some(AudioLocation::Desktop)
        } else {
            Some(AudioLocation::Phone)
        }
    }

    /// The codec the Desktop's Call audio negotiated, kept for as long as
    /// there are Calls. A codec report while no audio link is up (the
    /// Phone's refused attempts, or its 0 while idle) is not a change.
    #[must_use]
    pub fn codec(&self) -> Option<Codec> {
        self.codec
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
            });
        let call_window = self
            .calls
            .iter()
            .find(|call| call.has_call_window())
            .map(|call| CallWindowView {
                call: call.id,
                caller: call.caller.clone(),
                phase: match call.active_since {
                    Some(active_since) if call.state.is_connected() => CallWindowPhase::Live {
                        active_since,
                        audio_location: self.audio_location().unwrap_or(AudioLocation::Phone),
                        wideband: self.codec.is_some_and(Codec::is_wideband),
                    },
                    _ => CallWindowPhase::Connecting,
                },
            })
            .or_else(|| {
                self.ended.as_ref().map(|ended| CallWindowView {
                    call: ended.id,
                    caller: ended.caller.clone(),
                    phase: CallWindowPhase::Ended,
                })
            });
        Surfaces {
            banner,
            call_window,
        }
    }

    fn is_our_phone(&self, address: &str) -> bool {
        self.phone
            .as_ref()
            .is_some_and(|phone| phone.address.eq_ignore_ascii_case(address))
    }

    fn call_mut(&mut self, id: CallId) -> Option<&mut Call> {
        self.calls.iter_mut().find(|call| call.id == id)
    }

    fn has_desktop_audio(&self) -> bool {
        !self.calls.is_empty() && self.transport.is_up()
    }
}

/// Works out the new state and the effects that follow from `input`.
#[must_use]
pub fn step(state: State, input: Input) -> (State, Vec<Effect>) {
    let was_ringing = state.ringing();
    let (state, mut effects) = decide(state, input);
    match (was_ringing, state.ringing()) {
        (false, true) => effects.insert(0, Effect::StartRinging),
        (true, false) => effects.insert(0, Effect::StopRinging),
        _ => {}
    }
    (state, effects)
}

fn decide(mut state: State, input: Input) -> (State, Vec<Effect>) {
    let mut effects = Vec::new();
    match input {
        // Desktop Calls works with one Phone: the first one paired.
        Input::PhonePaired(phone) if state.phone.is_none() => {
            effects.push(Effect::AttachPhone {
                address: phone.address.clone(),
            });
            state.phone = Some(phone);
            state.connection = ConnectionState::Connecting;
        }
        Input::PhoneUnpaired { address } if state.is_our_phone(&address) => {
            return (State::default(), vec![Effect::DetachPhone]);
        }
        Input::PhoneConnected { address } if state.is_our_phone(&address) => {
            state.connection = ConnectionState::Connected;
            // A crash or a dropped link mid-Call must never leave the next
            // Call's audio jumping to the Desktop.
            if !state.has_desktop_audio() {
                effects.push(Effect::Phone(PhoneCommand::KeepAudioOnPhone));
            }
        }
        Input::PhoneDisconnected { address }
            if state.is_our_phone(&address) && state.connection == ConnectionState::Connected =>
        {
            state.connection = ConnectionState::Disconnected;
            state.transport = Transport::Idle;
        }
        Input::CallAdded {
            call,
            state: call_state,
            caller,
            at,
        } => {
            if state.calls.iter().all(|known| known.id != call) {
                if state.ended.as_ref().is_some_and(|ended| ended.id == call) {
                    state.ended = None;
                }
                state.calls.push(Call {
                    id: call,
                    direction: Direction::from_first_state(call_state),
                    state: call_state,
                    caller,
                    active_since: call_state.is_connected().then_some(at),
                    request: Request::None,
                });
            }
        }
        Input::CallChanged {
            call,
            state: call_state,
            at,
        } => {
            if let Some(known) = state.call_mut(call) {
                known.state = call_state;
                if call_state.is_connected() && known.active_since.is_none() {
                    known.active_since = Some(at);
                }
            }
        }
        Input::CallEnded { call } => end_call(&mut state, call, &mut effects),
        Input::TransportChanged(transport) => {
            state.transport = transport;
            if transport.is_up() && state.reported_codec.is_some() {
                state.codec = state.reported_codec;
            }
        }
        Input::CodecChanged(codec) => {
            state.reported_codec = codec;
            if codec.is_some() && state.transport.is_up() {
                state.codec = codec;
            }
        }
        Input::Answer(call) => answer(&mut state, call, &mut effects),
        Input::Decline(call) => decline(&mut state, call, &mut effects),
        Input::HangUp(call) => hang_up(&mut state, call, &mut effects),
        Input::PhoneCommandFailed(command) => command_failed(&mut state, &command, &mut effects),
        Input::TimerFired(Timer::CloseCallWindow(call)) => {
            if state.ended.as_ref().is_some_and(|ended| ended.id == call) {
                state.ended = None;
            }
        }
        Input::PhonePaired(_)
        | Input::PhoneUnpaired { .. }
        | Input::PhoneConnected { .. }
        | Input::PhoneDisconnected { .. } => {}
    }
    (state, effects)
}

fn answer(state: &mut State, call: CallId, effects: &mut Vec<Effect>) {
    match state.call_mut(call) {
        Some(known) if known.rings_here() => {
            known.request = Request::Answer;
            effects.push(Effect::Phone(PhoneCommand::Answer(call)));
            effects.push(Effect::Phone(PhoneCommand::MoveAudioToDesktop));
        }
        Some(known) if known.request == Request::Answer => {}
        Some(_) => effects.push(Effect::ReportError(Failure::CallNotRinging)),
        None => effects.push(Effect::ReportError(Failure::NoSuchCall)),
    }
}

fn decline(state: &mut State, call: CallId, effects: &mut Vec<Effect>) {
    match state.call_mut(call) {
        Some(known) if known.rings_here() => {
            known.request = Request::Decline;
            effects.push(Effect::Phone(PhoneCommand::HangUp(call)));
        }
        Some(known) if known.request == Request::Decline => {}
        Some(_) => effects.push(Effect::ReportError(Failure::CallNotRinging)),
        None => effects.push(Effect::ReportError(Failure::NoSuchCall)),
    }
}

fn hang_up(state: &mut State, call: CallId, effects: &mut Vec<Effect>) {
    match state.call_mut(call) {
        Some(known) if matches!(known.request, Request::HangUp | Request::Decline) => {}
        Some(known) => {
            known.request = if known.rings_here() {
                Request::Decline
            } else {
                Request::HangUp
            };
            effects.push(Effect::Phone(PhoneCommand::HangUp(call)));
        }
        None => effects.push(Effect::ReportError(Failure::NoSuchCall)),
    }
}

fn end_call(state: &mut State, call: CallId, effects: &mut Vec<Effect>) {
    let Some(index) = state.calls.iter().position(|known| known.id == call) else {
        return;
    };
    let ended = state.calls.remove(index);
    if ended.has_call_window() {
        state.ended = Some(EndedCall {
            id: ended.id,
            caller: ended.caller,
        });
        effects.push(Effect::StartTimer {
            timer: Timer::CloseCallWindow(call),
            after_ms: CALL_ENDED_MS,
        });
    }
    if state.calls.is_empty() {
        state.codec = None;
        effects.push(Effect::Phone(PhoneCommand::KeepAudioOnPhone));
    }
}

fn command_failed(state: &mut State, command: &PhoneCommand, effects: &mut Vec<Effect>) {
    match *command {
        PhoneCommand::Answer(call) => {
            if let Some(known) = state.call_mut(call)
                && known.request == Request::Answer
            {
                known.request = Request::None;
            }
            effects.push(Effect::ReportError(Failure::AnswerFailed));
            if !state.has_desktop_audio() {
                effects.push(Effect::Phone(PhoneCommand::KeepAudioOnPhone));
            }
        }
        PhoneCommand::HangUp(call) => {
            if let Some(known) = state.call_mut(call)
                && matches!(known.request, Request::HangUp | Request::Decline)
            {
                known.request = Request::None;
            }
            effects.push(Effect::ReportError(Failure::HangUpFailed));
        }
        PhoneCommand::MoveAudioToDesktop => {
            effects.push(Effect::ReportError(Failure::AudioNotMoved));
        }
        // The shell has logged it; the next Call end or connect tries again.
        PhoneCommand::KeepAudioOnPhone => {}
    }
}
