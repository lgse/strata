// SPDX-License-Identifier: MIT

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use rune::{Chord, El, KeyEvent, KeyPlatform, Keybind, Mods, PhysicalKeyCode, PointerButton, Tx};
use strata_kit::look::Look;

use crate::collection::selection::{Changed, Selection};
use crate::contract::{CollectionGeometry, Direction, EntryId, EntrySource, Point, Rect};

/// Which part of a pointer gesture an input is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PointerPhase {
    /// A button went down. A secondary press arrives only as this, with no move or release.
    Down,
    /// The pointer moved while the button is held.
    Move,
    /// The button came up, or the gesture was cancelled ([`PointerInput::cancelled`]).
    Up,
}

/// Where in a row a point falls, as Strata's pointer intent tells them apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Region {
    /// On the drawn text of the entry's name: where a slow second click renames.
    Name,
    /// On the rest of the entry itself: its icon or thumbnail.
    Content,
    /// Blank row space, or no entry at all.
    Gutter,
}

impl Region {
    /// Whether the point is on the entry (its name or the rest of its content), not in the gutter.
    pub fn on_entry(self) -> bool {
        matches!(self, Region::Name | Region::Content)
    }
}

/// What a pointer input is over.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Hit {
    /// The entry under the pointer, or `None` over empty space.
    pub index: Option<usize>,
    /// Which part of the entry's row. Empty space is [`Region::Gutter`].
    pub region: Region,
}

/// One pointer input on the collection, in content coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PointerInput {
    /// Down, move or up.
    pub phase: PointerPhase,
    /// The button that drove it: primary, middle, or secondary (a `PointerPhase::Down` only).
    pub button: PointerButton,
    /// The pointer in content coordinates (see `contract::geometry`).
    pub point: Point,
    /// The pointer in window points, for popups and native drags.
    pub window: Point,
    /// The pointer's travel since the press, in points.
    pub travel: Point,
    /// What the pointer is over.
    pub hit: Hit,
    /// The modifiers held.
    pub mods: Mods,
    /// Which click of a run the press is: 1, then 2.
    pub clicks: u32,
    /// On [`PointerPhase::Up`]: the gesture ended without a release (Escape, focus lost).
    pub cancelled: bool,
    /// The host's clock, in seconds (virtual in tests).
    pub now: f64,
}

impl PointerInput {
    /// An input at content point `point` over `hit`, as a controller's unit test builds one.
    pub fn new(phase: PointerPhase, button: PointerButton, point: Point, hit: Hit) -> Self {
        Self {
            phase,
            button,
            point,
            window: point,
            travel: Point::default(),
            hit,
            mods: Mods::default(),
            clicks: 1,
            cancelled: false,
            now: 0.0,
        }
    }
}

/// One key the focused collection heard.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct KeyInput<'a> {
    /// The key and its modifiers, as Rune reports them.
    pub event: &'a KeyEvent,
    /// The physical key behind it, for chords matched by position.
    pub physical: Option<PhysicalKeyCode>,
    /// The character the key types with no modifiers under the current layout.
    pub base: Option<char>,
    /// Whether the key is repeating.
    pub repeat: bool,
    /// The platform whose conventions chords resolve with: this build's.
    pub platform: KeyPlatform,
    /// The host's clock, in seconds.
    pub now: f64,
    pub(super) tx: Option<&'a Tx>,
}

impl<'a> KeyInput<'a> {
    /// `event` with no physical key or base, not repeating, on this build's platform, at time 0.
    pub fn new(event: &'a KeyEvent) -> Self {
        Self {
            event,
            physical: None,
            base: None,
            repeat: false,
            platform: KeyPlatform::current(),
            now: 0.0,
            tx: None,
        }
    }

    /// Whether the key is `chord`, matched as `Chord::matches_tx` matches it.
    pub fn matches(&self, chord: &Chord) -> bool {
        match self.tx {
            Some(tx) => chord.matches_tx_on(self.event, tx, self.platform),
            None => chord.matches_on(self.event, self.platform),
        }
    }

    /// Whether the key is any of `keybind`'s chords on this platform, as `KeyInput::matches` matches one.
    pub fn matches_keybind(&self, keybind: &Keybind) -> bool {
        match self.tx {
            Some(tx) => keybind.matches_tx_on(self.event, tx, self.platform),
            None => keybind.matches_on(self.event, self.platform),
        }
    }
}

/// Whether a controller took an input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Flow {
    /// It took it: nothing else hears it.
    Handled,
    /// It didn't: the next in line hears it (a key goes on to the page and to Rune).
    Pass,
}

/// How an entry was activated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Activation {
    /// A single click, in single-click mode.
    Click,
    /// A double click.
    DoubleClick,
    /// Return, or another key.
    Key,
}

/// What a controller asks the host to do.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Intent {
    /// Open the entry.
    Activate {
        /// The entry.
        id: EntryId,
        /// How.
        how: Activation,
    },
    /// A plain single click on the entry finished (released where it was pressed, no modifiers).
    Clicked {
        /// The entry.
        id: EntryId,
    },
    /// Open the entry in a new tab, or a new window (a middle click).
    OpenInNew {
        /// The entry.
        id: EntryId,
        /// A new window rather than a tab.
        window: bool,
    },
    /// Reveal the entry in its folder (a middle click on a file).
    Reveal {
        /// The entry.
        id: EntryId,
    },
    /// Start a drag of these entries: the host builds the `DragPayload` and starts the system drag.
    BeginDrag {
        /// The dragged entries, in display order.
        ids: Vec<EntryId>,
        /// Where the pointer is, in window points: where the drag image starts.
        window: Point,
    },
    /// Start renaming the entry.
    BeginRename {
        /// The entry.
        id: EntryId,
    },
    /// Open a context menu for the entry, or the background, at `anchor` (window points).
    ContextMenu {
        /// The entry, or `None` for the background.
        target: Option<EntryId>,
        /// Where to open it, in window points.
        anchor: Rect,
    },
    /// The cursor can't move this way: it is at the collection's edge.
    Edge(Direction),
    /// Anything else a controller and its host agree on.
    Other(Rc<dyn Any>),
}

/// What a controller asks the element to do after an input.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Effect {
    /// Scroll the entry at this index into view, mounted or not.
    Reveal(usize),
    /// Start a marquee at this content point, with these modifiers.
    BeginMarquee {
        /// The press, in content coordinates.
        at: Point,
        /// The modifiers held at the press.
        mods: Mods,
    },
    /// Scroll the viewport so its top shows content y `y`: a glide on Rune's default spring when `glide`, else a jump.
    ScrollTo {
        /// The content offset to show at the top.
        y: f32,
        /// Glide on Rune's spring rather than jump.
        glide: bool,
    },
    /// Glide the viewport so its top shows content y `y`, on `spring`.
    Glide {
        /// The content offset to show at the top.
        y: f32,
        /// The spring to glide on.
        spring: rune::Spring,
    },
    /// Halt any running glide or momentum on the viewport where it is.
    StopScroll,
    /// Keyboard mode on (the cursor outline shows, row hover waits for the pointer to move) or off.
    KeyboardMode(bool),
    /// Run the controllers' `tick` again after this long: a timeout.
    WakeAfter(Duration),
    /// Hand the host an intent.
    Intent(Intent),
}

/// What a controller sees and changes while it handles an input.
pub struct Ctx<'a> {
    /// The entries, in display order.
    pub source: &'a dyn EntrySource,
    /// Where they sit, mounted or not.
    pub geometry: &'a dyn CollectionGeometry,
    /// The part of the content the viewport shows, in content coordinates.
    pub visible: Rect,
    /// The host's clock, in seconds.
    pub now: f64,
    selection: &'a mut Selection,
    changed: Changed,
    effects: Vec<Effect>,
    read_only: bool,
    animations: bool,
    origin: Point,
    settings: Rc<Settings>,
    repaint: bool,
}

impl<'a> Ctx<'a> {
    /// A context over `source` and `geometry`, changing `selection`.
    pub fn new(
        source: &'a dyn EntrySource,
        geometry: &'a dyn CollectionGeometry,
        visible: Rect,
        now: f64,
        selection: &'a mut Selection,
    ) -> Self {
        Self {
            source,
            geometry,
            visible,
            now,
            selection,
            changed: Changed::NONE,
            effects: Vec::new(),
            read_only: false,
            animations: true,
            origin: Point::default(),
            settings: Rc::new(Settings::default()),
            repaint: false,
        }
    }

    /// The same context for a read-only collection: no rename, no drag.
    pub fn read_only_mode(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    /// The same context with animations on or off.
    pub fn with_animations(mut self, animations: bool) -> Self {
        self.animations = animations;
        self
    }

    /// The same context, with the viewport's top-left at `origin` in window points.
    pub fn window_origin(mut self, origin: Point) -> Self {
        self.origin = origin;
        self
    }

    /// The same context, with the host's settings for its controllers (see `Ctx::settings`).
    pub fn with_settings(mut self, settings: Rc<Settings>) -> Self {
        self.settings = settings;
        self
    }

    /// Whether the collection is read-only: a controller asks for no rename and no drag.
    pub fn read_only(&self) -> bool {
        self.read_only
    }

    /// Whether motion is on: false when the user or the desktop turned animations off (`Look::animations`).
    pub fn animations(&self) -> bool {
        self.animations
    }

    /// `rect`, in content coordinates, in window points: where a popup anchored to an entry opens.
    pub fn content_to_window(&self, rect: Rect) -> Rect {
        rect.offset(
            self.origin.x - self.visible.x,
            self.origin.y - self.visible.y,
        )
    }

    /// The host's settings value of type `T`, if it gave one (`Collection::settings`).
    pub fn settings<T: 'static>(&self) -> Option<&T> {
        self.settings.get::<T>()
    }

    /// Says something the view reads changed, though no selection operation shows it (a band shown or hidden.
    pub fn repaint(&mut self) {
        self.repaint = true;
    }

    /// How tall the content is: the bottom of the last entry, in content coordinates.
    pub fn content_extent(&self) -> f32 {
        match self.source.len() {
            0 => 0.0,
            n => self.geometry.rect_of(n - 1).bottom(),
        }
    }

    /// The selection, to read.
    pub fn selection(&self) -> &Selection {
        self.selection
    }

    /// Runs one selection operation on the source.
    pub fn apply(
        &mut self,
        op: impl FnOnce(&mut Selection, &dyn EntrySource) -> Changed,
    ) -> Changed {
        let changed = op(self.selection, self.source);
        self.changed |= changed;
        changed
    }

    /// Asks the element for `effect` once the input is handled.
    pub fn emit(&mut self, effect: Effect) {
        self.effects.push(effect);
    }

    /// What the operations so far changed.
    pub fn changed(&self) -> Changed {
        self.changed
    }

    /// The effects asked for so far, in order.
    pub fn effects(&self) -> &[Effect] {
        &self.effects
    }

    pub(super) fn finish(self) -> (Changed, Vec<Effect>, bool) {
        (self.changed, self.effects, self.repaint)
    }
}

/// The host's settings for its controllers, one value per type: what `Ctx::settings` reads.
#[derive(Clone, Default)]
pub struct Settings {
    values: HashMap<TypeId, Rc<dyn Any>>,
}

impl Settings {
    /// Adds `value`, replacing any value of its type.
    pub fn insert<T: Any>(&mut self, value: T) {
        self.values.insert(TypeId::of::<T>(), Rc::new(value));
    }

    /// The value of type `T`, if any.
    pub fn get<T: Any>(&self) -> Option<&T> {
        self.values
            .get(&TypeId::of::<T>())
            .and_then(|v| v.downcast_ref::<T>())
    }
}

impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settings")
            .field("values", &self.values.len())
            .finish()
    }
}

/// Pointer intent: presses, moves and releases on the collection.
pub trait PointerController {
    /// Handles one pointer input.
    fn pointer(&mut self, input: &PointerInput, cx: &mut Ctx<'_>) -> Flow;

    /// Runs when a timeout it asked for (`Effect::WakeAfter`) may have passed, from the host's poll (`Collection::tick`).
    fn tick(&mut self, _cx: &mut Ctx<'_>) -> Flow {
        Flow::Pass
    }

    /// Hears every key the collection hears, before the keyboard controller and whether or not anything takes it.
    fn key_seen(&mut self, _input: &KeyInput<'_>) {}

    /// Drops any gesture in progress: the collection shows another folder (`CollectionState::enter`).
    fn reset(&mut self) {}
}

/// Keyboard navigation: keys while the collection has focus.
pub trait KeyController {
    /// Handles one key. [`Flow::Pass`] lets the page and Rune hear it.
    fn key(&mut self, input: &KeyInput<'_>, cx: &mut Ctx<'_>) -> Flow;

    /// As `PointerController::tick` (a type-ahead timeout).
    fn tick(&mut self, _cx: &mut Ctx<'_>) -> Flow {
        Flow::Pass
    }

    /// Hears every pointer input the collection routes, before the pointer controller and the marquee.
    fn pointer_seen(&mut self, _input: &PointerInput) {}

    /// As [`PointerController::reset`] (a type-ahead buffer).
    fn reset(&mut self) {}
}

/// Marquee selection.
pub trait MarqueeController {
    /// Starts a band at `at` with `mods` held.
    fn begin(&mut self, at: Point, mods: Mods, cx: &mut Ctx<'_>);

    /// The pointer moved to `at` (content coordinates).
    fn moved(&mut self, at: Point, cx: &mut Ctx<'_>);

    /// The button came up: keep what the band selected.
    fn end(&mut self, cx: &mut Ctx<'_>);

    /// Escape, or a cancelled gesture: put the selection back. False if no band was open.
    fn cancel(&mut self, cx: &mut Ctx<'_>) -> bool;

    /// Whether a band is open.
    fn active(&self) -> bool;

    /// The band, in content coordinates.
    fn band(&self) -> Option<Rect>;

    /// What the band draws, in content coordinates: place it with `El::absolute(x, y)` at the band's content position.
    fn overlay(&self, _look: &Look) -> Option<El> {
        None
    }

    /// As [`PointerController::tick`] (edge auto-scroll).
    fn tick(&mut self, _cx: &mut Ctx<'_>) -> Flow {
        Flow::Pass
    }

    /// Drops an open band without touching the selection, which the new folder replaces (`CollectionState::enter`).
    fn reset(&mut self) {}

    /// A key moved the cursor, and the element is scrolling it into view.
    fn yield_scroll(&mut self) -> bool {
        false
    }
}

/// The controllers a collection routes to.
#[derive(Default)]
pub struct Controllers {
    /// Pointer intent.
    pub pointer: Option<Box<dyn PointerController>>,
    /// Keyboard navigation.
    pub keyboard: Option<Box<dyn KeyController>>,
    /// Marquee..
    pub marquee: Option<Box<dyn MarqueeController>>,
}

impl Controllers {
    /// Drops every controller's gesture in progress.
    pub fn reset(&mut self) {
        if let Some(p) = self.pointer.as_mut() {
            p.reset();
        }
        if let Some(k) = self.keyboard.as_mut() {
            k.reset();
        }
        if let Some(m) = self.marquee.as_mut() {
            m.reset();
        }
    }
}

impl std::fmt::Debug for Controllers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Controllers")
            .field("pointer", &self.pointer.is_some())
            .field("keyboard", &self.keyboard.is_some())
            .field("marquee", &self.marquee.is_some())
            .finish()
    }
}

const CLICK_SLOP: f32 = 4.0;

/// The pointer behaviour with no pointer controller.
pub fn default_pointer(input: &PointerInput, cx: &mut Ctx<'_>) -> Flow {
    if input.button != PointerButton::Primary {
        return Flow::Pass;
    }
    match input.phase {
        PointerPhase::Down => {
            match input.hit.index {
                Some(i) => cx.apply(|sel, src| sel.select_only(src, i)),
                None => cx.apply(|sel, _| sel.clear()),
            };
            Flow::Handled
        }
        PointerPhase::Up => {
            let plain = !(input.mods.shift || input.mods.ctrl || input.mods.cmd || input.mods.alt);
            let still = input.travel.x.abs() <= CLICK_SLOP && input.travel.y.abs() <= CLICK_SLOP;
            if let Some(i) = input.hit.index
                && plain
                && still
                && !input.cancelled
                && input.clicks == 1
                && i < cx.source.len()
            {
                let id = cx.source.id_at(i);
                cx.emit(Effect::Intent(Intent::Clicked { id }));
            }
            Flow::Handled
        }
        PointerPhase::Move => Flow::Pass,
    }
}
