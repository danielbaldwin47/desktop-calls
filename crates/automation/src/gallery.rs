//! The screenshot gallery: every surface in every state, in a light and a
//! dark Desktop without Omarchy (stock Adwaita) and under a light and a dark
//! Omarchy theme, with an `index.html` for a screenshot-review agent.
//!
//! Each surface ticket adds its states to [`STATES`].

use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use test_harness::vcards::{self, CallKind};
use test_harness::{
    AppState, BAR_COLOUR, Builder, Harness, Key, OmarchyTheme, Region, Screenshot, Theme,
};

/// How long the app may take to reach a state.
const DEADLINE: Duration = Duration::from_secs(10);

/// One state the gallery shows.
struct State {
    /// The file name stem, such as `bare-output`.
    name: &'static str,
    /// The caption.
    title: &'static str,
    /// Brings the scenario into the state.
    reach: fn(&Harness),
    /// Whether a capture under `look` shows the state yet.
    shown: fn(&Screenshot, Look) -> bool,
}

/// A caller in the range Ofcom keeps for drama (07700 900xxx).
const NUMBER: &str = "+447700900123";

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
        reach: connect,
        shown: |_, _| true,
    },
    State {
        name: "incoming-call-banner",
        title: "The Incoming Call Banner, ringing",
        reach: |harness| {
            connect(harness);
            ring(harness);
        },
        shown: |screenshot, look| shows(screenshot, look.green()) && shows(screenshot, look.red()),
    },
    State {
        name: "call-window-live",
        title: "The Call Window, answered here, with HD audio on this computer",
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
        name: "call-window-calling",
        title: "The Call Window, placing a Call, before the other end rings",
        reach: |harness| {
            connect(harness);
            dial(harness);
        },
        shown: |screenshot, look| shows(screenshot, look.red()) && !shows(screenshot, look.green()),
    },
    State {
        name: "call-window-ringing",
        title: "The Call Window, placing a Call, ringing at the other end",
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
        reach: |harness| {
            connect(harness);
            harness.simulated_phone().refuse_next_dial("cme:30");
            if let Err(refusal) = harness.dial(NUMBER) {
                panic!("the app refused to dial: {refusal}");
            }
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
        reach: |harness| {
            sync(harness);
            open_dialer(harness, "");
        },
        shown: |screenshot, _| shows_dialer(screenshot),
    },
    State {
        name: "dialer-results",
        title: "The Dialer, searching for smith, the second match selected",
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
];

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

/// Asks the app to Call [`NUMBER`] and waits for "Calling…".
fn dial(harness: &Harness) {
    if let Err(refusal) = harness.dial(NUMBER) {
        panic!("the app refused to dial: {refusal}");
    }
    wait_for_status(harness, "Calling…");
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

fn connect(harness: &Harness) {
    harness.simulated_phone().connect();
    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
}

/// Rings the Phone, waits for the Banner and returns the Call's id.
fn ring(harness: &Harness) -> u32 {
    let _path = harness.simulated_phone().ring(NUMBER);
    let state = harness.wait_for_app("the Banner shows", DEADLINE, |state| {
        state.surface("banner").is_some()
    });
    state.calls.first().map_or(0, |call| call.id)
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
                let harness = look
                    .builder()
                    .headless_screen()
                    .omarchy_bar()
                    .screenshot_directory(output.clone())
                    .start();
                (state.reach)(&harness);
                let _ = harness.screenshot_when(&file_stem(state, look), DEADLINE, |screenshot| {
                    (state.shown)(screenshot, look)
                });
            }
        }
        if options.repeat > 1 {
            println!("Rendered {repetition} of {} times.", options.repeat);
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

    let index = output.join("index.html");
    if let Err(error) = fs::write(&index, index_html(&shots)) {
        eprintln!("Can't write {}: {error}", index.display());
        return ExitCode::FAILURE;
    }
    let states = shots.len() / LOOKS.len();
    println!(
        "Gallery: {} screenshots of {states} state{}, light and dark, stock and under Omarchy.",
        shots.len(),
        if states == 1 { "" } else { "s" }
    );
    println!("{}", index.display());
    ExitCode::SUCCESS
}

/// A state's file name under `look`, without `.png`.
fn file_stem(state: &State, look: Look) -> String {
    format!("{}-{}", state.name, look.as_str())
}

/// The gallery page: one row per state, stock light and dark, then Omarchy
/// light and dark.
fn index_html(shots: &[(&State, Look, PathBuf)]) -> String {
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
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <title>Desktop Calls screenshot gallery</title>\n\
         <style>body{{font-family:sans-serif;margin:16px}}\
         main{{display:grid;grid-template-columns:repeat(4,1fr);gap:16px}}\
         img{{width:100%;border:1px solid #888}}</style>\n</head>\n<body>\n\
         <h1>Desktop Calls screenshot gallery</h1>\n<main>\n{figures}</main>\n</body>\n</html>\n"
    )
}
