// SPDX-License-Identifier: MIT

#[path = "../support/mod.rs"]
mod support;

mod w3;

mod wake;

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
