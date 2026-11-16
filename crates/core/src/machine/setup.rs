//! The Setup Window: pairing a Phone with the Desktop from nothing, or
//! pairing again, or switching to another Phone, and checking that the
//! Phone shares Calls.
//!
//! The steps, each shown in the window:
//!
//! 1. Choose: nearby Phones are listed by name as BlueZ finds them, and the
//!    Desktop is discoverable, so the person holding the Phone can start
//!    pairing from its own "Pair new device" screen instead.
//! 2. Pairing: the Desktop asked BlueZ to pair the chosen Phone, and waits
//!    for its code.
//! 3. Confirm: the six-digit code both screens show, with Confirm.
//! 4. Confirmed: waiting for the pairing to finish.
//! 5. Connecting: paired; the Phone becomes the one Desktop Calls follows,
//!    is trusted and connected. A Phone that connects without hands-free
//!    (its media, say) is asked for hands-free alone.
//! 6. Phone calls off: the Phone refused hands-free; the window says where
//!    to switch "Phone calls" on. Or pairing lost: the Phone answered
//!    without the pairing, as one that forgot the Desktop does, so the
//!    window offers Pair again. That, and only that, removes the Desktop's
//!    stale pairing; the Phone, found again, is paired as if chosen
//!    (ticket #377).
//! 7. Contacts warning: connected for Calls. The Phone is about to ask
//!    whether to share Contacts and Call History, so the window says so
//!    first, and the sync waits until the person has had time to read it.
//! 8. Contacts: the sync is under way. While the Phone asks, the window
//!    counts down to the prompt's deadline; if nobody answers in time, Try
//!    again asks once more. Once shared, it shows the Call History done as
//!    soon as it is read, then the Contacts being read.
//!    If the Phone leaves before Contacts are read, the window says so,
//!    and offers to carry on once it is back.
//! 9. Done: what was synced, or that Calls still work with numbers; an
//!    invitation to place a first Call, and the Desktop Calls keybinding,
//!    which it offers to add to the Hyprland config, writing nothing until
//!    the user confirms the exact line.
//!
//! Opened with a Phone set up and followed, the window opens on the finish
//! page for it instead, with Pair another phone to go back to choosing,
//! and looks for nothing until then (ticket #389). A Phone that forgot the
//! pairing still opens on choosing, to pair it again.

mod contacts;
mod focus;
mod keybinding;
mod opened;
mod step;

use contacts::{begin_contacts, finish, follow_contacts, start_sync};
pub(super) use contacts::{contacts_read, hold_sync, sync_progress};
pub use focus::SetupFocus;
use opened::{follow_opened, open, pair_another};
pub use step::SetupStep;

use super::dialling::phone_unpaired;
use super::input::{Effect, Input, PairingFailure, Timer};
use super::presence::connect_phone_now;
use super::setup_calls_off::{SetupCallsOff, keep_on_for_setup};
use super::{State, forget_phone_keeping_desktop, phone::phone_paired};
use crate::call::Timestamp;
use crate::connection::{ConnectionState, Phone};
use crate::contacts::{ContactsState, Unavailable};
use crate::keybinding::KeybindingCheck;
use crate::messages::ProblemKind;

/// How long a Phone that just paired may take to connect before the
/// Desktop asks it for hands-free alone. A Phone sharing Calls had its
/// hands-free up 0.6 s after connecting (ticket #19's notes).
const HANDS_FREE_WAIT_MS: u32 = 2_500;
/// How long a connect may take, from pairing, before the Desktop asks for
/// hands-free alone whatever became of it: longer than BlueZ's page
/// timeout, 5.12 s.
const CONNECT_WAIT_MS: u32 = 8_000;
/// How long the Phone may take to bring hands-free up once asked for it.
const HANDS_FREE_ASKED_MS: u32 = 10_000;
/// How long the pairing may take to finish once confirmed: the person
/// holding the Phone still has to tap Pair on it.
const CONFIRMED_MS: u32 = 30_000;
/// How long the window warns that the Phone is about to ask whether to
/// share Contacts before the sync starts and the Phone asks: long enough
/// to read one sentence and look at the Phone.
const WARNING_MS: u32 = 3_000;
/// About how long the Phone takes to give each Contact: the Pixel gave 319
/// in 9.7 s (2026-11-05).
pub(crate) const CONTACT_READ_MS: u64 = 30;
/// How soon after a sync fails the Phone leaving means the failure was the
/// link dropping: obexd's call fails a moment before hands-free goes down.
const LINK_DROP_MS: u32 = 2_000;

/// The open Setup Window.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Setup {
    step: Step,
    /// How this Desktop shows on Phones, once known.
    desktop_name: Option<String>,
    /// Why the last attempt didn't work, shown until the next.
    notice: Option<SetupNotice>,
    /// Which wait is current, so only its timer counts.
    wait: u32,
    opened: u32,
    /// The Phone connected, and its first sync waits for the warning.
    sync_held: bool,
    /// How far the sync the window follows has got.
    progress: Progress,
    keybinding: KeybindingStage,
    /// The confirmed keybinding is being written.
    adding: bool,
    /// The sync failed a moment ago, and the finish page shows it: if the
    /// Phone leaves now, the failure was the link dropping.
    just_failed: bool,
    /// What the window reported has the keyboard, and on which page.
    focus: Option<focus::ReportedFocus>,
    /// Where the window opened, which says whether it is looking for
    /// Phones.
    opened_on: OpenedOn,
}

/// Where the Setup Window opened.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum OpenedOn {
    /// Choosing a Phone: the Desktop looks for Phones and is discoverable
    /// until the window closes. Pair another phone goes here too.
    #[default]
    Choose,
    /// The finish page for the Phone already set up, which offers Pair
    /// another phone; nothing is looked for (ticket #389).
    Finish,
}

/// How far the sync the Setup Window follows has got.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Progress {
    /// The window started the sync and it hasn't finished, so the window
    /// waits for it rather than going on at the first Ready.
    awaiting_finish: bool,
    shared: bool,
    call_history: bool,
    contacts: ContactsProgress,
}

/// How far the sync has got with Contacts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ContactsProgress {
    #[default]
    NotStarted,
    Reading(ReadingContacts),
    /// Read, after reading as shown; older Calls may still be read.
    Read(Option<ReadingContacts>),
}

impl ContactsProgress {
    fn reading(self) -> Option<ReadingContacts> {
        match self {
            Self::NotStarted => None,
            Self::Reading(reading) => Some(reading),
            Self::Read(reading) => reading,
        }
    }

    fn is_read(self) -> bool {
        matches!(self, Self::Read(_))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum Step {
    #[default]
    Choose,
    Pairing(Phone),
    Confirm(Phone, u32),
    Confirmed(Phone),
    Connecting(Phone),
    /// Hands-free was asked for, alone.
    AskedForHandsFree(Phone),
    PhoneCallsOff(Phone),
    /// The Phone didn't answer: out of range, or its Bluetooth is off.
    PhoneUnreachable(Phone),
    /// The Phone, asked for hands-free, no longer had the Desktop paired.
    PairingLost(Phone),
    /// Pair again: the Desktop's stale pairing is being removed, and the
    /// Phone looked for, to pair it once found.
    PairingAgain(Phone),
    ContactsWarning(Phone),
    Contacts(Phone),
    /// The Phone left before Contacts were read.
    ContactsAway(Phone),
    Done(Phone),
}

/// The Contacts the sync is reading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadingContacts {
    /// How many, if the Phone said.
    pub count: Option<u32>,
    /// About when they will all be read, if the Phone said how many.
    pub done_by: Option<Timestamp>,
}

/// What became of Contacts, on the finish page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContactsOutcome {
    /// Synced: this many Contacts and Calls of the Call History.
    Synced { contacts: usize, calls: usize },
    /// The Phone refused to share them.
    Denied,
    /// Not shared, for another reason: nobody answered the Phone, or the
    /// sync failed.
    NotShared,
    /// Being read now, as after every connect: seen when the window opens
    /// on the finish page for a Phone already set up.
    Syncing,
    /// The Phone is asking whether to share them.
    Asking,
}

/// Where the finish page is with the keybinding.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum KeybindingStage {
    /// Finding what the Hyprland config can take.
    #[default]
    Checking,
    Checked(KeybindingCheck),
    /// Showing the exact line, for the user to confirm or decline.
    Confirming(KeybindingCheck),
}

/// What the Setup Window shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetupWindowView {
    pub step: SetupStep,
    /// How this Desktop shows on Phones, once known.
    pub desktop_name: Option<String>,
    /// Why the last attempt didn't work, unless a Problem card says it.
    pub notice: Option<SetupNotice>,
    /// Different each time the window opens.
    pub opened: u32,
    /// Calls are off: the window says so, with Turn calls on.
    pub calls_off: Option<SetupCallsOff>,
    /// The Problem whose button takes keyboard focus by itself, so Enter
    /// presses it instead of the step's default button.
    pub problem_has_focus: Option<ProblemKind>,
    /// The Desktop's Bluetooth is off or blocked, so nothing is looked for
    /// and nothing can pair until it is on.
    pub bluetooth_off: bool,
    /// What the window last reported has the keyboard on this page, while
    /// it is still there: after Tab or Shift+Tab, Enter presses it.
    pub focus: Option<SetupFocus>,
}

/// The Phone the user chose last in the Setup Window: its address, and its
/// name, which a choice kept before ticket #150 lacks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ChosenPhone {
    pub(super) address: String,
    pub(super) name: Option<String>,
}

impl ChosenPhone {
    /// Whether this is the Phone with `address`.
    pub(super) fn is(&self, address: &str) -> bool {
        self.address.eq_ignore_ascii_case(address)
    }
}

/// Why the Setup Window's last attempt didn't work, or why it opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetupNotice {
    /// It opened by itself at startup because the Phone the user chose
    /// last, with this name if it was kept, is no longer paired, and no
    /// other Phone is followed in its place (ticket #150).
    ChosenPhoneUnpaired { name: Option<String> },
    /// The Desktop can't look for Phones: its Bluetooth is off or missing.
    /// With Bluetooth off or blocked, the view leaves it to the Problem
    /// card.
    BluetoothUnavailable,
    /// Pairing was turned down, on the Phone or here, or cancelled.
    PairingRejected,
    /// The Phone being paired didn't answer.
    PairingUnreachable,
    /// Pairing didn't finish, for a reason BlueZ didn't give.
    PairingFailed,
    /// BlueZ didn't remove the Desktop's stale pairing with the Phone, for
    /// Pair again (ticket #377).
    PairingNotRemoved,
    /// The keybinding couldn't be written to the Hyprland config.
    KeybindingNotAdded,
    /// The Hyprland config changed after the line was shown, so nothing was
    /// written; the page shows what it can take now.
    KeybindingConfigChanged,
}

impl Setup {
    /// Whether the window is at its first step, choosing a Phone.
    pub(super) fn choosing(&self) -> bool {
        matches!(self.step, Step::Choose)
    }

    /// Whether the window is on the finish page it opened on, for the
    /// Phone already set up.
    pub(super) fn opened_on_finish(&self) -> bool {
        self.opened_on == OpenedOn::Finish && matches!(self.step, Step::Done(_))
    }
}

impl Step {
    /// Whether pairing with `address` is under way, so its finishing means
    /// the Phone is paired.
    fn pairs(&self, address: &str) -> bool {
        self.pairing()
            .is_some_and(|phone| phone.address.eq_ignore_ascii_case(address))
    }

    /// The Phone being paired, if pairing is under way.
    fn pairing(&self) -> Option<&Phone> {
        match self {
            Self::Pairing(phone) | Self::Confirm(phone, _) | Self::Confirmed(phone) => Some(phone),
            _ => None,
        }
    }

    /// The Phone that may be paired and not yet connected for Calls. A
    /// Phone paired again from its own screen may be connected for Calls
    /// as soon as the user confirmed: BlueZ says nothing more of it.
    fn connecting(&self) -> Option<&Phone> {
        match self {
            Self::Confirmed(phone)
            | Self::Connecting(phone)
            | Self::AskedForHandsFree(phone)
            | Self::PhoneCallsOff(phone)
            | Self::PhoneUnreachable(phone) => Some(phone),
            _ => None,
        }
    }
}

impl State {
    /// What the Setup Window shows, while it is open.
    pub(super) fn setup_window_view(&self) -> Option<SetupWindowView> {
        let setup = self.setup.as_ref()?;
        let step = match &setup.step {
            Step::Choose => SetupStep::Choose {
                phones: self
                    .already_paired_to_choose()
                    .chain(self.nearby_phones.iter())
                    .cloned()
                    .collect(),
            },
            Step::Pairing(phone) => SetupStep::Pairing {
                phone: phone.clone(),
            },
            Step::Confirm(phone, code) => SetupStep::Confirm {
                phone: phone.clone(),
                code: *code,
            },
            Step::Confirmed(phone) => SetupStep::Confirmed {
                phone: phone.clone(),
            },
            Step::Connecting(phone) | Step::AskedForHandsFree(phone) => SetupStep::Connecting {
                phone: phone.clone(),
            },
            Step::PhoneCallsOff(phone) => SetupStep::PhoneCallsOff {
                phone: phone.clone(),
            },
            Step::PhoneUnreachable(phone) => SetupStep::PhoneUnreachable {
                phone: phone.clone(),
            },
            Step::PairingLost(phone) => SetupStep::PairingLost {
                phone: phone.clone(),
            },
            Step::PairingAgain(phone) => SetupStep::PairingAgain {
                phone: phone.clone(),
            },
            Step::ContactsWarning(phone) => SetupStep::ContactsWarning {
                phone: phone.clone(),
            },
            Step::Contacts(phone) => self.contacts_step(setup, phone.clone()),
            Step::ContactsAway(phone) => SetupStep::ContactsPhoneAway {
                phone: phone.clone(),
                back: self.phone_is_here(),
            },
            Step::Done(phone) => SetupStep::Done {
                phone: phone.clone(),
                connected: self.phone_is_here(),
                contacts: self.contacts_outcome(),
                keybinding: setup.keybinding.clone(),
                pair_another: setup.opened_on == OpenedOn::Finish,
            },
        };
        // A Problem card already says Bluetooth is off or blocked, with its
        // Fix: the window says it once.
        let said_by_a_problem =
            setup.notice == Some(SetupNotice::BluetoothUnavailable) && self.desktop.bluetooth_off();
        // A Phone followed since, paired again outside the window: no
        // longer true.
        let back = matches!(setup.notice, Some(SetupNotice::ChosenPhoneUnpaired { .. }))
            && self.phone.is_some();
        Some(SetupWindowView {
            focus: self.setup_focus(setup.focus.as_ref(), &step),
            step,
            desktop_name: setup.desktop_name.clone(),
            notice: setup.notice.clone().filter(|_| !said_by_a_problem && !back),
            opened: setup.opened,
            calls_off: self.setup_calls_off_view(),
            problem_has_focus: self
                .problem_blocks_step()
                .then(|| self.desktop.problems().first().copied())
                .flatten(),
            bluetooth_off: self.desktop.bluetooth_off(),
        })
    }

    /// The Phones paired already that the Setup Window offers: while no
    /// Phone is followed, such as when the chosen Phone was unpaired, any of
    /// them can be chosen without pairing it again.
    fn already_paired_to_choose(&self) -> impl Iterator<Item = &Phone> {
        self.paired_phones.iter().filter(|_| self.phone.is_none())
    }

    /// What the window shows while Contacts are being synced.
    fn contacts_step(&self, setup: &Setup, phone: Phone) -> SetupStep {
        match self.contacts_sync {
            ContactsState::PermissionNeeded { deadline } => {
                SetupStep::ContactsPrompt { phone, deadline }
            }
            ContactsState::Unavailable(Unavailable::PromptTimedOut) => {
                SetupStep::ContactsPromptTimedOut { phone }
            }
            sync if setup.progress.shared || sync == ContactsState::Ready => {
                SetupStep::ContactsSyncing {
                    phone,
                    call_history: setup.progress.call_history,
                    contacts: setup.progress.contacts.reading(),
                    contacts_read: setup.progress.contacts.is_read(),
                }
            }
            // Until the Phone is known to ask, or to share, the warning
            // stays.
            _ => SetupStep::ContactsWarning { phone },
        }
    }

    fn contacts_outcome(&self) -> ContactsOutcome {
        match self.contacts_sync {
            ContactsState::Ready => ContactsOutcome::Synced {
                contacts: self.contacts.directory().len(),
                calls: self.call_history.entries().len(),
            },
            ContactsState::Denied => ContactsOutcome::Denied,
            ContactsState::Syncing => ContactsOutcome::Syncing,
            ContactsState::PermissionNeeded { .. } => ContactsOutcome::Asking,
            ContactsState::Unavailable(_) => ContactsOutcome::NotShared,
        }
    }

    /// Whether the Setup Window is pairing the Phone with `address`.
    pub(super) fn setup_pairs(&self, address: &str) -> bool {
        self.setup
            .as_ref()
            .is_some_and(|setup| setup.step.pairs(address))
    }
}

/// What the Setup Window, BlueZ's pairing and the user report.
pub(super) fn setup_input(state: &mut State, input: Input, effects: &mut Vec<Effect>) {
    match input {
        Input::PairedPhonesListed => {
            // Opens by itself once, at startup, with no Phone set up.
            if !std::mem::replace(&mut state.setup_offered, true) && state.phone.is_none() {
                // Opened by hand already: its notice, if any, stays.
                let opens = state.setup.is_none();
                open(state, effects);
                // A chosen Phone that is paired is followed, so it's gone.
                let gone = state.chosen_phone.as_ref().filter(|_| opens).map(|chosen| {
                    SetupNotice::ChosenPhoneUnpaired {
                        name: chosen.name.clone(),
                    }
                });
                if let (Some(setup), Some(gone)) = (state.setup.as_mut(), gone) {
                    setup.notice = Some(gone);
                }
            }
        }
        Input::OpenSetupWindow => open(state, effects),
        Input::PairAnotherPhone => pair_another(state, effects),
        Input::CloseSetupWindow => close(state, effects),
        Input::SetupReady { desktop_name } => {
            if let Some(setup) = state.setup.as_mut() {
                setup.desktop_name = Some(desktop_name);
            }
        }
        Input::SetupUnavailable => {
            if let Some(setup) = state.setup.as_mut() {
                setup.notice = Some(SetupNotice::BluetoothUnavailable);
            }
        }
        Input::PhoneNearby(phone) => {
            match state
                .nearby_phones
                .iter_mut()
                .find(|known| known.address.eq_ignore_ascii_case(&phone.address))
            {
                Some(known) => *known = phone.clone(),
                None => state.nearby_phones.push(phone.clone()),
            }
            found_again(state, &phone, effects);
        }
        Input::PhoneNotNearby { address } => not_nearby(state, &address),
        // Pairing waits while calls are off; a Phone pairing from its own
        // screen is turned down, as while another pairing is under way.
        Input::ChoosePhone { .. } if state.pairing_waits_for_calls() => {}
        Input::PairingCode { .. } if state.pairing_waits_for_calls() => {
            effects.push(Effect::AnswerPairingCode { confirm: false });
        }
        Input::ChoosePhone { address } => choose(state, &address, effects),
        Input::PairingCode { phone, code } => pairing_code(state, phone, code, effects),
        Input::ConfirmPairing => answer_code(state, true, effects),
        Input::RejectPairing => answer_code(state, false, effects),
        Input::PairingFailed(failure) => pairing_failed(state, failure),
        Input::PairDone { address } => pair_done(state, &address, effects),
        Input::HandsFreeRefused => hands_free_answered(state, Step::PhoneCallsOff),
        Input::HandsFreeUnreachable => hands_free_answered(state, Step::PhoneUnreachable),
        // Asking again would only start another pairing on the Phone: the
        // window offers Pair again instead.
        Input::HandsFreeUnpaired => hands_free_answered(state, Step::PairingLost),
        Input::PairAgain => pair_again(state, effects),
        Input::PairingNotRemoved => {
            if let Some(setup) = state.setup.as_mut()
                && let Step::PairingAgain(phone) = &setup.step
            {
                setup.step = Step::PairingLost(phone.clone());
                setup.notice = Some(SetupNotice::PairingNotRemoved);
            }
        }
        Input::RetryHandsFree => retry(state, effects),
        Input::SetupWindowFocus(focus) => focus::moved(state, focus),
        Input::RetryContacts => {
            if state.contacts_sync == ContactsState::Unavailable(Unavailable::PromptTimedOut)
                && state.phone_is_here()
            {
                start_sync(state, effects);
            }
        }
        Input::ContinueContacts => {
            if let Some(setup) = state.setup.as_mut()
                && let Step::ContactsAway(phone) = &setup.step
                && state.connection == ConnectionState::Connected
            {
                let phone = phone.clone();
                begin_contacts(state, phone, effects);
            }
        }
        Input::SkipContacts => {
            if let Some(setup) = state.setup.as_mut()
                && let Step::Contacts(phone) = &setup.step
            {
                let phone = phone.clone();
                finish(setup, phone, effects);
            }
        }
        input => keybinding::keybinding_input(state, input, effects),
    }
}

/// Try again, from "Phone calls" off or from a Phone that didn't answer:
/// the first asks for hands-free alone; the second connects the Phone
/// afresh, as it may be back.
fn retry(state: &mut State, effects: &mut Vec<Effect>) {
    let Some(setup) = state.setup.as_mut() else {
        return;
    };
    match &setup.step {
        Step::PhoneCallsOff(phone) => {
            let phone = phone.clone();
            ask_for_hands_free(setup, phone, effects);
        }
        Step::PhoneUnreachable(phone) => {
            let phone = phone.clone();
            paired(state, phone, effects);
        }
        _ => {}
    }
}

/// The Phone asked for hands-free didn't bring it up: the window goes on to
/// the step `next` makes for it.
fn hands_free_answered(state: &mut State, next: fn(Phone) -> Step) {
    if let Some(setup) = state.setup.as_mut()
        && let Step::AskedForHandsFree(phone) = &setup.step
    {
        setup.step = next(phone.clone());
    }
}

/// Pair again, for a Phone that no longer has the Desktop paired: the
/// Desktop's stale pairing is removed, the only time the Desktop removes
/// one, and the Phone is paired once BlueZ finds it again.
fn pair_again(state: &mut State, effects: &mut Vec<Effect>) {
    let Some(setup) = state.setup.as_mut() else {
        return;
    };
    let Step::PairingLost(phone) = &setup.step else {
        return;
    };
    let phone = phone.clone();
    setup.notice = None;
    setup.step = Step::PairingAgain(phone.clone());
    // Listed while it was paired, it is nearby only once found again.
    not_nearby(state, &phone.address);
    effects.push(Effect::RemovePairing {
        address: phone.address,
    });
}

/// A Phone found nearby: the one Pair again looks for is paired, as if the
/// user had chosen it.
fn found_again(state: &mut State, phone: &Phone, effects: &mut Vec<Effect>) {
    let Some(setup) = state.setup.as_mut() else {
        return;
    };
    if let Step::PairingAgain(lost) = &setup.step
        && lost.address.eq_ignore_ascii_case(&phone.address)
    {
        setup.step = Step::Pairing(phone.clone());
        effects.push(Effect::PairPhone {
            address: phone.address.clone(),
        });
    }
}

/// The Desktop's Bluetooth came on: a Setup Window that couldn't look for
/// Phones without it looks now.
pub(super) fn bluetooth_on(state: &mut State, effects: &mut Vec<Effect>) {
    if let Some(setup) = state.setup.as_mut()
        && setup.notice == Some(SetupNotice::BluetoothUnavailable)
    {
        setup.notice = None;
        effects.push(Effect::StartSetup);
    }
}

fn close(state: &mut State, effects: &mut Vec<Effect>) {
    let Some(setup) = state.setup.take() else {
        return;
    };
    if matches!(setup.step, Step::Confirm(..)) {
        effects.push(Effect::AnswerPairingCode { confirm: false });
    }
    // Opened on the finish page and closed there, nothing was looked for.
    if setup.opened_on == OpenedOn::Choose {
        effects.push(Effect::StopSetup);
    }
    // The first sync waited for the warning, which won't be seen now.
    if setup.sync_held && state.phone_is_here() {
        state.contacts_sync = ContactsState::Syncing;
        effects.push(Effect::SyncContacts);
    }
}

fn not_nearby(state: &mut State, address: &str) {
    state
        .nearby_phones
        .retain(|phone| !phone.address.eq_ignore_ascii_case(address));
}

fn choose(state: &mut State, address: &str, effects: &mut Vec<Effect>) {
    if !state
        .setup
        .as_ref()
        .is_some_and(|setup| setup.step == Step::Choose)
    {
        return;
    }
    // A Phone paired already is followed at once, with nothing to pair.
    let paired_already = state
        .already_paired_to_choose()
        .find(|phone| phone.address.eq_ignore_ascii_case(address))
        .cloned();
    if let Some(phone) = paired_already {
        if let Some(setup) = state.setup.as_mut() {
            setup.notice = None;
        }
        paired(state, phone, effects);
        return;
    }
    let Some(phone) = state
        .nearby_phones
        .iter()
        .find(|phone| phone.address.eq_ignore_ascii_case(address))
        .cloned()
    else {
        return;
    };
    let Some(setup) = state.setup.as_mut() else {
        return;
    };
    setup.notice = None;
    setup.step = Step::Pairing(phone.clone());
    effects.push(Effect::PairPhone {
        address: phone.address,
    });
}

/// BlueZ asks whether the code shows on the Phone too: for the Phone the
/// user chose, or one pairing from its own screen. Only one pairing at a
/// time; another is turned down.
fn pairing_code(state: &mut State, phone: Phone, code: u32, effects: &mut Vec<Effect>) {
    let Some(setup) = state.setup.as_mut() else {
        effects.push(Effect::AnswerPairingCode { confirm: false });
        return;
    };
    let welcome = match &setup.step {
        Step::Choose | Step::PhoneCallsOff(_) | Step::PhoneUnreachable(_) | Step::Done(_) => true,
        // Paired from the Phone's own screen before the Desktop found it.
        Step::Pairing(chosen) | Step::PairingAgain(chosen) => {
            chosen.address.eq_ignore_ascii_case(&phone.address)
        }
        _ => false,
    };
    if !welcome {
        effects.push(Effect::AnswerPairingCode { confirm: false });
        return;
    }
    setup.notice = None;
    setup.step = Step::Confirm(phone, code);
}

fn answer_code(state: &mut State, confirm: bool, effects: &mut Vec<Effect>) {
    let Some(setup) = state.setup.as_mut() else {
        return;
    };
    let Step::Confirm(phone, _) = &setup.step else {
        return;
    };
    let phone = phone.clone();
    effects.push(Effect::AnswerPairingCode { confirm });
    if confirm {
        setup.step = Step::Confirmed(phone);
        wait(setup, CONFIRMED_MS, effects);
    } else {
        setup.step = Step::Choose;
    }
}

fn pairing_failed(state: &mut State, failure: PairingFailure) {
    let Some(setup) = state.setup.as_mut() else {
        return;
    };
    if !matches!(
        setup.step,
        Step::Pairing(_) | Step::Confirm(..) | Step::Confirmed(_)
    ) {
        return;
    }
    setup.step = Step::Choose;
    setup.notice = Some(match failure {
        PairingFailure::Rejected => SetupNotice::PairingRejected,
        PairingFailure::Unreachable => SetupNotice::PairingUnreachable,
        PairingFailure::Failed => SetupNotice::PairingFailed,
    });
}

/// BlueZ paired the Phone the Setup Window is pairing, or had it paired
/// already: it goes on as paired, without waiting for BlueZ to signal it,
/// which BlueZ may never do (ticket #372). Whether the Phone offers Calls
/// is for asking it for hands-free to find out.
fn pair_done(state: &mut State, address: &str, effects: &mut Vec<Effect>) {
    let phone = state.setup.as_ref().and_then(|setup| setup.step.pairing());
    if let Some(phone) = phone.filter(|phone| phone.address.eq_ignore_ascii_case(address)) {
        paired(state, phone.clone(), effects);
    }
}

/// A paired Phone whose services don't show hands-free, as a paired Phone
/// if the user chose it: the one chosen last, or the one the Setup Window
/// is pairing (ticket #375).
pub(super) fn paired_as_chosen(state: &State, input: Input) -> Input {
    match input {
        Input::PhonePairedWithoutHandsFree(phone)
            if state.setup_pairs(&phone.address)
                || state
                    .chosen_phone
                    .as_ref()
                    .is_some_and(|chosen| chosen.is(&phone.address)) =>
        {
            Input::PhonePaired(phone)
        }
        input => input,
    }
}

/// An earlier run kept the Phone with `address`, and its `name`, as the
/// Phone the user chose last.
pub(super) fn kept(state: &mut State, address: String, name: Option<String>) {
    state.chosen_phone = Some(ChosenPhone { address, name });
}

/// The chosen Phone, followed again: kept again with its name, if that is
/// new or wasn't kept, so the Setup Window can name it once it's gone.
pub(super) fn chosen_followed(state: &mut State, phone: &Phone, effects: &mut Vec<Effect>) {
    let chosen = state.chosen_phone.as_ref();
    if chosen.is_some_and(|chosen| chosen.is(&phone.address)) {
        keep(state, phone, effects);
    }
}

/// Keeps `phone` as the Phone the user chose, when it or its name is new.
/// A name that is only the address, as BlueZ gives for a Phone with none
/// (with dashes), isn't one.
fn keep(state: &mut State, phone: &Phone, effects: &mut Vec<Effect>) {
    let chosen = ChosenPhone {
        address: phone.address.clone(),
        name: Some(phone.name.clone())
            .filter(|name| !phone.address.eq_ignore_ascii_case(&name.replace('-', ":"))),
    };
    let kept = state.chosen_phone.as_ref();
    if !kept.is_some_and(|kept| kept.is(&phone.address) && kept.name == chosen.name) {
        state.chosen_phone = Some(chosen);
        let phone = phone.clone();
        effects.push(Effect::KeepChosenPhone { phone });
    }
}

/// The Phone the Setup Window was pairing is paired: Desktop Calls follows
/// it from now on, instead of any Phone it followed before, trusts it and
/// connects it.
pub(super) fn paired(state: &mut State, phone: Phone, effects: &mut Vec<Effect>) {
    let already_followed = state
        .phone
        .as_ref()
        .is_some_and(|followed| followed.address.eq_ignore_ascii_case(&phone.address));
    // The user chose it, so a restart follows it, not the Phone BlueZ lists
    // first.
    keep(state, &phone, effects);
    if !already_followed {
        if state.phone.is_some() {
            switch_away(state, effects);
        }
        phone_paired(state, phone.clone(), effects);
    }
    not_nearby(state, &phone.address);
    effects.push(Effect::TrustPhone {
        address: phone.address.clone(),
    });
    keep_on_for_setup(state, effects);
    if let Some(setup) = state.setup.as_mut() {
        setup.step = Step::Connecting(phone);
        wait(setup, CONNECT_WAIT_MS, effects);
    }
    if state.connection != ConnectionState::Connected {
        connect_phone_now(state, effects);
    }
}

/// Stops following the Phone followed so far, keeping the Desktop's own
/// settings and the Setup Window.
fn switch_away(state: &mut State, effects: &mut Vec<Effect>) {
    effects.push(Effect::DetachPhone);
    phone_unpaired(state, effects);
    super::calls_off::phone_forgotten(state, effects);
    *state = forget_phone_keeping_desktop(state);
}

/// The Phone the Setup Window is connecting finished a connect, either
/// way: if hands-free isn't up shortly, it is asked for alone.
pub(super) fn connect_ended(state: &mut State, effects: &mut Vec<Effect>) {
    if let Some(setup) = state.setup.as_mut()
        && matches!(setup.step, Step::Connecting(_))
    {
        wait(setup, HANDS_FREE_WAIT_MS, effects);
    }
}

/// The Setup Window's wait with this number is over.
pub(super) fn setup_timer(state: &mut State, wait: u32, effects: &mut Vec<Effect>) {
    let followed = state.phone.clone();
    let Some(setup) = state.setup.as_mut() else {
        return;
    };
    if setup.wait != wait {
        return;
    }
    // A Phone paired again from its own screen, which BlueZ says nothing
    // more of, and which didn't connect by itself: connect it.
    if let (Step::Confirmed(phone), Some(followed)) = (&setup.step, followed)
        && phone.address.eq_ignore_ascii_case(&followed.address)
    {
        paired(state, followed, effects);
        return;
    }
    if let Step::ContactsWarning(phone) = &setup.step {
        let phone = phone.clone();
        begin_contacts(state, phone, effects);
        return;
    }
    match &setup.step {
        Step::Connecting(phone) => {
            let phone = phone.clone();
            ask_for_hands_free(setup, phone, effects);
        }
        Step::AskedForHandsFree(phone) => setup.step = Step::PhoneCallsOff(phone.clone()),
        Step::Done(_) => setup.just_failed = false,
        Step::Confirmed(_) => {
            setup.step = Step::Choose;
            setup.notice = Some(SetupNotice::PairingFailed);
        }
        _ => {}
    }
}

fn ask_for_hands_free(setup: &mut Setup, phone: Phone, effects: &mut Vec<Effect>) {
    effects.push(Effect::ConnectHandsFree {
        address: phone.address.clone(),
    });
    setup.step = Step::AskedForHandsFree(phone);
    wait(setup, HANDS_FREE_ASKED_MS, effects);
}

fn wait(setup: &mut Setup, after_ms: u32, effects: &mut Vec<Effect>) {
    setup.wait += 1;
    setup.just_failed = false;
    effects.push(Effect::StartTimer {
        timer: Timer::SetupStep(setup.wait),
        after_ms,
    });
}

/// After every step: the window follows the connection and the sync, and
/// forgets what had the keyboard once the page it was on is gone.
pub(super) fn follow(state: &mut State, effects: &mut Vec<Effect>) {
    follow_connection(state, effects);
    follow_contacts(state, effects);
    follow_opened(state, effects);
    focus::forget_once_gone(state);
}

/// The Phone being set up is connected for Calls once hands-free is up,
/// and the window warns that it is about to ask whether to share Contacts.
fn follow_connection(state: &mut State, effects: &mut Vec<Effect>) {
    let connected = state.connection == ConnectionState::Connected;
    let followed = state.phone.clone();
    let Some(setup) = state.setup.as_mut() else {
        return;
    };
    if let (true, Some(followed)) = (connected, followed)
        && setup
            .step
            .connecting()
            .is_some_and(|phone| phone.address.eq_ignore_ascii_case(&followed.address))
    {
        setup.step = Step::ContactsWarning(followed);
        wait(setup, WARNING_MS, effects);
    }
}
