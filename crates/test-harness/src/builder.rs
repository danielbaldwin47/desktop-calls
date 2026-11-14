//! Setting up a scenario's world: the private buses, the temporary home, the
//! environment, the compositor and the processes.

use std::fs;
use std::io::{BufRead as _, BufReader};
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use zbus::Connection;
use zbus::fdo::DBusProxy;

use super::app::{collect_app_errors, collect_missed_calls, collect_published};
use super::simulated_phone::PhoneIdentity;
use super::{Harness, JOURNAL_SOCKET};
use crate::process_group::ProcessGroup;
use crate::screen::{Screen, ScreenKind};
use crate::stand_ins::StandIns;
use crate::timeline::{Timeline, unique_name};
use crate::{OmarchyTheme, Theme, hyprland, omarchy_quiet, stand_ins};

/// Whether the Desktop animates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Motion {
    /// GTK animates, as it does by default.
    #[default]
    Full,
    /// The Desktop asks for reduced motion: GTK's `gtk-enable-animations`
    /// is off.
    Reduced,
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
    phone_name: Option<String>,
    omarchy_theme: Option<OmarchyTheme>,
    hyprland: Option<hyprland::Config>,
    installed: bool,
    motion: Motion,
    notification_state: NotificationState,
    /// The Simulated Phone's arguments saying who is nearby, and what the
    /// Desktop's Bluetooth is like.
    nearby: Vec<String>,
    /// The Desktop's `WirePlumber` drop-ins, or `None` for audio set up for
    /// Calls.
    wireplumber: Option<Vec<WirePlumberEntry>>,
}

/// One entry in the Desktop's `WirePlumber` drop-in folder.
enum WirePlumberEntry {
    /// A file, with its permission bits if they aren't the umask's.
    File {
        name: String,
        text: String,
        mode: Option<u32>,
    },
    /// A symbolic link to a file of the same name in `DOTFILES`.
    Link {
        name: String,
        text: String,
        mode: u32,
    },
    Folder {
        name: String,
    },
}

/// Where a linked `WirePlumber` drop-in's file is, under `config`.
pub(super) const DOTFILES: &str = "dotfiles";

/// The drop-in the Desktop has unless a scenario says otherwise: audio set
/// up for Calls, as on the user's Desktop, in one file of its own.
const AUDIO_SET_UP: &str = "\
monitor.bluez.properties = {
  bluez5.telephony.default-reject-sco = true
}
monitor.bluez.rules = [
  {
    matches = [ { device.name = \"~bluez_card.*\" } ]
    actions = { update-props = { bluez5.auto-connect = [ a2dp_sink a2dp_source hfp_hf ] } }
  }
]
";

/// Where `WirePlumber` reads the user's drop-ins, under `config`.
pub(super) const WIREPLUMBER_DROP_INS: &str = "wireplumber/wireplumber.conf.d";

/// Leaves the Desktop's `WirePlumber` drop-ins under `config` as `entries`
/// say, or set up for Calls.
fn write_wireplumber(config: &Path, entries: Option<Vec<WirePlumberEntry>>) {
    let folder = config.join(WIREPLUMBER_DROP_INS);
    fs::create_dir_all(&folder).expect("create the WirePlumber drop-in folder");
    let entries = entries.unwrap_or_else(|| {
        vec![WirePlumberEntry::File {
            name: "55-audio-for-calls.conf".to_owned(),
            text: AUDIO_SET_UP.to_owned(),
            mode: None,
        }]
    });
    for entry in entries {
        match entry {
            WirePlumberEntry::File { name, text, mode } => {
                let path = folder.join(name);
                fs::write(&path, text).and_then(|()| {
                    mode.map_or(Ok(()), |mode| {
                        fs::set_permissions(&path, fs::Permissions::from_mode(mode))
                    })
                })
            }
            WirePlumberEntry::Link { name, text, mode } => {
                let dotfiles = config.join(DOTFILES);
                let target = dotfiles.join(&name);
                fs::create_dir_all(&dotfiles)
                    .and_then(|()| fs::write(&target, text))
                    .and_then(|()| fs::set_permissions(&target, fs::Permissions::from_mode(mode)))
                    .and_then(|()| std::os::unix::fs::symlink(&target, folder.join(name)))
            }
            WirePlumberEntry::Folder { name } => fs::create_dir(folder.join(name)),
        }
        .expect("write a WirePlumber drop-in");
    }
}

/// What Omarchy's Do Not Disturb state file holds when the app starts.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum NotificationState {
    /// Empty, as Omarchy's shell creates it: off.
    #[default]
    Prepared,
    /// Written with Do Not Disturb on.
    DoNotDisturb,
    /// Not there at all: no Omarchy.
    Absent,
}

impl NotificationState {
    /// Leaves Omarchy's Do Not Disturb state file under `home` like this.
    fn set_up(self, home: &Path) {
        match self {
            Self::Prepared => omarchy_quiet::prepare_state(home),
            Self::DoNotDisturb => omarchy_quiet::write_do_not_disturb(home, true),
            Self::Absent => Ok(()),
        }
        .expect("set up Omarchy's Do Not Disturb state file");
    }
}

/// Serves the Hyprland stand-in with `config` in the scenario's
/// `directory`, and names it in `environment` as Hyprland names itself.
fn serve_hyprland(
    config: hyprland::Config,
    directory: &Path,
    environment: &mut Vec<(&'static str, String)>,
) -> hyprland::Hyprland {
    let hyprland = hyprland::serve_hyprland(&directory.join("runtime"), config)
        .expect("serve the Hyprland stand-in");
    environment.push((
        "HYPRLAND_INSTANCE_SIGNATURE",
        hyprland::HYPRLAND_SIGNATURE.to_owned(),
    ));
    hyprland
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

    /// Runs the app under a Hyprland stand-in whose `general:gaps_out` is
    /// `gaps_out` on every side. Without it the stand-in has no
    /// `general:gaps_out` to report, and without any Hyprland setting there
    /// is no Hyprland, as under sway. [`Harness::set_hyprland_gaps_out`]
    /// changes it later.
    pub fn hyprland_gaps_out(mut self, gaps_out: u32) -> Self {
        self.hyprland.get_or_insert_default().gaps_out = Some(gaps_out);
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

    /// Names the Simulated Phone `name`, as the user's Phone is named in its
    /// Bluetooth settings. "Simulated Pixel" by default.
    pub fn phone_name(mut self, name: &str) -> Self {
        self.phone_name = Some(name.to_owned());
        self
    }

    /// A fresh install: the Simulated Phone is nearby and not paired with
    /// the Desktop, so no Phone is set up.
    pub fn phone_unpaired(mut self) -> Self {
        self.nearby.push("--unpaired".to_owned());
        self
    }

    /// Headphones named `name` are nearby, not paired: discovery finds them
    /// beside the Phone.
    pub fn headphones_nearby(mut self, name: &str) -> Self {
        self.nearby.push("--headphones".to_owned());
        self.nearby.push(name.to_owned());
        self
    }

    /// The Desktop's Bluetooth is off; turning it on works.
    pub fn bluetooth_off(mut self) -> Self {
        self.nearby
            .extend(["--adapter-power", "off"].map(str::to_owned));
        self
    }

    /// The Desktop's Bluetooth is off and blocked by rfkill, as airplane
    /// mode leaves it: turning it on fails.
    pub fn bluetooth_blocked(mut self) -> Self {
        self.nearby
            .extend(["--adapter-power", "blocked"].map(str::to_owned));
        self
    }

    /// The Desktop's Bluetooth file-transfer service, which syncs Contacts,
    /// isn't installed: nobody serves `org.bluez.obex`.
    pub fn without_contacts_support(mut self) -> Self {
        self.nearby.push("--without-obex".to_owned());
        self
    }

    /// BlueZ's own experimental hands-free is on and holds the Phone's
    /// hands-free link: the Phone's device has an `org.bluez.Telephony1`.
    pub fn bluez_hands_free_on(mut self) -> Self {
        self.nearby.push("--bluez-telephony".to_owned());
        self
    }

    /// The job that restarts `WirePlumber` ends `result`, as systemd's
    /// would report it: `"failed"` or `"timeout"`, or `"never"` for a job
    /// that doesn't end. `WirePlumber` doesn't restart in any of them.
    pub fn wireplumber_restart_job(mut self, result: &str) -> Self {
        self.nearby
            .extend(["--wireplumber-restart-job", result].map(str::to_owned));
        self
    }

    /// Puts a `WirePlumber` drop-in named `name` holding `text` in the
    /// Desktop's user drop-in folder. Without any, the folder holds one
    /// that sets audio up for Calls.
    pub fn wireplumber_config(mut self, name: &str, text: &str) -> Self {
        self.wireplumber
            .get_or_insert_default()
            .push(WirePlumberEntry::File {
                name: name.to_owned(),
                text: text.to_owned(),
                mode: None,
            });
        self
    }

    /// Like [`Builder::wireplumber_config`], with the file's permission
    /// bits set to `mode` (for example `0o640`).
    pub fn wireplumber_config_with_mode(mut self, name: &str, text: &str, mode: u32) -> Self {
        self.wireplumber
            .get_or_insert_default()
            .push(WirePlumberEntry::File {
                name: name.to_owned(),
                text: text.to_owned(),
                mode: Some(mode),
            });
        self
    }

    /// Like [`Builder::wireplumber_config_with_mode`], but the drop-in is a
    /// symbolic link, as a dotfiles manager leaves it, to a file of that
    /// name outside the drop-in folder that holds `text` and `mode`.
    pub fn wireplumber_config_linked(mut self, name: &str, text: &str, mode: u32) -> Self {
        self.wireplumber
            .get_or_insert_default()
            .push(WirePlumberEntry::Link {
                name: name.to_owned(),
                text: text.to_owned(),
                mode,
            });
        self
    }

    /// Puts a folder named `name` in the Desktop's `WirePlumber` drop-in
    /// folder, where no file can then be written.
    pub fn wireplumber_config_folder(mut self, name: &str) -> Self {
        self.wireplumber
            .get_or_insert_default()
            .push(WirePlumberEntry::Folder {
                name: name.to_owned(),
            });
        self
    }

    /// Another Phone named `name` is paired with the Desktop, and out of
    /// range: the Phone the user switches from. It lists before the Phone
    /// in BlueZ's objects, by its address.
    /// `SimulatedPhone::connect_other_phone` brings it into range.
    pub fn old_phone_paired(mut self, name: &str) -> Self {
        self.nearby.push("--old-phone".to_owned());
        self.nearby.push(name.to_owned());
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

    /// Turns animations off, as a Desktop set for reduced motion: GTK's
    /// `gtk-enable-animations` is false.
    pub fn reduced_motion(mut self) -> Self {
        self.motion = Motion::Reduced;
        self
    }

    /// Turns Omarchy's Do Not Disturb on before the app starts, as a
    /// Desktop that was left with it on. See [`Harness::set_do_not_disturb`].
    pub fn do_not_disturb(mut self) -> Self {
        self.notification_state = NotificationState::DoNotDisturb;
        self
    }

    /// Leaves out Omarchy's notification state, as on a Desktop without
    /// Omarchy. By default it is there as Omarchy 4.5's shell prepares it at
    /// startup, with Do Not Disturb never yet turned on.
    pub fn no_omarchy_notification_state(mut self) -> Self {
        self.notification_state = NotificationState::Absent;
        self
    }

    /// Keeps screenshots in `directory`. By default they go to
    /// `target/screenshots/scenarios/<test name>/`.
    pub fn screenshot_directory(mut self, directory: PathBuf) -> Self {
        self.screenshot_directory = Some(directory);
        self
    }

    /// Sets up the scenario's home and config folders under `directory`:
    /// the Omarchy theme, the notification daemon's state and the
    /// `WirePlumber` drop-ins.
    fn set_up_home(&mut self, directory: &Path) {
        if let Some(theme) = self.omarchy_theme {
            stand_ins::switch_omarchy_theme(&directory.join("home"), theme)
                .expect("make the Omarchy theme current");
        }
        self.notification_state.set_up(&directory.join("home"));
        write_wireplumber(&directory.join("config"), self.wireplumber.take());
    }

    /// Starts private buses, the compositor if asked for, the Simulated Phone
    /// and the app, and returns once the app's control interface is on the
    /// bus.
    ///
    /// # Panics
    ///
    /// If any of them fails to start; the panic says which and where its log is.
    #[must_use]
    pub fn start(mut self) -> Harness {
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
            [("system", false), ("session", self.installed)].map(|(bus, installed)| {
                let address = start_bus(&directory, bus, installed, &group, &mut processes);
                timeline
                    .monitor(&address, bus)
                    .unwrap_or_else(|error| panic!("monitor the private {bus} bus: {error}"));
                address
            });
        let mut environment = environment(&directory, &system_bus, &session_bus);
        desktop_settings(&directory.join("config"), self.motion);
        if let Some(time_zone) = self.time_zone.take() {
            environment.push(("TZ", time_zone));
        }
        let hyprland = self
            .hyprland
            .map(|config| serve_hyprland(config, &directory, &mut environment));
        self.set_up_home(&directory);
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

        let phone = simulated_phone_identity(self.phone_name);
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
        stand_ins.set_do_not_disturb(self.notification_state == NotificationState::DoNotDisturb);
        // Subscribe before the app starts, so no Error signal is missed.
        let app_errors = collect_app_errors(&session);
        let missed_calls = collect_missed_calls(&session);
        let published = collect_published(&session);

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
            service_manager: None,
            screen,
            bar: None,
            hyprland,
            test_clients: Mutex::default(),
            screenshot_directory,
            app_errors,
            missed_calls,
            published,
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

        harness.start_simulated_phone(self.scenario.as_deref(), &self.nearby);
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

/// Where cargo put the workspace's binaries: `target/<profile>`, which holds
/// the automation binary, and whose `deps` holds test executables.
pub(super) fn bin_directory() -> PathBuf {
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
pub(super) fn without_journal(program: &Path) -> Command {
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

/// Starts a private `dbus-daemon` and returns its address. Where the app
/// is `installed`, the bus runs as the user's session bus does: it and
/// every service it starts run where the journal can't be reached (see
/// [`without_journal`]), and it leaves starting a service to the systemd
/// user manager.
fn start_bus(
    directory: &Path,
    name: &'static str,
    installed: bool,
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
    let mut command = if installed {
        without_journal(Path::new("dbus-daemon"))
    } else {
        Command::new("dbus-daemon")
    };
    let command = group.adopt(&mut command);
    command
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .arg("--nofork")
        .arg("--print-address")
        .arg(format!("--config-file={}", config.display()));
    if installed {
        // As the user's session bus starts a service: through the systemd
        // user manager, which the scenario's stand-in serves, not by
        // running the service file's `Exec=` itself.
        command.arg("--systemd-activation");
    }
    let mut child = command
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
pub(super) fn environment(
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

/// The Desktop's settings: the text cursor's blinking off, so a still
/// surface leaves the screen still (GTK fades the cursor in and out over
/// many frames, which would keep [`Harness`]'s actions waiting for the
/// screen to settle for up to 10 s after every key), and animations off
/// too when the Desktop asks for [`Motion::Reduced`]. GTK reads them from
/// `GSettings`, here kept in a key file in `config` (`GSETTINGS_BACKEND`
/// is GLib's own variable, read by every GLib program), and from
/// `gtk-4.0/settings.ini` where the `GSettings` schema is not installed.
fn desktop_settings(config: &Path, motion: Motion) {
    let write = |path: PathBuf, text: &str| {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create a settings directory");
        }
        fs::write(path, text).expect("write the Desktop's settings");
    };
    let animations = match motion {
        Motion::Full => "true",
        Motion::Reduced => "false",
    };
    write(
        config.join("glib-2.0/settings/keyfile"),
        &format!(
            "[org/gnome/desktop/interface]\ncursor-blink=false\nenable-animations={animations}\n"
        ),
    );
    write(
        config.join("gtk-4.0/settings.ini"),
        &format!("[Settings]\ngtk-cursor-blink=false\ngtk-enable-animations={animations}\n"),
    );
}

/// The Simulated Phone: called `name`, or "Simulated Pixel", at a
/// `random_address`.
fn simulated_phone_identity(name: Option<String>) -> PhoneIdentity {
    PhoneIdentity {
        name: name.unwrap_or_else(|| "Simulated Pixel".to_owned()),
        address: random_address(),
    }
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
