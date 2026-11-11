//! Handing `CLIPBOARD` to a clipboard manager before the process exits (freedesktop's
//! ClipboardManager spec, built on ICCCM). The owner converts `CLIPBOARD_MANAGER` to
//! `SAVE_TARGETS`, naming the targets to save in a property of its window; the manager then
//! takes every one of them from the owner (by `MULTIPLE`, or one by one), takes `CLIPBOARD`
//! over, and answers. The actor serves those requests while it waits, until the answer or the
//! deadline. Without a manager running nothing waits.

use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};

use x11rb::NONE;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, PropMode, SelectionNotifyEvent};
use x11rb::wrapper::ConnectionExt as _;

use super::{Actor, Cmd, Mailbox};

/// How handing the clipboard to a clipboard manager went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandOff {
    /// The manager took every type: the clipboard outlives the process.
    Saved,
    /// No clipboard manager runs: nothing outlives the process.
    NoManager,
    /// This process held no clipboard to hand over.
    NotHeld,
    /// The manager answered that it saved nothing.
    Refused,
    /// The manager didn't answer by the deadline.
    TimedOut,
    /// The X11 side is gone (shut down, or its connection failed).
    Gone,
}

/// A hand-off waiting for the manager's answer.
pub(super) struct Waiting {
    property: x11rb::protocol::xproto::Atom,
    deadline: Instant,
    reply: Sender<HandOff>,
}

impl Waiting {
    pub(super) fn deadline(&self) -> Instant {
        self.deadline
    }
}

/// Asks the actor behind `mailbox` to hand `CLIPBOARD` over, and waits for the answer, at
/// most `timeout` (plus a little for the actor's last turn).
pub(super) fn hand_off(mailbox: &Mailbox, timeout: Duration) -> HandOff {
    let (reply, answer) = mpsc::channel();
    let deadline = Instant::now() + timeout;
    if mailbox.post(Cmd::HandOff { deadline, reply }).is_err() {
        return HandOff::Gone;
    }
    match answer.recv_timeout(timeout + Duration::from_millis(500)) {
        Ok(h) => h,
        Err(mpsc::RecvTimeoutError::Timeout) => HandOff::TimedOut,
        Err(mpsc::RecvTimeoutError::Disconnected) => HandOff::Gone,
    }
}

impl Actor {
    pub(super) fn start_hand_off(&mut self, deadline: Instant, reply: Sender<HandOff>) {
        if let Some(w) = self.handoff.take() {
            // A second ask replaces the first, which hears it timed out.
            let _ = w.reply.send(HandOff::TimedOut);
        }
        let Some((time, mimes)) = self
            .owned
            .get(&self.atoms.CLIPBOARD)
            .map(|o| (o.time(), o.mimes()))
        else {
            let _ = reply.send(HandOff::NotHeld);
            return;
        };
        let manager = self
            .conn
            .get_selection_owner(self.atoms.CLIPBOARD_MANAGER)
            .ok()
            .and_then(|c| c.reply().ok())
            .map_or(NONE, |r| r.owner);
        if manager == NONE {
            let _ = reply.send(HandOff::NoManager);
            return;
        }
        let Some(property) = self.take_prop() else {
            let _ = reply.send(HandOff::Gone);
            return;
        };
        // Every type: the list a manager reads to know what to save.
        let targets: Vec<u32> = mimes.iter().filter_map(|m| self.atom(m)).collect();
        let _ = self.conn.change_property32(
            PropMode::REPLACE,
            self.win,
            property,
            AtomEnum::ATOM,
            &targets,
        );
        let _ = self.conn.convert_selection(
            self.win,
            self.atoms.CLIPBOARD_MANAGER,
            self.atoms.SAVE_TARGETS,
            property,
            time,
        );
        self.handoff = Some(Waiting {
            property,
            deadline,
            reply,
        });
    }

    /// The manager's answer, if `n` is it.
    pub(super) fn hand_off_answered(&mut self, n: &SelectionNotifyEvent) -> bool {
        if n.selection != self.atoms.CLIPBOARD_MANAGER || n.target != self.atoms.SAVE_TARGETS {
            return false;
        }
        let Some(w) = self.handoff.take() else {
            return true;
        };
        let _ = self.conn.delete_property(self.win, w.property);
        self.free_props.push(w.property);
        let _ = w.reply.send(if n.property == NONE {
            HandOff::Refused
        } else {
            HandOff::Saved
        });
        true
    }

    /// The deadline passed, or the actor stops: the wait ends.
    pub(super) fn end_hand_off(&mut self, now: Option<Instant>, why: HandOff) {
        if self
            .handoff
            .as_ref()
            .is_some_and(|w| now.is_none_or(|now| w.deadline <= now))
            && let Some(w) = self.handoff.take()
        {
            // Its property isn't reused: the manager may still answer into it.
            let _ = w.reply.send(why);
        }
    }
}
