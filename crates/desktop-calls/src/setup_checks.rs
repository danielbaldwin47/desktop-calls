//! The setup checks: what the Desktop lacks for Calls, found and reported
//! to the Call state machine as `Input::DesktopChecked`, and the fixes it
//! asks for once the user consents.
//!
//! - Bluetooth: each `org.bluez.Adapter1`'s `Powered` and `PowerState`,
//!   followed as they change. On if any adapter is powered; blocked if one
//!   is `"off-blocked"` (rfkill); otherwise off. With no adapter at all
//!   nothing is reported: the Setup Window already says Bluetooth is off or
//!   missing.
//! - BlueZ's own hands-free: whether any object has `org.bluez.Telephony1`,
//!   which BlueZ's experimental `hfp` plugin adds for a Phone whose
//!   hands-free link it holds. Followed as it changes.
//! - Contacts support: whether `org.bluez.obex` has an owner on the session
//!   bus, or can be started there. Checked on `Effect::CheckDesktop`.
//! - Audio: the user's `WirePlumber` drop-ins (`wireplumber.rs`). Checked on
//!   `Effect::CheckDesktop`.
//!
//! Turning Bluetooth on sets the first adapter's `Powered`. Setting up audio
//! writes the drop-in, then restarts `wireplumber.service` through the
//! systemd user manager and waits, at most `RESTART_JOB_DEADLINE`, for that
//! job's `JobRemoved` signal: the reply to `RestartUnit` only says the job
//! is queued, so only a result of `done` counts as restarted.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::time::Duration;

use desktop_calls_core::{BluetoothPower, DesktopCheck, DesktopFix, FixFailure, Input};
use futures_channel::mpsc::{UnboundedSender, unbounded};
use futures_util::StreamExt as _;
use futures_util::future::{Either, select};
use glib::MainContext;
use zbus::fdo::DBusProxy;
use zbus::zvariant::Variant;
use zbus::{Connection, OwnedObjectPath, OwnedVariant};

use crate::logging::EventKind;
use crate::object_manager::{self, ObjectEvent, WatchError, Watched};
use crate::wireplumber::{self, Settings};

const BLUEZ: &str = "org.bluez";
const ADAPTER: &str = "org.bluez.Adapter1";
const TELEPHONY: &str = "org.bluez.Telephony1";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const OBEX: &str = "org.bluez.obex";
const SYSTEMD: &str = "org.freedesktop.systemd1";
const SYSTEMD_PATH: &str = "/org/freedesktop/systemd1";
const SYSTEMD_MANAGER: &str = "org.freedesktop.systemd1.Manager";
const WIREPLUMBER: &str = "wireplumber.service";
/// How long the restart job may take to end before the audio counts as not
/// restarted. `WirePlumber` comes back in about a second.
const RESTART_JOB_DEADLINE: Duration = Duration::from_secs(10);

/// What BlueZ says of the Desktop's adapters and its own hands-free.
#[derive(Default)]
struct Bluez {
    /// Each adapter's `Powered` and `PowerState`, by path.
    adapters: BTreeMap<String, (bool, String)>,
    /// Objects with BlueZ's own hands-free.
    telephony: BTreeSet<String>,
    /// BlueZ has listed its objects since it last appeared.
    listed: bool,
}

impl Bluez {
    fn power(&self) -> Option<BluetoothPower> {
        if self.adapters.is_empty() {
            return None;
        }
        Some(if self.adapters.values().any(|(powered, _)| *powered) {
            BluetoothPower::On
        } else if self
            .adapters
            .values()
            .any(|(_, state)| state == "off-blocked")
        {
            BluetoothPower::Blocked
        } else {
            BluetoothPower::Off
        })
    }

    /// The first adapter that is off.
    fn adapter_off(&self) -> Option<String> {
        self.adapters
            .iter()
            .find(|(_, (powered, _))| !powered)
            .map(|(path, _)| path.clone())
    }

    fn event(&mut self, event: ObjectEvent) {
        match event {
            ObjectEvent::Added { path, interfaces } => {
                if let Some(adapter) = interfaces.get(ADAPTER) {
                    let entry = self.adapters.entry(path.clone()).or_default();
                    update(entry, adapter.iter());
                }
                if interfaces.contains_key(TELEPHONY) {
                    self.telephony.insert(path);
                }
            }
            ObjectEvent::Changed {
                path,
                interface,
                changed,
            } if interface == ADAPTER => {
                if let Some(entry) = self.adapters.get_mut(&path) {
                    update(entry, changed.iter());
                }
            }
            ObjectEvent::Changed { .. } => {}
            ObjectEvent::Removed { path, interfaces } => {
                if interfaces.iter().any(|interface| interface == ADAPTER) {
                    self.adapters.remove(&path);
                }
                if interfaces.iter().any(|interface| interface == TELEPHONY) {
                    self.telephony.remove(&path);
                }
            }
            ObjectEvent::ServiceGone => *self = Self::default(),
            ObjectEvent::Listed => self.listed = true,
        }
    }
}

fn update<'a>(
    entry: &mut (bool, String),
    properties: impl Iterator<Item = (&'a String, &'a OwnedVariant)>,
) {
    for (name, value) in properties {
        match name.as_str() {
            "Powered" => {
                if let Ok(powered) = bool::try_from(value) {
                    entry.0 = powered;
                }
            }
            "PowerState" => {
                if let Ok(state) = <&str>::try_from(value) {
                    state.clone_into(&mut entry.1);
                }
            }
            _ => {}
        }
    }
}

/// Finds what the Desktop lacks, and puts it right when asked.
#[derive(Clone)]
pub struct SetupChecks {
    system: Connection,
    session: Connection,
    inputs: UnboundedSender<Input>,
    bluez: Rc<RefCell<Bluez>>,
    /// What checking again last found: Contacts support, and the audio.
    last_checked: Rc<RefCell<(Option<DesktopCheck>, Option<DesktopCheck>)>>,
}

impl SetupChecks {
    /// Starts following BlueZ, and checks the rest once.
    pub fn start(system: Connection, session: Connection, inputs: UnboundedSender<Input>) -> Self {
        let checks = Self {
            system,
            session,
            inputs,
            bluez: Rc::default(),
            last_checked: Rc::default(),
        };
        MainContext::default().spawn_local(checks.clone().follow_bluez());
        checks.check_again();
        checks
    }

    async fn follow_bluez(self) {
        let (sender, mut events) = unbounded();
        let watching = async {
            let watched = Watched {
                service: BLUEZ,
                root: "/",
                nested_manager: None,
            };
            object_manager::watch(&self.system, watched, &sender).await?;
            drop(sender);
            Ok::<_, WatchError>(())
        };
        let reporting = async {
            let mut reported = (None, None);
            while let Some(event) = events.next().await {
                let now = {
                    let mut bluez = self.bluez.borrow_mut();
                    bluez.event(event);
                    bluez
                        .listed
                        .then(|| (bluez.power(), Some(!bluez.telephony.is_empty())))
                };
                let Some(now) = now else {
                    continue;
                };
                if let Some(power) = now.0
                    && reported.0 != now.0
                {
                    self.report(DesktopCheck::Bluetooth(power), true);
                }
                if let Some(on) = now.1
                    && reported.1 != now.1
                {
                    self.report(DesktopCheck::BluezHandsFree(on), true);
                }
                reported = now;
            }
            Ok(())
        };
        if let Err(error) = futures_util::future::try_join(watching, reporting).await {
            tracing::warn!(
                event_kind = %EventKind::WatchFailed,
                error_kind = error.as_str(),
                "Stopped following BlueZ for the setup checks"
            );
        }
    }

    /// Tells the Call state machine; logged as news only when the finding
    /// `changed`, so checking again every few seconds stays quiet.
    fn report(&self, check: DesktopCheck, changed: bool) {
        if changed {
            tracing::info!(
                event_kind = %EventKind::DesktopChecked,
                check = ?check,
                "A setup check found {check:?}"
            );
        } else {
            tracing::debug!(
                event_kind = %EventKind::DesktopChecked,
                check = ?check,
                "A setup check found {check:?} again"
            );
        }
        let _ = self.inputs.unbounded_send(Input::DesktopChecked(check));
    }

    /// Checks again what can't be followed as it changes.
    pub fn check_again(&self) {
        let checks = self.clone();
        MainContext::default().spawn_local(async move {
            let contacts = DesktopCheck::ContactsSupport(contacts_support(&checks.session).await);
            let audio = audio_set_up().map(DesktopCheck::AudioSetUp);
            let changed = {
                let mut last = checks.last_checked.borrow_mut();
                let changed = (last.0 != Some(contacts), audio.is_some() && last.1 != audio);
                *last = (Some(contacts), audio.or(last.1));
                changed
            };
            checks.report(contacts, changed.0);
            if let Some(audio) = audio {
                checks.report(audio, changed.1);
            }
        });
    }

    /// Puts right what the user asked for.
    pub fn fix(&self, fix: DesktopFix) {
        let checks = self.clone();
        MainContext::default().spawn_local(async move {
            let outcome = match fix {
                DesktopFix::TurnOnBluetooth => checks.turn_on_bluetooth().await,
                DesktopFix::SetUpAudio => checks.set_up_audio().await,
            };
            let input = match outcome {
                Ok(()) => {
                    tracing::info!(
                        event_kind = %EventKind::DesktopFixed,
                        fix = ?fix,
                        "Put right: {fix:?}"
                    );
                    Input::FixDone(fix)
                }
                Err((failure, detail)) => {
                    tracing::warn!(
                        event_kind = %EventKind::DesktopFixFailed,
                        error_kind = failure_kind(failure),
                        detail = %detail,
                        "Couldn't put right: {fix:?}"
                    );
                    Input::FixFailed(failure)
                }
            };
            let _ = checks.inputs.unbounded_send(input);
        });
    }

    async fn turn_on_bluetooth(&self) -> Result<(), (FixFailure, String)> {
        let failed = |detail: String| (FixFailure::BluetoothNotTurnedOn, detail);
        let adapter = self
            .bluez
            .borrow()
            .adapter_off()
            .ok_or_else(|| failed("no adapter is off".to_owned()))?;
        self.system
            .call_method(
                Some(BLUEZ),
                adapter.as_str(),
                Some(PROPERTIES),
                "Set",
                &(ADAPTER, "Powered", Variant::from(true)),
            )
            .await
            .map(drop)
            .map_err(|error| failed(error.to_string()))
    }

    async fn set_up_audio(&self) -> Result<(), (FixFailure, String)> {
        let not_saved = |detail: String| (FixFailure::AudioNotSaved, detail);
        let folder = wireplumber::drop_in_folder()
            .ok_or_else(|| not_saved("no HOME or XDG_CONFIG_HOME".to_owned()))?;
        Settings::read(&folder)
            .and_then(|settings| settings.write_set_up())
            .map_err(|error| not_saved(error.to_string()))?;
        self.restart_wireplumber()
            .await
            .map_err(|detail| (FixFailure::AudioNotRestarted, detail))
    }

    /// Restarts `WirePlumber` and waits for the job to end.
    async fn restart_wireplumber(&self) -> Result<(), String> {
        let error = |error: zbus::Error| error.to_string();
        let manager = zbus::Proxy::new(&self.session, SYSTEMD, SYSTEMD_PATH, SYSTEMD_MANAGER)
            .await
            .map_err(error)?;
        // systemd sends a job's end only to clients that subscribed, and the
        // signal can come before the reply: listen first.
        manager.call_method("Subscribe", &()).await.map_err(error)?;
        let result = match manager.receive_signal("JobRemoved").await {
            Ok(ended) => match select(
                Box::pin(restart_job_result(&manager, ended)),
                Box::pin(glib::timeout_future(RESTART_JOB_DEADLINE)),
            )
            .await
            {
                Either::Left((result, _)) => result,
                Either::Right(_) => Err(format!(
                    "the restart job didn't end within {} s",
                    RESTART_JOB_DEADLINE.as_secs()
                )),
            },
            Err(listening) => Err(listening.to_string()),
        };
        // The restart is over either way; a failure to unsubscribe changes
        // nothing the user sees.
        let _ = manager.call_method("Unsubscribe", &()).await;
        result
    }
}

/// Asks systemd to restart `WirePlumber`, then waits for that job's end in
/// `ended`: restarted if its result is `done`, otherwise why not.
async fn restart_job_result(
    manager: &zbus::Proxy<'_>,
    mut ended: zbus::proxy::SignalStream<'_>,
) -> Result<(), String> {
    let reply = manager
        .call_method("RestartUnit", &(WIREPLUMBER, "replace"))
        .await
        .map_err(|error| error.to_string())?;
    let job: OwnedObjectPath = reply
        .body()
        .deserialize()
        .map_err(|error| error.to_string())?;
    while let Some(signal) = ended.next().await {
        let Ok((_, removed, _, result)) = signal
            .body()
            .deserialize::<(u32, OwnedObjectPath, String, String)>()
        else {
            continue;
        };
        if removed == job {
            return if result == "done" {
                Ok(())
            } else {
                Err(format!("the restart job ended as {result:?}"))
            };
        }
    }
    Err("the session bus closed".to_owned())
}

fn failure_kind(failure: FixFailure) -> &'static str {
    match failure {
        FixFailure::BluetoothNotTurnedOn => "bluetooth-not-turned-on",
        FixFailure::AudioNotSaved => "audio-not-saved",
        FixFailure::AudioNotRestarted => "audio-not-restarted",
        FixFailure::CallInProgress => "call-in-progress",
    }
}

/// Whether the Desktop's Bluetooth file-transfer service runs, or can be
/// started. A bus that can't say counts as having it: no Problem is better
/// than a wrong one.
async fn contacts_support(session: &Connection) -> bool {
    let Ok(bus) = DBusProxy::new(session).await else {
        return true;
    };
    let Ok(name) = OBEX.try_into() else {
        return true;
    };
    if bus.name_has_owner(name).await.unwrap_or(true) {
        return true;
    }
    bus.list_activatable_names()
        .await
        .map_or(true, |names| names.iter().any(|name| name.as_str() == OBEX))
}

/// Whether the audio is set up for Calls, or `None` if the settings can't
/// be read.
fn audio_set_up() -> Option<bool> {
    let folder = wireplumber::drop_in_folder()?;
    match Settings::read(&folder) {
        Ok(settings) => Some(settings.set_up()),
        Err(error) => {
            tracing::warn!(
                event_kind = %EventKind::DesktopChecked,
                error_kind = "audio-settings-unreadable",
                detail = %error,
                "Can't read the audio settings in {}",
                folder.display()
            );
            None
        }
    }
}
