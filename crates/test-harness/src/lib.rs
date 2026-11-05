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
//! error of each process.
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

use std::collections::BTreeMap;
use std::fs;
use std::future::Future;
use std::io::{BufRead as _, BufReader};
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::process::{Child, ChildStdout, Command, Output, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

mod client;
mod input;
mod screen;
mod stand_ins;

pub use client::TestClient;
pub use input::Key;
pub use screen::{LAYOUT_SIZE, SCREEN_SIZE, Screenshot};
pub use stand_ins::{Notification, Theme, TrayItem};

use futures_channel::oneshot;
use futures_util::StreamExt as _;
use futures_util::future::{Either, select};
use zbus::fdo::{DBusProxy, PropertiesProxy};
use zbus::message::Type;
use zbus::names::{BusName, InterfaceName};
use zbus::proxy::CacheProperties;
use zbus::{Connection, MatchRule, MessageStream, proxy};

use crate::screen::{Screen, ScreenKind};
use crate::stand_ins::StandIns;

/// The app's control interface, as the contract in `CODING_STANDARDS.md` names it.
const APP_BUS_NAME: &str = "io.github.danielbaldwin47.DesktopCalls";
const APP_PATH: &str = "/io/github/danielbaldwin47/DesktopCalls";
const APP_INTERFACE: &str = "io.github.danielbaldwin47.DesktopCalls1";

/// The Simulated Phone's control interface.
const SIMULATED_PHONE_BUS_NAME: &str = "io.github.danielbaldwin47.DesktopCalls.SimulatedPhone";

/// How long a process may take to start and claim its bus name.
const STARTUP_DEADLINE: Duration = Duration::from_secs(10);

/// The journal's socket. While it is reachable the app logs there, so the
/// harness hides it from the app to get a log file per scenario.
const JOURNAL_SOCKET: &str = "/run/systemd/journal/socket";

#[proxy(
    interface = "io.github.danielbaldwin47.DesktopCalls.SimulatedPhone1",
    default_service = "io.github.danielbaldwin47.DesktopCalls.SimulatedPhone",
    default_path = "/io/github/danielbaldwin47/DesktopCalls/SimulatedPhone"
)]
trait SimulatedPhone1 {
    fn connect(&self) -> zbus::Result<()>;

    fn disconnect(&self) -> zbus::Result<()>;

    fn ring(&self, number: &str) -> zbus::Result<zbus::OwnedObjectPath>;

    fn set_codec(&self, codec: u8) -> zbus::Result<()>;

    fn enable_quirk(&self, name: &str) -> zbus::Result<()>;

    fn reregister_call(&self, call: &zbus::ObjectPath<'_>, after_ms: u32) -> zbus::Result<()>;

    #[zbus(property)]
    fn audio_gateway(&self) -> zbus::Result<zbus::OwnedObjectPath>;

    fn recorded_method_calls(&self) -> zbus::Result<Vec<(String, String, String, String, String)>>;
}

/// One method call the Simulated Phone received.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedMethodCall {
    /// `"system"` or `"session"`.
    pub bus: String,
    pub path: String,
    pub interface: String,
    pub member: String,
    /// The arguments in GVariant text form, such as `("org.bluez.Device1",)`.
    pub arguments: String,
}

/// Who the Simulated Phone says it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhoneIdentity {
    pub name: String,
    pub address: String,
}

/// The app's state, as its control interface publishes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppState {
    /// `ConnectionState`: `"not-set-up"`, `"connecting"`, `"connected"` or
    /// `"disconnected"`.
    pub connection_state: String,
    /// `Phone`, or `None` while it is empty.
    pub phone: Option<PhoneIdentity>,
    /// `Calls`, oldest first.
    pub calls: Vec<AppCall>,
    /// `AudioLocation`: `"desktop"`, `"phone"`, or `""` while there is no Call.
    pub audio_location: String,
    /// `Ringing`.
    pub ringing: bool,
    /// `Codec`'s name: `"cvsd"`, `"msbc"`, `"lc3-swb"`, or `""` while the
    /// Desktop has negotiated none.
    pub codec: String,
    /// `Codec`'s wideband flag ("HD").
    pub wideband: bool,
    /// `Surfaces`: each shown surface (`"banner"`, `"call-window"`) with its
    /// key content, as the surface shows it.
    pub surfaces: BTreeMap<String, BTreeMap<String, String>>,
}

impl AppState {
    /// The key content of the surface named `surface`, if it is shown.
    #[must_use]
    pub fn surface(&self, surface: &str) -> Option<&BTreeMap<String, String>> {
        self.surfaces.get(surface)
    }
}

/// One Call, as the app's `Calls` property lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppCall {
    /// The app-assigned stable Call id that `Answer`, `Decline` and `HangUp` take.
    pub id: u32,
    /// `"incoming"`, `"outgoing"` or `"unknown"`.
    pub direction: String,
    /// `"ringing"`, `"waiting"`, `"dialing"`, `"alerting"`, `"active"` or `"held"`.
    pub state: String,
    /// The other party's number as the Phone reports it, or `""` if withheld.
    pub number: String,
    /// The Contact's name, or `""`.
    pub display_name: String,
    /// When the Call connected, in milliseconds since the Unix epoch, or 0.
    pub active_since_ms: u64,
}

/// One `Error` signal from the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppError {
    /// The failure kind, such as `"answer-failed"`.
    pub kind: String,
    /// The sentence shown to the user.
    pub message: String,
}

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
    screenshot_directory: PathBuf,
    /// Every `Error` signal the app has sent, in order.
    app_errors: Arc<Mutex<Vec<AppError>>>,
    // Stopped in reverse order of starting: the app, the Simulated Phone,
    // the compositor, then the buses.
    processes: Vec<(&'static str, Child)>,
}

/// Chooses what a scenario's world has beyond the buses, the Simulated Phone
/// and the app. [`Harness::builder`] makes one.
#[derive(Default)]
#[must_use]
pub struct Builder {
    screen: Option<ScreenKind>,
    theme: Theme,
    scenario: Option<PathBuf>,
    screenshot_directory: Option<PathBuf>,
}

impl Builder {
    /// Gives the scenario an off-screen compositor of [`SCREEN_SIZE`], and
    /// runs the app on it.
    pub fn headless_screen(mut self) -> Self {
        self.screen = Some(ScreenKind::Headless);
        self
    }

    /// Gives the scenario a compositor that shows as a window on the
    /// compositor listening at `parent_socket`, and runs the app on it.
    pub fn nested_screen(mut self, parent_socket: PathBuf) -> Self {
        self.screen = Some(ScreenKind::Nested { parent_socket });
        self
    }

    /// Makes the Desktop light or dark, through the settings portal that GTK
    /// reads. Light by default.
    pub fn theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    /// Has the Simulated Phone act out the scenario file at `path` as soon
    /// as it is on the bus. See `crates/simulated-phone/src/scenario.rs`.
    pub fn scenario(mut self, path: PathBuf) -> Self {
        self.scenario = Some(path);
        self
    }

    /// Keeps screenshots in `directory`. By default they go to
    /// `target/screenshots/scenarios/<test name>/`.
    pub fn screenshot_directory(mut self, directory: PathBuf) -> Self {
        self.screenshot_directory = Some(directory);
        self
    }

    /// Starts private buses, the compositor if asked for, the Simulated Phone
    /// and the app, and returns once the app's control interface is on the
    /// bus.
    ///
    /// # Panics
    ///
    /// If any of them fails to start; the panic says which and where its log is.
    #[must_use]
    pub fn start(self) -> Harness {
        let directory = tempfile::Builder::new()
            .prefix("desktop-calls-test-")
            .tempdir()
            .expect("create the scenario directory")
            .keep();
        let bin_directory = bin_directory();
        let mut processes = Vec::new();

        let system_bus = start_bus(&directory, "system", &mut processes);
        let session_bus = start_bus(&directory, "session", &mut processes);
        let mut environment = environment(&directory, &system_bus, &session_bus);
        let screen = self.screen.as_ref().map(|kind| {
            let screen = Screen::start(kind, &directory, &environment, &mut processes);
            environment.push(("WAYLAND_DISPLAY", screen.wayland_display.clone()));
            screen
        });
        let screenshot_directory = self.screenshot_directory.unwrap_or_else(|| {
            bin_directory
                .parent()
                .unwrap_or(&bin_directory)
                .join("screenshots/scenarios")
                .join(scenario_name())
        });

        let phone = PhoneIdentity {
            name: "Simulated Pixel".to_owned(),
            address: random_address(),
        };
        // Real PipeWire numbered the Pixel's AudioGateway `ag1`; any index
        // from 1 up is plausible, and the app must not care which.
        let audio_gateway_index = 1 + random_u32() % 99;

        let session =
            zbus::block_on(zbus::connection::Builder::address(session_bus.as_str()).build())
                .expect("connect the harness to the private session bus");
        let system =
            zbus::block_on(zbus::connection::Builder::address(system_bus.as_str()).build())
                .expect("connect the harness to the private system bus");

        let stand_ins = StandIns::start(&session_bus, self.theme)
            .expect("serve the desktop stand-ins on the private session bus");
        // Subscribe before the app starts, so no Error signal is missed.
        let app_errors = collect_app_errors(&session);

        let mut harness = Harness {
            directory,
            bin_directory,
            environment,
            phone,
            audio_gateway_index,
            session,
            session_bus,
            system,
            stand_ins,
            screen,
            screenshot_directory,
            app_errors,
            processes,
        };

        let mut simulated_phone = harness.command("simulated-phone");
        simulated_phone
            .arg("--name")
            .arg(&harness.phone.name)
            .arg("--address")
            .arg(&harness.phone.address)
            .arg("--first-audio-gateway-index")
            .arg(harness.audio_gateway_index.to_string());
        if let Some(scenario) = &self.scenario {
            simulated_phone.arg("--scenario").arg(scenario);
        }
        let simulated_phone = simulated_phone
            .stdout(harness.log_file("simulated-phone.stdout"))
            .stderr(harness.log_file("simulated-phone.stderr"))
            .spawn()
            .expect("start the Simulated Phone");
        harness.processes.push(("simulated-phone", simulated_phone));
        harness.wait_for_bus_name(SIMULATED_PHONE_BUS_NAME, "simulated-phone");

        let app = harness
            .app_command()
            .stdout(harness.log_file("desktop-calls.stdout"))
            .stderr(harness.log_file("desktop-calls.stderr"))
            .spawn()
            .expect("start desktop-calls");
        harness.processes.push(("desktop-calls", app));
        harness.wait_for_bus_name(APP_BUS_NAME, "desktop-calls");

        harness
    }
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

    /// Captures the screen as `<name>.png` in the screenshot directory.
    ///
    /// # Panics
    ///
    /// If the scenario has no screen, or the capture fails.
    #[must_use]
    pub fn screenshot(&self, name: &str) -> Screenshot {
        self.screen();
        Screen::screenshot(
            &self.environment,
            &self.screenshot_directory.join(format!("{name}.png")),
        )
    }

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
        command.env_clear().envs(self.environment.iter().cloned());
        command
    }

    /// The notification server stand-in.
    #[must_use]
    pub fn notifications(&self) -> Notifications<'_> {
        Notifications {
            stand_ins: &self.stand_ins,
        }
    }

    /// The tray watcher stand-in.
    #[must_use]
    pub fn tray(&self) -> Tray<'_> {
        Tray {
            stand_ins: &self.stand_ins,
        }
    }

    /// Turns Omarchy's Do Not Disturb on or off, by rewriting its state file
    /// as Omarchy does.
    ///
    /// # Panics
    ///
    /// If the file can't be written.
    pub fn set_do_not_disturb(&self, on: bool) {
        stand_ins::write_do_not_disturb(&self.directory.join("home"), on)
            .expect("write Omarchy's notification state file");
    }

    /// Types on the screen's synthetic keyboard, into whatever has focus.
    ///
    /// # Panics
    ///
    /// If the scenario has no screen.
    #[must_use]
    pub fn keyboard(&self) -> Keyboard<'_> {
        Keyboard {
            screen: self.screen(),
        }
    }

    /// Moves and clicks the screen's synthetic pointer.
    ///
    /// # Panics
    ///
    /// If the scenario has no screen.
    #[must_use]
    pub fn pointer(&self) -> Pointer<'_> {
        Pointer {
            screen: self.screen(),
        }
    }

    /// Starts the harness's own client, which opens a window on the screen
    /// and prints the input it receives.
    ///
    /// # Panics
    ///
    /// If the scenario has no screen, or the client can't start.
    #[must_use]
    pub fn start_test_client(&self) -> TestClient {
        self.screen();
        let child = self
            .command("test-client")
            .stdout(Stdio::piped())
            .stderr(self.log_file("test-client.stderr"))
            .spawn()
            .expect("start test-client");
        TestClient::new(child)
    }

    /// Blocks until the screen closes: the compositor exits, or, for a
    /// nested screen, its window is closed.
    ///
    /// # Panics
    ///
    /// If the scenario has no screen.
    pub fn wait_until_screen_closes(&self) {
        self.screen().input().wait_until_closed();
    }

    fn screen(&self) -> &Screen {
        self.screen
            .as_ref()
            .expect("this needs a screen: start with Harness::builder().headless_screen()")
    }

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

    /// Acts as the Phone.
    #[must_use]
    pub fn simulated_phone(&self) -> SimulatedPhone<'_> {
        SimulatedPhone { harness: self }
    }

    /// Reads the app's state from its control interface.
    ///
    /// # Panics
    ///
    /// If the control interface doesn't answer.
    #[must_use]
    pub fn app_state(&self) -> AppState {
        zbus::block_on(read_app_state(&self.session)).expect("read the app's control interface")
    }

    /// Waits until the app's state satisfies `condition`, and returns that
    /// state.
    ///
    /// # Panics
    ///
    /// If `deadline` passes first. The panic names `description` and the last
    /// state seen.
    pub fn wait_for_app(
        &self,
        description: &str,
        deadline: Duration,
        condition: impl Fn(&AppState) -> bool,
    ) -> AppState {
        let waited = zbus::block_on(within(deadline, async {
            let properties = PropertiesProxy::builder(&self.session)
                .destination(APP_BUS_NAME)
                .path(APP_PATH)
                .build()
                .await?;
            // Subscribe before reading, so no change slips between the two.
            let mut changes = properties.receive_properties_changed().await?;
            loop {
                let state = read_app_state(&self.session).await?;
                if condition(&state) {
                    return Ok::<_, zbus::Error>(state);
                }
                if changes.next().await.is_none() {
                    return Err(zbus::Error::Failure(
                        "the app's PropertiesChanged stream ended".to_owned(),
                    ));
                }
            }
        }));
        match waited {
            Some(Ok(state)) => state,
            Some(Err(error)) => panic!("waiting for \"{description}\": {error}"),
            None => panic!(
                "\"{description}\" did not happen within {deadline:?}; the app's state is {:?}",
                zbus::block_on(read_app_state(&self.session))
            ),
        }
    }

    /// Checks that the app's state satisfies `condition` at every change for
    /// the whole of `window`, for something that must not happen, such as a
    /// Call ending. Returns the last state.
    ///
    /// # Panics
    ///
    /// At the first state that fails `condition`, naming `description`.
    pub fn app_holds(
        &self,
        description: &str,
        window: Duration,
        condition: impl Fn(&AppState) -> bool,
    ) -> AppState {
        let checked = zbus::block_on(async {
            let properties = PropertiesProxy::builder(&self.session)
                .destination(APP_BUS_NAME)
                .path(APP_PATH)
                .build()
                .await?;
            let mut changes = properties.receive_properties_changed().await?;
            let mut state = read_app_state(&self.session).await?;
            let watching = async {
                loop {
                    if !condition(&state) {
                        return Some(state.clone());
                    }
                    changes.next().await?;
                    state = read_app_state(&self.session).await.ok()?;
                }
            };
            let broken = within(window, watching).await.flatten();
            Ok::<_, zbus::Error>(broken)
        });
        match checked {
            Ok(None) => self.app_state(),
            Ok(Some(state)) => {
                panic!("\"{description}\" stopped holding; the app's state is {state:?}")
            }
            Err(error) => panic!("watching \"{description}\": {error}"),
        }
    }

    /// Waits until the app's log has a line containing `needle`, and returns
    /// that line.
    ///
    /// # Panics
    ///
    /// If `deadline` passes first.
    pub fn wait_for_app_log(&self, needle: &str, deadline: Duration) -> String {
        let log = self.directory.join("state/desktop-calls/desktop-calls.log");
        let started = std::time::Instant::now();
        loop {
            if let Some(line) = fs::read_to_string(&log)
                .unwrap_or_default()
                .lines()
                .find(|line| line.contains(needle))
            {
                return line.to_owned();
            }
            assert!(
                started.elapsed() < deadline,
                "the app logged nothing containing {needle:?} within {deadline:?}; see {}",
                log.display()
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Asks the app to answer the Call with this id, as any client of the
    /// control interface could.
    ///
    /// # Panics
    ///
    /// If the control interface refuses the call.
    pub fn answer(&self, call: u32) {
        self.call_app_method("Answer", call);
    }

    /// Asks the app to decline the ringing Call with this id.
    ///
    /// # Panics
    ///
    /// If the control interface refuses the call.
    pub fn decline(&self, call: u32) {
        self.call_app_method("Decline", call);
    }

    /// Asks the app to hang up the Call with this id.
    ///
    /// # Panics
    ///
    /// If the control interface refuses the call.
    pub fn hang_up(&self, call: u32) {
        self.call_app_method("HangUp", call);
    }

    fn call_app_method(&self, member: &str, call: u32) {
        zbus::block_on(self.session.call_method(
            Some(APP_BUS_NAME),
            APP_PATH,
            Some(APP_INTERFACE),
            member,
            &(call,),
        ))
        .unwrap_or_else(|error| panic!("{member}({call}) on the app: {error}"));
    }

    /// Waits for the app's first `Error` signal, and returns it.
    ///
    /// # Panics
    ///
    /// If `deadline` passes first.
    pub fn wait_for_app_error(&self, deadline: Duration) -> AppError {
        let started = std::time::Instant::now();
        loop {
            if let Some(error) = self.app_errors().into_iter().next() {
                return error;
            }
            assert!(
                started.elapsed() < deadline,
                "the app sent no Error within {deadline:?}"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Every `Error` signal the app has sent so far, in order.
    #[must_use]
    pub fn app_errors(&self) -> Vec<AppError> {
        self.app_errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Calls a method on the Phone's side of one of the private buses, as any
    /// D-Bus client could, and returns the reply.
    ///
    /// # Panics
    ///
    /// If the call fails or the reply is an error; the panic names the call.
    pub fn call_phone_method<B>(
        &self,
        bus: Bus,
        service: &str,
        path: &str,
        interface: &str,
        member: &str,
        body: &B,
    ) -> zbus::Message
    where
        B: zbus::export::serde::Serialize + zbus::DynamicType,
    {
        let connection = match bus {
            Bus::System => &self.system,
            Bus::Session => &self.session,
        };
        zbus::block_on(connection.call_method(Some(service), path, Some(interface), member, body))
            .unwrap_or_else(|error| panic!("{interface}.{member} on {service} {path}: {error}"))
    }

    /// Runs the `desktop-calls` CLI with `arguments` in this scenario's
    /// environment.
    ///
    /// # Panics
    ///
    /// If the binary can't be run at all.
    #[must_use]
    pub fn cli(&self, arguments: &[&str]) -> Output {
        self.command("desktop-calls")
            .args(arguments)
            .output()
            .expect("run the desktop-calls CLI")
    }

    /// A command for one of the workspace's binaries, with this scenario's
    /// environment and nothing else.
    fn command(&self, binary: &str) -> Command {
        let mut command = Command::new(self.bin_directory.join(binary));
        command.env_clear().envs(self.environment.iter().cloned());
        command
    }

    /// The command that runs the app's long-running process.
    ///
    /// Where the systemd journal is reachable, the app would log there, so it
    /// runs in a mount namespace with an empty directory over the journal's.
    /// It then finds no journal and logs to its file under `XDG_STATE_HOME`,
    /// exactly as it would on any machine without a journal. A second user
    /// namespace maps the user back to their own ID, so the app runs as the
    /// same user it would outside.
    fn app_command(&self) -> Command {
        let app = self.bin_directory.join("desktop-calls");
        if !Path::new(JOURNAL_SOCKET).exists() {
            let mut command = self.command("desktop-calls");
            command.arg("service");
            return command;
        }
        let user = fs::metadata("/proc/self").expect("read /proc/self");
        let mut command = Command::new("unshare");
        command
            .env_clear()
            .envs(self.environment.iter().cloned())
            .args(["--user", "--map-root-user", "--mount", "--", "sh", "-c"])
            .arg(concat!(
                "mount -t tmpfs none /run/systemd/journal && ",
                "uid=$1 gid=$2 && shift 2 && ",
                r#"exec unshare --user --map-user="$uid" --map-group="$gid" -- "$@""#
            ))
            .arg("sh")
            .arg(user.uid().to_string())
            .arg(user.gid().to_string())
            .arg(app)
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
                "{process} did not claim {name} within {STARTUP_DEADLINE:?} ({appeared:?}, exit status {exit:?}); see {}",
                self.directory.join(format!("{process}.stderr")).display()
            );
        }
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        while let Some((_, mut child)) = self.processes.pop() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if thread::panicking() {
            eprintln!("Kept this scenario's logs in {}", self.directory.display());
        } else {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }
}

/// The notification server stand-in. See [`Harness::notifications`].
pub struct Notifications<'harness> {
    stand_ins: &'harness StandIns,
}

impl Notifications<'_> {
    /// Every notification received so far, in order.
    #[must_use]
    pub fn received(&self) -> Vec<Notification> {
        self.stand_ins.notifications()
    }

    /// Waits until a received notification satisfies `condition`, and
    /// returns it.
    ///
    /// # Panics
    ///
    /// If `deadline` passes first. The panic names `description` and every
    /// notification received.
    pub fn wait_for(
        &self,
        description: &str,
        deadline: Duration,
        condition: impl Fn(&Notification) -> bool,
    ) -> Notification {
        self.stand_ins
            .wait_for_notification(deadline, condition)
            .unwrap_or_else(|received| {
                panic!(
                    "\"{description}\" did not happen within {deadline:?}; received {received:#?}"
                )
            })
    }

    /// Clicks notification `id`, as Omarchy does: invokes its `default`
    /// action and closes it.
    ///
    /// # Panics
    ///
    /// If the notification offered no `default` action.
    pub fn invoke_default_action(&self, id: u32) {
        if let Err(error) = self.stand_ins.invoke_default_action(id) {
            panic!("click notification {id}: {error}");
        }
    }
}

/// The tray watcher stand-in. See [`Harness::tray`].
pub struct Tray<'harness> {
    stand_ins: &'harness StandIns,
}

impl Tray<'_> {
    /// Every registered item, with its status and icon as it reports them now.
    #[must_use]
    pub fn items(&self) -> Vec<TrayItem> {
        self.stand_ins.tray_items()
    }

    /// Waits until the registered items satisfy `condition`, and returns them.
    ///
    /// # Panics
    ///
    /// If `deadline` passes first. The panic names `description` and the
    /// items.
    pub fn wait_for(
        &self,
        description: &str,
        deadline: Duration,
        condition: impl Fn(&[TrayItem]) -> bool,
    ) -> Vec<TrayItem> {
        self.stand_ins
            .wait_for_tray(deadline, condition)
            .unwrap_or_else(|error| {
                panic!("\"{description}\" did not happen within {deadline:?}: {error}")
            })
    }
}

/// The screen's synthetic keyboard. See [`Harness::keyboard`].
pub struct Keyboard<'harness> {
    screen: &'harness Screen,
}

impl Keyboard<'_> {
    /// Types `text` with a US layout, holding Shift where a character needs it.
    ///
    /// # Panics
    ///
    /// If `text` has a character the US layout can't type, or the compositor
    /// is gone.
    pub fn type_text(&self, text: &str) {
        if let Err(error) = self.screen.input().type_text(text) {
            panic!("type {text:?}: {error}");
        }
    }

    /// Presses and releases `key`.
    ///
    /// # Panics
    ///
    /// If the compositor is gone.
    pub fn press(&self, key: Key) {
        if let Err(error) = self.screen.input().press(key) {
            panic!("press {key:?}: {error}");
        }
    }
}

/// The screen's synthetic pointer, in layout coordinates: the headless
/// screen is [`LAYOUT_SIZE`]. See [`Harness::pointer`].
pub struct Pointer<'harness> {
    screen: &'harness Screen,
}

impl Pointer<'_> {
    /// Moves the pointer to (`x`, `y`).
    ///
    /// # Panics
    ///
    /// If the point is off the screen, or the compositor is gone.
    pub fn move_to(&self, x: f64, y: f64) {
        if let Err(error) = self.screen.input().move_to(x, y, LAYOUT_SIZE) {
            panic!("move the pointer to ({x}, {y}): {error}");
        }
    }

    /// Moves the pointer to (`x`, `y`) and clicks the left button there.
    ///
    /// # Panics
    ///
    /// If the point is off the screen, or the compositor is gone.
    pub fn click(&self, x: f64, y: f64) {
        self.move_to(x, y);
        if let Err(error) = self.screen.input().click() {
            panic!("click at ({x}, {y}): {error}");
        }
    }
}

/// Drives the Simulated Phone through its control interface.
pub struct SimulatedPhone<'harness> {
    harness: &'harness Harness,
}

impl SimulatedPhone<'_> {
    /// Brings hands-free up: the AudioGateway appears once it is fully set up.
    ///
    /// # Panics
    ///
    /// If the Simulated Phone refuses.
    pub fn connect(&self) {
        zbus::block_on(async { self.proxy().await?.connect().await })
            .expect("tell the Simulated Phone to connect");
    }

    /// Drops hands-free: the AudioGateway disappears.
    ///
    /// # Panics
    ///
    /// If the Simulated Phone refuses.
    pub fn disconnect(&self) {
        zbus::block_on(async { self.proxy().await?.disconnect().await })
            .expect("tell the Simulated Phone to disconnect");
    }

    /// Rings the Phone from `number`, as a caller does, and returns the new
    /// Call's object path.
    ///
    /// # Panics
    ///
    /// If the Simulated Phone refuses, as it does while hands-free is down.
    #[must_use]
    pub fn ring(&self, number: &str) -> String {
        zbus::block_on(async { self.proxy().await?.ring(number).await })
            .expect("ring the Simulated Phone")
            .to_string()
    }

    /// Makes the Phone's audio link negotiate the codec with this hands-free
    /// id from now on: 1 CVSD, 2 mSBC, 3 LC3-SWB (the Pixel's, by default).
    ///
    /// # Panics
    ///
    /// If the Simulated Phone refuses.
    pub fn set_codec(&self, codec: u8) {
        zbus::block_on(async { self.proxy().await?.set_codec(codec).await })
            .expect("set the Simulated Phone's codec");
    }

    /// Switches on a quirk: `"call1-hangup-missing"` (only
    /// `org.ofono.VoiceCall.Hangup` works) or `"hangup-too-few-arguments"`
    /// (`Hangup` fails with "Too few arguments" while the Call ends anyway).
    ///
    /// # Panics
    ///
    /// If the Simulated Phone doesn't know the quirk.
    pub fn enable_quirk(&self, name: &str) {
        zbus::block_on(async { self.proxy().await?.enable_quirk(name).await })
            .unwrap_or_else(|error| panic!("enable quirk {name:?}: {error}"));
    }

    /// Unregisters the Call at `call` and registers it again `after` later,
    /// in the same state, as PipeWire does when the hands-free profile
    /// briefly drops mid-Call. Returns once the Call is gone.
    ///
    /// # Panics
    ///
    /// If there is no such Call.
    pub fn reregister_call(&self, call: &str, after: Duration) {
        let after_ms = u32::try_from(after.as_millis()).unwrap_or(u32::MAX);
        zbus::block_on(async {
            let call = zbus::ObjectPath::try_from(call)?;
            self.proxy().await?.reregister_call(&call, after_ms).await
        })
        .unwrap_or_else(|error| panic!("reregister the Call at {call}: {error}"));
    }

    /// The object path of the current AudioGateway, or `"/"` while there is
    /// none.
    ///
    /// # Panics
    ///
    /// If the Simulated Phone doesn't answer.
    #[must_use]
    pub fn audio_gateway(&self) -> String {
        zbus::block_on(async {
            self.proxy()
                .await?
                .audio_gateway()
                .await
                .map(|path| path.to_string())
        })
        .expect("read the Simulated Phone's AudioGateway")
    }

    /// Every method call the Simulated Phone received so far, in order.
    ///
    /// # Panics
    ///
    /// If the Simulated Phone doesn't answer.
    #[must_use]
    pub fn recorded_method_calls(&self) -> Vec<RecordedMethodCall> {
        zbus::block_on(async { self.proxy().await?.recorded_method_calls().await })
            .expect("read the Simulated Phone's recorded method calls")
            .into_iter()
            .map(
                |(bus, path, interface, member, arguments)| RecordedMethodCall {
                    bus,
                    path,
                    interface,
                    member,
                    arguments,
                },
            )
            .collect()
    }

    async fn proxy(&self) -> zbus::Result<SimulatedPhone1Proxy<'static>> {
        SimulatedPhone1Proxy::builder(&self.harness.session)
            .cache_properties(CacheProperties::No)
            .build()
            .await
    }
}

async fn read_app_state(session: &Connection) -> zbus::Result<AppState> {
    // One GetAll, so the properties come from the same moment.
    let properties = PropertiesProxy::builder(session)
        .destination(APP_BUS_NAME)
        .path(APP_PATH)
        .build()
        .await?;
    let all = properties
        .get_all(InterfaceName::try_from(APP_INTERFACE)?)
        .await?;
    let property = |name: &str| {
        all.get(name)
            .ok_or_else(|| zbus::Error::Failure(format!("no {name} property")))?
            .try_clone()
    };
    let connection_state = String::try_from(property("ConnectionState")?)?;
    let (name, address) = <(String, String)>::try_from(property("Phone")?)?;
    let calls = <Vec<(u32, String, String, String, String, u64)>>::try_from(property("Calls")?)?
        .into_iter()
        .map(
            |(id, direction, state, number, display_name, active_since_ms)| AppCall {
                id,
                direction,
                state,
                number,
                display_name,
                active_since_ms,
            },
        )
        .collect();
    let (codec, wideband) = <(String, bool)>::try_from(property("Codec")?)?;
    let surfaces = <BTreeMap<String, BTreeMap<String, String>>>::try_from(property("Surfaces")?)?;
    Ok(AppState {
        connection_state,
        phone: (!address.is_empty()).then_some(PhoneIdentity { name, address }),
        calls,
        audio_location: String::try_from(property("AudioLocation")?)?,
        ringing: bool::try_from(property("Ringing")?)?,
        codec,
        wideband,
        surfaces,
    })
}

/// Collects every `Error` signal the app sends on `session`, from now on.
fn collect_app_errors(session: &Connection) -> Arc<Mutex<Vec<AppError>>> {
    let errors = Arc::new(Mutex::new(Vec::new()));
    let stream = zbus::block_on(async {
        let rule = MatchRule::builder()
            .msg_type(Type::Signal)
            .path(APP_PATH)
            .interface(APP_INTERFACE)
            .member("Error")
            .build()?;
        MessageStream::for_match_rule(rule, session, None).await
    })
    .expect("subscribe to the app's Error signal");
    let collected = errors.clone();
    thread::spawn(move || {
        zbus::block_on(async move {
            let mut stream = stream;
            while let Some(Ok(message)) = stream.next().await {
                if let Ok((kind, message)) = message.body().deserialize::<(String, String)>() {
                    collected
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push(AppError { kind, message });
                }
            }
        });
    });
    errors
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

/// Where cargo put the workspace's binaries: `target/<profile>`, which holds
/// the automation binary, and whose `deps` holds test executables.
fn bin_directory() -> PathBuf {
    let executable = std::env::current_exe().expect("find the running executable");
    let mut directory = executable
        .parent()
        .expect("the executable is in target/<profile>")
        .to_owned();
    if directory.ends_with("deps") {
        directory.pop();
    }
    for binary in ["desktop-calls", "simulated-phone", "test-client"] {
        assert!(
            directory.join(binary).exists(),
            "{binary} isn't built in {}; run `just test`, which builds it first",
            directory.display()
        );
    }
    directory
}

/// Starts a private `dbus-daemon` and returns its address.
fn start_bus(
    directory: &Path,
    name: &'static str,
    processes: &mut Vec<(&'static str, Child)>,
) -> String {
    let socket = directory.join(format!("{name}-bus"));
    let config = directory.join(format!("{name}-bus.conf"));
    fs::write(
        &config,
        format!(
            r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:path={}</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
"#,
            socket.display()
        ),
    )
    .expect("write the bus configuration");
    let mut child = Command::new("dbus-daemon")
        .env_clear()
        .arg("--nofork")
        .arg("--print-address")
        .arg(format!("--config-file={}", config.display()))
        .stdout(Stdio::piped())
        .stderr(
            fs::File::create(directory.join(format!("{name}-bus.stderr")))
                .expect("create the bus log"),
        )
        .spawn()
        .expect("start dbus-daemon; is the dbus package installed?");
    let address = read_address(child.stdout.take());
    processes.push((name, child));
    address.unwrap_or_else(|| panic!("the private {name} bus did not start"))
}

/// The first line `dbus-daemon --print-address` writes, once it is listening.
fn read_address(stdout: Option<ChildStdout>) -> Option<String> {
    let mut line = String::new();
    BufReader::new(stdout?).read_line(&mut line).ok()?;
    let address = line.trim().to_owned();
    (!address.is_empty()).then_some(address)
}

/// The only environment the scenario's processes get: private buses, a
/// temporary home and XDG directories inside the scenario directory.
fn environment(
    directory: &Path,
    system_bus: &str,
    session_bus: &str,
) -> Vec<(&'static str, String)> {
    let path_of = |name: &str| {
        let path = directory.join(name);
        fs::create_dir_all(&path).expect("create a scenario directory");
        path.display().to_string()
    };
    let runtime = path_of("runtime");
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700))
        .expect("make XDG_RUNTIME_DIR private");
    vec![
        ("DBUS_SYSTEM_BUS_ADDRESS", system_bus.to_owned()),
        ("DBUS_SESSION_BUS_ADDRESS", session_bus.to_owned()),
        ("HOME", path_of("home")),
        ("XDG_CONFIG_HOME", path_of("config")),
        ("XDG_CACHE_HOME", path_of("cache")),
        ("XDG_STATE_HOME", path_of("state")),
        ("XDG_RUNTIME_DIR", runtime),
        ("PATH", std::env::var("PATH").unwrap_or_default()),
        ("RUST_LOG", "debug".to_owned()),
        ("RUST_BACKTRACE", "1".to_owned()),
    ]
}

/// A locally administered, unicast Bluetooth address, different every run.
fn random_address() -> String {
    let bytes = (u64::from(random_u32()) << 32 | u64::from(random_u32())).to_be_bytes();
    let first = (bytes[2] & 0xfc) | 0x02;
    format!(
        "{first:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        bytes[3], bytes[4], bytes[5], bytes[6], bytes[7]
    )
}

/// The running test's name, from its thread, as a directory name.
fn scenario_name() -> String {
    match thread::current().name() {
        Some(name) if name != "main" => name.replace("::", "-"),
        _ => "scenario".to_owned(),
    }
}

fn random_u32() -> u32 {
    use std::hash::{BuildHasher as _, RandomState};
    let hash = RandomState::new().hash_one(std::time::SystemTime::now());
    u32::try_from(hash >> 32).unwrap_or(0)
}
