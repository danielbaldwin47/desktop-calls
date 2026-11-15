//! The Setup Window's Contacts steps: the sync it starts, follows and
//! finishes on.

use super::{
    CONTACT_READ_MS, ContactsProgress, KeybindingStage, LINK_DROP_MS, Progress, ReadingContacts,
    Setup, Step, wait,
};
use crate::call::Timestamp;
use crate::connection::Phone;
use crate::contacts::{ContactsState, SyncProgress, Unavailable};
use crate::machine::State;
use crate::machine::input::Effect;

/// The sync the window follows is over once the Phone refused, the sync
/// failed, or it finished, and the window goes on to the finish page. An
/// unanswered prompt waits for Try again or Skip. If the Phone leaves
/// before Contacts are read, the window says so instead; once they are,
/// only older Calls are missing, and the window finishes.
pub(super) fn follow_contacts(state: &mut State, effects: &mut Vec<Effect>) {
    let here = state.phone_is_here();
    let sync = state.contacts_sync;
    let Some(setup) = state.setup.as_mut() else {
        return;
    };
    let phone = match &setup.step {
        Step::ContactsWarning(phone) | Step::Contacts(phone) => phone.clone(),
        // The link dropping can fail the sync just before the Phone is
        // known to have left: that failure is the Phone leaving, so the
        // window says so rather than that Contacts couldn't be read.
        Step::Done(phone) if setup.just_failed && !here => {
            setup.step = Step::ContactsAway(phone.clone());
            return;
        }
        _ => return,
    };
    let at_contacts = matches!(setup.step, Step::Contacts(_));
    if !here {
        if at_contacts && setup.progress.contacts.is_read() {
            finish(setup, phone, effects);
        } else {
            setup.step = Step::ContactsAway(phone);
        }
        return;
    }
    let over = at_contacts
        && match sync {
            ContactsState::Denied => true,
            ContactsState::Unavailable(reason) => reason != Unavailable::PromptTimedOut,
            ContactsState::Ready => !setup.progress.awaiting_finish,
            ContactsState::Syncing | ContactsState::PermissionNeeded { .. } => false,
        };
    if over {
        finish(setup, phone, effects);
        if sync == ContactsState::Unavailable(Unavailable::Failed) {
            wait(setup, LINK_DROP_MS, effects);
            setup.just_failed = true;
        }
    }
}

/// How far the sync under way has got, for the Setup Window to show.
pub(in crate::machine) fn sync_progress(state: &mut State, progress: &SyncProgress) {
    let Some(setup) = state.setup.as_mut() else {
        return;
    };
    if !matches!(setup.step, Step::Contacts(_)) {
        return;
    }
    let seen = &mut setup.progress;
    match progress {
        SyncProgress::Shared => seen.shared = true,
        SyncProgress::CallHistory(_) => seen.call_history = true,
        SyncProgress::ReadingContacts { count, at } => {
            seen.contacts = ContactsProgress::Reading(ReadingContacts {
                count: *count,
                done_by: count.map(|count| Timestamp(at.0 + u64::from(count) * CONTACT_READ_MS)),
            });
        }
        SyncProgress::Finished => seen.awaiting_finish = false,
    }
}

/// The Contacts the window follows are read.
pub(in crate::machine) fn contacts_read(state: &mut State) {
    if let Some(setup) = state.setup.as_mut()
        && matches!(setup.step, Step::Contacts(_))
    {
        setup.progress.contacts = ContactsProgress::Read(setup.progress.contacts.reading());
    }
}

/// The Phone the Setup Window is connecting is connected for Calls: its
/// first sync waits until the window has warned that the Phone is about to
/// ask whether to share Contacts. Back after leaving mid-Contacts, it waits
/// for the user to carry on. Whether it waits.
pub(in crate::machine) fn hold_sync(state: &mut State) -> bool {
    let followed = state.phone.clone();
    let Some(setup) = state.setup.as_mut() else {
        return false;
    };
    let held = followed.is_some_and(|followed| {
        let waiting = match &setup.step {
            Step::ContactsAway(phone) => Some(phone),
            step => step.connecting(),
        };
        waiting.is_some_and(|phone| phone.address.eq_ignore_ascii_case(&followed.address))
    });
    setup.sync_held |= held;
    held
}

/// Starts a sync, which the Setup Window then follows if it is at Contacts.
pub(super) fn start_sync(state: &mut State, effects: &mut Vec<Effect>) {
    state.contacts_sync = ContactsState::Syncing;
    effects.push(Effect::SyncContacts);
    if let Some(setup) = state.setup.as_mut()
        && matches!(setup.step, Step::Contacts(_))
    {
        setup.progress = Progress {
            awaiting_finish: true,
            ..Progress::default()
        };
    }
}

/// The warning has been up long enough: the sync starts, and the Phone
/// asks. A sync already under way is followed instead.
pub(super) fn begin_contacts(state: &mut State, phone: Phone, effects: &mut Vec<Effect>) {
    let Some(setup) = state.setup.as_mut() else {
        return;
    };
    let held = std::mem::take(&mut setup.sync_held);
    setup.step = Step::Contacts(phone);
    setup.progress = Progress::default();
    let under_way = matches!(
        state.contacts_sync,
        ContactsState::Syncing | ContactsState::PermissionNeeded { .. }
    );
    if !under_way && state.phone_is_here() && (held || state.may_sync_contacts()) {
        start_sync(state, effects);
    }
}

pub(super) fn finish(setup: &mut Setup, phone: Phone, effects: &mut Vec<Effect>) {
    setup.step = Step::Done(phone);
    setup.keybinding = KeybindingStage::Checking;
    setup.adding = false;
    effects.push(Effect::CheckKeybinding);
}
