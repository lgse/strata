// SPDX-License-Identifier: MIT

//! The session host: `strata-rune --qa <directory>` in a real window, when the runner sets
//! `STRATA_QA_WINDOW=1` in a sway session (H4, `docs/rune/parity/packages/H4.md`). It serves
//! the same protocol as the offscreen host, for the runs that need what only a compositor
//! has: window focus, the system clipboard, drags between processes.
//!
//! Rune runs the window's loop on the main thread and owns the application state there. The
//! session runs on a thread of its own: it reads the tree from the frames the window draws,
//! reaches the state through `AppHandle::update_if` ([`link`]), and sends input through
//! Wayland's virtual keyboard and pointer ([`devices`]), so it crosses the compositor and
//! winit as a user's does. After each input it waits until Rune has dispatched it, then for
//! the frame that shows it.

mod devices;
mod link;
mod sway;

use std::{
    cell::{OnceCell, RefCell},
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
use strata_qa::{Chord, Keymap, Modifier};

use self::{
    devices::{Devices, UsKey, modifier_key},
    link::{Frames, Link},
    sway::Sway,
};
use super::{
    backend::Rune,
    host::{Activated, Host},
    input_log::{InputLog, modifier_name},
    keymap,
    replaced::Replacements,
    tree::Snapshot,
};
use crate::{
    App,
    host::windowed,
    launch,
    startup::{self, UiThread},
};

/// What the runner sets to have the QA mode open a real window.
const VARIABLE: &str = "STRATA_QA_WINDOW";
/// How long a settle waits for the frame after a change. The GTK harness waits this long.
const SETTLE_LIMIT: Duration = Duration::from_millis(250);
/// How long the session waits for Rune to dispatch input it sent: input the window does not
/// get, a motion outside it say, is not waited for longer.
const DISPATCH_LIMIT: Duration = Duration::from_millis(500);
const DISPATCH_POLL: Duration = Duration::from_millis(2);
/// The longest the session thread sleeps without polling the session again.
const IDLE_WAIT: Duration = Duration::from_secs(1);

thread_local! {
    /// The browser's replacements, counted on the window's thread, where the browser is.
    static REPLACEMENTS: OnceCell<Rc<Replacements>> = const { OnceCell::new() };
}

/// Whether the runner asks for the session host.
pub(super) fn wanted() -> bool {
    std::env::var_os(VARIABLE).is_some_and(|value| value == "1")
}

pub(super) fn serve(directory: &OsStr) -> Result<(), String> {
    // Before anything can log there: stdout is the replies' from here on.
    let protocol = strata_qa::reply_stream()?;
    criticals::capture();
    let settings = startup::initialize_process();
    let location = launch::location(Some(directory)).ok_or("the argument is not a location")?;
    launch::leave_launch_folder();
    let log = InputLog::from_environment()?;
    let frames = Arc::new(Frames::default());
    let recorder = Arc::clone(&frames);
    let ui = UiThread::claim()?;
    windowed::run_with(ui, settings, location, move |app| {
        let link = Link::new(app.handle(), frames);
        start(link, protocol, log);
        app.on_frame_drawn(move |runtime| recorder.record(runtime))
    })
}

/// Starts the session's thread. Requests it makes before the loop runs wait for it.
fn start(link: Link, protocol: File, log: InputLog) {
    let spawned = std::thread::Builder::new()
        .name("qa-session".to_owned())
        .spawn(move || {
            match Session::connect(link.clone(), log) {
                Ok(host) => {
                    let backend = Rune::on(host);
                    let commands = strata_qa::command_lines();
                    block_on(&backend, strata_qa::serve(&backend, protocol, commands));
                }
                Err(error) => eprintln!("strata-rune: the session host: {error}"),
            }
            link.quit();
        });
    if let Err(error) = spawned {
        eprintln!("strata-rune: cannot start the QA session: {error}");
    }
}

struct Unpark(Thread);

impl Wake for Unpark {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

/// Runs the session on this thread. Its steps block while they wait; the window's loop runs
/// on its own meanwhile.
fn block_on(backend: &Rune<Session>, session: impl Future<Output = ()>) {
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
    link: Link,
    devices: Devices,
    sway: Sway,
    log: InputLog,
    /// The modifiers a `hold` or a `key_down` step pressed and has not released.
    held: RefCell<Vec<Modifier>>,
    seen: RefCell<Option<(u64, Rc<Snapshot>)>>,
}

impl Session {
    fn connect(link: Link, log: InputLog) -> Result<Self, String> {
        let sway = Sway::from_environment()?;
        let devices = Devices::connect(sway.output()?)?;
        // As the offscreen QA host does: a column that closes leaves the tree at once.
        link.change(|app| {
            app.look.animations = false;
            REPLACEMENTS.with(|cell| cell.get_or_init(|| Replacements::watch(&app.browser)).now())
        })?;
        Ok(Self {
            link,
            devices,
            sway,
            log,
            held: RefCell::default(),
            seen: RefCell::default(),
        })
    }

    /// Waits until Rune has dispatched input sent after `sent`, for [`DISPATCH_LIMIT`] at most.
    fn dispatched(&self, sent: Instant) {
        let deadline = Instant::now() + DISPATCH_LIMIT;
        while Instant::now() < deadline {
            match self.link.ask(|_| rune::last_input()) {
                Ok(Some(at)) if at >= sent => return,
                Ok(_) => std::thread::sleep(DISPATCH_POLL),
                Err(_) => return,
            }
        }
    }

    /// Sends `send`'s input, then waits for Rune to take it and for the frame after.
    async fn input(&self, send: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
        let sent = Instant::now();
        send()?;
        self.dispatched(sent);
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

    /// Presses and releases `key` with `with` held around it, and Shift when the key needs it
    /// and it is not held already.
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

impl Host for Session {
    fn snapshot(&self) -> Rc<Snapshot> {
        let Some(frame) = self.link.frames.latest() else {
            return Rc::default();
        };
        let mut seen = self.seen.borrow_mut();
        match &*seen {
            Some((serial, snapshot)) if *serial == frame.serial => Rc::clone(snapshot),
            _ => {
                let snapshot = Rc::new(Snapshot::read(&frame.tree, &frame.drawn));
                *seen = Some((frame.serial, Rc::clone(&snapshot)));
                snapshot
            }
        }
    }

    fn with_app<T: Send + 'static>(
        &self,
        read: impl FnOnce(&App) -> T + Send + 'static,
    ) -> Result<T, String> {
        self.link.ask(move |app| read(app))
    }

    fn replacements(&self) -> u64 {
        let now = |_: &mut App| REPLACEMENTS.with(|cell| cell.get().map_or(0, |watch| watch.now()));
        self.link.ask(now).unwrap_or_default()
    }

    fn replaced_since(&self, entry: EntryId, since: u64) -> bool {
        let replaced = move |app: &mut App| {
            REPLACEMENTS.with(|cell| {
                cell.get()
                    .is_some_and(|watch| watch.replaced_since(app, entry, since))
            })
        };
        self.link.ask(replaced).unwrap_or_default()
    }

    /// Has the window commit a change, so it draws, and waits for that frame. A change that
    /// draws nothing new still draws a frame; one that never comes ends the wait at the limit.
    async fn settle(&self) {
        let before = self.link.frames.serial();
        if self.link.change(|_| ()).is_ok() {
            self.link.frames.wait_past(before, SETTLE_LIMIT);
        }
    }

    async fn sleep(&self, time: Duration) {
        std::thread::sleep(time);
    }

    fn pause(&self, limit: Duration) {
        std::thread::sleep(limit);
    }

    fn origin(&self) -> Result<(f64, f64), String> {
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
        if keymap::is_plain_us(keymap) {
            Ok(())
        } else {
            Err("the session host types on the US layout only".to_owned())
        }
    }

    async fn chord(&self, chord: &Chord) -> Result<(), String> {
        let key = self.devices.named(&chord.key)?;
        self.input(|| self.tap(&chord.key, key, &chord.modifiers))
            .await
    }

    async fn text_input(&self, character: char) -> Result<(), String> {
        let key = self.devices.typing(character)?;
        self.input(|| self.tap(&character.to_string(), key, &[]))
            .await
    }

    async fn hold(&self, modifiers: &[Modifier]) -> Result<(), String> {
        self.input(|| {
            for modifier in modifiers {
                self.modifier(*modifier, true)?;
                self.held.borrow_mut().push(*modifier);
            }
            Ok(())
        })
        .await
    }

    async fn release(&self, modifiers: &[Modifier]) {
        let released = self.input(|| {
            for modifier in modifiers.iter().rev() {
                self.held.borrow_mut().retain(|held| held != modifier);
                self.modifier(*modifier, false)?;
            }
            Ok(())
        });
        if let Err(error) = released.await {
            eprintln!("strata-rune: releasing the modifiers: {error}");
        }
    }

    async fn key(&self, name: &str, pressed: bool) -> Result<(), String> {
        let sent = Instant::now();
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
        self.dispatched(sent);
        Ok(())
    }

    async fn pointer_move(&self, x: i16, y: i16) -> Result<(), String> {
        let sent = Instant::now();
        let moves = self.devices.position() != (x, y);
        self.log.motion((x, y));
        self.devices.motion(x, y)?;
        if moves {
            self.dispatched(sent);
        }
        Ok(())
    }

    fn pointer_position(&self) -> (i16, i16) {
        self.devices.position()
    }

    async fn button(&self, number: u8, pressed: bool) -> Result<(), String> {
        let sent = Instant::now();
        self.devices.button(number, pressed)?;
        self.log.button(self.devices.position(), number, pressed);
        self.dispatched(sent);
        Ok(())
    }

    /// A click at the middle of the element: an assistive client's click needs the
    /// accessibility bus, which the session's processes do not reach.
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
        let sent = Instant::now();
        let clicked = self
            .devices
            .motion(x, y)
            .and_then(|()| self.devices.button(1, true))
            .and_then(|()| self.devices.button(1, false));
        self.dispatched(sent);
        clicked.is_ok()
    }
}
