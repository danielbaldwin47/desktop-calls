//! The Call Pill: the strip the Call Window collapses to during a Call, in
//! the bottom-right corner, with the caller, the timer, Mute and Hang up.
//!
//! It is on the top layer, like the Call Window, and never takes the
//! keyboard. A click on its body expands the Call Window, which takes the
//! keyboard as it does for the keybinding; its buttons act directly.

use super::{SharedReport, Surface, Who, hang_up_button, report_mute, seconds_since, status_label};
use crate::surfaces::GAP;
use desktop_calls_core::messages::{self, Label};
use desktop_calls_core::{CallWindowPhase, CallWindowView, Input, Mute};
use futures_channel::mpsc::UnboundedSender;
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell as _};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::rc::Rc;

/// How wide the Call Pill is: about a third of the Call Window less.
const PILL_WIDTH: i32 = 280;

/// The Call Pill.
pub(super) struct CallPill {
    pub(super) surface: Surface,
    caller: gtk::Label,
    status: gtk::Label,
    mute: gtk::Button,
    /// Whether Mute shows the microphone muted, so a click unmutes.
    muted: Rc<Cell<bool>>,
    hang_up: gtk::Button,
}

impl CallPill {
    pub(super) fn new(intents: &UnboundedSender<Input>, report: SharedReport) -> Self {
        let surface = Surface::new(
            "call-pill",
            Layer::Top,
            KeyboardMode::None,
            None,
            intents,
            report,
        );
        surface.window.add_css_class("call-pill");
        surface.window.set_default_size(PILL_WIDTH, -1);
        if gtk4_layer_shell::is_supported() {
            // The top-right corner is the Banner's and the toasts'.
            surface.window.set_anchor(Edge::Top, false);
            surface.window.set_anchor(Edge::Bottom, true);
            surface.window.set_margin(Edge::Top, 0);
            surface.window.set_margin(Edge::Bottom, GAP);
            surface.window.set_margin(Edge::Right, GAP);
        }
        let caller = gtk::Label::new(None);
        caller.add_css_class("pill-caller");
        caller.set_xalign(0.0);
        caller.set_ellipsize(gtk::pango::EllipsizeMode::End);
        // Room for a number written out in full, and most names.
        caller.set_width_chars(15);
        caller.set_max_width_chars(15);
        let status = gtk::Label::new(None);
        status.add_css_class("numeric");
        status.add_css_class("dim-label");
        status.set_xalign(0.0);
        // The caller over the timer, so the strip stays narrow and one row
        // of controls high.
        let who = gtk::Box::new(gtk::Orientation::Vertical, 0);
        who.set_hexpand(true);
        who.set_valign(gtk::Align::Center);
        who.append(&caller);
        who.append(&status);
        let mute = gtk::Button::with_label(messages::label(Label::Mute));
        mute.add_css_class("mute");
        let muted = Rc::new(Cell::new(false));
        let (shown_muted, mute_intents) = (muted.clone(), intents.clone());
        mute.connect_clicked(move |_| {
            let _ = mute_intents.unbounded_send(Input::SetMuted(!shown_muted.get()));
        });
        let hang_up = hang_up_button(&surface, intents);
        hang_up.set_child(Some(&gtk::Label::new(Some(messages::label(Label::HangUp)))));
        for button in [&mute, &hang_up] {
            button.set_valign(gtk::Align::Center);
        }
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.append(&who);
        row.append(&mute);
        row.append(&hang_up);
        surface.window.set_child(Some(&row));
        expand_on_click(&surface.window, intents);
        Self {
            surface,
            caller,
            status,
            mute,
            muted,
            hang_up,
        }
    }

    /// Shows the Call Pill for `view`, or brings it up to date.
    pub(super) fn render(&self, view: &CallWindowView) {
        self.surface.call.set(view.call);
        let who = Who::of(&view.caller, view.contact.as_ref());
        self.caller.set_text(&who.caller);
        let (status, mute) = match view.phase {
            CallWindowPhase::Live {
                active_since, mute, ..
            } => (messages::elapsed(seconds_since(active_since)), Some(mute)),
            phase => (messages::label(status_label(&phase)).to_owned(), None),
        };
        self.status.set_text(&status);
        let mute = mute.filter(|_| !view.connection_lost);
        let mute_text = messages::label(if mute == Some(Mute::On) {
            Label::Unmute
        } else {
            Label::Mute
        });
        self.mute.set_label(mute_text);
        self.muted.set(mute == Some(Mute::On));
        if self.muted.get() {
            self.mute.add_css_class("muted");
        } else {
            self.mute.remove_css_class("muted");
        }
        self.mute
            .set_sensitive(matches!(mute, Some(Mute::Off | Mute::On)));
        self.hang_up.set_sensitive(
            !view.waiting_first
                && !view.connection_lost
                && !matches!(
                    view.phase,
                    CallWindowPhase::Ended | CallWindowPhase::NotPlaced
                ),
        );

        let mut content = BTreeMap::from([
            ("status".to_owned(), status),
            ("call-window".to_owned(), "collapsed".to_owned()),
            (
                "hang-up".to_owned(),
                messages::label(Label::HangUp).to_owned(),
            ),
        ]);
        if let Some(call) = view.call {
            content.insert("call".to_owned(), call.to_string());
        }
        content.insert("caller".to_owned(), who.caller.clone());
        if let Some(mute) = mute {
            report_mute(mute, mute_text, &mut content);
        }
        self.report_geometry(&mut content);
        self.surface.show(content);
    }

    pub(super) fn hide(&self) {
        if self.surface.window.is_visible() {
            self.surface.hide();
        }
    }

    /// Adds where the Call Pill sits and how big it is, for the gallery's
    /// checks: the corner it is anchored to, its gap from both edges, and
    /// its size in layout pixels.
    fn report_geometry(&self, content: &mut BTreeMap<String, String>) {
        let window = &self.surface.window;
        let (_, natural_width, _, _) = window.measure(gtk::Orientation::Horizontal, -1);
        let width = natural_width.max(PILL_WIDTH);
        let (_, height, _, _) = window.measure(gtk::Orientation::Vertical, width);
        for (key, value) in [
            ("anchor", "bottom-right".to_owned()),
            ("margin", GAP.to_string()),
            ("width", width.to_string()),
            ("height", height.to_string()),
        ] {
            content.insert(key.to_owned(), value);
        }
    }
}

/// A click on the Call Pill's body, not on one of its buttons, expands the
/// Call Window.
fn expand_on_click(window: &gtk::Window, intents: &UnboundedSender<Input>) {
    let click = gtk::GestureClick::new();
    let (pill, intents) = (window.clone(), intents.clone());
    click.connect_released(move |_, _, x, y| {
        let on_button = pill
            .pick(x, y, gtk::PickFlags::DEFAULT)
            .is_some_and(|widget| widget.ancestor(gtk::Button::static_type()).is_some());
        if !on_button {
            let _ = intents.unbounded_send(Input::ExpandCallWindow);
        }
    });
    window.add_controller(click);
}
