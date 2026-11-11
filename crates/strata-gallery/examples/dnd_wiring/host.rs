//! One window of the demo: Strata's chrome and views on a real folder, every drop target and
//! drag source wired to P1 through the adapter, and Strata's policy stood in for (the
//! transfers are logged, and done with `--apply`).
//!
//! Tabs: 1 Columns, 2 List (C4), 3 Icons (C3), each on `--root`; a drop on the strip or New
//! tab opens a List tab per dragged folder. The sidebar (C8) has the root, its top folders,
//! the Trash (which takes files, logging them as trashed), one pin and the `usb` device
//! (another volume). The location bar (C6) names the active pane's folder. List and Icons
//! peek folders (C22).

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::time::Duration;

use rune::{Cx, DragOutcome, El, Mods, Tx, col, row, text};
use serde_json::{Value, json};
use strata_components::collection::element::{
    Collection, CollectionState, DropOver, Intent, RowMarks, entry_path, row_path, viewport_path,
};
use strata_components::collection::selection::{CursorGone, Listing, SyncRule};
use strata_components::columns::{
    ColumnSources, ColumnsIntent, ColumnsParts, ColumnsPartsMut, ColumnsState, ColumnsView, Focus,
    Opening,
};
use strata_components::contract::dnd::{
    ActionInput, CrossDevice, DragPayload, DropAction, DropActions, DropOffer, DropZone, Dropped,
    URI_LIST, choose_action,
};
use strata_components::contract::{EntryId, EntrySource, ListSource, Point};
use strata_components::folder_peek::{
    Change, FolderPeek, More, PeekContents, PeekState, PeekStatus,
};
use strata_components::icons_grid::{IconsGrid, IconsGridState, IconsParts};
use strata_components::list_table::{
    Cell as ListCell, Column, ColumnLayout, ColumnWidths, ListParts, ListTable, ListTableState,
    groups_from_runs,
};
use strata_components::location_bar::{
    Crumb, LocationBar, LocationBarState, LocationIntent, Trail, crumb_key,
};
use strata_components::scroll::Autoscroll;
use strata_components::sidebar::{
    DEFAULT_KEY as SIDEBAR, Device, DeviceGroup, DropSite as SideSite, ItemId, Pin, PinVerdict,
    Place, PlacesSidebar, SidebarData, SidebarIntent, SidebarState, Unchecked,
};
use strata_components::tabs::{
    BrowserTabs, BrowserTabsState, Tab, TabId, TabsData, TabsIntent, new_tab_key, tab_key,
};
use strata_kit::icons::IconName;
use strata_kit::look::Look;
use strata_platform::dnd::{DragIcon, Status};

use crate::adapter::{self, Wiring};
use crate::fsys::{self, Ids};
use crate::log;

/// The strip's key, and the location bar's.
const STRIP: &str = "tabs";
const LOC: &str = "loc";
/// The window root's key: the sentinel target.
pub const ROOT: &str = "root";
/// The spring's delay for folder and sidebar rows (Strata's 750 ms).
const SPRING: f64 = 0.75;
/// The types the sentinel takes: what any drag this demo sees offers.
const SENTINEL_TYPES: &[&str] = &[
    "text/uri-list",
    "text/plain;charset=utf-8",
    "text/plain",
    "text/x-moz-url",
    "text/html",
    "image/png",
    adapter::GNOME_COPIED_FILES,
];
/// What a tab takes from other apps besides files (C7's content types).
const CONTENT: &[&str] = &["image/png", "text/plain;charset=utf-8", "text/plain"];

/// A column's listing.
pub struct Listed {
    folder: PathBuf,
    name: String,
    source: ListSource,
    version: u64,
    /// The entry in the column before whose folder this is.
    from: Option<EntryId>,
}

/// The open columns, root first.
#[derive(Default)]
pub struct Listings(Vec<Listed>);

impl ColumnSources for Listings {
    fn len(&self) -> usize {
        self.0.len()
    }
    fn source(&self, depth: usize) -> &dyn EntrySource {
        &self.0[depth].source
    }
    fn version(&self, depth: usize) -> u64 {
        self.0[depth].version
    }
    fn title(&self, depth: usize) -> &str {
        &self.0[depth].name
    }
    fn open_child(&self, depth: usize) -> Option<EntryId> {
        self.0.get(depth + 1).and_then(|c| c.from)
    }
}

/// A List or Icons pane.
pub struct Flat {
    key: String,
    folder: PathBuf,
    listing: ListSource,
    coll: CollectionState,
    version: u64,
    table: ListTableState,
    grid: IconsGridState,
}

pub enum Pane {
    Columns {
        key: String,
        listings: Listings,
        state: ColumnsState,
    },
    List(Flat),
    Icons(Flat),
}

pub struct TabPane {
    id: TabId,
    pane: Pane,
}

/// The drag this window started.
struct Out {
    tab: TabId,
    ids: Vec<EntryId>,
}

/// A folder or sidebar row a drag rests on, and since when (the host's spring).
#[derive(Clone, Debug, PartialEq)]
enum SpringAt {
    Row {
        tab: TabId,
        depth: usize,
        id: EntryId,
    },
    Place(ItemId),
}

/// What a peek shows.
#[derive(Default)]
struct Peeked {
    name: String,
    status: PeekStatus,
    rows: ListSource,
    more: Option<More>,
}

pub struct Win {
    pub name: String,
    root: PathBuf,
    apply: bool,
    pub look: Look,
    scale: Cell<f32>,
    pub wiring: Wiring<Win>,
    ids: Ids,
    clock: f64,
    sidebar: SidebarState,
    places: Vec<Place>,
    pins: Vec<Pin>,
    devices: Vec<DeviceGroup>,
    tabs: Vec<TabPane>,
    tabs_state: BrowserTabsState,
    active: TabId,
    next_tab: u64,
    bar: LocationBarState,
    trail: Trail,
    peek: PeekState,
    peeked: Peeked,
    peek_ids: Ids,
    spring: Option<(SpringAt, f64, bool)>,
    out: Option<Out>,
    /// Whether the drag over Columns' strip gaps is one the strip refuses (no highlight), and
    /// if so whether it offers files (only those autoscroll).
    strip_refused: Option<bool>,
    /// Whether the drag with this serial opens tabs (all its items are folders): asked on
    /// every motion over the strip, so the folders are checked once per drag.
    opens: Cell<Option<(u64, bool)>>,
    covered: bool,
    layout: ColumnLayout,
    widths: ColumnWidths,
    /// What the next view reports where it drew (the `locate` command): a name and what it
    /// names, resolved to a key path at each view (a grid's paths change as it reflows).
    pub locate: RefCell<Vec<(String, Value)>>,
    pub status: Option<Status>,
}

fn name_of(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

impl Win {
    pub fn new(name: String, root: PathBuf, apply: bool, wiring: Wiring<Win>) -> Self {
        let look = Look::fallback(false);
        let mut ids = Ids::default();
        let mut places = vec![Place::folder(
            path_id(&root),
            name_of(&root),
            IconName::Home,
        )];
        for e in fsys::list(&mut ids, &root).entries() {
            let p = root.join(&e.name);
            if e.kind.is_folder_like() && e.name != "usb" && e.name != "pinned" {
                places.push(Place::folder(path_id(&p), e.name.clone(), IconName::Folder));
            }
        }
        places.push(Place::trash("Trash", false));
        let pins = vec![Pin::new(path_id(&root.join("pinned")), "pinned")];
        let usb = root.join("usb");
        let devices = if usb.is_dir() {
            vec![DeviceGroup::new(vec![Device::new(
                path_id(&usb),
                "USB",
                IconName::Usb,
                usb.display().to_string(),
            )])]
        } else {
            Vec::new()
        };
        // `Unchecked::Accept`, though P1 reads a drag's URIs while it hovers: under `Wait` a gap
        // refuses an unchecked drag, and C8 notes which drag is over its gaps only from a gap's
        // accepting hover, so the check would never start (a C8 finding, reported). Here the
        // gap's first hover starts it, the read URIs settle it, and a refused drag then stops
        // at the gap (its `on_reject`).
        let mut sidebar = SidebarState::new(&look);
        sidebar.pin_check.unchecked = Unchecked::Accept;
        let mut peek = PeekState::new();
        peek.set_enabled(true);
        let mut w = Win {
            name,
            root: root.clone(),
            apply,
            look,
            scale: Cell::new(1.0),
            wiring,
            ids,
            clock: 0.0,
            sidebar,
            places,
            pins,
            devices,
            tabs: Vec::new(),
            tabs_state: BrowserTabsState::new(STRIP),
            active: TabId(1),
            next_tab: 1,
            bar: LocationBarState::new(),
            trail: Trail::new(Vec::new(), ""),
            peek,
            peeked: Peeked::default(),
            peek_ids: Ids::default(),
            spring: None,
            out: None,
            strip_refused: None,
            opens: Cell::new(None),
            covered: false,
            layout: ColumnLayout::new(columns().iter().map(|c| c.id().clone())),
            widths: ColumnWidths::new(),
            locate: RefCell::default(),
            status: None,
        };
        let mut tx = Tx::default();
        w.add_tab(Kind::Columns, &root, None, &mut tx);
        w.add_tab(Kind::List, &root, None, &mut tx);
        w.add_tab(Kind::Icons, &root, None, &mut tx);
        w.active = TabId(1);
        w.retrail();
        w
    }

    fn log(&self, ev: &str, fields: Value) {
        log(&self.name, ev, fields);
    }

    // ---- Panes ----

    fn add_tab(&mut self, kind: Kind, folder: &Path, at: Option<usize>, tx: &mut Tx) -> TabId {
        let id = TabId(self.next_tab);
        self.next_tab += 1;
        let pane = match kind {
            Kind::Columns => {
                let key = format!("t{}-cols", id.0);
                let mut state = ColumnsState::new(key.clone(), Autoscroll::new());
                let source = fsys::list(&mut self.ids, folder);
                let listings = Listings(vec![Listed {
                    folder: folder.to_owned(),
                    name: name_of(folder),
                    source,
                    version: 1,
                    from: None,
                }]);
                state.push(
                    Opening::new(&listings.0[0].source, Listing::Complete),
                    Focus::Column,
                    &self.look,
                    tx,
                );
                Pane::Columns {
                    key,
                    listings,
                    state,
                }
            }
            Kind::List | Kind::Icons => {
                let key = format!(
                    "t{}-{}",
                    id.0,
                    if kind == Kind::List { "list" } else { "icons" }
                );
                let listing = fsys::list(&mut self.ids, folder);
                let mut coll = CollectionState::new();
                install(&mut coll);
                coll.enter(&key, &listing, None, Listing::Complete, tx);
                let flat = Flat {
                    key,
                    folder: folder.to_owned(),
                    listing,
                    coll,
                    version: 1,
                    table: ListTableState::new(),
                    grid: IconsGridState::new(),
                };
                if kind == Kind::List {
                    Pane::List(flat)
                } else {
                    Pane::Icons(flat)
                }
            }
        };
        let at = at.unwrap_or(self.tabs.len()).min(self.tabs.len());
        self.tabs.insert(at, TabPane { id, pane });
        id
    }

    fn tab(&self, id: TabId) -> &TabPane {
        self.tabs.iter().find(|t| t.id == id).expect("an open tab")
    }

    fn tab_mut(&mut self, id: TabId) -> &mut TabPane {
        self.tabs
            .iter_mut()
            .find(|t| t.id == id)
            .expect("an open tab")
    }

    fn flat(&self, id: TabId) -> &Flat {
        match &self.tab(id).pane {
            Pane::List(f) | Pane::Icons(f) => f,
            Pane::Columns { .. } => panic!("not a flat pane"),
        }
    }

    fn flat_mut(&mut self, id: TabId) -> &mut Flat {
        match &mut self.tab_mut(id).pane {
            Pane::List(f) | Pane::Icons(f) => f,
            Pane::Columns { .. } => panic!("not a flat pane"),
        }
    }

    /// The folder pane `id` shows: Columns' deepest column.
    fn folder_of(&self, id: TabId) -> PathBuf {
        match &self.tab(id).pane {
            Pane::List(f) | Pane::Icons(f) => f.folder.clone(),
            Pane::Columns { listings, .. } => listings
                .0
                .last()
                .map(|l| l.folder.clone())
                .unwrap_or_else(|| self.root.clone()),
        }
    }

    fn retrail(&mut self) {
        let folder = self.folder_of(self.active);
        let mut crumbs = Vec::new();
        let mut at = self.root.clone();
        // A crumb's id is no key (crumbs are keyed by index): the folder's path itself.
        crumbs.push(Crumb::new(at.display().to_string(), name_of(&at)).icon(IconName::Home));
        if let Ok(rest) = folder.strip_prefix(&self.root) {
            for seg in rest.components() {
                at = at.join(seg);
                crumbs.push(Crumb::new(at.display().to_string(), name_of(&at)));
            }
        }
        self.trail = Trail::new(crumbs, folder.display().to_string());
    }

    /// Shows `folder` in tab `id`: List and Icons go there, Columns opens it as its last
    /// column when it is a child of one, else starts over from it.
    fn navigate(&mut self, id: TabId, folder: &Path, tx: &mut Tx) {
        if let Pane::Columns { listings, .. } = &self.tab(id).pane {
            let keep = listings
                .0
                .iter()
                .position(|l| folder.parent() == Some(&l.folder))
                .map_or(0, |depth| depth + 1);
            if self.holds_source(id, keep, folder) {
                return;
            }
        }
        let look = self.look.clone();
        let listing = fsys::list(&mut self.ids, folder);
        let from = self.ids.of(folder);
        match &mut self.tab_mut(id).pane {
            Pane::List(f) | Pane::Icons(f) => {
                f.folder = folder.to_owned();
                f.listing = listing;
                f.version += 1;
                let (key, l) = (f.key.clone(), &f.listing);
                f.coll.enter(&key, l, None, Listing::Complete, tx);
            }
            Pane::Columns {
                listings, state, ..
            } => {
                let parent = listings
                    .0
                    .iter()
                    .position(|l| folder.parent() == Some(&l.folder));
                match parent {
                    Some(depth) => {
                        open_column(listings, state, depth, folder, from, listing, &look, tx);
                    }
                    None => {
                        listings.0.truncate(1);
                        listings.0[0] = Listed {
                            folder: folder.to_owned(),
                            name: name_of(folder),
                            source: listing,
                            version: listings.0[0].version + 1,
                            from: None,
                        };
                        state.truncate(1, &look, tx);
                        state.sync(
                            0,
                            &listings.0[0].source,
                            SyncRule::complete(CursorGone::Neighbor),
                            tx,
                        );
                    }
                }
            }
        }
        if self.peek.close() {
            self.peeked = Peeked::default();
        }
        if id == self.active {
            self.retrail();
            let _ = self.bar.navigated();
        }
        self.log("navigate", json!({ "tab": id.0, "folder": folder }));
    }

    /// Lists every pane showing `folder` again (after a transfer).
    fn relist(&mut self, folders: &[PathBuf], tx: &mut Tx) {
        let ids = &mut self.ids;
        for t in &mut self.tabs {
            match &mut t.pane {
                Pane::List(f) | Pane::Icons(f) => {
                    if folders.contains(&f.folder) {
                        f.listing = fsys::list(ids, &f.folder);
                        f.version += 1;
                        let (key, l) = (f.key.clone(), &f.listing);
                        f.coll
                            .sync(&key, l, SyncRule::complete(CursorGone::Neighbor), tx);
                    }
                }
                Pane::Columns {
                    listings, state, ..
                } => {
                    for depth in 0..listings.0.len() {
                        if folders.contains(&listings.0[depth].folder) {
                            let l = &mut listings.0[depth];
                            l.source = fsys::list(ids, &l.folder);
                            l.version += 1;
                            state.sync(
                                depth,
                                &listings.0[depth].source,
                                SyncRule::complete(CursorGone::Neighbor),
                                tx,
                            );
                        }
                    }
                }
            }
        }
    }

    fn select_tab(&mut self, id: TabId, via: &str, tx: &mut Tx) {
        if self.tabs.iter().any(|t| t.id == id) && self.active != id {
            self.active = id;
            if self.peek.close() {
                self.peeked = Peeked::default();
            }
            self.retrail();
            let _ = self.bar.navigated();
            tx.wake_after(Duration::ZERO);
            self.log("tab-select", json!({ "tab": id.0, "via": via }));
        }
    }

    // ---- Policy: actions and transfers ----

    /// The action a drop of `offer` into `dest` takes, by Strata's rule: none onto the items'
    /// own folder or into a dragged folder, else the copy-or-move rule with the demo's volume
    /// lookup and "Always ask" across devices.
    fn action_for(&self, dest: &Path, offer: &DropOffer, mods: Mods) -> Option<DropAction> {
        let sources = offer.uris.as_deref().map(fsys::paths).unwrap_or_default();
        if fsys::no_op(&sources, dest) {
            return None;
        }
        let volume = fsys::volume(&self.root, &sources, dest);
        choose_action(ActionInput::new(
            mods,
            offer.actions,
            volume,
            CrossDevice::Ask,
        ))
    }

    /// A drop into `dest`: files are transferred with the host's action (with `--apply`),
    /// content is noted. Logs a `transfer` line either way.
    fn transfer(&mut self, dest: &Path, dropped: &Dropped, via: &str, tx: &mut Tx) {
        // A drop ends the drag: C22's duties for one that didn't start in this window (the
        // adapter's end check may have run already, mid-drag, if the drag left every target).
        if self.out.is_none() {
            self.end_of_drag(tx);
        }
        let local = dropped.data.local.is_some();
        let uris = match &dropped.data.local {
            Some(p) => p.uris.clone(),
            None => dropped.data.uris(),
        };
        let types: Vec<&str> = dropped.data.items.iter().map(|i| i.mime.as_str()).collect();
        if uris.is_empty() {
            let sizes: Vec<(String, usize)> = dropped
                .data
                .items
                .iter()
                .map(|i| (i.mime.clone(), i.data.len()))
                .collect();
            self.log(
                "content",
                json!({ "via": via, "into": dest, "types": types, "sizes": sizes }),
            );
            return;
        }
        let sources = fsys::paths(&uris);
        let offer = dropped.offer.clone().uris(uris.clone());
        let action = self.action_for(dest, &offer, dropped.mods);
        let names: Vec<String> = sources.iter().map(|p| name_of(p)).collect();
        let action_name = match action {
            None => "none",
            Some(DropAction::Copy) => "copy",
            Some(DropAction::Move) => "move",
            Some(DropAction::Link) => "link",
            Some(DropAction::Ask) => "ask",
        };
        let mut fields = json!({
            "via": via, "into": dest, "action": action_name, "sources": names,
            "local": local, "types": types,
        });
        if self.apply && matches!(action, Some(DropAction::Copy | DropAction::Move)) {
            let moving = action == Some(DropAction::Move);
            match fsys::apply(&self.root, &sources, dest, moving) {
                Ok(written) => fields["written"] = json!(written),
                Err(e) => fields["error"] = json!(e),
            }
            let mut folders: Vec<PathBuf> = sources
                .iter()
                .filter_map(|p| p.parent().map(Path::to_owned))
                .collect();
            folders.push(dest.to_owned());
            self.relist(&folders, tx);
        }
        self.log("transfer", fields);
    }

    /// A drop on the sidebar Trash: Strata moves the files to Trash. The demo logs a
    /// `trash` transfer and never trashes anything, even with `--apply`.
    fn trash(&mut self, dropped: &Dropped, tx: &mut Tx) {
        if self.out.is_none() {
            self.end_of_drag(tx);
        }
        let local = dropped.data.local.is_some();
        let uris = match &dropped.data.local {
            Some(p) => p.uris.clone(),
            None => dropped.data.uris(),
        };
        let offer = dropped.offer.clone().uris(uris.clone());
        let action = if trash_action(&offer).is_some() {
            "trash"
        } else {
            "none"
        };
        let names: Vec<String> = fsys::paths(&uris).iter().map(|p| name_of(p)).collect();
        self.log(
            "transfer",
            json!({
                "via": "sidebar", "into": "trash", "action": action, "sources": names,
                "local": local,
            }),
        );
    }

    // ---- Drags out ----

    fn begin_drag(&mut self, tab: TabId, ids: Vec<EntryId>, tx: &mut Tx) {
        // C22: starting a drag closes any open peek.
        if self.peek.close() {
            self.peeked = Peeked::default();
            self.log("peek", json!({ "open": false, "why": "drag" }));
        }
        let uris: Vec<String> = ids.iter().filter_map(|id| self.ids.uri(*id)).collect();
        let names: Vec<String> = ids
            .iter()
            .filter_map(|id| self.ids.path(*id).map(name_of))
            .collect();
        let folders = ids
            .iter()
            .all(|id| self.ids.path(*id).is_some_and(Path::is_dir));
        let payload = DragPayload::new(ids.clone(), uris);
        let icon = drag_icon(ids.len(), folders, self.scale.get());
        let r = self
            .wiring
            .clone()
            .start(tx, payload, Some(icon), |w: &mut Win, outcome, tx| {
                w.drag_ended(outcome, tx)
            });
        match r {
            Ok(()) => {
                self.out = Some(Out { tab, ids });
                self.log(
                    "drag-start",
                    json!({ "tab": tab.0, "items": names, "mods": mods_json(tx.mods()) }),
                );
            }
            Err(e) => self.log("drag-error", json!({ "error": e.to_string() })),
        }
    }

    fn drag_ended(&mut self, outcome: DragOutcome, tx: &mut Tx) {
        self.out = None;
        self.end_of_drag(tx);
        let o = match outcome {
            DragOutcome::Copied => "copied",
            DragOutcome::Moved => "moved",
            DragOutcome::Linked => "linked",
            DragOutcome::Cancelled => "cancelled",
            _ => "other",
        };
        self.log("drag-end", json!({ "outcome": o }));
        tx.wake_after(Duration::ZERO);
    }

    /// Any drag's end, this window's or another app's: C22's duties, and the spring stops.
    fn end_of_drag(&mut self, _tx: &mut Tx) {
        if self.peek.close() {
            self.peeked = Peeked::default();
        }
        for t in &mut self.tabs {
            match &mut t.pane {
                Pane::List(f) | Pane::Icons(f) => f.coll.hold_rest(),
                Pane::Columns { state, .. } => {
                    state.drag_over(None);
                }
            }
        }
        self.spring = None;
    }

    // ---- Commands ----

    pub fn command(&mut self, cmd: &Value, tx: &mut Tx) {
        match cmd["cmd"].as_str().unwrap_or("") {
            "locate" => {
                let want = cmd["what"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|item| (item["name"].as_str().unwrap_or("").to_owned(), item.clone()))
                    .collect();
                *self.locate.borrow_mut() = want;
                tx.wake_after(Duration::ZERO);
            }
            "tab" => {
                let id = TabId(cmd["id"].as_u64().unwrap_or(1));
                self.select_tab(id, "command", tx);
            }
            "select" => self.select(cmd, tx),
            "group" => {
                // List's file-type groups (C4's), folders first: the stuck header.
                let id = TabId(cmd["tab"].as_u64().unwrap_or(self.active.0));
                if let Pane::List(f) = &mut self.tab_mut(id).pane {
                    let groups = groups_from_runs(f.listing.len(), |i| {
                        if f.listing.entry(i).kind.is_folder_like() {
                            (0, "Folders".into())
                        } else {
                            (1, "Files".into())
                        }
                    });
                    f.coll.set_groups(groups);
                    f.version += 1;
                    let (key, l) = (f.key.clone(), &f.listing);
                    f.coll
                        .sync(&key, l, SyncRule::complete(CursorGone::Neighbor), tx);
                }
                self.log("grouped", json!({ "tab": id.0 }));
            }
            "scroll" => {
                let id = TabId(cmd["tab"].as_u64().unwrap_or(self.active.0));
                let y = cmd["y"].as_f64().unwrap_or(0.0) as f32;
                if let Pane::List(f) | Pane::Icons(f) = &self.tab(id).pane {
                    tx.scroll_to_instant(viewport_path(&f.key), 0.0, y);
                }
                self.log("scrolled", json!({ "tab": id.0, "y": y }));
            }
            "pick" => {
                // What a drag's hit test (P1's) finds at window point (x, y).
                let (x, y) = (
                    cmd["x"].as_f64().unwrap_or(0.0) as f32,
                    cmd["y"].as_f64().unwrap_or(0.0) as f32,
                );
                tx.element_at_with(
                    x,
                    y,
                    rune::Pick::new().transparent(true),
                    move |w: &mut Win, picked: Option<rune::Picked>, _: &mut Tx| {
                        let path = picked.map(|p| p.path().to_owned());
                        w.log("picked", json!({ "x": x, "y": y, "path": path }));
                    },
                );
            }
            "open" => {
                let id = TabId(cmd["tab"].as_u64().unwrap_or(self.active.0));
                let path = self.root.join(cmd["path"].as_str().unwrap_or(""));
                if self.tabs.iter().any(|t| t.id == id) && path.starts_with(&self.root) {
                    self.navigate(id, &path, tx);
                } else {
                    self.log("bad-command", json!({ "cmd": "open" }));
                }
            }
            "state" => self.state(),
            "cancel" => {
                self.wiring.p1().cancel_drag();
                self.log("cancel-sent", json!({}));
            }
            "peek" => {
                let on = cmd["on"].as_bool().unwrap_or(true);
                self.peek.set_enabled(on);
                self.log("peeking", json!({ "on": on }));
            }
            other => self.log("bad-command", json!({ "cmd": other })),
        }
    }

    /// Selects the entries named `names` in tab `tab` (column `depth` in Columns).
    fn select(&mut self, cmd: &Value, tx: &mut Tx) {
        let id = TabId(cmd["tab"].as_u64().unwrap_or(self.active.0));
        let depth = cmd["depth"].as_u64().unwrap_or(0) as usize;
        let names: Vec<String> = cmd["names"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|n| n.as_str().map(str::to_owned))
            .collect();
        let pick = |source: &dyn EntrySource, coll: &mut CollectionState| {
            let indices: Vec<usize> = (0..source.len())
                .filter(|&i| names.iter().any(|n| n == source.entry(i).name))
                .collect();
            if let Some((&first, rest)) = indices.split_first() {
                let _ = coll.selection.select_only(source, first);
                for &i in rest {
                    let _ = coll.selection.toggle(source, i);
                }
                coll.note_selection(source);
            }
            indices.len()
        };
        let n = match &mut self.tab_mut(id).pane {
            Pane::List(f) | Pane::Icons(f) => pick(&f.listing, &mut f.coll),
            Pane::Columns {
                listings, state, ..
            } => match (listings.0.get(depth), state.column_mut(depth)) {
                (Some(l), Some(c)) => pick(&l.source, &mut c.collection),
                _ => 0,
            },
        };
        tx.wake_after(Duration::ZERO);
        self.log("selected", json!({ "count": n }));
    }

    fn state(&self) {
        let tab = self.tab(self.active);
        let mut fields = json!({
            "tab": self.active.0,
            "tabs": self.tabs.iter().map(|t| t.id.0).collect::<Vec<_>>(),
            "folder": self.folder_of(self.active),
            "pins": self.pins.iter().map(|p| p.label.clone()).collect::<Vec<_>>(),
        });
        // The active pane's rows drawn faded as the running drag's (`marks_of`, as drawn).
        let sources: Vec<&ListSource> = match &tab.pane {
            Pane::List(f) | Pane::Icons(f) => vec![&f.listing],
            Pane::Columns { listings, .. } => listings.0.iter().map(|l| &l.source).collect(),
        };
        let faded: Vec<String> = sources
            .into_iter()
            .flat_map(|src| (0..src.len()).map(|i| src.id_at(i)))
            .filter(|&id| marks_of(self, self.active, "", id).dragging)
            .filter_map(|id| self.ids.path(id).map(name_of))
            .collect();
        fields["faded"] = json!(faded);
        if let Pane::Columns { listings, .. } = &tab.pane {
            fields["columns"] = json!(
                listings
                    .0
                    .iter()
                    .map(|l| l.name.clone())
                    .collect::<Vec<_>>()
            );
        }
        self.log("state", fields);
    }

    /// The key path of what `item` names, for `locate`.
    fn resolve(&self, item: &Value) -> Option<String> {
        if let Some(name) = item["row"].as_str() {
            let tab = TabId(item["tab"].as_u64().unwrap_or(self.active.0));
            return match &self.tab(tab).pane {
                Pane::List(f) => {
                    let id = find(&f.listing, name)?;
                    Some(row_path(&f.key, id))
                }
                Pane::Icons(f) => {
                    let i = (0..f.listing.len()).find(|&i| f.listing.entry(i).name == name)?;
                    // The tile's icon: a press on a card's blank space starts a band.
                    let tile = entry_path(&f.key, &f.listing, i, f.coll.per_line());
                    Some(format!("{tile}/icon"))
                }
                Pane::Columns {
                    listings, state, ..
                } => {
                    let depth = item["depth"].as_u64().unwrap_or(0) as usize;
                    let id = find(&listings.0.get(depth)?.source, name)?;
                    Some(row_path(&state.column_key(depth)?, id))
                }
            };
        }
        if let Some(depth) = item["column"].as_u64() {
            let tab = TabId(item["tab"].as_u64().unwrap_or(self.active.0));
            if let Pane::Columns { state, .. } = &self.tab(tab).pane {
                return state.column_key(depth as usize);
            }
            return None;
        }
        if item["pane"].as_bool() == Some(true) {
            let tab = TabId(item["tab"].as_u64().unwrap_or(self.active.0));
            return Some(match &self.tab(tab).pane {
                Pane::List(f) | Pane::Icons(f) => f.key.clone(),
                Pane::Columns { key, .. } => format!("{key}-host"),
            });
        }
        if item["stuck"].as_bool() == Some(true) {
            // The stuck group header over List's top line.
            let tab = TabId(item["tab"].as_u64().unwrap_or(self.active.0));
            return match &self.tab(tab).pane {
                Pane::List(f) => Some(format!("{}/stuck", f.key)),
                _ => None,
            };
        }
        if let Some(i) = item["crumb"].as_u64() {
            return Some(crumb_key(LOC, i as usize));
        }
        if let Some(t) = item["tab"].as_u64().filter(|_| item.get("row").is_none()) {
            return Some(tab_key(STRIP, TabId(t)));
        }
        if item["new-tab"].as_bool() == Some(true) {
            return Some(new_tab_key(STRIP));
        }
        if let Some(i) = item["gap"].as_u64() {
            return Some(format!("{SIDEBAR}-band-{i}"));
        }
        if let Some(p) = item["place"].as_str() {
            return Some(ItemId::Place(path_id(&self.root.join(p))).key(SIDEBAR));
        }
        if item["trash"].as_bool() == Some(true) {
            return Some(ItemId::Place("trash".into()).key(SIDEBAR));
        }
        if item["device"].as_bool() == Some(true) {
            return Some(ItemId::Device(path_id(&self.root.join("usb"))).key(SIDEBAR));
        }
        item["key"].as_str().map(str::to_owned)
    }

    fn sidebar_into(&self, item: &ItemId) -> Option<PathBuf> {
        match item {
            ItemId::Place(id) | ItemId::Pin(id) | ItemId::Device(id) if id != "trash" => {
                Some(PathBuf::from(id.replace(':', "/")))
            }
            _ => None,
        }
    }
}

/// Strata's Trash rule: local files only, moved there (web addresses, alone or with files,
/// refuse the drop). While the URIs aren't known yet, a file list is taken.
fn trash_action(offer: &DropOffer) -> Option<DropAction> {
    let files = match offer.uris.as_deref() {
        Some(u) => !u.is_empty() && fsys::paths(u).len() == u.len(),
        None => offer.mime_types.iter().any(|m| m == URI_LIST),
    };
    (files && offer.actions.contains(DropActions::MOVE)).then_some(DropAction::Move)
}

/// The collections' drag hook: P1's guard and the phase a drag starts from, and a `press`
/// line for the session tests.
fn phase_hook(wiring: &Wiring<Win>, w: &mut Win, d: &rune::Drag, key: &str) -> bool {
    let owned = wiring.drag_phase(d, key);
    if d.phase == rune::Phase::Down {
        w.log("press", json!({ "key": key }));
    }
    owned
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Columns,
    List,
    Icons,
}

/// A sidebar row's id for folder `p`. Row keys carry the id (`ItemId::key`), and a key holds no
/// `/`, which separates a key path's segments: P1 finds targets by those segments.
fn path_id(p: &Path) -> String {
    p.display().to_string().replace('/', ":")
}

fn find(source: &dyn EntrySource, name: &str) -> Option<EntryId> {
    (0..source.len())
        .find(|&i| source.entry(i).name == name)
        .map(|i| source.id_at(i))
}

pub fn mods_json(m: Mods) -> Value {
    json!({ "ctrl": m.ctrl, "shift": m.shift })
}

fn install(coll: &mut CollectionState) {
    use strata_components::collection::{
        keyboard::Keyboard, marquee::Marquee, pointer::PointerIntent,
    };
    coll.controllers.pointer = Some(Box::new(PointerIntent::new()));
    coll.controllers.keyboard = Some(Box::<Keyboard>::default());
    coll.controllers.marquee = Some(Box::new(Marquee::new()));
}

#[expect(
    clippy::too_many_arguments,
    reason = "a column's whole opening, as C2's page has it"
)]
fn open_column(
    listings: &mut Listings,
    state: &mut ColumnsState,
    depth: usize,
    folder: &Path,
    from: EntryId,
    source: ListSource,
    look: &Look,
    tx: &mut Tx,
) {
    let child = depth + 1;
    listings.0.truncate(child);
    listings.0.push(Listed {
        folder: folder.to_owned(),
        name: name_of(folder),
        source,
        version: 1,
        from: Some(from),
    });
    let source = &listings.0[child].source;
    if child < state.len() {
        state.swap_from(
            child,
            Opening::new(source, Listing::Complete),
            Focus::Column,
            look,
            tx,
        );
    } else {
        state.push(
            Opening::new(source, Listing::Complete),
            Focus::Column,
            look,
            tx,
        );
    }
}

/// The drag's picture: the entry's icon, with the count on a badge for several.
fn drag_icon(count: usize, folders: bool, scale: f32) -> DragIcon {
    #[derive(Clone)]
    struct Pic {
        count: usize,
        folders: bool,
    }
    fn view(p: &Pic, _: &mut Cx<Pic>) -> El {
        let look = Look::fallback(false);
        let name = if p.folders {
            IconName::Folder
        } else {
            IconName::File
        };
        let glyph = rune::icon(look.icon(name), 32.0).color(look.kit.c.icon_base);
        let mut el = col().size(48.0, 48.0).center().child(glyph);
        if p.count > 1 {
            el = el.child(
                text(p.count.to_string())
                    .font(look.kit.ty.small)
                    .color(look.kit.c.submit_fg)
                    .fill(look.kit.c.bg_accent)
                    .radius(8.0)
                    .px(4.0)
                    .pin(None, Some(0.0), Some(0.0), None),
            );
        }
        el
    }
    DragIcon::snapshot(Pic { count, folders }, view, (48.0, 48.0), scale, false)
}

/// The List's columns: the name and the size.
fn columns() -> Vec<Column<Win>> {
    vec![
        Column::primary("name", "Name").width(240.0).min(160.0),
        Column::new("kind", "Kind", |_: &Win, rcx| {
            ListCell::text(if rcx.entry.kind.is_folder_like() {
                "Folder"
            } else {
                "File"
            })
            .muted()
        })
        .width(100.0)
        .min(80.0),
    ]
}

// ---- The view ----

/// The marks on a pane's rows: those of this window's running drag are drawn faded, and the
/// peek's anchor.
fn marks_of(w: &Win, tab: TabId, scope: &str, id: EntryId) -> RowMarks {
    let mut m = RowMarks::default();
    m.dragging = w
        .out
        .as_ref()
        .is_some_and(|o| o.tab == tab && o.ids.contains(&id));
    m.peek_anchor = w.peek.is_anchor(scope, id);
    m
}

fn peek() -> FolderPeek<Win> {
    FolderPeek::new(
        |w: &Win| &w.peek,
        |w: &mut Win| &mut w.peek,
        |w: &Win| {
            let p = &w.peeked;
            PeekContents::new(&p.name, &p.status, &p.rows).more(p.more)
        },
        |w: &Win| &w.look,
    )
    .on_change(|w: &mut Win, change, _| w.peek_changed(change))
    .on_open(|w: &mut Win, target, tx| {
        if let Some(p) = w.ids.path(target.folder).map(Path::to_owned) {
            let tab = w.active;
            w.navigate(tab, &p, tx);
        }
    })
}

impl Win {
    fn peek_changed(&mut self, change: Change) {
        match change {
            Change::None => {}
            Change::Opened => {
                let Some(t) = self.peek.target() else { return };
                let Some(folder) = self.ids.path(t.folder).map(Path::to_owned) else {
                    return;
                };
                // The first 64 rows (Strata's cap), and how many more.
                let all = fsys::list(&mut self.peek_ids, &folder);
                let n = all.len();
                let rows = ListSource::new(all.entries().iter().take(64).cloned().collect());
                self.peeked = Peeked {
                    name: name_of(&folder),
                    status: PeekStatus::Ready,
                    rows,
                    more: n.checked_sub(64).filter(|m| *m > 0).map(More::Exactly),
                };
                self.log("peek", json!({ "open": true, "folder": folder }));
            }
            Change::Closed => {
                self.peeked = Peeked::default();
                self.log("peek", json!({ "open": false }));
            }
        }
    }

    /// A pane's collection before its view adds its parts: drops, actions, the drag hooks.
    fn base(tab: TabId, key: &str, wiring: &Wiring<Win>) -> Collection<Win> {
        let phase = wiring.clone();
        Collection::new(
            key.to_owned(),
            move |w: &Win| {
                let f = w.flat(tab);
                (&f.listing, &f.coll)
            },
            move |w: &mut Win| {
                let f = w.flat_mut(tab);
                (&f.listing as &dyn EntrySource, &mut f.coll)
            },
            |w: &Win| &w.look,
            move |w: &Win| w.flat(tab).version,
        )
        .dnd(wiring.binder())
        .on_drag_phase(move |w: &mut Win, d, key, _| phase_hook(&phase, w, d, key))
        .on_drop(move |w: &mut Win, target, dropped, tx| {
            let dest = match target {
                Some(id) => w.ids.path(id).map(Path::to_owned),
                None => Some(w.flat(tab).folder.clone()),
            };
            let via = if target.is_some() {
                "row"
            } else {
                "background"
            };
            if let Some(dest) = dest {
                w.transfer(&dest, &dropped, via, tx);
            }
        })
        .drop_action(move |w: &Win, target, offer, mods| {
            let dest = match target {
                Some(id) => w.ids.path(id)?.to_owned(),
                None => w.flat(tab).folder.clone(),
            };
            w.action_for(&dest, offer, mods)
        })
        .on_intent(move |w: &mut Win, i, tx| w.flat_intent(tab, i, tx))
    }

    fn flat_intent(&mut self, tab: TabId, intent: Intent, tx: &mut Tx) {
        match intent {
            Intent::BeginDrag { ids, .. } => self.begin_drag(tab, ids, tx),
            Intent::Activate { id, .. } => {
                if let Some(p) = self.ids.path(id).filter(|p| p.is_dir()).map(Path::to_owned) {
                    self.navigate(tab, &p, tx);
                }
            }
            _ => {}
        }
    }

    fn columns_intent(&mut self, tab: TabId, intent: ColumnsIntent, tx: &mut Tx) {
        let look = self.look.clone();
        match intent {
            ColumnsIntent::Collection { intent, .. } => match intent {
                Intent::BeginDrag { ids, .. } => self.begin_drag(tab, ids, tx),
                Intent::Activate { id, .. } => self.open_in_columns(tab, id, &look, tx),
                _ => {}
            },
            ColumnsIntent::EnterFolder { id, .. } => self.open_in_columns(tab, id, &look, tx),
            ColumnsIntent::CloseFrom(depth) => {
                let depth = depth.max(1);
                let to = match &self.tab(tab).pane {
                    Pane::Columns { listings, .. } => {
                        listings.0.get(depth - 1).map(|l| l.folder.clone())
                    }
                    _ => None,
                };
                if let Some(to) = to
                    && !self.holds_source(tab, depth, &to)
                    && let Pane::Columns {
                        listings, state, ..
                    } = &mut self.tab_mut(tab).pane
                {
                    listings.0.truncate(depth);
                    state.truncate(depth, &look, tx);
                }
            }
            ColumnsIntent::Width(w) => {
                if let Pane::Columns { state, .. } = &mut self.tab_mut(tab).pane {
                    state.set_default_width(w);
                }
            }
            ColumnsIntent::Dropped {
                depth,
                target,
                dropped,
            } => {
                let dest = self.column_dest(tab, depth, target);
                let via = if target.is_some() {
                    "row"
                } else {
                    "background"
                };
                if let Some(dest) = dest {
                    self.transfer(&dest, &dropped, via, tx);
                }
            }
            _ => {}
        }
        if tab == self.active {
            self.retrail();
        }
    }

    fn column_dest(&self, tab: TabId, depth: usize, target: Option<EntryId>) -> Option<PathBuf> {
        match target {
            Some(id) => self.ids.path(id).map(Path::to_owned),
            None => match &self.tab(tab).pane {
                Pane::Columns { listings, .. } => listings.0.get(depth).map(|l| l.folder.clone()),
                _ => None,
            },
        }
    }

    fn open_in_columns(&mut self, tab: TabId, id: EntryId, look: &Look, tx: &mut Tx) {
        let Some(folder) = self.ids.path(id).filter(|p| p.is_dir()).map(Path::to_owned) else {
            return;
        };
        let source = fsys::list(&mut self.ids, &folder);
        if let Pane::Columns { listings, .. } = &self.tab(tab).pane {
            let Some(depth) = listings
                .0
                .iter()
                .position(|l| folder.parent() == Some(&l.folder))
            else {
                return;
            };
            if self.holds_source(tab, depth + 1, &folder) {
                return;
            }
            let Pane::Columns {
                listings, state, ..
            } = &mut self.tab_mut(tab).pane
            else {
                return;
            };
            open_column(listings, state, depth, &folder, id, source, look, tx);
        }
        self.log("navigate", json!({ "tab": tab.0, "folder": folder }));
    }

    /// Whether Columns tab `tab` keeping only its first `keep` columns would unmount the
    /// column a running drag started in: P1's rule keeps a drag's source mounted until the
    /// drag ends, so the navigation to `to` is dropped (logged `held`), not queued: nothing
    /// opens when the drag ends.
    fn holds_source(&self, tab: TabId, keep: usize, to: &Path) -> bool {
        let Some(out) = self.out.as_ref().filter(|o| o.tab == tab) else {
            return false;
        };
        let Pane::Columns { listings, .. } = &self.tab(tab).pane else {
            return false;
        };
        let source = listings.0.iter().position(|l| {
            let src = &l.source;
            (0..src.len()).any(|i| out.ids.contains(&src.id_at(i)))
        });
        let held = source.is_some_and(|depth| keep <= depth);
        if held {
            self.log(
                "held",
                json!({ "tab": tab.0, "folder": to, "source": source }),
            );
        }
        held
    }

    fn columns_view(tab: TabId, wiring: &Wiring<Win>) -> ColumnsView<Win> {
        let phase = wiring.clone();
        ColumnsView::new(
            move |w: &Win| {
                let (listings, state) = w.cols(tab);
                ColumnsParts {
                    sources: listings,
                    state,
                    look: &w.look,
                }
            },
            move |w: &mut Win| {
                let Win { tabs, look, .. } = w;
                let t = tabs.iter_mut().find(|t| t.id == tab).expect("an open tab");
                match &mut t.pane {
                    Pane::Columns {
                        listings, state, ..
                    } => ColumnsPartsMut {
                        sources: &*listings,
                        state,
                        look: &*look,
                    },
                    _ => panic!("not a columns pane"),
                }
            },
        )
        .collection(move |_: &Win, _, c| {
            let p = phase.clone();
            c.on_drag_phase(move |w: &mut Win, d, key, _| phase_hook(&p, w, d, key))
        })
        .marks(move |w: &Win, depth, id| {
            let scope = w.cols(tab).1.column_key(depth).unwrap_or_default();
            marks_of(w, tab, &scope, id)
        })
        .dnd(wiring.binder())
        .drop_action(move |w: &Win, at, offer, mods| {
            let (depth, target) = at?;
            let dest = w.column_dest(tab, depth, target)?;
            w.action_for(&dest, offer, mods)
        })
        .on_intent(move |w: &mut Win, i, tx| w.columns_intent(tab, i, tx))
    }

    fn cols(&self, tab: TabId) -> (&Listings, &ColumnsState) {
        match &self.tab(tab).pane {
            Pane::Columns {
                listings, state, ..
            } => (listings, state),
            _ => panic!("not a columns pane"),
        }
    }

    /// Columns' own target over the whole strip, for what no column's zone covers: the gaps
    /// between columns. Its moves feed C2's autoscroll and column highlight
    /// (`ColumnsState::drag_over`); a drop there goes where `ColumnsView::drop_target_at` says
    /// (the column beside it), and its action is asked with the drag's point (P1's
    /// `action_at`). Columns' rows and backgrounds take drags themselves, through C1e's zones.
    fn strip_zone(tab: TabId) -> DropZone<Win> {
        DropZone::new(move |w: &mut Win, dropped: Dropped, tx| {
            let at = dropped.point.map(|p| Point::new(p.window_x, p.window_y));
            let target = at.and_then(|at| w.strip_target(tab, at));
            w.strip_over(tab, None, tx);
            match target.and_then(|(depth, row)| Some((w.column_dest(tab, depth, row)?, row))) {
                Some((dest, row)) => {
                    let via = if row.is_some() { "row" } else { "background" };
                    w.transfer(&dest, &dropped, via, tx);
                }
                None => w.log("strip-miss", json!({})),
            }
        })
        // A refused drag over a gap stops there (no outer target takes it) and keeps
        // driving the autoscroll and the column wash, with no drop highlight.
        .on_hover(move |w: &mut Win, h, offer: &DropOffer, tx| {
            use strata_components::contract::dnd::DropHover;
            w.strip_refused =
                (h == DropHover::Rejecting).then(|| offer.mime_types.iter().any(|m| m == URI_LIST));
            if h == DropHover::Left {
                w.strip_over(tab, None, tx);
            }
        })
        .on_reject(move |w: &mut Win, _, tx| {
            w.strip_refused = None;
            w.strip_over(tab, None, tx);
        })
        .on_move(move |w: &mut Win, p, tx| {
            // Only a file drag autoscrolls (column-autoscroll).
            if w.strip_refused != Some(false) {
                w.strip_over(tab, Some(Point::new(p.window_x, p.window_y)), tx);
            }
        })
    }

    /// The strip's action, by where the drag is.
    fn strip_action(
        &self,
        tab: TabId,
        offer: &DropOffer,
        mods: Mods,
        at: Point,
    ) -> Option<DropAction> {
        let (depth, row) = self.strip_target(tab, at)?;
        let dest = self.column_dest(tab, depth, row)?;
        self.action_for(&dest, offer, mods)
    }

    fn strip_target(&self, tab: TabId, at: Point) -> Option<(usize, Option<EntryId>)> {
        Win::columns_view(tab, &self.wiring).drop_target_at(self, at)
    }

    /// Where a drag over the strip's gaps is now, taken or refused (`None`: it left or dropped).
    fn strip_over(&mut self, tab: TabId, at: Option<Point>, tx: &mut Tx) {
        let target = at
            .filter(|_| self.strip_refused.is_none())
            .and_then(|at| self.strip_target(tab, at));
        if let Pane::Columns { state, .. } = &mut self.tab_mut(tab).pane {
            let mut wake = state.drag_over(at);
            for depth in 0..state.len() {
                let drop = match target {
                    Some((d, Some(id))) if d == depth => Some(DropOver::Row(id)),
                    Some((d, None)) if d == depth => Some(DropOver::Background),
                    _ => None,
                };
                if let Some(c) = state.column_mut(depth)
                    && c.collection.drop_over() != drop
                {
                    c.collection.set_drop_over(drop);
                    wake = true;
                }
            }
            if wake {
                tx.wake_after(Duration::ZERO);
            }
        }
    }

    fn pane_el(&self, tab: TabId, cx: &mut Cx<Win>, layer: &mut Vec<El>) -> El {
        let wiring = self.wiring.clone();
        match &self.tab(tab).pane {
            Pane::Columns { key, .. } => {
                let strip = Win::columns_view(tab, &wiring).build(self);
                let host = col()
                    .key(format!("{key}-host"))
                    .w_full()
                    .h_full()
                    .min_h(0.0)
                    .child(strip);
                self.wiring.drop_target_by_point(
                    host,
                    &format!("{key}-host"),
                    Win::strip_zone(tab),
                    move |w: &Win, offer, mods, p| {
                        w.strip_action(tab, offer, mods, Point::new(p.window_x, p.window_y))
                    },
                )
            }
            Pane::List(f) => {
                let key = f.key.clone();
                let base = Win::base(tab, &key, &wiring)
                    .name(name_of(&f.folder))
                    .marks(move |w: &Win, id| marks_of(w, tab, &key, id));
                let table = ListTable::new(
                    &f.key,
                    columns(),
                    move |w: &Win| {
                        let f = w.flat(tab);
                        ListParts {
                            source: &f.listing,
                            collection: &f.coll,
                            table: &f.table,
                            layout: &w.layout,
                            widths: &w.widths,
                            sort: None,
                            look: &w.look,
                        }
                    },
                    move |w: &mut Win| &mut w.flat_mut(tab).table,
                );
                let rows = if tab == self.active {
                    peek().rows(base)
                } else {
                    base
                };
                table
                    .follow_sideways(|w: &Win| peek().follow_sideways(w))
                    .build(self, rows, layer)
            }
            Pane::Icons(f) => {
                let key = f.key.clone();
                let active = tab == self.active;
                let w2 = wiring.clone();
                let grid = IconsGrid::new(
                    &f.key,
                    move |w: &Win| {
                        let k = key.clone();
                        let base = Win::base(tab, &key, &w2)
                            .name(name_of(&w.flat(tab).folder))
                            .marks(move |w: &Win, id| marks_of(w, tab, &k, id));
                        if active { peek().rows(base) } else { base }
                    },
                    move |w: &Win| {
                        let f = w.flat(tab);
                        IconsParts {
                            source: &f.listing,
                            collection: &f.coll,
                            grid: &f.grid,
                            size: 64.0,
                            listing: f.version,
                            look: &w.look,
                            thumbs: None,
                        }
                    },
                    move |w: &mut Win| &mut w.flat_mut(tab).grid,
                );
                let _ = cx;
                grid.scale(self.scale.get()).build(self)
            }
        }
    }
}

pub fn view(w: &Win, cx: &mut Cx<Win>) -> El {
    w.scale.set(cx.env().scale);
    cx.reduced_motion(!w.look.animations);
    let look = &w.look;
    let mut layer = Vec::new();
    let wiring = w.wiring.clone();

    let tabs: Vec<Tab<'_>> = w
        .tabs
        .iter()
        .map(|t| Tab::new(t.id, name_of(&w.folder_of(t.id)), IconName::Folder))
        .collect();
    let data = TabsData::new(&tabs, Some(w.active));
    let strip = BrowserTabs::new(
        &data,
        &w.tabs_state,
        |w: &mut Win| &mut w.tabs_state,
        |w| &mut w.tabs_state.drag,
    )
    .dnd(wiring.binder())
    .content_types(CONTENT)
    .opens_tabs(|w: &Win, offer| {
        // Strata's own folder-only drags open tabs, never folders from another app.
        if !offer.local {
            return false;
        }
        if let Some((serial, opens)) = w.opens.get()
            && serial == offer.serial
        {
            return opens;
        }
        let opens = adapter::running()
            .is_some_and(|p| !p.uris.is_empty() && fsys::paths(&p.uris).iter().all(|p| p.is_dir()));
        w.opens.set(Some((offer.serial, opens)));
        opens
    })
    .drop_action(|w: &Win, id, offer, mods| w.action_for(&w.folder_of(id), offer, mods))
    .on_intent(|w: &mut Win, i, tx| w.tabs_intent(i, tx))
    .build_layered(look, cx, &mut layer);

    let bar = LocationBar::new(
        LOC,
        look,
        &w.trail,
        &w.bar,
        |w: &mut Win| &mut w.bar,
        |w: &mut Win, i, tx| w.location_intent(i, tx),
    )
    .dnd(wiring.binder())
    .drop_action(|w: &Win, id, offer, mods| w.action_for(Path::new(id), offer, mods))
    .build(cx, &mut layer);

    let data = SidebarData {
        places: &w.places,
        pins: &w.pins,
        devices: &w.devices,
        current: None,
    };
    let sidebar = PlacesSidebar::new(&w.sidebar, |w: &mut Win| &mut w.sidebar, data)
        .dnd(wiring.binder())
        .drop_action(|w: &Win, item, offer, mods| match item {
            ItemId::Place(id) if id == "trash" => trash_action(offer),
            _ => w.action_for(&w.sidebar_into(item)?, offer, mods),
        })
        .on_intent(|w: &mut Win, i, tx| w.sidebar_intent(i, tx))
        .build(look);

    // While a drag this window started runs, the pane of the tab it started in stays mounted
    // when another tab is active (P1's rule: the source stays mounted at its place), drawn
    // under the active pane, transparent, inert and hidden. No other inactive pane is kept.
    let mut panes = col().key("panes").grow(1.0).min_w(0.0).h_full();
    let keep = w.out.as_ref().map(|o| o.tab);
    for t in &w.tabs {
        if t.id == w.active || Some(t.id) != keep {
            continue;
        }
        let el = w.pane_el(t.id, cx, &mut layer);
        panes = panes.child(
            col()
                .key(format!("pane-{}", t.id.0))
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .opacity(0.0)
                .inert()
                .a11y_hidden()
                .child(el),
        );
    }
    let active = w.pane_el(w.active, cx, &mut layer);
    panes = panes.child(
        col()
            .key(format!("pane-{}", w.active.0))
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .fill(look.kit.c.bg_base)
            .child(active),
    );
    if matches!(w.tab(w.active).pane, Pane::List(_) | Pane::Icons(_)) {
        peek().card(w, &mut layer);
    }

    let page = col()
        .w_full()
        .h_full()
        .fill(look.kit.c.bg_base)
        .child(row().w_full().child(strip))
        .child(row().w_full().px(look.space.sm).child(bar))
        .child(
            row()
                .key("body")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .child(sidebar)
                .child(panes),
        );
    let root = rune_kit::root(page, layer);
    let root = col()
        .w_full()
        .h_full()
        .child(root)
        .on_layout(|w: &mut Win, r: [f32; 4]| w.log("layout", json!({ "w": r[2], "h": r[3] })))
        .on_window_focus(|w: &mut Win, focused: bool, _: &mut Tx| {
            w.log("focus", json!({ "focused": focused }))
        })
        .on_hover_change(|w: &mut Win, over: bool, _: &mut Tx| {
            w.log("pointer", json!({ "over": over }))
        });
    let root = w
        .wiring
        .sentinel(root, ROOT, SENTINEL_TYPES, |w: &mut Win, tx| {
            w.end_of_drag(tx);
            // This window's own drag leaving it ends later, at P1's outcome.
            let ev = if w.out.is_some() {
                "drag-left"
            } else {
                "foreign-end"
            };
            w.log(ev, json!({}));
        });
    // `locate`: where each asked-for element is in the window now. A layout read has no
    // value the first time a view asks for it (Rune then runs the view again with it), so an
    // element not found yet stays asked for.
    let asked = std::mem::take(&mut *w.locate.borrow_mut());
    let mut pending = Vec::new();
    for (name, item) in asked {
        match w
            .resolve(&item)
            .and_then(|path| cx.window_layout(&path).map(|r| (path, r)))
        {
            Some((path, r)) => w.log("rect", json!({ "name": name, "key": path, "r": r })),
            None => pending.push((name, item)),
        }
    }
    *w.locate.borrow_mut() = pending;
    root
}

impl Win {
    fn tabs_intent(&mut self, intent: TabsIntent, tx: &mut Tx) {
        match intent {
            TabsIntent::Select { id, via } => self.select_tab(id, &format!("{via:?}"), tx),
            TabsIntent::DropInto { id, dropped } => {
                let dest = self.folder_of(id);
                self.transfer(&dest, &dropped, "tab", tx);
                self.select_tab(id, "drop", tx);
            }
            TabsIntent::OpenTabs { at, dropped } => {
                let uris = match &dropped.data.local {
                    Some(p) => p.uris.clone(),
                    None => dropped.data.uris(),
                };
                let mut last = None;
                for (k, p) in fsys::paths(&uris)
                    .into_iter()
                    .filter(|p| p.is_dir())
                    .enumerate()
                {
                    let id = self.add_tab(Kind::List, &p, Some(at + k), tx);
                    self.log(
                        "tab-open",
                        json!({ "tab": id.0, "folder": p, "at": at + k }),
                    );
                    last = Some(id);
                }
                if let Some(id) = last {
                    self.select_tab(id, "open", tx);
                }
            }
            TabsIntent::DropHover { site, hover, .. } => {
                self.log(
                    "hover",
                    json!({ "site": format!("{site:?}"), "hover": format!("{hover:?}") }),
                );
            }
            // Logged, not applied: the session tests find each window by its title.
            TabsIntent::Title(t) => self.log("title", json!({ "title": t })),
            TabsIntent::New => {
                let root = self.root.clone();
                let id = self.add_tab(Kind::List, &root, None, tx);
                self.select_tab(id, "new", tx);
            }
            _ => {}
        }
    }

    fn location_intent(&mut self, intent: LocationIntent, tx: &mut Tx) {
        match intent {
            LocationIntent::Open { id, via } => {
                let tab = self.active;
                self.navigate(tab, Path::new(&id), tx);
                self.log(
                    "crumb-open",
                    json!({ "folder": id, "via": format!("{via:?}") }),
                );
            }
            LocationIntent::DropInto { id, dropped } => {
                self.transfer(Path::new(&id), &dropped, "crumb", tx);
            }
            LocationIntent::DropHover { id, hover } => {
                self.log(
                    "hover",
                    json!({ "site": format!("crumb {id}"), "hover": format!("{hover:?}") }),
                );
            }
            _ => {}
        }
    }

    fn sidebar_intent(&mut self, intent: SidebarIntent, tx: &mut Tx) {
        match intent {
            SidebarIntent::DropInto { item, dropped } => match self.sidebar_into(&item) {
                Some(dest) => self.transfer(&dest, &dropped, "sidebar", tx),
                None => self.trash(&dropped, tx),
            },
            SidebarIntent::PinHover {
                serial,
                uris,
                local,
            } => {
                let verdict = match uris {
                    None => PinVerdict::Checking,
                    Some(u) => {
                        let p = fsys::paths(&u);
                        if !p.is_empty() && p.iter().all(|p| p.is_dir()) {
                            PinVerdict::Folders
                        } else {
                            PinVerdict::Refused
                        }
                    }
                };
                if local {
                    self.sidebar.pin_check.local = verdict;
                } else {
                    self.sidebar.pin_check.foreign = Some((serial, verdict));
                }
                tx.wake_after(Duration::ZERO);
            }
            SidebarIntent::PinDrop { gap, dropped } => {
                let uris = match &dropped.data.local {
                    Some(p) => p.uris.clone(),
                    None => dropped.data.uris(),
                };
                let folders: Vec<PathBuf> = fsys::paths(&uris)
                    .into_iter()
                    .filter(|p| p.is_dir())
                    .collect();
                for (k, p) in folders.iter().enumerate() {
                    let at = (gap.index + k).min(self.pins.len());
                    self.pins.insert(at, Pin::new(path_id(p), name_of(p)));
                }
                let names: Vec<String> = folders.iter().map(|p| name_of(p)).collect();
                self.log("pin", json!({ "gap": gap.index, "folders": names }));
            }
            SidebarIntent::DropHover { site, hover } => {
                self.log(
                    "hover",
                    json!({ "site": format!("{site:?}"), "hover": format!("{hover:?}") }),
                );
            }
            SidebarIntent::Open { item, .. } => {
                if let Some(p) = self.sidebar_into(&item) {
                    let tab = self.active;
                    self.navigate(tab, &p, tx);
                }
            }
            _ => {}
        }
    }

    /// Where a drag rests now, for the spring: a folder row in the active pane, or a sidebar
    /// row.
    fn spring_target(&self) -> Option<SpringAt> {
        if let Some((
            SideSite::Item(item),
            strata_components::contract::dnd::DropHover::Accepting,
        )) = self.sidebar.drop_over()
        {
            return Some(SpringAt::Place(item.clone()));
        }
        let tab = self.active;
        match &self.tab(tab).pane {
            Pane::List(f) | Pane::Icons(f) => match f.coll.drop_over() {
                Some(DropOver::Row(id)) => Some(SpringAt::Row { tab, depth: 0, id }),
                _ => None,
            },
            Pane::Columns { state, .. } => {
                (0..state.len()).find_map(|d| match state.column(d)?.collection.drop_over() {
                    Some(DropOver::Row(id)) => Some(SpringAt::Row { tab, depth: d, id }),
                    _ => None,
                })
            }
        }
    }
}

/// The window's poll: the clock, the springs (C6's and C7's own, the host's for rows), the
/// views' ticks (Columns' autoscroll and reveals, the collections' edge scrolls), and the
/// adapter's drag-end check.
pub fn poll(w: &mut Win, t: f64, tx: &mut Tx) -> bool {
    w.clock = t;
    let mut changed = false;
    let polled = w.bar.tick(t, tx);
    changed |= polled.changed;
    for i in polled.intents {
        w.location_intent(i, tx);
        changed = true;
    }
    let polled = {
        let tabs: Vec<Tab<'_>> = w
            .tabs
            .iter()
            .map(|t| Tab::new(t.id, String::new(), IconName::Folder))
            .collect();
        let data = TabsData::new(&tabs, Some(w.active));
        w.tabs_state.tick(t, &data, tx)
    };
    changed |= polled.changed;
    for i in polled.intents {
        w.tabs_intent(i, tx);
        changed = true;
    }
    // The host's spring on folder and sidebar rows.
    let now = w.spring_target();
    match (&w.spring, now) {
        (Some((at, since, fired)), Some(n)) if *at == n => {
            if !fired && t - since >= SPRING {
                let at = at.clone();
                w.spring = Some((at.clone(), *since, true));
                w.spring_fire(at, tx);
                changed = true;
            } else if !fired {
                tx.wake_after(Duration::from_secs_f64((SPRING - (t - since)).max(0.0)));
            }
        }
        (_, Some(n)) => {
            w.spring = Some((n, t, false));
            tx.wake_after(Duration::from_secs_f64(SPRING));
        }
        (_, None) => w.spring = None,
    }
    // C22: rows under an overlay don't peek (the location bar's suggestions).
    let covered = w.bar.editing();
    if covered != w.covered {
        w.covered = covered;
        let change = w.peek.set_covered(covered);
        w.peek_changed(change);
    }
    let tab = w.active;
    let wiring = w.wiring.clone();
    changed |= match &w.tab(tab).pane {
        Pane::Columns { .. } => Win::columns_view(tab, &wiring).tick(w, t, tx),
        Pane::List(_) => {
            let key = w.flat(tab).key.clone();
            let base = Win::base(tab, &key, &wiring);
            let table = ListTable::new(
                &key,
                columns(),
                move |w: &Win| {
                    let f = w.flat(tab);
                    ListParts {
                        source: &f.listing,
                        collection: &f.coll,
                        table: &f.table,
                        layout: &w.layout,
                        widths: &w.widths,
                        sort: None,
                        look: &w.look,
                    }
                },
                move |w: &mut Win| &mut w.flat_mut(tab).table,
            );
            table.collection(w, base).tick(w, tx)
        }
        Pane::Icons(_) => {
            let key = w.flat(tab).key.clone();
            Win::base(tab, &key, &wiring).tick(w, tx)
        }
    };
    changed |= wiring.tick(w, tx);
    changed
}

impl Win {
    fn spring_fire(&mut self, at: SpringAt, tx: &mut Tx) {
        match at {
            SpringAt::Row { tab, depth, id } => {
                self.log("spring", json!({ "site": "row", "depth": depth }));
                match &self.tab(tab).pane {
                    Pane::Columns { .. } => {
                        let look = self.look.clone();
                        self.open_in_columns(tab, id, &look, tx);
                    }
                    _ => {
                        if let Some(p) = self.ids.path(id).map(Path::to_owned) {
                            self.navigate(tab, &p, tx);
                        }
                    }
                }
            }
            SpringAt::Place(item) => {
                self.log("spring", json!({ "site": "sidebar" }));
                if let Some(p) = self.sidebar_into(&item) {
                    let tab = self.active;
                    self.navigate(tab, &p, tx);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
