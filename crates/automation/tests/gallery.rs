//! `automation screenshot`: the gallery of every surface in every state,
//! light and dark, stock and under an Omarchy theme.

use std::path::Path;
use std::process::Command;

/// Renders the gallery exactly as `just screenshot` does, into its default
/// directory, `target/screenshots/gallery`. CI keeps that directory as an
/// artifact, so this test is also what makes CI's gallery.
#[test]
fn the_gallery_shows_every_state_in_light_and_dark_stock_and_under_omarchy() {
    let binary = Path::new(env!("CARGO_BIN_EXE_automation"));
    let output = binary
        .parent()
        .and_then(Path::parent)
        .expect("the binary is in target/<profile>/")
        .join("screenshots/gallery");

    let mut command = Command::new(binary);
    command.arg("screenshot");
    // The gallery is one test, so the test runner's limit on how many run at
    // once doesn't reach it. `just ci-local` sets that limit to what the
    // Desktop has to spare, and each state here is a compositor and an app.
    // Never more than one per CPU, which is what the gallery does unasked.
    let spare = std::env::var("RUST_TEST_THREADS")
        .ok()
        .and_then(|threads| threads.parse::<usize>().ok())
        .filter(|threads| *threads > 0);
    if let Some(spare) = spare {
        let cpus = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
        command.args(["--workers", &spare.min(cpus).to_string()]);
    }
    let rendered = command.output().expect("run automation screenshot");

    assert!(
        rendered.status.success(),
        "automation screenshot failed ({}). Its standard error:\n{}",
        rendered.status,
        String::from_utf8_lossy(&rendered.stderr)
    );
    let index =
        std::fs::read_to_string(output.join("index.html")).expect("the gallery has an index");
    let states = [
        "bare-output",
        "incoming-call-banner",
        "incoming-call-banner-identifying",
        "call-window-live",
        "call-window-muted",
        "call-window-keypad",
        "call-window-audio-on-phone",
        "call-window-ended",
        "call-window-connection-lost",
        "call-window-calling",
        "call-window-ringing",
        "call-window-not-placed",
        "call-waiting-banner",
        "call-window-held",
        "call-window-held-third-call-waiting",
        "call-pill",
        "call-pill-muted",
        "call-pill-audio-on-phone",
        "call-pill-on-your-phone",
        "call-pill-held",
        "call-pill-ended",
    ];
    let files = states.iter().flat_map(|state| {
        ["light", "dark", "omarchy-light", "omarchy-dark"].map(|look| format!("{state}-{look}.png"))
    });
    for file in files {
        let file = file.as_str();
        assert!(index.contains(file), "the index shows {file}: {index}");
        let png = std::fs::read(output.join(file)).expect("the screenshot exists");
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"), "{file} is a PNG");
        // Width and height from the PNG header: the 3840 by 2160 output.
        assert_eq!(&png[16..24], &[0, 0, 0x0f, 0, 0, 0, 0x08, 0x70], "{file}");
    }

    // The Indicator, drawn as Omarchy's bar draws it: a strip of bar 48 by
    // 26 logical pixels at 2x, each state's icon different.
    let mut icons = Vec::new();
    for state in [
        "connected",
        "disconnected",
        "ringing",
        "in-call",
        "problem",
        "off",
    ] {
        for look in ["omarchy-light", "omarchy-dark"] {
            let file = format!("indicator-{state}-{look}.png");
            assert!(index.contains(&file), "the index shows {file}: {index}");
            let png = std::fs::read(output.join(&file)).expect("the icon exists");
            assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"), "{file} is a PNG");
            assert_eq!(&png[16..24], &[0, 0, 0, 96, 0, 0, 0, 52], "{file}");
            if look == "omarchy-dark" {
                icons.push(png);
            }
        }
    }
    for (index, icon) in icons.iter().enumerate() {
        assert!(!icons[..index].contains(icon), "each state looks different");
    }
    indicator_menus_are_listed(&output, &index);
}

/// The menu of the Indicator in `state`, as the gallery at `output` lists it.
fn menu(output: &Path, state: &str) -> String {
    let path = output.join(format!("indicator-{state}-menu.txt"));
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("the {state} menu is listed at {}: {error}", path.display()))
}

/// The Indicator's menus in the gallery at `output`, whose page is `index`.
fn indicator_menus_are_listed(output: &Path, index: &str) {
    // Each state's menu, listed beneath its icon: calls off offers Turn
    // calls on, and calls on offers Turn calls off, until I turn them on.
    let off_menu = menu(output, "off");
    assert_eq!(
        off_menu,
        "Calls off\nOpen Dialer\nTurn calls on\nConnect your phone…\n"
    );
    assert!(
        index.contains("<pre>Calls off\nOpen Dialer\nTurn calls on\n"),
        "{index}"
    );
    let connected_menu = menu(output, "connected");
    assert!(
        connected_menu.contains("Turn calls off\n    Until I turn them on\n    For 1 hour (until "),
        "{connected_menu}"
    );
    assert!(
        connected_menu.contains(")\n    Until tomorrow morning (07:00)\n"),
        "{connected_menu}"
    );
    // Calls off until a time: the menu's first line says until when.
    let timed_menu = menu(output, "off-until-morning");
    assert_eq!(
        timed_menu,
        "Calls off until 07:00\nOpen Dialer\nTurn calls on\nConnect your phone…\n"
    );
    for look in ["omarchy-light", "omarchy-dark"] {
        let file = format!("indicator-off-until-morning-{look}.png");
        assert!(index.contains(&file), "the index shows {file}: {index}");
    }
    // Turned off during a Call: the menu says calls turn off when it ends,
    // and offers Keep calls on where Turn calls off was.
    let pending_menu = menu(output, "calls-off-pending");
    assert!(
        pending_menu.starts_with(
            "Calls turn off when this Call ends\nOpen Dialer\nKeep calls on\nConnect your phone…\n"
        ),
        "{pending_menu}"
    );
    for look in ["omarchy-light", "omarchy-dark"] {
        let file = format!("indicator-calls-off-pending-{look}.png");
        assert!(index.contains(&file), "the index shows {file}: {index}");
    }
}
