//! The control interface: the app's one seam on the session bus, for the
//! CLI, tests, the keybinding and the Indicator.
//!
//! It publishes the Call state machine's state as properties with change
//! signals. Bus name, object path and interface are in `CODING_STANDARDS.md`.

use desktop_calls_core::State;
use zbus::object_server::InterfaceRef;
use zbus::{Connection, interface};

pub const BUS_NAME: &str = "io.github.danielbaldwin47.DesktopCalls";
pub const PATH: &str = "/io/github/danielbaldwin47/DesktopCalls";

/// What the control interface publishes.
#[derive(Default)]
pub struct ControlInterface {
    state: State,
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
}

/// Publishes new state on the control interface.
pub struct Publisher {
    interface: InterfaceRef<ControlInterface>,
}

impl Publisher {
    /// Serves the control interface on `session` and claims its bus name.
    pub async fn serve(session: &Connection) -> zbus::Result<Self> {
        let server = session.object_server();
        server.at(PATH, ControlInterface::default()).await?;
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
        Ok(())
    }
}
