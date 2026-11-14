//! The Real-Phone Checklist: spec #1's seven groups, with the additions from
//! tickets #28 to #31 and their comments. A step with a script is runnable;
//! a step without one names the tickets whose features it waits on. When
//! such a ticket lands, its implementer writes the step's script.
//!
//! Each script observes through the control interface whatever a machine
//! can, and asks the user only what it cannot.

use std::time::{Duration, Instant};

use super::app_log::LogLine;
use super::sitting::{Seen, Sitting, Stop, seconds};
use super::watch::AppState;

mod audio_location;
mod calls_off_call;
mod during_a_call;
mod first_run;
mod incoming;
mod keyboard_and_look;
mod missed_calls;
mod range_and_power;

use audio_location::{earbuds_answer_on_the_desktop, earbuds_answer_on_the_earbuds, handoff_twice};
use calls_off_call::call_while_calls_off;
use during_a_call::{call_pill, calls_off_during_a_call, keypad_tones, mute};
use first_run::{forget_and_install, pair_to_first_call, prompt_times_out_then_allowed};
use incoming::{
    answer_on_the_desktop, answer_on_the_pixel, decline_on_the_desktop, decline_on_the_pixel,
    do_not_disturb, keybinding,
};
use keyboard_and_look::{
    banner_over_a_notification, dialer_first_frame, indicator_in_the_bar, keyboard_only_call,
};
use missed_calls::{ring_out, withheld_ring_out};
use range_and_power::{
    calls_off_until_past_a_suspend, calls_off_until_turned_on, out_of_range_and_back,
    quiet_hours_soon, suspend_and_resume, wireplumber_restart,
};

/// A step's script. It returns once the step is done; whether it passed is
/// in the sitting's findings.
pub type Script = fn(&mut Sitting) -> Result<(), Stop>;

pub struct Group {
    pub number: u8,
    pub title: &'static str,
    pub steps: &'static [Step],
}

pub struct Step {
    pub id: &'static str,
    /// What the user needs for this step, beyond the Pixel connected. Before
    /// a group starts, the runner lists what the steps about to run need.
    pub needs: &'static [&'static str],
    pub title: &'static str,
    pub script: Option<Script>,
    /// The tickets whose features the step waits on, while it has no script.
    pub lands_with: &'static [u32],
}

/// Steps without a script of their own that run only together with
/// another step, as (step, the step it runs with). None does now: 1.1,
/// which leaves the Phone unpaired, has its script, and 1.2 after it pairs
/// it again. `tools/checklist_tickets.py` reads this table.
pub const RUNS_WITH: &[(&str, &str)] = &[];

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

/// What a step says before it has the user click in the Call Window, which
/// collapses to the Call Pill about 5 s after a Call connects.
const EXPAND_FIRST: &str = "If the Call Window has collapsed to the Call Pill in the bottom-right corner, click the Call Pill or press your keybinding to expand it.";

/// What steps need, as the runner lists it before a group starts.
const HELPER: &str = "A helper with a second phone who can call you.";
const TEST_NUMBER: &str = "The Test Number already given to the agent.";
const TWO_HELPERS: &str = "Two helpers: one to take your Call on the Test Number's phone, and one with another phone who can call you during it.";
const THIRD_PHONE: &str = "A third phone, worked by the second helper or someone else, that can call you while you are on two Calls.";
const PIXEL_IN_HAND: &str =
    "The Pixel unlocked and in hand: it may ask to allow access to contacts and call history.";
const NOTIFICATIONS: &str = "Omarchy's notifications showing: Do Not Disturb off.";
const KEYBINDING: &str = "A Hyprland keybinding that runs `desktop-calls`.";
const KEYPAD_MENU: &str =
    "A number with a keypad menu you can call from the Pixel, such as your voicemail.";
const EARBUDS: &str = "The earbuds paired to the Pixel.";
const NO_OTHER_AUDIO: &str = "No other audio playing, because 6.3 restarts WirePlumber.";

pub const GROUPS: &[Group] = &[
    Group {
        number: 1,
        title: "Fresh first run, timed to the first Call",
        steps: &[
            Step {
                id: "1.1",
                needs: &[],
                title: "Forget the pairing on both the Pixel and the Desktop, then install and start Desktop Calls",
                script: Some(forget_and_install),
                lands_with: &[],
            },
            Step {
                id: "1.2",
                needs: &[TEST_NUMBER, PIXEL_IN_HAND],
                title: "Pair through the Setup Window, allow Contacts access, add the keybinding, and place a first Call to the Test Number (timed)",
                script: Some(pair_to_first_call),
                lands_with: &[],
            },
            Step {
                id: "1.3",
                needs: &[PIXEL_IN_HAND],
                title: "Turn Contacts access off on the Pixel, use Try again and let the prompt time out once, then allow it",
                script: Some(prompt_times_out_then_allowed),
                lands_with: &[],
            },
            Step {
                id: "1.4",
                needs: &[PIXEL_IN_HAND],
                title: "Read a second page of Call History: the Pixel honours paging",
                script: Some(call_history_paging),
                lands_with: &[],
            },
        ],
    },
    Group {
        number: 2,
        title: "Incoming Calls",
        steps: &[
            Step {
                id: "2.1",
                needs: &[HELPER],
                title: "Answer on the Desktop",
                script: Some(answer_on_the_desktop),
                lands_with: &[],
            },
            Step {
                id: "2.2",
                needs: &[HELPER],
                title: "Answer on the Pixel: the Banner closes and the audio stays on the Pixel",
                script: Some(answer_on_the_pixel),
                lands_with: &[],
            },
            Step {
                id: "2.3",
                needs: &[HELPER],
                title: "Decline on the Desktop",
                script: Some(decline_on_the_desktop),
                lands_with: &[],
            },
            Step {
                id: "2.4",
                needs: &[HELPER],
                title: "Decline on the Pixel: the Banner closes",
                script: Some(decline_on_the_pixel),
                lands_with: &[],
            },
            Step {
                id: "2.5",
                needs: &[HELPER, NOTIFICATIONS],
                title: "Let it ring out: the Missed Call notification appears; click it, then press Enter to call back",
                script: Some(ring_out),
                lands_with: &[],
            },
            Step {
                id: "2.6",
                needs: &[HELPER, NOTIFICATIONS],
                title: "The helper calls with their number withheld (for example #31#), and you let it ring out",
                script: Some(withheld_ring_out),
                lands_with: &[],
            },
            Step {
                id: "2.7",
                needs: &[HELPER, NOTIFICATIONS],
                title: "Turn Do Not Disturb on and receive two Calls: the Banner shows and the Desktop stays silent; decline the first on the Desktop and answer the second on the Pixel",
                script: Some(do_not_disturb),
                lands_with: &[],
            },
            Step {
                id: "2.8",
                needs: &[HELPER, KEYBINDING],
                title: "The keybinding focuses the Banner, Escape gives focus back, A answers and D declines",
                script: Some(keybinding),
                lands_with: &[],
            },
        ],
    },
    Group {
        number: 3,
        title: "During a Call",
        steps: &[
            Step {
                id: "3.1",
                needs: &[HELPER],
                title: "Handoff to the Phone and back, twice: confirm by ear that the audio lands on the earpiece",
                script: Some(handoff_twice),
                lands_with: &[],
            },
            Step {
                id: "3.2",
                needs: &[HELPER],
                title: "Mute and unmute: the helper hears silence while muted",
                script: Some(mute),
                lands_with: &[],
            },
            Step {
                id: "3.3",
                needs: &[KEYPAD_MENU],
                title: "Send keypad tones to a voicemail menu",
                script: Some(keypad_tones),
                lands_with: &[],
            },
            Step {
                id: "3.4",
                needs: &[HELPER, KEYBINDING],
                title: "On a real Call, the Call Window collapses to the Call Pill about 5 s after it connects; click the Call Pill to expand it where the Call Pill was, in the bottom-right corner; press Escape to collapse it; press the keybinding to expand it again",
                script: Some(call_pill),
                lands_with: &[],
            },
            Step {
                id: "3.5",
                needs: &[HELPER],
                title: "During a Call with the helper, turn calls off from the Indicator: the Call carries on; when the helper hangs up, the Pixel disconnects within a few seconds",
                script: Some(calls_off_during_a_call),
                lands_with: &[],
            },
        ],
    },
    Group {
        number: 4,
        title: "Call waiting",
        steps: &[
            Step {
                id: "4.1",
                needs: &[TWO_HELPERS, TEST_NUMBER],
                title: "Call the Test Number from the Desktop while the second helper calls you: Hold & Answer, then Swap, then hang up one; the other stays live",
                script: None,
                lands_with: &[148],
            },
            Step {
                id: "4.2",
                needs: &[TWO_HELPERS, TEST_NUMBER],
                title: "Repeat with End & Answer: your Call ends and the second helper's connects",
                script: None,
                lands_with: &[148],
            },
            Step {
                id: "4.3",
                needs: &[TWO_HELPERS, TEST_NUMBER],
                title: "Repeat with Decline: only the second helper's Call ends",
                script: None,
                lands_with: &[148],
            },
            Step {
                id: "4.4",
                needs: &[TWO_HELPERS, TEST_NUMBER],
                title: "Repeat, leaving the second helper's Call to ring out: a Missed Call notification appears",
                script: None,
                lands_with: &[148],
            },
            Step {
                id: "4.5",
                needs: &[TWO_HELPERS, TEST_NUMBER, THIRD_PHONE],
                title: "Repeat 4.1 up to Hold & Answer, so the Test Number's Call is on hold and the second helper's live, then have the second helper call you from the third phone: the Banner offers only End & Answer and Decline, and End & Answer ends only the second helper's first Call while the Test Number's stays on hold. If the Test Number's Call comes back live instead, note it, and which Call the Call Window shows",
                script: None,
                lands_with: &[148],
            },
            Step {
                id: "4.6",
                needs: &[TWO_HELPERS, TEST_NUMBER, THIRD_PHONE],
                title: "Then have the third phone hang up, leaving the Test Number's Call on hold alone, and have the second helper call again from their first phone: the Banner offers Answer and Decline, and Answer connects them while the Test Number's Call stays on hold. If the Test Number's Call comes back live when the third phone hangs up, or the new Call rings as an ordinary incoming Call, note it",
                script: None,
                lands_with: &[148],
            },
        ],
    },
    Group {
        number: 5,
        title: "Earbuds",
        steps: &[
            Step {
                id: "5.1",
                needs: &[HELPER, EARBUDS],
                title: "With the earbuds connected to the Pixel, answer on the Desktop and note where the audio goes",
                script: Some(earbuds_answer_on_the_desktop),
                lands_with: &[],
            },
            Step {
                id: "5.2",
                needs: &[HELPER, EARBUDS],
                title: "Repeat, answering on the earbuds",
                script: Some(earbuds_answer_on_the_earbuds),
                lands_with: &[],
            },
        ],
    },
    Group {
        number: 6,
        title: "Range and power",
        steps: &[
            Step {
                id: "6.1",
                needs: &[],
                title: "Take the Pixel out of range (or turn its Bluetooth off), then back: the runner times the reconnect",
                script: Some(out_of_range_and_back),
                lands_with: &[],
            },
            Step {
                id: "6.2",
                needs: &[],
                title: "Suspend the Desktop for a minute, then resume",
                script: Some(suspend_and_resume),
                lands_with: &[],
            },
            Step {
                id: "6.3",
                needs: &[NO_OTHER_AUDIO],
                title: "Restart WirePlumber while idle",
                script: Some(wireplumber_restart),
                lands_with: &[],
            },
            Step {
                id: "6.4",
                needs: &[],
                title: "Turn calls off from the Indicator until you turn them on: the Phone stays disconnected through its Bluetooth going off and on and an app restart; turn calls on and the runner times the reconnect",
                script: Some(calls_off_until_turned_on),
                lands_with: &[],
            },
            Step {
                id: "6.5",
                needs: &[],
                title: "Run `desktop-calls calls off --until HH:MM` two minutes ahead, suspend the Desktop past that time, and resume: calls come back on at the resume, and the runner times the reconnect from it",
                script: Some(calls_off_until_past_a_suspend),
                lands_with: &[],
            },
            Step {
                id: "6.6",
                needs: &[],
                title: "Run `desktop-calls quiet-hours HH:MM-HH:MM` from one minute ahead to three minutes ahead: the Pixel disconnects when they begin, and the runner times the reconnect from their end; then run `desktop-calls quiet-hours off`",
                script: Some(quiet_hours_soon),
                lands_with: &[],
            },
            Step {
                id: "6.7",
                needs: &[TEST_NUMBER],
                title: "With calls off, open the Dialer, type the Test Number, and press Enter: the Pixel connects and the Call is placed; hang up, and the Pixel disconnects again within a few seconds with `desktop-calls status` still saying calls are off",
                script: Some(call_while_calls_off),
                lands_with: &[],
            },
        ],
    },
    Group {
        number: 7,
        title: "Keyboard and look",
        steps: &[
            Step {
                id: "7.1",
                needs: &[KEYBINDING, TEST_NUMBER],
                title: "Place and end a Call using only the keyboard: the keybinding expands the collapsed Call Window with the keyboard in it",
                script: Some(keyboard_only_call),
                lands_with: &[],
            },
            Step {
                id: "7.2",
                needs: &[HELPER, NOTIFICATIONS],
                title: "The Banner flush top right over a real notification: in full above it while it rings, and the notification in view once it goes",
                script: Some(banner_over_a_notification),
                lands_with: &[],
            },
            Step {
                id: "7.3",
                needs: &[HELPER],
                title: "The Indicator in Omarchy's bar: pinned, it opens the Dialer and its menu runs a Call",
                script: Some(indicator_in_the_bar),
                lands_with: &[],
            },
            Step {
                id: "7.4",
                needs: &[KEYBINDING],
                title: "The Dialer draws its first frame within 150 ms of being asked",
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

/// How often the runner reads the app's log while it waits on a line.
const LOG_POLL: Duration = Duration::from_millis(200);

/// Waits until the app logs a line after `from_us` that `wanted` accepts,
/// still reading skip and quit, and returns the first such line. `None` if
/// `deadline` passes, or `give_up` holds for the app's state, first. The
/// timeline's cursor stays put, so the step's next wait still sees every
/// state published meanwhile.
fn logged_since(
    sitting: &mut Sitting,
    from_us: u64,
    deadline: Duration,
    wanted: impl Fn(&LogLine) -> bool,
    give_up: impl Fn(&AppState) -> bool,
) -> Result<Option<LogLine>, Stop> {
    let end = Instant::now() + deadline;
    loop {
        let logged = sitting.log_since(from_us).into_iter().find(&wanted);
        if logged.is_some() || Instant::now() >= end || give_up(&sitting.now()) {
            return Ok(logged);
        }
        sitting.pause(LOG_POLL)?;
    }
}

/// A line logging the Phone's connection state changing to one `wanted`
/// accepts.
fn connection_logged(wanted: impl Fn(&str) -> bool) -> impl Fn(&LogLine) -> bool {
    move |line| {
        line.event_kind == "connection-state-changed"
            && line.connection_state.as_deref().is_some_and(&wanted)
    }
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
/// instead. The control interface publishes a Call's state before the
/// surfaces that follow from it (ticket #107), so a Call answered or
/// declined on the Pixel is already listed as such when its Banner closes.
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
