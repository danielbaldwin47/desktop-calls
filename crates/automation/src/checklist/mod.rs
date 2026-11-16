//! `just real-phone-checklist [GROUP]`: walks the user through the
//! Real-Phone Checklist one step at a time, observing the app through its
//! control interface and its log, and asking only what a machine can't
//! observe. Results go to a dated file after every step, so an interrupted
//! run resumes where it stopped; the scorecard reads the latest file.
//!
//! With `--demo`, the same runner runs against the Simulated Phone, and
//! prints after `demo>` the action that plays each instruction, so an agent
//! can play the user.

mod app_log;
mod bus_capture;
mod fingerprint;
pub mod results;
mod sitting;
mod steps;
mod watch;

use std::collections::VecDeque;
use std::io::IsTerminal as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::time::Duration;

use test_harness::Harness;
use zbus::Connection;

use app_log::AppLog;
use results::{Mode, Outcome, ResultFile, StepRecord};
use sitting::{Demo, Sitting, Stop, flush};
use steps::{GROUPS, Group, Step};

/// Every step's id, in the Checklist's order, for the scorecard to tell a
/// whole run from part of one.
pub fn step_ids() -> Vec<&'static str> {
    steps::order()
}

/// What `automation real-phone-checklist` was asked to do.
pub struct Options {
    /// One group, or every group.
    pub group: Option<u8>,
    /// Run against the Simulated Phone.
    pub demo: bool,
    /// Under `--demo`, keep the compositor off-screen.
    pub headless: bool,
    /// Where the result files go.
    pub results: Option<PathBuf>,
    /// Start a new result file even if the last run is unfinished.
    pub fresh: bool,
}

/// How long the demo's app may take to connect to the Simulated Phone.
const DEMO_CONNECT_DEADLINE: Duration = Duration::from_secs(10);

/// `automation real-phone-checklist`.
pub fn run(options: &Options) -> ExitCode {
    let Some(groups) = groups_to_run(options) else {
        return ExitCode::FAILURE;
    };
    let Some(directory) = results_directory(options) else {
        return ExitCode::FAILURE;
    };
    let interrupted = Arc::new(AtomicBool::new(false));
    // Ctrl-C stops the run between waits, so the results are kept and, under
    // the Simulated Phone, every process is stopped.
    if let Err(error) =
        signal_hook::flag::register(signal_hook::consts::SIGINT, interrupted.clone())
    {
        eprintln!("Can't handle Ctrl-C: {error}");
        return ExitCode::FAILURE;
    }

    let world = if options.demo {
        World::demo(options.headless)
    } else {
        World::real()
    };
    let Some(world) = world else {
        return ExitCode::FAILURE;
    };

    let (sender, events) = mpsc::channel();
    let timeline = match watch::start(&world.bus, sender.clone(), sitting::locale_region()) {
        Ok(timeline) => timeline,
        Err(watch::WatchError::NotRunning) => {
            // Never `cargo run`: that would start a second app beside the
            // installed service. `just run` stops the service for its run and
            // starts it again afterwards (ADR-0006).
            eprintln!(
                "Desktop Calls isn't running, so the checklist can't observe it. Start the installed service with `systemctl --user start desktop-calls`, or, to try this checkout's app instead, run `just run` in another terminal (it stops the installed service for the run and starts it again afterwards). Then run this again."
            );
            return ExitCode::FAILURE;
        }
        Err(watch::WatchError::Unexpected(detail)) => {
            eprintln!("Couldn't read Desktop Calls' control interface: {detail}");
            return ExitCode::FAILURE;
        }
    };
    sitting::read_input(sender);
    let mode = world.mode;
    let log = world.log();
    let bus = world.bus.clone();
    let (system_bus, session_bus) = world.capture_buses();
    let mut sitting = Sitting::new(events, timeline, interrupted, bus, world.demo, log);

    let state = sitting.now();
    if state.connection_state != "connected" {
        eprintln!(
            "Desktop Calls is running, but the Phone isn't connected (it is {}). Connect the Pixel, then run this again.",
            if state.connection_state.is_empty() {
                "unknown"
            } else {
                &state.connection_state
            }
        );
        return ExitCode::FAILURE;
    }

    let command = resume_command(options);
    let mut file = choose_file(&directory, mode, &state.phone_name, &groups, options.fresh);
    file.begin_run(&command);
    println!(
        "Real-Phone Checklist, {}, with {}.",
        match options.group {
            Some(number) => format!("group {number}"),
            None => "every group".to_owned(),
        },
        state.phone_name
    );
    println!("Results: {}", file.path.display());
    let captures = bus_capture::directory_for(&file.path);
    println!(
        "Bluetooth D-Bus captures, a file a step: {}",
        captures.display()
    );
    sitting.capture = Some(bus_capture::BusCapture::start(
        system_bus,
        session_bus,
        captures,
    ));
    println!("While the runner waits, type skip to skip a step, or quit to stop.");
    flush();

    let order = steps::order();
    for group in &groups {
        match run_group(&mut sitting, &mut file, group, &order, options.demo) {
            Ok(()) => {}
            Err(Stop::Quit) => {
                println!();
                println!("Stopped.");
                summarise(&file, &groups, options, true);
                return ExitCode::SUCCESS;
            }
            Err(error) => {
                eprintln!("{error}");
                return ExitCode::FAILURE;
            }
        }
    }
    summarise(&file, &groups, options, false);
    ExitCode::SUCCESS
}

/// The groups asked for, or `None`, having said why, if the run can't start.
fn groups_to_run(options: &Options) -> Option<Vec<&'static Group>> {
    let groups: Vec<&Group> = match options.group {
        None => GROUPS.iter().collect(),
        Some(number) => {
            if let Some(group) = GROUPS.iter().find(|group| group.number == number) {
                vec![group]
            } else {
                eprintln!(
                    "There is no group {number}. The groups are 1 to {}.",
                    GROUPS.len()
                );
                return None;
            }
        }
    };
    // Against the real Phone the runner asks the user questions. Without a
    // terminal (Claude Code's `!` prefix, a pipe) it would read nothing and
    // stop with no reason, so it says why before it touches anything. The
    // demo is played through a pipe on purpose.
    if !options.demo && !std::io::stdin().is_terminal() {
        eprintln!(
            "The Real-Phone Checklist needs an interactive terminal to ask you questions, so run `{}` in your own terminal.",
            resume_command(options)
        );
        return None;
    }
    Some(groups)
}

/// The Phone and app the runner observes.
struct World {
    mode: Mode,
    bus: Connection,
    demo: Option<Demo>,
}

impl World {
    /// The user's Desktop: the session bus and the journal.
    fn real() -> Option<Self> {
        match zbus::block_on(Connection::session()) {
            Ok(bus) => Some(Self {
                mode: Mode::Real,
                bus,
                demo: None,
            }),
            Err(error) => {
                eprintln!("Can't reach the session bus: {error}");
                None
            }
        }
    }

    /// The app against the Simulated Phone, in a compositor of its own.
    fn demo(headless: bool) -> Option<Self> {
        // UTC, so the demo can tell the local clock from the Unix time: 6.6
        // sets Quiet Hours a minute ahead by it.
        let builder = Harness::builder().phone_connected().time_zone("UTC");
        let builder = match super::demo::desktop_socket() {
            Some(socket) if !headless => builder.nested_screen(socket),
            _ => builder.headless_screen(),
        };
        let harness = builder.start();
        harness.wait_for_app(
            "ConnectionState is connected",
            DEMO_CONNECT_DEADLINE,
            |state| state.connection_state == "connected",
        );
        let address = harness.session_bus_address();
        let bus = match zbus::block_on(async {
            zbus::connection::Builder::address(address).build().await
        }) {
            Ok(bus) => bus,
            Err(error) => {
                eprintln!("Can't reach the Simulated Phone's session bus: {error}");
                return None;
            }
        };
        println!("Demo: the Simulated Phone stands in for the Pixel.");
        println!("Logs: {}", harness.directory().display());
        Some(Self {
            mode: Mode::Demo,
            bus,
            demo: Some(Demo::new(harness)),
        })
    }

    /// New connections to the system and session buses, for the bus
    /// capture alone: the session one becomes a monitor.
    fn capture_buses(&self) -> (zbus::Result<Connection>, zbus::Result<Connection>) {
        let at = |address: &str| {
            zbus::block_on(async { zbus::connection::Builder::address(address).build().await })
        };
        match &self.demo {
            Some(demo) => (
                at(demo.harness.system_bus_address()),
                at(demo.harness.session_bus_address()),
            ),
            None => (
                zbus::block_on(Connection::system()),
                zbus::block_on(Connection::session()),
            ),
        }
    }

    fn log(&self) -> AppLog {
        match &self.demo {
            Some(demo) => AppLog::File(
                demo.harness
                    .directory()
                    .join("state/desktop-calls/desktop-calls.log"),
            ),
            None => AppLog::Journal,
        }
    }
}

/// Where this run's result files go: `--results`, else the user's state
/// directory against the real Phone, or `target/` under the Simulated
/// Phone. `None`, having said why, if there is nowhere safe.
fn results_directory(options: &Options) -> Option<PathBuf> {
    if let Some(directory) = &options.results {
        return Some(directory.clone());
    }
    if options.demo {
        return Some(
            super::target_directory()
                .unwrap_or_default()
                .join("real-phone-checklist-demo"),
        );
    }
    match results::default_directory() {
        Ok(directory) => Some(directory),
        Err(why) => {
            eprintln!("{why}");
            None
        }
    }
}

/// The latest result file, if it is an unfinished run of these groups;
/// otherwise a new one.
fn choose_file(
    directory: &Path,
    mode: Mode,
    phone: &str,
    groups: &[&Group],
    fresh: bool,
) -> ResultFile {
    let runnable: Vec<&Step> = groups
        .iter()
        .flat_map(|group| group.steps)
        .filter(|step| step.script.is_some())
        .collect();
    let latest = (!fresh)
        .then(|| ResultFile::latest(directory, mode))
        .flatten()
        .filter(|file| {
            let ran_any = runnable.iter().any(|step| file.step(step.id).is_some());
            let unfinished = runnable.iter().any(|step| {
                file.step(step.id)
                    .is_none_or(|record| !record.outcome.is_done())
            });
            ran_any && unfinished
        });
    match latest {
        Some(file) => {
            let passed: Vec<String> = runnable
                .iter()
                .filter_map(|step| file.step(step.id))
                .filter(|record| record.outcome.is_done())
                .map(|record| {
                    let unexpected = if record.outcome == Outcome::Unexpected {
                        ", observed, unexpected"
                    } else {
                        ""
                    };
                    format!("{} ({}{unexpected})", record.id, which_run(record.run))
                })
                .collect();
            println!(
                "Resuming the results started {}, as run {} of them. Done already: {}. (--fresh starts over.)",
                file.started,
                file.run() + 1,
                if passed.is_empty() {
                    "none".to_owned()
                } else {
                    passed.join(", ")
                }
            );
            file
        }
        None => ResultFile::new(directory, mode, phone),
    }
}

/// Runs one group: its list, then each runnable step not yet passed.
fn run_group(
    sitting: &mut Sitting,
    file: &mut ResultFile,
    group: &Group,
    order: &[&str],
    demo: bool,
) -> Result<(), Stop> {
    let to_run: Vec<&Step> = group
        .steps
        .iter()
        .filter(|step| step.script.is_some())
        .filter(|step| {
            file.step(step.id)
                .is_none_or(|record| !record.outcome.is_done())
        })
        .collect();
    let waiting: Vec<&Step> = group
        .steps
        .iter()
        .filter(|step| step.script.is_none())
        .collect();
    println!();
    println!("Group {}: {}", group.number, group.title);
    for step in &waiting {
        println!(
            "  Not yet runnable: {} {} ({}).",
            step.id,
            step.title,
            waits_on(step)
        );
    }
    if to_run.is_empty() {
        println!("  Nothing to run in this group now.");
        flush();
        return Ok(());
    }
    // Only what the steps about to run need, each once.
    let mut needs: Vec<&str> = vec!["The Pixel charged, nearby and connected."];
    for need in to_run.iter().flat_map(|step| step.needs) {
        if !needs.contains(need) {
            needs.push(need);
        }
    }
    println!("Before you start:");
    for need in needs {
        println!("  - {need}");
    }
    let ready = sitting.ready(&format!(
        "Press Enter to start group {}, or type skip or quit.",
        group.number
    ));
    // The file records the group once the user has started or skipped it.
    for step in waiting {
        if file.step(step.id).is_none() {
            let evidence = format!("Not yet runnable: {}.", waits_on(step));
            record(file, order, skipped(step, evidence));
        }
    }
    match ready {
        Ok(()) => {}
        Err(Stop::Skipped) => {
            for step in to_run {
                record(
                    file,
                    order,
                    skipped(step, "You skipped this group.".to_owned()),
                );
            }
            return Ok(());
        }
        Err(stop) => return Err(stop),
    }
    let mut queue: VecDeque<&Step> = to_run.into();
    while let Some(step) = queue.pop_front() {
        let (record_of_step, back_to, way_back) = run_step_captured(sitting, step)?;
        if let Some(record_of_step) = record_of_step {
            record(file, order, record_of_step);
        }
        if way_back {
            say_how_to_go_on(&sitting.now(), file, group, step, demo);
        }
        // Going back runs the earlier step and every runnable one after it
        // up to this one again, then carries on as before.
        let Some(back_to) = back_to else { continue };
        let position = |id: &str| group.steps.iter().position(|other| other.id == id);
        if let (Some(from), Some(to)) = (position(back_to), position(step.id)) {
            for again in group.steps[from..=to].iter().rev() {
                if again.script.is_some() {
                    queue.push_front(again);
                }
            }
        }
    }
    Ok(())
}

/// "run 2", or "an earlier run" for a record from before runs were kept.
fn which_run(run: u32) -> String {
    if run == 0 {
        "an earlier run".to_owned()
    } else {
        format!("run {run}")
    }
}

/// [`run_step`], with the step's Bluetooth D-Bus capture written beside
/// the result file and named in its record. A step quit part way keeps
/// its capture too, said on the terminal, since it has no record.
fn run_step_captured(
    sitting: &mut Sitting,
    step: &Step,
) -> Result<(Option<StepRecord>, Option<&'static str>, bool), Stop> {
    let started_us = results::unix_micros();
    if let Some(capture) = &sitting.capture {
        capture.snapshot("start");
    }
    let ran = run_step(sitting, step, started_us);
    let Some(capture) = &sitting.capture else {
        return ran;
    };
    match ran {
        Ok((Some(mut record), back_to, way_back)) => {
            capture.snapshot("end");
            record.bus = capture.write(step.id, started_us, results::unix_micros());
            Ok((Some(record), back_to, way_back))
        }
        Err(Stop::Quit) => {
            capture.snapshot("end");
            if let Some(name) = capture.write(step.id, started_us, results::unix_micros()) {
                println!(
                    "The Bluetooth D-Bus capture of step {}, to where you quit: {}",
                    step.id,
                    capture.path(&name).display()
                );
            }
            Err(Stop::Quit)
        }
        other => other,
    }
}

/// Runs one step's script and returns its record, if it has one; the step
/// to go back to, if the user chose to; and whether the user needs the way
/// back to it spelled out: it failed, or couldn't run yet (#392).
fn run_step(
    sitting: &mut Sitting,
    step: &Step,
    started_us: u64,
) -> Result<(Option<StepRecord>, Option<&'static str>, bool), Stop> {
    println!();
    println!("Step {}: {}", step.id, step.title);
    flush();
    let finished = match step.script {
        Some(script) => script(sitting),
        None => Err(Stop::Skipped),
    };
    let mut findings = sitting.take_findings();
    let mut back_to = None;
    let mut not_ready = false;
    let outcome = match finished {
        Ok(()) if findings.failed => Outcome::Fail,
        Ok(()) if findings.unexpected => {
            unexpected_unless_the_app_erred(&sitting.log, &mut findings, started_us)
        }
        Ok(()) => Outcome::Pass,
        // A step that failed before it offered to go back keeps its
        // failure; one that only found it couldn't start records nothing.
        Err(Stop::BackTo(id)) if findings.failed => {
            back_to = Some(id);
            Outcome::Fail
        }
        Err(Stop::BackTo(id)) => {
            println!("  Going back to step {id}.");
            flush();
            return Ok((None, Some(id), false));
        }
        // A check that failed before the skip keeps the step failed.
        Err(Stop::Skipped) if findings.failed => {
            findings.evidence.push("You skipped this step.".to_owned());
            Outcome::Fail
        }
        Err(Stop::Skipped) => {
            findings.evidence.push("You skipped this step.".to_owned());
            Outcome::Skipped
        }
        Err(Stop::TimedOut(what)) => {
            println!("  Failed: {what}.");
            findings.evidence.push(format!("Failed: {what}."));
            Outcome::Fail
        }
        Err(Stop::NotReady(why)) if findings.failed => {
            findings.evidence.push(why);
            Outcome::Fail
        }
        Err(Stop::NotReady(why)) => {
            not_ready = true;
            println!("  {why}");
            findings.evidence.push(why);
            Outcome::Skipped
        }
        Err(Stop::Quit) => return Err(Stop::Quit),
    };
    // A step the user skipped needs no way back spelled out, nor one the
    // runner is going back from.
    let way_back = back_to.is_none()
        && match outcome {
            Outcome::Fail => true,
            Outcome::Skipped => not_ready,
            Outcome::Pass | Outcome::Unexpected => false,
        };
    let ended_us = results::unix_micros();
    let journal = journal(&sitting.log, outcome, started_us, ended_us);
    println!(
        "  {} {}.",
        step.id,
        match outcome {
            Outcome::Pass => "passed",
            Outcome::Fail => "failed",
            Outcome::Skipped => "skipped",
            Outcome::Unexpected => "observed, unexpected",
        }
    );
    if outcome == Outcome::Unexpected {
        println!(
            "  The Pixel did something this step didn't expect, and Desktop Calls logged no error. What was seen is kept in the results for an agent to read, and the run carries on. The Pixel is left {}.",
            pixel_left(&sitting.now())
        );
    }
    if let Some(id) = back_to {
        println!("  Going back to step {id}.");
    }
    flush();
    let record_of_step = StepRecord {
        id: step.id.to_owned(),
        title: step.title.to_owned(),
        outcome,
        run: 0,
        evidence: findings.evidence,
        measurements: findings.measurements,
        started_us,
        ended_us,
        journal,
        bus: None,
    };
    Ok((Some(record_of_step), back_to, way_back))
}

/// A failed or unexpected step keeps the app's log from its time, for the
/// agent who files it; any other step, none.
fn journal(log: &AppLog, outcome: Outcome, from_us: u64, to_us: u64) -> Vec<String> {
    if !matches!(outcome, Outcome::Fail | Outcome::Unexpected) {
        return Vec::new();
    }
    log.lines(from_us, to_us)
        .iter()
        .map(app_log::LogLine::excerpt)
        .collect()
}

/// For `step`, which failed or couldn't run yet: how the Pixel is left, and
/// the command that runs it again, with the earlier steps of `group` that
/// command runs first, since it runs every one not yet passed (#392).
fn say_how_to_go_on(
    state: &watch::AppState,
    file: &ResultFile,
    group: &Group,
    step: &Step,
    demo: bool,
) {
    let first: Vec<&str> = group
        .steps
        .iter()
        .take_while(|earlier| earlier.id != step.id)
        .filter(|earlier| earlier.script.is_some())
        .filter(|earlier| {
            file.step(earlier.id)
                .is_none_or(|record| !record.outcome.is_done())
        })
        .map(|earlier| earlier.id)
        .collect();
    let command = resume_command(&Options {
        group: Some(group.number),
        demo,
        headless: false,
        results: None,
        fresh: false,
    });
    let first = if first.is_empty() {
        String::new()
    } else {
        format!(", which runs {} first", words_list(&first))
    };
    println!(
        "  The Pixel is left {}. To run {} again: `{command}`{first}.",
        pixel_left(state),
        step.id
    );
    flush();
}

/// How the Pixel stands with Desktop Calls, as "paired, connected and
/// sharing Contacts", for a step that stops short.
fn pixel_left(state: &watch::AppState) -> String {
    if state.phone_name.is_empty() {
        return "not paired with this computer".to_owned();
    }
    let connected = if state.connection_state == "connected" {
        "connected"
    } else {
        "not connected"
    };
    let contacts = match state.contacts_state.as_str() {
        "ready" => "sharing Contacts",
        "syncing" => "syncing Contacts",
        "permission-needed" => "asking whether to share Contacts",
        _ => "not sharing Contacts",
    };
    format!("paired, {connected} and {contacts}")
}

/// A record of `step` as skipped, with why.
fn skipped(step: &Step, evidence: String) -> StepRecord {
    let now = results::unix_micros();
    StepRecord {
        id: step.id.to_owned(),
        title: step.title.to_owned(),
        outcome: Outcome::Skipped,
        run: 0,
        evidence: vec![evidence],
        measurements: serde_json::Map::new(),
        started_us: now,
        ended_us: now,
        journal: Vec::new(),
        bus: None,
    }
}

fn record(file: &mut ResultFile, order: &[&str], step: StepRecord) {
    if let Err(error) = file.record(step, order) {
        eprintln!("Couldn't write {}: {error}", file.path.display());
    }
}

/// Ends a run, finished or `stopped`, with a plain summary (#380): each
/// runnable step of `groups`, how it stands and why, then the one command
/// to run next.
fn summarise(file: &ResultFile, groups: &[&Group], options: &Options, stopped: bool) {
    let run = file.run();
    let mut not_passed: Vec<&str> = Vec::new();
    let mut unexpected: Vec<&str> = Vec::new();
    println!();
    println!("Summary:");
    for group in groups {
        let mut waiting: Vec<&str> = Vec::new();
        for step in group.steps {
            if step.script.is_none() {
                waiting.push(step.id);
                continue;
            }
            let Some(record) = file.step(step.id) else {
                not_passed.push(step.id);
                println!("  {} not run.", step.id);
                continue;
            };
            let when = if record.run == run {
                String::new()
            } else {
                format!(" in {}", which_run(record.run))
            };
            match record.outcome {
                Outcome::Pass => println!("  {} passed{when}.", step.id),
                Outcome::Fail => {
                    not_passed.push(step.id);
                    println!("  {} failed{when}: {}", step.id, why_failed(record));
                }
                Outcome::Skipped => {
                    not_passed.push(step.id);
                    println!("  {} skipped{when}: {}", step.id, record.evidence.join(" "));
                }
                Outcome::Unexpected => {
                    unexpected.push(step.id);
                    println!(
                        "  {} observed, unexpected{when}: {}",
                        step.id,
                        what_was_unexpected(record)
                    );
                }
            }
        }
        if !waiting.is_empty() {
            println!(
                "  Not yet runnable in group {}: {}.",
                group.number,
                waiting.join(", ")
            );
        }
    }
    println!("Results: {}", file.path.display());
    if !unexpected.is_empty() {
        println!(
            "Observed, unexpected: {}. Not failures, and not run again; an agent reads what was seen before the next sitting.",
            words_list(&unexpected)
        );
    }
    let command = resume_command(options);
    let next_group = options
        .group
        .and_then(|number| GROUPS.iter().find(|group| group.number == number + 1));
    let next = match (not_passed.first(), next_group) {
        (Some(first), _) if stopped => format!("run `{command}` to carry on from {first}"),
        (Some(_), _) => format!("run `{command}` to retry {}", words_list(&not_passed)),
        (None, Some(group)) => format!(
            "run `{}` for group {}",
            resume_command(&Options {
                group: Some(group.number),
                demo: options.demo,
                headless: false,
                results: None,
                fresh: false,
            }),
            group.number
        ),
        (None, None) => "nothing to run; every runnable step has passed".to_owned(),
    };
    println!("Next: {next}.");
    flush();
}

/// Why `record` failed: its failed checks, or else its last evidence.
fn why_failed(record: &StepRecord) -> String {
    let failures: Vec<&str> = record
        .evidence
        .iter()
        .filter_map(|line| line.strip_prefix("Failed: "))
        .collect();
    if failures.is_empty() {
        record.evidence.last().cloned().unwrap_or_default()
    } else {
        failures.join(" ")
    }
}

/// What `record`, observed and unexpected, saw that the step didn't expect.
fn what_was_unexpected(record: &StepRecord) -> String {
    let seen: Vec<&str> = record
        .evidence
        .iter()
        .filter_map(|line| line.strip_prefix("Unexpected: "))
        .collect();
    seen.join(" ")
}

/// A step that ran to its end with the Phone doing something it didn't
/// expect is "observed, unexpected", unless the app logged an error in the
/// step's time: then the app may be at fault, and the step fails, with the
/// errors in its evidence.
fn unexpected_unless_the_app_erred(
    log: &AppLog,
    findings: &mut sitting::Findings,
    started_us: u64,
) -> Outcome {
    let Some(lines) = log.read(started_us, results::unix_micros()) else {
        let why = "The Pixel did something this step didn't expect, and Desktop Calls' log couldn't be read to check it logged no error";
        println!("  Failed: {why}.");
        findings.evidence.push(format!("Failed: {why}."));
        return Outcome::Fail;
    };
    let errors: Vec<String> = lines
        .iter()
        .filter(|line| line.level == "error")
        .map(app_log::LogLine::excerpt)
        .collect();
    if errors.is_empty() {
        return Outcome::Unexpected;
    }
    let why = "Desktop Calls logged an error while the Pixel did something this step didn't expect";
    println!("  Failed: {why}.");
    findings
        .evidence
        .push(format!("Failed: {why}: {}", errors.join(" | ")));
    Outcome::Fail
}

/// `1.1`, `1.1 and 1.3`, `1.1, 1.2 and 1.3`.
fn words_list(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The command that runs these groups again.
fn resume_command(options: &Options) -> String {
    let demo = if options.demo { " --demo" } else { "" };
    let group = options
        .group
        .map(|group| format!(" {group}"))
        .unwrap_or_default();
    format!("just real-phone-checklist{demo}{group}")
}

/// Why `step`, which has no script, can't run yet: the tickets it lands
/// with, or the step it runs together with.
fn waits_on(step: &Step) -> String {
    let partner = steps::RUNS_WITH
        .iter()
        .find(|(id, _)| *id == step.id)
        .map(|(_, partner)| *partner);
    let Some(partner) = partner else {
        return format!("lands with {}", tickets(step.lands_with));
    };
    let partner_lands_with = steps::GROUPS
        .iter()
        .flat_map(|group| group.steps)
        .find(|other| other.id == partner && other.script.is_none())
        .map(|other| other.lands_with);
    match partner_lands_with {
        Some(numbers) if !numbers.is_empty() => format!(
            "runs together with {partner}, which lands with {}",
            tickets(numbers)
        ),
        _ => format!("runs together with {partner}"),
    }
}

fn tickets(numbers: &[u32]) -> String {
    let named: Vec<String> = numbers.iter().map(|number| format!("#{number}")).collect();
    words_list(&named.iter().map(String::as_str).collect::<Vec<_>>())
}

impl std::fmt::Display for Stop {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Skipped => formatter.write_str("skipped"),
            Self::Quit => formatter.write_str("stopped"),
            Self::TimedOut(what) => formatter.write_str(what),
            Self::BackTo(id) => write!(formatter, "going back to step {id}"),
            Self::NotReady(why) => formatter.write_str(why),
        }
    }
}

/// The rule that keeps "observed, unexpected" from hiding a fault of the
/// app's own, at the runner's interface: the demo can't make the app log
/// an error on cue, so these give it a log file directly.
#[cfg(test)]
mod tests {
    use super::{AppLog, Outcome, results, sitting::Findings, unexpected_unless_the_app_erred};

    /// A log file holding one line at `level`, logged now.
    fn log_with(level: &str) -> (tempfile::TempDir, AppLog) {
        let directory = tempfile::tempdir().expect("a directory");
        let path = directory.path().join("desktop-calls.log");
        let line = format!(
            r#"{{"timestamp_us":{},"level":"{level}","event_kind":"phone-command-failed","message":"Hang up never confirmed"}}"#,
            results::unix_micros()
        );
        std::fs::write(&path, line + "\n").expect("write the log");
        (directory, AppLog::File(path))
    }

    #[test]
    fn an_unexpected_step_fails_when_the_app_logged_an_error_in_its_time() {
        let started = results::unix_micros() - 1_000_000;
        let (_directory, log) = log_with("ERROR");
        let mut findings = Findings::default();
        let outcome = unexpected_unless_the_app_erred(&log, &mut findings, started);
        assert_eq!(outcome, Outcome::Fail);
        assert!(
            findings.evidence.iter().any(|line| line
                .starts_with("Failed: Desktop Calls logged an error")
                && line.contains("Hang up never confirmed")),
            "{:?}",
            findings.evidence
        );
    }

    #[test]
    fn an_unexpected_step_stays_unexpected_when_the_app_logged_only_warnings() {
        let started = results::unix_micros() - 1_000_000;
        let (_directory, log) = log_with("WARN");
        let mut findings = Findings::default();
        let outcome = unexpected_unless_the_app_erred(&log, &mut findings, started);
        assert_eq!(outcome, Outcome::Unexpected);
        assert!(findings.evidence.is_empty(), "{:?}", findings.evidence);
    }

    #[test]
    fn an_unexpected_step_fails_when_the_apps_log_cant_be_read() {
        let directory = tempfile::tempdir().expect("a directory");
        let log = AppLog::File(directory.path().join("never-written.log"));
        let mut findings = Findings::default();
        let outcome = unexpected_unless_the_app_erred(&log, &mut findings, 0);
        assert_eq!(outcome, Outcome::Fail);
        assert!(
            findings
                .evidence
                .iter()
                .any(|line| line.contains("couldn't be read")),
            "{:?}",
            findings.evidence
        );
    }
}
