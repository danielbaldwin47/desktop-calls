//! The Setup Window's finish page offering the Desktop Calls keybinding:
//! offered, shown, confirmed and written.

use super::{KeybindingStage, SetupNotice, Step};
use crate::keybinding::KeybindingCheck;
use crate::machine::State;
use crate::machine::input::{Effect, Input};

/// The finish page's keybinding: offered, shown, confirmed and written.
pub(super) fn keybinding_input(state: &mut State, input: Input, effects: &mut Vec<Effect>) {
    let Some(setup) = state.setup.as_mut() else {
        return;
    };
    if !matches!(setup.step, Step::Done(_)) {
        return;
    }
    let adding = setup.adding;
    let current = std::mem::take(&mut setup.keybinding);
    setup.keybinding = match (input, current) {
        (Input::KeybindingChecked(config), _) => {
            KeybindingStage::Checked(KeybindingCheck::of(&config))
        }
        (
            Input::AddKeybinding,
            KeybindingStage::Checked(check @ KeybindingCheck::Available { .. }),
        ) => {
            setup.notice = None;
            KeybindingStage::Confirming(check)
        }
        (Input::CancelKeybinding, KeybindingStage::Confirming(check)) if !adding => {
            KeybindingStage::Checked(check)
        }
        (Input::ConfirmKeybinding, KeybindingStage::Confirming(check)) => {
            if let (
                KeybindingCheck::Available {
                    file, seen, added, ..
                },
                false,
            ) = (&check, adding)
            {
                setup.adding = true;
                setup.notice = None;
                effects.push(Effect::AddKeybinding {
                    file: file.clone(),
                    seen: seen.clone(),
                    added: added.clone(),
                });
            }
            KeybindingStage::Confirming(check)
        }
        (
            Input::KeybindingAdded,
            KeybindingStage::Confirming(KeybindingCheck::Available { keys, file, .. }),
        ) if adding => {
            setup.adding = false;
            KeybindingStage::Checked(KeybindingCheck::Added { keys, file })
        }
        (Input::KeybindingNotAdded, KeybindingStage::Confirming(check)) if adding => {
            setup.adding = false;
            setup.notice = Some(SetupNotice::KeybindingNotAdded);
            KeybindingStage::Checked(check)
        }
        // Offered afresh from what the file holds now.
        (Input::KeybindingConfigChanged(config), KeybindingStage::Confirming(_)) if adding => {
            setup.adding = false;
            setup.notice = Some(SetupNotice::KeybindingConfigChanged);
            KeybindingStage::Checked(KeybindingCheck::of(&config))
        }
        (_, unchanged) => unchanged,
    };
}
