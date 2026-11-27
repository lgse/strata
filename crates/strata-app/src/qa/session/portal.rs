// SPDX-License-Identifier: MIT

use std::{
    cell::{OnceCell, RefCell},
    fs::File,
    rc::Rc,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use strata_components::contract::EntryId;
use strata_core::logging::criticals;
use strata_qa::{Chord, ClipboardContent, Keymap, Modifier};

use super::{
    Session, Target, block_on, guard,
    guard::Admitted,
    link::{Frames, Link},
};
use crate::{
    App,
    host::portal::{self, Hooks, Quit},
    launch,
    qa::{
        backend::Rune,
        host::{Activated, Host},
        input_log::InputLog,
        tree::{Node, Snapshot},
    },
    startup::{self, UiThread},
};

const NO_CHOOSER: &str = "no chooser window is open";

/// A chooser window the daemon opened: its request's token, and how the session reaches it.
struct Opened {
    token: String,
    target: Target,
}

/// The chooser windows open now, kept by the windows' thread and read by the session's.
#[derive(Clone, Default)]
struct Registry(Arc<Mutex<Vec<Opened>>>);

impl Registry {
    fn with<T>(&self, read: impl FnOnce(&mut Vec<Opened>) -> T) -> T {
        read(&mut self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }

    fn target(&self, root: &str) -> Option<Target> {
        self.with(|opened| {
            let open = opened.iter().find(|open| open.target.root == root);
            open.map(|open| open.target.clone())
        })
    }

    /// The newest window, which the steps see until one is adopted, as on the offscreen host.
    fn newest(&self) -> Option<Target> {
        self.with(|opened| opened.last().map(|open| open.target.clone()))
    }

    fn roots(&self) -> Vec<String> {
        self.with(|opened| opened.iter().map(|open| open.target.root.clone()).collect())
    }
}

pub(in crate::qa) fn serve() -> Result<(), String> {
    // Before any window opens: a refused session touches no display.
    let admitted = guard::admit(|name| std::env::var(name).ok())?;
    // Before anything can log there: stdout is the replies' from here on.
    let protocol = strata_qa::reply_stream()?;
    criticals::capture();
    let settings = startup::initialize_process();
    launch::leave_launch_folder();
    let log = InputLog::from_environment()?;
    let registry = Registry::default();
    let ui = UiThread::claim()?;
    let hooks = hooks(&registry, (protocol, log, admitted));
    portal::run_with(ui, settings, hooks)
}

fn hooks(registry: &Registry, (protocol, log, admitted): (File, InputLog, Admitted)) -> Hooks {
    let opening = registry.clone();
    let opened = Rc::new(std::cell::Cell::new(0_u64));
    let closing = registry.clone();
    let serving = registry.clone();
    Hooks {
        started: Some(Box::new(move |quit| {
            start((serving, log, admitted), protocol, quit);
        })),
        chooser: Some(Box::new(move |app, token| {
            opened.set(opened.get() + 1);
            let frames = Arc::new(Frames::default());
            let recorder = Arc::clone(&frames);
            let target = Target {
                link: Link::new(app.handle(), frames),
                root: format!("window-{}", opened.get()),
            };
            let token = token.to_owned();
            opening.with(|opened| opened.push(Opened { token, target }));
            app.on_frame_drawn(move |runtime| recorder.record(runtime))
        })),
        closed: Some(Box::new(move |token| {
            closing.with(|opened| opened.retain(|open| open.token != token));
        })),
        stays: true,
    }
}

/// Starts the session's thread, which ends the daemon when the session ends.
fn start((registry, log, admitted): (Registry, InputLog, Admitted), protocol: File, quit: Quit) {
    let spawned = std::thread::Builder::new()
        .name("qa-session".to_owned())
        .spawn(move || {
            let backend = Rune::on(PortalSession::new(registry, log, admitted));
            let commands = strata_qa::command_lines();
            block_on(&backend, strata_qa::serve(&backend, protocol, commands));
            quit();
        });
    if let Err(error) = spawned {
        eprintln!("strata: cannot start the QA session: {error}");
    }
}

pub(in crate::qa) struct PortalSession {
    registry: Registry,
    /// What the session connects with at the first `adopt`.
    waiting: RefCell<Option<(InputLog, Admitted)>>,
    session: OnceCell<Session>,
    adopted: RefCell<Option<String>>,
}

impl PortalSession {
    fn new(registry: Registry, log: InputLog, admitted: Admitted) -> Self {
        Self {
            registry,
            waiting: RefCell::new(Some((log, admitted))),
            session: OnceCell::new(),
            adopted: RefCell::default(),
        }
    }

    /// The session, on the adopted chooser while it is open.
    fn adopted(&self) -> Result<&Session, String> {
        let open = self
            .adopted
            .borrow()
            .as_deref()
            .is_some_and(|root| self.registry.target(root).is_some());
        open.then(|| self.session.get())
            .flatten()
            .ok_or_else(|| NO_CHOOSER.to_owned())
    }

    /// Acts on `target`: the session connects on the first, and follows it after.
    fn act_on(&self, target: Target) -> Result<(), String> {
        if let Some(session) = self.session.get() {
            return session.retarget(target);
        }
        let (log, admitted) = self
            .waiting
            .borrow_mut()
            .take()
            .ok_or("the session host could not connect before")?;
        let session = Session::connect(target, log, admitted)?;
        let _ = self.session.set(session);
        Ok(())
    }
}

impl Host for PortalSession {
    fn windowless(&self) -> bool {
        true
    }

    fn window(&self) -> Option<Node> {
        let adopted = self.adopted.borrow().clone()?;
        self.registry.target(&adopted).map(|_| Node(adopted.into()))
    }

    fn toplevels(&self) -> Vec<Node> {
        let roots = self.registry.roots();
        roots.into_iter().map(|root| Node(root.into())).collect()
    }

    fn title_of(&self, window: &Node) -> Option<String> {
        let target = self.registry.target(&window.0)?;
        target.link.ask(|app| app.title().to_owned()).ok()
    }

    fn adopt(&self, window: &Node) {
        let Some(target) = self.registry.target(&window.0) else {
            return;
        };
        match self.act_on(target) {
            Ok(()) => *self.adopted.borrow_mut() = Some(window.0.to_string()),
            Err(error) => eprintln!("strata: the session host: adopting a chooser: {error}"),
        }
    }

    fn snapshot(&self) -> Rc<Snapshot> {
        if let Ok(session) = self.adopted() {
            return session.snapshot();
        }
        let frame = self
            .registry
            .newest()
            .and_then(|newest| Some((newest.link.frames.latest()?, newest.root)));
        frame.map_or_else(Rc::default, |(frame, root)| {
            Rc::new(Snapshot::read_named(&root, &frame.tree, &frame.drawn))
        })
    }

    fn with_app<T: Send + 'static>(
        &self,
        read: impl FnOnce(&App) -> T + Send + 'static,
    ) -> Result<T, String> {
        self.adopted()?.with_app(read)
    }

    fn replacements(&self) -> u64 {
        self.adopted().map_or(0, Host::replacements)
    }

    fn replaced_since(&self, entry: EntryId, since: u64) -> bool {
        self.adopted()
            .is_ok_and(|session| session.replaced_since(entry, since))
    }

    async fn settle(&self) {
        if let Ok(session) = self.adopted() {
            session.settle().await;
        }
    }

    async fn sleep(&self, time: Duration) {
        std::thread::sleep(time);
    }

    fn pause(&self, limit: Duration) {
        std::thread::sleep(limit);
    }

    fn origin(&self, node: &Node) -> Result<(f64, f64), String> {
        self.adopted()?.origin(node)
    }

    async fn resize(&self, width: f32, height: f32) -> Result<(), String> {
        self.adopted()?.resize(width, height).await
    }

    fn set_layout(&self, keymap: &Keymap) -> Result<(), String> {
        match self.session.get() {
            Some(session) => session.set_layout(keymap),
            None => super::keymap_is_us(keymap),
        }
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
        if let Ok(session) = self.adopted() {
            session.release(modifiers).await;
        }
    }

    async fn key(&self, name: &str, pressed: bool) -> Result<(), String> {
        self.adopted()?.key(name, pressed).await
    }

    async fn pointer_move(&self, x: i16, y: i16) -> Result<(), String> {
        self.adopted()?.pointer_move(x, y).await
    }

    fn pointer_position(&self) -> (i16, i16) {
        self.session.get().map_or((0, 0), Host::pointer_position)
    }

    async fn button(&self, number: u8, pressed: bool) -> Result<(), String> {
        self.adopted()?.button(number, pressed).await
    }

    fn activate(&self, target: &Activated) -> bool {
        self.adopted().is_ok_and(|session| session.activate(target))
    }

    async fn scroll_to(&self, at: (&Node, &str), x: f32, y: f32) -> Result<(), String> {
        self.adopted()?.scroll_to(at, x, y).await
    }

    async fn set_clipboard(&self, content: &ClipboardContent) -> Option<Result<(), String>> {
        match self.adopted() {
            Ok(session) => session.set_clipboard(content).await,
            Err(error) => Some(Err(error)),
        }
    }

    async fn read_clipboard(&self, mime: Option<&str>) -> Option<Result<Option<String>, String>> {
        match self.adopted() {
            Ok(session) => session.read_clipboard(mime).await,
            Err(error) => Some(Err(error)),
        }
    }
}
