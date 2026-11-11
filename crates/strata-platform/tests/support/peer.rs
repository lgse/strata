//! The client for `strata-test-peer` (`tests/support/strata-test-peer/`): typed requests over
//! its JSON-line protocol.

use std::cell::Cell;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde_json::Value;
use strata_test_peer::protocol::{
    Action, ActivationToken, Button, Command, Event, Exported, Mapped, Message, MultipleRead,
    Offer, Received, Request, Selection, Targets, TypedWindow, Via,
};

use super::{Display, Proc, Session};

/// One peer process, connected to one display of a session.
pub struct Peer {
    proc: Proc,
    next_id: Cell<u64>,
    /// Events up to here were matched by an earlier [`Peer::wait_event`].
    events_from: Cell<usize>,
}

/// The peer binary: `STRATA_TEST_PEER`, else the one cargo built beside the test binary
/// (`target/<profile>/strata-test-peer`; `mise run test-platform` builds it first).
fn binary() -> PathBuf {
    if let Some(p) = std::env::var_os("STRATA_TEST_PEER").filter(|v| !v.is_empty()) {
        return p.into();
    }
    let exe = std::env::current_exe().expect("test binary path");
    let path = exe
        .parent()
        .and_then(|deps| deps.parent())
        .map(|profile| profile.join("strata-test-peer"))
        .expect("target directory");
    assert!(
        path.is_file(),
        "no peer at {}: build it with `cargo build -p strata-test-peer`, or run \
         `mise run test-platform`, which does",
        path.display()
    );
    path
}

impl Peer {
    pub(super) fn spawn(session: &Session, display: Display, name: &str) -> Peer {
        let cmd = session.command(binary(), display);
        let peer = Peer {
            proc: session.spawn(name, cmd),
            next_id: Cell::new(1),
            events_from: Cell::new(0),
        };
        peer.wait_event("ready", |e| matches!(e, Event::Ready { .. }));
        peer
    }

    pub fn pid(&self) -> u32 {
        self.proc.pid()
    }

    /// Sends `command` and waits for its reply: the reply's value, or the peer's error.
    pub fn try_request(&self, command: Command) -> Result<Value, String> {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let line = serde_json::to_string(&Request {
            id,
            command: command.clone(),
        })
        .expect("request JSON");
        self.proc.send_line(&line);
        let what = format!("the reply to {command:?}");
        let (_, reply) = self.proc.wait_for(0, &what, |line| match parse(line) {
            Some(Message::Reply(r)) if r.id == id => Some(r),
            _ => None,
        });
        match reply.ok {
            true => Ok(reply.value),
            false => Err(reply.error.unwrap_or_default()),
        }
    }

    /// Sends `command` and panics unless it succeeds.
    pub fn request(&self, command: Command) -> Value {
        let what = format!("{command:?}");
        self.try_request(command).unwrap_or_else(|e| {
            panic!(
                "peer {}: {what} failed: {e}\n{}",
                self.pid(),
                self.proc.tails()
            )
        })
    }

    fn request_as<T: DeserializeOwned>(&self, command: Command) -> T {
        let what = format!("{command:?}");
        serde_json::from_value(self.request(command))
            .unwrap_or_else(|e| panic!("peer {}: the reply to {what}: {e}", self.pid()))
    }

    /// The next event, after the last one matched, that passes `pred`.
    pub fn wait_event(&self, what: &str, pred: impl Fn(&Event) -> bool) -> Event {
        let (at, event) =
            self.proc
                .wait_for(self.events_from.get(), what, |line| match parse(line) {
                    Some(Message::Event(e)) if pred(&e) => Some(e),
                    _ => None,
                });
        self.events_from.set(at + 1);
        event
    }

    /// Whether an event passing `pred` came from line `mark` on, waiting up to `wait`. For
    /// what must not happen (a cancelled drag's drop): it doesn't move `wait_event`'s place.
    #[cfg_attr(
        not(any(feature = "dnd", feature = "clipboard")),
        expect(dead_code, reason = "only P1's and P2's session tests use it")
    )]
    pub fn saw_since(&self, mark: usize, wait: Duration, pred: impl Fn(&Event) -> bool) -> bool {
        let deadline = Instant::now() + wait;
        loop {
            let seen = self
                .proc
                .lines()
                .iter()
                .skip(mark)
                .any(|line| matches!(parse(line), Some(Message::Event(e)) if pred(&e)));
            if seen || Instant::now() >= deadline {
                return seen;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// The first event from line `mark` on that passes `pred`, waiting for it, without
    /// moving `wait_event`'s place: for events that may come in any order.
    #[cfg_attr(
        not(feature = "dnd"),
        expect(dead_code, reason = "only P1's session tests, behind dnd, use it")
    )]
    pub fn event_since(&self, mark: usize, what: &str, pred: impl Fn(&Event) -> bool) -> Event {
        let (_, event) = self.proc.wait_for(mark, what, |line| match parse(line) {
            Some(Message::Event(e)) if pred(&e) => Some(e),
            _ => None,
        });
        event
    }

    /// How many lines the peer has written: a point to look for events after.
    pub fn line_count(&self) -> usize {
        self.proc.line_count()
    }

    /// Whether, from line `mark` on, the peer's last pointer event is an enter or a motion,
    /// waiting up to `wait` for one: the pointer reached its window after `mark`.
    pub fn pointer_since(&self, mark: usize, wait: Duration) -> bool {
        let deadline = Instant::now() + wait;
        loop {
            let last =
                self.proc
                    .lines()
                    .iter()
                    .skip(mark)
                    .rev()
                    .find_map(|line| match parse(line) {
                        Some(Message::Event(
                            Event::PointerEnter { .. } | Event::PointerMotion { .. },
                        )) => Some(true),
                        Some(Message::Event(Event::PointerLeave)) => Some(false),
                        _ => None,
                    });
            if last == Some(true) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    // ---- windows ----

    /// Maps the peer's toplevel (300×200 until the compositor says otherwise).
    pub fn map(&self, title: &str) -> Mapped {
        self.request_as(Command::Map {
            title: title.to_owned(),
            width: 300,
            height: 200,
        })
    }

    /// Wayland: an `xdg_activation_v1` token for the peer's focused window (P8).
    #[cfg_attr(
        not(feature = "single-instance"),
        expect(
            dead_code,
            reason = "only P8's session tests, behind single-instance, ask for one"
        )
    )]
    pub fn activation_token(&self) -> String {
        self.request_as::<ActivationToken>(Command::ActivationToken)
            .token
    }

    /// X11: the top-level windows whose `_NET_WM_WINDOW_TYPE` lists `ty`.
    #[cfg_attr(
        not(feature = "dnd"),
        expect(dead_code, reason = "only P1's session tests, behind dnd, use it")
    )]
    pub fn windows_of_type(&self, ty: &str) -> Vec<TypedWindow> {
        self.request_as::<Vec<TypedWindow>>(Command::WindowsOfType { ty: ty.to_owned() })
    }

    /// `wayland:<handle>` or `x11:<xid>`.
    pub fn export(&self) -> String {
        self.request_as::<Exported>(Command::Export).handle
    }

    // ---- virtual input (Wayland peers) ----

    pub(super) fn devices(&self, extent: [u32; 2]) {
        self.request(Command::Devices {
            pointer: true,
            keyboard: true,
            extent,
        });
    }

    pub fn pointer_move(&self, (x, y): (f64, f64)) {
        self.request(Command::PointerMove { x, y });
    }

    /// The left button, down or up.
    pub fn button(&self, pressed: bool) {
        self.button_of(Button::Left, pressed);
    }

    pub fn button_of(&self, button: Button, pressed: bool) {
        self.request(Command::PointerButton { button, pressed });
    }

    /// Moves to `to` in 20 steps, 10 ms apart: a drag that moves rather than jumps.
    pub fn path(&self, (x, y): (f64, f64)) {
        self.request(Command::PointerPath {
            to: [x, y],
            steps: 20,
            step_ms: 10,
        });
    }

    /// A left click at a layout point.
    pub fn click_at(&self, at: (f64, f64)) {
        self.pointer_move(at);
        self.button(true);
        self.button(false);
    }

    /// A right click at a layout point.
    pub fn right_click_at(&self, at: (f64, f64)) {
        self.pointer_move(at);
        self.button_of(Button::Right, true);
        self.button_of(Button::Right, false);
    }

    /// A chord: `Return`, `ctrl+c`.
    pub fn key(&self, chord: &str) {
        self.request(Command::Key {
            key: chord.to_owned(),
            pressed: None,
        });
    }

    /// Presses a chord's keys and holds them: `shift`, `ctrl+alt`.
    pub fn key_down(&self, chord: &str) {
        self.request(Command::Key {
            key: chord.to_owned(),
            pressed: Some(true),
        });
    }

    /// Releases a chord's keys.
    pub fn key_up(&self, chord: &str) {
        self.request(Command::Key {
            key: chord.to_owned(),
            pressed: Some(false),
        });
    }

    // ---- selections ----

    pub fn set_selection(&self, selection: Selection, via: Via, offers: &[Offer]) {
        self.request(Command::SelectionSet {
            selection,
            via,
            offers: offers.to_vec(),
        });
    }

    pub fn targets(&self, selection: Selection, via: Via) -> Option<Vec<String>> {
        self.request_as::<Targets>(Command::SelectionTargets { selection, via })
            .mimes
    }

    pub fn read(&self, selection: Selection, via: Via, mime: &str) -> Received {
        self.request_as(Command::SelectionRead {
            selection,
            via,
            mime: mime.to_owned(),
        })
    }

    /// X11: reads `mimes` in one `MULTIPLE` conversion, repeats included; None where refused.
    #[cfg_attr(
        not(feature = "clipboard"),
        expect(
            dead_code,
            reason = "only P2's session tests, behind clipboard, use it"
        )
    )]
    pub fn read_multiple(&self, selection: Selection, mimes: &[&str]) -> Vec<Option<Received>> {
        self.request_as::<MultipleRead>(Command::SelectionReadMultiple {
            selection,
            mimes: mimes.iter().map(|m| (*m).to_owned()).collect(),
        })
        .results
    }

    /// X11: become the clipboard manager; with `answer: false`, one that never answers.
    #[cfg_attr(
        not(feature = "clipboard"),
        expect(
            dead_code,
            reason = "only P2's session tests, behind clipboard, use it"
        )
    )]
    pub fn clipboard_manager(&self, answer: bool) {
        self.request(Command::ClipboardManager { answer });
    }

    // ---- drag and drop ----

    /// Starts a drag from the peer's window; a press on it must still be held.
    pub fn drag_start(&self, offers: &[Offer], actions: &[Action]) {
        self.request(Command::DragStart {
            offers: offers.to_vec(),
            actions: actions.to_vec(),
        });
    }

    /// Accept drops of the first of `mimes` offered, with any of `actions`, `preferred` first;
    /// no `mimes` refuses them.
    pub fn accept_drops(&self, mimes: &[&str], actions: &[Action], preferred: Action) {
        self.accept_drops_with(mimes, actions, preferred, false, None);
    }

    /// [`Peer::accept_drops`], reading every one of `mimes` offered when `read_all`, and
    /// settling a drop left on ask with `ask_answer`.
    pub fn accept_drops_with(
        &self,
        mimes: &[&str],
        actions: &[Action],
        preferred: Action,
        read_all: bool,
        ask_answer: Option<Action>,
    ) {
        self.request(Command::DropAccept {
            mimes: mimes.iter().map(|m| (*m).to_owned()).collect(),
            actions: actions.to_vec(),
            preferred,
            read_all,
            ask_answer,
        });
    }

    /// X11: drag messages for the peer's window go to a proxy window from now on.
    pub fn drop_proxy(&self) {
        self.request(Command::DropProxy);
    }

    /// Replies once everything the peer sent before has reached the server and come back.
    pub fn sync(&self) {
        self.request(Command::Sync);
    }
}

fn parse(line: &str) -> Option<Message> {
    serde_json::from_str(line).ok()
}
