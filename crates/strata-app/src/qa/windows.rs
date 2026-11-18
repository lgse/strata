// SPDX-License-Identifier: MIT

//! The offscreen host with the windows later launches ask for, beside the run's own.
//!
//! The QA process is the primary on the runner's private session bus, as the GTK application's
//! QA process is its GApplication's (`crate::application`, claimed as a service, so that its
//! own launch opens nothing: its folder is the run's). A `launch` step's `strata-rune` hands its
//! arguments over and exits, and each argument becomes a window here, offscreen like the first,
//! sharing its settings, as `crate::host::windowed` opens a real one.
//!
//! The steps act on the run's window, which has the root key `window`, as they do with one. The
//! others are toplevels the engine reads with `windows`, `some_window` and `other_window`, under
//! roots of their own (`window-2`, `window-3`…), and a node is read from the window it is in. An
//! `activate` aimed inside one presses it there.
//!
//! Each window's turn dispatches the process's one GLib context, and the host takes what the
//! dispatch asked for after every turn, as the portal host takes its requests.

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
    tree::{Node, Snapshot},
};
use crate::{App, application, host::offscreen::Offscreen, launch, shell, startup::UiThread};

/// The longest a pause waits while other windows are open: their timers are not the run's
/// window's, which is what a pause waits on.
const OTHERS_PAUSE: Duration = Duration::from_millis(20);

/// A window a later launch opened.
struct Open {
    root: Node,
    staged: Rc<Staged>,
}

pub(super) struct Windows {
    ui: UiThread,
    log: InputLog,
    size: (f32, f32),
    main: Staged,
    others: RefCell<Vec<Open>>,
    opened: Cell<u64>,
    keymap: RefCell<Option<Keymap>>,
}

impl Windows {
    pub(super) fn new(ui: UiThread, main: Staged, (log, size): (InputLog, (f32, f32))) -> Self {
        Self {
            ui,
            log,
            size,
            main,
            others: RefCell::default(),
            opened: Cell::new(1),
            keymap: RefCell::default(),
        }
    }

    fn others(&self) -> Vec<Rc<Staged>> {
        let others = self.others.borrow();
        others.iter().map(|open| Rc::clone(&open.staged)).collect()
    }

    fn other(&self, window: &Node) -> Option<Rc<Staged>> {
        let others = self.others.borrow();
        let open = others.iter().find(|open| open.root == *window);
        open.map(|open| Rc::clone(&open.staged))
    }

    /// Opens a window for each launch the bus handed over since the last look. True when any
    /// opened.
    fn open_asked(&self) -> bool {
        let asked = application::take();
        let opened = !asked.is_empty();
        for opening in asked {
            self.open(opening);
        }
        opened
    }

    fn open(&self, opening: application::Opening) {
        let id = self.opened.get() + 1;
        self.opened.set(id);
        let root = format!("window-{id}");
        let settings = self.main.app(shell::share_settings);
        let initial = launch::initial_for(opening, &settings);
        let app = App::new(
            &self.ui,
            Csd::new(Policy::Auto.frame(), Backend::Headless),
            settings,
        );
        let mut host = Offscreen::from_app(self.ui, app, self.size).without_animations();
        host.start(initial);
        let staged = Rc::new(Staged::named(root.clone(), host, self.log.clone()));
        if let Some(keymap) = self.keymap.borrow().as_ref() {
            let _ = staged.set_layout(keymap);
        }
        staged.turn();
        self.others.borrow_mut().push(Open {
            root: Node(root.into()),
            staged,
        });
    }

    /// One turn of every window, then the windows that turn asked for.
    fn turn_all(&self) -> bool {
        let mut worked = self.main.turn();
        for other in self.others() {
            worked |= other.turn();
        }
        worked | self.open_asked()
    }
}

impl Host for Windows {
    fn toplevels(&self) -> Vec<Node> {
        let main = self.main.window();
        let others = self.others.borrow();
        main.into_iter()
            .chain(others.iter().map(|open| open.root.clone()))
            .collect()
    }

    fn title_of(&self, window: &Node) -> Option<String> {
        match self.other(window) {
            Some(other) => Some(other.app(|app| app.title().to_owned())),
            None => self.main.title_of(window),
        }
    }

    fn idle_turn(&self) -> bool {
        self.turn_all()
    }

    fn snapshot(&self) -> Rc<Snapshot> {
        self.main.snapshot()
    }

    fn snapshot_of(&self, node: &Node) -> Rc<Snapshot> {
        match self.other(&node.window()) {
            Some(other) => other.snapshot(),
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

    /// Every window rests: a step in one may change another, as a setting does.
    async fn settle(&self) {
        self.open_asked();
        for other in self.others() {
            other.settle().await;
        }
        self.main.settle().await;
        if self.open_asked() {
            for other in self.others() {
                other.settle().await;
            }
        }
    }

    async fn sleep(&self, time: Duration) {
        if self.others.borrow().is_empty() {
            self.main.sleep(time).await;
            self.open_asked();
            return;
        }
        let deadline = Instant::now() + time;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            if left.is_zero() {
                return;
            }
            if !self.turn_all() {
                self.pause(left);
            }
        }
    }

    fn pause(&self, limit: Duration) {
        let limit = if self.others.borrow().is_empty() {
            limit
        } else {
            limit.min(OTHERS_PAUSE)
        };
        self.main.pause(limit);
    }

    fn origin(&self) -> Result<(f64, f64), String> {
        self.main.origin()
    }

    async fn resize(&self, width: f32, height: f32) -> Result<(), String> {
        self.main.resize(width, height).await
    }

    fn set_layout(&self, keymap: &Keymap) -> Result<(), String> {
        for other in self.others() {
            other.set_layout(keymap)?;
        }
        *self.keymap.borrow_mut() = Some(keymap.clone());
        self.main.set_layout(keymap)
    }

    async fn chord(&self, chord: &Chord) -> Result<(), String> {
        self.main.chord(chord).await
    }

    async fn text_input(&self, character: char) -> Result<(), String> {
        self.main.text_input(character).await
    }

    async fn hold(&self, modifiers: &[Modifier]) -> Result<(), String> {
        self.main.hold(modifiers).await
    }

    async fn release(&self, modifiers: &[Modifier]) {
        self.main.release(modifiers).await;
    }

    async fn key(&self, name: &str, pressed: bool) -> Result<(), String> {
        self.main.key(name, pressed).await
    }

    async fn pointer_move(&self, x: i16, y: i16) -> Result<(), String> {
        self.main.pointer_move(x, y).await
    }

    fn pointer_position(&self) -> (i16, i16) {
        self.main.pointer_position()
    }

    async fn button(&self, number: u8, pressed: bool) -> Result<(), String> {
        self.main.button(number, pressed).await
    }

    fn activate(&self, target: &Activated) -> bool {
        match self.other(&target.window) {
            Some(other) => other.activate(target),
            None => self.main.activate(target),
        }
    }

    async fn scroll_to(&self, key: &str, x: f32, y: f32) -> Result<(), String> {
        self.main.scroll_to(key, x, y).await
    }
}
