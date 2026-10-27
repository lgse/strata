// SPDX-License-Identifier: MIT

use std::{
    collections::{HashMap, VecDeque},
    fs, io,
    panic::AssertUnwindSafe,
    path::{Path, PathBuf},
    sync::{Condvar, Mutex, MutexGuard, OnceLock, PoisonError},
    time::{Duration, Instant},
};

use futures_channel::oneshot;

use super::{FileStamp, file_stamp};
use crate::util::ui_thread;

const EXIT_FLUSH_WAIT: Duration = Duration::from_secs(10);

type Job = Box<dyn FnOnce() + Send>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Policy {
    State,
    Config,
}

type Produce = Box<dyn FnOnce() -> io::Result<Vec<u8>> + Send>;

/// Also returns a lock the writer holds until the file has been replaced.
type ProduceLocked = Box<dyn FnOnce() -> io::Result<(Vec<u8>, Option<fs::File>)> + Send>;

enum Contents {
    Bytes(Vec<u8>),
    Produced(Produce),
    ProducedLocked(ProduceLocked),
    Removed,
}

struct Save {
    contents: Contents,
    policy: Policy,
    failure: Option<&'static str>,
    done: Vec<oneshot::Sender<io::Result<()>>>,
}

enum Task {
    Run(Job),
    Save(PathBuf),
}

#[derive(Default)]
struct Queue {
    tasks: VecDeque<Task>,
    saves: HashMap<PathBuf, Save>,
    running: bool,
    submitted: u64,
}

pub(crate) struct Writer {
    queue: Mutex<Queue>,
    wake: Condvar,
    idle: Condvar,
    known: Mutex<HashMap<PathBuf, FileStamp>>,
}

impl Writer {
    pub(crate) fn start(name: &str) -> &'static Self {
        let writer: &'static Self = Box::leak(Box::new(Self {
            queue: Mutex::default(),
            wake: Condvar::new(),
            idle: Condvar::new(),
            known: Mutex::default(),
        }));
        std::thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || writer.work())
            .expect("settings writer thread");
        writer
    }

    fn lock(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn work(&self) {
        let mut queue = self.lock();
        loop {
            let task = match queue.tasks.pop_front() {
                Some(Task::Run(job)) => job,
                Some(Task::Save(path)) => {
                    let Some(save) = queue.saves.remove(&path) else {
                        continue;
                    };
                    Box::new(move || self.write(&path, save)) as Box<dyn FnOnce() + Send + '_>
                }
                None => {
                    self.idle.notify_all();
                    queue = self
                        .wake
                        .wait(queue)
                        .unwrap_or_else(PoisonError::into_inner);
                    continue;
                }
            };
            queue.running = true;
            drop(queue);
            if std::panic::catch_unwind(AssertUnwindSafe(task)).is_err() {
                tracing::error!("a settings file job panicked");
            }
            queue = self.lock();
            queue.running = false;
        }
    }

    fn write(&self, path: &Path, save: Save) {
        let mut _lock = None;
        let contents = match save.contents {
            Contents::Bytes(contents) => Ok(Some(contents)),
            Contents::Produced(produce) => produce().map(Some),
            Contents::ProducedLocked(produce) => produce().map(|(contents, lock)| {
                _lock = lock;
                Some(contents)
            }),
            Contents::Removed => Ok(None),
        };
        let result = contents.and_then(|contents| match contents {
            Some(contents) => path
                .parent()
                .map_or(Ok(()), fs::create_dir_all)
                .and_then(|()| match save.policy {
                    Policy::State => super::atomic_write(path, &contents),
                    Policy::Config => super::atomic_write_config(path, &contents),
                }),
            None if !path.is_file() => Ok(()),
            None => fs::remove_file(path),
        });
        if result.is_ok() {
            self.note(path, file_stamp(path));
        }
        if let (Err(error), Some(failure)) = (&result, save.failure) {
            tracing::warn!(%error, path = %path.display(), "{failure}");
        }
        let mut done = save.done;
        let newest = done.pop();
        for older in done {
            let _ = older.send(
                result
                    .as_ref()
                    .copied()
                    .map_err(|error| io::Error::new(error.kind(), error.to_string())),
            );
        }
        if let Some(newest) = newest {
            let _ = newest.send(result);
        }
    }

    fn known(&self) -> MutexGuard<'_, HashMap<PathBuf, FileStamp>> {
        self.known.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn note(&self, path: &Path, stamp: Option<FileStamp>) {
        match stamp {
            Some(stamp) => self.known().insert(path.to_path_buf(), stamp),
            None => self.known().remove(path),
        };
    }

    fn changed_elsewhere(&self, path: &Path) -> Option<FileStamp> {
        file_stamp(path).filter(|stamp| self.known().get(path) != Some(stamp))
    }

    fn reread<T>(
        &self,
        path: &Path,
        parse: impl FnOnce(&str) -> io::Result<T>,
    ) -> io::Result<Option<T>> {
        let Some(stamp) = self.changed_elsewhere(path) else {
            return Ok(None);
        };
        let contents = match fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let parsed = parse(&contents)?;
        self.note(path, Some(stamp));
        Ok(Some(parsed))
    }

    fn submit(&self, task: Task) {
        let mut queue = self.lock();
        queue.tasks.push_back(task);
        queue.submitted += 1;
        drop(queue);
        self.wake.notify_one();
    }

    pub(crate) fn run<T: Send + 'static>(
        &self,
        job: impl FnOnce() -> T + Send + 'static,
    ) -> oneshot::Receiver<T> {
        let (send, receive) = oneshot::channel();
        self.submit(Task::Run(Box::new(move || {
            let _ = send.send(job());
        })));
        receive
    }

    fn save(
        &self,
        path: PathBuf,
        contents: Contents,
        policy: Policy,
        failure: Option<&'static str>,
    ) -> oneshot::Receiver<io::Result<()>> {
        let (send, receive) = oneshot::channel();
        let mut queue = self.lock();
        if let Some(waiting) = queue.saves.get_mut(&path) {
            waiting.contents = contents;
            waiting.policy = policy;
            waiting.failure = failure;
            waiting.done.push(send);
            return receive;
        }
        queue.saves.insert(
            path.clone(),
            Save {
                contents,
                policy,
                failure,
                done: vec![send],
            },
        );
        drop(queue);
        self.submit(Task::Save(path));
        receive
    }

    pub(crate) fn flush(&self, wait: Duration) -> bool {
        let deadline = Instant::now() + wait;
        let mut queue = self.lock();
        while queue.running || !queue.tasks.is_empty() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            queue = self
                .idle
                .wait_timeout(queue, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        true
    }

    #[cfg(test)]
    fn submitted(&self) -> u64 {
        self.lock().submitted
    }

    #[cfg(test)]
    pub(crate) fn hold(&self) -> Held {
        let (started, wait_started) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel::<()>();
        drop(self.run(move || {
            let _ = started.send(());
            let _ = released.recv();
        }));
        wait_started.recv().expect("held writer");
        Held(release)
    }
}

#[cfg(test)]
pub(crate) struct Held(
    #[expect(dead_code, reason = "dropping it releases the writer")] std::sync::mpsc::Sender<()>,
);

fn shared() -> &'static Writer {
    static SHARED: OnceLock<&'static Writer> = OnceLock::new();
    SHARED.get_or_init(|| Writer::start("strata-settings"))
}

pub(crate) fn run<T: Send + 'static>(
    job: impl FnOnce() -> T + Send + 'static,
) -> oneshot::Receiver<T> {
    shared().run(job)
}

pub(crate) fn save(
    path: PathBuf,
    contents: Option<Vec<u8>>,
    policy: Policy,
    failure: Option<&'static str>,
) -> oneshot::Receiver<io::Result<()>> {
    let contents = contents.map_or(Contents::Removed, Contents::Bytes);
    shared().save(path, contents, policy, failure)
}

pub(crate) fn save_produced(
    path: PathBuf,
    policy: Policy,
    failure: &'static str,
    produce: impl FnOnce() -> io::Result<Vec<u8>> + Send + 'static,
) {
    let contents = Contents::Produced(Box::new(produce));
    drop(shared().save(path, contents, policy, Some(failure)));
}

/// Like [`save`], with contents made on the writer thread together with a lock that
/// stays held until the file is replaced.
pub(crate) fn save_locked(
    path: PathBuf,
    policy: Policy,
    produce: impl FnOnce() -> io::Result<(Vec<u8>, Option<fs::File>)> + Send + 'static,
) -> oneshot::Receiver<io::Result<()>> {
    shared().save(
        path,
        Contents::ProducedLocked(Box::new(produce)),
        policy,
        None,
    )
}

pub(crate) fn reread<T: Send + 'static>(
    path: PathBuf,
    parse: impl FnOnce(&str) -> io::Result<T> + Send + 'static,
) -> oneshot::Receiver<io::Result<Option<T>>> {
    let writer = shared();
    writer.run(move || writer.reread(&path, parse))
}

pub(crate) fn note(path: &Path, stamp: Option<FileStamp>) {
    shared().note(path, stamp);
}

pub(crate) fn flush(wait: Duration) -> bool {
    ui_thread::assert_off();
    shared().flush(wait)
}

pub(crate) fn flush_at_exit() {
    let _exit = ui_thread::allow(ui_thread::Exception::ExitFlush);
    if !flush(EXIT_FLUSH_WAIT) {
        tracing::warn!("quitting before Strata's own files finished saving");
    }
}

#[cfg(test)]
pub(crate) fn hold() -> Held {
    shared().hold()
}

#[cfg(test)]
pub(crate) fn settle() {
    let context = gtk::glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let submitted = shared().submitted();
        {
            let _allowed = ui_thread::allow(ui_thread::Exception::TestSetup);
            assert!(
                flush(deadline.saturating_duration_since(Instant::now())),
                "the settings writer did not finish"
            );
        }
        for _ in 0..100 {
            if !context.iteration(false) {
                break;
            }
        }
        if shared().submitted() == submitted && shared().flush(Duration::ZERO) {
            return;
        }
        assert!(Instant::now() < deadline, "the settings writer kept busy");
    }
}

#[cfg(test)]
mod tests;
