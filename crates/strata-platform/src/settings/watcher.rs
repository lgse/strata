//! The watcher thread: a tokio current-thread runtime with one zbus session connection, as
//! Rune's own `color-scheme` reader runs (`platform/portal_appearance.rs`).

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::pin::pin;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use futures_util::future::select;
use futures_util::stream::{self, BoxStream, StreamExt};
use tokio::sync::oneshot;
use zbus::OwnedVariant;
use zbus::message::Sequence;
use zbus::proxy::{Builder, CacheProperties};
use zbus::{Connection, Message, Proxy};

use super::fallback::{Fallback, Reported};
use super::keys::{Key, NAMESPACES, Setting};
use super::parse::{self, Applied, Reply};
use super::snapshot::DesktopSettings;

const DESTINATION: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const INTERFACE: &str = "org.freedesktop.portal.Settings";

/// The watcher thread's name: 15 bytes, so `/proc` shows it whole.
pub const THREAD_NAME: &str = "settings-portal";

/// How long dropping a [`Watcher`] waits for its thread to stop before leaving it to finish
/// alone, so a drop never holds the UI thread for long.
const STOP_WAIT: Duration = Duration::from_millis(200);

/// What the watcher reports, in order: one [`Event::Snapshot`] first, then changes and
/// fallbacks as they happen.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Event {
    /// The first answer: the portal's values, or the defaults once the portal or the bus is
    /// known to be missing (a [`Event::Fallback`] then says why). Sent exactly once.
    Snapshot(DesktopSettings),
    /// One key changed, or became known: its new value, and every setting just after it.
    Changed {
        setting: Setting,
        settings: DesktopSettings,
    },
    /// A fallback, the first time its kind (and key) happens.
    Fallback(Fallback),
}

/// How the watcher waits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Options {
    /// How long [`Watcher::start`] waits for the first answer, so the app's first frame can use
    /// it: 100 ms by default, as Rune waits for `color-scheme`. A later answer arrives as
    /// changes. Zero never waits.
    pub first_answer: Duration,
    /// How long connecting may take, and how long a read waits for the portal's answer
    /// (activating it included) before the defaults go out with [`Fallback::NoPortal`]: 10 s by
    /// default. The read stays out after that, and a late answer arrives as changes.
    pub timeout: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            first_answer: Duration::from_millis(100),
            timeout: Duration::from_secs(10),
        }
    }
}

impl Options {
    /// These options with another first-answer wait.
    pub fn first_answer(self, first_answer: Duration) -> Self {
        Options {
            first_answer,
            ..self
        }
    }

    /// These options with another bound on connecting, activating and reading.
    pub fn timeout(self, timeout: Duration) -> Self {
        Options { timeout, ..self }
    }
}

/// Reads the desktop's settings and follows their changes on a thread of its own
/// ([`THREAD_NAME`]). Dropping it stops the thread.
pub struct Watcher {
    latest: Arc<Mutex<DesktopSettings>>,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
    /// Disconnects when the thread ends.
    ended: mpsc::Receiver<()>,
}

impl Watcher {
    /// Starts the watcher. `sink` hears every [`Event`] on the watcher's thread, in order;
    /// [`follow`](super::follow) is the sink that moves them onto Rune's loop. Waits up to
    /// [`Options::first_answer`] for the first answer, and fails only if the thread can't be
    /// spawned.
    pub fn start(
        options: Options,
        sink: impl Fn(Event) + Send + Sync + 'static,
    ) -> io::Result<Watcher> {
        let latest = Arc::new(Mutex::new(DesktopSettings::default()));
        let (stop, stopped) = oneshot::channel();
        let (answered, first) = mpsc::sync_channel(1);
        let (ended_tx, ended) = mpsc::channel::<()>();
        let mut feed = Feed {
            latest: Arc::clone(&latest),
            sink: Box::new(sink),
            settings: DesktopSettings::default(),
            reported: Reported::default(),
            answered: Some(answered),
            stamps: [None; Key::ALL.len()],
        };
        let thread = std::thread::Builder::new()
            .name(THREAD_NAME.into())
            .spawn(move || {
                let _ended = ended_tx;
                match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime.block_on(async {
                        // Stopping drops the watch wherever it waits.
                        let _ = select(pin!(watch(options, &mut feed)), stopped).await;
                    }),
                    Err(error) => feed.missing(Fallback::NoPortal(format!(
                        "the watcher couldn't start its runtime: {error}"
                    ))),
                }
            })?;
        if !options.first_answer.is_zero() {
            let _ = first.recv_timeout(options.first_answer);
        }
        Ok(Watcher {
            latest,
            stop: Some(stop),
            thread: Some(thread),
            ended,
        })
    }

    /// The latest settings: the defaults until the first answer. An app can seed its state
    /// with it right after [`Watcher::start`].
    pub fn snapshot(&self) -> DesktopSettings {
        *self.latest.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        // Join only a thread that has ended. One still busy after STOP_WAIT is detached: it
        // stops at its next await, and dropping never hangs the app on it.
        if let Err(RecvTimeoutError::Disconnected) = self.ended.recv_timeout(STOP_WAIT)
            && let Some(thread) = self.thread.take()
        {
            let _ = thread.join();
        }
    }
}

/// The watcher's side: the settings as it last reported them, and where reports go.
struct Feed {
    latest: Arc<Mutex<DesktopSettings>>,
    sink: Box<dyn Fn(Event) + Send + Sync>,
    settings: DesktopSettings,
    reported: Reported,
    /// Taken by the snapshot, which ends `start`'s wait.
    answered: Option<mpsc::SyncSender<()>>,
    /// Per key, the receive position of the message that last set it. Messages on one
    /// connection are ordered by it, so an older message never overwrites a newer one,
    /// whichever order the watcher's loop happens to take them in.
    stamps: [Option<Sequence>; Key::ALL.len()],
}

impl Feed {
    fn store(&self) {
        *self.latest.lock().unwrap_or_else(PoisonError::into_inner) = self.settings;
    }

    fn snapshot(&mut self) {
        self.store();
        (self.sink)(Event::Snapshot(self.settings));
        if let Some(answered) = self.answered.take() {
            let _ = answered.try_send(());
        }
    }

    fn fallback(&mut self, fallback: Fallback) {
        if self.reported.first(&fallback) {
            (self.sink)(Event::Fallback(fallback));
        }
    }

    /// The bus or the portal is missing: the snapshot, if none went out yet, then why.
    fn missing(&mut self, why: Fallback) {
        if self.answered.is_some() {
            self.snapshot();
        }
        self.fallback(why);
    }

    /// Whether a message received at `at` is newer than whatever last set `key`.
    fn fresh(&self, key: Key, at: Sequence) -> bool {
        self.stamps[key as usize].is_none_or(|stamp| stamp < at)
    }

    /// Applies a `ReadAll` reply to every key no newer change has set. Returns the unique name
    /// it came from, or None when it isn't one (reported as [`Fallback::NoPortal`]).
    fn reply(&mut self, reply: &Message) -> Option<String> {
        let Ok(values) = reply.body().deserialize::<Reply>() else {
            self.missing(Fallback::NoPortal(
                "the Settings portal's ReadAll answer isn't a{sa{sv}}".into(),
            ));
            return None;
        };
        let at = reply.recv_position();
        let fresh: Vec<Key> = Key::ALL
            .into_iter()
            .filter(|k| self.fresh(*k, at))
            .collect();
        let applied = parse::apply_reply(&mut self.settings, &values, |k| fresh.contains(&k));
        for key in fresh {
            self.stamps[key as usize] = Some(at);
        }
        self.report(applied);
        reply.header().sender().map(|name| name.to_string())
    }

    /// Applies one change, unless a newer message already set its key.
    fn change(&mut self, change: Change) {
        if !self.fresh(change.key, change.at) {
            return;
        }
        self.stamps[change.key as usize] = Some(change.at);
        let applied = parse::apply_change(
            &mut self.settings,
            change.key.namespace(),
            change.key.name(),
            &change.value,
        );
        self.report(applied);
    }

    /// Reports what a reply or a change did. Before the snapshot, the changes are folded into
    /// it; after, each is its own event.
    fn report(&mut self, applied: Applied) {
        if self.answered.is_some() {
            self.snapshot();
        } else if !applied.changes.is_empty() {
            self.store();
            for (setting, settings) in applied.changes {
                (self.sink)(Event::Changed { setting, settings });
            }
        }
        for fallback in applied.fallbacks {
            self.fallback(fallback);
        }
    }
}

/// A `SettingChanged` for a key the module reads, with where it was received.
struct Change {
    key: Key,
    value: OwnedVariant,
    at: Sequence,
}

impl Change {
    /// The change `message` carries, or None for a key the module doesn't read.
    fn of(message: &Message) -> Option<Change> {
        let (namespace, name, value) = message
            .body()
            .deserialize::<(String, String, OwnedVariant)>()
            .ok()?;
        Some(Change {
            key: Key::lookup(&namespace, &name)?,
            value,
            at: message.recv_position(),
        })
    }
}

enum Input {
    Changed(Option<Change>),
    Owner(Option<String>),
}

/// A `ReadAll` in flight.
type ReadAll = Pin<Box<dyn Future<Output = zbus::Result<Message>> + Send>>;

fn read_all(portal: &Proxy<'static>) -> ReadAll {
    let portal = portal.clone();
    Box::pin(async move { portal.call_method("ReadAll", &(&NAMESPACES[..],)).await })
}

/// `f`'s output, or never when there is none: a `select!` branch that is off.
async fn maybe<F: Future + Unpin>(f: &mut Option<F>) -> F::Output {
    match f {
        Some(f) => f.await,
        None => std::future::pending().await,
    }
}

async fn watch(options: Options, feed: &mut Feed) {
    let connection = match tokio::time::timeout(options.timeout, Connection::session()).await {
        Ok(Ok(connection)) => connection,
        Ok(Err(error)) => return feed.missing(Fallback::NoBus(error.to_string())),
        Err(_) => {
            return feed.missing(Fallback::NoBus(
                "timed out connecting to the session bus".into(),
            ));
        }
    };
    let subscribed = tokio::time::timeout(options.timeout, subscribe(&connection)).await;
    let (portal, mut inputs) = match subscribed {
        Ok(Ok(subscribed)) => subscribed,
        Ok(Err(error)) => return feed.missing(Fallback::NoPortal(error.to_string())),
        Err(_) => {
            return feed.missing(Fallback::NoPortal(
                "timed out subscribing to the Settings portal".into(),
            ));
        }
    };
    // Subscribed before the first read, so no change between the two is lost. The loop keeps
    // draining the changes while a read is out: zbus queues a bounded number per match rule and
    // stops reading the socket when the queue is full, and the reply waits behind them.
    let new_deadline = || Some(Box::pin(tokio::time::sleep(options.timeout)));
    let mut read = Some(read_all(&portal));
    let mut deadline = new_deadline();
    // Changes that arrive while a read is out, before its timeout, the latest per key: they
    // apply after the reply, and only where they are newer than it.
    let mut held: [Option<Change>; Key::ALL.len()] = Default::default();
    let release = |held: &mut [Option<Change>; Key::ALL.len()], feed: &mut Feed| {
        let mut changes: Vec<Change> = held.iter_mut().filter_map(Option::take).collect();
        changes.sort_by_key(|c| c.at);
        for change in changes {
            feed.change(change);
        }
    };
    // The unique name the last reply came from.
    let mut owner: Option<String> = None;
    // A new owner seen while a read was out: read again after it, unless the reply came from it.
    let mut reread_for: Option<String> = None;
    loop {
        // Biased, the reply first: zbus completes a reply before it hands the messages received
        // after it to their streams, so taking it first keeps the bus's order where both are
        // ready (a portal that answers and then leaves is lost after its answer). The stamps keep
        // a reply from overwriting a newer change either way.
        tokio::select! {
            biased;
            reply = maybe(&mut read) => {
                read = None;
                deadline = None;
                owner = match reply {
                    Ok(reply) => feed.reply(&reply),
                    Err(error) => {
                        feed.missing(Fallback::NoPortal(error.to_string()));
                        None
                    }
                };
                release(&mut held, feed);
                if let Some(name) = reread_for.take()
                    && owner.as_deref() != Some(name.as_str())
                {
                    read = Some(read_all(&portal));
                    deadline = new_deadline();
                }
            }
            () = maybe(&mut deadline) => {
                // A slow portal: the defaults go out now, and the read stays out, so its reply
                // still arrives as changes. Changes apply as they come from here on.
                deadline = None;
                feed.missing(Fallback::NoPortal("timed out reading the Settings portal".into()));
                release(&mut held, feed);
            }
            input = inputs.next() => match input {
                None => {
                    feed.missing(Fallback::NoBus("the session bus connection closed".into()));
                    break;
                }
                Some(Input::Changed(None)) => {}
                Some(Input::Changed(Some(change))) if deadline.is_some() => {
                    let slot = change.key as usize;
                    held[slot] = Some(change);
                }
                Some(Input::Changed(Some(change))) => feed.change(change),
                Some(Input::Owner(Some(name))) => {
                    if read.is_some() && deadline.is_none() {
                        // A new portal replaces one that never answered, and is the one to read.
                        read = Some(read_all(&portal));
                        deadline = new_deadline();
                        reread_for = None;
                    } else if read.is_some() {
                        reread_for = Some(name);
                    } else if owner.as_deref() != Some(name.as_str()) {
                        read = Some(read_all(&portal));
                        deadline = new_deadline();
                    }
                }
                Some(Input::Owner(None)) => {
                    if owner.take().is_some() {
                        feed.fallback(Fallback::PortalLost);
                    }
                }
            }
        }
    }
}

/// The portal proxy, and one stream of its changes and its owner changes. All of
/// `SettingChanged` comes through one match rule: a rule per namespace would let the merged
/// streams reorder changes across namespaces, which the bus delivers in order. Other
/// namespaces' changes wake the thread and are dropped by key.
async fn subscribe(
    connection: &Connection,
) -> zbus::Result<(Proxy<'static>, BoxStream<'static, Input>)> {
    let portal: Proxy<'static> = Builder::new(connection)
        .destination(DESTINATION)
        .path(PATH)
        .interface(INTERFACE)
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    let changes = portal
        .receive_signal("SettingChanged")
        .await?
        .map(|message| Input::Changed(Change::of(&message)));
    let owners = portal
        .receive_owner_changed()
        .await?
        .map(|owner| Input::Owner(owner.map(|name| name.to_string())));
    Ok((portal, stream::select(changes, owners).boxed()))
}
