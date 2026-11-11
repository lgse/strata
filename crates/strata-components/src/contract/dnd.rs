//! Drag and drop intents: what a drag carries, what a drop offers and delivers, the drop
//! zones components mark, and the rule that picks copy or move.
//!
//! The shapes mirror Rune's `drag_out`, `on_drop_files` and `on_drop_hover`, and the system
//! drag-and-drop shim's (P1) API in plan §4. Components never depend on that shim (work
//! README, rule 1): they attach zones and sources through a host-given [`Dnd`] binder. The
//! default, [`Dnd::rune`], wires Rune's own file drops, so drops are scriptable headless
//! today; X2 passes a binder that wires the shim. Drags that stay inside a component (header
//! reorder, pin reorder) use Rune's `on_drag` and don't go through these types.

use std::borrow::Cow;
use std::path::PathBuf;
use std::rc::Rc;

use rune::{Mods, PointerEvent, Tx};

use super::entry::EntryId;

mod binder;
mod uri;

#[cfg(test)]
mod tests;

pub use binder::{Dnd, ElementKey};
pub use uri::{file_uri, parse_uri_list, uri_to_path};

/// The `text/uri-list` MIME type: files as `file://` URIs, one per line.
///
/// Used by: C1e, C6, C7, C8, X2.
pub const URI_LIST: &str = "text/uri-list";

/// What a drop does with what it carries.
///
/// Used by: X2, Strata, and the gallery hosts of C1e, C6, C7 and C8.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DropAction {
    /// Copy the items into the target.
    Copy,
    /// Move the items into the target.
    Move,
    /// Link to the items from the target.
    Link,
    /// Ask whether to copy or move ("Copy or move?").
    ///
    /// A binder that negotiates with the drag source offers it to the source as
    /// [`DropAction::Copy`], as Strata's `Ask { default: Copy }` does, so the drag shows a copy
    /// and the source keeps its items until the host has asked. The drop is then delivered,
    /// and the host words the prompt from the [`VolumeRelation`] it looked up: only
    /// `Different` may say "another device". `Unknown` must never be presented as a confirmed
    /// device difference.
    Ask,
}

/// A set of the actions a drag source allows: copy, move and link. `Ask` is only ever a
/// result of [`choose_action`], never offered.
///
/// Used by: C1b (in the payload), C1e, X2.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct DropActions(u8);

impl DropActions {
    /// No action.
    pub const NONE: Self = Self(0);
    /// Copy.
    pub const COPY: Self = Self(1);
    /// Move.
    pub const MOVE: Self = Self(2);
    /// Link.
    pub const LINK: Self = Self(4);

    /// Whether every action in `other` is in this set.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// The actions in either set.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Whether the set is empty.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl std::ops::BitOr for DropActions {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        self.union(other)
    }
}

/// Whether a drop's destination is on the same volume as its sources. The host looks it up;
/// any source or destination it couldn't resolve makes the relation `Unknown`.
///
/// Used by: X2, Strata.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VolumeRelation {
    /// Every source is on the destination's volume.
    Same,
    /// Some source is on another volume.
    Different,
    /// The relation isn't known (yet).
    Unknown,
}

/// What a plain drop onto another device does: the "Drag & drop to another device" setting.
///
/// Used by: X2, Strata.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CrossDevice {
    /// Always copy.
    Copy,
    /// Always move.
    Move,
    /// Always ask, the default.
    #[default]
    Ask,
}

/// The inputs to [`choose_action`].
///
/// Fields may be added: build one with [`ActionInput::new`].
///
/// Used by: X2, Strata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ActionInput {
    /// The modifiers held as the drag hovers or drops.
    pub mods: Mods,
    /// What the drag source allows.
    pub offered: DropActions,
    /// The destination's volume against the sources'.
    pub volume: VolumeRelation,
    /// The cross-device setting.
    pub cross_device: CrossDevice,
}

impl ActionInput {
    /// The inputs for a drag allowing `offered`, with `mods` held, onto a destination whose
    /// volume relation is `volume`, under the `cross_device` setting.
    pub const fn new(
        mods: Mods,
        offered: DropActions,
        volume: VolumeRelation,
        cross_device: CrossDevice,
    ) -> Self {
        Self {
            mods,
            offered,
            volume,
            cross_device,
        }
    }
}

/// The copy-or-move rule (`operations/drag-and-drop/copy-or-move`), or `None` when the drop is
/// forbidden. It ports Strata's `drop_commit`:
///
/// - Ctrl copies and Shift moves, on the same or another volume. Ctrl wins when both are held.
///   A modifier asking for an action the source doesn't offer is ignored.
/// - A plain drop on the same volume moves, or copies when the source doesn't offer a move.
/// - A plain drop onto another volume, or one whose relation is unknown, follows the
///   cross-device setting. It asks only when the source offers both copy and move.
/// - With neither copy nor move offered, the drop is forbidden. No modifier picks a link.
///
/// The volume lookup and the guard against dropping items into their own folder stay with the
/// host.
///
/// Used by: X2 and Strata, at hover and at drop; the gallery hosts of C1e, C6, C7 and C8.
pub fn choose_action(input: ActionInput) -> Option<DropAction> {
    let can_copy = input.offered.contains(DropActions::COPY);
    let can_move = input.offered.contains(DropActions::MOVE);
    if !can_copy && !can_move {
        return None;
    }
    if input.mods.ctrl {
        if can_copy {
            return Some(DropAction::Copy);
        }
    } else if input.mods.shift && can_move {
        return Some(DropAction::Move);
    }
    let action = match input.volume {
        VolumeRelation::Same if can_move => DropAction::Move,
        VolumeRelation::Same => DropAction::Copy,
        VolumeRelation::Different | VolumeRelation::Unknown => match input.cross_device {
            CrossDevice::Ask if can_copy && can_move => DropAction::Ask,
            CrossDevice::Move if can_move => DropAction::Move,
            CrossDevice::Move => DropAction::Copy,
            CrossDevice::Copy | CrossDevice::Ask if can_copy => DropAction::Copy,
            CrossDevice::Copy | CrossDevice::Ask => DropAction::Move,
        },
    };
    Some(action)
}

/// What a drag from a collection carries: entry ids for the in-process fast path, plus the
/// URIs to offer other apps, and the actions it allows.
///
/// It carries no MIME list, unlike the W1 work package's wording ("the URIs and MIME types to
/// offer outside"): X2 derives every type it offers (`text/uri-list`, GNOME's and KDE's file
/// formats) from the URIs, and the system shim serialises them. Fields may be added, a MIME
/// list among them: build one with [`DragPayload::new`].
///
/// Used by: C1b (its begin-drag intent carries the ids; the host fills in the URIs), C1e, X2.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct DragPayload {
    /// The dragged entries, for a drop in this process.
    pub entries: Vec<EntryId>,
    /// The same entries as URIs (`file:///…` for local files), for other apps.
    pub uris: Vec<String>,
    /// What the drag allows: copy and move by default.
    pub actions: DropActions,
}

impl DragPayload {
    /// A drag of `entries`, offered outside as `uris`, allowing copy and move.
    pub fn new(entries: Vec<EntryId>, uris: Vec<String>) -> Self {
        Self {
            entries,
            uris,
            actions: DropActions::COPY | DropActions::MOVE,
        }
    }

    /// The same drag, allowing `actions`.
    pub fn actions(mut self, actions: DropActions) -> Self {
        self.actions = actions;
        self
    }
}

/// A shared list of MIME types: cloning it bumps a count, so every zone and offer can hold one.
type MimeList = Rc<[Cow<'static, str>]>;

thread_local! {
    /// `[URI_LIST]`, shared by every zone that reads only files and every file offer.
    static URI_LIST_ONLY: MimeList = Rc::from([Cow::Borrowed(URI_LIST)]);
}

/// `[URI_LIST]`, shared.
fn uri_list_only() -> MimeList {
    URI_LIST_ONLY.with(Rc::clone)
}

/// What a drag over a zone offers, known while it hovers: its MIME types, the actions it
/// allows, whether it comes from this process (and so carries a [`DragPayload`]), its URIs when
/// the binder could read them before the drop, and the drag's serial.
///
/// Fields may be added: build one with [`DropOffer::new`].
///
/// Used by: C1e, C6, C7, C8, X2.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct DropOffer {
    /// The MIME types the drag offers.
    pub mime_types: Rc<[Cow<'static, str>]>,
    /// The actions the drag allows.
    pub actions: DropActions,
    /// Whether the drag comes from this process.
    pub local: bool,
    /// The dragged URIs, when the binder could read them before the drop: what a host checks
    /// while the drag hovers (C8 pins only folders). `None` when they arrive only with the drop:
    /// Rune's binder hears no paths until then.
    pub uris: Option<Vec<String>>,
    /// The drag's serial, from its binder. Two drags never share one, so a host can key what it
    /// checked about a drag's items by it. A binder that can't tell where a drag begins may give
    /// one drag several serials (Rune's gives a new one each time the drag reaches a zone), so
    /// treat a check keyed by serial as a cache, not as the drag's identity.
    pub serial: u64,
}

impl DropOffer {
    /// An offer from another process, with no URIs read and serial 0.
    pub fn new(mime_types: impl Into<Rc<[Cow<'static, str>]>>, actions: DropActions) -> Self {
        Self {
            mime_types: mime_types.into(),
            actions,
            local: false,
            uris: None,
            serial: 0,
        }
    }

    /// The same offer, from this process or not.
    pub fn local(mut self, local: bool) -> Self {
        self.local = local;
        self
    }

    /// The same offer, with its URIs read.
    pub fn uris(mut self, uris: Vec<String>) -> Self {
        self.uris = Some(uris);
        self
    }

    /// The same offer, from the drag with serial `serial`.
    pub fn serial(mut self, serial: u64) -> Self {
        self.serial = serial;
        self
    }

    /// Whether the drag offers `mime`.
    pub fn offers(&self, mime: &str) -> bool {
        self.mime_types.iter().any(|m| m == mime)
    }
}

/// One MIME type's data, as read from a drop.
///
/// Used by: C1e, C6, C7, C8, X2.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DropItem {
    /// The MIME type.
    pub mime: String,
    /// Its bytes, as the source sent them.
    pub data: Vec<u8>,
}

impl DropItem {
    /// `data` of type `mime`.
    pub fn new(mime: impl Into<String>, data: impl Into<Vec<u8>>) -> Self {
        Self {
            mime: mime.into(),
            data: data.into(),
        }
    }
}

/// What a drop delivered: the in-process payload, if any, and the raw bytes of each MIME type
/// the binder read. Choosing between them (an image or its address, files or text) is the
/// host's policy (`operations/drag-and-drop/content-drops`).
///
/// Fields may be added: build one with [`DropData::new`] or [`DropData::files`].
///
/// Used by: C1e, C6, C7, C8, X2.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct DropData {
    /// The payload of a drag from this process.
    pub local: Option<DragPayload>,
    /// Each MIME type read, in the zone's order of preference.
    pub items: Vec<DropItem>,
}

impl DropData {
    /// Data of the given items, with no in-process payload.
    pub fn new(items: Vec<DropItem>) -> Self {
        Self { local: None, items }
    }

    /// Dropped files, as one `text/uri-list` item.
    pub fn files(paths: &[PathBuf]) -> Self {
        let mut list = String::new();
        for path in paths {
            list.push_str(&file_uri(path));
            list.push_str("\r\n");
        }
        Self::new(vec![DropItem::new(URI_LIST, list)])
    }

    /// The same data, with the payload of a drag from this process.
    pub fn local(mut self, payload: DragPayload) -> Self {
        self.local = Some(payload);
        self
    }

    /// The bytes read for `mime`, if any.
    pub fn get(&self, mime: &str) -> Option<&[u8]> {
        self.items
            .iter()
            .find(|i| i.mime == mime)
            .map(|i| i.data.as_slice())
    }

    /// The URIs of the `text/uri-list` item, if any.
    pub fn uris(&self) -> Vec<String> {
        self.get(URI_LIST).map(parse_uri_list).unwrap_or_default()
    }

    /// The local files among [`DropData::uris`].
    pub fn file_paths(&self) -> Vec<PathBuf> {
        self.uris().iter().filter_map(|u| uri_to_path(u)).collect()
    }
}

/// A drop on a zone: what was offered, what arrived, the modifiers held, and where it landed.
///
/// Fields may be added: build one with [`Dropped::new`].
///
/// Used by: C1e, C6, C7, C8, X2.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Dropped {
    /// What the drag offered.
    pub offer: DropOffer,
    /// What the drop delivered.
    pub data: DropData,
    /// The modifiers held at the drop, for [`choose_action`].
    pub mods: Mods,
    /// Where on the zone it landed, in the zone's own points and the window's, when the
    /// platform said.
    pub point: Option<PointerEvent>,
}

impl Dropped {
    /// A drop with no position.
    pub fn new(offer: DropOffer, data: DropData, mods: Mods) -> Self {
        Self {
            offer,
            data,
            mods,
            point: None,
        }
    }

    /// The same drop, at `point`.
    pub fn point(mut self, point: Option<PointerEvent>) -> Self {
        self.point = point;
        self
    }
}

/// A zone's hover state, as a drag moves over the window.
///
/// Used by: C1e, C6, C7, C8, X2.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DropHover {
    /// No drag is over the zone: it left, moved to another zone, or dropped.
    Left,
    /// A drag the zone accepts is over it: draw the drop highlight.
    Accepting,
    /// A drag the zone refuses is over it, and the zone has [`DropZone::on_reject`]: draw
    /// "can't drop here".
    Rejecting,
}

type DropFn<S> = Rc<dyn Fn(&mut S, Dropped, &mut Tx)>;
type HoverFn<S> = Rc<dyn Fn(&mut S, DropHover, &DropOffer, &mut Tx)>;
type MoveFn<S> = Rc<dyn Fn(&mut S, PointerEvent, &mut Tx)>;
type RejectFn<S> = Rc<dyn Fn(&mut S, DropOffer, &mut Tx)>;
type ActionFn<S> = Rc<dyn Fn(&S, &DropOffer, Mods) -> Option<DropAction>>;

/// A drop target: the MIME types it reads, and its callbacks for drop, hover, move and reject.
///
/// Each callback mirrors a Rune handler, with the app state `S`: `on_drop_files`,
/// `on_drop_hover` (and `on_drop_rejected_hover`), `on_drop_move` and `on_drop_rejected`. A
/// component builds one per target element as it builds its view, capturing that target (a
/// crumb, a tab, a gap's index, a folder's [`EntryId`]) in the callbacks, which report it in
/// the component's own typed intent. It attaches the zone through the host's [`Dnd`] binder.
///
/// # Accepting a drag
///
/// Whether a zone takes a drag is decided when the view is built, where the component sees
/// its state (binders' acceptance tests, Rune's among them, get none): by whether it builds the
/// zone, its MIME types and local flag, [`DropZone::on_reject`], and [`DropZone::action`].
///
/// A host that must look at the dragged items first (C8 pins only folders, even from another
/// app) hears the offer in [`DropZone::on_hover`]: its [`DropOffer::serial`], and its
/// [`DropOffer::uris`] when the binder could read them. It checks them, perhaps on a worker,
/// and keeps the result keyed by that serial. The component then builds the zone from the
/// host's checked state for that serial: refusing it with an `action` that returns `None`, or
/// with no MIME types and an `on_reject`. Until the check is in, the zone is built as the host
/// sees fit for an unchecked drag.
///
/// # Refusing a drag
///
/// [`DropZone::action`] returning `None` refuses the drag. A binder that consults it while the
/// drag hovers shows [`DropHover::Rejecting`] if the zone has an `on_reject`, and otherwise
/// passes the drag to the zone around it. A refused drag is never delivered to `on_drop`; on a
/// drop it goes to `on_reject`, if any. Rune's binder can't consult it while hovering (Rune
/// routes file drags with no state), so there the zone shows `Accepting` until the drop, then
/// refuses it at the drop: `on_reject` hears it, or nothing does. It can't pass it on.
///
/// Used by: C1e (folder rows, the pane background), C6 (crumbs), C7 (tabs, New tab), C8 (pin
/// gaps, places, Trash), X2.
pub struct DropZone<S> {
    mime_types: MimeList,
    local: bool,
    on_drop: DropFn<S>,
    on_hover: Option<HoverFn<S>>,
    on_move: Option<MoveFn<S>>,
    on_reject: Option<RejectFn<S>>,
    action: Option<ActionFn<S>>,
}

impl<S> Clone for DropZone<S> {
    fn clone(&self) -> Self {
        Self {
            mime_types: self.mime_types.clone(),
            local: self.local,
            on_drop: self.on_drop.clone(),
            on_hover: self.on_hover.clone(),
            on_move: self.on_move.clone(),
            on_reject: self.on_reject.clone(),
            action: self.action.clone(),
        }
    }
}

impl<S: 'static> DropZone<S> {
    /// A zone that reads `text/uri-list` and takes drags from this process, calling `on_drop`
    /// with each drop.
    pub fn new(on_drop: impl Fn(&mut S, Dropped, &mut Tx) + 'static) -> Self {
        Self {
            mime_types: uri_list_only(),
            local: true,
            on_drop: Rc::new(on_drop),
            on_hover: None,
            on_move: None,
            on_reject: None,
            action: None,
        }
    }

    /// The same zone, reading `preference` instead: the MIME types it takes, most wanted first.
    pub fn mime_types(mut self, preference: &[&'static str]) -> Self {
        self.mime_types = preference.iter().map(|&m| Cow::Borrowed(m)).collect();
        self
    }

    /// The same zone, taking drags from this process (with their [`DragPayload`]) or not.
    pub fn local(mut self, accept: bool) -> Self {
        self.local = accept;
        self
    }

    /// Hear the zone's hover state change, to draw a drop highlight, with what the drag offers:
    /// a host checks the offer's items here and keys the result by its serial (see "Accepting a
    /// drag" above).
    pub fn on_hover(
        mut self,
        f: impl Fn(&mut S, DropHover, &DropOffer, &mut Tx) + 'static,
    ) -> Self {
        self.on_hover = Some(Rc::new(f));
        self
    }

    /// Follow a drag the zone accepts as it moves, at most once a frame, in the zone's own
    /// points: for a drop caret or edge auto-scroll. Only a zone with this asks for frames
    /// while a drag moves. Only the innermost zone under the drag hears it.
    pub fn on_move(mut self, f: impl Fn(&mut S, PointerEvent, &mut Tx) + 'static) -> Self {
        self.on_move = Some(Rc::new(f));
        self
    }

    /// Hear a drop the zone refuses: one it doesn't accept, or one its [`DropZone::action`]
    /// refuses. With this, the zone stops a drag it doesn't accept, showing
    /// [`DropHover::Rejecting`], instead of passing it to the zone around it.
    pub fn on_reject(mut self, f: impl Fn(&mut S, DropOffer, &mut Tx) + 'static) -> Self {
        self.on_reject = Some(Rc::new(f));
        self
    }

    /// Say which action a drag over the zone would take, so a binder that negotiates with the
    /// drag source while it hovers (the system shim's) can show it before the drop. The host
    /// decides it, usually with [`choose_action`] and its volume lookup for this target.
    /// Without it, binders use [`choose_action`] with an unknown volume and the default
    /// cross-device setting.
    ///
    /// `None` refuses the drag: see "Refusing a drag" above. Rune's binder asks it only at the
    /// drop.
    pub fn action(
        mut self,
        f: impl Fn(&S, &DropOffer, Mods) -> Option<DropAction> + 'static,
    ) -> Self {
        self.action = Some(Rc::new(f));
        self
    }

    /// Whether the zone takes a drag offering `offer`: one from this process, if it takes
    /// those, or one offering any of its MIME types. Its `action` may still refuse it.
    pub fn accepts(&self, offer: &DropOffer) -> bool {
        (self.local && offer.local) || self.mime_types.iter().any(|m| offer.offers(m))
    }

    /// Whether the zone reads `mime`, so that a binder whose drags all offer one type checks
    /// acceptance without building an offer.
    pub fn reads(&self, mime: &str) -> bool {
        self.mime_types.iter().any(|m| m == mime)
    }

    /// The MIME types the zone reads, most wanted first. For binders.
    pub fn preferred_mime_types(&self) -> &[Cow<'static, str>] {
        &self.mime_types
    }

    /// Whether the zone takes drags from this process. For binders.
    pub fn accepts_local(&self) -> bool {
        self.local
    }

    /// Whether the zone follows moves ([`DropZone::on_move`]). For binders.
    pub fn follows_moves(&self) -> bool {
        self.on_move.is_some()
    }

    /// Whether the zone stops drags it refuses ([`DropZone::on_reject`]). For binders.
    pub fn rejects(&self) -> bool {
        self.on_reject.is_some()
    }

    /// Delivers a drop. For binders.
    pub fn deliver(&self, state: &mut S, dropped: Dropped, tx: &mut Tx) {
        (self.on_drop)(state, dropped, tx);
    }

    /// Whether the zone hears hover changes. For binders.
    pub fn hears_hover(&self) -> bool {
        self.on_hover.is_some()
    }

    /// Reports the hover state, with the drag's offer. For binders.
    pub fn hover(&self, state: &mut S, hover: DropHover, offer: &DropOffer, tx: &mut Tx) {
        if let Some(f) = &self.on_hover {
            f(state, hover, offer, tx);
        }
    }

    /// Reports a move. For binders.
    pub fn moved(&self, state: &mut S, point: PointerEvent, tx: &mut Tx) {
        if let Some(f) = &self.on_move {
            f(state, point, tx);
        }
    }

    /// Reports a refused drop. For binders.
    pub fn reject(&self, state: &mut S, offer: DropOffer, tx: &mut Tx) {
        if let Some(f) = &self.on_reject {
            f(state, offer, tx);
        }
    }

    /// The action a drag offering `offer` with `mods` held would take here, or `None` if the
    /// zone refuses it: it doesn't accept the offer, or its `action` says `None`. A binder
    /// delivers a drop only when this is `Some`. For binders.
    pub fn choose(&self, state: &S, offer: &DropOffer, mods: Mods) -> Option<DropAction> {
        if !self.accepts(offer) {
            return None;
        }
        match &self.action {
            Some(f) => f(state, offer, mods),
            None => choose_action(ActionInput {
                mods,
                offered: offer.actions,
                volume: VolumeRelation::Unknown,
                cross_device: CrossDevice::default(),
            }),
        }
    }
}

type PayloadFn<S> = Rc<dyn Fn(&S) -> Option<DragPayload>>;
type EndFn<S> = Rc<dyn Fn(&mut S, rune::DragOutcome, &mut Tx)>;

/// A drag source: the payload a drag from the element carries, read once the pointer passes
/// the drag threshold, and how the drag ended. It mirrors Rune's `drag_out` and
/// `on_drag_end`, and reuses Rune's `DragOutcome`.
///
/// Used by: C1e (rows, and C2, C3 and C4 through it), X2.
pub struct DragSource<S> {
    payload: PayloadFn<S>,
    on_end: Option<EndFn<S>>,
}

impl<S> Clone for DragSource<S> {
    fn clone(&self) -> Self {
        Self {
            payload: self.payload.clone(),
            on_end: self.on_end.clone(),
        }
    }
}

impl<S: 'static> DragSource<S> {
    /// A source whose drag carries what `payload` returns from the current state. `None`
    /// cancels the drag, and the press stays a press.
    pub fn new(payload: impl Fn(&S) -> Option<DragPayload> + 'static) -> Self {
        Self {
            payload: Rc::new(payload),
            on_end: None,
        }
    }

    /// Hear how the drag ended.
    pub fn on_end(mut self, f: impl Fn(&mut S, rune::DragOutcome, &mut Tx) + 'static) -> Self {
        self.on_end = Some(Rc::new(f));
        self
    }

    /// The payload a drag starting now carries. For binders.
    pub fn payload(&self, state: &S) -> Option<DragPayload> {
        (self.payload)(state)
    }

    /// Whether the source hears how its drags end. For binders.
    pub fn hears_end(&self) -> bool {
        self.on_end.is_some()
    }

    /// Reports how the drag ended. For binders.
    pub fn end(&self, state: &mut S, outcome: rune::DragOutcome, tx: &mut Tx) {
        if let Some(f) = &self.on_end {
            f(state, outcome, tx);
        }
    }
}
