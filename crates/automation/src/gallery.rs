//! The screenshot gallery: every surface in every state, in a light and a
//! dark Desktop without Omarchy (stock Adwaita) and under a light and a dark
//! Omarchy theme, with an `index.html` for a screenshot-review agent.
//!
//! Each surface ticket adds its states to [`STATES`], or to a list of its
//! own in a module here, such as [`dialer::DIALER_STATES`], [`call_pill::CALL_PILL_STATES`] or
//! [`banner::BANNER_STATES`] or [`setup_window::SETUP_PROBLEM_STATES`]. The Indicator's
//! states, in [`indicator::INDICATOR_STATES`], are drawn as Omarchy's bar draws its
//! icon, light and dark, and its menu's, in [`indicator::MENU_STATES`], as
//! Omarchy's tray draws the menu: a stand-in. The Missed Call notification,
//! in [`notification::NOTIFICATION_STATES`], is drawn as Omarchy's card draws it:
//! a stand-in too.

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use test_harness::vcards::{self, CallKind};

mod banner;
mod call_pill;
mod dialer;
mod durations;
mod indicator;
mod look;
mod notification;
mod page;
mod quiet_hours_window;
mod refusals;
mod render;
mod setup_window;
mod shard;

use banner::{BANNER_STATES, names_the_keybinding, with_keybinding};
use call_pill::{CALL_PILL_STATES, expanded, hold_call_window_expanded};
use dialer::DIALER_STATES;
use durations::DURATION_STATES;
use indicator::{
    INDICATOR_STATES, INDICATOR_THEMES, IndicatorState, MENU_STATES, draw_indicator, indicator_path,
};
use look::{LOOKS, Look};
use notification::{draw_notifications, drawn_notifications};
use page::index_html;
use quiet_hours_window::QUIET_HOURS_WINDOW_STATES;
use refusals::REFUSAL_STATES;
use render::render_all;
use setup_window::{SETUP_CONTACTS_STATES, SETUP_PROBLEM_STATES};
pub use shard::Shard;
use test_harness::{AppState, BAR_COLOUR, Builder, Harness, Key, OmarchyTheme, Region, Screenshot};

/// How many temporary captures the muted Call Window's state has taken.
static BEFORE_KEYPAD_CAPTURES: AtomicUsize = AtomicUsize::new(0);

/// How long the app may take to reach a state.
const DEADLINE: Duration = Duration::from_secs(10);

/// Whether the Call Window's audio tile says `label`.
fn handoff_shows(state: &AppState, label: &str) -> bool {
    state
        .surface("call-window")
        .and_then(|window| window.get("handoff"))
        .is_some_and(|handoff| handoff == label)
}

/// One state the gallery shows.
struct State {
    /// The file name stem, such as `bare-output`.
    name: &'static str,
    /// The caption.
    title: &'static str,
    /// Whether the app still reports the state, for one that doesn't last:
    /// only a capture taken while it does is kept, and if none is, the run
    /// fails rather than show the next state under this caption.
    still: fn(&AppState) -> bool,
    /// Brings the scenario into the state.
    reach: fn(&Harness),
    /// Whether a capture under `look` shows the state yet.
    shown: fn(&Screenshot, Look) -> bool,
    /// Sets the scenario up before it starts.
    prepare: fn(Builder) -> Builder,
}

/// The Simulated Phone's name, for the Setup Window's states.
const PHONE_NAME: &str = "My Pixel";

/// A caller in the range Ofcom keeps for drama (07700 900xxx).
const NUMBER: &str = "+447700900123";
/// A caller with a long number, in the London range Ofcom keeps for drama
/// (020 7946 0xxx): "+44 20 7946 0123", 16 characters.
const LONG_NUMBER: &str = "+442079460123";
/// The caller of a Waiting Call.
const SECOND_NUMBER: &str = "+447700900456";
/// A third caller, while the first Call is on hold and the second live.
const THIRD_NUMBER: &str = "+447700900789";

const STATES: &[State] = &[
    State {
        name: "bare-output",
        title: "The bare output, with the Phone connected and no surface shown",
        still: |_| true,
        prepare: as_is,
        reach: connect,
        shown: |_, _| true,
    },
    State {
        name: "call-window-live",
        title: "The Call Window, answered here: who and the timer, the state line with HD, three tiles and Hang up",
        still: expanded,
        prepare: as_is,
        reach: |harness| {
            connect(harness);
            harness.simulated_phone().set_codec(2);
            let call = ring(harness);
            harness.answer(call);
            harness.wait_for_app(
                "the Call Window shows HD audio on this computer",
                DEADLINE,
                |state| {
                    state.audio_location == "desktop"
                        && state.surface("call-window").is_some_and(|window| {
                            window.contains_key("hd") && window.contains_key("handoff")
                        })
                },
            );
            hold_call_window_expanded(harness);
        },
        shown: |screenshot, look| shows(screenshot, look.red()) && !shows(screenshot, look.green()),
    },
    State {
        name: "call-window-muted",
        title: "The Call Window, muted: the Mute tile, the state line and the border take the warning colour, and nothing moves",
        still: expanded,
        prepare: as_is,
        reach: |harness| {
            live_here(harness);
            harness
                .set_muted(true)
                .unwrap_or_else(|error| panic!("mute: {error:?}"));
            harness.wait_for_app("the Call Window says muted", DEADLINE, |state| {
                state.muted
                    && state
                        .surface("call-window")
                        .and_then(|window| window.get("mute"))
                        .is_some_and(|mute| mute == "Unmute")
            });
            hold_call_window_expanded(harness);
        },
        // Without the keypad, the window is under 300 px tall.
        shown: |screenshot, look| {
            shows(screenshot, look.red())
                && shows(screenshot, look.warning())
                && app_region(screenshot).is_some_and(|window| window.bottom - window.top < 300.0)
        },
    },
    State {
        name: "call-window-keypad",
        title: "The Call Window with the keypad open and tones sent: the tone entry and the keypad above the tiles, so Hang up stays put",
        still: |state| {
            state.surface("call-window").is_some_and(|window| {
                window.contains_key("keypad")
                    && window.get("tones").map(String::as_str) == Some("123#")
            })
        },
        prepare: as_is,
        reach: |harness| {
            live_here(harness);
            // Click the window, to give it the keyboard, then show the
            // keypad and type tones. The screenshot and the click both wait
            // for the Call Window to be drawn (#53).
            // Named apart from the other looks' captures, which render at
            // the same time into the same directory.
            let unique = BEFORE_KEYPAD_CAPTURES.fetch_add(1, Ordering::Relaxed);
            let screenshot = harness.screenshot(&format!("call-window-before-keypad-{unique}"));
            let window = app_region(&screenshot);
            let _ = fs::remove_file(&screenshot.path);
            if let Some(window) = window {
                harness.pointer().click(window.left + 8.0, window.top + 8.0);
            }
            harness.keyboard().type_text("k123#");
            harness.wait_for_app("the keypad shows the tones", DEADLINE, |state| {
                state.surface("call-window").is_some_and(|window| {
                    window.contains_key("keypad")
                        && window.get("tones").is_some_and(|tones| tones == "123#")
                })
            });
        },
        // Once the keypad is drawn the window is over 300 px tall; without
        // it, about 220.
        shown: |screenshot, look| {
            shows(screenshot, look.red())
                && !shows(screenshot, look.warning())
                && app_region(screenshot).is_some_and(|window| window.bottom - window.top > 300.0)
        },
    },
    State {
        name: "call-window-audio-on-phone",
        title: "The Call Window, answered on the phone: Mute disabled, the state line says where to mute, and Move here is the one accent tile",
        still: expanded,
        prepare: as_is,
        reach: |harness| {
            connect(harness);
            let path = harness.simulated_phone().ring(NUMBER);
            harness.wait_for_app("the Banner shows", DEADLINE, |state| {
                state.surface("banner").is_some()
            });
            harness.simulated_phone().answer_on_phone(&path);
            harness.wait_for_app(
                "the Call Window offers to move the audio here",
                DEADLINE,
                |state| {
                    state
                        .surface("call-window")
                        .and_then(|window| window.get("handoff"))
                        .is_some_and(|handoff| handoff == "Move here")
                },
            );
            hold_call_window_expanded(harness);
        },
        shown: |screenshot, look| {
            shows(screenshot, look.red())
                && shows(screenshot, look.accent())
                && !shows(screenshot, look.green())
                && !shows(screenshot, look.warning())
        },
    },
    State {
        name: "call-window-moving-audio",
        title: "The Call Window while the audio moves here from the phone: the audio tile says so, disabled, with a bar filling along its foot",
        still: |state| expanded(state) && handoff_shows(state, "Moving…"),
        prepare: as_is,
        reach: |harness| {
            connect(harness);
            let path = harness.simulated_phone().ring(NUMBER);
            harness.wait_for_app("the Banner shows", DEADLINE, |state| {
                state.surface("banner").is_some()
            });
            harness.simulated_phone().answer_on_phone(&path);
            harness.wait_for_app(
                "the Call Window offers to move the audio here",
                DEADLINE,
                |state| handoff_shows(state, "Move here"),
            );
            // The Simulated Phone opens the audio link at once; a real one
            // takes seconds, which is what the bar is for.
            harness
                .simulated_phone()
                .set_activate_delay(Duration::from_millis(2_500));
            harness
                .move_audio("desktop")
                .unwrap_or_else(|error| panic!("move the audio here: {error:?}"));
            harness.wait_for_app("the audio tile says it moves", DEADLINE, |state| {
                handoff_shows(state, "Moving…")
            });
        },
        shown: |screenshot, look| shows(screenshot, look.red()) && shows(screenshot, look.accent()),
    },
    State {
        name: "call-window-held",
        title: "The Call Window with two Calls: the answered one live, the first on hold in the row above it",
        still: |_| true,
        prepare: as_is,
        reach: held,
        shown: |screenshot, look| shows(screenshot, look.red()) && !shows(screenshot, look.green()),
    },
    State {
        name: "call-window-held-third-call-waiting",
        title: "Three Calls: one live and one on hold, Swap gone and Hang up dimmed until the Waiting Call is answered or declined, and a Waiting Call's Banner with End & answer, Decline and the keybinding",
        still: |_| true,
        prepare: with_keybinding,
        reach: |harness| {
            held(harness);
            let _path = harness.simulated_phone().ring(THIRD_NUMBER);
            harness.wait_for_app(
                "the Call Window says why the Held Call has no Swap or Hang up",
                DEADLINE,
                |state| {
                    state
                        .surface("call-window")
                        .is_some_and(|window| window.contains_key("held-waiting-first"))
                        && state
                            .surface("banner")
                            .is_some_and(|banner| banner.contains_key("options"))
                        && names_the_keybinding(state)
                },
            );
        },
        // The Banner's ringing dot is the only green on the screen.
        shown: |screenshot, look| shows(screenshot, look.red()) && shows(screenshot, look.green()),
    },
    State {
        name: "call-window-calling",
        title: "The Call Window, placing a Call, before the other end rings: who, the state line and Hang up, nothing disabled",
        still: |_| true,
        prepare: as_is,
        reach: |harness| {
            connect(harness);
            dial(harness);
        },
        shown: |screenshot, look| shows(screenshot, look.red()) && !shows(screenshot, look.green()),
    },
    State {
        name: "call-window-ringing",
        title: "The Call Window, placing a Call, ringing at the other end: who, the state line and Hang up",
        still: |_| true,
        prepare: as_is,
        reach: |harness| {
            connect(harness);
            dial(harness);
            let _path = harness.simulated_phone().alert();
            wait_for_status(harness, "Ringing…");
        },
        shown: |screenshot, look| shows(screenshot, look.red()) && !shows(screenshot, look.green()),
    },
    State {
        name: "call-window-not-placed",
        title: "The Call Window, after the Phone couldn't place the Call: who, and why, as a notice",
        still: |_| true,
        prepare: as_is,
        reach: |harness| {
            connect(harness);
            harness.simulated_phone().refuse_next_dial("cme:30");
            assert!(
                harness.dial(NUMBER).is_err(),
                "the app placed the Call the Phone refused"
            );
            harness.wait_for_app("the Call Window says Call not placed", DEADLINE, |state| {
                state
                    .surface("call-window")
                    .and_then(|window| window.get("notice-heading"))
                    .is_some_and(|heading| heading == "Call not placed")
            });
        },
        // No Hang up: there is no Call to hang up.
        shown: without_controls,
    },
    State {
        name: "setup-window-choose",
        title: "The Setup Window on a fresh install, listing the Phone nearby",
        still: |_| true,
        prepare: fresh_install,
        reach: |harness| {
            setup_window_at(harness, "choose");
            harness.wait_for_app("the Phone is listed", DEADLINE, |state| {
                setup_window_shows(state, "phones", PHONE_NAME)
                    && state
                        .surface("setup-window")
                        .is_some_and(|window| window.contains_key("desktop"))
            });
        },
        shown: |screenshot, _| app_region(screenshot).is_some(),
    },
    State {
        name: "setup-window-confirm",
        title: "The Setup Window, showing the code to check against the Phone's",
        still: |_| true,
        prepare: fresh_install,
        reach: |harness| {
            setup_window_at(harness, "choose");
            harness.wait_for_app("the Phone is listed", DEADLINE, |state| {
                setup_window_shows(state, "phones", PHONE_NAME)
            });
            harness.keyboard().press(Key::Return);
            setup_window_at(harness, "confirm");
        },
        shown: |screenshot, _| app_region(screenshot).is_some(),
    },
    State {
        name: "setup-window-phone-calls-off",
        title: "The Setup Window, after the Phone refused hands-free: where to switch Phone calls on",
        still: |_| true,
        prepare: fresh_install,
        reach: |harness| {
            harness.simulated_phone().set_hands_free("refused");
            pair_through_setup_window(harness);
            setup_window_at(harness, "phone-calls-off");
        },
        shown: |screenshot, _| app_region(screenshot).is_some(),
    },
    State {
        name: "setup-window-contacts-prompt",
        title: "The Setup Window, counting down while the Phone asks whether to share Contacts",
        still: |state| setup_window_shows(state, "step", "contacts-prompt"),
        prepare: fresh_install,
        reach: |harness| {
            harness.simulated_phone().set_contacts_prompt("ask");
            pair_through_setup_window(harness);
            setup_window_at(harness, "contacts-prompt");
        },
        shown: |screenshot, _| app_region(screenshot).is_some(),
    },
    State {
        name: "setup-window-contacts-syncing",
        title: "The Setup Window, with recent calls read and Contacts still being read",
        // Only while Contacts are still being read, not in the moment
        // after they are and before the finish page.
        still: |state| {
            state
                .surface("setup-window")
                .and_then(|window| window.get("progress"))
                .is_some_and(|progress| progress.contains(" s left"))
        },
        prepare: fresh_install,
        reach: |harness| {
            let phone = harness.simulated_phone();
            phone.set_pbap_folder("pb", &vcards::contacts(312, 0));
            phone.set_pbap_folder("cch", &vcards::call_history(200));
            // At the Pixel's pace: 312 Contacts take 9.4 s.
            phone.set_pull_delay_per_vcard(Duration::from_millis(30), Duration::from_millis(5));
            pair_through_setup_window(harness);
            harness.wait_for_app("Contacts are being read", DEADLINE, |state| {
                state
                    .surface("setup-window")
                    .and_then(|window| window.get("progress"))
                    .is_some_and(|progress| progress.starts_with("✓ Recent calls\nContacts · 312"))
            });
        },
        shown: |screenshot, _| app_region(screenshot).is_some(),
    },
    State {
        name: "setup-window-done",
        title: "The Setup Window's finish page: what was synced, the first Call and the keybinding, with the config file named from `~`",
        still: |_| true,
        prepare: |builder| fresh_install(builder).config_under_home(),
        reach: |harness| {
            with_hyprland_config(harness);
            with_contacts(harness);
            pair_through_setup_window(harness);
            setup_window_at(harness, "done");
            harness.wait_for_app("the keybinding is offered", DEADLINE, |state| {
                setup_window_shows(state, "buttons", "Add keybinding\nDone")
            });
        },
        shown: |screenshot, _| app_region(screenshot).is_some(),
    },
    State {
        name: "setup-window-keybinding",
        title: "The Setup Window, showing the exact keybinding line before writing it, the path wrapping only after a `/`",
        still: |_| true,
        prepare: |builder| fresh_install(builder).config_under_home(),
        reach: |harness| {
            with_hyprland_config(harness);
            with_contacts(harness);
            pair_through_setup_window(harness);
            harness.wait_for_app("the keybinding is offered", DEADLINE, |state| {
                setup_window_shows(state, "buttons", "Add keybinding\nDone")
            });
            harness.keyboard().press(Key::Return);
            harness.wait_for_app("the line is shown", DEADLINE, |state| {
                setup_window_shows(state, "buttons", "Confirm\nCancel")
            });
        },
        shown: |screenshot, _| app_region(screenshot).is_some(),
    },
];

/// Omarchy's Hyprland config in the scenario's home, so the Setup Window
/// offers to add the keybinding to it.
fn with_hyprland_config(harness: &Harness) {
    let directory = harness.directory().join("config/hypr");
    fs::create_dir_all(&directory)
        .and_then(|()| {
            fs::write(
                directory.join("bindings.lua"),
                "-- Keep only your personal keybinding overrides here.\n",
            )
        })
        .unwrap_or_else(|error| panic!("write the Hyprland config: {error}"));
}

/// Contacts and recent Calls on the Phone, so the finish page reports
/// a sync the size of a real one.
fn with_contacts(harness: &Harness) {
    let phone = harness.simulated_phone();
    phone.set_pbap_folder("pb", &vcards::contacts(312, 0));
    phone.set_pbap_folder("cch", &vcards::call_history(200));
}

/// The Call History [`sync`] sets, newest first, dated from today so the
/// Dialer's time column shows each of its forms: today's time, a weekday
/// and time, and a date.
fn recent_calls() -> [String; 4] {
    let at = |days, time| format!("{}T{time}", vcards::days_ago(days));
    [
        vcards::call(CallKind::Missed, Some("+447700900999"), &at(0, "094000")),
        vcards::call(CallKind::Missed, None, &at(1, "181200")),
        vcards::call(CallKind::Dialed, Some(NUMBER), &at(2, "100000")),
        vcards::call(CallKind::Received, Some("+447700900001"), &at(9, "090000")),
    ]
}

/// Syncs Contacts and a Call History with a Missed Call from the Phone,
/// connected.
fn sync(harness: &Harness) {
    let phone = harness.simulated_phone();
    let mut contacts = String::new();
    for (name, number) in [
        ("José Smith", "+447700900001"),
        ("Alice Smith", "+447700900123"),
        ("Joanna Smithers", "+447700900002"),
    ] {
        contacts.push_str(&vcards::contact(name, &[("CELL", number)]));
    }
    contacts.push_str(&vcards::contacts(20, 1_000));
    phone.set_pbap_folder("pb", &contacts);
    phone.set_pbap_folder("cch", &recent_calls().concat());
    connect(harness);
    harness.wait_for_app("Contacts and Call History are synced", DEADLINE, |state| {
        state.contacts.state == "ready" && state.contacts.call_history == 4
    });
}

/// Asks the app to Call [`NUMBER`] and waits for "Calling…" and for the
/// Phone to be asked to dial, so it can `Alert`. The app's reply waits for
/// the Phone, so nothing waits for it.
fn dial(harness: &Harness) {
    let _reply = harness.start_dial(NUMBER);
    wait_for_status(harness, "Calling…");
    harness
        .simulated_phone()
        .wait_for_recorded("the Phone is asked to dial", DEADLINE, |calls| {
            calls.iter().any(|call| call.member == "Dial")
        });
}

fn wait_for_status(harness: &Harness, status: &str) {
    harness.wait_for_app(
        &format!("the Call Window says {status}"),
        DEADLINE,
        |state| {
            state
                .surface("call-window")
                .and_then(|window| window.get("state"))
                .is_some_and(|shown| shown == status)
        },
    );
}

/// The scenario as it is set up by default: the Phone paired.
fn as_is(builder: Builder) -> Builder {
    builder
}

/// A fresh install: the Phone nearby and not paired, and headphones nearby
/// too, so the Setup Window opens by itself.
fn fresh_install(builder: Builder) -> Builder {
    builder
        .phone_name(PHONE_NAME)
        .phone_unpaired()
        .headphones_nearby("WH-1000XM5")
}

fn setup_window_shows(state: &AppState, key: &str, value: &str) -> bool {
    state
        .surface("setup-window")
        .and_then(|window| window.get(key))
        .is_some_and(|shown| shown == value)
}

fn setup_window_at(harness: &Harness, step: &str) {
    harness.wait_for_app(
        &format!("the Setup Window is at step {step:?}"),
        DEADLINE,
        |state| setup_window_shows(state, "step", step),
    );
}

/// Chooses the Phone in the Setup Window and confirms its code.
fn pair_through_setup_window(harness: &Harness) {
    setup_window_at(harness, "choose");
    harness.wait_for_app("the Phone is listed", DEADLINE, |state| {
        setup_window_shows(state, "phones", PHONE_NAME)
    });
    harness.keyboard().press(Key::Return);
    setup_window_at(harness, "confirm");
    harness.keyboard().press(Key::Return);
}

fn connect(harness: &Harness) {
    harness.simulated_phone().connect();
    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
}

/// Rings the Phone, waits for the Banner and returns the Call's id.
fn ring(harness: &Harness) -> u32 {
    ring_from(harness, NUMBER)
}

/// Rings the Phone from `number`, waits for the Banner and returns the
/// Call's id.
fn ring_from(harness: &Harness, number: &str) -> u32 {
    let _path = harness.simulated_phone().ring(number);
    let state = harness.wait_for_app("the Banner shows", DEADLINE, |state| {
        state.surface("banner").is_some()
    });
    state.calls.first().map_or(0, |call| call.id)
}

/// Rings the Phone, answers here and waits until the Call is live.
fn on_a_call(harness: &Harness) {
    on_a_call_from(harness, NUMBER);
}

/// Rings the Phone from `number`, answers here and waits until the Call is
/// live.
fn on_a_call_from(harness: &Harness, number: &str) {
    let call = ring_from(harness, number);
    harness.answer(call);
    harness.wait_for_app("the Call is live", DEADLINE, |state| {
        state
            .surface("call-window")
            .is_some_and(|window| window.contains_key("handoff"))
    });
}

/// A Call answered here, with its audio here and Mute offered: the Call
/// Window opens, reading "Mute", before the audio reaches the Desktop, and
/// Mute is refused until it has.
fn live_here(harness: &Harness) {
    connect(harness);
    let call = ring(harness);
    harness.answer(call);
    harness.wait_for_app(
        "the audio is on the Desktop and the Call Window offers Mute",
        DEADLINE,
        |state| {
            state.audio_location == "desktop"
                && state
                    .surface("call-window")
                    .and_then(|window| window.get("mute-shortcut"))
                    .is_some()
        },
    );
}

/// A Waiting Call rings during the Call: waits for its Banner.
fn waiting(harness: &Harness) {
    let _path = harness.simulated_phone().ring(SECOND_NUMBER);
    harness.wait_for_app("the Waiting Call's Banner shows", DEADLINE, |state| {
        state
            .surface("banner")
            .is_some_and(|banner| banner.contains_key("options"))
    });
}

/// Two Calls: the answered one live, the first on hold.
fn held(harness: &Harness) {
    connect(harness);
    on_a_call(harness);
    waiting(harness);
    if let Err(error) = harness.hold_and_answer() {
        panic!("Hold & answer: {error:?}");
    }
    harness.wait_for_app("the first Call is on hold", DEADLINE, |state| {
        state
            .surface("call-window")
            .is_some_and(|window| window.contains_key("held-caller"))
            && state.surface("banner").is_none()
    });
}

/// Where the app's surfaces are: everything that is neither the black
/// background nor the bar stand-in.
fn app_region(screenshot: &Screenshot) -> Option<Region> {
    screenshot.region_where(|pixel| pixel != [0, 0, 0] && pixel != BAR_COLOUR)
}

/// Whether a surface is on screen with no red: no Hang up, or a dimmed one.
fn without_controls(screenshot: &Screenshot, look: Look) -> bool {
    app_region(screenshot).is_some() && !shows(screenshot, look.red())
}

/// Whether `colour`, give or take antialiasing, is on screen.
fn shows(screenshot: &Screenshot, colour: [u8; 3]) -> bool {
    screenshot
        .region_where(|pixel| {
            pixel
                .iter()
                .zip(colour)
                .all(|(channel, wanted)| channel.abs_diff(wanted) <= 16)
        })
        .is_some()
}

/// Whether the surface on screen is edged all round in `colour`: the
/// pixels of that colour reach each of its four sides, as a border does
/// and a button or focus ring inside it does not.
fn bordered(screenshot: &Screenshot, colour: [u8; 3]) -> bool {
    let Some(surface) = app_region(screenshot) else {
        return false;
    };
    let Some(edged) = screenshot.region_where(|pixel| {
        pixel
            .iter()
            .zip(colour)
            .all(|(channel, wanted)| channel.abs_diff(wanted) <= 16)
    }) else {
        return false;
    };
    let reaches = |edge: f64, side: f64| (edge - side).abs() <= 2.0;
    reaches(edged.left, surface.left)
        && reaches(edged.top, surface.top)
        && reaches(edged.right, surface.right)
        && reaches(edged.bottom, surface.bottom)
}

/// What `automation screenshot` renders, and where.
pub struct Options {
    /// Where the gallery goes. Default: `target/screenshots/gallery`.
    pub output: Option<PathBuf>,
    /// Only this state, keeping the rest of the gallery.
    pub only: Option<String>,
    /// How many times to render it, at least once.
    pub repeat: u32,
    /// How many states to render at a time. Default: one per CPU.
    pub workers: Option<u32>,
    /// Only this part of the states, for one of several CI runners.
    pub shard: Option<Shard>,
    /// Print the states' names, in order, and render nothing.
    pub list_states: bool,
    /// How long to wait between reaching each state and capturing it, as a
    /// slow runner might (#248).
    pub late: Option<Duration>,
    /// Only this look, by its name, as a PR's CI renders the gallery.
    pub look: Option<String>,
}

/// The states to render: `only`, or the states of `shard`, or every state.
/// None, having said why, if `only` names no state.
fn chosen_states(options: &Options) -> Option<Vec<&'static State>> {
    let Some(only) = &options.only else {
        let in_shard = |position: usize| options.shard.is_none_or(|shard| shard.holds(position));
        return Some(
            all_states()
                .enumerate()
                .filter(|(position, _)| in_shard(*position))
                .map(|(_, state)| state)
                .collect(),
        );
    };
    let state = all_states().find(|state| state.name == only);
    if state.is_none() {
        let names: Vec<&str> = all_states().map(|state| state.name).collect();
        eprintln!(
            "The gallery has no state {only:?}. Its states: {}.",
            names.join(", ")
        );
    }
    state.map(|state| vec![state])
}

/// The looks to render: `look`, or every look. None, having said why, if
/// `look` names no look.
fn chosen_looks(options: &Options) -> Option<Vec<Look>> {
    let looks: Vec<Look> = LOOKS
        .into_iter()
        .filter(|look| {
            options
                .look
                .as_ref()
                .is_none_or(|only| only == look.as_str())
        })
        .collect();
    if looks.is_empty() {
        let names: Vec<&str> = LOOKS.iter().map(|look| look.as_str()).collect();
        eprintln!(
            "The gallery has no look {:?}. Its looks: {}.",
            options.look.as_deref().unwrap_or_default(),
            names.join(", ")
        );
        return None;
    }
    Some(looks)
}

/// `automation screenshot`: renders the gallery, or with `only` one state
/// of it, `repeat` times, into `output` or `target/screenshots/gallery`.
pub fn run(options: &Options) -> ExitCode {
    if options.list_states {
        for state in all_states() {
            println!("{}", state.name);
        }
        return ExitCode::SUCCESS;
    }
    let Some(states) = chosen_states(options) else {
        return ExitCode::FAILURE;
    };
    let Some(looks) = chosen_looks(options) else {
        return ExitCode::FAILURE;
    };
    let output = if let Some(output) = options.output.clone() {
        output
    } else {
        let Some(target) = crate::target_directory() else {
            eprintln!("Can't find the target directory; pass --output.");
            return ExitCode::FAILURE;
        };
        let default = target.join("screenshots/gallery");
        // The default gallery is ours alone, so stale states go, unless
        // only one state is being redone.
        if options.only.is_none() {
            let _ = fs::remove_dir_all(&default);
        }
        default
    };
    if let Err(error) = fs::create_dir_all(&output) {
        eprintln!("Can't create {}: {error}", output.display());
        return ExitCode::FAILURE;
    }

    let shots: Vec<(&State, Look)> = states
        .iter()
        .flat_map(|state| looks.iter().map(|look| (*state, *look)))
        .collect();
    for repetition in 1..=options.repeat {
        render_all(&shots, &output, options.workers, options.late);
        if options.repeat > 1 {
            println!("Rendered {repetition} of {} times.", options.repeat);
        }
    }

    // The Indicator is drawn with the whole gallery; redoing one state
    // keeps it as an earlier run drew it.
    if options.only.is_none() {
        let drawings = INDICATOR_STATES
            .iter()
            .map(|state| (state, false))
            .chain(MENU_STATES.iter().map(|state| (state, true)));
        for (state, menu) in drawings {
            for theme in INDICATOR_THEMES {
                if let Err(error) = draw_indicator(state, theme, menu, &output) {
                    eprintln!("Can't draw the Indicator {}: {error}", state.name);
                    return ExitCode::FAILURE;
                }
            }
        }
        if let Err(error) = draw_notifications(&output) {
            eprintln!("Can't draw {error}");
            return ExitCode::FAILURE;
        }
    }

    // Every state rendered, now or by an earlier run.
    let shots: Vec<(&State, Look, PathBuf)> = all_states()
        .flat_map(|state| LOOKS.map(|look| (state, look)))
        .map(|(state, look)| {
            let path = output.join(format!("{}.png", file_stem(state, look)));
            (state, look, path)
        })
        .filter(|(_, _, path)| path.exists())
        .collect();
    let drawn =
        |states: &'static [IndicatorState]| -> Vec<(&IndicatorState, OmarchyTheme, PathBuf)> {
            states
                .iter()
                .flat_map(|state| INDICATOR_THEMES.map(|theme| (state, theme)))
                .map(|(state, theme)| (state, theme, indicator_path(state, theme, &output)))
                .filter(|(_, _, path)| path.exists())
                .collect()
        };
    let indicators = drawn(INDICATOR_STATES);
    let menus = drawn(MENU_STATES);

    let index = output.join("index.html");
    let notifications = drawn_notifications(&output);
    if let Err(error) = fs::write(
        &index,
        index_html(&shots, &indicators, &menus, &notifications),
    ) {
        eprintln!("Can't write {}: {error}", index.display());
        return ExitCode::FAILURE;
    }
    let states = all_states()
        .filter(|state| shots.iter().any(|(shot, _, _)| shot.name == state.name))
        .count();
    let indicator_states = indicators.len() / INDICATOR_THEMES.len();
    let menu_states = menus.len() / INDICATOR_THEMES.len();
    println!(
        "Gallery: {} screenshots of {states} state{}; \
         the Indicator in {indicator_states} state{} and its menu in {menu_states}, light and dark.",
        shots.len(),
        if states == 1 { "" } else { "s" },
        if indicator_states == 1 { "" } else { "s" }
    );
    println!("{}", index.display());
    ExitCode::SUCCESS
}

/// Every state the gallery shows, in order.
fn all_states() -> impl Iterator<Item = &'static State> {
    STATES
        .iter()
        .chain(DIALER_STATES)
        .chain(BANNER_STATES)
        .chain(DURATION_STATES)
        .chain(CALL_PILL_STATES)
        .chain(QUIET_HOURS_WINDOW_STATES)
        .chain(SETUP_PROBLEM_STATES)
        .chain(SETUP_CONTACTS_STATES)
        .chain(REFUSAL_STATES)
}

/// A state's file name under `look`, without `.png`.
fn file_stem(state: &State, look: Look) -> String {
    format!("{}-{}", state.name, look.as_str())
}
