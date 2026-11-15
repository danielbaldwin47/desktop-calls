//! Which of the Setup Window's controls has the keyboard, as the window
//! reports it whenever focus moves, so the footer names the button Enter
//! presses after Tab or Shift+Tab (ticket #296).
//!
//! A report holds only on the page it was made on, and is forgotten once
//! that page is gone, even if the window comes back to it: on a new page
//! the window puts the keyboard on the default and reports again, and
//! until then the footer names the default rather than a button that is no
//! longer there.

use super::{KeybindingStage, SetupStep};
use crate::machine::State;
use crate::messages::ProblemKind;

/// A control in the Setup Window that can have the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupFocus {
    /// A Phone listed, while choosing: Enter pairs it.
    Phone,
    /// The step's button at this place among its buttons, the default one
    /// first.
    Button(usize),
    /// The button on this Problem's card.
    Problem(ProblemKind),
    /// Turn calls on, on the Calls Off card.
    TurnCallsOn,
}

/// What the window reported last, and the page it was on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ReportedFocus {
    page: Page,
    focus: SetupFocus,
}

/// A page of the Setup Window: its step, and on the finish page whether it
/// shows the keybinding's line, the only change of buttons within a step
/// that keeps two of them.
type Page = (&'static str, bool);

fn page(step: &SetupStep) -> Page {
    let line = matches!(
        step,
        SetupStep::Done {
            keybinding: KeybindingStage::Confirming(_),
            ..
        }
    );
    (step.as_str(), line)
}

/// The window says `focus` has the keyboard now, or nothing it knows.
pub(super) fn moved(state: &mut State, focus: Option<SetupFocus>) {
    let page = state.setup_window_view().map(|view| page(&view.step));
    if let (Some(setup), Some(page)) = (state.setup.as_mut(), page) {
        setup.focus = focus.map(|focus| ReportedFocus { page, focus });
    }
}

/// After every step: a report is forgotten once its page, or what it
/// names, is gone.
pub(super) fn forget_once_gone(state: &mut State) {
    if state
        .setup
        .as_ref()
        .is_none_or(|setup| setup.focus.is_none())
    {
        return;
    }
    let gone = state
        .setup_window_view()
        .is_none_or(|view| view.focus.is_none());
    if let Some(setup) = state.setup.as_mut().filter(|_| gone) {
        setup.focus = None;
    }
}

impl State {
    /// What has the keyboard on `step`, as reported on this page, while it
    /// is still there.
    pub(super) fn setup_focus(
        &self,
        reported: Option<&ReportedFocus>,
        step: &SetupStep,
    ) -> Option<SetupFocus> {
        let reported = reported.filter(|reported| reported.page == page(step))?;
        let there = match reported.focus {
            SetupFocus::Phone => {
                matches!(step, SetupStep::Choose { phones } if !phones.is_empty())
            }
            // Messages knows the step's buttons, and checks the place.
            SetupFocus::Button(_) => true,
            SetupFocus::Problem(kind) => self.desktop.problems().contains(&kind),
            SetupFocus::TurnCallsOn => self.setup_calls_off_view().is_some(),
        };
        there.then_some(reported.focus)
    }
}
