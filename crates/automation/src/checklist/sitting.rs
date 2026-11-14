//! One sitting with the user: what the runner says, what it waits for on the
//! control interface, what it asks, and what each step records.
//!
//! Everything the user types arrives as a line. While the runner waits on
//! the app, `skip` skips the step and `quit` stops the run; under the
//! Simulated Phone, the demo actions act as the Phone, the helper and the
//! user (see [`Demo`]).

use std::fs;
use std::io::{self, BufRead as _, Write as _};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Map, Value};
use zbus::Connection;
use zbus::Variant;

use super::app_log::{AppLog, LogLine};
use super::fingerprint::{Fingerprint, NoCountry};
use super::watch::{AppState, Timeline};

mod demo;

pub use demo::Demo;

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

    /// Shows a desktop notification, as any app could, that stays until the
    /// user dismisses it: critical urgency, which Omarchy shows with no time
    /// limit, where a normal card goes after 8 s, before a step's questions
    /// are done.
    pub fn notify(&self, summary: &str, body: &str) -> zbus::Result<()> {
        let hints: std::collections::HashMap<&str, Variant<'_>> =
            std::collections::HashMap::from([("urgency", Variant::from(2_u8))]);
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

    /// Asks BlueZ, as any program on the Desktop could, to connect the
    /// Phone with `address` (`Device1.Connect`), waiting up to `deadline`
    /// for the answer, and returns what BlueZ said: `Ok` if it connected,
    /// else its error, such as `org.bluez.Error.Failed:
    /// br-connection-profile-unavailable`. Under the Simulated Phone, it
    /// asks the demo's BlueZ.
    pub fn connect_the_phone(&self, address: &str, deadline: Duration) -> Result<(), String> {
        let connected = match &self.demo {
            Some(demo) => demo.connect_the_phone(address),
            None => zbus::block_on(async {
                let system = zbus::connection::Builder::system()
                    .method_timeout(deadline)
                    .build()
                    .await?;
                let path = format!("/org/bluez/hci0/dev_{}", address.replace(':', "_"));
                system
                    .call_method(
                        Some("org.bluez"),
                        path.as_str(),
                        Some("org.bluez.Device1"),
                        "Connect",
                        &(),
                    )
                    .await
                    .map(drop)
            }),
        };
        connected.map_err(|error| match error {
            zbus::Error::MethodError(name, Some(text), _) => format!("{name}: {text}"),
            zbus::Error::MethodError(name, None, _) => name.to_string(),
            other => other.to_string(),
        })
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

/// The demo's Test Number as the user's file has it: without its country
/// code, which the runner takes from the locale.
const DEMO_TEST_NUMBER_AS_FILED: &str = "07700 900999";

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
