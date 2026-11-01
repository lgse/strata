// SPDX-License-Identifier: MIT

#[cfg(test)]
mod tests;

use std::{
    os::fd::OwnedFd,
    sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError},
    time::Duration,
};

use futures_channel::oneshot;
use rustix::{
    event::{EventfdFlags, PollFd, PollFlags, Timespec, eventfd, poll},
    fs::{Mode, OFlags, SeekFrom},
    io::Errno,
};

use super::{MountTable, mount_capabilities::forget_mount_capabilities};

const SETTLE: Duration = Duration::from_millis(50);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Wake {
    Changed,
    Asked,
}

pub(super) trait Source: Send + 'static {
    fn read(&mut self) -> Vec<u8>;
    fn wait(&mut self) -> Option<Wake>;
    fn settle(&mut self);
    fn waker(&self) -> Box<dyn Fn() + Send + Sync>;
}

struct State {
    generation: u64,
    text: Vec<u8>,
    table: MountTable,
    asked: Vec<oneshot::Sender<MountTable>>,
    stopped: bool,
}

pub(super) struct Watcher {
    state: Mutex<State>,
    wake: Box<dyn Fn() + Send + Sync>,
    published: fn(),
}

impl Watcher {
    pub(super) fn start(mut source: impl Source, published: fn()) -> Arc<Self> {
        let text = source.read();
        let watcher = Arc::new(Self {
            state: Mutex::new(State {
                generation: 1,
                table: MountTable::parse(&text),
                text,
                asked: Vec::new(),
                stopped: false,
            }),
            wake: source.waker(),
            published,
        });
        let watching = watcher.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("strata-mounts".to_owned())
            .spawn(move || watching.watch(source))
        {
            tracing::warn!(%error, "could not start the mount watcher");
            watcher.stop();
        }
        watcher
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn watch(&self, mut source: impl Source) {
        while let Some(wake) = source.wait() {
            if wake == Wake::Changed {
                source.settle();
            }
            let asked = std::mem::take(&mut self.state().asked);
            let text = source.read();
            let table = if self.state().text == text {
                self.latest()
            } else {
                let table = MountTable::parse(&text);
                {
                    let mut state = self.state();
                    state.generation += 1;
                    state.table = table.clone();
                    state.text = text;
                }
                (self.published)();
                table
            };
            for sender in asked {
                let _ = sender.send(table.clone());
            }
        }
        tracing::warn!("stopped watching the mount table");
        self.stop();
    }

    fn stop(&self) {
        let mut state = self.state();
        state.stopped = true;
        state.asked.clear();
    }

    pub(super) fn latest(&self) -> MountTable {
        self.state().table.clone()
    }

    fn snapshot(&self) -> (u64, MountTable) {
        let state = self.state();
        (state.generation, state.table.clone())
    }

    pub(super) async fn fresh(&self) -> MountTable {
        let receiver = {
            let mut state = self.state();
            if state.stopped {
                return state.table.clone();
            }
            let (sender, receiver) = oneshot::channel();
            state.asked.push(sender);
            receiver
        };
        (self.wake)();
        match receiver.await {
            Ok(table) => table,
            Err(_) => self.latest(),
        }
    }
}

struct Mountinfo {
    table: OwnedFd,
    asked: Arc<OwnedFd>,
}

impl Mountinfo {
    fn open() -> Result<Self, Errno> {
        Ok(Self {
            table: rustix::fs::open(
                "/proc/self/mountinfo",
                OFlags::RDONLY | OFlags::CLOEXEC,
                Mode::empty(),
            )?,
            asked: Arc::new(eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)?),
        })
    }

    fn read_table(&self) -> Result<Vec<u8>, Errno> {
        rustix::fs::seek(&self.table, SeekFrom::Start(0))?;
        let mut table = Vec::with_capacity(16 * 1024);
        let mut chunk = vec![0; 16 * 1024];
        loop {
            match rustix::io::read(&self.table, &mut chunk[..]) {
                Ok(0) => return Ok(table),
                Ok(read) => table.extend_from_slice(&chunk[..read]),
                Err(Errno::INTR) => {}
                Err(error) => return Err(error),
            }
        }
    }
}

impl Source for Mountinfo {
    fn read(&mut self) -> Vec<u8> {
        self.read_table().unwrap_or_else(|error| {
            tracing::warn!(%error, "could not read the mount table");
            Vec::new()
        })
    }

    fn wait(&mut self) -> Option<Wake> {
        loop {
            let mut fds = [
                PollFd::new(&self.table, PollFlags::PRI),
                PollFd::new(&*self.asked, PollFlags::IN),
            ];
            match poll(&mut fds, None) {
                Ok(_) => {}
                Err(Errno::INTR) => continue,
                Err(error) => {
                    tracing::warn!(%error, "could not wait for mount changes");
                    return None;
                }
            }
            let changed = fds[0].revents().intersects(PollFlags::PRI | PollFlags::ERR);
            if fds[1].revents().contains(PollFlags::IN) {
                let _ = rustix::io::read(&*self.asked, &mut [0; 8]);
                return Some(Wake::Asked);
            }
            if changed {
                return Some(Wake::Changed);
            }
        }
    }

    fn settle(&mut self) {
        std::thread::sleep(SETTLE);
        let mut fds = [PollFd::new(&self.table, PollFlags::PRI)];
        let _ = poll(
            &mut fds,
            Some(&Timespec {
                tv_sec: 0,
                tv_nsec: 0,
            }),
        );
    }

    fn waker(&self) -> Box<dyn Fn() + Send + Sync> {
        let asked = self.asked.clone();
        Box::new(move || {
            let _ = rustix::io::write(&*asked, &1_u64.to_ne_bytes());
        })
    }
}

static WATCHER: OnceLock<Option<Arc<Watcher>>> = OnceLock::new();

fn watcher() -> Option<&'static Watcher> {
    WATCHER
        .get_or_init(|| {
            #[cfg(not(test))]
            crate::util::ui_thread::assert_off();
            match Mountinfo::open() {
                Ok(source) => Some(Watcher::start(source, forget_mount_capabilities)),
                Err(error) => {
                    tracing::warn!(%error, "could not watch the mount table");
                    None
                }
            }
        })
        .as_deref()
}

pub(crate) fn start() {
    let _startup = crate::util::ui_thread::allow(crate::util::ui_thread::Exception::Startup);
    let _ = watcher();
}

pub(crate) fn latest() -> MountTable {
    #[cfg(test)]
    if let Some(table) = table_for_test() {
        return table;
    }
    watcher().map_or_else(|| MountTable::parse(b""), Watcher::latest)
}

pub(crate) fn snapshot() -> (u64, MountTable) {
    let (generation, table) =
        watcher().map_or_else(|| (0, MountTable::parse(b"")), Watcher::snapshot);
    #[cfg(test)]
    if let Some(table) = table_for_test() {
        return (generation, table);
    }
    (generation, table)
}

pub(crate) async fn fresh() -> MountTable {
    #[cfg(test)]
    if let Some(table) = table_for_test() {
        return table;
    }
    match watcher() {
        Some(watcher) => watcher.fresh().await,
        None => MountTable::parse(b""),
    }
}

#[cfg(test)]
static PUBLISHED_FOR_TEST: Mutex<Option<MountTable>> = Mutex::new(None);

#[cfg(test)]
pub(crate) fn publish_for_isolated_test(table: Option<MountTable>) {
    assert!(
        std::env::var_os("STRATA_ISOLATED_GTK_TEST").is_some(),
        "the published mount table override needs an isolated test process"
    );
    *PUBLISHED_FOR_TEST
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = table;
}

#[cfg(test)]
fn table_for_test() -> Option<MountTable> {
    PUBLISHED_FOR_TEST
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .or_else(|| super::mounts::mountinfo_for_test().map(MountTable::parse))
}
