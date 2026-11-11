//! strata-platform's compositor and D-Bus tests: one binary, one module per work package, each
//! test in a private session from `tests/support/` (docs/testing.md, "Compositor and D-Bus
//! tests"). Every test that starts a session is `#[ignore]`d: `mise run test-platform` runs
//! them (`mise run test-platform p2::` runs one package's), and `mise run test` never does.
//!
//! A work package edits only its own module file. The declarations below are W3's.

#[path = "../support/mod.rs"]
mod support;

mod w3;

#[cfg(feature = "_transfer")]
mod p0;

#[cfg(feature = "dnd")]
mod p1;

#[cfg(feature = "clipboard")]
mod p2;

#[cfg(feature = "csd")]
mod p3;

#[cfg(feature = "foreign-parent")]
mod p4;

#[cfg(feature = "settings")]
mod p5;

#[cfg(feature = "single-instance")]
mod p8;
