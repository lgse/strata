// SPDX-License-Identifier: GPL-3.0-or-later

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

use crate::{
    app::{Browser, BrowserColumnSnapshot, BrowserEvent},
    model::{FileEntry, Location, MetadataValue, SortDirection, SortKey},
};

const EXPLORER_COLUMN_WIDTHS: [i32; 5] = [160, 110, 90, 120, 150];
const EXPLORER_COLUMN_MIN_WIDTHS: [i32; 5] = [160, 80, 70, 80, 110];
const DEFAULT_GRID_THUMBNAIL_SIZE: i32 = 64;
/// Margin and padding a grid card adds around its own width.
const GRID_CARD_SPACING: i32 = 4;
const FALLBACK_GRID_COLUMN_WIDTH: i32 = 160;
const MIN_GRID_THUMBNAIL_SIZE: i32 = 64;
const MAX_GRID_THUMBNAIL_SIZE: i32 = 256;

#[derive(Clone)]
struct ExplorerColumnLayout {
    widths: Rc<Vec<Cell<i32>>>,
    cells: Rc<Vec<RefCell<Vec<glib::WeakRef<gtk::Widget>>>>>,
    name_manually_resized: Rc<Cell<bool>>,
}

impl ExplorerColumnLayout {
    fn new() -> Self {
        Self {
            widths: Rc::new(EXPLORER_COLUMN_WIDTHS.into_iter().map(Cell::new).collect()),
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
    Grid,
    Explorer,
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
                BrowserMode::Grid | BrowserMode::Explorer => ClickCount::Two,
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

struct ActiveModeRename {
    field: gtk::Entry,
    label: gtk::Label,
}

struct ActiveModeNewEntry {
    is_directory: bool,
    field: gtk::Entry,
    placeholder: Option<gtk::StringList>,
    stack: Option<gtk::Stack>,
    source_model: Option<gtk::StringList>,
    view: gtk::Widget,
}

struct BoundModeItem {
    item: glib::WeakRef<gtk::ListItem>,
    widget: glib::WeakRef<gtk::Widget>,
}

/// One collection view inside a pane. A pane normally has a single section; a pane
/// that groups entries by file type has one per group, each with its own model and
/// selection over the same source entries.
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
    model: gtk::StringList,
    source_index: SourceIndexMap,
    filter_model: Option<gtk::FilterListModel>,
    /// The section that owns the pane's chrome and hosts the inline new-entry row. In
    /// a grouped grid it holds nothing else, since entries live in group sections.
    section: PaneSection,
    sections: Rc<RefCell<Vec<PaneSection>>>,
    groups: Option<Rc<GridGroups>>,
    grid: Option<Rc<GridContext>>,
    targets: super::marquee::MarqueeTargets,
    /// Set while a reload has detached the pane's models from their views.
    detached: Rc<Cell<bool>>,
    stack: gtk::Stack,
    status: gtk::Label,
    spinner: gtk::Spinner,
    truncated_hint: gtk::Image,
    marquee: super::marquee::Marquee,
    filter_entry: Option<gtk::Entry>,
    filter_button: Option<gtk::ToggleButton>,
    empty_trash_button: Option<gtk::Button>,
    new_entry_placeholder: Option<gtk::StringList>,
    new_entry_is_directory: Option<Rc<Cell<bool>>>,
    show_hidden: Rc<Cell<bool>>,
    filter: gtk::CustomFilter,
}

impl Pane {
    /// The sections that render entries, in visual order.
    fn item_sections(&self) -> Vec<PaneSection> {
        self.sections.borrow().clone()
    }

    /// Every section, including the one hosting the inline new-entry row.
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
    grid_root: gtk::Box,
    explorer_root: gtk::Box,
    grid_panes: Vec<Pane>,
    explorer_pane: Option<Pane>,
    browser: Rc<Browser>,
    single_click_previews: Rc<Cell<bool>>,
    grid_click_activation: Rc<Cell<ClickActivation>>,
    explorer_click_activation: Rc<Cell<ClickActivation>>,
    transfer_handler: TransferHandlerSlot,
    cut_locations: Rc<RefCell<HashSet<Location>>>,
    context_state: RefCell<Option<Weak<super::browser::ViewState>>>,
    active_rename: Rc<RefCell<Option<ActiveModeRename>>>,
    active_new_entry: Rc<RefCell<Option<ActiveModeNewEntry>>>,
    mode: BrowserMode,
    density: BrowserDensity,
    group_by_type: bool,
    grid_thumbnail_size: Rc<Cell<i32>>,
}

impl ModeViews {
    pub fn new(columns: &gtk::ScrolledWindow, browser: Rc<Browser>) -> Self {
        let grid_root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        grid_root.add_css_class("mode-grid-columns");
        grid_root.set_halign(gtk::Align::Fill);
        grid_root.set_hexpand(true);
        grid_root.set_vexpand(true);
        let grid_scroll = gtk::ScrolledWindow::builder()
            .child(&grid_root)
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .hexpand(true)
            .vexpand(true)
            .build();
        grid_scroll.add_css_class("fixed-scrollbar");

        let explorer_root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        explorer_root.add_css_class("mode-explorer");
        explorer_root.set_hexpand(true);
        explorer_root.set_vexpand(true);
        // The explorer pane header belongs to the viewport, while its user-resizable table
        // columns scroll independently below it.
        let explorer_scroll = gtk::ScrolledWindow::builder()
            .child(&explorer_root)
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
        stack.add_named(&grid_scroll, Some("grid"));
        stack.add_named(&explorer_scroll, Some("explorer"));
        stack.set_visible_child_name("columns");

        Self {
            stack,
            grid_root,
            explorer_root,
            grid_panes: Vec::new(),
            explorer_pane: None,
            browser,
            single_click_previews: Rc::new(Cell::new(true)),
            grid_click_activation: Rc::new(Cell::new(ClickActivation::default_for(
                BrowserMode::Grid,
            ))),
            explorer_click_activation: Rc::new(Cell::new(ClickActivation::default_for(
                BrowserMode::Explorer,
            ))),
            transfer_handler: Rc::new(RefCell::new(None)),
            cut_locations: Rc::new(RefCell::new(HashSet::new())),
            context_state: RefCell::new(None),
            active_rename: Rc::new(RefCell::new(None)),
            active_new_entry: Rc::new(RefCell::new(None)),
            mode: BrowserMode::Columns,
            density: BrowserDensity::Compact,
            group_by_type: false,
            grid_thumbnail_size: Rc::new(Cell::new(DEFAULT_GRID_THUMBNAIL_SIZE)),
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
            BrowserMode::Grid => self.grid_panes.first(),
            BrowserMode::Explorer => self.explorer_pane.as_ref(),
        }?;
        Some(pane.marquee.clone())
    }

    pub fn selected_positions(&self) -> Option<(usize, Vec<usize>)> {
        let pane = match self.mode {
            BrowserMode::Columns => return None,
            BrowserMode::Grid => self.grid_panes.first(),
            BrowserMode::Explorer => self.explorer_pane.as_ref(),
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

    pub fn new_entry_is_active(&self) -> bool {
        self.active_new_entry.borrow().is_some()
    }

    pub fn cancel_new_entry(&self) -> bool {
        let Some(active) = self.active_new_entry.take() else {
            return false;
        };
        active.field.set_text("");
        active.field.remove_css_class("error");
        active.field.set_tooltip_text(None);
        finish_mode_new_entry(&active);
        true
    }

    pub fn begin_new_entry(&self, depth: usize, is_directory: bool) -> bool {
        self.cancel_new_entry();
        self.cancel_rename();
        let pane = match self.mode {
            BrowserMode::Columns => return false,
            BrowserMode::Grid => self.grid_panes.iter().find(|pane| pane.depth == depth),
            BrowserMode::Explorer => self
                .explorer_pane
                .as_ref()
                .filter(|pane| pane.depth == depth),
        };
        let Some(pane) = pane else {
            return false;
        };
        let Some(placeholder) = pane.new_entry_placeholder.as_ref() else {
            return false;
        };
        let Some(entry_kind) = pane.new_entry_is_directory.as_ref() else {
            return false;
        };
        entry_kind.set(is_directory);
        placeholder.splice(0, placeholder.n_items(), &[""]);
        pane.stack.set_visible_child_name("content");
        let bound_items = pane.section.bound_items.clone();
        let active = self.active_new_entry.clone();
        let placeholder = placeholder.clone();
        let stack = pane.stack.clone();
        let source_model = pane.model.clone();
        let view = pane.section.view.clone();
        view.add_css_class("creating-entry");
        if let Ok(grid) = view.clone().downcast::<gtk::GridView>() {
            grid.scroll_to(0, gtk::ListScrollFlags::FOCUS, None);
        } else if let Ok(list) = view.clone().downcast::<gtk::ListView>() {
            list.scroll_to(0, gtk::ListScrollFlags::FOCUS, None);
        }
        glib::idle_add_local_once(move || {
            let field = bound_items.borrow().iter().find_map(|bound| {
                let item = bound.item.upgrade()?;
                if item.position() != 0 {
                    return None;
                }
                let widget = bound.widget.upgrade()?;
                descendant_with_class(&widget, "inline-rename")?
                    .downcast::<gtk::Entry>()
                    .ok()
            });
            let Some(field) = field else {
                placeholder.splice(0, placeholder.n_items(), &[]);
                view.remove_css_class("creating-entry");
                return;
            };
            field.set_text("");
            active.replace(Some(ActiveModeNewEntry {
                is_directory,
                field: field.clone(),
                placeholder: Some(placeholder),
                stack: Some(stack),
                source_model: Some(source_model),
                view,
            }));
            field.grab_focus();
        });
        true
    }

    pub fn cancel_rename(&self) -> bool {
        let Some(rename) = self.active_rename.take() else {
            return false;
        };
        rename.label.set_visible(true);
        rename.field.set_visible(false);
        rename.field.set_sensitive(true);
        true
    }

    pub fn begin_rename(&self, depth: usize, source_position: usize, entry: &FileEntry) -> bool {
        self.cancel_rename();
        let pane = match self.mode {
            BrowserMode::Columns => return false,
            BrowserMode::Grid => self.grid_panes.iter().find(|pane| pane.depth == depth),
            BrowserMode::Explorer => self
                .explorer_pane
                .as_ref()
                .filter(|pane| pane.depth == depth),
        };
        let Some(pane) = pane else {
            return false;
        };
        let widget = pane.item_sections().iter().find_map(|section| {
            let position =
                view_position_for_source(&pane.model, Some(&section.view_model), source_position)?;
            section.bound_items.borrow().iter().find_map(|bound| {
                let item = bound.item.upgrade()?;
                (item.position() == position).then(|| bound.widget.upgrade())?
            })
        });
        let Some(widget) = widget else {
            return false;
        };
        let Some(label) =
            descendant_with_class(&widget, "alternate-rename-label").and_downcast::<gtk::Label>()
        else {
            return false;
        };
        let Some(field) =
            descendant_with_class(&widget, "inline-rename").and_downcast::<gtk::Entry>()
        else {
            return false;
        };
        field.set_text(&entry.display_name);
        field.set_visible(true);
        label.set_visible(false);
        let browser = Rc::downgrade(&self.browser);
        let renamed_entry = entry.clone();
        let active = self.active_rename.clone();
        field.connect_activate(move |field| {
            let name = field.text().to_string();
            if name == renamed_entry.display_name {
                if let Some(rename) = active.take() {
                    rename.label.set_visible(true);
                    rename.field.set_visible(false);
                }
            } else if let Some(browser) = browser.upgrade() {
                field.set_sensitive(false);
                browser.rename(renamed_entry.clone(), name);
            }
        });
        field.grab_focus();
        field.select_region(0, super::browser::rename_stem_end(&entry.display_name));
        self.active_rename
            .replace(Some(ActiveModeRename { field, label }));
        true
    }

    pub fn filter_has_focus(&self) -> bool {
        let focused = self.stack.root().and_then(|root| root.focus());
        self.grid_panes
            .iter()
            .chain(self.explorer_pane.iter())
            .filter_map(|pane| pane.filter_entry.as_ref())
            .any(|entry| widget_has_focus(entry, focused.as_ref()))
    }

    pub fn item_view_has_focus(&self) -> bool {
        let focused = self.stack.root().and_then(|root| root.focus());
        self.grid_panes
            .iter()
            .chain(self.explorer_pane.iter())
            .any(|pane| {
                pane.all_sections()
                    .iter()
                    .any(|section| widget_has_focus(&section.view, focused.as_ref()))
            })
    }

    pub fn empty_filter_has_focus(&self) -> bool {
        let focused = self.stack.root().and_then(|root| root.focus());
        self.grid_panes
            .iter()
            .chain(self.explorer_pane.iter())
            .filter_map(|pane| pane.filter_entry.as_ref())
            .any(|entry| entry.text().is_empty() && widget_has_focus(entry, focused.as_ref()))
    }

    pub fn show_filter(&self) -> bool {
        let pane = match self.mode {
            BrowserMode::Columns => None,
            BrowserMode::Grid => self.grid_panes.first(),
            BrowserMode::Explorer => self.explorer_pane.as_ref(),
        };
        let Some(pane) = pane else {
            return false;
        };
        let (Some(entry), Some(button)) = (pane.filter_entry.as_ref(), pane.filter_button.as_ref())
        else {
            return false;
        };
        button.set_active(true);
        entry.grab_focus();
        true
    }

    pub fn dismiss_focused_filter(&self) -> bool {
        let focused = self.stack.root().and_then(|root| root.focus());
        let Some(pane) = self
            .grid_panes
            .iter()
            .chain(self.explorer_pane.iter())
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
        self.cancel_new_entry();
        self.cancel_rename();
        self.mode = mode;
        match mode {
            BrowserMode::Columns => {}
            BrowserMode::Grid => self.rebuild_grid(),
            BrowserMode::Explorer => self.rebuild_explorer(),
        }
    }

    pub fn show_mode(&self, mode: BrowserMode) {
        self.stack.set_visible_child_name(match mode {
            BrowserMode::Columns => "columns",
            BrowserMode::Grid => "grid",
            BrowserMode::Explorer => "explorer",
        });
    }

    pub fn clear_inactive_mode(&mut self, mode: BrowserMode) {
        if self.mode == mode {
            return;
        }
        match mode {
            BrowserMode::Columns => {}
            BrowserMode::Grid => self.clear_grid(),
            BrowserMode::Explorer => self.clear_explorer(),
        }
    }

    pub fn set_single_click_previews(&self, enabled: bool) {
        self.single_click_previews.set(enabled);
    }

    pub fn set_click_activation(&self, mode: BrowserMode, activation: ClickActivation) {
        match mode {
            BrowserMode::Columns => {}
            BrowserMode::Grid => self.grid_click_activation.set(activation),
            BrowserMode::Explorer => self.explorer_click_activation.set(activation),
        }
    }

    pub fn set_transfer_handler(&self, handler: TransferHandler) {
        self.transfer_handler.replace(Some(handler));
    }

    pub fn set_context_state(&self, state: Weak<super::browser::ViewState>) {
        self.context_state.replace(Some(state));
    }

    pub fn set_cut_locations(&self, locations: &[Location]) {
        self.cut_locations
            .replace(locations.iter().cloned().collect());
        for pane in self.grid_panes.iter().chain(self.explorer_pane.iter()) {
            refresh_cut_pane(pane, &self.browser, locations);
        }
    }

    pub fn set_density(&mut self, density: BrowserDensity) {
        self.density = density;
        for pane in &self.grid_panes {
            configure_grid_density(pane, density);
        }
        for root in [&self.grid_root, &self.explorer_root] {
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
        self.cancel_new_entry();
        self.cancel_rename();
        self.group_by_type = enabled;
        match self.mode {
            BrowserMode::Columns => {}
            BrowserMode::Grid => self.rebuild_grid(),
            BrowserMode::Explorer => self.rebuild_explorer(),
        }
    }

    pub fn handle(&mut self, event: &BrowserEvent) {
        if matches!(
            event,
            BrowserEvent::Reset
                | BrowserEvent::ColumnsTruncated { .. }
                | BrowserEvent::ColumnAdded { .. }
        ) {
            self.cancel_new_entry();
        }
        match event {
            BrowserEvent::Reset => {
                self.clear_grid();
                self.clear_explorer();
            }
            BrowserEvent::ColumnsTruncated { .. } => match self.mode {
                BrowserMode::Columns => {}
                BrowserMode::Grid => self.rebuild_grid(),
                BrowserMode::Explorer => self.rebuild_explorer(),
            },
            BrowserEvent::ColumnAdded { depth, .. }
                if self.browser.active_depth() == Some(*depth) =>
            {
                match self.mode {
                    BrowserMode::Columns => {}
                    BrowserMode::Grid => self.rebuild_grid(),
                    BrowserMode::Explorer => self.rebuild_explorer(),
                }
            }
            BrowserEvent::ColumnAdded { .. } => {}
            BrowserEvent::EntriesInserted { depth, insertions } => {
                for pane in self.panes_at(*depth) {
                    for insertion in insertions {
                        let values: Vec<String> = insertion
                            .entries
                            .iter()
                            .map(super::browser::entry_model_value)
                            .collect();
                        let values_ref: Vec<&str> = values.iter().map(String::as_str).collect();
                        pane.model.splice(insertion.position as u32, 0, &values_ref);
                    }
                    sync_grid_groups(pane);
                    if !pane.spinner.is_spinning() {
                        show_count(pane);
                    }
                }
            }
            BrowserEvent::EntriesReplaced { depth, count } => {
                for pane in self.panes_at(*depth) {
                    if *count > 0 {
                        pane.spinner.stop();
                        pane.spinner.set_visible(false);
                    }
                    replace_entries(pane, &self.browser, *count);
                }
            }
            BrowserEvent::EntriesPublished {
                depth,
                position,
                count,
            } => {
                for pane in self.panes_at(*depth) {
                    let values = self
                        .browser
                        .with_entries(
                            *depth,
                            *position..position.saturating_add(*count),
                            |entries| {
                                entries
                                    .iter()
                                    .map(super::browser::entry_model_value)
                                    .collect::<Vec<_>>()
                            },
                        )
                        .unwrap_or_default();
                    let values: Vec<_> = values.iter().map(String::as_str).collect();
                    pane.model.splice(*position as u32, 0, &values);
                    sync_grid_groups(pane);
                    if !pane.spinner.is_spinning() {
                        show_count(pane);
                    }
                }
            }
            BrowserEvent::MetadataFilled { depth, updates } => {
                if self.mode == BrowserMode::Explorer {
                    for pane in self.panes_at(*depth) {
                        update_bound_explorer_metadata(pane, updates);
                    }
                }
            }
            BrowserEvent::SortingStarted { depth } => {
                for pane in self.panes_at(*depth) {
                    pane.spinner.set_tooltip_text(Some("Sorting…"));
                    pane.spinner.set_visible(true);
                    pane.spinner.start();
                }
            }
            BrowserEvent::SortingFinished { depth } => {
                for pane in self.panes_at(*depth) {
                    pane.spinner.stop();
                    pane.spinner.set_visible(false);
                    pane.spinner.set_tooltip_text(None);
                }
            }
            BrowserEvent::EntriesSpliced { depth, splices, .. } => {
                for pane in self.panes_at(*depth) {
                    for splice in splices {
                        let values: Vec<String> = splice
                            .entries
                            .iter()
                            .map(super::browser::entry_model_value)
                            .collect();
                        let values_ref: Vec<&str> = values.iter().map(String::as_str).collect();
                        pane.model.splice(
                            splice.position as u32,
                            splice.removed as u32,
                            &values_ref,
                        );
                    }
                    sync_grid_groups(pane);
                    show_count(pane);
                }
            }
            BrowserEvent::ColumnReloaded { depth } => {
                for pane in self.panes_at(*depth) {
                    pane.detached.set(true);
                    for section in pane.all_sections() {
                        section.syncing.set(true);
                        section.selection.set_model(None::<&gio::ListModel>);
                    }
                    if let Some(filtered) = pane.filter_model.as_ref() {
                        filtered.set_model(None::<&gio::ListModel>);
                    }
                    pane.model.splice(0, pane.model.n_items(), &[]);
                    sync_grid_groups(pane);
                    pane.truncated_hint.set_visible(false);
                    pane.spinner.set_visible(true);
                    pane.spinner.start();
                    pane.stack.set_visible_child_name("loading");
                }
            }
            BrowserEvent::LoadFinished { depth, truncated } => {
                for pane in self.panes_at(*depth) {
                    reconnect_pane_model(pane);
                    sync_grid_groups(pane);
                    pane.spinner.stop();
                    pane.spinner.set_visible(false);
                    pane.truncated_hint.set_visible(*truncated);
                    show_count(pane);
                }
            }
            BrowserEvent::LoadFailed { depth, message } => {
                for pane in self.panes_at(*depth) {
                    reconnect_pane_model(pane);
                    pane.spinner.stop();
                    pane.status
                        .set_label(&format!("Unable to read this directory\n{message}"));
                    pane.status.add_css_class("error");
                    pane.stack.set_visible_child_name("status");
                }
            }
            BrowserEvent::SelectionSetChanged {
                depth,
                positions,
                take_focus,
                ..
            } => {
                for pane in self.panes_at(*depth) {
                    set_selections(pane, positions);
                }
                if *take_focus {
                    self.focus_visible_pane(*depth);
                }
            }
            BrowserEvent::FocusChanged { depth, position } => {
                for pane in self.panes_at(*depth) {
                    set_selections(pane, &position.iter().copied().collect::<Vec<_>>());
                }
                self.focus_visible_pane(*depth);
            }
            BrowserEvent::RenameCompleted => {
                self.cancel_rename();
            }
            BrowserEvent::RenameFailed { message } => {
                if let Some(rename) = self.active_rename.borrow().as_ref() {
                    rename.field.set_sensitive(true);
                    rename.field.add_css_class("error");
                    rename.field.set_tooltip_text(Some(message));
                    rename.field.grab_focus();
                }
            }
            _ => {}
        }
    }

    pub fn focus_visible_pane(&self, depth: usize) {
        let view = match self.mode {
            BrowserMode::Columns => return,
            BrowserMode::Grid => self
                .grid_panes
                .iter()
                .find(|pane| pane.depth == depth)
                .map(|pane| pane.focus_view()),
            BrowserMode::Explorer => self
                .explorer_pane
                .as_ref()
                .filter(|pane| pane.depth == depth)
                .map(|pane| pane.focus_view()),
        };
        let Some(view) = view else {
            return;
        };
        view.grab_focus();
        let view = view.clone();
        glib::idle_add_local_once(move || {
            view.grab_focus();
        });
    }

    fn all_panes(&self) -> Vec<&Pane> {
        self.grid_panes
            .iter()
            .chain(self.explorer_pane.as_ref())
            .collect()
    }

    fn panes_at(&self, depth: usize) -> Vec<&Pane> {
        match self.mode {
            BrowserMode::Columns => Vec::new(),
            BrowserMode::Grid => self
                .grid_panes
                .iter()
                .find(|pane| pane.depth == depth)
                .into_iter()
                .collect(),
            BrowserMode::Explorer => self
                .explorer_pane
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
        let sections = Rc::downgrade(&pane.sections);
        let entries = pane.model.clone();
        super::browser::install_folder_context_menu(
            &state,
            pane.stack.upcast_ref(),
            Rc::new(move || entries.n_items() > 0),
            Rc::new(move |picked| {
                sections.upgrade().is_some_and(|sections| {
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

    fn clear_grid(&mut self) {
        for pane in &self.grid_panes {
            detach_pane_models(pane);
        }
        clear_box(&self.grid_root);
        self.grid_panes.clear();
    }

    fn clear_explorer(&mut self) {
        if let Some(pane) = self.explorer_pane.as_ref() {
            detach_pane_models(pane);
        }
        clear_box(&self.explorer_root);
        self.explorer_pane = None;
    }

    fn rebuild_grid(&mut self) {
        let Some(depth) = self.browser.active_depth() else {
            self.clear_grid();
            return;
        };
        let Some(snapshot) = self.browser.column_snapshot(depth) else {
            return;
        };
        self.clear_grid();
        let pane = build_grid_pane(
            self.browser.clone(),
            ModeClickOptions {
                previews: self.single_click_previews.clone(),
                activation: self.grid_click_activation.clone(),
            },
            self.transfer_handler.clone(),
            self.cut_locations.clone(),
            GridOptions {
                state: self.context_state.borrow().clone(),
                thumbnail_size: self.grid_thumbnail_size.clone(),
                active_new_entry: self.active_new_entry.clone(),
                group_by_type: self.group_by_type,
                density: self.density,
            },
            depth,
            &snapshot.location.display_name(),
        );
        configure_grid_density(&pane, self.density);
        self.install_context_menu(&pane);
        self.grid_root.append(&pane.shell);
        apply_snapshot(&pane, &snapshot, &self.browser);
        self.grid_panes.push(pane);
    }

    fn rebuild_explorer(&mut self) {
        let Some(depth) = self.browser.active_depth() else {
            self.clear_explorer();
            return;
        };
        let Some(snapshot) = self.browser.column_snapshot(depth) else {
            return;
        };
        self.clear_explorer();
        let pane = build_explorer_pane(
            self.browser.clone(),
            ModeClickOptions {
                previews: self.single_click_previews.clone(),
                activation: self.explorer_click_activation.clone(),
            },
            self.transfer_handler.clone(),
            self.cut_locations.clone(),
            ExplorerOptions {
                state: self.context_state.borrow().clone(),
                active_new_entry: self.active_new_entry.clone(),
                group_by_type: self.group_by_type,
            },
            depth,
            &snapshot.location.display_name(),
        );
        self.install_context_menu(&pane);
        self.explorer_root.append(&pane.shell);
        apply_snapshot(&pane, &snapshot, &self.browser);
        self.explorer_pane = Some(pane);
    }
}

fn widget_has_focus(widget: &impl IsA<gtk::Widget>, focused: Option<&gtk::Widget>) -> bool {
    widget.has_focus()
        || focused.is_some_and(|focused| {
            focused == widget.as_ref() || focused.is_ancestor(widget.as_ref())
        })
}

#[derive(Clone)]
struct ExplorerOptions {
    state: Option<Weak<super::browser::ViewState>>,
    active_new_entry: Rc<RefCell<Option<ActiveModeNewEntry>>>,
    group_by_type: bool,
}

struct GridOptions {
    state: Option<Weak<super::browser::ViewState>>,
    thumbnail_size: Rc<Cell<i32>>,
    active_new_entry: Rc<RefCell<Option<ActiveModeNewEntry>>>,
    group_by_type: bool,
    density: BrowserDensity,
}

#[derive(Clone)]
struct ModeClickOptions {
    previews: Rc<Cell<bool>>,
    activation: Rc<Cell<ClickActivation>>,
}

fn submit_mode_new_entry(
    active: &RefCell<Option<ActiveModeNewEntry>>,
    browser: &Weak<Browser>,
    location: &Option<Location>,
    field: &gtk::Entry,
) {
    if !active
        .borrow()
        .as_ref()
        .is_some_and(|active| active.field == *field)
    {
        return;
    }
    let name = field.text().to_string();
    if !super::browser::update_basename_validation(field) {
        field.grab_focus();
        return;
    }
    let Some(active) = active.take() else {
        return;
    };
    finish_mode_new_entry(&active);
    if let (Some(browser), Some(location)) = (browser.upgrade(), location.clone()) {
        if active.is_directory {
            browser.create_directory(location, name);
        } else {
            browser.create_file(location, name);
        }
    }
}

fn finish_mode_new_entry(active: &ActiveModeNewEntry) {
    active.field.set_text("");
    active.field.remove_css_class("error");
    active.field.set_tooltip_text(None);
    active.view.remove_css_class("creating-entry");
    if let Some(placeholder) = active.placeholder.as_ref() {
        placeholder.splice(0, placeholder.n_items(), &[]);
    }
    if active
        .source_model
        .as_ref()
        .is_some_and(|model| model.n_items() == 0)
        && let Some(stack) = active.stack.as_ref()
    {
        stack.set_visible_child_name("status");
    }
}

struct GridControls {
    leading: gtk::Box,
    actions: gtk::Box,
    filter_entry: gtk::Entry,
    filter_revealer: gtk::Revealer,
    filter_button: gtk::ToggleButton,
    thumbnail_scale: gtk::Scale,
    thumbnail_value: gtk::Label,
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
    row.append(&crate::assets::primary_icon(
        crate::assets::icons::FUNNEL,
        16,
    ));
    row.append(&entry);
    let revealer = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideDown)
        .child(&row)
        .build();
    let button = gtk::ToggleButton::builder().tooltip_text(tooltip).build();
    button.set_child(Some(&crate::assets::primary_icon(
        crate::assets::icons::FUNNEL,
        16,
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

fn grid_controls(browser: &Rc<Browser>, depth: usize, thumbnail_size: i32) -> GridControls {
    let leading = explorer_navigation(browser);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    actions.add_css_class("grid-header-actions");

    let thumbnail_popover = gtk::Popover::new();
    thumbnail_popover.set_has_arrow(false);
    thumbnail_popover.add_css_class("grid-thumbnail-popover");
    let thumbnail_content = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let thumbnail_heading = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let thumbnail_title = gtk::Label::new(Some("Thumbnail size"));
    thumbnail_title.add_css_class("grid-thumbnail-title");
    thumbnail_title.set_xalign(0.0);
    thumbnail_title.set_hexpand(true);
    let thumbnail_value = gtk::Label::new(Some(&format!("{thumbnail_size} px")));
    thumbnail_value.add_css_class("grid-thumbnail-value");
    thumbnail_heading.append(&thumbnail_title);
    thumbnail_heading.append(&thumbnail_value);
    let thumbnail_scale = gtk::Scale::with_range(
        gtk::Orientation::Horizontal,
        f64::from(MIN_GRID_THUMBNAIL_SIZE),
        f64::from(MAX_GRID_THUMBNAIL_SIZE),
        16.0,
    );
    thumbnail_scale.add_css_class("grid-thumbnail-scale");
    thumbnail_scale.set_draw_value(false);
    thumbnail_scale.set_value(f64::from(thumbnail_size));
    thumbnail_scale.set_size_request(220, -1);
    let thumbnail_extremes = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    thumbnail_extremes.add_css_class("grid-thumbnail-extremes");
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
    thumbnail_menu.add_css_class("grid-thumbnail-menu");
    thumbnail_menu.set_child(Some(&crate::assets::primary_icon(
        crate::assets::icons::PICTURES,
        16,
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

    let (filter_entry, filter_revealer, filter_button) = filter_controls("Filter grid (Ctrl+F)");
    actions.append(&filter_button);
    GridControls {
        leading,
        actions,
        filter_entry,
        filter_revealer,
        filter_button,
        thumbnail_scale,
        thumbnail_value,
        empty_trash_button: is_trash.then_some(empty_trash),
    }
}

/// Shared wiring every grid view in a pane needs, so a pane that groups entries by
/// type can build one view per group without threading a dozen arguments through.
struct GridContext {
    browser: Rc<Browser>,
    depth: usize,
    click: ModeClickOptions,
    transfer: TransferHandlerSlot,
    cuts: Rc<RefCell<HashSet<Location>>>,
    state: Option<Weak<super::browser::ViewState>>,
    thumbnail_size: Rc<Cell<i32>>,
    active_new_entry: Rc<RefCell<Option<ActiveModeNewEntry>>>,
    new_entry_is_directory: Rc<Cell<bool>>,
    source_index: SourceIndexMap,
    sections: Weak<RefCell<Vec<PaneSection>>>,
    density: Cell<BrowserDensity>,
}

type GridGroupBuilder = Rc<dyn Fn(&str) -> GridGroup>;

#[derive(Clone)]
struct GridGroup {
    label: String,
    heading: gtk::Widget,
    section: PaneSection,
}

/// The grouped grid's heading-and-grid pairs, rebuilt as the file-type groups a
/// directory contains change.
struct GridGroups {
    container: gtk::Box,
    placeholder: gtk::Widget,
    groups: RefCell<Vec<GridGroup>>,
    build: RefCell<Option<GridGroupBuilder>>,
}

fn build_grid_pane(
    browser: Rc<Browser>,
    click_options: ModeClickOptions,
    transfer_handler: TransferHandlerSlot,
    cut_locations: Rc<RefCell<HashSet<Location>>>,
    options: GridOptions,
    depth: usize,
    title: &str,
) -> Pane {
    let controls = grid_controls(&browser, depth, options.thumbnail_size.get());
    let (shell, header, content, model, stack, status, spinner, truncated_hint) = pane_base(
        title,
        "grid-pane",
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
    let new_entry_placeholder = gtk::StringList::new(&[]);
    let new_entry_is_directory = Rc::new(Cell::new(true));
    let sections: Rc<RefCell<Vec<PaneSection>>> = Rc::new(RefCell::new(Vec::new()));
    let context = Rc::new(GridContext {
        browser,
        depth,
        click: click_options,
        transfer: transfer_handler,
        cuts: cut_locations,
        state: options.state,
        thumbnail_size: options.thumbnail_size.clone(),
        active_new_entry: options.active_new_entry,
        new_entry_is_directory: new_entry_is_directory.clone(),
        source_index: source_index.clone(),
        sections: Rc::downgrade(&sections),
        density: Cell::new(options.density),
    });
    let (root, pane_section, groups) = if options.group_by_type {
        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        container.add_css_class("grid-type-groups");
        let placeholder = build_grid_view(&context, &new_entry_placeholder, false);
        placeholder.view.set_visible(false);
        let placeholder_view = placeholder.view.clone();
        new_entry_placeholder.connect_items_changed(move |model, _, _, _| {
            placeholder_view.set_visible(model.n_items() > 0);
        });
        container.append(&placeholder.view);
        // Groups take their natural height and the filler soaks up what is left, so a
        // short group does not stretch to fill the viewport. It also keeps the blank
        // area below the last group inside the marquee's drag surface.
        let filler = gtk::Box::new(gtk::Orientation::Vertical, 0);
        filler.set_vexpand(true);
        container.append(&filler);
        let groups = Rc::new(GridGroups {
            container: container.clone(),
            placeholder: placeholder.view.clone(),
            groups: RefCell::new(Vec::new()),
            build: RefCell::new(None),
        });
        let build_context = context.clone();
        let build_model = filtered_model.clone();
        groups.build.replace(Some(Rc::new(move |label: &str| {
            build_grid_group(&build_context, &build_model, label)
        })));
        (container.upcast::<gtk::Widget>(), placeholder, Some(groups))
    } else {
        let flattened_models = gio::ListStore::new::<gio::ListModel>();
        flattened_models.append(&new_entry_placeholder.clone().upcast::<gio::ListModel>());
        flattened_models.append(&filtered_model.clone().upcast::<gio::ListModel>());
        let view_model = gtk::FlattenListModel::new(Some(flattened_models));
        let section = build_grid_view(&context, &view_model, true);
        sections.borrow_mut().push(section.clone());
        (section.view.clone(), section, None)
    };

    let pending_thumbnail_resize = Rc::new(RefCell::new(None::<glib::SourceId>));
    let groups_for_pane = groups.clone();
    let density_for_size = context.density.get();
    let sections_for_size = Rc::downgrade(&sections);
    let browser_for_size = Rc::downgrade(&context.browser);
    let source_index_for_size = source_index.clone();
    let thumbnail_size_for_change = options.thumbnail_size.clone();
    let value_for_change = controls.thumbnail_value.clone();
    controls
        .thumbnail_scale
        .connect_value_changed(move |scale| {
            let size = scale.value().round() as i32;
            value_for_change.set_label(&format!("{size} px"));
            if let Some(pending) = pending_thumbnail_resize.take() {
                pending.remove();
            }
            let pending_for_timeout = pending_thumbnail_resize.clone();
            let browser = browser_for_size.clone();
            let source_index = source_index_for_size.clone();
            let sections = sections_for_size.clone();
            let groups_for_size = groups_for_pane.clone();
            let size_state = thumbnail_size_for_change.clone();
            let source_id =
                glib::timeout_add_local_once(std::time::Duration::from_millis(100), move || {
                    pending_for_timeout.take();
                    size_state.set(size);
                    let Some(sections) = sections.upgrade() else {
                        return;
                    };
                    for section in sections.borrow().iter() {
                        refresh_grid_thumbnail_size(&browser, depth, &source_index, section, size);
                    }
                    if let Some(groups) = groups_for_size.as_ref() {
                        refresh_group_columns(groups, groups.container.width(), density_for_size);
                    }
                });
            pending_thumbnail_resize.replace(Some(source_id));
        });

    let scroll = gtk::ScrolledWindow::builder()
        .child(&root)
        .hscrollbar_policy(if groups.is_some() {
            // Grouped grids wrap to the pane's width; only the ungrouped grid manages
            // its own horizontal scrolling.
            gtk::PolicyType::Never
        } else {
            gtk::PolicyType::Automatic
        })
        .vexpand(true)
        .build();
    scroll.add_css_class("fixed-scrollbar");
    if let Some(groups) = groups.clone() {
        let context = Rc::downgrade(&context);
        scroll
            .hadjustment()
            .connect_page_size_notify(move |adjustment| {
                let Some(context) = context.upgrade() else {
                    return;
                };
                refresh_group_columns(
                    &groups,
                    adjustment.page_size() as i32,
                    context.density.get(),
                );
            });
    }
    let targets: super::marquee::MarqueeTargets = Rc::new(RefCell::new(Vec::new()));
    let (collection, marquee) =
        collection_with_marquee(&root, scroll, targets.clone(), "grid-card");
    content.append(&collection);
    marquee.add_origin_surface(&header);
    let pane = Pane {
        depth,
        shell,
        model,
        source_index,
        filter_model: Some(filtered_model),
        section: pane_section,
        sections,
        groups,
        grid: Some(context),
        targets,
        detached: Rc::new(Cell::new(false)),
        stack,
        status,
        spinner,
        truncated_hint,
        marquee,
        filter_entry: Some(controls.filter_entry),
        filter_button: Some(controls.filter_button),
        empty_trash_button: controls.empty_trash_button,
        new_entry_placeholder: Some(new_entry_placeholder),
        new_entry_is_directory: Some(new_entry_is_directory),
        show_hidden,
        filter: filter_for_pane,
    };
    refresh_marquee_targets(&pane);
    pane
}

/// A heading and the grid that renders one file-type group.
fn build_grid_group(
    context: &Rc<GridContext>,
    entries: &gtk::FilterListModel,
    label: &str,
) -> GridGroup {
    let heading = type_group_heading(label);
    let group_model = gtk::FilterListModel::new(
        Some(entries.clone()),
        Some(type_group_filter(label.to_owned())),
    );
    let section = build_grid_view(context, &group_model, true);
    let heading_for_items = heading.clone();
    let view_for_items = section.view.clone();
    let update_visibility = move |populated: bool| {
        heading_for_items.set_visible(populated);
        view_for_items.set_visible(populated);
    };
    update_visibility(group_model.n_items() > 0);
    group_model.connect_items_changed(move |model, _, _, _| {
        update_visibility(model.n_items() > 0);
    });
    GridGroup {
        label: label.to_owned(),
        heading: heading.upcast(),
        section,
    }
}

fn build_grid_view(
    context: &Rc<GridContext>,
    model: &impl IsA<gio::ListModel>,
    syncs_selection: bool,
) -> PaneSection {
    let depth = context.depth;
    let view_model = model.clone().upcast::<gio::ListModel>();
    let selection = gtk::MultiSelection::new(Some(view_model.clone()));
    let syncing_selection = Rc::new(Cell::new(false));
    let bound_items: Rc<RefCell<Vec<BoundModeItem>>> = Rc::new(RefCell::new(Vec::new()));
    let factory = gtk::SignalListItemFactory::new();
    let bound_items_for_setup = bound_items.clone();
    let selection_for_setup = selection.clone();
    let selection_anchor = Rc::new(Cell::new(None::<u32>));
    let browser_for_setup = Rc::downgrade(&context.browser);
    let previews_for_setup = context.click.previews.clone();
    let activation_for_setup = context.click.activation.clone();
    let filtered_for_setup = view_model.clone();
    let source_index_for_setup = context.source_index.clone();
    let transfers_for_setup = context.transfer.clone();
    let peek_for_setup = context.state.clone();
    let active_for_setup = context.active_new_entry.clone();
    let folder_location = context.browser.location_at(depth);
    factory.connect_setup(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
        card.add_css_class("grid-card");
        card.add_css_class("file-appear");
        let weak_card = card.downgrade();
        glib::idle_add_local_once(move || {
            if let Some(card) = weak_card.upgrade() {
                card.remove_css_class("file-appear");
            }
        });
        card.set_halign(gtk::Align::Center);
        card.set_valign(gtk::Align::Center);
        let centered = gtk::CenterBox::new();
        centered.set_orientation(gtk::Orientation::Vertical);
        centered.set_vexpand(true);
        let item_content = gtk::Box::new(gtk::Orientation::Vertical, 3);
        item_content.set_halign(gtk::Align::Center);
        item_content.set_valign(gtk::Align::Center);
        let icon = gtk::Image::new();
        icon.set_pixel_size(26);
        icon.add_css_class("grid-card-icon");
        let label = gtk::Label::new(None);
        label.add_css_class("grid-card-label");
        label.add_css_class("alternate-rename-label");
        label.set_justify(gtk::Justification::Center);
        label.set_width_chars(12);
        label.set_max_width_chars(16);
        label.set_wrap(true);
        label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        let field = gtk::Entry::new();
        field.add_css_class("inline-rename");
        field.set_width_chars(12);
        field.set_visible(false);
        field.connect_changed(|field| {
            super::browser::update_basename_validation(field);
        });
        let active_for_submit = active_for_setup.clone();
        let browser_for_submit = browser_for_setup.clone();
        let location_for_submit = folder_location.clone();
        field.connect_activate(move |field| {
            submit_mode_new_entry(
                &active_for_submit,
                &browser_for_submit,
                &location_for_submit,
                field,
            );
        });
        let focus = gtk::EventControllerFocus::new();
        let active_for_leave = active_for_setup.clone();
        let browser_for_leave = browser_for_setup.clone();
        let location_for_leave = folder_location.clone();
        let field_for_leave = field.clone();
        focus.connect_leave(move |_| {
            submit_mode_new_entry(
                &active_for_leave,
                &browser_for_leave,
                &location_for_leave,
                &field_for_leave,
            );
        });
        field.add_controller(focus);
        item_content.append(&icon);
        item_content.append(&label);
        item_content.append(&field);
        centered.set_center_widget(Some(&item_content));
        card.append(&centered);
        install_preview_click(
            &card,
            item,
            browser_for_setup.clone(),
            previews_for_setup.clone(),
            activation_for_setup.clone(),
            depth,
            Some((source_index_for_setup.clone(), filtered_for_setup.clone())),
        );
        install_modified_selection_click(
            &card,
            item,
            selection_for_setup.clone(),
            selection_anchor.clone(),
        );
        install_grid_peek(
            &card,
            item,
            peek_for_setup.clone(),
            browser_for_setup.clone(),
            source_index_for_setup.clone(),
            filtered_for_setup.clone(),
            depth,
        );
        install_explorer_drag_drop(
            &card,
            item,
            browser_for_setup.clone(),
            transfers_for_setup.clone(),
            depth,
            Some((source_index_for_setup.clone(), filtered_for_setup.clone())),
        );
        item.set_child(Some(&card));
        register_bound_mode_item(&bound_items_for_setup, item, &card);
    });
    let browser_for_bind = Rc::downgrade(&context.browser);
    let source_index_for_bind = context.source_index.clone();
    let cuts_for_bind = context.cuts.clone();
    let thumbnail_size_for_bind = context.thumbnail_size.clone();
    let entry_kind_for_bind = context.new_entry_is_directory.clone();
    factory.connect_bind(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(card) = item.child().and_downcast::<gtk::Box>() else {
            return;
        };
        let Some(centered) = card.first_child().and_downcast::<gtk::CenterBox>() else {
            return;
        };
        let Some(item_content) = centered.center_widget().and_downcast::<gtk::Box>() else {
            return;
        };
        let Some(icon) = item_content.first_child().and_downcast::<gtk::Image>() else {
            return;
        };
        let Some(label) = icon.next_sibling().and_downcast::<gtk::Label>() else {
            return;
        };
        let Some(field) = label.next_sibling().and_downcast::<gtk::Entry>() else {
            return;
        };
        let source_position = item
            .item()
            .and_then(|value| source_index_for_bind.of_item(&value));
        let browser = browser_for_bind.upgrade();
        let entry = browser.as_ref().and_then(|browser| {
            source_position.and_then(|position| browser.entry_at(depth, position))
        });
        if let Some(entry) = entry {
            label.set_visible(true);
            field.set_visible(false);
            set_mode_cut_style(&card, cuts_for_bind.borrow().contains(&entry.location));
            label.set_label(&entry.display_name);
            label.set_tooltip_text(Some(&entry.display_name));
            super::thumbnail::set_thumbnail_or_icon(
                &icon,
                &entry,
                super::browser::entry_icon(&entry),
                26,
                thumbnail_size_for_bind.get(),
            );
            if let Some(position) = metadata_fill_position(source_position, &entry, false)
                && let Some(browser) = browser.as_ref()
            {
                browser.request_metadata_fill(depth, position, entry.location.clone());
            }
            icon.set_opacity(if entry.is_directory() { 1.0 } else { 0.72 });
        } else {
            card.remove_css_class("cut-item");
            let icon_name = if entry_kind_for_bind.get() {
                crate::assets::icons::FOLDER
            } else {
                crate::assets::icons::DOCUMENTS
            };
            crate::assets::set_primary_icon(&icon, icon_name);
            icon.set_opacity(1.0);
            label.set_visible(false);
            field.set_visible(true);
        }
    });
    factory.connect_unbind(|_, item| super::thumbnail::cancel_list_item_thumbnails(item));
    let view = gtk::GridView::new(Some(selection.clone()), Some(factory));
    view.add_css_class("file-grid");
    view.set_vexpand(false);
    view.set_enable_rubberband(false);
    view.set_single_click_activate(false);
    configure_grid_view_density(&view, context.density.get());

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
    if syncs_selection {
        connect_selection(
            &section,
            context.sections.clone(),
            &context.browser,
            depth,
            context.source_index.clone(),
        );
        install_exclusive_section_click(&section, context);
    }
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

/// Keeps the grouped grid's sections in step with the file types the directory
/// holds, adding and removing a heading and grid per type.
fn sync_grid_groups(pane: &Pane) {
    let Some(groups) = pane.groups.clone() else {
        return;
    };
    let Some(build) = groups.build.borrow().clone() else {
        return;
    };
    let desired = source_type_groups(&pane.model);
    let existing = groups.groups.borrow().clone();
    if existing.len() == desired.len()
        && existing
            .iter()
            .zip(desired.iter())
            .all(|(group, label)| group.label == *label)
    {
        return;
    }
    for group in &existing {
        if !desired.contains(&group.label) {
            groups.container.remove(&group.heading);
            groups.container.remove(&group.section.view);
        }
    }
    let mut next = Vec::with_capacity(desired.len());
    let mut previous = groups.placeholder.clone();
    for label in &desired {
        let group = match existing.iter().find(|group| group.label == *label) {
            Some(group) => {
                groups
                    .container
                    .reorder_child_after(&group.heading, Some(&previous));
                groups
                    .container
                    .reorder_child_after(&group.section.view, Some(&group.heading));
                group.clone()
            }
            None => {
                let group = build(label);
                groups
                    .container
                    .insert_child_after(&group.heading, Some(&previous));
                groups
                    .container
                    .insert_child_after(&group.section.view, Some(&group.heading));
                group
            }
        };
        previous = group.section.view.clone();
        next.push(group);
    }
    *pane.sections.borrow_mut() = next.iter().map(|group| group.section.clone()).collect();
    *groups.groups.borrow_mut() = next;
    refresh_marquee_targets(pane);
    if let Some(context) = pane.grid.as_ref() {
        let density = context.density.get();
        let groups = groups.clone();
        // Cards bind during the next layout pass, so the columns they allow are only
        // measurable once it has run.
        glib::idle_add_local_once(move || {
            refresh_group_columns(&groups, groups.container.width(), density);
        });
    }
}

/// A grouped grid shares one scroller with its siblings, so it has to ask for the
/// height its own rows need. `GtkGridView` only knows its row count once its column
/// count is fixed, so the columns are pinned to what the viewport width allows and
/// recomputed whenever that width or the card size changes.
fn refresh_group_columns(groups: &Rc<GridGroups>, width: i32, density: BrowserDensity) {
    if width <= 0 {
        return;
    }
    let groups = groups.groups.borrow();
    let column = groups
        .iter()
        .find_map(|group| measured_card_width(&group.section))
        .unwrap_or(FALLBACK_GRID_COLUMN_WIDTH);
    let columns = (width / column.max(1)).clamp(1, density_grid_columns(density) as i32) as u32;
    for group in groups.iter() {
        let Ok(grid) = group.section.view.clone().downcast::<gtk::GridView>() else {
            continue;
        };
        if grid.min_columns() == columns && grid.max_columns() == columns {
            continue;
        }
        if columns > grid.max_columns() {
            grid.set_max_columns(columns);
            grid.set_min_columns(columns);
        } else {
            grid.set_min_columns(columns);
            grid.set_max_columns(columns);
        }
    }
}

fn measured_card_width(section: &PaneSection) -> Option<i32> {
    section.bound_items.borrow().iter().find_map(|bound| {
        let widget = bound.widget.upgrade()?;
        let (_, natural, _, _) = widget.measure(gtk::Orientation::Horizontal, -1);
        (natural > 0).then_some(natural + GRID_CARD_SPACING)
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

fn refresh_grid_thumbnail_size(
    browser: &Weak<Browser>,
    depth: usize,
    source_index: &SourceIndexMap,
    section: &PaneSection,
    size: i32,
) {
    let Some(browser) = browser.upgrade() else {
        return;
    };
    section.bound_items.borrow().iter().for_each(|bound| {
        let Some(item) = bound.item.upgrade() else {
            return;
        };
        let Some(card) = bound.widget.upgrade().and_downcast::<gtk::Box>() else {
            return;
        };
        let Some(icon) = card
            .first_child()
            .and_downcast::<gtk::CenterBox>()
            .and_then(|centered| centered.center_widget())
            .and_downcast::<gtk::Box>()
            .and_then(|content| content.first_child())
            .and_downcast::<gtk::Image>()
        else {
            return;
        };
        let Some(position) = item.item().and_then(|value| source_index.of_item(&value)) else {
            return;
        };
        let Some(entry) = browser.entry_at(depth, position) else {
            return;
        };
        super::thumbnail::set_thumbnail_or_icon(
            &icon,
            &entry,
            super::browser::entry_icon(&entry),
            26,
            size,
        );
        icon.set_opacity(if entry.is_directory() { 1.0 } else { 0.72 });
    });
}

fn configure_grid_density(pane: &Pane, density: BrowserDensity) {
    if let Some(context) = pane.grid.as_ref() {
        context.density.set(density);
    }
    for section in pane.all_sections() {
        if let Ok(grid) = section.view.clone().downcast::<gtk::GridView>() {
            configure_grid_view_density(&grid, density);
        }
    }
    if let Some(groups) = pane.groups.as_ref() {
        refresh_group_columns(groups, groups.container.width(), density);
    }
}

fn configure_grid_view_density(grid: &gtk::GridView, density: BrowserDensity) {
    grid.set_min_columns(1);
    grid.set_max_columns(density_grid_columns(density));
}

fn density_grid_columns(density: BrowserDensity) -> u32 {
    match density {
        BrowserDensity::Compact => 20,
        BrowserDensity::Airy => 16,
    }
}

fn explorer_headings(
    browser: &Rc<Browser>,
    depth: usize,
    columns: ExplorerColumnLayout,
) -> gtk::Box {
    let headings = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    headings.add_css_class("explorer-headings");
    let preferences = browser.column_preferences(depth).unwrap_or_default();
    let sorting = Rc::new(Cell::new((
        preferences.sort_key,
        preferences.sort_direction,
    )));
    let arrows: Rc<RefCell<Vec<(SortKey, gtk::Image)>>> = Rc::new(RefCell::new(Vec::new()));

    for (index, (text, key, width)) in [
        ("Name", Some(SortKey::Name), EXPLORER_COLUMN_WIDTHS[0]),
        ("Mode", None, EXPLORER_COLUMN_WIDTHS[1]),
        ("Size", Some(SortKey::Size), EXPLORER_COLUMN_WIDTHS[2]),
        ("Type", Some(SortKey::Type), EXPLORER_COLUMN_WIDTHS[3]),
        (
            "Modified",
            Some(SortKey::Modified),
            EXPLORER_COLUMN_WIDTHS[4],
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let cell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        cell.add_css_class("explorer-heading-cell");
        register_explorer_column_cell(&columns, index, &cell);

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
        button.add_css_class("explorer-heading-button");
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

fn register_explorer_column_cell(
    columns: &ExplorerColumnLayout,
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

fn set_explorer_column_width(columns: &ExplorerColumnLayout, index: usize, width: i32) {
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

fn column_resize_handle(
    columns: ExplorerColumnLayout,
    index: usize,
    initial_width: i32,
) -> gtk::Box {
    let handle = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    handle.add_css_class("explorer-column-resize-handle");
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
            set_explorer_column_width(
                &columns_for_autofit,
                index,
                explorer_column_width(index, natural),
            );
            gesture.set_state(gtk::EventSequenceState::Denied);
            return;
        }
        let width = columns_for_begin.cells[index]
            .borrow()
            .iter()
            .find_map(glib::WeakRef::upgrade)
            .map_or(initial_width, |widget| widget.width());
        starting_for_begin.set(explorer_column_width(index, width));
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
        set_explorer_column_width(
            &columns_for_update,
            index,
            explorer_column_width(index, width),
        );
    });
    handle.add_controller(resize);
    handle
}

fn explorer_column_width(index: usize, width: i32) -> i32 {
    width.max(EXPLORER_COLUMN_MIN_WIDTHS[index])
}

fn explorer_navigation(browser: &Rc<Browser>) -> gtk::Box {
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    actions.add_css_class("explorer-navigation");
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
        button.set_child(Some(&crate::assets::primary_icon(icon, 16)));
        button.add_css_class("explorer-navigation-button");
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

fn build_explorer_pane(
    browser: Rc<Browser>,
    click_options: ModeClickOptions,
    transfer_handler: TransferHandlerSlot,
    cut_locations: Rc<RefCell<HashSet<Location>>>,
    options: ExplorerOptions,
    depth: usize,
    title: &str,
) -> Pane {
    let active_new_entry = options.active_new_entry.clone();
    let navigation = explorer_navigation(&browser);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    actions.add_css_class("grid-header-actions");
    let empty_trash = super::browser::empty_trash_button(&browser);
    let is_trash = browser
        .location_at(depth)
        .is_some_and(|location| super::browser::is_trash_root(&location));
    empty_trash.set_visible(is_trash);
    empty_trash.set_sensitive(false);
    actions.append(&empty_trash);
    actions.append(&super::browser::pane_refresh_button(&browser, depth));
    let (filter_entry, filter_revealer, filter_button) =
        filter_controls("Filter explorer (Ctrl+F)");
    actions.append(&filter_button);
    let (shell, header, content, model, stack, status, spinner, truncated_hint) = pane_base(
        title,
        "explorer-pane",
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
    let new_entry_placeholder = gtk::StringList::new(&[]);
    let new_entry_is_directory = Rc::new(Cell::new(true));
    let flattened_models = gio::ListStore::new::<gio::ListModel>();
    flattened_models.append(&new_entry_placeholder.clone().upcast::<gio::ListModel>());
    flattened_models.append(&filtered_model.clone().upcast::<gio::ListModel>());
    let flattened = gtk::FlattenListModel::new(Some(flattened_models));
    let view_model = gtk::SortListModel::new(Some(flattened), None::<gtk::CustomSorter>);
    if options.group_by_type {
        let sorter = type_group_sorter();
        view_model.set_sorter(Some(&sorter));
        view_model.set_section_sorter(Some(&sorter));
    }
    let view_model_object = view_model.clone().upcast::<gio::ListModel>();
    let selection = gtk::MultiSelection::new(Some(view_model.clone()));
    let syncing_selection = Rc::new(Cell::new(false));
    let sections: Rc<RefCell<Vec<PaneSection>>> = Rc::new(RefCell::new(Vec::new()));

    let columns = ExplorerColumnLayout::new();
    let headings = explorer_headings(&browser, depth, columns.clone());

    let factory = gtk::SignalListItemFactory::new();
    let bound_items: Rc<RefCell<Vec<BoundModeItem>>> = Rc::new(RefCell::new(Vec::new()));
    let bound_items_for_setup = bound_items.clone();
    let selection_for_setup = selection.clone();
    let selection_anchor = Rc::new(Cell::new(None::<u32>));
    let browser_for_setup = Rc::downgrade(&browser);
    let previews_for_setup = click_options.previews;
    let activation_for_setup = click_options.activation;
    let transfers_for_setup = transfer_handler.clone();
    let active_for_setup = active_new_entry.clone();
    let source_index_for_setup = source_index.clone();
    let view_model_for_setup = view_model_object.clone();
    let folder_location = browser.location_at(depth);
    factory.connect_setup(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        row.add_css_class("explorer-row");
        row.add_css_class("file-appear");
        let weak_row = row.downgrade();
        glib::idle_add_local_once(move || {
            if let Some(row) = weak_row.upgrade() {
                row.remove_css_class("file-appear");
            }
        });
        let name_cell = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        name_cell.add_css_class("explorer-name-cell");
        let icon = gtk::Image::new();
        icon.set_pixel_size(18);
        let name = gtk::Label::new(None);
        name.add_css_class("alternate-rename-label");
        name.set_xalign(0.0);
        name.set_hexpand(true);
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        // Keep the label's natural width from widening this fixed-width table cell.
        name.set_max_width_chars(1);
        let field = gtk::Entry::new();
        field.add_css_class("inline-rename");
        field.set_hexpand(true);
        field.set_visible(false);
        field.connect_changed(|field| {
            super::browser::update_basename_validation(field);
        });
        let active_for_submit = active_for_setup.clone();
        let browser_for_submit = browser_for_setup.clone();
        let location_for_submit = folder_location.clone();
        field.connect_activate(move |field| {
            submit_mode_new_entry(
                &active_for_submit,
                &browser_for_submit,
                &location_for_submit,
                field,
            );
        });
        let focus = gtk::EventControllerFocus::new();
        let active_for_leave = active_for_setup.clone();
        let browser_for_leave = browser_for_setup.clone();
        let location_for_leave = folder_location.clone();
        let field_for_leave = field.clone();
        focus.connect_leave(move |_| {
            submit_mode_new_entry(
                &active_for_leave,
                &browser_for_leave,
                &location_for_leave,
                &field_for_leave,
            );
        });
        field.add_controller(focus);
        name_cell.append(&icon);
        name_cell.append(&name);
        name_cell.append(&field);
        let mode = explorer_metadata_label();
        let size = explorer_metadata_label();
        let kind = explorer_metadata_label();
        let modified = explorer_metadata_label();
        for (index, widget) in [
            name_cell.clone().upcast::<gtk::Widget>(),
            mode.clone().upcast(),
            size.clone().upcast(),
            kind.clone().upcast(),
            modified.clone().upcast(),
        ]
        .into_iter()
        .enumerate()
        {
            register_explorer_column_cell(&columns, index, &widget);
        }
        row.append(&name_cell);
        row.append(&mode);
        row.append(&size);
        row.append(&kind);
        row.append(&modified);
        install_preview_click(
            &row,
            item,
            browser_for_setup.clone(),
            previews_for_setup.clone(),
            activation_for_setup.clone(),
            depth,
            Some((source_index_for_setup.clone(), view_model_for_setup.clone())),
        );
        install_modified_selection_click(
            &row,
            item,
            selection_for_setup.clone(),
            selection_anchor.clone(),
        );
        install_explorer_drag_drop(
            &row,
            item,
            browser_for_setup.clone(),
            transfers_for_setup.clone(),
            depth,
            Some((source_index_for_setup.clone(), view_model_for_setup.clone())),
        );
        item.set_child(Some(&row));
        register_bound_mode_item(&bound_items_for_setup, item, &row);
    });
    let browser_for_bind = Rc::downgrade(&browser);
    let source_index_for_bind = source_index.clone();
    let cuts_for_bind = cut_locations.clone();
    let entry_kind_for_bind = new_entry_is_directory.clone();
    factory.connect_bind(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(row) = item.child().and_downcast::<gtk::Box>() else {
            return;
        };
        let Some(name_cell) = row.first_child().and_downcast::<gtk::Box>() else {
            return;
        };
        let Some(icon) = name_cell.first_child().and_downcast::<gtk::Image>() else {
            return;
        };
        let Some(name) = icon.next_sibling().and_downcast::<gtk::Label>() else {
            return;
        };
        let Some(field) = name.next_sibling().and_downcast::<gtk::Entry>() else {
            return;
        };
        let Some(mode) = name_cell.next_sibling().and_downcast::<gtk::Label>() else {
            return;
        };
        let Some(size) = mode.next_sibling().and_downcast::<gtk::Label>() else {
            return;
        };
        let Some(kind) = size.next_sibling().and_downcast::<gtk::Label>() else {
            return;
        };
        let Some(modified) = kind.next_sibling().and_downcast::<gtk::Label>() else {
            return;
        };
        let source_position = item
            .item()
            .and_then(|value| source_index_for_bind.of_item(&value));
        let browser = browser_for_bind.upgrade();
        let entry = browser.as_ref().and_then(|browser| {
            source_position.and_then(|position| browser.entry_at(depth, position))
        });
        if let Some(entry) = entry {
            name.set_visible(true);
            field.set_visible(false);
            set_mode_cut_style(&row, cuts_for_bind.borrow().contains(&entry.location));
            super::thumbnail::set_thumbnail_or_icon(
                &icon,
                &entry,
                super::browser::entry_icon(&entry),
                18,
                18,
            );
            if let Some(position) = metadata_fill_position(source_position, &entry, true)
                && let Some(browser) = browser.as_ref()
            {
                browser.request_metadata_fill(depth, position, entry.location.clone());
            }
            name.set_label(&entry.display_name);
            mode.set_label(&entry_mode(&entry));
            size.set_label(&entry_size(&entry));
            kind.set_label(entry_type(&entry));
            crate::util::set_modified_date(&modified, Some(&entry), "—");
        } else {
            row.remove_css_class("cut-item");
            let icon_name = if entry_kind_for_bind.get() {
                crate::assets::icons::FOLDER
            } else {
                crate::assets::icons::DOCUMENTS
            };
            crate::assets::set_primary_icon(&icon, icon_name);
            name.set_visible(false);
            field.set_visible(true);
            mode.set_label("");
            size.set_label("");
            kind.set_label("");
            crate::util::set_modified_date(&modified, None, "");
        }
    });
    factory.connect_unbind(|_, item| super::thumbnail::cancel_list_item_thumbnails(item));
    let view = gtk::ListView::new(Some(selection.clone()), Some(factory));
    view.add_css_class("explorer-list");
    if options.group_by_type {
        view.set_header_factory(Some(&type_group_header_factory()));
    }
    view.set_enable_rubberband(false);
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
            browser.activate(depth, position);
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
    sections.borrow_mut().push(section.clone());
    connect_selection(
        &section,
        Rc::downgrade(&sections),
        &browser,
        depth,
        source_index.clone(),
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
    let table = gtk::Box::new(gtk::Orientation::Vertical, 0);
    table.set_vexpand(true);
    table.append(&headings);
    let targets: super::marquee::MarqueeTargets = Rc::new(RefCell::new(Vec::new()));
    let (collection, marquee) =
        collection_with_marquee(view.upcast_ref(), scroll, targets.clone(), "explorer-row");
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
    content.append(&table_scroll);
    let pane = Pane {
        depth,
        shell,
        model,
        source_index,
        filter_model: Some(filtered_model),
        section,
        sections,
        groups: None,
        grid: None,
        targets,
        detached: Rc::new(Cell::new(false)),
        stack,
        status,
        spinner,
        truncated_hint,
        marquee,
        filter_entry: Some(filter_entry),
        filter_button: Some(filter_button),
        empty_trash_button: is_trash.then_some(empty_trash),
        new_entry_placeholder: Some(new_entry_placeholder),
        new_entry_is_directory: Some(new_entry_is_directory),
        show_hidden,
        filter: filter_for_pane,
    };
    refresh_marquee_targets(&pane);
    pane
}

fn pane_base(
    title: &str,
    class: &str,
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
    let shell = gtk::Box::new(gtk::Orientation::Vertical, 0);
    shell.add_css_class(class);
    shell.set_hexpand(true);
    shell.set_vexpand(true);
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    header.add_css_class("mode-pane-header");
    let heading_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    heading_box.set_hexpand(true);
    let heading = gtk::Label::new(Some(title));
    heading.set_xalign(0.0);
    let spinner = gtk::Spinner::new();
    spinner.start();
    let truncated_hint = crate::assets::primary_icon(crate::assets::icons::TRIANGLE_ALERT, 16);
    truncated_hint.set_tooltip_text(Some(
        "This directory has more entries than could be loaded; showing a partial listing.",
    ));
    truncated_hint.set_visible(false);
    heading_box.append(&heading);
    heading_box.append(&truncated_hint);
    if let Some(leading) = header_leading {
        header.append(&leading);
    }
    header.append(&heading_box);
    header.append(&spinner);
    if let Some(actions) = header_actions {
        header.append(&actions);
    }
    shell.append(&header);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.set_hexpand(true);
    content.set_vexpand(true);
    let loading = super::browser::loading_skeleton();
    let status = gtk::Label::new(Some("This directory is empty"));
    status.add_css_class("status-message");
    status.set_wrap(true);
    let stack = gtk::Stack::builder().hexpand(true).vexpand(true).build();
    stack.add_named(&content, Some("content"));
    stack.add_named(&loading, Some("loading"));
    stack.add_named(&status, Some("status"));
    stack.set_visible_child_name("loading");
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
) {
    let weak_item = glib::WeakRef::new();
    weak_item.set(Some(item));
    let weak_widget = glib::WeakRef::new();
    weak_widget.set(Some(widget.upcast_ref()));
    items.borrow_mut().push(BoundModeItem {
        item: weak_item,
        widget: weak_widget,
    });
}

fn collection_with_marquee(
    view: &gtk::Widget,
    scroll: gtk::ScrolledWindow,
    targets: super::marquee::MarqueeTargets,
    item_class: &'static str,
) -> (gtk::Overlay, super::marquee::Marquee) {
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&scroll));
    overlay.set_hexpand(true);
    overlay.set_vexpand(true);

    let marquee = super::marquee::install(super::marquee::MarqueeSetup {
        view: view.clone(),
        scroll,
        overlay: overlay.clone(),
        targets: targets.clone(),
        is_item: Rc::new(|widget| widget_or_ancestor_has_class(widget, item_class)),
    });

    let clear = gtk::GestureClick::new();
    clear.set_button(1);
    let press = Rc::new(Cell::new((0.0, 0.0)));
    let press_for_start = press.clone();
    clear.connect_pressed(move |_, _, x, y| press_for_start.set((x, y)));
    clear.connect_released(move |gesture, _, x, y| {
        let (start_x, start_y) = press.get();
        if (x - start_x).abs() > 3.0 || (y - start_y).abs() > 3.0 {
            return;
        }
        let target = gesture
            .widget()
            .and_then(|widget| widget.pick(x, y, gtk::PickFlags::DEFAULT));
        if !target.is_some_and(|widget| widget_or_ancestor_has_class(&widget, item_class)) {
            for target in targets.borrow().iter() {
                target.selection.unselect_all();
            }
        }
    });
    view.add_controller(clear);
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

fn widget_or_ancestor_has_class(widget: &gtk::Widget, class: &str) -> bool {
    let mut current = Some(widget.clone());
    while let Some(widget) = current {
        if widget.has_css_class(class) {
            return true;
        }
        current = widget.parent();
    }
    false
}

fn install_grid_peek(
    card: &gtk::Box,
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

fn install_explorer_drag_drop(
    row: &gtk::Box,
    item: &gtk::ListItem,
    browser: Weak<Browser>,
    transfer_handler: TransferHandlerSlot,
    depth: usize,
    position_map: Option<(SourceIndexMap, gio::ListModel)>,
) {
    let drag = gtk::DragSource::builder()
        .actions(gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE)
        .build();
    drag.set_propagation_phase(gtk::PropagationPhase::Capture);
    let dragged_item = item.downgrade();
    let browser_for_drag = browser.clone();
    let map_for_drag = position_map.clone();
    drag.connect_prepare(move |source, x, y| {
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
        let paintable = gtk::WidgetPaintable::new(source.widget().as_ref());
        source.set_icon(Some(&paintable), x.round() as i32, y.round() as i32);
        super::browser::file_drag_content(&entries)
    });
    let dragged_row = row.downgrade();
    drag.connect_drag_begin(move |_, _| {
        if let Some(row) = dragged_row.upgrade() {
            row.add_css_class("dragging");
        }
    });
    let dragged_row = row.downgrade();
    drag.connect_drag_end(move |_, _, _| {
        if let Some(row) = dragged_row.upgrade() {
            row.remove_css_class("dragging");
        }
    });
    row.add_controller(drag);

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
                .is_some_and(|entry| entry.is_directory())
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

fn install_modified_selection_click(
    widget: &impl IsA<gtk::Widget>,
    item: &gtk::ListItem,
    selection: gtk::MultiSelection,
    anchor: Rc<Cell<Option<u32>>>,
) {
    let click = gtk::GestureClick::new();
    click.set_button(1);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let item = item.downgrade();
    click.connect_pressed(move |gesture, _, _, _| {
        let Some(item) = item.upgrade() else {
            return;
        };
        let position = item.position();
        if position == gtk::INVALID_LIST_POSITION {
            return;
        }
        let modifiers = gesture.current_event_state();
        let control = modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK);
        let shift = modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK);
        if shift {
            let anchor = anchor.get().unwrap_or(position);
            let start = anchor.min(position);
            let count = anchor.max(position).saturating_sub(start) + 1;
            selection.select_range(start, count, true);
        } else if control {
            anchor.set(Some(position));
            if selection.is_selected(position) {
                selection.unselect_item(position);
            } else {
                selection.select_item(position, false);
            }
        } else {
            anchor.set(Some(position));
            return;
        }
        gesture.set_state(gtk::EventSequenceState::Claimed);
    });
    widget.add_controller(click);
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
    // Unfiltered views (and FlattenListModel with an empty placeholder) keep source order.
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

fn install_preview_click(
    widget: &impl IsA<gtk::Widget>,
    item: &gtk::ListItem,
    browser: Weak<Browser>,
    enabled: Rc<Cell<bool>>,
    click_activation: Rc<Cell<ClickActivation>>,
    depth: usize,
    position_map: Option<(SourceIndexMap, gio::ListModel)>,
) {
    let click = gtk::GestureClick::new();
    click.set_button(1);
    let item = item.downgrade();
    click.connect_released(move |gesture, press_count, _, _| {
        let modifiers = gesture.current_event_state();
        if modifiers
            .intersects(gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK)
        {
            return;
        }
        let Some(item) = item.upgrade() else {
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
        if should_activate_pointer_click(press_count, entry.is_directory(), click_activation.get())
        {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            browser.activate_in_place(depth, position);
        } else if press_count == 1
            && enabled.get()
            && !entry.is_directory()
            && super::browser::entry_supports_quick_preview(&entry)
        {
            browser.preview(depth, position);
        }
    });
    widget.add_controller(click);
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
) {
    let syncing = section.syncing.clone();
    let view_model = section.view_model.clone();
    let browser = Rc::downgrade(browser);
    section
        .selection
        .connect_selection_changed(move |selection, _, _| {
            if syncing.get() {
                return;
            }
            let (Some(sections), Some(browser)) = (sections.upgrade(), browser.upgrade()) else {
                return;
            };
            let focused = selected_source_positions(&source_index, &view_model, selection)
                .last()
                .copied();
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
    apply_collection_scroll(view, position);
    let view = view.clone();
    glib::idle_add_local_once(move || {
        apply_collection_scroll(&view, position);
    });
}

fn apply_collection_scroll(view: &gtk::Widget, position: u32) {
    if let Ok(list) = view.clone().downcast::<gtk::ListView>() {
        if position < list.model().map_or(0, |model| model.n_items()) {
            list.scroll_to(position, gtk::ListScrollFlags::FOCUS, None);
        }
        return;
    }
    if let Ok(grid) = view.clone().downcast::<gtk::GridView>()
        && position < grid.model().map_or(0, |model| model.n_items())
    {
        grid.scroll_to(position, gtk::ListScrollFlags::FOCUS, None);
    }
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
    let values_ref: Vec<&str> = values.iter().map(String::as_str).collect();
    pane.model.splice(0, pane.model.n_items(), &values_ref);
    sync_grid_groups(pane);
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
        pane.stack.set_visible_child_name("status");
    } else {
        pane.stack.set_visible_child_name("content");
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
        pane.stack.set_visible_child_name("loading");
    } else {
        pane.spinner.stop();
        if let Some(message) = snapshot.error.as_deref() {
            pane.status
                .set_label(&format!("Unable to read this directory\n{message}"));
            pane.status.add_css_class("error");
            pane.stack.set_visible_child_name("status");
        }
    }
}

fn clear_box(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

fn explorer_metadata_label() -> gtk::Label {
    let label = gtk::Label::new(None);
    label.add_css_class("explorer-metadata-cell");
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

fn update_bound_explorer_metadata(pane: &Pane, updates: &[(usize, FileEntry)]) {
    let updates: HashMap<usize, &FileEntry> = updates
        .iter()
        .map(|(position, entry)| (*position, entry))
        .collect();
    for section in pane.item_sections() {
        section.bound_items.borrow_mut().retain(|bound| {
            let (Some(item), Some(row)) = (bound.item.upgrade(), bound.widget.upgrade()) else {
                return false;
            };
            let Some(position) =
                source_position_for_view(&pane.model, Some(&section.view_model), item.position())
            else {
                return true;
            };
            let Some(entry) = updates.get(&position) else {
                return true;
            };
            let Some(name_cell) = row.first_child().and_downcast::<gtk::Box>() else {
                return true;
            };
            let Some(mode) = name_cell.next_sibling().and_downcast::<gtk::Label>() else {
                return true;
            };
            let Some(size) = mode.next_sibling().and_downcast::<gtk::Label>() else {
                return true;
            };
            let Some(kind) = size.next_sibling().and_downcast::<gtk::Label>() else {
                return true;
            };
            let Some(modified) = kind.next_sibling().and_downcast::<gtk::Label>() else {
                return true;
            };
            mode.set_label(&entry_mode(entry));
            size.set_label(&entry_size(entry));
            crate::util::set_modified_date(&modified, Some(entry), "—");
            true
        });
    }
}

/// Orders file-type groups: the inline new-entry row leads, then folders, then the
/// remaining labels alphabetically, so a group's place does not depend on which
/// entries happen to be loaded.
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

/// The group a model value belongs to. The inline new-entry row carries no value and
/// stays in a group of its own, ahead of the entries.
fn value_type_group(value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    super::browser::model_type_group(value)
}

/// Sorts entries into their file-type groups. `GtkSortListModel` sorts stably, so
/// entries keep the pane's own sort order inside each group, and the same sorter
/// marks where one section ends and the next begins.
fn type_group_sorter() -> gtk::CustomSorter {
    gtk::CustomSorter::new(|left, right| {
        compare_type_groups(
            &value_type_group(&model_value(left)),
            &value_type_group(&model_value(right)),
        )
        .into()
    })
}

fn type_group_filter(label: String) -> gtk::CustomFilter {
    gtk::CustomFilter::new(move |item| value_type_group(&model_value(item)) == label)
}

/// Every file-type group the loaded entries fall into, in the order they are shown.
fn source_type_groups(model: &gtk::StringList) -> Vec<String> {
    type_groups_of((0..model.n_items()).filter_map(|index| model.string(index)))
}

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

/// A plain click selects only what it lands on, so it clears the sections it did not
/// land in. Modified clicks extend the selection and leave them alone.
fn install_exclusive_section_click(section: &PaneSection, context: &Rc<GridContext>) {
    let click = gtk::GestureClick::new();
    click.set_button(1);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let sections = context.sections.clone();
    let browser = Rc::downgrade(&context.browser);
    let source_index = context.source_index.clone();
    let depth = context.depth;
    click.connect_pressed(move |gesture, _, x, y| {
        if gesture
            .current_event_state()
            .intersects(gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK)
        {
            return;
        }
        let Some(view) = gesture.widget() else {
            return;
        };
        if !view
            .pick(x, y, gtk::PickFlags::DEFAULT)
            .is_some_and(|picked| widget_or_ancestor_has_class(&picked, "grid-card"))
        {
            return;
        }
        let (Some(sections), Some(browser)) = (sections.upgrade(), browser.upgrade()) else {
            return;
        };
        let mut cleared = false;
        for other in sections.borrow().iter() {
            if other.view == view || other.selection.selection().is_empty() {
                continue;
            }
            other.syncing.set(true);
            other.selection.unselect_all();
            other.syncing.set(false);
            cleared = true;
        }
        if cleared {
            sync_browser_selection(&sections, &browser, depth, &source_index, None);
        }
    });
    section.view.add_controller(click);
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
