//! The Ringer Window (issue #432): the Ring Sound and its volume, opened
//! from the Indicator's Ringer… entry, `desktop-calls ringer` or Alt+R in
//! the Dialer, like the Quiet Hours Window. Each change is saved at once:
//! there is no confirm.
//!
//! - Choosing a sound saves it and plays it once; choosing it again plays
//!   it again. Letting go of the volume saves it and plays the sound at it.
//! - Stop, Escape and closing the window stop the sound; Escape with
//!   nothing playing closes the window.
//! - Nothing plays over the Ringer: while a Call rings here, choices are
//!   saved and play nothing, and a Call that starts ringing stops the
//!   sound playing.

use super::State;
use super::input::{Effect, Input};
use crate::ringer::{RingSound, RingerSettings};

/// The Ringer's settings, the window while open, and how many times it
/// has opened.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Ringer {
    settings: RingerSettings,
    open: Option<Window>,
    openings: u32,
    /// The user's own file the Ringer last couldn't play.
    unplayable: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Window {
    opened: u32,
    /// A sound plays once, as the window asked.
    previewing: bool,
}

/// What the Ringer Window shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RingerWindowView {
    /// What the Ringer plays, as saved.
    pub settings: RingerSettings,
    /// A sound plays once, as the window asked: Stop stops it.
    pub previewing: bool,
    /// The file chosen couldn't be played, so the Chime rings instead.
    pub unplayable: bool,
    /// Different each time the window is asked for, so the shell raises it.
    pub opened: u32,
}

impl State {
    /// What the Ringer plays.
    #[must_use]
    pub fn ringer(&self) -> &RingerSettings {
        &self.ringer.settings
    }

    /// What the Ringer Window shows, while it is open.
    pub(super) fn ringer_window_view(&self) -> Option<RingerWindowView> {
        let ringer = &self.ringer;
        let window = ringer.open.as_ref()?;
        Some(RingerWindowView {
            settings: ringer.settings.clone(),
            previewing: window.previewing,
            unplayable: ringer.settings.sound() == RingSound::File
                && ringer.unplayable.is_some()
                && ringer.unplayable.as_deref() == ringer.settings.file(),
            opened: window.opened,
        })
    }
}

/// What the window, its user and the Ringer report.
pub(super) fn ringer_input(state: &mut State, input: Input, effects: &mut Vec<Effect>) {
    let settings = state.ringer.settings.clone();
    match input {
        Input::RingerKept(kept) => state.ringer.settings = kept,
        Input::OpenRingerWindow => open(state),
        Input::CloseRingerWindow => {
            stop(state, effects);
            state.ringer.open = None;
        }
        Input::RingerWindowEscape => {
            if !stop(state, effects) {
                state.ringer.open = None;
            }
        }
        Input::ChooseRingSound(sound) => {
            let chosen = settings.clone().with_sound(sound);
            if chosen.sound() == sound {
                set(state, chosen, effects);
            }
        }
        Input::ChooseRingerFile(file) => set(state, settings.with_file(file), effects),
        Input::SetRingerVolume(volume) => set(state, settings.with_volume(volume), effects),
        Input::PreviewRingSound => preview(state, effects),
        Input::StopPreview => {
            stop(state, effects);
        }
        Input::PreviewEnded => {
            if let Some(window) = state.ringer.open.as_mut() {
                window.previewing = false;
            }
        }
        Input::RingSoundUnplayable { file } => state.ringer.unplayable = Some(file),
        _ => {}
    }
}

/// Saves `settings` if they changed, then plays them once.
fn set(state: &mut State, settings: RingerSettings, effects: &mut Vec<Effect>) {
    if settings.sound() == RingSound::File {
        // A fresh try: the Ringer says again if it still can't play it.
        state.ringer.unplayable = None;
    }
    if state.ringer.settings != settings {
        state.ringer.settings = settings.clone();
        effects.push(Effect::KeepRinger(settings));
    }
    preview(state, effects);
}

/// Plays the sound saved once, while the window is open and no Call rings.
fn preview(state: &mut State, effects: &mut Vec<Effect>) {
    if state.ringing() {
        return;
    }
    if let Some(window) = state.ringer.open.as_mut() {
        window.previewing = true;
        effects.push(Effect::PreviewRingSound(state.ringer.settings.clone()));
    }
}

/// Stops the sound playing, if one is: whether one was.
fn stop(state: &mut State, effects: &mut Vec<Effect>) -> bool {
    let Some(window) = state.ringer.open.as_mut() else {
        return false;
    };
    let previewing = std::mem::take(&mut window.previewing);
    if previewing {
        effects.push(Effect::StopPreview);
    }
    previewing
}

/// Opens the window, or raises it if it is open. The Dialer closes, so the
/// window has the keyboard, as for the Quiet Hours Window.
fn open(state: &mut State) {
    state.dialer = None;
    let windows = &mut state.ringer;
    windows.openings += 1;
    let opened = windows.openings;
    match windows.open.as_mut() {
        Some(window) => window.opened = opened,
        None => {
            windows.open = Some(Window {
                opened,
                previewing: false,
            });
        }
    }
}

/// After every input: a Call that starts ringing stops the sound playing.
pub(super) fn follow(state: &mut State, effects: &mut Vec<Effect>) {
    if state.ringing() {
        stop(state, effects);
    }
}
