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
//! 7. Done: the Phone is connected for Calls.

use super::dialling::phone_unpaired;
use super::input::{Effect, Input, PairingFailure, Timer};
use super::presence::connect_phone_now;
use super::{State, forget_phone_keeping_desktop, phone::phone_paired};
use crate::connection::{ConnectionState, Phone};

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
    Done(Phone),
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
    /// Connected for Calls.
    Done { phone: Phone },
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
            | Self::PhoneCallsOff(phone) => Some(phone),
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
                phones: self.nearby_phones.clone(),
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
            Step::Done(phone) => SetupStep::Done {
                phone: phone.clone(),
            },
        };
        Some(SetupWindowView {
            step,
            desktop_name: setup.desktop_name.clone(),
            notice: setup.notice,
            opened: setup.opened,
        })
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
        Input::RetryHandsFree => {
            if let Some(setup) = state.setup.as_mut()
                && let Step::PhoneCallsOff(phone) = &setup.step
            {
                let phone = phone.clone();
                ask_for_hands_free(setup, phone, effects);
            }
        }
        _ => {}
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
}

fn not_nearby(state: &mut State, address: &str) {
    state
        .nearby_phones
        .retain(|phone| !phone.address.eq_ignore_ascii_case(address));
}

fn choose(state: &mut State, address: &str, effects: &mut Vec<Effect>) {
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
    if setup.step != Step::Choose {
        return;
    }
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
        Step::Choose | Step::PhoneCallsOff(_) | Step::Done(_) => true,
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

/// After every step: the Phone being set up is done once it is connected
/// for Calls.
pub(super) fn follow_connection(state: &mut State) {
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
        setup.wait += 1;
        setup.step = Step::Done(followed);
    }
}
