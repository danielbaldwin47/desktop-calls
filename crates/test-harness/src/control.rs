//! Driving and observing the app: its control interface, its CLI, its log, and
//! starting it the ways a user does.

use std::fs;
use std::path::Path;
use std::process::Output;
use std::sync::PoisonError;
use std::thread;
use std::time::Duration;

use futures_util::StreamExt as _;
use zbus::fdo::PropertiesProxy;

use super::app::{
    AppCallHistoryEntry, AppDialerRow, AppError, AppState, MissedCall, ShownSurfaces,
    read_app_state,
};
use super::{APP_BUS_NAME, APP_INTERFACE, APP_PATH, DESKTOP_FILE, Harness, within};

impl Harness {
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
}

impl Harness {
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
}

impl Harness {
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
}
