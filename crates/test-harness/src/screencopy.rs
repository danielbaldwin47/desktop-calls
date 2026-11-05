//! A recording of the scenario's screen: its content now, then each time it
//! changes, through wlr-screencopy on a Wayland connection of the harness's
//! own.
//!
//! The first capture is a plain `copy`, of the screen as it is. Every later
//! one is a `copy_with_damage`, which the compositor completes only once
//! something on screen has changed, so a still screen costs nothing and no
//! drawn state is skipped for want of a capture. Damage that arrives between
//! two requests is kept by the compositor and completes the next request at
//! once. Two buffers take turns: the next request is sent before the last
//! frame is read out, so the screen is always being watched.

use std::fs;
use std::io::{BufWriter, Write as _};
use std::net::Shutdown;
use std::os::fd::AsFd as _;
use std::os::unix::fs::FileExt as _;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_buffer, wl_output, wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, WEnum, delegate_noop};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1::{self, Flags, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
};

/// One of the two buffers' slots.
type Slot = usize;

/// A frame's progress, as the compositor reports it.
#[derive(Default)]
struct Progress {
    /// The buffer it wants: format, width, height and stride.
    buffer: Option<(wl_shm::Format, u32, u32, u32)>,
    inverted: bool,
    /// `Some(true)` once ready, `Some(false)` if it failed.
    done: Option<bool>,
}

#[derive(Default)]
struct State {
    progress: [Progress; 2],
}

/// A shared-memory buffer the compositor copies the screen into.
struct ShmBuffer {
    file: fs::File,
    buffer: wl_buffer::WlBuffer,
    shape: (wl_shm::Format, u32, u32, u32),
}

/// A frame requested and not yet read.
struct Request {
    frame: ZwlrScreencopyFrameV1,
    with_damage: bool,
    copying: bool,
}

/// A recording of the screen. See the module documentation.
pub(crate) struct Recorder {
    connection: Connection,
    queue: EventQueue<State>,
    state: State,
    manager: ZwlrScreencopyManagerV1,
    output: wl_output::WlOutput,
    shm: wl_shm::WlShm,
    directory: PathBuf,
    buffers: [Option<ShmBuffer>; 2],
    requests: [Option<Request>; 2],
    current: Slot,
}

impl Recorder {
    /// Connects to the compositor at `socket`. Shared-memory buffers are
    /// files in `directory`. Also returns a handle that ends the recording
    /// from another thread: [`Stopper::stop`] makes a waiting
    /// [`Recorder::next_frame`] return an error.
    pub fn connect(socket: &Path, directory: &Path) -> Result<(Self, Stopper), String> {
        let stream = UnixStream::connect(socket)
            .map_err(|error| format!("connect to {}: {error}", socket.display()))?;
        let stopper = Stopper(stream.try_clone().map_err(|error| error.to_string())?);
        let connection = Connection::from_socket(stream).map_err(|error| error.to_string())?;
        let (globals, queue) =
            registry_queue_init::<State>(&connection).map_err(|error| error.to_string())?;
        let handle = queue.handle();
        let bind_error =
            |name: &'static str| move |error| format!("the compositor offers no {name}: {error}");
        // copy_with_damage arrived in version 2.
        let manager: ZwlrScreencopyManagerV1 = globals
            .bind(&handle, 2..=2, ())
            .map_err(bind_error("zwlr_screencopy_manager_v1 version 2"))?;
        let output: wl_output::WlOutput = globals
            .bind(&handle, 1..=1, ())
            .map_err(bind_error("wl_output"))?;
        let shm: wl_shm::WlShm = globals
            .bind(&handle, 1..=1, ())
            .map_err(bind_error("wl_shm"))?;
        fs::create_dir_all(directory).map_err(|error| error.to_string())?;
        let recorder = Self {
            connection,
            queue,
            state: State::default(),
            manager,
            output,
            shm,
            directory: directory.to_owned(),
            buffers: [None, None],
            requests: [None, None],
            current: 0,
        };
        Ok((recorder, stopper))
    }

    /// Waits for the screen's next content (the first call: its content
    /// now), and writes it to `path` as a binary PPM. Returns its width and
    /// height.
    pub fn next_frame(&mut self, path: &Path) -> Result<(u32, u32), String> {
        if self.requests[self.current].is_none() {
            self.request(self.current, false);
        }
        loop {
            // Events for this frame may have come in with the last one's.
            for slot in [0, 1] {
                self.copy_when_asked(slot)?;
            }
            match self.state.progress[self.current].done {
                None => {
                    self.queue
                        .blocking_dispatch(&mut self.state)
                        .map_err(|error| format!("the screen recording ended: {error}"))?;
                }
                Some(false) => return Err("the compositor failed to copy the screen".to_owned()),
                Some(true) => {
                    let slot = self.current;
                    self.current = 1 - slot;
                    if self.requests[self.current].is_none() {
                        self.request(self.current, true);
                    }
                    self.connection
                        .flush()
                        .map_err(|error| format!("the screen recording ended: {error}"))?;
                    if let Some(request) = self.requests[slot].take() {
                        request.frame.destroy();
                    }
                    return self.write_ppm(slot, path);
                }
            }
        }
    }

    fn request(&mut self, slot: Slot, with_damage: bool) {
        self.state.progress[slot] = Progress::default();
        let frame = self
            .manager
            .capture_output(0, &self.output, &self.queue.handle(), slot);
        self.requests[slot] = Some(Request {
            frame,
            with_damage,
            copying: false,
        });
    }

    /// Hands the compositor a buffer once it has said which it wants.
    fn copy_when_asked(&mut self, slot: Slot) -> Result<(), String> {
        let Some(shape) = self.state.progress[slot].buffer else {
            return Ok(());
        };
        let Some(request) = self.requests[slot]
            .as_ref()
            .filter(|request| !request.copying)
        else {
            return Ok(());
        };
        if self.buffers[slot]
            .as_ref()
            .is_none_or(|buffer| buffer.shape != shape)
        {
            self.buffers[slot] = Some(self.create_buffer(slot, shape)?);
        }
        if let Some(buffer) = &self.buffers[slot] {
            if request.with_damage {
                request.frame.copy_with_damage(&buffer.buffer);
            } else {
                request.frame.copy(&buffer.buffer);
            }
        }
        if let Some(request) = self.requests[slot].as_mut() {
            request.copying = true;
        }
        Ok(())
    }

    fn create_buffer(
        &self,
        slot: Slot,
        shape: (wl_shm::Format, u32, u32, u32),
    ) -> Result<ShmBuffer, String> {
        let (format, width, height, stride) = shape;
        let size = u64::from(stride) * u64::from(height);
        let path = self.directory.join(format!("screencopy-buffer-{slot}"));
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .map_err(|error| format!("create {}: {error}", path.display()))?;
        file.set_len(size).map_err(|error| error.to_string())?;
        let int = |value: u64| i32::try_from(value).map_err(|error| error.to_string());
        let handle = self.queue.handle();
        let pool = self.shm.create_pool(file.as_fd(), int(size)?, &handle, ());
        let buffer = pool.create_buffer(
            0,
            int(width.into())?,
            int(height.into())?,
            int(stride.into())?,
            format,
            &handle,
            (),
        );
        pool.destroy();
        Ok(ShmBuffer {
            file,
            buffer,
            shape,
        })
    }

    fn write_ppm(&self, slot: Slot, path: &Path) -> Result<(u32, u32), String> {
        let Some(buffer) = &self.buffers[slot] else {
            return Err("the compositor finished a frame it had no buffer for".to_owned());
        };
        let (format, width, height, stride) = buffer.shape;
        // Byte offsets of red, green and blue in one little-endian pixel.
        let (red, green, blue) = match format {
            wl_shm::Format::Xrgb8888 | wl_shm::Format::Argb8888 => (2, 1, 0),
            wl_shm::Format::Xbgr8888 | wl_shm::Format::Abgr8888 => (0, 1, 2),
            other => {
                return Err(format!(
                    "the screen came in {other:?}, which the harness can't read"
                ));
            }
        };
        let usize = |value: u32| usize::try_from(value).map_err(|error| error.to_string());
        let (columns, rows, stride) = (usize(width)?, usize(height)?, usize(stride)?);
        let mut pixels = vec![0; stride * rows];
        buffer
            .file
            .read_exact_at(&mut pixels, 0)
            .map_err(|error| error.to_string())?;
        let mut rgb = Vec::with_capacity(columns * rows * 3);
        for row in 0..rows {
            let row = if self.state.progress[slot].inverted {
                rows - 1 - row
            } else {
                row
            };
            for pixel in pixels[row * stride..row * stride + columns * 4]
                .as_chunks::<4>()
                .0
            {
                rgb.extend_from_slice(&[pixel[red], pixel[green], pixel[blue]]);
            }
        }
        let file = fs::File::create(path)
            .map_err(|error| format!("create {}: {error}", path.display()))?;
        let mut file = BufWriter::new(file);
        write!(file, "P6\n{width} {height}\n255\n")
            .and_then(|()| file.write_all(&rgb))
            .and_then(|()| file.flush())
            .map_err(|error| format!("write {}: {error}", path.display()))?;
        Ok((width, height))
    }
}

/// Ends a [`Recorder`]'s recording from another thread.
pub(crate) struct Stopper(UnixStream);

impl Stopper {
    pub fn stop(&self) {
        let _ = self.0.shutdown(Shutdown::Both);
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
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

impl Dispatch<ZwlrScreencopyFrameV1, Slot> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        slot: &Slot,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let progress = &mut state.progress[*slot];
        match event {
            zwlr_screencopy_frame_v1::Event::Buffer {
                format: WEnum::Value(format),
                width,
                height,
                stride,
            } => progress.buffer = Some((format, width, height, stride)),
            zwlr_screencopy_frame_v1::Event::Flags {
                flags: WEnum::Value(flags),
            } => progress.inverted = flags.contains(Flags::YInvert),
            zwlr_screencopy_frame_v1::Event::Ready { .. } => progress.done = Some(true),
            zwlr_screencopy_frame_v1::Event::Failed => progress.done = Some(false),
            _ => {}
        }
    }
}

delegate_noop!(State: ZwlrScreencopyManagerV1);
delegate_noop!(State: ignore wl_output::WlOutput);
delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_buffer::WlBuffer);
