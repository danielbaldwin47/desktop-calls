//! The control interface: the app's one seam on the session bus, for the
//! CLI, tests, the keybinding and the Indicator.
//!
//! It publishes the Call state machine's state as properties with change
//! signals, turns method calls into user intents for the state machine, and
//! signals failures as `Error(kind, message)` with the Messages text. Bus name,
//! object path and interface are in `CODING_STANDARDS.md`.

use std::collections::BTreeMap;

use desktop_calls_core::messages::{self, Failure};
use desktop_calls_core::{CallId, Caller, Input, State};
use futures_channel::mpsc::UnboundedSender;
use zbus::object_server::{InterfaceRef, SignalEmitter};
use zbus::{Connection, DBusError, interface};

pub const BUS_NAME: &str = "io.github.danielbaldwin47.DesktopCalls";
pub const PATH: &str = "/io/github/danielbaldwin47/DesktopCalls";
/// The prefix of the D-Bus error names for [`Refused`] requests.
pub const ERROR_PREFIX: &str = "io.github.danielbaldwin47.DesktopCalls1.Error.";

/// Each shown surface's key content, by surface name (`"banner"`,
/// `"call-window"`), as the surfaces report it.
pub type ShownSurfaces = BTreeMap<String, BTreeMap<String, String>>;

/// A request the Desktop refuses, as the D-Bus error a method returns: the
/// failure kind as the error name, and the Messages sentence.
#[derive(Debug, DBusError)]
#[zbus(prefix = "io.github.danielbaldwin47.DesktopCalls1.Error")]
pub enum Refused {
    NoLiveCall(String),
    MuteUnavailable(String),
    InvalidTones(String),
    Failed(String),
}

impl From<Failure> for Refused {
    fn from(failure: Failure) -> Self {
        let text = messages::failure(failure).text.to_owned();
        match failure {
            Failure::NoLiveCall => Self::NoLiveCall(text),
            Failure::MuteUnavailable => Self::MuteUnavailable(text),
            Failure::InvalidTones => Self::InvalidTones(text),
            _ => Self::Failed(text),
        }
    }
}

/// What the control interface publishes, and where its intents go.
pub struct ControlInterface {
    state: State,
    surfaces: ShownSurfaces,
    intents: UnboundedSender<Input>,
}

#[interface(interface = "io.github.danielbaldwin47.DesktopCalls1")]
impl ControlInterface {
    /// `"not-set-up"`, `"connecting"`, `"connected"` or `"disconnected"`.
    #[zbus(property)]
    fn connection_state(&self) -> String {
        self.state.connection_state().as_str().to_owned()
    }

    /// The Phone's name and Bluetooth address, or two empty strings while no
    /// Phone is set up.
    #[zbus(property)]
    fn phone(&self) -> (String, String) {
        self.state
            .phone()
            .map(|phone| (phone.name.clone(), phone.address.clone()))
            .unwrap_or_default()
    }

    /// The Phone's Calls, oldest first: id, direction (`"incoming"`,
    /// `"outgoing"`, `"unknown"`), state (`"ringing"`, `"waiting"`,
    /// `"dialing"`, `"alerting"`, `"active"`, `"held"`), number (empty if
    /// withheld), display name (empty until Contacts), and when the Call
    /// connected in milliseconds since the Unix epoch (0 until then).
    #[zbus(property)]
    fn calls(&self) -> Vec<(u32, String, String, String, String, u64)> {
        self.state
            .calls()
            .iter()
            .map(|call| {
                (
                    call.id().0,
                    call.direction().as_str().to_owned(),
                    call.state().as_str().to_owned(),
                    match call.caller() {
                        Caller::Number(number) => number.clone(),
                        Caller::Withheld => String::new(),
                    },
                    String::new(),
                    call.active_since().map_or(0, |since| since.0),
                )
            })
            .collect()
    }

    /// `"desktop"` or `"phone"`, or empty while there is no Call.
    #[zbus(property)]
    fn audio_location(&self) -> String {
        self.state
            .audio_location()
            .map_or("", |location| location.as_str())
            .to_owned()
    }

    /// Whether the Desktop is ringing.
    #[zbus(property)]
    fn ringing(&self) -> bool {
        self.state.ringing()
    }

    /// Whether the Desktop has muted its microphone for the Call.
    #[zbus(property)]
    fn muted(&self) -> bool {
        self.state.muted()
    }

    /// The codec the Desktop's Call audio negotiated (`"cvsd"`, `"msbc"`,
    /// `"lc3-swb"`, `"other"`, or empty) and whether it is wideband ("HD").
    #[zbus(property)]
    fn codec(&self) -> (String, bool) {
        self.state.codec().map_or((String::new(), false), |codec| {
            (codec.as_str().to_owned(), codec.is_wideband())
        })
    }

    /// The surfaces shown now, each with its key content as shown.
    #[zbus(property)]
    fn surfaces(&self) -> ShownSurfaces {
        self.surfaces.clone()
    }

    /// Answers the ringing Call with this id on the Desktop.
    fn answer(&self, call: u32) {
        let _ = self.intents.unbounded_send(Input::Answer(CallId(call)));
    }

    /// Declines the ringing Call with this id.
    fn decline(&self, call: u32) {
        let _ = self.intents.unbounded_send(Input::Decline(CallId(call)));
    }

    /// Hangs up the Call with this id.
    fn hang_up(&self, call: u32) {
        let _ = self.intents.unbounded_send(Input::HangUp(CallId(call)));
    }

    /// Mutes (`true`) or unmutes the Desktop's microphone. Mute works only
    /// while the Call's audio is on the Desktop; otherwise this fails with
    /// the reason.
    fn set_muted(&self, muted: bool) -> Result<(), Refused> {
        self.pass_on(Input::SetMuted(muted))
    }

    /// Plays keypad tones to the other party: digits, `*` and `#`, in order.
    /// Spaces, dashes, dots and brackets are left out; anything else fails.
    fn send_tones(&self, tones: String) -> Result<(), Refused> {
        self.pass_on(Input::SendTones(tones))
    }

    /// What the Desktop Calls keybinding does: gives keyboard focus to the
    /// Incoming Call Banner while a Call rings, otherwise to the Call Window
    /// while a Call is live. It never answers, declines or ends a Call.
    fn activate(&self) {
        let _ = self.intents.unbounded_send(Input::Activate);
    }

    /// Something the user asked for failed: the failure kind and the
    /// sentence to show.
    #[zbus(signal)]
    async fn error(emitter: &SignalEmitter<'_>, kind: &str, message: &str) -> zbus::Result<()>;
}

impl ControlInterface {
    /// Sends `intent` to the Call state machine, unless the state published
    /// now refuses it.
    fn pass_on(&self, intent: Input) -> Result<(), Refused> {
        if let Some(failure) = self.state.refusal(&intent) {
            return Err(failure.into());
        }
        let _ = self.intents.unbounded_send(intent);
        Ok(())
    }
}

/// Publishes new state on the control interface.
#[derive(Clone)]
pub struct Publisher {
    interface: InterfaceRef<ControlInterface>,
}

impl Publisher {
    /// Serves the control interface on `session`, sending its intents to
    /// `intents`, and claims its bus name.
    pub async fn serve(
        session: &Connection,
        intents: UnboundedSender<Input>,
    ) -> zbus::Result<Self> {
        let server = session.object_server();
        let control = ControlInterface {
            state: State::default(),
            surfaces: ShownSurfaces::new(),
            intents,
        };
        server.at(PATH, control).await?;
        let interface = server.interface::<_, ControlInterface>(PATH).await?;
        session.request_name(BUS_NAME).await?;
        Ok(Self { interface })
    }

    /// Publishes `state`, signalling each property that changed.
    pub async fn publish(&self, state: &State) -> zbus::Result<()> {
        let emitter = self.interface.signal_emitter();
        let mut published = self.interface.get_mut().await;
        let old = std::mem::replace(&mut published.state, state.clone());
        if old.connection_state() != state.connection_state() {
            published.connection_state_changed(emitter).await?;
        }
        if old.phone() != state.phone() {
            published.phone_changed(emitter).await?;
        }
        if old.calls() != state.calls() {
            published.calls_changed(emitter).await?;
        }
        if old.audio_location() != state.audio_location() {
            published.audio_location_changed(emitter).await?;
        }
        if old.ringing() != state.ringing() {
            published.ringing_changed(emitter).await?;
        }
        if old.muted() != state.muted() {
            published.muted_changed(emitter).await?;
        }
        if old.codec() != state.codec() {
            published.codec_changed(emitter).await?;
        }
        Ok(())
    }

    /// Publishes what the surfaces show now.
    pub async fn publish_surfaces(&self, surfaces: ShownSurfaces) -> zbus::Result<()> {
        let emitter = self.interface.signal_emitter();
        let mut published = self.interface.get_mut().await;
        if published.surfaces != surfaces {
            published.surfaces = surfaces;
            published.surfaces_changed(emitter).await?;
        }
        Ok(())
    }

    /// Signals `failure` as `Error(kind, message)`.
    pub async fn report_error(&self, failure: Failure) -> zbus::Result<()> {
        let message = messages::failure(failure);
        ControlInterface::error(
            self.interface.signal_emitter(),
            failure.as_str(),
            message.text,
        )
        .await
    }
}
