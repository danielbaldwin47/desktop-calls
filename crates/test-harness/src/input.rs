//! Synthetic keyboard and pointer input on the scenario's screen.
//!
//! The harness holds one virtual keyboard (`zwp_virtual_keyboard_v1`) and one
//! virtual pointer (`zwlr_virtual_pointer_v1`) for the whole scenario. A
//! headless compositor has no input devices, so its seat offers clients no
//! keyboard or pointer at all until these exist; creating them at start means
//! every client binds them before the first key is typed.

use std::fs;
use std::io::Write as _;
use std::os::fd::AsFd as _;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Instant;

use wayland_client::globals::{GlobalList, GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_pointer::ButtonState;
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, delegate_noop};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

/// The keymap the virtual keyboard uses: US English, as `xkbcommon` builds
/// it from the system's XKB data. `xkb_v1` keymaps are sent NUL-terminated.
const KEYMAP: &str = "xkb_keymap {\n\
    xkb_keycodes { include \"evdev+aliases(qwerty)\" };\n\
    xkb_types { include \"complete\" };\n\
    xkb_compat { include \"complete\" };\n\
    xkb_symbols { include \"pc+us+inet(evdev)\" };\n\
};\n\0";

/// `wl_keyboard.keymap_format.xkb_v1`.
const KEYMAP_FORMAT_XKB_V1: u32 = 1;

/// The Shift and Control modifiers' bits in the keymap above.
const SHIFT_MASK: u32 = 1;
const CONTROL_MASK: u32 = 4;

/// Key codes from `linux/input-event-codes.h`.
const KEY_LEFTSHIFT: u32 = 42;
const KEY_LEFTCTRL: u32 = 29;
const BTN_LEFT: u32 = 0x110;

/// Sub-pixel precision for absolute pointer motion.
const POINTER_PRECISION: f64 = 100.0;

/// A key that types no character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Return,
    Escape,
    Tab,
    Backspace,
    Delete,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
}

impl Key {
    fn code(self) -> u32 {
        match self {
            Self::Escape => 1,
            Self::Backspace => 14,
            Self::Tab => 15,
            Self::Return => 28,
            Self::Home => 102,
            Self::Up => 103,
            Self::PageUp => 104,
            Self::Left => 105,
            Self::Right => 106,
            Self::End => 107,
            Self::Down => 108,
            Self::PageDown => 109,
            Self::Delete => 111,
        }
    }
}

/// The virtual keyboard and pointer, on their own Wayland connection.
pub(crate) struct SyntheticInput {
    connection: Connection,
    globals: GlobalList,
    queue: EventQueue<Globals>,
    keyboard: ZwpVirtualKeyboardV1,
    pointer: ZwlrVirtualPointerV1,
    started: Instant,
}

/// The connection's state: nothing, because the harness only sends.
struct Globals;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Globals {
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

delegate_noop!(Globals: ignore wl_seat::WlSeat);
delegate_noop!(Globals: ZwpVirtualKeyboardManagerV1);
delegate_noop!(Globals: ZwpVirtualKeyboardV1);
delegate_noop!(Globals: ZwlrVirtualPointerManagerV1);
delegate_noop!(Globals: ZwlrVirtualPointerV1);

impl SyntheticInput {
    /// Connects to the compositor at `socket` and creates the devices. The
    /// keymap is written to `keymap_path` to be handed over.
    pub fn connect(socket: &Path, keymap_path: &Path) -> Result<Self, String> {
        let stream = UnixStream::connect(socket)
            .map_err(|error| format!("connect to {}: {error}", socket.display()))?;
        let connection = Connection::from_socket(stream).map_err(|error| error.to_string())?;
        let (globals, mut queue) =
            registry_queue_init::<Globals>(&connection).map_err(|error| error.to_string())?;
        let handle = queue.handle();
        let bind_error =
            |name: &'static str| move |error| format!("the compositor offers no {name}: {error}");
        let seat: wl_seat::WlSeat = globals
            .bind(&handle, 1..=1, ())
            .map_err(bind_error("wl_seat"))?;
        let keyboards: ZwpVirtualKeyboardManagerV1 = globals
            .bind(&handle, 1..=1, ())
            .map_err(bind_error("zwp_virtual_keyboard_manager_v1"))?;
        let pointers: ZwlrVirtualPointerManagerV1 = globals
            .bind(&handle, 1..=1, ())
            .map_err(bind_error("zwlr_virtual_pointer_manager_v1"))?;

        let keyboard = keyboards.create_virtual_keyboard(&seat, &handle, ());
        let mut keymap = fs::File::create(keymap_path).map_err(|error| error.to_string())?;
        keymap
            .write_all(KEYMAP.as_bytes())
            .map_err(|error| error.to_string())?;
        let keymap = fs::File::open(keymap_path).map_err(|error| error.to_string())?;
        let size = u32::try_from(KEYMAP.len()).map_err(|error| error.to_string())?;
        keyboard.keymap(KEYMAP_FORMAT_XKB_V1, keymap.as_fd(), size);
        let pointer = pointers.create_virtual_pointer(Some(&seat), &handle, ());

        queue
            .roundtrip(&mut Globals)
            .map_err(|error| error.to_string())?;
        Ok(Self {
            connection,
            globals,
            queue,
            keyboard,
            pointer,
            started: Instant::now(),
        })
    }

    /// Types `text`, one key at a time, holding Shift where the character
    /// needs it.
    pub fn type_text(&mut self, text: &str) -> Result<(), String> {
        for character in text.chars() {
            let (code, shifted) = character_key(character)
                .ok_or_else(|| format!("the harness can't type {character:?}"))?;
            if shifted {
                self.key(KEY_LEFTSHIFT, true);
                self.keyboard.modifiers(SHIFT_MASK, 0, 0, 0);
            }
            self.key(code, true);
            self.key(code, false);
            if shifted {
                self.key(KEY_LEFTSHIFT, false);
                self.keyboard.modifiers(0, 0, 0, 0);
            }
        }
        self.sync()
    }

    /// Presses and releases `character`'s key while holding Control.
    pub fn press_with_control(&mut self, character: char) -> Result<(), String> {
        let (code, false) = character_key(character)
            .ok_or_else(|| format!("the harness can't type {character:?}"))?
        else {
            return Err(format!("{character:?} needs Shift as well as Control"));
        };
        self.key(KEY_LEFTCTRL, true);
        self.keyboard.modifiers(CONTROL_MASK, 0, 0, 0);
        self.key(code, true);
        self.key(code, false);
        self.key(KEY_LEFTCTRL, false);
        self.keyboard.modifiers(0, 0, 0, 0);
        self.sync()
    }

    /// Presses and releases `key`.
    pub fn press(&mut self, key: Key) -> Result<(), String> {
        self.key(key.code(), true);
        self.key(key.code(), false);
        self.sync()
    }

    /// Moves the pointer to (`x`, `y`) in the layout, whose size is `extent`.
    pub fn move_to(&mut self, x: f64, y: f64, extent: (f64, f64)) -> Result<(), String> {
        let scaled = |value: f64| {
            let value = (value * POINTER_PRECISION).round();
            if (0.0..=f64::from(u32::MAX)).contains(&value) {
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "checked to be a whole number within u32 just above"
                )]
                let whole = value as u32;
                Ok(whole)
            } else {
                Err(format!("pointer coordinate {value} is off the screen"))
            }
        };
        let time = self.time();
        self.pointer.motion_absolute(
            time,
            scaled(x)?,
            scaled(y)?,
            scaled(extent.0)?,
            scaled(extent.1)?,
        );
        self.pointer.frame();
        self.sync()
    }

    /// Presses and releases the left button where the pointer is.
    pub fn click(&mut self) -> Result<(), String> {
        for state in [ButtonState::Pressed, ButtonState::Released] {
            let time = self.time();
            self.pointer.button(time, BTN_LEFT, state);
            self.pointer.frame();
        }
        self.sync()
    }

    /// Blocks until the compositor goes away or has no output left. A nested
    /// sway loses its only output when its window is closed, and carries on
    /// without one.
    pub fn wait_until_closed(&mut self) {
        while self.queue.blocking_dispatch(&mut Globals).is_ok() {
            let has_output = self
                .globals
                .contents()
                .with_list(|globals| globals.iter().any(|global| global.interface == "wl_output"));
            if !has_output {
                break;
            }
        }
    }

    fn key(&self, code: u32, pressed: bool) {
        self.keyboard.key(self.time(), code, u32::from(pressed));
    }

    /// Waits until the compositor has handled every request sent so far.
    fn sync(&mut self) -> Result<(), String> {
        self.connection.flush().map_err(|error| error.to_string())?;
        self.queue
            .roundtrip(&mut Globals)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    /// Event time in milliseconds, as input devices report it.
    fn time(&self) -> u32 {
        u32::try_from(self.started.elapsed().as_millis()).unwrap_or(u32::MAX)
    }
}

/// The key that types `character` on a US keyboard, and whether it needs Shift.
fn character_key(character: char) -> Option<(u32, bool)> {
    const UNSHIFTED: &str = "1234567890-=qwertyuiop[]asdfghjkl;'`\\zxcvbnm,./";
    const SHIFTED: &str = "!@#$%^&*()_+QWERTYUIOP{}ASDFGHJKL:\"~|ZXCVBNM<>?";
    // The key codes of the characters above, in the same order.
    const CODES: [u32; 47] = [
        2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, // 1 to =
        16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, // q to ]
        30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, // a to `
        43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, // \ to /
    ];
    match character {
        ' ' => return Some((57, false)),
        '\n' => return Some((Key::Return.code(), false)),
        _ => {}
    }
    let code = |position: usize| CODES.get(position).copied();
    if let Some(position) = UNSHIFTED.chars().position(|known| known == character) {
        return code(position).map(|code| (code, false));
    }
    SHIFTED
        .chars()
        .position(|known| known == character)
        .and_then(code)
        .map(|code| (code, true))
}
