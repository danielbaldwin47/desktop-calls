//! Where the Setup Window opens: on the finish page for the Phone set up
//! and followed, with Pair another phone to choose a different one, or on
//! choosing a Phone (ticket #389).

use super::{OpenedOn, Setup, State, Step, finish};
use crate::machine::input::Effect;

/// Opens the window: on the finish page for the Phone followed, so its
/// keybinding card is a click away from the Banner's footer (ticket #389),
/// or on choosing a Phone while none is followed or the one followed
/// forgot the pairing.
pub(super) fn open(state: &mut State, effects: &mut Vec<Effect>) {
    if state.setup.is_some() {
        return;
    }
    state.setup_openings += 1;
    let mut setup = Setup {
        opened: state.setup_openings,
        ..Setup::default()
    };
    match state.phone.clone().filter(|_| !state.presence.pairing_lost) {
        Some(phone) => {
            setup.opened_on = OpenedOn::Finish;
            finish(&mut setup, phone, effects);
        }
        // The Desktop looks for Phones, and is discoverable, until the
        // window closes.
        None => effects.push(Effect::StartSetup),
    }
    state.setup = Some(setup);
}

/// Pair another phone, from the finish page the window opened on: back to
/// choosing a Phone.
pub(super) fn pair_another(state: &mut State, effects: &mut Vec<Effect>) {
    let Some(setup) = state.setup.as_mut() else {
        return;
    };
    if setup.opened_on_finish() {
        choose(setup, effects);
    }
}

/// The window opened on the finish page stops being about the Phone set up
/// once that Phone forgets the pairing or is forgotten: it goes on to
/// choosing, as it would open then. And once it has left the finish page
/// (a Phone asked to pair from its own screen), it looks for Phones as a
/// window opened on choosing does, so turning that Phone down still leaves
/// one to choose.
pub(super) fn follow_opened(state: &mut State, effects: &mut Vec<Effect>) {
    let gone = state.phone.is_none() || state.presence.pairing_lost;
    let Some(setup) = state.setup.as_mut() else {
        return;
    };
    if setup.opened_on != OpenedOn::Finish {
        return;
    }
    if !matches!(setup.step, Step::Done(_)) {
        setup.opened_on = OpenedOn::Choose;
        effects.push(Effect::StartSetup);
    } else if gone {
        choose(setup, effects);
    }
}

/// From the finish page the window opened on, to choosing a Phone, looking
/// for Phones until the window closes.
fn choose(setup: &mut Setup, effects: &mut Vec<Effect>) {
    setup.opened_on = OpenedOn::Choose;
    setup.notice = None;
    setup.adding = false;
    setup.step = Step::Choose;
    effects.push(Effect::StartSetup);
}
