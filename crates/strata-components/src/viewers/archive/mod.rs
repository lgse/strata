//! The archive preview (`operations/archives/preview`): an archive's member tree, browsed like
//! a read-only folder, on C1e's collection element in read-only mode (no rename, no drops, no
//! drag out). Crumbs from "Contents" over a count line and the rows; a password prompt
//! ([`ArchivePrompt`]) for an encrypted archive.
//!
//! ```ignore
//! let tree = ArchiveTree::new(root, || ids.next());   // W1's EntryId rules: see `new`
//! s.archive = ArchiveState::new("archive", tree);     // passive: nothing highlighted
//! ArchiveView::new(|s: &App| &s.archive, |s: &mut App| &mut s.archive, |s: &App| &s.look)
//!     .letters(!s.type_to_search)
//!     .on_leave(|s: &mut App, _tx| s.focus_listing())
//!     .build(s)
//! ```
//!
//! # What it does
//!
//! - **Listing.** Folder rows end in "/" and show a chevron; file rows show their size, which
//!   assistive technology reads as the row's description, after its name (C1e's
//!   [`Describe`]). An empty folder says "This folder is empty". The order is the host's tree.
//! - **Crumbs.** "Contents" and the folders down to the one shown; inside a folder, a back arrow
//!   named "Go up one level" first. A crumb returns to its level.
//! - **Keys** (C1c with [`controllers::key_map`]): Up and Down (and Home, End, the page keys)
//!   move the highlight; Right or Enter opens a folder and does nothing on a file; Left goes to
//!   the parent. At the root Left reaches the host's [`ArchiveView::on_leave`] (Strata returns
//!   focus to the listing), and the tree takes the key; with no `on_leave`, it goes on past the
//!   tree to the host's key handling ([`controllers::LeftPasses`]). With Type to search off,
//!   `h j k l` act as the arrows ([`ArchiveView::letters`]). Edges are taken, so the keys never
//!   move focus out of the tree; only the cursor's member is outlined. Space isn't taken: it
//!   goes on to the host, which closes the preview.
//! - **Pointer** ([`controllers::ArchivePointer`]): a click highlights a member on its release,
//!   and opens a folder; modifiers act as a plain click; a double click opens one level. Hover
//!   tints a row and moves neither the highlight nor the cursor (C1e).
//! - **Passive until entered.** A new state highlights nothing ([`ArchiveState::passive`]), as
//!   when the cursor moves onto an archive with the preview open. [`ArchiveState::enter`]
//!   highlights the first member: Space opening the preview, Right from the listing, Enter in
//!   Columns, an Unlock.
//!
//! What stays in Strata: listing the archive (in its sandbox), the order, the limits and their
//! messages, and the routing around the tree: which keys enter it from the listing, Escape and
//! Space closing the preview, and where focus goes.
//!
//! The list's scroll bar (C24's, C24b) has no reset hook: Rune tells an element removed under a
//! drag nothing (Rune draft R-C4-2, in C4's docs), so a thumb drag cut off by the preview
//! closing leaves the bar held. If the host keeps the [`ArchiveState`] and shows it again, the
//! overlay bar shows until a later press on it ends; a fresh state starts clear.
//!
//! # Focus
//!
//! [`ArchiveState::enter`] highlights the first member; focus is the host's, for Strata's
//! "first member highlighted and focused": the list is the collection keyed
//! [`ArchiveState::scope`], one focus stop, so `tx.focus(archive.scope(), ring)` moves the
//! keyboard into it, as C9's
//! [`DrawerState::focus_content`](crate::preview_drawer::DrawerState::focus_content) does from
//! the listing. The prompt's field can be focused: it is a text field
//! ([`PromptState::focus`]).

pub mod controllers;
mod prompt;
mod tree;

#[cfg(test)]
mod tests;

use std::rc::Rc;

use rune::{El, Tx, col, icon, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant};
use strata_kit::icons::IconName;
use strata_kit::look::Look;

use crate::collection::element::{
    Collection, CollectionState, ContentSlot, Describe, Intent, viewport_path,
};
use crate::collection::selection::Listing;
use crate::contract::{Direction, EntrySource};
use crate::scroll::{Bar, BarLook, BarMode, ScrollFeed};

pub use prompt::{ArchivePrompt, PromptLabels, PromptState};
pub use tree::{ArchiveDir, ArchiveItem, ArchiveListing, ArchiveTree};

/// The words the archive preview shows and reads. Strata passes translated text; the defaults
/// are Strata's English.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ArchiveLabels {
    /// The root crumb.
    pub root: String,
    pub up: String,
    pub empty: String,
    /// The tree's accessible name.
    pub list: String,
    /// "3 files, 2 folders".
    pub counts: fn(usize, usize) -> String,
    /// "12.4 kB".
    pub size: fn(u64) -> String,
}

impl Default for ArchiveLabels {
    fn default() -> Self {
        ArchiveLabels {
            root: "Contents".into(),
            up: "Go up one level".into(),
            empty: "This folder is empty".into(),
            list: "Archive contents".into(),
            counts: |files, folders| {
                let count = |n: usize, one: &str, many: &str| match n {
                    1 => format!("1 {one}"),
                    n => format!("{n} {many}"),
                };
                match (files, folders) {
                    (0, 0) => "Empty folder".into(),
                    (f, 0) => count(f, "file", "files"),
                    (0, d) => count(d, "folder", "folders"),
                    (f, d) => format!(
                        "{}, {}",
                        count(f, "file", "files"),
                        count(d, "folder", "folders")
                    ),
                }
            },
            size: file_size,
        }
    }
}

/// Strata's `i18n::file_size`: decimal units, one decimal unless it would be ".0".
pub fn file_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    if bytes < 1_000 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1_000.0 && unit + 1 < UNITS.len() {
        value /= 1_000.0;
        unit += 1;
    }
    let mut rounded = (value * 10.0).round() / 10.0;
    if rounded >= 1_000.0 && unit + 1 < UNITS.len() {
        rounded /= 1_000.0;
        unit += 1;
    }
    if rounded.fract() == 0.0 {
        format!("{rounded:.0} {}", UNITS[unit])
    } else {
        format!("{rounded:.1} {}", UNITS[unit])
    }
}

/// The archive preview's state: the member tree, the folder shown, and the collection's state
/// over it.
#[derive(Debug)]
pub struct ArchiveState {
    scope: Rc<str>,
    listing: ArchiveListing,
    pub coll: CollectionState,
    labels: ArchiveLabels,
    version: u64,
    /// The list's scroll bar (C24) and where the list stands.
    bar: Bar,
    feed: ScrollFeed,
}

impl ArchiveState {
    /// The tree, at its root, passive. `scope` is the collection's key, which its rows' keys
    /// start with.
    pub fn new(scope: impl Into<Rc<str>>, tree: ArchiveTree) -> ArchiveState {
        let mut coll = CollectionState::new();
        coll.controllers.pointer = Some(Box::new(controllers::ArchivePointer::default()));
        coll.controllers.keyboard = Some(Box::<controllers::ArchiveKeys>::default());
        ArchiveState {
            scope: scope.into(),
            listing: ArchiveListing::new(tree),
            coll,
            labels: ArchiveLabels::default(),
            version: 0,
            bar: Bar::new(),
            feed: ScrollFeed::new(),
        }
    }

    pub fn with_labels(mut self, labels: ArchiveLabels) -> ArchiveState {
        self.labels = labels;
        self
    }

    pub fn scope(&self) -> &str {
        &self.scope
    }

    /// Where the list stands, as its scroll bar follows it.
    pub fn feed(&self) -> &ScrollFeed {
        &self.feed
    }

    /// When the list's scroll bar shows: overlay (Strata's, the default) or always.
    pub fn set_bar_mode(&mut self, mode: BarMode) {
        self.bar.set_mode(mode);
    }

    pub fn listing(&self) -> &ArchiveListing {
        &self.listing
    }

    pub fn labels(&self) -> &ArchiveLabels {
        &self.labels
    }

    /// Changes whenever the folder shown does: the collection's listing version.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// No member is highlighted: the preview shows the archive but hasn't been entered.
    pub fn passive(&self) -> bool {
        self.coll.selection.is_empty()
    }

    /// Entered: the first member is highlighted, with the cursor outline. Focus is the host's
    /// (module docs).
    pub fn enter(&mut self) {
        if self.listing.is_empty() {
            return;
        }
        self.coll.selection.select_only(&self.listing, 0);
        self.coll.keyboard_moved();
        self.coll.note_selection(&self.listing);
    }

    pub fn at_root(&self) -> bool {
        self.listing.depth() == 0
    }

    /// Opens the folder at `index`, its first member highlighted. False on a file.
    pub fn open(&mut self, index: usize, tx: &mut Tx) -> bool {
        if !self.listing.open(index) {
            return false;
        }
        self.show(Some(0), tx);
        true
    }

    /// Goes to the parent folder, with the folder left highlighted. False at the root.
    pub fn up(&mut self, tx: &mut Tx) -> bool {
        match self.listing.depth() {
            0 => false,
            depth => {
                self.go_to(depth - 1, tx);
                true
            }
        }
    }

    /// Goes to the folder `depth` levels below the root, on the way to the one shown (a
    /// crumb), with the folder it came through highlighted.
    pub fn go_to(&mut self, depth: usize, tx: &mut Tx) {
        if let Some(cursor) = self.listing.go_to(depth) {
            self.show(Some(cursor), tx);
        }
    }

    fn show(&mut self, cursor: Option<usize>, tx: &mut Tx) {
        let keyboard = self.coll.keyboard_mode();
        self.version += 1;
        self.coll
            .enter(&self.scope, &self.listing, None, Listing::Complete, tx);
        if let Some(i) = cursor.filter(|&i| i < self.listing.len()) {
            self.coll.selection.select_only(&self.listing, i);
            self.coll.reveal(&self.scope, &self.listing, i, tx);
        }
        // A folder opened from the keyboard keeps the cursor outline.
        if keyboard {
            self.coll.keyboard_moved();
        }
        self.coll.note_selection(&self.listing);
    }

    fn cursor_index(&self) -> Option<usize> {
        self.coll.selection.cursor_index(&self.listing)
    }

    /// What the collection's controllers asked for.
    fn heard(&mut self, intent: Intent, tx: &mut Tx) -> Heard {
        match intent {
            Intent::Activate { id, .. } => {
                if let Some(i) = self.listing.index_of(id) {
                    self.open(i, tx);
                }
                Heard::Done
            }
            Intent::Edge(Direction::Right) => {
                if let Some(i) = self.cursor_index() {
                    self.open(i, tx);
                }
                Heard::Done
            }
            Intent::Edge(Direction::Left) => {
                if self.up(tx) {
                    Heard::Done
                } else {
                    Heard::Leave
                }
            }
            _ => Heard::Done,
        }
    }
}

enum Heard {
    Done,
    /// Left at the root.
    Leave,
}

type GetFn<S> = fn(&S) -> &ArchiveState;
type GetMutFn<S> = fn(&mut S) -> &mut ArchiveState;
type LeaveFn<S> = Rc<dyn Fn(&mut S, &mut Tx)>;

/// The archive preview: `ArchiveView::new(get, get_mut, look) … .build(s)`. It fills its parent.
pub struct ArchiveView<S> {
    get: GetFn<S>,
    get_mut: GetMutFn<S>,
    look: fn(&S) -> &Look,
    letters: bool,
    on_leave: Option<LeaveFn<S>>,
}

impl<S: 'static> ArchiveView<S> {
    pub fn new(get: GetFn<S>, get_mut: GetMutFn<S>, look: fn(&S) -> &Look) -> Self {
        ArchiveView {
            get,
            get_mut,
            look,
            letters: false,
            on_leave: None,
        }
    }

    /// Whether `h j k l` act as the arrows: Strata's Type to search off.
    pub fn letters(mut self, on: bool) -> Self {
        self.letters = on;
        self
    }

    /// Left at the root: Strata returns focus to the listing, leaving the preview open.
    pub fn on_leave(mut self, f: impl Fn(&mut S, &mut Tx) + 'static) -> Self {
        self.on_leave = Some(Rc::new(f));
        self
    }

    /// The collection as the view builds it.
    fn collection(&self, s: &S) -> Collection<S> {
        let (get, get_mut, look) = (self.get, self.get_mut, self.look);
        let scope = get(s).scope.clone();
        let on_leave = self.on_leave.clone();
        let coll = Collection::new(
            scope,
            move |s: &S| {
                let a = get(s);
                (&a.listing as &dyn EntrySource, &a.coll)
            },
            move |s: &mut S| {
                let a = get_mut(s);
                (&a.listing as &dyn EntrySource, &mut a.coll)
            },
            self.look,
            move |s: &S| get(s).version,
        )
        .read_only()
        .settings(controllers::key_settings(self.letters))
        .settings(controllers::LeftPasses(
            get(s).at_root() && self.on_leave.is_none(),
        ))
        .on_intent(move |s: &mut S, intent, tx| {
            if let Heard::Leave = get_mut(s).heard(intent, tx)
                && let Some(f) = &on_leave
            {
                f(s, tx);
            }
        })
        .describe(Describe::new(
            move |s: &S, rcx| get(s).listing.size(rcx.index).unwrap_or(u64::MAX),
            move |s: &S, rcx| {
                let a = get(s);
                a.listing.size(rcx.index).map(a.labels.size)
            },
        ))
        .content(ContentSlot::new(
            move |s: &S, rcx| get(s).listing.size(rcx.index).unwrap_or(u64::MAX),
            move |s: &S, rcx, icon_el, label| {
                let look = rcx.look;
                let k = &look.kit;
                let a = get(s);
                // The size is the row's description (above): heard there, not as a stray
                // label inside the option.
                let trailing = match a.listing.size(rcx.index) {
                    Some(size) => text((a.labels.size)(size))
                        .font(k.ty.small)
                        .color(k.c.text_muted)
                        .a11y_hidden(),
                    None => icon(look.icon(IconName::ChevronRight), look.icon_size.sm)
                        .color(k.c.icon_muted)
                        .a11y_hidden(),
                };
                row()
                    .w_full()
                    .h_full()
                    .px(look.space.md)
                    .gap(look.space.lg)
                    .center_y()
                    .child(icon_el)
                    .child(label.grow(1.0))
                    .child(trailing)
            },
        ))
        .empty(move |s: &S| {
            let a = get(s);
            let k = &look(s).kit;
            Some(
                col()
                    .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                    .center()
                    .child(
                        text(a.labels.empty.clone())
                            .key(format!("{}-empty", a.scope))
                            .font(k.ty.ui)
                            .color(k.c.text_muted),
                    ),
            )
        });
        let a = get(s);
        let (bar, feed, edge_bar) = (a.bar.clone(), a.feed.clone(), a.bar.clone());
        let (bar_key, target) = (format!("{}-bar", a.scope), viewport_path(&a.scope));
        coll.on_scroll(move |p| {
            feed.set(p);
            bar.follow(p);
        })
        .edge(move |s: &S| Some(edge_bar.build::<S>(&bar_key, &target, BarLook::app(look(s)))))
        .name(a.labels.list.clone())
    }

    /// Runs the collection's controller timeouts (C1e's `tick`); call it from the host's poll.
    pub fn tick(self, s: &mut S, tx: &mut Tx) -> bool {
        self.collection(s).tick(s, tx)
    }

    pub fn build(self, s: &S) -> El {
        let a = (self.get)(s);
        let look = (self.look)(s);
        let k = &look.kit;
        let scope = a.scope.to_string();
        let get_mut = self.get_mut;
        let depth = a.listing.depth();

        let mut crumbs = row()
            .key(format!("{scope}-crumbs"))
            .gap(look.space.xs)
            .center_y();
        if depth > 0 {
            crumbs = crumbs.child(
                IconButton::new(look.icon(IconName::Back))
                    .variant(IconButtonVariant::Ghost)
                    .label(a.labels.up.clone())
                    .help(a.labels.up.clone())
                    .on_press_tx(move |s: &mut S, tx: &mut Tx| {
                        get_mut(s).up(tx);
                    })
                    .build(k)
                    // The tooltip repeats the name: no description, so it isn't read twice.
                    .description("")
                    .key(format!("{scope}-up")),
            );
        }
        let names = std::iter::once(a.labels.root.as_str()).chain(a.listing.crumbs());
        for (level, name) in names.enumerate() {
            if level == depth {
                crumbs = crumbs.child(
                    text(name.to_owned())
                        .key(format!("{scope}-crumb-current"))
                        .font(k.ty.ui_medium)
                        .color(k.c.text_base)
                        .px(look.space.sm),
                );
            } else {
                crumbs = crumbs.child(
                    Button::new(name.to_owned())
                        .variant(ButtonVariant::Ghost)
                        .control_size(ControlSize::Regular)
                        .on_press_tx(move |s: &mut S, tx: &mut Tx| get_mut(s).go_to(level, tx))
                        .build(k)
                        .key(format!("{scope}-crumb-{level}")),
                );
            }
        }
        let (files, folders) = a.listing.counts();
        let count = text((a.labels.counts)(files, folders))
            .key(format!("{scope}-count"))
            .font(k.ty.small)
            .color(k.c.text_muted)
            .truncate()
            .px(look.space.md);
        let list = self.collection(s).build(s);
        col()
            .key(format!("{scope}-preview"))
            .w_full()
            .h_full()
            .min_h(0.0)
            .gap(look.space.xs)
            .padding(look.space.sm)
            .child(row().w_full().scroll_x().child(crumbs))
            .child(count)
            .child(
                col()
                    .w_full()
                    .grow(1.0)
                    .min_h(0.0)
                    .child((self.get)(s).bar.area(list)),
            )
    }
}
