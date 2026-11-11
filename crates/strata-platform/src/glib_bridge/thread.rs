//! The bridge thread, its owner and its handle.

use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;
use std::{fmt, io, thread};

use rune::Waker;

use super::channel::{self, Emitter, Inbox};
use super::registry::{self, Key};
use super::request::{Asker, Requests};
use super::signal::{Attachment, Keep, Signal};
use super::task::Task;

/// The bridge thread's name, as `/proc/<pid>/task/*/comm` and debuggers show it.
pub const THREAD_NAME: &str = "glib-bridge";

/// How long dropping a [`GlibThread`] waits for its thread to exit.
const DROP_TIMEOUT: Duration = Duration::from_secs(1);

/// At most this many context iterations run after the loop stops, so a source that always has
/// more to do can't hold the thread.
const DRAIN_LIMIT: usize = 1000;

/// Owns the bridge thread. Keep it in app state, so it lives as long as the app; dropping it
/// shuts the thread down (see the [module docs](super#shutdown)).
pub struct GlibThread {
    glib: Glib,
    exited: mpsc::Receiver<()>,
    thread: Option<thread::JoinHandle<()>>,
}

/// Sends work to the bridge thread from any thread. Cheap to clone. Once the bridge has stopped,
/// work sent through it stops at once.
#[derive(Clone)]
pub struct Glib {
    inner: Arc<Inner>,
}

struct Inner {
    /// The context and its loop, until the bridge thread takes them as it exits. Only that
    /// thread drops them, so the context's last reference goes there: finalizing the context
    /// finalizes its sources, and a local future's source panics (in a C callback, which aborts
    /// the process) when it is finalized on another thread.
    main: Mutex<Option<(glib::MainContext, glib::MainLoop)>>,
    waker: Waker,
    stopped: AtomicBool,
    next_id: AtomicU64,
}

/// Why [`GlibThread::shutdown`] returned before the thread exited.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShutdownError {
    /// The thread was still busy when the timeout ran out (a blocking call on the bridge). It is
    /// left running and stops when that call returns.
    TimedOut,
    /// Called on the bridge thread itself, which can't wait for itself. The loop stops once the
    /// current callback returns.
    OnGlibThread,
    /// The thread panicked outside a task.
    Panicked,
}

impl fmt::Display for ShutdownError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ShutdownError::TimedOut => "the GLib bridge thread did not exit in time",
            ShutdownError::OnGlibThread => "the GLib bridge can't wait for itself to exit",
            ShutdownError::Panicked => "the GLib bridge thread panicked",
        })
    }
}

impl std::error::Error for ShutdownError {}

impl GlibThread {
    /// Starts the bridge thread, and returns once its main loop runs. `waker` is called whenever
    /// an answer lands: pass the setup `Waker`, or `Arc::new(move || handle.wake())` with an
    /// `AppHandle`.
    pub fn start(waker: Waker) -> io::Result<GlibThread> {
        let context = glib::MainContext::new();
        let main_loop = glib::MainLoop::new(Some(&context), false);
        let inner = Arc::new(Inner {
            main: Mutex::new(Some((context.clone(), main_loop.clone()))),
            waker,
            stopped: AtomicBool::new(false),
            next_id: AtomicU64::new(0),
        });
        let (ready_tx, ready) = mpsc::channel();
        let (exit_tx, exited) = mpsc::channel();
        let thread = thread::Builder::new().name(THREAD_NAME.into()).spawn({
            let inner = inner.clone();
            move || run(inner, context, main_loop, ready_tx, exit_tx)
        });
        let thread = match thread {
            Ok(thread) => thread,
            Err(e) => {
                // The closure, and its clones of the context, dropped with the failed spawn: no
                // source is attached yet, so dropping the rest here is safe.
                inner.take_main();
                return Err(e);
            }
        };
        if ready.recv().is_err() {
            let _ = thread.join();
            return Err(io::Error::other(
                "the GLib bridge thread could not own its main context",
            ));
        }
        Ok(GlibThread {
            glib: Glib { inner },
            exited,
            thread: Some(thread),
        })
    }

    /// A handle that sends work to this bridge.
    pub fn handle(&self) -> Glib {
        self.glib.clone()
    }

    /// Stops the bridge and waits up to `timeout` for its thread to exit. Dropping the
    /// `GlibThread` does the same with a one-second bound and ignores the result.
    pub fn shutdown(mut self, timeout: Duration) -> Result<(), ShutdownError> {
        self.stop(timeout)
    }

    fn stop(&mut self, timeout: Duration) -> Result<(), ShutdownError> {
        {
            // Under the lock `Glib::run` spawns under, so nothing is queued after the stop.
            let main = self.glib.inner.lock_main();
            self.glib.inner.stopped.store(true, Ordering::Release);
            if let Some((_, main_loop)) = &*main {
                main_loop.quit();
            }
        }
        let Some(thread) = self.thread.take() else {
            return Ok(());
        };
        if thread.thread().id() == thread::current().id() {
            return Err(ShutdownError::OnGlibThread);
        }
        // The thread drops its sender as it exits, so a disconnect means it is done.
        match self.exited.recv_timeout(timeout) {
            Err(RecvTimeoutError::Timeout) => Err(ShutdownError::TimedOut),
            Ok(()) | Err(RecvTimeoutError::Disconnected) => {
                thread.join().map_err(|_| ShutdownError::Panicked)
            }
        }
    }
}

impl Drop for GlibThread {
    fn drop(&mut self) {
        let _ = self.stop(DROP_TIMEOUT);
    }
}

/// The bridge thread's body. `exit` is never sent on: dropping it, on return or unwind, tells
/// [`GlibThread::stop`] the thread is done.
fn run(
    inner: Arc<Inner>,
    context: glib::MainContext,
    main_loop: glib::MainLoop,
    ready: mpsc::Sender<()>,
    exit: mpsc::Sender<()>,
) {
    // Fails only when another thread owns the context, which a fresh one can't be. `ready` then
    // drops unsent and `start` reports the error.
    let _ = context.with_thread_default(|| {
        registry::open();
        // Sent from inside the loop, so a `quit` from `stop` can't arrive before `run` starts
        // and be lost. Dropping a glib `JoinHandle` detaches its task; it doesn't cancel it.
        drop(context.spawn_local(async move {
            let _ = ready.send(());
        }));
        main_loop.run();
        registry::teardown();
        // Closures queued before the stop flag took effect run now, and find the registry
        // closed; dropped futures' sources finalize here, on this thread.
        for _ in 0..DRAIN_LIMIT {
            if !context.iteration(false) {
                break;
            }
        }
    });
    // The last references, once the context is no longer this thread's default: finalizing it
    // here drops what is still attached (an untracked local future, say) on this thread.
    inner.take_main();
    drop(main_loop);
    drop(context);
    drop(inner);
    drop(exit);
}

impl Inner {
    fn lock_main(&self) -> MutexGuard<'_, Option<(glib::MainContext, glib::MainLoop)>> {
        self.main.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn take_main(&self) {
        let main = self.lock_main().take();
        drop(main);
    }
}

impl Glib {
    /// Whether the bridge has stopped, or is stopping.
    pub fn is_stopped(&self) -> bool {
        self.inner.stopped.load(Ordering::Acquire)
    }

    /// Runs the future `make` builds on the GLib thread, and lands its output in the returned
    /// [`Task`]. `make` runs there too, so the future may create and hold GObjects; only `make`
    /// and the output cross threads.
    pub fn spawn<T, F>(&self, make: impl FnOnce() -> F + Send + 'static) -> Task<T>
    where
        T: Send + 'static,
        F: Future<Output = T> + 'static,
    {
        let key = Key::Task(self.next_id());
        let (task, delivery) = Task::new(self, key, self.inner.waker.clone());
        if self.is_stopped() {
            delivery.refuse();
            return task;
        }
        self.run(move || {
            // Cancelled, or drained after teardown: don't make what would only be dropped.
            if delivery.is_cancelled() || !registry::is_open() {
                return;
            }
            let future = make();
            // Refused during shutdown: the delivery drops with the future and stops the task.
            registry::spawn_tracked(key, async move {
                delivery.deliver(future.await);
            });
        });
        task
    }

    /// A queue from the GLib thread to the app: move the [`Emitter`] into an [`attach`] closure
    /// (several may share it), and drain the [`Inbox`] in `poll`.
    ///
    /// [`attach`]: Glib::attach
    pub fn channel<M: Send + 'static>(&self) -> (Emitter<M>, Inbox<M>) {
        channel::channel(self.inner.waker.clone())
    }

    /// The request and response path: the GLib side awaits [`Asker::ask`], and the app answers
    /// each [`Request`](super::Request) it drains from [`Requests`]. Move the asker into an
    /// [`attach`](Glib::attach) closure that connects the handler.
    pub fn requests<Q: Send + 'static, A: Send + 'static>(&self) -> (Asker<Q, A>, Requests<Q, A>) {
        let (emitter, inbox) = self.channel();
        (Asker::new(emitter), Requests::new(inbox))
    }

    /// Runs `make` on the GLib thread and keeps the [`Keep`] it returns there, until the
    /// [`Attachment`] drops or the bridge stops.
    pub fn attach(&self, make: impl FnOnce() -> Keep + Send + 'static) -> Attachment {
        Attachment::new(self, make)
    }

    /// The signal helper: `connect` runs on the GLib thread with an [`Emitter`], connects
    /// handlers that emit their arguments as `M`, and returns what to keep. The arguments arrive
    /// in the [`Signal`], drained in `poll`.
    ///
    /// ```ignore
    /// let changes = glib.listen(move |emit| {
    ///     let monitor = gio::File::for_path(&dir)
    ///         .monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
    ///         .expect("a local folder can be monitored");
    ///     let id = monitor.connect_changed(move |_, file, _, event| {
    ///         emit.emit((event, file.path()));
    ///     });
    ///     Keep::new().handler(&monitor, id)
    /// });
    /// ```
    pub fn listen<M: Send + 'static>(
        &self,
        connect: impl FnOnce(Emitter<M>) -> Keep + Send + 'static,
    ) -> Signal<M> {
        let (emitter, inbox) = self.channel();
        let attachment = self.attach(move || connect(emitter));
        Signal::new(attachment, inbox)
    }

    pub(super) fn next_id(&self) -> u64 {
        self.inner.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Queues `f` on the GLib thread. Dropped at once, on this thread, once the bridge stopped.
    ///
    /// `MainContext::spawn`, not `invoke`: `invoke` runs the closure on the calling thread when
    /// no thread owns the context, which is the case after shutdown.
    pub(super) fn run(&self, f: impl FnOnce() + Send + 'static) {
        let main = self.inner.lock_main();
        if let (false, Some((context, _))) = (self.is_stopped(), &*main) {
            drop(context.spawn(async move { f() }));
        }
    }
}
