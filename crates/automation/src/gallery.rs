//! The screenshot gallery: every surface in every state, in a light and a
//! dark Desktop, with an `index.html` for a screenshot-review agent.
//!
//! Each surface ticket adds its states to [`STATES`].

use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use test_harness::{Harness, Theme};

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
}

const STATES: &[State] = &[State {
    name: "bare-output",
    title: "The bare output, with the Phone connected and no surface shown",
    reach: |harness| {
        harness.simulated_phone().connect();
        harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
            state.connection_state == "connected"
        });
    },
}];

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
            let screenshot = harness.screenshot(&file);
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
