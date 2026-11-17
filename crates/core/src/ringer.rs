//! The Ringer's settings (issue #432): which Ring Sound rings, at what
//! volume, and the user's own file, as the Ringer Window sets them and the
//! config file keeps them.
//!
//! The file is one `key = value` line per setting, readable and writable by
//! hand:
//!
//! ```text
//! sound = chime
//! volume = 60
//! file = /home/user/Music/ring.ogg
//! ```
//!
//! A line that can't be read falls back to the default for that setting
//! alone and is named, for the shell to log: a Call is never silent because
//! of the file.

use std::fmt;

/// One sound the Ringer can play.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RingSound {
    /// The default: a soft four-note chime, like a marimba.
    Chime,
    /// Two soft bell strokes, falling.
    Bells,
    /// Three clear notes rising, like a glass struck.
    Glass,
    /// A quick plucked arpeggio.
    Harp,
    /// A low, soft double trill, like an old phone, muffled.
    Purr,
    /// Water drops.
    Drops,
    /// Android's ringtones of the same names, from the Android Open Source
    /// Project, bundled under the Apache License 2.0.
    Atria,
    Ganymede,
    Luna,
    Oberon,
    Triton,
    Umbriel,
    /// The user's own audio file.
    File,
}

impl RingSound {
    /// The sounds Desktop Calls makes itself, the default first.
    pub const BUILT_IN: [Self; 6] = [
        Self::Chime,
        Self::Bells,
        Self::Glass,
        Self::Harp,
        Self::Purr,
        Self::Drops,
    ];
    /// Android's ringtones, bundled.
    pub const ANDROID: [Self; 6] = [
        Self::Atria,
        Self::Ganymede,
        Self::Luna,
        Self::Oberon,
        Self::Triton,
        Self::Umbriel,
    ];
    /// Every Ring Sound.
    pub const ALL: [Self; 13] = [
        Self::Chime,
        Self::Bells,
        Self::Glass,
        Self::Harp,
        Self::Purr,
        Self::Drops,
        Self::Atria,
        Self::Ganymede,
        Self::Luna,
        Self::Oberon,
        Self::Triton,
        Self::Umbriel,
        Self::File,
    ];

    /// Its name in the config file, the control interface and the logs.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::Chime => "chime",
            Self::Bells => "bells",
            Self::Glass => "glass",
            Self::Harp => "harp",
            Self::Purr => "purr",
            Self::Drops => "drops",
            Self::Atria => "atria",
            Self::Ganymede => "ganymede",
            Self::Luna => "luna",
            Self::Oberon => "oberon",
            Self::Triton => "triton",
            Self::Umbriel => "umbriel",
            Self::File => "file",
        }
    }

    /// The Ring Sound `key` names, if any.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|sound| sound.key() == key)
    }
}

/// The volume the Ringer plays at by default, in percent of full stream
/// volume, under the output's own.
pub const DEFAULT_VOLUME: u8 = 60;
/// The least volume the Ringer plays at: low, but never silent.
pub const LEAST_VOLUME: u8 = 10;

/// What the Ringer plays, and how loud.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RingerSettings {
    sound: RingSound,
    volume: u8,
    file: Option<String>,
}

impl Default for RingerSettings {
    /// The Chime at 60 %, with no file.
    fn default() -> Self {
        Self {
            sound: RingSound::Chime,
            volume: DEFAULT_VOLUME,
            file: None,
        }
    }
}

/// A line of the config file that couldn't be read, and was left at its
/// default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RingerSettingsProblem {
    /// `sound =` names no Ring Sound.
    UnknownSound(String),
    /// `volume =` isn't a whole number.
    UnreadableVolume(String),
    /// `sound = file` with no `file =` line.
    NoFile,
    /// A line that isn't `sound`, `volume` or `file`.
    UnknownLine(String),
}

impl RingerSettingsProblem {
    /// The kind logs carry as `error_kind`.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::UnknownSound(_) => "ringer-unknown-sound",
            Self::UnreadableVolume(_) => "ringer-unreadable-volume",
            Self::NoFile => "ringer-no-file",
            Self::UnknownLine(_) => "ringer-unknown-line",
        }
    }

    /// What was read, for the log.
    #[must_use]
    pub fn detail(&self) -> &str {
        match self {
            Self::UnknownSound(text) | Self::UnreadableVolume(text) | Self::UnknownLine(text) => {
                text
            }
            Self::NoFile => "sound = file, and no file line",
        }
    }
}

impl RingerSettings {
    /// The Ring Sound that rings: [`RingSound::File`] only with a file.
    #[must_use]
    pub fn sound(&self) -> RingSound {
        self.sound
    }

    /// The volume, in percent, from [`LEAST_VOLUME`] to 100.
    #[must_use]
    pub fn volume(&self) -> u8 {
        self.volume
    }

    /// The user's own file, chosen last: kept while another sound rings.
    #[must_use]
    pub fn file(&self) -> Option<&str> {
        self.file.as_deref()
    }

    /// With `sound` ringing; [`RingSound::File`] only once a file is chosen.
    #[must_use]
    pub fn with_sound(self, sound: RingSound) -> Self {
        if sound == RingSound::File && self.file.is_none() {
            return self;
        }
        Self { sound, ..self }
    }

    /// At `volume` percent, kept from [`LEAST_VOLUME`] to 100.
    #[must_use]
    pub fn with_volume(self, volume: u8) -> Self {
        Self {
            volume: volume.clamp(LEAST_VOLUME, 100),
            ..self
        }
    }

    /// Ringing with the user's own `file`.
    #[must_use]
    pub fn with_file(self, file: String) -> Self {
        Self {
            sound: RingSound::File,
            file: Some(file),
            ..self
        }
    }

    /// The settings the config file's `text` holds, and every line of it
    /// that couldn't be read, each left at its default.
    #[must_use]
    pub fn parse(text: &str) -> (Self, Vec<RingerSettingsProblem>) {
        let mut problems = Vec::new();
        let (mut sound, mut volume, mut file) = (None, DEFAULT_VOLUME, None);
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                problems.push(RingerSettingsProblem::UnknownLine(line.to_owned()));
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "sound" => match RingSound::from_key(value) {
                    Some(named) => sound = Some(named),
                    None => problems.push(RingerSettingsProblem::UnknownSound(value.to_owned())),
                },
                "volume" => match value.parse::<u16>() {
                    Ok(percent) => volume = u8::try_from(percent.min(100)).unwrap_or(100),
                    Err(_) => {
                        problems.push(RingerSettingsProblem::UnreadableVolume(value.to_owned()));
                    }
                },
                "file" if !value.is_empty() => file = Some(value.to_owned()),
                _ => problems.push(RingerSettingsProblem::UnknownLine(line.to_owned())),
            }
        }
        let mut settings = Self::default().with_volume(volume);
        if let Some(file) = file {
            settings = settings.with_file(file);
        }
        match sound {
            Some(RingSound::File) if settings.file.is_none() => {
                problems.push(RingerSettingsProblem::NoFile);
                settings.sound = RingSound::Chime;
            }
            Some(sound) => settings.sound = sound,
            None => settings.sound = RingSound::Chime,
        }
        (settings, problems)
    }
}

impl fmt::Display for RingerSettings {
    /// The config file's text: `sound` and `volume`, then `file` once one
    /// is chosen.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "sound = {}", self.sound.key())?;
        writeln!(f, "volume = {}", self.volume)?;
        if let Some(file) = &self.file {
            writeln!(f, "file = {file}")?;
        }
        Ok(())
    }
}
