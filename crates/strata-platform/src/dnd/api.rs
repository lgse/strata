//! What a drag carries, what a drop offers and delivers, and the source and target an app
//! attaches to its elements.

use std::any::Any;
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;

use rune::{DragOutcome, Mods, Tx};

use crate::transfer::mime;
use crate::transfer::{SourceSpec, Writer};

use super::icon::DragIcon;

/// What a drop does with what it carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DragAction {
    Copy,
    Move,
    /// X11 only: Wayland's data device has no link action, so it is left out there.
    Link,
    /// The target asks the user which action, after the drop.
    Ask,
}

/// A set of actions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Actions(u8);

impl Actions {
    pub const NONE: Actions = Actions(0);
    pub const COPY: Actions = Actions(1);
    pub const MOVE: Actions = Actions(2);
    pub const LINK: Actions = Actions(4);
    pub const ASK: Actions = Actions(8);

    pub const fn of(action: DragAction) -> Actions {
        match action {
            DragAction::Copy => Actions::COPY,
            DragAction::Move => Actions::MOVE,
            DragAction::Link => Actions::LINK,
            DragAction::Ask => Actions::ASK,
        }
    }

    pub const fn union(self, other: Actions) -> Actions {
        Actions(self.0 | other.0)
    }

    pub const fn intersect(self, other: Actions) -> Actions {
        Actions(self.0 & other.0)
    }

    pub const fn has(self, action: DragAction) -> bool {
        self.0 & Actions::of(action).0 != 0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The actions in the set, in the order copy, move, link, ask.
    pub fn iter(self) -> impl Iterator<Item = DragAction> {
        [
            DragAction::Copy,
            DragAction::Move,
            DragAction::Link,
            DragAction::Ask,
        ]
        .into_iter()
        .filter(move |a| self.has(*a))
    }
}

impl std::ops::BitOr for Actions {
    type Output = Actions;
    fn bitor(self, rhs: Actions) -> Actions {
        self.union(rhs)
    }
}

impl From<DragAction> for Actions {
    fn from(a: DragAction) -> Actions {
        Actions::of(a)
    }
}

impl FromIterator<DragAction> for Actions {
    fn from_iter<I: IntoIterator<Item = DragAction>>(iter: I) -> Actions {
        iter.into_iter()
            .fold(Actions::NONE, |s, a| s | Actions::of(a))
    }
}

/// The plain-text types [`DragItem::text`] offers.
pub const TEXT_TYPES: [&str; 2] = ["text/plain;charset=utf-8", "text/plain"];

/// What a drag carries: MIME types, each with its bytes (made lazily, only when a target
/// asks), the actions it allows, its icon, and an in-process payload for targets in this
/// process. Mirrors Rune's `DragItem`, with any MIME type.
#[derive(Clone)]
pub struct DragItem {
    pub(crate) types: Vec<(String, Writer)>,
    pub(crate) actions: Actions,
    pub(crate) icon: Option<DragIcon>,
    pub(crate) local: Option<Rc<dyn Any>>,
}

impl Default for DragItem {
    fn default() -> Self {
        DragItem::new()
    }
}

impl DragItem {
    /// Nothing yet, allowing copy.
    pub fn new() -> Self {
        DragItem {
            types: Vec::new(),
            actions: Actions::COPY,
            icon: None,
            local: None,
        }
    }

    /// Offers `mime` with these bytes. A type offered twice keeps its first data.
    pub fn data(self, mime: impl Into<String>, bytes: impl Into<Arc<[u8]>>) -> Self {
        self.with(mime.into(), Writer::bytes(bytes))
    }

    /// Offers `mime`, made by `make` on a worker thread the first time a target asks for it,
    /// and kept for the drag's life (targets ask again).
    pub fn lazy(
        self,
        mime: impl Into<String>,
        make: impl FnOnce() -> Vec<u8> + Send + 'static,
    ) -> Self {
        self.with(mime.into(), Writer::lazy(make))
    }

    /// Offers `text` as `text/plain;charset=utf-8` and `text/plain`.
    pub fn text(self, text: &str) -> Self {
        let bytes: Arc<[u8]> = Arc::from(text.as_bytes());
        TEXT_TYPES
            .iter()
            .fold(self, |item, m| item.data(*m, bytes.clone()))
    }

    /// Offers `uris` as `text/uri-list` (one per line, CRLF).
    pub fn uris<U: AsRef<str>>(self, uris: &[U]) -> Self {
        self.data(mime::URI_LIST, mime::encode_uri_list(uris))
    }

    /// The actions the drag allows (copy alone by default). Ctrl, Shift or both at the start
    /// narrow them: see [`super::negotiate`].
    pub fn actions(mut self, actions: impl Into<Actions>) -> Self {
        self.actions = actions.into();
        self
    }

    pub fn icon(mut self, icon: DragIcon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// A typed payload for targets in this process ([`DropTarget::local`]), which get it
    /// instead of reading the serialized types. It stays on the UI thread.
    pub fn local<T: Any>(mut self, payload: T) -> Self {
        self.local = Some(Rc::new(payload));
        self
    }

    pub fn mimes(&self) -> impl Iterator<Item = &str> {
        self.types.iter().map(|(m, _)| m.as_str())
    }

    pub fn allowed(&self) -> Actions {
        self.actions
    }

    /// Whether there is anything to drag: a type or a local payload, and an action.
    pub fn is_empty(&self) -> bool {
        (self.types.is_empty() && self.local.is_none()) || self.actions.is_empty()
    }

    fn with(mut self, mime: String, writer: Writer) -> Self {
        if !self.types.iter().any(|(m, _)| *m == mime) {
            self.types.push((mime, writer));
        }
        self
    }

    /// P0's source for these types, plus `extra` (the in-process token).
    pub(crate) fn spec(&self, extra: Option<&str>) -> SourceSpec {
        let spec = self
            .types
            .iter()
            .fold(SourceSpec::new(), |s, (m, w)| s.with(m.clone(), w.clone()));
        match extra {
            Some(token) => spec.with(token, Writer::bytes(Vec::new())),
            None => spec,
        }
    }
}

impl fmt::Debug for DragItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DragItem")
            .field("mimes", &self.mimes().collect::<Vec<_>>())
            .field("actions", &self.actions)
            .field("local", &self.local.is_some())
            .finish()
    }
}

/// What a drag over a target offers, known while it hovers.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct DropOffer {
    /// Its MIME types, the in-process token left out.
    pub mimes: Vec<String>,
    /// The actions the source allows. Empty until the source says (Wayland tells it just
    /// after the drag enters).
    pub actions: Actions,
    /// The drag comes from this process and carries a typed payload.
    pub local: bool,
    /// The modifiers held, where they can be known: X11 always; Wayland only for a drag from
    /// this process (as they were at its start). A drag from another client on Wayland
    /// reports none: its action is the compositor's.
    pub mods: Option<Mods>,
    /// A number for this drag, new each time a drag enters a window.
    pub serial: u64,
    /// The types read while the drag hovers ([`DropTarget::peek`]), as they arrive.
    pub peeked: Vec<(String, Vec<u8>)>,
}

impl DropOffer {
    pub fn offers(&self, mime: &str) -> bool {
        self.mimes.iter().any(|m| m == mime)
    }

    /// The bytes peeked for `mime`, if they have arrived.
    pub fn peeked(&self, mime: &str) -> Option<&[u8]> {
        self.peeked
            .iter()
            .find(|(m, _)| m == mime)
            .map(|(_, b)| b.as_slice())
    }
}

/// Where a drag is or dropped: in the target element's own points (from where it was last
/// drawn), and in the window's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DropPoint {
    pub x: f32,
    pub y: f32,
    pub window_x: f32,
    pub window_y: f32,
}

/// What a drop delivered.
#[derive(Clone)]
#[non_exhaustive]
pub struct DropData {
    pub offer: DropOffer,
    /// Each type read, with its bytes.
    pub items: Vec<(String, Vec<u8>)>,
    /// The types whose read failed, and why.
    pub failed: Vec<(String, String)>,
    /// The action the drop took. [`DragAction::Ask`] only in [`DropTarget::on_ask`].
    pub action: DragAction,
    pub point: Option<DropPoint>,
    pub(crate) local: Option<Rc<dyn Any>>,
}

impl DropData {
    pub fn get(&self, mime: &str) -> Option<&[u8]> {
        self.items
            .iter()
            .find(|(m, _)| m == mime)
            .map(|(_, b)| b.as_slice())
    }

    /// The URIs of the `text/uri-list` item, if read.
    pub fn uris(&self) -> Vec<String> {
        self.get(mime::URI_LIST)
            .map(mime::decode_uri_list)
            .unwrap_or_default()
    }

    /// The in-process payload, if the drag came from this process with one of type `T`.
    pub fn local<T: Any>(&self) -> Option<&T> {
        self.local.as_deref()?.downcast_ref::<T>()
    }
}

impl fmt::Debug for DropData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DropData")
            .field("offer", &self.offer)
            .field(
                "items",
                &self
                    .items
                    .iter()
                    .map(|(m, b)| (m, b.len()))
                    .collect::<Vec<_>>(),
            )
            .field("failed", &self.failed)
            .field("action", &self.action)
            .field("point", &self.point)
            .field("local", &self.local.is_some())
            .finish()
    }
}

/// A target's hover state as a drag moves over the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hover {
    /// No drag over the target: it left, moved to another target, dropped or ended.
    Left,
    /// A drag the target takes is over it, with the action a drop would take.
    Accepting(DragAction),
    /// A drag the target refuses is over it, and the target has [`DropTarget::on_reject`].
    Rejecting,
}

/// The answer to a drop whose action is ask. Dropping it unanswered refuses the drop.
pub struct AskReply(pub(crate) Option<Box<dyn FnOnce(Option<DragAction>)>>);

impl AskReply {
    /// Takes the drop with `action` (copy, move or link; ask refuses it).
    pub fn choose(mut self, action: DragAction) {
        if let Some(f) = self.0.take() {
            f((action != DragAction::Ask).then_some(action));
        }
    }

    /// Refuses the drop: the source keeps its data.
    pub fn refuse(mut self) {
        if let Some(f) = self.0.take() {
            f(None);
        }
    }
}

impl Drop for AskReply {
    fn drop(&mut self) {
        if let Some(f) = self.0.take() {
            f(None);
        }
    }
}

impl fmt::Debug for AskReply {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AskReply")
    }
}

pub(crate) type ItemFn<S> = Rc<dyn Fn(&S, Mods) -> Option<DragItem>>;
pub(crate) type StartFn<S> = Rc<dyn Fn(&mut S, &mut Tx)>;
pub(crate) type EndFn<S> = Rc<dyn Fn(&mut S, DragOutcome, &mut Tx)>;
pub(crate) type InnerDragFn<S> = Rc<dyn Fn(&mut S, &mut rune::Drag, &mut Tx)>;

/// A drag source: what the element's drag carries, read once the pointer crosses the
/// threshold, and how the drag ended. Mirrors Rune's `drag_out`, `on_drag_start` and
/// `on_drag_end`.
pub struct DragSource<S> {
    pub(crate) item: ItemFn<S>,
    pub(crate) on_start: Option<StartFn<S>>,
    pub(crate) on_end: Option<EndFn<S>>,
    pub(crate) inner: Option<InnerDragFn<S>>,
    pub(crate) threshold: f32,
}

impl<S> Clone for DragSource<S> {
    fn clone(&self) -> Self {
        DragSource {
            item: self.item.clone(),
            on_start: self.on_start.clone(),
            on_end: self.on_end.clone(),
            inner: self.inner.clone(),
            threshold: self.threshold,
        }
    }
}

/// Rune's drag threshold on Linux, in points.
pub const DRAG_THRESHOLD: f32 = 8.0;

impl<S: 'static> DragSource<S> {
    /// `item` runs once the pointer has travelled the threshold from the press, with the
    /// modifiers held then; `None` keeps the press a press.
    pub fn new(item: impl Fn(&S, Mods) -> Option<DragItem> + 'static) -> Self {
        DragSource {
            item: Rc::new(item),
            on_start: None,
            on_end: None,
            inner: None,
            threshold: DRAG_THRESHOLD,
        }
    }

    pub fn on_start(mut self, f: impl Fn(&mut S, &mut Tx) + 'static) -> Self {
        self.on_start = Some(Rc::new(f));
        self
    }

    /// How the drag ended: `Copied`, `Moved`, `Linked`, or `Cancelled` (dropped nowhere that
    /// took it, cancelled, or the platform failed).
    pub fn on_end(mut self, f: impl Fn(&mut S, DragOutcome, &mut Tx) + 'static) -> Self {
        self.on_end = Some(Rc::new(f));
        self
    }

    /// The element's own `on_drag`, which hears every phase that isn't the system drag's:
    /// the press, moves before the threshold, and a release before it.
    pub fn on_drag(mut self, f: impl Fn(&mut S, &mut rune::Drag, &mut Tx) + 'static) -> Self {
        self.inner = Some(Rc::new(f));
        self
    }

    /// The travel from the press that starts the drag, in points (8 by default, Rune's on
    /// Linux; the desktop's comes from P5's settings).
    pub fn threshold(mut self, points: f32) -> Self {
        self.threshold = points;
        self
    }
}

pub(crate) type DropFn<S> = Rc<dyn Fn(&mut S, DropData, &mut Tx)>;
pub(crate) type HoverFn<S> = Rc<dyn Fn(&mut S, Hover, &DropOffer, &mut Tx)>;
pub(crate) type MoveFn<S> = Rc<dyn Fn(&mut S, DropPoint, &mut Tx)>;
pub(crate) type RejectFn<S> = Rc<dyn Fn(&mut S, &DropOffer, &mut Tx)>;
pub(crate) type ActionFn<S> = Rc<dyn Fn(&S, &DropOffer, Option<Mods>) -> Option<DragAction>>;
pub(crate) type ActionAtFn<S> =
    Rc<dyn Fn(&S, &DropOffer, Option<Mods>, DropPoint) -> Option<DragAction>>;
pub(crate) type AskFn<S> = Rc<dyn Fn(&mut S, DropData, AskReply, &mut Tx)>;

/// A drop target: the MIME types it reads, and its callbacks. Mirrors Rune's
/// `on_drop_files`, `on_drop_hover`, `on_drop_move` and `on_drop_rejected`, for any type.
///
/// The target under the drag is the innermost element with a target that takes it. A
/// target takes a drag that offers one of its types, or (with [`DropTarget::local`]) one
/// from this process carrying a payload of its type, unless its [`DropTarget::action`]
/// refuses it. A target that doesn't take the drag passes it to the target around it,
/// unless it has [`DropTarget::on_reject`]: then it stops the drag and shows
/// [`Hover::Rejecting`].
pub struct DropTarget<S> {
    pub(crate) mimes: Vec<String>,
    pub(crate) read_all: bool,
    pub(crate) peek: Vec<String>,
    pub(crate) local: Option<std::any::TypeId>,
    pub(crate) on_drop: DropFn<S>,
    pub(crate) on_hover: Option<HoverFn<S>>,
    pub(crate) on_move: Option<MoveFn<S>>,
    pub(crate) on_reject: Option<RejectFn<S>>,
    pub(crate) action: Option<ActionFn<S>>,
    pub(crate) action_at: Option<ActionAtFn<S>>,
    pub(crate) on_ask: Option<AskFn<S>>,
}

impl<S> Clone for DropTarget<S> {
    fn clone(&self) -> Self {
        DropTarget {
            mimes: self.mimes.clone(),
            read_all: self.read_all,
            peek: self.peek.clone(),
            local: self.local,
            on_drop: self.on_drop.clone(),
            on_hover: self.on_hover.clone(),
            on_move: self.on_move.clone(),
            on_reject: self.on_reject.clone(),
            action: self.action.clone(),
            action_at: self.action_at.clone(),
            on_ask: self.on_ask.clone(),
        }
    }
}

impl<S: 'static> DropTarget<S> {
    /// A target reading `text/uri-list`, calling `on_drop` with each drop it takes.
    pub fn new(on_drop: impl Fn(&mut S, DropData, &mut Tx) + 'static) -> Self {
        DropTarget {
            mimes: vec![mime::URI_LIST.to_owned()],
            read_all: false,
            peek: Vec::new(),
            local: None,
            on_drop: Rc::new(on_drop),
            on_hover: None,
            on_move: None,
            on_reject: None,
            action: None,
            action_at: None,
            on_ask: None,
        }
    }

    /// The types it takes, most wanted first. A drop reads the first one offered, or with
    /// [`DropTarget::read_all`] every one offered.
    pub fn mime_types<M: AsRef<str>>(mut self, preference: &[M]) -> Self {
        self.mimes = preference.iter().map(|m| m.as_ref().to_owned()).collect();
        self
    }

    /// At the drop, read every one of its types the drag offers, not just the first.
    pub fn read_all(mut self) -> Self {
        self.read_all = true;
        self
    }

    /// Read these types while the drag hovers, before the drop: they arrive in
    /// [`DropOffer::peeked`] through `on_hover`.
    pub fn peek<M: AsRef<str>>(mut self, mimes: &[M]) -> Self {
        self.peek = mimes.iter().map(|m| m.as_ref().to_owned()).collect();
        self
    }

    /// Takes drags from this process carrying a payload of type `T` ([`DragItem::local`]),
    /// which [`DropData::local`] then gives, with nothing read.
    pub fn local<T: Any>(mut self) -> Self {
        self.local = Some(std::any::TypeId::of::<T>());
        self
    }

    pub fn on_hover(mut self, f: impl Fn(&mut S, Hover, &DropOffer, &mut Tx) + 'static) -> Self {
        self.on_hover = Some(Rc::new(f));
        self
    }

    /// Follows a drag over the target, on every motion, in the target's own points: one it
    /// takes, and, if the target has [`DropTarget::on_reject`], one it refuses (so a view can
    /// autoscroll under any drag, as a file manager's does).
    pub fn on_move(mut self, f: impl Fn(&mut S, DropPoint, &mut Tx) + 'static) -> Self {
        self.on_move = Some(Rc::new(f));
        self
    }

    /// Hears a drop the target refuses, and makes it stop drags it refuses instead of
    /// passing them on.
    pub fn on_reject(mut self, f: impl Fn(&mut S, &DropOffer, &mut Tx) + 'static) -> Self {
        self.on_reject = Some(Rc::new(f));
        self
    }

    /// Which action a drop here takes, asked on every motion and at the drop, with the
    /// modifiers when known. `None` refuses the drag. It wins over the modifier rule; without
    /// it, the rule in [`super::negotiate`] decides. An action the source doesn't allow is
    /// ignored, and the rule decides.
    pub fn action(
        mut self,
        f: impl Fn(&S, &DropOffer, Option<Mods>) -> Option<DragAction> + 'static,
    ) -> Self {
        self.action = Some(Rc::new(f));
        self
    }

    /// [`DropTarget::action`], told where the drag is: in the target's own points (from where
    /// it was last drawn) and the window's, as [`DropTarget::on_move`] is told just after. For
    /// a target that routes a drop by point, whose action depends on what is under the drag.
    /// It replaces `action`.
    pub fn action_at(
        mut self,
        f: impl Fn(&S, &DropOffer, Option<Mods>, DropPoint) -> Option<DragAction> + 'static,
    ) -> Self {
        self.action_at = Some(Rc::new(f));
        self
    }

    /// Hears a drop whose action is ask, with its data read: answer with the action the
    /// user picks. Without it, an ask drop takes the rule's action.
    pub fn on_ask(mut self, f: impl Fn(&mut S, DropData, AskReply, &mut Tx) + 'static) -> Self {
        self.on_ask = Some(Rc::new(f));
        self
    }

    /// Whether the target takes a drag offering `offer` (its `action` may still refuse).
    pub(crate) fn takes(&self, offer: &DropOffer, local: Option<&Rc<dyn Any>>) -> bool {
        self.takes_local(local) || self.mimes.iter().any(|m| offer.offers(m))
    }

    pub(crate) fn takes_local(&self, local: Option<&Rc<dyn Any>>) -> bool {
        match (self.local, local) {
            (Some(t), Some(p)) => (**p).type_id() == t,
            _ => false,
        }
    }

    /// The types a drop reads.
    pub(crate) fn reads(&self, offer: &DropOffer) -> Vec<String> {
        let offered = self.mimes.iter().filter(|m| offer.offers(m)).cloned();
        if self.read_all {
            offered.collect()
        } else {
            offered.take(1).collect()
        }
    }
}

/// What a window's drag and drop can do.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Status {
    /// `wayland` or `x11`.
    pub backend: &'static str,
    /// Drags into the app's windows are heard.
    pub drag_in: bool,
    /// Drags out of the app's windows can start.
    pub drag_out: bool,
    /// Why drag and drop is missing, when it is.
    pub unsupported: Option<String>,
}

/// A failed drag-and-drop call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// This platform or session has no route for it: libwayland < 1.23, no data device, not
    /// Wayland or X11.
    Unsupported(String),
    /// The window isn't attached ([`super::Dnd::attach`]), or the substrate was shut down.
    NotAttached,
    /// A request refused before it reached the platform.
    Refused(&'static str),
    /// No press to start a Wayland drag from.
    NoSerial,
    Platform(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unsupported(why) => write!(f, "drag and drop is unsupported here: {why}"),
            Error::NotAttached => f.write_str("drag and drop isn't attached to this window"),
            Error::Refused(why) => write!(f, "refused: {why}"),
            Error::NoSerial => f.write_str("no press to start the drag from"),
            Error::Platform(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<crate::transfer::Error> for Error {
    fn from(e: crate::transfer::Error) -> Self {
        use crate::transfer::Error as T;
        match e {
            T::Unsupported(u) => Error::Unsupported(u.to_string()),
            T::Shutdown => Error::NotAttached,
            T::NoSerial => Error::NoSerial,
            T::Refused(r) => Error::Refused(r),
            other => Error::Platform(other.to_string()),
        }
    }
}
