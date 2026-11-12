// SPDX-License-Identifier: MIT

//! Alternate browser presentations.
//!
//! This module is deliberately isolated from the Miller-column implementation. It consumes the
//! same application events and emits the same navigation/selection intents, so adding another
//! presentation does not require scattering mode checks throughout the main browser view.

use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    rc::{Rc, Weak},
};

use gtk::{glib, prelude::*};

pub(super) use super::icons_cell::{
    ICONS_CARD_SPACING, MAX_ICONS_THUMBNAIL_SIZE, MIN_ICONS_THUMBNAIL_SIZE, icons_card_extent,
    icons_card_icon_slot,
};
use crate::{
    app::{Browser, BrowserColumnSnapshot},
    model::Location,
    ui::{
        browser::{ClipboardMarks, ViewState},
        collection_edit::ActiveEdit,
        inline_search::{SearchSelectionChanged, SearchSelectionHandlers},
        marquee::Marquee,
    },
};

mod clicks;
mod column_options;
mod context_menu;
mod cursor;
mod density;
mod drag_drop;
mod events;
mod filter;
mod filter_session;
mod first_visit;
mod focus;
mod grouped_model;
mod icons_card;
mod icons_controls;
mod icons_pane;
mod icons_view;
mod list_columns;
mod list_factory;
mod list_headings;
mod list_pane;
mod metadata;
mod navigation;
mod pane;
mod pane_header;
mod pane_shell;
mod pane_state;
mod rebuild;
mod rename;
mod scroll_settle;
mod search_options;
mod selection;
mod source_index;
#[cfg(test)]
mod tests;
mod thumbnail_size;
mod type_groups;

use clicks::{SlowClickRename, install_modified_selection_click, install_preview_click};
pub(super) use density::DensityScale;
pub(crate) use density::browser_density_custom_css;
pub use density::{BrowserDensity, FrameDensity};
use drag_drop::{install_folder_peek, install_list_drag_drop};
pub(crate) use filter::filter_controls;
use focus::{collection_keeps_cursor, focus_bound_cursor, focus_collection_item};
pub(super) use icons_controls::disable_scale_long_press_zoom;
use icons_pane::{DEFAULT_ICONS_THUMBNAIL_SIZE, configure_icons_density};
pub(in crate::ui) use icons_pane::{configure_compact_view, wheel_scrolls_sideways};
use list_columns::{ListColumnLayout, apply_columns_layout, register_list_column_cell};
use list_pane::{assemble_list_row, list_row_parts};
use metadata::{entry_mode, entry_type, metadata_fill_position, set_label_if_changed};
pub(super) use metadata::{entry_size, icon_details_need_fill};
use pane::{
    BoundModeItem, Pane, PaneSection, bound_edit, focus_pane_surface, install_edit_unbind,
    refresh_mark_pane, register_bound_mode_item, set_mode_mark_style,
};
pub(super) use search_options::{FilteredFileClick, filtered_file_click};
use selection::{reset_native_range_origin, scroll_collection_to, set_selections};
use source_index::{PanePositions, SourceIndexMap, view_position_for_source};
pub(super) use thumbnail_size::ThumbnailResize;
#[cfg(test)]
use {
    crate::model::{FileEntry, MetadataValue, SortDirection, SortKey},
    clicks::move_range_origin,
    icons_pane::density_icons_columns,
    scroll_settle::{ScrollSettle, install_scroll_refresh},
    selection::bitset_positions,
    type_groups::TypeGroupSorter,
};

type TransferHandler = super::browser::DropHandler;
type TransferHandlerSlot = Rc<RefCell<Option<TransferHandler>>>;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BrowserMode {
    #[default]
    Columns,
    Icons,
    List,
    /// Small icons with names beside them, flowing down columns that scroll sideways.
    Compact,
}

impl BrowserMode {
    /// File-type headings are List-only. Icons clustering is wired but gated off.
    pub fn supports_type_grouping(self) -> bool {
        matches!(self, Self::List)
    }

    /// Items have neighbours on both axes, so arrows move spatially in a grid.
    pub fn is_grid(self) -> bool {
        matches!(self, Self::Icons | Self::Compact)
    }
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
                BrowserMode::Icons | BrowserMode::List | BrowserMode::Compact => ClickCount::Two,
            },
        }
    }
}

impl Default for ClickActivation {
    fn default() -> Self {
        Self::default_for(BrowserMode::Columns)
    }
}

pub struct ModeViews {
    stack: gtk::Stack,
    icons_root: gtk::Box,
    list_root: gtk::Box,
    compact_root: gtk::Box,
    icons_panes: Vec<Pane>,
    list_pane: Option<Pane>,
    compact_pane: Option<Pane>,
    pane_navigation: RefCell<navigation::PaneNavigation>,
    browser: Rc<Browser>,
    single_click_previews: Rc<Cell<bool>>,
    multiple_selection: Rc<Cell<bool>>,
    icons_click_activation: Rc<Cell<ClickActivation>>,
    list_click_activation: Rc<Cell<ClickActivation>>,
    transfer_handler: TransferHandlerSlot,
    clipboard_marks: Rc<RefCell<ClipboardMarks>>,
    context_state: Rc<RefCell<Option<Weak<ViewState>>>>,
    new_folder_state: RefCell<Option<Weak<ViewState>>>,
    active_rename: Rc<RefCell<Option<ActiveEdit>>>,
    search_selection_handlers: SearchSelectionHandlers,
    mode: BrowserMode,
    density: BrowserDensity,
    group_by_type: bool,
    icons_thumbnail_size: Rc<Cell<i32>>,
    /// The size preferences gave the shown folder; a size changed on a folder
    /// that is not remembered lasts until this changes.
    icons_resolved_size: Cell<i32>,
    focus_before_header: RefCell<Option<glib::WeakRef<gtk::Widget>>>,
    /// Page Up/Down scrolls the viewport itself; skip the follow-up `scroll_to`
    /// that `FocusChanged` would otherwise schedule from stale GridView estimates.
    suppress_focus_scroll: Cell<bool>,
    /// Set while a footer prompt moves the cursor; the prompt keeps the keys.
    cursor_keeps_focus: Cell<bool>,
    cursor_focus_generation: Rc<Cell<u64>>,
    /// A pane whose results, rather than its hidden listing, take the focus a
    /// `FocusChanged` asked for. The view state focuses them on their cursor.
    results_take_focus: Cell<Option<usize>>,
}

/// The box an Icons pane goes in, inside its sideways scroller.
fn icons_root() -> (gtk::Box, gtk::ScrolledWindow) {
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
    icons_scroll.add_css_class("mode-scroll");
    (icons_root, icons_scroll)
}

/// The box the List pane goes in.
fn list_root() -> (gtk::Box, gtk::ScrolledWindow) {
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
    (list_root, list_scroll)
}

/// The box the Compact pane goes in; its grid scrolls sideways on its own.
fn compact_root() -> (gtk::Box, gtk::ScrolledWindow) {
    let compact_root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    compact_root.add_css_class("mode-icons");
    compact_root.add_css_class("mode-compact");
    compact_root.set_hexpand(true);
    compact_root.set_vexpand(true);
    let compact_scroll = gtk::ScrolledWindow::builder()
        .child(&compact_root)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .hexpand(true)
        .vexpand(true)
        .build();
    compact_scroll.add_css_class("mode-scroll");
    (compact_root, compact_scroll)
}

struct ModeScrolls<'a> {
    columns: &'a gtk::ScrolledWindow,
    icons: &'a gtk::ScrolledWindow,
    list: &'a gtk::ScrolledWindow,
    compact: &'a gtk::ScrolledWindow,
}

fn mode_stack(scrolls: ModeScrolls) -> gtk::Stack {
    let stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::None)
        .hexpand(true)
        .vexpand(true)
        .build();
    for mode in [
        BrowserMode::Columns,
        BrowserMode::Icons,
        BrowserMode::List,
        BrowserMode::Compact,
    ] {
        let page = match mode {
            BrowserMode::Columns => scrolls.columns,
            BrowserMode::Icons => scrolls.icons,
            BrowserMode::List => scrolls.list,
            BrowserMode::Compact => scrolls.compact,
        };
        stack.add_named(page, Some(mode.page_name()));
    }
    stack.set_visible_child_name(BrowserMode::Columns.page_name());
    stack
}

impl BrowserMode {
    /// The `gtk::Stack` page that shows this mode.
    pub(super) fn page_name(self) -> &'static str {
        match self {
            Self::Columns => "columns",
            Self::Icons => "icons",
            Self::List => "list",
            Self::Compact => "compact",
        }
    }

    pub(super) fn from_page_name(name: &str) -> Self {
        [Self::Icons, Self::List, Self::Compact]
            .into_iter()
            .find(|mode| mode.page_name() == name)
            .unwrap_or(Self::Columns)
    }
}

impl ModeViews {
    pub fn new(
        columns: &gtk::ScrolledWindow,
        browser: Rc<Browser>,
        multiple_selection: Rc<Cell<bool>>,
    ) -> Self {
        let (icons_root, icons_scroll) = icons_root();
        let (list_root, list_scroll) = list_root();
        let (compact_root, compact_scroll) = compact_root();
        Self {
            stack: mode_stack(ModeScrolls {
                columns,
                icons: &icons_scroll,
                list: &list_scroll,
                compact: &compact_scroll,
            }),
            icons_root,
            list_root,
            compact_root,
            icons_panes: Vec::new(),
            list_pane: None,
            compact_pane: None,
            pane_navigation: RefCell::new(navigation::PaneNavigation::default()),
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
            clipboard_marks: Rc::new(RefCell::new(ClipboardMarks::new())),
            context_state: Rc::new(RefCell::new(None)),
            new_folder_state: RefCell::new(None),
            active_rename: Rc::new(RefCell::new(None)),
            search_selection_handlers: Rc::new(RefCell::new(Vec::new())),
            mode: BrowserMode::Columns,
            density: BrowserDensity::Compact,
            group_by_type: false,
            icons_thumbnail_size: Rc::new(Cell::new(DEFAULT_ICONS_THUMBNAIL_SIZE)),
            icons_resolved_size: Cell::new(DEFAULT_ICONS_THUMBNAIL_SIZE),
            focus_before_header: RefCell::new(None),
            suppress_focus_scroll: Cell::new(false),
            cursor_keeps_focus: Cell::new(false),
            cursor_focus_generation: Rc::default(),
            results_take_focus: Cell::new(None),
        }
    }

    pub fn widget(&self) -> gtk::Stack {
        self.stack.clone()
    }

    pub fn set_show_hidden(&self, show_hidden: bool) {
        for pane in self.all_panes() {
            pane.show_hidden.set(show_hidden);
            pane.filter.changed(gtk::FilterChange::Different);
            pane.search.set_show_hidden(show_hidden);
        }
    }

    pub fn mode(&self) -> BrowserMode {
        self.mode
    }

    /// The marquee of the pane nearest the window's start edge, so chrome outside the
    /// panes can run a drag into whichever view the current mode shows.
    pub(super) fn leading_marquee(&self) -> Option<Marquee> {
        let pane = self.single_pane()?;
        pane.search
            .active_marquee()
            .or_else(|| Some(pane.marquee.clone()))
    }

    fn single_pane(&self) -> Option<&Pane> {
        self.mode_panes(self.mode).into_iter().next()
    }

    fn mode_panes(&self, mode: BrowserMode) -> Vec<&Pane> {
        match mode {
            BrowserMode::Columns => Vec::new(),
            BrowserMode::Icons => self.icons_panes.iter().collect(),
            BrowserMode::List => self.list_pane.iter().collect(),
            BrowserMode::Compact => self.compact_pane.iter().collect(),
        }
    }

    pub fn refresh_source_filter(&self) {
        for pane in self.visible_panes() {
            pane.search.refresh_source_filter(&self.browser);
        }
    }

    pub fn prune_stale_search_results(&self) {
        if let Some(pane) = self.single_pane() {
            pane.search.prune_missing(&self.browser);
        }
    }

    pub fn set_single_click_previews(&self, enabled: bool) {
        self.single_click_previews.set(enabled);
    }

    pub fn set_click_activation(&self, mode: BrowserMode, activation: ClickActivation) {
        match mode {
            BrowserMode::Columns => {}
            // Compact shares the Icons click settings.
            BrowserMode::Icons | BrowserMode::Compact => {
                self.icons_click_activation.set(activation)
            }
            BrowserMode::List => self.list_click_activation.set(activation),
        }
    }

    pub fn set_transfer_handler(&self, handler: TransferHandler) {
        self.transfer_handler.replace(Some(handler));
    }

    pub fn set_new_folder_state(&self, state: Weak<ViewState>) {
        self.new_folder_state.replace(Some(state));
    }

    pub fn set_context_state(&self, state: Weak<ViewState>) {
        self.context_state.replace(Some(state));
    }

    pub fn connect_search_selection_changed(&self, handler: SearchSelectionChanged) {
        self.search_selection_handlers.borrow_mut().push(handler);
    }

    pub fn set_clipboard_marks(&self, marks: &ClipboardMarks) {
        self.clipboard_marks.replace(marks.clone());
        for pane in self.all_panes() {
            refresh_mark_pane(pane, &self.browser, marks);
        }
    }

    pub fn set_density(&mut self, density: BrowserDensity) {
        self.density = density;
        for pane in self.icons_panes.iter().chain(self.compact_pane.iter()) {
            configure_icons_density(pane, density);
        }
        let class = match density {
            BrowserDensity::Compact => "density-compact",
            BrowserDensity::Airy => "density-airy",
            BrowserDensity::Custom(_) => "density-custom",
        };
        for root in [&self.icons_root, &self.list_root, &self.compact_root] {
            root.remove_css_class("density-compact");
            root.remove_css_class("density-airy");
            root.remove_css_class("density-custom");
            root.add_css_class(class);
        }
    }

    pub fn suppress_focus_scroll(&self) {
        self.suppress_focus_scroll.set(true);
    }

    pub(in crate::ui) fn set_cursor_keeps_focus(&self, keep: bool) {
        self.cursor_keeps_focus.set(keep);
    }

    pub(in crate::ui) fn take_results_focus(&self) -> Option<usize> {
        self.results_take_focus.take()
    }

    fn visible_panes(&self) -> Vec<&Pane> {
        self.mode_panes(self.mode)
    }

    fn all_panes(&self) -> Vec<&Pane> {
        self.icons_panes
            .iter()
            .chain(self.list_pane.as_ref())
            .chain(self.compact_pane.as_ref())
            .collect()
    }

    pub(crate) fn show_leftovers(&self, depth: usize, shown: bool) {
        for pane in self.panes_at(depth) {
            pane.leftovers_button.set_visible(shown);
        }
    }

    /// The Icons or List pane showing `depth`.
    fn mode_pane(&self, depth: usize) -> Option<&Pane> {
        self.panes_at(depth).into_iter().next()
    }

    fn panes_at(&self, depth: usize) -> Vec<&Pane> {
        self.visible_panes()
            .into_iter()
            .filter(|pane| pane.depth == depth)
            .take(1)
            .collect()
    }
}
