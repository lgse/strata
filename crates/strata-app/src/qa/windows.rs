// SPDX-License-Identifier: MIT

//! The offscreen host with the browser window and, while one is open, the floating
//! destination chooser it opened (`crate::chooser::floating`): two toplevels, as GTK's
//! `Copy to`, `Move to` and `Extract to` windows stand beside the browser.
//!
//! The steps read one tree: the chooser's nodes stand beside the browser's, under a root of
//! their own ([`Snapshot::with_toplevel`]), so a target `within` the chooser's toplevel finds
//! them, while the window the steps act on stays the browser. The chooser is in front: keys
//! go to it and the focus is its own. It stands to the right of the browser on the virtual
//! screen ([`FLOATING_X`]), so a pointer aimed at one of its nodes goes to it.
//!
//! Each window is an `Rc<Staged>` cloned before a step acts on it: a step that answers the
//! chooser closes it, and the clone keeps it alive until the step returns.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

use strata_components::contract::EntryId;
use strata_qa::{Chord, Keymap, Modifier};

use super::{
    host::{Activated, Host},
    input_log::InputLog,
    staged::Staged,
    toplevel::{ChooserParts, Placement, Toplevel},
    tree::{Node, Snapshot},
};
use crate::{App, chooser::geometry::chooser_initial_dimensions};

/// The root path of the floating chooser's tree, beside the browser's `window`.
const FLOATING_ROOT: &str = "floating";
/// Where the floating chooser's corner stands on the virtual screen: right of any browser
/// window the scenarios size.
const FLOATING_X: i16 = 8000;

/// The browser's tree and the chooser's merged with it, kept while neither changes.
struct Merged {
    main: Rc<Snapshot>,
    floating: Rc<Snapshot>,
    both: Rc<Snapshot>,
}

impl Merged {
    /// Whether this merge is of these two trees, which a window keeps while it is unchanged.
    fn is_of(&self, main: &Rc<Snapshot>, floating: &Rc<Snapshot>) -> bool {
        Rc::ptr_eq(&self.main, main) && Rc::ptr_eq(&self.floating, floating)
    }
}

pub(super) struct Windows {
    main: Rc<Staged>,
    log: InputLog,
    /// The browser window's size, which GTK sizes the chooser against.
    parent_size: (i32, i32),
    floating: RefCell<Option<Toplevel>>,
    keymap: RefCell<Option<Keymap>>,
    /// The pointer's last move went to the floating chooser.
    pointer_in_floating: Cell<bool>,
    merged: RefCell<Option<Merged>>,
}

impl Windows {
    pub(super) fn new(main: Staged, log: InputLog, parent_size: (i32, i32)) -> Self {
        Self {
            main: Rc::new(main),
            log,
            parent_size,
            floating: RefCell::default(),
            keymap: RefCell::default(),
            pointer_in_floating: Cell::new(false),
            merged: RefCell::default(),
        }
    }

    /// The browser window's state, for a test to reach behind the steps.
    #[cfg(test)]
    pub(super) fn app<T>(&self, read: impl FnOnce(&App) -> T) -> T {
        self.main.app(read)
    }

    /// Runs `act` on the browser window's state as a handler would, then settles.
    #[cfg(test)]
    pub(super) fn update(&self, act: impl FnOnce(&mut App, &mut rune::Tx)) {
        self.main.update(act);
    }

    /// The floating chooser while it is open.
    fn floating(&self) -> Option<Rc<Staged>> {
        let mut floating = self.floating.borrow_mut();
        if floating.as_ref().is_some_and(Toplevel::is_closed) {
            *floating = None;
            self.pointer_in_floating.set(false);
        }
        floating.as_ref().map(|open| open.staged.clone())
    }

    /// The window that takes the keys: the chooser in front, else the browser.
    fn keyed(&self) -> Rc<Staged> {
        self.floating().unwrap_or_else(|| self.main.clone())
    }

    /// The window that draws `node`.
    fn drawing(&self, node: &Node) -> Rc<Staged> {
        let floating = is_floating(node).then(|| self.floating()).flatten();
        floating.unwrap_or_else(|| self.main.clone())
    }

    /// The window the pointer is over.
    fn pointed(&self) -> Rc<Staged> {
        let floating = self.pointer_in_floating.get().then(|| self.floating());
        floating.flatten().unwrap_or_else(|| self.main.clone())
    }

    /// Opens the chooser the browser asked for, if it asked.
    fn open_requested(&self) -> bool {
        let Some(request) = self.main.take_floating() else {
            return false;
        };
        let keymap = self.keymap.borrow();
        let placement = Placement {
            ui: self.main.ui(),
            log: &self.log,
            keymap: keymap.as_ref(),
            root: FLOATING_ROOT.to_owned(),
            size: chooser_initial_dimensions(None, Some(self.parent_size)),
        };
        let parts = ChooserParts {
            request: request.request,
            source: request.source,
            destination: Some(request.destination),
            completion: request.completion,
        };
        let opened = Toplevel::open(placement, parts);
        drop(keymap);
        *self.floating.borrow_mut() = Some(opened);
        true
    }

    /// One turn of every window, opening a chooser the browser asked for on the way.
    fn turn_all(&self) -> bool {
        let mut worked = self.main.turn();
        worked |= self.open_requested();
        if let Some(floating) = self.floating() {
            worked |= floating.turn();
        }
        worked
    }

    fn merged(&self, floating: &Staged) -> Rc<Snapshot> {
        let (main, other) = (self.main.snapshot(), floating.snapshot());
        let mut kept = self.merged.borrow_mut();
        if let Some(merged) = kept.as_ref().filter(|merged| merged.is_of(&main, &other)) {
            return merged.both.clone();
        }
        let both = Rc::new(main.with_toplevel(&other));
        *kept = Some(Merged {
            main,
            floating: other,
            both: both.clone(),
        });
        both
    }
}

fn is_floating(node: &Node) -> bool {
    node.is_within(&Node(FLOATING_ROOT.into()))
}

impl Host for Windows {
    fn toplevels(&self) -> Vec<Node> {
        let main = self.main.window();
        let floating = self.floating.borrow();
        let floating = floating.as_ref().filter(|open| !open.is_closed());
        main.into_iter()
            .chain(floating.map(|open| open.root.clone()))
            .collect()
    }

    fn title_of(&self, window: &Node) -> Option<String> {
        if !is_floating(window) {
            return self.main.title_of(window);
        }
        let floating = self.floating.borrow();
        floating.as_ref().map(Toplevel::title)
    }

    fn idle_turn(&self) -> bool {
        self.turn_all()
    }

    fn snapshot(&self) -> Rc<Snapshot> {
        match self.floating() {
            Some(floating) => self.merged(&floating),
            None => self.main.snapshot(),
        }
    }

    fn with_app<T: Send + 'static>(
        &self,
        read: impl FnOnce(&App) -> T + Send + 'static,
    ) -> Result<T, String> {
        self.main.with_app(read)
    }

    fn replacements(&self) -> u64 {
        self.main.replacements()
    }

    fn replaced_since(&self, entry: EntryId, since: u64) -> bool {
        self.main.replaced_since(entry, since)
    }

    /// The chooser answers into the browser, which may then ask for another: each settles,
    /// the browser after the chooser.
    async fn settle(&self) {
        if let Some(floating) = self.floating() {
            floating.settle().await;
        }
        self.main.settle().await;
        if self.open_requested()
            && let Some(floating) = self.floating()
        {
            floating.settle().await;
        }
    }

    async fn sleep(&self, time: Duration) {
        let deadline = Instant::now() + time;
        loop {
            let worked = self.turn_all();
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return;
            };
            if left.is_zero() {
                return;
            }
            if !worked {
                self.pause(left);
            }
        }
    }

    fn pause(&self, limit: Duration) {
        self.main.pause(limit);
    }

    fn origin(&self, node: &Node) -> Result<(f64, f64), String> {
        if is_floating(node) {
            return Ok((f64::from(FLOATING_X), 0.0));
        }
        self.main.origin(node)
    }

    async fn resize(&self, width: f32, height: f32) -> Result<(), String> {
        self.main.resize(width, height).await
    }

    fn set_layout(&self, keymap: &Keymap) -> Result<(), String> {
        self.main.set_layout(keymap)?;
        if let Some(floating) = self.floating() {
            floating.set_layout(keymap)?;
        }
        *self.keymap.borrow_mut() = Some(keymap.clone());
        Ok(())
    }

    async fn chord(&self, chord: &Chord) -> Result<(), String> {
        self.keyed().chord(chord).await
    }

    async fn text_input(&self, character: char) -> Result<(), String> {
        self.keyed().text_input(character).await
    }

    async fn hold(&self, modifiers: &[Modifier]) -> Result<(), String> {
        self.keyed().hold(modifiers).await
    }

    async fn release(&self, modifiers: &[Modifier]) {
        self.keyed().release(modifiers).await;
    }

    async fn key(&self, name: &str, pressed: bool) -> Result<(), String> {
        self.keyed().key(name, pressed).await
    }

    async fn pointer_move(&self, x: i16, y: i16) -> Result<(), String> {
        let floating = self.floating().filter(|_| x >= FLOATING_X);
        self.pointer_in_floating.set(floating.is_some());
        match floating {
            Some(floating) => floating.pointer_move(x - FLOATING_X, y).await,
            None => self.main.pointer_move(x, y).await,
        }
    }

    fn pointer_position(&self) -> (i16, i16) {
        let (x, y) = self.pointed().pointer_position();
        if self.pointer_in_floating.get() {
            (x.saturating_add(FLOATING_X), y)
        } else {
            (x, y)
        }
    }

    async fn button(&self, number: u8, pressed: bool) -> Result<(), String> {
        self.pointed().button(number, pressed).await
    }

    fn activate(&self, target: &Activated) -> bool {
        self.drawing(&target.node).activate(target)
    }

    async fn scroll_to(&self, key: &str, x: f32, y: f32) -> Result<(), String> {
        self.main.scroll_to(key, x, y).await
    }
}
