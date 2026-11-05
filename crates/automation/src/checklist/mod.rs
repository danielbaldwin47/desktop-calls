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
mod fingerprint;
pub mod results;
mod sitting;
mod steps;
mod watch;

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
    let timeline = match watch::start(&world.bus, sender.clone()) {
        Ok(timeline) => timeline,
        Err(watch::WatchError::NotRunning) => {
            eprintln!(
                // Until `just install` lands (#20), the service runs from a
                // terminal.
                "Desktop Calls isn't running, so the checklist can't observe it. Start it in another terminal with `cargo run --release -p desktop-calls -- service`, then run this again."
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
    println!(
        "Real-Phone Checklist, {}, with {}.",
        match options.group {
            Some(number) => format!("group {number}"),
            None => "every group".to_owned(),
        },
        state.phone_name
    );
    println!("Results: {}", file.path.display());
    println!("While the runner waits, type skip to skip a step, or quit to stop.");
    flush();

    let order = steps::order();
    for group in &groups {
        match run_group(&mut sitting, &mut file, group, &order) {
            Ok(()) => {}
            Err(Stop::Quit) => {
                println!();
                println!(
                    "Stopped. Your results so far are in {}.",
                    file.path.display()
                );
                println!("To carry on where you stopped, run `{command}`.");
                return ExitCode::SUCCESS;
            }
            Err(error) => {
                eprintln!("{error}");
                return ExitCode::FAILURE;
            }
        }
    }
    summarise(&file, &groups, &command);
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
        let builder = Harness::builder().phone_connected();
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
                    .is_none_or(|record| record.outcome != Outcome::Pass)
            });
            ran_any && unfinished
        });
    match latest {
        Some(file) => {
            let passed: Vec<&str> = runnable
                .iter()
                .filter(|step| {
                    file.step(step.id)
                        .is_some_and(|record| record.outcome == Outcome::Pass)
                })
                .map(|step| step.id)
                .collect();
            println!(
                "Resuming the run started {}. Passed already: {}. (--fresh starts over.)",
                file.started,
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
) -> Result<(), Stop> {
    let to_run: Vec<&Step> = group
        .steps
        .iter()
        .filter(|step| step.script.is_some())
        .filter(|step| {
            file.step(step.id)
                .is_none_or(|record| record.outcome != Outcome::Pass)
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
    for step in to_run {
        let record_of_step = run_step(sitting, step)?;
        record(file, order, record_of_step);
    }
    Ok(())
}

/// Runs one step's script and returns its record.
fn run_step(sitting: &mut Sitting, step: &Step) -> Result<StepRecord, Stop> {
    println!();
    println!("Step {}: {}", step.id, step.title);
    flush();
    let started_us = results::unix_micros();
    let finished = match step.script {
        Some(script) => script(sitting),
        None => Err(Stop::Skipped),
    };
    let mut findings = sitting.take_findings();
    let outcome = match finished {
        Ok(()) if findings.failed => Outcome::Fail,
        Ok(()) => Outcome::Pass,
        Err(Stop::Skipped) => {
            findings.evidence.push("You skipped this step.".to_owned());
            Outcome::Skipped
        }
        Err(Stop::TimedOut(what)) => {
            println!("  Failed: {what}.");
            findings.evidence.push(format!("Failed: {what}."));
            Outcome::Fail
        }
        Err(Stop::Quit) => return Err(Stop::Quit),
    };
    let ended_us = results::unix_micros();
    // A failed step keeps the app's log from its time, for the agent who
    // files it.
    let journal = if outcome == Outcome::Fail {
        sitting
            .log
            .lines(started_us, ended_us)
            .iter()
            .map(app_log::LogLine::excerpt)
            .collect()
    } else {
        Vec::new()
    };
    println!(
        "  {} {}.",
        step.id,
        match outcome {
            Outcome::Pass => "passed",
            Outcome::Fail => "failed",
            Outcome::Skipped => "skipped",
        }
    );
    flush();
    Ok(StepRecord {
        id: step.id.to_owned(),
        title: step.title.to_owned(),
        outcome,
        evidence: findings.evidence,
        measurements: findings.measurements,
        started_us,
        ended_us,
        journal,
    })
}

/// A record of `step` as skipped, with why.
fn skipped(step: &Step, evidence: String) -> StepRecord {
    let now = results::unix_micros();
    StepRecord {
        id: step.id.to_owned(),
        title: step.title.to_owned(),
        outcome: Outcome::Skipped,
        evidence: vec![evidence],
        measurements: serde_json::Map::new(),
        started_us: now,
        ended_us: now,
        journal: Vec::new(),
    }
}

fn record(file: &mut ResultFile, order: &[&str], step: StepRecord) {
    if let Err(error) = file.record(step, order) {
        eprintln!("Couldn't write {}: {error}", file.path.display());
    }
}

fn summarise(file: &ResultFile, groups: &[&Group], command: &str) {
    println!();
    let mut retry = false;
    for group in groups {
        let count = |outcome: Outcome| {
            group
                .steps
                .iter()
                .filter(|step| step.script.is_some())
                .filter(|step| {
                    file.step(step.id)
                        .is_some_and(|record| record.outcome == outcome)
                })
                .count()
        };
        let runnable = group
            .steps
            .iter()
            .filter(|step| step.script.is_some())
            .count();
        let waiting = group.steps.len() - runnable;
        let (passed, failed, skipped) = (
            count(Outcome::Pass),
            count(Outcome::Fail),
            count(Outcome::Skipped),
        );
        retry |= failed + skipped > 0;
        println!(
            "Group {}: {passed} of {runnable} runnable steps pass{}{}; {waiting} not yet runnable.",
            group.number,
            if failed > 0 {
                format!(", {failed} failed")
            } else {
                String::new()
            },
            if skipped > 0 {
                format!(", {skipped} skipped")
            } else {
                String::new()
            },
        );
    }
    println!("Results: {}", file.path.display());
    if retry {
        println!("To retry the steps that didn't pass, run `{command}`.");
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
    match named.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

impl std::fmt::Display for Stop {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Skipped => formatter.write_str("skipped"),
            Self::Quit => formatter.write_str("stopped"),
            Self::TimedOut(what) => formatter.write_str(what),
        }
    }
}
