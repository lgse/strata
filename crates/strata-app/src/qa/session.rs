// SPDX-License-Identifier: MIT

mod clipboard;
mod devices;
mod guard;
mod link;
mod portal;
mod sway;
#[cfg(test)]
mod tests;

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    ffi::OsStr,
    fs::File,
    future::Future,
    pin::pin,
    rc::Rc,
    sync::Arc,
    task::{Context, Wake, Waker},
    thread::Thread,
    time::{Duration, Instant},
};

use strata_components::contract::EntryId;
use strata_core::logging::criticals;
use strata_qa::{Chord, ClipboardContent, Keymap, Modifier};

use self::{
    devices::{Devices, UsKey, modifier_key},
    guard::Admitted,
    link::{Frames, Link},
    sway::Sway,
};
use super::{
    backend::Rune,
    host::{Activated, Host},
    input_log::{InputLog, modifier_name},
    keymap,
    replaced::Replacements,
    tree::{Node, Snapshot},
};
use crate::{
    App,
    host::windowed,
    launch,
    startup::{self, UiThread},
};

/// What the runner sets to have the QA mode open a real window.
const VARIABLE: &str = "STRATA_QA_WINDOW";
const SETTLE_LIMIT: Duration = Duration::from_millis(250);
const DISPATCH_LIMIT: Duration = Duration::from_millis(500);
const DISPATCH_POLL: Duration = Duration::from_millis(2);
const DRAG_MOTION_PACE: Duration = Duration::from_millis(16);
/// How long the window may take to show in the admitted sway.
const WINDOW_LIMIT: Duration = Duration::from_secs(30);
const WINDOW_POLL: Duration = Duration::from_millis(20);
/// The longest the session thread sleeps without polling the session again.
const IDLE_WAIT: Duration = Duration::from_secs(1);

thread_local! {
    /// Each window's replacements, by its tree's root, counted on the windows' thread.
    static REPLACEMENTS: RefCell<HashMap<String, Rc<Replacements>>> =
        RefCell::new(HashMap::new());
}

/// The window the session acts on: its link, and the root its tree is read under.
#[derive(Clone)]
struct Target {
    link: Link,
    root: String,
}

impl Target {
    /// The browser window, the only one a folder's session has.
    fn browser(link: Link) -> Self {
        Self {
            link,
            root: "window".to_owned(),
        }
    }

    fn prepare(&self) -> Result<(), String> {
        let root = self.root.clone();
        self.link.change(move |app| {
            app.look.animations = false;
            replacements_of(&root, app);
        })
    }
}

fn replacements_of(root: &str, app: &App) -> Rc<Replacements> {
    REPLACEMENTS.with(|watched| {
        Rc::clone(
            watched
                .borrow_mut()
                .entry(root.to_owned())
                .or_insert_with(|| Replacements::watch(app)),
        )
    })
}

/// Whether the runner asks for the session host.
pub(super) fn wanted() -> bool {
    std::env::var_os(VARIABLE).is_some_and(|value| value == "1")
}

pub(super) use portal::serve as serve_portal;

pub(super) fn serve(directory: &OsStr) -> Result<(), String> {
    // Before the window opens: a refused session touches no display.
    let admitted = guard::admit(|name| std::env::var(name).ok())?;
    // Before anything can log there: stdout is the replies' from here on.
    let protocol = strata_qa::reply_stream()?;
    criticals::capture();
    let settings = startup::initialize_process();
    let initial = super::initial(directory, &settings)?;
    launch::leave_launch_folder();
    let log = InputLog::from_environment()?;
    let frames = Arc::new(Frames::default());
    let recorder = Arc::clone(&frames);
    let ui = UiThread::claim()?;
    windowed::run_with(ui, settings, initial, move |app| {
        let link = Link::new(app.handle(), frames);
        start(Target::browser(link), protocol, log, admitted);
        app.on_frame_drawn(move |runtime| recorder.record(runtime))
    })
}

/// Starts the session's thread. Requests it makes before the loop runs wait for it.
fn start(target: Target, protocol: File, log: InputLog, admitted: Admitted) {
    let link = target.link.clone();
    let spawned = std::thread::Builder::new()
        .name("qa-session".to_owned())
        .spawn(move || {
            match Session::connect(target, log, admitted) {
                Ok(host) => {
                    let backend = Rune::on(host);
                    let commands = strata_qa::command_lines();
                    block_on(&backend, strata_qa::serve(&backend, protocol, commands));
                }
                Err(error) => eprintln!("strata: the session host: {error}"),
            }
            link.quit();
        });
    if let Err(error) = spawned {
        eprintln!("strata: cannot start the QA session: {error}");
    }
}

/// Waits until `sway` lists this process's window, for [`WINDOW_LIMIT`] at most.
fn own_window(sway: &Sway) -> Result<(), String> {
    let deadline = Instant::now() + WINDOW_LIMIT;
    loop {
        match sway.window(std::process::id()) {
            Ok(_) => return Ok(()),
            Err(error) if Instant::now() >= deadline => {
                return Err(format!(
                    "the sway of SWAYSOCK did not show this process's window within {} s, so \
                     WAYLAND_DISPLAY may be another compositor: {error}",
                    WINDOW_LIMIT.as_secs()
                ));
            }
            Err(_) => std::thread::sleep(WINDOW_POLL),
        }
    }
}

struct Unpark(Thread);

impl Wake for Unpark {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

fn block_on<H: Host>(backend: &Rune<H>, session: impl Future<Output = ()>) {
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut session = pin!(session);
    while !backend.quitting() {
        if session.as_mut().poll(&mut context).is_ready() {
            return;
        }
        std::thread::park_timeout(IDLE_WAIT);
    }
}

pub(super) struct Session {
    target: RefCell<Target>,
    devices: Devices,
    sway: Sway,
    log: InputLog,
    /// The modifiers a `hold` or a `key_down` step pressed and has not released.
    held: RefCell<Vec<Modifier>>,
    /// The last snapshot read, with its window's root and its frame's serial.
    seen: RefCell<Option<(String, u64, Rc<Snapshot>)>>,
    clipboard: RefCell<Option<clipboard::ClipboardOwner>>,
    held_elsewhere: Cell<bool>,
}

impl Session {
    fn connect(target: Target, log: InputLog, admitted: Admitted) -> Result<Self, String> {
        let Admitted { sway, screen } = admitted;
        own_window(&sway)?;
        let devices = Devices::connect(screen)?;
        target.prepare()?;
        Ok(Self {
            target: RefCell::new(target),
            devices,
            sway,
            log,
            held: RefCell::default(),
            seen: RefCell::default(),
            clipboard: RefCell::default(),
            held_elsewhere: Cell::new(false),
        })
    }

    fn link(&self) -> Link {
        self.target.borrow().link.clone()
    }

    fn root(&self) -> String {
        self.target.borrow().root.clone()
    }

    /// Acts on `target` from the next step on: another of the process's windows.
    fn retarget(&self, target: Target) -> Result<(), String> {
        target.prepare()?;
        *self.target.borrow_mut() = target;
        Ok(())
    }

    fn wait_clipboard_settled(&self, before: u64) {
        let deadline = Instant::now() + DISPATCH_LIMIT;
        while Instant::now() < deadline {
            match self
                .link()
                .ask(move |app| crate::clipboard::settled_since(app, before))
            {
                Ok(true) => return,
                Ok(false) => std::thread::sleep(DISPATCH_POLL),
                Err(error) => {
                    eprintln!("strata: the session host: clipboard_set: {error}");
                    return;
                }
            }
        }
    }

    fn dispatched(&self, what: &str) -> bool {
        let sent = self.devices.last_sent();
        let deadline = Instant::now() + DISPATCH_LIMIT;
        while Instant::now() < deadline {
            match self.link().ask(|_| rune::last_input()) {
                Ok(Some(at)) if at >= sent => return true,
                Ok(_) => std::thread::sleep(DISPATCH_POLL),
                Err(error) => {
                    eprintln!("strata: the session host: {what}: {error}");
                    return false;
                }
            }
        }
        eprintln!(
            "strata: the session host: the window had not taken {what} after {} ms",
            DISPATCH_LIMIT.as_millis()
        );
        false
    }

    /// Whether a drag this window started runs: its motions go to the compositor's data device.
    fn dragging_out(&self) -> bool {
        self.link()
            .ask(|app| crate::dnd::is_dragging(app))
            .unwrap_or(false)
    }

    async fn input(
        &self,
        what: &str,
        send: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        let before = self.devices.last_sent();
        send()?;
        if self.devices.last_sent() != before {
            let _ = self.dispatched(what);
        }
        self.settle().await;
        Ok(())
    }

    fn press(&self, key: UsKey, pressed: bool) -> Result<(), String> {
        self.devices.key(key.keycode, pressed)
    }

    fn modifier(&self, modifier: Modifier, pressed: bool) -> Result<(), String> {
        self.log.key(modifier_name(modifier), pressed);
        self.devices.key(modifier_key(modifier), pressed)
    }

    fn tap(&self, name: &str, key: UsKey, with: &[Modifier]) -> Result<(), String> {
        let shifted =
            self.held.borrow().contains(&Modifier::Shift) || with.contains(&Modifier::Shift);
        let mut around = with.to_vec();
        if key.shift && !shifted {
            around.push(Modifier::Shift);
        }
        for modifier in &around {
            self.modifier(*modifier, true)?;
        }
        self.log.key(name, true);
        self.press(key, true)?;
        self.log.key(name, false);
        self.press(key, false)?;
        for modifier in around.iter().rev() {
            self.modifier(*modifier, false)?;
        }
        Ok(())
    }

    fn window(&self) -> Result<sway::Area, String> {
        self.sway.window(std::process::id())
    }
}

/// The session host types on the US layout only.
fn keymap_is_us(keymap: &Keymap) -> Result<(), String> {
    if keymap::is_plain_us(keymap) {
        Ok(())
    } else {
        Err("the session host types on the US layout only".to_owned())
    }
}

fn presses_elsewhere(
    window: Option<sway::Area>,
    at: (i16, i16),
    taken: impl FnOnce() -> bool,
) -> bool {
    match window {
        Some(window) if !window.contains(at) => true,
        Some(_) => {
            let _ = taken();
            false
        }
        None => !taken(),
    }
}

impl Host for Session {
    fn snapshot(&self) -> Rc<Snapshot> {
        let Some(frame) = self.link().frames.latest() else {
            return Rc::default();
        };
        let root = self.root();
        let mut seen = self.seen.borrow_mut();
        match &*seen {
            Some((read, serial, snapshot)) if *read == root && *serial == frame.serial => {
                Rc::clone(snapshot)
            }
            _ => {
                let snapshot = Rc::new(Snapshot::read_named(&root, &frame.tree, &frame.drawn));
                *seen = Some((root, frame.serial, Rc::clone(&snapshot)));
                snapshot
            }
        }
    }

    fn with_app<T: Send + 'static>(
        &self,
        read: impl FnOnce(&App) -> T + Send + 'static,
    ) -> Result<T, String> {
        self.link().ask(move |app| read(app))
    }

    fn replacements(&self) -> u64 {
        let root = self.root();
        let now = move |app: &mut App| {
            let watch = replacements_of(&root, app);
            watch.follow(app);
            watch.now()
        };
        self.link().ask(now).unwrap_or_default()
    }

    fn replaced_since(&self, entry: EntryId, since: u64) -> bool {
        let root = self.root();
        let replaced = move |app: &mut App| {
            let watch = replacements_of(&root, app);
            watch.follow(app);
            watch.replaced_since(app, entry, since)
        };
        self.link().ask(replaced).unwrap_or_default()
    }

    async fn settle(&self) {
        let before = self.link().frames.serial();
        let drawn = match self.link().change(|_| ()) {
            Ok(()) => self.link().frames.wait_past(before, SETTLE_LIMIT),
            Err(error) => {
                eprintln!("strata: the session host: a settle: {error}");
                return;
            }
        };
        if !drawn {
            eprintln!(
                "strata: the session host: the window drew no frame within {} ms of a settle",
                SETTLE_LIMIT.as_millis()
            );
        }
    }

    async fn sleep(&self, time: Duration) {
        std::thread::sleep(time);
    }

    fn pause(&self, limit: Duration) {
        std::thread::sleep(limit);
    }

    fn origin(&self, _node: &Node) -> Result<(f64, f64), String> {
        let window = self.window()?;
        Ok((window.x, window.y))
    }

    async fn resize(&self, width: f32, height: f32) -> Result<(), String> {
        let pid = std::process::id();
        self.sway.command(&format!(
            "[pid={pid}] resize set width {width} px height {height} px"
        ))?;
        self.settle().await;
        Ok(())
    }

    fn set_layout(&self, keymap: &Keymap) -> Result<(), String> {
        keymap_is_us(keymap)
    }

    async fn chord(&self, chord: &Chord) -> Result<(), String> {
        let key = self.devices.named(&chord.key)?;
        let what = format!("the key {}", chord.key);
        self.input(&what, || self.tap(&chord.key, key, &chord.modifiers))
            .await
    }

    async fn text_input(&self, character: char) -> Result<(), String> {
        let key = self.devices.typing(character)?;
        let what = format!("the typed {character:?}");
        self.input(&what, || self.tap(&character.to_string(), key, &[]))
            .await
    }

    async fn hold(&self, modifiers: &[Modifier]) -> Result<(), String> {
        self.input("the held modifiers", || {
            for modifier in modifiers {
                self.modifier(*modifier, true)?;
                self.held.borrow_mut().push(*modifier);
            }
            Ok(())
        })
        .await
    }

    async fn release(&self, modifiers: &[Modifier]) {
        let released = self.input("the released modifiers", || {
            for modifier in modifiers.iter().rev() {
                self.held.borrow_mut().retain(|held| held != modifier);
                self.modifier(*modifier, false)?;
            }
            Ok(())
        });
        if let Err(error) = released.await {
            eprintln!("strata: releasing the modifiers: {error}");
        }
    }

    async fn key(&self, name: &str, pressed: bool) -> Result<(), String> {
        if let Ok(modifier) = Modifier::parse(name) {
            self.modifier(modifier, pressed)?;
            let mut held = self.held.borrow_mut();
            held.retain(|held| *held != modifier);
            if pressed {
                held.push(modifier);
            }
        } else {
            let key = self.devices.named(name)?;
            self.log.key(name, pressed);
            self.press(key, pressed)?;
        }
        let direction = if pressed { "press" } else { "release" };
        let _ = self.dispatched(&format!("the {direction} of {name}"));
        Ok(())
    }

    async fn pointer_move(&self, x: i16, y: i16) -> Result<(), String> {
        let moves = self.devices.position() != (x, y);
        self.log.motion((x, y));
        self.devices.motion(x, y)?;
        if !moves {
            return Ok(());
        }
        if self.held_elsewhere.get() || self.dragging_out() {
            std::thread::sleep(DRAG_MOTION_PACE);
        } else {
            let _ = self.dispatched(&format!("the motion to ({x}, {y})"));
        }
        Ok(())
    }

    fn pointer_position(&self) -> (i16, i16) {
        self.devices.position()
    }

    async fn button(&self, number: u8, pressed: bool) -> Result<(), String> {
        let at = self.devices.position();
        let window = if pressed { self.window().ok() } else { None };
        self.devices.button(number, pressed)?;
        self.log.button(at, number, pressed);
        let direction = if pressed { "press" } else { "release" };
        let what = format!("the {direction} of button {number}");
        let elsewhere = if pressed {
            presses_elsewhere(window, at, || self.dispatched(&what))
        } else {
            let held = self.held_elsewhere.get();
            if !held {
                let _ = self.dispatched(&what);
            }
            held
        };
        self.held_elsewhere.set(pressed && elsewhere);
        Ok(())
    }

    fn activate(&self, target: &Activated) -> bool {
        let (Some(bounds), Ok(window)) = (target.bounds, self.window()) else {
            return false;
        };
        let point = |origin: f64, start: f64, length: f64| {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "a point on the output fits i16"
            )]
            let point = (origin + start + length / 2.0).round() as i16;
            point
        };
        let x = point(window.x, bounds.x, bounds.width);
        let y = point(window.y, bounds.y, bounds.height);
        let clicked = self
            .devices
            .motion(x, y)
            .and_then(|()| self.devices.button(1, true))
            .and_then(|()| self.devices.button(1, false));
        let _ = self.dispatched(&format!("the click on {}", target.key));
        clicked.is_ok()
    }

    async fn scroll_to(&self, (_, key): (&Node, &str), x: f32, y: f32) -> Result<(), String> {
        let key = key.to_owned();
        let before = self.link().frames.serial();
        self.link()
            .change_tx(move |_, tx| tx.scroll_to_instant(key, x, y))?;
        let _ = self.link().frames.wait_past(before, SETTLE_LIMIT);
        self.settle().await;
        Ok(())
    }

    async fn set_clipboard(&self, content: &ClipboardContent) -> Option<Result<(), String>> {
        let before = match self.with_app(crate::clipboard::offer_serial) {
            Ok(serial) => serial,
            Err(error) => return Some(Err(error)),
        };
        let owner = match clipboard::ClipboardOwner::spawn(content) {
            Ok(owner) => owner,
            Err(error) => return Some(Err(error)),
        };
        *self.clipboard.borrow_mut() = Some(owner);
        self.wait_clipboard_settled(before);
        self.settle().await;
        Some(Ok(()))
    }

    async fn read_clipboard(&self, mime: Option<&str>) -> Option<Result<Option<String>, String>> {
        let result = clipboard::read(mime);
        self.clipboard.borrow_mut().take();
        Some(result)
    }
}
