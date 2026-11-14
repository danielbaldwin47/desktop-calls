//! The Setup Window while calls are off (ticket #207).
//!
//! Calls are off, so the Phone is blocked and nothing connects it: pairing
//! or switching to another Phone would only connect one, and the Phone
//! just set up would be blocked at once. So the window says calls are off,
//! with Turn calls on, and pairing and switching wait until calls are on.
//! Opened by the Indicator, the command line or a Problem's entry, it
//! says so at every step; at the first step, choosing a Phone, it also
//! takes the keyboard (Enter turns calls on) and leaves nothing to choose.
//!
//! A Phone paired during Quiet Hours is not blocked the moment it pairs:
//! pairing is the user turning calls on for the Phone they are setting up,
//! so, as for Turn calls on, they stay on until Quiet Hours next begin.

use super::State;
use super::calls_off::CallsOff;
use super::input::Effect;
use super::setup::Setup;
use crate::clock::TimeOfDay;

/// What the Setup Window says while calls are off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SetupCallsOff {
    /// When calls come back on by themselves, if at a time.
    pub until: Option<TimeOfDay>,
    /// Quiet Hours turned them off: until they end.
    pub quiet_hours: bool,
    /// The window is choosing a Phone, which can't be paired yet: the
    /// button takes keyboard focus, so Enter turns calls on, and the
    /// step's words are dimmed. Later steps keep Enter for themselves.
    pub blocks_step: bool,
}

impl State {
    /// What the Setup Window says of calls being off, while it is open and
    /// they are.
    pub(super) fn setup_calls_off_view(&self) -> Option<SetupCallsOff> {
        self.setup.as_ref()?;
        self.calls_off().then(|| SetupCallsOff {
            until: self.calls_off_until().map(|until| until.shown),
            quiet_hours: self.calls_off_for_quiet_hours(),
            blocks_step: self.setup.as_ref().is_some_and(Setup::choosing),
        })
    }

    /// Whether pairing and switching Phone wait: calls are off.
    pub(super) fn pairing_waits_for_calls(&self) -> bool {
        self.calls_off()
    }
}

/// A Phone is being set up in the Setup Window: during Quiet Hours calls
/// stay on until they next begin, so the Phone isn't blocked the moment it
/// is followed. Nothing changes when calls are already kept on, or when
/// Quiet Hours aren't under way.
pub(super) fn keep_on_for_setup(state: &mut State, effects: &mut Vec<Effect>) {
    if state.calls_off != CallsOff::On || state.kept_on_until.is_some() {
        return;
    }
    if let Some(quiet) = state.quiet_hours_under_way() {
        state.kept_on_until = Some(quiet.next_start);
        effects.push(Effect::KeepCallsOff(state.calls_on_record()));
    }
}
