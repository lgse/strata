// SPDX-License-Identifier: MIT

//! The `--qa --portal` host: the portal service on the session bus the runner provides, and
//! one offscreen chooser per request it hears. It starts with no window, as the GTK backend's
//! `Host::Portal` does: `adopt` waits for a chooser with the title, and the steps after it act
//! on that chooser until it closes.
//!
//! The service's handlers only queue ([`PortalInbox`]); a turn of this host takes the queue
//! after GLib's dispatch returns. Each chooser is an `Rc<Staged>` that a step clones before it
//! acts, so a step that finishes the request, which closes the chooser, never drops the host
//! it runs on.

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
use crate::{
    App,
    chooser::{geometry::chooser_initial_dimensions, source::ChooserFileSource},
    portal::{
        ChooserRequest, Completion, PortalError,
        dbus::PortalService,
        inbox::{PortalEvent, PortalInbox},
    },
    startup::{self, UiThread},
};

/// The QA runner's screen (Xvfb at 1440x900), which the GTK chooser sizes itself against.
const SCREEN: (i32, i32) = (1440, 900);
const NO_CHOOSER: &str = "no chooser window is open";

pub(super) struct PortalStaged {
    ui: UiThread,
    log: InputLog,
    inbox: PortalInbox,
    _service: PortalService,
    context: glib::MainContext,
    choosers: RefCell<Vec<Toplevel>>,
    adopted: RefCell<Option<Node>>,
    opened: Cell<u64>,
    keymap: RefCell<Option<Keymap>>,
}

impl PortalStaged {
    pub(super) fn start(ui: UiThread, log: InputLog) -> Result<Self, String> {
        let inbox = PortalInbox::default();
        // The idle loop takes the queue on its next turn: nothing to wake.
        let service = PortalService::start(inbox.callbacks(Rc::new(|| {})))?;
        Ok(Self {
            ui,
            log,
            inbox,
            _service: service,
            context: glib::MainContext::default(),
            choosers: RefCell::default(),
            adopted: RefCell::default(),
            opened: Cell::new(0),
            keymap: RefCell::default(),
        })
    }

    /// The chooser the steps act on: the adopted one, else the newest, while it is open.
    fn current(&self) -> Option<Rc<Staged>> {
        self.prune();
        let adopted = self.adopted.borrow().clone();
        let choosers = self.choosers.borrow();
        let open = match adopted {
            Some(root) => choosers.iter().find(|open| open.root == root),
            None => choosers.last(),
        };
        open.map(|open| open.staged.clone())
    }

    fn adopted(&self) -> Result<Rc<Staged>, String> {
        let adopted = self.adopted.borrow().is_some();
        adopted
            .then(|| self.current())
            .flatten()
            .ok_or_else(|| NO_CHOOSER.to_owned())
    }

    /// Forgets the choosers that have closed.
    fn prune(&self) {
        let closed: Vec<Toplevel> = {
            let mut choosers = self.choosers.borrow_mut();
            let (closed, open) = std::mem::take(&mut *choosers)
                .into_iter()
                .partition(Toplevel::is_closed);
            *choosers = open;
            closed
        };
        let mut adopted = self.adopted.borrow_mut();
        if closed
            .iter()
            .any(|open| adopted.as_ref() == Some(&open.root))
        {
            *adopted = None;
        }
    }

    fn all(&self) -> Vec<Rc<Staged>> {
        self.prune();
        let choosers = self.choosers.borrow();
        choosers.iter().map(|open| open.staged.clone()).collect()
    }

    /// Dispatches what GLib has ready, then runs what the service queued.
    fn dispatch(&self) -> bool {
        let started = Instant::now();
        let mut worked = false;
        while self.context.iteration(false) {
            worked = true;
            if started.elapsed() >= crate::host::DISPATCH_BUDGET {
                break;
            }
        }
        for event in self.inbox.take() {
            worked = true;
            self.heard(event);
        }
        worked
    }

    fn heard(&self, event: PortalEvent) {
        match event {
            PortalEvent::Present {
                cancelled,
                completion,
                ..
            } if cancelled.load(std::sync::atomic::Ordering::SeqCst) => {
                completion(Err(PortalError::Cancelled(
                    "file chooser request was cancelled".into(),
                )));
            }
            PortalEvent::Present {
                request,
                completion,
                ..
            } => self.present(*request, completion),
            PortalEvent::Cancel(token) => self.cancel(&token),
            // The runner's private bus never takes the name away.
            PortalEvent::Quit => {}
        }
    }

    fn present(&self, request: ChooserRequest, completion: Completion) {
        let id = self.opened.get() + 1;
        self.opened.set(id);
        let keymap = self.keymap.borrow();
        let placement = Placement {
            ui: self.ui,
            log: &self.log,
            keymap: keymap.as_ref(),
            root: format!("window-{id}"),
            size: chooser_initial_dimensions(Some(SCREEN), request.parent_size_hint),
        };
        let parts = ChooserParts {
            settings: startup::load_settings(),
            request,
            source: ChooserFileSource::new(None),
            destination: None,
            completion,
        };
        let opened = Toplevel::open(placement, parts);
        drop(keymap);
        self.choosers.borrow_mut().push(opened);
    }

    fn cancel(&self, token: &str) {
        let staged = {
            let choosers = self.choosers.borrow();
            let open = choosers.iter().find(|open| open.token == token);
            open.map(|open| open.staged.clone())
        };
        if let Some(staged) = staged {
            staged.update(|app, tx| {
                crate::chooser::cancel(app, tx);
            });
        }
    }
}

impl Host for PortalStaged {
    fn windowless(&self) -> bool {
        true
    }

    fn window(&self) -> Option<Node> {
        self.prune();
        self.adopted.borrow().clone()
    }

    fn toplevels(&self) -> Vec<Node> {
        self.prune();
        let choosers = self.choosers.borrow();
        choosers.iter().map(|open| open.root.clone()).collect()
    }

    fn title_of(&self, window: &Node) -> Option<String> {
        let staged = {
            let choosers = self.choosers.borrow();
            let open = choosers.iter().find(|open| &open.root == window);
            open.map(|open| open.staged.clone())
        }?;
        Some(staged.app(|app| app.title().to_owned()))
    }

    fn adopt(&self, window: &Node) {
        *self.adopted.borrow_mut() = Some(window.clone());
    }

    fn idle_turn(&self) -> bool {
        let mut worked = self.dispatch();
        for staged in self.all() {
            worked |= staged.turn();
        }
        worked
    }

    fn snapshot(&self) -> Rc<Snapshot> {
        self.current()
            .map_or_else(|| Rc::new(Snapshot::default()), |staged| staged.snapshot())
    }

    fn with_app<T: Send + 'static>(
        &self,
        read: impl FnOnce(&App) -> T + Send + 'static,
    ) -> Result<T, String> {
        self.adopted()?.with_app(read)
    }

    fn replacements(&self) -> u64 {
        self.current().map_or(0, |staged| staged.replacements())
    }

    fn replaced_since(&self, entry: EntryId, since: u64) -> bool {
        self.current()
            .is_some_and(|staged| staged.replaced_since(entry, since))
    }

    async fn settle(&self) {
        self.dispatch();
        if let Some(staged) = self.current() {
            staged.settle().await;
        }
    }

    async fn sleep(&self, time: Duration) {
        let deadline = Instant::now() + time;
        loop {
            let worked = self.idle_turn();
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
        if let Some(staged) = self.current() {
            staged.pause(limit);
            return;
        }
        let timer = glib::timeout_source_new(limit, None, glib::Priority::DEFAULT, || {
            glib::ControlFlow::Break
        });
        timer.attach(Some(&self.context));
        self.context.iteration(true);
        timer.destroy();
    }

    fn origin(&self, node: &Node) -> Result<(f64, f64), String> {
        self.adopted()?.origin(node)
    }

    async fn resize(&self, width: f32, height: f32) -> Result<(), String> {
        self.adopted()?.resize(width, height).await
    }

    fn set_layout(&self, keymap: &Keymap) -> Result<(), String> {
        for staged in self.all() {
            staged.set_layout(keymap)?;
        }
        *self.keymap.borrow_mut() = Some(keymap.clone());
        Ok(())
    }

    async fn chord(&self, chord: &Chord) -> Result<(), String> {
        self.adopted()?.chord(chord).await
    }

    async fn text_input(&self, character: char) -> Result<(), String> {
        self.adopted()?.text_input(character).await
    }

    async fn hold(&self, modifiers: &[Modifier]) -> Result<(), String> {
        self.adopted()?.hold(modifiers).await
    }

    async fn release(&self, modifiers: &[Modifier]) {
        if let Ok(staged) = self.adopted() {
            staged.release(modifiers).await;
        }
    }

    async fn key(&self, name: &str, pressed: bool) -> Result<(), String> {
        self.adopted()?.key(name, pressed).await
    }

    async fn pointer_move(&self, x: i16, y: i16) -> Result<(), String> {
        self.adopted()?.pointer_move(x, y).await
    }

    fn pointer_position(&self) -> (i16, i16) {
        self.current()
            .map_or((0, 0), |staged| staged.pointer_position())
    }

    async fn button(&self, number: u8, pressed: bool) -> Result<(), String> {
        self.adopted()?.button(number, pressed).await
    }

    fn activate(&self, target: &Activated) -> bool {
        self.adopted().is_ok_and(|staged| staged.activate(target))
    }

    async fn scroll_to(&self, at: (&Node, &str), x: f32, y: f32) -> Result<(), String> {
        self.adopted()?.scroll_to(at, x, y).await
    }
}
