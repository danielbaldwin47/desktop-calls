//! The audio tile's progress bar while a Handoff is under way (#234), so
//! the two or three seconds the audio takes to move don't look stuck.
//!
//! The Phone says nothing until the audio has moved, so the bar shows time,
//! not a measured share: it fills fast, eases to 90% by the time a Handoff
//! usually takes, then creeps on towards 98% and stops there. It never
//! loops, and it goes as soon as the Handoff is over, either way.
//!
//! With reduced motion it is a cut: the bar moves on in steps, twice a
//! second, instead of smoothly.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::glib;
use gtk::prelude::*;

use super::super::animations_on;
use crate::logging::EventKind;
use crate::theme::class;

/// How long a Handoff usually takes: the bar is at 90% by then.
const USUAL: Duration = Duration::from_millis(2_500);
/// How long the bar creeps on after that, from 90% to 98%.
const CREEP: Duration = Duration::from_secs(5);
/// How far the bar starts, so it shows at once.
const START: f64 = 0.1;
/// Where it is once the usual time is up.
const USUAL_END: f64 = 0.9;
/// Where it stops.
const END: f64 = 0.98;
/// How often the bar moves on, as steps, with reduced motion.
const STEP: Duration = Duration::from_millis(500);

/// The bar along the audio tile's foot.
pub(super) struct HandoffProgress {
    bar: gtk::ProgressBar,
    /// Which run of the bar is under way: a new start or a stop ends the
    /// ticks of the one before.
    run: Rc<Cell<u64>>,
    started: RefCell<Option<Instant>>,
}

impl HandoffProgress {
    /// A hidden bar along the foot of the audio tile, which is `overlay`'s
    /// child.
    pub(super) fn new(overlay: &gtk::Overlay) -> Self {
        let bar = gtk::ProgressBar::new();
        bar.add_css_class(class::HANDOFF_PROGRESS);
        bar.set_valign(gtk::Align::End);
        bar.set_can_target(false);
        bar.set_visible(false);
        overlay.add_overlay(&bar);
        Self {
            bar,
            run: Rc::default(),
            started: RefCell::default(),
        }
    }

    /// Shows the bar while `moving`, from where it was if it already shows;
    /// hides it otherwise.
    pub(super) fn show(&self, moving: bool) {
        if !moving {
            if self.started.take().is_some() {
                self.run.set(self.run.get() + 1);
                self.bar.set_visible(false);
            }
            return;
        }
        if self.started.borrow().is_some() {
            return;
        }
        let started = Instant::now();
        *self.started.borrow_mut() = Some(started);
        self.run.set(self.run.get() + 1);
        self.bar.set_fraction(fraction(Duration::ZERO));
        self.bar.set_visible(true);
        let animated = animations_on();
        tracing::info!(
            event_kind = %EventKind::HandoffProgressStarted,
            animated,
            "The audio tile started showing the Handoff's progress"
        );
        let (run, this_run) = (self.run.clone(), self.run.get());
        let moves_on = move |bar: &gtk::ProgressBar| {
            if run.get() != this_run {
                return glib::ControlFlow::Break;
            }
            let elapsed = started.elapsed();
            bar.set_fraction(fraction(elapsed));
            if elapsed >= USUAL + CREEP {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        };
        if animated {
            self.bar.add_tick_callback(move |bar, _| moves_on(bar));
        } else {
            let bar = self.bar.clone();
            glib::timeout_add_local(STEP, move || moves_on(&bar));
        }
    }
}

/// How full the bar is `elapsed` into a Handoff: from [`START`], easing out
/// to [`USUAL_END`] over [`USUAL`], then on to [`END`] over [`CREEP`].
fn fraction(elapsed: Duration) -> f64 {
    let usual = (elapsed.as_secs_f64() / USUAL.as_secs_f64()).min(1.0);
    let eased = 1.0 - (1.0 - usual).powi(3);
    let creep = (elapsed.saturating_sub(USUAL).as_secs_f64() / CREEP.as_secs_f64()).min(1.0);
    START + (USUAL_END - START) * eased + (END - USUAL_END) * creep
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bar_shows_at_once_reaches_90_percent_in_the_usual_time_and_stops_short_of_full() {
        assert!((fraction(Duration::ZERO) - START).abs() < 1e-9);
        assert!((fraction(USUAL) - USUAL_END).abs() < 1e-9);
        assert!((fraction(USUAL + CREEP) - END).abs() < 1e-9);
        assert!((fraction(Duration::from_secs(60)) - END).abs() < 1e-9);
        let mut last = 0.0;
        for tenth in 0..=80 {
            let now = fraction(Duration::from_millis(tenth * 100));
            assert!(now >= last, "the bar went back at {tenth} tenths");
            last = now;
        }
    }
}
