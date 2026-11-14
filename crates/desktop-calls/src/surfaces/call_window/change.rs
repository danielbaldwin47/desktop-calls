//! The change between the Call Window and the Call Pill: the Call Window
//! grows out of the Call Pill's corner, and shrinks back into it, in 200 ms,
//! easing out. Both are anchored in the bottom-right corner, so the window
//! grows upwards and its controls stay where they will be. It starts from,
//! and ends at, the Call Pill's size, 320 by 44, with its content faded
//! out, so the Call Pill's own content takes over without a jump. Where the
//! Desktop asks for reduced motion (GTK's `gtk-enable-animations` false),
//! the change is a cut.
//!
//! The control interface reports the end of the change at once: the Call
//! Window as shown once it starts to grow, and the Call Pill as shown once
//! the window starts to shrink. The app logs each change's start and end.

use super::super::WIDTH;
use super::CallWindow;
use super::pill::PILL_WIDTH;
use crate::logging::EventKind;
use adw::prelude::AnimationExt as _;
use gtk::prelude::*;

/// How long the change takes, in milliseconds.
pub(super) const CHANGE_MS: u32 = 200;

/// The Call Window's content area at its smallest, so that with its padding
/// it starts to grow at the Call Pill's height, 44 px.
pub(super) const SMALLEST_CONTENT: i32 = 12;

/// The change under way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Change {
    /// The Call Window grows out of the Call Pill.
    Expanding,
    /// The Call Window shrinks into the Call Pill.
    Collapsing,
}

impl Change {
    /// The form the Call Window ends in, as logged.
    fn form(self) -> &'static str {
        match self {
            Self::Expanding => "expanded",
            Self::Collapsing => "collapsed",
        }
    }
}

impl CallWindow {
    /// Starts the Call Window growing out of the Call Pill, which is
    /// already gone. Shown, mid-shrink, it grows back from where it is.
    pub(super) fn start_expanding(&self) {
        self.started(Change::Expanding);
        if self.surface.window.is_visible() {
            self.surface.publish();
            self.fade(1.0);
            self.reveal.set_reveal_child(true);
        } else {
            // Unmapped, it closes at once: the window opens at the Call
            // Pill's size, empty, and grows once mapped.
            self.reveal.set_reveal_child(false);
            shape(&self.surface.window, &self.reveal, 0.0);
        }
    }

    /// The window has been mapped: it grows, if it is growing.
    pub(super) fn mapped(&self) {
        if self.change.get() == Some(Change::Expanding) {
            self.fade(1.0);
            self.reveal.set_reveal_child(true);
        }
    }

    /// Starts the Call Window shrinking into the Call Pill, which is
    /// reported as shown from now on.
    pub(super) fn start_collapsing(&self) {
        self.started(Change::Collapsing);
        self.surface.report_gone();
        // Faded first: with animations off, the shrink ends at once, and
        // the window, hidden, must be left ready to show in full.
        self.fade(0.0);
        self.reveal.set_reveal_child(false);
    }

    /// The window's content has finished growing (`revealed`) or
    /// shrinking. Shrunk, the window gives way to the Call Pill.
    pub(super) fn change_finished(&self, revealed: bool) {
        let ((Some(change @ Change::Expanding), true) | (Some(change @ Change::Collapsing), false)) =
            (self.change.get(), revealed)
        else {
            return;
        };
        self.change.set(None);
        tracing::info!(
            event_kind = %EventKind::CallWindowChangeFinished,
            form = change.form(),
            "The Call Window finished changing to {}",
            change.form()
        );
        if change == Change::Collapsing {
            self.surface.hide();
            self.reset_change();
            self.update();
        }
    }

    /// Drops any change under way, and leaves the window, hidden, ready to
    /// show in full.
    pub(super) fn reset_change(&self) {
        self.change.set(None);
        if let Some(fade) = self.fade.take() {
            fade.skip();
        }
        self.reveal.set_reveal_child(true);
        shape(&self.surface.window, &self.reveal, 1.0);
    }

    /// Fades the content to `to` (1 in full, 0 gone) and widens or narrows
    /// the window with it, from wherever the last fade left them.
    fn fade(&self, to: f64) {
        let from = self
            .fade
            .borrow()
            .as_ref()
            .map_or(1.0 - to, adw::prelude::AnimationExt::value);
        if let Some(fade) = self.fade.take() {
            fade.pause();
        }
        let (window, reveal) = (self.surface.window.clone(), self.reveal.clone());
        let target = adw::CallbackAnimationTarget::new(move |progress| {
            shape(&window, &reveal, progress);
        });
        let fade = adw::TimedAnimation::new(&self.surface.window, from, to, CHANGE_MS, target);
        fade.set_easing(adw::Easing::EaseOutCubic);
        // Skipped at once where animations are off, or the window isn't on
        // screen.
        fade.play();
        *self.fade.borrow_mut() = Some(fade);
    }

    fn started(&self, change: Change) {
        self.change.set(Some(change));
        let animated =
            gtk::Settings::default().is_none_or(|settings| settings.is_gtk_enable_animations());
        tracing::info!(
            event_kind = %EventKind::CallWindowChangeStarted,
            form = change.form(),
            animated,
            "The Call Window started changing to {}",
            change.form()
        );
    }
}

/// The window `progress` of the way from the Call Pill (0) to the Call
/// Window (1): as wide, and its content as opaque.
fn shape(window: &gtk::Window, reveal: &gtk::Revealer, progress: f64) {
    let progress = progress.clamp(0.0, 1.0);
    let width = f64::from(PILL_WIDTH) + f64::from(WIDTH - PILL_WIDTH) * progress;
    // Between 320 and 340, so it fits.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the width is between the Call Pill's and the Call Window's"
    )]
    window.set_default_size(width.round() as i32, -1);
    if let Some(content) = reveal.child() {
        content.set_opacity(progress);
    }
}
