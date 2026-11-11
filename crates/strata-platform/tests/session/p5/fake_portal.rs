//! A fake `org.freedesktop.portal.Settings`, served from the test process on the session's
//! private bus only: it connects to the address it is given (`Session::bus().address()`), never
//! to a bus from the environment. It answers `ReadAll`, `Read` and `ReadOne` from the values
//! it holds, emits `SettingChanged` when a test changes one, logs every call, and can delay
//! or hold back its `ReadAll` answers, as a slow portal does. A restarted portal is a second `FakePortal`:
//! another connection, under a new unique name.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use tokio::runtime::Handle;
use tokio::sync::oneshot;
use zbus::fdo::RequestNameFlags;
use zbus::message::Header;
use zbus::{Connection, OwnedVariant, Variant, fdo};

const NAME: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const INTERFACE: &str = "org.freedesktop.portal.Settings";

/// One method call the fake answered.
#[derive(Clone, Debug)]
pub struct Call {
    pub method: String,
    pub sender: String,
}

#[derive(Default)]
struct State {
    values: BTreeMap<(String, String), OwnedVariant>,
    calls: Vec<Call>,
    /// How long `ReadAll` waits before it answers (with the values as they are then).
    delay: Duration,
    /// What the next `ReadAll` does just after it answers.
    after_answer: Option<After>,
}

/// Something the fake does right after a `ReadAll` answer goes out: it is queued on the fake's
/// runtime as the answer is built, so it reaches the bus just after the answer.
enum After {
    /// Store this value and announce it.
    Change(String, String, OwnedVariant),
    /// Give up the portal's name.
    Leave,
}

struct Settings {
    state: Arc<Mutex<State>>,
    /// Open (true) unless a test holds `ReadAll` answers back.
    gate: tokio::sync::watch::Receiver<bool>,
}

fn owned(value: &Variant<'_>) -> OwnedVariant {
    value.try_to_owned().expect("a plain value")
}

fn not_found() -> fdo::Error {
    fdo::Error::Failed("no such key".into())
}

impl Settings {
    fn record(&self, method: &str, header: &Header<'_>) {
        let sender = header.sender().map(|s| s.to_string()).unwrap_or_default();
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .calls
            .push(Call {
                method: method.into(),
                sender,
            });
    }

    fn value(&self, namespace: &str, key: &str) -> Option<OwnedVariant> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let value = state.values.get(&(namespace.to_owned(), key.to_owned()))?;
        Some(value.try_clone().expect("a plain value"))
    }
}

#[zbus::interface(name = "org.freedesktop.portal.Settings")]
impl Settings {
    async fn read_all(
        &self,
        namespaces: Vec<String>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> HashMap<String, HashMap<String, OwnedVariant>> {
        self.record("ReadAll", &header);
        let delay = self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .delay;
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        let _ = self.gate.clone().wait_for(|open| *open).await;
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let mut reply: HashMap<String, HashMap<String, OwnedVariant>> = HashMap::new();
        for ((namespace, key), value) in &state.values {
            if namespaces.is_empty() || namespaces.contains(namespace) {
                reply
                    .entry(namespace.clone())
                    .or_default()
                    .insert(key.clone(), value.try_clone().expect("a plain value"));
            }
        }
        let connection = connection.clone();
        match state.after_answer.take() {
            Some(After::Change(namespace, key, value)) => {
                state.values.insert(
                    (namespace.clone(), key.clone()),
                    value.try_clone().expect("a plain value"),
                );
                tokio::spawn(async move {
                    let _ = connection
                        .emit_signal(
                            None::<&str>,
                            PATH,
                            INTERFACE,
                            "SettingChanged",
                            &(namespace, key, value),
                        )
                        .await;
                });
            }
            Some(After::Leave) => {
                tokio::spawn(async move {
                    let _ = connection.release_name(NAME).await;
                });
            }
            None => {}
        }
        reply
    }

    /// Wraps its answer in a second variant, as real portals do.
    async fn read(
        &self,
        namespace: String,
        key: String,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<OwnedVariant> {
        self.record("Read", &header);
        let value = self.value(&namespace, &key).ok_or_else(not_found)?;
        Ok(owned(&Variant::Variant(Box::new(Variant::from(value)))))
    }

    async fn read_one(
        &self,
        namespace: String,
        key: String,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<OwnedVariant> {
        self.record("ReadOne", &header);
        self.value(&namespace, &key).ok_or_else(not_found)
    }

    #[zbus(property)]
    async fn version(&self) -> u32 {
        2
    }
}

pub struct FakePortal {
    runtime: Handle,
    /// Dropped on the runtime's watch, before the runtime stops.
    connection: Option<Connection>,
    state: Arc<Mutex<State>>,
    gate: tokio::sync::watch::Sender<bool>,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl FakePortal {
    /// Serves `values` on the bus at `address`, owning the portal's name unless `own` is false.
    pub fn start(address: &str, values: &[(&str, &str, Variant<'static>)], own: bool) -> Self {
        let state = Arc::new(Mutex::new(State::default()));
        for (namespace, key, value) in values {
            state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .values
                .insert(((*namespace).into(), (*key).into()), owned(value));
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("the fake portal's runtime");
        let handle = runtime.handle().clone();
        let (stop, stopped) = oneshot::channel::<()>();
        // The runtime's own thread drives its I/O; the test thread hands it work through
        // `Handle::block_on`.
        let thread = std::thread::Builder::new()
            .name("fake-settings-portal".into())
            .spawn(move || {
                let _ = runtime.block_on(stopped);
            })
            .expect("the fake portal's thread");
        let (gate, open) = tokio::sync::watch::channel(true);
        let settings = Settings {
            state: Arc::clone(&state),
            gate: open,
        };
        let connection = handle.block_on(async {
            zbus::connection::Builder::address(address)
                .serve_at(PATH, settings)
                .build()
                .await
                .expect("connecting the fake portal to the private bus")
        });
        let fake = FakePortal {
            runtime: handle,
            connection: Some(connection),
            state,
            gate,
            stop: Some(stop),
            thread: Some(thread),
        };
        if own {
            fake.acquire();
        }
        fake
    }

    /// Takes the portal's name.
    fn acquire(&self) {
        self.runtime
            .block_on(self.connection().request_name(NAME))
            .expect("owning the portal's name");
    }

    /// Takes the portal's name so that another portal may take it over, taking it over from
    /// the current owner if there is one. The replaced portal stays on the bus and can still
    /// answer a read it was asked.
    pub fn take_over(&self) {
        let flags = RequestNameFlags::ALLOW_REPLACEMENT
            | RequestNameFlags::REPLACE_EXISTING
            | RequestNameFlags::DO_NOT_QUEUE;
        self.runtime
            .block_on(self.connection().request_name_with_flags(NAME, flags))
            .expect("taking over the portal's name");
    }

    /// Right after the next `ReadAll` answer, stores and announces this value.
    pub fn change_after_answer(&self, namespace: &str, key: &str, value: Variant<'static>) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .after_answer = Some(After::Change(namespace.into(), key.into(), owned(&value)));
    }

    /// Right after the next `ReadAll` answer, gives up the portal's name.
    pub fn leave_after_answer(&self) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .after_answer = Some(After::Leave);
    }

    /// Sets a value and announces it with `SettingChanged`.
    pub fn set(&self, namespace: &str, key: &str, value: Variant<'static>) {
        self.set_quietly(namespace, key, value.try_clone().expect("a plain value"));
        self.emit(namespace, key, value);
    }

    /// Sets a value without a signal: the next answer holds it, no change announces it.
    pub fn set_quietly(&self, namespace: &str, key: &str, value: Variant<'static>) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values
            .insert((namespace.into(), key.into()), owned(&value));
    }

    /// Emits `SettingChanged` without storing the value: noise and bad values.
    pub fn emit(&self, namespace: &str, key: &str, value: Variant<'static>) {
        self.runtime
            .block_on(self.connection().emit_signal(
                None::<&str>,
                PATH,
                INTERFACE,
                "SettingChanged",
                &(namespace, key, value),
            ))
            .expect("emitting SettingChanged");
    }

    fn connection(&self) -> &Connection {
        self.connection
            .as_ref()
            .expect("the fake portal's connection")
    }

    /// Makes each later `ReadAll` wait `delay` before it answers. Signals still go out
    /// meanwhile.
    pub fn set_delay(&self, delay: Duration) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .delay = delay;
    }

    /// Holds every `ReadAll` answer back until [`FakePortal::answer`].
    pub fn hold(&self) {
        self.gate.send_replace(false);
    }

    /// Lets held and later `ReadAll` calls answer.
    pub fn answer(&self) {
        self.gate.send_replace(true);
    }

    /// Every call received so far.
    pub fn calls(&self) -> Vec<Call> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .calls
            .clone()
    }
}

impl Drop for FakePortal {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.take() {
            let _ = self.runtime.block_on(connection.close());
        }
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
