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

use std::collections::BTreeMap;
use std::fs;
use std::future::Future;
use std::io::{BufRead as _, BufReader};
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::process::{Child, ChildStdout, Command, Output, Stdio};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::thread;
use std::time::{Duration, Instant};

mod client;
mod input;
mod process_group;
mod redraw;
mod screen;
mod screencopy;
mod stand_ins;
mod timeline;
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

use crate::process_group::ProcessGroup;
use crate::redraw::OnScreen;
use crate::screen::{Screen, ScreenKind};
use crate::screencopy::Recorder;
use crate::stand_ins::StandIns;
use crate::timeline::{Timeline, unique_name};

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

/// The latest frame in the run of frames, starting with `found`, that
/// satisfy `shown`, up to the first that doesn't. Frames passed over, and
/// any left in `frames`, are deleted.
fn latest_shown(
    mut found: Screenshot,
    frames: &mpsc::Receiver<Screenshot>,
    shown: impl Fn(&Screenshot) -> bool,
) -> Screenshot {
    let mut run_ended = false;
    for frame in frames.try_iter() {
        if !run_ended && shown(&frame) {
            let _ = fs::remove_file(std::mem::replace(&mut found, frame).path);
        } else {
            run_ended = true;
            let _ = fs::remove_file(frame.path);
        }
    }
    found
}

/// The script `just install` runs.
const INSTALL_SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tools/install.py");

/// The environment variable that makes passing scenarios keep their
/// directories, as failing ones always do: `KEEP_SCENARIO_LOGS=1 just test`.
/// CI sets it when run by hand with `keep_scenario_logs`.
pub const KEEP_SCENARIO_LOGS: &str = "KEEP_SCENARIO_LOGS";

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

    fn ring_with_caller_id(&self, caller_id: &str) -> zbus::Result<zbus::OwnedObjectPath>;

    fn set_codec(&self, codec: u8) -> zbus::Result<()>;

    fn enable_quirk(&self, name: &str) -> zbus::Result<()>;

    fn reregister_call(&self, call: &zbus::ObjectPath<'_>, after_ms: u32) -> zbus::Result<()>;

    fn restart_telephony(&self, gap_ms: u32) -> zbus::Result<()>;

    fn alert(&self) -> zbus::Result<zbus::OwnedObjectPath>;

    fn callee_answers(&self) -> zbus::Result<()>;

    fn dial_on_phone(&self, number: &str) -> zbus::Result<zbus::OwnedObjectPath>;

    fn set_earbuds(&self, connected: bool) -> zbus::Result<()>;

    fn refuse_next_dial(&self, error: &str) -> zbus::Result<()>;
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

/// The `Surfaces` property: each shown surface (`"banner"`,
/// `"call-window"`) with its key content, as the surface shows it.
pub type ShownSurfaces = BTreeMap<String, BTreeMap<String, String>>;

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

/// One row of the Dialer, as the app's `Search` method returns it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppDialerRow {
    /// `"contact"`, `"call-history"` or `"call-number"` (the number typed).
    pub kind: String,
    /// For a Call History row, `"incoming"`, `"outgoing"` or `"missed"`;
    /// otherwise `""`.
    pub call_kind: String,
    /// The Contact's name, or the name the Phone gave, or `""`.
    pub name: String,
    /// The number's label, such as `"mobile"`, or `""`.
    pub number_label: String,
    /// The number Enter dials, or `""` if it was withheld or can't be dialled.
    pub number: String,
    /// The number grouped for reading, or `""` if withheld.
    pub shown_number: String,
    /// For a Call History row, when, in milliseconds since the Unix epoch;
    /// otherwise 0.
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

/// One `MissedCall` signal from the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissedCall {
    /// The latest Missed Call's id.
    pub call: u32,
    /// The caller's number, or `""` if withheld.
    pub number: String,
    /// The Contact's name, or `""`.
    pub display_name: String,
    /// How many Missed Calls from this caller the notification tells of.
    pub count: u32,
    /// When the latest one started ringing, in milliseconds since the Unix
    /// epoch.
    pub at_ms: u64,
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
    installed: bool,
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

    /// Installs Desktop Calls into the scenario's home as `just install`
    /// does, and leaves the service stopped: the session bus starts it
    /// through D-Bus activation once something calls it, as the user's
    /// session bus does.
    pub fn installed(mut self) -> Self {
        self.installed = true;
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
        let timeline = Timeline::start(&directory);
        let group = ProcessGroup::start(&directory);
        let mut processes = Vec::new();

        // A service the session bus starts runs inside it, so where the app
        // is installed, the session bus too runs with the journal hidden.
        let [system_bus, session_bus] =
            [("system", false), ("session", self.installed)].map(|(bus, hide_journal)| {
                let address = start_bus(&directory, bus, hide_journal, &group, &mut processes);
                timeline
                    .monitor(&address, bus)
                    .unwrap_or_else(|error| panic!("monitor the private {bus} bus: {error}"));
                address
            });
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
            let screen = Screen::start(kind, &directory, &environment, &group, &mut processes);
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
        share_environment(&session, &environment);
        let system =
            zbus::block_on(zbus::connection::Builder::address(system_bus.as_str()).build())
                .expect("connect the harness to the private system bus");
        timeline.label(&unique_name(&session), "harness");
        timeline.label(&unique_name(&system), "harness");

        let stand_ins = StandIns::start(&session_bus, self.theme)
            .expect("serve the desktop stand-ins on the private session bus");
        // Subscribe before the app starts, so no Error signal is missed.
        let app_errors = collect_app_errors(&session);
        let missed_calls = collect_missed_calls(&session);
        let surfaces_seen = collect_surfaces(&session);

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
            test_clients: Mutex::default(),
            screenshot_directory,
            app_errors,
            missed_calls,
            surfaces_seen,
            processes,
            timeline,
            group,
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

        harness.start_simulated_phone(self.scenario.as_deref());
        if self.phone_connected {
            harness.simulated_phone().connect();
        }

        if self.installed {
            harness.install();
        } else {
            harness.start_app();
        }
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

    /// Captures the screen as `<name>.png` in the screenshot directory, once
    /// it has finished redrawing (see `redraw.rs`), so the capture shows
    /// every state change reported so far.
    ///
    /// # Panics
    ///
    /// If the scenario has no screen, the screen doesn't finish redrawing,
    /// or the capture fails.
    #[must_use]
    pub fn screenshot(&self, name: &str) -> Screenshot {
        self.wait_for_redraw();
        Screen::screenshot(
            &self.environment,
            &self.group,
            &self.screenshot_directory.join(format!("{name}.png")),
        )
    }

    /// Captures only `region` of the screen, as `<name>.png` in the
    /// screenshot directory. A small region takes a fraction of a full
    /// capture's time, for checks that are timed. The capture's own
    /// coordinates start at the region's top-left corner. Like
    /// [`Harness::screenshot`], it waits for the screen to finish redrawing.
    ///
    /// # Panics
    ///
    /// If the scenario has no screen, the screen doesn't finish redrawing,
    /// or the capture fails.
    #[must_use]
    pub fn screenshot_of(&self, name: &str, region: Region) -> Screenshot {
        self.wait_for_redraw();
        Screen::screenshot_of(
            &self.environment,
            &self.group,
            &self.screenshot_directory.join(format!("{name}.png")),
            region,
        )
    }

    /// Captures the screen until a capture satisfies `shown`, and returns
    /// that capture, kept as `<name>.png`. What the app reports can run a
    /// frame ahead of what is on screen; this waits for the screen.
    ///
    /// The screen is recorded on a thread of its own: its content when this
    /// is called, then a frame each time anything on screen changes, as the
    /// compositor reports it (see `screencopy.rs`). `shown` judges the frames
    /// in order as they arrive. So a state on screen for less time than one
    /// `shown` takes, such as the Call Window's "Call ended" for 2 s, is
    /// still caught, and a still screen costs no CPU that the app needs to
    /// draw. At most [`FRAMES_AHEAD`] frames wait for `shown` at a time.
    ///
    /// Once a frame satisfies `shown`, this waits for the screen to finish
    /// redrawing, as every action does (see `redraw.rs`): a window can match
    /// while still mid-resize, and a test that clicks where the capture shows
    /// a button needs where it ends up. It then returns a capture of the
    /// redrawn screen if that still satisfies `shown`; otherwise the state
    /// was brief and has gone, and it returns the latest frame of the run
    /// that satisfied `shown`.
    ///
    /// # Panics
    ///
    /// If no frame within `deadline` satisfies `shown`; the panic names the
    /// last frame, kept as `<name>.png`. Also if the recording fails.
    #[must_use]
    pub fn screenshot_when(
        &self,
        name: &str,
        deadline: Duration,
        shown: impl Fn(&Screenshot) -> bool,
    ) -> Screenshot {
        let give_up = Instant::now() + deadline;
        let captures = self.directory.join("captures");
        let (mut recorder, stopper) = Recorder::connect(&self.screen().socket, &captures)
            .unwrap_or_else(|error| panic!("record the screen: {error}"));
        let (found, last, failed) = thread::scope(|scope| {
            let (sender, receiver) = mpsc::sync_channel(FRAMES_AHEAD);
            let recording = scope.spawn(move || {
                for count in 1_u32.. {
                    let path = captures.join(format!("{name}-{count}.ppm"));
                    let (width, height) = recorder.next_frame(&path)?;
                    if sender
                        .send(Screenshot {
                            path,
                            width,
                            height,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                Ok::<_, String>(())
            });
            let mut last: Option<Screenshot> = None;
            let mut found = None;
            let mut ended_early = false;
            while let Some(remaining) = give_up.checked_duration_since(Instant::now()) {
                let frame = match receiver.recv_timeout(remaining) {
                    Ok(frame) => frame,
                    Err(mpsc::RecvTimeoutError::Timeout) => break,
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        ended_early = true;
                        break;
                    }
                };
                if shown(&frame) {
                    self.wait_for_redraw();
                    found = Some(latest_shown(frame, &receiver, &shown));
                    break;
                }
                // Each frame is 25 MB; keep only the latest, for the panic.
                if let Some(previous) = last.replace(frame) {
                    let _ = fs::remove_file(previous.path);
                }
            }
            stopper.stop();
            drop(receiver);
            let ended = recording
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
            (found, last, ended.err().filter(|_| ended_early))
        });
        if let Some(error) = failed {
            panic!("recording the screen for {name} failed: {error}");
        }
        let kept = |capture: &Screenshot| {
            capture.save_png(&self.screenshot_directory.join(format!("{name}.png")))
        };
        match (found, last) {
            (Some(capture), _) => {
                let redrawn = self.capture_now(&format!("{name}-redrawn.ppm"));
                if shown(&redrawn) {
                    let _ = fs::remove_file(&capture.path);
                    kept(&redrawn)
                } else {
                    let _ = fs::remove_file(&redrawn.path);
                    kept(&capture)
                }
            }
            (None, Some(capture)) => panic!(
                "the screen never showed {name} within {deadline:?}; see {}{}",
                kept(&capture).path.display(),
                self.timeline.render()
            ),
            (None, None) => panic!(
                "no capture of {name} was made within {deadline:?}{}",
                self.timeline.render()
            ),
        }
    }

    /// One frame of the screen as it is now, as a PPM under `captures/`.
    fn capture_now(&self, file: &str) -> Screenshot {
        let captures = self.directory.join("captures");
        let path = captures.join(file);
        let (width, height) = Recorder::connect(&self.screen().socket, &captures)
            .and_then(|(mut recorder, _)| recorder.next_frame(&path))
            .unwrap_or_else(|error| panic!("capture the screen: {error}"));
        Screenshot {
            path,
            width,
            height,
        }
    }

    /// Waits until the screen has finished redrawing after every state
    /// change so far. Every action that clicks, types or takes a screenshot
    /// calls this first, so no scenario can race a redraw. See `redraw.rs`.
    fn wait_for_redraw(&self) {
        let compositor = self.screen().pid;
        let on_screen = || {
            let app = self
                .processes
                .iter()
                .filter(|(name, _)| *name == "desktop-calls")
                .map(|(_, app)| app.id());
            let test_clients = self
                .test_clients
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            OnScreen {
                compositor,
                clients: app.chain(test_clients).collect(),
            }
        };
        if let Err(error) = redraw::wait_for_redraw(on_screen, REDRAW_DEADLINE) {
            panic!("{error}{}", self.timeline.render());
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
        self.group
            .adopt(&mut command)
            .env_clear()
            .envs(self.environment.iter().cloned());
        command
    }

    /// The notification server stand-in.
    #[must_use]
    pub fn notifications(&self) -> Notifications<'_> {
        Notifications {
            stand_ins: &self.stand_ins,
            timeline: &self.timeline,
        }
    }

    /// The tray watcher stand-in.
    #[must_use]
    pub fn tray(&self) -> Tray<'_> {
        Tray {
            stand_ins: &self.stand_ins,
            timeline: &self.timeline,
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
        self.screen();
        Keyboard { harness: self }
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
                "the clipboard never held {text:?}; wl-paste said {:?}{}",
                String::from_utf8_lossy(&pasted.stderr),
                self.timeline.render()
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
        self.screen();
        Pointer { harness: self }
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

    /// Starts the harness's test client as an app slow to redraw: each key
    /// it gets is a state change, which it reports at once and draws only
    /// after 300 ms of work and 20 frames. See `src/bin/test-client.rs`.
    ///
    /// # Panics
    ///
    /// If the scenario has no screen, or the client can't be started.
    #[must_use]
    pub fn start_slowly_redrawing_test_client(&self) -> TestClient {
        self.spawn_test_client(&["--slow-redraw"])
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
        self.test_clients
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(child.id());
        TestClient::new(child, Arc::clone(&self.timeline))
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
                "\"{description}\" did not happen within {deadline:?}; the app's state is {:?}{}",
                zbus::block_on(read_app_state(&self.session)),
                self.timeline.render()
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
                panic!(
                    "\"{description}\" stopped holding; the app's state is {state:?}{}",
                    self.timeline.render()
                )
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
                "the app logged nothing containing {needle:?} within {deadline:?}; see {}{}",
                log.display(),
                self.timeline.render()
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

    /// Asks the app to move the Calls' audio to `location`, `"desktop"` or
    /// `"phone"`, and returns the error the control interface answers with,
    /// if any.
    ///
    /// # Errors
    ///
    /// The app's refusal: its D-Bus error name and plain-language message.
    pub fn move_audio(&self, location: &str) -> Result<(), AppError> {
        self.try_app_method("MoveAudio", &(location,))
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

    /// Calls the app's `Activate`, as the keybinding does, without the
    /// CLI's process start in the way of a timing.
    ///
    /// # Panics
    ///
    /// If the control interface doesn't answer.
    pub fn activate(&self) {
        if let Err(error) = self.try_app_method("Activate", &()) {
            panic!("Activate on the app: {error:?}");
        }
    }

    /// Calls the app's `OpenDialer` with `query` typed and the Call with id
    /// `call` selected (0 for none).
    ///
    /// # Panics
    ///
    /// If the control interface doesn't answer.
    pub fn open_dialer(&self, query: &str, call: u32) {
        if let Err(error) = self.try_app_method("OpenDialer", &(query, call)) {
            panic!("OpenDialer on the app: {error:?}");
        }
    }

    /// Starts the app from the app launcher: installs its desktop file and
    /// launches it, as a launcher does, with no link.
    ///
    /// # Panics
    ///
    /// If `gio` can't launch it.
    pub fn launch_from_app_launcher(&self) {
        let applications = self.directory.join("home/.local/share/applications");
        fs::create_dir_all(&applications).expect("create the applications directory");
        let desktop_file = Path::new(DESKTOP_FILE);
        let name = desktop_file
            .file_name()
            .expect("the desktop file has a name");
        let installed = applications.join(name);
        fs::copy(desktop_file, &installed).expect("install the desktop file");
        let path = std::env::var("PATH").unwrap_or_default();
        let output = self
            .external_command("gio")
            .arg("launch")
            .arg(&installed)
            .env("PATH", format!("{}:{path}", self.bin_directory.display()))
            .output()
            .expect("run gio");
        assert!(
            output.status.success(),
            "gio launch failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// The Dialer's rows for `query`, best first, from the app's `Search`
    /// method.
    ///
    /// # Panics
    ///
    /// If the control interface doesn't answer.
    #[must_use]
    pub fn search(&self, query: &str) -> Vec<AppDialerRow> {
        let reply = zbus::block_on(self.session.call_method(
            Some(APP_BUS_NAME),
            APP_PATH,
            Some(APP_INTERFACE),
            "Search",
            &(query,),
        ))
        .unwrap_or_else(|error| panic!("Search({query:?}) on the app: {error}"));
        reply
            .body()
            .deserialize::<Vec<(String, String, String, String, String, String, u64)>>()
            .expect("Search replies with a(sssssst)")
            .into_iter()
            .map(
                |(kind, call_kind, name, number_label, number, shown_number, at_ms)| AppDialerRow {
                    kind,
                    call_kind,
                    name,
                    number_label,
                    number,
                    shown_number,
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
                "the app sent no Error within {deadline:?}{}",
                self.timeline.render()
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

    /// Every `MissedCall` signal the app has sent so far, in order.
    #[must_use]
    pub fn missed_calls(&self) -> Vec<MissedCall> {
        self.missed_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Every value of the app's `Surfaces` property signalled since the
    /// scenario started, in order: everything the surfaces showed, however
    /// briefly.
    #[must_use]
    pub fn surfaces_seen(&self) -> Vec<ShownSurfaces> {
        self.surfaces_seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Asks the app to Call `number`, a number or a `tel:` link, as any
    /// client of the control interface could. Returns the error's message
    /// if the app refuses.
    ///
    /// # Errors
    ///
    /// The message of the app's refusal.
    pub fn dial(&self, number: &str) -> Result<(), String> {
        zbus::block_on(self.session.call_method(
            Some(APP_BUS_NAME),
            APP_PATH,
            Some(APP_INTERFACE),
            "Dial",
            &(number,),
        ))
        .map(drop)
        .map_err(|error| match error {
            zbus::Error::MethodError(_, Some(message), _) => message,
            other => other.to_string(),
        })
    }

    /// Opens `link` as a Desktop app would, with `gio open`, after
    /// installing the app's desktop file as the user's handler for its
    /// scheme and putting the app on `PATH`, as installing it does. Returns
    /// once `gio` has launched the handler.
    ///
    /// # Panics
    ///
    /// If the desktop file can't be installed, or `gio` fails.
    pub fn open_link(&self, link: &str) {
        let applications = self.directory.join("home/.local/share/applications");
        fs::create_dir_all(&applications).expect("create the applications directory");
        let desktop_file = Path::new(DESKTOP_FILE);
        let name = desktop_file
            .file_name()
            .expect("the desktop file has a name");
        fs::copy(desktop_file, applications.join(name)).expect("install the desktop file");
        let name = name.to_string_lossy();
        fs::write(
            self.directory.join("config/mimeapps.list"),
            format!("[Default Applications]\nx-scheme-handler/tel={name}\n"),
        )
        .expect("make the app the handler for tel: links");
        let path = std::env::var("PATH").unwrap_or_default();
        let output = self
            .external_command("gio")
            .arg("open")
            .arg(link)
            .env("PATH", format!("{}:{path}", self.bin_directory.display()))
            .output()
            .expect("run gio");
        assert!(
            output.status.success(),
            "gio open {link} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
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

/// The notification server stand-in. See [`Harness::notifications`].
pub struct Notifications<'harness> {
    stand_ins: &'harness StandIns,
    timeline: &'harness Timeline,
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
                    "\"{description}\" did not happen within {deadline:?}; received {received:#?}{}",
                    self.timeline.render()
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

    /// Presses notification `id`'s button for action `key`, such as
    /// `"call-back"`, and closes it, as Omarchy does.
    ///
    /// # Panics
    ///
    /// If the notification offered no such action.
    pub fn invoke_action(&self, id: u32, key: &str) {
        if let Err(error) = self.stand_ins.invoke_action(id, key) {
            panic!("press {key:?} on notification {id}: {error}");
        }
    }
}

/// The tray watcher stand-in. See [`Harness::tray`].
pub struct Tray<'harness> {
    stand_ins: &'harness StandIns,
    timeline: &'harness Timeline,
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
                panic!(
                    "\"{description}\" did not happen within {deadline:?}: {error}{}",
                    self.timeline.render()
                )
            })
    }
}

/// The screen's synthetic keyboard. See [`Harness::keyboard`]. Keys go to
/// the screen as it is once it has finished redrawing (see `redraw.rs`).
pub struct Keyboard<'harness> {
    harness: &'harness Harness,
}

impl Keyboard<'_> {
    /// Types `text` with a US layout, holding Shift where a character needs it.
    ///
    /// # Panics
    ///
    /// If `text` has a character the US layout can't type, the screen
    /// doesn't finish redrawing, or the compositor is gone.
    pub fn type_text(&self, text: &str) {
        self.harness.wait_for_redraw();
        if let Err(error) = self.harness.screen().input().type_text(text) {
            panic!("type {text:?}: {error}");
        }
    }

    /// Presses and releases `key`.
    ///
    /// # Panics
    ///
    /// If the screen doesn't finish redrawing, or the compositor is gone.
    pub fn press(&self, key: Key) {
        self.harness.wait_for_redraw();
        if let Err(error) = self.harness.screen().input().press(key) {
            panic!("press {key:?}: {error}");
        }
    }

    /// Presses `character`'s key while holding Control, such as Ctrl+V.
    ///
    /// # Panics
    ///
    /// If the US layout has no unshifted key for `character`, the screen
    /// doesn't finish redrawing, or the compositor is gone.
    pub fn press_with_control(&self, character: char) {
        self.harness.wait_for_redraw();
        if let Err(error) = self.harness.screen().input().press_with_control(character) {
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
/// screen is [`LAYOUT_SIZE`]. See [`Harness::pointer`]. It acts on the
/// screen as it is once it has finished redrawing (see `redraw.rs`).
pub struct Pointer<'harness> {
    harness: &'harness Harness,
}

impl Pointer<'_> {
    /// Moves the pointer to (`x`, `y`).
    ///
    /// # Panics
    ///
    /// If the point is off the screen, the screen doesn't finish redrawing,
    /// or the compositor is gone.
    pub fn move_to(&self, x: f64, y: f64) {
        self.harness.wait_for_redraw();
        self.move_now(x, y);
    }

    /// Moves the pointer to (`x`, `y`) and clicks the left button there.
    /// Moving onto a button can redraw it (hover), so the click waits for
    /// that redraw too.
    ///
    /// # Panics
    ///
    /// If the point is off the screen, the screen doesn't finish redrawing,
    /// or the compositor is gone.
    pub fn click(&self, x: f64, y: f64) {
        self.move_to(x, y);
        self.harness.wait_for_redraw();
        if let Err(error) = self.harness.screen().input().click() {
            panic!("click at ({x}, {y}): {error}");
        }
    }

    fn move_now(&self, x: f64, y: f64) {
        if let Err(error) = self.harness.screen().input().move_to(x, y, LAYOUT_SIZE) {
            panic!("move the pointer to ({x}, {y}): {error}");
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

    /// Rings the Phone with this caller ID as the Phone sends it to the
    /// Desktop, such as `"",129` for a withheld number, read as PipeWire
    /// reads it. Returns the new Call's object path.
    ///
    /// # Panics
    ///
    /// If the Simulated Phone refuses, as it does while hands-free is down.
    #[must_use]
    pub fn ring_with_caller_id(&self, caller_id: &str) -> String {
        zbus::block_on(async { self.proxy().await?.ring_with_caller_id(caller_id).await })
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

    /// Restarts the Telephony service, as restarting `WirePlumber` does: the
    /// service unannounces the AudioGateway's Calls and the AudioGateway,
    /// leaves the bus, and `gap` later comes back on a new connection with
    /// the same objects at the same paths, the Calls 40 ms after the
    /// AudioGateway. Returns once the service has left the bus. On the real
    /// Pixel the gap was 0.73 s.
    ///
    /// # Panics
    ///
    /// If the Simulated Phone refuses.
    pub fn restart_telephony(&self, gap: Duration) {
        let gap_ms = u32::try_from(gap.as_millis()).unwrap_or(u32::MAX);
        zbus::block_on(async { self.proxy().await?.restart_telephony(gap_ms).await })
            .expect("restart the Simulated Phone's Telephony service");
    }

    /// Makes the Call the Desktop dialled ring at the other end, as the
    /// Pixel reports it: the Call appears, `alerting`. Returns its path.
    ///
    /// # Panics
    ///
    /// If nothing was dialled.
    #[must_use]
    pub fn alert(&self) -> String {
        zbus::block_on(async { self.proxy().await?.alert().await })
            .expect("make the dialled Call ring at the other end")
            .to_string()
    }

    /// The person called answers the Call the Desktop dialled: it goes
    /// `active`.
    ///
    /// # Panics
    ///
    /// If there is no such Call.
    pub fn callee_answers(&self) {
        zbus::block_on(async { self.proxy().await?.callee_answers().await })
            .expect("answer the dialled Call at the other end");
    }

    /// Places a Call to `number` on the Phone itself, as the person holding
    /// it does: the Call appears `dialing`, and the audio stays on the Phone.
    /// Returns the Call's object path.
    ///
    /// # Panics
    ///
    /// If the Simulated Phone refuses, as it does while hands-free is down.
    #[must_use]
    pub fn dial_on_phone(&self, number: &str) -> String {
        zbus::block_on(async { self.proxy().await?.dial_on_phone(number).await })
            .expect("place a Call on the Simulated Phone")
            .to_string()
    }

    /// Connects earbuds to the Phone beside the Desktop (`true`), or
    /// disconnects them. Connected earbuds take the audio of Calls answered
    /// on them, and the Phone ignores `Activate` until the Desktop answers.
    ///
    /// # Panics
    ///
    /// If the Simulated Phone can't be reached.
    pub fn set_earbuds(&self, connected: bool) {
        zbus::block_on(async { self.proxy().await?.set_earbuds(connected).await })
            .expect("connect or disconnect the Simulated Phone's earbuds");
    }

    /// Makes the Phone refuse the next `Dial` with a Telephony error:
    /// `"invalid-format"`, `"invalid-state"`, `"not-supported"`,
    /// `"in-progress"`, `"failed"`, or `"cme:N"` for the Phone's `+CME ERROR: N`.
    ///
    /// # Panics
    ///
    /// If the Simulated Phone doesn't know the error.
    pub fn refuse_next_dial(&self, error: &str) {
        zbus::block_on(async { self.proxy().await?.refuse_next_dial(error).await })
            .unwrap_or_else(|failure| panic!("refuse the next Dial with {error:?}: {failure}"));
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
                "\"{description}\" did not happen within {deadline:?}; the Simulated Phone received {recorded:#?}{}",
                self.harness.timeline.render()
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
                "\"{description}\" stopped holding; the Simulated Phone received {recorded:#?}{}",
                self.harness.timeline.render()
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
    collect_signal(session, "Error", |message| {
        let (kind, message) = message.body().deserialize::<(String, String)>().ok()?;
        Some(AppError { kind, message })
    })
}

/// Collects every `MissedCall` signal the app sends on `session`, from now
/// on.
fn collect_missed_calls(session: &Connection) -> Arc<Mutex<Vec<MissedCall>>> {
    collect_signal(session, "MissedCall", |message| {
        let (call, number, display_name, count, at_ms) = message
            .body()
            .deserialize::<(u32, String, String, u32, u64)>()
            .ok()?;
        Some(MissedCall {
            call,
            number,
            display_name,
            count,
            at_ms,
        })
    })
}

/// Collects every signal `member` of the app's control interface on
/// `session`, from now on, as `read` reads it.
fn collect_signal<T: Send + 'static>(
    session: &Connection,
    member: &'static str,
    read: fn(&zbus::Message) -> Option<T>,
) -> Arc<Mutex<Vec<T>>> {
    let signals = Arc::new(Mutex::new(Vec::new()));
    let stream = zbus::block_on(async {
        let rule = MatchRule::builder()
            .msg_type(Type::Signal)
            .path(APP_PATH)
            .interface(APP_INTERFACE)
            .member(member)
            .build()?;
        MessageStream::for_match_rule(rule, session, None).await
    })
    .unwrap_or_else(|error| panic!("subscribe to the app's {member} signal: {error}"));
    let collected = signals.clone();
    thread::spawn(move || {
        zbus::block_on(async move {
            let mut stream = stream;
            while let Some(Ok(message)) = stream.next().await {
                if let Some(signal) = read(&message) {
                    collected
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push(signal);
                }
            }
        });
    });
    signals
}

/// Collects every value of the app's `Surfaces` property that its
/// `PropertiesChanged` signals carry, from now on.
fn collect_surfaces(session: &Connection) -> Arc<Mutex<Vec<ShownSurfaces>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let stream = zbus::block_on(async {
        let rule = MatchRule::builder()
            .msg_type(Type::Signal)
            .path(APP_PATH)
            .interface("org.freedesktop.DBus.Properties")
            .member("PropertiesChanged")
            .build()?;
        MessageStream::for_match_rule(rule, session, None).await
    })
    .expect("subscribe to the app's PropertiesChanged");
    let collected = seen.clone();
    thread::spawn(move || {
        zbus::block_on(async move {
            let mut stream = stream;
            while let Some(Ok(message)) = stream.next().await {
                let Ok((_, mut changed, _)) = message.body().deserialize::<(
                    String,
                    std::collections::HashMap<String, zbus::OwnedVariant>,
                    Vec<String>,
                )>() else {
                    continue;
                };
                let Some(surfaces) = changed.remove("Surfaces") else {
                    continue;
                };
                if let Ok(surfaces) = ShownSurfaces::try_from(surfaces) {
                    collected
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push(surfaces);
                }
            }
        });
    });
    seen
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

/// Gives the bus `session` is on `environment` for every service it starts,
/// as the Desktop's session does with `dbus-update-activation-environment`,
/// so such a service runs in the scenario's world.
fn share_environment(session: &Connection, environment: &[(&'static str, String)]) {
    zbus::block_on(async {
        DBusProxy::new(session)
            .await?
            .update_activation_environment(
                environment
                    .iter()
                    .map(|(name, value)| (*name, value.as_str()))
                    .collect(),
            )
            .await
    })
    .expect("give the private session bus the scenario's environment");
}

/// A command that runs `program` where the systemd journal can't be
/// reached, if it can be reached here. Arguments added to the command go to
/// `program`.
///
/// Where the journal is reachable, the app would log there, so `program`
/// runs in a mount namespace with an empty directory over the journal's. The
/// app then finds no journal and logs to its file under `XDG_STATE_HOME`,
/// exactly as it would on any machine without a journal. A second user
/// namespace maps the user back to their own ID, so `program` runs as the
/// same user it would outside.
fn without_journal(program: &Path) -> Command {
    if !Path::new(JOURNAL_SOCKET).exists() {
        return Command::new(program);
    }
    let user = fs::metadata("/proc/self").expect("read /proc/self");
    let mut command = Command::new("unshare");
    command
        .args(["--user", "--map-root-user", "--mount", "--", "sh", "-c"])
        .arg(concat!(
            "mount -t tmpfs none /run/systemd/journal && ",
            "uid=$1 gid=$2 && shift 2 && ",
            r#"exec unshare --user --map-user="$uid" --map-group="$gid" -- "$@""#
        ))
        .arg("sh")
        .arg(user.uid().to_string())
        .arg(user.gid().to_string())
        .arg(program);
    command
}

/// Starts a private `dbus-daemon` and returns its address. With
/// `hide_journal`, it and every service it starts run where the journal
/// can't be reached (see [`without_journal`]).
fn start_bus(
    directory: &Path,
    name: &'static str,
    hide_journal: bool,
    group: &ProcessGroup,
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
  <servicedir>{}</servicedir>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
"#,
            socket.display(),
            // Where the user's own D-Bus services are installed, as a
            // session bus looks for them.
            directory
                .join("home/.local/share/dbus-1/services")
                .display()
        ),
    )
    .expect("write the bus configuration");
    let mut command = if hide_journal {
        without_journal(Path::new("dbus-daemon"))
    } else {
        Command::new("dbus-daemon")
    };
    let mut child = group
        .adopt(&mut command)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
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
    let config = path_of("config");
    still_cursor(Path::new(&config));
    vec![
        ("DBUS_SYSTEM_BUS_ADDRESS", system_bus.to_owned()),
        ("DBUS_SESSION_BUS_ADDRESS", session_bus.to_owned()),
        ("HOME", path_of("home")),
        ("XDG_CONFIG_HOME", config),
        ("GSETTINGS_BACKEND", "keyfile".to_owned()),
        ("XDG_CACHE_HOME", path_of("cache")),
        ("XDG_STATE_HOME", path_of("state")),
        ("XDG_RUNTIME_DIR", runtime),
        ("PATH", std::env::var("PATH").unwrap_or_default()),
        ("RUST_LOG", "debug".to_owned()),
        ("RUST_BACKTRACE", "1".to_owned()),
    ]
}

/// Turns the text cursor's blinking off, as a Desktop setting, so a still
/// surface leaves the screen still: GTK fades the cursor in and out over
/// many frames, which would keep [`Harness`]'s actions waiting for the
/// screen to settle for up to 10 s after every key. GTK reads the setting
/// from `GSettings`, here kept in a key file in `config` (`GSETTINGS_BACKEND`
/// is GLib's own variable, read by every GLib program), and from
/// `gtk-4.0/settings.ini` where the `GSettings` schema is not installed.
fn still_cursor(config: &Path) {
    let write = |path: PathBuf, text: &str| {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create a settings directory");
        }
        fs::write(path, text).expect("write the Desktop's settings");
    };
    write(
        config.join("glib-2.0/settings/keyfile"),
        "[org/gnome/desktop/interface]\ncursor-blink=false\n",
    );
    write(
        config.join("gtk-4.0/settings.ini"),
        "[Settings]\ngtk-cursor-blink=false\n",
    );
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
