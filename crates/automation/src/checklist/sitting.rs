//! One sitting with the user: what the runner says, what it waits for on the
//! control interface, what it asks, and what each step records.
//!
//! Everything the user types arrives as a line. While the runner waits on
//! the app, `skip` skips the step and `quit` stops the run; under the
//! Simulated Phone, the demo actions act as the Phone, the helper and the
//! user (see [`Demo`]).

use std::fs;
use std::io::{self, BufRead as _, Write as _};
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use serde_json::{Map, Value};
use test_harness::{Bus, Harness, Key, TestClient};
use zbus::Connection;
use zbus::Variant;

use super::app_log::{AppLog, LogLine};
use super::fingerprint::{Fingerprint, NoCountry};
use super::watch::{AppState, Timeline};

/// What reaches the sitting: a change on the control interface, or a line
/// the user typed.
pub enum Event {
    Changed,
    Line(String),
    /// Standard input closed.
    Closed,
}

/// Why a step stopped before its end.
#[derive(Debug)]
pub enum Stop {
    /// The user typed `skip`.
    Skipped,
    /// The user typed `quit`, pressed Ctrl-C, or closed standard input.
    Quit,
    /// What the step waited for did not happen in time.
    TimedOut(String),
}

/// A state the app published, and when.
pub struct Seen {
    pub at: Instant,
    pub state: AppState,
}

/// What one step has found so far.
#[derive(Default)]
pub struct Findings {
    pub evidence: Vec<String>,
    pub measurements: Map<String, Value>,
    pub failed: bool,
}

/// How often the runner looks for Ctrl-C while nothing else happens.
const TICK: Duration = Duration::from_millis(200);

pub struct Sitting {
    events: Receiver<Event>,
    timeline: Timeline,
    /// The earliest timeline entry a wait may still match.
    cursor: usize,
    interrupted: Arc<AtomicBool>,
    bus: Connection,
    demo: Option<Demo>,
    pub log: AppLog,
    pub findings: Findings,
}

impl Sitting {
    pub fn new(
        events: Receiver<Event>,
        timeline: Timeline,
        interrupted: Arc<AtomicBool>,
        bus: Connection,
        demo: Option<Demo>,
        log: AppLog,
    ) -> Self {
        Self {
            events,
            timeline,
            cursor: 0,
            interrupted,
            bus,
            demo,
            log,
            findings: Findings::default(),
        }
    }

    /// Tells the user what to do next. Under the Simulated Phone, also
    /// prints the demo actions that do it, one per line.
    pub fn say(&self, text: &str, demo_actions: &[&str]) {
        println!("  {text}");
        if self.demo.is_some() {
            for action in demo_actions {
                println!("  demo> {action}");
            }
        }
        flush();
    }

    /// Records what the runner saw.
    pub fn observed(&mut self, text: String) {
        println!("  Observed: {text}");
        flush();
        self.findings.evidence.push(text);
    }

    /// Records a check: `passed` decides whether the step can pass.
    pub fn check(&mut self, passed: bool, text: String) {
        if passed {
            self.observed(text);
        } else {
            println!("  Failed: {text}");
            flush();
            self.findings.evidence.push(format!("Failed: {text}"));
            self.findings.failed = true;
        }
    }

    /// Records a measurement for the result file.
    pub fn measure(&mut self, name: &str, value: impl Into<Value>) {
        self.findings
            .measurements
            .insert(name.to_owned(), value.into());
    }

    /// The app's state now.
    pub fn now(&self) -> AppState {
        self.timeline
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .last()
            .map(|(_, state)| state.clone())
            .unwrap_or_default()
    }

    /// Waits until the app publishes a state that satisfies `condition`, and
    /// returns the first such state with the moment it was published. Each
    /// wait starts from the state the last one found, so waits in a row see
    /// changes in order.
    pub fn wait_for(
        &mut self,
        description: &str,
        deadline: Duration,
        condition: impl Fn(&AppState) -> bool,
    ) -> Result<Seen, Stop> {
        let end = Instant::now() + deadline;
        loop {
            if let Some(seen) = self.first_since_cursor(&condition) {
                return Ok(seen);
            }
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(Stop::TimedOut(format!(
                    "{description} did not happen within {}",
                    seconds(deadline)
                )));
            }
            match self.next_event(left.min(TICK))? {
                Some(Event::Line(line)) => self.handle_line(&line)?,
                Some(Event::Closed) => return Err(Stop::Quit),
                Some(Event::Changed) | None => {}
            }
        }
    }

    /// Where the next wait starts looking, for [`Sitting::seen_since`] to
    /// look back over everything published from here on.
    pub fn mark(&self) -> usize {
        self.cursor
    }

    /// The first state published since `mark` that satisfies `condition`,
    /// for something that must not have happened at any point, such as the
    /// Desktop ringing. Waits still start where they would have.
    pub fn seen_since(&self, mark: usize, condition: impl Fn(&AppState) -> bool) -> Option<Seen> {
        self.timeline
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .skip(mark)
            .find(|(_, state)| condition(state))
            .map(|(at, state)| Seen {
                at: *at,
                state: state.clone(),
            })
    }

    /// Whether the app publishes a state that satisfies `condition` within
    /// `window`: for something that must not happen, such as the audio
    /// leaving the Phone.
    pub fn happens_within(
        &mut self,
        description: &str,
        window: Duration,
        condition: impl Fn(&AppState) -> bool,
    ) -> Result<Option<Seen>, Stop> {
        match self.wait_for(description, window, condition) {
            Ok(seen) => Ok(Some(seen)),
            Err(Stop::TimedOut(_)) => Ok(None),
            Err(stop) => Err(stop),
        }
    }

    /// Lets `window` pass while still reading what the user types, without
    /// moving where the next wait starts: for polling something outside the
    /// timeline, such as the app's log.
    pub fn pause(&mut self, window: Duration) -> Result<(), Stop> {
        let end = Instant::now() + window;
        loop {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(());
            }
            match self.next_event(left.min(TICK))? {
                Some(Event::Line(line)) => self.handle_line(&line)?,
                Some(Event::Closed) => return Err(Stop::Quit),
                Some(Event::Changed) | None => {}
            }
        }
    }

    /// The last state published since `mark` that satisfies `condition`:
    /// how something stood when it ended, such as which surface had the
    /// keyboard as a Call ended.
    pub fn last_seen_since(
        &self,
        mark: usize,
        condition: impl Fn(&AppState) -> bool,
    ) -> Option<Seen> {
        self.timeline
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .skip(mark)
            .rev()
            .find(|(_, state)| condition(state))
            .map(|(at, state)| Seen {
                at: *at,
                state: state.clone(),
            })
    }

    /// Whether, since `mark`, the app published a state satisfying `first`
    /// and then, after it, one satisfying `then`: a change, not a state
    /// that held all along.
    pub fn seen_in_order(
        &self,
        mark: usize,
        first: impl Fn(&AppState) -> bool,
        then: impl Fn(&AppState) -> bool,
    ) -> bool {
        let timeline = self
            .timeline
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut states = timeline.iter().skip(mark).map(|(_, state)| state);
        states.any(first) && states.any(then)
    }

    /// Asks the app to hang up `call`, as the Call Window's Hang up does.
    pub fn hang_up(&self, call: u32) -> zbus::Result<()> {
        zbus::block_on(self.bus.call_method(
            Some(super::watch::BUS_NAME),
            super::watch::PATH,
            Some(super::watch::INTERFACE),
            "HangUp",
            &(call,),
        ))
        .map(drop)
    }

    /// The Test Number, from `~/.config/desktop-calls/test-number`, read
    /// in the Desktop's locale if it has no country code. The number
    /// itself is never kept or printed.
    pub fn test_number(&self) -> TestNumber {
        let number = if self.demo.is_some() {
            DEMO_TEST_NUMBER_AS_FILED.to_owned()
        } else {
            let Some(config) = std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .filter(|config| config.is_absolute())
                .or_else(|| {
                    std::env::var_os("HOME")
                        .map(PathBuf::from)
                        .filter(|home| home.is_absolute())
                        .map(|home| home.join(".config"))
                })
            else {
                return TestNumber::NotSetUp;
            };
            match fs::read_to_string(config.join("desktop-calls/test-number")) {
                Ok(number) => number,
                Err(_) => return TestNumber::NotSetUp,
            }
        };
        match Fingerprint::of_test_number(&number, locale_region().as_deref()) {
            Ok(Some((number, assumed))) => TestNumber::Ready { number, assumed },
            Ok(None) => TestNumber::NotSetUp,
            Err(NoCountry) => TestNumber::NoCountry,
        }
    }

    /// Asks a yes-or-no question only the user can answer, and returns the
    /// answer.
    pub fn ask(&mut self, question: &str) -> Result<bool, Stop> {
        println!("  Question: {question} [y/n]");
        flush();
        loop {
            match self.next_event(TICK)? {
                Some(Event::Line(line)) => match line.trim().to_ascii_lowercase().as_str() {
                    "y" | "yes" => return Ok(true),
                    "n" | "no" => return Ok(false),
                    "skip" => return Err(Stop::Skipped),
                    "quit" => return Err(Stop::Quit),
                    _ => {
                        println!("  Type y or n (or skip, or quit).");
                        flush();
                    }
                },
                Some(Event::Closed) => return Err(Stop::Quit),
                Some(Event::Changed) | None => {}
            }
        }
    }

    /// Waits for Enter. `skip` skips what the prompt offers to start.
    pub fn ready(&mut self, prompt: &str) -> Result<(), Stop> {
        println!("{prompt}");
        flush();
        loop {
            match self.next_event(TICK)? {
                Some(Event::Line(line)) => match line.trim() {
                    "" => return Ok(()),
                    "skip" => return Err(Stop::Skipped),
                    "quit" => return Err(Stop::Quit),
                    other => self.demo_action(other),
                },
                Some(Event::Closed) => return Err(Stop::Quit),
                Some(Event::Changed) | None => {}
            }
        }
    }

    /// Shows a desktop notification, as any app could.
    pub fn notify(&self, summary: &str, body: &str) -> zbus::Result<()> {
        let hints: std::collections::HashMap<&str, Variant<'_>> = std::collections::HashMap::new();
        zbus::block_on(self.bus.call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &(
                "Real-Phone Checklist",
                0_u32,
                "",
                summary,
                body,
                Vec::<&str>::new(),
                hints,
                -1_i32,
            ),
        ))
        .map(drop)
    }

    /// Runs the PBAP probe on the Phone, as `just pbap-probe` does, its
    /// progress shown as it goes,
    /// and returns the report it wrote, or why there is none. Under the
    /// Simulated Phone, it runs against the demo's buses and home. Ctrl-C
    /// during the probe stops the run.
    pub fn pbap_probe(&mut self) -> Result<Result<Value, String>, Stop> {
        let prepared = match &self.demo {
            Some(demo) => demo.pbap_probe_command(),
            None => Ok((Command::new(SYSTEM_PYTHON), user_cache())),
        };
        let (mut command, cache) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => return Ok(Err(format!("Couldn't prepare the probe: {error}"))),
        };
        let out = cache.join("desktop-calls").join(format!(
            "pbap-probe-checklist-{}.json",
            super::results::unix_micros() / 1_000_000
        ));
        flush();
        // Standard input stays with the runner, which reads skip and quit.
        let status = command
            .args([PBAP_PROBE, "--out"])
            .arg(&out)
            .stdin(Stdio::null())
            .status();
        if self.interrupted.load(Ordering::Relaxed) {
            return Err(Stop::Quit);
        }
        let report = match status {
            Err(error) => Err(format!(
                "Couldn't run {SYSTEM_PYTHON} {PBAP_PROBE}: {error}."
            )),
            Ok(status) if !status.success() => Err(format!(
                "The PBAP probe failed ({status}); its output is above."
            )),
            Ok(_) => fs::read_to_string(&out)
                .map_err(|error| format!("Couldn't read {}: {error}.", out.display()))
                .and_then(|text| {
                    serde_json::from_str(&text).map_err(|error| {
                        format!("{} isn't the probe's report: {error}.", out.display())
                    })
                }),
        };
        Ok(report)
    }

    /// The app's log lines from `from_us` until now.
    pub fn log_since(&self, from_us: u64) -> Vec<LogLine> {
        self.log.lines(from_us, super::results::unix_micros())
    }

    /// Starts the next step's findings, and returns the last step's.
    pub fn take_findings(&mut self) -> Findings {
        std::mem::take(&mut self.findings)
    }

    fn first_since_cursor(&mut self, condition: impl Fn(&AppState) -> bool) -> Option<Seen> {
        let timeline = self
            .timeline
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let found = timeline
            .iter()
            .enumerate()
            .skip(self.cursor)
            .find(|(_, (_, state))| condition(state));
        if let Some((index, (at, state))) = found {
            self.cursor = index;
            Some(Seen {
                at: *at,
                state: state.clone(),
            })
        } else {
            // The latest state stays in play for the next look.
            self.cursor = timeline.len().saturating_sub(1).max(self.cursor);
            None
        }
    }

    /// The next event within `timeout`, or `None`. Ctrl-C stops the run.
    fn next_event(&self, timeout: Duration) -> Result<Option<Event>, Stop> {
        if self.interrupted.load(Ordering::Relaxed) {
            return Err(Stop::Quit);
        }
        match self.events.recv_timeout(timeout) {
            Ok(event) => Ok(Some(event)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(Stop::Quit),
        }
    }

    fn handle_line(&mut self, line: &str) -> Result<(), Stop> {
        match line.trim() {
            "skip" => Err(Stop::Skipped),
            "quit" => Err(Stop::Quit),
            "" => Ok(()),
            other => {
                self.demo_action(other);
                Ok(())
            }
        }
    }

    fn demo_action(&mut self, action: &str) {
        let state = self.now();
        if let Some(demo) = &mut self.demo {
            demo.act(action, &state);
        } else {
            println!(
                "  The runner is waiting on the Desktop. Type skip to skip this step, or quit to stop."
            );
            flush();
        }
    }
}

/// The Simulated Phone and the app it runs against, for `--demo`. Typed
/// actions act out what the Phone, the helper and the user would do:
///
/// - the helper: `call` rings the Phone, `call-withheld` rings it with the
///   number withheld; `hang-up-on-phone` ends the Call from the other end,
///   and `ring-out` gives up while it rings, leaving a Missed Call in the
///   Phone's Call History;
/// - the person holding the Phone: `answer-on-phone`, `decline-on-phone`;
/// - the Phone itself: `phone-dials` places the Call the app asked for;
/// - the user at the Desktop, through the control interface, the
///   notification server and the keyboard: `answer`, `decline`, `hang-up`,
///   `mute`, `unmute`, `keybinding`, `escape`, `a`, `d`,
///   `click-notification`, `enter`;
/// - the user's Desktop: `do-not-disturb-on`, `do-not-disturb-off`, as
///   Omarchy's bell records it; `suspend`, `resume`, as logind announces
///   them, with the Phone leaving on suspend and returning after resume as
///   the Pixel does; `resume-slowly`, with the Phone answering the
///   Desktop's page only a few seconds after resume; `suspend-phone-stays`
///   and `resume-then-phone-leaves`, with the Phone leaving only after
///   resume; `restart-wireplumber`, as Telephony leaves and returns;
/// - the user's keyboard and Call Window, beyond the above:
///   `type-test-number`, `type-wrong-number` (the last digit off, as a
///   slip would be), `type-wrong-first-digit` (the first digit after the
///   country code off), `type-wrong-country` (another country code),
///   `ctrl-h`, `move-audio-to-phone`, `move-audio-here`,
///   `move-audio-to-phone-and-back` (each move waited for);
/// - the Phone and its surroundings: `callee-answers` (the Test Number
///   picks up), `earbuds-on`, `out-of-range`, `return-to-range`;
/// - a first run: `forget-pairing` (the Phone forgets the Desktop and the
///   Desktop removes the Phone, which stays nearby), `restart-app` (as
///   `just install` restarts it), `open-setup-window` (`desktop-calls
///   connect`), `contacts-access-off` (the Phone asks again before it
///   shares Contacts), `allow-contacts` (Allow on the Phone's prompt);
/// - the Phone's quirks: `quirk NAME`, as the Simulated Phone names them.
pub struct Demo {
    /// The user's own window, which has keyboard focus until the keybinding
    /// takes it, and gets it back on Escape, as on a real Desktop.
    _window: TestClient,
    pub harness: Harness,
    /// The Simulated Phone's object path for the last Call the helper made.
    call_path: Option<String>,
    /// The number the helper last called from, or `None` if withheld.
    calling_from: Option<&'static str>,
    /// The Missed Calls the demo's Phone has added to its Call History,
    /// newest first, as PBAP vCards.
    missed: Vec<String>,
    /// How many of the app's Dial requests the demo's Phone has placed.
    dials_placed: usize,
}

/// The number the demo's helper calls from: an Ofcom drama number.
const DEMO_NUMBER: &str = "+447700900461";

/// The demo's Test Number, for the PBAP probe to redact: another Ofcom
/// drama number.
const DEMO_TEST_NUMBER: &str = "+447700900999";

/// The demo's Test Number as the user's file has it: without its country
/// code, which the runner takes from the locale.
const DEMO_TEST_NUMBER_AS_FILED: &str = "07700 900999";

/// The demo's Test Number with its last digit slipped.
const DEMO_WRONG_NUMBER: &str = "+447700900998";

/// The demo's Test Number with its first digit after the country code
/// slipped.
const DEMO_WRONG_FIRST_DIGIT: &str = "+448700900999";

/// The demo's Test Number's national number under another country code.
const DEMO_WRONG_COUNTRY: &str = "+337700900999";

/// How long after `resume-then-phone-leaves` the demo's Phone is back in
/// range.
const LATE_LEAVING_RETURN: Duration = Duration::from_secs(1);

/// How long the demo waits for the audio to move.
const AUDIO_DEADLINE: Duration = Duration::from_secs(10);

/// How long the demo's Phone takes to answer the Desktop's page after
/// `resume-slowly`: longer than the runner reads the log for a line the
/// app logs at once.
const SLOW_RETURN: Duration = Duration::from_secs(4);

/// How long Telephony is off the bus when the demo restarts `WirePlumber`.
const WIREPLUMBER_GAP: Duration = Duration::from_millis(500);

/// How many Calls the demo's Call History holds: more than the 50 the
/// Pixel gives per pull, so there is a second page to read.
const DEMO_CALL_HISTORY: usize = 120;

/// What `just pbap-probe` runs. The real-Phone tools need the system
/// Python, the only one with dbus-python (#54); `just` itself may be a
/// version manager's shim, which fails in the demo's temporary home.
const SYSTEM_PYTHON: &str = "/usr/bin/python3";
const PBAP_PROBE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tools/real-phone/pbap_probe.py"
);

/// `$XDG_CACHE_HOME`, or `~/.cache`.
fn user_cache() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .filter(|cache| !cache.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .unwrap_or_default()
}

/// The demo's Omarchy bindings: one of the user's own.
const DEMO_BINDINGS: &str = "-- Keep only your personal keybinding overrides here.\n\
                             o.bind(\"SUPER + SHIFT + R\", \"SSH\", \"alacritty -e ssh your-server\")\n";

/// How long the keybinding's focus, or the user's window, may take to
/// arrive in the demo.
const FOCUS_DEADLINE: Duration = Duration::from_secs(5);

impl Demo {
    pub fn new(harness: Harness) -> Self {
        let window = harness.start_test_client();
        window.wait_for("your window has keyboard focus", FOCUS_DEADLINE, |line| {
            line == "keyboard-enter"
        });
        // As a real Phone, hands-free comes up a moment after BlueZ marks
        // the Phone connected, so a reconnect takes time to measure.
        harness.simulated_phone().enable_quirk("slow-service-level");
        // Omarchy's personal bindings, for step 1.2 to add the keybinding to.
        let bindings = harness.directory().join("config/hypr/bindings.lua");
        let written = bindings
            .parent()
            .map(fs::create_dir_all)
            .and_then(Result::ok)
            .and_then(|()| fs::write(&bindings, DEMO_BINDINGS).ok());
        if written.is_none() {
            println!("  demo: couldn't write {}.", bindings.display());
        }
        Self {
            _window: window,
            harness,
            call_path: None,
            calling_from: None,
            missed: Vec::new(),
            dials_placed: 0,
        }
    }

    /// The Phone places the Call the app asked it to dial, as a real Phone
    /// does on its own: the Simulated Phone waits to be told.
    fn phone_dials(&mut self) {
        let placed = self.dials_placed;
        let phone = self.harness.simulated_phone();
        phone.wait_for_recorded(
            "the app asking the Phone to dial",
            FOCUS_DEADLINE,
            |calls| calls.iter().filter(|call| call.member == "Dial").count() > placed,
        );
        self.dials_placed += 1;
        let _ = phone.alert();
    }

    /// The helper's Call rings out: like a real Phone, the demo's Phone
    /// adds a Missed Call to its Call History as the Call ends.
    fn ring_out(&mut self) {
        let Some(path) = &self.call_path else {
            println!("  demo: the helper hasn't called yet; type call first.");
            return;
        };
        self.missed.insert(
            0,
            test_harness::vcards::call(
                test_harness::vcards::CallKind::Missed,
                self.calling_from,
                &super::results::irmc_timestamp(SystemTime::now()),
            ),
        );
        let phone = self.harness.simulated_phone();
        phone.set_pbap_folder("cch", &self.missed.concat());
        phone.caller_hangs_up(path);
    }

    /// The Phone forgets the Desktop, which drops the connection, and the
    /// Desktop removes the Phone in its Bluetooth settings.
    fn forget_pairing(&self) {
        self.harness.simulated_phone().forget_pairing();
        let address = self.harness.phone().address.replace(':', "_");
        let device = format!("/org/bluez/hci0/dev_{address}");
        match zbus::ObjectPath::try_from(device.as_str()) {
            Ok(device) => drop(self.harness.call_phone_method(
                Bus::System,
                "org.bluez",
                "/org/bluez/hci0",
                "org.bluez.Adapter1",
                "RemoveDevice",
                &(device,),
            )),
            Err(error) => println!("  demo: {device} isn't a device path: {error}"),
        }
    }

    /// The user clicks the newest notification from Desktop Calls.
    fn click_notification(&self) {
        let notifications = self.harness.notifications();
        match notifications
            .received()
            .into_iter()
            .rfind(|notification| notification.app_name == "Desktop Calls")
        {
            Some(notification) => notifications.invoke_default_action(notification.id),
            None => println!("  demo: Desktop Calls hasn't shown a notification."),
        }
    }

    /// The PBAP probe for the demo, and the cache directory it writes
    /// to: the scenario's buses and home, a made-up Test Number for the
    /// probe to redact, and a Call History longer than one pull gives.
    fn pbap_probe_command(&self) -> io::Result<(Command, PathBuf)> {
        let directory = self.harness.directory();
        let config = directory.join("config/desktop-calls");
        fs::create_dir_all(&config)?;
        fs::write(config.join("test-number"), DEMO_TEST_NUMBER)?;
        self.harness.simulated_phone().set_pbap_folder(
            "cch",
            &test_harness::vcards::call_history(DEMO_CALL_HISTORY),
        );
        Ok((
            self.harness.external_command(SYSTEM_PYTHON),
            directory.join("cache"),
        ))
    }

    fn act(&mut self, action: &str, state: &AppState) {
        let call = state.calls.first().map(|call| call.id);
        // The harness is test code: an action it can't carry out panics with
        // the reason. A demo goes on after a mistyped or ill-timed action,
        // so the panic is caught and reported.
        let acted = panic::catch_unwind(AssertUnwindSafe(|| {
            self.act_or_panic(action, call);
        }));
        if acted.is_err() {
            println!("  demo: {action} didn't work now; see the message above.");
            flush();
        }
    }

    fn act_or_panic(&mut self, action: &str, call: Option<u32>) {
        match action {
            "call" => {
                self.calling_from = Some(DEMO_NUMBER);
                self.call_path = Some(self.harness.simulated_phone().ring(DEMO_NUMBER));
            }
            // As PipeWire reads the caller ID a Phone sends for a withheld
            // number.
            "call-withheld" => {
                self.calling_from = None;
                self.call_path = Some(
                    self.harness
                        .simulated_phone()
                        .ring_with_caller_id("\"\",129"),
                );
            }
            "ring-out" => self.ring_out(),
            "forget-pairing" => self.forget_pairing(),
            "restart-app" => self.harness.restart_app(),
            "open-setup-window" => drop(self.harness.cli(&["connect"])),
            "contacts-access-off" => self.harness.simulated_phone().set_contacts_prompt("ask"),
            // Android remembers the answer for the Desktop.
            "allow-contacts" => {
                let phone = self.harness.simulated_phone();
                phone.answer_contacts_prompt("grant");
                phone.set_contacts_prompt("grant");
            }
            "click-notification" => self.click_notification(),
            "enter" => self.harness.keyboard().press(Key::Return),
            "phone-dials" => self.phone_dials(),
            "do-not-disturb-on" => self.harness.set_do_not_disturb(true),
            "do-not-disturb-off" => self.harness.set_do_not_disturb(false),
            "type-test-number" => self.harness.keyboard().type_text(DEMO_TEST_NUMBER),
            "type-wrong-number" => self.harness.keyboard().type_text(DEMO_WRONG_NUMBER),
            "type-wrong-first-digit" => self.harness.keyboard().type_text(DEMO_WRONG_FIRST_DIGIT),
            "type-wrong-country" => self.harness.keyboard().type_text(DEMO_WRONG_COUNTRY),
            "ctrl-h" => self.harness.keyboard().press_with_control('h'),
            "callee-answers" => self.harness.simulated_phone().callee_answers(),
            "earbuds-on" => self.harness.simulated_phone().set_earbuds(true),
            "out-of-range" => self.harness.simulated_phone().disconnect(),
            "return-to-range" => self.harness.simulated_phone().return_to_range(),
            // The Desktop's Bluetooth sleeps with it, so the Phone leaves,
            // and comes back only once the Desktop is awake.
            "suspend" => {
                self.harness.simulated_phone().prepare_for_sleep(true);
                self.harness.simulated_phone().disconnect();
            }
            "resume" => {
                self.harness.simulated_phone().prepare_for_sleep(false);
                self.harness.simulated_phone().return_to_range();
            }
            // The Phone stays through the suspend, and leaves only after
            // resume, as real BlueZ may report it; it is back a moment later.
            "suspend-phone-stays" => self.harness.simulated_phone().prepare_for_sleep(true),
            "resume-then-phone-leaves" => {
                let phone = self.harness.simulated_phone();
                phone.prepare_for_sleep(false);
                phone.disconnect();
                phone.return_to_range_after(LATE_LEAVING_RETURN);
            }
            "resume-slowly" => {
                self.harness.simulated_phone().prepare_for_sleep(false);
                self.harness
                    .simulated_phone()
                    .return_to_range_after(SLOW_RETURN);
            }
            "restart-wireplumber" => self
                .harness
                .simulated_phone()
                .restart_telephony(WIREPLUMBER_GAP),
            "move-audio-to-phone-and-back" => self.move_audio_to_phone_and_back(),
            "move-audio-to-phone" | "move-audio-here" => {
                let to = if action == "move-audio-here" {
                    "desktop"
                } else {
                    "phone"
                };
                if let Err(error) = self.harness.move_audio(to) {
                    println!("  demo: {}", error.message);
                }
            }
            other => self.act_on_call(other, call),
        }
        flush();
    }

    /// Moves the audio to the Phone and, once it is there, straight back,
    /// returning once it is back: an audio change the runner sees only in
    /// the app's published states, while it is still reading the log.
    fn move_audio_to_phone_and_back(&self) {
        // The Call Window offers each way back once the Handoff before it
        // has settled (ADR-0005).
        for (location, back) in [
            ("phone", "Move audio here"),
            ("desktop", "Move audio to phone"),
        ] {
            if let Err(error) = self.harness.move_audio(location) {
                println!("  demo: {}", error.message);
                return;
            }
            self.harness
                .wait_for_app("the Handoff settling", AUDIO_DEADLINE, |state| {
                    state.audio_location == location
                        && state.connection_state == "connected"
                        && state
                            .surfaces
                            .get("call-window")
                            .and_then(|window| window.get("handoff"))
                            .is_some_and(|label| label == back)
                });
        }
    }

    fn act_on_call(&self, action: &str, call: Option<u32>) {
        let harness = &self.harness;
        let phone = harness.simulated_phone();
        let on_phone = |act: &dyn Fn(&str)| match &self.call_path {
            Some(path) => act(path),
            None => println!("  demo: the helper hasn't called yet; type call first."),
        };
        let on_desktop = |act: &dyn Fn(u32)| match call {
            Some(call) => act(call),
            None => println!("  demo: there is no Call to act on."),
        };
        match action {
            "answer-on-phone" => on_phone(&|path| phone.answer_on_phone(path)),
            "decline-on-phone" => on_phone(&|path| phone.decline_on_phone(path)),
            "hang-up-on-phone" => on_phone(&|path| phone.caller_hangs_up(path)),
            "answer" => on_desktop(&|call| harness.answer(call)),
            "decline" => on_desktop(&|call| harness.decline(call)),
            "hang-up" => on_desktop(&|call| harness.hang_up(call)),
            "mute" | "unmute" => {
                if let Err(error) = harness.set_muted(action == "mute") {
                    println!("  demo: {}", error.message);
                }
            }
            "keybinding" => {
                let _ = harness.cli(&[]);
                harness.wait_for_app("a surface takes keyboard focus", FOCUS_DEADLINE, |state| {
                    state
                        .surfaces
                        .values()
                        .any(|content| content.get("focused").map(String::as_str) == Some("true"))
                });
            }
            "escape" => harness.keyboard().press(Key::Escape),
            "indicator-click" => harness.tray().activate(),
            "indicator-mute" => harness.tray().click("Mute"),
            "indicator-hang-up" => harness.tray().click("Hang up"),
            "a" | "d" => harness.keyboard().type_text(action),
            other => match other.strip_prefix("quirk ") {
                Some(quirk) => phone.enable_quirk(quirk.trim()),
                None => println!(
                    "  demo: unknown action {other:?}. The actions: call, call-withheld, \
                     answer-on-phone, decline-on-phone, hang-up-on-phone, ring-out, answer, \
                     decline, hang-up, mute, unmute, keybinding, escape, a, d, \
                     click-notification, enter, phone-dials, forget-pairing, restart-app, \
                     open-setup-window, contacts-access-off, allow-contacts, indicator-click, indicator-mute, \
                     indicator-hang-up, do-not-disturb-on, do-not-disturb-off, type-test-number, type-wrong-number, \
                     type-wrong-first-digit, type-wrong-country, ctrl-h, callee-answers, \
                     earbuds-on, out-of-range, return-to-range, suspend, resume, resume-slowly, \
                     suspend-phone-stays, resume-then-phone-leaves, \
                     restart-wireplumber, move-audio-to-phone, move-audio-here, \
                     move-audio-to-phone-and-back, \
                     quirk NAME; or skip, or quit."
                ),
            },
        }
    }
}

/// Reads standard input line by line into `events`.
pub fn read_input(events: Sender<Event>) {
    thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if events.send(Event::Line(line)).is_err() {
                return;
            }
        }
        let _ = events.send(Event::Closed);
    });
}

/// `duration` for reading: `4 ms`, `210 ms`, `1.50 s`, `3 s`.
pub fn seconds(duration: Duration) -> String {
    if duration < Duration::from_secs(1) {
        format!("{} ms", duration.as_millis())
    } else if duration.subsec_millis() == 0 {
        format!("{} s", duration.as_secs())
    } else {
        format!("{:.2} s", duration.as_secs_f64())
    }
}

pub fn flush() {
    let _ = io::stdout().flush();
}

/// The Test Number, as step 7.1 needs it.
pub enum TestNumber {
    /// `~/.config/desktop-calls/test-number` is missing or empty.
    NotSetUp,
    /// The number has no country code, and the locale names no country.
    NoCountry,
    Ready {
        number: Fingerprint,
        /// The country code taken from the locale, if the number had none.
        assumed: Option<u16>,
    },
}

/// The region of the Desktop's locale for telephone numbers, such as `US`
/// for `en_US.UTF-8`: from `LC_TELEPHONE`, then `LC_ALL`, then `LANG`.
fn locale_region() -> Option<String> {
    ["LC_TELEPHONE", "LC_ALL", "LANG"]
        .into_iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.is_empty())
        .and_then(|locale| {
            let (_, rest) = locale.split_once('_')?;
            let region: String = rest.chars().take_while(char::is_ascii_alphabetic).collect();
            (region.len() == 2).then_some(region)
        })
}
