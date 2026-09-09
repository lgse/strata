// SPDX-License-Identifier: MIT

//! Alternate browser presentations.
//!
//! This module is deliberately isolated from the Miller-column implementation. It consumes the
//! same application events and emits the same navigation/selection intents, so adding another
//! presentation does not require scattering mode checks throughout the main browser view.

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::{Rc, Weak},
};

use gtk::{gio, glib, prelude::*};

pub(super) use super::icons_cell::{
    ICONS_CARD_SPACING, MAX_ICONS_THUMBNAIL_SIZE, MIN_ICONS_THUMBNAIL_SIZE, icons_card_extent,
    icons_card_icon_slot,
};
use crate::{
    app::{Browser, BrowserColumnSnapshot},
    model::{FileEntry, Location, MetadataValue, SortDirection, SortKey},
    ui::browser::paths::is_trash_location,
};

mod events;
mod list_factory;

use list_factory::{ListFactory, refresh_list_section};

const LIST_COLUMN_WIDTHS: [i32; 5] = [160, 160, 90, 120, 150];
const LIST_COLUMN_MIN_WIDTHS: [i32; 5] = [160, 80, 70, 80, 110];
const DEFAULT_ICONS_THUMBNAIL_SIZE: i32 = 64;
const SCROLL_SETTLE_DELAY: std::time::Duration = std::time::Duration::from_millis(80);
const FALLBACK_ICONS_COLUMN_WIDTH: i32 = 160;

#[derive(Clone)]
struct ListColumnLayout {
    widths: Rc<Vec<Cell<i32>>>,
    cells: Rc<Vec<RefCell<Vec<glib::WeakRef<gtk::Widget>>>>>,
    name_manually_resized: Rc<Cell<bool>>,
}

impl ListColumnLayout {
    fn new() -> Self {
        Self {
            widths: Rc::new(LIST_COLUMN_WIDTHS.into_iter().map(Cell::new).collect()),
            cells: Rc::new((0..5).map(|_| RefCell::new(Vec::new())).collect()),
            name_manually_resized: Rc::new(Cell::new(false)),
        }
    }
}

type TransferHandler = Rc<dyn Fn(Location, Vec<Location>, bool)>;
type TransferHandlerSlot = Rc<RefCell<Option<TransferHandler>>>;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BrowserMode {
    #[default]
    Columns,
    Icons,
    List,
}

impl BrowserMode {
    /// File-type headings are List-only. Icons clustering is wired but gated off.
    pub fn supports_type_grouping(self) -> bool {
        matches!(self, Self::List)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BrowserDensity {
    #[default]
    Compact,
    Airy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClickCount {
    One,
    Two,
}

impl ClickCount {
    pub fn from_stored(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::One),
            2 => Some(Self::Two),
            _ => None,
        }
    }

    pub fn stored(self) -> u8 {
        match self {
            Self::One => 1,
            Self::Two => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClickActivation {
    pub files: ClickCount,
    pub folders: ClickCount,
}

impl ClickActivation {
    pub fn default_for(mode: BrowserMode) -> Self {
        Self {
            files: ClickCount::Two,
            folders: match mode {
                BrowserMode::Columns => ClickCount::One,
                BrowserMode::Icons | BrowserMode::List => ClickCount::Two,
            },
        }
    }
}

impl Default for ClickActivation {
    fn default() -> Self {
        Self::default_for(BrowserMode::Columns)
    }
}

/// Maps a `StringList` item to its source index. Filter, sort, and flatten models
/// pass those objects through, so bind can resolve without scanning the source.
#[derive(Clone, Default)]
struct SourceIndexMap {
    by_item: Rc<RefCell<HashMap<glib::Object, usize>>>,
}

impl SourceIndexMap {
    fn watch(source: &gtk::StringList) -> Self {
        let map = Self::default();
        let tracked = map.clone();
        // Use the signal's list. Cloning it into this handler would pin the
        // StringList (and every item) after the pane is dropped.
        source.connect_items_changed(move |source, position, removed, added| {
            tracked.apply(source, position, removed, added);
        });
        map.rebuild(source);
        map
    }

    fn apply(&self, source: &gtk::StringList, position: u32, removed: u32, added: u32) {
        let can_append = {
            let by_item = self.by_item.borrow();
            removed == 0
                && position == by_item.len() as u32
                && position.saturating_add(added) == source.n_items()
        };
        if can_append {
            let mut by_item = self.by_item.borrow_mut();
            for index in position..position.saturating_add(added) {
                if let Some(item) = source.item(index) {
                    by_item.insert(item, index as usize);
                }
            }
            return;
        }
        self.rebuild(source);
    }

    fn rebuild(&self, source: &gtk::StringList) {
        let n_items = source.n_items() as usize;
        let mut by_item = HashMap::with_capacity(n_items);
        for position in 0..source.n_items() {
            if let Some(item) = source.item(position) {
                by_item.insert(item, position as usize);
            }
        }
        *self.by_item.borrow_mut() = by_item;
    }

    fn of_item(&self, item: &glib::Object) -> Option<usize> {
        self.by_item.borrow().get(item).copied()
    }

    fn of_view_position(&self, view: &impl IsA<gio::ListModel>, position: u32) -> Option<usize> {
        view.item(position).and_then(|item| self.of_item(&item))
    }
}

pub(in crate::ui) struct ActiveModeRename {
    entry: FileEntry,
    field: gtk::Entry,
    label: gtk::Widget,
    viewport_tick: Option<gtk::TickCallbackId>,
}

struct BoundModeItem {
    item: glib::WeakRef<gtk::ListItem>,
    widget: glib::WeakRef<gtk::Widget>,
    rename_label: glib::WeakRef<gtk::Widget>,
}

/// One collection view inside a pane. Icons and List each keep a single section.
#[derive(Clone)]
struct PaneSection {
    view: gtk::Widget,
    view_model: gio::ListModel,
    selection: gtk::MultiSelection,
    bound_items: Rc<RefCell<Vec<BoundModeItem>>>,
    syncing: Rc<Cell<bool>>,
    visit: super::marquee::ItemVisitor,
}

#[derive(Clone)]
struct Pane {
    depth: usize,
    shell: gtk::Box,
    header: gtk::Box,
    model: gtk::StringList,
    source_index: SourceIndexMap,
    filter_model: Option<gtk::FilterListModel>,
    /// The primary section that owns the pane's chrome.
    section: PaneSection,
    sections: Rc<RefCell<Vec<PaneSection>>>,
    icons: Option<Rc<IconsContext>>,
    targets: super::marquee::MarqueeTargets,
    /// Set while a reload has detached the pane's models from their views.
    detached: Rc<Cell<bool>>,
    stack: gtk::Stack,
    loading: super::loading_skeleton::DelayedLoading,
    status: gtk::Label,
    spinner: gtk::Spinner,
    truncated_hint: gtk::Image,
    marquee: super::marquee::Marquee,
    search: super::inline_search::InlineSearch,
    filter_entry: Option<gtk::Entry>,
    filter_button: Option<gtk::ToggleButton>,
    empty_trash_button: Option<gtk::Button>,
    show_hidden: Rc<Cell<bool>>,
    filter: gtk::CustomFilter,
}

impl Pane {
    /// The sections that render entries, in visual order.
    fn item_sections(&self) -> Vec<PaneSection> {
        self.sections.borrow().clone()
    }

    /// Include the primary section even while the item sections are empty.
    fn all_sections(&self) -> Vec<PaneSection> {
        let mut sections = self.item_sections();
        if !sections
            .iter()
            .any(|section| section.view == self.section.view)
        {
            sections.push(self.section.clone());
        }
        sections
    }

    fn focus_view(&self) -> gtk::Widget {
        self.item_sections()
            .first()
            .map_or_else(|| self.section.view.clone(), |section| section.view.clone())
    }
}

pub struct ModeViews {
    stack: gtk::Stack,
    icons_root: gtk::Box,
    list_root: gtk::Box,
    icons_panes: Vec<Pane>,
    list_pane: Option<Pane>,
    browser: Rc<Browser>,
    single_click_previews: Rc<Cell<bool>>,
    multiple_selection: Rc<Cell<bool>>,
    icons_click_activation: Rc<Cell<ClickActivation>>,
    list_click_activation: Rc<Cell<ClickActivation>>,
    transfer_handler: TransferHandlerSlot,
    cut_locations: Rc<RefCell<HashSet<Location>>>,
    context_state: RefCell<Option<Weak<super::browser::ViewState>>>,
    new_folder_state: RefCell<Option<Weak<super::browser::ViewState>>>,
    active_rename: Rc<RefCell<Option<ActiveModeRename>>>,
    mode: BrowserMode,
    density: BrowserDensity,
    group_by_type: bool,
    icons_thumbnail_size: Rc<Cell<i32>>,
    focus_before_header: RefCell<Option<glib::WeakRef<gtk::Widget>>>,
    /// Page Up/Down scrolls the viewport itself; skip the follow-up `scroll_to`
    /// that `FocusChanged` would otherwise schedule from stale GridView estimates.
    suppress_focus_scroll: Cell<bool>,
}

impl ModeViews {
    pub fn new(
        columns: &gtk::ScrolledWindow,
        browser: Rc<Browser>,
        multiple_selection: Rc<Cell<bool>>,
    ) -> Self {
        let icons_root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        icons_root.add_css_class("mode-icons");
        icons_root.set_halign(gtk::Align::Fill);
        icons_root.set_hexpand(true);
        icons_root.set_vexpand(true);
        let icons_scroll = gtk::ScrolledWindow::builder()
            .child(&icons_root)
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .hexpand(true)
            .vexpand(true)
            .build();
        icons_scroll.add_css_class("fixed-scrollbar");

        let list_root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list_root.add_css_class("mode-list");
        list_root.set_hexpand(true);
        list_root.set_vexpand(true);
        // The list pane header belongs to the viewport, while its user-resizable table
        // columns scroll independently below it.
        let list_scroll = gtk::ScrolledWindow::builder()
            .child(&list_root)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .hexpand(true)
            .vexpand(true)
            .build();

        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::None)
            .hexpand(true)
            .vexpand(true)
            .build();
        stack.add_named(columns, Some("columns"));
        stack.add_named(&icons_scroll, Some("icons"));
        stack.add_named(&list_scroll, Some("list"));
        stack.set_visible_child_name("columns");

        Self {
            stack,
            icons_root,
            list_root,
            icons_panes: Vec::new(),
            list_pane: None,
            browser,
            single_click_previews: Rc::new(Cell::new(true)),
            multiple_selection,
            icons_click_activation: Rc::new(Cell::new(ClickActivation::default_for(
                BrowserMode::Icons,
            ))),
            list_click_activation: Rc::new(Cell::new(ClickActivation::default_for(
                BrowserMode::List,
            ))),
            transfer_handler: Rc::new(RefCell::new(None)),
            cut_locations: Rc::new(RefCell::new(HashSet::new())),
            context_state: RefCell::new(None),
            new_folder_state: RefCell::new(None),
            active_rename: Rc::new(RefCell::new(None)),
            mode: BrowserMode::Columns,
            density: BrowserDensity::Compact,
            group_by_type: false,
            icons_thumbnail_size: Rc::new(Cell::new(DEFAULT_ICONS_THUMBNAIL_SIZE)),
            focus_before_header: RefCell::new(None),
            suppress_focus_scroll: Cell::new(false),
        }
    }

    pub fn widget(&self) -> gtk::Stack {
        self.stack.clone()
    }

    pub fn set_show_hidden(&self, show_hidden: bool) {
        for pane in self.all_panes() {
            pane.show_hidden.set(show_hidden);
            pane.filter.changed(gtk::FilterChange::Different);
        }
    }

    pub fn mode(&self) -> BrowserMode {
        self.mode
    }

    /// The marquee of the pane nearest the window's start edge, so chrome outside the
    /// panes can run a drag into whichever view the current mode shows.
    pub(super) fn leading_marquee(&self) -> Option<super::marquee::Marquee> {
        let pane = match self.mode {
            BrowserMode::Columns => return None,
            BrowserMode::Icons => self.icons_panes.first(),
            BrowserMode::List => self.list_pane.as_ref(),
        }?;
        Some(pane.marquee.clone())
    }

    fn single_pane(&self) -> Option<&Pane> {
        match self.mode {
            BrowserMode::Columns => None,
            BrowserMode::Icons => self.icons_panes.first(),
            BrowserMode::List => self.list_pane.as_ref(),
        }
    }

    pub fn header_has_focus(&self) -> bool {
        let focused = self.stack.root().and_then(|root| root.focus());
        self.single_pane()
            .is_some_and(|pane| widget_has_focus(&pane.header, focused.as_ref()))
    }

    pub fn focus_header_from_top_item(&self, actions_only: bool) -> bool {
        let Some(pane) = self.single_pane() else {
            return false;
        };
        let Some(focused) = self.stack.root().and_then(|root| root.focus()) else {
            return false;
        };
        let at_top = focused == *pane.stack.upcast_ref::<gtk::Widget>()
            || (pane
                .item_sections()
                .iter()
                .all(|section| section.view_model.n_items() == 0)
                && widget_has_focus(&pane.stack, Some(&focused)))
            || pane
                .item_sections()
                .into_iter()
                .find(|section| section.view_model.n_items() > 0)
                .is_some_and(|section| {
                    let Some((position, bounds)) = focused_section_item(&section, &focused) else {
                        return false;
                    };
                    if position == 0 {
                        return true;
                    }
                    if self.mode != BrowserMode::Icons {
                        return false;
                    }
                    let mut first_row = false;
                    (section.visit)(&mut |position, widget| {
                        if position == 0
                            && let Some(first) = widget.compute_bounds(&section.view)
                        {
                            first_row = (first.y() - bounds.y()).abs() < 1.0;
                        }
                    });
                    first_row
                });
        if !at_top {
            return false;
        }
        let target = if actions_only {
            pane.header.last_child()
        } else {
            Some(pane.header.clone().upcast())
        };
        if target.is_some_and(|target| target.child_focus(gtk::DirectionType::TabForward)) {
            self.focus_before_header.replace(Some(focused.downgrade()));
            return true;
        }
        false
    }

    pub fn move_header_focus(&self, direction: gtk::DirectionType) -> bool {
        let direction = match direction {
            gtk::DirectionType::Left => gtk::DirectionType::TabBackward,
            gtk::DirectionType::Right => gtk::DirectionType::TabForward,
            _ => return false,
        };
        self.single_pane()
            .is_some_and(|pane| pane.header.child_focus(direction))
    }

    pub fn focus_items_from_header(&self) -> bool {
        if self
            .focus_before_header
            .borrow_mut()
            .take()
            .and_then(|weak| weak.upgrade())
            .is_some_and(|view| view.is_mapped() && view.grab_focus())
        {
            return true;
        }
        self.single_pane()
            .is_some_and(|pane| pane.focus_view().grab_focus() || pane.stack.grab_focus())
    }

    /// Use rendered rows: filtering, grouping, and resizing change the icons geometry.
    pub fn at_left_edge(&self) -> bool {
        if self.mode != BrowserMode::Icons {
            return true;
        }
        let Some(focused) = self.stack.root().and_then(|root| root.focus()) else {
            return true;
        };
        self.icons_panes
            .iter()
            .flat_map(Pane::item_sections)
            .find_map(|section| {
                let (_, bounds) = focused_section_item(&section, &focused)?;
                let mut has_left_neighbor = false;
                (section.visit)(&mut |_, widget| {
                    if let Some(other) = widget.compute_bounds(&section.view) {
                        has_left_neighbor |=
                            other.x() < bounds.x() - 1.0 && (other.y() - bounds.y()).abs() < 1.0;
                    }
                });
                Some(!has_left_neighbor)
            })
            .unwrap_or(true)
    }

    pub fn selected_positions(&self) -> Option<(usize, Vec<usize>)> {
        let pane = match self.mode {
            BrowserMode::Columns => return None,
            BrowserMode::Icons => self.icons_panes.first(),
            BrowserMode::List => self.list_pane.as_ref(),
        }?;
        let mut positions: Vec<usize> = pane
            .item_sections()
            .iter()
            .flat_map(|section| {
                selected_source_positions(
                    &pane.source_index,
                    &section.view_model,
                    &section.selection,
                )
            })
            .collect();
        positions.sort_unstable();
        positions.dedup();
        Some((pane.depth, positions))
    }

    pub fn rename_is_active(&self) -> bool {
        self.active_rename.borrow().is_some()
    }

    pub(in crate::ui) fn active_rename_field(&self) -> Option<gtk::Entry> {
        self.active_rename
            .borrow()
            .as_ref()
            .map(|rename| rename.field.clone())
    }

    pub(in crate::ui) fn take_active_rename(
        &self,
        field: &gtk::Entry,
    ) -> Option<(ActiveModeRename, FileEntry, String)> {
        let active = self
            .active_rename
            .borrow()
            .as_ref()
            .filter(|active| active.field == *field && field.is_sensitive())
            .is_some();
        if !active {
            return None;
        }
        let rename = self.active_rename.take()?;
        let entry = rename.entry.clone();
        let name = field.text().to_string();
        Some((rename, entry, name))
    }

    pub(in crate::ui) fn take_rename(&self) -> Option<ActiveModeRename> {
        self.active_rename.take()
    }

    pub(in crate::ui) fn rename_label_widgets(
        &self,
        old_location: &Location,
        new_location: Option<&Location>,
    ) -> Vec<gtk::Widget> {
        let mut labels = Vec::new();
        for pane in self.all_panes() {
            for section in pane.item_sections() {
                section.bound_items.borrow_mut().retain(|bound| {
                    let (Some(item), Some(_widget)) =
                        (bound.item.upgrade(), bound.widget.upgrade())
                    else {
                        return false;
                    };
                    let Some(position) = item
                        .item()
                        .and_then(|value| pane.source_index.of_item(&value))
                    else {
                        return true;
                    };
                    let Some(entry) = self.browser.entry_at(pane.depth, position) else {
                        return true;
                    };
                    if (entry.location == *old_location
                        || new_location.is_some_and(|location| location == &entry.location))
                        && let Some(label) = bound.rename_label.upgrade()
                    {
                        labels.push(label);
                    }
                    true
                });
            }
        }
        labels
    }

    pub fn cancel_rename(&self) -> bool {
        let Some(rename) = self.active_rename.take() else {
            return false;
        };
        finish_mode_rename(rename);
        true
    }

    pub(in crate::ui) fn clear_filter(&self, depth: usize) {
        for pane in self
            .icons_panes
            .iter()
            .chain(self.list_pane.iter())
            .filter(|pane| pane.depth == depth)
        {
            if let Some(field) = pane.filter_entry.as_ref() {
                field.set_text("");
            }
        }
    }

    pub(in crate::ui) fn list_rename_view(&self, depth: usize) -> Option<gtk::ListView> {
        self.list_pane
            .as_ref()
            .filter(|pane| self.mode == BrowserMode::List && pane.depth == depth)?
            .section
            .view
            .clone()
            .downcast()
            .ok()
    }

    pub(in crate::ui) fn list_rename_row(
        &self,
        depth: usize,
        source_position: usize,
    ) -> Option<(u32, Option<gtk::Widget>)> {
        let pane = self.list_pane.as_ref().filter(|pane| pane.depth == depth)?;
        let section = &pane.section;
        let position =
            view_position_for_source(&pane.model, Some(&section.view_model), source_position)?;
        let row = section.bound_items.borrow().iter().find_map(|bound| {
            (bound.item.upgrade()?.position() == position)
                .then(|| bound.widget.upgrade())
                .flatten()
        });
        Some((position, row))
    }

    pub fn begin_rename(&self, depth: usize, source_position: usize, entry: &FileEntry) -> bool {
        self.cancel_rename();
        let pane = match self.mode {
            BrowserMode::Columns => return false,
            BrowserMode::Icons => self.icons_panes.iter().find(|pane| pane.depth == depth),
            BrowserMode::List => self.list_pane.as_ref().filter(|pane| pane.depth == depth),
        };
        let Some(pane) = pane else {
            return false;
        };
        let widget = pane.item_sections().iter().find_map(|section| {
            let position =
                view_position_for_source(&pane.model, Some(&section.view_model), source_position)?;
            section.bound_items.borrow().iter().find_map(|bound| {
                let item = bound.item.upgrade()?;
                (item.position() == position).then(|| {
                    bound
                        .widget
                        .upgrade()
                        .map(|widget| (widget, section.view.clone(), position))
                })?
            })
        });
        let Some((widget, collection, position)) = widget else {
            return false;
        };
        if !widget.is_mapped() || widget.width() <= 0 || pane.stack.is_transition_running() {
            return false;
        }
        let Some(label) = descendant_with_class(&widget, "alternate-rename-label") else {
            return false;
        };
        let field = widget
            .clone()
            .downcast::<gtk::Box>()
            .ok()
            .filter(|card| card.has_css_class("icons-card"))
            .and_then(|card| super::icons_cell::ensure_rename_field(&card))
            .or_else(|| {
                descendant_with_class(&widget, "inline-rename").and_downcast::<gtk::Entry>()
            });
        let Some(field) = field else {
            return false;
        };
        super::browser::prepare_collection_inline_edit(&collection, position);
        field.set_text(&entry.display_name);
        field.set_visible(true);
        label.set_visible(false);
        install_mode_rename_handlers(
            &field,
            self.active_rename.clone(),
            Rc::downgrade(&self.browser),
            self.context_state.borrow().clone().unwrap_or_default(),
        );
        field.set_sensitive(true);
        field.remove_css_class("error");
        field.set_tooltip_text(None);
        let viewport_tick = (self.mode == BrowserMode::List).then(|| {
            let state = self.context_state.borrow().clone().unwrap_or_default();
            let generation = state
                .upgrade()
                .map(|state| state.rename_reveal_generation());
            let row = widget.downgrade();
            let scroll = collection
                .ancestor(gtk::ScrolledWindow::static_type())
                .and_downcast::<gtk::ScrolledWindow>()
                .map(|scroll| scroll.downgrade());
            field.add_tick_callback(move |_, _| {
                if state
                    .upgrade()
                    .map(|state| state.rename_reveal_generation())
                    != generation
                {
                    return glib::ControlFlow::Continue;
                }
                if let (Some(row), Some(scroll)) = (
                    row.upgrade(),
                    scroll.as_ref().and_then(|scroll| scroll.upgrade()),
                ) {
                    super::browser::reveal_rename_row(&row, &scroll, None);
                }
                glib::ControlFlow::Continue
            })
        });
        self.active_rename.replace(Some(ActiveModeRename {
            entry: entry.clone(),
            field: field.clone(),
            label,
            viewport_tick,
        }));
        field.grab_focus();
        field.select_region(
            0,
            if entry.is_directory() {
                -1
            } else {
                super::browser::rename_stem_end(&entry.display_name)
            },
        );
        true
    }

    pub fn filter_has_focus(&self) -> bool {
        let focused = self.stack.root().and_then(|root| root.focus());
        self.icons_panes
            .iter()
            .chain(self.list_pane.iter())
            .filter_map(|pane| pane.filter_entry.as_ref())
            .any(|entry| widget_has_focus(entry, focused.as_ref()))
    }

    pub fn selected_search_result(&self) -> Option<FileEntry> {
        self.single_pane()?.search.selected_entry()
    }

    pub fn selected_search_results(&self) -> Option<Vec<FileEntry>> {
        self.single_pane()?.search.selected_entries()
    }

    pub fn item_view_has_focus(&self) -> bool {
        let focused = self.stack.root().and_then(|root| root.focus());
        self.icons_panes
            .iter()
            .chain(self.list_pane.iter())
            .any(|pane| {
                focused.as_ref() == Some(pane.stack.upcast_ref())
                    || pane
                        .all_sections()
                        .iter()
                        .any(|section| widget_has_focus(&section.view, focused.as_ref()))
            })
    }

    pub fn empty_filter_has_focus(&self) -> bool {
        let focused = self.stack.root().and_then(|root| root.focus());
        self.icons_panes
            .iter()
            .chain(self.list_pane.iter())
            .filter_map(|pane| pane.filter_entry.as_ref())
            .any(|entry| entry.text().is_empty() && widget_has_focus(entry, focused.as_ref()))
    }

    pub fn show_filter_with_query(&self, query: Option<&str>) -> bool {
        let pane = match self.mode {
            BrowserMode::Columns => None,
            BrowserMode::Icons => self.icons_panes.first(),
            BrowserMode::List => self.list_pane.as_ref(),
        };
        let Some(pane) = pane else {
            return false;
        };
        let (Some(entry), Some(button)) = (pane.filter_entry.as_ref(), pane.filter_button.as_ref())
        else {
            return false;
        };
        button.set_active(true);
        super::browser::focus_filter_entry(entry, query);
        true
    }

    pub fn dismiss_focused_filter(&self) -> bool {
        let focused = self.stack.root().and_then(|root| root.focus());
        let Some(pane) = self
            .icons_panes
            .iter()
            .chain(self.list_pane.iter())
            .find(|pane| {
                pane.filter_entry
                    .as_ref()
                    .is_some_and(|entry| widget_has_focus(entry, focused.as_ref()))
            })
        else {
            return false;
        };
        if let Some(button) = pane.filter_button.as_ref() {
            button.set_active(false);
        }
        pane.focus_view().grab_focus();
        true
    }

    pub fn prepare_mode(&mut self, mode: BrowserMode) {
        if self.mode == mode {
            return;
        }
        self.cancel_rename();
        self.mode = mode;
        match mode {
            BrowserMode::Columns => {}
            BrowserMode::Icons => self.rebuild_icons(),
            BrowserMode::List => self.rebuild_list(),
        }
    }

    pub fn show_mode(&self, mode: BrowserMode) {
        self.stack.set_visible_child_name(match mode {
            BrowserMode::Columns => "columns",
            BrowserMode::Icons => "icons",
            BrowserMode::List => "list",
        });
    }

    pub fn clear_inactive_mode(&mut self, mode: BrowserMode) {
        if self.mode == mode {
            return;
        }
        match mode {
            BrowserMode::Columns => {}
            BrowserMode::Icons => self.clear_icons(),
            BrowserMode::List => self.clear_list(),
        }
    }

    pub fn set_single_click_previews(&self, enabled: bool) {
        self.single_click_previews.set(enabled);
    }

    #[cfg(test)]
    pub(in crate::ui) fn single_click_previews_enabled(&self) -> bool {
        self.single_click_previews.get()
    }

    pub fn set_click_activation(&self, mode: BrowserMode, activation: ClickActivation) {
        match mode {
            BrowserMode::Columns => {}
            BrowserMode::Icons => self.icons_click_activation.set(activation),
            BrowserMode::List => self.list_click_activation.set(activation),
        }
    }

    pub fn set_transfer_handler(&self, handler: TransferHandler) {
        self.transfer_handler.replace(Some(handler));
    }

    pub fn set_new_folder_state(&self, state: Weak<super::browser::ViewState>) {
        self.new_folder_state.replace(Some(state));
    }

    pub fn set_context_state(&self, state: Weak<super::browser::ViewState>) {
        self.context_state.replace(Some(state));
    }

    pub fn set_cut_locations(&self, locations: &[Location]) {
        self.cut_locations
            .replace(locations.iter().cloned().collect());
        for pane in self.icons_panes.iter().chain(self.list_pane.iter()) {
            refresh_cut_pane(pane, &self.browser, locations);
        }
    }

    pub fn set_density(&mut self, density: BrowserDensity) {
        self.density = density;
        for pane in &self.icons_panes {
            configure_icons_density(pane, density);
        }
        for root in [&self.icons_root, &self.list_root] {
            root.remove_css_class("density-compact");
            root.remove_css_class("density-airy");
            root.add_css_class(match density {
                BrowserDensity::Compact => "density-compact",
                BrowserDensity::Airy => "density-airy",
            });
        }
    }

    pub fn set_group_by_type(&mut self, enabled: bool) {
        if self.group_by_type == enabled {
            return;
        }
        self.cancel_rename();
        self.group_by_type = enabled;
        if self.mode.supports_type_grouping() {
            self.rebuild_list();
        }
    }

    fn visible_panes(&self) -> Vec<&Pane> {
        match self.mode {
            BrowserMode::Columns => Vec::new(),
            BrowserMode::Icons => self.icons_panes.iter().collect(),
            BrowserMode::List => self.list_pane.iter().collect(),
        }
    }

    pub fn resume_native_selection(&self) {
        let Some((depth, focused, _)) = self.browser.focused_item() else {
            return;
        };
        if !self.browser.selected_positions(depth).is_empty() {
            return;
        }
        // Seed GTK's empty selection before the arrow moves it, without scheduling
        // a focus restore that would undo the native move after key dispatch.
        for pane in self.panes_at(depth) {
            set_selections(pane, &[focused]);
        }
        self.browser.set_selection(depth, &[focused], Some(focused));
    }

    pub fn focused_position(&self) -> Option<(usize, usize)> {
        let focused = self.stack.root()?.focus()?;
        for pane in self.visible_panes() {
            let sections = pane.item_sections();
            let position = sections.iter().find_map(|section| {
                section.bound_items.borrow().iter().find_map(|bound| {
                    let widget = bound.widget.upgrade()?;
                    if widget != focused
                        && !widget.is_ancestor(&focused)
                        && !focused.is_ancestor(&widget)
                    {
                        return None;
                    }
                    source_position_for_view(
                        &pane.source_index,
                        Some(&section.view_model),
                        bound.item.upgrade()?.position(),
                    )
                })
            });
            if let Some(position) = position {
                return Some((pane.depth, position));
            }
        }
        None
    }

    pub fn item_at_left_edge(&self) -> bool {
        let Some(focused) = self.stack.root().and_then(|root| root.focus()) else {
            return false;
        };
        let panes = self.visible_panes();
        let Some(pane) = panes.first() else {
            return false;
        };
        let Some(section) = pane
            .item_sections()
            .into_iter()
            .find(|section| widget_has_focus(&section.view, Some(&focused)))
        else {
            return false;
        };
        if self.mode == BrowserMode::List {
            return true;
        }
        let bounds = section
            .bound_items
            .borrow()
            .iter()
            .filter_map(|bound| {
                if bound.item.upgrade()?.position() == gtk::INVALID_LIST_POSITION {
                    return None;
                }
                let widget = bound
                    .widget
                    .upgrade()
                    .filter(|widget| widget.is_mapped() && widget.width() > 0)?;
                let has_focus = widget == focused
                    || widget.is_ancestor(&focused)
                    || focused.is_ancestor(&widget);
                Some((has_focus, widget.compute_bounds(&section.view)?))
            })
            .collect::<Vec<_>>();
        let Some((_, current)) = bounds.iter().find(|(has_focus, _)| *has_focus) else {
            return false;
        };
        !bounds
            .iter()
            .any(|(_, bounds)| bounds.x() < current.x() - 1.0)
    }

    pub fn visual_order(&self, depth: usize) -> Vec<usize> {
        let Some(pane) = self
            .visible_panes()
            .into_iter()
            .find(|pane| pane.depth == depth)
        else {
            return Vec::new();
        };
        pane.item_sections()
            .into_iter()
            .flat_map(|section| {
                (0..section.view_model.n_items())
                    .filter_map(|position| {
                        source_position_for_view(
                            &pane.source_index,
                            Some(&section.view_model),
                            position,
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    pub fn suppress_focus_scroll(&self) {
        self.suppress_focus_scroll.set(true);
    }

    pub fn focus_visible_pane(&self, depth: usize) {
        if self.rename_is_active() {
            return;
        }
        let Some(pane) = self
            .visible_panes()
            .into_iter()
            .find(|pane| pane.depth == depth)
        else {
            return;
        };
        let target = self
            .browser
            .focused_item()
            .filter(|(focused_depth, _, _)| *focused_depth == depth)
            .and_then(|(_, source, _)| {
                pane.item_sections().into_iter().find_map(|section| {
                    let position =
                        view_position_for_source(&pane.model, Some(&section.view_model), source)?;
                    (position < section.view_model.n_items()).then_some((section.view, position))
                })
            });
        let (view, position) = target.map_or_else(
            || (pane.focus_view(), None),
            |(view, position)| (view, Some(position)),
        );
        if !view.grab_focus() {
            for pane in self.panes_at(depth) {
                pane.stack.grab_focus();
            }
            return;
        }
        if self.suppress_focus_scroll.replace(false) {
            if let Some(position) = position
                && let Some(items) = pane
                    .item_sections()
                    .into_iter()
                    .find(|section| section.view == view)
                    .map(|section| section.bound_items.clone())
            {
                focus_collection_cursor_when_bound(view.downgrade(), items, position);
            }
            return;
        }
        if let Some(position) = position {
            focus_collection_item(&view, position);
        }
        let view = view.downgrade();
        glib::idle_add_local_once(move || {
            if let Some(view) = view.upgrade()
                && widget_has_focus(&view, view.root().and_then(|root| root.focus()).as_ref())
            {
                if view
                    .root()
                    .and_then(|root| root.focus())
                    .is_some_and(|focused| {
                        super::focus_navigation::editable(&focused)
                            || super::focus_navigation::in_popover(&focused)
                    })
                {
                    return;
                }
                if let Some(position) = position {
                    focus_collection_item(&view, position);
                } else {
                    view.grab_focus();
                }
            }
        });
    }

    fn all_panes(&self) -> Vec<&Pane> {
        self.icons_panes
            .iter()
            .chain(self.list_pane.as_ref())
            .collect()
    }

    fn panes_at(&self, depth: usize) -> Vec<&Pane> {
        match self.mode {
            BrowserMode::Columns => Vec::new(),
            BrowserMode::Icons => self
                .icons_panes
                .iter()
                .find(|pane| pane.depth == depth)
                .into_iter()
                .collect(),
            BrowserMode::List => self
                .list_pane
                .as_ref()
                .filter(|pane| pane.depth == depth)
                .into_iter()
                .collect(),
        }
    }

    /// The menu for the pane's own background. Item menus belong to the sections that
    /// render them, so a grouped view installs one per group as it is built.
    fn install_context_menu(&self, pane: &Pane) {
        let Some(state) = self.context_state.borrow().as_ref().and_then(Weak::upgrade) else {
            return;
        };
        let Some(location) = self.browser.location_at(pane.depth) else {
            return;
        };
        pane.search.install_context_menu(&state, pane.depth);
        let search = pane.search.clone();
        let sections = Rc::downgrade(&pane.sections);
        let entries = pane.model.downgrade();
        super::browser::install_folder_context_menu(
            &state,
            pane.stack.upcast_ref(),
            Rc::new(move || {
                entries
                    .upgrade()
                    .is_some_and(|entries| entries.n_items() > 0)
            }),
            Rc::new(move |picked| {
                search.is_item_target(picked)
                    || sections.upgrade().is_some_and(|sections| {
                        sections
                            .borrow()
                            .iter()
                            .any(|section| section_item_position(section, picked).is_some())
                    })
            }),
            pane.depth,
            location,
        );
    }

    fn clear_icons(&mut self) {
        for pane in &self.icons_panes {
            detach_pane_models(pane);
        }
        clear_box(&self.icons_root);
        self.icons_panes.clear();
    }

    fn clear_list(&mut self) {
        if let Some(pane) = self.list_pane.as_ref() {
            detach_pane_models(pane);
        }
        clear_box(&self.list_root);
        self.list_pane = None;
    }

    fn rebuild_icons(&mut self) {
        let Some(depth) = self.browser.active_depth() else {
            self.clear_icons();
            return;
        };
        let Some(snapshot) = self.browser.column_snapshot(depth) else {
            return;
        };
        self.clear_icons();
        let pane = build_icons_pane(
            self.browser.clone(),
            ModeClickOptions {
                previews: self.single_click_previews.clone(),
                activation: self.icons_click_activation.clone(),
                multiple_selection: self.multiple_selection.clone(),
            },
            self.transfer_handler.clone(),
            self.cut_locations.clone(),
            IconsOptions {
                state: self.context_state.borrow().clone(),
                new_folder_state: self.new_folder_state.borrow().clone(),
                thumbnail_size: self.icons_thumbnail_size.clone(),
                group_by_type: false,
                density: self.density,
            },
            depth,
            &snapshot.location.display_name(),
        );
        configure_icons_density(&pane, self.density);
        self.install_context_menu(&pane);
        self.icons_root.append(&pane.shell);
        apply_snapshot(&pane, &snapshot, &self.browser);
        self.icons_panes.push(pane);
    }

    fn rebuild_list(&mut self) {
        let Some(depth) = self.browser.active_depth() else {
            self.clear_list();
            return;
        };
        let Some(snapshot) = self.browser.column_snapshot(depth) else {
            return;
        };
        self.clear_list();
        let pane = build_list_pane(
            self.browser.clone(),
            ModeClickOptions {
                previews: self.single_click_previews.clone(),
                activation: self.list_click_activation.clone(),
                multiple_selection: self.multiple_selection.clone(),
            },
            self.transfer_handler.clone(),
            self.cut_locations.clone(),
            ListOptions {
                state: self.context_state.borrow().clone(),
                new_folder_state: self.new_folder_state.borrow().clone(),
                group_by_type: self.group_by_type,
            },
            depth,
            &snapshot.location.display_name(),
        );
        self.install_context_menu(&pane);
        self.list_root.append(&pane.shell);
        apply_snapshot(&pane, &snapshot, &self.browser);
        self.list_pane = Some(pane);
    }
}

fn widget_has_focus(widget: &impl IsA<gtk::Widget>, focused: Option<&gtk::Widget>) -> bool {
    widget.has_focus()
        || focused.is_some_and(|focused| {
            focused == widget.as_ref() || focused.is_ancestor(widget.as_ref())
        })
}

fn pane_holds_keyboard_focus(pane: &Pane) -> bool {
    let focused = pane.stack.root().and_then(|root| root.focus());
    widget_has_focus(&pane.stack, focused.as_ref())
        || pane
            .item_sections()
            .iter()
            .any(|section| widget_has_focus(&section.view, focused.as_ref()))
}

#[derive(Clone)]
struct ListOptions {
    state: Option<Weak<super::browser::ViewState>>,
    new_folder_state: Option<Weak<super::browser::ViewState>>,
    group_by_type: bool,
}

struct IconsOptions {
    state: Option<Weak<super::browser::ViewState>>,
    new_folder_state: Option<Weak<super::browser::ViewState>>,
    thumbnail_size: Rc<Cell<i32>>,
    group_by_type: bool,
    density: BrowserDensity,
}

#[derive(Clone)]
struct ModeClickOptions {
    previews: Rc<Cell<bool>>,
    activation: Rc<Cell<ClickActivation>>,
    multiple_selection: Rc<Cell<bool>>,
}

pub(in crate::ui) fn finish_mode_rename(rename: ActiveModeRename) {
    if let Some(tick) = rename.viewport_tick {
        tick.remove();
    }
    rename.label.set_visible(true);
    rename.field.set_visible(false);
    rename.field.set_sensitive(true);
    rename.field.remove_css_class("error");
    rename.field.set_tooltip_text(None);
}

fn submit_mode_rename(
    active: &RefCell<Option<ActiveModeRename>>,
    browser: &Weak<Browser>,
    field: &gtk::Entry,
) {
    let entry = active
        .borrow()
        .as_ref()
        .filter(|active| active.field == *field && field.is_sensitive())
        .map(|active| active.entry.clone());
    let Some(entry) = entry else { return };
    let name = field.text().to_string();
    if let Some(rename) = active.take() {
        finish_mode_rename(rename);
    }
    if let Some(browser) = browser.upgrade() {
        super::browser::queue_rename(&browser, entry, name);
    }
}

fn install_mode_rename_handlers(
    field: &gtk::Entry,
    active: Rc<RefCell<Option<ActiveModeRename>>>,
    browser: Weak<Browser>,
    state: Weak<super::browser::ViewState>,
) {
    if field.has_css_class("mode-rename-wired") {
        return;
    }
    field.add_css_class("mode-rename-wired");
    field.connect_changed(|field| {
        super::browser::update_basename_validation(field);
    });
    let active = Rc::downgrade(&active);
    let submit_active = active.clone();
    let submit_browser = browser.clone();
    let submit_state = state.clone();
    field.connect_activate(move |field| {
        if let Some(state) = submit_state.upgrade() {
            state.submit_mode_rename(field);
        } else if let Some(active) = submit_active.upgrade() {
            submit_mode_rename(&active, &submit_browser, field);
        }
    });
    let focus = gtk::EventControllerFocus::new();
    focus.connect_leave(move |controller| {
        if let Some(field) = controller.widget().and_downcast::<gtk::Entry>() {
            if let Some(state) = state.upgrade() {
                state.submit_mode_rename(&field);
            } else if let Some(active) = active.upgrade() {
                submit_mode_rename(&active, &browser, &field);
            }
        }
    });
    field.add_controller(focus);
}

struct IconsControls {
    leading: gtk::Box,
    actions: gtk::Box,
    filter_entry: gtk::Entry,
    filter_revealer: gtk::Revealer,
    filter_button: gtk::ToggleButton,
    thumbnail_scale: gtk::Scale,
    thumbnail_value: gtk::Label,
    thumbnail_popover: gtk::Popover,
    empty_trash_button: Option<gtk::Button>,
}

fn filter_controls(tooltip: &str) -> (gtk::Entry, gtk::Revealer, gtk::ToggleButton) {
    let entry = gtk::Entry::builder()
        .placeholder_text("Filter items…")
        .has_frame(false)
        .hexpand(true)
        .build();
    entry.add_css_class("column-filter-entry");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    row.add_css_class("column-filter");
    row.append(&crate::assets::chrome_icon(crate::assets::icons::FUNNEL));
    row.append(&entry);
    let revealer = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideDown)
        .child(&row)
        .build();
    let button = gtk::ToggleButton::builder().tooltip_text(tooltip).build();
    button.set_child(Some(&crate::assets::chrome_icon(
        crate::assets::icons::FUNNEL,
    )));
    button.add_css_class("column-header-action");
    let shown_filter = revealer.clone();
    let focused_filter = entry.clone();
    button.connect_toggled(move |button| {
        shown_filter.set_reveal_child(button.is_active());
        if button.is_active() {
            focused_filter.grab_focus();
        } else {
            focused_filter.set_text("");
        }
    });
    (entry, revealer, button)
}

fn icons_controls(browser: &Rc<Browser>, depth: usize, thumbnail_size: i32) -> IconsControls {
    let leading = list_navigation(browser);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    actions.add_css_class("icons-header-actions");

    let thumbnail_popover = gtk::Popover::new();
    thumbnail_popover.set_has_arrow(false);
    thumbnail_popover.add_css_class("icons-thumbnail-popover");
    let thumbnail_content = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let thumbnail_heading = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let thumbnail_title = gtk::Label::new(Some("Thumbnail size"));
    thumbnail_title.add_css_class("icons-thumbnail-title");
    thumbnail_title.set_xalign(0.0);
    thumbnail_title.set_hexpand(true);
    let thumbnail_value = gtk::Label::new(Some(&format!("{thumbnail_size} px")));
    thumbnail_value.add_css_class("icons-thumbnail-value");
    thumbnail_heading.append(&thumbnail_title);
    thumbnail_heading.append(&thumbnail_value);
    let thumbnail_scale = gtk::Scale::with_range(
        gtk::Orientation::Horizontal,
        f64::from(MIN_ICONS_THUMBNAIL_SIZE),
        f64::from(MAX_ICONS_THUMBNAIL_SIZE),
        16.0,
    );
    thumbnail_scale.set_increments(16.0, 1.0);
    thumbnail_scale.add_css_class("icons-thumbnail-scale");
    thumbnail_scale.set_draw_value(false);
    thumbnail_scale.set_value(f64::from(thumbnail_size));
    thumbnail_scale.set_size_request(220, -1);
    disable_scale_long_press_zoom(&thumbnail_scale);
    let thumbnail_extremes = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    thumbnail_extremes.add_css_class("icons-thumbnail-extremes");
    let small = gtk::Label::new(Some("Small"));
    small.set_xalign(0.0);
    small.set_hexpand(true);
    let large = gtk::Label::new(Some("Large"));
    large.set_xalign(1.0);
    thumbnail_extremes.append(&small);
    thumbnail_extremes.append(&large);
    thumbnail_content.append(&thumbnail_heading);
    thumbnail_content.append(&thumbnail_scale);
    thumbnail_content.append(&thumbnail_extremes);
    thumbnail_popover.set_child(Some(&thumbnail_content));
    let thumbnail_menu = gtk::MenuButton::builder()
        .tooltip_text("Thumbnail size")
        .popover(&thumbnail_popover)
        .build();
    thumbnail_menu.add_css_class("column-header-action");
    thumbnail_menu.add_css_class("icons-thumbnail-menu");
    thumbnail_menu.set_child(Some(&crate::assets::chrome_icon(
        crate::assets::icons::PICTURES,
    )));
    let empty_trash = super::browser::empty_trash_button(browser);
    let is_trash = browser
        .location_at(depth)
        .is_some_and(|location| super::browser::is_trash_root(&location));
    empty_trash.set_visible(is_trash);
    empty_trash.set_sensitive(false);
    actions.append(&empty_trash);
    actions.append(&super::browser::pane_refresh_button(browser, depth));
    actions.append(&thumbnail_menu);
    actions.append(&super::browser::column_sort_direction_toggle(
        browser, depth,
    ));
    actions.append(&super::browser::column_sort_menu(browser, depth));

    let (filter_entry, filter_revealer, filter_button) = filter_controls("Filter icons (Ctrl+F)");
    actions.append(&filter_button);
    IconsControls {
        leading,
        actions,
        filter_entry,
        filter_revealer,
        filter_button,
        thumbnail_scale,
        thumbnail_value,
        thumbnail_popover,
        empty_trash_button: is_trash.then_some(empty_trash),
    }
}

fn disable_scale_long_press_zoom(scale: &gtk::Scale) {
    let controllers = scale.observe_controllers();
    let long_presses: Vec<gtk::GestureLongPress> = (0..controllers.n_items())
        .filter_map(|index| {
            controllers
                .item(index)?
                .downcast::<gtk::GestureLongPress>()
                .ok()
        })
        .collect();
    for long_press in long_presses {
        scale.remove_controller(&long_press);
    }
}

/// Shared wiring every icons view in a pane needs, so a pane that groups entries by
/// type can build one view per group without threading a dozen arguments through.
struct IconsContext {
    browser: Rc<Browser>,
    depth: usize,
    click: ModeClickOptions,
    transfer: TransferHandlerSlot,
    cuts: Rc<RefCell<HashSet<Location>>>,
    state: Option<Weak<super::browser::ViewState>>,
    thumbnail_size: Rc<Cell<i32>>,
    source_index: SourceIndexMap,
    sections: Weak<RefCell<Vec<PaneSection>>>,
    density: Cell<BrowserDensity>,
    scrolling: Rc<Cell<bool>>,
    filter_query: Rc<RefCell<String>>,
}

fn build_icons_pane(
    browser: Rc<Browser>,
    click_options: ModeClickOptions,
    transfer_handler: TransferHandlerSlot,
    cut_locations: Rc<RefCell<HashSet<Location>>>,
    options: IconsOptions,
    depth: usize,
    title: &str,
) -> Pane {
    let controls = icons_controls(&browser, depth, options.thumbnail_size.get());
    if let Some(state) = options.new_folder_state {
        controls
            .actions
            .prepend(&super::browser::pane_new_folder_button(state, depth));
    }
    let (shell, header, content, model, stack, status, spinner, truncated_hint) = pane_base(
        title,
        BrowserMode::Icons,
        "icons-pane",
        &icons_loading_skeleton(options.thumbnail_size.get(), options.density),
        Some(controls.leading.clone().upcast()),
        Some(controls.actions.clone().upcast()),
    );
    let source_index = SourceIndexMap::watch(&model);
    if let Some(destination) = browser.location_at(depth) {
        install_mode_directory_drop_target(&stack, destination, transfer_handler.clone());
    }
    content.append(&controls.filter_revealer);
    let filter_query = Rc::new(RefCell::new(String::new()));
    let initial_show_hidden = browser
        .column_preferences(depth)
        .map_or_else(|| browser.preferences().show_hidden, |p| p.show_hidden);
    let show_hidden = Rc::new(Cell::new(initial_show_hidden));
    let filter = super::browser::entry_filter(show_hidden.clone(), filter_query.clone());
    let filtered_model = gtk::FilterListModel::new(Some(model.clone()), Some(filter.clone()));
    let filter_for_pane = filter.clone();
    let query_for_filter = filter_query.clone();
    let filter_for_settled = filter.clone();
    super::browser::debounce_filter_entry(&controls.filter_entry, move |text| {
        super::browser::notify_filter_query(&filter_for_settled, &query_for_filter, text);
    });
    let sections: Rc<RefCell<Vec<PaneSection>>> = Rc::new(RefCell::new(Vec::new()));
    let context = Rc::new(IconsContext {
        browser,
        depth,
        click: click_options,
        transfer: transfer_handler,
        cuts: cut_locations,
        state: options.state,
        thumbnail_size: options.thumbnail_size.clone(),
        source_index: source_index.clone(),
        sections: Rc::downgrade(&sections),
        density: Cell::new(options.density),
        scrolling: Rc::new(Cell::new(false)),
        filter_query: filter_query.clone(),
    });
    let view_model = if options.group_by_type {
        let sorted =
            gtk::SortListModel::new(Some(filtered_model.clone()), None::<gtk::CustomSorter>);
        let sorter = type_group_sorter();
        sorted.set_sorter(Some(&sorter));
        sorted.upcast::<gio::ListModel>()
    } else {
        filtered_model.clone().upcast()
    };
    let pane_section = build_icons_view(&context, &view_model);
    pane_section.view.set_vexpand(true);
    let browser_for_filter_activate = Rc::downgrade(&context.browser);
    let selection_for_filter_activate = pane_section.selection.clone();
    let view_for_filter_activate = view_model.clone();
    let source_index_for_filter_activate = context.source_index.clone();
    let depth_for_filter_activate = context.depth;
    let filter_query_for_activate = filter_query.clone();
    controls.filter_entry.connect_activate(move |_| {
        if filter_query_for_activate.borrow().trim().is_empty() {
            return;
        }
        activate_filtered_item(
            &browser_for_filter_activate,
            &selection_for_filter_activate,
            &view_for_filter_activate,
            &source_index_for_filter_activate,
            depth_for_filter_activate,
        );
    });
    sections.borrow_mut().push(pane_section.clone());
    let root = pane_section.view.clone();

    let density_for_size = context.density.get();
    let sections_for_size = Rc::downgrade(&sections);
    let thumbnail_size_for_change = options.thumbnail_size.clone();
    let value_for_change = controls.thumbnail_value.clone();
    let loading_stack = stack.downgrade();
    let loading_context = Rc::downgrade(&context);
    controls
        .thumbnail_scale
        .connect_value_changed(move |scale| {
            let size = scale.value().round() as i32;
            value_for_change.set_label(&format!("{size} px"));
            thumbnail_size_for_change.set(size);
            if let (Some(stack), Some(context)) =
                (loading_stack.upgrade(), loading_context.upgrade())
            {
                let was_loading = stack.visible_child_name().as_deref() == Some("loading");
                if let Some(old) = stack.child_by_name("loading") {
                    stack.remove(&old);
                }
                stack.add_named(
                    &icons_loading_skeleton(size, context.density.get()),
                    Some("loading"),
                );
                if was_loading {
                    stack.set_visible_child_name("loading");
                }
            }
            let Some(sections) = sections_for_size.upgrade() else {
                return;
            };
            for section in sections.borrow().iter() {
                resize_icons_thumbnail_slots(section, size);
            }
            let density = loading_context
                .upgrade()
                .map(|context| context.density.get())
                .unwrap_or(density_for_size);
            for section in sections.borrow().iter() {
                pin_ungrouped_icons_columns(section, viewport_page_width(&section.view), density);
            }
        });

    let scroll = gtk::ScrolledWindow::builder()
        .child(&root)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vexpand(true)
        .build();
    scroll.add_css_class("fixed-scrollbar");
    scroll.add_css_class("browser-listing-scroll");
    super::scrolling::popover::dismiss_on_outside_scroll(&controls.thumbnail_popover);
    let browser_for_settle = Rc::downgrade(&context.browser);
    let source_index_for_settle = context.source_index.clone();
    let sections_for_settle = context.sections.clone();
    let cuts_for_settle = context.cuts.clone();
    let depth_for_settle = context.depth;
    install_scroll_settle(&scroll, context.scrolling.clone(), None, move || {
        let Some(browser) = browser_for_settle.upgrade() else {
            return;
        };
        let Some(sections) = sections_for_settle.upgrade() else {
            return;
        };
        let cuts = cuts_for_settle.borrow();
        for section in sections.borrow().iter() {
            refresh_icons_section(
                &browser,
                depth_for_settle,
                &source_index_for_settle,
                section,
                &cuts,
            );
        }
    });
    let section = pane_section.clone();
    let width_context = Rc::downgrade(&context);
    after_icons_viewport_width_changes(&scroll, move |width| {
        let Some(context) = width_context.upgrade() else {
            return;
        };
        pin_ungrouped_icons_columns(&section, width, context.density.get());
    });
    let targets: super::marquee::MarqueeTargets = Rc::new(RefCell::new(Vec::new()));
    let (collection, marquee) = collection_with_marquee(&root, scroll, targets.clone());
    let search = super::inline_search::wrap(
        &collection,
        &controls.filter_entry,
        context
            .browser
            .location_at(depth)
            .and_then(|location| location.native_path().map(std::path::Path::to_path_buf)),
        &context.browser,
    );
    content.append(&search.widget);
    marquee.add_origin_surface(&header);
    let pane = Pane {
        depth,
        shell,
        header,
        model,
        source_index,
        filter_model: Some(filtered_model),
        section: pane_section,
        sections,
        icons: Some(context),
        targets,
        detached: Rc::new(Cell::new(false)),
        loading: super::loading_skeleton::DelayedLoading::new(&stack),
        stack,
        status,
        spinner,
        truncated_hint,
        marquee,
        search,
        filter_entry: Some(controls.filter_entry),
        filter_button: Some(controls.filter_button),
        empty_trash_button: controls.empty_trash_button,
        show_hidden,
        filter: filter_for_pane,
    };
    refresh_marquee_targets(&pane);
    pane
}

fn pane_directory_name(browser: &Rc<Browser>, depth: usize) -> String {
    browser
        .location_at(depth)
        .map(|location| location.display_name())
        .unwrap_or_default()
}

fn build_icons_view(context: &Rc<IconsContext>, model: &impl IsA<gio::ListModel>) -> PaneSection {
    let depth = context.depth;
    let view_model = model.clone().upcast::<gio::ListModel>();
    let selection = gtk::MultiSelection::new(Some(view_model.clone()));
    let syncing_selection = Rc::new(Cell::new(false));
    let bound_items: Rc<RefCell<Vec<BoundModeItem>>> = Rc::new(RefCell::new(Vec::new()));
    let factory = gtk::SignalListItemFactory::new();
    let bound_items_for_setup = bound_items.clone();
    let filter_query_for_setup = context.filter_query.clone();
    let selection_for_setup = selection.clone();
    let browser_for_setup = Rc::downgrade(&context.browser);
    let previews_for_setup = context.click.previews.clone();
    let activation_for_setup = context.click.activation.clone();
    let filtered_for_setup = view_model.clone();
    let source_index_for_setup = context.source_index.clone();
    let positions_for_setup = PanePositions {
        index: source_index_for_setup.clone(),
        view: filtered_for_setup.clone(),
    };
    let transfers_for_setup = context.transfer.clone();
    let peek_for_setup = context.state.clone();
    let thumbnail_size_for_setup = context.thumbnail_size.clone();
    factory.connect_setup(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let thumbnail_size = icons_card_icon_slot(thumbnail_size_for_setup.get());
        let card = super::icons_cell::new_card(thumbnail_size);
        let Some((_, rename_label)) = super::icons_cell::parts(&card) else {
            return;
        };
        install_preview_click(
            &card,
            item,
            browser_for_setup.clone(),
            previews_for_setup.clone(),
            activation_for_setup.clone(),
            depth,
            Some((source_index_for_setup.clone(), filtered_for_setup.clone())),
            filter_query_for_setup.clone(),
        );
        let content_click = install_modified_selection_click(
            &card,
            item,
            selection_for_setup.clone(),
            browser_for_setup.clone(),
            depth,
            positions_for_setup.clone(),
        );
        install_icons_peek(
            &card,
            item,
            peek_for_setup.clone(),
            browser_for_setup.clone(),
            source_index_for_setup.clone(),
            filtered_for_setup.clone(),
            depth,
        );
        install_list_drag_drop(
            &card,
            item,
            browser_for_setup.clone(),
            transfers_for_setup.clone(),
            depth,
            Some((source_index_for_setup.clone(), filtered_for_setup.clone())),
            (None, &content_click),
        );
        item.set_child(Some(&card));
        if let Some(parent) = card.parent() {
            parent.set_halign(gtk::Align::Center);
            parent.set_valign(gtk::Align::Start);
        }
        register_bound_mode_item(&bound_items_for_setup, item, &card, &rename_label);
    });
    let browser_for_bind = Rc::downgrade(&context.browser);
    let source_index_for_bind = context.source_index.clone();
    let cuts_for_bind = context.cuts.clone();
    let thumbnail_size_for_bind = context.thumbnail_size.clone();
    let scrolling_for_bind = context.scrolling.clone();
    let state_for_bind = context.state.clone();
    factory.connect_bind(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(card) = item.child().and_downcast::<gtk::Box>() else {
            return;
        };
        let source_position = item
            .item()
            .and_then(|value| source_index_for_bind.of_item(&value));
        let browser = browser_for_bind.upgrade();
        let state = state_for_bind.as_ref().and_then(Weak::upgrade);
        let entry = browser.as_ref().and_then(|browser| {
            source_position.and_then(|position| browser.entry_at(depth, position))
        });
        let thumbnail_size = icons_card_icon_slot(thumbnail_size_for_bind.get());
        super::icons_cell::set_slot(&card, thumbnail_size);
        if let Some(entry) = entry {
            apply_icons_entry(
                Some(item),
                &card,
                &entry,
                &cuts_for_bind.borrow(),
                thumbnail_size,
                scrolling_for_bind.get(),
                state.as_deref(),
            );
            if !scrolling_for_bind.get()
                && let Some(position) = metadata_fill_position(source_position, &entry, false)
                && let Some(browser) = browser.as_ref()
            {
                browser.request_metadata_fill(depth, position, entry.location.clone());
            }
        }
    });
    factory.connect_unbind(|_, item| super::thumbnail::cancel_list_item_thumbnails(item));
    let view = gtk::GridView::new(Some(selection.clone()), Some(factory));
    view.add_css_class("file-icons");
    view.set_vexpand(false);
    view.set_enable_rubberband(false);
    view.set_single_click_activate(false);
    configure_icons_view_density(&view, context.density.get());
    super::accessibility::describe_entry_container(
        &view,
        &pane_directory_name(&context.browser, depth),
    );

    let weak_browser = Rc::downgrade(&context.browser);
    let source_index_for_activation = context.source_index.clone();
    let filtered_for_activation = view_model.clone();
    view.connect_activate(move |_, position| {
        if let Some(browser) = weak_browser.upgrade()
            && let Some(position) = source_position_for_view(
                &source_index_for_activation,
                Some(&filtered_for_activation),
                position,
            )
        {
            browser.activate_in_place(depth, position);
        }
    });
    let section = PaneSection {
        view: view.clone().upcast(),
        view_model,
        selection,
        bound_items: bound_items.clone(),
        syncing: syncing_selection,
        visit: bound_item_visitor(bound_items),
    };
    connect_selection(
        &section,
        context.sections.clone(),
        &context.browser,
        depth,
        context.source_index.clone(),
        context.click.multiple_selection.clone(),
    );
    if let Some(state) = context.state.as_ref().and_then(Weak::upgrade) {
        install_section_context_menu(
            &state,
            &section,
            context.sections.clone(),
            &context.source_index,
            depth,
        );
    }
    section
}

fn pin_ungrouped_icons_columns(section: &PaneSection, width: i32, density: BrowserDensity) {
    if width <= 0 {
        return;
    }
    let Ok(icons) = section.view.clone().downcast::<gtk::GridView>() else {
        return;
    };
    let column = measured_card_width(section).unwrap_or(FALLBACK_ICONS_COLUMN_WIDTH);
    let columns = icons_columns_for_width(width, column, density);
    // GTK's GridView pool is max_columns*30. Leave min at 1 so cells keep natural width.
    pin_ungrouped_grid_columns(&icons, columns);
}

fn icons_columns_for_width(width: i32, column: i32, density: BrowserDensity) -> u32 {
    (width / column.max(1)).clamp(1, density_icons_columns(density) as i32) as u32
}

fn pin_ungrouped_grid_columns(icons: &gtk::GridView, columns: u32) {
    if icons.max_columns() != columns {
        icons.set_max_columns(columns.max(icons.min_columns()));
    }
}

/// GridView applies `max_columns` during allocate, then updates `page-size`.
/// `queue_resize` from inside that allocate is ignored, so a shrink reflows from
/// the new width but a grow (closing preview, hiding the sidebar, widening the
/// window) keeps the old cap. Pin on the next idle instead.
fn after_icons_viewport_width_changes(
    scroll: &gtk::ScrolledWindow,
    on_width: impl Fn(i32) + 'static,
) {
    let pending = Rc::new(Cell::new(false));
    let on_width = Rc::new(on_width);
    scroll
        .hadjustment()
        .connect_page_size_notify(move |adjustment| {
            if pending.replace(true) {
                return;
            }
            let pending = pending.clone();
            let on_width = on_width.clone();
            let adjustment = adjustment.clone();
            glib::idle_add_local_once(move || {
                pending.set(false);
                on_width(adjustment.page_size() as i32);
            });
        });
}

fn viewport_page_width(widget: &gtk::Widget) -> i32 {
    let mut ancestor = widget.parent();
    while let Some(widget) = ancestor {
        ancestor = widget.parent();
        if let Ok(scroll) = widget.downcast::<gtk::ScrolledWindow>() {
            return scroll.hadjustment().page_size() as i32;
        }
    }
    widget.width()
}

fn measured_card_width(section: &PaneSection) -> Option<i32> {
    section.bound_items.borrow().iter().find_map(|bound| {
        let widget = bound.widget.upgrade()?;
        let (_, natural, _, _) = widget.measure(gtk::Orientation::Horizontal, -1);
        (natural > 0).then_some(natural + ICONS_CARD_SPACING)
    })
}

fn refresh_marquee_targets(pane: &Pane) {
    *pane.targets.borrow_mut() = pane
        .sections
        .borrow()
        .iter()
        .map(|section| super::marquee::MarqueeTarget {
            selection: section.selection.clone(),
            visit_items: section.visit.clone(),
        })
        .collect();
}

fn install_scroll_settle(
    scroll: &gtk::ScrolledWindow,
    scrolling: Rc<Cell<bool>>,
    css_class: Option<&'static str>,
    on_settle: impl Fn() + 'static,
) {
    let pending = Rc::new(RefCell::new(None::<glib::SourceId>));
    let on_settle = Rc::new(on_settle);
    for adjustment in [scroll.vadjustment(), scroll.hadjustment()] {
        let pending = pending.clone();
        let scrolling = scrolling.clone();
        let scroll = scroll.clone();
        let on_settle = on_settle.clone();
        adjustment.connect_value_changed(move |_| {
            let started = !scrolling.replace(true);
            if started && let Some(css_class) = css_class {
                let scrolling = scrolling.clone();
                let scroll = scroll.clone();
                glib::idle_add_local_once(move || {
                    if scrolling.get() {
                        scroll.add_css_class(css_class);
                    }
                });
            }
            if let Some(source) = pending.borrow_mut().take() {
                source.remove();
            }
            let pending_for_timeout = pending.clone();
            let scrolling = scrolling.clone();
            let scroll = scroll.clone();
            let on_settle = on_settle.clone();
            pending.replace(Some(glib::timeout_add_local_once(
                SCROLL_SETTLE_DELAY,
                move || {
                    pending_for_timeout.borrow_mut().take();
                    scrolling.set(false);
                    if let Some(css_class) = css_class {
                        scroll.remove_css_class(css_class);
                    }
                    on_settle();
                },
            )));
        });
    }
}

fn resize_icons_thumbnail_slots(section: &PaneSection, size: i32) {
    let size = icons_card_icon_slot(size);
    section.bound_items.borrow().iter().for_each(|bound| {
        let Some(card) = bound.widget.upgrade().and_downcast::<gtk::Box>() else {
            return;
        };
        super::icons_cell::set_slot(&card, size);
    });
}

fn ensure_icons_card_slot(card: &gtk::Box, thumbnail_size: i32) {
    let (width, height) = icons_card_extent(thumbnail_size);
    if card.width_request() != width || card.height_request() != height {
        card.set_size_request(width, height);
    }
}

fn configure_icons_density(pane: &Pane, density: BrowserDensity) {
    if let Some(loading) = pane.stack.child_by_name("loading")
        && let Some(scroll) = loading.first_child().and_downcast::<gtk::ScrolledWindow>()
        && let Some(icons) = scroll.child().and_downcast::<gtk::GridView>()
    {
        configure_icons_view_density(&icons, density);
    }
    if let Some(context) = pane.icons.as_ref() {
        context.density.set(density);
    }
    for section in pane.all_sections() {
        if let Ok(icons) = section.view.clone().downcast::<gtk::GridView>() {
            configure_icons_view_density(&icons, density);
        }
    }
    for section in pane.all_sections() {
        pin_ungrouped_icons_columns(&section, viewport_page_width(&section.view), density);
    }
}

fn configure_icons_view_density(icons: &gtk::GridView, density: BrowserDensity) {
    icons.set_min_columns(1);
    icons.set_max_columns(density_icons_columns(density));
}

fn density_icons_columns(density: BrowserDensity) -> u32 {
    match density {
        BrowserDensity::Compact => 20,
        BrowserDensity::Airy => 16,
    }
}

fn list_headings(browser: &Rc<Browser>, depth: usize, columns: ListColumnLayout) -> gtk::Box {
    let headings = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    headings.add_css_class("list-headings");
    let preferences = browser.column_preferences(depth).unwrap_or_default();
    let sorting = Rc::new(Cell::new((
        preferences.sort_key,
        preferences.sort_direction,
    )));
    let arrows: Rc<RefCell<Vec<(SortKey, gtk::Image)>>> = Rc::new(RefCell::new(Vec::new()));

    for (index, (text, key, width)) in [
        ("Name", Some(SortKey::Name), LIST_COLUMN_WIDTHS[0]),
        ("Mode", None, LIST_COLUMN_WIDTHS[1]),
        ("Size", Some(SortKey::Size), LIST_COLUMN_WIDTHS[2]),
        ("Type", Some(SortKey::Type), LIST_COLUMN_WIDTHS[3]),
        ("Modified", Some(SortKey::Modified), LIST_COLUMN_WIDTHS[4]),
    ]
    .into_iter()
    .enumerate()
    {
        let cell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        cell.add_css_class("list-heading-cell");
        register_list_column_cell(&columns, index, &cell);

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 5);
        let label = gtk::Label::new(Some(text));
        label.set_xalign(0.0);
        label.set_hexpand(true);
        let arrow = crate::assets::primary_icon(
            if preferences.sort_direction == SortDirection::Ascending {
                crate::assets::icons::ARROW_UP
            } else {
                crate::assets::icons::ARROW_DOWN
            },
            12,
        );
        arrow.set_visible(key.is_some_and(|k| preferences.sort_key == k));
        row.append(&label);
        row.append(&arrow);
        let button = gtk::Button::builder().child(&row).build();
        button.add_css_class("list-heading-button");
        button.set_hexpand(true);
        if let Some(key) = key {
            let weak_browser = Rc::downgrade(browser);
            let sorting_for_click = sorting.clone();
            let arrows_for_click = arrows.clone();
            button.connect_clicked(move |_| {
                let (current_key, current_direction) = sorting_for_click.get();
                let direction = if current_key == key {
                    match current_direction {
                        SortDirection::Ascending => SortDirection::Descending,
                        SortDirection::Descending => SortDirection::Ascending,
                    }
                } else {
                    SortDirection::Ascending
                };
                sorting_for_click.set((key, direction));
                for (arrow_key, arrow) in arrows_for_click.borrow().iter() {
                    arrow.set_visible(*arrow_key == key);
                    if *arrow_key == key {
                        crate::assets::set_primary_icon(
                            arrow,
                            if direction == SortDirection::Ascending {
                                crate::assets::icons::ARROW_UP
                            } else {
                                crate::assets::icons::ARROW_DOWN
                            },
                        );
                    }
                }
                if let Some(browser) = weak_browser.upgrade() {
                    browser.set_sort(depth, key, direction);
                }
            });
            arrows.borrow_mut().push((key, arrow));
        }
        let button_overlay = gtk::Overlay::new();
        button_overlay.set_child(Some(&button));
        button_overlay.set_hexpand(true);
        button_overlay.add_overlay(&column_resize_handle(columns.clone(), index, width));
        cell.append(&button_overlay);
        headings.append(&cell);
    }
    headings
}

fn register_list_column_cell(
    columns: &ListColumnLayout,
    index: usize,
    widget: &impl IsA<gtk::Widget>,
) {
    widget.set_width_request(columns.widths[index].get());
    // Until the user resizes it, Name absorbs space left after the fixed metadata columns.
    widget.set_hexpand(index == 0 && !columns.name_manually_resized.get());
    let weak = glib::WeakRef::new();
    weak.set(Some(widget.upcast_ref()));
    columns.cells[index].borrow_mut().push(weak);
}

fn set_list_column_width(columns: &ListColumnLayout, index: usize, width: i32) {
    columns.widths[index].set(width);
    if index == 0 {
        columns.name_manually_resized.set(true);
    }
    columns.cells[index].borrow_mut().retain(|weak| {
        let Some(widget) = weak.upgrade() else {
            return false;
        };
        widget.set_width_request(width);
        if index == 0 {
            widget.set_hexpand(false);
        }
        true
    });
}

fn column_resize_handle(columns: ListColumnLayout, index: usize, initial_width: i32) -> gtk::Box {
    let handle = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    handle.add_css_class("list-column-resize-handle");
    handle.set_width_request(7);
    handle.set_halign(gtk::Align::End);
    handle.set_valign(gtk::Align::Fill);
    handle.set_cursor_from_name(Some("col-resize"));
    let resize = gtk::GestureDrag::new();
    resize.set_button(1);
    let starting_width = Rc::new(Cell::new(initial_width));
    let pointer_start = Rc::new(Cell::new(None::<f64>));
    let last_press = Rc::new(Cell::new(0u64));
    let starting_for_begin = starting_width.clone();
    let pointer_for_begin = pointer_start.clone();
    let last_press_for_begin = last_press.clone();
    let columns_for_begin = columns.clone();
    let columns_for_autofit = columns.clone();
    resize.connect_drag_begin(move |gesture, _, _| {
        let now = glib::monotonic_time() as u64;
        let prev = last_press_for_begin.get();
        last_press_for_begin.set(now);
        if now.wrapping_sub(prev) <= 400_000 {
            let natural = columns_for_autofit.cells[index]
                .borrow()
                .iter()
                .filter_map(glib::WeakRef::upgrade)
                .map(|widget| super::browser::max_child_natural_width(&widget))
                .max()
                .unwrap_or(initial_width);
            set_list_column_width(
                &columns_for_autofit,
                index,
                list_column_width(index, natural),
            );
            gesture.set_state(gtk::EventSequenceState::Denied);
            return;
        }
        let width = columns_for_begin.cells[index]
            .borrow()
            .iter()
            .find_map(glib::WeakRef::upgrade)
            .map_or(initial_width, |widget| widget.width());
        starting_for_begin.set(list_column_width(index, width));
        pointer_for_begin.set(
            gesture
                .current_event()
                .and_then(|event| event.position())
                .map(|(pointer_x, _)| pointer_x),
        );
        gesture.set_state(gtk::EventSequenceState::Claimed);
    });
    let columns_for_update = columns.clone();
    resize.connect_drag_update(move |gesture, fallback_offset_x, _| {
        let pointer_x = gesture
            .current_event()
            .and_then(|event| event.position())
            .map(|(pointer_x, _)| pointer_x);
        let offset_x = pointer_start
            .get()
            .zip(pointer_x)
            .map_or(fallback_offset_x, |(start, current)| current - start);
        let width = (f64::from(starting_width.get()) + offset_x).round() as i32;
        set_list_column_width(&columns_for_update, index, list_column_width(index, width));
    });
    handle.add_controller(resize);
    handle
}

fn list_column_width(index: usize, width: i32) -> i32 {
    width.max(LIST_COLUMN_MIN_WIDTHS[index])
}

fn list_navigation(browser: &Rc<Browser>) -> gtk::Box {
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    actions.add_css_class("list-navigation");
    for (icon, tooltip, action, available) in [
        (
            crate::assets::icons::ARROW_LEFT,
            "Back (Alt+Left)",
            Browser::back as fn(&Rc<Browser>),
            browser.can_go_back(),
        ),
        (
            crate::assets::icons::ARROW_RIGHT,
            "Forward (Alt+Right)",
            Browser::forward as fn(&Rc<Browser>),
            browser.can_go_forward(),
        ),
        (
            crate::assets::icons::ARROW_UP,
            "Parent folder (Alt+Up)",
            Browser::parent as fn(&Rc<Browser>),
            browser.can_go_parent(),
        ),
    ] {
        let button = gtk::Button::builder()
            .tooltip_text(tooltip)
            .sensitive(available)
            .build();
        button.set_child(Some(&crate::assets::chrome_icon(icon)));
        button.add_css_class("list-navigation-button");
        let weak_browser = Rc::downgrade(browser);
        button.connect_clicked(move |_| {
            if let Some(browser) = weak_browser.upgrade() {
                action(&browser);
            }
        });
        actions.append(&button);
    }
    actions
}

fn build_list_pane(
    browser: Rc<Browser>,
    click_options: ModeClickOptions,
    transfer_handler: TransferHandlerSlot,
    cut_locations: Rc<RefCell<HashSet<Location>>>,
    options: ListOptions,
    depth: usize,
    title: &str,
) -> Pane {
    let navigation = list_navigation(&browser);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    actions.add_css_class("icons-header-actions");
    let empty_trash = super::browser::empty_trash_button(&browser);
    let is_trash = browser
        .location_at(depth)
        .is_some_and(|location| super::browser::is_trash_root(&location));
    empty_trash.set_visible(is_trash);
    empty_trash.set_sensitive(false);
    actions.append(&empty_trash);
    if let Some(state) = options.new_folder_state.as_ref() {
        actions.append(&super::browser::pane_new_folder_button(
            state.clone(),
            depth,
        ));
    }
    actions.append(&super::browser::pane_refresh_button(&browser, depth));
    let (filter_entry, filter_revealer, filter_button) = filter_controls("Filter list (Ctrl+F)");
    actions.append(&filter_button);
    let columns = ListColumnLayout::new();
    let (shell, header, content, model, stack, status, spinner, truncated_hint) = pane_base(
        title,
        BrowserMode::List,
        "list-pane",
        &list_loading_skeleton(&columns),
        Some(navigation.upcast()),
        Some(actions.upcast()),
    );
    let source_index = SourceIndexMap::watch(&model);
    if let Some(destination) = browser.location_at(depth) {
        install_mode_directory_drop_target(&stack, destination, transfer_handler.clone());
    }
    content.append(&filter_revealer);
    let filter_query = Rc::new(RefCell::new(String::new()));
    let initial_show_hidden = browser
        .column_preferences(depth)
        .map_or_else(|| browser.preferences().show_hidden, |p| p.show_hidden);
    let show_hidden = Rc::new(Cell::new(initial_show_hidden));
    let filter = super::browser::entry_filter(show_hidden.clone(), filter_query.clone());
    let filtered_model = gtk::FilterListModel::new(Some(model.clone()), Some(filter.clone()));
    let filter_for_pane = filter.clone();
    let query_for_filter = filter_query.clone();
    let filter_for_settled = filter.clone();
    super::browser::debounce_filter_entry(&filter_entry, move |text| {
        super::browser::notify_filter_query(&filter_for_settled, &query_for_filter, text);
    });
    let view_model =
        gtk::SortListModel::new(Some(filtered_model.clone()), None::<gtk::CustomSorter>);
    if options.group_by_type {
        let sorter = type_group_sorter();
        view_model.set_sorter(Some(&sorter));
        view_model.set_section_sorter(Some(&sorter));
    }
    let view_model_object = view_model.clone().upcast::<gio::ListModel>();
    let selection = gtk::MultiSelection::new(Some(view_model.clone()));
    let syncing_selection = Rc::new(Cell::new(false));
    let sections: Rc<RefCell<Vec<PaneSection>>> = Rc::new(RefCell::new(Vec::new()));

    let headings = list_headings(&browser, depth, columns.clone());

    let bound_items = Rc::new(RefCell::new(Vec::new()));
    let scrolling = Rc::new(Cell::new(false));
    let factory = ListFactory {
        browser: Rc::downgrade(&browser),
        depth,
        positions: PanePositions {
            index: source_index.clone(),
            view: view_model_object.clone(),
        },
        selection: selection.clone(),
        previews: click_options.previews,
        activation: click_options.activation,
        transfers: transfer_handler.clone(),
        cuts: cut_locations.clone(),
        columns,
        scrolling: scrolling.clone(),
        bound_items: bound_items.clone(),
        state: options.state.clone(),
        filter_query: filter_query.clone(),
    }
    .build();
    let view = gtk::ListView::new(Some(selection.clone()), Some(factory));
    view.add_css_class("file-list-mode");
    super::accessibility::describe_entry_container(&view, &pane_directory_name(&browser, depth));
    if options.group_by_type {
        view.set_header_factory(Some(&type_group_header_factory()));
    }
    view.set_enable_rubberband(false);
    view.set_vexpand(true);
    // GTK bundles single-click activation with hover selection, which collapses
    // multi-selection. Per-row gestures honor the configured click behavior instead.
    view.set_single_click_activate(false);
    let weak_browser = Rc::downgrade(&browser);
    let source_index_for_activation = source_index.clone();
    let view_model_for_activation = view_model_object.clone();
    view.connect_activate(move |_, position| {
        if let Some(browser) = weak_browser.upgrade()
            && let Some(position) = source_position_for_view(
                &source_index_for_activation,
                Some(&view_model_for_activation),
                position,
            )
        {
            browser.activate_in_place(depth, position);
        }
    });
    let section = PaneSection {
        view: view.clone().upcast(),
        view_model: view_model_object,
        selection,
        bound_items: bound_items.clone(),
        syncing: syncing_selection,
        visit: bound_item_visitor(bound_items),
    };
    let browser_for_filter_activate = Rc::downgrade(&browser);
    let selection_for_filter_activate = section.selection.clone();
    let view_for_filter_activate = section.view_model.clone();
    let source_index_for_filter_activate = source_index.clone();
    let filter_query_for_activate = filter_query.clone();
    filter_entry.connect_activate(move |_| {
        if filter_query_for_activate.borrow().trim().is_empty() {
            return;
        }
        activate_filtered_item(
            &browser_for_filter_activate,
            &selection_for_filter_activate,
            &view_for_filter_activate,
            &source_index_for_filter_activate,
            depth,
        );
    });
    sections.borrow_mut().push(section.clone());
    connect_selection(
        &section,
        Rc::downgrade(&sections),
        &browser,
        depth,
        source_index.clone(),
        click_options.multiple_selection,
    );
    if let Some(state) = options.state.as_ref().and_then(Weak::upgrade) {
        install_section_context_menu(
            &state,
            &section,
            Rc::downgrade(&sections),
            &source_index,
            depth,
        );
    }
    let scroll = gtk::ScrolledWindow::builder()
        .child(&view)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();
    scroll.add_css_class("fixed-scrollbar");
    scroll.add_css_class("browser-listing-scroll");
    let browser_for_settle = Rc::downgrade(&browser);
    let source_index_for_settle = source_index.clone();
    let sections_for_settle = Rc::downgrade(&sections);
    let cuts_for_settle = cut_locations.clone();
    install_scroll_settle(&scroll, scrolling, Some("list-fast-scroll"), move || {
        let Some(browser) = browser_for_settle.upgrade() else {
            return;
        };
        let Some(sections) = sections_for_settle.upgrade() else {
            return;
        };
        let cuts = cuts_for_settle.borrow();
        for section in sections.borrow().iter() {
            refresh_list_section(&browser, depth, &source_index_for_settle, section, &cuts);
        }
    });
    let table = gtk::Box::new(gtk::Orientation::Vertical, 0);
    table.set_vexpand(true);
    table.append(&headings);
    let targets: super::marquee::MarqueeTargets = Rc::new(RefCell::new(Vec::new()));
    let (collection, marquee) = collection_with_marquee(view.upcast_ref(), scroll, targets.clone());
    table.append(&collection);
    marquee.add_origin_surface(&header);
    marquee.add_origin_surface(&headings);
    let table_scroll = gtk::ScrolledWindow::builder()
        .child(&table)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .hexpand(true)
        .vexpand(true)
        .build();
    table_scroll.add_css_class("fixed-scrollbar");
    if let Some(viewport) = table_scroll.child().and_downcast::<gtk::Viewport>() {
        // The outer viewport must not horizontally reveal oversized metadata rows; the inner
        // ListView still reveals focused rows vertically.
        viewport.set_scroll_to_focus(false);
    }
    let search = super::inline_search::wrap(
        &table_scroll,
        &filter_entry,
        browser
            .location_at(depth)
            .and_then(|location| location.native_path().map(std::path::Path::to_path_buf)),
        &browser,
    );
    content.append(&search.widget);
    let pane = Pane {
        depth,
        shell,
        header,
        model,
        source_index,
        filter_model: Some(filtered_model),
        section,
        sections,
        icons: None,
        targets,
        detached: Rc::new(Cell::new(false)),
        loading: super::loading_skeleton::DelayedLoading::new(&stack),
        stack,
        status,
        spinner,
        truncated_hint,
        marquee,
        search,
        filter_entry: Some(filter_entry),
        filter_button: Some(filter_button),
        empty_trash_button: is_trash.then_some(empty_trash),
        show_hidden,
        filter: filter_for_pane,
    };
    refresh_marquee_targets(&pane);
    pane
}

fn icons_loading_skeleton(thumbnail_size: i32, density: BrowserDensity) -> gtk::Box {
    use super::loading_skeleton::{block, container, name_width, scroll};

    let skeleton = container();
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let card = gtk::Box::new(gtk::Orientation::Vertical, 3);
        card.add_css_class("icons-card");
        card.set_halign(gtk::Align::Fill);
        ensure_icons_card_slot(&card, thumbnail_size);
        let slot = icons_card_icon_slot(thumbnail_size);
        let icon = block(slot, slot);
        icon.set_halign(gtk::Align::Center);
        card.append(&icon);
        let label = block(96, 10);
        label.set_halign(gtk::Align::Center);
        label.set_margin_top(3);
        card.append(&label);
        item.set_child(Some(&card));
    });
    factory.connect_bind(|_, item| {
        if let Some(item) = item.downcast_ref::<gtk::ListItem>()
            && let Some(card) = item.child()
            && let Some(label) = card.last_child()
        {
            label.set_width_request(name_width(item.position()));
        }
    });
    let model = gtk::StringList::new(&[""; 60]);
    let icons = gtk::GridView::new(Some(gtk::NoSelection::new(Some(model))), Some(factory));
    icons.add_css_class("file-icons");
    icons.set_valign(gtk::Align::Start);
    configure_icons_view_density(&icons, density);
    let scroll = scroll(&icons);
    scroll.set_vexpand(true);
    skeleton.append(&scroll);
    skeleton
}

fn list_loading_skeleton(columns: &ListColumnLayout) -> gtk::Box {
    use super::loading_skeleton::{ROW_COUNT, block, container, name_width, scroll};

    let skeleton = container();
    let table = gtk::Box::new(gtk::Orientation::Vertical, 0);
    table.set_valign(gtk::Align::Start);
    let headings = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    headings.add_css_class("list-headings");
    for (index, width) in [40, 36, 28, 30, 58].into_iter().enumerate() {
        let cell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        cell.add_css_class("list-heading-cell");
        let bar = block(width, 8);
        bar.set_margin_start(12);
        cell.append(&bar);
        register_list_column_cell(columns, index, &cell);
        headings.append(&cell);
    }
    table.append(&headings);
    for index in 0..ROW_COUNT {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        row.add_css_class("list-row");
        for (column, width) in [name_width(index).min(92), 60, 38, 56, 94]
            .into_iter()
            .enumerate()
        {
            let cell = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            cell.add_css_class("list-metadata-cell");
            if column == 0 {
                cell.append(&block(18, 18));
            }
            cell.append(&block(width, 10));
            register_list_column_cell(columns, column, &cell);
            row.append(&cell);
        }
        table.append(&row);
    }
    let scroll = scroll(&table);
    scroll.set_vexpand(true);
    skeleton.append(&scroll);
    skeleton
}

fn pane_base(
    title: &str,
    mode: BrowserMode,
    class: &str,
    loading: &gtk::Box,
    header_leading: Option<gtk::Widget>,
    header_actions: Option<gtk::Widget>,
) -> (
    gtk::Box,
    gtk::Box,
    gtk::Box,
    gtk::StringList,
    gtk::Stack,
    gtk::Label,
    gtk::Spinner,
    gtk::Image,
) {
    let shell = super::accessibility::pane_box();
    shell.add_css_class(class);
    shell.set_hexpand(true);
    shell.set_vexpand(true);
    super::accessibility::describe_pane(&shell, title, mode);
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    header.add_css_class("mode-pane-header");
    let heading_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    heading_box.set_hexpand(true);
    heading_box.set_valign(gtk::Align::Center);
    let heading = gtk::Label::new(Some(title));
    heading.set_xalign(0.0);
    let spinner = gtk::Spinner::new();
    spinner.set_valign(gtk::Align::Center);
    spinner.start();
    let truncated_hint = crate::assets::primary_icon(crate::assets::icons::TRIANGLE_ALERT, 16);
    truncated_hint.set_tooltip_text(Some(
        "This directory has more entries than could be loaded; showing a partial listing.",
    ));
    truncated_hint.set_visible(false);
    heading_box.append(&heading);
    heading_box.append(&truncated_hint);
    if let Some(leading) = header_leading {
        leading.set_valign(gtk::Align::Center);
        header.append(&leading);
    }
    header.append(&heading_box);
    header.append(&spinner);
    if let Some(actions) = header_actions {
        actions.set_valign(gtk::Align::Center);
        header.append(&actions);
    }
    shell.append(&header);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.set_hexpand(true);
    content.set_vexpand(true);
    let status = gtk::Label::new(Some("This directory is empty"));
    status.add_css_class("status-message");
    status.set_wrap(true);
    let stack = gtk::Stack::builder()
        .hexpand(true)
        .vexpand(true)
        .focusable(true)
        .build();
    stack.add_named(&content, Some("content"));
    stack.add_named(loading, Some("loading"));
    stack.add_named(&status, Some("status"));
    shell.append(&stack);

    let model = gtk::StringList::new(&[]);
    (
        shell,
        header,
        content,
        model,
        stack,
        status,
        spinner,
        truncated_hint,
    )
}

fn register_bound_mode_item(
    items: &Rc<RefCell<Vec<BoundModeItem>>>,
    item: &gtk::ListItem,
    widget: &impl IsA<gtk::Widget>,
    rename_label: &impl IsA<gtk::Widget>,
) {
    let weak_item = glib::WeakRef::new();
    weak_item.set(Some(item));
    let weak_widget = glib::WeakRef::new();
    weak_widget.set(Some(widget.upcast_ref()));
    let weak_rename_label = glib::WeakRef::new();
    weak_rename_label.set(Some(rename_label.upcast_ref()));
    items.borrow_mut().push(BoundModeItem {
        item: weak_item,
        widget: weak_widget,
        rename_label: weak_rename_label,
    });
}

fn collection_with_marquee(
    view: &gtk::Widget,
    scroll: gtk::ScrolledWindow,
    targets: super::marquee::MarqueeTargets,
) -> (gtk::Overlay, super::marquee::Marquee) {
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&scroll));
    overlay.set_hexpand(true);
    overlay.set_vexpand(true);
    super::scrolling::install_autoscroll(&scroll, &overlay);

    let marquee = super::marquee::install(super::marquee::MarqueeSetup {
        view: view.clone(),
        surface: scroll.clone().upcast(),
        scroll,
        overlay: overlay.clone(),
        targets: targets.clone(),
        is_item: Rc::new(super::pointer::hits_item_content),
        clear_selection: Rc::new(move || {
            let selections: Vec<_> = targets
                .borrow()
                .iter()
                .map(|target| target.selection.clone())
                .collect();
            for selection in selections {
                selection.unselect_all();
            }
        }),
    });
    (overlay, marquee)
}

fn descendant_with_class(widget: &gtk::Widget, class: &str) -> Option<gtk::Widget> {
    if widget.has_css_class(class) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(found) = descendant_with_class(&widget, class) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

fn install_icons_peek(
    card: &impl IsA<gtk::Widget>,
    item: &gtk::ListItem,
    state: Option<Weak<super::browser::ViewState>>,
    browser: Weak<Browser>,
    source_index: SourceIndexMap,
    filtered: gio::ListModel,
    depth: usize,
) {
    let Some(state) = state else {
        return;
    };
    let card = card.as_ref();
    let motion = gtk::EventControllerMotion::new();
    let entered_item = item.downgrade();
    let state_for_enter = state.clone();
    motion.connect_enter(move |controller, _, _| {
        let Some(entered_item) = entered_item.upgrade() else {
            return;
        };
        let position = entered_item.position();
        if position == gtk::INVALID_LIST_POSITION {
            return;
        }
        let source_position = source_position_for_view(&source_index, Some(&filtered), position);
        let entry = browser.upgrade().and_then(|browser| {
            source_position.and_then(|position| browser.entry_at(depth, position))
        });
        if let (Some(state), Some(entry), Some(anchor)) =
            (state_for_enter.upgrade(), entry, controller.widget())
            && entry.is_directory()
        {
            state.schedule_peek(depth, entry.location, anchor);
        }
    });
    motion.connect_leave(move |_| {
        if let Some(state) = state.upgrade() {
            state.schedule_close_peek();
        }
    });
    card.add_controller(motion);
}

fn install_mode_directory_drop_target(
    widget: &impl IsA<gtk::Widget>,
    destination: Location,
    transfer_handler: TransferHandlerSlot,
) {
    if transfer_handler.borrow().is_none() || is_trash_location(&destination) {
        return;
    }
    widget.add_css_class("file-drop-zone");
    let drop = gtk::DropTarget::new(
        gtk::gdk::FileList::static_type(),
        gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE,
    );
    drop.connect_enter(|target, _, _| super::browser::file_drop_action(target));
    drop.connect_motion(|target, _, _| super::browser::file_drop_action(target));
    drop.connect_drop(move |target, value, _, _| {
        let Some(sources) = super::browser::locations_from_file_list_value(value) else {
            return false;
        };
        let Some(handler) = transfer_handler.borrow().clone() else {
            return false;
        };
        handler(
            destination.clone(),
            sources,
            super::browser::file_drop_action(target) == gtk::gdk::DragAction::MOVE,
        );
        true
    });
    widget.add_controller(drop);
}

fn install_list_drag_drop(
    row: &impl IsA<gtk::Widget>,
    item: &gtk::ListItem,
    browser: Weak<Browser>,
    transfer_handler: TransferHandlerSlot,
    depth: usize,
    position_map: Option<(SourceIndexMap, gio::ListModel)>,
    drag_icon_and_content_click: (Option<&gtk::Widget>, &gtk::GestureClick),
) {
    let (drag_icon, content_click) = drag_icon_and_content_click;
    if transfer_handler.borrow().is_none() {
        return;
    }
    let row = row.as_ref();
    let drag = gtk::DragSource::builder()
        .actions(gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE)
        .build();
    drag.set_propagation_phase(gtk::PropagationPhase::Capture);
    let dragged_item = item.downgrade();
    let browser_for_drag = browser.clone();
    let map_for_drag = position_map.clone();
    let drag_icon = drag_icon.map(gtk::Widget::downgrade);
    drag.connect_prepare(move |source, x, y| {
        if !super::pointer::hits_item_content(&source.widget()?, x, y) {
            return None;
        }
        source.set_actions(super::browser::drag_actions_for_modifiers(
            source.current_event_state(),
        ));
        let browser = browser_for_drag.upgrade()?;
        let dragged_item = dragged_item.upgrade()?;
        let position = dragged_item.position();
        if position == gtk::INVALID_LIST_POSITION {
            return None;
        }
        let position = map_for_drag
            .as_ref()
            .map_or(Some(position as usize), |(source, filtered)| {
                source_position_for_view(source, Some(filtered), position)
            })?;
        let entry = browser.entry_at(depth, position)?;
        let selected = browser.selected_entries();
        let entries = if selected
            .iter()
            .any(|selected| selected.location == entry.location)
        {
            selected
        } else {
            vec![entry]
        };
        let compact_icon = drag_icon.as_ref().and_then(glib::WeakRef::upgrade);
        let fallback_icon = source.widget();
        let paintable = gtk::WidgetPaintable::new(compact_icon.as_ref().or(fallback_icon.as_ref()));
        let (hot_x, hot_y) = if compact_icon.is_some() {
            (0, 0)
        } else {
            (x.round() as i32, y.round() as i32)
        };
        source.set_icon(Some(&paintable), hot_x, hot_y);
        super::browser::file_drag_content(&entries)
    });
    let dragged_row = row.downgrade();
    drag.connect_drag_begin(move |_, _| {
        if let Some(row) = dragged_row.upgrade() {
            row.add_css_class("dragging");
        }
    });
    let dragged_row = row.downgrade();
    drag.connect_drag_end(move |source, _, _| {
        source.set_actions(gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE);
        if let Some(row) = dragged_row.upgrade() {
            row.remove_css_class("dragging");
        }
    });
    row.add_controller(drag.clone());
    // Grouping requires both gestures already attached; claiming the modifier-click
    // on content must not deny this drag before it reaches the threshold.
    drag.group_with(content_click);

    let drop = gtk::DropTarget::new(
        gtk::gdk::FileList::static_type(),
        gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE,
    );
    let highlighted_row = row.downgrade();
    drop.connect_enter(move |target, _, _| {
        if let Some(row) = highlighted_row.upgrade() {
            row.add_css_class("drop-destination");
        }
        super::browser::file_drop_action(target)
    });
    let highlighted_row = row.downgrade();
    drop.connect_motion(move |target, _, _| {
        if let Some(row) = highlighted_row.upgrade() {
            row.add_css_class("drop-destination");
        }
        super::browser::file_drop_action(target)
    });
    let highlighted_row = row.downgrade();
    drop.connect_leave(move |_| {
        if let Some(row) = highlighted_row.upgrade() {
            row.remove_css_class("drop-destination");
        }
    });
    let accepted_item = item.downgrade();
    let browser_for_accept = browser.clone();
    let map_for_accept = position_map.clone();
    drop.connect_accept(move |_, offered| {
        let Some(browser) = browser_for_accept.upgrade() else {
            return false;
        };
        let Some(accepted_item) = accepted_item.upgrade() else {
            return false;
        };
        let position = accepted_item.position();
        let position = map_for_accept.as_ref().map_or(
            (position != gtk::INVALID_LIST_POSITION).then_some(position as usize),
            |(map, view)| source_position_for_view(map, Some(view), position),
        );
        position.is_some()
            && browser
                .entry_at(depth, position.unwrap_or_default())
                .is_some_and(|entry| entry.is_directory() && !is_trash_location(&entry.location))
            && offered
                .formats()
                .contains_type(gtk::gdk::FileList::static_type())
    });
    let dropped_item = item.downgrade();
    let browser_for_drop = browser;
    let map_for_drop = position_map;
    let dropped_row = row.downgrade();
    drop.connect_drop(move |target, value, _, _| {
        if let Some(row) = dropped_row.upgrade() {
            row.remove_css_class("drop-destination");
        }
        let Some(browser) = browser_for_drop.upgrade() else {
            return false;
        };
        let Some(dropped_item) = dropped_item.upgrade() else {
            return false;
        };
        let position = dropped_item.position();
        let position = map_for_drop.as_ref().map_or(
            (position != gtk::INVALID_LIST_POSITION).then_some(position as usize),
            |(map, view)| source_position_for_view(map, Some(view), position),
        );
        let Some(destination) = position
            .and_then(|position| browser.entry_at(depth, position))
            .filter(FileEntry::is_directory)
            .map(|entry| entry.location)
        else {
            return false;
        };
        let Some(sources) = super::browser::locations_from_file_list_value(value) else {
            return false;
        };
        let Some(handler) = transfer_handler.borrow().clone() else {
            return false;
        };
        handler(
            destination,
            sources,
            super::browser::file_drop_action(target) == gtk::gdk::DragAction::MOVE,
        );
        true
    });
    row.add_controller(drop);
}

#[derive(Clone)]
struct PanePositions {
    index: SourceIndexMap,
    view: gio::ListModel,
}

impl PanePositions {
    fn source_position(&self, view_position: u32) -> Option<usize> {
        source_position_for_view(&self.index, Some(&self.view), view_position)
    }

    fn view_position(&self, source_position: usize) -> Option<u32> {
        let guessed = source_position as u32;
        // Unfiltered views keep source order; a non-source prefix shifts positions.
        [guessed, guessed.saturating_add(1)]
            .into_iter()
            .find(|candidate| self.source_position(*candidate) == Some(source_position))
            .or_else(|| {
                (0..self.view.n_items())
                    .find(|candidate| self.source_position(*candidate) == Some(source_position))
            })
    }
}

fn install_modified_selection_click(
    widget: &impl IsA<gtk::Widget>,
    item: &gtk::ListItem,
    selection: gtk::MultiSelection,
    browser: Weak<Browser>,
    depth: usize,
    positions: PanePositions,
) -> gtk::GestureClick {
    let click = gtk::GestureClick::new();
    click.set_button(1);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let item = item.downgrade();
    click.connect_pressed(move |gesture, _, x, y| {
        let Some(item) = item.upgrade() else {
            return;
        };
        let position = item.position();
        if position == gtk::INVALID_LIST_POSITION {
            return;
        }
        let Some(browser) = browser.upgrade() else {
            return;
        };
        let modifiers = gesture.current_event_state();
        let control = modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK);
        let shift = modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK);
        if shift {
            let anchor = browser
                .selection_anchor_position(depth)
                .and_then(|anchor| positions.view_position(anchor))
                .unwrap_or(position);
            let start = anchor.min(position);
            let count = anchor.max(position).saturating_sub(start) + 1;
            selection.select_range(start, count, true);
        } else if control {
            anchor_at(&browser, depth, &positions, position);
            if selection.is_selected(position) {
                selection.unselect_item(position);
            } else {
                selection.select_item(position, false);
            }
        } else {
            anchor_at(&browser, depth, &positions, position);
            return;
        }
        if let Some(widget) = gesture.widget()
            && super::pointer::hits_item_content(&widget, x, y)
        {
            if let Some(item_widget) = widget.parent() {
                item_widget.grab_focus();
            }
            gesture.set_state(gtk::EventSequenceState::Claimed);
        }
    });
    click.connect_released(|gesture, _, _, _| {
        if gesture
            .current_event_state()
            .intersects(gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK)
        {
            gesture.set_state(gtk::EventSequenceState::Claimed);
        }
    });
    widget.add_controller(click.clone());
    click
}

fn anchor_at(browser: &Rc<Browser>, depth: usize, positions: &PanePositions, view_position: u32) {
    if let Some(source_position) = positions.source_position(view_position) {
        browser.set_selection_anchor(depth, source_position);
    }
}

fn source_position_for_view(
    map: &SourceIndexMap,
    view: Option<&gio::ListModel>,
    position: u32,
) -> Option<usize> {
    let Some(view) = view else {
        return Some(position as usize);
    };
    map.of_view_position(view, position)
}

fn metadata_fill_position(
    position: Option<usize>,
    entry: &FileEntry,
    include_mode: bool,
) -> Option<usize> {
    position.filter(|_| {
        super::browser::metadata_needs_fill(entry)
            || (include_mode && entry.mode == MetadataValue::Unknown)
    })
}

fn view_position_for_source(
    source: &gtk::StringList,
    filtered: Option<&gio::ListModel>,
    position: usize,
) -> Option<u32> {
    let Some(filtered) = filtered else {
        return Some(position as u32);
    };
    let item = source.item(position as u32)?;
    let guessed = position as u32;
    // Unfiltered views keep source order.
    if filtered.item(guessed).is_some_and(|value| value == item) {
        return Some(guessed);
    }
    let shifted = guessed.saturating_add(1);
    if filtered.item(shifted).is_some_and(|value| value == item) {
        return Some(shifted);
    }
    (0..filtered.n_items())
        .find(|candidate| filtered.item(*candidate).is_some_and(|value| value == item))
}

fn activate_filtered_item(
    browser: &Weak<Browser>,
    selection: &gtk::MultiSelection,
    view: &gio::ListModel,
    source_index: &SourceIndexMap,
    depth: usize,
) {
    let Some(position) = (0..selection.n_items()).find(|position| selection.is_selected(*position))
    else {
        return;
    };
    let Some(source_position) = source_index.of_view_position(view, position) else {
        return;
    };
    let Some(browser) = browser.upgrade() else {
        return;
    };
    browser.select(depth, source_position);
    let Some(entry) = browser.entry_at(depth, source_position) else {
        return;
    };
    if browser.is_chooser_mode() && !entry.is_directory() {
        browser.open_location(entry.location);
    } else {
        browser.activate_in_place(depth, source_position);
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "mode-specific click setup keeps these inputs explicit"
)]
fn install_preview_click(
    widget: &impl IsA<gtk::Widget>,
    item: &gtk::ListItem,
    browser: Weak<Browser>,
    enabled: Rc<Cell<bool>>,
    click_activation: Rc<Cell<ClickActivation>>,
    depth: usize,
    position_map: Option<(SourceIndexMap, gio::ListModel)>,
    filter_query: Rc<RefCell<String>>,
) {
    let click = gtk::GestureClick::new();
    click.set_button(1);
    let clicked_item = item.downgrade();
    super::pointer::connect_click_release(&click, item, move |gesture, press_count| {
        let modifiers = gesture.current_event_state();
        if modifiers
            .intersects(gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK)
        {
            return;
        }
        let Some(item) = clicked_item.upgrade() else {
            return;
        };
        let position = item.position();
        if position == gtk::INVALID_LIST_POSITION {
            return;
        }
        let source_position = position_map
            .as_ref()
            .map_or(Some(position as usize), |(source, filtered)| {
                source_position_for_view(source, Some(filtered), position)
            });
        let Some(browser) = browser.upgrade() else {
            return;
        };
        let Some(position) = source_position else {
            return;
        };
        let Some(entry) = browser.entry_at(depth, position) else {
            return;
        };
        if should_activate_filtered_pointer(press_count, &filter_query) {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            if press_count == 1 {
                browser.select(depth, position);
                if !browser.is_chooser_mode() {
                    browser.activate_in_place(depth, position);
                }
            }
            return;
        }
        if should_activate_pointer_click(press_count, entry.is_directory(), click_activation.get())
        {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            browser.activate_in_place(depth, position);
        } else if press_count == 1
            && enabled.get()
            && !entry.is_directory()
            && super::preview::entry_supports_quick_preview(&entry)
        {
            browser.preview(depth, position);
        }
    });
    widget.add_controller(click);
}

fn should_activate_filtered_pointer(press_count: i32, query: &RefCell<String>) -> bool {
    press_count == 1 && !query.borrow().trim().is_empty()
}

fn should_activate_pointer_click(
    press_count: i32,
    is_directory: bool,
    activation: ClickActivation,
) -> bool {
    let configured = if is_directory {
        activation.folders
    } else {
        activation.files
    };
    press_count == 1 && configured == ClickCount::One
}

fn connect_selection(
    section: &PaneSection,
    sections: Weak<RefCell<Vec<PaneSection>>>,
    browser: &Rc<Browser>,
    depth: usize,
    source_index: SourceIndexMap,
    multiple_selection: Rc<Cell<bool>>,
) {
    let commit = gtk::GestureClick::new();
    commit.set_propagation_phase(gtk::PropagationPhase::Capture);
    let browser_for_commit = Rc::downgrade(browser);
    commit.connect_pressed(move |_, _, _, _| {
        if let Some(browser) = browser_for_commit.upgrade() {
            browser.commit_selection();
        }
    });
    section.view.add_controller(commit);
    let syncing = section.syncing.clone();
    let weak_view = section.view.downgrade();
    let weak_bound_items = Rc::downgrade(&section.bound_items);
    let view_model = section.view_model.clone();
    let browser = Rc::downgrade(browser);
    section
        .selection
        .connect_selection_changed(move |selection, position, count| {
            if syncing.get() {
                return;
            }
            let (Some(sections), Some(browser)) = (sections.upgrade(), browser.upgrade()) else {
                return;
            };
            if !multiple_selection.get() {
                let positions = bitset_positions(&selection.selection());
                let end = position.saturating_add(count) as usize;
                if let Some(focused) = positions
                    .iter()
                    .rev()
                    .copied()
                    .find(|candidate| *candidate >= position as usize && *candidate < end)
                    .or_else(|| positions.last().copied())
                {
                    for other in sections.borrow().iter() {
                        other.syncing.set(true);
                        if other.selection == *selection {
                            other.selection.select_item(focused as u32, true);
                        } else {
                            other.selection.unselect_all();
                        }
                        other.syncing.set(false);
                    }
                }
            }
            let selected_positions =
                selected_source_positions(&source_index, &view_model, selection);
            let native_focus = weak_view
                .upgrade()
                .and_then(|view| view.root())
                .and_then(|root| root.focus())
                .and_then(|focused| {
                    let bound_items = weak_bound_items.upgrade()?;
                    bound_items.borrow().iter().find_map(|bound| {
                        let widget = bound.widget.upgrade()?;
                        let owns_focus = widget == focused
                            || focused.is_ancestor(&widget)
                            || widget.parent().as_ref() == Some(&focused);
                        owns_focus
                            .then(|| bound.item.upgrade().map(|item| item.position()))
                            .flatten()
                    })
                })
                .and_then(|position| {
                    source_position_for_view(&source_index, Some(&view_model), position)
                })
                .filter(|position| selected_positions.contains(position));
            let focused = native_focus.or_else(|| selected_positions.last().copied());
            sync_browser_selection(&sections, &browser, depth, &source_index, focused);
        });
}

fn set_selections(pane: &Pane, positions: &[usize]) {
    for section in pane.item_sections() {
        section.syncing.set(true);
        section.selection.unselect_all();
        for position in positions {
            if let Some(position) =
                view_position_for_source(&pane.model, Some(&section.view_model), *position)
            {
                section.selection.select_item(position, false);
            }
        }
        section.syncing.set(false);
    }
}

fn scroll_pane_to_source(pane: &Pane, source_position: usize) {
    for section in pane.item_sections() {
        let Some(position) =
            view_position_for_source(&pane.model, Some(&section.view_model), source_position)
        else {
            continue;
        };
        if position >= section.view_model.n_items() {
            continue;
        }
        scroll_collection_to(&section.view, position);
        return;
    }
}

fn scroll_collection_to(view: &gtk::Widget, position: u32) {
    super::browser::scroll_collection_when_allocated(view, position);
}

fn focus_collection_item(view: &gtk::Widget, position: u32) {
    super::browser::focus_collection_item_when_allocated(view, position);
}

fn focus_collection_cursor_when_bound(
    view: glib::object::WeakRef<gtk::Widget>,
    items: Rc<RefCell<Vec<BoundModeItem>>>,
    position: u32,
) {
    glib::idle_add_local_once(move || {
        let Some(view) = view.upgrade() else {
            return;
        };
        if !collection_keeps_cursor(&view) {
            return;
        }
        if focus_bound_cursor(&items, position) {
            return;
        }
        let frames = Cell::new(0u8);
        let items = items.clone();
        view.add_tick_callback(move |view, _| {
            if !collection_keeps_cursor(view)
                || focus_bound_cursor(&items, position)
                || frames.get() >= 8
            {
                return glib::ControlFlow::Break;
            }
            frames.set(frames.get().saturating_add(1));
            glib::ControlFlow::Continue
        });
    });
}

fn collection_keeps_cursor(view: &gtk::Widget) -> bool {
    let focused = view.root().and_then(|root| root.focus());
    if focused.as_ref().is_some_and(|focused| {
        super::focus_navigation::editable(focused) || super::focus_navigation::in_popover(focused)
    }) {
        return false;
    }
    focused.as_ref().is_none_or(|focused| {
        focused == view || view.is_ancestor(focused) || focused.is_ancestor(view)
    })
}

fn focus_bound_cursor(items: &RefCell<Vec<BoundModeItem>>, position: u32) -> bool {
    let Some(widget) = items.borrow().iter().find_map(|bound| {
        let item = bound.item.upgrade()?;
        (item.position() == position)
            .then(|| bound.widget.upgrade())
            .flatten()
            .filter(|widget| widget.is_mapped())
    }) else {
        return false;
    };
    widget
        .parent()
        .map(|parent| parent.grab_focus())
        .unwrap_or_else(|| widget.grab_focus())
}

fn set_mode_cut_style(widget: &impl IsA<gtk::Widget>, cut: bool) {
    if cut {
        widget.add_css_class("cut");
    } else {
        widget.remove_css_class("cut");
    }
}

fn refresh_cut_pane(pane: &Pane, browser: &Browser, cuts: &[Location]) {
    for section in pane.item_sections() {
        section.bound_items.borrow_mut().retain(|bound| {
            let (Some(item), Some(widget)) = (bound.item.upgrade(), bound.widget.upgrade()) else {
                return false;
            };
            let source = item
                .item()
                .and_then(|value| pane.source_index.of_item(&value));
            let cut = source
                .and_then(|position| browser.entry_at(pane.depth, position))
                .is_some_and(|entry| cuts.contains(&entry.location));
            set_mode_cut_style(&widget, cut);
            true
        });
    }
}

fn replace_entries(pane: &Pane, browser: &Browser, count: usize) {
    let values = browser
        .with_entries(pane.depth, 0..count, |entries| {
            entries
                .iter()
                .map(super::browser::entry_model_value)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    pane.splice_values(0, pane.model.n_items(), &values);
    show_count(pane);
}

fn detach_pane_models(pane: &Pane) {
    pane.detached.set(true);
    for section in pane.all_sections() {
        section.syncing.set(true);
        section.selection.set_model(None::<&gio::ListModel>);
        super::browser::detach_collection_view(&section.view);
    }
    if let Some(filtered) = pane.filter_model.as_ref() {
        filtered.set_model(None::<&gio::ListModel>);
    }
}

fn reconnect_pane_model(pane: &Pane) {
    if !pane.detached.replace(false) {
        return;
    }
    if let Some(filtered) = pane.filter_model.as_ref() {
        filtered.set_model(Some(&pane.model));
    }
    for section in pane.all_sections() {
        section.selection.set_model(Some(&section.view_model));
        section.syncing.set(false);
    }
}

fn show_count(pane: &Pane) {
    let count = pane.model.n_items();
    if count == 0 {
        pane.status.remove_css_class("error");
        pane.status.set_label("This directory is empty");
        pane.loading.show("status");
    } else {
        pane.loading.show("content");
    }
    if let Some(button) = &pane.empty_trash_button {
        button.set_sensitive(count > 0);
    }
}

fn apply_snapshot(pane: &Pane, snapshot: &BrowserColumnSnapshot, browser: &Browser) {
    replace_entries(pane, browser, snapshot.count);
    set_selections(pane, &snapshot.selected_positions);
    if let Some(&focused) = snapshot.selected_positions.last() {
        scroll_pane_to_source(pane, focused);
    }
    pane.truncated_hint.set_visible(snapshot.truncated);
    if snapshot.loading {
        pane.spinner.start();
        pane.loading.start();
    } else {
        pane.spinner.stop();
        if let Some(message) = snapshot.error.as_deref() {
            pane.status
                .set_label(&format!("Unable to read this directory\n{message}"));
            pane.status.add_css_class("error");
            pane.loading.show("status");
        }
    }
}

fn clear_box(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

fn assemble_list_row() -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.add_css_class("list-row");
    let name_cell = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    name_cell.add_css_class("list-name-cell");
    let icon = super::thumbnail::ThumbnailSlot::new(18);
    let name = gtk::Label::new(None);
    name.add_css_class("alternate-rename-label");
    name.set_xalign(0.0);
    name.set_hexpand(true);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    // Keep the label's natural width from widening this fixed-width table cell.
    name.set_max_width_chars(1);
    let field = gtk::Entry::new();
    field.add_css_class("inline-rename");
    super::accessibility::set_label(&field, "Rename");
    field.set_hexpand(true);
    field.set_visible(false);
    name_cell.append(&icon);
    name_cell.append(&name);
    name_cell.append(&field);
    row.append(&name_cell);
    row.append(&list_metadata_label());
    row.append(&list_metadata_label());
    row.append(&list_metadata_label());
    row.append(&list_metadata_label());
    row
}

fn list_row_parts(
    row: &gtk::Box,
) -> Option<(
    super::thumbnail::ThumbnailSlot,
    gtk::Label,
    gtk::Entry,
    gtk::Label,
    gtk::Label,
    gtk::Label,
    gtk::Label,
)> {
    let name_cell = row.first_child()?.downcast::<gtk::Box>().ok()?;
    let icon = name_cell
        .first_child()?
        .downcast::<super::thumbnail::ThumbnailSlot>()
        .ok()?;
    let name = icon.next_sibling()?.downcast::<gtk::Label>().ok()?;
    let field = name.next_sibling()?.downcast::<gtk::Entry>().ok()?;
    let mode = name_cell.next_sibling()?.downcast::<gtk::Label>().ok()?;
    let size = mode.next_sibling()?.downcast::<gtk::Label>().ok()?;
    let kind = size.next_sibling()?.downcast::<gtk::Label>().ok()?;
    let modified = kind.next_sibling()?.downcast::<gtk::Label>().ok()?;
    Some((icon, name, field, mode, size, kind, modified))
}

fn set_label_if_changed(label: &gtk::Label, text: &str) {
    if label.label() != text {
        label.set_label(text);
    }
}

fn apply_icons_entry(
    item: Option<&gtk::ListItem>,
    card: &gtk::Box,
    entry: &FileEntry,
    cuts: &HashSet<Location>,
    thumbnail_size: i32,
    scrolling: bool,
    state: Option<&super::browser::ViewState>,
) {
    let Some((icon, label)) = super::icons_cell::parts(card) else {
        return;
    };
    label.set_visible(true);
    if let Some(field) = super::icons_cell::rename_field(card) {
        field.set_visible(false);
    }
    let pending_name = state.and_then(|state| state.pending_rename_name(entry));
    let shown_name = pending_name.as_deref().unwrap_or(&entry.display_name);
    if label.text().as_deref() != Some(shown_name) {
        label.set_text(Some(shown_name));
    }
    if scrolling {
        super::thumbnail::show_fallback_icon(
            &icon,
            super::browser::entry_icon(entry),
            thumbnail_size,
        );
    } else {
        super::thumbnail::set_thumbnail_or_icon(
            &icon,
            entry,
            super::browser::entry_icon(entry),
            thumbnail_size,
            thumbnail_size,
        );
        refresh_icons_card_chrome(item, card, &icon, &label, entry, cuts);
    }
    if let Some(item) = item.filter(|_| pending_name.is_some()) {
        label.set_tooltip_text(Some(shown_name));
        super::accessibility::describe_entry(item, shown_name, Some(entry));
    }
}

fn refresh_icons_card_chrome(
    item: Option<&gtk::ListItem>,
    card: &gtk::Box,
    icon: &super::thumbnail::ThumbnailSlot,
    label: &gtk::Inscription,
    entry: &FileEntry,
    cuts: &HashSet<Location>,
) {
    label.set_tooltip_text(Some(&entry.display_name));
    if let Some(item) = item {
        super::accessibility::describe_entry(item, &entry.display_name, Some(entry));
    }
    set_mode_cut_style(card, cuts.contains(&entry.location));
    icon.set_opacity(if entry.is_directory() { 1.0 } else { 0.72 });
}

fn refresh_icons_section(
    browser: &Rc<Browser>,
    depth: usize,
    source_index: &SourceIndexMap,
    section: &PaneSection,
    cuts: &HashSet<Location>,
) {
    section.bound_items.borrow().iter().for_each(|bound| {
        let Some(card) = bound.widget.upgrade().and_downcast::<gtk::Box>() else {
            return;
        };
        let Some((icon, label)) = super::icons_cell::parts(&card) else {
            return;
        };
        let Some(item) = bound.item.upgrade() else {
            return;
        };
        let Some(position) = item.item().and_then(|value| source_index.of_item(&value)) else {
            return;
        };
        let Some(entry) = browser.entry_at(depth, position) else {
            return;
        };
        refresh_icons_card_chrome(Some(&item), &card, &icon, &label, &entry, cuts);
        super::thumbnail::set_thumbnail_or_icon(
            &icon,
            &entry,
            super::browser::entry_icon(&entry),
            icon.slot_size(),
            icon.slot_size(),
        );
        if let Some(position) = metadata_fill_position(Some(position), &entry, false) {
            browser.request_metadata_fill(depth, position, entry.location.clone());
        }
    });
}

fn list_metadata_label() -> gtk::Label {
    let label = gtk::Label::new(None);
    label.add_css_class("list-metadata-cell");
    label.set_xalign(0.0);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    // Metadata must truncate rather than overriding a resized column's width.
    label.set_max_width_chars(1);
    label
}

fn entry_size(entry: &FileEntry) -> String {
    if entry.is_directory() {
        return String::new();
    }
    match entry.size {
        MetadataValue::Known(bytes) => super::browser::format_file_size(bytes),
        MetadataValue::Unknown | MetadataValue::Unavailable => String::new(),
    }
}

fn entry_type(entry: &FileEntry) -> &'static str {
    use crate::model::EntryKind;
    match entry.kind {
        EntryKind::Directory => "Folder",
        EntryKind::DirectorySymbolicLink => "Folder link",
        EntryKind::File => "File",
        EntryKind::FileSymbolicLink => "File link",
        EntryKind::SymbolicLink => "Broken link",
        EntryKind::Other => "Other",
    }
}

fn entry_mode(entry: &FileEntry) -> String {
    match entry.mode {
        MetadataValue::Known(mode) => super::browser::format_permissions(mode),
        MetadataValue::Unknown | MetadataValue::Unavailable => String::new(),
    }
}

fn update_bound_list_metadata(pane: &Pane, updates: &[(usize, FileEntry)]) {
    let updates: HashMap<usize, &FileEntry> = updates
        .iter()
        .map(|(position, entry)| (*position, entry))
        .collect();
    for section in pane.item_sections() {
        section.bound_items.borrow_mut().retain(|bound| {
            let (Some(item), Some(row)) = (bound.item.upgrade(), bound.widget.upgrade()) else {
                return false;
            };
            let Some(position) = source_position_for_view(
                &pane.source_index,
                Some(&section.view_model),
                item.position(),
            ) else {
                return true;
            };
            let Some(entry) = updates.get(&position) else {
                return true;
            };
            let Some(row) = row.downcast::<gtk::Box>().ok() else {
                return true;
            };
            let Some((_, _, _, mode, size, _, modified)) = list_row_parts(&row) else {
                return true;
            };
            mode.set_label(&entry_mode(entry));
            size.set_label(&entry_size(entry));
            crate::util::set_modified_date(&modified, Some(entry), "—");
            true
        });
    }
}

/// Orders empty model values first, then folders and the remaining type labels
/// alphabetically, independently of which entries have loaded.
fn compare_type_groups(left: &str, right: &str) -> std::cmp::Ordering {
    fn rank(label: &str) -> u8 {
        match label {
            "" => 0,
            super::browser::FOLDER_TYPE_GROUP => 1,
            _ => 2,
        }
    }
    rank(left)
        .cmp(&rank(right))
        .then_with(|| left.to_lowercase().cmp(&right.to_lowercase()))
}

fn model_value(item: &glib::Object) -> String {
    item.downcast_ref::<gtk::StringObject>()
        .map(|value| value.string().to_string())
        .unwrap_or_default()
}

/// Empty model values have no file-type group.
fn value_type_group(value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    super::browser::model_type_group(value)
}

/// Sorts entries into their file-type groups. `GtkSortListModel` sorts stably, so
/// entries keep the pane's own sort order inside each group. List also uses this
/// sorter as `section_sorter` so headings mark where one type ends and the next begins.
fn type_group_sorter() -> gtk::CustomSorter {
    gtk::CustomSorter::new(|left, right| {
        compare_type_groups(
            &value_type_group(&model_value(left)),
            &value_type_group(&model_value(right)),
        )
        .into()
    })
}

#[cfg(test)]
fn type_groups_of(values: impl Iterator<Item = impl AsRef<str>>) -> Vec<String> {
    let mut labels: Vec<String> = Vec::new();
    for value in values {
        let label = super::browser::model_type_group(value.as_ref());
        if let Err(position) =
            labels.binary_search_by(|candidate| compare_type_groups(candidate, &label))
        {
            labels.insert(position, label);
        }
    }
    labels
}

fn type_group_heading(label: &str) -> gtk::Label {
    let heading = gtk::Label::new(Some(label));
    heading.add_css_class("type-group-heading");
    heading.set_xalign(0.0);
    heading
}

/// Section headings for a grouped list view.
fn type_group_header_factory() -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, header| {
        let Some(header) = header.downcast_ref::<gtk::ListHeader>() else {
            return;
        };
        header.set_child(Some(&type_group_heading("")));
    });
    factory.connect_bind(|_, header| {
        let Some(header) = header.downcast_ref::<gtk::ListHeader>() else {
            return;
        };
        let Some(heading) = header.child().and_downcast::<gtk::Label>() else {
            return;
        };
        let value = header
            .item()
            .map(|item| model_value(&item))
            .unwrap_or_default();
        let group = value_type_group(&value);
        heading.set_label(&group);
        heading.set_visible(!group.is_empty());
    });
    factory
}

fn bound_item_visitor(bound_items: Rc<RefCell<Vec<BoundModeItem>>>) -> super::marquee::ItemVisitor {
    Rc::new(move |visit| {
        bound_items.borrow_mut().retain(|bound| {
            let (Some(item), Some(widget)) = (bound.item.upgrade(), bound.widget.upgrade()) else {
                return false;
            };
            visit(item.position(), &widget);
            true
        });
    })
}

fn selected_source_positions(
    source_index: &SourceIndexMap,
    view_model: &gio::ListModel,
    selection: &gtk::MultiSelection,
) -> Vec<usize> {
    bitset_positions(&selection.selection())
        .into_iter()
        .filter_map(|position| {
            source_position_for_view(source_index, Some(view_model), position as u32)
        })
        .collect()
}

/// Reports the selection of every section in a pane, so a grouped view keeps items
/// picked in other groups selected.
fn sync_browser_selection(
    sections: &Rc<RefCell<Vec<PaneSection>>>,
    browser: &Browser,
    depth: usize,
    source_index: &SourceIndexMap,
    focused: Option<usize>,
) {
    let mut positions: Vec<usize> = {
        let sections = sections.borrow();
        sections
            .iter()
            .flat_map(|section| {
                selected_source_positions(source_index, &section.view_model, &section.selection)
            })
            .collect()
    };
    positions.sort_unstable();
    positions.dedup();
    let focused = focused.or_else(|| positions.last().copied());
    browser.set_selection(depth, &positions, focused);
}

fn focused_section_item(
    section: &PaneSection,
    focused: &gtk::Widget,
) -> Option<(u32, gtk::graphene::Rect)> {
    section.bound_items.borrow().iter().find_map(|bound| {
        let widget = bound.widget.upgrade()?;
        let owns_focus = widget == *focused
            || focused.is_ancestor(&widget)
            || widget.parent().as_ref() == Some(focused);
        if !owns_focus {
            return None;
        }
        Some((
            bound.item.upgrade()?.position(),
            widget.compute_bounds(&section.view)?,
        ))
    })
}

/// The view position of the item `picked` belongs to, when it is one of the section's
/// rendered items.
fn section_item_position(section: &PaneSection, picked: &gtk::Widget) -> Option<u32> {
    let mut candidate = Some(picked.clone());
    while let Some(widget) = candidate {
        let position = section.bound_items.borrow().iter().find_map(|bound| {
            let bound_widget = bound.widget.upgrade()?;
            let item = bound.item.upgrade()?;
            (bound_widget == widget).then_some(item.position())
        });
        if position.is_some() {
            return position;
        }
        candidate = widget.parent();
    }
    None
}

fn install_section_context_menu(
    state: &Rc<super::browser::ViewState>,
    section: &PaneSection,
    sections: Weak<RefCell<Vec<PaneSection>>>,
    source_index: &SourceIndexMap,
    depth: usize,
) {
    let owner = section.clone();
    let pick_position = Rc::new(move |picked: &gtk::Widget| section_item_position(&owner, picked));
    let source_index = source_index.clone();
    let view_model = section.view_model.clone();
    let source_position = Rc::new(move |position| {
        source_position_for_view(&source_index, Some(&view_model), position)
    });
    let owner_view = section.view.clone();
    let clear_other_selections = Rc::new(move || {
        let Some(sections) = sections.upgrade() else {
            return;
        };
        for other in sections.borrow().iter() {
            if other.view == owner_view {
                continue;
            }
            other.syncing.set(true);
            other.selection.unselect_all();
            other.syncing.set(false);
        }
    });
    super::browser::install_item_context_menu(
        state,
        &section.view,
        &section.selection,
        pick_position,
        source_position,
        clear_other_selections,
        depth,
    );
}

fn bitset_positions(bitset: &gtk::Bitset) -> Vec<usize> {
    let Some((iterator, first)) = gtk::BitsetIter::init_first(bitset) else {
        return Vec::new();
    };
    std::iter::once(first)
        .chain(iterator)
        .map(|position| position as usize)
        .collect()
}

#[cfg(test)]
mod tests;
