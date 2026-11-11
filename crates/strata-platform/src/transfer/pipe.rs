//! Pipes for moving one MIME type between processes: the reader's end of an offer and the
//! writer's end of a source. Every pipe is `O_CLOEXEC`, every read has a size cap and an idle
//! timeout, and a write to a reader that left reports it rather than raising `SIGPIPE` (Rust
//! programs ignore it, and a library mustn't change the disposition).
#![expect(unsafe_code, reason = "pipe2, fcntl and poll through libc")]

use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::time::{Duration, Instant};

use super::data::ReadLimits;
use super::error::ReadError;

/// A pipe: the read end, then the write end. Both close on exec.
pub(crate) fn pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [-1; 2];
    // SAFETY: `fds` is a writable array of two ints, as pipe2 requires.
    let ret = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
    if ret != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: pipe2 succeeded, so fds[0] is a new descriptor that nothing else owns.
    let r = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    // SAFETY: as above, for fds[1].
    let w = unsafe { OwnedFd::from_raw_fd(fds[1]) };
    Ok((r, w))
}

/// Makes `fd` non-blocking, so a read or write never waits past its poll.
pub(crate) fn set_nonblocking(fd: BorrowedFd<'_>) -> io::Result<()> {
    // SAFETY: F_GETFL on a descriptor the borrow keeps open; no pointers involved.
    let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: F_SETFL with an int flag set, on the same open descriptor.
    let ret = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) };
    if ret < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Waits up to `timeout` (None: no limit) for any of `fds` to be ready for its events, and
/// returns each one's `revents`. EINTR counts as nothing ready.
pub(crate) fn poll(
    fds: &[(BorrowedFd<'_>, i16)],
    timeout: Option<Duration>,
) -> io::Result<Vec<i16>> {
    let mut pfds: Vec<libc::pollfd> = fds
        .iter()
        .map(|(fd, events)| libc::pollfd {
            fd: fd.as_raw_fd(),
            events: *events,
            revents: 0,
        })
        .collect();
    let ms = timeout.map_or(-1, |t| {
        // Round up, so a short wait isn't a busy loop.
        i32::try_from(
            t.as_millis()
                .saturating_add(u128::from(t.subsec_nanos() % 1_000_000 != 0)),
        )
        .unwrap_or(i32::MAX)
    });
    // SAFETY: `pfds` is a live array of `pfds.len()` pollfd structs; every fd in it is kept
    // open by the borrows in `fds` for the whole call.
    let ret = unsafe { libc::poll(pfds.as_mut_ptr(), pfds.len() as libc::nfds_t, ms) };
    if ret < 0 {
        let e = io::Error::last_os_error();
        if e.kind() == io::ErrorKind::Interrupted {
            return Ok(vec![0; pfds.len()]);
        }
        return Err(e);
    }
    Ok(pfds.iter().map(|p| p.revents).collect())
}

const READY_IN: i16 = libc::POLLIN | libc::POLLHUP | libc::POLLERR;
const READY_OUT: i16 = libc::POLLOUT | libc::POLLHUP | libc::POLLERR;

/// Reads `fd` to its end within `limits`.
pub(crate) fn read_all(fd: OwnedFd, limits: ReadLimits) -> Result<Vec<u8>, ReadError> {
    let io = |e: io::Error| ReadError::Io(e.to_string());
    set_nonblocking(fd.as_fd()).map_err(io)?;
    let mut file = File::from(fd);
    let mut out = Vec::new();
    let mut buf = vec![0; 64 * 1024];
    let mut deadline = Instant::now() + limits.idle;
    loop {
        match file.read(&mut buf) {
            Ok(0) => return Ok(out),
            Ok(n) => {
                if out.len() + n > limits.max_bytes {
                    return Err(ReadError::TooLarge);
                }
                out.extend_from_slice(&buf[..n]);
                deadline = Instant::now() + limits.idle;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Err(ReadError::TimedOut);
                }
                poll(&[(file.as_fd(), READY_IN)], Some(left)).map_err(io)?;
            }
            Err(e) => return Err(io(e)),
        }
    }
}

/// Why a source write stopped.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum WriteError {
    /// The reader closed its end (EPIPE): it took what it wanted, or gave up.
    ReaderLeft,
    /// The reader stopped reading for the whole idle time.
    TimedOut,
    Io(io::ErrorKind),
}

/// Writes all of `data` to `fd`, waiting at most `idle` for the reader each time the pipe is
/// full. The fd closes when this returns, which is the reader's end of data.
pub(crate) fn write_all(fd: OwnedFd, data: &[u8], idle: Duration) -> Result<(), WriteError> {
    set_nonblocking(fd.as_fd()).map_err(|e| WriteError::Io(e.kind()))?;
    let mut file = File::from(fd);
    let mut rest = data;
    let mut deadline = Instant::now() + idle;
    while !rest.is_empty() {
        match file.write(rest) {
            Ok(n) => {
                rest = &rest[n..];
                deadline = Instant::now() + idle;
            }
            Err(e) if e.kind() == io::ErrorKind::BrokenPipe => return Err(WriteError::ReaderLeft),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Err(WriteError::TimedOut);
                }
                poll(&[(file.as_fd(), READY_OUT)], Some(left))
                    .map_err(|e| WriteError::Io(e.kind()))?;
            }
            Err(e) => return Err(WriteError::Io(e.kind())),
        }
    }
    Ok(())
}

/// Reads `fd` on a worker thread and hands the result to `done` there.
pub(crate) fn spawn_read(
    fd: OwnedFd,
    limits: ReadLimits,
    done: impl FnOnce(Result<Vec<u8>, ReadError>) + Send + 'static,
) {
    // Shared, so a failed spawn can still answer.
    fn take<T>(d: &std::sync::Mutex<Option<T>>) -> Option<T> {
        d.lock().ok().and_then(|mut d| d.take())
    }
    let done = std::sync::Arc::new(std::sync::Mutex::new(Some(done)));
    let worker = done.clone();
    let spawned = std::thread::Builder::new()
        .name("transfer-read".into())
        .spawn(move || {
            let result = read_all(fd, limits);
            if let Some(done) = take(&worker) {
                done(result);
            }
        });
    if let Err(e) = spawned
        && let Some(done) = take(&done)
    {
        done(Err(ReadError::Io(format!("no thread for the read: {e}"))));
    }
}

/// Runs `writer` (if it hasn't run yet) and writes its bytes to `fd`, on a worker thread.
pub(crate) fn spawn_write(fd: OwnedFd, writer: super::data::Writer) {
    let spawned = std::thread::Builder::new()
        .name("transfer-write".into())
        .spawn(move || {
            // A writer that panicked sends nothing: closing the fd ends the reader's read.
            if let Some(bytes) = writer.get() {
                let _ = write_all(fd, &bytes, super::data::WRITE_IDLE);
            }
        });
    if let Err(e) = spawned {
        eprintln!("strata-platform transfer: no thread for a write: {e}");
    }
}

#[cfg(test)]
mod tests;
