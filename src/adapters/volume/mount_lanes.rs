// SPDX-License-Identifier: MIT

#[cfg(test)]
mod tests;

use std::{
    any::Any,
    cell::{Cell, RefCell},
    collections::HashMap,
    future::Future,
    io,
    path::{Path, PathBuf},
    pin::Pin,
    rc::{Rc, Weak},
    sync::{
        Arc, LazyLock, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use futures_channel::oneshot;
use gtk::{gio, glib, prelude::*};
use rustix::io::Errno;

use super::{MountTable, mount_watch};

pub(crate) const STALLED_AFTER: Duration = Duration::from_secs(2);
pub(crate) const NOT_RESPONDING_AFTER: Duration = Duration::from_secs(10);
const BEAT_EVERY: Duration = Duration::from_millis(100);
const LEARNED_PATHS: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Class {
    Interactive,
    Background,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Unanswered {
    TimedOut,
    Skipped,
    Failed,
}

pub(crate) use crate::services::Responsiveness;

pub(crate) trait Outcome {
    fn made_progress(&self) -> bool;
}

fn answered(error: Errno) -> bool {
    matches!(
        error,
        Errno::NOENT | Errno::NOTDIR | Errno::STALE | Errno::ACCESS | Errno::PERM
    )
}

impl<T> Outcome for Result<T, Errno> {
    fn made_progress(&self) -> bool {
        self.as_ref().err().is_none_or(|error| answered(*error))
    }
}

impl<T> Outcome for io::Result<T> {
    fn made_progress(&self) -> bool {
        self.as_ref()
            .err()
            .is_none_or(|error| Errno::from_io_error(error).is_some_and(answered))
    }
}

impl<T: Outcome> Outcome for Result<T, gio::Cancelled> {
    fn made_progress(&self) -> bool {
        self.as_ref().is_ok_and(Outcome::made_progress)
    }
}

impl<T> Outcome for Result<T, glib::Error> {
    fn made_progress(&self) -> bool {
        self.as_ref().err().is_none_or(|error| {
            [
                gio::IOErrorEnum::NotFound,
                gio::IOErrorEnum::NotDirectory,
                gio::IOErrorEnum::PermissionDenied,
            ]
            .into_iter()
            .any(|code| error.matches(code))
        })
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum LaneKey {
    Mount(u64),
    Path { mount: Option<u64>, prefix: PathBuf },
    Remote(String),
}

struct Target<'a> {
    key: LaneKey,
    path: Option<&'a Path>,
    lexical: Option<u64>,
}

impl LaneKey {
    fn covers(&self, target: &Target<'_>) -> bool {
        *self == target.key
            || match self {
                Self::Mount(id) => target.lexical == Some(*id),
                Self::Path { mount, prefix } => {
                    *mount == target.lexical
                        && target.path.is_some_and(|path| path.starts_with(prefix))
                }
                Self::Remote(_) => false,
            }
    }

    fn on(&self, gone: impl Fn(u64) -> bool) -> bool {
        match self {
            Self::Mount(id)
            | Self::Path {
                mount: Some(id), ..
            } => gone(*id),
            Self::Path { mount: None, .. } | Self::Remote(_) => false,
        }
    }
}

struct Ticket {
    key: LaneKey,
    reaching: Option<LaneKey>,
    progress_at: Instant,
}

impl Ticket {
    fn holds(&self) -> &LaneKey {
        self.reaching.as_ref().unwrap_or(&self.key)
    }

    fn quiet(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.progress_at)
    }
}

type JoinKey = (PathBuf, String);

struct Lanes {
    tickets: HashMap<u64, Ticket>,
    stalls: HashMap<LaneKey, Instant>,
    serials: u64,
    learned: HashMap<PathBuf, u64>,
    generation: u64,
    mounts: MountTable,
    jobs: HashMap<JoinKey, Job>,
    watching: bool,
}

impl Default for Lanes {
    fn default() -> Self {
        Self {
            tickets: HashMap::new(),
            stalls: HashMap::new(),
            serials: 0,
            learned: HashMap::new(),
            generation: 0,
            mounts: MountTable::parse(b""),
            jobs: HashMap::new(),
            watching: false,
        }
    }
}

impl Lanes {
    fn serial(&mut self) -> u64 {
        self.serials += 1;
        self.serials
    }

    fn follow(&mut self, generation: u64, mounts: &MountTable) -> Vec<Job> {
        if generation <= self.generation {
            return Vec::new();
        }
        let before = std::mem::replace(&mut self.mounts, mounts.clone());
        self.generation = generation;
        let gone = |id: u64| before.mount_of(id).is_some() && mounts.mount_of(id).is_none();
        let arrived: Vec<&Path> = mounts
            .points()
            .filter(|(id, _)| before.mount_of(*id).is_none())
            .map(|(_, point)| point)
            .collect();
        self.learned
            .retain(|path, id| !gone(*id) && !arrived.iter().any(|point| path.starts_with(point)));
        self.stalls.retain(|key, _| !key.on(gone));
        let dropped: Vec<u64> = self
            .tickets
            .extract_if(|_, ticket| {
                ticket.key.on(gone) || ticket.reaching.as_ref().is_some_and(|key| key.on(gone))
            })
            .map(|(ticket, _)| ticket)
            .collect();
        self.jobs
            .extract_if(|_, job| dropped.contains(&job.ticket))
            .map(|(_, job)| job)
            .collect()
    }

    fn key_for(&self, path: &Path, lexical: Option<u64>) -> LaneKey {
        match self.learned.get(path) {
            Some(&id) => LaneKey::Mount(id),
            None => LaneKey::Path {
                mount: lexical,
                prefix: path.to_owned(),
            },
        }
    }

    fn target<'a>(&self, path: &'a Path, mounts: &MountTable) -> Target<'a> {
        let lexical = mounts.mount_for(path).map(|mount| mount.mount_id);
        Target {
            key: self.key_for(path, lexical),
            path: Some(path),
            lexical,
        }
    }

    fn verdict(
        &self,
        target: &Target<'_>,
        class: Class,
        now: Instant,
        asking: Option<u64>,
    ) -> Result<Option<LaneKey>, Unanswered> {
        let others = || {
            self.tickets
                .iter()
                .filter(move |(ticket, _)| Some(**ticket) != asking)
                .map(|(_, ticket)| ticket)
        };
        let mut stalls = self
            .stalls
            .iter()
            .filter(|(key, _)| key.covers(target))
            .peekable();
        if matches!(class, Class::Background) {
            let busy =
                others().any(|ticket| ticket.key.covers(target) || ticket.holds().covers(target));
            return match busy || stalls.peek().is_some() {
                true => Err(Unanswered::Skipped),
                false => Ok(None),
            };
        }
        if others()
            .any(|ticket| ticket.holds().covers(target) && ticket.quiet(now) >= STALLED_AFTER)
        {
            return Err(Unanswered::TimedOut);
        }
        let mut probe: Option<(&LaneKey, Instant)> = None;
        for (key, since) in stalls {
            if others().any(|ticket| ticket.holds() == key) {
                return Err(Unanswered::TimedOut);
            }
            if probe.is_none_or(|(_, oldest)| *since < oldest) {
                probe = Some((key, *since));
            }
        }
        Ok(probe.map(|(key, _)| key.clone()))
    }

    fn admit(&mut self, key: LaneKey, now: Instant) -> u64 {
        let ticket = self.serial();
        self.tickets.insert(
            ticket,
            Ticket {
                key,
                reaching: None,
                progress_at: now,
            },
        );
        self.keep_watching();
        ticket
    }

    fn beat(&mut self, ticket: u64, now: Instant) {
        let Self {
            tickets, stalls, ..
        } = self;
        if let Some(ticket) = tickets.get_mut(&ticket) {
            ticket.progress_at = now;
            stalls.remove(ticket.holds());
            self.keep_watching();
        }
    }

    fn keep_watching(&mut self) {
        if !std::mem::replace(&mut self.watching, true) {
            watch_after(STALLED_AFTER);
        }
    }

    fn watch(&mut self, now: Instant) -> bool {
        let next = self
            .tickets
            .values()
            .map(|ticket| ticket.progress_at + STALLED_AFTER)
            .filter(|stalls_at| *stalls_at > now)
            .min();
        self.watching = next.is_some();
        if let Some(stalls_at) = next {
            watch_after(stalls_at.saturating_duration_since(now));
        }
        !self.stalls.is_empty()
            || self
                .tickets
                .values()
                .any(|ticket| ticket.quiet(now) >= STALLED_AFTER)
    }

    fn learn(&mut self, ticket: u64, path: &Path, id: u64, now: Instant) {
        if self.learned.len() >= LEARNED_PATHS {
            self.learned.clear();
        }
        self.learned.insert(path.to_owned(), id);
        self.beat(ticket, now);
        if let Some(ticket) = self.tickets.get_mut(&ticket) {
            ticket.key = LaneKey::Mount(id);
        }
    }

    fn outlived(&mut self, key: LaneKey, quiet_since: Instant, now: Instant) {
        if now.saturating_duration_since(quiet_since) >= STALLED_AFTER {
            let since = self.stalls.entry(key).or_insert(quiet_since);
            *since = (*since).min(quiet_since);
        }
    }

    fn finish(&mut self, ticket: u64, now: Instant) {
        if let Some(ticket) = self.tickets.remove(&ticket) {
            let key = ticket.reaching.unwrap_or(ticket.key);
            self.outlived(key, ticket.progress_at, now);
        }
    }

    fn withdraw(&mut self, ticket: u64) {
        self.tickets.remove(&ticket);
    }

    fn enter(&mut self, ticket: u64, path: &Path, lexical: Option<u64>, now: Instant) -> bool {
        let target = Target {
            key: self.key_for(path, lexical),
            path: Some(path),
            lexical,
        };
        let Ok(probe) = self.verdict(&target, Class::Interactive, now, Some(ticket)) else {
            return false;
        };
        if let Some(ticket) = self.tickets.get_mut(&ticket) {
            ticket.reaching = Some(probe.unwrap_or(target.key));
            ticket.progress_at = now;
            self.keep_watching();
        }
        true
    }

    fn leave(&mut self, ticket: u64, answered: bool, now: Instant) {
        let Some(ticket) = self.tickets.get_mut(&ticket) else {
            return;
        };
        let Some(key) = ticket.reaching.take() else {
            return;
        };
        let since = std::mem::replace(&mut ticket.progress_at, now);
        if answered {
            self.stalls.remove(&key);
        } else {
            self.outlived(key, since, now);
        }
        self.keep_watching();
    }

    fn responsiveness(&self, target: &Target<'_>, now: Instant) -> Responsiveness {
        let requests = self
            .tickets
            .values()
            .filter(|ticket| ticket.holds().covers(target))
            .map(|ticket| ticket.quiet(now));
        let stalls = self
            .stalls
            .iter()
            .filter(|(key, _)| key.covers(target))
            .map(|(_, since)| now.saturating_duration_since(*since));
        match requests.chain(stalls).max() {
            Some(quiet) if quiet >= NOT_RESPONDING_AFTER => Responsiveness::NotResponding,
            Some(quiet) if quiet >= STALLED_AFTER => Responsiveness::Stalled,
            _ => Responsiveness::Answering,
        }
    }
}

static LANES: LazyLock<Mutex<Lanes>> = LazyLock::new(Mutex::default);

thread_local! {
    static FOLLOWERS: RefCell<Vec<Weak<dyn Fn()>>> = RefCell::default();
}

pub(crate) fn follow(notify: &Rc<dyn Fn()>) {
    FOLLOWERS.with(|followers| {
        let mut followers = followers.borrow_mut();
        followers.retain(|follower| follower.strong_count() > 0);
        followers.push(Rc::downgrade(notify));
    });
}

fn watch_after(delay: Duration) {
    if !cfg!(test) {
        glib::timeout_add_once(delay, watch_stalls);
    }
}

fn watch_stalls() {
    if !lanes().watch(Instant::now()) {
        return;
    }
    let followers: Vec<Rc<dyn Fn()>> = FOLLOWERS.with(|followers| {
        followers
            .borrow()
            .iter()
            .filter_map(Weak::upgrade)
            .collect()
    });
    for notify in followers {
        notify();
    }
}

fn lanes() -> MutexGuard<'static, Lanes> {
    LANES.lock().unwrap_or_else(PoisonError::into_inner)
}

fn lanes_following_mounts() -> (MutexGuard<'static, Lanes>, MountTable) {
    let (generation, mounts) = mount_watch::snapshot();
    (lanes_following(generation, &mounts), mounts)
}

fn lanes_following(generation: u64, mounts: &MountTable) -> MutexGuard<'static, Lanes> {
    let mut held = lanes();
    let stranded = held.follow(generation, mounts);
    if stranded.is_empty() {
        return held;
    }
    drop(held);
    stranded.into_iter().for_each(Job::abandon);
    lanes()
}

pub(crate) struct Progress {
    ticket: Option<u64>,
    path: PathBuf,
    lexical: Option<u64>,
    started: Instant,
    last_beat: AtomicU64,
    reading: Arc<AtomicBool>,
}

impl Progress {
    fn new(ticket: u64, path: &Path, lexical: Option<u64>, reading: Arc<AtomicBool>) -> Self {
        Self {
            ticket: Some(ticket),
            path: path.to_owned(),
            lexical,
            started: Instant::now(),
            last_beat: AtomicU64::new(0),
            reading,
        }
    }

    #[cfg(test)]
    pub(crate) fn unlaned() -> Self {
        Self {
            ticket: None,
            path: PathBuf::new(),
            lexical: None,
            started: Instant::now(),
            last_beat: AtomicU64::new(0),
            reading: Arc::default(),
        }
    }

    pub(crate) fn beat(&self) {
        let Some(ticket) = self.ticket else {
            return;
        };
        let now = Instant::now();
        let at = u64::try_from(now.duration_since(self.started).as_millis()).unwrap_or(u64::MAX);
        let last = self.last_beat.load(Ordering::Relaxed);
        if last != 0 && at.saturating_sub(last) < BEAT_EVERY.as_millis() as u64 {
            return;
        }
        self.last_beat.store(at.max(1), Ordering::Relaxed);
        lanes().beat(ticket, now);
    }

    pub(crate) fn answered(&self, outcome: &impl Outcome) {
        if outcome.made_progress() {
            self.beat();
        }
    }

    pub(crate) fn learn(&self, id: u64) {
        if let Some(ticket) = self.ticket {
            lanes().learn(ticket, &self.path, id, Instant::now());
        }
    }

    pub(crate) fn begin_reading(&self) {
        self.reading.store(true, Ordering::Relaxed);
    }

    pub(crate) fn reach<T: Outcome>(
        &self,
        path: &Path,
        mounted: Option<u64>,
        work: impl FnOnce() -> T,
    ) -> Option<T> {
        let Some(ticket) = self.ticket else {
            return Some(work());
        };
        if !lanes().enter(ticket, path, mounted.or(self.lexical), Instant::now()) {
            return None;
        }
        let answer = work();
        lanes().leave(ticket, answer.made_progress(), Instant::now());
        self.last_beat.store(0, Ordering::Relaxed);
        Some(answer)
    }
}

pub(crate) struct Admitted {
    progress: Progress,
    calling: Cell<bool>,
}

impl Admitted {
    pub(crate) fn progress(&self) -> &Progress {
        &self.progress
    }

    pub(crate) async fn call<F: Future>(&self, call: F) -> F::Output {
        self.calling.set(true);
        let answer = call.await;
        self.calling.set(false);
        answer
    }
}

impl Drop for Admitted {
    fn drop(&mut self) {
        if let Some(ticket) = self.progress.ticket {
            match self.calling.get() {
                true => lanes().withdraw(ticket),
                false => lanes().finish(ticket, Instant::now()),
            }
        }
    }
}

fn admitted(lanes: &mut Lanes, target: Target<'_>, class: Class) -> Result<Admitted, Unanswered> {
    let now = Instant::now();
    let probe = lanes.verdict(&target, class, now, None)?;
    let path = target.path.unwrap_or(Path::new(""));
    let ticket = lanes.admit(probe.unwrap_or(target.key), now);
    Ok(Admitted {
        progress: Progress::new(ticket, path, target.lexical, Arc::default()),
        calling: Cell::new(false),
    })
}

pub(crate) fn admit(path: &Path, class: Class) -> Result<Admitted, Unanswered> {
    let (mut lanes, mounts) = lanes_following_mounts();
    let target = lanes.target(path, &mounts);
    admitted(&mut lanes, target, class)
}

pub(crate) fn run<T: Send + 'static>(
    path: &Path,
    class: Class,
    work: impl FnOnce(&Progress) -> T + Send + 'static,
) -> Pin<Box<dyn Future<Output = Result<T, Unanswered>>>> {
    let admitted = match admit(path, class) {
        Ok(admitted) => admitted,
        Err(refused) => return Box::pin(std::future::ready(Err(refused))),
    };
    let worker = gio::spawn_blocking(move || work(admitted.progress()));
    Box::pin(async move { worker.await.map_err(|_| Unanswered::Failed) })
}

pub(crate) fn authority(uri: &str) -> String {
    match glib::Uri::parse(uri, glib::UriFlags::NONE) {
        Ok(parsed) => format!(
            "{}://{}@{}:{}",
            parsed.scheme(),
            parsed.userinfo().unwrap_or_default(),
            parsed.host().unwrap_or_default(),
            parsed.port()
        ),
        Err(_) => uri
            .split_once(':')
            .map_or(uri, |(scheme, _)| scheme)
            .to_owned(),
    }
}

pub(crate) fn admit_remote(authority: String, class: Class) -> Result<Admitted, Unanswered> {
    let target = Target {
        key: LaneKey::Remote(authority),
        path: None,
        lexical: None,
    };
    admitted(&mut lanes(), target, class)
}

pub(crate) async fn run_remote<T: Outcome>(
    authority: String,
    class: Class,
    call: impl Future<Output = T>,
) -> Result<T, Unanswered> {
    let admitted = admit_remote(authority, class)?;
    let answer = admitted.call(call).await;
    admitted.progress().answered(&answer);
    Ok(answer)
}

type Shared = Arc<dyn Any + Send + Sync>;
type Work = Box<dyn FnOnce(&Progress, &gio::Cancellable) -> Shared + Send>;
type Reply = oneshot::Sender<Result<Shared, Unanswered>>;

struct Run {
    serial: u64,
    answers: Vec<Reply>,
    waiting: usize,
    cancellable: gio::Cancellable,
    reading: Arc<AtomicBool>,
}

impl Run {
    fn new(serial: u64, answer: Reply) -> Self {
        Self {
            serial,
            answers: vec![answer],
            waiting: 1,
            cancellable: gio::Cancellable::new(),
            reading: Arc::default(),
        }
    }

    fn joinable(&self) -> bool {
        !self.reading.load(Ordering::Relaxed) && !self.cancellable.is_cancelled()
    }

    fn join(&mut self, answer: Reply) -> u64 {
        self.answers.push(answer);
        self.waiting += 1;
        self.serial
    }
}

struct Job {
    ticket: u64,
    running: Run,
    next: Option<(Run, Work)>,
}

impl Job {
    fn abandon(self) {
        self.running.cancellable.cancel();
        let queued = self.next.map(|(run, _)| run.answers).unwrap_or_default();
        for sender in self.running.answers.into_iter().chain(queued) {
            let _ = sender.send(Err(Unanswered::TimedOut));
        }
    }
}

struct Start {
    ticket: u64,
    join: JoinKey,
    lexical: Option<u64>,
    serial: u64,
    cancellable: gio::Cancellable,
    reading: Arc<AtomicBool>,
    work: Work,
}

impl Start {
    fn new(ticket: u64, join: JoinKey, lexical: Option<u64>, run: &Run, work: Work) -> Self {
        Self {
            ticket,
            join,
            lexical,
            serial: run.serial,
            cancellable: run.cancellable.clone(),
            reading: run.reading.clone(),
            work,
        }
    }

    fn spawn(self) {
        let Self {
            ticket,
            join,
            lexical,
            serial,
            cancellable,
            reading,
            work,
        } = self;
        let progress = Progress::new(ticket, &join.0, lexical, reading);
        let mut done = Done {
            ticket,
            join,
            serial,
            sent: false,
        };
        drop(gio::spawn_blocking(move || {
            let answer = work(&progress, &cancellable);
            done.send(Some(answer));
        }));
    }
}

enum Next {
    Start(Start),
    Refused(Vec<Reply>, Unanswered),
}

struct Done {
    ticket: u64,
    join: JoinKey,
    serial: u64,
    sent: bool,
}

impl Done {
    fn send(&mut self, answer: Option<Shared>) {
        self.sent = true;
        let (answers, next) =
            lanes().complete(self.ticket, &self.join, self.serial, Instant::now());
        if let Some(answer) = answer {
            for sender in answers {
                let _ = sender.send(Ok(answer.clone()));
            }
        }
        match next {
            Some(Next::Start(start)) => start.spawn(),
            Some(Next::Refused(answers, refusal)) => {
                for sender in answers {
                    let _ = sender.send(Err(refusal));
                }
            }
            None => {}
        }
    }
}

impl Drop for Done {
    fn drop(&mut self) {
        if !self.sent {
            self.send(None);
        }
    }
}

impl Lanes {
    fn complete(
        &mut self,
        ticket: u64,
        join: &JoinKey,
        serial: u64,
        now: Instant,
    ) -> (Vec<Reply>, Option<Next>) {
        self.finish(ticket, now);
        let Some(mut job) = self.jobs.remove(join) else {
            return (Vec::new(), None);
        };
        if job.running.serial != serial {
            self.jobs.insert(join.clone(), job);
            return (Vec::new(), None);
        }
        let answers = std::mem::take(&mut job.running.answers);
        let Some((run, work)) = job.next.take() else {
            return (answers, None);
        };
        let mounts = self.mounts.clone();
        let target = self.target(&join.0, &mounts);
        let probe = match self.verdict(&target, Class::Interactive, now, None) {
            Ok(probe) => probe,
            Err(refusal) => return (answers, Some(Next::Refused(run.answers, refusal))),
        };
        let lexical = target.lexical;
        let ticket = self.admit(probe.unwrap_or(target.key), now);
        let start = Start::new(ticket, join.clone(), lexical, &run, work);
        self.jobs.insert(
            join.clone(),
            Job {
                ticket,
                running: run,
                next: None,
            },
        );
        (answers, Some(Next::Start(start)))
    }

    fn leave_job(&mut self, join: &JoinKey, serial: u64) {
        let Some(job) = self.jobs.get_mut(join) else {
            return;
        };
        if job.running.serial == serial {
            job.running.waiting -= 1;
            if job.running.waiting == 0 {
                job.running.cancellable.cancel();
            }
        } else if job
            .next
            .as_ref()
            .is_some_and(|(run, _)| run.serial == serial)
        {
            let emptied = job.next.as_mut().is_some_and(|(run, _)| {
                run.waiting -= 1;
                run.waiting == 0
            });
            if emptied {
                job.next = None;
            }
        }
    }
}

struct Waiting {
    join: JoinKey,
    serial: u64,
}

impl Drop for Waiting {
    fn drop(&mut self) {
        lanes().leave_job(&self.join, self.serial);
    }
}

pub(crate) async fn run_joined<T: Send + Sync + 'static>(
    path: &Path,
    kind: String,
    work: impl FnOnce(&Progress, &gio::Cancellable) -> T + Send + 'static,
) -> Result<Arc<T>, Unanswered> {
    run_joined_in(path, kind, Class::Interactive, work).await
}

pub(crate) async fn run_joined_in<T: Send + Sync + 'static>(
    path: &Path,
    kind: String,
    class: Class,
    work: impl FnOnce(&Progress, &gio::Cancellable) -> T + Send + 'static,
) -> Result<Arc<T>, Unanswered> {
    let join = (path.to_owned(), kind);
    let (sender, answer) = oneshot::channel();
    let work: Work = Box::new(move |progress, cancellable| Arc::new(work(progress, cancellable)));
    let (waiting, start) = {
        let (mut lanes, mounts) = lanes_following_mounts();
        let now = Instant::now();
        let target = lanes.target(path, &mounts);
        let probe = lanes.verdict(&target, class, now, None)?;
        let fresh = lanes.serial();
        match lanes.jobs.get_mut(&join) {
            Some(job) if job.running.joinable() => (job.running.join(sender), None),
            Some(job) => match &mut job.next {
                Some((run, _)) => (run.join(sender), None),
                None => {
                    job.next = Some((Run::new(fresh, sender), work));
                    (fresh, None)
                }
            },
            None => {
                let lexical = target.lexical;
                let ticket = lanes.admit(probe.unwrap_or(target.key), now);
                let run = Run::new(fresh, sender);
                let start = Start::new(ticket, join.clone(), lexical, &run, work);
                lanes.jobs.insert(
                    join.clone(),
                    Job {
                        ticket,
                        running: run,
                        next: None,
                    },
                );
                (fresh, Some(start))
            }
        }
    };
    let waiting = Waiting {
        join,
        serial: waiting,
    };
    if let Some(start) = start {
        start.spawn();
    }
    let answer = answer.await;
    drop(waiting);
    match answer {
        Ok(Ok(answer)) => answer.downcast::<T>().map_err(|_| Unanswered::Failed),
        Ok(Err(refusal)) => Err(refusal),
        Err(_) => Err(Unanswered::Failed),
    }
}

pub(crate) fn responsiveness(path: &Path) -> Responsiveness {
    let (lanes, mounts) = lanes_following_mounts();
    let target = lanes.target(path, &mounts);
    lanes.responsiveness(&target, Instant::now())
}

pub(crate) fn remote_responsiveness(authority: String) -> Responsiveness {
    let target = Target {
        key: LaneKey::Remote(authority),
        path: None,
        lexical: None,
    };
    lanes().responsiveness(&target, Instant::now())
}
