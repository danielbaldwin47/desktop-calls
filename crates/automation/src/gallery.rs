//! The screenshot gallery: every surface in every state, in a light and a
//! dark Desktop, with an `index.html` for a screenshot-review agent.
//!
//! Each surface ticket adds its states to [`STATES`].

use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use test_harness::{Harness, Screenshot, Theme};

/// How long the app may take to reach a state.
const DEADLINE: Duration = Duration::from_secs(10);

/// One state the gallery shows.
struct State {
    /// The file name stem, such as `bare-output`.
    name: &'static str,
    /// The caption.
    title: &'static str,
    /// Brings the scenario into the state.
    reach: fn(&Harness),
    /// Whether a capture shows the state yet.
    shown: fn(&Screenshot) -> bool,
}

/// A caller in the range Ofcom keeps for drama (07700 900xxx).
const NUMBER: &str = "+447700900123";

/// The Answer button's green and the Decline and Hang up buttons' red.
const GREEN: [u8; 3] = [0x26, 0xa2, 0x69];
const RED: [u8; 3] = [0xc0, 0x1c, 0x28];

const STATES: &[State] = &[
    State {
        name: "bare-output",
        title: "The bare output, with the Phone connected and no surface shown",
        reach: connect,
        shown: |_| true,
    },
    State {
        name: "incoming-call-banner",
        title: "The Incoming Call Banner, ringing",
        reach: |harness| {
            connect(harness);
            ring(harness);
        },
        shown: |screenshot| shows(screenshot, GREEN) && shows(screenshot, RED),
    },
    State {
        name: "call-window-live",
        title: "The Call Window, answered here, with HD audio on this computer",
        reach: |harness| {
            connect(harness);
            harness.simulated_phone().set_codec(2);
            let call = ring(harness);
            harness.answer(call);
            harness.wait_for_app("the Call Window shows HD", DEADLINE, |state| {
                state.surface("call-window").is_some_and(|window| {
                    window.contains_key("hd") && window.contains_key("audio-location")
                })
            });
        },
        shown: |screenshot| shows(screenshot, RED) && !shows(screenshot, GREEN),
    },
    State {
        name: "call-window-ended",
        title: "The Call Window, just after the Call ended",
        reach: |harness| {
            connect(harness);
            let call = ring(harness);
            harness.answer(call);
            harness.wait_for_app("the Call is live", DEADLINE, |state| {
                state
                    .surface("call-window")
                    .is_some_and(|window| window.contains_key("audio-location"))
            });
            harness.hang_up(call);
            harness.wait_for_app("the Call Window says Call ended", DEADLINE, |state| {
                state
                    .surface("call-window")
                    .and_then(|window| window.get("status"))
                    .is_some_and(|status| status == "Call ended")
            });
        },
        // Hang up is dimmed once the Call has ended.
        shown: |screenshot| {
            !shows(screenshot, RED)
                && screenshot
                    .region_where(|pixel| pixel != [0, 0, 0])
                    .is_some()
        },
    },
];

fn connect(harness: &Harness) {
    harness.simulated_phone().connect();
    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
}

/// Rings the Phone, waits for the Banner and returns the Call's id.
fn ring(harness: &Harness) -> u32 {
    let _path = harness.simulated_phone().ring(NUMBER);
    let state = harness.wait_for_app("the Banner shows", DEADLINE, |state| {
        state.surface("banner").is_some()
    });
    state.calls.first().map_or(0, |call| call.id)
}

/// Whether `colour`, give or take antialiasing, is on screen.
fn shows(screenshot: &Screenshot, colour: [u8; 3]) -> bool {
    screenshot
        .region_where(|pixel| {
            pixel
                .iter()
                .zip(colour)
                .all(|(channel, wanted)| channel.abs_diff(wanted) <= 16)
        })
        .is_some()
}

const THEMES: [Theme; 2] = [Theme::Light, Theme::Dark];

/// `automation screenshot`: renders the gallery into `output`, or
/// `target/screenshots/gallery`.
pub fn run(output: Option<PathBuf>) -> ExitCode {
    let output = if let Some(output) = output {
        output
    } else {
        let Some(target) = crate::target_directory() else {
            eprintln!("Can't find the target directory; pass --output.");
            return ExitCode::FAILURE;
        };
        let default = target.join("screenshots/gallery");
        // The default gallery is ours alone, so stale states go.
        let _ = fs::remove_dir_all(&default);
        default
    };
    if let Err(error) = fs::create_dir_all(&output) {
        eprintln!("Can't create {}: {error}", output.display());
        return ExitCode::FAILURE;
    }

    let mut shots = Vec::new();
    for state in STATES {
        for theme in THEMES {
            let harness = Harness::builder()
                .headless_screen()
                .theme(theme)
                .screenshot_directory(output.clone())
                .start();
            (state.reach)(&harness);
            let file = format!("{}-{}", state.name, theme.as_str());
            let screenshot = harness.screenshot_when(&file, DEADLINE, state.shown);
            shots.push((state, theme, screenshot.path));
        }
    }

    let index = output.join("index.html");
    if let Err(error) = fs::write(&index, index_html(&shots)) {
        eprintln!("Can't write {}: {error}", index.display());
        return ExitCode::FAILURE;
    }
    println!(
        "Gallery: {} screenshots of {} states, light and dark.",
        shots.len(),
        STATES.len()
    );
    println!("{}", index.display());
    ExitCode::SUCCESS
}

/// The gallery page: one row per state, light beside dark.
fn index_html(shots: &[(&State, Theme, PathBuf)]) -> String {
    let mut figures = String::new();
    for (state, theme, path) in shots {
        let file = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let _ = writeln!(
            figures,
            "<figure><a href=\"{file}\"><img src=\"{file}\" alt=\"{title}, {theme}\"></a>\
             <figcaption>{title} ({theme})</figcaption></figure>",
            title = state.title,
            theme = theme.as_str(),
        );
    }
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <title>Desktop Calls screenshot gallery</title>\n\
         <style>body{{font-family:sans-serif;margin:16px}}\
         main{{display:grid;grid-template-columns:repeat(2,1fr);gap:16px}}\
         img{{width:100%;border:1px solid #888}}</style>\n</head>\n<body>\n\
         <h1>Desktop Calls screenshot gallery</h1>\n<main>\n{figures}</main>\n</body>\n</html>\n"
    )
}
