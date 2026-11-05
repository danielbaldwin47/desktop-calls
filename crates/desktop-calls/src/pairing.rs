//! Pairing a Phone through BlueZ for the Setup Window (ADR-0004's BlueZ
//! Device API, plus the adapter and the agent manager):
//!
//! - While the Setup Window is open, the Desktop looks for Phones
//!   (`StartDiscovery`, classic Bluetooth only), is discoverable and
//!   pairable, so a Phone's "Pair new device" screen lists it, and is the
//!   default pairing agent, with the `DisplayYesNo` capability: BlueZ asks
//!   it whether the six-digit code shows on both screens. Once the window
//!   closes, all of that stops. Being discoverable lasts BlueZ's
//!   `DiscoverableTimeout` at most, so it is asked again every
//!   [`REASSERT_DISCOVERABLE`].
//! - `Device1.Pair` pairs the Phone the user chose; `Trusted` lets it
//!   connect without asking each time; `ConnectProfile` asks a Phone that
//!   connected without hands-free for hands-free alone.
//!
//! The agent has no methods for other kinds of pairing (PIN codes, passkeys
//! to type, pairing without a code): BlueZ asks for those only of devices
//! without a screen or without Secure Simple Pairing, and fails such a
//! pairing when the method is missing. A Phone pairs by comparing codes.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use desktop_calls_core::{Effect, Input, PairingFailure, Phone};
use futures_channel::mpsc::UnboundedSender;
use futures_channel::oneshot;
use glib::MainContext;
use zbus::fdo::{ObjectManagerProxy, PropertiesProxy};
use zbus::{Connection, DBusError, ObjectPath, OwnedObjectPath, OwnedVariant, Variant, interface};

use crate::bluez;
use crate::logging::EventKind;

const SERVICE: &str = "org.bluez";
const ADAPTER: &str = "org.bluez.Adapter1";
const DEVICE: &str = "org.bluez.Device1";
const AGENT_MANAGER: &str = "org.bluez.AgentManager1";
const AGENT_MANAGER_PATH: &str = "/org/bluez";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
/// Where the Desktop's pairing agent is, on the system bus.
const AGENT_PATH: &str = "/io/github/danielbaldwin47/DesktopCalls/agent";
/// The Hands-Free Audio Gateway profile: the Phone's side of hands-free.
const HANDS_FREE_AUDIO_GATEWAY_UUID: &str = "0000111f-0000-1000-8000-00805f9b34fb";
/// How often being discoverable is asked for again while the window is
/// open: well inside BlueZ's default `DiscoverableTimeout`, 180 s.
const REASSERT_DISCOVERABLE: Duration = Duration::from_secs(60);

/// The pairing code BlueZ asked about last, until the user answers.
type PendingCode = Arc<Mutex<Option<oneshot::Sender<bool>>>>;

/// Carries out the Setup Window's effects on BlueZ.
pub struct Pairing {
    system: Connection,
    inputs: UnboundedSender<Input>,
    pending: PendingCode,
    /// Which opening of the Setup Window is current: a start still under
    /// way when the window closes undoes itself.
    opening: Rc<Cell<u32>>,
}

impl Pairing {
    pub fn new(system: Connection, inputs: UnboundedSender<Input>) -> Self {
        Self {
            system,
            inputs,
            pending: PendingCode::default(),
            opening: Rc::default(),
        }
    }

    /// Carries out one of the Setup Window's effects; others are ignored.
    pub fn carry_out(&self, effect: Effect) {
        let (system, inputs) = (self.system.clone(), self.inputs.clone());
        match effect {
            Effect::StartSetup => {
                self.opening.set(self.opening.get() + 1);
                let (opening, pending) = (self.opening.clone(), self.pending.clone());
                MainContext::default().spawn_local(async move {
                    start(&system, &inputs, &opening, pending).await;
                });
            }
            Effect::StopSetup => {
                self.opening.set(self.opening.get() + 1);
                answer(&self.pending, false);
                MainContext::default().spawn_local(async move { stop(&system).await });
            }
            Effect::AnswerPairingCode { confirm } => answer(&self.pending, confirm),
            Effect::PairPhone { address } => {
                MainContext::default().spawn_local(async move {
                    if let Err(failure) = pair(&system, &address).await {
                        let _ = inputs.unbounded_send(Input::PairingFailed(failure));
                    }
                });
            }
            Effect::TrustPhone { address } => {
                MainContext::default().spawn_local(async move { trust(&system, &address).await });
            }
            Effect::ConnectHandsFree { address } => {
                MainContext::default().spawn_local(async move {
                    if !connect_hands_free(&system, &address).await {
                        let _ = inputs.unbounded_send(Input::HandsFreeRefused);
                    }
                });
            }
            _ => {}
        }
    }
}

/// Gives the user's answer to the pairing code BlueZ asked about, if any.
fn answer(pending: &PendingCode, confirm: bool) {
    let sender = pending
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    if let Some(sender) = sender {
        let _ = sender.send(confirm);
    }
}

/// Registers the agent, looks for Phones and becomes discoverable, then
/// says how the Desktop shows on Phones.
async fn start(
    system: &Connection,
    inputs: &UnboundedSender<Input>,
    opening: &Rc<Cell<u32>>,
    pending: PendingCode,
) {
    let this_opening = opening.get();
    let Some(adapter) = powered_adapter(system).await else {
        setup_failed("bluetooth-unavailable", "BlueZ has no powered adapter");
        let _ = inputs.unbounded_send(Input::SetupUnavailable);
        return;
    };
    let agent = Agent {
        inputs: inputs.clone(),
        pending,
    };
    // Already there from an earlier opening, if its stop hadn't finished.
    let _ = system.object_server().at(AGENT_PATH, agent).await;
    let agent_path = (ObjectPath::from_static_str_unchecked(AGENT_PATH),);
    for (member, body) in [
        (
            "RegisterAgent",
            call(
                system,
                AGENT_MANAGER,
                "RegisterAgent",
                &(agent_path.0.clone(), "DisplayYesNo"),
            )
            .await,
        ),
        (
            "RequestDefaultAgent",
            call(system, AGENT_MANAGER, "RequestDefaultAgent", &agent_path).await,
        ),
    ] {
        if let Err(error) = body {
            setup_failed(member, &error.to_string());
        }
    }
    let filter = HashMap::from([("Transport", Variant::from("bredr"))]);
    for (member, outcome) in [
        (
            "SetDiscoveryFilter",
            adapter_call(system, &adapter, "SetDiscoveryFilter", &(filter,)).await,
        ),
        (
            "StartDiscovery",
            adapter_call(system, &adapter, "StartDiscovery", &()).await,
        ),
        (
            "Pairable",
            set(system, &adapter, ADAPTER, "Pairable", true).await,
        ),
        (
            "Discoverable",
            set(system, &adapter, ADAPTER, "Discoverable", true).await,
        ),
    ] {
        if let Err(error) = outcome {
            setup_failed(member, &error.to_string());
        }
    }
    let desktop_name = adapter_alias(system, &adapter).await;
    tracing::info!(
        event_kind = %EventKind::SetupStarted,
        "Looking for Phones, discoverable as {desktop_name:?}"
    );
    if opening.get() != this_opening {
        // The window closed meanwhile.
        stop(system).await;
        return;
    }
    let _ = inputs.unbounded_send(Input::SetupReady { desktop_name });
    keep_discoverable(system.clone(), adapter, opening.clone(), this_opening);
}

/// Asks to stay discoverable until the window closes.
fn keep_discoverable(system: Connection, adapter: String, opening: Rc<Cell<u32>>, this: u32) {
    MainContext::default().spawn_local(async move {
        loop {
            glib::timeout_future(REASSERT_DISCOVERABLE).await;
            if opening.get() != this {
                return;
            }
            if let Err(error) = set(&system, &adapter, ADAPTER, "Discoverable", true).await {
                setup_failed("Discoverable", &error.to_string());
            }
        }
    });
}

/// Stops looking for Phones, stops being discoverable, and leaves pairing
/// to whoever else is BlueZ's agent.
async fn stop(system: &Connection) {
    if let Some(adapter) = powered_adapter(system).await {
        // Not looking already is fine.
        let _ = adapter_call(system, &adapter, "StopDiscovery", &()).await;
        if let Err(error) = set(system, &adapter, ADAPTER, "Discoverable", false).await {
            setup_failed("Discoverable", &error.to_string());
        }
    }
    let agent_path = (ObjectPath::from_static_str_unchecked(AGENT_PATH),);
    let _ = call(system, AGENT_MANAGER, "UnregisterAgent", &agent_path).await;
    let _ = system.object_server().remove::<Agent, _>(AGENT_PATH).await;
    tracing::info!(
        event_kind = %EventKind::SetupStopped,
        "Stopped looking for Phones and being discoverable"
    );
}

/// `Device1.Pair` on the Phone with `address`. Its code comes through the
/// agent; once paired, BlueZ's `Paired` says so.
async fn pair(system: &Connection, address: &str) -> Result<(), PairingFailure> {
    let Ok(Some(device)) = bluez::find_device(system, address).await else {
        return Err(pairing_failed(
            address,
            PairingFailure::Unreachable,
            "no such device",
        ));
    };
    match system
        .call_method(Some(SERVICE), device.as_str(), Some(DEVICE), "Pair", &())
        .await
    {
        Ok(_) => Ok(()),
        Err(zbus::Error::MethodError(name, message, _)) => {
            let detail = format!("{name}: {}", message.unwrap_or_default());
            let failure = match name.as_str() {
                // Paired already, from the Phone's side meanwhile.
                "org.bluez.Error.AlreadyExists" => return Ok(()),
                "org.bluez.Error.AuthenticationRejected"
                | "org.bluez.Error.AuthenticationCanceled" => PairingFailure::Rejected,
                "org.bluez.Error.AuthenticationTimeout"
                | "org.bluez.Error.ConnectionAttemptFailed" => PairingFailure::Unreachable,
                "org.bluez.Error.Failed" if detail.contains("page-timeout") => {
                    PairingFailure::Unreachable
                }
                _ => PairingFailure::Failed,
            };
            Err(pairing_failed(address, failure, &detail))
        }
        Err(error) => Err(pairing_failed(
            address,
            PairingFailure::Failed,
            &error.to_string(),
        )),
    }
}

/// Sets `Trusted` on the Phone with `address`, so it connects without
/// asking each time.
async fn trust(system: &Connection, address: &str) {
    let outcome = match bluez::find_device(system, address).await {
        Ok(Some(device)) => set(system, &device, DEVICE, "Trusted", true).await,
        Ok(None) => Err(zbus::Error::Failure("no such device".to_owned())),
        Err(error) => Err(error),
    };
    if let Err(error) = outcome {
        setup_failed("Trusted", &error.to_string());
    }
}

/// `ConnectProfile` for hands-free alone: whether the Phone agreed, or
/// had it up already.
async fn connect_hands_free(system: &Connection, address: &str) -> bool {
    let Ok(Some(device)) = bluez::find_device(system, address).await else {
        return false;
    };
    match system
        .call_method(
            Some(SERVICE),
            device.as_str(),
            Some(DEVICE),
            "ConnectProfile",
            &(HANDS_FREE_AUDIO_GATEWAY_UUID,),
        )
        .await
    {
        Ok(_) => true,
        Err(zbus::Error::MethodError(name, _, _))
            if matches!(
                name.as_str(),
                "org.bluez.Error.AlreadyConnected" | "org.bluez.Error.InProgress"
            ) =>
        {
            true
        }
        Err(error) => {
            tracing::info!(
                event_kind = %EventKind::HandsFreeRefused,
                error_kind = "hands-free-refused",
                phone_address = %address,
                detail = %error,
                "The Phone refused hands-free: its Phone calls setting is off"
            );
            false
        }
    }
}

/// The first adapter BlueZ has that is powered.
async fn powered_adapter(system: &Connection) -> Option<String> {
    let objects = ObjectManagerProxy::builder(system)
        .destination(SERVICE)
        .path("/")
        .build()
        .await
        .ok()?
        .get_managed_objects()
        .await
        .ok()?;
    objects.into_iter().find_map(|(path, interfaces)| {
        let adapter = interfaces
            .into_iter()
            .find(|(name, _)| name.as_str() == ADAPTER)?
            .1;
        let powered = adapter
            .get("Powered")
            .and_then(|powered| bool::try_from(powered.clone()).ok())
            .unwrap_or(false);
        powered.then(|| path.to_string())
    })
}

/// How the Desktop shows on Phones: the adapter's `Alias`.
async fn adapter_alias(system: &Connection, adapter: &str) -> String {
    let alias = async {
        PropertiesProxy::builder(system)
            .destination(SERVICE)
            .path(adapter.to_owned())
            .build()
            .await?
            .get(ADAPTER.try_into()?, "Alias")
            .await
            .map_err(zbus::Error::from)
    };
    alias
        .await
        .ok()
        .and_then(|alias| String::try_from(alias).ok())
        .unwrap_or_default()
}

async fn call<B>(system: &Connection, interface: &str, member: &str, body: &B) -> zbus::Result<()>
where
    B: zbus::export::serde::Serialize + zbus::zvariant::DynamicType,
{
    system
        .call_method(
            Some(SERVICE),
            AGENT_MANAGER_PATH,
            Some(interface),
            member,
            body,
        )
        .await
        .map(drop)
}

async fn adapter_call<B>(
    system: &Connection,
    adapter: &str,
    member: &str,
    body: &B,
) -> zbus::Result<()>
where
    B: zbus::export::serde::Serialize + zbus::zvariant::DynamicType,
{
    system
        .call_method(Some(SERVICE), adapter, Some(ADAPTER), member, body)
        .await
        .map(drop)
}

/// Sets one boolean property on BlueZ's object at `path`.
async fn set(
    system: &Connection,
    path: &str,
    interface: &str,
    name: &str,
    value: bool,
) -> zbus::Result<()> {
    system
        .call_method(
            Some(SERVICE),
            path,
            Some(PROPERTIES),
            "Set",
            &(interface, name, Variant::from(value)),
        )
        .await
        .map(drop)
}

fn setup_failed(step: &str, detail: &str) {
    tracing::warn!(
        event_kind = %EventKind::SetupFailed,
        error_kind = "bluez-refused",
        step,
        detail,
        "BlueZ refused part of setting up a Phone"
    );
}

fn pairing_failed(address: &str, failure: PairingFailure, detail: &str) -> PairingFailure {
    tracing::info!(
        event_kind = %EventKind::PairingFailed,
        error_kind = match failure {
            PairingFailure::Rejected => "pairing-rejected",
            PairingFailure::Unreachable => "pairing-unreachable",
            PairingFailure::Failed => "pairing-failed",
        },
        phone_address = %address,
        detail,
        "Pairing with the Phone didn't happen"
    );
    failure
}

/// The Desktop's pairing agent (`org.bluez.Agent1`): BlueZ asks it whether
/// a pairing's code shows on both screens.
struct Agent {
    inputs: UnboundedSender<Input>,
    pending: PendingCode,
}

/// The errors BlueZ's agent API names.
#[derive(Debug, DBusError)]
#[zbus(prefix = "org.bluez.Error")]
enum AgentError {
    Rejected(String),
    Canceled(String),
}

#[interface(interface = "org.bluez.Agent1")]
#[expect(
    clippy::unused_self,
    reason = "zbus calls Agent1's methods through self; letting the Phone just paired connect needs nothing from the agent"
)]
impl Agent {
    /// BlueZ no longer uses this agent: a code still waiting is turned down.
    fn release(&self) {
        answer(&self.pending, false);
    }

    /// Asks the user, through the Setup Window, whether `passkey` shows on
    /// the Phone too, and waits for the answer.
    async fn request_confirmation(
        &self,
        device: OwnedObjectPath,
        passkey: u32,
        #[zbus(connection)] system: &Connection,
    ) -> Result<(), AgentError> {
        let phone = phone_at(system, device.as_str()).await;
        let (sender, receiver) = oneshot::channel();
        // A request BlueZ asked earlier is turned down.
        if let Some(earlier) = self
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .replace(sender)
        {
            let _ = earlier.send(false);
        }
        let _ = self.inputs.unbounded_send(Input::PairingCode {
            phone,
            code: passkey,
        });
        match receiver.await {
            Ok(true) => Ok(()),
            Ok(false) => Err(AgentError::Rejected("Turned down".to_owned())),
            Err(_) => Err(AgentError::Canceled("Cancelled".to_owned())),
        }
    }

    /// A paired device not yet trusted connects a profile: the Phone just
    /// paired, before `Trusted` is set.
    fn authorize_service(&self, device: OwnedObjectPath, uuid: &str) {
        tracing::debug!(
            event_kind = %EventKind::ProfileAuthorized,
            device = %device.into_inner(),
            uuid,
            "Let the Phone just paired connect a profile"
        );
    }

    /// BlueZ gave up on the request: the Phone turned it down, or it timed
    /// out.
    fn cancel(&self) {
        let cancelled = self
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .is_some();
        if cancelled {
            let _ = self
                .inputs
                .unbounded_send(Input::PairingFailed(PairingFailure::Rejected));
        }
    }
}

/// The Phone at BlueZ's `device`: its name and address.
async fn phone_at(system: &Connection, device: &str) -> Phone {
    let properties: HashMap<String, OwnedVariant> = async {
        PropertiesProxy::builder(system)
            .destination(SERVICE)
            .path(device.to_owned())
            .build()
            .await?
            .get_all(DEVICE.try_into()?)
            .await
            .map_err(zbus::Error::from)
    }
    .await
    .unwrap_or_default();
    let text = |name: &str| {
        properties
            .get(name)
            .and_then(|value| String::try_from(value.clone()).ok())
    };
    let address = text("Address").unwrap_or_default();
    Phone {
        name: text("Alias")
            .or_else(|| text("Name"))
            .unwrap_or_else(|| address.clone()),
        address,
    }
}
