// SPDX-License-Identifier: MIT
use super::protocol::{FRAME_LIMIT, checked_reply};
use super::{Manifest, PATH_LIMIT, Registration, Reply, Request};
use std::{
    collections::{BTreeMap, VecDeque},
    io::{self, Read, Write},
    os::unix::{net::UnixStream, process::CommandExt},
    process::{Command, Stdio},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    },
    thread,
    time::{Duration, Instant},
};

const DEADLINE: Duration = Duration::from_secs(8);
const UPDATE_LIMIT: usize = 32;

pub(crate) enum Update {
    Reply(Box<Reply>),
    Offline {
        unsent: Vec<u64>,
        in_flight: Option<u64>,
    },
}

pub(crate) struct Requests {
    refresh: SyncSender<Request>,
    activate: SyncSender<Request>,
    closed: Arc<AtomicBool>,
}

impl Drop for Requests {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Relaxed);
    }
}

impl Requests {
    pub(crate) fn try_send(&self, request: Request) -> Result<(), TrySendError<Request>> {
        if !request.valid() || !super::selection_fits(&request.paths) {
            return Err(TrySendError::Full(request));
        }
        if request.method == "activate" {
            self.activate.try_send(request)
        } else {
            self.refresh.try_send(request)
        }
    }

    #[cfg(test)]
    pub(super) fn send(&self, request: Request) -> Result<(), mpsc::SendError<Request>> {
        if request.method == "activate" {
            self.activate.send(request)
        } else {
            self.refresh.send(request)
        }
    }
}

#[derive(Default)]
struct UpdateState {
    replies: VecDeque<Update>,
    reply_count: usize,
    global: Option<Reply>,
    scopes: BTreeMap<String, Option<u64>>,
    closed: bool,
}

impl UpdateState {
    fn flush_events(&mut self) {
        self.replies.extend(
            self.global
                .take()
                .map(|reply| Update::Reply(Box::new(reply))),
        );
        let mut grouped: BTreeMap<Option<u64>, Vec<String>> = BTreeMap::new();
        for (path, revision) in std::mem::take(&mut self.scopes) {
            grouped.entry(revision).or_default().push(path);
        }
        self.replies
            .extend(grouped.into_iter().map(|(revision, paths)| {
                Update::Reply(Box::new(Reply {
                    version: 1,
                    event: Some("invalidate".into()),
                    paths: Some(paths),
                    revision,
                    ..Reply::default()
                }))
            }));
    }
}

#[derive(Default)]
pub(crate) struct Updates {
    state: Mutex<UpdateState>,
    changed: Condvar,
}

impl Updates {
    pub(crate) fn drain(&self) -> (Vec<Update>, bool) {
        let mut state = self.state.lock().expect("provider update queue");
        state.flush_events();
        state.reply_count = 0;
        (state.replies.drain(..).collect(), state.closed)
    }

    #[cfg(test)]
    pub(crate) fn try_recv(&self) -> Result<Update, TryRecvError> {
        let mut state = self.state.lock().expect("provider update queue");
        if state.replies.is_empty() {
            state.flush_events();
        }
        let update = state.replies.pop_front().ok_or(if state.closed {
            TryRecvError::Disconnected
        } else {
            TryRecvError::Empty
        })?;
        if !matches!(&update, Update::Reply(reply) if reply.event.is_some()) {
            state.reply_count -= 1;
        }
        Ok(update)
    }

    #[cfg(test)]
    pub(crate) fn recv_timeout(&self, timeout: Duration) -> Result<Update, mpsc::RecvTimeoutError> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.try_recv() {
                Ok(update) => return Ok(update),
                Err(TryRecvError::Disconnected) => {
                    return Err(mpsc::RecvTimeoutError::Disconnected);
                }
                Err(TryRecvError::Empty) => (),
            }
            let state = self.state.lock().expect("provider update queue");
            if !state.replies.is_empty() || state.global.is_some() || !state.scopes.is_empty() {
                continue;
            }
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return Err(mpsc::RecvTimeoutError::Timeout);
            };
            let _ = self
                .changed
                .wait_timeout(state, left)
                .expect("provider update wait");
        }
    }

    fn has_room(&self) -> bool {
        self.state
            .lock()
            .expect("provider update queue")
            .reply_count
            < UPDATE_LIMIT
    }

    fn publish(&self, reply: Reply) {
        let mut state = self.state.lock().expect("provider update queue");
        if reply.event.is_some() {
            if let Some(paths) = &reply.paths {
                for path in paths {
                    let revision = state.scopes.entry(path.clone()).or_default();
                    *revision = (*revision).into_iter().chain(reply.revision).max();
                }
                if state.scopes.len() > PATH_LIMIT
                    || state.scopes.keys().map(String::len).sum::<usize>() > 65536
                {
                    let revision = state
                        .scopes
                        .values()
                        .filter_map(|r| *r)
                        .chain(state.global.as_ref().and_then(|r| r.revision))
                        .max();
                    state.scopes.clear();
                    state.global = Some(Reply {
                        paths: None,
                        revision,
                        ..reply
                    });
                }
            } else {
                let revision = state
                    .global
                    .as_ref()
                    .and_then(|r| r.revision)
                    .into_iter()
                    .chain(reply.revision)
                    .max();
                state.scopes.retain(|_, at| {
                    at.is_some_and(|at| revision.is_none_or(|revision| at > revision))
                });
                state.global = Some(Reply { revision, ..reply });
            }
        } else {
            // One in-flight request reserves its reply slot before dispatch.
            assert!(state.reply_count < UPDATE_LIMIT);
            // Coalesce only between replies, preserving the provider's snapshot ordering.
            state.flush_events();
            state.replies.push_back(Update::Reply(Box::new(reply)));
            state.reply_count += 1;
        }
        self.changed.notify_one();
    }

    fn offline(&self, unsent: Vec<u64>, in_flight: Option<u64>) {
        let mut state = self.state.lock().expect("provider update queue");
        state.flush_events();
        state
            .replies
            .push_back(Update::Offline { unsent, in_flight });
        state.reply_count += 1;
        self.changed.notify_one();
    }

    fn close(&self) {
        self.state.lock().expect("provider update queue").closed = true;
        self.changed.notify_all();
    }
}

pub(crate) struct Client {
    pub requests: Requests,
    pub updates: Arc<Updates>,
}

struct Incoming {
    refresh: Receiver<Request>,
    activate: Receiver<Request>,
    closed: Arc<AtomicBool>,
}

impl Incoming {
    fn next(&self) -> Result<Option<Request>, ()> {
        let action_closed = match self.activate.try_recv() {
            Ok(r) => return Ok(Some(r)),
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => true,
        };
        match self.refresh.try_recv() {
            Ok(r) => Ok(Some(r)),
            Err(TryRecvError::Disconnected) if action_closed => Err(()),
            _ => Ok(None),
        }
    }

    fn discard(&self) -> Vec<u64> {
        self.activate
            .try_iter()
            .chain(self.refresh.try_iter())
            .map(|r| r.id)
            .collect()
    }
}

pub(crate) fn start(registration: Registration) -> Client {
    let (refresh, rx) = mpsc::sync_channel(16);
    let (activate, actions) = mpsc::sync_channel(8);
    let updates = Arc::new(Updates::default());
    let out = updates.clone();
    let closed = Arc::new(AtomicBool::new(false));
    let worker_closed = closed.clone();
    thread::spawn(move || {
        let incoming = Incoming {
            refresh: rx,
            activate: actions,
            closed: worker_closed,
        };
        let mut last_diagnostic = None;
        loop {
            let first = loop {
                if incoming.closed.load(Ordering::Relaxed) {
                    out.close();
                    return;
                }
                if out.has_room() {
                    match incoming.next() {
                        Ok(Some(r)) => break r,
                        Err(()) => {
                            out.close();
                            return;
                        }
                        _ => (),
                    }
                }
                thread::sleep(Duration::from_millis(25));
            };
            match run(&registration.manifest, &incoming, &out, first) {
                Ok(()) => break,
                Err(failure) => {
                    if last_diagnostic
                        .is_none_or(|at: Instant| at.elapsed() >= Duration::from_secs(10))
                    {
                        eprintln!(
                            "Strata provider {} unavailable: {}",
                            registration.manifest.id, failure.reason
                        );
                        last_diagnostic = Some(Instant::now());
                    }
                    let mut unsent = incoming.discard();
                    unsent.extend(failure.unsent);
                    out.offline(unsent, failure.in_flight);
                    thread::sleep(Duration::from_secs(2));
                }
            }
        }
        out.close();
    });
    Client {
        requests: Requests {
            refresh,
            activate,
            closed,
        },
        updates,
    }
}

struct Failure {
    reason: &'static str,
    unsent: Option<u64>,
    in_flight: Option<u64>,
}

impl From<io::Error> for Failure {
    fn from(_: io::Error) -> Self {
        Self {
            reason: "transport failure",
            unsent: None,
            in_flight: None,
        }
    }
}

fn run(
    manifest: &Manifest,
    requests: &Incoming,
    updates: &Updates,
    first: Request,
) -> Result<(), Failure> {
    let first_id = first.id;
    let spawn = || -> io::Result<_> {
        let (stream, peer) = UnixStream::pair()?;
        stream.set_read_timeout(Some(Duration::from_millis(25)))?;
        stream.set_write_timeout(Some(Duration::from_millis(250)))?;
        let child = Command::new(&manifest.command[0])
            .args(&manifest.command[1..])
            .current_dir("/")
            .process_group(0)
            .stdin(Stdio::from(std::os::fd::OwnedFd::from(peer.try_clone()?)))
            .stdout(Stdio::from(std::os::fd::OwnedFd::from(peer)))
            .stderr(Stdio::null())
            .spawn()?;
        Ok((stream, child))
    };
    let (mut stream, mut child) = spawn().map_err(|_| Failure {
        reason: "spawn failure",
        unsent: Some(first_id),
        in_flight: None,
    })?;
    let mut next = Some(first);
    let mut in_flight = None;
    let result = (|| {
        let mut buffer = Vec::new();
        let mut pending: Option<(Request, Instant)> = None;
        let mut revision_mode = false;
        let mut event_revision = None;
        loop {
            if requests.closed.load(Ordering::Relaxed) {
                return Ok(());
            }
            if pending.is_none() && updates.has_room() {
                if next.is_none() {
                    next = requests.next().map_err(|_| Failure {
                        reason: "host closed",
                        unsent: None,
                        in_flight: None,
                    })?;
                }
                if let Some(r) = next.take() {
                    let mut line = serde_json::to_vec(&r).map_err(io::Error::other)?;
                    line.push(b'\n');
                    if !r.valid() || line.len() > FRAME_LIMIT {
                        return Err(Failure {
                            reason: "invalid host request",
                            unsent: Some(r.id),
                            in_flight: None,
                        });
                    }
                    in_flight = Some(r.id);
                    stream.write_all(&line)?;
                    pending = Some((r, Instant::now()));
                }
            }
            if pending
                .as_ref()
                .is_some_and(|(_, at)| at.elapsed() > DEADLINE)
            {
                return Err(Failure {
                    reason: "request timeout",
                    unsent: None,
                    in_flight: None,
                });
            }
            let mut bytes = [0; 8192];
            match stream.read(&mut bytes) {
                Ok(0) => {
                    return Err(Failure {
                        reason: "provider closed",
                        unsent: None,
                        in_flight: None,
                    });
                }
                Ok(n) => buffer.extend_from_slice(&bytes[..n]),
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) => {}
                Err(e) => return Err(e.into()),
            }
            for frame in take_frames(&mut buffer).map_err(|_| Failure {
                reason: "frame limit",
                unsent: None,
                in_flight: None,
            })? {
                let reply = checked_reply(&frame, manifest).map_err(|_| Failure {
                    reason: "invalid response",
                    unsent: None,
                    in_flight: None,
                })?;
                if reply.event.is_some() {
                    if (revision_mode && reply.revision.is_none())
                        || reply
                            .revision
                            .zip(event_revision)
                            .is_some_and(|(new, old)| new < old)
                    {
                        return Err(Failure {
                            reason: "invalid revision ordering",
                            unsent: None,
                            in_flight: None,
                        });
                    }
                    revision_mode |= reply.revision.is_some();
                    event_revision = reply.revision;
                    updates.publish(reply);
                } else if pending
                    .as_ref()
                    .is_some_and(|(r, _)| reply.id == Some(r.id))
                {
                    let (request, _) = pending.take().expect("matched response");
                    if !reply.matches(&request)
                        || (revision_mode
                            && request.method != "activate"
                            && reply.error.is_none()
                            && reply.revision.is_none())
                    {
                        return Err(Failure {
                            reason: "invalid method response",
                            unsent: None,
                            in_flight: None,
                        });
                    }
                    revision_mode |= reply.revision.is_some();
                    updates.publish(reply);
                    in_flight = None;
                }
            }
        }
    })();
    if let Some(pid) = rustix::process::Pid::from_raw(child.id() as i32) {
        let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
    }
    let _ = child.kill();
    let _ = child.wait();
    match result {
        Err(Failure {
            reason: "host closed",
            ..
        }) => Ok(()),
        Err(mut failure) => {
            failure.in_flight = in_flight;
            Err(failure)
        }
        Ok(()) => Ok(()),
    }
}

pub(super) fn take_frames(buffer: &mut Vec<u8>) -> io::Result<Vec<Vec<u8>>> {
    let mut frames = Vec::new();
    let mut start = 0;
    for (end, byte) in buffer.iter().enumerate() {
        if *byte == b'\n' {
            if end - start > FRAME_LIMIT {
                return Err(io::Error::other("frame limit"));
            }
            frames.push(buffer[start..end].to_vec());
            start = end + 1;
        }
    }
    buffer.drain(..start);
    if buffer.len() > FRAME_LIMIT {
        return Err(io::Error::other("frame limit"));
    }
    Ok(frames)
}

#[cfg(test)]
mod tests;
