//! The Ringer Window's states in the gallery: as it opens, the Chime
//! chosen; playing one of Android's ringtones once, Stop shown; and a file
//! of the user's own that can't be played, with its notice.

use std::fs;

use test_harness::{AppState, Harness, Key};

use super::{DEADLINE, State, app_region, connect};

const NAME: &str = "ringer-window";

pub(super) const RINGER_WINDOW_STATES: &[State] = &[
    State {
        name: "ringer-window",
        title: "The Ringer Window as it opens: the Chime chosen and focused, 60 %, no file of the user's own yet",
        still: |_| true,
        prepare: |builder| builder,
        moment: None,
        reach: |harness| {
            connect(harness);
            harness.tray().click("Ringer…");
            wait_for(harness, &[("sound", "chime"), ("focus", "chime")]);
        },
        shown: |screenshot, _| app_region(screenshot).is_some(),
    },
    State {
        name: "ringer-window-playing",
        title: "The Ringer Window playing Atria once, just chosen: Play reads Stop, and so does Esc",
        still: |_| true,
        prepare: |builder| builder,
        moment: None,
        reach: |harness| {
            connect(harness);
            harness.tray().click("Ringer…");
            wait_for(harness, &[("focus", "chime")]);
            for _ in 0..6 {
                harness.keyboard().press(Key::Tab);
            }
            wait_for(harness, &[("focus", "atria")]);
            harness.keyboard().type_text(" ");
            wait_for(harness, &[("sound", "atria"), ("play", "Stop")]);
        },
        shown: |screenshot, _| app_region(screenshot).is_some(),
    },
    State {
        name: "ringer-window-file-unplayable",
        title: "The Ringer Window with a file of the user's own that can't be played: the notice says the Chime rings instead",
        still: |_| true,
        prepare: |builder| builder,
        moment: None,
        reach: |harness| {
            connect(harness);
            harness.tray().click("Ringer…");
            wait_for(harness, &[("focus", "chime")]);
            let music = harness.directory().join("home/Music");
            fs::create_dir_all(&music)
                .unwrap_or_else(|error| panic!("create a music folder: {error}"));
            let file = music.join("Morning birds.ogg");
            fs::write(&file, "not a sound")
                .unwrap_or_else(|error| panic!("write the file: {error}"));
            let output = harness.cli(&[
                "ringer",
                file.to_str().unwrap_or_else(|| panic!("a UTF-8 path")),
            ]);
            assert!(
                output.status.success(),
                "desktop-calls ringer FILE: {output:?}"
            );
            wait_for(
                harness,
                &[
                    ("sound", "file"),
                    ("notice", "Can't play this file\nChime rings instead"),
                ],
            );
        },
        shown: |screenshot, _| app_region(screenshot).is_some(),
    },
];

fn shows(state: &AppState, key: &str, value: &str) -> bool {
    state
        .surface(NAME)
        .and_then(|window| window.get(key))
        .is_some_and(|shown| shown == value)
}

/// Waits until the window shows each of `fields`, with keyboard focus.
fn wait_for(harness: &Harness, fields: &[(&str, &str)]) {
    harness.wait_for_app(
        &format!("the Ringer Window shows {fields:?}"),
        DEADLINE,
        |state| {
            shows(state, "focused", "true")
                && fields.iter().all(|(key, value)| shows(state, key, value))
        },
    );
}
