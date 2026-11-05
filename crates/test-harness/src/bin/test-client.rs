//! `test-client`: the test harness's own Wayland client.
//!
//! It opens one plain window and prints each keyboard and pointer event it
//! receives on standard output, one per line:
//!
//! - `configured <width> <height>`, once the window is drawn at a size the
//!   compositor chose, and the drawing has been sent
//! - `keyboard-enter`, `keyboard-leave`
//! - `key <code> pressed` or `key <code> released` (Linux key codes)
//! - `modifiers <mask>`, whenever the depressed modifiers change
//! - `pointer-enter <x> <y>`, `pointer-leave`
//! - `button <code> pressed at <x> <y> showing state <n>` or `... released
//!   ...`, where `<n>` is the state the window last finished drawing
//!
//! The harness's self-tests drive it to prove that synthetic input arrives.
//! With `--fullscreen` it asks to be fullscreen, as a video player does.
//!
//! With `--slow-redraw` it stands in for an app slow to redraw after a state
//! change, as the app is on a loaded CI runner: each key pressed is a state
//! change, numbered from 1, that it prints as `state <n>` at once, before
//! redrawing. It then keeps the CPU busy for [`BUSY`], draws [`STEPS`]
//! frames in [`DRAWING_PIXEL`], one each time the compositor shows the last,
//! and only then the state itself: [`ODD_STATE_PIXEL`] for an odd state,
//! [`PIXEL`] for an even one. The harness's self-tests use it to prove that
//! an action straight after a state change waits for the redraw.
//!
//! With `--bar` it is instead a stand-in for Omarchy's bar: a layer-shell
//! surface across the top of the output, [`BAR_HEIGHT`] tall, that reserves
//! its height as an exclusive zone, drawn in [`BAR_PIXEL`]. It prints
//! `configured <width> <height>` once drawn.

use std::io::Write as _;
use std::os::fd::AsFd as _;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_buffer, wl_callback, wl_compositor, wl_keyboard, wl_pointer, wl_registry, wl_seat, wl_shm,
    wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

/// The window's size when the compositor leaves it to the client.
const DEFAULT_SIZE: (i32, i32) = (640, 480);

/// An opaque mid grey, in `ARGB8888` as little-endian bytes.
const PIXEL: [u8; 4] = [0x80, 0x80, 0x80, 0xff];

/// With `--slow-redraw`: how long the CPU is kept busy after a state change
/// before the first frame, and how many frames come before the state's own.
const BUSY: Duration = Duration::from_millis(300);
const STEPS: u32 = 20;
/// The colour of the frames drawn on the way to a state, RGB 0x40 0x40 0x40,
/// and of an odd state, RGB 0x20 0x60 0xa0, in `ARGB8888` as little-endian
/// bytes.
const DRAWING_PIXEL: [u8; 4] = [0x40, 0x40, 0x40, 0xff];
const ODD_STATE_PIXEL: [u8; 4] = [0xa0, 0x60, 0x20, 0xff];

/// Omarchy's bar's height. Keep in step with `test_harness::BAR_HEIGHT`.
const BAR_HEIGHT: u32 = 26;
/// The bar stand-in's colour, RGB 0x30 0x30 0x60, in `ARGB8888` as
/// little-endian bytes. Keep in step with `test_harness::BAR_COLOUR`.
const BAR_PIXEL: [u8; 4] = [0x60, 0x30, 0x30, 0xff];

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("test-client: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let connection = Connection::connect_to_env().map_err(|error| error.to_string())?;
    let (globals, mut queue) =
        registry_queue_init::<Client>(&connection).map_err(|error| error.to_string())?;
    let handle = queue.handle();
    let missing = |name: &'static str| move |error| format!("no {name}: {error}");
    let compositor: wl_compositor::WlCompositor = globals
        .bind(&handle, 4..=6, ())
        .map_err(missing("wl_compositor"))?;
    let shm: wl_shm::WlShm = globals
        .bind(&handle, 1..=1, ())
        .map_err(missing("wl_shm"))?;
    let wm_base: xdg_wm_base::XdgWmBase = globals
        .bind(&handle, 1..=5, ())
        .map_err(missing("xdg_wm_base"))?;
    let _seat: wl_seat::WlSeat = globals
        .bind(&handle, 5..=7, ())
        .map_err(missing("wl_seat"))?;

    let surface = compositor.create_surface(&handle, ());
    let bar = std::env::args().any(|argument| argument == "--bar");
    if bar {
        let layer_shell: zwlr_layer_shell_v1::ZwlrLayerShellV1 =
            globals
                .bind(&handle, 1..=4, ())
                .map_err(missing("zwlr_layer_shell_v1"))?;
        let layer_surface = layer_shell.get_layer_surface(
            &surface,
            None,
            zwlr_layer_shell_v1::Layer::Top,
            "omarchy-bar".to_owned(),
            &handle,
            (),
        );
        layer_surface.set_anchor(
            zwlr_layer_surface_v1::Anchor::Top
                | zwlr_layer_surface_v1::Anchor::Left
                | zwlr_layer_surface_v1::Anchor::Right,
        );
        layer_surface.set_size(0, BAR_HEIGHT);
        layer_surface.set_exclusive_zone(i32::try_from(BAR_HEIGHT).unwrap_or(i32::MAX));
    } else {
        let xdg_surface = wm_base.get_xdg_surface(&surface, &handle, ());
        let toplevel = xdg_surface.get_toplevel(&handle, ());
        toplevel.set_title("Desktop Calls test client".to_owned());
        toplevel.set_app_id("desktop-calls-test-client".to_owned());
        if std::env::args().any(|argument| argument == "--fullscreen") {
            toplevel.set_fullscreen(None);
        }
    }
    surface.commit();

    let mut client = Client {
        shm,
        surface,
        size: DEFAULT_SIZE,
        size_chosen: false,
        pixel: if bar { BAR_PIXEL } else { PIXEL },
        keyboard: None,
        pointer: None,
        modifiers: 0,
        pointer_position: (0.0, 0.0),
        slow_redraw: std::env::args().any(|argument| argument == "--slow-redraw"),
        state: 0,
        shown: 0,
        steps_left: 0,
        failure: None,
    };
    while client.failure.is_none() {
        queue
            .blocking_dispatch(&mut client)
            .map_err(|error| error.to_string())?;
    }
    Err(client.failure.unwrap_or_default())
}

struct Client {
    shm: wl_shm::WlShm,
    surface: wl_surface::WlSurface,
    size: (i32, i32),
    /// Whether the compositor chose `size`.
    size_chosen: bool,
    /// The colour the surface is filled with.
    pixel: [u8; 4],
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    modifiers: u32,
    pointer_position: (f64, f64),
    /// Whether each key pressed is a state change, redrawn slowly.
    slow_redraw: bool,
    /// The latest state, and the one the window last finished drawing.
    state: u32,
    shown: u32,
    /// Frames still to draw before the latest state's own.
    steps_left: u32,
    failure: Option<String>,
}

impl Client {
    /// Draws at the size the compositor chose and prints `configured`.
    fn configured(&mut self, connection: &Connection, handle: &QueueHandle<Self>) {
        let drawn = self
            .draw(handle)
            .and_then(|()| connection.flush().map_err(|error| error.to_string()));
        match drawn {
            Ok(()) if self.size_chosen => {
                let (width, height) = self.size;
                print(&format!("configured {width} {height}"));
            }
            Ok(()) => {}
            Err(error) => self.failure = Some(error),
        }
    }

    /// With `--slow-redraw`, a key was pressed: a new state, printed at once
    /// and then redrawn slowly.
    fn change_state(&mut self, connection: &Connection, handle: &QueueHandle<Self>) {
        self.state += 1;
        print(&format!("state {}", self.state));
        let busy_until = Instant::now() + BUSY;
        while Instant::now() < busy_until {
            std::hint::spin_loop();
        }
        self.steps_left = STEPS;
        self.next_frame(connection, handle);
    }

    /// Draws the next frame on the way to the latest state, or the state
    /// itself once there are no more.
    fn next_frame(&mut self, connection: &Connection, handle: &QueueHandle<Self>) {
        if self.steps_left == 0 {
            self.pixel = if self.state % 2 == 1 {
                ODD_STATE_PIXEL
            } else {
                PIXEL
            };
            self.shown = self.state;
        } else {
            self.steps_left -= 1;
            self.pixel = DRAWING_PIXEL;
            self.surface.frame(handle, ());
        }
        let drawn = self
            .draw(handle)
            .and_then(|()| connection.flush().map_err(|error| error.to_string()));
        if let Err(error) = drawn {
            self.failure = Some(error);
        }
    }

    /// Fills the window with one colour at its current size.
    fn draw(&mut self, handle: &QueueHandle<Self>) -> Result<(), String> {
        let (width, height) = self.size;
        let stride = width * 4;
        let length = usize::try_from(stride * height).map_err(|error| error.to_string())?;
        let mut file = tempfile::tempfile().map_err(|error| error.to_string())?;
        file.write_all(&self.pixel.repeat(length / 4))
            .map_err(|error| error.to_string())?;
        let pool = self
            .shm
            .create_pool(file.as_fd(), stride * height, handle, ());
        let buffer = pool.create_buffer(
            0,
            width,
            height,
            stride,
            wl_shm::Format::Argb8888,
            handle,
            (),
        );
        pool.destroy();
        self.surface.attach(Some(&buffer), 0, 0);
        self.surface.damage_buffer(0, 0, width, height);
        self.surface.commit();
        Ok(())
    }
}

/// Prints one event line. Standard output is line-buffered, so the harness
/// sees each line as soon as it is printed.
fn print(line: &str) {
    println!("{line}");
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Client {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for Client {
    fn event(
        _: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for Client {
    fn event(
        client: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            xdg_toplevel::Event::Configure { width, height, .. } => {
                client.size_chosen = width > 0 && height > 0;
                client.size = if client.size_chosen {
                    (width, height)
                } else {
                    DEFAULT_SIZE
                };
            }
            xdg_toplevel::Event::Close => client.failure = Some("closed".to_owned()),
            _ => {}
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for Client {
    fn event(
        client: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        (): &(),
        connection: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
            client.configured(connection, handle);
        }
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for Client {
    fn event(
        client: &mut Self,
        layer_surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        (): &(),
        connection: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => {
                layer_surface.ack_configure(serial);
                let size = (i32::try_from(width), i32::try_from(height));
                if let (Ok(width), Ok(height)) = size {
                    client.size = (width, height);
                    client.size_chosen = width > 0 && height > 0;
                }
                client.configured(connection, handle);
            }
            zwlr_layer_surface_v1::Event::Closed => client.failure = Some("closed".to_owned()),
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for Client {
    fn event(
        client: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        (): &(),
        _: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
        {
            if capabilities.contains(wl_seat::Capability::Keyboard) && client.keyboard.is_none() {
                client.keyboard = Some(seat.get_keyboard(handle, ()));
            }
            if capabilities.contains(wl_seat::Capability::Pointer) && client.pointer.is_none() {
                client.pointer = Some(seat.get_pointer(handle, ()));
            }
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for Client {
    fn event(
        client: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        (): &(),
        connection: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Enter { .. } => print("keyboard-enter"),
            wl_keyboard::Event::Leave { .. } => print("keyboard-leave"),
            wl_keyboard::Event::Key { key, state, .. } => {
                let pressed = matches!(state, WEnum::Value(wl_keyboard::KeyState::Pressed));
                let state = if pressed { "pressed" } else { "released" };
                print(&format!("key {key} {state}"));
                if pressed && client.slow_redraw {
                    client.change_state(connection, handle);
                }
            }
            wl_keyboard::Event::Modifiers { mods_depressed, .. }
                if mods_depressed != client.modifiers =>
            {
                client.modifiers = mods_depressed;
                print(&format!("modifiers {mods_depressed}"));
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for Client {
    fn event(
        client: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter {
                surface_x,
                surface_y,
                ..
            } => {
                client.pointer_position = (surface_x, surface_y);
                print(&format!("pointer-enter {surface_x} {surface_y}"));
            }
            wl_pointer::Event::Leave { .. } => print("pointer-leave"),
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => client.pointer_position = (surface_x, surface_y),
            wl_pointer::Event::Button { button, state, .. } => {
                let state = match state {
                    WEnum::Value(wl_pointer::ButtonState::Pressed) => "pressed",
                    _ => "released",
                };
                let (x, y) = client.pointer_position;
                print(&format!(
                    "button {button} {state} at {x} {y} showing state {}",
                    client.shown
                ));
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for Client {
    fn event(
        client: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        (): &(),
        connection: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        // The compositor has shown the last frame: time for the next.
        if let wl_callback::Event::Done { .. } = event {
            client.next_frame(connection, handle);
        }
    }
}

delegate_noop!(Client: ignore wl_compositor::WlCompositor);
delegate_noop!(Client: ignore wl_surface::WlSurface);
delegate_noop!(Client: ignore wl_shm::WlShm);
delegate_noop!(Client: ignore wl_shm_pool::WlShmPool);
delegate_noop!(Client: ignore wl_buffer::WlBuffer);
delegate_noop!(Client: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);
