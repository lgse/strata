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
const GRID_SCROLL_SETTLE_DELAY: std::time::Duration = std::time::Duration::from_millis(80);
/// Margin and padding a grid card adds around its own width.
const GRID_CARD_SPACING: i32 = 4;
const FALLBACK_GRID_COLUMN_WIDTH: i32 = 160;
const MIN_GRID_THUMBNAIL_SIZE: i32 = 64;
const MAX_GRID_THUMBNAIL_SIZE: i32 = 256;
const GRID_CARD_LABEL_CHARS: i32 = 16;
const GRID_CARD_LABEL_LINES: i32 = 2;
const GRID_CARD_LABEL_LINE_PX: i32 = 18;
const GRID_CARD_PAD_Y: i32 = 4;

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
    label: gtk::Widget,
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

/// One collection view inside a pane. Explorer and Grid each use a single section;
/// type grouping is a sort of that section, not extra views.
#[derive(Clone)]
struct PaneSection {
    view: gtk::Widget,
    view_model: gio::ListModel,
    selection: gtk::MultiSelection,
    bound_items: Rc<RefCell<Vec<BoundModeItem>>>,
    syncing: Rc<Cell<bool>>,
    visit: super::marquee::ItemVisitor,
}

impl PaneSection {
    fn item_bounds(&self, position: u32) -> Option<gtk::graphene::Rect> {
        self.bound_items.borrow().iter().find_map(|bound| {
            let item = bound.item.upgrade()?;
            if item.position() != position {
                return None;
            }
            let widget = bound.widget.upgrade()?;
            widget
                .is_mapped()
                .then(|| widget.compute_bounds(&self.view))
                .flatten()
        })
    }

    fn first_row_contains(&self, position: u32) -> bool {
        if position == 0 {
            return true;
        }
        if !self.view.is::<gtk::GridView>() {
            return false;
        }
        self.item_bounds(0)
            .zip(self.item_bounds(position))
            .is_some_and(|(first, current)| (first.y() - current.y()).abs() <= 1.0)
    }
}

#[derive(Clone)]
struct Pane {
    depth: usize,
    shell: gtk::Box,
    header: gtk::Box,
    model: gtk::StringList,
    source_index: SourceIndexMap,
    filter_model: Option<gtk::FilterListModel>,
    /// The section that owns the pane's chrome and hosts the inline new-entry row.
    section: PaneSection,
    sections: Rc<RefCell<Vec<PaneSection>>>,
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
    multiple_selection: Rc<Cell<bool>>,
    grid_click_activation: Rc<Cell<ClickActivation>>,
    explorer_click_activation: Rc<Cell<ClickActivation>>,
    transfer_handler: TransferHandlerSlot,
    cut_locations: Rc<RefCell<HashSet<Location>>>,
    context_state: RefCell<Option<Weak<super::browser::ViewState>>>,
    new_folder_state: RefCell<Option<Weak<super::browser::ViewState>>>,
    active_rename: Rc<RefCell<Option<ActiveModeRename>>>,
    active_new_entry: Rc<RefCell<Option<ActiveModeNewEntry>>>,
    mode: BrowserMode,
    density: BrowserDensity,
    group_by_type: bool,
    grid_thumbnail_size: Rc<Cell<i32>>,
    focus_before_header: RefCell<Option<glib::WeakRef<gtk::Widget>>>,
}

impl ModeViews {
    pub fn new(
        columns: &gtk::ScrolledWindow,
        browser: Rc<Browser>,
        multiple_selection: Rc<Cell<bool>>,
    ) -> Self {
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
            multiple_selection,
            grid_click_activation: Rc::new(Cell::new(ClickActivation::default_for(
                BrowserMode::Grid,
            ))),
            explorer_click_activation: Rc::new(Cell::new(ClickActivation::default_for(
                BrowserMode::Explorer,
            ))),
            transfer_handler: Rc::new(RefCell::new(None)),
            cut_locations: Rc::new(RefCell::new(HashSet::new())),
            context_state: RefCell::new(None),
            new_folder_state: RefCell::new(None),
            active_rename: Rc::new(RefCell::new(None)),
            active_new_entry: Rc::new(RefCell::new(None)),
            mode: BrowserMode::Columns,
            density: BrowserDensity::Compact,
            group_by_type: false,
            grid_thumbnail_size: Rc::new(Cell::new(DEFAULT_GRID_THUMBNAIL_SIZE)),
            focus_before_header: RefCell::new(None),
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

    fn single_pane(&self) -> Option<&Pane> {
        match self.mode {
            BrowserMode::Columns => None,
            BrowserMode::Grid => self.grid_panes.first(),
            BrowserMode::Explorer => self.explorer_pane.as_ref(),
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
                    if self.mode != BrowserMode::Grid {
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

    /// Use rendered rows: filtering, grouping, and resizing change the grid geometry.
    pub fn at_left_edge(&self) -> bool {
        if self.mode != BrowserMode::Grid {
            return true;
        }
        let Some(focused) = self.stack.root().and_then(|root| root.focus()) else {
            return true;
        };
        self.grid_panes
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

    /// GTK handles spatial movement within a grid; separate type groups need a handoff.
    pub fn move_grid_group(&self, direction: gtk::DirectionType) -> bool {
        if self.mode != BrowserMode::Grid {
            return false;
        }
        let Some(pane) = self.grid_panes.first() else {
            return false;
        };
        let sections = pane.item_sections();
        if sections.len() < 2 {
            return false;
        }
        let Some(focused) = self.stack.root().and_then(|root| root.focus()) else {
            return false;
        };
        let Some((index, position)) = sections.iter().enumerate().find_map(|(index, section)| {
            focused_section_item(section, &focused).map(|(position, _)| (index, position))
        }) else {
            return false;
        };
        let Some(grid) = sections[index].view.downcast_ref::<gtk::GridView>() else {
            return false;
        };
        let columns = grid.max_columns().max(1);
        let row = position / columns;
        let last_row = sections[index].view_model.n_items().saturating_sub(1) / columns;
        let next = match direction {
            gtk::DirectionType::Up if row == 0 => index.checked_sub(1),
            gtk::DirectionType::Down if row == last_row => Some(index + 1),
            _ => None,
        };
        let Some(target) = next.and_then(|index| sections.get(index)) else {
            return false;
        };
        let Some(last) = target.view_model.n_items().checked_sub(1) else {
            return false;
        };
        let Some(target_grid) = target.view.downcast_ref::<gtk::GridView>() else {
            return false;
        };
        let target_columns = target_grid.max_columns().max(1);
        let column = (position % columns).min(target_columns - 1);
        let target_position = if direction == gtk::DirectionType::Up {
            (last / target_columns * target_columns + column).min(last)
        } else {
            column.min(last)
        };
        let Some(source) = pane
            .source_index
            .of_view_position(&target.view_model, target_position)
        else {
            return false;
        };
        set_selections(pane, &[source]);
        self.browser
            .set_selection(pane.depth, &[source], Some(source));
        target_grid.scroll_to(target_position, gtk::ListScrollFlags::FOCUS, None);
        target_grid.grab_focus();
        true
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
            // Recycled ordinary cells also contain a hidden rename field at position zero.
            let field = bound_items.borrow().iter().find_map(|bound| {
                let item = bound.item.upgrade()?;
                if item.position() != 0 {
                    return None;
                }
                let widget = bound.widget.upgrade()?;
                descendant_with_class(&widget, "inline-rename")?
                    .downcast::<gtk::Entry>()
                    .ok()
                    .filter(gtk::prelude::WidgetExt::is_visible)
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
        let Some(label) = descendant_with_class(&widget, "alternate-rename-label") else {
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
                focused.as_ref() == Some(pane.stack.upcast_ref())
                    || pane
                        .all_sections()
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

    pub fn show_filter_with_query(&self, query: Option<&str>) -> bool {
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
        super::browser::focus_filter_entry(entry, query);
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

    pub fn set_new_folder_state(&self, state: Weak<super::browser::ViewState>) {
        self.new_folder_state.replace(Some(state));
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
                    pane.truncated_hint.set_visible(false);
                    pane.spinner.set_visible(true);
                    pane.spinner.start();
                    pane.stack.set_visible_child_name("loading");
                }
            }
            BrowserEvent::LoadFinished { depth, truncated } => {
                for pane in self.panes_at(*depth) {
                    reconnect_pane_model(pane);
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

    fn visible_panes(&self) -> Vec<&Pane> {
        match self.mode {
            BrowserMode::Columns => Vec::new(),
            BrowserMode::Grid => self.grid_panes.iter().collect(),
            BrowserMode::Explorer => self.explorer_pane.iter().collect(),
        }
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
        if self.mode == BrowserMode::Explorer {
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

    pub fn group_boundary_target(&self, direction: gtk::DirectionType) -> Option<(usize, usize)> {
        let (depth, source) = self.focused_position()?;
        let pane = self
            .visible_panes()
            .into_iter()
            .find(|pane| pane.depth == depth)?;
        let sections = pane.item_sections();
        if sections.len() < 2 {
            return None;
        }
        let (index, position) = sections.iter().enumerate().find_map(|(index, section)| {
            view_position_for_source(&pane.model, Some(&section.view_model), source)
                .map(|position| (index, position))
        })?;
        let current = &sections[index];
        let last = current.view_model.n_items().checked_sub(1)?;
        let previous = match direction {
            gtk::DirectionType::Up if current.first_row_contains(position) => true,
            gtk::DirectionType::Left if position == 0 => true,
            gtk::DirectionType::Down
                if current
                    .item_bounds(position)
                    .zip(current.item_bounds(last))
                    .is_some_and(|(a, b)| (a.y() - b.y()).abs() <= 1.0) =>
            {
                false
            }
            gtk::DirectionType::Right if position == last => false,
            _ => return None,
        };
        let adjacent = if previous {
            index.checked_sub(1)?
        } else {
            index + 1
        };
        let target = sections.get(adjacent)?;
        let edge = if previous {
            target.view_model.n_items().checked_sub(1)?
        } else {
            0
        };
        let mut target_position = edge;
        if matches!(direction, gtk::DirectionType::Up | gtk::DirectionType::Down)
            && let (Some(origin), Some(row)) =
                (current.item_bounds(position), target.item_bounds(edge))
        {
            target_position = target
                .bound_items
                .borrow()
                .iter()
                .filter_map(|bound| {
                    let position = bound.item.upgrade()?.position();
                    let bounds = bound.widget.upgrade()?.compute_bounds(&target.view)?;
                    ((bounds.y() - row.y()).abs() <= 1.0)
                        .then_some((position, (bounds.x() - origin.x()).abs()))
                })
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map_or(edge, |(position, _)| position);
        }
        source_position_for_view(
            &pane.source_index,
            Some(&target.view_model),
            target_position,
        )
        .map(|position| (depth, position))
    }

    pub fn focus_visible_pane(&self, depth: usize) {
        if self.rename_is_active() || self.new_entry_is_active() {
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
                view.grab_focus();
                if let Some(position) = position {
                    scroll_collection_to(&view, position);
                }
            }
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
                multiple_selection: self.multiple_selection.clone(),
            },
            self.transfer_handler.clone(),
            self.cut_locations.clone(),
            GridOptions {
                state: self.context_state.borrow().clone(),
                new_folder_state: self.new_folder_state.borrow().clone(),
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
                multiple_selection: self.multiple_selection.clone(),
            },
            self.transfer_handler.clone(),
            self.cut_locations.clone(),
            ExplorerOptions {
                state: self.context_state.borrow().clone(),
                new_folder_state: self.new_folder_state.borrow().clone(),
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
    new_folder_state: Option<Weak<super::browser::ViewState>>,
    active_new_entry: Rc<RefCell<Option<ActiveModeNewEntry>>>,
    group_by_type: bool,
}

struct GridOptions {
    state: Option<Weak<super::browser::ViewState>>,
    new_folder_state: Option<Weak<super::browser::ViewState>>,
    thumbnail_size: Rc<Cell<i32>>,
    active_new_entry: Rc<RefCell<Option<ActiveModeNewEntry>>>,
    group_by_type: bool,
    density: BrowserDensity,
}

#[derive(Clone)]
struct ModeClickOptions {
    previews: Rc<Cell<bool>>,
    activation: Rc<Cell<ClickActivation>>,
    multiple_selection: Rc<Cell<bool>>,
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
    thumbnail_scale.set_increments(16.0, 1.0);
    thumbnail_scale.add_css_class("grid-thumbnail-scale");
    thumbnail_scale.set_draw_value(false);
    thumbnail_scale.set_value(f64::from(thumbnail_size));
    thumbnail_scale.set_size_request(220, -1);
    disable_scale_long_press_zoom(&thumbnail_scale);
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

fn close_thumbnail_popover_on_outside_scroll(popover: &gtk::Popover, scroll: &gtk::ScrolledWindow) {
    let wheel = gtk::EventControllerScroll::new(
        gtk::EventControllerScrollFlags::VERTICAL | gtk::EventControllerScrollFlags::HORIZONTAL,
    );
    wheel.set_propagation_phase(gtk::PropagationPhase::Capture);
    let popover_for_scroll = popover.clone();
    let scroll = scroll.clone();
    wheel.connect_scroll(move |controller, dx, dy| {
        if !popover_for_scroll.is_visible() || pointer_over_widget(&popover_for_scroll) {
            return glib::Propagation::Proceed;
        }
        let over_grid = pointer_over_widget(&scroll);
        popover_for_scroll.popdown();
        if over_grid {
            apply_scrolled_window_wheel(&scroll, controller, dx, dy);
        }
        glib::Propagation::Stop
    });
    popover.add_controller(wheel);
}

fn pointer_over_widget(widget: &impl IsA<gtk::Widget>) -> bool {
    let widget = widget.as_ref();
    let Some(native) = widget.native() else {
        return false;
    };
    let Some(surface) = native.surface() else {
        return false;
    };
    let Some(pointer) = widget
        .display()
        .default_seat()
        .and_then(|seat| seat.pointer())
    else {
        return false;
    };
    let Some((x, y, _)) = surface.device_position(&pointer) else {
        return false;
    };
    let (ox, oy) = native.surface_transform();
    let Some(bounds) = widget.compute_bounds(native.upcast_ref::<gtk::Widget>()) else {
        return false;
    };
    bounds.contains_point(&gtk::graphene::Point::new((x - ox) as f32, (y - oy) as f32))
}

fn apply_scrolled_window_wheel(
    scroll: &gtk::ScrolledWindow,
    controller: &gtk::EventControllerScroll,
    mut dx: f64,
    mut dy: f64,
) {
    if controller
        .current_event_state()
        .contains(gtk::gdk::ModifierType::SHIFT_MASK)
    {
        std::mem::swap(&mut dx, &mut dy);
    }
    let unit = controller.unit();
    if dx != 0.0 {
        apply_adjustment_scroll(&scroll.hadjustment(), dx, unit);
    }
    if dy != 0.0 {
        apply_adjustment_scroll(&scroll.vadjustment(), dy, unit);
    }
}

fn apply_adjustment_scroll(adjustment: &gtk::Adjustment, delta: f64, unit: gtk::gdk::ScrollUnit) {
    let max = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
    adjustment.set_value(
        (adjustment.value() + scroll_delta_for_unit(delta, adjustment.page_size(), unit))
            .clamp(adjustment.lower(), max),
    );
}

fn scroll_delta_for_unit(delta: f64, page_size: f64, unit: gtk::gdk::ScrollUnit) -> f64 {
    delta
        * match unit {
            gtk::gdk::ScrollUnit::Wheel => page_size.powf(2.0 / 3.0),
            gtk::gdk::ScrollUnit::Surface => 2.5,
            _ => 1.0,
        }
}

/// Shared wiring the grid pane needs when building its view and binding cards.
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
    scrolling: Rc<Cell<bool>>,
    density: Cell<BrowserDensity>,
    type_heading: RefCell<Option<gtk::Label>>,
    type_heading_refresh: RefCell<Option<Rc<dyn Fn()>>>,
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
    if let Some(state) = options.new_folder_state {
        controls
            .actions
            .prepend(&super::browser::pane_new_folder_button(state, depth));
    }
    let (shell, header, content, model, stack, status, spinner, truncated_hint) = pane_base(
        title,
        "grid-pane",
        &grid_loading_skeleton(options.thumbnail_size.get(), options.density),
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
        scrolling: Rc::new(Cell::new(false)),
        density: Cell::new(options.density),
        type_heading: RefCell::new(None),
        type_heading_refresh: RefCell::new(None),
    });
    let flattened_models = gio::ListStore::new::<gio::ListModel>();
    flattened_models.append(&new_entry_placeholder.clone().upcast::<gio::ListModel>());
    flattened_models.append(&filtered_model.clone().upcast::<gio::ListModel>());
    let flattened = gtk::FlattenListModel::new(Some(flattened_models));
    let view_model = if options.group_by_type {
        let sorted = gtk::SortListModel::new(Some(flattened), None::<gtk::CustomSorter>);
        let sorter = type_group_sorter();
        sorted.set_sorter(Some(&sorter));
        sorted.set_section_sorter(Some(&sorter));
        sorted.upcast::<gio::ListModel>()
    } else {
        flattened.upcast()
    };
    let pane_section = build_grid_view(&context, &view_model);
    pane_section.view.set_vexpand(true);
    sections.borrow_mut().push(pane_section.clone());
    let type_heading = options.group_by_type.then(grid_type_heading);

    let sections_for_size = Rc::downgrade(&sections);
    let thumbnail_size_for_change = options.thumbnail_size.clone();
    let value_for_change = controls.thumbnail_value.clone();
    let loading_stack = stack.downgrade();
    let loading_context = Rc::downgrade(&context);
    let heading_context_for_size = Rc::downgrade(&context);
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
                    &grid_loading_skeleton(size, context.density.get()),
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
                resize_grid_thumbnail_slots(section, size);
            }
            if let Some(context) = heading_context_for_size.upgrade() {
                queue_grid_type_heading_refresh(&context);
            }
        });

    let scroll = gtk::ScrolledWindow::builder()
        .child(&pane_section.view)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vexpand(true)
        .build();
    scroll.add_css_class("fixed-scrollbar");
    close_thumbnail_popover_on_outside_scroll(&controls.thumbnail_popover, &scroll);
    install_grid_scroll_settle(&scroll, &context);
    if let Some(heading) = type_heading.as_ref() {
        install_grid_type_heading(&context, heading, &pane_section, &scroll);
    }
    let targets: super::marquee::MarqueeTargets = Rc::new(RefCell::new(Vec::new()));
    let (collection, marquee) =
        collection_with_marquee(&pane_section.view, scroll, targets.clone(), "grid-card");
    if let Some(heading) = type_heading.as_ref() {
        content.append(heading);
    }
    content.append(&super::inline_search::wrap(
        &collection,
        &controls.filter_entry,
        context
            .browser
            .location_at(depth)
            .and_then(|location| location.native_path().map(std::path::Path::to_path_buf)),
        &context.browser,
    ));
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

fn build_grid_view(context: &Rc<GridContext>, model: &impl IsA<gio::ListModel>) -> PaneSection {
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
    let thumbnail_size_for_setup = context.thumbnail_size.clone();
    let scrolling_for_setup = context.scrolling.clone();
    let folder_location = context.browser.location_at(depth);
    factory.connect_setup(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let card = gtk::Box::new(gtk::Orientation::Vertical, 3);
        card.add_css_class("grid-card");
        if !scrolling_for_setup.get() {
            card.add_css_class("file-appear");
            let weak_card = card.downgrade();
            glib::idle_add_local_once(move || {
                if let Some(card) = weak_card.upgrade() {
                    card.remove_css_class("file-appear");
                }
            });
        }
        card.set_halign(gtk::Align::Fill);
        card.set_valign(gtk::Align::Fill);
        card.set_overflow(gtk::Overflow::Hidden);
        let thumbnail_size = grid_card_icon_slot(thumbnail_size_for_setup.get());
        ensure_grid_card_slot(&card, thumbnail_size);
        let icon = gtk::Image::new();
        super::thumbnail::ensure_image_slot(&icon, thumbnail_size);
        icon.add_css_class("grid-card-icon");
        let label = gtk::Inscription::new(None);
        label.add_css_class("grid-card-label");
        label.add_css_class("alternate-rename-label");
        configure_grid_card_label(&label);
        let field = gtk::Entry::new();
        field.add_css_class("inline-rename");
        field.set_width_chars(GRID_CARD_LABEL_CHARS);
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
        card.append(&icon);
        card.append(&label);
        card.append(&field);
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
    let scrolling_for_bind = context.scrolling.clone();
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
        let Some((icon, label, field)) = grid_card_parts(&card) else {
            return;
        };
        let source_position = item
            .item()
            .and_then(|value| source_index_for_bind.of_item(&value));
        let browser = browser_for_bind.upgrade();
        let entry = browser.as_ref().and_then(|browser| {
            source_position.and_then(|position| browser.entry_at(depth, position))
        });
        let thumbnail_size = grid_card_icon_slot(thumbnail_size_for_bind.get());
        ensure_grid_card_slot(&card, thumbnail_size);
        if let Some(entry) = entry {
            label.set_visible(true);
            field.set_visible(false);
            if label.text().as_deref() != Some(entry.display_name.as_str()) {
                label.set_text(Some(&entry.display_name));
            }
            if !scrolling_for_bind.get() {
                set_mode_cut_style(&card, cuts_for_bind.borrow().contains(&entry.location));
                label.set_tooltip_text(Some(&entry.display_name));
                super::thumbnail::set_thumbnail_or_icon(
                    &icon,
                    &entry,
                    super::browser::entry_icon(&entry),
                    thumbnail_size,
                    thumbnail_size,
                );
                if let Some(position) = metadata_fill_position(source_position, &entry, false)
                    && let Some(browser) = browser.as_ref()
                {
                    browser.request_metadata_fill(depth, position, entry.location.clone());
                }
                icon.set_opacity(if entry.is_directory() { 1.0 } else { 0.72 });
            }
        } else {
            set_mode_cut_style(&card, false);
            let icon_name = if entry_kind_for_bind.get() {
                crate::assets::icons::FOLDER
            } else {
                crate::assets::icons::DOCUMENTS
            };
            super::thumbnail::ensure_image_slot(&icon, thumbnail_size);
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
    connect_selection(
        &section,
        context.sections.clone(),
        &context.browser,
        depth,
        context.source_index.clone(),
        context.click.multiple_selection.clone(),
    );
    install_exclusive_section_click(&section, context);
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

fn grid_type_heading() -> gtk::Label {
    let heading = type_group_heading("");
    heading.add_css_class("grid-type-heading");
    heading.set_hexpand(true);
    heading.set_visible(false);
    heading
}

/// Pins a type-group label above the single scrollable GridView. GridView has no
/// header factory, so the label tracks the first bound card instead of sitting
/// inside the view. Updates run on idle after the model changes, and again when
/// scroll settles, so a fling does not walk bound cards on every wheel tick.
fn install_grid_type_heading(
    context: &Rc<GridContext>,
    heading: &gtk::Label,
    section: &PaneSection,
    scroll: &gtk::ScrolledWindow,
) {
    context.type_heading.replace(Some(heading.clone()));
    let pending = Rc::new(Cell::new(false));
    let weak_context = Rc::downgrade(context);
    let queue = Rc::new(move || {
        if pending.replace(true) {
            return;
        }
        let pending = pending.clone();
        let weak_context = weak_context.clone();
        glib::idle_add_local_once(move || {
            pending.set(false);
            let Some(context) = weak_context.upgrade() else {
                return;
            };
            if context.scrolling.get() {
                return;
            }
            refresh_grid_type_heading(&context);
        });
    });
    let queue_for_items = queue.clone();
    section.view_model.connect_items_changed(move |_, _, _, _| {
        queue_for_items();
    });
    for adjustment in [scroll.vadjustment(), scroll.hadjustment()] {
        let queue_for_layout = queue.clone();
        adjustment.connect_changed(move |_| {
            queue_for_layout();
        });
    }
    context.type_heading_refresh.replace(Some(queue.clone()));
    queue();
}

fn queue_grid_type_heading_refresh(context: &GridContext) {
    if let Some(queue) = context.type_heading_refresh.borrow().as_ref() {
        queue();
    }
}

fn refresh_grid_type_heading(context: &GridContext) {
    let Some(heading) = context.type_heading.borrow().clone() else {
        return;
    };
    let Some(sections) = context.sections.upgrade() else {
        return;
    };
    let Some(section) = sections.borrow().first().cloned() else {
        return;
    };
    let group = first_visible_type_group(&section);
    heading.set_visible(!group.is_empty());
    if heading.label() != group {
        heading.set_label(&group);
    }
}

fn first_visible_type_group(section: &PaneSection) -> String {
    let viewport_height = section.view.height() as f32;
    type_group_for_first_intersecting(
        section.bound_items.borrow().iter().filter_map(|bound| {
            let item = bound.item.upgrade()?;
            let widget = bound.widget.upgrade()?;
            if !widget.is_mapped() {
                return None;
            }
            let bounds = widget.compute_bounds(&section.view)?;
            Some((
                item.position(),
                model_value(&item.item()?),
                bounds.y(),
                bounds.height(),
            ))
        }),
        viewport_height,
    )
    .unwrap_or_else(|| first_model_type_group(&section.view_model))
}

fn type_group_for_first_intersecting(
    items: impl Iterator<Item = (u32, String, f32, f32)>,
    viewport_height: f32,
) -> Option<String> {
    items
        .filter(|(_, value, y, height)| {
            !value.is_empty() && *height > 0.0 && *y < viewport_height && *y + *height > 0.0
        })
        .min_by(|left, right| {
            left.2
                .total_cmp(&right.2)
                .then_with(|| left.0.cmp(&right.0))
        })
        .map(|(_, value, _, _)| value_type_group(&value))
}

fn first_model_type_group(model: &gio::ListModel) -> String {
    (0..model.n_items())
        .find_map(|index| {
            let group = value_type_group(&model_value(&model.item(index)?));
            (!group.is_empty()).then_some(group)
        })
        .unwrap_or_default()
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

fn install_grid_scroll_settle(scroll: &gtk::ScrolledWindow, context: &Rc<GridContext>) {
    let pending = Rc::new(RefCell::new(None::<glib::SourceId>));
    for adjustment in [scroll.vadjustment(), scroll.hadjustment()] {
        let pending = pending.clone();
        let context = Rc::downgrade(context);
        let scroll = scroll.clone();
        adjustment.connect_value_changed(move |_| {
            let Some(context) = context.upgrade() else {
                return;
            };
            context.scrolling.set(true);
            scroll.add_css_class("grid-fast-scroll");
            if let Some(source) = pending.borrow_mut().take() {
                source.remove();
            }
            let pending_for_timeout = pending.clone();
            let context = Rc::downgrade(&context);
            let scroll = scroll.clone();
            pending.replace(Some(glib::timeout_add_local_once(
                GRID_SCROLL_SETTLE_DELAY,
                move || {
                    pending_for_timeout.borrow_mut().take();
                    let Some(context) = context.upgrade() else {
                        return;
                    };
                    context.scrolling.set(false);
                    scroll.remove_css_class("grid-fast-scroll");
                    refresh_grid_expensive_content(&context);
                },
            )));
        });
    }
}

fn refresh_grid_expensive_content(context: &GridContext) {
    refresh_grid_type_heading(context);
    let Some(sections) = context.sections.upgrade() else {
        return;
    };
    let cuts = context.cuts.borrow();
    for section in sections.borrow().iter() {
        refresh_grid_section(
            &Rc::downgrade(&context.browser),
            context.depth,
            &context.source_index,
            section,
            context.thumbnail_size.get(),
            true,
            Some(&cuts),
        );
    }
}

fn resize_grid_thumbnail_slots(section: &PaneSection, size: i32) {
    let size = grid_card_icon_slot(size);
    section.bound_items.borrow().iter().for_each(|bound| {
        let Some(card) = bound.widget.upgrade().and_downcast::<gtk::Box>() else {
            return;
        };
        let Some((icon, _, _)) = grid_card_parts(&card) else {
            return;
        };
        super::thumbnail::ensure_image_slot(&icon, size);
        ensure_grid_card_slot(&card, size);
    });
}

fn refresh_grid_section(
    browser: &Weak<Browser>,
    depth: usize,
    source_index: &SourceIndexMap,
    section: &PaneSection,
    size: i32,
    request_metadata: bool,
    cuts: Option<&HashSet<Location>>,
) {
    let Some(browser) = browser.upgrade() else {
        return;
    };
    let size = grid_card_icon_slot(size);
    section.bound_items.borrow().iter().for_each(|bound| {
        let Some(card) = bound.widget.upgrade().and_downcast::<gtk::Box>() else {
            return;
        };
        let Some((icon, label, _)) = grid_card_parts(&card) else {
            return;
        };
        super::thumbnail::ensure_image_slot(&icon, size);
        ensure_grid_card_slot(&card, size);
        let Some(item) = bound.item.upgrade() else {
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
            size,
            size,
        );
        if request_metadata {
            label.set_tooltip_text(Some(&entry.display_name));
            if let Some(cuts) = cuts {
                set_mode_cut_style(&card, cuts.contains(&entry.location));
            }
            if let Some(position) = metadata_fill_position(Some(position), &entry, false) {
                browser.request_metadata_fill(depth, position, entry.location.clone());
            }
        }
        icon.set_opacity(if entry.is_directory() { 1.0 } else { 0.72 });
    });
}

fn grid_card_icon_slot(thumbnail_size: i32) -> i32 {
    thumbnail_size.clamp(MIN_GRID_THUMBNAIL_SIZE, MAX_GRID_THUMBNAIL_SIZE)
}

fn grid_card_extent(thumbnail_size: i32) -> (i32, i32) {
    let slot = grid_card_icon_slot(thumbnail_size);
    let width = slot.max(FALLBACK_GRID_COLUMN_WIDTH - GRID_CARD_SPACING);
    let height = slot + GRID_CARD_LABEL_LINE_PX * GRID_CARD_LABEL_LINES + GRID_CARD_PAD_Y + 3;
    (width, height)
}

fn ensure_grid_card_slot(card: &gtk::Box, thumbnail_size: i32) {
    let (width, height) = grid_card_extent(thumbnail_size);
    if card.width_request() != width || card.height_request() != height {
        card.set_size_request(width, height);
    }
}

fn configure_grid_card_label(label: &gtk::Inscription) {
    let chars = GRID_CARD_LABEL_CHARS as u32;
    let lines = GRID_CARD_LABEL_LINES as u32;
    label.set_min_chars(chars);
    label.set_nat_chars(chars);
    label.set_min_lines(lines);
    label.set_nat_lines(lines);
    label.set_xalign(0.5);
    label.set_yalign(0.0);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_text_overflow(gtk::InscriptionOverflow::EllipsizeEnd);
}

fn grid_card_parts(card: &gtk::Box) -> Option<(gtk::Image, gtk::Inscription, gtk::Entry)> {
    let icon = card.first_child()?.downcast::<gtk::Image>().ok()?;
    let label = icon.next_sibling()?.downcast::<gtk::Inscription>().ok()?;
    let field = label.next_sibling()?.downcast::<gtk::Entry>().ok()?;
    Some((icon, label, field))
}

fn configure_grid_density(pane: &Pane, density: BrowserDensity) {
    if let Some(loading) = pane.stack.child_by_name("loading")
        && let Some(scroll) = loading.first_child().and_downcast::<gtk::ScrolledWindow>()
        && let Some(grid) = scroll.child().and_downcast::<gtk::GridView>()
    {
        configure_grid_view_density(&grid, density);
    }
    if let Some(context) = pane.grid.as_ref() {
        context.density.set(density);
    }
    for section in pane.all_sections() {
        if let Ok(grid) = section.view.clone().downcast::<gtk::GridView>() {
            configure_grid_view_density(&grid, density);
        }
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
    if let Some(state) = options.new_folder_state.as_ref() {
        actions.append(&super::browser::pane_new_folder_button(
            state.clone(),
            depth,
        ));
    }
    actions.append(&super::browser::pane_refresh_button(&browser, depth));
    let (filter_entry, filter_revealer, filter_button) =
        filter_controls("Filter explorer (Ctrl+F)");
    actions.append(&filter_button);
    let columns = ExplorerColumnLayout::new();
    let (shell, header, content, model, stack, status, spinner, truncated_hint) = pane_base(
        title,
        "explorer-pane",
        &explorer_loading_skeleton(&columns),
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
    content.append(&super::inline_search::wrap(
        &table_scroll,
        &filter_entry,
        browser
            .location_at(depth)
            .and_then(|location| location.native_path().map(std::path::Path::to_path_buf)),
        &browser,
    ));
    let pane = Pane {
        depth,
        shell,
        header,
        model,
        source_index,
        filter_model: Some(filtered_model),
        section,
        sections,
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

fn grid_loading_skeleton(thumbnail_size: i32, density: BrowserDensity) -> gtk::Box {
    use super::loading_skeleton::{block, container, name_width, scroll};

    let skeleton = container();
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let card = gtk::Box::new(gtk::Orientation::Vertical, 3);
        card.add_css_class("grid-card");
        card.set_halign(gtk::Align::Fill);
        ensure_grid_card_slot(&card, thumbnail_size);
        let slot = grid_card_icon_slot(thumbnail_size);
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
    let grid = gtk::GridView::new(Some(gtk::NoSelection::new(Some(model))), Some(factory));
    grid.add_css_class("file-grid");
    grid.set_valign(gtk::Align::Start);
    configure_grid_view_density(&grid, density);
    let scroll = scroll(&grid);
    scroll.set_vexpand(true);
    skeleton.append(&scroll);
    skeleton
}

fn explorer_loading_skeleton(columns: &ExplorerColumnLayout) -> gtk::Box {
    use super::loading_skeleton::{ROW_COUNT, block, container, name_width, scroll};

    let skeleton = container();
    let table = gtk::Box::new(gtk::Orientation::Vertical, 0);
    table.set_valign(gtk::Align::Start);
    let headings = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    headings.add_css_class("explorer-headings");
    for (index, width) in [40, 36, 28, 30, 58].into_iter().enumerate() {
        let cell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        cell.add_css_class("explorer-heading-cell");
        let bar = block(width, 8);
        bar.set_margin_start(12);
        cell.append(&bar);
        register_explorer_column_cell(columns, index, &cell);
        headings.append(&cell);
    }
    table.append(&headings);
    for index in 0..ROW_COUNT {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        row.add_css_class("explorer-row");
        for (column, width) in [name_width(index).min(92), 60, 38, 56, 94]
            .into_iter()
            .enumerate()
        {
            let cell = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            cell.add_css_class("explorer-metadata-cell");
            if column == 0 {
                cell.append(&block(18, 18));
            }
            cell.append(&block(width, 10));
            register_explorer_column_cell(columns, column, &cell);
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
    super::scrolling::install_autoscroll(&scroll, &overlay);

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
    if transfer_handler.borrow().is_none() {
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

fn install_explorer_drag_drop(
    row: &gtk::Box,
    item: &gtk::ListItem,
    browser: Weak<Browser>,
    transfer_handler: TransferHandlerSlot,
    depth: usize,
    position_map: Option<(SourceIndexMap, gio::ListModel)>,
) {
    if transfer_handler.borrow().is_none() {
        return;
    }
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
    multiple_selection: Rc<Cell<bool>>,
) {
    let syncing = section.syncing.clone();
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
    super::browser::scroll_collection_when_allocated(view, position);
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
