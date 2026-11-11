//! The gallery page for C23 Motion effects: its scenes, demo state, view and key and poll hooks.
//! Scene names start with `c23-` and end in `-light` or `-dark`.
//!
//! Work package: `docs/work/phase-1-leaves/C23-motion-effects.md`.
//!
//! The page draws C8's real sidebar beside a real C1e list, with the motion layer over both.
//! Its buttons play Strata's part with fake files: each operation "finishes before its
//! progress card would show", so it plays its effect, and the page removes or shows rows at
//! the effects' events as Strata will. Move to Trash flies the selection into the sidebar's
//! Trash, Delete permanently dissolves it, Restore flies what was trashed back onto its rows,
//! and Restore here lifts the selection off its rows as a restore in the Trash's own view
//! does. Buttons start the effect at the next poll, where scenes time theirs.

use rune::{Cx, El, KeyEvent, Tx, col, el, icon, row, text};
use rune_kit::Button;
use strata_components::collection::element::{
    Collection, CollectionState, ICON, RowMarks, part_path, row_path, viewport_path,
};
use strata_components::collection::selection::{CursorGone, SyncRule};
use strata_components::contract::{EntryId, EntryKind, ListEntry, ListSource};
use strata_components::motion::{
    Dissolve, EffectId, Effects, Event, Flight, MotionLayer, dress_target,
};
use strata_components::sidebar::{
    DEFAULT_KEY, ItemId, Place, PlacesSidebar, SidebarData, SidebarState, TRASH_TARGET,
};
use strata_kit::icons::IconName;
use strata_kit::look::Look;
use strata_kit::slots::collection as tokens;

use crate::{Gallery, Scene};

/// The list's key.
pub const SCOPE: &str = "c23-files";

/// What the page does next, at the next poll.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Trash,
    Delete,
    Restore,
    Release,
}

/// C23's demo state, held in `Gallery::c23`.
pub struct Demo {
    pub fx: Effects,
    pub sidebar: SidebarState,
    pub places: Vec<Place>,
    pub listing: ListSource,
    pub coll: CollectionState,
    pub version: u64,
    /// What the Trash holds, to restore.
    pub trashed: Vec<ListEntry>,
    /// Each playing effect the page owes rows to, and what it does with them: effects
    /// overlap (a Move to Trash while a restore's icons are in the air). A restore's rows are
    /// owed until Landed, the rest until Done.
    pub owed: Vec<(EffectId, Op, Vec<EntryId>)>,
    /// An operation asked for, and the clock time it may start (scenes time theirs).
    pub ask: Option<(Op, f64)>,
    pub clock: f64,
    /// What the last event was, shown in `c23-log`.
    pub log: String,
}

impl Default for Demo {
    fn default() -> Self {
        let look = Look::fallback(false);
        Self {
            fx: Effects::new(),
            sidebar: SidebarState::new(&look),
            places: vec![
                Place::folder("home", "Home", IconName::Home),
                Place::folder("documents", "Documents", IconName::Documents),
                Place::folder("downloads", "Downloads", IconName::Downloads),
                Place::folder("pictures", "Pictures", IconName::Pictures),
                Place::trash("Trash", false),
                Place::other("network", "Network", IconName::Network),
            ],
            listing: ListSource::new(files()),
            coll: CollectionState::new(),
            version: 1,
            trashed: Vec::new(),
            owed: Vec::new(),
            ask: None,
            clock: 0.0,
            log: String::new(),
        }
    }
}

const FILES: [(&str, &str); 14] = [
    ("budget-2026.ods", "file"),
    ("cover-letter.odt", "file_text"),
    ("holiday-01.jpg", "file_image"),
    ("holiday-02.jpg", "file_image"),
    ("holiday-03.jpg", "file_image"),
    ("invoice-0412.pdf", "file_text"),
    ("meeting-notes.md", "file_text"),
    ("notes.txt", "file_text"),
    ("plan.odt", "file_text"),
    ("receipt.png", "file_image"),
    ("report-q3.pdf", "file_text"),
    ("slides.odp", "file"),
    ("todo.txt", "file_text"),
    ("vacation.mp4", "file"),
];

fn files() -> Vec<ListEntry> {
    FILES
        .iter()
        .enumerate()
        .map(|(i, (name, icon))| {
            ListEntry::new(EntryId::new(i as u64 + 1), *name, EntryKind::File, *icon)
        })
        .collect()
}

fn entry_icon(look: &Look, key: &str) -> &'static rune::Icon {
    look.icon_named(key)
        .unwrap_or_else(|| look.icon(IconName::File))
}

impl Demo {
    /// Whether the row `id` is listed but hidden: a restore's until its icons land, a
    /// dissolve's until it ends, and while any effect still holds it so.
    pub fn vanished(&self, id: EntryId) -> bool {
        self.owed
            .iter()
            .any(|(_, op, ids)| matches!(op, Op::Restore | Op::Delete) && ids.contains(&id))
    }

    fn selected(&self) -> Vec<EntryId> {
        self.coll.selection.selected_ids(&self.listing)
    }

    fn entry(&self, id: EntryId) -> Option<&ListEntry> {
        self.listing.entries().iter().find(|e| e.id == id)
    }

    /// Takes `ids` out of the listing, keeping the selection and cursor in step.
    fn remove(&mut self, ids: &[EntryId], tx: &mut Tx) {
        self.coll
            .before_removal(&self.listing, |id| ids.contains(&id));
        self.listing.retain(|e| !ids.contains(&e.id));
        self.version += 1;
        self.coll.sync(
            SCOPE,
            &self.listing,
            SyncRule::complete(CursorGone::Neighbor),
            tx,
        );
    }

    /// Puts the trashed entries back, in name order.
    fn put_back(&mut self, tx: &mut Tx) -> Vec<EntryId> {
        let back: Vec<ListEntry> = std::mem::take(&mut self.trashed);
        let ids = back.iter().map(|e| e.id).collect();
        let mut entries = self.listing.entries().to_vec();
        entries.extend(back);
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        self.listing.replace(entries);
        self.version += 1;
        self.coll.sync(
            SCOPE,
            &self.listing,
            SyncRule::complete(CursorGone::Neighbor),
            tx,
        );
        ids
    }

    /// Runs operation `op` on the selection, as if it finished before its card showed.
    fn run(&mut self, op: Op, look: &Look, tx: &mut Tx) {
        let ids = match op {
            Op::Restore => Vec::new(),
            _ => self.selected(),
        };
        if op != Op::Restore && ids.is_empty() || op == Op::Restore && self.trashed.is_empty() {
            return;
        }
        let legs = |d: &Demo, ids: &[EntryId], part: bool| {
            ids.iter()
                .filter_map(|&id| {
                    let e = d.entry(id)?;
                    let key = if part {
                        part_path(SCOPE, id, ICON)
                    } else {
                        row_path(SCOPE, id)
                    };
                    Some((key, entry_icon(look, &e.icon)))
                })
                .collect::<Vec<_>>()
        };
        let target = trash_target();
        let started = match op {
            Op::Trash => {
                let f = Flight::into_target(&target, legs(self, &ids, true))
                    .target_in(sidebar_scroll())
                    .rows_in(viewport_path(SCOPE));
                self.fx.fly(f, look, tx)
            }
            Op::Release => {
                let f = Flight::release(&target, legs(self, &ids, true))
                    .target_in(sidebar_scroll())
                    .rows_in(viewport_path(SCOPE));
                self.fx.fly(f, look, tx)
            }
            Op::Delete => {
                // A row already hidden (a restore's icon still in the air) has nothing to
                // break up: it just stays hidden until it goes.
                let rows = ids
                    .iter()
                    .filter(|&&id| !self.vanished(id))
                    .map(|&id| (row_path(SCOPE, id), id.get()));
                self.fx
                    .dissolve(Dissolve::rows(rows).within(viewport_path(SCOPE)), look, tx)
            }
            Op::Restore => {
                let back = self.put_back(tx);
                // The restored rows are listed now and hidden until the icons land.
                let f = Flight::out_of_target(&target, legs(self, &back, false))
                    .target_in(sidebar_scroll())
                    .rows_in(viewport_path(SCOPE));
                let started = self.fx.fly(f, look, tx);
                self.log = "Restored".into();
                self.owed.extend(started.map(|id| (id, op, back)));
                return;
            }
        };
        match started {
            Some(id) => self.owed.push((id, op, ids)),
            // Nothing plays (animations off): the end state at once.
            None => self.finish(op, &ids, tx),
        }
    }

    fn finish(&mut self, op: Op, ids: &[EntryId], tx: &mut Tx) {
        match op {
            Op::Trash => {
                self.trashed.extend(
                    self.listing
                        .entries()
                        .iter()
                        .filter(|e| ids.contains(&e.id))
                        .cloned(),
                );
                self.remove(ids, tx);
                self.log = format!("Moved {} to Trash", ids.len());
            }
            Op::Release => {
                self.remove(ids, tx);
                self.log = format!("Restored {}", ids.len());
            }
            Op::Delete => {
                self.remove(ids, tx);
                self.log = format!("Deleted {}", ids.len());
            }
            // Its rows show once the page owes them nothing.
            Op::Restore => {}
        }
        self.places[4] = Place::trash("Trash", !self.trashed.is_empty());
    }

    /// One effect event: the rows the page owes its effect.
    fn heard(&mut self, event: Event, tx: &mut Tx) {
        let (Event::Launched(id) | Event::Landed(id) | Event::Done(id)) = event;
        let Some(i) = self.owed.iter().position(|o| o.0 == id) else {
            return;
        };
        let (_, op, ids) = self.owed[i].clone();
        match (event, op) {
            // Not an instant commit: it carries the flight's swap (motion module docs).
            (Event::Launched(_), Op::Trash | Op::Release) => self.finish(op, &ids, tx),
            (Event::Landed(_), Op::Restore) => self.finish(op, &ids, tx),
            (Event::Done(_), Op::Delete) => {
                // The survivors land in their new places at once.
                tx.instant();
                self.finish(op, &ids, tx);
            }
            _ => {}
        }
        if matches!(
            (event, op),
            (Event::Done(_), _) | (Event::Landed(_), Op::Restore)
        ) {
            self.owed.remove(i);
        }
    }
}

fn trash_target() -> String {
    TRASH_TARGET.to_owned()
}

fn sidebar_scroll() -> String {
    format!("{DEFAULT_KEY}-scroll")
}

fn sidebar_state(g: &mut Gallery) -> &mut SidebarState {
    &mut g.c23.sidebar
}

fn marks(g: &Gallery, id: EntryId) -> RowMarks {
    let mut m = RowMarks::default();
    m.vanished = g.c23.vanished(id);
    m
}

fn collection() -> Collection<Gallery> {
    Collection::new(
        SCOPE,
        |g: &Gallery| (&g.c23.listing, &g.c23.coll),
        |g: &mut Gallery| (&g.c23.listing, &mut g.c23.coll),
        |g: &Gallery| g.look(),
        |g: &Gallery| g.c23.version,
    )
    .name("Files")
    .marks(marks)
}

/// How a row looked before it vanished: C1e's default row, selected (a dissolve's rows are
/// the selection).
fn ghost(g: &Gallery, token: u64, look: &Look) -> El {
    let kit = &look.kit;
    let Some(e) = g.c23.entry(EntryId::new(token)) else {
        return el();
    };
    row().w_full().h_full().px(tokens::row_inset(look)).child(
        row()
            .w_full()
            .h_full()
            .radius(tokens::row_radius(look))
            .px(look.space.xs)
            .gap(look.space.sm)
            .center_y()
            .fill(rune_kit::TableStyle::from_kit(kit).selected)
            .child(icon(entry_icon(look, &e.icon), look.icon_size.md).color(kit.c.icon_base))
            .child(
                text(e.name.to_owned())
                    .font(kit.ty.ui_compact)
                    .color(kit.c.text_base)
                    .truncate(),
            ),
    )
}

fn select(g: &mut Gallery, indices: &[usize]) {
    let d = &mut g.c23;
    for (n, &i) in indices.iter().enumerate() {
        if n == 0 {
            d.coll.selection.select_only(&d.listing, i);
        } else {
            d.coll.selection.toggle(&d.listing, i);
        }
    }
    d.coll.note_selection(&d.listing);
}

fn scene(g: &mut Gallery, dark: bool) {
    g.open("c23", dark);
    g.c23 = Demo {
        sidebar: SidebarState::new(&g.look),
        ..Demo::default()
    };
}

/// Three rows selected, flying into the Trash from about 0.9 s: mid-flight at the 1 s capture.
fn flight(g: &mut Gallery, dark: bool) {
    scene(g, dark);
    select(g, &[2, 3, 6]);
    g.c23.ask = Some((Op::Trash, 0.88));
}

/// Three rows dissolving from 0.72 s: mid-dissolve at the capture.
fn dissolving(g: &mut Gallery, dark: bool) {
    scene(g, dark);
    select(g, &[3, 4, 5]);
    g.c23.ask = Some((Op::Delete, 0.72));
}

/// Animations off: the trashed rows are simply gone, with nothing over the page.
fn off(g: &mut Gallery, dark: bool) {
    scene(g, dark);
    g.look.animations = false;
    select(g, &[2, 3, 6]);
    g.c23.ask = Some((Op::Trash, 0.5));
}

/// Landed at about 0.93 s: the Trash mid-impact at the capture.
fn pulse(g: &mut Gallery, dark: bool) {
    scene(g, dark);
    select(g, &[1, 8]);
    g.c23.ask = Some((Op::Trash, 0.55));
}

/// Two files in the Trash, restored at 0.85 s.
fn restore(g: &mut Gallery, dark: bool) {
    scene(g, dark);
    let d = &mut g.c23;
    let gone: Vec<EntryId> = [3, 7].map(EntryId::new).to_vec();
    d.trashed = d
        .listing
        .entries()
        .iter()
        .filter(|e| gone.contains(&e.id))
        .cloned()
        .collect();
    d.listing.retain(|e| !gone.contains(&e.id));
    d.places[4] = Place::trash("Trash", true);
    d.ask = Some((Op::Restore, 0.85));
}

/// Two rows lifting off at 0.88 s.
fn release(g: &mut Gallery, dark: bool) {
    scene(g, dark);
    select(g, &[4, 9]);
    g.c23.ask = Some((Op::Release, 0.88));
}

/// Golden scenes: each gets an accessibility and a PNG golden. Each calls
/// `g.open("c23", dark)`, then fills in `g.c23`.
pub const SCENES: &[Scene] = &[
    ("c23-flight-light", |g| flight(g, false)),
    ("c23-flight-dark", |g| flight(g, true)),
    ("c23-dissolve-light", |g| dissolving(g, false)),
    ("c23-dissolve-dark", |g| dissolving(g, true)),
    ("c23-off-light", |g| off(g, false)),
    ("c23-off-dark", |g| off(g, true)),
];

/// Scenes for review with `--scene` only, kept out of the goldens.
pub const REVIEW: &[Scene] = &[
    ("c23-rest-light", |g| scene(g, false)),
    ("c23-rest-dark", |g| scene(g, true)),
    ("c23-pulse-light", |g| pulse(g, false)),
    ("c23-pulse-dark", |g| pulse(g, true)),
    ("c23-restore-light", |g| restore(g, false)),
    ("c23-restore-dark", |g| restore(g, true)),
    ("c23-release-light", |g| release(g, false)),
    ("c23-release-dark", |g| release(g, true)),
];

pub fn view(g: &Gallery, cx: &mut Cx<Gallery>) -> El {
    let look = g.look();
    cx.reduced_motion(!look.animations);
    let k = &look.kit;
    let d = &g.c23;
    let button = |key: &str, label: &str, op: Op| {
        Button::new(label)
            .on_press(move |g: &mut Gallery| g.c23.ask = Some((op, 0.0)))
            .build(k)
            .key(format!("c23-{key}"))
    };
    let animations = Button::new(if look.animations {
        "Animations on"
    } else {
        "Animations off"
    })
    .on_press(|g: &mut Gallery| g.look.animations = !g.look.animations)
    .build(k)
    .key("c23-animations");
    let header = row()
        .key("c23-header")
        .w_full()
        .h(look.frame.bar + look.space.md)
        .px(look.space.md)
        .gap(look.space.sm)
        .center_y()
        .child(button("trash", "Move to Trash", Op::Trash))
        .child(button("delete", "Delete permanently", Op::Delete))
        .child(button("restore", "Restore", Op::Restore))
        .child(button("release", "Restore here", Op::Release))
        .child(animations)
        .child(
            text(if d.log.is_empty() { "—" } else { &d.log })
                .key("c23-log")
                .font(k.ty.small)
                .color(k.c.text_muted),
        );
    let data = SidebarData {
        places: &d.places,
        pins: &[],
        devices: &[],
        current: Some(&ItemId::Place("home".into())),
    };
    let sidebar = PlacesSidebar::new(&d.sidebar, sidebar_state, data)
        .trash_icon(|el| dress_target(&d.fx, TRASH_TARGET, el))
        .build(look);
    let files = col()
        .key("c23-pane")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .fill(k.c.layer_01)
        .py(look.content.pad)
        .child(collection().build(g));
    let layer = MotionLayer::new(|g: &Gallery| &g.c23.fx)
        .ghost(ghost)
        .build(g, look, cx);
    col()
        .key("c23-page")
        .w_full()
        .h_full()
        .child(header)
        .child(
            row()
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .child(sidebar)
                .child(files),
        )
        .child(layer)
}

/// Keys no focused element took while this page is open: shortcuts, and keys with nothing
/// focused. True consumes the key; false lets the gallery's Escape go back to the index.
pub fn on_key(_g: &mut Gallery, _key: &KeyEvent, _tx: &mut Tx) -> bool {
    false
}

/// Runs at the start of every frame while this page is open, with the clock in seconds: the
/// effects' tick and their events, then an operation asked for.
pub fn poll(g: &mut Gallery, t: f64, tx: &mut Tx) -> bool {
    g.c23.clock = t;
    let mut changed = false;
    for event in g.c23.fx.tick(t, tx) {
        g.c23.heard(event, tx);
        changed = true;
    }
    if let Some((op, at)) = g.c23.ask
        && t + 1e-9 >= at
    {
        g.c23.ask = None;
        let look = g.look.clone();
        g.c23.run(op, &look, tx);
        changed = true;
    } else if let Some((_, at)) = g.c23.ask {
        tx.wake_after(std::time::Duration::from_secs_f64(at - t));
    }
    changed | collection().tick(g, tx)
}
