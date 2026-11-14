//! The demo's Simulated Phone and app, and the actions a typed line acts out.

use std::fs;
use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, SystemTime};

use test_harness::{Bus, Harness, Key, LAYOUT_SIZE, TestClient};

use super::{SYSTEM_PYTHON, flush};
use crate::checklist::results;
use crate::checklist::watch::AppState;

/// The Simulated Phone and the app it runs against, for `--demo`. Typed
/// actions act out what the Phone, the helper and the user would do:
///
/// - the helper: `call` rings the Phone, `call-withheld` rings it with the
///   number withheld; `hang-up-on-phone` ends the Call from the other end,
///   and `ring-out` gives up while it rings, leaving a Missed Call in the
///   Phone's Call History;
/// - the person holding the Phone: `answer-on-phone`, `decline-on-phone`;
/// - the Phone itself: `phone-dials` places the Call the app asked for;
/// - the user at the Desktop, through the control interface, the
///   notification server and the keyboard: `answer`, `decline`, `hang-up`,
///   `mute`, `unmute`, `keybinding`, `escape`, `a`, `d`,
///   `click-notification`, `enter`;
/// - the user's Desktop: `do-not-disturb-on`, `do-not-disturb-off`, as
///   Omarchy's bell records it; `suspend`, `resume`, as logind announces
///   them, with the Phone leaving on suspend and returning after resume as
///   the Pixel does; `resume-slowly`, with the Phone answering the
///   Desktop's page only a few seconds after resume; `suspend-phone-stays`
///   and `resume-then-phone-leaves`, with the Phone leaving only after
///   resume; `restart-wireplumber`, as Telephony leaves and returns;
/// - the user's keyboard and Call Window, beyond the above:
///   `type-test-number`, `type-wrong-number` (the last digit off, as a
///   slip would be), `type-wrong-first-digit` (the first digit after the
///   country code off), `type-wrong-country` (another country code),
///   `ctrl-h`, `move-audio-to-phone`, `move-audio-here`,
///   `move-audio-to-phone-and-back` (each move waited for);
/// - the Phone and its surroundings: `callee-answers` (the Test Number
///   picks up), `earbuds-on`, `out-of-range`, `return-to-range`;
/// - a first run: `forget-pairing` (the Phone forgets the Desktop and the
///   Desktop removes the Phone, which stays nearby), `restart-app` (as
///   `just install` restarts it), `open-setup-window` (`desktop-calls
///   connect`), `contacts-access-off` (the Phone asks again before it
///   shares Contacts), `allow-contacts` (Allow on the Phone's prompt),
///   `bind-super-alt-c-elsewhere` (the user's Hyprland config already uses
///   those keys for another program), `cancel-then-confirm` (on the
///   keybinding's line: Cancel, then Add keybinding and Confirm after all);
/// - the Phone's quirks: `quirk NAME`, as the Simulated Phone names them.
pub struct Demo {
    /// The user's own window, which has keyboard focus until the keybinding
    /// takes it, and gets it back on Escape, as on a real Desktop.
    _window: TestClient,
    pub harness: Harness,
    /// The Simulated Phone's object path for the last Call the helper made.
    call_path: Option<String>,
    /// The number the helper last called from, or `None` if withheld.
    calling_from: Option<&'static str>,
    /// The Missed Calls the demo's Phone has added to its Call History,
    /// newest first, as PBAP vCards.
    missed: Vec<String>,
    /// How many of the app's Dial requests the demo's Phone has placed.
    dials_placed: usize,
    /// The Unix time in seconds the demo last turned calls off until.
    calls_off_until: u64,
}

/// The number the demo's helper calls from: an Ofcom drama number.
const DEMO_NUMBER: &str = "+447700900461";

/// The demo's Test Number, for the PBAP probe to redact: another Ofcom
/// drama number.
const DEMO_TEST_NUMBER: &str = "+447700900999";

/// The demo's Test Number with its last digit slipped.
const DEMO_WRONG_NUMBER: &str = "+447700900998";

/// The demo's Test Number with its first digit after the country code
/// slipped.
const DEMO_WRONG_FIRST_DIGIT: &str = "+448700900999";

/// The demo's Test Number's national number under another country code.
const DEMO_WRONG_COUNTRY: &str = "+337700900999";

/// How long after `resume-then-phone-leaves` the demo's Phone is back in
/// range.
const LATE_LEAVING_RETURN: Duration = Duration::from_secs(1);

/// How long the demo waits for the audio to move.
const AUDIO_DEADLINE: Duration = Duration::from_secs(10);

/// How long the demo's Phone takes to answer the Desktop's page after
/// `resume-slowly`: longer than the runner reads the log for a line the
/// app logs at once.
const SLOW_RETURN: Duration = Duration::from_secs(4);

/// How far ahead `calls-off-soon` turns calls off until: long enough for
/// the runner to see it and the demo to suspend before it passes.
const CALLS_OFF_SOON: Duration = Duration::from_secs(5);

/// How long Telephony is off the bus when the demo restarts `WirePlumber`.
const WIREPLUMBER_GAP: Duration = Duration::from_millis(500);

/// How many Calls the demo's Call History holds: more than the 50 the
/// Pixel gives per pull, so there is a second page to read.
const DEMO_CALL_HISTORY: usize = 120;

/// The demo's Omarchy bindings: one of the user's own.
const DEMO_BINDINGS: &str = "-- Keep only your personal keybinding overrides here.\n\
                             o.bind(\"SUPER + SHIFT + R\", \"SSH\", \"alacritty -e ssh your-server\")\n";

/// How long the keybinding's focus, or the user's window, may take to
/// arrive in the demo.
const FOCUS_DEADLINE: Duration = Duration::from_secs(5);

/// How long the Call Window may take to collapse to the Call Pill in the
/// demo: 5 s after the Call connects, and a margin.
const COLLAPSE_DEADLINE: Duration = Duration::from_secs(10);

/// Whether the Setup Window has keyboard focus and shows exactly `buttons`.
fn setup_window_shows(state: &test_harness::AppState, buttons: &str) -> bool {
    state.surface("setup-window").is_some_and(|window| {
        window.get("buttons").map(String::as_str) == Some(buttons)
            && window.get("focused").map(String::as_str) == Some("true")
    })
}

impl Demo {
    /// `Device1.Connect` on the Phone with `address`, through the demo's
    /// system bus, as [`super::Sitting::connect_the_phone`] does on the
    /// Desktop's.
    pub fn connect_the_phone(&self, address: &str) -> zbus::Result<()> {
        let path = format!("/org/bluez/hci0/dev_{}", address.replace(':', "_"));
        self.harness
            .try_call_phone_method(
                Bus::System,
                "org.bluez",
                &path,
                "org.bluez.Device1",
                "Connect",
                &(),
            )
            .map(drop)
    }

    pub fn new(harness: Harness) -> Self {
        let window = harness.start_test_client();
        window.wait_for("your window has keyboard focus", FOCUS_DEADLINE, |line| {
            line == "keyboard-enter"
        });
        // As a real Phone, hands-free comes up a moment after BlueZ marks
        // the Phone connected, so a reconnect takes time to measure.
        harness.simulated_phone().enable_quirk("slow-service-level");
        // Omarchy's personal bindings, for step 1.2 to add the keybinding to.
        let bindings = harness.directory().join("config/hypr/bindings.lua");
        let written = bindings
            .parent()
            .map(fs::create_dir_all)
            .and_then(Result::ok)
            .and_then(|()| fs::write(&bindings, DEMO_BINDINGS).ok());
        if written.is_none() {
            println!("  demo: couldn't write {}.", bindings.display());
        }
        Self {
            _window: window,
            harness,
            call_path: None,
            calling_from: None,
            missed: Vec::new(),
            dials_placed: 0,
            calls_off_until: 0,
        }
    }

    /// The Phone places the Call the app asked it to dial, as a real Phone
    /// does on its own: the Simulated Phone waits to be told.
    /// Turns calls off until [`CALLS_OFF_SOON`] from now.
    fn calls_off_soon(&mut self) {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();
        self.calls_off_until = (now + CALLS_OFF_SOON).as_secs();
        if let Err(error) = self.harness.turn_calls_off(self.calls_off_until) {
            panic!("calls didn't turn off: {error:?}");
        }
    }

    /// Waits out the rest of the time calls are off until, as the Desktop
    /// sleeps through it, then resumes it. The Phone is still in range.
    fn resume_past_calls_off_deadline(&self) {
        let until = SystemTime::UNIX_EPOCH + Duration::from_secs(self.calls_off_until);
        if let Ok(left) = until.duration_since(SystemTime::now()) {
            std::thread::sleep(left + Duration::from_secs(1));
        }
        self.harness.simulated_phone().prepare_for_sleep(false);
    }

    fn phone_dials(&mut self) {
        let placed = self.dials_placed;
        let phone = self.harness.simulated_phone();
        phone.wait_for_recorded(
            "the app asking the Phone to dial",
            FOCUS_DEADLINE,
            |calls| calls.iter().filter(|call| call.member == "Dial").count() > placed,
        );
        self.dials_placed += 1;
        let _ = phone.alert();
    }

    /// The helper's Call rings out: like a real Phone, the demo's Phone
    /// adds a Missed Call to its Call History as the Call ends.
    fn ring_out(&mut self) {
        let Some(path) = &self.call_path else {
            println!("  demo: the helper hasn't called yet; type call first.");
            return;
        };
        self.missed.insert(
            0,
            test_harness::vcards::call(
                test_harness::vcards::CallKind::Missed,
                self.calling_from,
                &results::irmc_timestamp(SystemTime::now()),
            ),
        );
        let phone = self.harness.simulated_phone();
        phone.set_pbap_folder("cch", &self.missed.concat());
        phone.caller_hangs_up(path);
    }

    /// The Phone forgets the Desktop, which drops the connection, and the
    /// Desktop removes the Phone in its Bluetooth settings.
    fn forget_pairing(&self) {
        self.harness.simulated_phone().forget_pairing();
        let address = self.harness.phone().address.replace(':', "_");
        let device = format!("/org/bluez/hci0/dev_{address}");
        match zbus::ObjectPath::try_from(device.as_str()) {
            Ok(device) => drop(self.harness.call_phone_method(
                Bus::System,
                "org.bluez",
                "/org/bluez/hci0",
                "org.bluez.Adapter1",
                "RemoveDevice",
                &(device,),
            )),
            Err(error) => println!("  demo: {device} isn't a device path: {error}"),
        }
    }

    /// The user's Hyprland config already binds Super + Alt + C to another
    /// program, so the finish page can't offer that keybinding.
    fn bind_super_alt_c_elsewhere(&self) {
        let bindings = self.harness.directory().join("config/hypr/bindings.lua");
        let taken = format!(
            "{DEMO_BINDINGS}o.bind(\"super + alt + c\", \"Calculator\", \"gnome-calculator\")\n"
        );
        if fs::write(&bindings, taken).is_err() {
            println!("  demo: couldn't write {}.", bindings.display());
        }
    }

    /// With the keybinding's line showing, the user chooses Cancel, so the
    /// finish page offers the keybinding again; then Add keybinding and
    /// Confirm after all.
    fn cancel_then_confirm(&self) {
        let keyboard = self.harness.keyboard();
        keyboard.press(Key::Tab);
        keyboard.press(Key::Return);
        self.harness.wait_for_app(
            "the finish page offering the keybinding again",
            FOCUS_DEADLINE,
            |state| setup_window_shows(state, "Add keybinding\nDone"),
        );
        keyboard.press(Key::Return);
        self.harness.wait_for_app(
            "the keybinding's line showing again",
            FOCUS_DEADLINE,
            |state| setup_window_shows(state, "Confirm\nCancel"),
        );
        keyboard.press(Key::Return);
    }

    /// The user clicks the newest notification from Desktop Calls.
    fn click_notification(&self) {
        let notifications = self.harness.notifications();
        match notifications
            .received()
            .into_iter()
            .rfind(|notification| notification.app_name == "Desktop Calls")
        {
            Some(notification) => notifications.invoke_default_action(notification.id),
            None => println!("  demo: Desktop Calls hasn't shown a notification."),
        }
    }

    /// The PBAP probe for the demo, and the cache directory it writes
    /// to: the scenario's buses and home, a made-up Test Number for the
    /// probe to redact, and a Call History longer than one pull gives.
    pub(super) fn pbap_probe_command(&self) -> io::Result<(Command, PathBuf)> {
        let directory = self.harness.directory();
        let config = directory.join("config/desktop-calls");
        fs::create_dir_all(&config)?;
        fs::write(config.join("test-number"), DEMO_TEST_NUMBER)?;
        self.harness.simulated_phone().set_pbap_folder(
            "cch",
            &test_harness::vcards::call_history(DEMO_CALL_HISTORY),
        );
        Ok((
            self.harness.external_command(SYSTEM_PYTHON),
            directory.join("cache"),
        ))
    }

    pub(super) fn act(&mut self, action: &str, state: &AppState) {
        let call = state.calls.first().map(|call| call.id);
        // The harness is test code: an action it can't carry out panics with
        // the reason. A demo goes on after a mistyped or ill-timed action,
        // so the panic is caught and reported.
        let acted = panic::catch_unwind(AssertUnwindSafe(|| {
            self.act_or_panic(action, call);
        }));
        if acted.is_err() {
            println!("  demo: {action} didn't work now; see the message above.");
            flush();
        }
    }

    fn act_or_panic(&mut self, action: &str, call: Option<u32>) {
        match action {
            "call" => {
                self.calling_from = Some(DEMO_NUMBER);
                self.call_path = Some(self.harness.simulated_phone().ring(DEMO_NUMBER));
            }
            // As PipeWire reads the caller ID a Phone sends for a withheld
            // number.
            "call-withheld" => {
                self.calling_from = None;
                self.call_path = Some(
                    self.harness
                        .simulated_phone()
                        .ring_with_caller_id("\"\",129"),
                );
            }
            "ring-out" => self.ring_out(),
            "forget-pairing" => self.forget_pairing(),
            "restart-app" => self.harness.restart_app(),
            "open-setup-window" => drop(self.harness.cli(&["connect"])),
            "contacts-access-off" => self.harness.simulated_phone().set_contacts_prompt("ask"),
            // Android remembers the answer for the Desktop.
            "bind-super-alt-c-elsewhere" => self.bind_super_alt_c_elsewhere(),
            "cancel-then-confirm" => self.cancel_then_confirm(),
            "allow-contacts" => {
                let phone = self.harness.simulated_phone();
                phone.answer_contacts_prompt("grant");
                phone.set_contacts_prompt("grant");
            }
            "click-notification" => self.click_notification(),
            "enter" => self.harness.keyboard().press(Key::Return),
            "phone-dials" => self.phone_dials(),
            "do-not-disturb-on" => self.harness.set_do_not_disturb(true),
            "do-not-disturb-off" => self.harness.set_do_not_disturb(false),
            "type-test-number" => self.harness.keyboard().type_text(DEMO_TEST_NUMBER),
            "type-wrong-number" => self.harness.keyboard().type_text(DEMO_WRONG_NUMBER),
            "type-wrong-first-digit" => self.harness.keyboard().type_text(DEMO_WRONG_FIRST_DIGIT),
            "type-wrong-country" => self.harness.keyboard().type_text(DEMO_WRONG_COUNTRY),
            "ctrl-h" => self.harness.keyboard().press_with_control('h'),
            "callee-answers" => self.harness.simulated_phone().callee_answers(),
            "earbuds-on" => self.harness.simulated_phone().set_earbuds(true),
            "out-of-range" => self.harness.simulated_phone().disconnect(),
            "return-to-range" => self.harness.simulated_phone().return_to_range(),
            "calls-off" => self.harness.tray().click("Until I turn them on"),
            "calls-on" => self.harness.tray().click("Turn calls on"),
            // As `desktop-calls calls off --until` a moment ahead: the CLI
            // takes whole minutes, too long for a demo.
            "calls-off-soon" => self.calls_off_soon(),
            "resume-past-calls-off-deadline" => self.resume_past_calls_off_deadline(),
            // The Phone, blocked and out of the Desktop's hands, pages it as
            // its Bluetooth comes back on: BlueZ refuses it.
            "bluetooth-off-on" => {
                let _refused = self.harness.simulated_phone().page_desktop();
            }
            // The Desktop's Bluetooth sleeps with it, so the Phone leaves,
            // and comes back only once the Desktop is awake.
            "suspend" => {
                self.harness.simulated_phone().prepare_for_sleep(true);
                self.harness.simulated_phone().disconnect();
            }
            "resume" => {
                self.harness.simulated_phone().prepare_for_sleep(false);
                self.harness.simulated_phone().return_to_range();
            }
            // The Phone stays through the suspend, and leaves only after
            // resume, as real BlueZ may report it; it is back a moment later.
            "suspend-phone-stays" => self.harness.simulated_phone().prepare_for_sleep(true),
            "resume-then-phone-leaves" => {
                let phone = self.harness.simulated_phone();
                phone.prepare_for_sleep(false);
                phone.disconnect();
                phone.return_to_range_after(LATE_LEAVING_RETURN);
            }
            "resume-slowly" => {
                self.harness.simulated_phone().prepare_for_sleep(false);
                self.harness
                    .simulated_phone()
                    .return_to_range_after(SLOW_RETURN);
            }
            "restart-wireplumber" => self
                .harness
                .simulated_phone()
                .restart_telephony(WIREPLUMBER_GAP),
            "move-audio-to-phone-and-back" => self.move_audio_to_phone_and_back(),
            "move-audio-to-phone" | "move-audio-here" => {
                let to = if action == "move-audio-here" {
                    "desktop"
                } else {
                    "phone"
                };
                if let Err(error) = self.harness.move_audio(to) {
                    println!("  demo: {}", error.message);
                }
            }
            other => self.act_on_call(other, call),
        }
        flush();
    }

    /// Moves the audio to the Phone and, once it is there, straight back,
    /// returning once it is back: an audio change the runner sees only in
    /// the app's published states, while it is still reading the log.
    fn move_audio_to_phone_and_back(&self) {
        // The Call Window offers each way back once the Handoff before it
        // has settled (ADR-0005).
        for (location, back) in [
            ("phone", "Move audio here"),
            ("desktop", "Move audio to phone"),
        ] {
            if let Err(error) = self.harness.move_audio(location) {
                println!("  demo: {}", error.message);
                return;
            }
            self.harness
                .wait_for_app("the Handoff settling", AUDIO_DEADLINE, |state| {
                    state.audio_location == location
                        && state.connection_state == "connected"
                        && state
                            .surfaces
                            .get("call-window")
                            .and_then(|window| window.get("handoff"))
                            .is_some_and(|label| label == back)
                });
        }
    }

    fn act_on_call(&self, action: &str, call: Option<u32>) {
        let harness = &self.harness;
        let phone = harness.simulated_phone();
        let on_phone = |act: &dyn Fn(&str)| match &self.call_path {
            Some(path) => act(path),
            None => println!("  demo: the helper hasn't called yet; type call first."),
        };
        let on_desktop = |act: &dyn Fn(u32)| match call {
            Some(call) => act(call),
            None => println!("  demo: there is no Call to act on."),
        };
        match action {
            "answer-on-phone" => on_phone(&|path| phone.answer_on_phone(path)),
            "decline-on-phone" => on_phone(&|path| phone.decline_on_phone(path)),
            "hang-up-on-phone" => on_phone(&|path| phone.caller_hangs_up(path)),
            "answer" => on_desktop(&|call| harness.answer(call)),
            "decline" => on_desktop(&|call| harness.decline(call)),
            "hang-up" => on_desktop(&|call| harness.hang_up(call)),
            "mute" | "unmute" => {
                if let Err(error) = harness.set_muted(action == "mute") {
                    println!("  demo: {}", error.message);
                }
            }
            "keybinding" => {
                let _ = harness.cli(&[]);
                harness.wait_for_app("a surface takes keyboard focus", FOCUS_DEADLINE, |state| {
                    state
                        .surfaces
                        .values()
                        .any(|content| content.get("focused").map(String::as_str) == Some("true"))
                });
            }
            "escape" => harness.keyboard().press(Key::Escape),
            "pill-click" => click_call_pill(harness),
            "wait-for-call-pill" => {
                harness.wait_for_app("the Call Pill", COLLAPSE_DEADLINE, |state| {
                    state.surface("call-pill").is_some()
                });
            }
            "expand" => {
                if harness.app_state().surface("call-pill").is_some() {
                    click_call_pill(harness);
                }
            }
            "indicator-click" => harness.tray().activate(),
            "indicator-mute" => harness.tray().click("Mute"),
            "indicator-hang-up" => harness.tray().click("Hang up"),
            "a" | "d" => harness.keyboard().type_text(action),
            other => match other.strip_prefix("quirk ") {
                Some(quirk) => phone.enable_quirk(quirk.trim()),
                None => println!(
                    "  demo: unknown action {other:?}. The actions: call, call-withheld, \
                     answer-on-phone, decline-on-phone, hang-up-on-phone, ring-out, answer, \
                     decline, hang-up, mute, unmute, keybinding, escape, pill-click, expand, wait-for-call-pill, a, d, \
                     click-notification, enter, phone-dials, forget-pairing, restart-app, \
                     open-setup-window, contacts-access-off, allow-contacts, \
                     bind-super-alt-c-elsewhere, cancel-then-confirm,indicator-click, indicator-mute, \
                     indicator-hang-up, do-not-disturb-on, do-not-disturb-off, type-test-number, type-wrong-number, \
                     type-wrong-first-digit, type-wrong-country, ctrl-h, callee-answers, \
                     earbuds-on, out-of-range, return-to-range, suspend, resume, resume-slowly, \
                     suspend-phone-stays, resume-then-phone-leaves, \
                     restart-wireplumber, move-audio-to-phone, move-audio-here, \
                     move-audio-to-phone-and-back, \
                     quirk NAME; or skip, or quit."
                ),
            },
        }
    }
}

/// Clicks the Call Pill's body, away from its buttons, where the app says
/// it sits: anchored bottom right, its gap from both edges, at its size.
fn click_call_pill(harness: &Harness) {
    let state = harness.app_state();
    let Some(pill) = state.surface("call-pill") else {
        println!("  demo: the Call Pill isn't showing.");
        return;
    };
    let number = |key: &str| {
        pill.get(key)
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(0.0)
    };
    let (screen_width, screen_height) = LAYOUT_SIZE;
    let left = screen_width - number("margin") - number("width");
    let middle = screen_height - number("margin") - number("height") / 2.0;
    harness.pointer().click(left + 30.0, middle);
}
