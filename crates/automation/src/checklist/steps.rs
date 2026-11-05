//! The Real-Phone Checklist: spec #1's seven groups, with the additions from
//! tickets #28 to #31 and their comments. A step with a script is runnable;
//! a step without one names the tickets whose features it waits on. When
//! such a ticket lands, its implementer writes the step's script.
//!
//! Each script observes through the control interface whatever a machine
//! can, and asks the user only what it cannot.

use std::time::{Duration, Instant};

use super::sitting::{Seen, Sitting, Stop, seconds};
use super::watch::AppState;

mod during_a_call;
mod incoming;
mod keyboard_and_look;
mod missed_calls;

use during_a_call::{keypad_tones, mute};
use incoming::{
    answer_on_the_desktop, answer_on_the_pixel, decline_on_the_desktop, decline_on_the_pixel,
    do_not_disturb, keybinding,
};
use keyboard_and_look::{banner_beside_a_notification, dialer_first_frame, indicator_in_the_bar};
use missed_calls::{ring_out, withheld_ring_out};

/// A step's script. It returns once the step is done; whether it passed is
/// in the sitting's findings.
pub type Script = fn(&mut Sitting) -> Result<(), Stop>;

pub struct Group {
    pub number: u8,
    pub title: &'static str,
    /// What the user needs before the group starts.
    pub before: &'static [&'static str],
    pub steps: &'static [Step],
}

pub struct Step {
    pub id: &'static str,
    pub title: &'static str,
    pub script: Option<Script>,
    /// The tickets whose features the step waits on, while it has no script.
    pub lands_with: &'static [u32],
}

/// The polish scorecard's targets (`docs/wiki/polish-scorecard.md`).
const BANNER_TARGET: Duration = Duration::from_secs(1);
const AUDIO_TARGET: Duration = Duration::from_millis(1_500);
/// The Dialer's first frame after the keybinding: spec story 100, judged on
/// the user's Desktop (ticket #47).
const DIALER_TARGET: Duration = Duration::from_millis(150);
/// How long the app may take to log what it has just shown.
const LOG_DEADLINE: Duration = Duration::from_secs(2);

/// How long the helper may take to call.
const CALL_DEADLINE: Duration = Duration::from_secs(180);
/// How long the user may take to act on a ringing Call.
const ACT_DEADLINE: Duration = Duration::from_secs(120);
/// How long the user may take over a live Call before hanging up.
const TALK_DEADLINE: Duration = Duration::from_secs(300);
/// How long the app may take to show what follows from the Phone's news.
const APP_DEADLINE: Duration = Duration::from_secs(10);
/// How long the audio must stay on the Phone after answering there.
const STAYS_ON_PHONE: Duration = Duration::from_secs(3);

pub const GROUPS: &[Group] = &[
    Group {
        number: 1,
        title: "Fresh first run, timed to the first Call",
        before: &[
            "The Pixel charged and nearby.",
            "The Test Number already given to the agent.",
            "The Pixel unlocked and in hand for 1.4: it may ask to allow access to contacts and call history.",
        ],
        steps: &[
            Step {
                id: "1.1",
                title: "Forget the pairing on both the Pixel and the Desktop, then install and start Desktop Calls",
                script: None,
                lands_with: &[20, 21],
            },
            Step {
                id: "1.2",
                title: "Pair through the Setup Window, allow Contacts access, add the keybinding, and place a first Call to the Test Number (timed)",
                script: None,
                lands_with: &[21, 22, 10],
            },
            Step {
                id: "1.3",
                title: "Turn Contacts access off on the Pixel, use Try again and let the prompt time out once, then allow it",
                script: None,
                lands_with: &[22],
            },
            Step {
                id: "1.4",
                title: "Read a second page of Call History: the Pixel honours paging",
                script: Some(call_history_paging),
                lands_with: &[],
            },
        ],
    },
    Group {
        number: 2,
        title: "Incoming Calls",
        before: &[
            "The Pixel charged, nearby and paired with this Desktop.",
            "Desktop Calls running, with the Pixel connected.",
            "A helper with a second phone who can call you several times.",
            "A Hyprland keybinding that runs `desktop-calls` (the Setup Window offers one once #22 lands).",
            "Omarchy's Do Not Disturb off. Step 2.7 has you turn it on, then off again.",
        ],
        steps: &[
            Step {
                id: "2.1",
                title: "Answer on the Desktop",
                script: Some(answer_on_the_desktop),
                lands_with: &[],
            },
            Step {
                id: "2.2",
                title: "Answer on the Pixel: the Banner closes and the audio stays on the Pixel",
                script: Some(answer_on_the_pixel),
                lands_with: &[],
            },
            Step {
                id: "2.3",
                title: "Decline on the Desktop",
                script: Some(decline_on_the_desktop),
                lands_with: &[],
            },
            Step {
                id: "2.4",
                title: "Decline on the Pixel: the Banner closes",
                script: Some(decline_on_the_pixel),
                lands_with: &[],
            },
            Step {
                id: "2.5",
                title: "Let it ring out: the Missed Call notification appears; click it, then press Enter to call back",
                script: Some(ring_out),
                lands_with: &[],
            },
            Step {
                id: "2.6",
                title: "The helper calls with their number withheld (for example #31#), and you let it ring out",
                script: Some(withheld_ring_out),
                lands_with: &[],
            },
            Step {
                id: "2.7",
                title: "Turn Do Not Disturb on and receive a Call: the Banner shows and the Desktop stays silent",
                script: Some(do_not_disturb),
                lands_with: &[],
            },
            Step {
                id: "2.8",
                title: "The keybinding focuses the Banner, Escape gives focus back, A answers and D declines",
                script: Some(keybinding),
                lands_with: &[],
            },
        ],
    },
    Group {
        number: 3,
        title: "During a Call",
        before: &[
            "The Pixel charged, nearby and connected.",
            "A helper with a second phone who can call you.",
            "A number with a keypad menu you can call from the Pixel, such as your voicemail.",
        ],
        steps: &[
            Step {
                id: "3.1",
                title: "Handoff to the Phone and back, twice: confirm by ear that the audio lands on the earpiece",
                script: None,
                lands_with: &[13, 14],
            },
            Step {
                id: "3.2",
                title: "Mute and unmute: the helper hears silence while muted",
                script: Some(mute),
                lands_with: &[],
            },
            Step {
                id: "3.3",
                title: "Send keypad tones to a voicemail menu",
                script: Some(keypad_tones),
                lands_with: &[],
            },
        ],
    },
    Group {
        number: 4,
        title: "Call waiting",
        before: &[
            "A helper with a second phone who can call you.",
            "The Test Number already given to the agent.",
        ],
        steps: &[
            Step {
                id: "4.1",
                title: "Call the Test Number from the Desktop while the helper calls you: Hold & answer, then Swap, then hang up one",
                script: None,
                lands_with: &[10, 16],
            },
            Step {
                id: "4.2",
                title: "Repeat with End & answer",
                script: None,
                lands_with: &[10, 16],
            },
            Step {
                id: "4.3",
                title: "Repeat with Decline",
                script: None,
                lands_with: &[10, 16],
            },
        ],
    },
    Group {
        number: 5,
        title: "Earbuds",
        before: &[
            "The earbuds paired to the Pixel.",
            "A helper with a second phone who can call you.",
        ],
        steps: &[
            Step {
                id: "5.1",
                title: "With the earbuds connected to the Pixel, answer on the Desktop and note where the audio goes",
                script: None,
                lands_with: &[13],
            },
            Step {
                id: "5.2",
                title: "Repeat, answering on the earbuds",
                script: None,
                lands_with: &[13],
            },
        ],
    },
    Group {
        number: 6,
        title: "Range and power",
        before: &[
            "The Pixel charged and nearby. No helper needed.",
            "No other audio playing, because 6.3 restarts WirePlumber.",
        ],
        steps: &[
            Step {
                id: "6.1",
                title: "Take the Pixel out of range (or turn its Bluetooth off), then back: the runner times the reconnect",
                script: None,
                lands_with: &[19],
            },
            Step {
                id: "6.2",
                title: "Suspend the Desktop for a minute, then resume",
                script: None,
                lands_with: &[19],
            },
            Step {
                id: "6.3",
                title: "Restart WirePlumber while idle",
                script: None,
                lands_with: &[20],
            },
        ],
    },
    Group {
        number: 7,
        title: "Keyboard and look",
        before: &[
            "A helper with a second phone who can call you.",
            "Omarchy notifications showing: Do Not Disturb off.",
        ],
        steps: &[
            Step {
                id: "7.1",
                title: "Place and end a Call using only the keyboard",
                script: None,
                lands_with: &[10, 11],
            },
            Step {
                id: "7.2",
                title: "The Banner beside a real notification: neither covers the other",
                script: Some(banner_beside_a_notification),
                lands_with: &[],
            },
            Step {
                id: "7.3",
                title: "The Indicator in Omarchy's bar: pinned, it opens the Dialer and its menu runs a Call",
                script: Some(indicator_in_the_bar),
                lands_with: &[],
            },
            Step {
                id: "7.4",
                title: "The Dialer draws its first frame within 150 ms of the keybinding",
                script: Some(dialer_first_frame),
                lands_with: &[],
            },
        ],
    },
];

/// Every step id, in checklist order.
pub fn order() -> Vec<&'static str> {
    GROUPS
        .iter()
        .flat_map(|group| group.steps.iter().map(|step| step.id))
        .collect()
}

/// Makes sure no Call is left over from an earlier step.
fn no_call_left(sitting: &mut Sitting) -> Result<(), Stop> {
    if sitting.now().calls.is_empty() {
        return Ok(());
    }
    sitting.say(
        "A Call is still going. End it, on either device.",
        &["hang-up"],
    );
    sitting.wait_for("the last Call ending", TALK_DEADLINE, |state| {
        state.calls.is_empty()
    })?;
    Ok(())
}

/// The helper calls: waits for the Call to ring and the Banner to show,
/// and checks the Banner's latency. Returns the Call's id.
fn helper_calls(sitting: &mut Sitting, request: &str) -> Result<u32, Stop> {
    helper_calls_as(sitting, request, "call")
}

/// As [`helper_calls`], with `demo_action` ringing the Simulated Phone.
fn helper_calls_as(sitting: &mut Sitting, request: &str, demo_action: &str) -> Result<u32, Stop> {
    no_call_left(sitting)?;
    sitting.say(request, &[demo_action]);
    let rang = sitting.wait_for("the helper's Call ringing", CALL_DEADLINE, |state| {
        state.ringing_call().is_some()
    })?;
    let Some(call) = rang.state.ringing_call().map(|call| call.id) else {
        return Err(Stop::TimedOut("the helper's Call ringing".to_owned()));
    };
    let banner = sitting.wait_for("the Incoming Call Banner showing", APP_DEADLINE, |state| {
        state.shows("banner") || state.call(call).is_none()
    })?;
    if !banner.state.shows("banner") {
        return Err(Stop::TimedOut(
            "the Incoming Call Banner showing before the Call stopped ringing".to_owned(),
        ));
    }
    // The app logs `call-added` as soon as its Phone link sees the Call,
    // before the control interface lists it, so the latency counts from
    // there when the log has it.
    let banner_us = unix_micros_at(banner.at);
    let rang_us = unix_micros_at(rang.at);
    let seen_by_phone_link = sitting
        .log_since(rang_us.saturating_sub(LOG_LOOKBACK_US))
        .iter()
        .find(|line| line.event_kind == "call-added" && line.call_id == Some(u64::from(call)))
        .map(|line| line.at_us);
    let (latency, since) = match seen_by_phone_link {
        Some(added_us) => (
            Duration::from_micros(banner_us.saturating_sub(added_us)),
            "the Desktop first saw the Call",
        ),
        None => (
            banner.at - rang.at,
            "the Call appeared on the control interface (the log had no call-added line)",
        ),
    };
    sitting.measure("banner_latency_ms", millis(latency));
    sitting.check(
        latency <= BANNER_TARGET,
        format!(
            "Banner shown {} after {since} (target {}).",
            seconds(latency),
            seconds(BANNER_TARGET)
        ),
    );
    Ok(call)
}

/// How far before the Call appeared on the control interface to look for
/// its `call-added` log line.
const LOG_LOOKBACK_US: u64 = 5_000_000;

/// `at` in microseconds since the Unix epoch.
fn unix_micros_at(at: Instant) -> u64 {
    let ago = u64::try_from(at.elapsed().as_micros()).unwrap_or(u64::MAX);
    super::results::unix_micros().saturating_sub(ago)
}

/// Waits for the Call to end.
fn call_ends(
    sitting: &mut Sitting,
    call: u32,
    request: &str,
    actions: &[&str],
) -> Result<Seen, Stop> {
    sitting.say(request, actions);
    sitting.wait_for("the Call ending", TALK_DEADLINE, |state| {
        state.call(call).is_none()
    })
}

/// Whether the user has pressed Answer or Decline on the Desktop while the
/// Phone still rings.
///
/// Without Do Not Disturb, the Desktop stops ringing. `Ringing` is published
/// with the Call's state, so this can't be mistaken for the Pixel answering.
/// Under Do Not Disturb the Desktop never rings, so the Banner closing tells
/// instead. `Surfaces` follows the Call's state a moment later, so for that
/// moment a Call answered on the Pixel looks the same; 2.7, the one step run
/// under Do Not Disturb, asks only for Decline on the Desktop.
fn handled_on_desktop(state: &AppState, call: u32) -> bool {
    let still_ringing = state
        .call(call)
        .is_some_and(|known| known.state == "ringing");
    let desktop_let_go = if state.do_not_disturb {
        !state.shows("banner")
    } else {
        !state.ringing
    };
    still_ringing && desktop_let_go
}

/// Whether the Call has stopped ringing on the Phone: answered or gone.
fn stopped_ringing(state: &AppState, call: u32) -> bool {
    state
        .call(call)
        .is_none_or(|known| known.state != "ringing")
}

/// 1.4: `just pbap-probe` pulls a second page of `cch` (Offset 50), which
/// settles whether the Phone honours paging, as the Call History sync
/// assumes (#9). The probe keeps only the shape of what it reads.
fn call_history_paging(sitting: &mut Sitting) -> Result<(), Stop> {
    sitting.say(
        "Reading Contacts and Call History with `just pbap-probe`. If the Pixel asks to allow access to contacts and call history, allow it.",
        &[],
    );
    let report = match sitting.pbap_probe()? {
        Ok(report) => report,
        Err(error) => {
            sitting.check(false, error);
            return Ok(());
        }
    };
    let paging = &report["report"]["call history paging"];
    for (name, key) in [
        ("page_2_entries", "page 2 entries"),
        (
            "page_2_entries_also_on_page_1",
            "page 2 entries also on page 1",
        ),
        ("page_2_older_than_page_1", "page 2 older than page 1"),
        ("honours_offset", "honours offset"),
    ] {
        sitting.measure(name, paging[key].clone());
    }
    let verdict = paging["verdict"]
        .as_str()
        .unwrap_or("The probe's report has no paging verdict.")
        .to_owned();
    // The sync reads page after page from the newest Call, so both must hold.
    let honoured = paging["honours offset"] == true && paging["page 2 older than page 1"] != false;
    sitting.check(honoured, verdict);
    Ok(())
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
