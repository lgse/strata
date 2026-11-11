//! Claiming the app id, and the primary that then serves it.

use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use tokio::sync::oneshot;
use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::{Connection, connection};

use super::app_id::AppId;
use super::event::{CommandLine, Event};
use super::forward::{self, Forward};
use super::interfaces::{
    FILE_MANAGER1, FILE_MANAGER1_PATH, FileManager1, FreedesktopApplication, GtkActions,
    GtkApplication,
};
use super::launch::Launch;

/// The bus thread's name.
pub const THREAD_NAME: &str = "single-instance-bus";

/// How many times, in all, a launch tries to claim the app id or reach its primary: a
/// primary that goes away mid-launch costs one try.
const ATTEMPTS: usize = 3;

/// How [`claim`] behaves.
#[derive(Clone, Debug)]
pub struct Options {
    /// The longest the claim waits to connect to the session bus and hear whether it owns the
    /// app id. A bus that never answers leaves the launch [`Unclaimed::Timeout`].
    pub bus_timeout: Duration,
    /// The longest a forwarded launch waits for the primary's answer, which comes once the app
    /// is done with its [`CommandLine`].
    pub forward_timeout: Duration,
    /// A launch the session bus started to provide a service (D-Bus activation): it queues
    /// no [`Event::CommandLine`] of its own, and when another process already is the primary
    /// it ends with `Claim::Forwarded(0)`, forwarding nothing.
    ///
    /// GLib apps mark such a launch with `--gapplication-service`, which GApplication parses
    /// itself, and their `.service` files pass it (a GTK app's `Exec=app
    /// --gapplication-service`). Nothing here reads arguments: a host that keeps such a
    /// `.service` file takes the flag out of its arguments and sets this instead.
    pub service: bool,
    /// Also serve `org.freedesktop.FileManager1` as the primary, delivering its calls as
    /// [`Event::Reveal`]. The name is queued for, never taken: while another file manager
    /// holds it, calls keep reaching that one. Asking for it is best effort: a bus that
    /// refuses it (a sandbox without permission to own it) leaves the primary claimed without
    /// it, and [`Primary::file_manager1`] says so.
    pub file_manager1: bool,
    /// The actions `ActivateAction` (and `org.gtk.Actions.Activate`) may name, delivered as
    /// [`Event::Action`]. Any other name is refused with `InvalidArgs`, as a GApplication
    /// refuses an action it doesn't export. Parameters aren't type-checked.
    pub actions: Vec<String>,
    /// Tests only: the name to ask for instead of `org.freedesktop.FileManager1`.
    #[doc(hidden)]
    pub file_manager1_name: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            bus_timeout: Duration::from_secs(5),
            forward_timeout: Duration::from_secs(30),
            service: false,
            file_manager1: false,
            actions: Vec::new(),
            file_manager1_name: None,
        }
    }
}

/// How a launch ended up.
#[derive(Debug)]
pub enum Claim {
    /// This process owns the app id. Its own launch is the first [`Event::CommandLine`].
    Primary(Primary),
    /// Another process is the primary and took this launch. Exit with this status: the
    /// primary's answer, or 1 when it couldn't be reached (said on stderr).
    Forwarded(i32),
    /// The app id couldn't be claimed: no session bus, one that didn't answer within
    /// `Options::bus_timeout`, or one that answered with an error ([`Unclaimed`]). This
    /// process runs on its own, unclaimed. The [`Primary`] still delivers its own launch, and
    /// nothing else.
    Standalone(Primary, Unclaimed),
}

/// What became of the request for a name beside the app id (`org.freedesktop.FileManager1`),
/// when the claim made it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameRequest {
    /// The primary owns it.
    Owned,
    /// Another process owns it; the primary waits in the bus's queue and takes it when that
    /// one lets go.
    Queued,
    /// The bus refused it, with this error. The primary runs without it.
    Failed(String),
}

/// Why a launch couldn't claim the app id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unclaimed {
    /// No session bus answered: none is set, or connecting failed.
    NoBus(String),
    /// The bus took longer than `Options::bus_timeout`.
    Timeout,
    /// The bus answered with an error.
    Error(String),
}

/// Claims `id` on the session bus for this launch.
///
/// Call it first in `main`, before Rune starts. The bus decides between launches at the same
/// moment, so exactly one becomes the primary.
///
/// - [`Claim::Primary`]: attach the app ([`Primary::attach_rune`]) and run it.
/// - [`Claim::Forwarded`]: the primary got the launch (its arguments, working directory and
///   activation token); exit with the status.
/// - [`Claim::Standalone`]: run as the primary would, alone.
///
/// The bus is `DBUS_SESSION_BUS_ADDRESS`, else `$XDG_RUNTIME_DIR/bus`.
///
/// A launch whose primary goes away before answering (it quit, crashed or released the app id
/// meanwhile) tries again, and may become the primary itself. A primary that had already acted
/// on the launch, then went away before its answer left, has seen it once already: the launch
/// is then handled twice, by the old primary and by whichever takes it next.
pub fn claim(id: &AppId, launch: Launch, options: &Options) -> Claim {
    let mut gone = None;
    for _ in 0..ATTEMPTS {
        match try_primary(id, &launch, options) {
            Attempt::Primary(primary) => return Claim::Primary(primary),
            Attempt::Unclaimed(reason) => {
                return Claim::Standalone(Primary::standalone(launch, options), reason);
            }
            Attempt::Taken if options.service => return Claim::Forwarded(0),
            Attempt::Taken => match forward::forward(id, &launch, options) {
                Forward::Answered(status) => return Claim::Forwarded(status),
                Forward::Gone(why) => gone = Some(why),
                Forward::Failed(why) => return failed(id, &why),
            },
        }
    }
    failed(
        id,
        gone.as_deref()
            .unwrap_or("the running instance kept going away"),
    )
}

fn failed(id: &AppId, why: &str) -> Claim {
    super::event::write_ignoring_errors(
        &mut std::io::stderr().lock(),
        &format!("{id}: could not reach the running instance: {why}\n"),
    );
    Claim::Forwarded(1)
}

enum Attempt {
    Primary(Primary),
    Taken,
    Unclaimed(Unclaimed),
}

/// What the bus thread tells `claim`.
enum Setup {
    Primary(Bus, Option<NameRequest>),
    Taken,
    Unclaimed(Unclaimed),
}

/// One try at the app id, on a new bus thread. The thread stays as the primary's, or ends.
fn try_primary(id: &AppId, launch: &Launch, options: &Options) -> Attempt {
    let inbox = Arc::new(Inbox::default());
    let status = Arc::new(AtomicI32::new(0));
    // Queued before the name is, so this launch comes before any forwarded one.
    if !options.service {
        inbox.deliver(Event::CommandLine(CommandLine::local(
            launch.clone(),
            status.clone(),
        )));
    }
    let (tx, rx) = mpsc::channel();
    // Set when `claim` stopped waiting: a late connection must not become a primary nobody
    // serves.
    let abandoned = Arc::new(AtomicBool::new(false));
    let thread = {
        let (id, inbox, abandoned) = (id.clone(), inbox.clone(), abandoned.clone());
        let options = options.clone();
        std::thread::Builder::new()
            .name(THREAD_NAME.into())
            .spawn(move || serve(id, inbox, options, abandoned, tx))
    };
    if let Err(e) = thread {
        return Attempt::Unclaimed(Unclaimed::Error(format!("starting {THREAD_NAME}: {e}")));
    }
    // The thread times itself out; the margin covers a thread that never got to run.
    let setup = rx.recv_timeout(options.bus_timeout + Duration::from_secs(1));
    match setup {
        Ok(Setup::Primary(bus, file_manager1)) => Attempt::Primary(Primary {
            inbox,
            status,
            bus: Some(bus),
            file_manager1,
        }),
        Ok(Setup::Taken) => Attempt::Taken,
        Ok(Setup::Unclaimed(reason)) => Attempt::Unclaimed(reason),
        Err(_) => {
            abandoned.store(true, Ordering::SeqCst);
            Attempt::Unclaimed(Unclaimed::Timeout)
        }
    }
}

/// The bus thread: claims the name, then serves until the primary is released or dropped.
fn serve(
    id: AppId,
    inbox: Arc<Inbox>,
    options: Options,
    abandoned: Arc<AtomicBool>,
    tx: mpsc::Sender<Setup>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            let _ = tx.send(Setup::Unclaimed(Unclaimed::Error(format!(
                "starting the bus runtime: {e}"
            ))));
            return;
        }
    };
    runtime.block_on(async move {
        let extra = options.file_manager1.then(|| {
            options
                .file_manager1_name
                .clone()
                .unwrap_or_else(|| FILE_MANAGER1.to_owned())
        });
        let in_flight = Arc::new(InFlight::default());
        let setup = tokio::time::timeout(
            options.bus_timeout,
            become_primary(&id, &inbox, &in_flight, extra.as_deref(), &options.actions),
        )
        .await;
        let (conn, file_manager1) = match setup {
            Ok(Ok(Some(claimed))) => claimed,
            other => {
                let _ = tx.send(match other {
                    Ok(Ok(_)) => Setup::Taken,
                    Ok(Err(reason)) => Setup::Unclaimed(reason),
                    Err(_) => Setup::Unclaimed(Unclaimed::Timeout),
                });
                return;
            }
        };
        let (stop, stopped) = oneshot::channel::<mpsc::Sender<()>>();
        let bus = Bus {
            conn: Mutex::new(Some(conn.clone())),
            runtime: tokio::runtime::Handle::current(),
            stop: Mutex::new(Some(stop)),
            bus_timeout: options.bus_timeout,
            thread: std::thread::current().id(),
        };
        let names: Vec<&str> = std::iter::once(id.as_str())
            .chain(extra.as_deref())
            .collect();
        if abandoned.load(Ordering::SeqCst) || tx.send(Setup::Primary(bus, file_manager1)).is_err()
        {
            release(&conn, &names).await;
            return;
        }
        // Released (`Primary::release`) or dropped: give the names up, so the next launch
        // claims them, and let calls still waiting fail, so their launches try again.
        let done = stopped.await.ok();
        release(&conn, &names).await;
        finish_answers(&conn, &in_flight, options.bus_timeout).await;
        drop(conn);
        if let Some(done) = done {
            let _ = done.send(());
        }
    });
}

/// Exports the interfaces, then asks for the name. They are exported first so that a launch
/// that finds the name taken always reaches them.
async fn become_primary(
    id: &AppId,
    inbox: &Arc<Inbox>,
    in_flight: &Arc<InFlight>,
    file_manager1: Option<&str>,
    actions: &[String],
) -> Result<Option<(Connection, Option<NameRequest>)>, Unclaimed> {
    let conn = connection::Builder::session()
        .build()
        .await
        .map_err(|e| Unclaimed::NoBus(e.to_string()))?;
    let error = |e: zbus::Error| Unclaimed::Error(e.to_string());
    let path = id.object_path();
    let server = conn.object_server();
    let inbox = || inbox.clone();
    let actions: Arc<[String]> = actions.into();
    server
        .at(
            path.as_str(),
            GtkApplication {
                inbox: inbox(),
                in_flight: in_flight.clone(),
            },
        )
        .await
        .map_err(error)?;
    server
        .at(
            path.as_str(),
            FreedesktopApplication {
                inbox: inbox(),
                actions: actions.clone(),
            },
        )
        .await
        .map_err(error)?;
    server
        .at(
            path.as_str(),
            GtkActions {
                inbox: inbox(),
                actions,
            },
        )
        .await
        .map_err(error)?;
    if file_manager1.is_some() {
        server
            .at(FILE_MANAGER1_PATH, FileManager1 { inbox: inbox() })
            .await
            .map_err(error)?;
    }
    match conn
        .request_name_with_flags(id.as_str(), RequestNameFlags::DO_NOT_QUEUE)
        .await
    {
        Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => {}
        Err(zbus::Error::NameTaken) | Ok(RequestNameReply::Exists) => return Ok(None),
        Ok(RequestNameReply::InQueue) => {
            return Err(Unclaimed::Error(format!("{id} was queued, not owned")));
        }
        Err(e) => return Err(error(e)),
    }
    let file_manager1 = match file_manager1 {
        None => None,
        // No flags: wait in the queue behind another file manager, never replace it. Best
        // effort: the app id is what makes the primary.
        Some(name) => Some(
            match conn
                .request_name_with_flags(name, RequestNameFlags::empty())
                .await
            {
                Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => {
                    NameRequest::Owned
                }
                Ok(RequestNameReply::InQueue) => NameRequest::Queued,
                Ok(RequestNameReply::Exists) => NameRequest::Failed(format!("{name} is taken")),
                Err(e) => NameRequest::Failed(e.to_string()),
            },
        ),
    };
    Ok(Some((conn, file_manager1)))
}

async fn release(conn: &Connection, names: &[&str]) {
    for name in names {
        let _ = conn.release_name(*name).await;
    }
}

/// After the names are given up and before the connection and runtime go (dropping the runtime
/// aborts every handler still running): lets each forwarded launch already answered finish
/// sending its output and status, and tells those the app still holds that the primary left,
/// so they claim the app id again. Bounded by `limit`.
///
/// Then a round trip to the bus on the same connection: zbus writes a handler's reply after
/// the handler returns, so once none is left in flight and the bus has answered a later call,
/// it has read every reply written before it.
async fn finish_answers(conn: &Connection, in_flight: &InFlight, limit: Duration) {
    in_flight.stop_waiting();
    let _ = tokio::time::timeout(limit, async {
        in_flight.drained().await;
        // The last handler's own task writes its reply after the handler returns.
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
        let _ = conn
            .call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus.Peer"),
                "Ping",
                &(),
            )
            .await;
    })
    .await;
}

/// The forwarded launches the primary is answering (`CommandLine` calls in flight), and
/// whether it is giving the app id up.
#[derive(Debug, Default)]
pub(super) struct InFlight {
    state: Mutex<(usize, bool)>,
    changed: tokio::sync::Notify,
}

/// One forwarded launch in flight, until dropped.
pub(super) struct Answering<'a>(&'a InFlight);

impl Drop for Answering<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.state.lock() {
            state.0 -= 1;
        }
        self.0.changed.notify_waiters();
    }
}

impl InFlight {
    /// Counts a launch in, unless the primary is giving the app id up.
    pub(super) fn enter(&self) -> Option<Answering<'_>> {
        let mut state = self.state.lock().ok()?;
        if state.1 {
            return None;
        }
        state.0 += 1;
        Some(Answering(self))
    }

    fn stop_waiting(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.1 = true;
        }
        self.changed.notify_waiters();
    }

    /// Returns once the primary is giving the app id up.
    pub(super) async fn stopped(&self) {
        self.wait_for(|(_, stopping)| stopping).await;
    }

    async fn drained(&self) {
        self.wait_for(|(count, _)| count == 0).await;
    }

    async fn wait_for(&self, done: impl Fn((usize, bool)) -> bool) {
        loop {
            // Registered before the check, so a change between the two isn't missed.
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.state.lock().map_or(true, |s| done(*s)) {
                return;
            }
            changed.await;
        }
    }
}

/// The primary's side of the bus.
#[derive(Debug)]
struct Bus {
    /// Taken on release: once every clone is gone the socket closes, and a launch still
    /// waiting for an answer hears at once that the primary left.
    conn: Mutex<Option<Connection>>,
    runtime: tokio::runtime::Handle,
    stop: Mutex<Option<oneshot::Sender<mpsc::Sender<()>>>>,
    bus_timeout: Duration,
    /// The bus thread: `release` from it can't wait for it.
    thread: std::thread::ThreadId,
}

/// The instance that handles launches: this process's own, then every one forwarded to it.
#[derive(Debug)]
pub struct Primary {
    inbox: Arc<Inbox>,
    status: Arc<AtomicI32>,
    bus: Option<Bus>,
    file_manager1: Option<NameRequest>,
}

impl Primary {
    fn standalone(launch: Launch, options: &Options) -> Primary {
        let inbox = Arc::new(Inbox::default());
        let status = Arc::new(AtomicI32::new(0));
        if !options.service {
            inbox.deliver(Event::CommandLine(CommandLine::local(
                launch,
                status.clone(),
            )));
        }
        Primary {
            inbox,
            status,
            bus: None,
            file_manager1: None,
        }
    }

    /// Hands every event to `sink`: those queued since the claim at once, on this thread and in
    /// order, then each as it comes, on the bus thread. A second call replaces the sink.
    ///
    /// Events keep the order their calls arrived in. zbus runs each call as a task of its own,
    /// started in arrival order on the bus thread's single-threaded runtime, and every handler
    /// delivers its event before it first waits. Handlers can't run inline instead
    /// (`#[interface(spawn = false)]`): `CommandLine` waits for the app's answer, and inline it
    /// would hold every later call back until then.
    pub fn attach(&self, sink: impl Fn(Event) + Send + 'static) {
        self.inbox.attach(Box::new(sink));
    }

    /// Delivers every event on Rune's loop: each runs `on` as an [`rune::AppHandle::update`],
    /// in arrival order, also while no window shows. `on` may open windows there
    /// (`rune::open_window`).
    pub fn attach_rune<S: 'static>(
        &self,
        handle: rune::AppHandle<S>,
        on: fn(&mut S, Event, &mut rune::Tx),
    ) {
        self.attach(move |event| handle.update(move |s, tx| on(s, event, tx)));
    }

    /// Whether this process owns the app id on the bus (false when standalone, or released).
    pub fn is_claimed(&self) -> bool {
        self.bus
            .as_ref()
            .is_some_and(|b| b.stop.lock().is_ok_and(|s| s.is_some()))
    }

    /// The status this process's own [`CommandLine`] set, once it is dropped: exit with it.
    pub fn exit_status(&self) -> i32 {
        self.status.load(Ordering::Relaxed)
    }

    /// The primary's session bus connection, to serve more on it (a portal backend, say).
    /// Use it from futures run with [`Primary::spawn`]: it lives on the bus thread's runtime.
    /// None when standalone or released. Drop the clone with the work it serves: a clone kept
    /// past [`Primary::release`] keeps the connection open.
    pub fn connection(&self) -> Option<Connection> {
        self.bus
            .as_ref()
            .and_then(|b| b.conn.lock().ok().and_then(|c| c.clone()))
    }

    /// Runs `future` on the bus thread. False when standalone or released, and `future` is
    /// dropped.
    pub fn spawn(&self, future: impl Future<Output = ()> + Send + 'static) -> bool {
        match &self.bus {
            Some(bus) if self.is_claimed() => {
                bus.runtime.spawn(future);
                true
            }
            _ => false,
        }
    }

    /// What became of `org.freedesktop.FileManager1` at the claim, with
    /// `Options::file_manager1`; None without it, or standalone. A [`NameRequest::Queued`]
    /// primary takes the name later, when its owner lets go.
    pub fn file_manager1(&self) -> Option<&NameRequest> {
        self.file_manager1.as_ref()
    }

    /// Stops being the primary: gives the app id (and `org.freedesktop.FileManager1`) up,
    /// and returns once they are released, or after `Options::bus_timeout`. Call it when the
    /// app decides to quit, so a launch at that moment starts a new primary instead of
    /// reaching one about to exit. Launches still waiting for an answer fail, and try again.
    ///
    /// Forwarded launches the app has answered still get their answer before the connection
    /// goes; those it hasn't are told the primary left, and try again.
    ///
    /// From the bus thread itself (an [`Primary::attach`] sink), it asks and returns at once:
    /// the thread it would wait for is its own. From another thread while an attach sink holds
    /// the bus thread up, it waits until the sink returns, or `Options::bus_timeout`.
    pub fn release(&self) {
        let Some(bus) = &self.bus else { return };
        let Some(stop) = bus.stop.lock().ok().and_then(|mut s| s.take()) else {
            return;
        };
        if let Ok(mut conn) = bus.conn.lock() {
            conn.take();
        }
        let (done, released) = mpsc::channel();
        let on_bus_thread = std::thread::current().id() == bus.thread;
        if stop.send(done).is_ok() && !on_bus_thread {
            let _ = released.recv_timeout(bus.bus_timeout);
        }
    }
}

impl Drop for Primary {
    fn drop(&mut self) {
        self.release();
    }
}

pub(super) type Sink = Box<dyn Fn(Event) + Send>;

/// Events on their way to the app: queued until a sink is attached, then handed straight on.
#[derive(Default)]
pub(super) struct Inbox {
    state: Mutex<InboxState>,
}

#[derive(Default)]
struct InboxState {
    queued: Vec<Event>,
    sink: Option<Sink>,
}

impl std::fmt::Debug for Inbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Inbox")
    }
}

impl Inbox {
    pub(super) fn deliver(&self, event: Event) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        match &state.sink {
            Some(sink) => sink(event),
            None => state.queued.push(event),
        }
    }

    pub(super) fn attach(&self, sink: Sink) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        // Under the lock, so an event arriving now waits for the queued ones.
        for event in std::mem::take(&mut state.queued) {
            sink(event);
        }
        state.sink = Some(sink);
    }
}
