//! Test harness for Desktop Calls, used only by tests.
//!
//! [`Harness::start`] gives one scenario a world of its own: private session
//! and system buses, a temporary home, the Simulated Phone, and the real
//! `desktop-calls` binary pointed at them through the standard D-Bus and XDG
//! variables only. Scenarios drive the Simulated Phone and observe the app
//! through its control interface and its CLI, waiting on observable state with
//! a deadline. Every scenario can run in parallel with every other.
//!
//! When a scenario fails, its directory is kept and its path printed. It
//! holds every process's log: the app's own log file, which the app writes
//! because the harness hides the systemd journal from it, and the standard
//! error of each process. With [`KEEP_SCENARIO_LOGS`] set, passing scenarios
//! keep theirs too.
//!
//! Every process a scenario starts dies with it, even when the test process
//! is killed: see `process_group.rs`.
//!
//! A scenario that asks for a screen also gets a compositor of its own, and
//! the app runs on it. Screenshots are kept under `target/screenshots/`.
//!
//! The binaries are found next to the test executable, so build them first;
//! `just test` does.

// The harness is test code compiled as a library, so clippy's exemption for
// `#[test]` code doesn't reach it. Like a `#[test]`, a setup step that fails
// must fail the scenario at once with a message saying which step it was.
#![expect(
    clippy::expect_used,
    reason = "test code: a failed setup step fails the scenario with its message"
)]

use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

mod app;
mod builder;
mod client;
mod control;
mod desktop;
mod hyprland;
mod input;
mod omarchy_quiet;
mod process_group;
mod redraw;
mod screen;
mod screencopy;
mod screenshots;
mod simulated_phone;
mod stand_ins;
mod timeline;
mod tray_icon;
pub mod vcards;

pub use app::{
    AppCall, AppCallHistoryEntry, AppContactsState, AppDialerRow, AppError, AppProblem, AppState,
    MissedCall, ShownSurfaces,
};
pub use builder::Builder;
pub use client::TestClient;
pub use control::{AppEvent, PendingDial, RunningCli};
pub use desktop::{Clipboard, Keyboard, Notifications, Pointer, Tray};
pub use input::Key;
pub use screen::{LAYOUT_SIZE, Region, SCREEN_SIZE, Screenshot};
pub use simulated_phone::{PhoneIdentity, RecordedMethodCall, SimulatedPhone};
pub use stand_ins::{
    BAR_COLOUR, BAR_HEIGHT, MenuEntry, Notification, OmarchyTheme, Theme, TrayItem,
};
pub use tray_icon::draw_tray_icon;

use futures_channel::oneshot;
use futures_util::StreamExt as _;
use futures_util::future::{Either, select};
use zbus::Connection;
use zbus::fdo::DBusProxy;
use zbus::names::BusName;

use crate::builder::without_journal;
use crate::process_group::ProcessGroup;
use crate::screen::Screen;
use crate::stand_ins::StandIns;
use crate::timeline::Timeline;

/// The app's control interface, as the contract in `CODING_STANDARDS.md` names it.
const APP_BUS_NAME: &str = "io.github.danielbaldwin47.DesktopCalls";
const APP_PATH: &str = "/io/github/danielbaldwin47/DesktopCalls";
const APP_INTERFACE: &str = "io.github.danielbaldwin47.DesktopCalls1";

/// The Simulated Phone's control interface.
const SIMULATED_PHONE_BUS_NAME: &str = "io.github.danielbaldwin47.DesktopCalls.SimulatedPhone";

/// How long a process may take to start and claim its bus name.
const STARTUP_DEADLINE: Duration = Duration::from_secs(10);

/// The app's desktop file, which `just install` installs.
const DESKTOP_FILE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../data/io.github.danielbaldwin47.DesktopCalls.desktop"
);
/// How many frames [`Harness::screenshot_when`] records ahead of the check
/// that judges them, at 25 MB each. Frames come only when the screen
/// changes, so a few is plenty; past this, the recording waits and the next
/// frame shows the screen as it is by then.
const FRAMES_AHEAD: usize = 8;

/// How long the screen may take to finish redrawing before an action. The
/// first frame of a new surface held the app for 0.6 s on a CI-like machine
/// (`docs/research/2026-11-04-ci-banner-latency.md`); this allows for a
/// loaded runner many times over.
const REDRAW_DEADLINE: Duration = Duration::from_secs(10);

/// The script `just install` runs.
const INSTALL_SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tools/install.py");

/// The environment variable that makes passing scenarios keep their
/// directories, as failing ones always do: `KEEP_SCENARIO_LOGS=1 just test`.
/// CI sets it when run by hand with `keep_scenario_logs`.
pub const KEEP_SCENARIO_LOGS: &str = "KEEP_SCENARIO_LOGS";

/// The journal's socket. While it is reachable the app logs there, so the
/// harness hides it from the app to get a log file per scenario.
const JOURNAL_SOCKET: &str = "/run/systemd/journal/socket";

/// One of a scenario's two private buses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bus {
    /// Where BlueZ lives.
    System,
    /// Where PipeWire's Telephony service and obexd live.
    Session,
}

/// One scenario's private world. Dropping it stops every process, and
/// removes the scenario's directory unless the scenario is failing.
pub struct Harness {
    directory: PathBuf,
    bin_directory: PathBuf,
    environment: Vec<(&'static str, String)>,
    phone: PhoneIdentity,
    audio_gateway_index: u32,
    session: Connection,
    session_bus: String,
    system: Connection,
    stand_ins: StandIns,
    screen: Option<Screen>,
    /// The stand-in for Omarchy's bar, if the scenario has one.
    bar: Option<TestClient>,
    /// The Hyprland stand-in, if the scenario runs under one.
    hyprland: Option<hyprland::Hyprland>,
    /// The process IDs of the test clients started, which draw on the
    /// screen as the app does. Some may have stopped since.
    test_clients: Mutex<Vec<u32>>,
    screenshot_directory: PathBuf,
    /// Every `Error` signal the app has sent, in order.
    app_errors: Arc<Mutex<Vec<AppError>>>,
    /// Every `MissedCall` signal the app has sent, in order.
    missed_calls: Arc<Mutex<Vec<MissedCall>>>,
    /// Every value the app's `Surfaces` property has signalled, in order.
    surfaces_seen: Arc<Mutex<Vec<ShownSurfaces>>>,
    // Stopped in reverse order of starting: the app, the Simulated Phone,
    // the compositor, then the buses.
    processes: Vec<(&'static str, Child)>,
    /// What happened, for the panic when a wait times out.
    timeline: Arc<Timeline>,
    /// Holds every process above and their children. Dropped last, so it
    /// kills whatever they left running.
    group: ProcessGroup,
}

impl Harness {
    /// Starts a scenario with no screen. See [`Builder::start`].
    ///
    /// # Panics
    ///
    /// If any process fails to start; the panic says which and where its log is.
    #[must_use]
    pub fn start() -> Self {
        Self::builder().start()
    }

    /// Chooses a screen and more before starting.
    pub fn builder() -> Builder {
        Builder::default()
    }
}

impl Harness {
    /// The address of the scenario's private session bus, for a test that
    /// acts as another Desktop client.
    #[must_use]
    pub fn session_bus_address(&self) -> &str {
        &self.session_bus
    }

    /// A command for any program on `PATH`, with this scenario's environment
    /// and nothing else, as a Desktop client would run.
    #[must_use]
    pub fn external_command(&self, program: &str) -> Command {
        let mut command = Command::new(program);
        self.group
            .adopt(&mut command)
            .env_clear()
            .envs(self.environment.iter().cloned());
        command
    }
}

impl Harness {
    /// This scenario's directory: the temporary home, the XDG directories,
    /// the buses and every log.
    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Who the Simulated Phone says it is in this scenario.
    #[must_use]
    pub fn phone(&self) -> &PhoneIdentity {
        &self.phone
    }

    /// The index of the first AudioGateway the Simulated Phone registers in
    /// this scenario (`agN`). It differs from run to run and is never 0.
    #[must_use]
    pub fn audio_gateway_index(&self) -> u32 {
        self.audio_gateway_index
    }
}

impl Harness {
    /// Stops the app as a crash would (SIGKILL), and starts it again in the
    /// same world: the same buses, home and Simulated Phone.
    ///
    /// # Panics
    ///
    /// If the app doesn't start again.
    pub fn restart_app(&mut self) {
        if let Some(index) = self
            .processes
            .iter()
            .position(|(name, _)| *name == "desktop-calls")
        {
            let (_, mut app) = self.processes.remove(index);
            let _ = app.kill();
            let _ = app.wait();
        }
        let left = zbus::block_on(within(STARTUP_DEADLINE, async {
            let bus = DBusProxy::new(&self.session).await?;
            let mut owner_changes = bus.receive_name_owner_changed().await?;
            let name = BusName::try_from(APP_BUS_NAME)?;
            while bus.name_has_owner(name.clone()).await? {
                owner_changes.next().await;
            }
            Ok::<_, zbus::Error>(())
        }));
        assert!(
            matches!(left, Some(Ok(()))),
            "the stopped app stayed on the bus: {left:?}{}",
            self.timeline.render()
        );
        self.start_app();
    }

    /// Starts the Simulated Phone, acting out `scenario` if there is one,
    /// and waits until it is on the bus.
    fn start_simulated_phone(&mut self, scenario: Option<&Path>) {
        let mut simulated_phone = self.command("simulated-phone");
        simulated_phone
            .arg("--name")
            .arg(&self.phone.name)
            .arg("--address")
            .arg(&self.phone.address)
            .arg("--first-audio-gateway-index")
            .arg(self.audio_gateway_index.to_string());
        if let Some(scenario) = scenario {
            simulated_phone.arg("--scenario").arg(scenario);
        }
        let simulated_phone = simulated_phone
            .stdout(self.log_file("simulated-phone.stdout"))
            .stderr(self.log_file("simulated-phone.stderr"))
            .spawn()
            .expect("start the Simulated Phone");
        self.processes.push(("simulated-phone", simulated_phone));
        self.wait_for_bus_name(SIMULATED_PHONE_BUS_NAME, "simulated-phone");
    }

    fn start_app(&mut self) {
        let app = self
            .app_command()
            .stdout(self.log_file("desktop-calls.stdout"))
            .stderr(self.log_file("desktop-calls.stderr"))
            .spawn()
            .expect("start desktop-calls");
        self.processes.push(("desktop-calls", app));
        self.wait_for_bus_name(APP_BUS_NAME, "desktop-calls");
    }

    /// Installs the app into the scenario's home with `tools/install.py`,
    /// which `just install` runs.
    fn install(&self) {
        let output = self
            // The system Python: a version manager's `python3` shim (mise's)
            // reads its configuration from the real home, which a scenario
            // hides.
            .external_command("/usr/bin/python3")
            .arg(INSTALL_SCRIPT)
            .arg(self.bin_directory.join("desktop-calls"))
            .output()
            .expect("run tools/install.py");
        assert!(
            output.status.success(),
            "tools/install.py failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// Whether the app's control interface is on the session bus now.
    ///
    /// # Panics
    ///
    /// If the bus doesn't answer.
    #[must_use]
    pub fn app_on_bus(&self) -> bool {
        zbus::block_on(async {
            DBusProxy::new(&self.session)
                .await?
                .name_has_owner(BusName::try_from(APP_BUS_NAME)?)
                .await
                .map_err(zbus::Error::from)
        })
        .expect("ask the session bus whether the app is on it")
    }

    /// Stops the app if the session bus started it: such an app is the
    /// bus's child, not the harness's.
    fn stop_activated_app(&self) {
        if self
            .processes
            .iter()
            .any(|(name, _)| *name == "desktop-calls")
        {
            return;
        }
        let pid = zbus::block_on(async {
            DBusProxy::new(&self.session)
                .await?
                .get_connection_unix_process_id(BusName::try_from(APP_BUS_NAME)?)
                .await
                .map_err(zbus::Error::from)
        });
        if let Ok(pid) = pid {
            let _ = Command::new("kill")
                .arg("-KILL")
                .arg(pid.to_string())
                .status();
        }
    }
}

impl Harness {
    /// A command for one of the workspace's binaries, with this scenario's
    /// environment and nothing else.
    fn command(&self, binary: &str) -> Command {
        let mut command = Command::new(self.bin_directory.join(binary));
        self.group
            .adopt(&mut command)
            .env_clear()
            .envs(self.environment.iter().cloned());
        command
    }

    /// The command that runs the app's long-running process, where it can't
    /// reach the journal, so it logs to the scenario's directory.
    fn app_command(&self) -> Command {
        let mut command = without_journal(&self.bin_directory.join("desktop-calls"));
        self.group
            .adopt(&mut command)
            .env_clear()
            .envs(self.environment.iter().cloned())
            .arg("service");
        command
    }

    fn log_file(&self, name: &str) -> fs::File {
        fs::File::create(self.directory.join(name)).expect("create a log file")
    }

    /// Waits until `name` has an owner on the session bus.
    fn wait_for_bus_name(&mut self, name: &'static str, process: &str) {
        let appeared = zbus::block_on(within(STARTUP_DEADLINE, async {
            let bus = DBusProxy::new(&self.session).await?;
            let mut owner_changes = bus.receive_name_owner_changed().await?;
            let bus_name = BusName::try_from(name)?;
            if bus.name_has_owner(bus_name.clone()).await? {
                return Ok::<_, zbus::Error>(());
            }
            while let Some(change) = owner_changes.next().await {
                if change.args()?.name() == &bus_name
                    && bus.name_has_owner(bus_name.clone()).await?
                {
                    return Ok(());
                }
            }
            Err(zbus::Error::Failure("NameOwnerChanged ended".to_owned()))
        }));
        if !matches!(appeared, Some(Ok(()))) {
            let exit = self
                .processes
                .iter_mut()
                .find(|(running, _)| *running == process)
                .and_then(|(_, child)| child.try_wait().ok().flatten());
            panic!(
                "{process} did not claim {name} within {STARTUP_DEADLINE:?} ({appeared:?}, exit status {exit:?}); see {}{}",
                self.directory.join(format!("{process}.stderr")).display(),
                self.timeline.render()
            );
        }
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.stop_activated_app();
        while let Some((_, mut child)) = self.processes.pop() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if thread::panicking() || keep_scenario_logs() {
            eprintln!("Kept this scenario's logs in {}", self.directory.display());
        } else {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }
}

/// Whether passing scenarios keep their directories too: when
/// [`KEEP_SCENARIO_LOGS`] is set to anything but empty or `0`.
#[must_use]
pub fn keep_scenario_logs() -> bool {
    std::env::var_os(KEEP_SCENARIO_LOGS).is_some_and(|value| !value.is_empty() && value != "0")
}

/// Runs `future`, giving up after `deadline`.
async fn within<F: Future>(deadline: Duration, future: F) -> Option<F::Output> {
    let (expired, expiry) = oneshot::channel();
    thread::spawn(move || {
        thread::sleep(deadline);
        let _ = expired.send(());
    });
    match select(pin!(future), expiry).await {
        Either::Left((output, _)) => Some(output),
        Either::Right(_) => None,
    }
}
