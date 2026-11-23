// SPDX-License-Identifier: MIT

use std::{rc::Rc, time::Duration};

use strata_components::contract::EntryId;
use strata_qa::{Chord, ClipboardContent, Keymap, Modifier, Rect};

use super::tree::{Node, Snapshot};
use crate::App;

/// What `activate` presses: the element's key, and where it is drawn.
pub(super) struct Activated {
    /// The node pressed, which tells a host with more than one window which one has it.
    pub(super) node: Node,
    pub(super) key: String,
    pub(super) bounds: Option<Rect>,
}

pub(super) trait Host {
    /// True for a host that accepts steps before a window opens (`--portal`).
    fn windowless(&self) -> bool {
        false
    }
    /// The active window root node, or `None` while windowless before `adopt`.
    fn window(&self) -> Option<Node> {
        self.snapshot().root().and_then(|root| root.node.clone())
    }
    /// Every open window root node.
    fn toplevels(&self) -> Vec<Node> {
        self.window().into_iter().collect()
    }
    /// The title of `window`.
    fn title_of(&self, _window: &Node) -> Option<String> {
        self.with_app(|app| app.title().to_owned()).ok()
    }
    /// Makes `window` the active target for subsequent steps.
    fn adopt(&self, _window: &Node) {}
    /// One idle turn of the host between steps.
    fn idle_turn(&self) -> bool {
        false
    }

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

    /// Where the corner of the window that draws `node` is on the screen.
    fn origin(&self, node: &Node) -> Result<(f64, f64), String>;
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
    async fn scroll_to(&self, at: (&Node, &str), x: f32, y: f32) -> Result<(), String>;

    async fn set_clipboard(&self, _content: &ClipboardContent) -> Option<Result<(), String>> {
        None
    }

    async fn read_clipboard(&self, _mime: Option<&str>) -> Option<Result<Option<String>, String>> {
        None
    }
}
