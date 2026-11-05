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
use std::time::{Duration, Instant};

use serde_json::{Map, Value};
use test_harness::{Harness, Key, TestClient};
use zbus::Connection;
use zbus::Variant;

use super::app_log::{AppLog, LogLine};
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
/// - the helper: `call` rings the Phone; `hang-up-on-phone` ends the Call
///   from the other end;
/// - the person holding the Phone: `answer-on-phone`, `decline-on-phone`;
/// - the user at the Desktop, through the control interface and the
///   keyboard: `answer`, `decline`, `hang-up`, `mute`, `unmute`,
///   `keybinding`, `escape`, `a`, `d`;
/// - the Phone's quirks: `quirk NAME`, as the Simulated Phone names them.
pub struct Demo {
    /// The user's own window, which has keyboard focus until the keybinding
    /// takes it, and gets it back on Escape, as on a real Desktop.
    _window: TestClient,
    pub harness: Harness,
    /// The Simulated Phone's object path for the last Call the helper made.
    call_path: Option<String>,
}

/// The number the demo's helper calls from: an Ofcom drama number.
const DEMO_NUMBER: &str = "+447700900461";

/// The demo's Test Number, for the PBAP probe to redact: another Ofcom
/// drama number.
const DEMO_TEST_NUMBER: &str = "+447700900999";

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

/// How long the keybinding's focus, or the user's window, may take to
/// arrive in the demo.
const FOCUS_DEADLINE: Duration = Duration::from_secs(5);

impl Demo {
    pub fn new(harness: Harness) -> Self {
        let window = harness.start_test_client();
        window.wait_for("your window has keyboard focus", FOCUS_DEADLINE, |line| {
            line == "keyboard-enter"
        });
        Self {
            _window: window,
            harness,
            call_path: None,
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
            "call" => self.call_path = Some(phone.ring(DEMO_NUMBER)),
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
            "a" | "d" => harness.keyboard().type_text(action),
            other => match other.strip_prefix("quirk ") {
                Some(quirk) => phone.enable_quirk(quirk.trim()),
                None => println!(
                    "  demo: unknown action {other:?}. The actions: call, answer-on-phone, \
                     decline-on-phone, hang-up-on-phone, answer, decline, hang-up, mute, unmute, \
                     keybinding, escape, a, d, quirk NAME; or skip, or quit."
                ),
            },
        }
        flush();
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
