//! A screen of the scenario's own: sway, a wlroots compositor, either headless
//! or as a window on the Desktop (`just demo`), and screenshots of it.
//!
//! sway is the compositor because it has everything the app's surfaces and
//! the tests need: layer-shell, screencopy (which `grim` uses), and virtual
//! keyboards and pointers. Its headless backend needs no GPU, seat or input
//! devices, so it runs in CI.

use std::fs;
use std::io::{BufRead as _, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, MutexGuard, PoisonError, mpsc};
use std::thread;
use std::time::Duration;

use crate::input::SyntheticInput;

/// The size of the headless output in pixels: the user's monitor.
pub const SCREEN_SIZE: (u32, u32) = (3840, 2160);

/// The user's monitor's scale, so surfaces are laid out as they will be on
/// the Desktop.
const SCREEN_SCALE: &str = "1.6";

/// The headless output's size in layout coordinates, which is what clients
/// and the pointer work in: [`SCREEN_SIZE`] divided by the scale.
pub const LAYOUT_SIZE: (f64, f64) = (2400.0, 1350.0);

/// How long sway may take to start.
const STARTUP_DEADLINE: Duration = Duration::from_secs(10);

/// Where the screen is.
#[derive(Clone, Debug)]
pub(crate) enum ScreenKind {
    /// Off-screen, at [`SCREEN_SIZE`].
    Headless,
    /// A window on the Desktop's compositor, whose socket this is.
    Nested { parent_socket: PathBuf },
}

/// A running compositor.
pub(crate) struct Screen {
    /// `WAYLAND_DISPLAY` for the scenario's clients, relative to its
    /// `XDG_RUNTIME_DIR`.
    pub wayland_display: String,
    input: Mutex<SyntheticInput>,
}

/// One captured screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Screenshot {
    /// The PNG file.
    pub path: PathBuf,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

/// A rectangle on screen, in layout coordinates: what clients and the
/// pointer work in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

impl Region {
    /// The middle of the region, where a click lands squarely.
    #[must_use]
    pub fn centre(&self) -> (f64, f64) {
        (
            f64::midpoint(self.left, self.right),
            f64::midpoint(self.top, self.bottom),
        )
    }

    /// Whether the two regions overlap.
    #[must_use]
    pub fn intersects(&self, other: &Self) -> bool {
        self.left < other.right
            && other.left < self.right
            && self.top < other.bottom
            && other.top < self.bottom
    }
}

impl Screenshot {
    /// The smallest region holding every pixel whose RGB colour satisfies
    /// `matches`, or `None` if no pixel does.
    ///
    /// # Panics
    ///
    /// If the PNG can't be decoded.
    #[must_use]
    pub fn region_where(&self, matches: impl Fn([u8; 3]) -> bool) -> Option<Region> {
        let file = fs::File::open(&self.path).expect("open the screenshot");
        let mut decoder = png::Decoder::new(BufReader::new(file));
        decoder.set_transformations(png::Transformations::normalize_to_color8());
        let mut reader = decoder.read_info().expect("read the screenshot's header");
        let mut pixels = vec![0; reader.output_buffer_size().expect("the screenshot's size")];
        let frame = reader
            .next_frame(&mut pixels)
            .expect("decode the screenshot");
        let channels = frame.color_type.samples();
        let width = usize::try_from(frame.width).expect("the screenshot's width");
        let (mut left, mut top, mut right, mut bottom) = (usize::MAX, usize::MAX, 0, 0);
        for (row, line) in pixels[..frame.buffer_size()]
            .chunks_exact(frame.line_size)
            .enumerate()
        {
            for column in 0..width {
                let pixel = &line[column * channels..column * channels + 3];
                if matches([pixel[0], pixel[1], pixel[2]]) {
                    left = left.min(column);
                    right = right.max(column + 1);
                    top = top.min(row);
                    bottom = bottom.max(row + 1);
                }
            }
        }
        if left == usize::MAX {
            return None;
        }
        let to_layout = |pixels: usize| {
            let pixels = u32::try_from(pixels).expect("a screen coordinate");
            f64::from(pixels) * LAYOUT_SIZE.0 / f64::from(SCREEN_SIZE.0)
        };
        Some(Region {
            left: to_layout(left),
            top: to_layout(top),
            right: to_layout(right),
            bottom: to_layout(bottom),
        })
    }
}

impl Screen {
    /// Starts sway in `directory` with `environment`, and returns once its
    /// clients can connect. Pushes the process onto `processes`.
    pub fn start(
        kind: &ScreenKind,
        directory: &Path,
        environment: &[(&'static str, String)],
        processes: &mut Vec<(&'static str, Child)>,
    ) -> Self {
        let ready = directory.join("compositor-ready");
        let made = Command::new("mkfifo")
            .arg(&ready)
            .status()
            .expect("run mkfifo");
        assert!(made.success(), "mkfifo {} failed", ready.display());

        let config = directory.join("compositor.conf");
        fs::write(&config, config_text(kind, &ready)).expect("write the compositor's config");

        let mut command = Command::new("sway");
        command
            .env_clear()
            .envs(environment.iter().cloned())
            .arg("--config")
            .arg(&config)
            .stdout(Stdio::null())
            .stderr(fs::File::create(directory.join("compositor.stderr")).expect("create a log"));
        match kind {
            ScreenKind::Headless => {
                command
                    .env("WLR_BACKENDS", "headless")
                    .env("WLR_RENDERER", "pixman")
                    .env("WLR_LIBINPUT_NO_DEVICES", "1");
            }
            ScreenKind::Nested { parent_socket } => {
                command
                    .env("WLR_BACKENDS", "wayland")
                    .env("WAYLAND_DISPLAY", parent_socket);
            }
        }
        let child = match command.spawn() {
            Ok(child) => child,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => panic!(
                "start sway ({error}); the screen harness needs the sway and grim packages: \
                 sudo pacman -S --needed sway grim"
            ),
            // Arch's sway carries the file capability cap_sys_nice, and a
            // container whose capability bounding set lacks it can't run it.
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => panic!(
                "start sway ({error}); if this is a container, sway's file capability is \
                 outside its bounding set: setcap -r /usr/bin/sway"
            ),
            Err(error) => panic!("start sway ({error})"),
        };
        processes.push(("compositor", child));

        let wayland_display = read_ready(&ready).unwrap_or_else(|| {
            panic!(
                "sway did not start within {STARTUP_DEADLINE:?}; see {}",
                directory.join("compositor.stderr").display()
            )
        });
        let runtime = environment
            .iter()
            .find(|(name, _)| *name == "XDG_RUNTIME_DIR")
            .map(|(_, value)| PathBuf::from(value))
            .expect("the scenario has an XDG_RUNTIME_DIR");
        let input = SyntheticInput::connect(
            &runtime.join(&wayland_display),
            &directory.join("keymap.xkb"),
        )
        .unwrap_or_else(|error| panic!("set up the synthetic keyboard and pointer: {error}"));
        Self {
            wayland_display,
            input: Mutex::new(input),
        }
    }

    /// The synthetic keyboard and pointer.
    pub fn input(&self) -> MutexGuard<'_, SyntheticInput> {
        self.input.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Captures every output into `path` with `grim`. `environment` names
    /// this screen in `WAYLAND_DISPLAY`.
    pub fn screenshot(environment: &[(&'static str, String)], path: &Path) -> Screenshot {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create the screenshot directory");
        }
        let captured = Command::new("grim")
            .env_clear()
            .envs(environment.iter().cloned())
            .arg(path)
            .output();
        let captured = match captured {
            Ok(captured) => captured,
            Err(error) => panic!("run grim ({error}); install it: sudo pacman -S --needed grim"),
        };
        assert!(
            captured.status.success(),
            "grim failed ({}): {}",
            captured.status,
            String::from_utf8_lossy(&captured.stderr)
        );
        let png = fs::read(path).expect("read the screenshot");
        let (width, height) =
            png_size(&png).unwrap_or_else(|| panic!("grim wrote no PNG to {}", path.display()));
        Screenshot {
            path: path.to_owned(),
            width,
            height,
        }
    }
}

/// sway's configuration: the output, nothing else on screen, and a line that
/// says sway is ready by writing its `WAYLAND_DISPLAY` into `ready`.
fn config_text(kind: &ScreenKind, ready: &Path) -> String {
    let output = match kind {
        ScreenKind::Headless => format!(
            "output HEADLESS-1 resolution {}x{} position 0 0 scale {SCREEN_SCALE}\n",
            SCREEN_SIZE.0, SCREEN_SIZE.1
        ),
        ScreenKind::Nested { .. } => String::new(),
    };
    format!(
        "# Written by the Desktop Calls test harness.\n\
         {output}\
         exec printf '%s\\n' \"$WAYLAND_DISPLAY\" > '{}'\n",
        ready.display()
    )
}

/// Waits for sway to write its `WAYLAND_DISPLAY` into the FIFO at `ready`.
fn read_ready(ready: &Path) -> Option<String> {
    let (sender, receiver) = mpsc::channel();
    let fifo = ready.to_owned();
    thread::spawn(move || {
        // Opening a FIFO for reading blocks until sway opens it for writing.
        let line = fs::File::open(&fifo).ok().and_then(|file| {
            let mut line = String::new();
            BufReader::new(file).read_line(&mut line).ok()?;
            Some(line.trim().to_owned())
        });
        let _ = sender.send(line);
    });
    if let Ok(display) = receiver.recv_timeout(STARTUP_DEADLINE) {
        display.filter(|display| !display.is_empty())
    } else {
        // The reader is still waiting for a writer; release it.
        let _ = fs::OpenOptions::new().write(true).open(ready);
        None
    }
}

/// The width and height in a PNG's header.
fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    let header = png.get(..24)?;
    if !header.starts_with(b"\x89PNG\r\n\x1a\n") || &header[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(header[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(header[20..24].try_into().ok()?);
    Some((width, height))
}
