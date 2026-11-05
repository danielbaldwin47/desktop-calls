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
//!    to switch "Phone calls" on.
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

use super::dialling::phone_unpaired;
use super::input::{Effect, Input, PairingFailure, Timer};
use super::presence::connect_phone_now;
use super::{State, forget_phone_keeping_desktop, phone::phone_paired};
use crate::call::Timestamp;
use crate::connection::{ConnectionState, Phone};
use crate::contacts::{ContactsState, SyncProgress, Unavailable};
use crate::keybinding::KeybindingCheck;

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
const CONTACT_READ_MS: u64 = 30;

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
    /// Why the last attempt didn't work.
    pub notice: Option<SetupNotice>,
    /// Different each time the window opens.
    pub opened: u32,
}

/// Why the Setup Window's last attempt didn't work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupNotice {
    /// The Desktop can't look for Phones: its Bluetooth is off or missing.
    BluetoothUnavailable,
    /// Pairing was turned down, on the Phone or here, or cancelled.
    PairingRejected,
    /// The Phone being paired didn't answer.
    PairingUnreachable,
    /// Pairing didn't finish, for a reason BlueZ didn't give.
    PairingFailed,
    /// The keybinding couldn't be written to the Hyprland config.
    KeybindingNotAdded,
    /// The Hyprland config changed after the line was shown, so nothing was
    /// written; the page shows what it can take now.
    KeybindingConfigChanged,
}

/// The Setup Window's step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetupStep {
    /// Choose from the nearby Phones, in the order found.
    Choose { phones: Vec<Phone> },
    /// Waiting for the chosen Phone's pairing code.
    Pairing { phone: Phone },
    /// The code both screens should show, and Confirm.
    Confirm { phone: Phone, code: u32 },
    /// Confirmed; waiting for the pairing to finish.
    Confirmed { phone: Phone },
    /// Paired; connecting for Calls.
    Connecting { phone: Phone },
    /// The Phone refused hands-free: "Phone calls" is off for the Desktop.
    PhoneCallsOff { phone: Phone },
    /// The Phone didn't answer: out of range, or its Bluetooth is off.
    PhoneUnreachable { phone: Phone },
    /// Connected for Calls; the Phone is about to ask whether to share
    /// Contacts.
    ContactsWarning { phone: Phone },
    /// The Phone is asking whether to share Contacts, until `deadline`.
    ContactsPrompt { phone: Phone, deadline: Timestamp },
    /// Nobody answered the Phone before its prompt timed out.
    ContactsPromptTimedOut { phone: Phone },
    /// Shared: the sync is reading. `call_history` once the newest Calls
    /// are read, `contacts` while Contacts are read, and `contacts_read`
    /// once they are, while older Calls still are.
    ContactsSyncing {
        phone: Phone,
        call_history: bool,
        contacts: Option<ReadingContacts>,
        contacts_read: bool,
    },
    /// The Phone left before Contacts were read; `back` once it is
    /// connected again, to carry on.
    ContactsPhoneAway { phone: Phone, back: bool },
    /// Finished: what became of Contacts, and the keybinding. `connected`
    /// while the Phone is.
    Done {
        phone: Phone,
        connected: bool,
        contacts: ContactsOutcome,
        keybinding: KeybindingStage,
    },
}

impl Setup {
    /// Whether the window is at its first step, choosing a Phone.
    pub(super) fn choosing(&self) -> bool {
        matches!(self.step, Step::Choose)
    }
}

impl SetupStep {
    /// The lower-case name the control interface's `Surfaces` and the
    /// journal use.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Choose { .. } => "choose",
            Self::Pairing { .. } => "pairing",
            Self::Confirm { .. } => "confirm",
            Self::Confirmed { .. } => "confirmed",
            Self::Connecting { .. } => "connecting",
            Self::PhoneCallsOff { .. } => "phone-calls-off",
            Self::PhoneUnreachable { .. } => "phone-unreachable",
            Self::ContactsWarning { .. } => "contacts-warning",
            Self::ContactsPrompt { .. } => "contacts-prompt",
            Self::ContactsPromptTimedOut { .. } => "contacts-prompt-timed-out",
            Self::ContactsSyncing { .. } => "contacts-syncing",
            Self::ContactsPhoneAway { .. } => "contacts-phone-away",
            Self::Done { .. } => "done",
        }
    }
}

impl Step {
    /// Whether pairing with `address` is under way, so its finishing means
    /// the Phone is paired.
    fn pairs(&self, address: &str) -> bool {
        matches!(self, Self::Pairing(phone) | Self::Confirm(phone, _) | Self::Confirmed(phone)
            if phone.address.eq_ignore_ascii_case(address))
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
            },
        };
        Some(SetupWindowView {
            step,
            desktop_name: setup.desktop_name.clone(),
            notice: setup.notice,
            opened: setup.opened,
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
            _ => ContactsOutcome::NotShared,
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
                open(state, effects);
            }
        }
        Input::OpenSetupWindow => open(state, effects),
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
                Some(known) => *known = phone,
                None => state.nearby_phones.push(phone),
            }
        }
        Input::PhoneNotNearby { address } => not_nearby(state, &address),
        Input::ChoosePhone { address } => choose(state, &address, effects),
        Input::PairingCode { phone, code } => pairing_code(state, phone, code, effects),
        Input::ConfirmPairing => answer_code(state, true, effects),
        Input::RejectPairing => answer_code(state, false, effects),
        Input::PairingFailed(failure) => pairing_failed(state, failure),
        Input::HandsFreeRefused => {
            if let Some(setup) = state.setup.as_mut()
                && let Step::AskedForHandsFree(phone) = &setup.step
            {
                setup.step = Step::PhoneCallsOff(phone.clone());
            }
        }
        Input::HandsFreeUnreachable => {
            if let Some(setup) = state.setup.as_mut()
                && let Step::AskedForHandsFree(phone) = &setup.step
            {
                setup.step = Step::PhoneUnreachable(phone.clone());
            }
        }
        Input::RetryHandsFree => retry(state, effects),
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
        input => keybinding_input(state, input, effects),
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

fn open(state: &mut State, effects: &mut Vec<Effect>) {
    if state.setup.is_some() {
        return;
    }
    state.setup_openings += 1;
    state.setup = Some(Setup {
        opened: state.setup_openings,
        ..Setup::default()
    });
    effects.push(Effect::StartSetup);
}

fn close(state: &mut State, effects: &mut Vec<Effect>) {
    let Some(setup) = state.setup.take() else {
        return;
    };
    if matches!(setup.step, Step::Confirm(..)) {
        effects.push(Effect::AnswerPairingCode { confirm: false });
    }
    effects.push(Effect::StopSetup);
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
        Step::Pairing(chosen) => chosen.address.eq_ignore_ascii_case(&phone.address),
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
    if !state
        .chosen_phone
        .as_ref()
        .is_some_and(|chosen| chosen.eq_ignore_ascii_case(&phone.address))
    {
        state.chosen_phone = Some(phone.address.clone());
        effects.push(Effect::KeepChosenPhone {
            address: phone.address.clone(),
        });
    }
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
    effects.push(Effect::StartTimer {
        timer: Timer::SetupStep(setup.wait),
        after_ms,
    });
}

/// After every step: the Phone being set up is connected for Calls once
/// hands-free is up, and the window warns that it is about to ask whether
/// to share Contacts.
pub(super) fn follow_connection(state: &mut State, effects: &mut Vec<Effect>) {
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

/// After every step: the sync the window follows is over once the Phone
/// refused, the sync failed, or it finished, and the window goes on to the
/// finish page. An unanswered prompt waits for Try again or Skip. If the
/// Phone leaves before Contacts are read, the window says so instead; once
/// they are, only older Calls are missing, and the window finishes.
pub(super) fn follow_contacts(state: &mut State, effects: &mut Vec<Effect>) {
    let here = state.phone_is_here();
    let sync = state.contacts_sync;
    let Some(setup) = state.setup.as_mut() else {
        return;
    };
    let phone = match &setup.step {
        Step::ContactsWarning(phone) | Step::Contacts(phone) => phone.clone(),
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
    }
}

/// The finish page's keybinding: offered, shown, confirmed and written.
fn keybinding_input(state: &mut State, input: Input, effects: &mut Vec<Effect>) {
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

/// How far the sync under way has got, for the Setup Window to show.
pub(super) fn sync_progress(state: &mut State, progress: &SyncProgress) {
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
pub(super) fn contacts_read(state: &mut State) {
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
pub(super) fn hold_sync(state: &mut State) -> bool {
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
fn start_sync(state: &mut State, effects: &mut Vec<Effect>) {
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
fn begin_contacts(state: &mut State, phone: Phone, effects: &mut Vec<Effect>) {
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

fn finish(setup: &mut Setup, phone: Phone, effects: &mut Vec<Effect>) {
    setup.step = Step::Done(phone);
    setup.keybinding = KeybindingStage::Checking;
    setup.adding = false;
    effects.push(Effect::CheckKeybinding);
}
