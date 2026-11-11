//! A module built on the X11 actor (P1's XDND) runs here, on the actor thread, through a
//! [`Hook`]: it hears the events P0 doesn't handle itself (client messages, XI2), takes
//! commands posted from other threads, and uses the actor's connection, window and
//! selection code through a [`HookCx`]. One connection then serves selections and drag and
//! drop alike, and a reply awaited by the hook never steals events from the actor's poll.

use std::any::Any;
use std::time::Instant;

use x11rb::CURRENT_TIME;
use x11rb::connection::Connection as _;
use x11rb::protocol::Event as XEvent;
use x11rb::protocol::xproto::{Atom, ConnectionExt as _, Window};
use x11rb::rust_connection::RustConnection;

use super::super::data::{ReadLimits, SourceSpec};
use super::super::event::{Done, SourceId};
use super::selection::{Owning, Then, Wanted};
use super::{Actor, Reply, lock};

/// A module running on the actor thread.
pub(crate) trait Hook: Send {
    /// An event the actor doesn't handle itself: client messages, XI2 and other extension
    /// events, and events about other windows.
    fn event(&mut self, cx: &mut HookCx<'_>, event: &XEvent);

    /// A command posted with [`super::X11::post_hook`].
    fn command(&mut self, cx: &mut HookCx<'_>, cmd: Box<dyn Any + Send>);

    /// When [`Hook::tick`] should next run, if ever.
    fn deadline(&self) -> Option<Instant> {
        None
    }

    /// Runs once the actor has passed [`Hook::deadline`].
    fn tick(&mut self, _cx: &mut HookCx<'_>, _now: Instant) {}

    /// The actor is quitting: undo what the hook set on other clients' windows, while the
    /// connection still works.
    fn stop(&mut self, _cx: &mut HookCx<'_>) {}
}

/// The actor, as a hook sees it.
pub(crate) struct HookCx<'a> {
    pub(super) actor: &'a mut Actor,
}

impl HookCx<'_> {
    pub(crate) fn conn(&self) -> &RustConnection {
        &self.actor.conn
    }

    /// The actor's own window: unmapped, override-redirect.
    pub(crate) fn win(&self) -> Window {
        self.actor.win
    }

    pub(crate) fn root(&self) -> Window {
        self.actor.root
    }

    pub(crate) fn atom(&mut self, name: &str) -> Option<Atom> {
        self.actor.atom(name)
    }

    pub(crate) fn name(&mut self, atom: Atom) -> Option<String> {
        self.actor.name(atom)
    }

    /// Takes `selection` with `spec`, with a server timestamp, as the actor takes
    /// `CLIPBOARD`: its writers then answer conversions (with INCR), and a refusal or a
    /// later loss arrives as `SourceCancelled`.
    pub(crate) fn own(&mut self, selection: Atom, id: SourceId, spec: SourceSpec) {
        self.actor
            .take_with_server_time(Owning::new(selection, id, spec));
    }

    /// Gives up `selection` if this actor holds it, quietly: no `SourceCancelled`.
    pub(crate) fn disown(&mut self, selection: Atom) {
        if let Some(owned) = self.actor.owned.remove(&selection) {
            lock(&self.actor.shared).owned.remove(&owned.id().get());
            let _ = self
                .actor
                .conn
                .set_selection_owner(x11rb::NONE, selection, owned.time());
        }
        // A take still waiting for its timestamp is dropped too.
        self.actor.owning.cancel_selection(selection);
    }

    /// Converts `selection` to `mime` as of `time` (an XDND drop's), with INCR. `done` runs
    /// once, on the actor thread.
    pub(crate) fn read(
        &mut self,
        selection: Atom,
        mime: &str,
        time: u32,
        limits: ReadLimits,
        done: Done,
    ) {
        let reply = Reply::new(done);
        let Some(target) = self.actor.atom(mime) else {
            return reply.send(Err(super::super::error::ReadError::Io(
                "couldn't intern the type".into(),
            )));
        };
        let time = if time == 0 { CURRENT_TIME } else { time };
        self.actor.start_read(Wanted::new(
            selection,
            time,
            target,
            limits,
            Then::User(reply),
        ));
    }

    pub(crate) fn flush(&self) {
        let _ = self.actor.conn.flush();
    }
}

impl Actor {
    /// Runs `f` with the hook, if one is installed.
    pub(super) fn with_hook(&mut self, f: impl FnOnce(&mut dyn Hook, &mut HookCx<'_>)) {
        let Some(mut hook) = self.hook.take() else {
            return;
        };
        f(hook.as_mut(), &mut HookCx { actor: self });
        if self.hook.is_none() {
            self.hook = Some(hook);
        }
    }
}
