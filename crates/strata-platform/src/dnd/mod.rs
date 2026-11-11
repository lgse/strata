//! Drag and drop on Wayland and X11, with any MIME types, for Rune apps.
//!
//! The API mirrors Rune's: a [`DragSource`] is `drag_out` plus `on_drag_start` and
//! `on_drag_end`, with a [`DragItem`] of any MIME types (each made lazily) and the actions it
//! allows. A [`DropTarget`] is `on_drop_files` with its hover, move and reject hooks, for any
//! MIME type, with the action negotiated as the drag moves. A drag between this process's
//! own windows carries a typed payload ([`DragItem::local`], [`DropTarget::local`]) instead
//! of serialized types. Each window has a [`Dnd`]: attach it from `App::on_window`, and
//! build targets and sources through it.
//!
//! ```ignore
//! dnd.drop_target(el().child(text("Drop here")), "zone",
//!     DropTarget::new(|s: &mut App, data, _| s.files = data.uris())
//!         .mime_types(&["text/uri-list", "text/plain;charset=utf-8"])
//!         .on_hover(|s: &mut App, hover, _, _| s.over = hover != Hover::Left))
//! dnd.drag_source(el().child(text("Drag me")), "tile",
//!     DragSource::new(|s: &App, _| Some(DragItem::new().uris(&s.selected).actions(Actions::COPY | Actions::MOVE)))
//!         .on_end(|s: &mut App, outcome, _| s.last = Some(outcome)))
//! ```
//!
//! Call [`shutdown`] on the UI thread before the event loop ends: it ends a running drag and
//! tears down P0's substrate while winit's display is alive (see `transfer`). Call it before
//! `Tx::quit` or `AppHandle::quit`, **and** in the last window's `on_close`. Rune's
//! `App::on_quit` runs before teardown on `Tx::quit` and `AppHandle::quit`, so call it there
//! too, but it runs in no window when the last window closes, and Linux has no system quit to
//! hook: the last window's `on_close` stays needed.
//!
//! # What Rune lacks
//!
//! Rune issues: none filed yet; the drafts are in `docs/decisions/S5-upstream.md`.
//!
//! - **U1** (draft, not filed): `drag_out` starts nothing off macOS, takes no MIME types and
//!   no actions, and its live-frame element snapshot is `pub(crate)`. P1's source is an
//!   `on_drag` element, and its snapshot icon renders a second `Runtime` ([`DragIcon`]).
//! - **U2** (draft, not filed): Wayland drops reach no handler (winit 0.30 reports none),
//!   and no handler takes MIME types or a position per motion on either backend. P1 hears
//!   drags through P0's `wl_data_device`, and on X11 through `XdndProxy` (which turns off
//!   Rune's own X11 `on_drop_files`: take file drops through P1 there).
//! - **U9** (draft, not filed): no display handle, no hook before winit closes the display,
//!   no every-turn hook. P0 recovers the display and runs its own reader (S2's driver C);
//!   P1 adds neither a queue nor a thread to it (the owner's ruling on S2 Q2).
//! - **U10** (draft, not filed): no public way to end a pointer capture. After a Wayland
//!   drag starts, Rune would stay pressing. **Workaround:** P1 sends Escape through
//!   `Tx::send_key`, which Rune takes as "Escape during a drag cancels it", and swallows the
//!   cancelled `Up` that follows (the owner's ruling on S2 Q3). An Escape that finds no
//!   capture would reach the app's keys, so it goes out only under the checks in `session`:
//!   once the compositor proves it took the pointer (the drag's own `wl_data_device.enter`
//!   on an app window, or a target's `target` or `action` answer), the drag hasn't dropped
//!   or ended, the pressed element is still live where it was, and P1 saw the press. A drag
//!   no proof reaches keeps its capture until the window's next press or focus change.
//!   Rune 2a44156 still has no public way to end it. "Rune's capture on Wayland" below lists
//!   what remains. Tests guard it at the pinned Rune rev.
//!
//!   Rune #408 (`dece2aff`, in the pin) rewrote how Rune holds a press (`capture::Held`
//!   replaces the runtime's capture fields). The Escape relies on Rune's `cancel_drag` finding
//!   the held element by its path and answering with a cancelled `Up`, which it still does:
//!   P1's session suites (`mise run test-platform p1::`, among them
//!   `wayland_runes_capture_ends_when_the_drag_starts`) pass at 2a44156. Re-run them at every
//!   Rune bump.
//! - **U3** (draft, not filed): file-drop acceptance sees no app state and no drag identity.
//! - **U-P1-1** (draft, not filed): an element has one handler of each kind, and a shim
//!   can't read an element's key. So [`Dnd::drag_source`] takes the element's `on_drag` and
//!   [`Dnd::drop_target`] its `drawn_rect_into`, and targets are found by the key the app
//!   gives them. Ask: composable handlers, or an extension slot readable from a hit test's
//!   `Picked`.
//! - **U-P1-2** (draft, not filed): `Tx::element_at` gives a path and a key, but no rect and
//!   no point in the element's own coordinates (Rune's own drops have `drop_point`). P1 reads
//!   the target's drawn rect instead, so a point is known only once the target has drawn.
//!
//! # Known limits
//!
//! - **Ctrl and Shift on Wayland** (the owner's ruling): a drag from another client brings
//!   no modifiers (the target has no keyboard focus, and wlroots' drag grab passes none), so
//!   its action is the compositor's: sway and Hyprland don't map modifiers to actions;
//!   mutter and KWin are hand tests pending. Ctrl and Shift are honoured on X11 in both
//!   directions (throughout the drag), at the start of a Wayland drag out (a source's
//!   actions are set once, before `start_drag`), and in a drag between this process's own
//!   windows (as held at its start).
//! - **Link is X11 only:** Wayland's data device has no link action.
//! - **Escape on Wayland:** the compositor's drag grab takes the keyboard, and wlroots
//!   compositors (sway) don't cancel on Escape; [`Dnd::cancel_drag`] is the only route
//!   there. Other compositors' Escape is a hand test pending.
//! - **X11 cursor:** the source is grab-less (winit holds the grab), so the cursor shows no
//!   copy or move feedback; the icon follows the pointer.
//! - **libwayland < 1.23** (Ubuntu 24.04): unsupported, as 1.23 is the minimum (S3 decision
//!   1). winit's display is out of reach there, and so is drag and drop on Wayland
//!   ([`Status::unsupported`]).
//!
//! # Rune's capture on Wayland
//!
//! Every path where Rune's capture of a drag's press is left held, or P1's Escape reaches
//! the app's keys. Each is a residue of U10 (draft, not filed): with a public way to end a
//! capture, none would remain.
//!
//! 1. **Focus lost before the proof's update.** Keyboard focus leaves the window for a
//!    reason other than the drop (the user switching windows, a window mapping and taking
//!    focus) between the drag's start and the update that handles the compositor's proof
//!    (normally a few milliseconds: one P0 dispatch and one window update). Rune ends the
//!    capture at that focus loss; if its cancelled `Up` hasn't reached P1 by the update, the
//!    Escape finds no capture and the app's keys hear one Escape.
//! 2. **A modal mounting before the proof's update.** A modal that mounts in the window
//!    between the drag's start and that update (one opened from `on_start`, say) clears
//!    Rune's capture without an `Up`, which P1 can't see: the proof's Escape goes to the
//!    modal or the app's keys, once, and a modal that closes on Escape closes. Not a race:
//!    a modal opened from `on_start` takes it every time. Don't open one while a drag starts.
//! 3. **No proof: the capture is held until the window's next press or focus change.** A
//!    drag that never enters the app's windows and that no target answers (one started and
//!    released over no surface, or over clients that never answer) gets no Escape from P1.
//!    Rune's capture then stays held until the compositor's focus change at the drop where
//!    there is one (sway's, about 14 ms after it), else the window's next press (which
//!    replaces it), its next focus change, or a modal mounting. Meanwhile hover is frozen,
//!    and the user's first Escape ends the capture instead of reaching the app's keys. P1
//!    ends nothing later on a timer: Rune drops a capture without an `Up` when a press
//!    replaces it or a modal mounts, and an Escape sent after that would reach the app.
//! 4. **The source left its place.** The element the drag started from leaves the tree,
//!    starts exiting, or moves to another path (the app hides what it drags, a tab torn
//!    off, a popover closing), or no longer covers the press point, before the proof's
//!    update. P1 sends no Escape, and the capture is held as in 3.
//! 5. **The source lost its `on_drag`.** The element keeps its key and path but is drawn
//!    without its drag source (`on_drag`) before the proof's update. P1 can't see handlers:
//!    it sends the Escape, Rune's `cancel_drag` refuses an element with no `on_drag`, the
//!    app's keys hear one Escape, and the capture is held as in 3.
//! 6. **A compositor that accepts `start_drag` after the button was released.** wlroots
//!    refuses one. Elsewhere, winit would have had the release, and an Escape sent before
//!    Rune ran the release's handler would reach the app's keys. Hand tests (mutter, KWin)
//!    pending.
//! 7. **A drag source the guard didn't see pressed** (an app's own `on_drag` that calls
//!    [`Dnd::start_drag`] without routing the press through [`Dnd::owns`]): no Escape, and
//!    the capture is held as in 3.
//!
//! So a drag's source must **stay mounted at the same path, not exiting, covering the press
//! point, and keep its `on_drag` (stay a drag source) until the drag ends**, and nothing
//! should open a modal as a drag starts.
//!
//! X11 has none of these: winit keeps the grab, and Rune hears the real release.
//!
//! # The in-process token
//!
//! A drag lists `application/x-local-drag;id=<counter>.<nonce>` (no process id: the owner's
//! ruling) so a drop in this process knows the drag as its own. On X11 it is listed only for
//! a drag with a [`DragItem::local`] payload. On Wayland it is listed on every drag: it is
//! also how P1 recognises its own drag entering its windows, the compositor's proof above,
//! so other Wayland clients see it.
//!
//! # For X2 (wiring components to P1)
//!
//! - Zones need keys: a W1 `DropZone` carries none, and [`Dnd::drop_target`] needs one
//!   unique in the window (U-P1-1). Key the element and pass the same key, or, for a key
//!   unique only inside a keyed ancestor (rows inside their list), [`Dnd::drop_target_in`]
//!   with that ancestor's key as the scope.
//! - C1b's pointer intent owns the row's `on_drag`, so a row can't take
//!   [`Dnd::drag_source`]. From C1b's begin-drag intent call [`Dnd::start_drag`] with the
//!   row's `Drag` and its key, and route every phase through [`Dnd::owns`] first, ignoring
//!   the ones it claims.
//! - A row the drag starts from must stay mounted at the same path, not exiting, and keep
//!   its `on_drag` (stay a drag source) until the drag ends (paths 4 and 5 above): a
//!   virtualized row scrolled out, a row removed when its entry moves, or a list re-keyed
//!   during the drag leaves Rune's capture held; a row redrawn without C1b's `on_drag`
//!   lets one Escape reach the keys. Hide a dragged row by styling it, not by dropping it
//!   from the tree or its handlers, until `on_end`. Don't open a modal from the drag's start
//!   (path 2).
//! - W1's `choose_action` maps onto [`DropTarget::action`], which wins over P1's modifier
//!   rule.
//!
//! Work package: P1, `docs/work/phase-2-composites/P1-drag-and-drop.md`.

mod api;
mod hub;
mod icon;
mod local;
pub mod negotiate;
mod registry;
mod session;
mod wayland;
mod window;
mod x11;

pub use api::{
    Actions, AskReply, DRAG_THRESHOLD, DragAction, DragItem, DragSource, DropData, DropOffer,
    DropPoint, DropTarget, Error, Hover, Status, TEXT_TYPES,
};
#[doc(hidden)]
pub use hub::TestRoute;
pub use icon::{DragIcon, DragImage};
pub use local::LOCAL_MIME;
pub use window::Dnd;

/// Ends a running drag and tears down drag and drop with P0's substrate, on the UI thread
/// before the event loop ends. True if a substrate was bound.
pub fn shutdown() -> Result<bool, Error> {
    hub::shutdown()
}

/// Where problems that no call could return go (a drop that couldn't be finished, a read
/// that failed while peeking): stderr by default. On the UI thread.
pub fn on_diagnostic(f: impl Fn(&str) + 'static) {
    hub::set_diagnostics(std::rc::Rc::new(f));
}

#[cfg(test)]
mod tests;
