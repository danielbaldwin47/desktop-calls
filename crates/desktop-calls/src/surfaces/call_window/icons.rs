//! The Call Window's icons, drawn in the colour of the text around them, so
//! they follow the theme and every state's colour (muted, the accent tile,
//! Hang up's label) without an icon theme. Each is drawn on a 16-unit grid,
//! with 1.5-unit round strokes.

use gtk::cairo;
use gtk::prelude::*;
use std::cell::Cell;
use std::f64::consts::PI;
use std::rc::Rc;

/// The icons the Call Window draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Icon {
    Microphone,
    MicrophoneMuted,
    Keypad,
    Phone,
    Computer,
    HangUp,
    /// A Call placed here, on its way out.
    Outgoing,
    /// A Call answered here, connecting.
    Incoming,
    OnHold,
    Warning,
    /// What a click on the Call Pill's body does: it expands upwards.
    ChevronUp,
}

/// An icon, 16 px square, that can be changed in place.
#[derive(Clone)]
pub(super) struct IconView {
    pub(super) area: gtk::DrawingArea,
    icon: Rc<Cell<Icon>>,
}

impl IconView {
    pub(super) fn new(icon: Icon) -> Self {
        let area = gtk::DrawingArea::new();
        area.set_content_width(16);
        area.set_content_height(16);
        area.set_valign(gtk::Align::Center);
        area.set_halign(gtk::Align::Center);
        let icon = Rc::new(Cell::new(icon));
        let shown = icon.clone();
        area.set_draw_func(move |area, cr, width, height| {
            let colour = area.color();
            cr.set_source_rgba(
                f64::from(colour.red()),
                f64::from(colour.green()),
                f64::from(colour.blue()),
                f64::from(colour.alpha()),
            );
            cr.translate(
                (f64::from(width) - 16.0) / 2.0,
                (f64::from(height) - 16.0) / 2.0,
            );
            cr.set_line_width(1.5);
            cr.set_line_cap(cairo::LineCap::Round);
            cr.set_line_join(cairo::LineJoin::Round);
            // A failed stroke leaves the icon blank; there is nothing else
            // to draw instead.
            let _ = draw(cr, shown.get());
        });
        Self { area, icon }
    }

    pub(super) fn set(&self, icon: Icon) {
        if self.icon.replace(icon) != icon {
            self.area.queue_draw();
        }
    }
}

/// A rectangle with corners of `radius`, as a path.
fn rounded(cr: &cairo::Context, x: f64, y: f64, width: f64, height: f64, radius: f64) {
    cr.new_sub_path();
    cr.arc(x + width - radius, y + radius, radius, -PI / 2.0, 0.0);
    cr.arc(
        x + width - radius,
        y + height - radius,
        radius,
        0.0,
        PI / 2.0,
    );
    cr.arc(x + radius, y + height - radius, radius, PI / 2.0, PI);
    cr.arc(x + radius, y + radius, radius, PI, 1.5 * PI);
    cr.close_path();
}

fn line(cr: &cairo::Context, from: (f64, f64), to: (f64, f64)) {
    cr.move_to(from.0, from.1);
    cr.line_to(to.0, to.1);
}

fn microphone(cr: &cairo::Context) {
    rounded(cr, 6.0, 2.0, 4.0, 7.0, 2.0);
    cr.new_sub_path();
    cr.arc_negative(8.0, 7.5, 4.5, PI, 0.0);
    line(cr, (8.0, 12.0), (8.0, 14.0));
}

fn draw(cr: &cairo::Context, icon: Icon) -> Result<(), cairo::Error> {
    match icon {
        Icon::Microphone => microphone(cr),
        Icon::MicrophoneMuted => {
            microphone(cr);
            line(cr, (2.5, 2.5), (13.5, 13.5));
        }
        Icon::Keypad => {
            for y in [4.0, 8.0, 12.0] {
                for x in [4.0, 8.0, 12.0] {
                    cr.new_sub_path();
                    cr.arc(x, y, 1.1, 0.0, 2.0 * PI);
                }
            }
            return cr.fill();
        }
        Icon::Phone => {
            rounded(cr, 4.5, 1.5, 7.0, 13.0, 1.5);
            line(cr, (7.2, 12.3), (8.8, 12.3));
        }
        Icon::Computer => {
            rounded(cr, 3.0, 3.5, 10.0, 7.0, 1.0);
            line(cr, (1.5, 12.5), (14.5, 12.5));
        }
        Icon::HangUp => {
            // A handset on its back, as phones draw End.
            cr.move_to(1.8, 9.7);
            cr.curve_to(5.4, 6.3, 10.6, 6.3, 14.2, 9.7);
            cr.line_to(12.5, 11.6);
            cr.line_to(10.0, 10.4);
            cr.line_to(10.0, 8.8);
            cr.curve_to(8.7, 8.37, 7.3, 8.37, 6.0, 8.8);
            cr.line_to(6.0, 10.4);
            cr.line_to(3.5, 11.6);
            cr.close_path();
            return cr.fill();
        }
        Icon::Outgoing => {
            line(cr, (4.0, 12.0), (11.0, 5.0));
            cr.move_to(6.0, 5.0);
            cr.line_to(11.0, 5.0);
            cr.line_to(11.0, 10.0);
        }
        Icon::Incoming => {
            line(cr, (12.0, 4.0), (5.0, 11.0));
            cr.move_to(10.0, 11.0);
            cr.line_to(5.0, 11.0);
            cr.line_to(5.0, 6.0);
        }
        Icon::OnHold => {
            line(cr, (6.0, 4.0), (6.0, 12.0));
            line(cr, (10.0, 4.0), (10.0, 12.0));
        }
        Icon::ChevronUp => {
            cr.move_to(4.0, 10.0);
            cr.line_to(8.0, 6.0);
            cr.line_to(12.0, 10.0);
        }
        Icon::Warning => {
            cr.move_to(8.0, 2.2);
            cr.line_to(14.5, 13.5);
            cr.line_to(1.5, 13.5);
            cr.close_path();
            line(cr, (8.0, 6.5), (8.0, 10.0));
            line(cr, (8.0, 11.7), (8.0, 12.0));
        }
    }
    cr.stroke()
}
