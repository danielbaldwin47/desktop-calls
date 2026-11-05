//! The Desktop's own Problems, found by the setup checks: what the Desktop
//! needs for Calls that the Phone can't give it.
//!
//! - Bluetooth: off, or blocked by the Desktop's radio switch.
//! - Contacts support: the Desktop's Bluetooth file-transfer service, which
//!   syncs Contacts and Call History, isn't installed.
//! - Audio: the Desktop's audio isn't set up to connect hands-free to the
//!   Phone by itself, or to keep a Call's audio on the Phone until asked.
//! - BlueZ's own hands-free: an experimental part of BlueZ claims the
//!   hands-free link to the Phone, so Calls never reach Desktop Calls.
//!
//! The shell reports each check's finding as it changes. Bluetooth and
//! BlueZ's hands-free are followed as they change; Contacts support and
//! audio can't be, so while either is a Problem it is checked again every
//! few seconds, until it is put right however the user puts it right.
//!
//! Two can be put right from the Setup Window, after the user asks:
//! Bluetooth turned on, and audio set up.

use super::State;
use super::input::{Effect, Timer};
use super::setup;
use crate::messages::ProblemKind;

/// How often Problems that can't be followed as they change are checked
/// again.
const CHECK_AGAIN_MS: u32 = 3_000;

/// Whether the Desktop's Bluetooth is on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BluetoothPower {
    #[default]
    On,
    Off,
    /// Off, and kept off by the Desktop's radio switch: turning it on fails
    /// until it is unblocked.
    Blocked,
}

/// One setup check's finding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesktopCheck {
    Bluetooth(BluetoothPower),
    /// Whether the Desktop can sync Contacts and Call History from the
    /// Phone: its Bluetooth file-transfer service is installed.
    ContactsSupport(bool),
    /// Whether the Desktop's audio connects hands-free to the Phone by
    /// itself, and keeps a Call's audio on the Phone until asked.
    AudioSetUp(bool),
    /// Whether BlueZ's own experimental hands-free is on, competing with
    /// Desktop Calls for the Phone's hands-free link.
    BluezHandsFree(bool),
}

/// Something the Desktop puts right after the user asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesktopFix {
    TurnOnBluetooth,
    /// Set the Desktop's audio up for Calls, and restart it.
    SetUpAudio,
}

/// Why putting a Problem right didn't work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FixFailure {
    /// Bluetooth refused to turn on.
    BluetoothNotTurnedOn,
    /// The audio settings couldn't be saved.
    AudioNotSaved,
    /// The audio settings were saved, and the Desktop's audio didn't
    /// restart.
    AudioNotRestarted,
    /// Restarting the Desktop's audio would cut off the Call under way.
    CallInProgress,
}

/// What the setup checks found, and what is being put right.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct DesktopChecks {
    bluetooth: Option<BluetoothPower>,
    contacts_support: Option<bool>,
    audio_set_up: Option<bool>,
    bluez_hands_free: Option<bool>,
    fixing: Option<DesktopFix>,
    notice: Option<FixFailure>,
    /// The audio settings were saved but `WirePlumber` didn't restart, so
    /// they aren't in use yet: the Problem stands until a restart works.
    restart_pending: bool,
    /// A check-again timer is pending.
    checking_again: bool,
}

/// What the Setup Window shows of the Desktop's Problems.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetupProblemsView {
    /// The Desktop's Problems, the one to put right first first.
    pub problems: Vec<ProblemKind>,
    /// What is being put right.
    pub fixing: Option<DesktopFix>,
    /// Why putting a Problem right didn't work, until the user tries again.
    pub notice: Option<FixFailure>,
}

impl DesktopChecks {
    /// The Desktop's Problems, the one to put right first first: without
    /// Bluetooth nothing else matters, and without hands-free no Call
    /// reaches the Desktop.
    pub(super) fn problems(&self) -> Vec<ProblemKind> {
        let mut problems = Vec::new();
        match self.bluetooth {
            Some(BluetoothPower::Off) => problems.push(ProblemKind::BluetoothOff),
            Some(BluetoothPower::Blocked) => problems.push(ProblemKind::BluetoothBlocked),
            Some(BluetoothPower::On) | None => {}
        }
        if self.bluez_hands_free == Some(true) {
            problems.push(ProblemKind::BluezHandsFree);
        }
        if self.audio_set_up == Some(false) || self.restart_pending {
            problems.push(ProblemKind::AudioNotSetUp);
        }
        if self.contacts_support == Some(false) {
            problems.push(ProblemKind::ContactsSupportMissing);
        }
        problems
    }

    /// Whether a Problem stands that can't be followed as it changes.
    fn unfollowed_problem(&self) -> bool {
        self.contacts_support == Some(false) || self.audio_set_up == Some(false)
    }

    /// Whether Bluetooth is off or blocked.
    pub(super) fn bluetooth_off(&self) -> bool {
        matches!(
            self.bluetooth,
            Some(BluetoothPower::Off | BluetoothPower::Blocked)
        )
    }
}

impl State {
    /// What the Setup Window shows of the Desktop's Problems, while it is
    /// open and there are any.
    pub(super) fn setup_problems_view(&self) -> Option<SetupProblemsView> {
        self.setup.as_ref()?;
        let problems = self.desktop.problems();
        (!problems.is_empty()).then_some(SetupProblemsView {
            problems,
            fixing: self.desktop.fixing,
            notice: self.desktop.notice,
        })
    }
}

/// A setup check found `check`.
pub(super) fn checked(state: &mut State, check: DesktopCheck, effects: &mut Vec<Effect>) {
    let checks = &mut state.desktop;
    let bluetooth_was_off = checks.bluetooth_off();
    match check {
        DesktopCheck::Bluetooth(power) => {
            checks.bluetooth = Some(power);
            if checks.fixing == Some(DesktopFix::TurnOnBluetooth) && power == BluetoothPower::On {
                checks.fixing = None;
            }
        }
        DesktopCheck::ContactsSupport(installed) => checks.contacts_support = Some(installed),
        DesktopCheck::AudioSetUp(set_up) => checks.audio_set_up = Some(set_up),
        DesktopCheck::BluezHandsFree(on) => checks.bluez_hands_free = Some(on),
    }
    if checks.unfollowed_problem() && !checks.checking_again {
        checks.checking_again = true;
        effects.push(Effect::StartTimer {
            timer: Timer::CheckDesktop,
            after_ms: CHECK_AGAIN_MS,
        });
    }
    if bluetooth_was_off && !state.desktop.bluetooth_off() {
        setup::bluetooth_on(state, effects);
    }
}

/// The check-again timer fired.
pub(super) fn check_again(state: &mut State, effects: &mut Vec<Effect>) {
    state.desktop.checking_again = false;
    if state.desktop.unfollowed_problem() {
        effects.push(Effect::CheckDesktop);
    }
}

/// The user asked to put `problem` right. Only a standing Problem the
/// Desktop can put right itself is, once at a time.
pub(super) fn fix(state: &mut State, problem: ProblemKind, effects: &mut Vec<Effect>) {
    let fix = match problem {
        ProblemKind::BluetoothOff => DesktopFix::TurnOnBluetooth,
        ProblemKind::AudioNotSetUp => DesktopFix::SetUpAudio,
        _ => return,
    };
    let calls = !state.calls.is_empty();
    let checks = &mut state.desktop;
    if checks.fixing.is_some() || !checks.problems().contains(&problem) {
        return;
    }
    // Restarting the audio drops hands-free, and the Call's audio with it.
    if fix == DesktopFix::SetUpAudio && calls {
        checks.notice = Some(FixFailure::CallInProgress);
        return;
    }
    checks.fixing = Some(fix);
    checks.notice = None;
    effects.push(Effect::Fix(fix));
}

/// The Desktop did what `fix` asked: check again for the outcome.
pub(super) fn fix_done(state: &mut State, fix: DesktopFix, effects: &mut Vec<Effect>) {
    if state.desktop.fixing == Some(fix) {
        state.desktop.fixing = None;
    }
    if fix == DesktopFix::SetUpAudio {
        state.desktop.restart_pending = false;
    }
    effects.push(Effect::CheckDesktop);
}

/// Putting a Problem right didn't work: the Setup Window says why, and
/// the user may try again.
pub(super) fn fix_failed(state: &mut State, failure: FixFailure) {
    state.desktop.fixing = None;
    state.desktop.notice = Some(failure);
    if failure == FixFailure::AudioNotRestarted {
        state.desktop.restart_pending = true;
    }
}
