//! First-frame work, done while the main loop is idle after startup, so the
//! first Banner and the first Dialer draw as fast as later ones (ticket #47).
//!
//! The first time GTK's renderer draws each kind of thing (a colour, a
//! rounded corner, a border, text, a clip, a translucent layer), it compiles
//! a GPU shader program for it. On a cold shader cache that took most of the
//! Dialer's first frame: about 294 of its 306 ms on the Desktop, and over a
//! second under CI-like load (ticket #83). Compiled programs are shared by
//! every window, so drawing each kind once, offscreen, through any
//! surface's renderer leaves nothing to compile when a surface first shows.
//!
//! The kinds below are the programs that drawing every surface in every
//! state compiles (the screenshot gallery, traced with `GSK_DEBUG=shaders`,
//! 2026-11-05). The work starts only once the app is quiet: the Phone
//! Connected, no Call, nothing shown, so startup never waits for it. Each
//! kind is one piece of work in its own idle turn of the main loop, at a
//! lower priority than anything else on it. The moment the app is busy again
//! (a Call, or any surface about to show), the work is abandoned for good,
//! before anything is shown, so a Call that rings meanwhile waits for one
//! piece at most. Nothing is mapped: the drawing goes to a texture that is
//! thrown away.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::gdk::RGBA;
use gtk::glib::{self, ControlFlow};
use gtk::graphene::{Rect, Size};
use gtk::gsk::{self, RoundedRect};
use gtk::prelude::*;

use crate::logging::EventKind;

/// The size of each offscreen drawing.
const SIZE: f32 = 32.0;

/// Text with the glyphs a caller's number and the Dialer's rows use most.
const TEXT: &str = "+1 (555) 0123-456789 Answer Decline";

/// One kind of thing to draw once: its name in the log, and how to draw it.
type Kind = (&'static str, fn(&gtk::Snapshot, &gtk::Window));

/// Each kind of thing the surfaces draw, one compiled program apiece. The
/// colour is translucent and the clips off the pixel grid, because GTK
/// draws an opaque rectangle or a clip on whole pixels without a program.
const KINDS: [Kind; 10] = [
    // `gskgpucolor`: backgrounds.
    ("colour", |snapshot, _| {
        snapshot.append_color(&grey(), &square());
    }),
    // `gskgpuroundedcolor`: backgrounds with rounded corners.
    ("rounded-colour", |snapshot, _| {
        snapshot.push_rounded_clip(&rounded());
        snapshot.append_color(&grey(), &square());
        snapshot.pop();
    }),
    // `gskgpucolor` under a rounded clip: buttons inside rounded corners.
    ("colour-in-rounded-clip", |snapshot, _| {
        snapshot.push_rounded_clip(&rounded());
        snapshot.append_color(&grey(), &Rect::new(4.0, 4.0, SIZE, SIZE));
        snapshot.pop();
    }),
    // `gskgpuborder`: the entry, and the HD mark.
    ("border", |snapshot, _| {
        snapshot.append_border(&rounded(), &[1.0; 4], &[grey(); 4]);
    }),
    // `gskgpucolorize`: text.
    ("text", |snapshot, window| {
        snapshot.append_layout(&window.create_pango_layout(Some(TEXT)), &grey());
    }),
    // `gskgpucolorize` under a clip: the Dialer's rows, in their scrolled list.
    ("clipped-text", |snapshot, window| {
        snapshot.push_clip(&Rect::new(0.5, 0.5, SIZE / 2.0, SIZE / 2.0));
        snapshot.append_layout(&window.create_pango_layout(Some(TEXT)), &grey());
        snapshot.pop();
    }),
    // A Cairo drawing, uploaded and drawn as a texture: the icons, such as
    // the Dialer's search icon (ticket #227). Each clip is a variant of its
    // own: without these three the Dialer's first frame took 26 ms longer.
    ("cairo", |snapshot, _| stroke(snapshot, &square())),
    ("cairo-in-clip", |snapshot, _| {
        snapshot.push_clip(&Rect::new(0.5, 0.5, SIZE / 2.0, SIZE / 2.0));
        stroke(snapshot, &square());
        snapshot.pop();
    }),
    ("cairo-in-rounded-clip", |snapshot, _| {
        snapshot.push_rounded_clip(&rounded());
        stroke(snapshot, &Rect::new(4.0, 4.0, SIZE, SIZE));
        snapshot.pop();
    }),
    // `gskgpuconvert`: a layer drawn offscreen, then blended in.
    ("translucent-layer", |snapshot, _| {
        snapshot.push_opacity(0.5);
        snapshot.append_color(&grey(), &square());
        snapshot.append_color(&grey(), &Rect::new(4.0, 4.0, SIZE, SIZE));
        snapshot.pop();
    }),
];

/// A circle stroked with Cairo over `bounds`, as an icon is drawn.
fn stroke(snapshot: &gtk::Snapshot, bounds: &Rect) {
    let cairo = snapshot.append_cairo(bounds);
    cairo.set_source_rgba(0.5, 0.5, 0.5, 0.5);
    cairo.arc(16.0, 16.0, 8.0, 0.0, std::f64::consts::TAU);
    // A failed stroke draws nothing; there is nothing to warm up instead.
    let _ = cairo.stroke();
}

fn grey() -> RGBA {
    RGBA::new(0.5, 0.5, 0.5, 0.5)
}

fn square() -> Rect {
    Rect::new(0.0, 0.0, SIZE, SIZE)
}

fn rounded() -> RoundedRect {
    let corner = Size::new(6.0, 6.0);
    RoundedRect::new(square(), corner, corner, corner, corner)
}

/// The first-frame work: started once the app is quiet, and abandoned for
/// good the moment it is not.
pub(super) struct FirstFrames {
    work: Rc<Work>,
}

struct Work {
    /// A realized surface window, never mapped here, whose renderer draws.
    window: gtk::Window,
    phase: Cell<Phase>,
    /// The next kind to draw.
    next: Cell<usize>,
    /// The work's own time so far, not counting the waits between pieces.
    took: Cell<Duration>,
    /// The longest piece so far, and its kind.
    longest: Cell<(Duration, &'static str)>,
    /// When the work started, for the time it spanned.
    started: Cell<Option<Instant>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// The app has not yet been quiet.
    Waiting,
    Running,
    /// Every kind drawn, abandoned, or impossible.
    Over,
}

impl FirstFrames {
    /// Work drawn through `window`'s renderer. `window` must be realized.
    pub(super) fn new(window: gtk::Window) -> Self {
        Self {
            work: Rc::new(Work {
                window,
                phase: Cell::new(Phase::Waiting),
                next: Cell::new(0),
                took: Cell::new(Duration::ZERO),
                longest: Cell::new((Duration::ZERO, "")),
                started: Cell::new(None),
            }),
        }
    }

    /// Starts the work the first time the app is `quiet`, and abandons it
    /// for good the first time after that it is not.
    pub(super) fn while_quiet(&self, quiet: bool) {
        let work = &self.work;
        match (work.phase.get(), quiet) {
            (Phase::Waiting, true) => {
                work.phase.set(Phase::Running);
                let work = work.clone();
                glib::idle_add_local_full(glib::Priority::LOW, move || work.draw_next());
            }
            (Phase::Running, false) => {
                work.phase.set(Phase::Over);
                abandoned(work.next.get());
            }
            _ => {}
        }
    }
}

impl Work {
    /// Draws the next kind: one piece of work per idle turn of the main loop.
    fn draw_next(&self) -> ControlFlow {
        if self.phase.get() != Phase::Running {
            return ControlFlow::Break;
        }
        let Some(&(kind, draw)) = KINDS.get(self.next.get()) else {
            return ControlFlow::Break;
        };
        let Some(renderer) = self.window.renderer() else {
            self.phase.set(Phase::Over);
            unprepared("unrealized");
            return ControlFlow::Break;
        };
        let started = Instant::now();
        self.started.set(self.started.get().or(Some(started)));
        let snapshot = gtk::Snapshot::new();
        draw(&snapshot, &self.window);
        if let Some(node) = snapshot.to_node() {
            render(&renderer, &node);
        }
        let took = started.elapsed();
        self.took.set(self.took.get() + took);
        if took > self.longest.get().0 {
            self.longest.set((took, kind));
        }
        self.next.set(self.next.get() + 1);
        if self.next.get() < KINDS.len() {
            return ControlFlow::Continue;
        }
        self.phase.set(Phase::Over);
        self.prepared();
        ControlFlow::Break
    }

    fn prepared(&self) {
        let (longest, longest_kind) = self.longest.get();
        let spanned = self
            .started
            .get()
            .map_or(Duration::ZERO, |started| started.elapsed());
        tracing::info!(
            event_kind = %EventKind::FirstFramesPrepared,
            took_ms = millis(self.took.get()),
            longest_ms = millis(longest),
            longest_kind,
            spanned_ms = millis(spanned),
            "Prepared the surfaces' first frames in {} ms of idle time",
            millis(self.took.get())
        );
    }
}

/// Draws `node` into a texture, which is thrown away: the point is what the
/// renderer compiles on the way.
fn render(renderer: &gsk::Renderer, node: &gsk::RenderNode) {
    let _texture = renderer.render_texture(node, Some(&square()));
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn abandoned(drawn: usize) {
    tracing::info!(
        event_kind = %EventKind::FirstFramesAbandoned,
        drawn,
        kinds = KINDS.len(),
        "Abandoned the surfaces' first frames after {drawn} of {} kinds: the app is busy",
        KINDS.len()
    );
}

fn unprepared(error_kind: &str) {
    tracing::warn!(
        event_kind = %EventKind::FirstFramesUnprepared,
        error_kind,
        "Couldn't prepare the surfaces' first frames; the first of each may be slow"
    );
}
