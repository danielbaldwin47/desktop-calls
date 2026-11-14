//! The Desktop's own conditions, which the setup checks look for: its
//! Bluetooth adapter's power, whether its Bluetooth file-transfer service
//! (`obexd`) is installed, BlueZ's own experimental hands-free, and the
//! systemd user manager that restarts `WirePlumber`. They stand beside the
//! Phone on the same private buses.
//!
//! - `--adapter-power off`: the adapter is off (`Powered` false,
//!   `PowerState` `"off"`); setting `Powered` true turns it on. `blocked`:
//!   rfkill keeps it off (`PowerState` `"off-blocked"`), and setting
//!   `Powered` fails with `org.bluez.Error.Blocked`, as BlueZ 5.87's
//!   `property_set_mode_complete` answers. Either way `StartDiscovery` and
//!   `Device1.Connect` fail with `org.bluez.Error.NotReady`, as BlueZ does
//!   while the adapter is down: `Connect`'s message is
//!   `"br-connection-adapter-not-powered"`, as the real Desktop logged it
//!   (ticket #213). A `Connect` paging when the adapter goes down fails the
//!   same way, which is inferred.
//! - The adapter going down while the Phone is connected drops the Phone,
//!   as going out of range does. It stays in range: when the adapter comes
//!   back on, it connects only when paged.
//! - `--without-obex`: nobody takes `org.bluez.obex` on the session bus.
//! - `--bluez-telephony`: the Phone's device has an `org.bluez.Telephony1`
//!   child, as BlueZ's experimental `hfp` plugin adds once it holds the
//!   Phone's hands-free link. Its path's last part is inferred.
//! - `org.freedesktop.systemd1` on the session bus answers `RestartUnit`:
//!   restarting `wireplumber.service` restarts the Telephony service as
//!   [`crate::Control::restart_telephony`] does, with the real Pixel's
//!   0.73 s gap. Each call is recorded. The call returns the job's path
//!   (ids count up from 7), and the job ends with a `JobRemoved` signal
//!   sent only to the clients that called `Subscribe`, as systemd's manual
//!   says: `done`, unless `--wireplumber-restart-job` says it ends `failed`
//!   or `timeout` (`WirePlumber` doesn't restart then), or never ends. The
//!   signals go out before the reply, an unrelated job's first.
//!
//! The control interface's `SimulatedDesktop1.SetAdapterPower` changes the
//! adapter's power as another app, or the user, would.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use zbus::message::Header;
use zbus::names::BusName;
use zbus::object_server::{ObjectServer, SignalEmitter};
use zbus::{Connection, ObjectPath, OwnedObjectPath, OwnedVariant, interface};

use crate::objects::{Interfaces, MethodCall, ObjectTree, Properties, Reply};
use crate::telephony::Signal;
use crate::{Bus, CONTROL_PATH, Control, SharedWorld, TelephonyConnection, World, lock};

const ADAPTER: &str = "org.bluez.Adapter1";
const ADAPTER_PATH: &str = "/org/bluez/hci0";
const TELEPHONY: &str = "org.bluez.Telephony1";
const SYSTEMD_PATH: &str = "/org/freedesktop/systemd1";
const SYSTEMD_MANAGER: &str = "org.freedesktop.systemd1.Manager";
pub const SYSTEMD_SERVICE: &str = "org.freedesktop.systemd1";
/// How long `WirePlumber` takes to come back, as on the real Desktop.
const WIREPLUMBER_GAP_MS: u32 = 730;
/// The Hands-Free Audio Gateway profile, which BlueZ's `hfp` plugin claims.
const HANDS_FREE_AUDIO_GATEWAY_UUID: &str = "0000111f-0000-1000-8000-00805f9b34fb";

/// The adapter's power.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum AdapterPower {
    #[default]
    On,
    Off,
    Blocked,
}

impl AdapterPower {
    fn power_state(self) -> &'static str {
        match self {
            Self::On => "on",
            Self::Off => "off",
            Self::Blocked => "off-blocked",
        }
    }
}

/// How the job that restarts `wireplumber.service` ends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum RestartJob {
    /// The unit restarted.
    #[default]
    Done,
    /// The unit failed to start again.
    Failed,
    /// The unit took too long to start again.
    Timeout,
    /// The job never ends.
    Never,
}

impl RestartJob {
    /// The result systemd gives the job's end, or none if it never ends.
    fn result(self) -> Option<&'static str> {
        match self {
            Self::Done => Some("done"),
            Self::Failed => Some("failed"),
            Self::Timeout => Some("timeout"),
            Self::Never => None,
        }
    }
}

/// The Desktop's conditions, from the command line.
#[derive(clap::Args)]
pub struct Conditions {
    /// The Desktop's Bluetooth adapter's power.
    #[arg(long, value_enum, default_value_t)]
    adapter_power: AdapterPower,
    /// The Desktop's Bluetooth file-transfer service isn't installed.
    #[arg(long)]
    without_obex: bool,
    /// BlueZ's experimental hands-free holds the Phone's hands-free link.
    #[arg(long)]
    bluez_telephony: bool,
    /// How the job that restarts `wireplumber.service` ends.
    #[arg(long, value_enum, default_value_t)]
    wireplumber_restart_job: RestartJob,
}

impl Conditions {
    /// Gives the adapter its power, and the Phone's device BlueZ's own
    /// hands-free if asked.
    pub fn apply(&self, bluez: &mut ObjectTree, device: &OwnedObjectPath) -> zbus::Result<()> {
        set_power(bluez, self.adapter_power)?;
        if self.bluez_telephony {
            let path = OwnedObjectPath::try_from(format!("{device}/telephony_ag1"))?;
            let mut telephony = Properties::new();
            telephony.insert(
                "UUID".into(),
                crate::bluez::text(HANDS_FREE_AUDIO_GATEWAY_UUID),
            );
            telephony.insert("State".into(), crate::bluez::text("connected"));
            bluez.insert(
                &path,
                Interfaces::from([(TELEPHONY.to_owned(), telephony)]),
                vec![crate::objects::PROPERTIES, crate::objects::INTROSPECTABLE],
            );
        }
        Ok(())
    }

    /// How the job that restarts `wireplumber.service` ends.
    pub fn restart_job(&self) -> RestartJob {
        self.wireplumber_restart_job
    }

    /// Whether the Desktop has its Bluetooth file-transfer service.
    pub fn has_obex(&self) -> bool {
        !self.without_obex
    }
}

fn set_power(bluez: &mut ObjectTree, power: AdapterPower) -> zbus::Result<Vec<Signal>> {
    let adapter = OwnedObjectPath::try_from(ADAPTER_PATH)?;
    let mut changed = Properties::new();
    let powered = OwnedVariant::from(power == AdapterPower::On);
    let state = crate::bluez::text(power.power_state());
    for (name, value) in [("Powered", powered), ("PowerState", state)] {
        let copy = value.try_clone()?;
        if bluez.set(&adapter, ADAPTER, name, value) {
            changed.insert(name.to_owned(), copy);
        }
    }
    Ok(if changed.is_empty() {
        Vec::new()
    } else {
        vec![Signal::PropertiesChanged {
            path: adapter,
            interface: ADAPTER,
            changed,
        }]
    })
}

/// Whether the adapter is on.
pub fn powered(world: &World) -> bool {
    OwnedObjectPath::try_from(ADAPTER_PATH)
        .ok()
        .and_then(|adapter| world.bluez.get(&adapter, ADAPTER, "Powered").cloned())
        .and_then(|powered| bool::try_from(powered).ok())
        .unwrap_or(false)
}

/// The error BlueZ answers while the adapter is down.
pub fn not_ready() -> Reply {
    Reply::Error("org.bluez.Error.NotReady", "Resource Not Ready".to_owned())
}

/// `Properties.Set` of the adapter's `Powered`: turns it on or off, unless
/// rfkill blocks it.
pub fn set_powered(world: &mut World, on: bool) -> (Reply, Vec<Signal>) {
    let blocked = OwnedObjectPath::try_from(ADAPTER_PATH)
        .ok()
        .and_then(|adapter| world.bluez.get(&adapter, ADAPTER, "PowerState").cloned())
        .and_then(|state| String::try_from(state).ok())
        .is_some_and(|state| state == "off-blocked");
    if blocked {
        return (
            Reply::Error(
                "org.bluez.Error.Blocked",
                "Blocked through rfkill".to_owned(),
            ),
            Vec::new(),
        );
    }
    let power = if on {
        AdapterPower::On
    } else {
        AdapterPower::Off
    };
    match set_power(&mut world.bluez, power) {
        Ok(signals) => (Reply::body(()), signals),
        Err(error) => (
            Reply::Error("org.bluez.Error.Failed", error.to_string()),
            Vec::new(),
        ),
    }
}

/// The error BlueZ answers `Device1.Connect` with while the adapter is down,
/// as the real Desktop logged it 84,162 times (ticket #213).
pub fn adapter_not_powered() -> Reply {
    Reply::Error("org.bluez.Error.NotReady", ADAPTER_NOT_POWERED.to_owned())
}

/// The message of [`adapter_not_powered`].
pub const ADAPTER_NOT_POWERED: &str = "br-connection-adapter-not-powered";

/// The control interface for the Desktop's conditions.
pub struct SimulatedDesktop {
    pub world: SharedWorld,
    pub system: Connection,
    pub telephony: TelephonyConnection,
}

#[interface(interface = "io.github.danielbaldwin47.DesktopCalls.SimulatedDesktop1")]
impl SimulatedDesktop {
    /// Turns the adapter `on` or `off`, or has rfkill block it
    /// (`blocked`), as another app or the user would.
    async fn set_adapter_power(&self, power: &str) -> zbus::fdo::Result<()> {
        let power = match power {
            "on" => AdapterPower::On,
            "off" => AdapterPower::Off,
            "blocked" => AdapterPower::Blocked,
            other => {
                return Err(zbus::fdo::Error::InvalidArgs(format!(
                    "no adapter power {other:?}"
                )));
            }
        };
        let (signals, dropped) = {
            let mut world = lock(&self.world);
            let signals = set_power(&mut world.bluez, power)?;
            // The adapter going down takes the Phone's connection with it.
            let dropped = if power == AdapterPower::On {
                None
            } else {
                world.obex.link_lost();
                crate::range::hands_free_down(&mut world)?
            };
            (signals, dropped)
        };
        for signal in signals {
            signal.emit(&self.system).await?;
        }
        if let Some(change) = dropped {
            crate::range::emit(&self.system, &self.telephony, change).await?;
        }
        Ok(())
    }
}

/// The first job id the manager hands out. Real ids count up from the
/// manager's start, so a job's path says nothing a client can rely on.
const FIRST_JOB_ID: u32 = 7;

/// The systemd user manager, as far as setting up audio uses it.
pub struct Systemd {
    world: SharedWorld,
    restart_job: RestartJob,
    /// The clients that called `Subscribe`, by unique name, once per call.
    subscribers: Mutex<Vec<String>>,
    next_job_id: AtomicU32,
}

impl Systemd {
    pub fn new(world: SharedWorld, restart_job: RestartJob) -> Self {
        Self {
            world,
            restart_job,
            subscribers: Mutex::new(Vec::new()),
            next_job_id: AtomicU32::new(FIRST_JOB_ID),
        }
    }

    /// Records that the manager was called.
    fn record(&self, member: &str, arguments: String) {
        lock(&self.world).recorded.push((
            Bus::Session,
            MethodCall {
                path: SYSTEMD_PATH.to_owned(),
                interface: SYSTEMD_MANAGER.to_owned(),
                member: member.to_owned(),
                arguments,
            },
        ));
    }

    /// The path of the job with `id`.
    fn job_path(id: u32) -> zbus::Result<OwnedObjectPath> {
        OwnedObjectPath::try_from(format!("{SYSTEMD_PATH}/job/{id}"))
    }

    /// Sends `JobRemoved` for job `id` to each client that called
    /// `Subscribe`, and to nobody else, as systemd's manual says
    /// (`org.freedesktop.systemd1(5)`).
    async fn send_job_removed(
        &self,
        emitter: &SignalEmitter<'_>,
        id: u32,
        unit: &str,
        result: &str,
    ) -> zbus::Result<()> {
        let job = Self::job_path(id)?;
        let subscribers = lock(&self.subscribers).clone();
        for subscriber in subscribers {
            let to = emitter
                .to_owned()
                .set_destination(BusName::try_from(subscriber)?);
            Self::job_removed(&to, id, job.as_ref(), unit, result).await?;
        }
        Ok(())
    }
}

#[interface(interface = "org.freedesktop.systemd1.Manager")]
impl Systemd {
    /// Has systemd send `JobRemoved` to this client.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "the interface macro hands the header over by value"
    )]
    fn subscribe(&self, #[zbus(header)] header: Header<'_>) {
        self.record("Subscribe", "()".to_owned());
        if let Some(sender) = header.sender() {
            lock(&self.subscribers).push(sender.to_string());
        }
    }

    /// Ends one `subscribe` of this client's.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "the interface macro hands the header over by value"
    )]
    fn unsubscribe(&self, #[zbus(header)] header: Header<'_>) -> zbus::fdo::Result<()> {
        self.record("Unsubscribe", "()".to_owned());
        let sender = header.sender().map(ToString::to_string);
        let mut subscribers = lock(&self.subscribers);
        let position = subscribers
            .iter()
            .position(|subscriber| Some(subscriber) == sender.as_ref());
        match position {
            Some(position) => {
                subscribers.remove(position);
                Ok(())
            }
            None => Err(zbus::fdo::Error::Failed(
                "org.freedesktop.systemd1.NotSubscribed: Caller is not subscribed".to_owned(),
            )),
        }
    }

    /// Restarts `name`: for `wireplumber.service`, the Telephony service
    /// restarts, unless the job is to end `failed` or `timeout`, or never.
    /// Returns the job's path.
    ///
    /// The job is fast: before the reply goes out, `JobRemoved` goes to the
    /// subscribers, first for an unrelated job that ended the other way,
    /// then for this one. A client that matches on anything but the job's
    /// own path, or that listens only once it has the reply, goes wrong.
    async fn restart_unit(
        &self,
        #[zbus(object_server)] server: &ObjectServer,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        name: String,
        mode: String,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        self.record("RestartUnit", format!("({name:?}, {mode:?})"));
        let job_ends = if name == "wireplumber.service" {
            self.restart_job
        } else {
            RestartJob::Done
        };
        if name == "wireplumber.service" && job_ends == RestartJob::Done {
            let control = server.interface::<_, Control>(CONTROL_PATH).await?;
            control
                .get()
                .await
                .restart_telephony(WIREPLUMBER_GAP_MS)
                .await?;
        }
        let unrelated_id = self.next_job_id.fetch_add(1, Ordering::Relaxed);
        let id = self.next_job_id.fetch_add(1, Ordering::Relaxed);
        let result = job_ends.result();
        let unrelated_result = if result == Some("done") {
            "failed"
        } else {
            "done"
        };
        self.send_job_removed(
            &emitter,
            unrelated_id,
            "unrelated.service",
            unrelated_result,
        )
        .await?;
        if let Some(result) = result {
            self.send_job_removed(&emitter, id, &name, result).await?;
        }
        Ok(Self::job_path(id)?)
    }

    /// A job ended, with `result`: `done`, `canceled`, `timeout`, `failed`,
    /// `dependency` or `skipped`.
    #[zbus(signal)]
    async fn job_removed(
        emitter: &SignalEmitter<'_>,
        id: u32,
        job: ObjectPath<'_>,
        unit: &str,
        result: &str,
    ) -> zbus::Result<()>;
}

/// Serves the Desktop's control interface and its systemd stand-in on
/// `builder`'s connection.
pub fn serve<'a>(
    builder: zbus::connection::Builder<'a>,
    world: &SharedWorld,
    (system, telephony): (&Connection, &TelephonyConnection),
    restart_job: RestartJob,
) -> zbus::connection::Builder<'a> {
    builder
        .serve_at(
            CONTROL_PATH,
            SimulatedDesktop {
                world: world.clone(),
                system: system.clone(),
                telephony: telephony.clone(),
            },
        )
        .serve_at(SYSTEMD_PATH, Systemd::new(world.clone(), restart_job))
        .name(SYSTEMD_SERVICE)
}
