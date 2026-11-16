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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

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
    /// Connects to the compositor at `socket`, giving up if it hasn't
    /// answered within `deadline`. Shared-memory buffers are files in
    /// `directory`. Also returns a handle that ends the recording from
    /// another thread: [`Stopper::stop`] makes a waiting
    /// [`Recorder::next_frame`] return an error.
    pub fn connect(
        socket: &Path,
        directory: &Path,
        deadline: Duration,
    ) -> Result<(Self, Stopper), String> {
        let (recorder, stopper, watch) = Self::connect_watched(socket, directory, deadline)?;
        watch.judge(Ok((recorder, stopper)))
    }

    /// As [`Recorder::connect`], with the watch on `deadline` left running,
    /// for the caller to end once it has what it waited for.
    fn connect_watched(
        socket: &Path,
        directory: &Path,
        deadline: Duration,
    ) -> Result<(Self, Stopper, Watch), String> {
        let stream = UnixStream::connect(socket)
            .map_err(|error| format!("connect to {}: {error}", socket.display()))?;
        let clone = || stream.try_clone().map_err(|error| error.to_string());
        let stopper = Stopper(clone()?);
        let watch = Watch::start(clone()?, deadline);
        let (recorder, stopper) = watch.judge(Self::start(stream, stopper, directory))?;
        Ok((recorder, stopper, watch))
    }

    fn start(
        stream: UnixStream,
        stopper: Stopper,
        directory: &Path,
    ) -> Result<(Self, Stopper), String> {
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
        self.next_raw_frame()?.write_ppm(path)
    }

    /// As [`Recorder::next_frame`], keeping the frame in memory as the
    /// compositor sent it, which costs a copy and nothing more.
    pub fn next_raw_frame(&mut self) -> Result<RawFrame, String> {
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
                    return self.read_frame(slot);
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

    fn read_frame(&self, slot: Slot) -> Result<RawFrame, String> {
        let Some(buffer) = &self.buffers[slot] else {
            return Err("the compositor finished a frame it had no buffer for".to_owned());
        };
        let (format, width, height, stride) = buffer.shape;
        let usize = |value: u32| usize::try_from(value).map_err(|error| error.to_string());
        let mut pixels = vec![0; usize(stride)? * usize(height)?];
        buffer
            .file
            .read_exact_at(&mut pixels, 0)
            .map_err(|error| error.to_string())?;
        Ok(RawFrame {
            pixels,
            shape: (format, width, height, stride),
            inverted: self.state.progress[slot].inverted,
        })
    }
}

/// One frame of the screen as the compositor sent it: its pixels in the
/// compositor's format, and that format, width, height and stride.
pub(crate) struct RawFrame {
    pixels: Vec<u8>,
    shape: (wl_shm::Format, u32, u32, u32),
    inverted: bool,
}

impl RawFrame {
    /// Writes the frame to `path` as a binary PPM. Returns its width and
    /// height.
    pub fn write_ppm(&self, path: &Path) -> Result<(u32, u32), String> {
        let (format, width, height, stride) = self.shape;
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
        let pixels = &self.pixels;
        // Converted into place, pixel by pixel through zipped iterators,
        // which stays fast at the harness's opt-level 1: pushing each pixel
        // onto a Vec took 160 ms a frame there, and the recorder hands the
        // compositor its next buffer only once this frame is written, so it
        // missed a window shown for 200 ms (ticket #55).
        let mut rgb = vec![0; columns * rows * 3];
        for (row, converted) in rgb.chunks_exact_mut(columns * 3).enumerate() {
            let row = if self.inverted { rows - 1 - row } else { row };
            let line = &pixels[row * stride..row * stride + columns * 4];
            for (converted, pixel) in converted
                .as_chunks_mut::<3>()
                .0
                .iter_mut()
                .zip(line.as_chunks::<4>().0)
            {
                *converted = [pixel[red], pixel[green], pixel[blue]];
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

/// One frame of the screen as it is now, from the compositor at `socket`,
/// written to `path` as a binary PPM, with buffers in `directory`.
/// Gives up if the compositor hasn't sent it within `deadline`.
pub(crate) fn capture_once(
    socket: &Path,
    directory: &Path,
    path: &Path,
    deadline: Duration,
) -> Result<(u32, u32), String> {
    let (mut recorder, _stopper, watch) = Recorder::connect_watched(socket, directory, deadline)?;
    watch.judge(recorder.next_frame(path))
}

/// Shuts a connection to the compositor down if `deadline` passes before
/// this is dropped, so a call blocked on it returns an error instead of
/// waiting for ever on a compositor that has hung.
struct Watch {
    deadline: Duration,
    fired: Arc<AtomicBool>,
    cancel: Option<mpsc::Sender<()>>,
    watcher: Option<thread::JoinHandle<()>>,
}

impl Watch {
    fn start(stream: UnixStream, deadline: Duration) -> Self {
        let fired = Arc::new(AtomicBool::new(false));
        let (cancel, cancelled) = mpsc::channel::<()>();
        let flag = Arc::clone(&fired);
        let watcher = thread::spawn(move || {
            if let Err(mpsc::RecvTimeoutError::Timeout) = cancelled.recv_timeout(deadline) {
                flag.store(true, Ordering::SeqCst);
                let _ = stream.shutdown(Shutdown::Both);
            }
        });
        Self {
            deadline,
            fired,
            cancel: Some(cancel),
            watcher: Some(watcher),
        }
    }

    /// `result`, unless the deadline passed: then the connection is gone,
    /// and the error says so.
    fn judge<T>(&self, result: Result<T, String>) -> Result<T, String> {
        if !self.fired.load(Ordering::SeqCst) {
            return result;
        }
        let detail = result
            .err()
            .map_or_else(String::new, |error| format!(" ({error})"));
        Err(format!(
            "the compositor did not answer within {:?}{detail}",
            self.deadline
        ))
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        drop(self.cancel.take());
        if let Some(watcher) = self.watcher.take() {
            let _ = watcher.join();
        }
    }
}

delegate_noop!(State: ZwlrScreencopyManagerV1);
delegate_noop!(State: ignore wl_output::WlOutput);
delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_buffer::WlBuffer);

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixListener;

    use super::*;

    /// A compositor that takes the connection and never answers, as a hung
    /// sway does: the capture gives up at its deadline rather than hold the
    /// test, and the job, until GitHub's limit.
    #[test]
    fn a_capture_from_a_compositor_that_never_answers_fails_at_its_deadline() {
        let directory = tempfile::tempdir().expect("a directory");
        let socket = directory.path().join("wayland-silent");
        let listener = UnixListener::bind(&socket).expect("listen");
        let (release, released) = mpsc::channel::<()>();
        thread::spawn(move || {
            let connection = listener.accept();
            // Kept open, unanswered, until the test ends.
            let _ = released.recv();
            drop(connection);
        });
        let (done, result) = mpsc::channel();
        let buffers = directory.path().join("captures");
        let path = buffers.join("frame.ppm");
        thread::spawn(move || {
            let _ = done.send(capture_once(
                &socket,
                &buffers,
                &path,
                Duration::from_millis(300),
            ));
        });
        let captured = result
            .recv_timeout(Duration::from_secs(5))
            .expect("the capture gave up at its deadline instead of hanging");
        drop(release);
        let error = captured.expect_err("nothing could be captured");
        assert!(error.contains("within 300ms"), "{error}");
    }
}
