// SPDX-License-Identifier: MIT

//! Hierarchical outline view. The root level mirrors Browser depth 0 through
//! the same row events as the list pane, so root rows resolve through the
//! application model. Expanded subdirectories load lazily through the shared
//! file source into view-local branch levels, because `Browser` only keeps
//! the single entered column path loaded. All tree verbs resolve entries from
//! the pane's own caches, which keeps nested rows correct in 10xer mode
//! without round-tripping the column stack.

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::{Rc, Weak},
    time::Duration,
};

use gtk::{gio, glib, prelude::*};

use super::{BoundModeItem, ClickActivation, register_bound_mode_item, set_label_if_changed};
use crate::{
    app::{Browser, sort_entries},
    model::{FileEntry, Location, ViewPreferences},
    services::{DirectoryEvent, DirectoryRequest, FileSource, LoadHandle, RequestId},
    ui::{
        accessibility,
        browser::{
            self, ClipboardMark, ClipboardMarks, ViewState, entry_icon, entry_model_value, mark_in,
            metadata_needs_fill, model_is_hidden, with_filter_terms,
        },
        collection_edit,
    },
};

const NATIVE_BRANCH_BATCH_SIZE: usize = 512;
const REMOTE_BRANCH_BATCH_SIZE: usize = 128;
const MAX_BRANCH_ENTRIES: usize = 100_000;
const BRANCH_LOAD_TIME_BUDGET: Duration = Duration::from_secs(10);

fn tree_model_value(entry: &FileEntry) -> String {
    let uri = crate::adapters::gio_file_for_location(&entry.location).uri();
    format!("{}\t{}", entry_model_value(entry), uri)
}

fn item_location(item: &glib::Object) -> Option<Location> {
    let string_object = item.downcast_ref::<gtk::StringObject>()?;
    let string = string_object.string();
    let (_, uri) = string.rsplit_once('\t')?;
    crate::adapters::location_for_file(&gio::File::for_uri(uri))
}

fn tree_model_display_name(value: &str) -> &str {
    let after_first_tab = value.split_once('\t').map_or(value, |(_, rest)| rest);
    after_first_tab
        .rsplit_once('\t')
        .map_or(after_first_tab, |(name, _)| name)
}

fn tree_entry_matches(value: &str, show_hidden: bool, query: &str) -> bool {
    (show_hidden || !model_is_hidden(value))
        && (query.trim().is_empty() || {
            let name = crate::services::fold_for_search(tree_model_display_name(value));
            if crate::ui::tenxer_mode::chrome_suppressed() {
                with_filter_terms(query, |terms| terms.score(&name, 0).is_some())
            } else {
                crate::services::filter_name_matches(&name, query)
            }
        })
}

fn tree_filter(
    show_hidden: Rc<Cell<bool>>,
    filter_query: Rc<RefCell<String>>,
) -> gtk::CustomFilter {
    gtk::CustomFilter::new(move |item| {
        let Some(item) = item.downcast_ref::<gtk::StringObject>() else {
            return false;
        };
        let value = item.string();
        tree_entry_matches(&value, show_hidden.get(), &filter_query.borrow())
    })
}

/// One visible level: the mirrored root or a lazily loaded branch.
struct TreeLevel {
    store: gtk::StringList,
    filtered: gtk::FilterListModel,
    load: Option<LoadHandle>,
    watch: Option<LoadHandle>,
    loaded: bool,
}

type TreeItemTrigger = Rc<dyn Fn(f64, f64)>;

struct TreeInner {
    page: gtk::Box,
    status: gtk::Label,
    view: gtk::ListView,
    selection: gtk::MultiSelection,
    tree_model: gtk::TreeListModel,
    browser: Weak<Browser>,
    source: Rc<dyn FileSource>,
    state: RefCell<Weak<ViewState>>,
    previews: Rc<Cell<bool>>,
    activation: Rc<Cell<ClickActivation>>,
    marks: Rc<RefCell<ClipboardMarks>>,
    root_store: gtk::StringList,
    root_index: super::SourceIndexMap,
    root_location: RefCell<Option<Location>>,
    /// Every visible row's entry, keyed by its model item. Branch rows can
    /// never resolve through `Browser`, so verbs read this map instead.
    entries: RefCell<HashMap<glib::Object, Rc<FileEntry>>>,
    /// Entries indexed by Location so lookups succeed immediately before
    /// StringList finishes attaching item wrappers.
    locations: RefCell<HashMap<Location, Rc<FileEntry>>>,
    /// Directory rows by location, for parent-row and re-expand lookups.
    directory_items: RefCell<HashMap<Location, glib::Object>>,
    /// Item to the branch level holding it (`None` is the mirrored root).
    item_levels: RefCell<HashMap<glib::Object, Option<Location>>>,
    branches: RefCell<HashMap<Location, TreeLevel>>,
    expanded: RefCell<HashSet<Location>>,
    visual_anchor: RefCell<Option<glib::Object>>,
    focused_item: RefCell<Option<glib::Object>>,
    show_hidden: Rc<Cell<bool>>,
    _columns: super::ListColumnLayout,
    _sorting: super::ListSorting,
    filter: gtk::CustomFilter,
    filter_query: Rc<RefCell<String>>,
    bound_items: Rc<RefCell<Vec<BoundModeItem>>>,
    syncing: Rc<Cell<bool>>,
    next_branch_id: Cell<u64>,
    menus_installed: Cell<bool>,
    item_trigger: RefCell<Option<TreeItemTrigger>>,
    /// Rows already submitted for metadata fill in this root generation.
    fill_requested: RefCell<HashSet<Location>>,
    /// Footer/status refresh callbacks for tree selection changes.
    selection_handlers: RefCell<Vec<Rc<dyn Fn()>>>,
    /// Set while an idle focus sync into Browser is already queued.
    focus_sync_scheduled: Cell<bool>,
}

#[derive(Clone)]
pub(in crate::ui) struct TreePane {
    inner: Rc<TreeInner>,
}

impl TreePane {
    pub(super) fn new(
        browser: &Rc<Browser>,
        source: Rc<dyn FileSource>,
        previews: Rc<Cell<bool>>,
        activation: Rc<Cell<ClickActivation>>,
        marks: Rc<RefCell<ClipboardMarks>>,
    ) -> Self {
        let initial_show_hidden = browser
            .active_depth()
            .and_then(|depth| browser.column_preferences(depth))
            .map_or_else(
                || browser.preferences().show_hidden,
                |prefs| prefs.show_hidden,
            );
        let show_hidden = Rc::new(Cell::new(initial_show_hidden));
        let filter_query = Rc::new(RefCell::new(String::new()));
        let filter = tree_filter(show_hidden.clone(), filter_query.clone());

        let root_store = gtk::StringList::new(&[]);
        let root_index = super::SourceIndexMap::watch(&root_store);
        let root_filtered =
            gtk::FilterListModel::new(Some(root_store.clone()), Some(filter.clone()));

        let columns = super::ListColumnLayout::new(browser);
        let (headings, sorting) = super::list_headings(browser, 0, columns.clone());
        headings.set_hexpand(true);

        let inner = Rc::new_cyclic(|weak: &Weak<TreeInner>| {
            let bound_items: Rc<RefCell<Vec<BoundModeItem>>> = Rc::new(RefCell::new(Vec::new()));
            let create_child = {
                let weak = weak.clone();
                move |item: &glib::Object| {
                    weak.upgrade()
                        .and_then(|inner| TreePane { inner }.child_model_for(item))
                }
            };
            let tree_model =
                gtk::TreeListModel::new(root_filtered.clone(), false, false, create_child);
            let selection = gtk::MultiSelection::new(Some(tree_model.clone()));
            let view =
                gtk::ListView::new(Some(selection.clone()), None::<gtk::SignalListItemFactory>);
            view.add_css_class("file-tree-mode");
            view.add_css_class("file-list-mode");
            view.set_single_click_activate(false);
            view.set_show_separators(false);

            let factory = TreeRowFactory::build(weak.clone(), bound_items.clone(), columns.clone());
            view.set_factory(Some(&factory));
            super::install_edit_unbind(&factory, &bound_items);
            super::super::accessibility::describe_entry_container(&view, "");

            let scroll = gtk::ScrolledWindow::builder()
                .child(&view)
                .hscrollbar_policy(gtk::PolicyType::Automatic)
                .vscrollbar_policy(gtk::PolicyType::Automatic)
                .hexpand(true)
                .vexpand(true)
                .build();
            scroll.add_css_class("mode-scroll");
            scroll.add_css_class("browser-listing-scroll");
            scroll.add_css_class("list-listing-scroll");
            let status = gtk::Label::new(None);
            status.add_css_class("tree-status");
            status.set_visible(false);
            status.set_hexpand(true);
            status.set_vexpand(true);
            let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
            page.add_css_class("mode-tree");
            page.add_css_class("mode-list");
            page.set_hexpand(true);
            page.set_vexpand(true);
            page.append(&headings);
            page.append(&scroll);
            page.append(&status);

            TreeInner {
                page,
                status,
                view,
                selection,
                tree_model,
                _columns: columns,
                _sorting: sorting,
                browser: Rc::downgrade(browser),
                source,
                state: RefCell::new(Weak::new()),
                previews,
                activation,
                marks,
                root_store,
                root_index,
                root_location: RefCell::new(None),
                entries: RefCell::new(HashMap::new()),
                locations: RefCell::new(HashMap::new()),
                directory_items: RefCell::new(HashMap::new()),
                item_levels: RefCell::new(HashMap::new()),
                branches: RefCell::new(HashMap::new()),
                expanded: RefCell::new(HashSet::new()),
                visual_anchor: RefCell::new(None),
                focused_item: RefCell::new(None),
                show_hidden,
                filter,
                filter_query,
                bound_items,
                syncing: Rc::new(Cell::new(false)),
                next_branch_id: Cell::new(0),
                menus_installed: Cell::new(false),
                item_trigger: RefCell::new(None),
                fill_requested: RefCell::new(HashSet::new()),
                selection_handlers: RefCell::new(Vec::new()),
                focus_sync_scheduled: Cell::new(false),
            }
        });

        {
            let weak = Rc::downgrade(&inner);
            inner.selection.connect_selection_changed(move |_, _, _| {
                if let Some(inner) = weak.upgrade() {
                    TreePane { inner }.on_selection_changed();
                }
            });
        }
        TreeRowInteractions::install(&inner);

        Self { inner }
    }

    pub(super) fn widget(&self) -> gtk::Widget {
        self.inner.page.clone().upcast()
    }

    pub(super) fn set_state(&self, state: Weak<ViewState>) {
        self.inner.state.replace(state);
    }

    fn browser(&self) -> Option<Rc<Browser>> {
        self.inner.browser.upgrade()
    }

    /// Entries for the current GTK selection, falling back to the focused
    /// cursor row. Mirrors `command_entries` fill-or-cursor semantics so
    /// 10xer verbs never silently operate on an unrelated column.
    pub(super) fn command_entries(&self) -> Vec<FileEntry> {
        let selected = self.selected_entries();
        if selected.is_empty() {
            return self.focused_entry().into_iter().collect();
        }
        selected
    }

    fn entry_for_item(&self, item: &glib::Object) -> Option<Rc<FileEntry>> {
        if let Some(entry) = self.inner.entries.borrow().get(item) {
            return Some(entry.clone());
        }
        let location = item_location(item)?;
        let entry = self.inner.locations.borrow().get(&location).cloned()?;
        if let Ok(mut known) = self.inner.entries.try_borrow_mut() {
            known.insert(item.clone(), entry.clone());
        }
        Some(entry)
    }

    pub(super) fn selected_entries(&self) -> Vec<FileEntry> {
        super::bitset_positions(&self.inner.selection.selection())
            .into_iter()
            .filter_map(|position| self.row_at(position as u32))
            .filter_map(|row| row.item())
            .filter_map(|item| self.entry_for_item(&item).map(|entry| (*entry).clone()))
            .collect()
    }

    pub(super) fn focused_entry(&self) -> Option<FileEntry> {
        let item = self.focused_object()?;
        self.entry_for_item(&item).map(|entry| (*entry).clone())
    }

    fn focused_object(&self) -> Option<glib::Object> {
        self.inner.focused_item.borrow().clone()
    }

    fn row_at(&self, position: u32) -> Option<gtk::TreeListRow> {
        self.inner.tree_model.item(position)?.downcast().ok()
    }

    /// Full root rebuild, e.g. after navigation or reload. Expansion survives
    /// through the `expanded` location set.
    pub(super) fn set_root(&self, location: Option<Location>, entries: Vec<Rc<FileEntry>>) {
        let changed = self.inner.root_location.borrow().clone() != location;
        if changed {
            self.inner.root_location.replace(location.clone());
            self.prune_expansion();
            self.clear_branches();
            self.inner.focused_item.replace(None);
            self.inner.fill_requested.borrow_mut().clear();
        }
        // Rebuilds mint new model items; keep the cursor on its location.
        let focused_location = self.inner.focused_item.borrow().as_ref().and_then(|item| {
            self.inner
                .entries
                .borrow()
                .get(item)
                .map(|entry| entry.location.clone())
        });
        self.inner.syncing.set(true);
        self.replace_level_items(None, &entries);
        self.inner.syncing.set(false);
        if !changed && let Some(location) = focused_location {
            let item = self
                .inner
                .entries
                .borrow()
                .iter()
                .find(|(_, entry)| entry.location == location)
                .map(|(item, _)| item.clone());
            self.inner.focused_item.replace(item);
        }
        self.restore_expansion();
        self.update_status();
        let title = self
            .inner
            .root_location
            .borrow()
            .as_ref()
            .map(|location| location.display_name())
            .unwrap_or_default();
        super::super::accessibility::describe_pane(
            &self.inner.page,
            &title,
            super::BrowserMode::Tree,
        );
    }

    fn prune_expansion(&self) {
        let root = self.inner.root_location.borrow().clone();
        self.inner.expanded.borrow_mut().retain(|location| {
            root.as_ref()
                .is_some_and(|root| location == root || is_descendant(location, root))
        });
    }

    fn clear_branches(&self) {
        self.collapse_all();
        self.inner.branches.borrow_mut().clear();
        self.inner.locations.borrow_mut().clear();
        self.inner.visual_anchor.replace(None);
    }

    fn collapse_all(&self) {
        for position in 0..self.inner.tree_model.n_items() {
            if let Some(row) = self.row_at(position)
                && row.is_expanded()
            {
                row.set_expanded(false);
            }
        }
    }

    fn restore_expansion(&self) {
        let wanted: Vec<Location> = self.inner.expanded.borrow().iter().cloned().collect();
        if wanted.is_empty() {
            return;
        }
        for _ in 0..32 {
            let mut expanded_any = false;
            let directory_items = self.inner.directory_items.borrow().clone();
            for location in &wanted {
                if self.inner.expanded.borrow().contains(location)
                    && let Some(item) = directory_items.get(location)
                    && let Some(row) = self.row_for_item(item)
                    && !row.is_expanded()
                {
                    row.set_expanded(true);
                    expanded_any = true;
                }
            }
            if !expanded_any {
                break;
            }
        }
    }

    fn row_for_item(&self, item: &glib::Object) -> Option<gtk::TreeListRow> {
        for position in 0..self.inner.tree_model.n_items() {
            if let Some(row) = self.row_at(position)
                && row.item().as_ref() == Some(item)
            {
                return Some(row);
            }
        }
        None
    }

    /// Replaces every item of a level with freshly sorted entries.
    fn replace_level_items(&self, level: Option<Location>, entries: &[Rc<FileEntry>]) {
        let values: Vec<String> = entries
            .iter()
            .map(|entry| tree_model_value(entry))
            .collect();
        let store = match level.clone() {
            None => Some(self.inner.root_store.clone()),
            Some(location) => self
                .inner
                .branches
                .borrow()
                .get(&location)
                .map(|branch| branch.store.clone()),
        };
        let Some(store) = store else {
            return;
        };
        let old: Vec<glib::Object> = (0..store.n_items()).filter_map(|i| store.item(i)).collect();
        {
            let mut known = self.inner.entries.borrow_mut();
            let mut levels = self.inner.item_levels.borrow_mut();
            let mut directories = self.inner.directory_items.borrow_mut();
            let mut locations = self.inner.locations.borrow_mut();
            for item in &old {
                if let Some(entry) = known.remove(item) {
                    locations.remove(&entry.location);
                }
                levels.remove(item);
                directories.retain(|_, known| known != item);
            }
            for entry in entries {
                locations.insert(entry.location.clone(), entry.clone());
            }
        }
        let refs: Vec<&str> = values.iter().map(String::as_str).collect();
        store.splice(0, store.n_items(), &refs);
        {
            let mut known = self.inner.entries.borrow_mut();
            let mut levels = self.inner.item_levels.borrow_mut();
            let mut directories = self.inner.directory_items.borrow_mut();
            for (position, entry) in entries.iter().enumerate() {
                if let Some(item) = store.item(position as u32) {
                    if entry.is_directory() {
                        directories.insert(entry.location.clone(), item.clone());
                    }
                    levels.insert(item.clone(), level.clone());
                    known.insert(item, entry.clone());
                }
            }
        }
        self.rebind_items();
    }

    pub(super) fn insert_root(&self, position: usize, entries: Vec<Rc<FileEntry>>) {
        let values: Vec<String> = entries
            .iter()
            .map(|entry| tree_model_value(entry))
            .collect();
        {
            let mut locations = self.inner.locations.borrow_mut();
            for entry in &entries {
                locations.insert(entry.location.clone(), entry.clone());
            }
        }
        let refs: Vec<&str> = values.iter().map(String::as_str).collect();
        self.inner.root_store.splice(position as u32, 0, &refs);
        {
            let mut known = self.inner.entries.borrow_mut();
            let mut levels = self.inner.item_levels.borrow_mut();
            let mut directories = self.inner.directory_items.borrow_mut();
            for (offset, entry) in entries.iter().enumerate() {
                if let Some(item) = self.inner.root_store.item((position + offset) as u32) {
                    if entry.is_directory() {
                        directories.insert(entry.location.clone(), item.clone());
                    }
                    levels.insert(item.clone(), None);
                    known.insert(item, entry.clone());
                }
            }
        }
        self.rebind_items();
        self.update_status();
    }

    pub(super) fn splice_root(&self, position: usize, removed: usize, entries: Vec<Rc<FileEntry>>) {
        let values: Vec<String> = entries
            .iter()
            .map(|entry| tree_model_value(entry))
            .collect();
        let refs: Vec<&str> = values.iter().map(String::as_str).collect();
        let old: Vec<glib::Object> = (position as u32..(position + removed) as u32)
            .filter_map(|i| self.inner.root_store.item(i))
            .collect();
        {
            let mut known = self.inner.entries.borrow_mut();
            let mut levels = self.inner.item_levels.borrow_mut();
            let mut directories = self.inner.directory_items.borrow_mut();
            let mut locations = self.inner.locations.borrow_mut();
            for item in &old {
                if let Some(entry) = known.remove(item) {
                    locations.remove(&entry.location);
                }
                levels.remove(item);
                directories.retain(|_, known| known != item);
            }
            for entry in &entries {
                locations.insert(entry.location.clone(), entry.clone());
            }
        }
        self.inner
            .root_store
            .splice(position as u32, removed as u32, &refs);
        {
            let mut known = self.inner.entries.borrow_mut();
            let mut levels = self.inner.item_levels.borrow_mut();
            let mut directories = self.inner.directory_items.borrow_mut();
            for (offset, entry) in entries.iter().enumerate() {
                if let Some(item) = self.inner.root_store.item((position + offset) as u32) {
                    if entry.is_directory() {
                        directories.insert(entry.location.clone(), item.clone());
                    }
                    levels.insert(item.clone(), None);
                    known.insert(item, entry.clone());
                }
            }
        }
        self.rebind_items();
        self.update_status();
    }

    pub(super) fn update_status(&self) {
        let empty = self.inner.root_store.n_items() == 0;
        let has_root = self.inner.root_location.borrow().is_some();
        self.inner.status.set_visible(has_root && empty);
        if has_root && empty {
            self.inner.status.set_label("This folder is empty");
        }
    }

    /// Replaces the whole root from the application model, e.g. after a
    /// reload or sort. Expansion survives by location.
    pub(super) fn replace_root_from_browser(&self, browser: &Browser, count: usize) {
        let entries = browser
            .with_entries(0, 0..count, |entries| {
                entries
                    .iter()
                    .map(|entry| Rc::new(entry.clone()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let location = browser.location_at(0);
        self.set_root(location, entries);
    }

    pub(super) fn load_failed(&self, message: &str) {
        self.inner
            .status
            .set_label(&format!("Unable to read this directory\n{message}"));
        self.inner.status.set_visible(true);
    }

    /// Submits a visible root row for metadata fill through the column
    /// pipeline. Branches stat inline at enumeration; only the mirrored root
    /// needs fills.
    fn request_fill_for(&self, entry: &FileEntry) {
        let Some(browser) = self.browser() else {
            return;
        };
        let Some(child) = self
            .inner
            .entries
            .borrow()
            .iter()
            .find(|(_, known)| known.location == entry.location)
            .map(|(item, _)| item.clone())
        else {
            return;
        };
        let at_root = self
            .inner
            .item_levels
            .borrow()
            .get(&child)
            .is_some_and(|level| level.is_none());
        if !at_root {
            return;
        }
        if !self
            .inner
            .fill_requested
            .borrow_mut()
            .insert(entry.location.clone())
        {
            return;
        }
        if let Some(position) = self.inner.root_index.of_item(&child) {
            browser.request_metadata_fill(0, position, entry.location.clone(), false);
        }
    }

    /// Applies column metadata fills to mirrored root rows and refreshes
    /// their visible labels.
    pub(super) fn apply_metadata(&self, updates: &[(usize, FileEntry)]) {
        if updates.is_empty() {
            return;
        }
        let fresh: HashMap<glib::Object, Rc<FileEntry>> = updates
            .iter()
            .filter_map(|(position, entry)| {
                let item = self.inner.root_store.item(*position as u32)?;
                Some((item, Rc::new(entry.clone())))
            })
            .collect();
        if fresh.is_empty() {
            return;
        }
        {
            let mut known = self.inner.entries.borrow_mut();
            for (item, entry) in &fresh {
                known.insert(item.clone(), entry.clone());
            }
        }
        for bound in self.inner.bound_items.borrow().iter() {
            let Some(item) = bound.item.upgrade() else {
                continue;
            };
            let Some(child) = item
                .item()
                .and_downcast::<gtk::TreeListRow>()
                .and_then(|treerow| treerow.item())
            else {
                continue;
            };
            let Some(entry) = fresh.get(&child) else {
                continue;
            };
            let Some(widget) = bound.widget.upgrade().and_downcast::<gtk::Box>() else {
                continue;
            };
            let Some(row) = widget
                .parent()
                .and_downcast::<gtk::TreeExpander>()
                .and_then(TreeRowWidgets::from_expander)
            else {
                continue;
            };
            set_label_if_changed(&row.size, &super::entry_size(entry));
            set_label_if_changed(&row.modified, &crate::util::modified_date(entry));
        }
    }

    /// Footer/status refresh hook for tree selection changes.
    pub(super) fn connect_selection_changed(&self, handler: Rc<dyn Fn()>) {
        self.inner.selection_handlers.borrow_mut().push(handler);
    }

    /// Child model for an expandable row. Directories resolve to their branch
    /// level, created and enumerated on first expansion.
    fn child_model_for(&self, item: &glib::Object) -> Option<gio::ListModel> {
        let entry = self.entry_for_item(item)?;
        if !entry.is_directory() {
            return None;
        }
        let location = entry.location.clone();
        if !self.inner.branches.borrow().contains_key(&location) {
            self.start_branch_load(&location);
        }
        self.inner
            .branches
            .borrow()
            .get(&location)
            .map(|branch| branch.filtered.clone().upcast())
    }

    fn branch_preferences(&self) -> ViewPreferences {
        self.browser()
            .and_then(|browser| browser.column_preferences(0))
            .unwrap_or_default()
    }

    fn start_branch_load(&self, location: &Location) {
        let store = gtk::StringList::new(&[]);
        let filtered =
            gtk::FilterListModel::new(Some(store.clone()), Some(self.inner.filter.clone()));
        self.inner.branches.borrow_mut().insert(
            location.clone(),
            TreeLevel {
                store: store.clone(),
                filtered,
                load: None,
                watch: None,
                loaded: false,
            },
        );
        let watch = {
            let weak = Rc::downgrade(&self.inner);
            let watched = location.clone();
            let show_hidden = self.inner.show_hidden.get();
            self.inner.source.watch(
                location.clone(),
                show_hidden,
                Rc::new(move |change| {
                    if let Some(inner) = weak.upgrade() {
                        TreePane { inner }.apply_branch_change(&watched, change);
                    }
                }),
            )
        };
        if let Some(level) = self.inner.branches.borrow_mut().get_mut(location) {
            level.watch = watch;
        }
        let Some(browser) = self.browser() else {
            return;
        };
        // Branches sort and display on their own values; stat inline so rows
        // never show blank sizes waiting on a fill the column model will not
        // run for unloaded directories.
        let id = self.inner.next_branch_id.get();
        self.inner.next_branch_id.set(id.saturating_add(1));
        let request_id = RequestId(u64::MAX.saturating_sub(id));
        let weak = Rc::downgrade(&self.inner);
        let branch_location = location.clone();
        let batch_size = if location.native_path().is_some() {
            NATIVE_BRANCH_BATCH_SIZE
        } else {
            REMOTE_BRANCH_BATCH_SIZE
        };
        let load = self.inner.source.enumerate(
            DirectoryRequest {
                id: request_id,
                location: location.clone(),
                batch_size,
                include_metadata: true,
                max_entries: MAX_BRANCH_ENTRIES,
                time_budget: BRANCH_LOAD_TIME_BUDGET,
            },
            Rc::new(move |event| {
                if let Some(inner) = weak.upgrade() {
                    TreePane { inner }.handle_branch_event(
                        &branch_location,
                        request_id,
                        &event,
                        &browser,
                    );
                }
            }),
        );
        if let Some(level) = self.inner.branches.borrow_mut().get_mut(location) {
            level.load = Some(load);
        }
    }

    fn handle_branch_event(
        &self,
        location: &Location,
        request_id: RequestId,
        event: &DirectoryEvent,
        browser: &Browser,
    ) {
        let _ = browser;
        match event {
            DirectoryEvent::Batch {
                request_id: id,
                entries,
            } if *id == request_id => {
                self.merge_branch_batch(location, entries);
            }
            DirectoryEvent::Finished { request_id: id, .. } if *id == request_id => {
                if let Some(level) = self.inner.branches.borrow_mut().get_mut(location) {
                    level.load = None;
                    level.loaded = true;
                }
            }
            DirectoryEvent::Failed { request_id: id, .. } if *id == request_id => {
                if let Some(level) = self.inner.branches.borrow_mut().get_mut(location) {
                    level.load = None;
                    level.loaded = true;
                }
            }
            _ => {}
        }
    }

    fn merge_branch_batch(&self, location: &Location, batch: &[FileEntry]) {
        let mut merged: Vec<FileEntry> = self.level_entries(location);
        merged.extend(batch.iter().cloned());
        self.set_branch_entries(location, merged);
    }

    fn level_entries(&self, location: &Location) -> Vec<FileEntry> {
        self.inner
            .entries
            .borrow()
            .iter()
            .filter(|(item, _)| {
                self.inner
                    .item_levels
                    .borrow()
                    .get(*item)
                    .is_some_and(|level| level.as_ref() == Some(location))
            })
            .map(|(_, entry)| (**entry).clone())
            .collect()
    }

    fn set_branch_entries(&self, location: &Location, entries: Vec<FileEntry>) {
        if !self.inner.branches.borrow().contains_key(location) {
            return;
        }
        let deduped = dedupe_by_location(entries.into_iter().map(Rc::new).collect());
        let sorted = sort_entries(
            deduped.into_iter().map(|entry| (*entry).clone()).collect(),
            self.branch_preferences(),
        );
        let sorted: Vec<Rc<FileEntry>> = sorted.into_iter().map(Rc::new).collect();
        self.replace_level_items(Some(location.clone()), &sorted);
    }

    fn apply_branch_change(&self, location: &Location, change: crate::services::DirectoryChange) {
        use crate::services::DirectoryChange;
        if !self.inner.branches.borrow().contains_key(location) {
            return;
        }
        match change {
            DirectoryChange::Rescan => {
                self.restart_branch_load(location);
            }
            DirectoryChange::Upsert(entry) => {
                let mut entries = self.level_entries(location);
                if let Some(slot) = entries
                    .iter_mut()
                    .find(|known| known.location == entry.location)
                {
                    *slot = entry;
                } else {
                    entries.push(entry);
                }
                self.set_branch_entries(location, entries);
            }
            DirectoryChange::Remove(removed) => {
                let mut entries = self.level_entries(location);
                entries.retain(|known| known.location != removed);
                self.collapse_branch_row(location);
                self.set_branch_entries(location, entries);
            }
            DirectoryChange::Move { from, entry } => {
                let mut entries = self.level_entries(location);
                entries.retain(|known| known.location != from);
                if let Some(slot) = entries
                    .iter_mut()
                    .find(|known| known.location == entry.location)
                {
                    *slot = entry.clone();
                } else {
                    entries.push(entry.clone());
                }
                self.set_branch_entries(location, entries);
            }
        }
    }

    fn collapse_branch_row(&self, location: &Location) {
        let item = self.inner.directory_items.borrow().get(location).cloned();
        if let Some(item) = item
            && let Some(row) = self.row_for_item(&item)
            && row.is_expanded()
        {
            row.set_expanded(false);
        }
        self.inner.expanded.borrow_mut().remove(location);
    }

    fn restart_branch_load(&self, location: &Location) {
        self.collapse_branch_row(location);
        self.inner.branches.borrow_mut().remove(location);
        self.start_branch_load(location);
        self.restore_expansion();
    }

    /// Re-sorts every loaded branch after the sort preferences change. The
    /// root order arrives through Browser row events.
    pub(super) fn resort_branches(&self) {
        let locations: Vec<Location> = self.inner.branches.borrow().keys().cloned().collect();
        for location in locations {
            let current = self.level_entries(&location);
            if current.is_empty() {
                continue;
            }
            let sorted = sort_entries(current, self.branch_preferences());
            let sorted: Vec<Rc<FileEntry>> = sorted.into_iter().map(Rc::new).collect();
            self.replace_level_items(Some(location), &sorted);
        }
        self.inner.filter.changed(gtk::FilterChange::Different);
    }

    pub(super) fn set_show_hidden(&self, show_hidden: bool) {
        if self.inner.show_hidden.get() == show_hidden {
            return;
        }
        self.inner.show_hidden.set(show_hidden);
        self.resort_branches();
    }
}

fn is_descendant(location: &Location, ancestor: &Location) -> bool {
    let mut current = location.parent();
    while let Some(parent) = current {
        if &parent == ancestor {
            return true;
        }
        current = parent.parent();
    }
    false
}

fn dedupe_by_location(entries: Vec<Rc<FileEntry>>) -> Vec<Rc<FileEntry>> {
    let mut seen = HashSet::new();
    let mut deduped = Vec::with_capacity(entries.len());
    for entry in entries {
        if seen.insert(entry.location.clone()) {
            deduped.push(entry);
        }
    }
    deduped
}

struct TreeRowWidgets {
    expander: gtk::TreeExpander,
    row: gtk::Box,
    _name_cell: gtk::Widget,
    icon: crate::ui::thumbnail::ThumbnailSlot,
    name: gtk::Label,
    field: gtk::Entry,
    mode: gtk::Label,
    size: gtk::Label,
    kind: gtk::Label,
    modified: gtk::Label,
}

struct TreeRowFactory {
    inner: Weak<TreeInner>,
    bound_items: Rc<RefCell<Vec<BoundModeItem>>>,
    columns: super::ListColumnLayout,
}

impl TreeRowFactory {
    fn build(
        inner: Weak<TreeInner>,
        bound_items: Rc<RefCell<Vec<BoundModeItem>>>,
        columns: super::ListColumnLayout,
    ) -> gtk::SignalListItemFactory {
        let context = Rc::new(Self {
            inner,
            bound_items,
            columns,
        });
        let factory = gtk::SignalListItemFactory::new();
        let setup = context.clone();
        factory.connect_setup(move |_, item| setup.setup(item));
        let bind = context.clone();
        factory.connect_bind(move |_, item| bind.bind(item));
        factory
    }

    fn setup(&self, object: &glib::Object) {
        let Some(item) = object.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let expander = gtk::TreeExpander::new();
        expander.set_hexpand(true);

        let row = super::assemble_list_row();
        row.add_css_class("tree-row");
        row.set_hexpand(true);

        let Some((_icon, name, field, mode, size, kind, modified)) = super::list_row_parts(&row)
        else {
            return;
        };
        let name_cell = row.first_child().expect("name cell");
        for (index, widget) in [
            name_cell.clone(),
            mode.clone().upcast(),
            size.clone().upcast(),
            kind.clone().upcast(),
            modified.clone().upcast(),
        ]
        .into_iter()
        .enumerate()
        {
            super::register_list_column_cell(&self.columns, index, &widget);
        }

        expander.set_child(Some(&row));
        item.set_child(Some(&expander));

        let edit = collection_edit::EditWidgets::new(&field, &name);
        register_bound_mode_item(&self.bound_items, item, &row, &name, edit);
        TreeRowInteractions::install_row(&self.inner, item, &row);
    }

    fn bind(&self, object: &glib::Object) {
        let Some(item) = object.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(inner) = self.inner.upgrade() else {
            return;
        };
        TreePane { inner }.bind_list_item(item);
    }
}

impl TreePane {
    fn bind_list_item(&self, item: &gtk::ListItem) {
        let Some(row) = item
            .child()
            .and_downcast::<gtk::TreeExpander>()
            .and_then(TreeRowWidgets::from_expander)
        else {
            return;
        };
        let treerow = item.item().and_downcast::<gtk::TreeListRow>();
        row.expander.set_list_row(treerow.as_ref());
        let entry = treerow
            .as_ref()
            .and_then(|treerow| treerow.item())
            .and_then(|child| self.entry_for_item(&child));
        let Some(entry) = entry else {
            row.clear();
            return;
        };
        if metadata_needs_fill(&entry) {
            self.request_fill_for(&entry);
        }
        row.expander.set_hide_expander(!entry.is_directory());
        set_label_if_changed(&row.name, &entry.display_name);
        row.name
            .set_opacity(if entry.is_hidden { 0.65 } else { 1.0 });
        crate::ui::thumbnail::set_thumbnail_or_icon(&row.icon, &entry, entry_icon(&entry), 18, 18);
        row.icon.set_hidden(entry.is_hidden);
        row.icon
            .set_base_opacity(if entry.is_directory() { 1.0 } else { 0.72 });
        set_label_if_changed(&row.mode, &super::entry_mode(&entry));
        set_label_if_changed(&row.size, &super::entry_size(&entry));
        set_label_if_changed(&row.kind, &super::entry_type(&entry));
        set_label_if_changed(&row.modified, &crate::util::modified_date(&entry));
        let state = self.inner.state.borrow().upgrade();
        let pending_name = state
            .as_ref()
            .and_then(|state| state.pending_rename_name(&entry));
        if let Some(pending) = pending_name.as_deref() {
            row.name.set_label(pending);
        }
        let edit = super::bound_edit(&self.inner.bound_items, item);
        if let Some(edit) = &edit {
            edit.bind(&entry.location);
            edit.display.set_visible(!edit.is_editing());
            edit.field.set_visible(edit.is_editing());
        }
        accessibility::describe_entry(item, &entry.display_name, Some(&entry));
        let mark = mark_in(&self.inner.marks.borrow(), &entry.location);
        super::set_mode_mark_style(&row.row, mark);
        if let Some(state) = state {
            browser::find::highlight_listing_name(
                row.name.upcast_ref(),
                state.find_highlight().as_deref(),
                &self.inner.filter_query.borrow(),
            );
        }
    }

    fn rebind_items(&self) {
        for bound in self.inner.bound_items.borrow().iter() {
            if let Some(item) = bound.item.upgrade() {
                self.bind_list_item(&item);
            }
        }
    }
}

impl TreeRowWidgets {
    fn from_expander(expander: gtk::TreeExpander) -> Option<Self> {
        let row = expander.child().and_downcast::<gtk::Box>()?;
        let (icon, name, field, mode, size, kind, modified) = super::list_row_parts(&row)?;
        let name_cell = row.first_child()?;
        Some(Self {
            expander,
            row,
            _name_cell: name_cell,
            icon,
            name,
            field,
            mode,
            size,
            kind,
            modified,
        })
    }
    fn clear(&self) {
        self.expander.set_list_row(None);
        super::set_mode_mark_style(&self.row, ClipboardMark::None);
        crate::ui::thumbnail::show_fallback_icon(&self.icon, crate::assets::icons::DOCUMENTS, 18);
        self.icon.set_hidden(false);
        self.icon.set_base_opacity(1.0);
        self.name.set_label("");
        self.name.set_opacity(1.0);
        self.mode.set_label("");
        self.size.set_label("");
        self.kind.set_label("");
        self.modified.set_label("");
        self.field.set_visible(false);
    }
}

impl TreePane {
    /// Single-click activation honoring the tree's click preferences.
    /// Defaults to double activation like the list; single-click folders
    /// navigate, single-click files launch.
    fn single_click_activates(&self, entry: &FileEntry) -> bool {
        use super::ClickCount;
        let activation = self.inner.activation.get();
        let clicks = if entry.is_directory() {
            activation.folders
        } else {
            activation.files
        };
        clicks == ClickCount::One
    }

    /// Location of the directory containing the focused row: the paste and
    /// create destination for tree verbs.
    pub(super) fn focused_directory(&self) -> Option<Location> {
        let item = self.focused_object()?;
        let entry = self.entry_for_item(&item)?;
        if entry.is_directory() {
            return Some(entry.location.clone());
        }
        self.parent_location_of(&item)
    }

    fn parent_location_of(&self, item: &glib::Object) -> Option<Location> {
        self.inner
            .item_levels
            .borrow()
            .get(item)
            .cloned()
            .flatten()
            .or_else(|| self.inner.root_location.borrow().clone())
    }

    fn on_selection_changed(&self) {
        if self.inner.syncing.get() {
            return;
        }
        // Keep the sticky cursor inside the selection when the user moves
        // natively; verbs read the selection first, so the cursor only backs
        // the empty-selection case.
        let focused_inside = self
            .inner
            .focused_item
            .borrow()
            .as_ref()
            .and_then(|item| self.view_position_of(item))
            .is_some_and(|position| self.inner.selection.is_selected(position));
        if !focused_inside {
            let first = super::bitset_positions(&self.inner.selection.selection())
                .into_iter()
                .filter_map(|position| self.row_at(position as u32)?.item())
                .next();
            self.inner.focused_item.replace(first);
        }
        self.schedule_focus_sync();
        for handler in self.inner.selection_handlers.borrow().iter() {
            handler();
        }
    }

    /// Emits into Browser only from idle. Selection changes can fire while
    /// callers still hold mode_views borrows, and ViewState::handle takes
    /// borrow_mut, so a synchronous emit here would panic on reentry.
    fn schedule_focus_sync(&self) {
        if self.inner.focus_sync_scheduled.replace(true) {
            return;
        }
        let weak = Rc::downgrade(&self.inner);
        glib::idle_add_local_once(move || {
            let Some(inner) = weak.upgrade() else {
                return;
            };
            inner.focus_sync_scheduled.set(false);
            let pane = TreePane { inner };
            let item = pane.inner.focused_item.borrow().clone();
            let entry = item.as_ref().and_then(|item| pane.entry_for_item(item));
            if let (Some(item), Some(entry)) = (item, entry) {
                pane.preview_for_focus(&item, &entry);
            }
        });
    }

    fn preview_for_focus(&self, item: &glib::Object, entry: &FileEntry) {
        let Some(browser) = self.browser() else {
            return;
        };
        let at_root = self
            .inner
            .item_levels
            .borrow()
            .get(item)
            .is_some_and(|level| level.is_none());
        if at_root {
            if let Some(position) = self.inner.root_index.of_item(item) {
                let already = browser
                    .focused_item()
                    .is_some_and(|(depth, focused, _)| depth == 0 && focused == position);
                if !already {
                    browser.select(0, position);
                }
            }
            return;
        }
        browser.request_automatic_preview(entry.clone());
    }
}

impl TreePane {
    pub(super) fn item_view_has_focus(&self) -> bool {
        let focused = self.inner.page.root().and_then(|root| root.focus());
        super::widget_has_focus(&self.inner.view, focused.as_ref())
    }

    pub(super) fn focus_view(&self) -> bool {
        self.inner.view.grab_focus()
    }

    pub(super) fn rename_view(&self) -> gtk::Widget {
        self.inner.view.clone().upcast()
    }

    pub(super) fn bound_items(&self) -> Rc<RefCell<Vec<BoundModeItem>>> {
        self.inner.bound_items.clone()
    }

    pub(super) fn entry_for_location(&self, location: &Location) -> Option<FileEntry> {
        self.inner
            .locations
            .borrow()
            .get(location)
            .map(|entry| (**entry).clone())
            .or_else(|| {
                self.inner
                    .entries
                    .borrow()
                    .values()
                    .find(|entry| &entry.location == location)
                    .map(|entry| (**entry).clone())
            })
    }

    pub(super) fn entry_for_child(&self, child: &glib::Object) -> Option<FileEntry> {
        self.entry_for_item(child).map(|entry| (*entry).clone())
    }

    fn view_position_of(&self, item: &glib::Object) -> Option<u32> {
        for position in 0..self.inner.tree_model.n_items() {
            if let Some(row) = self.row_at(position)
                && row.item().as_ref() == Some(item)
            {
                return Some(position);
            }
        }
        None
    }

    /// Moves the cursor, preserving an existing multi-selection like the
    /// column cursor does. `steps == usize::MAX` jumps to an end.
    pub(super) fn move_focus(&self, direction: i32, steps: usize) -> bool {
        if direction == 0 {
            return false;
        }
        let count = self.inner.tree_model.n_items();
        if count == 0 {
            return false;
        }
        let last = count - 1;
        let current = self.focused_view_position();
        let target = if steps == usize::MAX {
            if direction < 0 { 0 } else { last }
        } else {
            let steps = steps.max(1) as u32;
            if direction < 0 {
                current.unwrap_or(last).saturating_sub(steps)
            } else {
                current
                    .map(|position| position.saturating_add(steps).min(last))
                    .unwrap_or(0)
            }
        };
        self.focus_position(target);
        true
    }

    fn focused_view_position(&self) -> Option<u32> {
        let item = self.inner.focused_item.borrow().clone()?;
        self.view_position_of(&item)
    }

    /// Flat view position of a root row by source position.
    pub(super) fn flat_position_for_source(&self, source: usize) -> Option<u32> {
        let item = self.inner.root_store.item(source as u32)?;
        self.view_position_of(&item)
    }

    /// Focuses a visible row and keeps it on screen. Selection bits are left
    /// alone so marked fills survive cursor moves; the `selected` cursor
    /// still follows focus for verb resolution.
    pub(super) fn focus_position(&self, position: u32) {
        if position >= self.inner.tree_model.n_items() {
            return;
        }
        if !self.inner.view.has_focus() {
            self.inner.view.grab_focus();
        }
        let tenxer = super::super::preferences::PreferenceManager::shared().tenxer_mode();
        if !tenxer {
            self.inner
                .view
                .activate_action(
                    "list.select-item",
                    Some(&(position, false, false).to_variant()),
                )
                .ok();
            self.inner
                .view
                .scroll_to(position, gtk::ListScrollFlags::SELECT, None);
        } else {
            self.inner
                .view
                .scroll_to(position, gtk::ListScrollFlags::FOCUS, None);
        }
        self.extend_visual_to(position);
        if let Some(row) = self.row_at(position)
            && let Some(item) = row.item()
        {
            self.inner.focused_item.replace(Some(item));
            self.schedule_focus_sync();
        }
    }

    /// Adds or removes a single visible row without disturbing the rest.
    fn set_bit(&self, position: u32, select: bool) {
        let mask = gtk::Bitset::new_range(position, 1);
        let selected = gtk::Bitset::new_empty();
        if select {
            selected.add(position);
        }
        self.inner.syncing.set(true);
        self.inner.selection.set_selection(&selected, &mask);
        self.inner.syncing.set(false);
    }

    /// Synchronizes visible selection bits with an external source-position
    /// set (root-store indices). Nested rows are left alone.
    pub(super) fn sync_root_selection(&self, positions: &[usize]) {
        let wanted: HashSet<u32> = positions
            .iter()
            .filter_map(|position| self.inner.root_store.item(*position as u32))
            .filter_map(|item| self.view_position_of(&item))
            .collect();
        self.inner.syncing.set(true);
        for position in 0..self.inner.tree_model.n_items() {
            let is_root = self
                .row_at(position)
                .and_then(|row| row.item())
                .is_some_and(|item| {
                    self.inner
                        .item_levels
                        .borrow()
                        .get(&item)
                        .is_some_and(|level| level.is_none())
                });
            if !is_root {
                continue;
            }
            if wanted.contains(&position) {
                self.set_bit(position, true);
            } else {
                self.set_bit(position, false);
            }
        }
        self.inner.syncing.set(false);
    }

    /// Focuses a root row by source position without stealing focus from
    /// outside the tree. Returns whether focus moved.
    pub(super) fn focus_source_row(&self, position: usize) -> bool {
        let Some(item) = self.inner.root_store.item(position as u32) else {
            return false;
        };
        if self.inner.focused_item.borrow().as_ref() == Some(&item) {
            return false;
        }
        let Some(view_position) = self.view_position_of(&item) else {
            return false;
        };
        self.focus_position(view_position);
        true
    }

    /// Expands the focused folder, or steps into its first child when already
    /// expanded. Returns false on files and empty folders.
    pub(super) fn expand_focused(&self) -> bool {
        let Some(item) = self.focused_object() else {
            return false;
        };
        let Some(row) = self.row_for_item(&item) else {
            return false;
        };
        if !row.is_expandable() {
            return false;
        }
        if !row.is_expanded() {
            self.capture_expansion();
            row.set_expanded(true);
            if let Some(entry) = self.entry_for_item(&item) {
                self.inner
                    .expanded
                    .borrow_mut()
                    .insert(entry.location.clone());
            }
            return true;
        }
        let position = self.view_position_of(&item);
        let depth = row.depth();
        if let Some(position) = position {
            for next in position + 1..self.inner.tree_model.n_items() {
                if let Some(next_row) = self.row_at(next) {
                    if next_row.depth() > depth {
                        self.focus_position(next);
                        return true;
                    }
                    break;
                }
            }
        }
        false
    }

    /// Collapses the focused folder, or steps out to its parent row.
    pub(super) fn collapse_focused(&self) -> bool {
        let Some(item) = self.focused_object() else {
            return false;
        };
        let Some(row) = self.row_for_item(&item) else {
            return false;
        };
        if row.is_expanded() {
            self.capture_expansion();
            row.set_expanded(false);
            if let Some(entry) = self.entry_for_item(&item) {
                self.inner.expanded.borrow_mut().remove(&entry.location);
            }
            return true;
        }
        let Some(parent) = row.parent() else {
            return false;
        };
        let Some(parent_item) = parent.item() else {
            return false;
        };
        let Some(position) = self.view_position_of(&parent_item) else {
            return false;
        };
        self.focus_position(position);
        true
    }

    /// Toggles expansion of the focused folder.
    pub(super) fn toggle_focused(&self) -> bool {
        let Some(item) = self.focused_object() else {
            return false;
        };
        let Some(entry) = self.entry_for_item(&item) else {
            return false;
        };
        let Some(row) = self.row_for_item(&item) else {
            return false;
        };
        if !row.is_expandable() {
            return false;
        }
        self.capture_expansion();
        row.set_expanded(!row.is_expanded());
        if row.is_expanded() {
            self.inner
                .expanded
                .borrow_mut()
                .insert(entry.location.clone());
        } else {
            self.inner.expanded.borrow_mut().remove(&entry.location);
        }
        true
    }

    fn capture_expansion(&self) {
        for position in 0..self.inner.tree_model.n_items() {
            if let Some(row) = self.row_at(position)
                && row.is_expanded()
                && let Some(item) = row.item()
                && let Some(entry) = self.entry_for_item(&item)
            {
                self.inner
                    .expanded
                    .borrow_mut()
                    .insert(entry.location.clone());
            }
        }
    }

    /// Opens the focused row: navigates into folders, launches files.
    pub(in crate::ui) fn activate_focused(&self) -> bool {
        let Some(entry) = self.focused_entry() else {
            return false;
        };
        let Some(browser) = self.browser() else {
            return false;
        };
        if entry.is_directory() {
            browser.navigate(entry.location.clone());
        } else {
            browser.open_location(entry.location.clone());
        }
        true
    }

    /// Opens the preview drawer for the focused file.
    pub(super) fn preview_focused(&self) -> bool {
        let Some(entry) = self.focused_entry() else {
            return false;
        };
        if entry.is_directory() {
            return false;
        }
        let Some(browser) = self.browser() else {
            return false;
        };
        browser.request_preview(entry);
        true
    }

    /// Toggles the focused row's mark and steps on, mirroring the 10xer
    /// cursor mark. Selection bits drive verb targets, so the advance keeps
    /// the new cursor unmarked until the next toggle.
    pub(super) fn toggle_focused_selection(&self) -> bool {
        let Some(position) = self.focused_view_position() else {
            return false;
        };
        self.set_bit(position, !self.inner.selection.is_selected(position));
        self.move_focus(1, 1);
        true
    }

    pub(super) fn select_all_visible(&self) -> bool {
        if self.inner.tree_model.n_items() == 0 {
            return false;
        }
        self.inner.syncing.set(true);
        self.inner.selection.select_all();
        self.inner.syncing.set(false);
        true
    }

    pub(super) fn invert_visible(&self) -> bool {
        let count = self.inner.tree_model.n_items();
        if count == 0 {
            return false;
        }
        let mut unselected = Vec::new();
        for position in 0..count {
            if !self.inner.selection.is_selected(position) {
                unselected.push(position);
            }
        }
        self.inner.syncing.set(true);
        let mask = gtk::Bitset::new_range(0, count);
        let selected = gtk::Bitset::new_empty();
        for position in unselected {
            selected.add(position);
        }
        self.inner.selection.set_selection(&selected, &mask);
        self.inner.syncing.set(false);
        true
    }

    /// 10xer visual ranges: anchor at the focused row, then extend the native
    /// selection to every cursor stop while anchored.
    pub(super) fn toggle_visual(&self) -> bool {
        if self.inner.visual_anchor.borrow().is_some() {
            self.inner.visual_anchor.replace(None);
            return true;
        }
        let Some(item) = self.focused_object() else {
            return false;
        };
        let Some(position) = self.view_position_of(&item) else {
            return false;
        };
        self.set_bit(position, true);
        self.inner.visual_anchor.replace(Some(item));
        true
    }

    pub(super) fn visual_active(&self) -> bool {
        self.inner.visual_anchor.borrow().is_some()
    }

    /// Starts a visual range at the focused row when none is active.
    pub(super) fn ensure_visual_anchor(&self) -> bool {
        if self.inner.visual_anchor.borrow().is_some() {
            return true;
        }
        let Some(item) = self.focused_object() else {
            return false;
        };
        let Some(position) = self.view_position_of(&item) else {
            return false;
        };
        self.set_bit(position, true);
        self.inner.visual_anchor.replace(Some(item));
        true
    }

    pub(super) fn clear_visual(&self) {
        self.inner.visual_anchor.replace(None);
    }

    /// Clears the visual anchor and every selected row. Reports whether
    /// anything was held, for Escape dismissal.
    pub(super) fn clear_selection(&self) -> bool {
        let had_visual = self.inner.visual_anchor.replace(None).is_some();
        let had_selection = !self.inner.selection.selection().is_empty();
        if had_selection {
            self.inner.syncing.set(true);
            self.inner.selection.unselect_all();
            self.inner.syncing.set(false);
        }
        if had_visual || had_selection {
            self.inner.focused_item.replace(None);
            true
        } else {
            false
        }
    }

    fn extend_visual_to(&self, position: u32) {
        let Some(anchor) = self.inner.visual_anchor.borrow().clone() else {
            return;
        };
        let Some(anchor_position) = self.view_position_of(&anchor) else {
            self.inner.visual_anchor.replace(None);
            return;
        };
        let (from, to) = if anchor_position <= position {
            (anchor_position, position)
        } else {
            (position, anchor_position)
        };
        self.inner.syncing.set(true);
        let mask = gtk::Bitset::new_range(from, to - from + 1);
        let selected = self.inner.selection.selection();
        for stop in from..=to {
            selected.add(stop);
        }
        self.inner.selection.set_selection(&selected, &mask);
        self.inner.syncing.set(false);
    }

    /// Cancels branch loads and drops branch levels. Expansion intent survives
    /// in the `expanded` set and re-enumerates on return.
    pub(super) fn deactivate(&self) {
        self.capture_expansion();
        self.collapse_all();
        self.inner.branches.borrow_mut().clear();
        self.inner.locations.borrow_mut().clear();
        self.inner.visual_anchor.replace(None);
        self.inner.focused_item.replace(None);
    }

    pub(super) fn set_filter_query(&self, query: String) {
        if self.inner.filter_query.borrow().as_str() == query {
            return;
        }
        self.inner.filter_query.replace(query);
        self.inner.filter.changed(gtk::FilterChange::Different);
    }

    pub(super) fn filter_query_text(&self) -> String {
        self.inner.filter_query.borrow().clone()
    }

    pub(super) fn refresh_marks(&self) {
        let marks = self.inner.marks.borrow().clone();
        for bound in self.inner.bound_items.borrow().iter() {
            let Some(item) = bound.item.upgrade() else {
                continue;
            };
            let Some(widget) = bound.widget.upgrade() else {
                continue;
            };
            let entry = item
                .item()
                .and_downcast::<gtk::TreeListRow>()
                .and_then(|row| row.item())
                .and_then(|child| self.entry_for_item(&child));
            if let Some(entry) = entry {
                super::set_mode_mark_style(&widget, mark_in(&marks, &entry.location));
            }
        }
    }
}

struct TreeRowInteractions;

impl TreeRowInteractions {
    fn install(inner: &Rc<TreeInner>) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(inner);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let Some(inner) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if modifiers.intersects(
                gtk::gdk::ModifierType::CONTROL_MASK
                    | gtk::gdk::ModifierType::ALT_MASK
                    | gtk::gdk::ModifierType::SUPER_MASK,
            ) {
                return glib::Propagation::Proceed;
            }
            if inner
                .view
                .root()
                .and_then(|root| root.focus())
                .is_some_and(|focused| super::super::focus_navigation::editable(&focused))
            {
                return glib::Propagation::Proceed;
            }
            let pane = TreePane { inner };
            let handled = match key {
                gtk::gdk::Key::Right | gtk::gdk::Key::KP_Right => pane.expand_focused(),
                gtk::gdk::Key::Left | gtk::gdk::Key::KP_Left => pane.collapse_focused(),
                gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter => pane.activate_focused(),
                gtk::gdk::Key::space => {
                    if pane
                        .focused_entry()
                        .is_some_and(|entry| entry.is_directory())
                    {
                        pane.toggle_focused()
                    } else {
                        pane.preview_focused()
                    }
                }
                _ => false,
            };
            if handled {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        inner.view.add_controller(keys);
    }

    fn install_row(inner: &Weak<TreeInner>, item: &gtk::ListItem, row_box: &gtk::Box) {
        let click = gtk::GestureClick::new();
        click.set_button(1);
        let weak = inner.clone();
        let item = item.clone();
        click.connect_pressed(move |_, n_press, _, _| {
            let Some(inner) = weak.upgrade() else {
                return;
            };
            let pane = TreePane { inner };
            let entry = item
                .item()
                .and_downcast::<gtk::TreeListRow>()
                .and_then(|treerow| treerow.item())
                .and_then(|child| pane.entry_for_item(&child));
            let Some(entry) = entry else {
                return;
            };
            let Some(browser) = pane.browser() else {
                return;
            };
            if n_press == 2 || pane.single_click_activates(&entry) {
                if entry.is_directory() {
                    browser.navigate(entry.location.clone());
                } else {
                    browser.open_location(entry.location.clone());
                }
            } else if pane.inner.previews.get() && !entry.is_directory() {
                browser.request_automatic_preview((*entry).clone());
            }
        });
        row_box.add_controller(click);
    }
}

impl TreePane {
    pub(super) fn root_location(&self) -> Option<Location> {
        self.inner.root_location.borrow().clone()
    }

    pub(super) fn item_trigger(&self) -> Option<Rc<dyn Fn(f64, f64)>> {
        self.inner.item_trigger.borrow().clone()
    }

    /// Installs the row context menu. Runs once per pane; the pane itself is
    /// recreated when the root's trash state flips, so baked menu chrome
    /// never goes stale across navigation.
    pub(super) fn install_context_menus(&self, state: &Rc<ViewState>) {
        if self.inner.menus_installed.replace(true) {
            return;
        }
        let weak = Rc::downgrade(&self.inner);
        let resolve: crate::ui::browser::ContextResolver = Rc::new(move |picked: &gtk::Widget| {
            let inner = weak.upgrade()?;
            let pane = TreePane { inner };
            let mut current = Some(picked.clone());
            while let Some(widget) = current {
                if let Some(entry) = pane.entry_for_row_widget(&widget) {
                    let position = pane.view_position_of_widget(&widget)?;
                    if !pane.inner.selection.is_selected(position) {
                        pane.inner.syncing.set(true);
                        pane.inner.selection.select_item(position, true);
                        pane.inner.syncing.set(false);
                    }
                    return Some((None, entry));
                }
                current = widget.parent();
            }
            None
        });
        let trigger = crate::ui::browser::install_resolved_item_context_menu(
            state,
            &self.inner.view.clone().upcast(),
            resolve,
            0,
        );
        self.inner.item_trigger.replace(Some(trigger));
    }

    fn entry_for_row_widget(&self, widget: &gtk::Widget) -> Option<FileEntry> {
        for bound in self.inner.bound_items.borrow().iter() {
            let row = bound.widget.upgrade()?;
            if row == *widget {
                let item = bound.item.upgrade()?;
                let child = item
                    .item()
                    .and_downcast::<gtk::TreeListRow>()
                    .and_then(|treerow| treerow.item())?;
                return self.entry_for_item(&child).map(|entry| (*entry).clone());
            }
        }
        None
    }

    fn view_position_of_widget(&self, widget: &gtk::Widget) -> Option<u32> {
        for bound in self.inner.bound_items.borrow().iter() {
            if bound.widget.upgrade().as_ref() == Some(widget) {
                let item = bound.item.upgrade()?;
                return Some(item.position());
            }
        }
        None
    }

    /// Selected root rows as source positions, plus the focused root row:
    /// best-effort column selection when leaving tree mode. Nested branches
    /// have no column counterpart and do not carry over.
    pub(in crate::ui) fn export_root_selection(&self) -> (Vec<usize>, Option<usize>) {
        let mut positions = Vec::new();
        for position in super::bitset_positions(&self.inner.selection.selection()) {
            if let Some(row) = self.row_at(position as u32)
                && let Some(item) = row.item()
                && self
                    .inner
                    .item_levels
                    .borrow()
                    .get(&item)
                    .is_some_and(|level| level.is_none())
                && let Some(source) = self.inner.root_index.of_item(&item)
            {
                positions.push(source);
            }
        }
        let focused = self.inner.focused_item.borrow().clone().and_then(|item| {
            self.inner
                .item_levels
                .borrow()
                .get(&item)
                .is_some_and(|level| level.is_none())
                .then(|| self.inner.root_index.of_item(&item))
                .flatten()
        });
        positions.sort_unstable();
        (positions, focused)
    }

    /// Editor target for inline rename: the bound edit widgets, row, and
    /// collection view of the visible row showing `location`.
    pub(super) fn rename_target_for(
        &self,
        location: &Location,
    ) -> Option<(
        super::super::collection_edit::EditWidgets,
        gtk::Widget,
        gtk::Widget,
    )> {
        for bound in self.inner.bound_items.borrow().iter() {
            let item = bound.item.upgrade()?;
            let child = item.item()?.downcast::<gtk::TreeListRow>().ok()?;
            let child_item = child.item()?;
            let known = self.entry_for_item(&child_item)?;
            if known.location != *location {
                continue;
            }
            let widget = bound.widget.upgrade()?;
            if !widget.is_mapped() || !widget.is_ancestor(&self.inner.view) {
                continue;
            }
            return Some((bound.edit.clone(), widget, self.inner.view.clone().upcast()));
        }
        None
    }
}
