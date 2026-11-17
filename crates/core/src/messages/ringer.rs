//! What the Ringer Window says: a few words per label, never a sentence,
//! so each reads at a glance (issue #432).

use crate::machine::RingerWindowView;
use crate::ringer::RingSound;

/// The Indicator's menu entry that opens the window.
pub(super) const MENU_ENTRY: &str = "Ringer…";

/// The Ringer Window's text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RingerWindowText {
    /// The window's title.
    pub title: &'static str,
    pub heading: &'static str,
    /// The small heading over the sounds Desktop Calls makes.
    pub built_in: &'static str,
    pub built_in_sounds: Vec<(RingSound, &'static str)>,
    /// The small heading over Android's ringtones.
    pub android: &'static str,
    pub android_sounds: Vec<(RingSound, &'static str)>,
    /// The small heading over the user's own file.
    pub own_file: &'static str,
    /// The file chosen last, by its name, once there is one.
    pub file: Option<String>,
    /// The button that opens the file chooser.
    pub choose_file: &'static str,
    /// The file chooser's title, and the name of its filter.
    pub file_chooser: &'static str,
    pub audio_files: &'static str,
    pub volume_heading: &'static str,
    /// The volume, such as "60 %".
    pub volume: String,
    /// The button that plays the sound chosen once, or stops it.
    pub play: &'static str,
    /// Why the file chosen doesn't ring, and what rings instead.
    pub notice: Option<&'static str>,
    pub notice_detail: Option<&'static str>,
    /// The keys that work, each with what it does, for the footer.
    pub keys: Vec<(&'static [&'static str], &'static str)>,
}

/// A Ring Sound's name, as the window and the command line show it.
#[must_use]
pub fn ring_sound_name(sound: RingSound) -> &'static str {
    match sound {
        RingSound::Chime => "Chime",
        RingSound::Bells => "Bells",
        RingSound::Glass => "Glass",
        RingSound::Harp => "Harp",
        RingSound::Purr => "Purr",
        RingSound::Drops => "Drops",
        RingSound::Atria => "Atria",
        RingSound::Ganymede => "Ganymede",
        RingSound::Luna => "Luna",
        RingSound::Oberon => "Oberon",
        RingSound::Triton => "Triton",
        RingSound::Umbriel => "Umbriel",
        RingSound::File => "Your file",
    }
}

/// What the Ringer Window says for `view`.
#[must_use]
pub fn ringer_window(view: &RingerWindowView) -> RingerWindowText {
    let named = |sounds: [RingSound; 6]| {
        sounds
            .into_iter()
            .map(|sound| (sound, ring_sound_name(sound)))
            .collect()
    };
    RingerWindowText {
        title: "Ringer",
        heading: "Ringer",
        built_in: "Built in",
        built_in_sounds: named(RingSound::BUILT_IN),
        android: "Android",
        android_sounds: named(RingSound::ANDROID),
        own_file: "Your file",
        file: view.settings.file().map(file_name),
        choose_file: "Choose a file…",
        file_chooser: "Choose a ring sound",
        audio_files: "Audio files",
        volume_heading: "Volume",
        volume: ringer_volume(view.settings.volume()),
        play: if view.previewing { "Stop" } else { "Play" },
        notice: view.unplayable.then_some("Can't play this file"),
        notice_detail: view.unplayable.then_some("Chime rings instead"),
        keys: vec![
            (&["Space"], "Choose"),
            (&["Esc"], if view.previewing { "Stop" } else { "Close" }),
        ],
    }
}

/// A Ringer volume, such as "60 %", as the window shows it, while the
/// slider moves too.
#[must_use]
pub fn ringer_volume(percent: u8) -> String {
    format!("{percent} %")
}

/// The last part of `path`: the file's own name.
fn file_name(path: &str) -> String {
    path.rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or(path)
        .to_owned()
}

/// What `desktop-calls ringer FILE` says once the file rings.
#[must_use]
pub fn ringer_file_set(path: &str) -> String {
    format!("Calls ring with {}.", file_name(path))
}

/// What `desktop-calls ringer FILE` says when there is no such file.
#[must_use]
pub fn ringer_file_missing() -> &'static str {
    "There's no file there. Check the path and try again."
}
