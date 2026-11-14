// SPDX-License-Identifier: MIT

//! What the QA backend asks of the host it drives. The backend reads the accessibility tree
//! the same way on both; the hosts differ in where the tree comes from, how input reaches
//! Rune and how time passes:
//!
//! - [`Staged`](super::staged::Staged), the offscreen host: this thread owns the `Runtime`,
//!   input goes in at Rune's input layer, and a settle turns the host until it rests.
//! - [`Session`](super::session::Session), the session host: Rune runs a real window's loop
//!   on the main thread, the tree comes from the frames it draws, and input crosses the
//!   compositor through a virtual keyboard and pointer.

use std::{rc::Rc, time::Duration};

use strata_components::contract::EntryId;
use strata_qa::{Chord, Keymap, Modifier, Rect};

use super::tree::Snapshot;
use crate::App;

/// What `activate` presses: the element's key, and where it is drawn.
pub(super) struct Activated {
    pub(super) key: String,
    pub(super) bounds: Option<Rect>,
}

pub(super) trait Host {
    /// The tree as of the host's latest frame.
    fn snapshot(&self) -> Rc<Snapshot>;
    /// Runs `read` on the window's state, on the thread that owns it.
    fn with_app<T: Send + 'static>(
        &self,
        read: impl FnOnce(&App) -> T + Send + 'static,
    ) -> Result<T, String>;
    /// How many replacements the browser has made (`qa::replaced`).
    fn replacements(&self) -> u64;
    /// Whether the entry `entry` was in a replacement after `since`.
    fn replaced_since(&self, entry: EntryId, since: u64) -> bool;

    /// Returns once the window has drawn what its last input and work changed.
    async fn settle(&self);
    /// Lets the application run for `time`.
    async fn sleep(&self, time: Duration);
    /// Waits up to `limit` for the host to have work.
    fn pause(&self, limit: Duration);

    /// Where the window's corner is on the screen.
    fn origin(&self) -> Result<(f64, f64), String>;
    async fn resize(&self, width: f32, height: f32) -> Result<(), String>;
    fn set_layout(&self, keymap: &Keymap) -> Result<(), String>;

    async fn chord(&self, chord: &Chord) -> Result<(), String>;
    async fn text_input(&self, character: char) -> Result<(), String>;
    async fn hold(&self, modifiers: &[Modifier]) -> Result<(), String>;
    async fn release(&self, modifiers: &[Modifier]);
    async fn key(&self, name: &str, pressed: bool) -> Result<(), String>;
    async fn pointer_move(&self, x: i16, y: i16) -> Result<(), String>;
    fn pointer_position(&self) -> (i16, i16);
    async fn button(&self, number: u8, pressed: bool) -> Result<(), String>;
    /// Presses the element as an assistive client's click would. False when it cannot.
    fn activate(&self, target: &Activated) -> bool;
}
