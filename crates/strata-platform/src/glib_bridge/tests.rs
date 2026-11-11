//! Headless tests of the bridge: GIO on the bridge thread, answers on a `rune::Runtime`'s loop.
//!
//! Each test re-runs itself in a child copy of this binary (see [`isolated`]) with GIO kept off
//! every D-Bus bus, because GIO reaches the session bus (and starts gvfsd) unless
//! `GIO_USE_VFS=local` is in its environment, and a test can't set that in-process without
//! `unsafe`.

use std::io::{ErrorKind, Read};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};
use std::{fs, thread};

use gio::prelude::*;
use rune::{Cx, El, Env, Runtime, Tx};

use super::*;

// The handles the module docs say cross threads.
const _: fn() = || {
    fn send<T: Send>() {}
    send::<GlibThread>();
    send::<Glib>();
    send::<Task<String>>();
    send::<Signal<String>>();
    send::<Attachment>();
    send::<Inbox<String>>();
    send::<Emitter<String>>();
    send::<Requests<String, String>>();
    send::<Request<String, String>>();
    send::<Asker<String, String>>();
};

/// Set in the child, which then runs the test body.
const CHILD: &str = "STRATA_GLIB_BRIDGE_TEST_CHILD";

/// How long a test waits for something the GLib thread sends.
const DEADLINE: Duration = Duration::from_secs(5);

/// How long the child copy of a test may run before it is killed.
const CHILD_DEADLINE: Duration = Duration::from_secs(30);

/// Runs `body` in a child copy of this test binary, where GIO stays local and every bus address
/// points at a socket this process listens on. Fails if the child fails, ran no test, or
/// connected to either bus.
fn isolated(name: &str, body: fn()) {
    if std::env::var_os(CHILD).is_some() {
        body();
        return;
    }
    let scratch = Scratch::new();
    let session = scratch.path().join("session-bus");
    let system = scratch.path().join("system-bus");
    let buses = [("session", listen(&session)), ("system", listen(&system))];
    let runtime_dir = scratch.path().join("run");
    fs::create_dir(&runtime_dir).expect("make the runtime dir");
    fs::set_permissions(&runtime_dir, fs::Permissions::from_mode(0o700))
        .expect("make the runtime dir private");

    let module = module_path!();
    let module = module.split_once("::").map_or(module, |(_, rest)| rest);
    let test = format!("{module}::{name}");
    let mut child = Command::new(std::env::current_exe().expect("the test binary's path"))
        .args([test.as_str(), "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD, "1")
        .env("GIO_USE_VFS", "local")
        .env("GSETTINGS_BACKEND", "memory")
        .env("DBUS_SESSION_BUS_ADDRESS", bus_address(&session))
        .env("DBUS_SYSTEM_BUS_ADDRESS", bus_address(&system))
        .env("XDG_RUNTIME_DIR", &runtime_dir)
        .env("TMPDIR", scratch.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the child test");
    // Read on threads, so a full pipe can't stall the child while this one waits.
    let stdout = read_all(child.stdout.take().expect("the child's stdout"));
    let stderr = read_all(child.stderr.take().expect("the child's stderr"));
    let deadline = Instant::now() + CHILD_DEADLINE;
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait for the child test") {
            break Some(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        thread::sleep(Duration::from_millis(20));
    };
    let stdout = stdout.join().expect("read the child's stdout");
    let stderr = stderr.join().expect("read the child's stderr");
    let Some(status) = status else {
        panic!("{test} ran past {CHILD_DEADLINE:?} in the child:\n{stdout}\n{stderr}");
    };
    assert!(
        status.success(),
        "{test} failed in the child ({status}):\n{stdout}\n{stderr}"
    );
    assert!(
        stdout.contains("test result: ok. 1 passed"),
        "the child ran no test named {test}:\n{stdout}"
    );
    for (bus, listener) in buses {
        match listener.accept() {
            Err(e) if e.kind() == ErrorKind::WouldBlock => {}
            Ok(_) => panic!("{test} connected to the {bus} bus"),
            Err(e) => panic!("checking the {bus} bus: {e}"),
        }
    }
}

fn read_all(mut pipe: impl Read + Send + 'static) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = pipe.read_to_end(&mut bytes);
        String::from_utf8_lossy(&bytes).into_owned()
    })
}

fn listen(path: &Path) -> UnixListener {
    let listener = UnixListener::bind(path).expect("listen on a stand-in bus");
    listener
        .set_nonblocking(true)
        .expect("a non-blocking listener");
    listener
}

fn bus_address(path: &Path) -> String {
    format!("unix:path={}", path.display())
}

/// A directory removed on drop. Its name is short, as the stand-in bus sockets inside it must
/// fit a socket address (108 bytes).
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("glib-bridge-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("make a scratch dir");
        Scratch(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Reports the name of the thread it drops on.
struct DropReport(mpsc::Sender<Option<String>>);

impl Drop for DropReport {
    fn drop(&mut self) {
        let _ = self.0.send(thread::current().name().map(String::from));
    }
}

/// A waker that signals the test thread, as Rune's setup waker wakes the loop.
fn waker() -> (rune::Waker, Receiver<()>) {
    let (tx, rx) = mpsc::channel();
    (
        Arc::new(move || {
            let _ = tx.send(());
        }),
        rx,
    )
}

fn view<S>(_: &S, _: &mut Cx<S>) -> El {
    rune::el()
}

fn runtime<S: 'static>(state: S, poll: fn(&mut S, f64, &mut Tx) -> bool) -> Runtime<S> {
    let mut rt = Runtime::new(state, view::<S>, (320.0, 240.0), Env::default());
    rt.poll = Some(poll);
    rt
}

/// Steps frames on the virtual clock until `done`, sleeping on the waker between them.
fn frames_until<S>(
    rt: &mut Runtime<S>,
    wakes: &Receiver<()>,
    what: &str,
    done: impl Fn(&S) -> bool,
) {
    let deadline = Instant::now() + DEADLINE;
    loop {
        rt.frame(1.0 / 60.0);
        if done(&rt.state) {
            return;
        }
        let now = Instant::now();
        assert!(now < deadline, "timed out waiting for {what}");
        let _ = wakes.recv_timeout((deadline - now).min(Duration::from_millis(50)));
    }
}

/// Steps frames for `period` of real time, for checks that something does not arrive.
fn frames_for<S>(rt: &mut Runtime<S>, period: Duration) {
    let end = Instant::now() + period;
    while Instant::now() < end {
        rt.frame(1.0 / 60.0);
        thread::sleep(Duration::from_millis(10));
    }
}

/// The bridge threads alive in this process.
fn bridge_threads() -> usize {
    fs::read_dir("/proc/self/task")
        .expect("list this process's threads")
        .filter_map(Result::ok)
        .filter_map(|task| fs::read_to_string(task.path().join("comm")).ok())
        .filter(|comm| comm.trim_end() == THREAD_NAME)
        .count()
}

#[derive(Debug, PartialEq)]
struct Info {
    name: String,
    size: i64,
    queried_on: Option<String>,
}

struct Query {
    _bridge: GlibThread,
    task: Task<Result<Info, String>>,
    landed: Option<(Result<Info, String>, thread::ThreadId)>,
}

fn poll_query(s: &mut Query, _: f64, _: &mut Tx) -> bool {
    let Some(output) = s.task.take() else {
        return false;
    };
    s.landed = Some((output, thread::current().id()));
    true
}

#[test]
fn a_file_query_lands_on_runes_loop() {
    isolated("a_file_query_lands_on_runes_loop", || {
        let scratch = Scratch::new();
        let path = scratch.path().join("note.txt");
        fs::write(&path, b"hello").expect("write the file");
        let (waker, wakes) = waker();
        let bridge = GlibThread::start(waker).expect("the bridge starts");

        let task = bridge.handle().spawn(move || async move {
            let info = gio::File::for_path(&path)
                .query_info_future(
                    "standard::name,standard::size",
                    gio::FileQueryInfoFlags::NONE,
                    glib::Priority::DEFAULT,
                )
                .await
                .map_err(|e| e.to_string())?;
            Ok(Info {
                name: info.name().display().to_string(),
                size: info.size(),
                queried_on: thread::current().name().map(String::from),
            })
        });
        let mut rt = runtime(
            Query {
                _bridge: bridge,
                task,
                landed: None,
            },
            poll_query,
        );
        frames_until(&mut rt, &wakes, "the query", |s| s.landed.is_some());

        let (output, polled_on) = rt.state.landed.take().expect("the query landed");
        let expected = Info {
            name: "note.txt".into(),
            size: 5,
            queried_on: Some(THREAD_NAME.into()),
        };
        assert_eq!(output, Ok(expected));
        assert_eq!(
            polled_on,
            thread::current().id(),
            "the answer lands on the Runtime's thread"
        );
        assert_eq!(rt.state.task.state(), TaskState::Done);
    });
}

enum Change {
    Ready,
    Created(String),
}

struct Watch {
    _bridge: GlibThread,
    changes: Option<Signal<Change>>,
    ready: bool,
    created: Vec<String>,
}

fn poll_watch(s: &mut Watch, _: f64, _: &mut Tx) -> bool {
    let Some(changes) = &s.changes else {
        return false;
    };
    let mut changed = false;
    for change in changes.drain() {
        changed = true;
        match change {
            Change::Ready => s.ready = true,
            Change::Created(name) => s.created.push(name),
        }
    }
    changed
}

#[test]
fn a_file_monitor_change_arrives() {
    isolated("a_file_monitor_change_arrives", || {
        let scratch = Scratch::new();
        let dir = scratch.path().join("watched");
        fs::create_dir(&dir).expect("make the folder");
        let (waker, wakes) = waker();
        let bridge = GlibThread::start(waker).expect("the bridge starts");
        let handled = Arc::new(AtomicUsize::new(0));
        let (released_tx, released) = mpsc::channel();

        let changes = bridge.handle().listen({
            let dir = dir.clone();
            let handled = handled.clone();
            move |emit| {
                let monitor = gio::File::for_path(&dir)
                    .monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
                    .expect("a local folder can be monitored");
                let ready = emit.clone();
                let id = monitor.connect_changed(move |_, file, _, event| {
                    handled.fetch_add(1, Ordering::SeqCst);
                    if event == gio::FileMonitorEvent::Created {
                        let name = file.basename().expect("a file name");
                        emit.emit(Change::Created(name.display().to_string()));
                    }
                });
                ready.emit(Change::Ready);
                Keep::new()
                    .handler(&monitor, id)
                    .value(DropReport(released_tx))
            }
        });
        let state = Watch {
            _bridge: bridge,
            changes: Some(changes),
            ready: false,
            created: Vec::new(),
        };
        let mut rt = runtime(state, poll_watch);
        frames_until(&mut rt, &wakes, "the monitor", |s| s.ready);

        fs::write(dir.join("first.txt"), b"1").expect("write a file");
        frames_until(&mut rt, &wakes, "the created event", |s| {
            s.created.iter().any(|n| n == "first.txt")
        });

        // Dropping the Signal releases its Keep on the GLib thread, and the handler with it.
        rt.state.changes = None;
        let released_on = released
            .recv_timeout(DEADLINE)
            .expect("the Keep is released");
        assert_eq!(released_on.as_deref(), Some(THREAD_NAME));
        let handled_before = handled.load(Ordering::SeqCst);
        fs::write(dir.join("second.txt"), b"2").expect("write a file");
        frames_for(&mut rt, Duration::from_millis(300));
        assert_eq!(
            handled.load(Ordering::SeqCst),
            handled_before,
            "the handler is disconnected"
        );
        assert!(!rt.state.created.iter().any(|n| n == "second.txt"));
    });
}

#[derive(Debug, PartialEq)]
struct Reply {
    result: String,
    password: Option<String>,
}

struct Mount {
    _bridge: GlibThread,
    _attachment: Attachment,
    asks: Requests<String, String>,
    replies: Inbox<Reply>,
    answer: Option<String>,
    asked: Vec<String>,
    seen: Vec<Reply>,
}

fn poll_mount(s: &mut Mount, _: f64, _: &mut Tx) -> bool {
    let mut changed = false;
    for request in s.asks.drain() {
        changed = true;
        s.asked.push(request.question().clone());
        match &s.answer {
            Some(password) => assert!(request.respond(password.clone()), "the handler waits"),
            None => drop(request),
        }
    }
    for reply in s.replies.drain() {
        changed = true;
        s.seen.push(reply);
    }
    changed
}

/// A `gio::MountOperation` on the bridge whose `ask-password` handler asks the app, as C26's
/// binding does. The test plays GIO's part and emits `ask-password` itself. Returns what the app
/// was asked and the replies the operation made.
fn ask_for_a_password(answer: Option<&str>) -> (Vec<String>, Vec<Reply>) {
    let (waker, wakes) = waker();
    let bridge = GlibThread::start(waker).expect("the bridge starts");
    let glib = bridge.handle();
    let (asker, asks) = glib.requests::<String, String>();
    let (emit, replies) = glib.channel::<Reply>();

    let attachment = glib.attach(move || {
        let op = gio::MountOperation::new();
        let ask = op.connect_ask_password(move |op, message, _user, _domain, _flags| {
            let (op, asker, message) = (op.clone(), asker.clone(), message.to_owned());
            spawn_local(async move {
                match asker.ask(message).await {
                    Some(password) => {
                        op.set_password(Some(&password));
                        op.reply(gio::MountOperationResult::Handled);
                    }
                    None => op.reply(gio::MountOperationResult::Aborted),
                }
            });
        });
        let reply = op.connect_reply(move |op, result| {
            emit.emit(Reply {
                result: format!("{result:?}"),
                password: op.password().map(String::from),
            });
        });
        op.emit_by_name::<()>(
            "ask-password",
            &[
                &"Password for the share",
                &"ann",
                &"WORKGROUP",
                &gio::AskPasswordFlags::NEED_PASSWORD,
            ],
        );
        Keep::new().handler(&op, ask).handler(&op, reply)
    });
    let state = Mount {
        _bridge: bridge,
        _attachment: attachment,
        asks,
        replies,
        answer: answer.map(String::from),
        asked: Vec::new(),
        seen: Vec::new(),
    };
    let mut rt = runtime(state, poll_mount);
    frames_until(&mut rt, &wakes, "the reply", |s| !s.seen.is_empty());
    // Long enough for a second reply to show, if anything else answered too.
    frames_for(&mut rt, Duration::from_millis(200));
    let Mount { asked, seen, .. } = rt.state;
    (asked, seen)
}

#[test]
fn a_request_round_trips_through_a_fake_handler() {
    isolated("a_request_round_trips_through_a_fake_handler", || {
        let (asked, replies) = ask_for_a_password(Some("hunter2"));
        assert_eq!(asked, ["Password for the share"]);
        let handled = Reply {
            result: "Handled".into(),
            password: Some("hunter2".into()),
        };
        assert_eq!(replies, [handled]);
    });
}

#[test]
fn an_unanswered_request_aborts() {
    isolated("an_unanswered_request_aborts", || {
        let (asked, replies) = ask_for_a_password(None);
        assert_eq!(asked, ["Password for the share"]);
        let aborted = Reply {
            result: "Aborted".into(),
            password: None,
        };
        assert_eq!(replies, [aborted]);
    });
}

#[test]
fn dropping_a_task_cancels_it_on_the_glib_thread() {
    isolated("dropping_a_task_cancels_it_on_the_glib_thread", || {
        let (waker, _wakes) = waker();
        let bridge = GlibThread::start(waker).expect("the bridge starts");
        let (started_tx, started) = mpsc::channel();
        let (dropped_tx, dropped) = mpsc::channel();

        let task = bridge.handle().spawn(move || async move {
            let _report = DropReport(dropped_tx);
            let _ = started_tx.send(());
            glib::timeout_future(Duration::from_secs(3600)).await;
        });
        started.recv_timeout(DEADLINE).expect("the task starts");
        drop(task);

        let dropped_on = dropped
            .recv_timeout(DEADLINE)
            .expect("the future is dropped");
        assert_eq!(dropped_on.as_deref(), Some(THREAD_NAME));
    });
}

struct Running {
    _bridge: GlibThread,
    _task: Task<()>,
    _signal: Signal<()>,
}

#[test]
fn the_thread_exits_when_the_runtime_drops() {
    isolated("the_thread_exits_when_the_runtime_drops", || {
        let (waker, _wakes) = waker();
        let bridge = GlibThread::start(waker).expect("the bridge starts");
        let handle = bridge.handle();
        let (started_tx, started) = mpsc::channel();
        let (dropped_tx, dropped) = mpsc::channel();

        let task = handle.spawn({
            let started = started_tx.clone();
            let dropped = dropped_tx.clone();
            move || async move {
                let _report = DropReport(dropped);
                let _ = started.send(());
                glib::timeout_future(Duration::from_secs(3600)).await;
            }
        });
        let signal = handle.listen(move |_: Emitter<()>| {
            let _ = started_tx.send(());
            Keep::new().value(DropReport(dropped_tx))
        });
        for what in ["the task", "the attachment"] {
            started
                .recv_timeout(DEADLINE)
                .unwrap_or_else(|_| panic!("{what} starts"));
        }
        let state = Running {
            _bridge: bridge,
            _task: task,
            _signal: signal,
        };
        let mut rt = runtime(state, |_, _, _| false);
        rt.frame(1.0 / 60.0);
        assert_eq!(bridge_threads(), 1);

        drop(rt);

        // The drop joined the thread, so its drops are already done.
        for what in ["the pending future", "the Keep"] {
            let dropped_on = dropped
                .try_recv()
                .unwrap_or_else(|_| panic!("{what} is dropped"));
            assert_eq!(
                dropped_on.as_deref(),
                Some(THREAD_NAME),
                "{what} drops on the GLib thread"
            );
        }
        // The kernel's thread list can lag the join a little, so that is polled.
        let deadline = Instant::now() + Duration::from_secs(1);
        while bridge_threads() > 0 {
            assert!(Instant::now() < deadline, "the bridge thread exited");
            thread::sleep(Duration::from_millis(5));
        }
        assert!(handle.is_stopped());
        assert_eq!(handle.spawn(|| async {}).state(), TaskState::Stopped);
    });
}

#[test]
fn an_untracked_future_pending_at_shutdown_drops_on_the_glib_thread() {
    isolated(
        "an_untracked_future_pending_at_shutdown_drops_on_the_glib_thread",
        || {
            let (waker, _wakes) = waker();
            let bridge = GlibThread::start(waker).expect("the bridge starts");
            let handle = bridge.handle();
            let (started_tx, started) = mpsc::channel();
            let (dropped_tx, dropped) = mpsc::channel();

            let _attachment = handle.attach(move || {
                // Against the module's rule (use spawn_local): the bridge doesn't track it.
                glib::spawn_future_local(async move {
                    let _report = DropReport(dropped_tx);
                    let _ = started_tx.send(());
                    glib::timeout_future(Duration::from_secs(3600)).await;
                });
                Keep::new()
            });
            started.recv_timeout(DEADLINE).expect("the future starts");
            drop(bridge);

            let dropped_on = dropped
                .recv_timeout(DEADLINE)
                .expect("the future is dropped");
            assert_eq!(dropped_on.as_deref(), Some(THREAD_NAME));
            drop(handle);
        },
    );
}

thread_local! {
    /// The withdrawal test's ask, on the GLib thread.
    static ASK: std::cell::RefCell<Option<LocalTask>> = const { std::cell::RefCell::new(None) };
}

struct Held {
    _bridge: GlibThread,
    _attachment: Attachment,
    asks: Requests<String, String>,
    held: Option<Request<String, String>>,
    withdrawn: bool,
}

fn poll_held(s: &mut Held, _: f64, _: &mut Tx) -> bool {
    if let Some(request) = s.asks.drain().next() {
        s.held = Some(request);
    }
    let withdrawn = s.held.as_ref().is_some_and(Request::is_withdrawn);
    let changed = withdrawn != s.withdrawn;
    s.withdrawn = withdrawn;
    changed
}

#[test]
fn a_dropped_ask_withdraws_its_request() {
    isolated("a_dropped_ask_withdraws_its_request", || {
        let (waker, wakes) = waker();
        let bridge = GlibThread::start(waker).expect("the bridge starts");
        let handle = bridge.handle();
        let (asker, asks) = handle.requests::<String, String>();
        let attachment = handle.attach(move || {
            let ask = spawn_local(async move {
                let _ = asker.ask("Unlock the drive?".into()).await;
            });
            ASK.set(Some(ask));
            Keep::new()
        });
        let state = Held {
            _bridge: bridge,
            _attachment: attachment,
            asks,
            held: None,
            withdrawn: false,
        };
        let mut rt = runtime(state, poll_held);
        frames_until(&mut rt, &wakes, "the request", |s| s.held.is_some());
        assert!(!rt.state.withdrawn, "the asker is still waiting");

        // The GLib side gives up on the question, as a cancelled mount does.
        while wakes.try_recv().is_ok() {}
        let _abort = handle.attach(|| {
            ASK.take().expect("the ask is running").abort();
            Keep::new()
        });
        wakes
            .recv_timeout(DEADLINE)
            .expect("the withdrawal wakes the app");
        rt.frame(1.0 / 60.0);
        assert!(rt.state.withdrawn, "poll sees the request withdrawn");
    });
}
