//! Scenarios: the Dialer on a screen of its own, opened from the CLI, the
//! keybinding, `OpenDialer` and the app launcher, driven by synthetic keys,
//! and observed through the control interface's `Surfaces`, what the
//! Simulated Phone was asked to do, and screenshots.

use std::collections::BTreeMap;
use std::thread;
use std::time::{Duration, Instant};

use test_harness::vcards::{self, CallKind};
use test_harness::{
    AppEvent, AppState, BAR_COLOUR, BAR_HEIGHT, Harness, Key, LAYOUT_SIZE, Screenshot,
};

/// How long the app may take to show a change.
const DEADLINE: Duration = Duration::from_secs(5);
/// How long a whole sync may take.
const SYNC_DEADLINE: Duration = Duration::from_secs(15);
/// How long a surface may take to be drawn on screen. Each capture and its
/// scan take over a second on a busy CI runner, so this allows several.
const SCREEN_DEADLINE: Duration = Duration::from_secs(10);
/// How long the Dialer may take to draw its first frame after `Activate`
/// reaches the app, by the app's log (issues #11, #83 and #47): spec story
/// 100's 150 ms, here and under `just test-ci-like`.
const SHOW_DEADLINE: Duration = Duration::from_millis(150);
/// The same in CI, whose runner paints in software: run alone there, the
/// Dialer's own first paint took 144 of 150.4 ms. 250 ms still catches its
/// shaders compiling cold again, which took 306 ms and more. The 150 ms
/// target is judged on the user's Desktop, by the Real-Phone Checklist's
/// step 7.4.
const CI_SHOW_DEADLINE: Duration = Duration::from_millis(250);
/// Set by `just test-ci-like`, which holds the Dialer to 150 ms even where
/// `CI` is set.
const CI_LIKE: &str = "DESKTOP_CALLS_CI_LIKE";

/// The Dialer's first-frame deadline for where the tests run.
fn show_deadline() -> Duration {
    if std::env::var_os("CI").is_some() && std::env::var_os(CI_LIKE).is_none() {
        CI_SHOW_DEADLINE
    } else {
        SHOW_DEADLINE
    }
}
/// How long the app may take to do its first-frame work after it starts.
const FIRST_FRAMES_DEADLINE: Duration = Duration::from_secs(15);
/// How long the Dialer's rows may take to follow a keystroke, from the key
/// reaching the app to the frame that shows them, by the app's log.
const KEYSTROKE_DEADLINE: Duration = Duration::from_millis(50);
/// How long the tests' own round trip may take: the request to the app, and
/// the change back on the control interface. Looser than the targets above
/// because a loaded CI runner stretches it even when the app is fast
/// (ticket #83); the targets are asserted on the app's log instead.
const ROUND_TRIP: Duration = Duration::from_secs(3);
/// How long something that must not happen is watched for.
const HOLD: Duration = Duration::from_millis(500);

fn contacts() -> String {
    let mut all = String::new();
    all.push_str(&vcards::contact("José Smith", &[("CELL", "+447700900001")]));
    all.push_str(&vcards::contact(
        "Alice Smith",
        &[("CELL", "+44 7700 900123"), ("WORK", "020 7946 0123")],
    ));
    all.push_str(&vcards::contact(
        "Joanna Smithers",
        &[("CELL", "+447700900002")],
    ));
    all.push_str(&vcards::contacts(20, 1_000));
    all
}

fn call_history() -> String {
    [
        vcards::call(CallKind::Missed, Some("+447700900999"), "20261104T110000"),
        vcards::call(CallKind::Dialed, Some("+447700900123"), "20261104T100000"),
        vcards::call(CallKind::Received, Some("+447700900001"), "20261104T090000"),
    ]
    .concat()
}

/// A connected, synced Phone, on a screen with Omarchy's bar.
fn synced_on_screen() -> Harness {
    let harness = Harness::builder().headless_screen().omarchy_bar().start();
    let phone = harness.simulated_phone();
    phone.set_pbap_folder("pb", &contacts());
    phone.set_pbap_folder("cch", &call_history());
    phone.connect();
    harness.wait_for_app(
        "the Phone is connected, and Contacts and Call History synced",
        SYNC_DEADLINE,
        |state| {
            state.connection_state == "connected"
                && state.contacts.state == "ready"
                && state.contacts.call_history == 3
        },
    );
    harness
}

fn dialer(state: &AppState) -> Option<&BTreeMap<String, String>> {
    state.surface("dialer")
}

fn shown<'a>(state: &'a AppState, key: &str) -> Option<&'a str> {
    dialer(state)
        .and_then(|dialer| dialer.get(key))
        .map(String::as_str)
}

fn focused(state: &AppState) -> bool {
    shown(state, "focused") == Some("true")
}

fn run(harness: &Harness, arguments: &[&str]) {
    let output = harness.cli(arguments);
    assert!(
        output.status.success(),
        "desktop-calls {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Opens the Dialer with `desktop-calls dialer` and waits until it has the
/// keyboard.
fn open(harness: &Harness) -> AppState {
    run(harness, &["dialer"]);
    harness.wait_for_app("the Dialer has keyboard focus", DEADLINE, focused)
}

/// Types `text` into the Dialer and waits until it shows the rows for it.
fn type_query(harness: &Harness, text: &str) -> AppState {
    harness.keyboard().type_text(text);
    harness.wait_for_app(
        &format!("the Dialer searched for {text:?}"),
        DEADLINE,
        |state| shown(state, "query") == Some(text),
    )
}

/// Types `text` into the Dialer and waits until it shows the rows for
/// `query`.
fn type_query_from(harness: &Harness, text: &str, query: &str) -> AppState {
    harness.keyboard().type_text(text);
    harness.wait_for_app(
        &format!("the Dialer searched for {query:?}"),
        DEADLINE,
        |state| shown(state, "query") == Some(query),
    )
}

fn dialled(harness: &Harness, number: &str) {
    let expected = format!("(\"{number}\",)");
    harness.simulated_phone().wait_for_recorded(
        &format!("the Phone is asked to dial {number}"),
        DEADLINE,
        |calls| {
            calls
                .iter()
                .any(|call| call.member == "Dial" && call.arguments == expected)
        },
    );
}

fn never_dialled(harness: &Harness) {
    harness
        .simulated_phone()
        .recorded_holds("the Phone is never asked to dial", HOLD, |calls| {
            !calls.iter().any(|call| call.member == "Dial")
        });
}

fn app_region(screenshot: &Screenshot) -> Option<test_harness::Region> {
    screenshot.region_where(|pixel| pixel != [0, 0, 0] && pixel != BAR_COLOUR)
}

/// Omarchy's Super+W closes the focused window, and a Hyprland bind can
/// hand it to the Dialer instead (ticket #125); the Dialer then closes, as
/// on Escape, and calls no one. sway here binds no keys, so Super+W reaches
/// the Dialer as it would through that bind.
#[test]
fn super_w_closes_the_dialer_as_escape_does() {
    let harness = synced_on_screen();
    open(&harness);
    type_query(&harness, "jose");

    harness.keyboard().press_with_super('w');

    harness.wait_for_app("the Dialer closes", DEADLINE, |state| {
        dialer(state).is_none()
    });
    never_dialled(&harness);
}

#[test]
fn the_dialer_command_shows_the_dialer_centred_with_focus_listing_the_call_history() {
    let harness = synced_on_screen();

    let state = open(&harness);

    assert_eq!(shown(&state, "query"), Some(""));
    assert_eq!(shown(&state, "rows"), Some("3"));
    assert_eq!(shown(&state, "selected"), Some("0"));
    let row = |index: usize| shown(&state, &format!("row-{index}")).unwrap_or_default();
    assert!(row(0).contains("Missed"), "{:?}", dialer(&state));
    assert!(row(1).contains("Alice Smith"), "{:?}", dialer(&state));
    assert!(row(2).contains("José Smith"), "{:?}", dialer(&state));
    assert!(!row(1).contains("Missed") && !row(2).contains("Missed"));

    let screenshot = harness.screenshot_when("dialer", SCREEN_DEADLINE, |screenshot| {
        app_region(screenshot).is_some_and(|region| region.right - region.left > 300.0)
    });
    let region = app_region(&screenshot).expect("the Dialer is on screen");
    let (x, y) = region.centre();
    let (width, height) = LAYOUT_SIZE;
    assert!(
        (x - width / 2.0).abs() <= 2.0,
        "not centred across: {region:?}"
    );
    // Centred in the space below the bar.
    let middle = f64::midpoint(height, BAR_HEIGHT);
    assert!((y - middle).abs() <= 2.0, "not centred down: {region:?}");
}

#[test]
fn down_down_enter_dials_the_selected_row_and_the_dialer_closes_as_the_call_starts() {
    let harness = synced_on_screen();
    open(&harness);
    let state = type_query(&harness, "smith");
    let rows = harness.search("smith");
    assert!(rows.len() >= 3, "{rows:?}");
    assert_eq!(shown(&state, "rows"), Some(rows.len().to_string().as_str()));

    harness.keyboard().press(Key::Down);
    harness.keyboard().press(Key::Down);
    harness.wait_for_app("the third row is selected", DEADLINE, |state| {
        shown(state, "selected") == Some("2")
    });
    harness.keyboard().press(Key::Return);

    dialled(&harness, &rows[2].number);
    let state = harness.wait_for_app(
        "the Dialer closes and the Call Window says Calling…",
        DEADLINE,
        |state| {
            dialer(state).is_none()
                && state
                    .surface("call-window")
                    .and_then(|window| window.get("status"))
                    .is_some_and(|status| status == "Calling…")
        },
    );
    assert_eq!(
        state
            .surface("call-window")
            .and_then(|window| window.get("caller"))
            .map(String::as_str),
        Some(rows[2].name.as_str())
    );
}

#[test]
fn typing_a_dialable_number_shows_a_call_row_and_enter_dials_it() {
    let harness = synced_on_screen();
    open(&harness);

    let state = type_query(&harness, "07700 900888");

    assert_eq!(shown(&state, "row-0"), Some("Call 07700 900888"));
    harness.keyboard().press(Key::Return);
    dialled(&harness, "07700900888");
}

#[test]
fn pasting_a_tel_link_finds_its_contact_and_enter_dials_it() {
    let harness = synced_on_screen();
    open(&harness);
    let _clipboard = harness.set_clipboard("tel:+447700900001");

    harness.keyboard().press_with_control('v');

    let state = harness.wait_for_app("the Dialer searched for the link", DEADLINE, |state| {
        shown(state, "query") == Some("tel:+447700900001")
    });
    assert!(
        shown(&state, "row-0").is_some_and(|row| row.contains("José Smith")),
        "{:?}",
        dialer(&state)
    );
    harness.keyboard().press(Key::Return);
    dialled(&harness, "+447700900001");
}

#[test]
fn an_undialable_number_is_flagged_before_enter() {
    let harness = synced_on_screen();
    open(&harness);

    let state = type_query(&harness, "07700 9x");

    assert_eq!(shown(&state, "rows"), Some("0"));
    assert_eq!(
        shown(&state, "notice"),
        Some("That isn't a number your phone can dial; use digits, spaces and + * # only.")
    );
    harness.keyboard().press(Key::Return);
    never_dialled(&harness);
}

#[test]
fn escape_closes_the_dialer_without_calling() {
    let harness = synced_on_screen();
    open(&harness);
    type_query(&harness, "jose");

    harness.keyboard().press(Key::Escape);

    harness.wait_for_app("the Dialer closes", DEADLINE, |state| {
        dialer(state).is_none()
    });
    never_dialled(&harness);
    // It opens afresh, with nothing typed.
    let state = open(&harness);
    assert_eq!(shown(&state, "query"), Some(""));
}

#[test]
fn with_the_phone_disconnected_the_dialer_searches_cached_contacts_and_enter_says_why_not() {
    let harness = synced_on_screen();
    harness.simulated_phone().disconnect();
    harness.wait_for_app("the Phone is disconnected", DEADLINE, |state| {
        state.connection_state == "disconnected"
    });
    open(&harness);

    let state = type_query(&harness, "jose");
    assert!(
        shown(&state, "row-0").is_some_and(|row| row.contains("José Smith")),
        "{:?}",
        dialer(&state)
    );
    harness.keyboard().press(Key::Return);

    let state = harness.wait_for_app("the Dialer says why not", DEADLINE, |state| {
        shown(state, "notice").is_some()
    });
    assert_eq!(
        shown(&state, "notice"),
        Some("Phone not connected: it's out of range or has Bluetooth off.")
    );
    assert!(
        state.surface("call-window").is_none(),
        "{:?}",
        state.surfaces
    );
    never_dialled(&harness);
}

#[test]
fn during_a_live_call_enter_in_the_dialer_says_hang_up_the_current_call_first() {
    let harness = synced_on_screen();
    let _path = harness.simulated_phone().ring("+447700900123");
    let state = harness.wait_for_app("the Phone rings", DEADLINE, |state| state.ringing);
    harness.answer(state.calls[0].id);
    harness.wait_for_app("the Call is live", DEADLINE, |state| {
        state.calls.iter().any(|call| call.state == "active")
    });
    run(&harness, &["dialer", "jose"]);
    let state = harness.wait_for_app("the Dialer has keyboard focus", DEADLINE, focused);
    assert_eq!(shown(&state, "query"), Some("jose"));
    // Typing adds to the query it opened with.
    let state = type_query_from(&harness, " sm", "jose sm");
    assert!(
        shown(&state, "row-0").is_some_and(|row| row.contains("José Smith")),
        "{:?}",
        dialer(&state)
    );

    harness.keyboard().press(Key::Return);

    let state = harness.wait_for_app("the Dialer says why not", DEADLINE, |state| {
        shown(state, "notice").is_some()
    });
    assert_eq!(
        shown(&state, "notice"),
        Some("Hang up the current Call first.")
    );
    never_dialled(&harness);
}

#[test]
fn with_nothing_ringing_or_live_the_keybinding_opens_the_dialer() {
    let harness = synced_on_screen();

    run(&harness, &[]);

    let state = harness.wait_for_app("the Dialer has keyboard focus", DEADLINE, focused);
    assert_eq!(shown(&state, "rows"), Some("3"));
}

#[test]
fn the_app_launcher_opens_the_dialer() {
    let harness = synced_on_screen();

    harness.launch_from_app_launcher();

    harness.wait_for_app("the Dialer has keyboard focus", DEADLINE, focused);
}

#[test]
fn open_dialer_selects_the_call_asked_for() {
    let harness = synced_on_screen();
    let path = harness.simulated_phone().ring("+447700900123");
    let state = harness.wait_for_app("the Phone rings", DEADLINE, |state| state.ringing);
    let call = state.calls[0].id;
    harness.simulated_phone().caller_hangs_up(&path);
    harness.wait_for_app("the Call is over", DEADLINE, |state| state.calls.is_empty());

    harness.open_dialer("", call);

    let state = harness.wait_for_app("the Dialer shows", DEADLINE, |state| {
        dialer(state).is_some()
    });
    let selected: usize = shown(&state, "selected")
        .and_then(|selected| selected.parse().ok())
        .expect("a row is selected");
    assert!(
        shown(&state, &format!("row-{selected}")).is_some_and(|row| row.contains("Alice Smith")),
        "{:?}",
        dialer(&state)
    );
}

/// A timing judged one scenario at a time, as on the Desktop, which runs one
/// app (issue #47). CI runs it alone, after the whole run: with the whole run's
/// scenarios at once on its runner, the first frame took 226 ms; one scenario
/// at a time under `just test-ci-like`, 97 to 102 ms in 5 of 5 runs.
#[test]
#[ignore = "one scenario at a time: just test-file dialer_surface --ignored --test-threads=1 --ci-like"]
fn with_5000_contacts_the_dialer_shows_and_follows_each_keystroke_promptly() {
    let harness = Harness::builder().headless_screen().omarchy_bar().start();
    let phone = harness.simulated_phone();
    phone.set_pbap_folder("pb", &vcards::contacts(5_000, 0));
    phone.set_pbap_folder("cch", &vcards::call_history(200));
    phone.connect();
    harness.wait_for_app("5000 Contacts are synced", SYNC_DEADLINE, |state| {
        state.contacts.state == "ready"
            && state.contacts.contacts == 5_000
            && state.contacts.call_history == 200
    });

    // The app has had its idle time since login, as it has by the time a
    // person first opens the Dialer: the first frame is then as fast as later
    // ones. Opened before that, on a cold shader cache, it took 306 to 319 ms
    // here (2026-11-05, #83).
    harness.wait_for_first_frames_prepared(FIRST_FRAMES_DEADLINE);
    harness.activate();
    harness.wait_for_app("the Dialer shows", ROUND_TRIP, |state| {
        dialer(state).is_some()
    });
    let first_frame_after = harness.assert_app_within(
        "the Dialer to draw its first frame once asked",
        &AppEvent::kind("activate-requested"),
        &AppEvent::kind("surface-shown").with("surface", "dialer"),
        show_deadline(),
    );
    harness.wait_for_app("the Dialer has keyboard focus", DEADLINE, focused);

    let mut query = String::new();
    let mut slowest = Duration::ZERO;
    for character in "contact 49".chars() {
        query.push(character);
        harness.keyboard().type_text(&character.to_string());
        harness.wait_for_app(
            &format!("the Dialer shows the rows for {query:?}"),
            ROUND_TRIP,
            |state| shown(state, "query") == Some(query.as_str()),
        );
        let length = query.chars().count();
        let took = harness.assert_app_within(
            &format!("the Dialer to show the rows for {query:?} once the key reached it"),
            &AppEvent::kind("dialer-query-typed").with("query_chars", length),
            &AppEvent::kind("dialer-query-shown").with("query_chars", length),
            KEYSTROKE_DEADLINE,
        );
        slowest = slowest.max(took);
    }
    eprintln!("Dialer's first frame after {first_frame_after:?}; slowest keystroke {slowest:?}");
    let state = harness.app_state();
    assert!(
        shown(&state, "row-0").is_some_and(|row| row.starts_with("Contact 49")),
        "{:?}",
        dialer(&state)
    );
}

/// One way of opening the Dialer.
type OpenDialer = fn(&Harness);

/// The app's log, in order: `true` for each `activate-requested`, `false`
/// for each first frame the Dialer drew.
fn asked_and_drawn(harness: &Harness) -> Vec<bool> {
    let asked = AppEvent::kind("activate-requested");
    let drawn = AppEvent::kind("surface-shown").with("surface", "dialer");
    harness
        .app_log()
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|event| {
            if asked.matches(&event) {
                Some(true)
            } else if drawn.matches(&event) {
                Some(false)
            } else {
                None
            }
        })
        .collect()
}

/// Step 7.4 of the Real-Phone Checklist times the Dialer's first frame from
/// the app's `activate-requested` line, so every way of opening the Dialer
/// must log one first (ticket #125: opened from the Indicator, it logged
/// none, and 7.4 had nothing to measure).
#[test]
fn every_way_of_opening_the_dialer_logs_that_it_was_asked() {
    let harness = synced_on_screen();
    let ways: [(&str, OpenDialer); 6] = [
        ("a left click on the Indicator", |harness| {
            harness.tray().activate();
        }),
        ("Open Dialer in the Indicator's menu", |harness| {
            harness.tray().click("Open Dialer");
        }),
        ("the keybinding's command", |harness| run(harness, &[])),
        ("`desktop-calls dialer`", |harness| {
            run(harness, &["dialer"]);
        }),
        ("`OpenDialer` on the control interface", |harness| {
            harness.open_dialer("", 0);
        }),
        ("the app launcher", Harness::launch_from_app_launcher),
    ];

    for (opened, (way, open)) in (1..).zip(ways) {
        open(&harness);
        harness.wait_for_app(&format!("{way} opens the Dialer"), DEADLINE, focused);
        // The first frame is logged just after the Dialer shows.
        let started = Instant::now();
        let events = loop {
            let events = asked_and_drawn(&harness);
            let drawn = events.iter().filter(|asked| !**asked).count();
            if drawn >= opened || started.elapsed() > DEADLINE {
                break events;
            }
            thread::sleep(Duration::from_millis(50));
        };
        let drawn = events.iter().filter(|asked| !**asked).count();
        let asked = events.len() - drawn;
        assert_eq!(
            drawn, opened,
            "opened by {way}, the Dialer's first frames logged"
        );
        assert_eq!(
            asked, opened,
            "opened by {way}, the app logged activate-requested {asked} times for {opened} openings"
        );
        // This opening's activate-requested comes before its first frame.
        assert_eq!(
            events.last(),
            Some(&false),
            "opened by {way}, the app logged activate-requested after the Dialer's first frame"
        );
        assert_eq!(
            events.get(events.len().wrapping_sub(2)),
            Some(&true),
            "opened by {way}, the Dialer drew its first frame before the app logged activate-requested"
        );
        harness.keyboard().press(Key::Escape);
        harness.wait_for_app("the Dialer closes", DEADLINE, |state| {
            dialer(state).is_none()
        });
    }
}
