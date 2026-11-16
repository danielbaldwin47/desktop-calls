//! `automation screenshot`: the gallery of every surface in every state,
//! light and dark, stock and under an Omarchy theme.

use std::path::Path;
use std::process::Command;

/// The states the gallery shows, each in four looks.
const STATES: &[&str] = &[
    "bare-output",
    "incoming-call-banner",
    "incoming-call-banner-identifying",
    "incoming-call-banner-keyboard",
    "incoming-call-banner-not-in-contacts",
    "incoming-call-banner-no-keybinding",
    "incoming-call-banner-keybinding-unreadable",
    "incoming-call-banner-withheld",
    "call-waiting-banner-keyboard",
    "call-waiting-banner-without-hold",
    "call-window-live",
    "call-window-muted",
    "call-window-keypad",
    "call-window-audio-on-phone",
    "call-window-moving-audio",
    "call-window-ended",
    "call-window-connection-lost",
    "call-window-calling",
    "call-window-ringing",
    "call-window-not-placed",
    "call-waiting-banner",
    "call-window-held",
    "call-window-held-third-call-waiting",
    "call-window-calls-off-after",
    "call-pill",
    "call-pill-calls-off-after",
    "call-pill-muted-calls-off-after",
    "call-window-calls-off-after-long-number",
    "call-pill-calls-off-after-long-number",
    "call-pill-muted-long-number",
    "call-pill-held-calls-off-after",
    "call-pill-muted",
    "call-pill-audio-on-phone",
    "call-pill-on-your-phone",
    "call-pill-held",
    "call-pill-ended",
    "dialer-call-history",
    "dialer-new-missed-calls",
    "dialer-results",
    "dialer-no-matches",
    "dialer-not-connected",
    "dialer-calls-off",
    "dialer-withheld-missed-call",
    "dialer-call-in-progress",
    "dialer-undialable",
    "call-window-audio-not-moved",
    "call-window-phone-kept-audio",
    "quiet-hours-window-off",
    "quiet-hours-window-on",
    "quiet-hours-window-on-switch-focused",
    "quiet-hours-window-refused",
    "setup-window-choose",
    "setup-window-confirm",
    "setup-window-phone-calls-off",
    "setup-window-contacts-prompt",
    "setup-window-contacts-syncing",
    "setup-window-done",
    "setup-window-contacts-prompt-timed-out",
    "setup-window-done-contacts-denied",
    "setup-window-keybinding",
    "setup-window-calls-off",
    "setup-window-calls-off-bluetooth-off",
    "setup-problem-bluetooth-off",
    "setup-problem-bluetooth-blocked",
    "setup-problem-contacts-support-missing",
    "setup-problem-audio-not-set-up",
    "setup-problem-bluez-hands-free",
    "setup-window-opened-set-up",
    "setup-window-opened-keybinding-unreadable",
    "setup-window-opened-contacts-denied",
    "setup-window-opened-calls-off-bluetooth-off",
];

/// The environment variable `name`, unless it is unset or empty.
fn set(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

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

    // CI renders the gallery in parts, one per runner (#286): GALLERY_SHARD
    // is K/N, and this part holds every N-th of the gallery's states from
    // the K-th, in the order `--list-states` prints.
    let shard = set("GALLERY_SHARD");
    let listed = Command::new(binary)
        .args(["screenshot", "--list-states"])
        .output()
        .expect("run automation screenshot --list-states");
    let listed = String::from_utf8(listed.stdout).expect("the states' names are text");
    let order: Vec<&str> = listed.lines().collect();
    for state in STATES {
        assert!(
            order.contains(state),
            "the gallery lists {state}: {order:?}"
        );
    }
    let in_this_part = |state: &str| {
        let Some(shard) = &shard else { return true };
        let (number, count) = shard.split_once('/').expect("GALLERY_SHARD is K/N");
        let (number, count): (usize, usize) = (
            number.parse().expect("K is a number"),
            count.parse().expect("N is a number"),
        );
        let position = order.iter().position(|listed| *listed == state);
        position.is_some_and(|position| position % count == number - 1)
    };

    // A PR's CI renders one look, GALLERY_LOOK, enough to catch a broken
    // surface; the run on main renders every look.
    let look = set("GALLERY_LOOK");

    let mut command = Command::new(binary);
    command.arg("screenshot");
    for (flag, value) in [("--shard", &shard), ("--look", &look)] {
        command.args(value.iter().flat_map(|value| [flag, value]));
    }
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
    let files = STATES
        .iter()
        .filter(|state| in_this_part(state))
        .flat_map(|state| {
            ["light", "dark", "omarchy-light", "omarchy-dark"]
                .into_iter()
                .filter(|each| look.as_deref().is_none_or(|only| only == *each))
                .map(move |look| format!("{state}-{look}.png"))
        });
    for file in files {
        let file = file.as_str();
        assert!(index.contains(file), "the index shows {file}: {index}");
        let png = std::fs::read(output.join(file)).expect("the screenshot exists");
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"), "{file} is a PNG");
        // Width and height from the PNG header: the 3840 by 2160 output.
        assert_eq!(&png[16..24], &[0, 0, 0x0f, 0, 0, 0, 0x08, 0x70], "{file}");
    }

    indicator_icons_are_drawn(&output, &index);
    indicator_menus_are_drawn(&output, &index);
    missed_call_notifications_are_drawn(&output, &index);
}

/// The Indicator in the gallery at `output`, whose page is `index`, drawn
/// as Omarchy's bar draws it: a strip of bar 48 by 26 logical pixels at
/// 2x, each state's icon different.
fn indicator_icons_are_drawn(output: &Path, index: &str) {
    let mut icons = Vec::new();
    for state in [
        "connected",
        "missed-calls",
        "missed-calls-out-of-range",
        "disconnected",
        "ringing",
        "in-call",
        "problem",
        "off",
    ] {
        for look in ["omarchy-light", "omarchy-dark"] {
            let file = format!("indicator-{state}-{look}.png");
            assert!(index.contains(&file), "the index shows {file}: {index}");
            let png = std::fs::read(output.join(&file))
                .unwrap_or_else(|error| panic!("{file} is drawn: {error}"));
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
    for state in ["off-until-morning", "off-quiet-hours", "calls-off-pending"] {
        for look in ["omarchy-light", "omarchy-dark"] {
            let file = format!("indicator-{state}-{look}.png");
            assert!(index.contains(&file), "the index shows {file}: {index}");
        }
    }
}

/// The Missed Call notification in the gallery at `output`, whose page is
/// `index`: a stand-in for Omarchy's card, single and grouped, light and
/// dark, captioned as one. Omarchy's card is 380 logical pixels wide, and
/// with a body and a bar of buttons, 2 + 10 + 40 + 10 + 35 + 2 high.
fn missed_call_notifications_are_drawn(output: &Path, index: &str) {
    for state in ["missed-call", "missed-calls-grouped"] {
        for look in ["omarchy-light", "omarchy-dark"] {
            let file = format!("notification-{state}-{look}.png");
            assert!(index.contains(&file), "the index shows {file}: {index}");
            let png = std::fs::read(output.join(&file))
                .unwrap_or_else(|error| panic!("{file} is drawn: {error}"));
            assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"), "{file} is a PNG");
            assert_eq!(
                png[16..20],
                760_u32.to_be_bytes(),
                "{file} is 380 wide at 2x"
            );
            assert_eq!(
                png[20..24],
                198_u32.to_be_bytes(),
                "{file} is 99 high at 2x"
            );
        }
    }
    assert_eq!(
        index
            .matches("Stand-in: drawn from the notification the app sends")
            .count(),
        4,
        "each notification's caption says it is a stand-in"
    );
}

/// The Indicator's menu in the gallery at `output`, whose page is `index`:
/// a stand-in for Omarchy's, in four states, light and dark, captioned as
/// one. Omarchy's menu is 248 logical pixels wide with its padding, and
/// each state's menu is as tall as its entries: 8 of padding above and
/// below, 30 for each row and 11 for each separator.
fn indicator_menus_are_drawn(output: &Path, index: &str) {
    for (state, rows, separators) in [
        // Status, New call…, Turn calls off, Quiet hours…, then setup.
        ("menu-calls-on", 5, 1),
        // As calls on, with the Missed Calls' line under the status.
        ("menu-missed-calls", 6, 1),
        // The header, its line, and the three choices.
        ("menu-turn-off-choices", 4, 1),
        // As calls on, with Turn calls on, then the Problem.
        ("menu-off-problem", 6, 1),
        // With Mute, Move audio and Hang up, and Keep calls on.
        ("menu-calls-off-pending", 8, 1),
    ] {
        let height: u32 = 2 * (8 + 30 * rows + 11 * separators + 8);
        for look in ["omarchy-light", "omarchy-dark"] {
            let file = format!("indicator-{state}-{look}.png");
            assert!(index.contains(&file), "the index shows {file}: {index}");
            let png = std::fs::read(output.join(&file))
                .unwrap_or_else(|error| panic!("{file} is drawn: {error}"));
            assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"), "{file} is a PNG");
            assert_eq!(
                png[16..20],
                496_u32.to_be_bytes(),
                "{file} is 248 wide at 2x"
            );
            assert_eq!(png[20..24], height.to_be_bytes(), "{file} is {height} high");
        }
    }
    assert_eq!(
        index
            .matches("Stand-in: drawn from the menu layout the app publishes")
            .count(),
        10,
        "each menu's caption says it is a stand-in"
    );
}
