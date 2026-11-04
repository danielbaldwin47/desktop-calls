//! zbus on the GLib main context: the app's one thread and one event loop.
//!
//! zbus runs each connection on whatever implements its runtime traits. This
//! one hands every task, timer and socket wait to a GLib main context, so D-Bus
//! traffic, state changes and (later) GTK all happen on the same thread, with
//! no second async runtime and no helper threads.

use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::os::fd::{AsRawFd as _, OwnedFd};
use std::path::Path;
use std::pin::Pin;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use futures_channel::oneshot;
use glib::{ControlFlow, IOCondition, MainContext, Priority};
use socket2::{Domain, SockAddr, Socket, Type};
use zbus::runtime::traits::{PollIo, Runtime, TaskHandle};
use zbus::runtime::{Interest, IoSource};

/// How long to wait before retrying a connect that a full listen backlog
/// turned away.
const CONNECT_RETRY: Duration = Duration::from_millis(10);

/// A zbus runtime that runs everything on one GLib main context.
#[derive(Clone, Debug)]
pub struct GlibRuntime {
    context: MainContext,
}

impl GlibRuntime {
    pub fn new(context: MainContext) -> Self {
        Self { context }
    }
}

impl Runtime for GlibRuntime {
    type RegisteredIoSource = Registration;
    type Sleep = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;
    type Task<T>
        = Task<T>
    where
        T: Send + 'static;

    fn register_io_source(&self, source: IoSource) -> io::Result<Registration> {
        Ok(Registration {
            source,
            context: self.context.clone(),
            waits: Mutex::default(),
        })
    }

    fn sleep(&self, duration: Duration) -> Self::Sleep {
        glib::timeout_future(duration)
    }

    fn spawn<T>(&self, _name: &str, future: impl Future<Output = T> + Send + 'static) -> Task<T>
    where
        T: Send + 'static,
    {
        let (sender, receiver) = oneshot::channel();
        let handle = self.context.spawn(async move {
            // The handle was dropped: nobody wants the output.
            let _ = sender.send(future.await);
        });
        Task {
            receiver,
            source: Some(handle.source().clone()),
        }
    }

    fn connect_tcp(
        &self,
        _address: SocketAddr,
    ) -> impl Future<Output = io::Result<IoSource>> + Send {
        std::future::ready(Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Desktop Calls reaches D-Bus over unix-domain sockets only",
        )))
    }

    fn connect_unix(&self, path: &Path) -> impl Future<Output = io::Result<IoSource>> + Send {
        let address = SockAddr::unix(path);
        async move {
            let address = address?;
            let socket = Socket::new(Domain::UNIX, Type::STREAM, None)?;
            socket.set_nonblocking(true)?;
            loop {
                match socket.connect(&address) {
                    Ok(()) => return Ok(IoSource::from(OwnedFd::from(socket))),
                    // A unix-domain listener with a full backlog turns a
                    // non-blocking connect away; try again shortly.
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        glib::timeout_future(CONNECT_RETRY).await;
                    }
                    Err(error) => return Err(error),
                }
            }
        }
    }

    fn spawn_process(
        &self,
        _command: Command,
        _stdin: Stdio,
        _stdout: Stdio,
        _stderr: Stdio,
    ) -> io::Result<Pin<Box<dyn Future<Output = io::Result<ExitStatus>> + Send + 'static>>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Desktop Calls doesn't reach D-Bus through a helper process",
        ))
    }
}

/// A task on the main context. Dropping it cancels the task.
pub struct Task<T> {
    receiver: oneshot::Receiver<T>,
    /// The task's GLib source, destroyed on drop unless the task is detached.
    source: Option<glib::Source>,
}

impl<T> Future for Task<T> {
    type Output = io::Result<T>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.receiver)
            .poll(cx)
            .map(|output| output.map_err(|_| io::Error::other("the task was cancelled")))
    }
}

impl<T: Send + 'static> TaskHandle<T> for Task<T> {
    fn detach(mut self) {
        self.source = None;
    }
}

impl<T> Drop for Task<T> {
    fn drop(&mut self) {
        if let Some(source) = self.source.take() {
            source.destroy();
        }
    }
}

/// A socket zbus does I/O on, watched by the main context while an operation
/// waits for it.
pub struct Registration {
    source: IoSource,
    context: MainContext,
    /// One wait per kind of readiness: readable, writable, either.
    waits: Mutex<[Option<Wait>; 3]>,
}

/// A one-shot GLib watch on the socket that wakes the waiting task.
struct Wait {
    watch: glib::Source,
    waker: Arc<Mutex<Option<Waker>>>,
}

impl PollIo for Registration {
    fn poll_io<T>(
        &self,
        cx: &mut Context<'_>,
        interest: Interest,
        mut operation: impl FnMut() -> io::Result<T>,
    ) -> Poll<io::Result<T>> {
        match operation() {
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                self.wake_when_ready(interest, cx.waker());
                Poll::Pending
            }
            done => Poll::Ready(done),
        }
    }
}

impl Registration {
    /// Wakes `waker` once the socket is ready for `interest`. GLib polls level-
    /// triggered, so readiness that arrived since the failed attempt still
    /// fires the watch.
    fn wake_when_ready(&self, interest: Interest, waker: &Waker) {
        let (slot, condition) = match interest {
            Interest::Readable => (0, IOCondition::IN),
            Interest::Writable => (1, IOCondition::OUT),
            _ => (2, IOCondition::IN | IOCondition::OUT),
        };
        let mut waits = self.waits.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(wait) = &waits[slot]
            && !wait.watch.is_destroyed()
        {
            *wait.waker.lock().unwrap_or_else(PoisonError::into_inner) = Some(waker.clone());
            return;
        }
        let shared_waker = Arc::new(Mutex::new(Some(waker.clone())));
        let to_wake = shared_waker.clone();
        let watch = glib_unix::fd_source_new(
            self.source.as_raw_fd(),
            condition | IOCondition::HUP | IOCondition::ERR,
            None,
            Priority::DEFAULT,
            move |_, _| {
                if let Some(waker) = to_wake
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take()
                {
                    waker.wake();
                }
                ControlFlow::Break
            },
        );
        watch.attach(Some(&self.context));
        waits[slot] = Some(Wait {
            watch,
            waker: shared_waker,
        });
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        // Stop watching before the descriptor can be closed.
        let waits = self.waits.get_mut().unwrap_or_else(PoisonError::into_inner);
        for wait in waits.iter_mut().filter_map(Option::take) {
            wait.watch.destroy();
        }
    }
}
