//! Bringing the Call Window up to date with the Call state machine's view
//! and the clock, and reporting what it shows.

use super::{CallWindow, Change, Icon, Who, class, seconds_since, set_class, shows};
use desktop_calls_core::messages::{self, Label};
use desktop_calls_core::{
    AudioLocation, CallAudio, CallWindowPhase, CallWindowView, Mute, StateLine,
};
use gtk::prelude::*;
use std::collections::BTreeMap;
use std::rc::Rc;

impl CallWindow {
    pub(in super::super) fn render(self: &Rc<Self>, view: Option<&CallWindowView>) {
        let new_call =
            self.view.borrow().as_ref().map(|view| view.call) != view.map(|view| view.call);
        *self.view.borrow_mut() = view.cloned();
        if new_call {
            self.tones.set_text("");
            self.keypad_shown.button.set_active(false);
            self.lost_timer.replace(None);
        }
        let Some(view) = view else {
            self.stop_ticker();
            self.surface.hide();
            self.pill.hide();
            self.reset_change();
            return;
        };
        // While contact is lost, the timer stands still at the loss.
        let timing = matches!(
            view.phase,
            CallWindowPhase::Live { .. } | CallWindowPhase::OnHold { .. }
        ) && !view.connection_lost;
        if timing {
            self.start_ticker();
        } else {
            self.stop_ticker();
        }
        if view.collapsed {
            // The window keeps its keypad and tones for when it expands.
            if self.surface.window.is_visible() && self.change.get() != Some(Change::Collapsing) {
                self.start_collapsing();
            }
            self.update();
            return;
        }
        let from_pill =
            self.pill.surface.window.is_visible() || self.change.get() == Some(Change::Collapsing);
        self.pill.hide();
        self.surface.call.set(view.call);
        if from_pill {
            self.start_expanding();
        }
        self.update();
        self.surface.focus(view.focused);
    }

    /// Brings everything the window shows up to date with the view and the
    /// clock, and reports it.
    pub(super) fn update(&self) {
        let Some(view) = self.view.borrow().clone() else {
            return;
        };
        if view.collapsed {
            // While the window shrinks into it, the Call Pill is reported
            // as shown, and shows once the window has gone.
            self.pill
                .render(&view, self.change.get() != Some(Change::Collapsing));
            return;
        }
        let mut content = BTreeMap::new();
        if let Some(call) = view.call {
            content.insert("call".to_owned(), call.to_string());
        }
        self.show_who(&view, &mut content);
        self.show_timer(&view, &mut content);
        self.show_state_line(&view, &mut content);
        self.show_notice(&view, &mut content);
        let live = matches!(view.phase, CallWindowPhase::Live { .. }) && !view.connection_lost;
        self.show_tiles(&view, live, &mut content);
        self.show_keypad(live, &mut content);
        let lone = view.held.is_none()
            && !view.connection_lost
            && matches!(view.phase, CallWindowPhase::OnHold { .. });
        self.show_resume(lone, view.waiting_first, &mut content);
        self.show_hang_up(&view, &mut content);
        set_class(
            &self.surface.window,
            class::MUTED,
            live && matches!(view.phase, CallWindowPhase::Live { mute: Mute::On, .. }),
        );
        set_class(
            &self.surface.window,
            class::CONTACT_LOST,
            view.connection_lost,
        );
        set_class(
            &self.surface.window,
            class::ENDED,
            view.phase == CallWindowPhase::Ended,
        );
        self.held.update(&view, &mut content);
        // Each shortcut whose keycap is on screen and whose control works.
        content.extend(
            self.shortcuts
                .iter()
                .filter(|(_, (_, cap))| cap.as_ref().is_none_or(shows))
                .filter(|(key, _)| self.works(key))
                .map(|(key, (shown, _))| (key.clone(), shown.clone())),
        );
        self.surface.show(content);
    }

    /// Whether the control behind the shortcut reported as `key` works now.
    fn works(&self, key: &str) -> bool {
        match key {
            "mute-shortcut" => self.mute.button.is_sensitive(),
            "handoff-shortcut" => self.handoff.button.is_sensitive(),
            "hang-up-shortcut" => self.hang_up.is_sensitive(),
            "swap-shortcut" => self.resume.is_sensitive(),
            _ => true,
        }
    }

    /// Who the Call is with, and under it the number's label and the
    /// number, or "Not in Contacts" for a bare number.
    fn show_who(&self, view: &CallWindowView, content: &mut BTreeMap<String, String>) {
        let who = Who::of(&view.caller, view.contact.as_ref());
        who.show(&self.caller);
        who.show_detail(&self.meta);
        if who.bare_number {
            self.meta.set_text(messages::label(Label::NotInContacts));
            self.meta.set_visible(true);
        }
        if self.meta.is_visible() {
            content.insert("meta".to_owned(), self.meta.text().to_string());
        }
        who.report(content);
    }

    /// The timer, while the Call is connected: struck through and still
    /// once contact with it is lost.
    fn show_timer(&self, view: &CallWindowView, content: &mut BTreeMap<String, String>) {
        let running = match view.phase {
            CallWindowPhase::Live { active_since, .. }
            | CallWindowPhase::OnHold { active_since } => {
                Some(messages::elapsed(seconds_since(active_since)))
            }
            _ => None,
        };
        let timer = if view.connection_lost {
            self.lost_timer
                .borrow_mut()
                .get_or_insert_with(|| running.clone().unwrap_or_default())
                .clone()
        } else {
            self.lost_timer.replace(None);
            running.unwrap_or_default()
        };
        self.timer.set_text(&timer);
        self.timer.set_visible(!timer.is_empty());
        set_class(&self.timer, class::STRUCK, view.connection_lost);
        if !timer.is_empty() {
            content.insert("timer".to_owned(), timer);
        }
    }

    fn show_state_line(&self, view: &CallWindowView, content: &mut BTreeMap<String, String>) {
        let Some(line) = view.state_line else {
            self.state.row.set_visible(false);
            return;
        };
        let text = messages::state_line(line, &view.caller, view.contact.as_ref());
        self.state.text.set_text(&text);
        let icon = match line {
            StateLine::Connecting => Some(Icon::Incoming),
            StateLine::Calling | StateLine::Ringing => Some(Icon::Outgoing),
            StateLine::OnThisComputer { .. } => Some(Icon::Computer),
            StateLine::Muted => Some(Icon::MicrophoneMuted),
            StateLine::ConnectingToPhone | StateLine::OnYourPhone => Some(Icon::Phone),
            StateLine::OnHold => Some(Icon::OnHold),
            StateLine::Ended { .. } => None,
        };
        if let Some(icon) = icon {
            self.state.icon.set(icon);
        }
        self.state.icon.area.set_visible(icon.is_some());
        let wideband = matches!(line, StateLine::OnThisComputer { wideband: true });
        self.state.hd.set_visible(wideband);
        set_class(&self.state.row, class::MUTED, line == StateLine::Muted);
        self.state.row.set_visible(true);
        content.insert("state".to_owned(), text);
        if wideband {
            content.insert("hd".to_owned(), messages::label(Label::Hd).to_owned());
        }
    }

    /// The notice: while contact with the Call is lost, a heading and what
    /// to do; for a Call not placed, that, and why; otherwise why something
    /// failed, if anything did.
    fn show_notice(&self, view: &CallWindowView, content: &mut BTreeMap<String, String>) {
        let (heading, text) = match view.notice {
            Some(notice) if view.connection_lost => {
                let (heading, text) = messages::contact_lost(notice);
                (Some(heading), Some(text))
            }
            Some(notice) if view.phase == CallWindowPhase::NotPlaced => (
                Some(messages::label(Label::CallNotPlaced)),
                Some(messages::failure(notice).text),
            ),
            Some(notice) => (None, Some(messages::failure(notice).text)),
            None => (None, None),
        };
        self.notice.heading.set_text(heading.unwrap_or_default());
        self.notice.heading.set_visible(heading.is_some());
        self.notice.text.set_text(text.unwrap_or_default());
        self.notice.row.set_visible(text.is_some());
        if let Some(heading) = heading {
            content.insert("notice-heading".to_owned(), heading.to_owned());
        }
        if let Some(text) = text {
            content.insert("notice".to_owned(), text.to_owned());
        }
    }

    /// Mute, Keypad and the audio move, while the Call is live here or on
    /// the Phone and contact with it is intact.
    fn show_tiles(
        &self,
        view: &CallWindowView,
        live: bool,
        content: &mut BTreeMap<String, String>,
    ) {
        self.tiles.set_visible(live);
        // Hidden, the tiles ignore their shortcuts.
        let mute = match view.phase {
            CallWindowPhase::Live { mute, .. } if live => Some(mute),
            _ => None,
        };
        let muted = mute == Some(Mute::On);
        let mute_text = messages::label(if muted { Label::Unmute } else { Label::Mute });
        self.mute.label.set_text(mute_text);
        self.mute.icon.set(if muted {
            Icon::MicrophoneMuted
        } else {
            Icon::Microphone
        });
        self.mute
            .button
            .set_sensitive(matches!(mute, Some(Mute::Off | Mute::On)));
        set_class(&self.mute.button, class::MUTED, muted);
        self.keypad_shown.button.set_sensitive(live);
        let handoff = self.show_handoff(view.audio.filter(|_| live));
        if live {
            content.insert("mute".to_owned(), mute_text.to_owned());
            content.insert("keypad-toggle".to_owned(), "shown".to_owned());
            content.insert("handoff".to_owned(), handoff.to_owned());
        }
    }

    /// Brings the audio tile up to date with the Call's audio, `None` with
    /// no live Call, and returns its label. It names where the audio will
    /// go, is the one accent tile while the audio is on the Phone, and is
    /// disabled while the audio is on its way.
    fn show_handoff(&self, audio: Option<CallAudio>) -> &'static str {
        let (label, icon) = match audio {
            Some(CallAudio {
                moving: Some(_), ..
            }) => (Label::Moving, Icon::Phone),
            Some(CallAudio {
                location: AudioLocation::Desktop,
                ..
            }) => (Label::ToPhone, Icon::Phone),
            Some(CallAudio {
                location: AudioLocation::Phone,
                ..
            })
            | None => (Label::MoveHere, Icon::Computer),
        };
        let label = messages::label(label);
        self.handoff.label.set_text(label);
        self.handoff.icon.set(icon);
        set_class(
            &self.handoff.button,
            class::ACCENT,
            audio.is_some_and(|audio| {
                audio.location == AudioLocation::Phone && audio.moving.is_none()
            }),
        );
        self.handoff
            .button
            .set_sensitive(audio.is_some_and(|audio| audio.moving.is_none()));
        label
    }

    /// The tone entry and the keypad, while the keypad is open on a live
    /// Call. Closed, or with contact lost, both are hidden, and the tones
    /// typed stay for when they show again.
    fn show_keypad(&self, live: bool, content: &mut BTreeMap<String, String>) {
        let open = live && self.keypad_shown.button.is_active();
        let opening = open && !WidgetExt::is_visible(&self.tones);
        self.tones.set_visible(open);
        self.keypad.set_visible(open);
        self.tones.set_sensitive(live);
        self.keypad.set_sensitive(live);
        if opening {
            // Typing and pasting go to the tone entry, without selecting
            // the tones typed so far, which the next one would replace.
            self.tones.grab_focus_without_selecting();
        }
        if open {
            content.insert("tone-entry".to_owned(), "shown".to_owned());
            content.insert("keypad".to_owned(), "shown".to_owned());
            let tones = self.tones.text();
            if !tones.is_empty() {
                content.insert("tones".to_owned(), tones.to_string());
            }
        }
    }

    /// Resume, in place of the tiles, while the only Call is on hold. While
    /// a Waiting Call rings, the Phone would act on it instead, and the
    /// window says so in its place.
    fn show_resume(&self, lone: bool, waiting_first: bool, content: &mut BTreeMap<String, String>) {
        self.resume.set_visible(lone && !waiting_first);
        self.resume.set_sensitive(lone && !waiting_first);
        if lone && !waiting_first {
            content.insert("swap".to_owned(), messages::label(Label::Resume).to_owned());
        }
    }

    /// Hang up, at the foot of the window whenever there is a Call to hang
    /// up from here. While a Call is on hold and a Waiting Call rings, the
    /// Phone would hang up the Waiting Call instead, so it is disabled.
    fn show_hang_up(&self, view: &CallWindowView, content: &mut BTreeMap<String, String>) {
        let there = !view.connection_lost
            && !matches!(
                view.phase,
                CallWindowPhase::Ended | CallWindowPhase::NotPlaced
            );
        self.hang_up.set_visible(there);
        self.hang_up.set_sensitive(there && !view.waiting_first);
        if there {
            content.insert(
                "hang-up".to_owned(),
                messages::label(Label::HangUp).to_owned(),
            );
        }
    }
}
