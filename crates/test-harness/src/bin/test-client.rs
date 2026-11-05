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
//! - `button <code> pressed at <x> <y>` or `... released at ...`
//!
//! The harness's self-tests drive it to prove that synthetic input arrives.

use std::io::Write as _;
use std::os::fd::AsFd as _;
use std::process::ExitCode;

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_keyboard, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool,
    wl_surface,
};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

/// The window's size when the compositor leaves it to the client.
const DEFAULT_SIZE: (i32, i32) = (640, 480);

/// An opaque mid grey, in `ARGB8888` as little-endian bytes.
const PIXEL: [u8; 4] = [0x80, 0x80, 0x80, 0xff];

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
    let xdg_surface = wm_base.get_xdg_surface(&surface, &handle, ());
    let toplevel = xdg_surface.get_toplevel(&handle, ());
    toplevel.set_title("Desktop Calls test client".to_owned());
    toplevel.set_app_id("desktop-calls-test-client".to_owned());
    surface.commit();

    let mut client = Client {
        shm,
        surface,
        size: DEFAULT_SIZE,
        size_chosen: false,
        keyboard: None,
        pointer: None,
        modifiers: 0,
        pointer_position: (0.0, 0.0),
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
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    modifiers: u32,
    pointer_position: (f64, f64),
    failure: Option<String>,
}

impl Client {
    /// Fills the window with one colour at its current size.
    fn draw(&mut self, handle: &QueueHandle<Self>) -> Result<(), String> {
        let (width, height) = self.size;
        let stride = width * 4;
        let length = usize::try_from(stride * height).map_err(|error| error.to_string())?;
        let mut file = tempfile::tempfile().map_err(|error| error.to_string())?;
        file.write_all(&PIXEL.repeat(length / 4))
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
            let drawn = client
                .draw(handle)
                .and_then(|()| connection.flush().map_err(|error| error.to_string()));
            match drawn {
                Ok(()) if client.size_chosen => {
                    let (width, height) = client.size;
                    print(&format!("configured {width} {height}"));
                }
                Ok(()) => {}
                Err(error) => client.failure = Some(error),
            }
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
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Enter { .. } => print("keyboard-enter"),
            wl_keyboard::Event::Leave { .. } => print("keyboard-leave"),
            wl_keyboard::Event::Key { key, state, .. } => {
                let state = match state {
                    WEnum::Value(wl_keyboard::KeyState::Pressed) => "pressed",
                    _ => "released",
                };
                print(&format!("key {key} {state}"));
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
                print(&format!("button {button} {state} at {x} {y}"));
            }
            _ => {}
        }
    }
}

delegate_noop!(Client: ignore wl_compositor::WlCompositor);
delegate_noop!(Client: ignore wl_surface::WlSurface);
delegate_noop!(Client: ignore wl_shm::WlShm);
delegate_noop!(Client: ignore wl_shm_pool::WlShmPool);
delegate_noop!(Client: ignore wl_buffer::WlBuffer);
