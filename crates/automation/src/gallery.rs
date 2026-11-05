//! The screenshot gallery: every surface in every state, in a light and a
//! dark Desktop without Omarchy (stock Adwaita) and under a light and a dark
//! Omarchy theme, with an `index.html` for a screenshot-review agent.
//!
//! Each surface ticket adds its states to [`STATES`]. The Indicator's
//! states, in [`INDICATOR_STATES`], are drawn as Omarchy's bar draws its
//! icon, light and dark.

use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use test_harness::vcards::{self, CallKind};
use test_harness::{
    AppState, BAR_COLOUR, Builder, Harness, Key, OmarchyTheme, Region, Screenshot, Theme,
    draw_tray_icon,
};

/// How long the app may take to reach a state.
const DEADLINE: Duration = Duration::from_secs(10);

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
/// A second caller, for call waiting.
const SECOND_NUMBER: &str = "+447700900456";
/// A third caller, while the first Call is on hold and the second live.
const THIRD_NUMBER: &str = "+447700900789";

/// The stock Answer button's green, the stock Decline and Hang up buttons'
/// red, and the orange that marks the microphone muted under every look.
const GREEN: [u8; 3] = [0x26, 0xa2, 0x69];
const RED: [u8; 3] = [0xc0, 0x1c, 0x28];
const ORANGE: [u8; 3] = [0xe6, 0x61, 0x00];

/// How the Desktop looks in one column of the gallery.
#[derive(Clone, Copy)]
enum Look {
    /// No Omarchy, light or dark as the settings portal says.
    Stock(Theme),
    Omarchy(OmarchyTheme),
}

const LOOKS: [Look; 4] = [
    Look::Stock(Theme::Light),
    Look::Stock(Theme::Dark),
    Look::Omarchy(OmarchyTheme::Light),
    Look::Omarchy(OmarchyTheme::Dark),
];

impl Look {
    /// Its name, for file names and captions.
    fn as_str(self) -> &'static str {
        match self {
            Self::Stock(theme) => theme.as_str(),
            Self::Omarchy(theme) => theme.as_str(),
        }
    }

    /// The Answer button's colour.
    fn green(self) -> [u8; 3] {
        match self {
            Self::Stock(_) => GREEN,
            Self::Omarchy(theme) => theme.colour("green"),
        }
    }

    /// The Decline and Hang up buttons' colour.
    fn red(self) -> [u8; 3] {
        match self {
            Self::Stock(_) => RED,
            Self::Omarchy(theme) => theme.colour("red"),
        }
    }

    /// Sets the Desktop up to look this way. Omarchy also tells the settings
    /// portal whether its theme is light or dark.
    fn builder(self) -> Builder {
        match self {
            Self::Stock(theme) => Harness::builder().theme(theme),
            Self::Omarchy(theme) => Harness::builder().theme(theme.mode()).omarchy_theme(theme),
        }
    }
}

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
        name: "incoming-call-banner",
        title: "The Incoming Call Banner, ringing",
        still: |_| true,
        prepare: as_is,
        reach: |harness| {
            connect(harness);
            ring(harness);
        },
        shown: |screenshot, look| shows(screenshot, look.green()) && shows(screenshot, look.red()),
    },
    State {
        name: "incoming-call-banner-identifying",
        title: "The Incoming Call Banner, ringing with no number yet, which may still come",
        // The Banner identifies the caller for only 2 s, so a capture on a
        // slow runner could show "Withheld number" under this caption.
        still: identifying,
        prepare: as_is,
        reach: |harness| {
            connect(harness);
            // A withheld number: the Banner identifies the caller for as long
            // as the Phone may still report one, longer than the
            // `late-line-identification` quirk's number takes.
            let _path = harness.simulated_phone().ring("");
            harness.wait_for_app(
                "the Banner is identifying the caller",
                DEADLINE,
                identifying,
            );
        },
        shown: |screenshot, look| shows(screenshot, look.green()) && shows(screenshot, look.red()),
    },
    State {
        name: "call-window-live",
        title: "The Call Window, answered here, with HD audio on this computer",
        still: |_| true,
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
                            window.contains_key("hd") && window.contains_key("audio-location")
                        })
                },
            );
        },
        shown: |screenshot, look| shows(screenshot, look.red()) && !shows(screenshot, look.green()),
    },
    State {
        name: "call-window-muted",
        title: "The Call Window, muted, with the keypad shown and tones sent",
        still: |_| true,
        prepare: as_is,
        reach: |harness| {
            connect(harness);
            let call = ring(harness);
            harness.answer(call);
            // The Call Window opens, reading "Mute", before the audio reaches
            // the Desktop, and Mute is refused until it has.
            harness.wait_for_app(
                "the audio is on the Desktop and the Call Window offers Mute",
                DEADLINE,
                |state| {
                    state.audio_location == "desktop"
                        && state.surface("call-window").is_some_and(|window| {
                            window.get("mute").is_some_and(|mute| mute == "Mute")
                                && !window.contains_key("mute-unavailable")
                        })
                },
            );
            harness
                .set_muted(true)
                .unwrap_or_else(|error| panic!("mute: {error:?}"));
            // Click the window, to give it the keyboard, then show the
            // keypad and type tones. The screenshot and the click both wait
            // for the muted Call Window to be drawn (#53).
            let screenshot = harness.screenshot("call-window-before-keypad");
            let window = app_region(&screenshot);
            let _ = fs::remove_file(&screenshot.path);
            if let Some(window) = window {
                harness.pointer().click(window.left + 8.0, window.top + 8.0);
            }
            harness.keyboard().type_text("k123#");
            harness.wait_for_app("the keypad shows the tones", DEADLINE, |state| {
                state.muted
                    && state.surface("call-window").is_some_and(|window| {
                        window.contains_key("keypad")
                            && window.get("tones").is_some_and(|tones| tones == "123#")
                    })
            });
        },
        // Once the keypad is drawn the window is over 300 px tall; without
        // it, about 220.
        shown: |screenshot, look| {
            shows(screenshot, look.red())
                && shows(screenshot, ORANGE)
                && app_region(screenshot).is_some_and(|window| window.bottom - window.top > 300.0)
        },
    },
    State {
        name: "call-window-mute-unavailable",
        title: "The Call Window, answered on the phone, so Mute is unavailable",
        still: |_| true,
        prepare: as_is,
        reach: |harness| {
            connect(harness);
            let path = harness.simulated_phone().ring(NUMBER);
            harness.wait_for_app("the Banner shows", DEADLINE, |state| {
                state.surface("banner").is_some()
            });
            harness.simulated_phone().answer_on_phone(&path);
            harness.wait_for_app(
                "the Call Window says why Mute is unavailable",
                DEADLINE,
                |state| {
                    state
                        .surface("call-window")
                        .is_some_and(|window| window.contains_key("mute-unavailable"))
                },
            );
        },
        shown: |screenshot, look| {
            shows(screenshot, look.red())
                && !shows(screenshot, look.green())
                && !shows(screenshot, ORANGE)
        },
    },
    State {
        name: "call-window-ended",
        title: "The Call Window, just after the Call ended",
        still: |_| true,
        prepare: as_is,
        reach: |harness| {
            connect(harness);
            let call = ring(harness);
            harness.answer(call);
            harness.wait_for_app("the Call is live", DEADLINE, |state| {
                state
                    .surface("call-window")
                    .is_some_and(|window| window.contains_key("audio-location"))
            });
            harness.hang_up(call);
            harness.wait_for_app("the Call Window says Call ended", DEADLINE, |state| {
                state
                    .surface("call-window")
                    .and_then(|window| window.get("status"))
                    .is_some_and(|status| status == "Call ended")
            });
        },
        // Hang up is dimmed once the Call has ended.
        shown: |screenshot, look| {
            !shows(screenshot, look.red()) && app_region(screenshot).is_some()
        },
    },
    State {
        name: "call-waiting-banner",
        title: "A Waiting Call: the Banner, with Hold & Answer, below the Call Window",
        still: |_| true,
        prepare: as_is,
        reach: |harness| {
            connect(harness);
            on_a_call(harness);
            waiting(harness);
        },
        shown: |screenshot, look| shows(screenshot, look.green()) && shows(screenshot, look.red()),
    },
    State {
        name: "call-window-held",
        title: "The Call Window with two Calls: the answered one live, the first on hold",
        still: |_| true,
        prepare: as_is,
        reach: |harness| {
            connect(harness);
            on_a_call(harness);
            waiting(harness);
            if let Err(error) = harness.hold_and_answer() {
                panic!("Hold & Answer: {error:?}");
            }
            harness.wait_for_app("the first Call is on hold", DEADLINE, |state| {
                state
                    .surface("call-window")
                    .is_some_and(|window| window.contains_key("held-caller"))
                    && state.surface("banner").is_none()
            });
        },
        shown: |screenshot, look| shows(screenshot, look.red()) && !shows(screenshot, look.green()),
    },
    State {
        name: "call-window-held-third-call-waiting",
        title: "Three Calls: one live and one on hold, neither with Hang up, and a Waiting Call's Banner with End & Answer and Decline only",
        still: |_| true,
        prepare: as_is,
        reach: |harness| {
            connect(harness);
            on_a_call(harness);
            waiting(harness);
            if let Err(error) = harness.hold_and_answer() {
                panic!("Hold & Answer: {error:?}");
            }
            harness.wait_for_app("the first Call is on hold", DEADLINE, |state| {
                state
                    .surface("call-window")
                    .is_some_and(|window| window.contains_key("held-caller"))
                    && state.surface("banner").is_none()
            });
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
                },
            );
        },
        // Decline is red; with no Hold & Answer, nothing is green.
        shown: |screenshot, look| shows(screenshot, look.red()) && !shows(screenshot, look.green()),
    },
    State {
        name: "call-window-connection-lost",
        title: "The Call Window, after the audio went to the phone and hands-free didn't come back",
        still: |_| true,
        prepare: as_is,
        reach: |harness| {
            connect(harness);
            let call = ring(harness);
            harness.answer(call);
            harness.wait_for_app("the audio is on the Desktop", DEADLINE, |state| {
                state.audio_location == "desktop"
            });
            harness
                .simulated_phone()
                .enable_quirk("connect-profile-fails");
            harness
                .move_audio("phone")
                .unwrap_or_else(|error| panic!("move the audio to the phone: {error:?}"));
            harness.wait_for_app(
                "the Call Window says the connection is lost",
                DEADLINE,
                |state| {
                    state
                        .surface("call-window")
                        .is_some_and(|window| window.contains_key("notice"))
                },
            );
        },
        shown: |screenshot, look| shows(screenshot, look.red()) && !shows(screenshot, look.green()),
    },
    State {
        name: "call-window-calling",
        title: "The Call Window, placing a Call, before the other end rings",
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
        title: "The Call Window, placing a Call, ringing at the other end",
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
        title: "The Call Window, after the Phone couldn't place the Call",
        still: |_| true,
        prepare: as_is,
        reach: |harness| {
            connect(harness);
            harness.simulated_phone().refuse_next_dial("cme:30");
            assert!(
                harness.dial(NUMBER).is_err(),
                "the app placed the Call the Phone refused"
            );
            wait_for_status(harness, "Call not placed");
        },
        // Hang up is dimmed: there is no Call to hang up.
        shown: |screenshot, look| {
            !shows(screenshot, look.red()) && app_region(screenshot).is_some()
        },
    },
    State {
        name: "dialer-call-history",
        title: "The Dialer, before typing: the Call History, newest first, Missed Calls marked",
        still: |_| true,
        prepare: as_is,
        reach: |harness| {
            sync(harness);
            open_dialer(harness, "");
        },
        shown: |screenshot, _| shows_dialer(screenshot),
    },
    State {
        name: "dialer-results",
        title: "The Dialer, searching for smith, the second match selected",
        still: |_| true,
        prepare: as_is,
        reach: |harness| {
            sync(harness);
            open_dialer(harness, "smith");
            harness.keyboard().press(Key::Down);
            harness.wait_for_app("the second row is selected", DEADLINE, |state| {
                dialer_shows(state, "selected", "1")
            });
        },
        shown: |screenshot, _| shows_dialer(screenshot),
    },
    State {
        name: "dialer-not-connected",
        title: "The Dialer, after Enter with the Phone out of range",
        still: |_| true,
        prepare: as_is,
        reach: |harness| {
            sync(harness);
            harness.simulated_phone().disconnect();
            harness.wait_for_app("the Phone is disconnected", DEADLINE, |state| {
                state.connection_state == "disconnected"
            });
            open_dialer(harness, "jose");
            harness.keyboard().press(Key::Return);
            harness.wait_for_app("the Dialer says why not", DEADLINE, |state| {
                state
                    .surface("dialer")
                    .is_some_and(|dialer| dialer.contains_key("notice"))
            });
        },
        shown: |screenshot, _| shows_dialer(screenshot),
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
        name: "setup-window-done",
        title: "The Setup Window, paired and connected for Calls",
        still: |_| true,
        prepare: fresh_install,
        reach: |harness| {
            pair_through_setup_window(harness);
            setup_window_at(harness, "done");
        },
        shown: |screenshot, _| app_region(screenshot).is_some(),
    },
];

/// One state of the Indicator the gallery shows.
struct IndicatorState {
    /// The file name stem, after `indicator-`.
    name: &'static str,
    /// The caption.
    title: &'static str,
    /// Brings the scenario into the state.
    reach: fn(&Harness),
    /// What the Indicator's tooltip says once it shows the state.
    tooltip: &'static str,
}

/// The Indicator is drawn in Omarchy's light and dark themes.
const INDICATOR_THEMES: [OmarchyTheme; 2] = [OmarchyTheme::Light, OmarchyTheme::Dark];

const INDICATOR_STATES: &[IndicatorState] = &[
    IndicatorState {
        name: "connected",
        title: "The Indicator, the Phone connected",
        reach: |harness| harness.simulated_phone().connect(),
        tooltip: "Simulated Pixel connected",
    },
    IndicatorState {
        name: "disconnected",
        title: "The Indicator, the Phone out of range",
        reach: |harness| {
            connect(harness);
            harness.simulated_phone().disconnect();
        },
        tooltip: "Simulated Pixel out of range",
    },
    IndicatorState {
        name: "ringing",
        title: "The Indicator, a Call ringing",
        reach: |harness| {
            connect(harness);
            let _path = harness.simulated_phone().ring(NUMBER);
        },
        tooltip: "Call from +44 7700 900123",
    },
    IndicatorState {
        name: "in-call",
        title: "The Indicator, in a Call",
        reach: |harness| {
            connect(harness);
            let _path = harness.simulated_phone().ring(NUMBER);
            let state = harness.wait_for_app("the Call rings", DEADLINE, |state| state.ringing);
            harness.answer(state.calls.first().map_or(0, |call| call.id));
        },
        tooltip: "In a Call with +44 7700 900123",
    },
    IndicatorState {
        name: "problem",
        title: "The Indicator, a Problem: Contacts not shared",
        reach: |harness| {
            harness.simulated_phone().set_contacts_prompt("deny");
            harness.simulated_phone().connect();
        },
        tooltip: "Simulated Pixel connected · Contacts not shared",
    },
];

/// Where the Indicator in `state` under `theme` is drawn.
fn indicator_path(
    state: &IndicatorState,
    theme: OmarchyTheme,
    output: &std::path::Path,
) -> PathBuf {
    output.join(format!("indicator-{}-{}.png", state.name, theme.as_str()))
}

/// Brings a scenario under `theme` into `state` and draws the Indicator
/// as Omarchy's bar shows it, in the theme's background and foreground,
/// at its `indicator_path`.
fn draw_indicator(
    state: &IndicatorState,
    theme: OmarchyTheme,
    output: &std::path::Path,
) -> Result<(), String> {
    let harness = Look::Omarchy(theme).builder().start();
    (state.reach)(&harness);
    let items = harness.tray().wait_for(
        &format!("the Indicator says {:?}", state.tooltip),
        DEADLINE,
        |items| {
            items
                .iter()
                .any(|item| item.tooltip.as_deref() == Some(state.tooltip))
        },
    );
    let path = indicator_path(state, theme, output);
    let item = items.first().ok_or("no tray item")?;
    draw_tray_icon(
        item,
        theme.colour("background"),
        theme.colour("foreground"),
        &path,
    )
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
    phone.set_pbap_folder(
        "cch",
        &[
            vcards::call(CallKind::Missed, Some("+447700900999"), "20261104T110000"),
            vcards::call(CallKind::Missed, None, "20261104T103000"),
            vcards::call(CallKind::Dialed, Some(NUMBER), "20261104T100000"),
            vcards::call(CallKind::Received, Some("+447700900001"), "20261103T090000"),
        ]
        .concat(),
    );
    connect(harness);
    harness.wait_for_app("Contacts and Call History are synced", DEADLINE, |state| {
        state.contacts.state == "ready" && state.contacts.call_history == 4
    });
}

/// Opens the Dialer with `query` typed, and waits until it has the keyboard.
fn open_dialer(harness: &Harness, query: &str) {
    harness.open_dialer(query, 0);
    harness.wait_for_app("the Dialer has keyboard focus", DEADLINE, |state| {
        dialer_shows(state, "focused", "true") && dialer_shows(state, "query", query)
    });
}

fn dialer_shows(state: &AppState, key: &str, value: &str) -> bool {
    state
        .surface("dialer")
        .and_then(|dialer| dialer.get(key))
        .is_some_and(|shown| shown == value)
}

/// Whether the Dialer, wider than the other surfaces, is on screen.
fn shows_dialer(screenshot: &Screenshot) -> bool {
    app_region(screenshot).is_some_and(|region| region.right - region.left > 500.0)
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
                .and_then(|window| window.get("status"))
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

/// Whether the Banner's caller line says the caller is being identified.
fn identifying(state: &AppState) -> bool {
    state
        .surface("banner")
        .and_then(|banner| banner.get("caller"))
        .is_some_and(|caller| caller == "Identifying caller…")
}

/// Rings the Phone, waits for the Banner and returns the Call's id.
fn ring(harness: &Harness) -> u32 {
    let _path = harness.simulated_phone().ring(NUMBER);
    let state = harness.wait_for_app("the Banner shows", DEADLINE, |state| {
        state.surface("banner").is_some()
    });
    state.calls.first().map_or(0, |call| call.id)
}

/// Rings the Phone, answers here and waits until the Call is live.
fn on_a_call(harness: &Harness) {
    let call = ring(harness);
    harness.answer(call);
    harness.wait_for_app("the Call is live", DEADLINE, |state| {
        state
            .surface("call-window")
            .is_some_and(|window| window.contains_key("audio-location"))
    });
}

/// A second caller rings during the Call: waits for its Banner.
fn waiting(harness: &Harness) {
    let _path = harness.simulated_phone().ring(SECOND_NUMBER);
    harness.wait_for_app("the Waiting Call's Banner shows", DEADLINE, |state| {
        state
            .surface("banner")
            .is_some_and(|banner| banner.contains_key("options"))
    });
}

/// Where the app's surfaces are: everything that is neither the black
/// background nor the bar stand-in.
fn app_region(screenshot: &Screenshot) -> Option<Region> {
    screenshot.region_where(|pixel| pixel != [0, 0, 0] && pixel != BAR_COLOUR)
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

/// What `automation screenshot` renders, and where.
pub struct Options {
    /// Where the gallery goes. Default: `target/screenshots/gallery`.
    pub output: Option<PathBuf>,
    /// Only this state, keeping the rest of the gallery.
    pub only: Option<String>,
    /// How many times to render it, at least once.
    pub repeat: u32,
}

/// `automation screenshot`: renders the gallery, or with `only` one state
/// of it, `repeat` times, into `output` or `target/screenshots/gallery`.
pub fn run(options: &Options) -> ExitCode {
    let states: Vec<&State> = match &options.only {
        None => STATES.iter().collect(),
        Some(only) => {
            let Some(state) = STATES.iter().find(|state| state.name == only) else {
                let names: Vec<&str> = STATES.iter().map(|state| state.name).collect();
                eprintln!(
                    "The gallery has no state {only:?}. Its states: {}.",
                    names.join(", ")
                );
                return ExitCode::FAILURE;
            };
            vec![state]
        }
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

    for repetition in 1..=options.repeat {
        for state in &states {
            for look in LOOKS {
                let harness = (state.prepare)(look.builder())
                    .headless_screen()
                    .omarchy_bar()
                    .screenshot_directory(output.clone())
                    .start();
                (state.reach)(&harness);
                // A frame is judged after it is taken. A surface queues its
                // new content for `Surfaces` as it renders it, and a task on
                // the same GLib main context publishes it. That task runs at
                // default priority, before GTK's redraw, which GLib runs at a
                // lower one. Only that scheduling order, not any proof in the
                // code, makes a frame judged while the app still reports the
                // state show that state.
                let _ = harness.screenshot_when(&file_stem(state, look), DEADLINE, |screenshot| {
                    (state.shown)(screenshot, look) && (state.still)(&harness.app_state())
                });
            }
        }
        if options.repeat > 1 {
            println!("Rendered {repetition} of {} times.", options.repeat);
        }
    }

    // The Indicator is drawn with the whole gallery; redoing one state
    // keeps it as an earlier run drew it.
    if options.only.is_none() {
        for state in INDICATOR_STATES {
            for theme in INDICATOR_THEMES {
                if let Err(error) = draw_indicator(state, theme, &output) {
                    eprintln!("Can't draw the Indicator {}: {error}", state.name);
                    return ExitCode::FAILURE;
                }
            }
        }
    }

    // Every state rendered, now or by an earlier run.
    let shots: Vec<(&State, Look, PathBuf)> = STATES
        .iter()
        .flat_map(|state| LOOKS.map(|look| (state, look)))
        .map(|(state, look)| {
            let path = output.join(format!("{}.png", file_stem(state, look)));
            (state, look, path)
        })
        .filter(|(_, _, path)| path.exists())
        .collect();
    let indicators: Vec<(&IndicatorState, OmarchyTheme, PathBuf)> = INDICATOR_STATES
        .iter()
        .flat_map(|state| INDICATOR_THEMES.map(|theme| (state, theme)))
        .map(|(state, theme)| (state, theme, indicator_path(state, theme, &output)))
        .filter(|(_, _, path)| path.exists())
        .collect();

    let index = output.join("index.html");
    if let Err(error) = fs::write(&index, index_html(&shots, &indicators)) {
        eprintln!("Can't write {}: {error}", index.display());
        return ExitCode::FAILURE;
    }
    let states = shots.len() / LOOKS.len();
    let indicator_states = indicators.len() / INDICATOR_THEMES.len();
    println!(
        "Gallery: {} screenshots of {states} state{}, light and dark, stock and under Omarchy; \
         the Indicator in {indicator_states} state{}, light and dark.",
        shots.len(),
        if states == 1 { "" } else { "s" },
        if indicator_states == 1 { "" } else { "s" }
    );
    println!("{}", index.display());
    ExitCode::SUCCESS
}

/// A state's file name under `look`, without `.png`.
fn file_stem(state: &State, look: Look) -> String {
    format!("{}-{}", state.name, look.as_str())
}

/// The gallery page: one row per state, stock light and dark, then Omarchy
/// light and dark; then the Indicator, one row per state, Omarchy light and
/// dark, enlarged without smoothing so each pixel shows.
fn index_html(
    shots: &[(&State, Look, PathBuf)],
    indicators: &[(&IndicatorState, OmarchyTheme, PathBuf)],
) -> String {
    let mut figures = String::new();
    for (state, look, path) in shots {
        let file = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let _ = writeln!(
            figures,
            "<figure><a href=\"{file}\"><img src=\"{file}\" alt=\"{title}, {look}\"></a>\
             <figcaption>{title} ({look})</figcaption></figure>",
            title = state.title,
            look = look.as_str(),
        );
    }
    let mut icons = String::new();
    for (state, theme, path) in indicators {
        let file = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let _ = writeln!(
            icons,
            "<figure><a href=\"{file}\"><img class=\"icon\" src=\"{file}\" \
             alt=\"{title}, {look}\"></a><figcaption>{title} ({look})</figcaption></figure>",
            title = state.title,
            look = theme.as_str(),
        );
    }
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <title>Desktop Calls screenshot gallery</title>\n\
         <style>body{{font-family:sans-serif;margin:16px}}\
         main{{display:grid;grid-template-columns:repeat(4,1fr);gap:16px}}\
         img{{width:100%;border:1px solid #888}}\
         img.icon{{image-rendering:pixelated}}\
         main.indicator{{grid-template-columns:repeat(2,minmax(0,384px))}}</style>\n\
         </head>\n<body>\n\
         <h1>Desktop Calls screenshot gallery</h1>\n<main>\n{figures}</main>\n\
         <h2>The Indicator in Omarchy's bar</h2>\n\
         <p>Each icon as Omarchy's tray draws it: the SVG the app names, 12 logical pixels \
         at 2× scale, painted in the bar's foreground through its alpha mask.</p>\n\
         <main class=\"indicator\">\n{icons}</main>\n</body>\n</html>\n"
    )
}
