//! The peer for strata-platform's compositor tests: a second process, written against raw
//! `wayland-client` and `x11rb`, that sets and reads selections, drags and accepts drops,
//! exports its window, and drives a virtual pointer and keyboard. Tests drive it with JSON
//! lines ([`protocol`]); `crates/strata-platform/tests/support/peer.rs` is the typed client.
//!
//! It is the stand-in for "another app", as Strata's `tests/qa/foreign_drag_source.py` is. It
//! talks Wayland when `WAYLAND_DISPLAY` is set and X11 (`DISPLAY`) otherwise.

pub mod protocol;

#[cfg(test)]
mod tests;
