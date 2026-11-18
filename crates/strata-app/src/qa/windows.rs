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
//!
//! # Windows a launch opens
//!
//! The QA process is the primary on the runner's private session bus, as the GTK application's
//! QA process is its GApplication's (`crate::application`, claimed as a service, so that its
//! own launch opens nothing: its folder is the run's). A `launch` step's `strata-rune` hands its
//! arguments over and exits, and each argument becomes a browser window here, offscreen like the
//! first, sharing its settings, as `crate::host::windowed` opens a real one (A21). Their trees
//! stand beside the browser's under roots of their own (`window-2`, `window-3`…), behind it:
//! the steps act on the run's window, which keeps the keys, the pointer and the focus, while a
//! target `within` another's toplevel (`other_window`, `some_window`) finds its nodes, and an
//! `activate` aimed at one presses it there.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

use strata_components::contract::EntryId;
use strata_platform::csd::{Backend, Csd, Policy};
use strata_qa::{Chord, Keymap, Modifier};

use super::{
    host::{Activated, Host},
    input_log::InputLog,
    staged::Staged,
    toplevel::{ChooserParts, Placement, Toplevel},
    tree::{Node, Snapshot},
};
use crate::{
    App, application, chooser::geometry::chooser_initial_dimensions, host::offscreen::Offscreen,
    launch, shell,
};

/// The root path of the floating chooser's tree, beside the browser's `window`.
const FLOATING_ROOT: &str = "floating";
/// Where the floating chooser's corner stands on the virtual screen: right of any browser
/// window the scenarios size.
const FLOATING_X: i16 = 8000;

/// The longest a pause waits while a launch's windows are open beside the browser: their
/// timers are not the browser's, which is what a pause waits on.
const OTHERS_PAUSE: Duration = Duration::from_millis(20);

/// The windows' trees merged, kept while none changes: the browser's, a launch's windows' and
/// the chooser's, in that order.
struct Merged {
    parts: Vec<Rc<Snapshot>>,
    both: Rc<Snapshot>,
}

impl Merged {
    /// Whether this merge is of these trees, which a window keeps while it is unchanged.
    fn is_of(&self, parts: &[Rc<Snapshot>]) -> bool {
        self.parts.len() == parts.len()
            && self
                .parts
                .iter()
                .zip(parts)
                .all(|(kept, part)| Rc::ptr_eq(kept, part))
    }
}

/// A browser window a later launch opened.
struct Launched {
    root: Node,
    staged: Rc<Staged>,
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
    /// The browser windows later launches opened, and how many windows have opened.
    launched: RefCell<Vec<Launched>>,
    opened: Cell<u64>,
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
            launched: RefCell::default(),
            opened: Cell::new(1),
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
        floating
            .or_else(|| self.launched_drawing(node))
            .unwrap_or_else(|| self.main.clone())
    }

    /// The launch's window `node` is in, if it is in one.
    fn launched_drawing(&self, node: &Node) -> Option<Rc<Staged>> {
        let launched = self.launched.borrow();
        let open = launched.iter().find(|open| node.is_within(&open.root));
        open.map(|open| open.staged.clone())
    }

    fn launched(&self) -> Vec<Rc<Staged>> {
        let launched = self.launched.borrow();
        launched.iter().map(|open| open.staged.clone()).collect()
    }

    /// Opens a browser window for each launch the bus handed over since the last look. True
    /// when any opened.
    fn open_launched(&self) -> bool {
        let asked = application::take();
        let opened = !asked.is_empty();
        for opening in asked {
            self.open_launch(opening);
        }
        opened
    }

    fn open_launch(&self, opening: application::Opening) {
        let id = self.opened.get() + 1;
        self.opened.set(id);
        let root = format!("window-{id}");
        let ui = self.main.ui();
        let settings = self.main.app(shell::share_settings);
        let initial = launch::initial_for(opening, &settings);
        let csd = Csd::new(Policy::Auto.frame(), Backend::Headless);
        let app = App::new(&ui, csd, settings);
        #[expect(clippy::cast_precision_loss, reason = "a window's size fits f32")]
        let size = (self.parent_size.0 as f32, self.parent_size.1 as f32);
        let mut host = Offscreen::from_app(ui, app, size).without_animations();
        host.start(initial);
        let staged = Rc::new(Staged::named(root.clone(), host, self.log.clone()));
        if let Some(keymap) = self.keymap.borrow().as_ref() {
            let _ = staged.set_layout(keymap);
        }
        staged.turn();
        self.launched.borrow_mut().push(Launched {
            root: Node(root.into()),
            staged,
        });
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
            size: chooser_initial_dimensions(
                None,
                request.request.parent_size_hint.or(Some(self.parent_size)),
            ),
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

    /// One turn of every window, opening a chooser the browser asked for and the windows a
    /// launch asked for on the way.
    fn turn_all(&self) -> bool {
        let mut worked = self.main.turn();
        for launched in self.launched() {
            worked |= launched.turn();
        }
        worked |= self.open_requested();
        if let Some(floating) = self.floating() {
            worked |= floating.turn();
        }
        worked | self.open_launched()
    }

    /// The browser's tree with the launch's windows' behind it and the chooser's in front.
    fn merged(&self, floating: Option<&Staged>) -> Rc<Snapshot> {
        let mut parts = vec![self.main.snapshot()];
        parts.extend(self.launched().iter().map(|launched| launched.snapshot()));
        parts.extend(floating.map(Staged::snapshot));
        let mut kept = self.merged.borrow_mut();
        if let Some(merged) = kept.as_ref().filter(|merged| merged.is_of(&parts)) {
            return merged.both.clone();
        }
        let last = parts.len() - 1;
        let mut both = Snapshot::default();
        for (at, part) in parts.iter().enumerate() {
            both = match at {
                0 => continue,
                1 => join(&parts[0], part, floating.is_some() && at == last),
                _ => join(&both, part, floating.is_some() && at == last),
            };
        }
        let both = Rc::new(both);
        *kept = Some(Merged {
            parts,
            both: both.clone(),
        });
        both
    }
}

fn is_floating(node: &Node) -> bool {
    node.is_within(&Node(FLOATING_ROOT.into()))
}

/// `other`'s tree beside `base`'s: in front, with the focus, or behind.
fn join(base: &Snapshot, other: &Snapshot, in_front: bool) -> Snapshot {
    if in_front {
        base.with_toplevel(other)
    } else {
        base.with_window_behind(other)
    }
}

impl Host for Windows {
    fn toplevels(&self) -> Vec<Node> {
        let main = self.main.window();
        let launched = self.launched.borrow();
        let floating = self.floating.borrow();
        let floating = floating.as_ref().filter(|open| !open.is_closed());
        main.into_iter()
            .chain(launched.iter().map(|open| open.root.clone()))
            .chain(floating.map(|open| open.root.clone()))
            .collect()
    }

    fn title_of(&self, window: &Node) -> Option<String> {
        if let Some(launched) = self.launched_drawing(window) {
            return Some(launched.app(|app| app.title().to_owned()));
        }
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
        let floating = self.floating();
        if floating.is_none() && self.launched.borrow().is_empty() {
            return self.main.snapshot();
        }
        self.merged(floating.as_deref())
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
        self.open_launched();
        if let Some(floating) = self.floating() {
            floating.settle().await;
        }
        for launched in self.launched() {
            launched.settle().await;
        }
        self.main.settle().await;
        if self.open_requested()
            && let Some(floating) = self.floating()
        {
            floating.settle().await;
        }
        if self.open_launched() {
            for launched in self.launched() {
                launched.settle().await;
            }
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
        let limit = if self.launched.borrow().is_empty() {
            limit
        } else {
            limit.min(OTHERS_PAUSE)
        };
        self.main.pause(limit);
    }

    fn origin(&self, node: &Node) -> Result<(f64, f64), String> {
        if is_floating(node) {
            return Ok((f64::from(FLOATING_X), 0.0));
        }
        self.drawing(node).origin(node)
    }

    async fn resize(&self, width: f32, height: f32) -> Result<(), String> {
        self.main.resize(width, height).await
    }

    fn set_layout(&self, keymap: &Keymap) -> Result<(), String> {
        self.main.set_layout(keymap)?;
        for launched in self.launched() {
            launched.set_layout(keymap)?;
        }
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

    async fn scroll_to(&self, at: (&Node, &str), x: f32, y: f32) -> Result<(), String> {
        self.drawing(at.0).scroll_to(at, x, y).await
    }
}
