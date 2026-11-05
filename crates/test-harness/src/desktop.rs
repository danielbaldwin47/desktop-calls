//! The desktop around the app: the notification and tray stand-ins, the
//! keyboard, the clipboard, the pointer and test clients.

use std::process::{Child, Stdio};
use std::sync::{Arc, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use super::Harness;
use crate::screen::Screen;
use crate::stand_ins::StandIns;
use crate::timeline::Timeline;
use crate::{
    Key, LAYOUT_SIZE, MenuEntry, Notification, OmarchyTheme, TestClient, TrayItem, stand_ins,
};

impl Harness {
    /// The notification server stand-in.
    #[must_use]
    pub fn notifications(&self) -> Notifications<'_> {
        Notifications {
            stand_ins: &self.stand_ins,
            timeline: &self.timeline,
        }
    }

    /// The tray watcher stand-in.
    #[must_use]
    pub fn tray(&self) -> Tray<'_> {
        Tray {
            stand_ins: &self.stand_ins,
            timeline: &self.timeline,
        }
    }

    /// Turns Omarchy's Do Not Disturb on or off, by rewriting its state file
    /// as Omarchy does. The notification server stand-in silences the
    /// notifications that arrive from now on as Omarchy would.
    ///
    /// # Panics
    ///
    /// If the file can't be written.
    pub fn set_do_not_disturb(&self, on: bool) {
        stand_ins::write_do_not_disturb(&self.directory.join("home"), on)
            .expect("write Omarchy's notification state file");
        self.stand_ins.set_do_not_disturb(on);
    }

    /// Switches Omarchy's current theme to `theme`, as `omarchy-theme-set`
    /// does: the theme directory is deleted and a new one moved into its
    /// place. Returns once the switch is on disk.
    ///
    /// # Panics
    ///
    /// If the theme can't be written.
    pub fn switch_omarchy_theme(&self, theme: OmarchyTheme) {
        stand_ins::switch_omarchy_theme(&self.directory.join("home"), theme)
            .expect("switch Omarchy's theme");
    }

    /// Changes Hyprland's `decoration:rounding` and announces the reload, as
    /// Hyprland does when its configuration is reloaded.
    ///
    /// # Panics
    ///
    /// If the scenario has no Hyprland stand-in.
    pub fn set_hyprland_rounding(&self, rounding: u32) {
        self.hyprland
            .as_ref()
            .expect("this needs Hyprland: start with Harness::builder().hyprland_rounding(…)")
            .set_rounding(rounding);
    }

    /// Types on the screen's synthetic keyboard, into whatever has focus.
    ///
    /// # Panics
    ///
    /// If the scenario has no screen.
    #[must_use]
    pub fn keyboard(&self) -> Keyboard<'_> {
        self.screen();
        Keyboard { harness: self }
    }

    /// Puts `text` on the screen's clipboard, as copying it in another app
    /// does, and waits until a paste would get it.
    ///
    /// # Panics
    ///
    /// If the scenario has no screen, `wl-copy` (from `wl-clipboard`) can't
    /// start, or a paste doesn't get `text` within 5 s.
    pub fn set_clipboard(&self, text: &str) -> Clipboard {
        self.screen();
        let owner = self
            .external_command("wl-copy")
            .args(["--foreground", "--", text])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start wl-copy, from wl-clipboard");
        let clipboard = Clipboard { owner };
        let started = Instant::now();
        loop {
            let pasted = self
                .external_command("wl-paste")
                .args(["--no-newline", "--type", "text/plain"])
                .stdin(Stdio::null())
                .output()
                .expect("run wl-paste, from wl-clipboard");
            if pasted.status.success() && pasted.stdout == text.as_bytes() {
                return clipboard;
            }
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "the clipboard never held {text:?}; wl-paste said {:?}{}",
                String::from_utf8_lossy(&pasted.stderr),
                self.timeline.render()
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Moves and clicks the screen's synthetic pointer.
    ///
    /// # Panics
    ///
    /// If the scenario has no screen.
    #[must_use]
    pub fn pointer(&self) -> Pointer<'_> {
        self.screen();
        Pointer { harness: self }
    }

    /// Starts the harness's own client, which opens a window on the screen
    /// and prints the input it receives.
    ///
    /// # Panics
    ///
    /// If the scenario has no screen, or the client can't start.
    #[must_use]
    pub fn start_test_client(&self) -> TestClient {
        self.spawn_test_client(&[])
    }

    /// Starts the harness's test client fullscreen, as a video player would
    /// be. See [`Harness::start_test_client`].
    ///
    /// # Panics
    ///
    /// If the scenario has no screen, or the client can't be started.
    #[must_use]
    pub fn start_fullscreen_test_client(&self) -> TestClient {
        self.spawn_test_client(&["--fullscreen"])
    }

    /// Starts the harness's test client as an app slow to redraw: each key
    /// it gets is a state change, which it reports at once and draws only
    /// after 300 ms of work and 20 frames. See `src/bin/test-client.rs`.
    ///
    /// # Panics
    ///
    /// If the scenario has no screen, or the client can't be started.
    #[must_use]
    pub fn start_slowly_redrawing_test_client(&self) -> TestClient {
        self.spawn_test_client(&["--slow-redraw"])
    }

    pub(super) fn spawn_test_client(&self, arguments: &[&str]) -> TestClient {
        self.screen();
        let child = self
            .command("test-client")
            .args(arguments)
            .stdout(Stdio::piped())
            .stderr(self.log_file("test-client.stderr"))
            .spawn()
            .expect("start test-client");
        self.test_clients
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(child.id());
        TestClient::new(child, Arc::clone(&self.timeline))
    }

    /// Blocks until the screen closes: the compositor exits, or, for a
    /// nested screen, its window is closed.
    ///
    /// # Panics
    ///
    /// If the scenario has no screen.
    pub fn wait_until_screen_closes(&self) {
        self.screen().input().wait_until_closed();
    }

    pub(super) fn screen(&self) -> &Screen {
        self.screen
            .as_ref()
            .expect("this needs a screen: start with Harness::builder().headless_screen()")
    }
}

/// The notification server stand-in. See [`Harness::notifications`].
pub struct Notifications<'harness> {
    stand_ins: &'harness StandIns,
    timeline: &'harness Timeline,
}

impl Notifications<'_> {
    /// Every notification received so far, in order.
    #[must_use]
    pub fn received(&self) -> Vec<Notification> {
        self.stand_ins.notifications()
    }

    /// Waits until a received notification satisfies `condition`, and
    /// returns it.
    ///
    /// # Panics
    ///
    /// If `deadline` passes first. The panic names `description` and every
    /// notification received.
    pub fn wait_for(
        &self,
        description: &str,
        deadline: Duration,
        condition: impl Fn(&Notification) -> bool,
    ) -> Notification {
        self.stand_ins
            .wait_for_notification(deadline, condition)
            .unwrap_or_else(|received| {
                panic!(
                    "\"{description}\" did not happen within {deadline:?}; received {received:#?}{}",
                    self.timeline.render()
                )
            })
    }

    /// Clicks notification `id`, as Omarchy does: invokes its `default`
    /// action and closes it.
    ///
    /// # Panics
    ///
    /// If the notification offered no `default` action.
    pub fn invoke_default_action(&self, id: u32) {
        if let Err(error) = self.stand_ins.invoke_default_action(id) {
            panic!("click notification {id}: {error}");
        }
    }

    /// Presses notification `id`'s button for action `key`, such as
    /// `"call-back"`, and closes it, as Omarchy does.
    ///
    /// # Panics
    ///
    /// If the notification offered no such action.
    pub fn invoke_action(&self, id: u32, key: &str) {
        if let Err(error) = self.stand_ins.invoke_action(id, key) {
            panic!("press {key:?} on notification {id}: {error}");
        }
    }
}

/// The tray watcher stand-in. See [`Harness::tray`].
pub struct Tray<'harness> {
    stand_ins: &'harness StandIns,
    timeline: &'harness Timeline,
}

impl Tray<'_> {
    /// Every registered item, with its status and icon as it reports them now.
    #[must_use]
    pub fn items(&self) -> Vec<TrayItem> {
        self.stand_ins.tray_items()
    }

    /// Waits until the registered items satisfy `condition`, and returns them.
    ///
    /// # Panics
    ///
    /// If `deadline` passes first. The panic names `description` and the
    /// items.
    pub fn wait_for(
        &self,
        description: &str,
        deadline: Duration,
        condition: impl Fn(&[TrayItem]) -> bool,
    ) -> Vec<TrayItem> {
        self.stand_ins
            .wait_for_tray(deadline, condition)
            .unwrap_or_else(|error| {
                panic!(
                    "\"{description}\" did not happen within {deadline:?}: {error}{}",
                    self.timeline.render()
                )
            })
    }

    /// Every status the registered items have reported so far, in order,
    /// each change once, as the tray read them on each signal.
    #[must_use]
    pub fn statuses_seen(&self) -> Vec<String> {
        self.stand_ins.tray_statuses()
    }

    /// Left-clicks the one registered item, as Omarchy's bar does.
    ///
    /// # Panics
    ///
    /// If there isn't exactly one item, or it refuses.
    pub fn activate(&self) {
        if let Err(error) = self.stand_ins.activate_tray_item() {
            panic!("activate the tray item: {error}");
        }
    }

    /// The one registered item's menu, as the tray reads it on opening.
    ///
    /// # Panics
    ///
    /// If there isn't exactly one item, or its menu can't be read.
    #[must_use]
    pub fn menu(&self) -> Vec<MenuEntry> {
        self.stand_ins
            .tray_menu()
            .unwrap_or_else(|error| panic!("read the tray item's menu: {error}"))
    }

    /// Clicks the entry labelled `label` in the one registered item's menu,
    /// here or in a submenu.
    ///
    /// # Panics
    ///
    /// If the menu has no such entry, it is disabled, or the click fails.
    pub fn click(&self, label: &str) {
        if let Err(error) = self.stand_ins.click_tray_menu(label) {
            panic!("click {label:?} in the tray item's menu: {error}");
        }
    }
}

/// The screen's synthetic keyboard. See [`Harness::keyboard`]. Keys go to
/// the screen as it is once it has finished redrawing (see `redraw.rs`).
pub struct Keyboard<'harness> {
    harness: &'harness Harness,
}

impl Keyboard<'_> {
    /// Types `text` with a US layout, holding Shift where a character needs it.
    ///
    /// # Panics
    ///
    /// If `text` has a character the US layout can't type, the screen
    /// doesn't finish redrawing, or the compositor is gone.
    pub fn type_text(&self, text: &str) {
        self.harness.wait_for_redraw();
        if let Err(error) = self.harness.screen().input().type_text(text) {
            panic!("type {text:?}: {error}");
        }
    }

    /// Presses and releases `key`.
    ///
    /// # Panics
    ///
    /// If the screen doesn't finish redrawing, or the compositor is gone.
    pub fn press(&self, key: Key) {
        self.harness.wait_for_redraw();
        if let Err(error) = self.harness.screen().input().press(key) {
            panic!("press {key:?}: {error}");
        }
    }

    /// Presses `character`'s key while holding Control, such as Ctrl+V.
    ///
    /// # Panics
    ///
    /// If the US layout has no unshifted key for `character`, the screen
    /// doesn't finish redrawing, or the compositor is gone.
    pub fn press_with_control(&self, character: char) {
        self.harness.wait_for_redraw();
        if let Err(error) = self.harness.screen().input().press_with_control(character) {
            panic!("press Ctrl+{character}: {error}");
        }
    }
}

/// The program holding the screen's clipboard. Dropping it empties the
/// clipboard. See [`Harness::set_clipboard`].
#[must_use = "the clipboard empties when this is dropped"]
pub struct Clipboard {
    owner: Child,
}

impl Drop for Clipboard {
    fn drop(&mut self) {
        let _ = self.owner.kill();
        let _ = self.owner.wait();
    }
}

/// The screen's synthetic pointer, in layout coordinates: the headless
/// screen is [`LAYOUT_SIZE`]. See [`Harness::pointer`]. It acts on the
/// screen as it is once it has finished redrawing (see `redraw.rs`).
pub struct Pointer<'harness> {
    harness: &'harness Harness,
}

impl Pointer<'_> {
    /// Moves the pointer to (`x`, `y`).
    ///
    /// # Panics
    ///
    /// If the point is off the screen, the screen doesn't finish redrawing,
    /// or the compositor is gone.
    pub fn move_to(&self, x: f64, y: f64) {
        self.harness.wait_for_redraw();
        self.move_now(x, y);
    }

    /// Moves the pointer to (`x`, `y`) and clicks the left button there.
    /// Moving onto a button can redraw it (hover), so the click waits for
    /// that redraw too.
    ///
    /// # Panics
    ///
    /// If the point is off the screen, the screen doesn't finish redrawing,
    /// or the compositor is gone.
    pub fn click(&self, x: f64, y: f64) {
        self.move_to(x, y);
        self.harness.wait_for_redraw();
        if let Err(error) = self.harness.screen().input().click() {
            panic!("click at ({x}, {y}): {error}");
        }
    }

    fn move_now(&self, x: f64, y: f64) {
        if let Err(error) = self.harness.screen().input().move_to(x, y, LAYOUT_SIZE) {
            panic!("move the pointer to ({x}, {y}): {error}");
        }
    }
}
