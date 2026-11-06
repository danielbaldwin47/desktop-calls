//! The systemd user manager stand-in: `org.freedesktop.systemd1` on the
//! scenario's session bus, with enough of systemd to run the installed
//! `desktop-calls.service` as the user's session runs it.
//!
//! It reads the unit file `just install` installed into the scenario's
//! home, and acts on what the unit says: it runs `ExecStart` with the
//! scenario's environment; for `RuntimeDirectory=` it makes the directory
//! (`RuntimeDirectoryMode=`), passes it as `RUNTIME_DIRECTORY`, and removes
//! it when the service stops, or also when it restarts, as
//! `RuntimeDirectoryPreserve=` says; with `Type=dbus` the service is active
//! once its own process owns `BusName=`; with `Restart=on-failure` it starts the service
//! again `RestartSec=` after it dies of a signal or fails.
//!
//! Over D-Bus it answers what the Real-Phone tools ask the real one:
//! `Manager.GetUnit`, `Manager.StartUnit` and `Manager.StopUnit` (each
//! returning a job at once and doing the work after), and the unit's
//! `ActiveState` and `Job`. `Job` names a start job until the start ends,
//! active or not, as systemd's does; stop jobs aren't shown there.
//!
//! The scenario's session bus runs with `--systemd-activation`, as the
//! user's does: a caller reaching for the app's name while the service is
//! down has the bus signal `Activator.ActivationRequest` here, and the
//! stand-in starts the unit, rather than the bus running the app itself.

use std::fs::{self, DirBuilder, OpenOptions};
use std::os::unix::fs::DirBuilderExt as _;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

use futures_util::StreamExt as _;

use zbus::OwnedObjectPath;
use zbus::fdo::DBusProxy;
use zbus::message::Type;
use zbus::names::BusName;
use zbus::{Connection, DBusError, MessageStream, interface};

use crate::builder::without_journal;

/// The one unit the stand-in knows.
pub const UNIT_NAME: &str = "desktop-calls.service";
/// Where `just install` puts it, under the home.
const UNIT_FILE: &str = ".local/share/systemd/user/desktop-calls.service";
const MANAGER_NAME: &str = "org.freedesktop.systemd1";
const MANAGER_PATH: &str = "/org/freedesktop/systemd1";
/// The unit's object path, escaped as systemd escapes it.
const UNIT_PATH: &str = "/org/freedesktop/systemd1/unit/desktop_2dcalls_2eservice";
/// How long a stopping service may take before it is killed, as systemd's
/// `TimeoutStopSec=` (shortened from its 90 s).
const STOP_TIMEOUT: Duration = Duration::from_secs(10);
/// How often the stand-in looks at the service's process and bus name.
const TICK: Duration = Duration::from_millis(20);

/// What the stand-in acts on, from the installed unit file.
struct UnitFile {
    exec_start: Vec<String>,
    runtime_directory: Option<String>,
    runtime_directory_mode: u32,
    preserve: Preserve,
    restart_on_failure: bool,
    restart_after: Duration,
    bus_name: Option<String>,
}

/// `RuntimeDirectoryPreserve=`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Preserve {
    No,
    Restart,
    Yes,
}

impl UnitFile {
    fn read(path: &Path) -> Option<Self> {
        let text = fs::read_to_string(path).ok()?;
        let value = |key: &str| {
            text.lines()
                .rev()
                .find_map(|line| line.trim().strip_prefix(key)?.strip_prefix('='))
                .map(str::trim)
        };
        let unit_type = value("Type").unwrap_or("simple");
        Some(Self {
            exec_start: value("ExecStart")?
                .split_whitespace()
                .map(str::to_owned)
                .collect(),
            runtime_directory: value("RuntimeDirectory").map(str::to_owned),
            runtime_directory_mode: value("RuntimeDirectoryMode")
                .and_then(|mode| u32::from_str_radix(mode, 8).ok())
                .unwrap_or(0o755),
            preserve: match value("RuntimeDirectoryPreserve") {
                Some("yes" | "true") => Preserve::Yes,
                Some("restart") => Preserve::Restart,
                _ => Preserve::No,
            },
            restart_on_failure: value("Restart") == Some("on-failure"),
            restart_after: value("RestartSec").map_or(Duration::from_millis(100), duration),
            bus_name: (unit_type == "dbus")
                .then(|| value("BusName").map(str::to_owned))
                .flatten(),
        })
    }
}

/// A systemd time span such as `1s` or `500ms`.
fn duration(text: &str) -> Duration {
    if let Some(milliseconds) = text.strip_suffix("ms") {
        return Duration::from_millis(milliseconds.trim().parse().unwrap_or(100));
    }
    let seconds = text.strip_suffix('s').unwrap_or(text).trim();
    Duration::from_secs_f64(seconds.parse().unwrap_or(0.1))
}

/// The stand-in, serving until dropped.
pub struct ServiceManager {
    unit: Arc<Unit>,
    _connection: Connection,
}

/// The unit and its process.
struct Unit {
    file: PathBuf,
    runtime: PathBuf,
    logs: PathBuf,
    environment: Vec<(&'static str, String)>,
    group: i32,
    session: Connection,
    running: Mutex<Running>,
    jobs: AtomicU32,
}

#[derive(Default)]
struct Running {
    active_state: &'static str,
    child: Option<Child>,
    /// Bumped by every start and stop, so a watcher of an older process
    /// gives up.
    generation: u64,
    /// Every `ActiveState` the unit has had, in order.
    history: Vec<&'static str>,
    /// Set once the scenario ends: nothing starts again.
    shut: bool,
    /// The start job still running, or 0.
    job: u32,
}

impl Running {
    /// Moves to `state`. Any state but `activating` ends a start job.
    fn set(&mut self, state: &'static str) {
        if state != "activating" {
            self.job = 0;
        }
        if self.active_state != state {
            self.active_state = state;
            self.history.push(state);
        }
    }
}

impl ServiceManager {
    /// Serves the stand-in on the session bus at `address`, for a unit
    /// installed under `home`, run with `environment` in process group
    /// `group`, logging to `logs`.
    pub fn start(
        address: &str,
        home: &Path,
        logs: &Path,
        environment: Vec<(&'static str, String)>,
        group: i32,
    ) -> zbus::Result<Self> {
        let session = zbus::block_on(zbus::connection::Builder::address(address).build())?;
        let runtime = environment
            .iter()
            .find(|(name, _)| *name == "XDG_RUNTIME_DIR")
            .map(|(_, value)| PathBuf::from(value))
            .unwrap_or_default();
        let unit = Arc::new(Unit {
            file: home.join(UNIT_FILE),
            runtime,
            logs: logs.to_owned(),
            environment,
            group,
            session,
            running: Mutex::new(Running {
                active_state: "inactive",
                ..Running::default()
            }),
            jobs: AtomicU32::new(0),
        });
        let connection = zbus::block_on(
            zbus::connection::Builder::address(address)
                .serve_at(MANAGER_PATH, Manager { unit: unit.clone() })
                .serve_at(UNIT_PATH, UnitObject { unit: unit.clone() })
                .name(MANAGER_NAME)
                .build(),
        )?;
        let requests = MessageStream::from(&connection);
        let requested = unit.clone();
        thread::spawn(move || zbus::block_on(start_when_asked(requests, &requested)));
        Ok(Self {
            unit,
            _connection: connection,
        })
    }

    /// Starts the service, as `systemctl --user start` does, and returns
    /// at once.
    pub fn start_unit(&self) {
        Unit::start(&self.unit);
    }

    /// Stops the service, as `systemctl --user stop` does, and returns once
    /// it has stopped.
    pub fn stop_unit(&self) {
        self.unit.stop();
    }

    /// Kills the service's process with SIGKILL, as a crash would.
    pub fn crash(&self) {
        if let Some(child) = self.unit.running().child.as_mut() {
            let _ = child.kill();
        }
    }

    /// The unit's `ActiveState` now.
    pub fn active_state(&self) -> &'static str {
        self.unit.running().active_state
    }

    /// Every `ActiveState` the unit has had, in order.
    pub fn history(&self) -> Vec<&'static str> {
        self.unit.running().history.clone()
    }

    /// The service's process ID, while it has a process.
    pub fn main_pid(&self) -> Option<u32> {
        self.unit.running().child.as_ref().map(Child::id)
    }
}

impl Drop for ServiceManager {
    fn drop(&mut self) {
        let mut running = self.unit.running();
        running.shut = true;
        running.generation += 1;
        if let Some(mut child) = running.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Unit {
    fn running(&self) -> MutexGuard<'_, Running> {
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn next_job(&self) -> u32 {
        self.jobs.fetch_add(1, Ordering::Relaxed) + 1
    }

    fn installed(&self) -> bool {
        self.file.is_file()
    }

    /// Starts the service unless it is running, and watches it.
    fn start(unit: &Arc<Self>) {
        let mut running = unit.running();
        if running.shut || running.active_state == "active" {
            running.job = 0;
            return;
        }
        if running.active_state == "activating" {
            return;
        }
        let Some(file) = UnitFile::read(&unit.file) else {
            running.set("failed");
            return;
        };
        running.generation += 1;
        let generation = running.generation;
        running.set("activating");
        let Some(child) = unit.spawn(&file) else {
            running.set("failed");
            return;
        };
        running.child = Some(child);
        drop(running);
        let unit = unit.clone();
        thread::spawn(move || unit.watch(&file, generation));
    }

    /// Runs `ExecStart`, with the runtime directory made.
    fn spawn(&self, file: &UnitFile) -> Option<Child> {
        let (program, arguments) = file.exec_start.split_first()?;
        let mut command: Command = without_journal(Path::new(program));
        command
            .process_group(self.group)
            .env_clear()
            .envs(self.environment.iter().cloned())
            .args(arguments);
        if let Some(directory) = self.runtime_directory(file) {
            DirBuilder::new()
                .recursive(true)
                .mode(file.runtime_directory_mode)
                .create(&directory)
                .ok()?;
            command.env("RUNTIME_DIRECTORY", directory);
        }
        let log = |name: &str| {
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.logs.join(name))
        };
        command
            .stdout(log("desktop-calls-service.stdout").ok()?)
            .stderr(log("desktop-calls-service.stderr").ok()?)
            .spawn()
            .ok()
    }

    fn runtime_directory(&self, file: &UnitFile) -> Option<PathBuf> {
        file.runtime_directory
            .as_ref()
            .map(|name| self.runtime.join(name))
    }

    fn remove_runtime_directory(&self, file: &UnitFile, preserved: bool) {
        if let Some(directory) = self.runtime_directory(file)
            && !preserved
        {
            let _ = fs::remove_dir_all(directory);
        }
    }

    /// Follows the process started as `generation`: active once it owns
    /// its bus name, started again if it fails and the unit says so.
    fn watch(&self, file: &UnitFile, generation: u64) {
        loop {
            thread::sleep(TICK);
            let mut running = self.running();
            if running.generation != generation || running.shut {
                return;
            }
            let exited = running
                .child
                .as_mut()
                .and_then(|child| child.try_wait().ok().flatten());
            if let Some(status) = exited {
                running.child = None;
                if file.restart_on_failure && !status.success() {
                    running.set("activating");
                    drop(running);
                    thread::sleep(file.restart_after);
                    self.remove_runtime_directory(file, file.preserve != Preserve::No);
                    let mut running = self.running();
                    if running.generation != generation || running.shut {
                        return;
                    }
                    let Some(child) = self.spawn(file) else {
                        running.set("failed");
                        return;
                    };
                    running.child = Some(child);
                    continue;
                }
                running.set(if status.success() {
                    "inactive"
                } else {
                    "failed"
                });
                drop(running);
                self.remove_runtime_directory(file, file.preserve == Preserve::Yes);
                return;
            }
            if running.active_state == "activating" {
                let pid = running.child.as_ref().map(Child::id);
                drop(running);
                if pid.is_some_and(|pid| self.owns_bus_name(file, pid)) {
                    let mut running = self.running();
                    if running.generation == generation && running.child.is_some() {
                        running.set("active");
                    }
                }
            }
        }
    }

    /// Whether the service's own process, `pid`, owns `BusName=`. Another
    /// process owning the name doesn't make the service active: systemd
    /// waits for the name to pass to the service, so a copy of the app left
    /// on the bus can't pass for a started service.
    fn owns_bus_name(&self, file: &UnitFile, pid: u32) -> bool {
        let Some(name) = &file.bus_name else {
            return true;
        };
        zbus::block_on(async {
            DBusProxy::new(&self.session)
                .await?
                .get_connection_unix_process_id(BusName::try_from(name.as_str())?)
                .await
                .map_err(zbus::Error::from)
        })
        .is_ok_and(|owner| owner == pid)
    }

    /// Stops the service: SIGTERM, then SIGKILL after [`STOP_TIMEOUT`].
    fn stop(&self) {
        let mut running = self.running();
        running.generation += 1;
        let Some(mut child) = running.child.take() else {
            if running.active_state != "failed" {
                running.set("inactive");
            }
            return;
        };
        running.set("deactivating");
        drop(running);
        let _ = Command::new("kill")
            .arg("-TERM")
            .arg(child.id().to_string())
            .status();
        let started = std::time::Instant::now();
        while !matches!(child.try_wait(), Ok(Some(_))) {
            if started.elapsed() > STOP_TIMEOUT {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
            thread::sleep(TICK);
        }
        if let Some(file) = UnitFile::read(&self.file) {
            self.remove_runtime_directory(&file, file.preserve == Preserve::Yes);
        }
        self.running().set("inactive");
    }
}

/// Starts the unit each time the session bus asks the manager to, as the
/// bus does when a caller reaches for the name of a service it has no owner
/// for: a bus run with `--systemd-activation` signals
/// `Activator.ActivationRequest` with the unit's name to
/// `org.freedesktop.systemd1`, and leaves the starting to the manager.
async fn start_when_asked(mut messages: MessageStream, unit: &Arc<Unit>) {
    while let Some(Ok(message)) = messages.next().await {
        let header = message.header();
        let asks = message.message_type() == Type::Signal
            && header.interface().is_some_and(|interface| {
                interface.as_str() == "org.freedesktop.systemd1.Activator"
            })
            && header
                .member()
                .is_some_and(|member| member.as_str() == "ActivationRequest");
        if asks && message.body().deserialize::<String>().as_deref() == Ok(UNIT_NAME) {
            Unit::start(unit);
        }
    }
}

/// A unit the stand-in doesn't know, as systemd names the error.
#[derive(Debug, DBusError)]
#[zbus(prefix = "org.freedesktop.systemd1")]
enum ManagerError {
    NoSuchUnit(String),
    Failed(String),
}

struct Manager {
    unit: Arc<Unit>,
}

#[interface(interface = "org.freedesktop.systemd1.Manager")]
impl Manager {
    fn get_unit(&self, name: &str) -> Result<OwnedObjectPath, ManagerError> {
        self.known(name)?;
        OwnedObjectPath::try_from(UNIT_PATH)
            .map_err(|error| ManagerError::Failed(error.to_string()))
    }

    /// Every `mode` acts as `"replace"`, the one the tools pass.
    fn start_unit(&self, name: &str, mode: &str) -> Result<OwnedObjectPath, ManagerError> {
        self.known(name)?;
        let _ = mode;
        let job = self.unit.next_job();
        // Before the reply, so a caller never sees the unit without its job.
        self.unit.running().job = job;
        let unit = self.unit.clone();
        // systemd runs the job from its event loop after it replies; a tick
        // later keeps a caller that reads ActiveState too soon from passing
        // by luck.
        thread::spawn(move || {
            thread::sleep(TICK);
            Unit::start(&unit);
        });
        job_path(job)
    }

    /// Every `mode` acts as `"replace"`, the one the tools pass.
    fn stop_unit(&self, name: &str, mode: &str) -> Result<OwnedObjectPath, ManagerError> {
        self.known(name)?;
        let _ = mode;
        let job = self.unit.next_job();
        let unit = self.unit.clone();
        thread::spawn(move || unit.stop());
        job_path(job)
    }
}

impl Manager {
    fn known(&self, name: &str) -> Result<(), ManagerError> {
        if name == UNIT_NAME && self.unit.installed() {
            Ok(())
        } else {
            Err(ManagerError::NoSuchUnit(format!("Unit {name} not loaded.")))
        }
    }
}

fn job_path(job: u32) -> Result<OwnedObjectPath, ManagerError> {
    OwnedObjectPath::try_from(format!("{MANAGER_PATH}/job/{job}"))
        .map_err(|error| ManagerError::Failed(error.to_string()))
}

struct UnitObject {
    unit: Arc<Unit>,
}

#[interface(interface = "org.freedesktop.systemd1.Unit")]
impl UnitObject {
    #[zbus(property)]
    fn active_state(&self) -> String {
        self.unit.running().active_state.to_owned()
    }

    /// The start job still running, as systemd's `(uo)`: its number and
    /// path, or `(0, "/")`.
    #[zbus(property)]
    fn job(&self) -> zbus::fdo::Result<(u32, OwnedObjectPath)> {
        let job = self.unit.running().job;
        let path = if job == 0 {
            OwnedObjectPath::try_from("/")
        } else {
            OwnedObjectPath::try_from(format!("{MANAGER_PATH}/job/{job}"))
        };
        path.map(|path| (job, path))
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
    }
}

/// The installed service, in a scenario that installs the app
/// ([`crate::Builder::installed`]).
impl crate::Harness {
    fn service_manager(&self) -> &ServiceManager {
        self.service_manager
            .as_ref()
            .expect("only a scenario that installs the app has the installed service")
    }

    /// Starts the installed service, as `systemctl --user start` does, and
    /// waits until systemd would call it active.
    ///
    /// # Panics
    ///
    /// If it isn't active within the startup deadline.
    pub fn start_service(&self) {
        self.service_manager().start_unit();
        self.wait_for_service("active");
    }

    /// Stops the installed service, as `systemctl --user stop` does, and
    /// waits until it has stopped.
    pub fn stop_service(&self) {
        self.service_manager().stop_unit();
    }

    /// Kills the installed service's process, as a crash would. The unit
    /// then restarts it as systemd would.
    pub fn kill_service(&self) {
        self.service_manager().crash();
    }

    /// The installed service's process ID, while it has a process.
    #[must_use]
    pub fn service_process_id(&self) -> Option<u32> {
        self.service_manager().main_pid()
    }

    /// The installed service's `ActiveState` now, as systemd names it:
    /// `"inactive"`, `"activating"`, `"active"`, `"deactivating"` or
    /// `"failed"`.
    #[must_use]
    pub fn service_state(&self) -> &'static str {
        self.service_manager().active_state()
    }

    /// Every `ActiveState` the installed service has had, in order,
    /// starting from the first after `"inactive"`.
    #[must_use]
    pub fn service_history(&self) -> Vec<&'static str> {
        self.service_manager().history()
    }

    /// Waits until the installed service is active again in a process
    /// other than `crashed`, its last.
    ///
    /// # Panics
    ///
    /// If it isn't within twice the startup deadline.
    pub fn wait_for_service_restart(&self, crashed: Option<u32>) {
        let started = std::time::Instant::now();
        while self.service_state() != "active" || self.service_process_id() == crashed {
            assert!(
                started.elapsed() < 2 * crate::STARTUP_DEADLINE,
                "the installed service wasn't restarted (states: {:?}); see {}",
                self.service_history(),
                self.directory()
                    .join("desktop-calls-service.stderr")
                    .display(),
            );
            thread::sleep(TICK);
        }
    }

    /// Waits until the installed service's `ActiveState` is `state`.
    ///
    /// # Panics
    ///
    /// If it isn't within the startup deadline.
    pub fn wait_for_service(&self, state: &str) {
        let started = std::time::Instant::now();
        while self.service_state() != state {
            assert!(
                started.elapsed() < crate::STARTUP_DEADLINE,
                "the installed service is {}, not {state}, after {:?} (states: {:?}); see {}{}",
                self.service_state(),
                crate::STARTUP_DEADLINE,
                self.service_history(),
                self.directory()
                    .join("desktop-calls-service.stderr")
                    .display(),
                self.timeline.render()
            );
            thread::sleep(TICK);
        }
    }
}
