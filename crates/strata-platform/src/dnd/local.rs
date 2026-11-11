//! The in-process fast path: a drag from this process offers a private MIME type naming the
//! drag, `application/x-local-drag;id=<counter>.<nonce>`, with no process id in it (the
//! owner's ruling: other clients may see it). A drop in this process that sees the token of
//! the drag it started takes the typed payload instead of reading any type. Seeing the drag
//! on our own data device isn't proof (other clients' drags arrive there too); the token is.
//! The nonce is 64 bits from a process-random hash key, so another process's token, or one
//! copied from a listing, doesn't match a later drag of ours.

use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// The token type's prefix, before `id=`.
pub const LOCAL_MIME: &str = "application/x-local-drag";

/// A new token type for a drag.
pub(crate) fn new_token() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    // std's RandomState draws its keys from the OS once per thread: hashing the counter and
    // the clock with them makes the nonce unguessable, with no crate for it.
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(n);
    h.write_u64(nanos);
    format!("{LOCAL_MIME};id={n}.{:016x}", h.finish())
}

/// Whether `mime` is a local-drag token at all (from this process or another).
pub(crate) fn is_token(mime: &str) -> bool {
    mime.strip_prefix(LOCAL_MIME)
        .is_some_and(|rest| rest.starts_with(";id="))
}

/// The token among `mimes`, if any.
pub(crate) fn token_in(mimes: &[String]) -> Option<&str> {
    mimes.iter().map(String::as_str).find(|m| is_token(m))
}

/// `mimes` without local-drag tokens: what an app sees.
pub(crate) fn visible(mimes: &[String]) -> Vec<String> {
    mimes.iter().filter(|m| !is_token(m)).cloned().collect()
}

#[cfg(test)]
mod tests;
