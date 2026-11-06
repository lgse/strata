// SPDX-License-Identifier: MIT

//! Blocking workers sharing one LIFO job queue. Taking the newest job first
//! walks trees depth-first, which bounds the directories held open at once.

use std::{
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

pub(super) trait Job: Send + Sized + 'static {
    type Error: Clone + Send + 'static;

    fn run(self, queue: &WorkQueue<Self>);
    fn cancelled() -> Self::Error;
    fn panicked() -> Self::Error;
    fn not_started(error: io::Error) -> Self::Error;
}

pub(super) struct Group<J> {
    remaining: AtomicUsize,
    final_job: Mutex<Option<J>>,
}

impl<J> Group<J> {
    pub(super) fn new(members: usize, final_job: J) -> Arc<Self> {
        Arc::new(Self {
            remaining: AtomicUsize::new(members),
            final_job: Mutex::new(Some(final_job)),
        })
    }
}

struct State<J> {
    jobs: Vec<J>,
    active: usize,
}

pub(super) struct WorkQueue<J: Job> {
    state: Mutex<State<J>>,
    wake: Condvar,
    cancelled: Arc<AtomicBool>,
    failed: AtomicBool,
    error: Mutex<Option<J::Error>>,
    deferred: Mutex<Option<J::Error>>,
}

impl<J: Job> WorkQueue<J> {
    pub(super) fn new(cancelled: Arc<AtomicBool>) -> Self {
        Self {
            state: Mutex::new(State {
                jobs: Vec::new(),
                active: 0,
            }),
            wake: Condvar::new(),
            cancelled,
            failed: AtomicBool::new(false),
            error: Mutex::new(None),
            deferred: Mutex::new(None),
        }
    }

    pub(super) fn is_stopped(&self) -> bool {
        self.cancelled.load(Ordering::Acquire) || self.failed.load(Ordering::Acquire)
    }

    pub(super) fn enqueue(&self, job: J) {
        if self.is_stopped() {
            return;
        }
        let mut state = lock(&self.state);
        if self.is_stopped() {
            return;
        }
        state.jobs.push(job);
        self.wake.notify_one();
    }

    /// Stops the queue; only the first error is kept, and none after a cancel.
    pub(super) fn fail(&self, error: J::Error) {
        if Self::record(&self.cancelled, &self.error, error) {
            self.failed.store(true, Ordering::Release);
            self.wake.notify_all();
        }
    }

    /// Keeps `error` for the result like [`Self::fail`], but lets the other jobs finish. An error
    /// that stops the queue still wins, since only it explains what the unfinished jobs left.
    pub(super) fn defer(&self, error: J::Error) {
        Self::record(&self.cancelled, &self.deferred, error);
    }

    fn record(cancelled: &AtomicBool, slot: &Mutex<Option<J::Error>>, error: J::Error) -> bool {
        if cancelled.load(Ordering::Acquire) {
            return false;
        }
        let mut first_error = lock(slot);
        if first_error.is_none() {
            *first_error = Some(error);
        }
        true
    }

    pub(super) fn next_job(&self) -> Option<J> {
        let mut state = lock(&self.state);
        loop {
            if self.is_stopped() {
                return None;
            }
            if let Some(job) = state.jobs.pop() {
                state.active += 1;
                return Some(job);
            }
            if state.active == 0 {
                return None;
            }
            state = self
                .wake
                .wait_timeout(state, Duration::from_millis(10))
                .unwrap_or_else(|poison| poison.into_inner())
                .0;
        }
    }

    pub(super) fn finish_job(&self) {
        let mut state = lock(&self.state);
        state.active = state.active.saturating_sub(1);
        if state.active == 0 && state.jobs.is_empty() {
            self.wake.notify_all();
        }
    }

    pub(super) fn complete(&self, group: Option<Arc<Group<J>>>) {
        let Some(group) = group else {
            return;
        };
        if group.remaining.fetch_sub(1, Ordering::AcqRel) != 1 {
            return;
        }
        let final_job = lock(&group.final_job).take();
        if let Some(final_job) = final_job {
            self.enqueue(final_job);
        }
    }

    pub(super) fn result(&self) -> Result<(), J::Error> {
        if self.cancelled.load(Ordering::Acquire) {
            Err(J::cancelled())
        } else if let Some(error) = lock(&self.error)
            .clone()
            .or_else(|| lock(&self.deferred).clone())
        {
            Err(error)
        } else {
            Ok(())
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

pub(super) fn run_workers<J: Job>(
    queue: &Arc<WorkQueue<J>>,
    worker_count: usize,
    nice: Option<i32>,
    mut spawn: impl FnMut(Box<dyn FnOnce() + Send>) -> io::Result<thread::JoinHandle<()>>,
) {
    let mut workers = Vec::with_capacity(worker_count);
    for _ in 0..worker_count {
        let worker_queue = queue.clone();
        match spawn(Box::new(move || {
            let queue = worker_queue;
            if let Some(nice) = nice {
                let _priority = rustix::process::setpriority_process(None, nice);
            }
            while let Some(job) = queue.next_job() {
                if catch_unwind(AssertUnwindSafe(|| job.run(&queue))).is_err() {
                    queue.fail(J::panicked());
                }
                queue.finish_job();
            }
        })) {
            Ok(worker) => workers.push(worker),
            Err(error) => {
                queue.fail(J::not_started(error));
                break;
            }
        }
    }
    for worker in workers {
        if worker.join().is_err() {
            queue.fail(J::panicked());
        }
    }
}
