//! The Call Pill: the strip the Call Window collapses to during a Call, in
//! the bottom-right corner, 320 px wide and 44 high. From the left: a live
//! dot or a state icon, the caller, a tag where one applies, the timer, a
//! divider, then Mute and Hang up as icon buttons with tooltips. It shares
//! its bottom-right corner with the expanded Call Window.
//!
//! Nothing on it is red until the pointer is over Hang up, or Hang up has
//! keyboard focus; one click still hangs up. Muted, the whole strip takes
//! the warning tint and border. Once the Call has ended, or contact with it
//! is lost, it shows only who and what happened.
//!
//! It is on the top layer, like the Call Window, and never takes the
//! keyboard. A click on its body expands the Call Window, which takes the
//! keyboard as it does for the keybinding; its buttons act directly.

use super::{
    Anchor, HANG_UP_SHORTCUT, Icon, IconView, MUTE_SHORTCUT, SharedReport, Surface, Who, class,
    hang_up_button, keycap, report_mute, seconds_since, set_class, status_label,
};
use desktop_calls_core::messages::{self, Label};
use desktop_calls_core::{
    AudioLocation, CallPillLead, CallWindowPhase, CallWindowView, Input, Mute, StateLine,
};
use futures_channel::mpsc::UnboundedSender;
use gtk::prelude::*;
use gtk4_layer_shell::{KeyboardMode, Layer};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::rc::Rc;

/// The Call Pill's width. The caller takes what the rest leaves.
pub(super) const PILL_WIDTH: i32 = 320;
/// Mute's and Hang up's size: 32 px squares.
const BUTTON_SIZE: i32 = 32;

/// What leads the strip at rest, by its name in the lead's stack.
const LIVE_DOT: &str = "live";
const STATE_ICON: &str = "state";
const CHEVRON: &str = "chevron";

/// The Call Pill.
pub(super) struct CallPill {
    pub(super) surface: Surface,
    /// The live dot, the state icon or, under the pointer, the chevron.
    lead: gtk::Stack,
    state_icon: IconView,
    /// Whether the pointer is over the body, and what leads at rest.
    pointer: Rc<Pointer>,
    caller: gtk::Label,
    tag: gtk::Label,
    status: gtk::Label,
    divider: gtk::Separator,
    mute: gtk::Button,
    mute_icon: IconView,
    /// Whether Mute shows the microphone muted, so a click unmutes.
    muted: Rc<Cell<bool>>,
    hang_up: gtk::Button,
    /// Mute's and Hang up's shortcuts in the expanded Call Window, as
    /// shown.
    mute_shortcut: String,
    hang_up_shortcut: String,
}

impl CallPill {
    pub(super) fn new(intents: &UnboundedSender<Input>, report: SharedReport) -> Self {
        // A live Call's corner, which the expanded Call Window shares.
        let surface = Surface::new(
            "call-pill",
            Anchor::BottomRight,
            Layer::Top,
            KeyboardMode::None,
            None,
            intents,
            report,
        );
        surface.window.add_css_class("call-pill");
        surface.window.set_default_size(PILL_WIDTH, -1);

        let (lead, state_icon) = lead();
        let caller = gtk::Label::new(None);
        caller.add_css_class("pill-caller");
        caller.set_xalign(0.0);
        caller.set_hexpand(true);
        caller.set_ellipsize(gtk::pango::EllipsizeMode::End);
        // Asks for little, so the strip stays 320 px wide; the caller then
        // takes all the room the rest leaves.
        caller.set_width_chars(4);
        caller.set_max_width_chars(4);
        let tag = gtk::Label::new(None);
        tag.add_css_class("pill-tag");
        let status = gtk::Label::new(None);
        status.add_css_class(class::TIMER);
        status.set_xalign(1.0);
        let body = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        body.set_hexpand(true);
        body.set_tooltip_text(Some(&messages::show_call_tooltip()));
        for part in [
            lead.upcast_ref::<gtk::Widget>(),
            caller.upcast_ref(),
            tag.upcast_ref(),
            status.upcast_ref(),
        ] {
            part.set_valign(gtk::Align::Center);
            body.append(part);
        }

        let divider = gtk::Separator::new(gtk::Orientation::Vertical);
        divider.add_css_class("pill-divider");
        let mute_icon = IconView::new(Icon::Microphone);
        let mute = icon_button(&mute_icon, "mute");
        let muted = Rc::new(Cell::new(false));
        let (shown_muted, mute_intents) = (muted.clone(), intents.clone());
        mute.connect_clicked(move |_| {
            let _ = mute_intents.unbounded_send(Input::SetMuted(!shown_muted.get()));
        });
        let hang_up = hang_up_button(&surface, intents);
        hang_up.add_css_class("pill-button");
        hang_up.set_child(Some(&IconView::new(Icon::HangUp).area));
        hang_up.set_valign(gtk::Align::Center);
        hang_up.set_focus_on_click(false);
        let hang_up_shortcut = keycap(HANG_UP_SHORTCUT).1;
        hang_up.set_tooltip_text(Some(&messages::call_pill_tooltip(
            Label::HangUp,
            &hang_up_shortcut,
        )));

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        // As high as Mute and Hang up, so the Call Pill stays 44 px high
        // once they go.
        row.set_size_request(-1, BUTTON_SIZE);
        row.append(&body);
        row.append(&divider);
        row.append(&mute);
        row.append(&hang_up);
        surface.window.set_child(Some(&row));
        expand_on_click(&surface.window, intents);
        let pointer = Rc::new(Pointer::default());
        chevron_on_hover(&body, &surface.window, &lead, &pointer);
        Self {
            surface,
            lead,
            state_icon,
            pointer,
            caller,
            tag,
            status,
            divider,
            mute,
            mute_icon,
            muted,
            hang_up,
            mute_shortcut: keycap(MUTE_SHORTCUT).1,
            hang_up_shortcut,
        }
    }

    /// Brings the Call Pill up to date with `view`, and shows it if
    /// `on_screen`; otherwise only reports it as shown, while the Call
    /// Window shrinks into it.
    pub(super) fn render(&self, view: &CallWindowView, on_screen: bool) {
        self.surface.call.set(view.call);
        let who = Who::of(&view.caller, view.contact.as_ref());
        self.caller.set_text(&who.caller);
        set_class(&self.caller, class::NUMBER, who.bare_number);
        let mute = match view.phase {
            CallWindowPhase::Live { mute, .. } if !view.connection_lost => Some(mute),
            _ => None,
        };
        let status = status(view);
        let quiet = view.call_pill_quiet();
        set_class(&self.surface.window, "quiet", quiet);
        // Monospace only for a time on its own.
        set_class(&self.status, class::WORDS, mute.is_none());
        self.status.set_text(&status);
        let tag = view.call_pill_tag();
        self.tag
            .set_text(tag.map(messages::call_pill_tag).unwrap_or_default());
        self.tag.set_visible(tag.is_some());
        let audio_on_phone = !view.connection_lost
            && view
                .audio
                .is_some_and(|audio| audio.location == AudioLocation::Phone);
        self.show_lead(view.call_pill_lead());

        self.muted.set(mute == Some(Mute::On));
        let mute_label = if self.muted.get() {
            Label::Unmute
        } else {
            Label::Mute
        };
        let mute_text = messages::label(mute_label);
        let mute_tooltip = messages::call_pill_tooltip(mute_label, &self.mute_shortcut);
        self.mute.set_tooltip_text(Some(&mute_tooltip));
        self.mute_icon.set(if self.muted.get() {
            Icon::MicrophoneMuted
        } else {
            Icon::Microphone
        });
        set_class(&self.mute, class::MUTED, self.muted.get());
        set_class(&self.surface.window, class::MUTED, self.muted.get());
        set_class(
            &self.surface.window,
            class::CONTACT_LOST,
            view.connection_lost,
        );
        self.mute
            .set_sensitive(matches!(mute, Some(Mute::Off | Mute::On)));
        self.hang_up.set_sensitive(
            !quiet && !view.waiting_first && view.phase != CallWindowPhase::NotPlaced,
        );
        for control in [
            self.divider.upcast_ref::<gtk::Widget>(),
            self.mute.upcast_ref(),
            self.hang_up.upcast_ref(),
        ] {
            control.set_visible(!quiet);
        }

        let mut content = BTreeMap::from([
            ("status".to_owned(), status),
            ("call-window".to_owned(), "collapsed".to_owned()),
            ("caller".to_owned(), who.caller.clone()),
            ("tooltip".to_owned(), messages::show_call_tooltip()),
        ]);
        if let Some(call) = view.call {
            content.insert("call".to_owned(), call.to_string());
        }
        if !quiet {
            content.insert(
                "hang-up".to_owned(),
                messages::label(Label::HangUp).to_owned(),
            );
            content.insert(
                "hang-up-tooltip".to_owned(),
                messages::call_pill_tooltip(Label::HangUp, &self.hang_up_shortcut),
            );
            content.insert("mute-tooltip".to_owned(), mute_tooltip);
        }
        if let Some(tag) = tag {
            content.insert("tag".to_owned(), messages::call_pill_tag(tag).to_owned());
        }
        if let Some(mute) = mute {
            report_mute(mute, mute_text, &mut content);
        }
        if audio_on_phone {
            content.insert(
                "audio-location".to_owned(),
                messages::label(Label::AudioOnPhone).to_owned(),
            );
        }
        if view.held.is_some() {
            content.insert(
                "held".to_owned(),
                messages::label(Label::OneOnHold).to_owned(),
            );
        }
        self.report_geometry(&mut content);
        if on_screen {
            self.surface.show(content);
        } else {
            self.surface.report_ahead(content);
        }
    }

    /// Shows what leads the strip at rest, or nothing once it is quiet.
    /// The chevron takes its place while the pointer is over the body.
    fn show_lead(&self, lead: Option<CallPillLead>) {
        let icon = match lead {
            Some(CallPillLead::Muted) => Some(Icon::MicrophoneMuted),
            Some(CallPillLead::OnPhone) => Some(Icon::Phone),
            Some(CallPillLead::OnHold) => Some(Icon::OnHold),
            Some(CallPillLead::Live) | None => None,
        };
        if let Some(icon) = icon {
            self.state_icon.set(icon);
        }
        set_class(
            &self.state_icon.area,
            class::MUTED,
            lead == Some(CallPillLead::Muted),
        );
        let at_rest = lead.map(|lead| match lead {
            CallPillLead::Live => LIVE_DOT,
            CallPillLead::Muted | CallPillLead::OnPhone | CallPillLead::OnHold => STATE_ICON,
        });
        self.pointer.at_rest.set(at_rest);
        self.lead.set_visible(at_rest.is_some());
        self.pointer.show(&self.lead);
    }

    /// Hides the Call Pill, or takes back the report of it shown ahead.
    pub(super) fn hide(&self) {
        if self.surface.window.is_visible() {
            self.surface.hide();
        } else {
            self.surface.report_gone();
        }
    }

    /// Adds how big the Call Pill is, in layout pixels, for the gallery's
    /// checks. The surface adds the corner it is anchored to and its gap
    /// from both edges.
    fn report_geometry(&self, content: &mut BTreeMap<String, String>) {
        let window = &self.surface.window;
        let (_, natural_width, _, _) = window.measure(gtk::Orientation::Horizontal, -1);
        let width = natural_width.max(PILL_WIDTH);
        let (_, height, _, _) = window.measure(gtk::Orientation::Vertical, width);
        for (key, value) in [("width", width.to_string()), ("height", height.to_string())] {
            content.insert(key.to_owned(), value);
        }
    }
}

/// What the Call Pill shows in place of a timer, or the timer: while
/// contact is lost, where the Call goes on; once it has ended, "Call ended"
/// and how long it lasted, as the Call Window says it.
fn status(view: &CallWindowView) -> String {
    match view.phase {
        // Its timer would mislead: the Desktop can't tell whether the Call
        // still goes on.
        _ if view.connection_lost => messages::label(Label::OnYourPhone).to_owned(),
        CallWindowPhase::Live { active_since, .. } => {
            messages::elapsed(seconds_since(active_since))
        }
        CallWindowPhase::Ended => {
            let line = view.state_line.unwrap_or(StateLine::Ended { lasted: None });
            messages::state_line(line, &view.caller, view.contact.as_ref())
        }
        phase => messages::label(status_label(&phase)).to_owned(),
    }
}

/// The strip's lead, 16 px square: the live dot, the state icon, or the
/// chevron. Returns it and the state icon.
fn lead() -> (gtk::Stack, IconView) {
    let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    dot.add_css_class(class::LIVE_DOT);
    dot.set_halign(gtk::Align::Center);
    dot.set_valign(gtk::Align::Center);
    let state_icon = IconView::new(Icon::Phone);
    state_icon.area.add_css_class("pill-state");
    let chevron = IconView::new(Icon::ChevronUp);
    chevron.area.add_css_class("pill-state");
    let lead = gtk::Stack::new();
    lead.set_size_request(16, 16);
    lead.add_named(&dot, Some(LIVE_DOT));
    lead.add_named(&state_icon.area, Some(STATE_ICON));
    lead.add_named(&chevron.area, Some(CHEVRON));
    (lead, state_icon)
}

/// A 32 px square button showing `icon`, with `class`.
fn icon_button(icon: &IconView, class: &str) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class(class);
    button.add_css_class("pill-button");
    button.set_child(Some(&icon.area));
    button.set_valign(gtk::Align::Center);
    button.set_focus_on_click(false);
    button
}

/// Whether the pointer is over the Call Pill's body, and what leads the
/// strip when it isn't.
#[derive(Default)]
struct Pointer {
    over_body: Cell<bool>,
    at_rest: Cell<Option<&'static str>>,
}

impl Pointer {
    /// Shows the chevron in `lead` while the pointer is over the body,
    /// otherwise what leads at rest.
    fn show(&self, lead: &gtk::Stack) {
        if let Some(at_rest) = self.at_rest.get() {
            lead.set_visible_child_name(if self.over_body.get() {
                CHEVRON
            } else {
                at_rest
            });
        }
    }
}

/// While the pointer is over the Call Pill's body, not its buttons, the
/// lead shows the chevron, and the strip lightens: a click expands it
/// upwards.
fn chevron_on_hover(
    body: &gtk::Box,
    window: &gtk::Window,
    lead: &gtk::Stack,
    pointer: &Rc<Pointer>,
) {
    let motion = gtk::EventControllerMotion::new();
    let hovered = move |pointer: &Pointer, window: &gtk::Window, lead: &gtk::Stack, over: bool| {
        pointer.over_body.set(over);
        set_class(window, "hovered", over);
        pointer.show(lead);
    };
    let (shown, strip, state) = (lead.clone(), window.clone(), pointer.clone());
    motion.connect_enter(move |_, _, _| hovered(&state, &strip, &shown, true));
    let (shown, strip, state) = (lead.clone(), window.clone(), pointer.clone());
    motion.connect_leave(move |_| hovered(&state, &strip, &shown, false));
    body.add_controller(motion);
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
