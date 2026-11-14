//! A screen of the scenario's own: sway, a wlroots compositor, either headless
//! or as a window on the Desktop (`just demo`), and screenshots of it.
//!
//! sway is the compositor because it has everything the app's surfaces and
//! the tests need: layer-shell, screencopy (which `grim` uses), and virtual
//! keyboards and pointers. Its headless backend needs no GPU, seat or input
//! devices, so it runs in CI.

use std::fs;
use std::io::{BufRead as _, BufReader, BufWriter, Cursor};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, MutexGuard, PoisonError, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use crate::input::SyntheticInput;
use crate::process_group::ProcessGroup;

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
    /// The compositor's socket, for the harness's own connections.
    pub socket: PathBuf,
    /// The compositor's process ID.
    pub pid: u32,
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
    /// If the screenshot can't be decoded.
    #[must_use]
    pub fn region_where(&self, matches: impl Fn([u8; 3]) -> bool) -> Option<Region> {
        self.region_within(
            Region {
                left: 0.0,
                top: 0.0,
                right: f64::INFINITY,
                bottom: f64::INFINITY,
            },
            matches,
        )
    }

    /// As [`Screenshot::region_where`], counting only the pixels inside
    /// `area`, such as one half of the screen.
    ///
    /// # Panics
    ///
    /// If the screenshot can't be decoded.
    #[must_use]
    pub fn region_within(&self, area: Region, matches: impl Fn([u8; 3]) -> bool) -> Option<Region> {
        let pixels = Pixels::read(&self.path);
        let (mut left, mut top, mut right, mut bottom) = (usize::MAX, usize::MAX, 0, 0);
        for (row, line) in pixels.data.chunks_exact(pixels.line_size).enumerate() {
            let y = to_layout(row);
            if y < area.top || y >= area.bottom {
                continue;
            }
            for column in 0..pixels.width {
                let x = to_layout(column);
                if x < area.left || x >= area.right {
                    continue;
                }
                let pixel = &line[column * pixels.channels..column * pixels.channels + 3];
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
        Some(Region {
            left: to_layout(left),
            top: to_layout(top),
            right: to_layout(right),
            bottom: to_layout(bottom),
        })
    }

    /// The colour at the layout point (`x`, `y`).
    ///
    /// # Panics
    ///
    /// If the screenshot can't be decoded, or the point is off the screen.
    #[must_use]
    pub fn colour_at(&self, x: f64, y: f64) -> [u8; 3] {
        let pixels = Pixels::read(&self.path);
        let at = to_pixel(y) * pixels.line_size + to_pixel(x) * pixels.channels;
        [pixels.data[at], pixels.data[at + 1], pixels.data[at + 2]]
    }

    /// The runs, left to right, of pixels along the layout row `y` whose
    /// colour is not within 4 of `background` in every channel, as layout
    /// `(left, right)`. Runs less than `gap` layout units apart are one.
    ///
    /// # Panics
    ///
    /// If the screenshot can't be decoded, or the row is off the screen.
    #[must_use]
    pub fn runs_along_row(&self, y: f64, background: [u8; 3], gap: f64) -> Vec<(f64, f64)> {
        let pixels = Pixels::read(&self.path);
        let row = &pixels.data[to_pixel(y) * pixels.line_size..][..pixels.line_size];
        let mut runs: Vec<(f64, f64)> = Vec::new();
        for column in 0..pixels.width {
            let pixel = &row[column * pixels.channels..column * pixels.channels + 3];
            if pixel
                .iter()
                .zip(background)
                .all(|(channel, wanted)| channel.abs_diff(wanted) <= 4)
            {
                continue;
            }
            let (left, right) = (to_layout(column), to_layout(column + 1));
            match runs.last_mut() {
                Some(run) if left - run.1 < gap => run.1 = right,
                _ => runs.push((left, right)),
            }
        }
        runs
    }

    /// Writes this screenshot to `path` as a PNG, and returns the copy.
    pub(crate) fn save_png(&self, path: &Path) -> Screenshot {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create the screenshot directory");
        }
        if self
            .path
            .extension()
            .is_some_and(|extension| extension == "png")
        {
            fs::copy(&self.path, path).expect("copy the screenshot");
        } else {
            let pixels = Pixels::read(&self.path);
            let file = fs::File::create(path).expect("create the screenshot");
            let mut encoder = png::Encoder::new(BufWriter::new(file), self.width, self.height);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_compression(png::Compression::Fast);
            let mut writer = encoder
                .write_header()
                .expect("write the screenshot's header");
            writer
                .write_image_data(&pixels.data)
                .expect("write the screenshot");
            writer.finish().expect("finish the screenshot");
        }
        Screenshot {
            path: path.to_owned(),
            ..self.clone()
        }
    }
}

/// The layout coordinate of a pixel column or row.
fn to_layout(pixels: usize) -> f64 {
    let pixels = u32::try_from(pixels).expect("a screen coordinate");
    f64::from(pixels) * LAYOUT_SIZE.0 / f64::from(SCREEN_SIZE.0)
}

/// The pixel column or row of a layout coordinate.
fn to_pixel(layout: f64) -> usize {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a layout coordinate on the screen is a small non-negative number"
    )]
    let pixel = (layout * f64::from(SCREEN_SIZE.0) / LAYOUT_SIZE.0) as usize;
    pixel
}

/// A screenshot's decoded pixels: rows of `line_size` bytes, `channels`
/// bytes a pixel, red, green and blue first.
struct Pixels {
    width: usize,
    channels: usize,
    line_size: usize,
    data: Vec<u8>,
}

impl Pixels {
    /// Decodes the PNG or binary PPM (`P6`, as `grim -t ppm` writes) at `path`.
    fn read(path: &Path) -> Self {
        let bytes = fs::read(path).expect("read the screenshot");
        if bytes.starts_with(b"P6") {
            return Self::from_ppm(bytes)
                .unwrap_or_else(|| panic!("{} is not a PPM grim writes", path.display()));
        }
        let mut decoder = png::Decoder::new(Cursor::new(bytes));
        decoder.set_transformations(png::Transformations::normalize_to_color8());
        let mut reader = decoder.read_info().expect("read the screenshot's header");
        let mut data = vec![0; reader.output_buffer_size().expect("the screenshot's size")];
        let frame = reader.next_frame(&mut data).expect("decode the screenshot");
        data.truncate(frame.buffer_size());
        Self {
            width: usize::try_from(frame.width).expect("the screenshot's width"),
            channels: frame.color_type.samples(),
            line_size: frame.line_size,
            data,
        }
    }

    /// `P6`, width, height and a maximum of 255, each followed by one
    /// whitespace byte, then the RGB pixels.
    fn from_ppm(mut bytes: Vec<u8>) -> Option<Self> {
        let mut fields = Vec::new();
        let mut start = 0;
        while fields.len() < 4 {
            let end = start
                + bytes
                    .get(start..)?
                    .iter()
                    .position(u8::is_ascii_whitespace)?;
            fields.push(std::str::from_utf8(&bytes[start..end]).ok()?.to_owned());
            start = end + 1;
        }
        let width: usize = fields[1].parse().ok()?;
        let height: usize = fields[2].parse().ok()?;
        if fields[0] != "P6" || fields[3] != "255" || bytes.len() - start != width * height * 3 {
            return None;
        }
        bytes.drain(..start);
        Some(Self {
            width,
            channels: 3,
            line_size: width * 3,
            data: bytes,
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
        group: &ProcessGroup,
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
        group
            .adopt(&mut command)
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
        let mut child = match command.spawn() {
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
        let Some(wayland_display) = read_ready(&ready, &mut child) else {
            let status = match child.try_wait() {
                Ok(Some(status)) => format!("it exited ({status})"),
                Ok(None) => format!("it is still running after {STARTUP_DEADLINE:?}"),
                Err(error) => format!("its exit status is unknown ({error})"),
            };
            processes.push(("compositor", child));
            let stderr = directory.join("compositor.stderr");
            panic!(
                "sway did not start: {status}; {}. {}\n(all of it: {})",
                shm_free(),
                said(&stderr),
                stderr.display()
            );
        };
        let pid = child.id();
        processes.push(("compositor", child));
        let runtime = environment
            .iter()
            .find(|(name, _)| *name == "XDG_RUNTIME_DIR")
            .map(|(_, value)| PathBuf::from(value))
            .expect("the scenario has an XDG_RUNTIME_DIR");
        let socket = runtime.join(&wayland_display);
        let input = SyntheticInput::connect(&socket, &directory.join("keymap.xkb"))
            .unwrap_or_else(|error| panic!("set up the synthetic keyboard and pointer: {error}"));
        Self {
            wayland_display,
            socket,
            pid,
            input: Mutex::new(input),
        }
    }

    /// The synthetic keyboard and pointer.
    pub fn input(&self) -> MutexGuard<'_, SyntheticInput> {
        self.input.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Captures every output into `path` with `grim`. `environment` names
    /// this screen in `WAYLAND_DISPLAY`.
    pub fn screenshot(
        environment: &[(&'static str, String)],
        group: &ProcessGroup,
        path: &Path,
    ) -> Screenshot {
        Self::grim(environment, group, &[], path)
    }

    /// Captures only `region`, in layout coordinates, into `path`.
    pub fn screenshot_of(
        environment: &[(&'static str, String)],
        group: &ProcessGroup,
        path: &Path,
        region: Region,
    ) -> Screenshot {
        // grim takes whole layout pixels; round outwards.
        let (left, top) = (region.left.floor(), region.top.floor());
        let (width, height) = (region.right.ceil() - left, region.bottom.ceil() - top);
        let geometry = format!("{left},{top} {width}x{height}");
        Self::grim(environment, group, &["-g", &geometry], path)
    }

    fn grim(
        environment: &[(&'static str, String)],
        group: &ProcessGroup,
        arguments: &[&str],
        path: &Path,
    ) -> Screenshot {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create the screenshot directory");
        }
        let captured = group
            .adopt(&mut Command::new("grim"))
            .env_clear()
            .envs(environment.iter().cloned())
            .args(arguments)
            .arg(path)
            .output();
        let captured = match captured {
            Ok(captured) => captured,
            Err(error) => panic!("run grim ({error}); install it: sudo pacman -S --needed grim"),
        };
        assert!(
            captured.status.success(),
            "grim failed ({}); {}. grim said: {:?}",
            captured.status,
            shm_free(),
            String::from_utf8_lossy(&captured.stderr).trim()
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
    // No Xwayland and no swaybg: the app needs neither, and their errors
    // (CI has neither installed) hid sway's own.
    format!(
        "# Written by the Desktop Calls test harness.\n\
         xwayland disable\n\
         swaybg_command -\n\
         {output}\
         exec printf '%s\\n' \"$WAYLAND_DISPLAY\" > '{}'\n",
        ready.display()
    )
}

/// How much of `/dev/shm` is free, where grim and headless sway keep their
/// frames: 33 MB each at [`SCREEN_SIZE`]. Docker's default of 64 MB once
/// killed grim with SIGBUS and kept sway from starting (ticket #41).
pub(crate) fn shm_free() -> String {
    let df = Command::new("df")
        .args(["--block-size=1M", "--output=avail,size", "/dev/shm"])
        .output();
    let sizes = df.as_ref().ok().and_then(|df| {
        let text = String::from_utf8_lossy(&df.stdout);
        let line = text.lines().nth(1)?.to_owned();
        let mut fields = line.split_whitespace();
        Some((fields.next()?.to_owned(), fields.next()?.to_owned()))
    });
    match (sizes, df) {
        (Some((free, size)), _) => format!("/dev/shm has {free} MB free of {size} MB"),
        (None, Ok(df)) => format!(
            "/dev/shm's free space is unknown (df said {:?})",
            String::from_utf8_lossy(&df.stderr).trim()
        ),
        (None, Err(error)) => format!("/dev/shm's free space is unknown (df: {error})"),
    }
}

/// What sway wrote to `stderr`, its log, without the errors about Xwayland
/// and swaybg, which it prints wherever they are missing.
fn said(stderr: &Path) -> String {
    let log = fs::read_to_string(stderr).unwrap_or_default();
    let own: Vec<&str> = log
        .lines()
        .filter(|line| !line.contains("Xwayland") && !line.contains("swaybg"))
        .collect();
    if own.is_empty() {
        "sway wrote nothing to its log".to_owned()
    } else {
        format!("sway said:\n  {}", own.join("\n  "))
    }
}

/// Waits for sway to write its `WAYLAND_DISPLAY` into the FIFO at `ready`,
/// for at most [`STARTUP_DEADLINE`], and not at all once `sway` has exited.
fn read_ready(ready: &Path, sway: &mut Child) -> Option<String> {
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
    let give_up = Instant::now() + STARTUP_DEADLINE;
    while Instant::now() < give_up {
        match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(display) => return display.filter(|display| !display.is_empty()),
            Err(mpsc::RecvTimeoutError::Timeout) if !matches!(sway.try_wait(), Ok(Some(_))) => {}
            Err(_) => break,
        }
    }
    // The reader is still waiting for a writer; release it.
    let _ = fs::OpenOptions::new().write(true).open(ready);
    None
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
