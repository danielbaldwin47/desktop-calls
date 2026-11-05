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
use std::time::{Duration, Instant};

mod client;
mod input;
mod screen;
mod stand_ins;
pub mod vcards;

pub use client::TestClient;
pub use input::Key;
pub use screen::{LAYOUT_SIZE, Region, SCREEN_SIZE, Screenshot};
pub use stand_ins::{BAR_COLOUR, BAR_HEIGHT, Notification, OmarchyTheme, Theme, TrayItem};

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

    fn set_pbap_folder(&self, folder: &str, vcards: &str) -> zbus::Result<()>;

    fn remove_pbap_folder(&self, folder: &str) -> zbus::Result<()>;

    fn set_contacts_prompt(&self, answer: &str) -> zbus::Result<()>;

    fn set_pull_delay(&self, delay_ms: u32) -> zbus::Result<()>;

    fn set_microphone_volume(&self, volume: u8) -> zbus::Result<()>;

    fn answer_on_phone(&self, call: &zbus::ObjectPath<'_>) -> zbus::Result<()>;

    fn decline_on_phone(&self, call: &zbus::ObjectPath<'_>) -> zbus::Result<()>;

    fn caller_hangs_up(&self, call: &zbus::ObjectPath<'_>) -> zbus::Result<()>;

    #[zbus(property)]
    fn audio_gateway(&self) -> zbus::Result<zbus::OwnedObjectPath>;

    #[zbus(property)]
    fn most_pbap_sessions_at_once(&self) -> zbus::Result<u32>;

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
    /// `Muted`: whether the Desktop has muted its microphone.
    pub muted: bool,
    /// `Codec`'s name: `"cvsd"`, `"msbc"`, `"lc3-swb"`, or `""` while the
    /// Desktop has negotiated none.
    pub codec: String,
    /// `Codec`'s wideband flag ("HD").
    pub wideband: bool,
    /// `Surfaces`: each shown surface (`"banner"`, `"call-window"`) with its
    /// key content, as the surface shows it.
    pub surfaces: BTreeMap<String, BTreeMap<String, String>>,
    /// `ContactsState`.
    pub contacts: AppContactsState,
    /// `Problems`, in the order the app lists them.
    pub problems: Vec<AppProblem>,
}

/// The app's `ContactsState`: how far Contacts and Call History are synced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppContactsState {
    /// `"syncing"`, `"ready"`, `"permission-needed"`, `"denied"` or
    /// `"unavailable"`.
    pub state: String,
    /// How many Contacts the Desktop holds.
    pub contacts: u32,
    /// How many Call History entries the Desktop holds.
    pub call_history: u32,
    /// While `"permission-needed"`: when the Phone's prompt times out, in
    /// milliseconds since the Unix epoch; otherwise 0.
    pub deadline_ms: u64,
    /// While `"unavailable"`: why, such as `"prompt-timed-out"`; otherwise
    /// empty.
    pub reason: String,
}

/// One entry of the app's `Problems`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppProblem {
    /// The problem's kind, such as `"contacts-denied"`.
    pub kind: String,
    /// The sentence shown to the user.
    pub message: String,
    /// What the user can do, or empty.
    pub action: String,
}

/// One entry of the app's Call History.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppCallHistoryEntry {
    /// `"incoming"`, `"outgoing"` or `"missed"`.
    pub kind: String,
    /// The other party's number as the Phone reports it, or `""` if withheld.
    pub number: String,
    /// The Contact's name, or `""`.
    pub display_name: String,
    /// The number's label, such as `"mobile"`, or `""`.
    pub number_label: String,
    /// When, in milliseconds since the Unix epoch, or 0 if the Phone didn't say.
    pub at_ms: u64,
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

/// One `Error` signal from the app, or the error a method of its control
/// interface returned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppError {
    /// The failure kind, such as `"answer-failed"`; for a method's error,
    /// the D-Bus error name.
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
    /// The stand-in for Omarchy's bar, if the scenario has one.
    bar: Option<TestClient>,
    /// The Hyprland stand-in, if the scenario runs under one.
    hyprland: Option<stand_ins::Hyprland>,
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
    omarchy_bar: bool,
    time_zone: Option<String>,
    phone_connected: bool,
    omarchy_theme: Option<OmarchyTheme>,
    hyprland: Option<HyprlandSettings>,
}

/// What the Hyprland stand-in reports.
#[derive(Clone, Copy)]
struct HyprlandSettings {
    toast_column: u32,
    rounding: u32,
}

impl Default for HyprlandSettings {
    /// As on the user's Desktop: Omarchy 4.5's toast column, and Omarchy's
    /// default square corners.
    fn default() -> Self {
        Self {
            toast_column: stand_ins::OMARCHY_TOAST_COLUMN_WIDTH,
            rounding: 0,
        }
    }
}

impl HyprlandSettings {
    /// Serves the Hyprland stand-in in the scenario's `directory`, and names
    /// it in `environment` as Hyprland names itself.
    fn serve(
        self,
        directory: &Path,
        environment: &mut Vec<(&'static str, String)>,
    ) -> stand_ins::Hyprland {
        let hyprland =
            stand_ins::serve_hyprland(&directory.join("runtime"), self.toast_column, self.rounding)
                .expect("serve the Hyprland stand-in");
        environment.push((
            "HYPRLAND_INSTANCE_SIGNATURE",
            stand_ins::HYPRLAND_SIGNATURE.to_owned(),
        ));
        hyprland
    }
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

    /// Puts a stand-in for Omarchy's bar on the screen: [`BAR_HEIGHT`] tall
    /// across the top, reserved as an exclusive zone, drawn in
    /// [`BAR_COLOUR`]. Needs a screen.
    pub fn omarchy_bar(mut self) -> Self {
        self.omarchy_bar = true;
        self
    }

    /// Runs the app under a Hyprland stand-in whose `hyprctl layers -j`
    /// reports Omarchy's notification-toast column `width` wide. Without
    /// it there is no Hyprland, as under sway.
    pub fn hyprland_toast_column(mut self, width: u32) -> Self {
        self.hyprland.get_or_insert_default().toast_column = width;
        self
    }

    /// Runs the app under a Hyprland stand-in whose `decoration:rounding` is
    /// `rounding`. [`Harness::set_hyprland_rounding`] changes it later.
    pub fn hyprland_rounding(mut self, rounding: u32) -> Self {
        self.hyprland.get_or_insert_default().rounding = rounding;
        self
    }

    /// Makes `theme` Omarchy's current theme before the app starts. Without
    /// it there is no Omarchy theme, as on a Desktop without Omarchy.
    /// [`Harness::switch_omarchy_theme`] switches it later.
    pub fn omarchy_theme(mut self, theme: OmarchyTheme) -> Self {
        self.omarchy_theme = Some(theme);
        self
    }

    /// Sets the Desktop's time zone through the standard `TZ` variable, as
    /// an IANA name or a POSIX rule such as `"<+0530>-5:30"`. Without it the
    /// app reads `/etc/localtime`.
    pub fn time_zone(mut self, time_zone: &str) -> Self {
        self.time_zone = Some(time_zone.to_owned());
        self
    }

    /// Brings the Simulated Phone's hands-free up before the app starts, so
    /// the app starts with the Phone's AudioGateway already there, as after a
    /// login or a restart with the Phone in range.
    pub fn phone_connected(mut self) -> Self {
        self.phone_connected = true;
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
        if let Some(time_zone) = self.time_zone {
            environment.push(("TZ", time_zone));
        }
        let hyprland = self
            .hyprland
            .map(|settings| settings.serve(&directory, &mut environment));
        if let Some(theme) = self.omarchy_theme {
            stand_ins::switch_omarchy_theme(&directory.join("home"), theme)
                .expect("make the Omarchy theme current");
        }
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
            bar: None,
            hyprland,
            screenshot_directory,
            app_errors,
            processes,
        };

        if self.omarchy_bar {
            let bar = harness.spawn_test_client(&["--bar"]);
            bar.wait_for(
                "the bar stand-in is drawn",
                Duration::from_secs(10),
                |line| line.starts_with("configured "),
            );
            harness.bar = Some(bar);
        }

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
        if self.phone_connected {
            harness.simulated_phone().connect();
        }

        harness.start_app();
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

    /// Captures only `region` of the screen, as `<name>.png` in the
    /// screenshot directory. A small region takes a fraction of a full
    /// capture's time, for checks that are timed. The capture's own
    /// coordinates start at the region's top-left corner.
    ///
    /// # Panics
    ///
    /// If the scenario has no screen, or the capture fails.
    #[must_use]
    pub fn screenshot_of(&self, name: &str, region: Region) -> Screenshot {
        self.screen();
        Screen::screenshot_of(
            &self.environment,
            &self.screenshot_directory.join(format!("{name}.png")),
            region,
        )
    }

    /// Captures the screen as `<name>.png` until the capture satisfies
    /// `shown`, and returns it. What the app reports can run a frame ahead
    /// of what is on screen; this waits for the screen.
    ///
    /// # Panics
    ///
    /// If no capture satisfies `shown` within `deadline`; the panic names the
    /// last capture.
    #[must_use]
    pub fn screenshot_when(
        &self,
        name: &str,
        deadline: Duration,
        shown: impl Fn(&Screenshot) -> bool,
    ) -> Screenshot {
        let started = Instant::now();
        loop {
            let screenshot = self.screenshot(name);
            if shown(&screenshot) {
                return screenshot;
            }
            assert!(
                started.elapsed() < deadline,
                "the screen never showed {name} within {deadline:?}; see {}",
                screenshot.path.display()
            );
        }
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

    /// Switches Omarchy's current theme to `theme`, as `omarchy-theme-set`
    /// does: the theme directory is deleted and a new one moved into its
    /// place. Returns once the switch is on disk.
    ///
    /// # Panics
    ///
    /// If the theme can't be written.
    pub fn switch_omarchy_theme(&self, theme: OmarchyTheme) {
        stand_ins::switch_omarchy_theme(&self.directory.join("home"), theme)
            .expect("switch Omarchy's theme");
    }

    /// Changes Hyprland's `decoration:rounding` and announces the reload, as
    /// Hyprland does when its configuration is reloaded.
    ///
    /// # Panics
    ///
    /// If the scenario has no Hyprland stand-in.
    pub fn set_hyprland_rounding(&self, rounding: u32) {
        self.hyprland
            .as_ref()
            .expect("this needs Hyprland: start with Harness::builder().hyprland_rounding(…)")
            .set_rounding(rounding);
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

    /// Puts `text` on the screen's clipboard, as copying it in another app
    /// does, and waits until a paste would get it.
    ///
    /// # Panics
    ///
    /// If the scenario has no screen, `wl-copy` (from `wl-clipboard`) can't
    /// start, or a paste doesn't get `text` within 5 s.
    pub fn set_clipboard(&self, text: &str) -> Clipboard {
        self.screen();
        let owner = self
            .external_command("wl-copy")
            .args(["--foreground", "--", text])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start wl-copy, from wl-clipboard");
        let clipboard = Clipboard { owner };
        let started = Instant::now();
        loop {
            let pasted = self
                .external_command("wl-paste")
                .args(["--no-newline", "--type", "text/plain"])
                .stdin(Stdio::null())
                .output()
                .expect("run wl-paste, from wl-clipboard");
            if pasted.status.success() && pasted.stdout == text.as_bytes() {
                return clipboard;
            }
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "the clipboard never held {text:?}; wl-paste said {:?}",
                String::from_utf8_lossy(&pasted.stderr)
            );
            thread::sleep(Duration::from_millis(20));
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
        self.spawn_test_client(&[])
    }

    /// Starts the harness's test client fullscreen, as a video player would
    /// be. See [`Harness::start_test_client`].
    ///
    /// # Panics
    ///
    /// If the scenario has no screen, or the client can't be started.
    #[must_use]
    pub fn start_fullscreen_test_client(&self) -> TestClient {
        self.spawn_test_client(&["--fullscreen"])
    }

    fn spawn_test_client(&self, arguments: &[&str]) -> TestClient {
        self.screen();
        let child = self
            .command("test-client")
            .args(arguments)
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

    /// Asks the app to mute (`true`) or unmute the Desktop's microphone, and
    /// returns the error the control interface answers with, if any.
    ///
    /// # Errors
    ///
    /// The app's refusal: its D-Bus error name and plain-language message.
    pub fn set_muted(&self, muted: bool) -> Result<(), AppError> {
        self.try_app_method("SetMuted", &(muted,))
    }

    /// Asks the app to send keypad tones during the Call.
    ///
    /// # Errors
    ///
    /// The app's refusal: its D-Bus error name and plain-language message.
    pub fn send_tones(&self, tones: &str) -> Result<(), AppError> {
        self.try_app_method("SendTones", &(tones,))
    }

    fn try_app_method<B>(&self, member: &str, body: &B) -> Result<(), AppError>
    where
        B: zbus::export::serde::Serialize + zbus::DynamicType,
    {
        match zbus::block_on(self.session.call_method(
            Some(APP_BUS_NAME),
            APP_PATH,
            Some(APP_INTERFACE),
            member,
            body,
        )) {
            Ok(_) => Ok(()),
            Err(zbus::Error::MethodError(name, message, _)) => Err(AppError {
                kind: name.to_string(),
                message: message.unwrap_or_default(),
            }),
            Err(error) => panic!("{member} on the app: {error}"),
        }
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

    /// Every entry of the app's Call History, newest first, from its
    /// `GetCallHistory` method.
    ///
    /// # Panics
    ///
    /// If the control interface doesn't answer.
    #[must_use]
    pub fn call_history(&self) -> Vec<AppCallHistoryEntry> {
        let reply = zbus::block_on(self.session.call_method(
            Some(APP_BUS_NAME),
            APP_PATH,
            Some(APP_INTERFACE),
            "GetCallHistory",
            &(),
        ))
        .expect("GetCallHistory on the app");
        reply
            .body()
            .deserialize::<Vec<(String, String, String, String, u64)>>()
            .expect("GetCallHistory replies with a(ssssst)")
            .into_iter()
            .map(
                |(kind, number, display_name, number_label, at_ms)| AppCallHistoryEntry {
                    kind,
                    number,
                    display_name,
                    number_label,
                    at_ms,
                },
            )
            .collect()
    }

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
            "the stopped app stayed on the bus: {left:?}"
        );
        self.start_app();
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

    /// Presses `character`'s key while holding Control, such as Ctrl+V.
    ///
    /// # Panics
    ///
    /// If the US layout has no unshifted key for `character`, or the
    /// compositor is gone.
    pub fn press_with_control(&self, character: char) {
        if let Err(error) = self.screen.input().press_with_control(character) {
            panic!("press Ctrl+{character}: {error}");
        }
    }
}

/// The program holding the screen's clipboard. Dropping it empties the
/// clipboard. See [`Harness::set_clipboard`].
#[must_use = "the clipboard empties when this is dropped"]
pub struct Clipboard {
    owner: Child,
}

impl Drop for Clipboard {
    fn drop(&mut self) {
        let _ = self.owner.kill();
        let _ = self.owner.wait();
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

    /// Gives the Phone the PBAP folder `folder` (`"pb"` for Contacts;
    /// `"cch"`, `"ich"`, `"och"` or `"mch"` for Call History) with exactly
    /// this vCard text, which it serves whatever format is asked for. See
    /// [`crate::vcards`] for builders.
    ///
    /// # Panics
    ///
    /// If the Simulated Phone refuses.
    pub fn set_pbap_folder(&self, folder: &str, vcards: &str) {
        zbus::block_on(async { self.proxy().await?.set_pbap_folder(folder, vcards).await })
            .unwrap_or_else(|error| panic!("set the {folder} folder: {error}"));
    }

    /// Makes the Phone lack the folder `folder`, as some Phones lack
    /// `cch`: selecting it fails.
    ///
    /// # Panics
    ///
    /// If the Simulated Phone refuses.
    pub fn remove_pbap_folder(&self, folder: &str) {
        zbus::block_on(async { self.proxy().await?.remove_pbap_folder(folder).await })
            .unwrap_or_else(|error| panic!("remove the {folder} folder: {error}"));
    }

    /// How the person holding the Phone answers its "Allow access to
    /// contacts and call history?" prompt for each new PBAP session:
    /// `"grant"` (the default), `"deny"` or `"ignore"`.
    ///
    /// # Panics
    ///
    /// If the Simulated Phone doesn't know the answer.
    pub fn set_contacts_prompt(&self, answer: &str) {
        zbus::block_on(async { self.proxy().await?.set_contacts_prompt(answer).await })
            .unwrap_or_else(|error| panic!("set the contacts prompt to {answer:?}: {error}"));
    }

    /// Makes every PBAP pull take `delay` before it completes.
    ///
    /// # Panics
    ///
    /// If the Simulated Phone refuses.
    pub fn set_pull_delay(&self, delay: Duration) {
        let delay_ms = u32::try_from(delay.as_millis()).unwrap_or(u32::MAX);
        zbus::block_on(async { self.proxy().await?.set_pull_delay(delay_ms).await })
            .expect("set the Simulated Phone's pull delay");
    }

    /// The most PBAP sessions the Phone has had open at once, counting a
    /// session from its `CreateSession` to its `RemoveSession`.
    ///
    /// # Panics
    ///
    /// If the Simulated Phone doesn't answer.
    #[must_use]
    pub fn most_pbap_sessions_at_once(&self) -> u32 {
        zbus::block_on(async { self.proxy().await?.most_pbap_sessions_at_once().await })
            .expect("read the Simulated Phone's PBAP session count")
    }

    /// Makes the Phone report this hands-free microphone gain, as `+VGM`
    /// does: 0 to 15, or 255, which is how PipeWire sends its -1 ("the Phone
    /// hasn't said"). A later AudioGateway starts with it too.
    ///
    /// # Panics
    ///
    /// If the Simulated Phone refuses.
    pub fn set_microphone_volume(&self, volume: u8) {
        zbus::block_on(async { self.proxy().await?.set_microphone_volume(volume).await })
            .expect("set the Simulated Phone's microphone volume");
    }

    /// Answers the ringing Call at `call` on the Phone, as the person holding
    /// it does. Returns once the Phone has reported the Call active.
    ///
    /// # Panics
    ///
    /// If there is no such Call, or it isn't ringing.
    pub fn answer_on_phone(&self, call: &str) {
        zbus::block_on(async {
            let call = zbus::ObjectPath::try_from(call)?;
            self.proxy().await?.answer_on_phone(&call).await
        })
        .unwrap_or_else(|error| panic!("answer the Call at {call} on the Phone: {error}"));
    }

    /// Waits until the method calls the Simulated Phone has received, in
    /// order, satisfy `condition`, and returns them.
    ///
    /// The app sends its Phone commands on a connection of its own, so a
    /// call can reach the Phone after the app's state has moved on. That is
    /// why there is no snapshot of the calls to assert on: every check of
    /// what the Phone was asked waits, here or in [`Self::recorded_holds`].
    ///
    /// # Panics
    ///
    /// If `deadline` passes first; the panic names `description` and every
    /// call received.
    pub fn wait_for_recorded(
        &self,
        description: &str,
        deadline: Duration,
        condition: impl Fn(&[RecordedMethodCall]) -> bool,
    ) -> Vec<RecordedMethodCall> {
        let started = Instant::now();
        loop {
            let recorded = self.recorded_method_calls();
            if condition(&recorded) {
                return recorded;
            }
            assert!(
                started.elapsed() < deadline,
                "\"{description}\" did not happen within {deadline:?}; the Simulated Phone received {recorded:#?}"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Checks that the method calls the Simulated Phone has received satisfy
    /// `condition` for the whole of `window`, for something the Phone must
    /// not be asked, such as to hang up. Returns the calls at the end.
    ///
    /// # Panics
    ///
    /// At the first check that fails `condition`, naming `description` and
    /// every call received.
    pub fn recorded_holds(
        &self,
        description: &str,
        window: Duration,
        condition: impl Fn(&[RecordedMethodCall]) -> bool,
    ) -> Vec<RecordedMethodCall> {
        let started = Instant::now();
        loop {
            let recorded = self.recorded_method_calls();
            assert!(
                condition(&recorded),
                "\"{description}\" stopped holding; the Simulated Phone received {recorded:#?}"
            );
            if started.elapsed() >= window {
                return recorded;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Declines the ringing Call at `call` on the Phone, as the person holding
    /// it does. Returns once the Phone has removed the Call.
    ///
    /// # Panics
    ///
    /// If there is no such Call, or it isn't ringing.
    pub fn decline_on_phone(&self, call: &str) {
        zbus::block_on(async {
            let call = zbus::ObjectPath::try_from(call)?;
            self.proxy().await?.decline_on_phone(&call).await
        })
        .unwrap_or_else(|error| panic!("decline the Call at {call} on the Phone: {error}"));
    }

    /// Ends the Call at `call` from the other end, as the caller hanging up
    /// does, whether it is ringing or live. Returns once the Phone has
    /// removed the Call.
    ///
    /// # Panics
    ///
    /// If there is no such Call.
    pub fn caller_hangs_up(&self, call: &str) {
        zbus::block_on(async {
            let call = zbus::ObjectPath::try_from(call)?;
            self.proxy().await?.caller_hangs_up(&call).await
        })
        .unwrap_or_else(|error| panic!("hang up the Call at {call} as the caller: {error}"));
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
    /// Private, so no test can assert on a snapshot: see
    /// [`Self::wait_for_recorded`].
    fn recorded_method_calls(&self) -> Vec<RecordedMethodCall> {
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
    let (state, contacts, call_history, deadline_ms, reason) =
        <(String, u32, u32, u64, String)>::try_from(property("ContactsState")?)?;
    let problems = <Vec<(String, String, String)>>::try_from(property("Problems")?)?
        .into_iter()
        .map(|(kind, message, action)| AppProblem {
            kind,
            message,
            action,
        })
        .collect();
    Ok(AppState {
        connection_state,
        phone: (!address.is_empty()).then_some(PhoneIdentity { name, address }),
        calls,
        audio_location: String::try_from(property("AudioLocation")?)?,
        ringing: bool::try_from(property("Ringing")?)?,
        muted: bool::try_from(property("Muted")?)?,
        codec,
        wideband,
        surfaces,
        contacts: AppContactsState {
            state,
            contacts,
            call_history,
            deadline_ms,
            reason,
        },
        problems,
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
