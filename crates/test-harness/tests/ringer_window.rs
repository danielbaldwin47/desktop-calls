//! Scenarios: the Ringer Window (issue #432), opened from the Indicator's
//! Ringer… entry, `desktop-calls ringer` or Alt+R in the Dialer, and worked
//! with synthetic keys. Choosing a Ring Sound saves it and plays it once
//! through the player stand-in (`pw-play`); the volume plays it again once
//! set. Each change is kept across a restart, and the next Call rings with
//! it. A file of the user's own that is gone, that `pw-play` can't play, or
//! that has no sound in it, rings the Chime instead, with a notice in the
//! window and a line in the log; so does a config file that doesn't read,
//! and a volume out of range in it is brought into range. A Call is never
//! silent because of the Ringer's settings. Calls off and Quiet Hours
//! don't stop a preview: it is the user asking to hear it.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use test_harness::{AppState, Harness, Key, MenuEntry, Play};

/// How long the app may take to notice a change.
const DEADLINE: Duration = Duration::from_secs(5);
/// How long a Ring Sound may take to start playing: written out first.
const PLAY_DEADLINE: Duration = Duration::from_secs(5);
/// How long a preview of a built-in sound may take to play to its end.
const PREVIEW_ENDS: Duration = Duration::from_secs(8);
/// How long the player may take to stop once told to.
const STOP_DEADLINE: Duration = Duration::from_secs(2);
const PHONE_NAME: &str = "My Pixel";
const NAME: &str = "ringer-window";
const NUMBER: &str = "+447700900123";

/// The Phone connected, the Ringer as it starts.
fn connected() -> Harness {
    let harness = Harness::builder()
        .headless_screen()
        .phone_name(PHONE_NAME)
        .start();
    harness.simulated_phone().connect();
    wait_connected(&harness);
    harness
}

fn wait_connected(harness: &Harness) {
    harness.wait_for_app("the Phone is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
}

fn window(state: &AppState) -> Option<&BTreeMap<String, String>> {
    state.surface(NAME)
}

fn field<'state>(state: &'state AppState, name: &str) -> Option<&'state str> {
    window(state)?.get(name).map(String::as_str)
}

/// Waits until the window shows each of `fields`, with keyboard focus.
fn wait_for_window(harness: &Harness, fields: &[(&str, &str)]) -> AppState {
    harness.wait_for_app(
        &format!("the Ringer Window shows {fields:?}, focused"),
        DEADLINE,
        |state| {
            field(state, "focused") == Some("true")
                && fields
                    .iter()
                    .all(|(name, value)| field(state, name) == Some(*value))
        },
    )
}

/// Chooses Ringer… in the Indicator's menu, once the menu offers it.
fn open_from_menu(harness: &Harness) {
    let ringer = |entry: &&MenuEntry| entry.label == "Ringer…";
    let menu = harness
        .tray()
        .wait_for_menu("the menu has Ringer…", DEADLINE, |menu| {
            menu.iter().any(|entry| ringer(&entry))
        });
    let entry = menu
        .iter()
        .find(ringer)
        .unwrap_or_else(|| panic!("the menu has Ringer…: {menu:#?}"));
    harness.tray().click(&entry.label);
}

/// Opens the window from the command line, as `desktop-calls ringer` does.
fn open(harness: &Harness) {
    let output = harness.cli(&["ringer"]);
    assert!(output.status.success(), "desktop-calls ringer: {output:?}");
}

/// Moves keyboard focus forwards `times` times with Tab, and waits until
/// it is on `focus`.
fn tab_to(harness: &Harness, times: usize, focus: &str) {
    for _ in 0..times {
        harness.keyboard().press(Key::Tab);
    }
    wait_for_window(harness, &[("focus", focus)]);
}

/// The Ringer's config file.
fn config(harness: &Harness) -> PathBuf {
    harness.directory().join("config/desktop-calls/ringer")
}

fn kept(harness: &Harness) -> String {
    fs::read_to_string(config(harness)).unwrap_or_default()
}

fn file_name(play: &Play) -> &str {
    play.file
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
}

/// Rings the Desktop with one Call, and returns its id.
fn ring(harness: &Harness) -> u32 {
    let _call = harness.simulated_phone().ring(NUMBER);
    harness
        .wait_for_app("the Desktop rings with one Call", DEADLINE, |state| {
            state.ringing && state.calls.len() == 1
        })
        .calls[0]
        .id
}

/// Declines the Call ringing, and waits until nothing plays.
fn decline(harness: &Harness, call: u32) {
    harness.decline(call);
    harness.wait_for_app("the Desktop stops ringing", DEADLINE, |state| {
        !state.ringing && state.calls.is_empty()
    });
    harness.wait_until_nothing_plays(STOP_DEADLINE);
}

/// A copy of one of Android's ringtones, as a file of the user's own.
fn own_file(harness: &Harness, name: &str) -> PathBuf {
    let music = harness.directory().join("home/Music");
    fs::create_dir_all(&music).unwrap_or_else(|error| panic!("create a music folder: {error}"));
    let path = music.join(name);
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/ringtones/android/Luna.ogg");
    fs::copy(&source, &path).unwrap_or_else(|error| panic!("copy {}: {error}", source.display()));
    path
}

/// Writes the Ringer's config file by hand, and restarts the app to read it.
fn hand_edit(harness: &mut Harness, contents: &str) {
    let path = config(harness);
    fs::create_dir_all(path.parent().unwrap_or_else(|| panic!("a folder")))
        .unwrap_or_else(|error| panic!("create the config folder: {error}"));
    fs::write(&path, contents).unwrap_or_else(|error| panic!("write the config file: {error}"));
    harness.restart_app();
    wait_connected(harness);
}

/// A 16-bit mono WAV file of `frames` frames of silence at 48 kHz.
fn wav(frames: u32) -> Vec<u8> {
    let data = frames * 2;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&48_000_u32.to_le_bytes());
    bytes.extend_from_slice(&96_000_u32.to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    bytes.resize(bytes.len() + data as usize, 0);
    bytes
}

fn choose_file(harness: &Harness, path: &Path) -> String {
    let path = path.to_str().unwrap_or_else(|| panic!("a UTF-8 path"));
    let output = harness.cli(&["ringer", path]);
    assert!(
        output.status.success(),
        "desktop-calls ringer FILE: {output:?}"
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

#[test]
fn opened_from_the_indicator_it_offers_the_sounds_in_a_few_words_and_escape_closes_it() {
    let harness = connected();
    open_from_menu(&harness);

    let state = wait_for_window(
        &harness,
        &[
            ("sound", "chime"),
            ("volume", "60 %"),
            ("play", "Play"),
            ("focus", "chime"),
        ],
    );
    assert_eq!(
        field(&state, "built-in"),
        Some("Chime\nBells\nGlass\nHarp\nPurr\nDrops")
    );
    assert_eq!(
        field(&state, "android"),
        Some("Atria\nGanymede\nLuna\nOberon\nTriton\nUmbriel")
    );
    assert_eq!(field(&state, "keys"), Some("Space Choose\nEsc Close"));
    assert_eq!(field(&state, "file"), None, "no file of the user's own yet");
    assert_eq!(field(&state, "notice"), None);
    assert!(harness.plays().is_empty(), "opening plays nothing");

    harness.keyboard().press(Key::Escape);
    harness.wait_for_app("the Ringer Window is closed", DEADLINE, |state| {
        window(state).is_none()
    });
    assert!(!config(&harness).exists(), "nothing changed, nothing saved");

    // Alt+R in the Dialer opens it too, and closes the Dialer.
    let output = harness.cli(&["dialer"]);
    assert!(output.status.success(), "desktop-calls dialer: {output:?}");
    harness.wait_for_app("the Dialer has the keyboard", DEADLINE, |state| {
        state
            .surface("dialer")
            .and_then(|dialer| dialer.get("focused"))
            .is_some_and(|focused| focused == "true")
    });
    harness.keyboard().press_with_alt('r');
    let state = wait_for_window(&harness, &[("focus", "chime")]);
    assert!(state.surface("dialer").is_none(), "the Dialer closed");
}

#[test]
fn choosing_a_sound_plays_it_once_saves_it_and_the_next_call_rings_with_it_after_a_restart() {
    let mut harness = connected();
    open(&harness);
    wait_for_window(&harness, &[("focus", "chime")]);

    // Tab to Bells; Space chooses it, saves it and plays it once.
    tab_to(&harness, 1, "bells");
    harness.keyboard().type_text(" ");
    wait_for_window(&harness, &[("sound", "bells"), ("play", "Stop")]);
    let plays = harness.wait_for_plays(1, PLAY_DEADLINE);
    assert_eq!(file_name(&plays[0]), "bells.wav", "{plays:?}");
    assert_eq!(plays[0].volume, Some(0.6), "{plays:?}");
    assert_eq!(kept(&harness), "sound = bells\nvolume = 60\n");

    // Once it has played, Play plays it again.
    let plays = harness.wait_until_nothing_plays(PREVIEW_ENDS);
    assert!(plays[0].finished.is_some(), "played to its end: {plays:?}");
    wait_for_window(&harness, &[("play", "Play")]);
    assert_eq!(harness.plays().len(), 1, "once, not over and over");

    harness.keyboard().press(Key::Escape);
    harness.wait_for_app("the Ringer Window is closed", DEADLINE, |state| {
        window(state).is_none()
    });

    harness.restart_app();
    wait_connected(&harness);
    let call = ring(&harness);
    let plays = harness.wait_for_plays(2, PLAY_DEADLINE);
    assert_eq!(file_name(&plays[1]), "bells-first.wav", "{plays:?}");
    assert_eq!(plays[1].volume, Some(0.6), "{plays:?}");
    decline(&harness, call);
}

#[test]
fn escape_stops_the_sound_playing_and_a_call_that_starts_ringing_stops_it_too() {
    let harness = connected();
    open(&harness);
    wait_for_window(&harness, &[("focus", "chime")]);

    // Atria, Android's, plays for 13.5 s, at the Ringer's volume less its
    // gain.
    tab_to(&harness, 6, "atria");
    harness.keyboard().type_text(" ");
    wait_for_window(
        &harness,
        &[
            ("sound", "atria"),
            ("play", "Stop"),
            ("keys", "Space Choose\nEsc Stop"),
        ],
    );
    let plays = harness.wait_for_plays(1, PLAY_DEADLINE);
    assert_eq!(file_name(&plays[0]), "atria.ogg", "{plays:?}");
    assert_eq!(
        plays[0].volume,
        Some(0.552),
        "60 % less Atria's gain: {plays:?}"
    );

    // Escape stops it, and leaves the window open.
    harness.keyboard().press(Key::Escape);
    let plays = harness.wait_until_nothing_plays(STOP_DEADLINE);
    assert!(plays[0].finished.is_none(), "stopped mid-sound: {plays:?}");
    wait_for_window(
        &harness,
        &[("play", "Play"), ("keys", "Space Choose\nEsc Close")],
    );

    // Chosen again, it plays again, until a Call rings: the ring stops it
    // and plays in its place, one Android ringtone after another.
    harness.keyboard().type_text(" ");
    harness.wait_for_plays(2, PLAY_DEADLINE);
    let call = ring(&harness);
    let plays = harness.wait_for_plays(3, PLAY_DEADLINE);
    assert!(
        plays[1].finished.is_none(),
        "the preview stopped: {plays:?}"
    );
    assert_eq!(file_name(&plays[2]), "atria.ogg", "{plays:?}");
    harness.wait_for_app("the window shows nothing playing", DEADLINE, |state| {
        field(state, "play") == Some("Play")
    });

    // While it rings, a choice is saved and plays nothing.
    tab_to(&harness, 1, "ganymede");
    harness.keyboard().type_text(" ");
    wait_for_window(&harness, &[("sound", "ganymede")]);
    assert_eq!(harness.plays().len(), 3, "nothing plays over the ring");
    decline(&harness, call);
}

#[test]
fn letting_the_volume_rest_saves_it_and_plays_the_sound_at_it() {
    let harness = connected();
    open(&harness);
    wait_for_window(&harness, &[("focus", "chime")]);

    // Shift+Tab goes round, past Play, to the volume.
    harness.keyboard().press_with_shift(Key::Tab);
    wait_for_window(&harness, &[("focus", "play")]);
    harness.keyboard().press_with_shift(Key::Tab);
    wait_for_window(&harness, &[("focus", "volume")]);
    harness.keyboard().press(Key::Left);
    harness.keyboard().press(Key::Left);

    wait_for_window(&harness, &[("volume", "50 %"), ("play", "Stop")]);
    let plays = harness.wait_for_plays(1, PLAY_DEADLINE);
    assert_eq!(plays.len(), 1, "played once the keys rested: {plays:?}");
    assert_eq!(file_name(&plays[0]), "chime.wav", "{plays:?}");
    assert_eq!(plays[0].volume, Some(0.5), "{plays:?}");
    assert_eq!(kept(&harness), "sound = chime\nvolume = 50\n");
}

#[test]
fn every_built_in_sound_is_about_as_loud_as_the_chime() {
    let harness = connected();
    open(&harness);
    wait_for_window(&harness, &[("focus", "chime")]);
    let built_in = ["chime", "bells", "glass", "harp", "purr", "drops"];
    for (index, sound) in built_in.iter().enumerate() {
        if index > 0 {
            tab_to(&harness, 1, sound);
        }
        harness.keyboard().type_text(" ");
        harness.wait_for_plays(index + 1, PLAY_DEADLINE);
    }

    let plays = harness.plays();
    let chime = plays[0].loudness();
    // The Chime is -17.6 LUFS in the file, by ffmpeg's ebur128, which
    // weights its notes up a little: -19.6 dB here.
    assert!(
        (-20.5..=-18.5).contains(&chime),
        "the Chime is soft: {chime:.1} dB"
    );
    for play in &plays {
        let (loudness, peak) = (play.loudness(), play.peak());
        assert!(
            (loudness - chime).abs() <= 1.0,
            "{} is {loudness:.1} dB, the Chime {chime:.1} dB",
            file_name(play)
        );
        assert!(
            peak <= 0.5,
            "{} peaks under -6 dBFS: {peak:.2}",
            file_name(play)
        );
    }
}

#[test]
fn a_file_of_your_own_rings_and_once_gone_the_chime_rings_with_a_notice() {
    let harness = connected();
    let path = own_file(&harness, "my song.ogg");

    assert_eq!(choose_file(&harness, &path), "Calls ring with my song.ogg.");
    harness.wait_for_app_log("\"ringer-set\"", DEADLINE);
    assert!(
        kept(&harness).contains("sound = file\n"),
        "{}",
        kept(&harness)
    );
    open(&harness);
    wait_for_window(&harness, &[("sound", "file"), ("file", "my song.ogg")]);

    let call = ring(&harness);
    let plays = harness.wait_for_plays(1, PLAY_DEADLINE);
    assert_eq!(plays[0].file, path, "{plays:?}");
    assert_eq!(plays[0].volume, Some(0.6), "{plays:?}");
    decline(&harness, call);

    fs::remove_file(&path).unwrap_or_else(|error| panic!("remove the file: {error}"));
    let call = ring(&harness);
    let plays = harness.wait_for_plays(2, PLAY_DEADLINE);
    assert_eq!(file_name(&plays[1]), "chime-first.wav", "{plays:?}");
    harness.wait_for_app_log("\"ring-sound-unplayable\"", DEADLINE);
    let state = harness.wait_for_app("the notice shows", DEADLINE, |state| {
        field(state, "notice").is_some()
    });
    assert_eq!(
        field(&state, "notice"),
        Some("Can't play this file\nChime rings instead")
    );
    decline(&harness, call);

    // Choosing a sound of the app's own takes the notice away.
    harness.keyboard().press(Key::Tab);
    wait_for_window(&harness, &[("focus", "choose-file")]);
    harness.keyboard().press_with_shift(Key::Tab);
    wait_for_window(&harness, &[("focus", "file")]);
    harness.keyboard().press_with_shift(Key::Tab);
    wait_for_window(&harness, &[("focus", "umbriel")]);
    harness.keyboard().type_text(" ");
    let state = wait_for_window(&harness, &[("sound", "umbriel")]);
    assert_eq!(field(&state, "notice"), None);
    assert_eq!(field(&state, "file"), Some("my song.ogg"), "still offered");
}

#[test]
fn a_file_pw_play_cant_play_rings_the_chime_instead() {
    let harness = connected();
    let music = harness.directory().join("home/Music");
    fs::create_dir_all(&music).unwrap_or_else(|error| panic!("create a music folder: {error}"));
    let path = music.join("not a sound.ogg");
    fs::write(&path, "not a sound").unwrap_or_else(|error| panic!("write the file: {error}"));
    choose_file(&harness, &path);
    harness.wait_for_app_log("\"ringer-set\"", DEADLINE);

    let call = ring(&harness);
    let plays = harness.wait_for_plays(1, PLAY_DEADLINE);
    assert_eq!(file_name(&plays[0]), "chime-first.wav", "{plays:?}");
    harness.wait_for_plays(2, Duration::from_secs(10));
    harness.wait_for_app_log("\"ring-sound-unplayable\"", DEADLINE);
    decline(&harness, call);
}

#[test]
fn a_file_with_no_sound_in_it_rings_the_chime_instead_with_the_notice() {
    let harness = connected();
    let music = harness.directory().join("home/Music");
    fs::create_dir_all(&music).unwrap_or_else(|error| panic!("create a music folder: {error}"));
    // One with no frames at all, and one a few milliseconds long: each
    // "plays" in next to no time, and would leave a Call silent.
    for (name, frames) in [("empty.wav", 0), ("click.wav", 240)] {
        let path = music.join(name);
        fs::write(&path, wav(frames)).unwrap_or_else(|error| panic!("write {name}: {error}"));
        choose_file(&harness, &path);
        harness.wait_for_app_log("\"ringer-set\"", DEADLINE);

        let before = harness.plays().len();
        let call = ring(&harness);
        let plays = harness.wait_for_plays(before + 2, PLAY_DEADLINE);
        assert_eq!(plays[before].file, path, "tried first: {plays:?}");
        assert_eq!(
            file_name(&plays[before + 1]),
            "chime-first.wav",
            "the Chime at once: {plays:?}"
        );
        decline(&harness, call);
    }
    harness.wait_for_app_log("next to no sound in it", DEADLINE);

    // The window says so too, once a preview finds it.
    open(&harness);
    wait_for_window(
        &harness,
        &[("sound", "file"), ("file", "click.wav"), ("focus", "file")],
    );
    harness.keyboard().type_text(" ");
    let state = harness.wait_for_app("the notice shows", DEADLINE, |state| {
        field(state, "notice").is_some()
    });
    assert_eq!(
        field(&state, "notice"),
        Some("Can't play this file\nChime rings instead")
    );
    let plays = harness.wait_until_nothing_plays(PREVIEW_ENDS);
    assert_eq!(file_name(&plays[plays.len() - 1]), "chime.wav", "{plays:?}");
}

#[test]
fn a_preview_plays_while_calls_are_off_and_during_quiet_hours() {
    let harness = Harness::builder()
        .headless_screen()
        .phone_name(PHONE_NAME)
        .time_zone("UTC0")
        .start();
    harness.simulated_phone().connect();
    wait_connected(&harness);

    let output = harness.cli(&["calls", "off"]);
    assert!(output.status.success(), "{output:?}");
    harness.wait_for_app("calls are off", DEADLINE, |state| state.calls_off == "off");
    open(&harness);
    wait_for_window(&harness, &[("focus", "chime")]);
    tab_to(&harness, 1, "bells");
    harness.keyboard().type_text(" ");
    let plays = harness.wait_for_plays(1, PLAY_DEADLINE);
    assert_eq!(file_name(&plays[0]), "bells.wav", "{plays:?}");
    harness.wait_until_nothing_plays(PREVIEW_ENDS);

    // Quiet Hours from an hour ago to an hour from now.
    let output = harness.cli(&["calls", "on"]);
    assert!(output.status.success(), "{output:?}");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_else(|error| panic!("the clock is before 1970: {error}"))
        .as_secs()
        / 60
        % 1_440;
    let clock = |minutes: u64| format!("{:02}:{:02}", minutes / 60 % 24, minutes % 60);
    let hours = format!("{}-{}", clock(now + 1_380), clock(now + 60));
    let output = harness.cli(&["quiet-hours", &hours]);
    assert!(output.status.success(), "{output:?}");
    harness.wait_for_app("Quiet Hours turn calls off", DEADLINE, |state| {
        state.calls_off != "on"
    });
    tab_to(&harness, 1, "glass");
    harness.keyboard().type_text(" ");
    let plays = harness.wait_for_plays(2, PLAY_DEADLINE);
    assert_eq!(file_name(&plays[1]), "glass.wav", "{plays:?}");
}

#[test]
fn a_hand_edited_volume_out_of_range_is_brought_into_range() {
    let mut harness = connected();
    for (written, shown, played) in [("250", "100 %", 1.0), ("3", "10 %", 0.1)] {
        hand_edit(
            &mut harness,
            &format!("sound = chime\nvolume = {written}\n"),
        );
        open(&harness);
        wait_for_window(&harness, &[("volume", shown)]);
        let before = harness.plays().len();
        let call = ring(&harness);
        let plays = harness.wait_for_plays(before + 1, PLAY_DEADLINE);
        assert_eq!(plays[before].volume, Some(played), "{written}: {plays:?}");
        decline(&harness, call);
    }
}

#[test]
fn the_command_line_refuses_a_file_that_isnt_there() {
    let harness = connected();
    let output = harness.cli(&["ringer", "/nowhere/ring.ogg"]);
    assert!(!output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stderr).trim(),
        "No file there. Check the path."
    );
    assert!(!config(&harness).exists(), "nothing saved");
}

#[test]
fn a_config_file_that_doesnt_read_rings_the_chime_at_60_percent_and_is_logged() {
    let mut harness = connected();
    hand_edit(&mut harness, "sound = kazoo\nvolume = loud\n");
    let failed = harness.wait_for_app_log("\"ringer-failed\"", DEADLINE);
    assert!(failed.contains("ringer-unknown-sound"), "{failed}");

    let call = ring(&harness);
    let plays = harness.wait_for_plays(1, PLAY_DEADLINE);
    assert_eq!(file_name(&plays[0]), "chime-first.wav", "{plays:?}");
    assert_eq!(plays[0].volume, Some(0.6), "{plays:?}");
    decline(&harness, call);
}
