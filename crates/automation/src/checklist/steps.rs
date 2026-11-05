//! The Real-Phone Checklist: spec #1's seven groups, with the additions from
//! tickets #28 to #31 and their comments. A step with a script is runnable;
//! a step without one names the tickets whose features it waits on. When
//! such a ticket lands, its implementer writes the step's script.
//!
//! Each script observes through the control interface whatever a machine
//! can, and asks the user only what it cannot.

use std::time::{Duration, Instant};

use super::sitting::{Seen, Sitting, Stop, seconds};
use super::watch::{AppMissedCall, AppState};

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
                script: None,
                lands_with: &[17],
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

/// Whether the user has pressed Answer or Decline on the Desktop: the
/// Desktop stops ringing while the Phone still rings.
fn handled_on_desktop(state: &AppState, call: u32) -> bool {
    state
        .call(call)
        .is_some_and(|known| known.state == "ringing" && !state.ringing)
}

/// Whether the Call has stopped ringing on the Phone: answered or gone.
fn stopped_ringing(state: &AppState, call: u32) -> bool {
    state
        .call(call)
        .is_none_or(|known| known.state != "ringing")
}

/// Waits until the Call's audio reaches the Desktop after Answer at
/// `answered`, and checks the audio latency and the codec.
fn audio_reaches_the_desktop(
    sitting: &mut Sitting,
    call: u32,
    answered: &Seen,
) -> Result<bool, Stop> {
    let audio = sitting.wait_for(
        "the Call audio reaching the Desktop",
        APP_DEADLINE,
        |state| state.audio_location == "desktop" || state.call(call).is_none(),
    )?;
    if audio.state.audio_location != "desktop" {
        sitting.check(
            false,
            "The Call ended before its audio reached the Desktop.".to_owned(),
        );
        return Ok(false);
    }
    let latency = audio.at - answered.at;
    sitting.measure("audio_latency_ms", millis(latency));
    sitting.check(
        latency <= AUDIO_TARGET,
        format!(
            "Audio on the Desktop {} after Answer (target {}).",
            seconds(latency),
            seconds(AUDIO_TARGET)
        ),
    );
    let codec = if audio.state.codec.is_empty() {
        sitting
            .wait_for("the codec being reported", APP_DEADLINE, |state| {
                !state.codec.is_empty()
            })?
            .state
    } else {
        audio.state
    };
    sitting.measure("codec", codec.codec.clone());
    sitting.measure("wideband", codec.wideband);
    sitting.observed(format!(
        "Codec: {} ({}).",
        codec.codec,
        if codec.wideband {
            "wideband"
        } else {
            "narrowband"
        }
    ));
    Ok(true)
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

/// 2.1: the helper calls, the user answers on the Desktop. Observes the
/// Banner latency, the audio latency, the codec and the hang-up path; asks
/// whether the helper heard clearly.
fn answer_on_the_desktop(sitting: &mut Sitting) -> Result<(), Stop> {
    let started_us = super::results::unix_micros();
    let call = helper_calls(sitting, "Ask your helper to call you. Don't answer yet.")?;
    sitting.say("Click Answer on the Banner.", &["answer"]);
    let answered = sitting.wait_for("Answer on the Desktop", ACT_DEADLINE, |state| {
        handled_on_desktop(state, call) || stopped_ringing(state, call)
    })?;
    if !handled_on_desktop(&answered.state, call) {
        sitting.check(
            false,
            "The Call stopped ringing without Answer on the Desktop (answered on the Pixel, or ended).".to_owned(),
        );
        return Ok(());
    }
    if !audio_reaches_the_desktop(sitting, call, &answered)? {
        return Ok(());
    }
    let heard =
        sitting.ask("Does your helper hear you clearly, with no echo of their own voice?")?;
    sitting.check(
        heard,
        if heard {
            "Your helper heard you clearly, with no echo.".to_owned()
        } else {
            "Your helper did not hear you clearly, or heard an echo.".to_owned()
        },
    );
    call_ends(sitting, call, "Hang up from the Call Window.", &["hang-up"])?;
    let fallback = sitting
        .log_since(started_us)
        .iter()
        .any(|line| line.event_kind == "phone-command-fallback");
    if fallback {
        sitting.measure("hang_up", "org.ofono.VoiceCall");
        sitting.observed(
            "Hang-up: Call1.Hangup doesn't exist here, so the org.ofono.VoiceCall fallback hung up.".to_owned(),
        );
    } else {
        sitting.measure("hang_up", "Call1");
        sitting.observed(
            "Hang-up: no fallback was logged, so Call1.Hangup worked, if you hung up on the Desktop.".to_owned(),
        );
    }
    Ok(())
}

/// 2.2: answered on the Pixel, the Banner closes and the audio stays there.
fn answer_on_the_pixel(sitting: &mut Sitting) -> Result<(), Stop> {
    let call = helper_calls(sitting, "Ask your helper to call you.")?;
    sitting.say(
        "Answer on the Pixel, not on the Desktop.",
        &["answer-on-phone"],
    );
    let answered = sitting.wait_for("the Call being answered", ACT_DEADLINE, |state| {
        handled_on_desktop(state, call) || stopped_ringing(state, call)
    })?;
    if handled_on_desktop(&answered.state, call) {
        sitting.check(
            false,
            "The Call was handled on the Desktop, not on the Pixel.".to_owned(),
        );
        return Ok(());
    }
    if answered.state.call(call).is_none() {
        sitting.check(false, "The Call ended without being answered.".to_owned());
        return Ok(());
    }
    let closed = sitting.wait_for("the Banner closing", APP_DEADLINE, |state| {
        !state.shows("banner")
    })?;
    sitting.check(
        true,
        format!(
            "The Banner closed {} after the Pixel answered.",
            seconds(closed.at - answered.at)
        ),
    );
    let moved =
        sitting.happens_within("the audio moving to the Desktop", STAYS_ON_PHONE, |state| {
            state.audio_location == "desktop"
        })?;
    sitting.check(
        moved.is_none(),
        if moved.is_none() {
            format!(
                "The audio stayed on the Pixel for {} after answering.",
                seconds(STAYS_ON_PHONE)
            )
        } else {
            "The audio moved to the Desktop, though the Call was answered on the Pixel.".to_owned()
        },
    );
    call_ends(
        sitting,
        call,
        "Hang up, on either device.",
        &["hang-up-on-phone"],
    )?;
    Ok(())
}

/// 2.3: declined on the Desktop, the Call ends unanswered.
fn decline_on_the_desktop(sitting: &mut Sitting) -> Result<(), Stop> {
    let call = helper_calls(sitting, "Ask your helper to call you.")?;
    sitting.say("Click Decline on the Banner.", &["decline"]);
    let declined = sitting.wait_for("Decline on the Desktop", ACT_DEADLINE, |state| {
        handled_on_desktop(state, call) || stopped_ringing(state, call)
    })?;
    if !handled_on_desktop(&declined.state, call) {
        sitting.check(
            false,
            "The Call stopped ringing without Decline on the Desktop.".to_owned(),
        );
        return Ok(());
    }
    let ended = sitting.wait_for("the declined Call ending", APP_DEADLINE, |state| {
        stopped_ringing(state, call)
    })?;
    let unanswered = ended.state.call(call).is_none();
    sitting.check(
        unanswered,
        if unanswered {
            format!(
                "The Call ended {} after Decline, unanswered.",
                seconds(ended.at - declined.at)
            )
        } else {
            "The Call was answered instead of ending.".to_owned()
        },
    );
    if !unanswered {
        call_ends(sitting, call, "Hang up, on either device.", &["hang-up"])?;
    }
    Ok(())
}

/// 2.4: declined on the Pixel, the Banner closes, and no Missed Call is
/// notified: the Pixel's missed Call History, refreshed as the Call ends,
/// doesn't list a Call it declined.
fn decline_on_the_pixel(sitting: &mut Sitting) -> Result<(), Stop> {
    let call = helper_calls(sitting, "Ask your helper to call you.")?;
    sitting.say(
        "Decline on the Pixel, not on the Desktop.",
        &["decline-on-phone"],
    );
    let declined = sitting.wait_for("Decline on the Pixel", ACT_DEADLINE, |state| {
        handled_on_desktop(state, call) || stopped_ringing(state, call)
    })?;
    if handled_on_desktop(&declined.state, call) {
        sitting.check(
            false,
            "The Call was handled on the Desktop, not on the Pixel.".to_owned(),
        );
        return Ok(());
    }
    if declined.state.call(call).is_some() {
        sitting.check(
            false,
            "The Call was answered instead of declined.".to_owned(),
        );
        call_ends(
            sitting,
            call,
            "Hang up, on either device.",
            &["hang-up-on-phone"],
        )?;
        return Ok(());
    }
    let closed = sitting.wait_for("the Banner closing", APP_DEADLINE, |state| {
        !state.shows("banner")
    })?;
    sitting.check(
        true,
        format!(
            "The Call ended unanswered and the Banner closed {} after the Pixel declined.",
            seconds(closed.at - declined.at)
        ),
    );
    let refresh = call_history_refresh(sitting, &declined)?;
    let notified =
        sitting.happens_within("a Missed Call notification", NOTIFY_WINDOW, |state| {
            state.missed_call(call).is_some()
        })?;
    sitting.check(
        notified.is_none(),
        match (notified.is_none(), refresh) {
            (true, Refresh::Done) => {
                "No Missed Call was notified: the Pixel's missed Call History, \
                                      refreshed as the Call ended, doesn't list it."
                    .to_owned()
            }
            (true, Refresh::Failed | Refresh::Unseen) => "No Missed Call was notified.".to_owned(),
            (false, Refresh::Done) => "A Missed Call was notified: the Pixel lists a Call it \
                                       declined among its missed Calls."
                .to_owned(),
            (false, Refresh::Failed) => "A Missed Call was notified, because the Call History \
                                         couldn't be refreshed to tell a declined Call apart."
                .to_owned(),
            (false, Refresh::Unseen) => "A Missed Call was notified.".to_owned(),
        },
    );
    Ok(())
}

/// How long after the Call History refresh a Missed Call notification may
/// take to show.
const NOTIFY_WINDOW: Duration = Duration::from_secs(2);
/// How long the Call History refresh may take after a Call ends: one PBAP
/// pull from the Phone.
const REFRESH_DEADLINE: Duration = Duration::from_secs(30);

/// How the Call History refresh that follows a Call went, as the app's log
/// tells it.
#[derive(Clone, Copy)]
enum Refresh {
    Done,
    Failed,
    /// The log said neither within [`REFRESH_DEADLINE`].
    Unseen,
}

/// Waits for the app to log how the Call History refresh after `ended`
/// went.
fn call_history_refresh(sitting: &mut Sitting, ended: &Seen) -> Result<Refresh, Stop> {
    let since = unix_micros_at(ended.at).saturating_sub(LOG_LOOKBACK_US);
    let end = Instant::now() + REFRESH_DEADLINE;
    loop {
        let logged =
            sitting
                .log_since(since)
                .into_iter()
                .find_map(|line| match line.event_kind.as_str() {
                    "call-history-refreshed" => Some(Refresh::Done),
                    "call-history-refresh-failed" => Some(Refresh::Failed),
                    _ => None,
                });
        if let Some(refresh) = logged {
            return Ok(refresh);
        }
        if Instant::now() >= end {
            return Ok(Refresh::Unseen);
        }
        // Nothing to wait for on the control interface: this only passes
        // the time while still reading what the user types.
        sitting.happens_within("the Call History refresh", LOG_POLL, |_| false)?;
    }
}

/// How often to read the app's log while waiting for a line.
const LOG_POLL: Duration = Duration::from_millis(250);

/// Waits for the helper's Call to ring out, then for its Missed Call
/// notification. Returns the Missed Call, or `None` if the step can't go
/// on, with the reason in the findings.
fn rings_out(sitting: &mut Sitting, call: u32) -> Result<Option<AppMissedCall>, Stop> {
    sitting.say(
        "Don't answer or decline, on either device: let it ring until it stops.",
        &["ring-out"],
    );
    let ended = sitting.wait_for("the Call ringing out", TALK_DEADLINE, |state| {
        stopped_ringing(state, call)
    })?;
    if ended.state.call(call).is_some() {
        sitting.check(
            false,
            "The Call was answered instead of ringing out.".to_owned(),
        );
        call_ends(sitting, call, "Hang up, on either device.", &["hang-up"])?;
        return Ok(None);
    }
    let refresh = call_history_refresh(sitting, &ended)?;
    let notified =
        sitting.happens_within("the Missed Call notification", NOTIFY_WINDOW, |state| {
            state.missed_call(call).is_some()
        })?;
    let Some(missed) = notified
        .as_ref()
        .and_then(|seen| seen.state.missed_call(call).cloned())
    else {
        sitting.check(
            false,
            match refresh {
                Refresh::Done => "No Missed Call was notified: the Pixel's missed Call History, \
                                  refreshed as the Call ended, doesn't list it."
                    .to_owned(),
                Refresh::Failed | Refresh::Unseen => "No Missed Call was notified.".to_owned(),
            },
        );
        return Ok(None);
    };
    if let Some(seen) = &notified {
        let after = seen.at - ended.at;
        sitting.measure("missed_call_notified_ms", millis(after));
        sitting.check(
            true,
            format!(
                "A Missed Call was notified {} after the Call stopped ringing{}.",
                seconds(after),
                match refresh {
                    Refresh::Done => ", confirmed against the Pixel's missed Call History",
                    Refresh::Failed => ", unconfirmed: the Call History couldn't be refreshed",
                    Refresh::Unseen => "",
                }
            ),
        );
    }
    Ok(Some(missed))
}

/// Asks the user to click the Missed Call notification, and waits for the
/// Dialer to open with keyboard focus. Returns the Dialer's selected row,
/// or `None` if it didn't open, with the reason in the findings.
fn notification_opens_the_dialer(sitting: &mut Sitting) -> Result<Option<String>, Stop> {
    sitting.say(
        "Click the notification itself, not one of its buttons.",
        &["click-notification"],
    );
    let opened = sitting.happens_within("the Dialer opening", ACT_DEADLINE, |state| {
        state.focused("dialer")
    })?;
    let selected = opened
        .as_ref()
        .and_then(|seen| seen.state.dialer_selection())
        .map(str::to_owned);
    sitting.check(
        selected.is_some(),
        if selected.is_some() {
            "Clicking the notification opened the Dialer with keyboard focus.".to_owned()
        } else {
            "Clicking the notification didn't open the Dialer with a row selected.".to_owned()
        },
    );
    Ok(selected)
}

/// 2.5: a Call that rings out is notified as missed; clicking the
/// notification opens the Dialer on it, and Enter calls back.
fn ring_out(sitting: &mut Sitting) -> Result<(), Stop> {
    let call = helper_calls(
        sitting,
        "Ask your helper to call you, and to let it ring until it stops.",
    )?;
    let Some(missed) = rings_out(sitting, call)? else {
        return Ok(());
    };
    let shown = sitting.ask(
        "Does a notification say you missed a call from your helper, by name or number, with the time?",
    )?;
    sitting.check(
        shown,
        if shown {
            "The notification names the caller and gives the time.".to_owned()
        } else {
            "The notification didn't name the caller with the time.".to_owned()
        },
    );
    let Some(selected) = notification_opens_the_dialer(sitting)? else {
        return Ok(());
    };
    // The row reads the caller's name or number, which stay out of the
    // result file.
    let on_the_missed_call = selected.contains("Missed");
    sitting.check(
        on_the_missed_call,
        if on_the_missed_call {
            "The Dialer opened with the Missed Call selected.".to_owned()
        } else {
            "The Dialer opened with another row than the Missed Call selected.".to_owned()
        },
    );
    sitting.say("Press Enter.", &["enter", "phone-dials"]);
    let placed = sitting.happens_within("the Call back", ACT_DEADLINE, |state| {
        state
            .calls
            .iter()
            .any(|known| known.direction == "outgoing")
    })?;
    let Some(back) = placed.and_then(|seen| {
        seen.state
            .calls
            .iter()
            .find(|known| known.direction == "outgoing")
            .cloned()
    }) else {
        sitting.check(false, "Enter placed no Call.".to_owned());
        return Ok(());
    };
    let same_number = !missed.withheld && back.number == missed.number;
    sitting.check(
        same_number,
        if same_number {
            "Enter called back the number that rang.".to_owned()
        } else {
            "Enter called a different number from the one that rang.".to_owned()
        },
    );
    call_ends(
        sitting,
        back.id,
        "Your helper's phone rings: hang up, on either device.",
        &["hang-up"],
    )?;
    Ok(())
}

/// 2.6: a withheld number rings as "Withheld number", and ringing out is
/// notified as missed; the Dialer says it can't be called back, and Enter
/// on it explains instead of calling.
fn withheld_ring_out(sitting: &mut Sitting) -> Result<(), Stop> {
    let call = helper_calls_as(
        sitting,
        "Ask your helper to call you with their number withheld: on most phones, #31# before \
         the number. They let it ring until it stops.",
        "call-withheld",
    )?;
    let caller = sitting
        .now()
        .surfaces
        .get("banner")
        .and_then(|banner| banner.get("caller"))
        .cloned()
        .unwrap_or_default();
    let shows_withheld = caller == "Withheld number";
    sitting.check(
        shows_withheld,
        if shows_withheld {
            "The Banner showed \"Withheld number\".".to_owned()
        } else if sitting.now().call(call).is_none() {
            "The Call stopped ringing before the runner read the Banner.".to_owned()
        } else {
            // A number in the evidence would be the helper's: say only
            // that one was shown.
            "The Banner showed a caller, not \"Withheld number\": the helper's number wasn't \
             withheld."
                .to_owned()
        },
    );
    let Some(missed) = rings_out(sitting, call)? else {
        return Ok(());
    };
    sitting.check(
        missed.withheld,
        if missed.withheld {
            "The Missed Call was notified as from a withheld number.".to_owned()
        } else {
            "The Missed Call was notified with a number.".to_owned()
        },
    );
    let shown = sitting.ask(
        "Does a notification say you missed a call from a withheld number, and that it can't be called back?",
    )?;
    sitting.check(
        shown,
        if shown {
            "The notification says the withheld number can't be called back.".to_owned()
        } else {
            "The notification didn't say the withheld number can't be called back.".to_owned()
        },
    );
    let Some(selected) = notification_opens_the_dialer(sitting)? else {
        return Ok(());
    };
    let says_so = selected.contains("Withheld number") && selected.contains("can't be called back");
    sitting.check(
        says_so,
        if says_so {
            format!("The Dialer opened on the Missed Call, reading {selected:?}.")
        } else {
            "The Dialer's selected row isn't the withheld Missed Call.".to_owned()
        },
    );
    sitting.say("Press Enter.", &["enter"]);
    let answered = sitting.happens_within("the Dialer explaining", ACT_DEADLINE, |state| {
        dialer_notice(state).is_some()
            || state
                .calls
                .iter()
                .any(|known| known.direction == "outgoing")
    })?;
    let notice = answered
        .as_ref()
        .and_then(|seen| dialer_notice(&seen.state))
        .map(str::to_owned);
    let dialled = answered.as_ref().is_some_and(|seen| {
        seen.state
            .calls
            .iter()
            .any(|known| known.direction == "outgoing")
    });
    sitting.check(
        notice.is_some() && !dialled,
        match (&notice, dialled) {
            (Some(notice), false) => format!("Enter explained instead of calling: {notice:?}"),
            (_, true) => "Enter placed a Call, though the number was withheld.".to_owned(),
            (None, false) => "Enter neither explained nor called.".to_owned(),
        },
    );
    if dialled {
        no_call_left(sitting)?;
    }
    sitting.say("Press Escape to close the Dialer.", &["escape"]);
    sitting.wait_for("the Dialer closing", ACT_DEADLINE, |state| {
        !state.shows("dialer")
    })?;
    Ok(())
}

/// The Dialer's notice, such as why a row can't be called, while it shows
/// one.
fn dialer_notice(state: &AppState) -> Option<&str> {
    state
        .surfaces
        .get("dialer")
        .and_then(|dialer| dialer.get("notice"))
        .map(String::as_str)
}

/// 2.8: the keybinding focuses the Banner, Escape gives focus back, A
/// answers and, on a second Call, D declines.
fn keybinding(sitting: &mut Sitting) -> Result<(), Stop> {
    let call = helper_calls(sitting, "Ask your helper to call you. Don't answer yet.")?;
    sitting.say("Press your Desktop Calls keybinding.", &["keybinding"]);
    let focused = sitting.wait_for("the Banner taking keyboard focus", ACT_DEADLINE, |state| {
        state.focused("banner") || stopped_ringing(state, call)
    })?;
    sitting.check(
        focused.state.focused("banner"),
        "The keybinding gave the Banner keyboard focus.".to_owned(),
    );
    sitting.say("Press Escape.", &["escape"]);
    let released = sitting.wait_for(
        "the Banner giving keyboard focus back",
        ACT_DEADLINE,
        |state| !state.focused("banner") || stopped_ringing(state, call),
    )?;
    sitting.check(
        released.state.shows("banner") && !released.state.focused("banner"),
        "Escape gave keyboard focus back, and the Banner stayed.".to_owned(),
    );
    sitting.say("Press the keybinding again, then A.", &["keybinding", "a"]);
    let answered = sitting.wait_for("A answering on the Desktop", ACT_DEADLINE, |state| {
        handled_on_desktop(state, call) || stopped_ringing(state, call)
    })?;
    if !handled_on_desktop(&answered.state, call) {
        sitting.check(
            false,
            "The Call stopped ringing without A answering it on the Desktop.".to_owned(),
        );
        return Ok(());
    }
    let audio = sitting.wait_for(
        "the Call audio reaching the Desktop",
        APP_DEADLINE,
        |state| state.audio_location == "desktop" || state.call(call).is_none(),
    )?;
    sitting.check(
        audio.state.audio_location == "desktop",
        "A answered the Call on the Desktop.".to_owned(),
    );
    call_ends(sitting, call, "Hang up from the Call Window.", &["hang-up"])?;

    let call = helper_calls(sitting, "Ask your helper to call you again.")?;
    sitting.say("Press the keybinding, then D.", &["keybinding", "d"]);
    let declined = sitting.wait_for("D declining", ACT_DEADLINE, |state| {
        handled_on_desktop(state, call) || stopped_ringing(state, call)
    })?;
    if !handled_on_desktop(&declined.state, call) {
        sitting.check(
            false,
            "The Call stopped ringing without D declining it on the Desktop.".to_owned(),
        );
        return Ok(());
    }
    let ended = sitting.wait_for("the declined Call ending", APP_DEADLINE, |state| {
        stopped_ringing(state, call)
    })?;
    sitting.check(
        ended.state.call(call).is_none(),
        "D declined the Call.".to_owned(),
    );
    Ok(())
}

/// 3.2: mute and unmute on a Call answered on the Desktop; the helper says
/// whether they heard silence, which decides #51.
fn mute(sitting: &mut Sitting) -> Result<(), Stop> {
    let call = helper_calls(sitting, "Ask your helper to call you.")?;
    sitting.say("Click Answer on the Banner.", &["answer"]);
    let answered = sitting.wait_for("Answer on the Desktop", ACT_DEADLINE, |state| {
        handled_on_desktop(state, call) || stopped_ringing(state, call)
    })?;
    if !handled_on_desktop(&answered.state, call) {
        sitting.check(
            false,
            "The Call stopped ringing without Answer on the Desktop.".to_owned(),
        );
        return Ok(());
    }
    if !audio_reaches_the_desktop(sitting, call, &answered)? {
        return Ok(());
    }
    sitting.say(
        "Click Mute in the Call Window, then keep talking to your helper.",
        &["mute"],
    );
    let muted = sitting.wait_for("Mute", ACT_DEADLINE, |state| {
        state.muted || state.call(call).is_none()
    })?;
    if !muted.state.muted {
        sitting.check(false, "The Call ended before Mute.".to_owned());
        return Ok(());
    }
    sitting.observed("The Desktop muted its microphone.".to_owned());
    let silent = sitting.ask("While you were muted, did your helper hear silence?")?;
    sitting.check(
        silent,
        if silent {
            "Your helper heard silence while you were muted.".to_owned()
        } else {
            "Your helper still heard you while muted: the Pixel ignores MicrophoneVolume 0, so #51 is needed.".to_owned()
        },
    );
    sitting.say("Click Unmute.", &["unmute"]);
    let unmuted = sitting.wait_for("Unmute", ACT_DEADLINE, |state| {
        !state.muted || state.call(call).is_none()
    })?;
    if unmuted.state.call(call).is_none() {
        sitting.check(false, "The Call ended before Unmute.".to_owned());
        return Ok(());
    }
    let heard = sitting.ask("Does your helper hear you again?")?;
    sitting.check(
        heard,
        if heard {
            "Your helper heard you again after Unmute.".to_owned()
        } else {
            "Your helper still couldn't hear you after Unmute.".to_owned()
        },
    );
    call_ends(sitting, call, "Hang up from the Call Window.", &["hang-up"])?;
    Ok(())
}

/// 3.3: keypad tones from the Call Window reach a voicemail menu.
fn keypad_tones(sitting: &mut Sitting) -> Result<(), Stop> {
    no_call_left(sitting)?;
    // The Simulated Phone can't yet place a Call from the Phone's side.
    sitting.say(
        "On the Pixel, call your voicemail, or any number whose menu answers keypad tones.",
        &["skip"],
    );
    let connected = sitting.wait_for("your Call connecting", CALL_DEADLINE, |state| {
        state
            .calls
            .iter()
            .any(|call| call.direction != "incoming" && call.state == "active")
    })?;
    let Some(call) = connected
        .state
        .calls
        .iter()
        .find(|call| call.state == "active")
        .map(|call| call.id)
    else {
        return Err(Stop::TimedOut("your Call connecting".to_owned()));
    };
    sitting.observed("Your Call from the Pixel connected.".to_owned());
    sitting.say(
        "When the menu asks, click Keypad in the Call Window and press the digits it wants.",
        &[],
    );
    let answered = sitting.ask("Did the menu respond to the tones?")?;
    sitting.check(
        answered,
        if answered {
            "The menu responded to the keypad tones.".to_owned()
        } else {
            "The menu did not respond to the keypad tones.".to_owned()
        },
    );
    call_ends(sitting, call, "Hang up.", &["hang-up"])?;
    Ok(())
}

/// 7.2: the Banner beside a real notification, neither covering the other.
fn banner_beside_a_notification(sitting: &mut Sitting) -> Result<(), Stop> {
    let call = helper_calls(sitting, "Ask your helper to call you. Don't answer yet.")?;
    match sitting.notify(
        "A sample notification",
        "Shown by the Real-Phone Checklist, to compare with the Incoming Call Banner.",
    ) {
        Ok(()) => sitting.observed("Sent a sample notification while the Banner shows.".to_owned()),
        Err(error) => {
            sitting.check(
                false,
                format!("Couldn't show a sample notification: {error}"),
            );
            return Ok(());
        }
    }
    let clear = sitting.ask(
        "Do the Banner and the notification both show in full, with neither covering the other?",
    )?;
    sitting.check(
        clear,
        if clear {
            "The Banner and the notification were both in full view.".to_owned()
        } else {
            "The Banner and the notification overlapped.".to_owned()
        },
    );
    call_ends(
        sitting,
        call,
        "Decline the Call, on either device.",
        &["decline"],
    )?;
    Ok(())
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
