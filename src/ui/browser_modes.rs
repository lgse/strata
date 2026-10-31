// SPDX-License-Identifier: MIT

//! Alternate browser presentations.
//!
//! This module is deliberately isolated from the Miller-column implementation. It consumes the
//! same application events and emits the same navigation/selection intents, so adding another
//! presentation does not require scattering mode checks throughout the main browser view.

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    path::Path,
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
    services::DropCommit,
    ui::browser::{ClipboardMark, ClipboardMarks, FilterFocus, mark_in, paths::is_trash_location},
};

use crate::ui::preferences::ListColumnId;

mod column_options;
mod events;
mod list_factory;
mod navigation;
#[cfg(test)]
mod tests;
mod thumbnail_size;

use list_factory::{ListFactory, ListRow, refresh_list_section};
pub(super) use thumbnail_size::ThumbnailResize;

const LIST_COLUMN_WIDTHS: [i32; 5] = [160, 160, 90, 120, 150];
const LIST_COLUMN_MIN_WIDTHS: [i32; 5] = [160, 80, 70, 80, 110];
const DEFAULT_ICONS_THUMBNAIL_SIZE: i32 = 64;
const FALLBACK_ICONS_COLUMN_WIDTH: i32 = 120;

#[derive(Clone)]
struct ListColumnLayout {
    widths: Rc<Vec<Cell<i32>>>,
    cells: Rc<Vec<RefCell<Vec<glib::WeakRef<gtk::Widget>>>>>,
    name_manually_resized: Rc<Cell<bool>>,
    scale: Rc<Cell<f64>>,
    drag: Rc<Cell<Option<ListColumnDrag>>>,
    chooser: bool,
}

impl ListColumnLayout {
    fn new(browser: &Browser) -> Self {
        let chooser = browser.is_chooser_mode();
        let preferences = super::preferences::PreferenceManager::shared();
        let saved = if chooser {
            preferences.chooser_list_columns()
        } else {
            preferences.browser_list_columns()
        };
        let widths = saved.map_or(LIST_COLUMN_WIDTHS, |saved| {
            let mut widths = [
                saved.name.unwrap_or(LIST_COLUMN_WIDTHS[0]),
                saved.mode,
                saved.size,
                saved.kind,
                saved.modified,
            ];
            for (index, width) in widths.iter_mut().enumerate() {
                *width = list_column_width(index, *width);
            }
            widths
        });
        Self {
            widths: Rc::new(widths.into_iter().map(Cell::new).collect()),
            cells: Rc::new((0..5).map(|_| RefCell::new(Vec::new())).collect()),
            name_manually_resized: Rc::new(Cell::new(
                saved.is_some_and(|saved| saved.name.is_some()),
            )),
            scale: Rc::new(Cell::new(1.0)),
            drag: Rc::default(),
            chooser,
        }
    }

    fn finish_drag(
        &self,
        headings: &gtk::Box,
        bound_items: &Rc<RefCell<Vec<BoundModeItem>>>,
        released: Option<(f64, f64)>,
    ) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        headings.set_cursor(None);
        headings.remove_css_class("column-reordering");
        column_drop_feedback(headings, None);
        if drag.active
            && let Some(offset) = released
            && let Some((target, after)) = column_drop_site(headings, self, drag, offset)
        {
            super::preferences::PreferenceManager::shared().move_list_column(
                drag.column,
                target,
                after,
            );
            apply_columns_layout(headings, self, bound_items);
        }
    }

    fn remember(&self) {
        let scale = self.scale.get();
        let unscaled = |index: usize| (f64::from(self.widths[index].get()) / scale).round() as i32;
        let saved = Some(super::preferences::ListColumns {
            name: self.name_manually_resized.get().then(|| unscaled(0)),
            mode: unscaled(1),
            size: unscaled(2),
            kind: unscaled(3),
            modified: unscaled(4),
        });
        let preferences = super::preferences::PreferenceManager::shared();
        if self.chooser {
            preferences.set_chooser_list_columns(saved);
        } else {
            preferences.set_browser_list_columns(saved);
        }
    }
}

type TransferHandler = Rc<dyn Fn(Location, Vec<Location>, DropCommit)>;
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

/// One position on a density slider: a px value, where `COMPACT_PX` and `AIRY_PX`
/// are the snap points and every other value in `0..=MAX_PX` is Custom.
pub(super) trait DensityScale: Copy + PartialEq + 'static {
    const COMPACT_PX: u8;
    const AIRY_PX: u8;
    const MAX_PX: u8;
    fn from_px(px: u8) -> Self;
    fn px(self) -> u8;
}

macro_rules! density_scale {
    ($(#[$meta:meta])* $name:ident { compact: $compact:literal, airy: $airy:literal, max: $max:literal }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
        pub enum $name {
            #[default]
            Compact,
            Airy,
            /// `from_px` maps snap-point values to their preset rather than to Custom.
            Custom(u8),
        }

        impl DensityScale for $name {
            const COMPACT_PX: u8 = $compact;
            const AIRY_PX: u8 = $airy;
            const MAX_PX: u8 = $max;

            fn from_px(px: u8) -> Self {
                match px.min(Self::MAX_PX) {
                    $compact => Self::Compact,
                    $airy => Self::Airy,
                    px => Self::Custom(px),
                }
            }

            fn px(self) -> u8 {
                match self {
                    Self::Compact => Self::COMPACT_PX,
                    Self::Airy => Self::AIRY_PX,
                    Self::Custom(px) => px.min(Self::MAX_PX),
                }
            }
        }
    };
}

density_scale!(
    /// File-row and icon-card vertical spacing in logical px.
    BrowserDensity { compact: 4, airy: 8, max: 16 }
);

density_scale!(
    /// Extra vertical inset shared by the header, tab strip, sidebar rows, and
    /// Miller/preview headers, so their heights stay in lock step.
    FrameDensity { compact: 0, airy: 4, max: 12 }
);

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

// Capture must snapshot selection before GTK changes it for the bubble gesture.
#[derive(Default)]
struct SlowClickRename {
    was_selected: Cell<bool>,
    selected_count_before: Cell<u64>,
}

/// A view's source positions by view position, built for the two maps' generations.
type ViewSources = ((u64, u64), Vec<u32>);

/// Filter, sort, and flatten models preserve item identity across position changes.
#[derive(Clone, Default)]
struct SourceIndexMap {
    by_item: Rc<RefCell<HashMap<glib::Object, usize>>>,
    generation: Rc<Cell<u64>>,
    // Reading a filter model's item walks its match set, so selections map through this.
    sources: Rc<RefCell<Option<ViewSources>>>,
}

impl SourceIndexMap {
    fn watch(source: &impl IsA<gio::ListModel>) -> Self {
        let source = source.as_ref();
        let map = Self::default();
        let tracked = map.clone();
        // Capturing the model here would keep it alive through its own signal.
        source.connect_items_changed(move |source, position, removed, added| {
            tracked.apply(source, position, removed, added);
        });
        map.rebuild(source);
        map
    }

    fn apply(&self, source: &gio::ListModel, position: u32, removed: u32, added: u32) {
        self.generation.set(self.generation.get().wrapping_add(1));
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
        let mut by_item = self.by_item.borrow_mut();
        let previous_count = source.n_items() - added + removed;
        if by_item.len() != previous_count as usize || (position == 0 && removed == previous_count)
        {
            drop(by_item);
            self.rebuild(source);
            return;
        }
        let end = position.saturating_add(removed) as usize;
        by_item.retain(|_, index| {
            if *index < position as usize {
                true
            } else if *index < end {
                false
            } else {
                *index = *index - removed as usize + added as usize;
                true
            }
        });
        for index in position..position.saturating_add(added) {
            if let Some(item) = source.item(index) {
                by_item
                    .entry(item)
                    .and_modify(|last| *last = (*last).max(index as usize))
                    .or_insert(index as usize);
            }
        }
    }

    fn rebuild(&self, source: &gio::ListModel) {
        self.generation.set(self.generation.get().wrapping_add(1));
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

    /// Positions in `source` of this map's `view` items at `positions`.
    fn sources_of(
        &self,
        source: &SourceIndexMap,
        view: &gio::ListModel,
        positions: impl Iterator<Item = u32>,
    ) -> Vec<Option<usize>> {
        const UNMAPPED: u32 = u32::MAX;
        let key = (self.generation.get(), source.generation.get());
        let mut cache = self.sources.borrow_mut();
        let forward = match &mut *cache {
            Some((built, forward)) if *built == key => forward,
            slot => {
                let mut forward = vec![UNMAPPED; view.n_items() as usize];
                let by_source_item = source.by_item.borrow();
                for (item, position) in self.by_item.borrow().iter() {
                    if let Some(slot) = forward.get_mut(*position)
                        && let Some(source) = by_source_item
                            .get(item)
                            .and_then(|source| u32::try_from(*source).ok())
                    {
                        *slot = source;
                    }
                }
                &mut slot.insert((key, forward)).1
            }
        };
        positions
            .map(|position| match forward.get(position as usize) {
                Some(&source) if source != UNMAPPED => Some(source as usize),
                _ => source.of_view_position(view, position),
            })
            .collect()
    }
}

struct BoundModeItem {
    item: glib::WeakRef<gtk::ListItem>,
    widget: glib::WeakRef<gtk::Widget>,
    rename_label: glib::WeakRef<gtk::Widget>,
    edit: super::collection_edit::EditWidgets,
}

/// One collection view inside a pane. Icons and List each keep a single section.
#[derive(Clone)]
struct PaneSection {
    view: gtk::Widget,
    view_model: gio::ListModel,
    view_index: SourceIndexMap,
    selection: gtk::MultiSelection,
    bound_items: Rc<RefCell<Vec<BoundModeItem>>>,
    syncing: Rc<Cell<bool>>,
    visit: super::marquee::ItemVisitor,
    item_context_trigger: Rc<dyn Fn(f64, f64)>,
}

impl PaneSection {
    fn source_to_view(&self, source: &gtk::StringList, position: usize) -> Option<u32> {
        self.view_index
            .of_item(&source.item(position as u32)?)
            .map(|position| position as u32)
    }
}

type ListSorting = Rc<HeadingSort>;

struct HeadingSort {
    current: Cell<(SortKey, SortDirection)>,
    arrows: RefCell<Vec<(SortKey, gtk::Image)>>,
}

impl HeadingSort {
    fn get(&self) -> (SortKey, SortDirection) {
        self.current.get()
    }

    fn show(&self, key: SortKey, direction: SortDirection) {
        self.current.set((key, direction));
        for (arrow_key, arrow) in self.arrows.borrow().iter() {
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
    }
}

#[derive(Clone)]
struct Pane {
    depth: usize,
    location: Option<Location>,
    group_by_type: bool,
    type_group_sorter: Option<TypeGroupSorter>,
    sorting: Option<ListSorting>,
    sort_direction_button: Option<gtk::Button>,
    shell: gtk::Box,
    header: gtk::Box,
    model: gtk::StringList,
    source_index: SourceIndexMap,
    filter_model: Option<gtk::FilterListModel>,
    /// The primary section that owns the pane's chrome.
    section: PaneSection,
    sections: Rc<RefCell<Vec<PaneSection>>>,
    icons: Option<Rc<IconsContext>>,
    thumbnail_scale: Option<gtk::Scale>,
    targets: super::marquee::MarqueeTargets,
    /// Set while a reload has detached the pane's models from their views.
    detached: Rc<Cell<bool>>,
    stack: gtk::Stack,
    loading: super::loading_skeleton::DelayedLoading,
    status: gtk::Label,
    /// Shown under `status` when only permissions kept the user out of the folder.
    administrator: gtk::Button,
    spinner: gtk::Spinner,
    truncated_hint: gtk::Image,
    not_responding: gtk::Box,
    marquee: super::marquee::Marquee,
    search: super::inline_search::InlineSearch,
    filter_entry: Option<gtk::Entry>,
    filter_button: Option<gtk::ToggleButton>,
    empty_trash_button: Option<gtk::Button>,
    leftovers_button: gtk::Button,
    show_hidden: Rc<Cell<bool>>,
    filter: gtk::CustomFilter,
    filter_query: Rc<RefCell<String>>,
    folder_context_trigger: Rc<dyn Fn(f64, f64)>,
    list_headings: Option<gtk::Box>,
    list_columns: Option<ListColumnLayout>,
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
    pane_navigation: RefCell<navigation::PaneNavigation>,
    browser: Rc<Browser>,
    single_click_previews: Rc<Cell<bool>>,
    multiple_selection: Rc<Cell<bool>>,
    icons_click_activation: Rc<Cell<ClickActivation>>,
    list_click_activation: Rc<Cell<ClickActivation>>,
    transfer_handler: TransferHandlerSlot,
    clipboard_marks: Rc<RefCell<ClipboardMarks>>,
    context_state: Rc<RefCell<Option<Weak<super::browser::ViewState>>>>,
    new_folder_state: RefCell<Option<Weak<super::browser::ViewState>>>,
    active_rename: Rc<RefCell<Option<super::collection_edit::ActiveEdit>>>,
    search_selection_handlers: super::inline_search::SearchSelectionHandlers,
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
        icons_scroll.add_css_class("mode-scroll");

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
    pub(super) fn leading_marquee(&self) -> Option<super::marquee::Marquee> {
        let pane = match self.mode {
            BrowserMode::Columns => return None,
            BrowserMode::Icons => self.icons_panes.first(),
            BrowserMode::List => self.list_pane.as_ref(),
        }?;
        pane.search
            .active_marquee()
            .or_else(|| Some(pane.marquee.clone()))
    }

    fn single_pane(&self) -> Option<&Pane> {
        match self.mode {
            BrowserMode::Columns => None,
            BrowserMode::Icons => self.icons_panes.first(),
            BrowserMode::List => self.list_pane.as_ref(),
        }
    }

    pub fn refresh_source_filter(&self) {
        for pane in self.visible_panes() {
            pane.search.refresh_source_filter(&self.browser);
        }
    }

    pub fn prune_stale_search_results(&self) {
        if let Some(pane) = self.single_pane() {
            pane.search.prune_missing();
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
        self.focus_pane_surface()
    }

    pub fn focus_pane_surface(&self) -> bool {
        self.single_pane().is_some_and(focus_pane_surface)
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
            .flat_map(|section| selected_source_positions(&pane.source_index, section))
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
    ) -> Option<(FileEntry, String)> {
        super::collection_edit::take_submission(&self.active_rename, field)
    }

    pub(in crate::ui) fn take_rename(&self) -> Option<super::collection_edit::ActiveEdit> {
        self.active_rename.take()
    }

    pub(in crate::ui) fn rename_label_widgets(
        &self,
        old_location: &Location,
        new_location: Option<&Location>,
    ) -> Vec<gtk::Widget> {
        let mut labels = Vec::new();
        for pane in self.all_panes() {
            labels.extend(pane.search.rename_label_widgets(old_location, new_location));
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
        super::collection_edit::cancel(&self.active_rename)
    }

    fn rename_submit(&self) -> super::collection_edit::Submit {
        let state = self.context_state.borrow().clone().unwrap_or_default();
        let active = Rc::downgrade(&self.active_rename);
        let browser = Rc::downgrade(&self.browser);
        Rc::new(move |field| {
            if let Some(state) = state.upgrade() {
                state.submit_mode_rename(field);
            } else if let Some(active) = active.upgrade()
                && let Some((entry, name)) = super::collection_edit::take_submission(&active, field)
                && let Some(browser) = browser.upgrade()
            {
                super::browser::queue_rename(&browser, entry, name);
            }
        })
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

    pub(in crate::ui) fn icons_rename_view(&self, depth: usize) -> Option<gtk::Widget> {
        self.icons_panes
            .iter()
            .find(|pane| self.mode == BrowserMode::Icons && pane.depth == depth)
            .map(Pane::focus_view)
    }

    pub(in crate::ui) fn icons_rename_row(
        &self,
        depth: usize,
        source_position: usize,
    ) -> Option<(gtk::Widget, u32, Option<gtk::Widget>)> {
        let pane = self.icons_panes.iter().find(|pane| pane.depth == depth)?;
        pane.item_sections().iter().find_map(|section| {
            let position =
                view_position_for_source(&pane.model, Some(&section.view_model), source_position)?;
            let row = section.bound_items.borrow().iter().find_map(|bound| {
                (bound.item.upgrade()?.position() == position)
                    .then(|| bound.widget.upgrade())
                    .flatten()
                    .filter(|row| row.is_mapped() && row.is_ancestor(&section.view))
            });
            Some((section.view.clone(), position, row))
        })
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
                .filter(|row| row.is_mapped() && row.is_ancestor(&section.view))
        });
        Some((position, row))
    }

    pub fn begin_search_rename(&self, depth: usize, entry: &FileEntry) -> bool {
        let pane = match self.mode {
            BrowserMode::Columns => return false,
            BrowserMode::Icons => self.icons_panes.iter().find(|pane| pane.depth == depth),
            BrowserMode::List => self.list_pane.as_ref().filter(|pane| pane.depth == depth),
        };
        let Some(pane) = pane else {
            return false;
        };
        self.cancel_rename();
        let Some(mut target) = pane.search.edit_target(entry) else {
            return false;
        };
        self.bind_rename_cancellation(&mut target, &entry.location);
        super::collection_edit::begin(
            &self.active_rename,
            entry.clone(),
            target,
            self.rename_submit(),
        )
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
        let target = pane.item_sections().iter().find_map(|section| {
            let position =
                view_position_for_source(&pane.model, Some(&section.view_model), source_position)?;
            super::browser::prepare_collection_inline_edit(&section.view, position);
            section.bound_items.borrow().iter().find_map(|bound| {
                if bound.item.upgrade()?.position() != position {
                    return None;
                }
                let widget = bound.widget.upgrade()?;
                if !widget.is_mapped()
                    || widget.width() <= 0
                    || !widget.is_ancestor(&section.view)
                    || pane.stack.is_transition_running()
                {
                    return None;
                }
                Some((bound.edit.clone(), widget, section.view.clone()))
            })
        });
        let Some((widgets, row, collection)) = target else {
            return false;
        };
        let mut target = super::collection_edit::EditTarget::from(widgets);
        self.bind_rename_cancellation(&mut target, &entry.location);
        if self.mode == BrowserMode::List {
            let state = self.context_state.borrow().clone().unwrap_or_default();
            let generation = state
                .upgrade()
                .map(|state| state.rename_reveal_generation());
            let row = row.downgrade();
            let scroll = collection
                .ancestor(gtk::ScrolledWindow::static_type())
                .and_downcast::<gtk::ScrolledWindow>()
                .map(|scroll| scroll.downgrade());
            target.reveal = Some(Rc::new(move |_| {
                if state
                    .upgrade()
                    .map(|state| state.rename_reveal_generation())
                    != generation
                {
                    return;
                }
                if let (Some(row), Some(scroll)) = (
                    row.upgrade(),
                    scroll.as_ref().and_then(|scroll| scroll.upgrade()),
                ) {
                    super::browser::reveal_rename_row(&row, &scroll, None);
                }
            }));
        }
        super::collection_edit::begin(
            &self.active_rename,
            entry.clone(),
            target,
            self.rename_submit(),
        )
    }

    fn bind_rename_cancellation(
        &self,
        target: &mut super::collection_edit::EditTarget,
        location: &Location,
    ) {
        let state = self.context_state.borrow().clone().unwrap_or_default();
        let location = location.clone();
        target.cancelled = Some(Rc::new(move || {
            if let Some(state) = state.upgrade() {
                state.cancel_group_naming(&location);
            }
        }));
    }

    pub(in crate::ui) fn filter_focus(&self) -> Option<FilterFocus> {
        let focused = self.stack.root().and_then(|root| root.focus());
        // Previous-mode panes stay in the tree unmapped; a just-revealed field
        // can have focus before GTK maps it.
        self.visible_panes()
            .into_iter()
            .find_map(|pane| pane_filter_focus(pane, focused.as_ref()))
    }

    /// Whether the listing of `depth` holds focus, rather than its filter session.
    pub(in crate::ui) fn listing_holds_focus(&self, depth: usize) -> bool {
        self.panes_at(depth)
            .iter()
            .any(|pane| pane_holds_keyboard_focus(pane))
    }

    /// Whether a background update of the listing of `depth` may move focus: nothing in
    /// the window has it, or that listing already holds it. A focused widget that left
    /// the window, like a row a reload removed, leaves nothing focused.
    pub(in crate::ui) fn listing_may_take_focus(&self, depth: usize) -> bool {
        self.stack
            .root()
            .and_then(|root| root.focus())
            .is_none_or(|focused| focused.root().is_none())
            || self.listing_holds_focus(depth)
    }

    pub fn selected_search_result(&self) -> Option<FileEntry> {
        self.single_pane()?.search.selected_entry()
    }

    pub fn focus_search_results(&self) -> bool {
        self.single_pane()
            .is_some_and(|pane| pane.search.focus_current())
    }

    pub(in crate::ui) fn keyboard_peek_target(&self) -> Option<(gtk::Widget, usize, Location)> {
        if let Some(pane) = self.single_pane()
            && pane.search.selected_entries().is_some()
        {
            let (widget, entry) = pane.search.selected_anchor()?;
            return entry
                .is_directory()
                .then_some((widget, pane.depth, entry.location));
        }
        let (depth, position, entry) = self.browser.focused_item()?;
        if !entry.is_directory() {
            return None;
        }
        let pane = self.panes_at(depth).into_iter().next()?;
        for section in pane.item_sections() {
            let Some(view_position) =
                view_position_for_source(&pane.model, Some(&section.view_model), position)
            else {
                continue;
            };
            let Some(widget) = section.bound_items.borrow().iter().find_map(|bound| {
                let item = bound.item.upgrade()?;
                (item.position() == view_position).then(|| bound.widget.upgrade())?
            }) else {
                continue;
            };
            return Some((widget, depth, entry.location));
        }
        None
    }

    pub fn focus_search_result(&self, path: &std::path::Path) -> bool {
        self.single_pane()
            .is_some_and(|pane| pane.search.focus_result(path))
    }

    pub fn select_search_result(&self, path: &std::path::Path) -> bool {
        self.single_pane()
            .is_some_and(|pane| pane.search.select_result(path))
    }

    pub fn select_all_search_results(&self) -> bool {
        self.single_pane()
            .is_some_and(|pane| pane.search.select_all())
    }

    pub fn selected_search_results(&self) -> Option<Vec<FileEntry>> {
        self.single_pane()?.search.selected_entries()
    }

    pub(in crate::ui) fn filter_active(&self) -> bool {
        self.selected_search_results().is_some()
            || self
                .visible_panes()
                .into_iter()
                .filter_map(|pane| pane.filter_entry.as_ref())
                .any(|entry| !entry.text().trim().is_empty())
    }

    pub fn item_view_has_focus(&self) -> bool {
        let focused = self.stack.root().and_then(|root| root.focus());
        self.icons_panes
            .iter()
            .chain(self.list_pane.iter())
            .any(|pane| {
                pane.search.has_item_focus(focused.as_ref())
                    || focused.as_ref() == Some(pane.stack.upcast_ref())
                    || pane
                        .all_sections()
                        .iter()
                        .any(|section| widget_has_focus(&section.view, focused.as_ref()))
            })
    }

    pub fn empty_filter_has_focus(&self) -> bool {
        let focused = self.stack.root().and_then(|root| root.focus());
        self.visible_panes()
            .into_iter()
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

    /// The active pane's filter field, its funnel toggle, and the results that
    /// replace its listing while the field has text.
    pub(in crate::ui) fn active_filter(
        &self,
    ) -> Option<(
        gtk::Entry,
        gtk::ToggleButton,
        super::inline_search::InlineSearch,
    )> {
        self.filter_at(self.browser.active_depth()?)
    }

    pub(in crate::ui) fn filter_at(
        &self,
        depth: usize,
    ) -> Option<(
        gtk::Entry,
        gtk::ToggleButton,
        super::inline_search::InlineSearch,
    )> {
        let pane = self.panes_at(depth).into_iter().next()?;
        Some((
            pane.filter_entry.clone()?,
            pane.filter_button.clone()?,
            pane.search.clone(),
        ))
    }

    pub(in crate::ui) fn pane_searches(&self) -> Vec<super::inline_search::InlineSearch> {
        self.icons_panes
            .iter()
            .chain(self.list_pane.iter())
            .map(|pane| pane.search.clone())
            .collect()
    }

    pub(in crate::ui) fn hidden_filter_entries(&self) -> Vec<gtk::Entry> {
        self.icons_panes
            .iter()
            .chain(self.list_pane.iter())
            .filter(|pane| {
                pane.filter_button
                    .as_ref()
                    .is_some_and(|button| !button.is_active())
            })
            .filter_map(|pane| pane.filter_entry.clone())
            .filter(|entry| !entry.text().is_empty())
            .collect()
    }

    pub(crate) fn capture_active_filter(&self) -> super::browser::ActivePaneFilter {
        let Some(depth) = self.browser.active_depth() else {
            return super::browser::ActivePaneFilter::default();
        };
        let Some(pane) = self.panes_at(depth).into_iter().next() else {
            return super::browser::ActivePaneFilter::default();
        };
        super::browser::ActivePaneFilter {
            query: pane
                .filter_entry
                .as_ref()
                .map(|entry| entry.text().to_string())
                .unwrap_or_default(),
            revealed: pane
                .filter_button
                .as_ref()
                .is_some_and(|button| button.is_active()),
        }
    }

    pub(crate) fn restore_active_filter(&self, filter: &super::browser::ActivePaneFilter) {
        let Some(depth) = self.browser.active_depth() else {
            return;
        };
        let Some(pane) = self.panes_at(depth).into_iter().next() else {
            return;
        };
        let (Some(entry), Some(button)) = (pane.filter_entry.as_ref(), pane.filter_button.as_ref())
        else {
            return;
        };
        super::browser::restore_filter_controls(button, entry, filter);
        if !pane.search.replaces_listing() {
            super::browser::notify_filter_query(
                &pane.filter,
                &pane.filter_query,
                filter.query.clone(),
            );
        }
        if filter.query.trim().is_empty() {
            pane.search.show_directory_listing();
        }
    }

    /// Dismisses the filter whose field or results have focus, and returns focus to the
    /// directory cursor.
    pub fn dismiss_focused_filter(&self) -> bool {
        let focused = self.stack.root().and_then(|root| root.focus());
        let Some(pane) = self
            .visible_panes()
            .into_iter()
            .find(|pane| pane_filter_focus(pane, focused.as_ref()).is_some())
        else {
            return false;
        };
        if let Some(button) = pane.filter_button.as_ref() {
            button.set_active(false);
        }
        self.pane_navigation.borrow_mut().filter_dismissed();
        // Show the listing now rather than after the debounce, so focus lands on it.
        if pane.search.replaces_listing() {
            pane.search.flush_query();
        } else {
            super::browser::notify_filter_query(&pane.filter, &pane.filter_query, String::new());
        }
        if self.rename_is_active() {
            focus_pane_surface(pane);
        } else {
            self.focus_visible_pane(pane.depth);
        }
        true
    }

    pub fn cancel_pending_restore(&mut self) {
        self.cursor_focus_generation
            .set(self.cursor_focus_generation.get().wrapping_add(1));
        self.pane_navigation.borrow_mut().cancel();
    }

    pub fn prepare_mode(&mut self, mode: BrowserMode) {
        if self.mode == mode {
            return;
        }
        self.cancel_rename();
        self.cancel_pending_restore();
        self.mode = mode;
        match mode {
            BrowserMode::Columns => {}
            BrowserMode::Icons => self.prepare_icons(),
            BrowserMode::List => self.prepare_list(),
        }
    }

    fn prepare_icons(&mut self) {
        let Some(depth) = self.browser.active_depth() else {
            self.clear_icons();
            return;
        };
        let Some(snapshot) = self.browser.column_snapshot(depth) else {
            return;
        };
        if let Some(pane) = self.icons_panes.first()
            && pane.depth == depth
            && pane.location.as_ref() == Some(&snapshot.location)
        {
            apply_snapshot(pane, &snapshot, &self.browser);
            return;
        }
        self.rebuild_icons();
    }

    fn grouping_for_snapshot(&self, depth: usize, snapshot: &BrowserColumnSnapshot) -> bool {
        if snapshot.location.is_recent_root() {
            return false;
        }
        // GTK 4.22 cannot safely section interleaved camera batches. Device order
        // must also remain ungrouped after completion rather than reshuffling rows.
        let device_order = self
            .browser
            .column_preferences(depth)
            .is_some_and(|preferences| preferences.sort_key == SortKey::DeviceOrder);
        self.group_by_type
            && !(snapshot.location.is_camera_photo_root() && (snapshot.loading || device_order))
    }

    fn prepare_list(&mut self) {
        let Some(depth) = self.browser.active_depth() else {
            self.clear_list();
            return;
        };
        let Some(snapshot) = self.browser.column_snapshot(depth) else {
            return;
        };
        if let Some(pane) = self.list_pane.as_ref()
            && pane.depth == depth
            && pane.location.as_ref() == Some(&snapshot.location)
            && pane.group_by_type == self.grouping_for_snapshot(depth, &snapshot)
            && pane.sorting.as_ref().map(|sorting| sorting.get())
                == self
                    .browser
                    .column_preferences(depth)
                    .map(|preferences| (preferences.sort_key, preferences.sort_direction))
        {
            apply_snapshot(pane, &snapshot, &self.browser);
            return;
        }
        self.rebuild_list();
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
            BrowserMode::Icons => self.deactivate_icons(),
            BrowserMode::List => self.deactivate_list(),
        }
    }

    fn deactivate_icons(&self) {
        for pane in &self.icons_panes {
            deactivate_pane_models(pane);
        }
    }

    fn deactivate_list(&self) {
        if let Some(pane) = self.list_pane.as_ref() {
            deactivate_pane_models(pane);
        }
    }

    pub fn set_single_click_previews(&self, enabled: bool) {
        self.single_click_previews.set(enabled);
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

    pub fn connect_search_selection_changed(
        &self,
        handler: super::inline_search::SearchSelectionChanged,
    ) {
        self.search_selection_handlers.borrow_mut().push(handler);
    }

    pub fn set_clipboard_marks(&self, marks: &ClipboardMarks) {
        self.clipboard_marks.replace(marks.clone());
        for pane in self.icons_panes.iter().chain(self.list_pane.iter()) {
            refresh_mark_pane(pane, &self.browser, marks);
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
            root.remove_css_class("density-custom");
            root.add_css_class(match density {
                BrowserDensity::Compact => "density-compact",
                BrowserDensity::Airy => "density-airy",
                BrowserDensity::Custom(_) => "density-custom",
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

    /// Also updates a hidden List pane, which is reused when List is shown again.
    pub fn set_list_columns(&self) {
        if let Some(pane) = &self.list_pane
            && let (Some(headings), Some(columns)) = (&pane.list_headings, &pane.list_columns)
        {
            apply_columns_layout(headings, columns, &pane.section.bound_items);
        }
    }

    pub fn open_list_columns_popover(&self) {
        if self.mode == BrowserMode::List
            && let Some(pane) = &self.list_pane
            && let (Some(headings), Some(columns)) = (&pane.list_headings, &pane.list_columns)
        {
            column_options::open_column_options_popover(
                headings,
                headings,
                columns,
                &pane.section.bound_items,
                None,
            );
        }
    }

    /// The Icons slider and its folder, while Icons shows.
    pub(super) fn icons_thumbnail_scale(&self) -> Option<(gtk::Scale, Option<Location>)> {
        if self.mode != BrowserMode::Icons {
            return None;
        }
        let pane = self.icons_panes.first()?;
        let scale = pane.thumbnail_scale.clone()?;
        Some((scale, self.browser.location_at(pane.depth)))
    }

    pub fn apply_resolved_icons_size(&mut self, size: i32) {
        if self.icons_resolved_size.replace(size) != size {
            self.set_icons_thumbnail_size(size);
        }
    }

    fn set_icons_thumbnail_size(&mut self, size: i32) {
        if self.icons_thumbnail_size.replace(size) == size {
            return;
        }
        if let Some(scale) = self
            .icons_panes
            .first()
            .and_then(|pane| pane.thumbnail_scale.as_ref())
        {
            scale.set_value(f64::from(size));
        }
    }

    fn visible_panes(&self) -> Vec<&Pane> {
        match self.mode {
            BrowserMode::Columns => Vec::new(),
            BrowserMode::Icons => self.icons_panes.iter().collect(),
            BrowserMode::List => self.list_pane.iter().collect(),
        }
    }

    pub fn resume_native_selection(&self) -> bool {
        let Some((depth, focused, _)) = self.browser.focused_item() else {
            return false;
        };
        if !self.browser.selected_positions(depth).is_empty() {
            return false;
        }
        // Seed GTK's empty selection before the arrow moves it, without scheduling
        // a focus restore that would undo the native move after key dispatch.
        for pane in self.panes_at(depth) {
            set_selections(pane, &[focused]);
            reset_native_range_origin(pane, focused);
        }
        self.browser.set_selection(depth, &[focused], Some(focused));
        self.browser.set_selection_anchor(depth, focused);
        true
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

    pub(super) fn page_target(&self, depth: usize, direction: i32, page: usize) -> Option<usize> {
        let pane = self
            .visible_panes()
            .into_iter()
            .find(|pane| pane.depth == depth)?;
        let sections = pane.item_sections();
        let count: usize = sections
            .iter()
            .map(|section| section.view_model.n_items() as usize)
            .sum();
        let last = count.checked_sub(1)?;
        let focused = self.browser.focused_item().filter(|(d, _, _)| *d == depth);
        let current = if let Some((_, source, _)) = focused {
            let mut offset = 0;
            let current = sections.iter().find_map(|section| {
                let position = section.source_to_view(&pane.model, source);
                let start = offset;
                offset += section.view_model.n_items() as usize;
                position.map(|position| start + position as usize)
            });
            // A filtered-out cursor still needs the existing visual-order fallback.
            Some(current?)
        } else {
            None
        };
        let mut target = match (current, direction.cmp(&0)) {
            (_, std::cmp::Ordering::Equal) => return None,
            // A full jump is absolute even without a cursor, as in `NavigationState`.
            (_, std::cmp::Ordering::Less) if page == usize::MAX => 0,
            (_, std::cmp::Ordering::Greater) if page == usize::MAX => last,
            (None, std::cmp::Ordering::Less) => last,
            (None, _) => 0,
            (Some(current), std::cmp::Ordering::Less) => current.saturating_sub(page.max(1)),
            (Some(current), _) => current.saturating_add(page.max(1)).min(last),
        };
        for section in sections {
            let count = section.view_model.n_items() as usize;
            if target < count {
                return pane
                    .source_index
                    .of_view_position(&section.view_model, target as u32);
            }
            target -= count;
        }
        None
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

    pub fn reveal_selected_entry(&self, depth: usize, source_position: usize) {
        for pane in self
            .visible_panes()
            .into_iter()
            .filter(|pane| pane.depth == depth)
        {
            for section in pane.item_sections() {
                if let Some(position) = view_position_for_source(
                    &pane.model,
                    Some(&section.view_model),
                    source_position,
                ) {
                    super::browser::reveal_collection_after_layout(
                        &section.view,
                        position,
                        section.visit.clone(),
                    );
                    return;
                }
            }
        }
    }

    pub fn suppress_focus_scroll(&self) {
        self.suppress_focus_scroll.set(true);
    }

    pub(in crate::ui) fn set_cursor_keeps_focus(&self, keep: bool) {
        self.cursor_keeps_focus.set(keep);
    }

    pub(in crate::ui) fn cursor_view(
        &self,
        depth: usize,
        source: usize,
    ) -> Option<(gtk::Widget, u32)> {
        let panes = self.visible_panes();
        let pane = panes.iter().find(|pane| pane.depth == depth)?;
        pane.item_sections().into_iter().find_map(|section| {
            let position = section.source_to_view(&pane.model, source)?;
            (position < section.view_model.n_items()).then_some((section.view, position))
        })
    }

    pub(in crate::ui) fn refresh_name_highlights(&self, find: Option<&str>) {
        for pane in self.all_panes() {
            for section in pane.item_sections() {
                for bound in section.bound_items.borrow().iter() {
                    if let Some(label) = bound.rename_label.upgrade() {
                        super::browser::find::highlight_listing_name(
                            &label,
                            find,
                            &pane.filter_query.borrow(),
                        );
                    }
                }
            }
            pane.search.refresh_result_highlights(find);
        }
    }

    pub fn view_position_in(&self, view: &gtk::Widget, depth: usize, source: usize) -> Option<u32> {
        let panes = self.visible_panes();
        let pane = panes.iter().find(|pane| pane.depth == depth)?;
        pane.item_sections().into_iter().find_map(|section| {
            if section.view == *view {
                section.source_to_view(&pane.model, source)
            } else {
                None
            }
        })
    }

    pub(in crate::ui) fn take_results_focus(&self) -> Option<usize> {
        self.results_take_focus.take()
    }

    pub(in crate::ui) fn cancel_column_drag(&self) -> bool {
        if self.mode != BrowserMode::List {
            return false;
        }
        self.visible_panes().into_iter().any(|pane| {
            let Some(columns) = pane
                .list_columns
                .as_ref()
                .filter(|columns| columns.drag.get().is_some_and(|drag| drag.active))
            else {
                return false;
            };
            let Some(headings) = pane.list_headings.as_ref() else {
                return false;
            };
            // Reset can emit drag-end; discard the pending commit first.
            columns.finish_drag(headings, &pane.section.bound_items, None);
            let controllers = headings.observe_controllers();
            if let Some(drag) = (0..controllers.n_items())
                .find_map(|index| controllers.item(index).and_downcast::<gtk::GestureDrag>())
            {
                drag.reset();
            }
            true
        })
    }

    pub fn focus_visible_pane(&self, depth: usize) {
        if self.rename_is_active() || !super::focus_navigation::browser_can_take_focus(&self.stack)
        {
            return;
        }
        let Some(pane) = self
            .visible_panes()
            .into_iter()
            .find(|pane| pane.depth == depth)
        else {
            return;
        };
        if super::loading_skeleton::surface_takes_focus(&pane.stack) {
            pane.stack.grab_focus();
            return;
        }
        let focused = self
            .browser
            .focused_item()
            .filter(|(focused_depth, _, _)| *focused_depth == depth);
        let target = focused.as_ref().and_then(|(_, source, _)| {
            pane.item_sections().into_iter().find_map(|section| {
                let position = section.source_to_view(&pane.model, *source)?;
                (position < section.view_model.n_items()).then_some((section.view, position))
            })
        });
        let (view, position) = target.map_or_else(
            || (pane.focus_view(), None),
            |(view, position)| (view, Some(position)),
        );
        let generation = self.cursor_focus_generation.get();
        let had_live_focus = view
            .root()
            .and_then(|root| root.focus())
            .is_some_and(|focused| focused.is_mapped());
        if !view.grab_focus() {
            for pane in self.panes_at(depth) {
                pane.stack.grab_focus();
            }
            return;
        }
        // A recycled, unmapped rename row needs GTK's cursor restored, not just widget focus.
        if self.suppress_focus_scroll.replace(false) && had_live_focus {
            if let Some(position) = position
                && let Some(items) = pane
                    .item_sections()
                    .into_iter()
                    .find(|section| section.view == view)
                    .map(|section| section.bound_items.clone())
                && let Some((depth, source, entry)) = focused
            {
                focus_collection_cursor_when_bound(
                    view.downgrade(),
                    items,
                    position,
                    Rc::downgrade(&self.browser),
                    (depth, source, entry.location),
                );
            }
            return;
        }
        if let Some(position) = position {
            focus_collection_item(&view, position);
        }
        let view = view.downgrade();
        let stack = pane.stack.downgrade();
        let current_generation = self.cursor_focus_generation.clone();
        glib::idle_add_local_once(move || {
            if current_generation.get() != generation {
                return;
            }
            if let Some(view) = view.upgrade()
                && stack
                    .upgrade()
                    .is_some_and(|stack| !super::loading_skeleton::surface_takes_focus(&stack))
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

    pub(crate) fn show_leftovers(&self, depth: usize, shown: bool) {
        for pane in self.panes_at(depth) {
            pane.leftovers_button.set_visible(shown);
        }
    }

    /// The Icons or List pane showing `depth`.
    fn mode_pane(&self, depth: usize) -> Option<&Pane> {
        self.panes_at(depth).into_iter().next()
    }

    /// Filter results count as outside: they belong to the filter session, which a
    /// reload restores only the viewport for.
    fn focus_owner(&self, pane: &Pane) -> navigation::FocusOwner {
        let focused = pane.stack.root().and_then(|root| root.focus());
        match pane_filter_focus(pane, focused.as_ref()) {
            Some(FilterFocus::Entry) => navigation::FocusOwner::FilterEntry,
            Some(FilterFocus::Results) => navigation::FocusOwner::Outside,
            None if pane_holds_keyboard_focus(pane) => navigation::FocusOwner::Items,
            None => navigation::FocusOwner::Outside,
        }
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

    fn install_context_menu(&self, pane: &Pane) -> Rc<dyn Fn(f64, f64)> {
        let Some(state) = self.context_state.borrow().as_ref().and_then(Weak::upgrade) else {
            return Rc::new(|_, _| {});
        };
        let Some(location) = self.browser.location_at(pane.depth) else {
            return Rc::new(|_, _| {});
        };
        pane.search.install_context_menu(|view, resolve| {
            super::browser::install_resolved_item_context_menu(&state, view, resolve, pane.depth)
        });
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
        )
    }

    pub(super) fn context_menu_target(
        &self,
        depth: usize,
        position: Option<usize>,
    ) -> Option<crate::ui::browser::ContextMenuTarget> {
        let pane = self.panes_at(depth).into_iter().next()?;
        if pane.search.selected_entries().is_some() {
            if let Some(target) = pane.search.context_menu_target() {
                return Some(target);
            }
        } else if let Some(position) = position {
            for section in pane.item_sections() {
                let Some(view_position) =
                    view_position_for_source(&pane.model, Some(&section.view_model), position)
                else {
                    continue;
                };
                let Some(widget) = section.bound_items.borrow().iter().find_map(|bound| {
                    let item = bound.item.upgrade()?;
                    (item.position() == view_position).then(|| bound.widget.upgrade())?
                }) else {
                    continue;
                };
                if let Some(bounds) = widget.compute_bounds(&section.view) {
                    let x = if self.mode == BrowserMode::List {
                        bounds.center().x()
                    } else {
                        bounds.x() + bounds.width()
                    };
                    return Some((
                        section.item_context_trigger.clone(),
                        f64::from(x),
                        f64::from(bounds.center().y()),
                    ));
                }
            }
            return None;
        }
        let width = f64::from(pane.stack.width());
        let height = f64::from(pane.stack.height());
        (width > 0.0 && height > 0.0).then(|| {
            (
                pane.folder_context_trigger.clone(),
                width / 2.0,
                height / 2.0,
            )
        })
    }

    fn clear_icons(&mut self) {
        self.pane_navigation.borrow_mut().cancel();
        for pane in &self.icons_panes {
            detach_pane_models(pane);
        }
        clear_box(&self.icons_root);
        self.icons_panes.clear();
    }

    fn clear_list(&mut self) {
        self.pane_navigation.borrow_mut().cancel();
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
        let size = crate::ui::preferences::PreferenceManager::shared()
            .icons_size_for(Some(&snapshot.location));
        self.icons_resolved_size.set(size);
        self.icons_thumbnail_size.set(size);
        let mut pane = build_icons_pane(
            self.browser.clone(),
            ModeClickOptions {
                previews: self.single_click_previews.clone(),
                activation: self.icons_click_activation.clone(),
                multiple_selection: self.multiple_selection.clone(),
            },
            self.transfer_handler.clone(),
            self.clipboard_marks.clone(),
            IconsOptions {
                state: self.context_state.borrow().clone(),
                new_folder_state: self.new_folder_state.borrow().clone(),
                search_selection_handlers: self.search_selection_handlers.clone(),
                search_context_state: self.context_state.clone(),
                thumbnail_size: self.icons_thumbnail_size.clone(),
                group_by_type: false,
                density: self.density,
            },
            depth,
            &snapshot.location.display_name(),
        );
        install_pane_surface(&pane, &self.browser, &snapshot.location.display_name());
        configure_icons_density(&pane, self.density);
        pane.folder_context_trigger = self.install_context_menu(&pane);
        self.icons_root.append(&pane.shell);
        apply_snapshot(&pane, &snapshot, &self.browser);
        self.pane_navigation.borrow_mut().prepare(&pane, &snapshot);
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
        let mut pane = build_list_pane(
            self.browser.clone(),
            ModeClickOptions {
                previews: self.single_click_previews.clone(),
                activation: self.list_click_activation.clone(),
                multiple_selection: self.multiple_selection.clone(),
            },
            self.transfer_handler.clone(),
            self.clipboard_marks.clone(),
            ListOptions {
                state: self.context_state.borrow().clone(),
                new_folder_state: self.new_folder_state.borrow().clone(),
                search_selection_handlers: self.search_selection_handlers.clone(),
                search_context_state: self.context_state.clone(),
                group_by_type: self.grouping_for_snapshot(depth, &snapshot),
            },
            depth,
            &snapshot.location.display_name(),
        );
        install_pane_surface(&pane, &self.browser, &snapshot.location.display_name());
        pane.folder_context_trigger = self.install_context_menu(&pane);
        self.list_root.append(&pane.shell);
        apply_snapshot(&pane, &snapshot, &self.browser);
        self.pane_navigation.borrow_mut().prepare(&pane, &snapshot);
        self.list_pane = Some(pane);
    }
}

fn widget_has_focus(widget: &impl IsA<gtk::Widget>, focused: Option<&gtk::Widget>) -> bool {
    widget.has_focus()
        || focused.is_some_and(|focused| {
            focused == widget.as_ref() || focused.is_ancestor(widget.as_ref())
        })
}

/// Only a revealed funnel counts: a 10xer footer filter leaves it closed and has its own
/// Escape order. Away from native folders the filter narrows the listing in place, so its
/// narrowed rows are the results.
fn pane_filter_focus(pane: &Pane, focused: Option<&gtk::Widget>) -> Option<FilterFocus> {
    if pane
        .filter_entry
        .as_ref()
        .is_some_and(|entry| widget_has_focus(entry, focused))
    {
        return Some(FilterFocus::Entry);
    }
    let revealed = pane
        .filter_button
        .as_ref()
        .is_some_and(gtk::ToggleButton::is_active);
    let results = if pane.search.replaces_listing() {
        pane.search.results_view().is_some() && pane.search.has_item_focus(focused)
    } else {
        !pane.filter_query.borrow().is_empty()
            && pane
                .item_sections()
                .iter()
                .any(|section| widget_has_focus(&section.view, focused))
    };
    (revealed && results).then_some(FilterFocus::Results)
}

/// Any part of the pane holds focus, its filter session included.
fn pane_contains_focus(pane: &Pane) -> bool {
    let focused = pane.stack.root().and_then(|root| root.focus());
    widget_has_focus(&pane.stack, focused.as_ref())
        || pane
            .item_sections()
            .iter()
            .any(|section| widget_has_focus(&section.view, focused.as_ref()))
}

/// The listing itself holds focus: the pane surface or one of its item views, not the
/// filter field or the results that replace the listing, 10xer's included. Loads and
/// live changes refocus the listing only then.
fn pane_holds_keyboard_focus(pane: &Pane) -> bool {
    let focused = pane.stack.root().and_then(|root| root.focus());
    let filter_session = pane
        .filter_entry
        .as_ref()
        .is_some_and(|entry| widget_has_focus(entry, focused.as_ref()))
        || pane.search.has_item_focus(focused.as_ref());
    !filter_session && pane_contains_focus(pane)
}

/// Where GTK leaves focus it moved off a pane a reload hid: nowhere, on a widget that
/// left the window, on the pane surface, or on a container around the pane.
fn reload_focus_fell_back(pane: &Pane) -> bool {
    let Some(focused) = pane.stack.root().and_then(|root| root.focus()) else {
        return true;
    };
    focused.root().is_none()
        || focused == *pane.stack.upcast_ref::<gtk::Widget>()
        || pane.stack.is_ancestor(&focused)
}

fn install_tab_landing(view: &gtk::Widget, state: Option<Weak<super::browser::ViewState>>) {
    let Some(state) = state else {
        return;
    };
    let focus = gtk::EventControllerFocus::new();
    focus.connect_enter(move |_| {
        if let Some(state) = state.upgrade() {
            state.land_tab_crossing();
        }
    });
    view.add_controller(focus);
}

/// The page a List or Icons pane shows for an empty or unreadable directory.
const STATUS_PAGE: &str = "status";

fn install_pane_surface(pane: &Pane, browser: &Rc<Browser>, directory: &str) {
    let depth = pane.depth;
    let weak = Rc::downgrade(browser);
    pane.administrator.connect_clicked(move |_| {
        if let Some(browser) = weak.upgrade()
            && let Some(location) = browser
                .column_snapshot(depth)
                .and_then(|snapshot| snapshot.administrator)
        {
            browser.navigate_location(location, true);
        }
    });
    super::loading_skeleton::DirectorySurface::install(
        &pane.stack,
        &pane.status,
        &pane.section.view,
        pane.search.result_collection_view().as_ref(),
        browser,
        directory,
    );
}

/// Focuses the pane's collection view, or the pane itself while it shows a status or
/// loading page.
fn focus_pane_surface(pane: &Pane) -> bool {
    super::loading_skeleton::focus_surface_or(&pane.stack, &pane.focus_view())
}

#[derive(Clone)]
struct ListOptions {
    state: Option<Weak<super::browser::ViewState>>,
    new_folder_state: Option<Weak<super::browser::ViewState>>,
    search_selection_handlers: super::inline_search::SearchSelectionHandlers,
    search_context_state: Rc<RefCell<Option<Weak<super::browser::ViewState>>>>,
    group_by_type: bool,
}

struct IconsOptions {
    state: Option<Weak<super::browser::ViewState>>,
    new_folder_state: Option<Weak<super::browser::ViewState>>,
    search_selection_handlers: super::inline_search::SearchSelectionHandlers,
    search_context_state: Rc<RefCell<Option<Weak<super::browser::ViewState>>>>,
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

fn search_collection_options(
    presentation: super::inline_search::SearchPresentation,
    browser: &Rc<Browser>,
    previews: Rc<Cell<bool>>,
    activation: Rc<Cell<ClickActivation>>,
    multiple_selection: Rc<Cell<bool>>,
    selection_handlers: super::inline_search::SearchSelectionHandlers,
    context_state: Rc<RefCell<Option<Weak<super::browser::ViewState>>>>,
) -> super::inline_search::SearchCollectionOptions {
    let weak_browser = Rc::downgrade(browser);
    let activate = Rc::new(move |entry: FileEntry| {
        let Some(browser) = weak_browser.upgrade() else {
            return;
        };
        if entry.is_directory() {
            browser.navigate(entry.location);
        } else {
            browser.open_location(entry.location);
        }
    });
    let weak_browser = Rc::downgrade(browser);
    let pointer_click = Rc::new(move |entry: FileEntry, presses: i32| {
        let Some(browser) = weak_browser.upgrade() else {
            return;
        };
        if entry.is_directory() {
            if presses == 1 {
                browser.navigate(entry.location);
            }
            return;
        }
        let chooser = browser.is_chooser_mode();
        let files = activation.get().files;
        if presses >= 2 {
            // With one-click files the first press already opened it.
            if !chooser && files != ClickCount::One {
                browser.open_location(entry.location);
            }
            return;
        }
        match filtered_file_click(&entry, chooser, previews.get(), files) {
            FilteredFileClick::Open => browser.open_location(entry.location),
            FilteredFileClick::Preview => browser.request_preview(entry),
            FilteredFileClick::Ignore => {}
        }
    });
    let focus_state = context_state.clone();
    let focus_items = Rc::new(move || {
        if let Some(state) = focus_state.borrow().as_ref().and_then(Weak::upgrade) {
            super::browser::claim_keyboard_navigation(&state);
        }
    });
    let results_changed = Rc::new(move || {
        if let Some(state) = context_state.borrow().as_ref().and_then(Weak::upgrade) {
            state.notify_filter_results_changed();
        }
    });
    let selection_changed = Rc::new(move |entries: Vec<FileEntry>| {
        for handler in selection_handlers.borrow().iter() {
            handler(entries.clone());
        }
    });
    super::inline_search::SearchCollectionOptions {
        presentation,
        multiple_selection,
        activate,
        pointer_click,
        selection_changed,
        focus_items,
        results_changed,
    }
}

struct IconsControls {
    sort_direction_button: gtk::Button,
    leading: gtk::Box,
    actions: gtk::Box,
    filter_entry: gtk::Entry,
    filter_revealer: gtk::Revealer,
    filter_button: gtk::ToggleButton,
    thumbnail_scale: gtk::Scale,
    thumbnail_value: gtk::Label,
    thumbnail_popover: gtk::Popover,
    empty_trash_button: Option<gtk::Button>,
    leftovers_button: gtk::Button,
}

/// `tooltip` names the filter shortcut with `%{keys}`.
pub(crate) fn filter_controls(
    tooltip: &'static str,
) -> (gtk::Entry, gtk::Revealer, gtk::ToggleButton) {
    let entry = gtk::Entry::builder()
        .placeholder_text(super::browser::filter_placeholder(0))
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
    let button = gtk::ToggleButton::new();
    let labelled_entry = entry.clone();
    // 10xer hides the button, so it names the default map's key.
    super::shortcut_reference::follow_keys(&button, move |button| {
        let keys = super::keys::label_in_map(
            &rust_i18n::locale(),
            &super::keys::Keys::One(super::keys::bindings::FILTER.shown()),
            super::keys::Map::Default,
        );
        let label = rust_i18n::t!(tooltip, keys = keys);
        button.set_tooltip_text(Some(&label));
        super::accessibility::set_label(button, &label);
        super::accessibility::set_label(&labelled_entry, &label);
    });
    button.set_child(Some(&crate::assets::chrome_icon(
        crate::assets::icons::FUNNEL,
    )));
    crate::ui::controls::pane_header_action(&button);
    let shown_filter = revealer.clone();
    let focused_filter = entry.clone();
    button.connect_toggled(move |button| {
        if crate::ui::tenxer_mode::chrome_suppressed() {
            return;
        }
        shown_filter.set_reveal_child(button.is_active());
        if button.is_active() {
            focused_filter.grab_focus();
        } else {
            focused_filter.set_text("");
        }
    });
    crate::ui::tenxer_mode::hide_filter_while_enabled(&button, &revealer);
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
    let thumbnail_title = gtk::Label::new(Some(&crate::i18n::tr("Thumbnail size")));
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
    let small = gtk::Label::new(Some(&crate::i18n::tr("Small")));
    small.set_xalign(0.0);
    small.set_hexpand(true);
    let large = gtk::Label::new(Some(&crate::i18n::tr("Large")));
    large.set_xalign(1.0);
    thumbnail_extremes.append(&small);
    thumbnail_extremes.append(&large);
    crate::ui::accessibility::set_labelled_by(&thumbnail_scale, &[thumbnail_title.upcast_ref()]);
    thumbnail_content.append(&thumbnail_heading);
    thumbnail_content.append(&thumbnail_scale);
    thumbnail_content.append(&thumbnail_extremes);
    append_icons_size_actions(
        browser,
        depth,
        &thumbnail_content,
        &thumbnail_popover,
        &thumbnail_scale,
    );
    thumbnail_popover.set_child(Some(&thumbnail_content));
    let thumbnail_menu = gtk::MenuButton::builder()
        .tooltip_text(crate::i18n::tr("Thumbnail size"))
        .popover(&thumbnail_popover)
        .build();
    super::accessibility::set_label(&thumbnail_menu, &crate::i18n::tr("Thumbnail size"));
    crate::ui::controls::pane_header_action(&thumbnail_menu);
    thumbnail_menu.add_css_class("icons-thumbnail-menu");
    thumbnail_menu.set_child(Some(&crate::assets::chrome_icon(
        crate::assets::icons::PICTURES,
    )));
    let leftovers = super::browser::leftovers_button(browser, depth);
    actions.append(&leftovers);
    let empty_trash = super::browser::empty_trash_button(browser);
    let is_trash = browser
        .location_at(depth)
        .is_some_and(|location| super::browser::is_trash_root(&location));
    empty_trash.set_visible(is_trash);
    empty_trash.set_sensitive(false);
    actions.append(&empty_trash);
    actions.append(&super::browser::pane_refresh_button(browser, depth));
    actions.append(&thumbnail_menu);
    let sort_direction_button = super::browser::column_sort_direction_toggle(browser, depth);
    actions.append(&sort_direction_button);
    actions.append(&super::browser::column_sort_menu(browser, depth));

    let (filter_entry, filter_revealer, filter_button) = filter_controls("Filter icons (%{keys})");
    actions.append(&filter_button);
    IconsControls {
        sort_direction_button,
        leading,
        actions,
        filter_entry,
        filter_revealer,
        filter_button,
        thumbnail_scale,
        thumbnail_value,
        thumbnail_popover,
        empty_trash_button: is_trash.then_some(empty_trash),
        leftovers_button: leftovers,
    }
}

/// Shown only when a remembered folder stores its own size, i.e. it differs from the default.
fn append_icons_size_actions(
    browser: &Rc<Browser>,
    depth: usize,
    content: &gtk::Box,
    popover: &gtk::Popover,
    scale: &gtk::Scale,
) {
    let remembered_location = {
        let browser = Rc::downgrade(browser);
        move || {
            let location = browser.upgrade()?.location_at(depth)?;
            crate::ui::preferences::PreferenceManager::shared()
                .remembers(&location)
                .then_some(location)
        }
    };
    let make_default = {
        let scale = scale.downgrade();
        move || {
            if let Some(scale) = scale.upgrade() {
                crate::ui::preferences::PreferenceManager::shared()
                    .set_default_icons_size(scale.value().round() as i32);
            }
        }
    };
    let reset = {
        let remembered_location = remembered_location.clone();
        move || {
            if let Some(location) = remembered_location() {
                crate::ui::preferences::PreferenceManager::shared()
                    .reset_folder_icons_size(&location);
            }
        }
    };
    let actions = crate::ui::controls::DefaultActions::new(
        popover,
        (
            &crate::i18n::tr("Make this the default size"),
            &crate::i18n::tr("Reset to default size"),
        ),
        make_default,
        reset,
    );
    actions.widget().add_css_class("column-menu");
    content.append(actions.widget());

    let sync = {
        let scale = scale.downgrade();
        move || {
            let Some(scale) = scale.upgrade() else {
                return;
            };
            let differs = remembered_location().is_some()
                && scale.value().round() as i32
                    != crate::ui::preferences::PreferenceManager::shared().icons_thumbnail_size();
            actions.set_visible(differs, differs);
        }
    };
    sync();
    {
        let sync = sync.clone();
        popover.connect_map(move |_| sync());
    }
    scale.connect_value_changed(move |_| sync());
}

pub(super) fn disable_scale_long_press_zoom(scale: &gtk::Scale) {
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
    marks: Rc<RefCell<ClipboardMarks>>,
    state: Option<Weak<super::browser::ViewState>>,
    thumbnail_size: Rc<Cell<i32>>,
    source_index: SourceIndexMap,
    sections: Weak<RefCell<Vec<PaneSection>>>,
    density: Cell<BrowserDensity>,
    scrolling: Rc<Cell<bool>>,
    filter_query: Rc<RefCell<String>>,
    enter_target: Option<super::enter_target::EnterTarget>,
}

fn build_icons_pane(
    browser: Rc<Browser>,
    click_options: ModeClickOptions,
    transfer_handler: TransferHandlerSlot,
    clipboard_marks: Rc<RefCell<ClipboardMarks>>,
    options: IconsOptions,
    depth: usize,
    title: &str,
) -> Pane {
    let location = browser.location_at(depth);
    let search_selection_handlers = options.search_selection_handlers.clone();
    let controls = icons_controls(&browser, depth, options.thumbnail_size.get());
    if let Some(state) = options.new_folder_state {
        controls
            .actions
            .prepend(&super::browser::pane_new_folder_button(state, depth));
    }
    let (
        shell,
        header,
        content,
        model,
        stack,
        status,
        administrator,
        spinner,
        truncated_hint,
        not_responding,
    ) = pane_base(
        title,
        BrowserMode::Icons,
        "icons-pane",
        &icons_loading_skeleton(options.thumbnail_size.get(), options.density),
        Some(controls.leading.clone().upcast()),
        Some(controls.actions.clone().upcast()),
    );
    if location.as_ref().is_some_and(Location::is_administrator) {
        shell.insert_child_after(
            &super::browser::administrator_notice(),
            Some(&not_responding),
        );
    }
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
    // Recursive-filter results replace a native listing; hiding its rows too
    // would drop their selection out of sight.
    let results_replace_listing = Rc::new(Cell::new(false));
    let enter_target = in_place_enter_target(&controls.filter_entry, &browser, depth);
    super::browser::bind_listing_filter(
        &controls.filter_entry,
        &filter,
        &filter_query,
        results_replace_listing.clone(),
        options.state.clone(),
        enter_target.clone(),
    );
    let sections: Rc<RefCell<Vec<PaneSection>>> = Rc::new(RefCell::new(Vec::new()));
    let context = Rc::new(IconsContext {
        browser,
        depth,
        click: click_options,
        transfer: transfer_handler,
        marks: clipboard_marks,
        state: options.state,
        thumbnail_size: options.thumbnail_size.clone(),
        source_index: source_index.clone(),
        sections: Rc::downgrade(&sections),
        density: Cell::new(options.density),
        scrolling: Rc::new(Cell::new(false)),
        filter_query: filter_query.clone(),
        enter_target: enter_target.clone(),
    });
    let type_group_sorter = options
        .group_by_type
        .then(|| pane_type_group_sorter(&context.browser, depth));
    let view_model = if let Some(sorter) = &type_group_sorter {
        let sorted =
            gtk::SortListModel::new(Some(filtered_model.clone()), None::<gtk::CustomSorter>);
        sorted.set_sorter(Some(&sorter.sorter));
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
    let enter_for_activate = enter_target.clone();
    controls.filter_entry.connect_activate(move |_| {
        if filter_query_for_activate.borrow().trim().is_empty() {
            return;
        }
        activate_filtered_item(
            &browser_for_filter_activate,
            filter_activation_position(enter_for_activate.as_ref(), &selection_for_filter_activate),
            &view_for_filter_activate,
            &source_index_for_filter_activate,
            depth_for_filter_activate,
        );
    });
    if let Some(enter_target) = &enter_target {
        connect_in_place_down(
            &controls.filter_entry,
            enter_target,
            &pane_section.view,
            context.state.clone(),
        );
    }
    sections.borrow_mut().push(pane_section.clone());
    let root = pane_section.view.clone();

    let density_for_size = context.density.get();
    let sections_for_size = Rc::downgrade(&sections);
    let browser_for_size = Rc::downgrade(&context.browser);
    let depth_for_size = context.depth;
    let thumbnail_size_for_change = options.thumbnail_size.clone();
    let value_for_change = controls.thumbnail_value.clone();
    let loading_stack = stack.downgrade();
    let loading_context = Rc::downgrade(&context);
    controls
        .thumbnail_scale
        .connect_value_changed(move |scale| {
            let size = scale.value().round() as i32;
            value_for_change.set_label(&format!("{size} px"));
            // A size applied from preferences updates the cell first and is not saved back.
            if thumbnail_size_for_change.replace(size) != size {
                let location = browser_for_size
                    .upgrade()
                    .and_then(|browser| browser.location_at(depth_for_size));
                crate::ui::preferences::PreferenceManager::shared()
                    .set_folder_icons_size(location.as_ref(), size);
            }
            if let (Some(stack), Some(context)) =
                (loading_stack.upgrade(), loading_context.upgrade())
            {
                let was_loading = stack.visible_child_name().as_deref()
                    == Some(super::loading_skeleton::LOADING_PAGE);
                if let Some(old) = stack.child_by_name(super::loading_skeleton::LOADING_PAGE) {
                    stack.remove(&old);
                }
                stack.add_named(
                    &icons_loading_skeleton(size, context.density.get()),
                    Some(super::loading_skeleton::LOADING_PAGE),
                );
                if was_loading {
                    stack.set_visible_child_name(super::loading_skeleton::LOADING_PAGE);
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
    scroll.add_css_class("browser-listing-scroll");
    if let Some(enter_target) = &enter_target {
        attach_in_place(enter_target, &pane_section, &scroll);
    }
    super::scrolling::popover::dismiss_on_outside_scroll(&controls.thumbnail_popover);
    let browser_for_settle = Rc::downgrade(&context.browser);
    let source_index_for_settle = context.source_index.clone();
    let sections_for_settle = context.sections.clone();
    let marks_for_settle = context.marks.clone();
    let depth_for_settle = context.depth;
    install_scroll_refresh(&scroll, context.scrolling.clone(), move || {
        let Some(browser) = browser_for_settle.upgrade() else {
            return;
        };
        let Some(sections) = sections_for_settle.upgrade() else {
            return;
        };
        let marks = marks_for_settle.borrow();
        for section in sections.borrow().iter() {
            refresh_icons_section(
                &browser,
                depth_for_settle,
                &source_index_for_settle,
                section,
                &marks,
            );
        }
    });
    let width_view = pane_section.view.downgrade();
    let width_context = Rc::downgrade(&context);
    after_icons_viewport_width_changes(&scroll, move |width| {
        let (Some(context), Some(view)) = (width_context.upgrade(), width_view.upgrade()) else {
            return;
        };
        let Some(sections) = context.sections.upgrade() else {
            return;
        };
        let sections = sections.borrow();
        let Some(section) = sections.iter().find(|section| section.view == view) else {
            return;
        };
        pin_ungrouped_icons_columns(section, width, context.density.get());
    });
    let targets: super::marquee::MarqueeTargets = Rc::new(RefCell::new(Vec::new()));
    let (collection, marquee) = collection_with_marquee(
        &root,
        scroll,
        targets.clone(),
        false,
        context.click.multiple_selection.clone(),
    );
    let browser_for_search = context.browser.clone();
    let search = super::inline_search::wrap(
        &collection,
        &controls.filter_entry,
        move || {
            browser_for_search
                .location_at(depth)
                .and_then(|location| location.native_path().map(std::path::Path::to_path_buf))
        },
        &context.browser,
        search_collection_options(
            super::inline_search::SearchPresentation::Icons {
                thumbnail_size: context.thumbnail_size.clone(),
                max_columns: density_icons_columns(context.density.get()),
            },
            &context.browser,
            context.click.previews.clone(),
            context.click.activation.clone(),
            context.click.multiple_selection.clone(),
            search_selection_handlers,
            options.search_context_state.clone(),
        ),
    );
    results_replace_listing.set(search.replaces_listing());
    content.append(&search.widget);
    marquee.add_origin_surface(&header);
    let pane = Pane {
        depth,
        location,
        group_by_type: false,
        type_group_sorter,
        sorting: None,
        shell,
        header,
        model,
        source_index,
        filter_model: Some(filtered_model),
        section: pane_section,
        sections,
        icons: Some(context),
        thumbnail_scale: Some(controls.thumbnail_scale),
        targets,
        detached: Rc::new(Cell::new(false)),
        loading: super::loading_skeleton::DelayedLoading::new(&stack),
        stack,
        status,
        administrator,
        spinner,
        truncated_hint,
        not_responding,
        marquee,
        search,
        filter_entry: Some(controls.filter_entry),
        filter_button: Some(controls.filter_button),
        empty_trash_button: controls.empty_trash_button,
        leftovers_button: controls.leftovers_button,
        sort_direction_button: Some(controls.sort_direction_button),
        show_hidden,
        filter: filter_for_pane,
        filter_query,
        folder_context_trigger: Rc::new(|_, _| {}),
        list_headings: None,
        list_columns: None,
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
    let view_index = SourceIndexMap::watch(&view_model);
    let selection = gtk::MultiSelection::new(Some(view_model.clone()));
    let syncing_selection = Rc::new(Cell::new(false));
    let bound_items: Rc<RefCell<Vec<BoundModeItem>>> = Rc::new(RefCell::new(Vec::new()));
    let factory = gtk::SignalListItemFactory::new();
    let bound_items_for_setup = bound_items.clone();
    let filter_query_for_setup = context.filter_query.clone();
    let selection_for_setup = selection.clone();
    let syncing_for_setup = syncing_selection.clone();
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
    let state_for_clicks = context.state.clone();
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
        install_icons_content_hover(&card);
        let slow_click = Rc::new(SlowClickRename::default());
        let weak_state_for_clicks = state_for_clicks.clone().unwrap_or_default();
        install_preview_click(
            &card,
            item,
            &rename_label,
            browser_for_setup.clone(),
            weak_state_for_clicks.clone(),
            previews_for_setup.clone(),
            activation_for_setup.clone(),
            depth,
            Some((source_index_for_setup.clone(), filtered_for_setup.clone())),
            filter_query_for_setup.clone(),
            slow_click.clone(),
        );
        let content_click = install_modified_selection_click(
            &card,
            item,
            selection_for_setup.clone(),
            syncing_for_setup.clone(),
            browser_for_setup.clone(),
            weak_state_for_clicks.clone(),
            depth,
            positions_for_setup.clone(),
            slow_click.clone(),
        );
        install_folder_peek(
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
            peek_for_setup.clone(),
            (&content_click, false, slow_click),
        );
        item.set_child(Some(&card));
        if let Some(parent) = card.parent() {
            parent.set_halign(gtk::Align::Center);
            parent.set_valign(gtk::Align::Start);
        }
        let field = super::icons_cell::ensure_rename_field(&card).expect("card editor");
        let edit = super::collection_edit::EditWidgets::new(&field, &rename_label);
        register_bound_mode_item(&bound_items_for_setup, item, &card, &rename_label, edit);
    });
    let browser_for_bind = Rc::downgrade(&context.browser);
    let source_index_for_bind = context.source_index.clone();
    let marks_for_bind = context.marks.clone();
    let thumbnail_size_for_bind = context.thumbnail_size.clone();
    let scrolling_for_bind = context.scrolling.clone();
    let state_for_bind = context.state.clone();
    let filter_query_for_bind = context.filter_query.clone();
    let enter_for_bind = context.enter_target.clone();
    factory.connect_bind(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(card) = item.child().and_downcast::<gtk::Box>() else {
            return;
        };
        if let Some(enter_target) = &enter_for_bind {
            enter_target.decorate(&card, item);
        }
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
            let edit = bound_edit(item);
            if let Some(edit) = &edit {
                edit.bind(&entry.location);
            }
            apply_icons_entry(
                Some(item),
                &card,
                &entry,
                &marks_for_bind.borrow(),
                thumbnail_size,
                scrolling_for_bind.get(),
                state.as_deref(),
            );
            if let Some((_, label)) = super::icons_cell::parts(&card) {
                super::browser::find::highlight_listing_name(
                    label.upcast_ref(),
                    state
                        .as_ref()
                        .and_then(|state| state.find_highlight())
                        .as_deref(),
                    &filter_query_for_bind.borrow(),
                );
            }
            if let Some(edit) = &edit {
                edit.display.set_visible(!edit.is_editing());
                edit.field.set_visible(edit.is_editing());
            }
            if let Some(position) = metadata_fill_position(source_position, &entry, false, true)
                && let Some(browser) = browser.as_ref()
                && let Some((icon, _)) = super::icons_cell::parts(&card)
            {
                super::thumbnail::request_metadata(
                    &icon,
                    &card,
                    browser,
                    depth,
                    position,
                    entry.location.clone(),
                    true,
                );
            }
        }
    });
    install_edit_unbind(&factory, &bound_items);
    let view = gtk::GridView::new(Some(selection.clone()), Some(factory));
    view.add_css_class("file-icons");
    view.set_vexpand(false);
    view.set_enable_rubberband(false);
    view.set_single_click_activate(false);
    view.set_tab_behavior(gtk::ListTabBehavior::Item);
    install_tab_landing(view.upcast_ref(), context.state.clone());
    configure_icons_view_density(&view, context.density.get());
    super::accessibility::describe_entry_container(
        &view,
        &pane_directory_name(&context.browser, depth),
    );

    let weak_browser = Rc::downgrade(&context.browser);
    let weak_state_for_activate = context.state.clone();
    let source_index_for_activation = context.source_index.clone();
    let filtered_for_activation = view_model.clone();
    view.connect_activate(move |_, position| {
        if let Some(state) = weak_state_for_activate.as_ref().and_then(Weak::upgrade) {
            state.cancel_click_rename();
        }
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
    let mut section = PaneSection {
        view: view.clone().upcast(),
        view_index,
        view_model,
        selection,
        bound_items: bound_items.clone(),
        syncing: syncing_selection,
        visit: bound_item_visitor(bound_items),
        item_context_trigger: Rc::new(|_, _| {}),
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
        section.item_context_trigger = install_section_context_menu(
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

fn install_scroll_refresh(
    scroll: &gtk::ScrolledWindow,
    scrolling: Rc<Cell<bool>>,
    on_refresh: impl Fn() + 'static,
) {
    let settle = Rc::new(ScrollSettle {
        scroll: scroll.downgrade(),
        scrolling,
        last_scroll: Cell::new(std::time::Instant::now()),
        started: Cell::new(0),
        armed: Cell::new(false),
        on_refresh: Box::new(on_refresh),
    });
    for adjustment in [scroll.vadjustment(), scroll.hadjustment()] {
        let settle = settle.clone();
        adjustment.connect_value_changed(move |_| {
            settle.scrolling.set(true);
            settle.last_scroll.set(std::time::Instant::now());
            if !settle.armed.replace(true) {
                settle.started.set(glib::monotonic_time());
                ScrollSettle::arm(&settle, settle.delay());
            }
        });
    }
}

struct ScrollSettle {
    scroll: glib::WeakRef<gtk::ScrolledWindow>,
    scrolling: Rc<Cell<bool>>,
    last_scroll: Cell<std::time::Instant>,
    started: Cell<i64>,
    armed: Cell<bool>,
    on_refresh: Box<dyn Fn()>,
}

impl ScrollSettle {
    const DELAY: std::time::Duration = std::time::Duration::from_millis(25);
    const MINIMUM_FRAMES: f64 = 2.0;

    fn delay(&self) -> std::time::Duration {
        let frames = self
            .scroll
            .upgrade()
            .and_then(|scroll| scroll.frame_clock())
            // fps() averages the whole frame history, so an idle gap before this scroll would
            // stretch the delay.
            .filter(|clock| {
                clock
                    .timings(clock.history_start())
                    .is_some_and(|timings| timings.frame_time() >= self.started.get())
            })
            .map(|clock| clock.fps())
            .filter(|fps| *fps > 0.0)
            .map_or(std::time::Duration::ZERO, |fps| {
                std::time::Duration::from_secs_f64(Self::MINIMUM_FRAMES / fps)
            });
        Self::DELAY.max(frames)
    }

    fn arm(settle: &Rc<Self>, delay: std::time::Duration) {
        let settle = Rc::downgrade(settle);
        glib::timeout_add_local_once(delay, move || {
            let Some(settle) = settle.upgrade() else {
                return;
            };
            let still = settle.last_scroll.get().elapsed();
            let delay = settle.delay();
            if still < delay {
                Self::arm(&settle, delay - still);
                return;
            }
            settle.armed.set(false);
            settle.scrolling.set(false);
            if settle.scroll.upgrade().is_some() {
                (settle.on_refresh)();
            }
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
    pane.search
        .set_icons_max_columns(density_icons_columns(density));
    if let Some(loading) = pane
        .stack
        .child_by_name(super::loading_skeleton::LOADING_PAGE)
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
    if density.px() < BrowserDensity::AIRY_PX {
        20
    } else {
        16
    }
}

/// Spacing for a Custom slider position, extrapolated linearly through the static
/// Compact and Airy rules in `style.css` so the slider has no jump at either preset.
pub(crate) fn browser_density_custom_css(px: u8) -> String {
    let step = (f64::from(px) - f64::from(BrowserDensity::COMPACT_PX))
        / f64::from(BrowserDensity::AIRY_PX - BrowserDensity::COMPACT_PX);
    let between = |compact: f64, airy: f64| (compact + (airy - compact) * step).max(0.0).round();
    let row_margin = px;
    let row_content_margin = px.min(4);
    let row_min_height = between(26.0, 42.0);
    let grid_padding = between(0.0, 5.0);
    let child_margin = between(1.0, 3.0);
    let card_vertical = between(2.0, 5.0);
    let card_horizontal = between(4.0, 7.0);
    format!(
        ".density-custom .file-row, .mode-list.density-custom .list-row {{ min-height: {row_min_height}px; }}\n\
         .density-custom .file-row > *, .mode-list.density-custom .list-row > * {{ margin-top: {row_margin}px; margin-bottom: {row_margin}px; }}\n\
         .density-custom .file-row > .file-row-content {{ margin-top: {row_content_margin}px; margin-bottom: {row_content_margin}px; }}\n\
         .mode-icons.density-custom .file-icons {{ padding: {grid_padding}px; }}\n\
         .mode-icons.density-custom .file-icons > child {{ margin: {child_margin}px; }}\n\
         .mode-icons.density-custom .icons-card {{ padding: {card_vertical}px {card_horizontal}px; }}\n"
    )
}

fn find_heading_cell(headings: &gtk::Box, col: ListColumnId) -> Option<gtk::Widget> {
    let class = col.css_class();
    let mut current = headings.first_child();
    while let Some(child) = current {
        if child.has_css_class(class) {
            return Some(child);
        }
        current = child.next_sibling();
    }
    None
}

fn find_column_at_x(
    headings: &gtk::Box,
    x: f64,
    columns: &ListColumnLayout,
) -> Option<(ListColumnId, bool)> {
    let manager = super::preferences::PreferenceManager::shared();
    let visible_cols = manager.list_visible_columns();

    for &col in &visible_cols {
        if let Some(cell) = find_heading_cell(headings, col)
            && let Some(bounds) = cell.compute_bounds(headings)
            && bounds.width() > 0.0
        {
            let left = f64::from(bounds.x());
            let right = left + f64::from(bounds.width());
            if x >= left && x <= right {
                let is_resize = x >= (right - 8.0);
                return Some((col, is_resize));
            }
        }
    }

    let mut x_acc = 0.0;
    for col in visible_cols {
        let w = f64::from(columns.widths[col.index()].get());
        let left = x_acc;
        let right = x_acc + w;
        if x >= left && x <= right {
            let is_resize = x >= (right - 8.0);
            return Some((col, is_resize));
        }
        x_acc += w;
    }
    None
}

fn column_midpoint(
    headings: &gtk::Box,
    col: ListColumnId,
    columns: &ListColumnLayout,
) -> Option<f64> {
    if let Some(cell) = find_heading_cell(headings, col)
        && let Some(bounds) = cell.compute_bounds(headings)
        && bounds.width() > 0.0
    {
        return Some(f64::from(bounds.x()) + (f64::from(bounds.width()) / 2.0));
    }
    let manager = super::preferences::PreferenceManager::shared();
    let mut x_acc = 0.0;
    for c in manager.list_visible_columns() {
        let w = f64::from(columns.widths[c.index()].get());
        if c == col {
            return Some(x_acc + w / 2.0);
        }
        x_acc += w;
    }
    None
}

#[derive(Clone, Copy)]
struct ListColumnDrag {
    column: ListColumnId,
    origin: (f64, f64),
    active: bool,
}

fn column_drop_site(
    headings: &gtk::Box,
    columns: &ListColumnLayout,
    drag: ListColumnDrag,
    offset: (f64, f64),
) -> Option<(ListColumnId, bool)> {
    let (x, y) = (drag.origin.0 + offset.0, drag.origin.1 + offset.1);
    if x < 0.0 || x > f64::from(headings.width()) || y < 0.0 || y > f64::from(headings.height()) {
        return None;
    }
    let manager = super::preferences::PreferenceManager::shared();
    let mut visible = manager.list_visible_columns();
    let original = visible.iter().position(|column| *column == drag.column)?;
    visible.remove(original);
    for (position, column) in visible.iter().copied().enumerate() {
        if x < column_midpoint(headings, column, columns)? {
            return (position != original).then_some((column, false));
        }
    }
    let last = visible.last().copied()?;
    (visible.len() != original).then_some((last, true))
}

fn column_drop_feedback(headings: &gtk::Box, destination: Option<(ListColumnId, bool)>) {
    for column in ListColumnId::ALL {
        let Some(cell) = find_heading_cell(headings, column) else {
            continue;
        };
        cell.remove_css_class("reorder-before");
        cell.remove_css_class("reorder-after");
        if let Some((target, after)) = destination
            && target == column
        {
            cell.add_css_class(if after {
                "reorder-after"
            } else {
                "reorder-before"
            });
        }
    }
}

fn list_headings(
    browser: &Rc<Browser>,
    depth: usize,
    columns: ListColumnLayout,
    bound_items: &Rc<RefCell<Vec<BoundModeItem>>>,
) -> (gtk::Box, ListSorting) {
    let headings = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    headings.add_css_class("list-headings");
    let preferences = browser.column_preferences(depth).unwrap_or_default();
    let sorting = Rc::new(HeadingSort {
        current: Cell::new((preferences.sort_key, preferences.sort_direction)),
        arrows: RefCell::default(),
    });

    let manager = super::preferences::PreferenceManager::shared();
    let order = manager.list_column_order();
    let hidden = manager.list_hidden_columns();

    for col in order {
        let index = col.index();
        let (text, key) = match col {
            ListColumnId::Name => ("Name", Some(SortKey::Name)),
            ListColumnId::Mode => ("Mode", None),
            ListColumnId::Size => ("Size", Some(SortKey::Size)),
            ListColumnId::Type => ("Type", Some(SortKey::Type)),
            ListColumnId::Modified => ("Modified", Some(SortKey::Modified)),
        };
        let is_visible = !hidden.contains(&col) || col == ListColumnId::Name;
        let width = columns.widths[index].get();
        let cell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        cell.add_css_class("list-heading-cell");
        cell.add_css_class(col.css_class());
        if !is_visible {
            cell.set_visible(false);
        }
        register_list_column_cell(&columns, index, &cell);

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 5);
        let label = gtk::Label::new(Some(&crate::i18n::tr(text)));
        label.set_xalign(0.0);
        label.set_hexpand(true);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.set_max_width_chars(1);
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
            button.set_cursor_from_name(Some("pointer"));
            let weak_browser = Rc::downgrade(browser);
            let sorting_for_click = Rc::downgrade(&sorting);
            button.connect_clicked(move |_| {
                let Some(sorting) = sorting_for_click.upgrade() else {
                    return;
                };
                let (current_key, current_direction) = sorting.get();
                let direction = if current_key == key {
                    match current_direction {
                        SortDirection::Ascending => SortDirection::Descending,
                        SortDirection::Descending => SortDirection::Ascending,
                    }
                } else {
                    SortDirection::Ascending
                };
                sorting.show(key, direction);
                if let Some(browser) = weak_browser.upgrade() {
                    browser.set_sort(depth, key, direction);
                }
            });
            sorting.arrows.borrow_mut().push((key, arrow));
        }

        let button_overlay = gtk::Overlay::new();
        button_overlay.set_child(Some(&button));
        button_overlay.set_hexpand(true);
        button_overlay.add_overlay(&column_resize_handle(columns.clone(), index, width, &cell));
        cell.append(&button_overlay);
        headings.append(&cell);
    }

    let drag = gtk::GestureDrag::new();
    drag.set_name(Some("column-drag"));
    drag.set_button(1);
    drag.set_propagation_phase(gtk::PropagationPhase::Capture);

    let drag_state = columns.drag.clone();
    let headings_weak = headings.downgrade();
    let finish_drag = Rc::new({
        let headings_weak = headings_weak.clone();
        let columns = columns.clone();
        let bound_items = bound_items.clone();
        move |released: Option<(f64, f64)>| {
            if let Some(headings) = headings_weak.upgrade() {
                columns.finish_drag(&headings, &bound_items, released);
            } else {
                columns.drag.take();
            }
        }
    });

    drag.connect_drag_begin({
        let finish_drag = finish_drag.clone();
        let drag_state = drag_state.clone();
        let headings_weak = headings_weak.clone();
        let columns = columns.clone();
        move |gesture, start_x, start_y| {
            finish_drag(None);
            let Some(headings) = headings_weak.upgrade() else {
                return;
            };
            let Some((column, is_resize)) = find_column_at_x(&headings, start_x, &columns) else {
                return;
            };
            if is_resize {
                gesture.set_state(gtk::EventSequenceState::Denied);
                return;
            }
            drag_state.set(Some(ListColumnDrag {
                column,
                origin: (start_x, start_y),
                active: false,
            }));
        }
    });

    drag.connect_drag_update({
        let headings_weak = headings_weak.clone();
        let columns = columns.clone();
        move |gesture, offset_x, offset_y| {
            let Some(mut drag) = drag_state.get() else {
                return;
            };
            let Some(headings) = headings_weak.upgrade() else {
                return;
            };
            if !drag.active {
                if offset_x.abs() < 10.0 {
                    return;
                }
                drag.active = true;
                drag_state.set(Some(drag));
                gesture.set_state(gtk::EventSequenceState::Claimed);
                headings.set_cursor_from_name(Some("grabbing"));
                headings.add_css_class("column-reordering");
            }
            column_drop_feedback(
                &headings,
                column_drop_site(&headings, &columns, drag, (offset_x, offset_y)),
            );
        }
    });

    drag.connect_drag_end({
        let finish_drag = finish_drag.clone();
        move |_, x, y| finish_drag(Some((x, y)))
    });
    drag.connect_cancel({
        let finish_drag = finish_drag.clone();
        move |_, _| finish_drag(None)
    });
    headings.connect_unmap(move |_| finish_drag(None));

    headings.add_controller(drag);
    let scaled_columns = columns.clone();
    super::preferences::PreferenceManager::shared().bind_interface_scale(
        &headings,
        move |_, scale| {
            let ratio = scale / scaled_columns.scale.replace(scale);
            for (index, width) in scaled_columns.widths.iter().enumerate() {
                let scaled = (f64::from(width.get()) * ratio).round() as i32;
                width.set(scaled);
                scaled_columns.cells[index].borrow_mut().retain(|weak| {
                    let Some(cell) = weak.upgrade() else {
                        return false;
                    };
                    cell.set_width_request(scaled);
                    true
                });
            }
        },
    );
    (headings, sorting)
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

fn column_resize_handle(
    columns: ListColumnLayout,
    index: usize,
    initial_width: i32,
    heading: &gtk::Box,
) -> gtk::Box {
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
    let heading = heading.downgrade();
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
                .map(|widget| list_cell_content_width(&widget))
                .max()
                .unwrap_or(initial_width);
            set_list_column_width(
                &columns_for_autofit,
                index,
                list_column_width(index, natural),
            );
            columns_for_autofit.remember();
            gesture.set_state(gtk::EventSequenceState::Denied);
            return;
        }
        // Registered cells also include unmapped loading placeholders with stale allocations.
        let width = heading
            .upgrade()
            .and_then(|widget| widget.compute_bounds(&widget))
            .map_or(initial_width, |bounds| bounds.width().round() as i32);
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
    resize.connect_drag_end(move |_, _, _| columns.remember());
    handle.add_controller(resize);
    handle
}

fn list_column_width(index: usize, width: i32) -> i32 {
    width.max(LIST_COLUMN_MIN_WIDTHS[index])
}

/// Registered cells carry the column's fixed width and their labels cap natural
/// width at one character, so autofit must measure with both constraints lifted.
fn list_cell_content_width(cell: &gtk::Widget) -> i32 {
    let cell_request = cell.width_request();
    if cell_request >= 0 {
        cell.set_width_request(-1);
    }
    let mut labels = Vec::new();
    let mut stack = vec![cell.clone()];
    while let Some(widget) = stack.pop() {
        if let Some(label) = widget.downcast_ref::<gtk::Label>()
            && label.max_width_chars() >= 0
        {
            labels.push((label.clone(), label.max_width_chars()));
            label.set_max_width_chars(-1);
        }
        let mut child = widget.first_child();
        while let Some(c) = child {
            stack.push(c.clone());
            child = c.next_sibling();
        }
    }
    let (_, natural, _, _) = cell.measure(gtk::Orientation::Horizontal, -1);
    if cell_request >= 0 {
        cell.set_width_request(cell_request);
    }
    for (label, chars) in labels {
        label.set_max_width_chars(chars);
    }
    natural
}

fn list_navigation(browser: &Rc<Browser>) -> gtk::Box {
    use super::keys::{
        Keys,
        bindings::{HISTORY_BACK, HISTORY_FORWARD, PARENT_FOLDER},
    };
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    actions.add_css_class("list-navigation");
    for (icon, tooltip, binding, action, available) in [
        (
            crate::assets::icons::ARROW_LEFT,
            "Back (%{keys})",
            HISTORY_BACK,
            Browser::back as fn(&Rc<Browser>),
            browser.can_go_back(),
        ),
        (
            crate::assets::icons::ARROW_RIGHT,
            "Forward (%{keys})",
            HISTORY_FORWARD,
            Browser::forward as fn(&Rc<Browser>),
            browser.can_go_forward(),
        ),
        (
            crate::assets::icons::ARROW_UP,
            "Parent folder (%{keys})",
            PARENT_FOLDER,
            Browser::parent as fn(&Rc<Browser>),
            browser.can_go_parent(),
        ),
    ] {
        let button = gtk::Button::builder().sensitive(available).build();
        let keys = Keys::One(binding.alternate(0).shown());
        super::shortcut_reference::follow_tooltip(&button, tooltip, keys);
        super::shortcut_reference::follow_label(&button, tooltip, keys);
        button.set_child(Some(&crate::assets::chrome_icon(icon)));
        button.add_css_class("list-navigation-button");
        button.set_cursor_from_name(Some("pointer"));
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
    clipboard_marks: Rc<RefCell<ClipboardMarks>>,
    options: ListOptions,
    depth: usize,
    title: &str,
) -> Pane {
    let location = browser.location_at(depth);
    let search_selection_handlers = options.search_selection_handlers.clone();
    let navigation = list_navigation(&browser);
    let columns = ListColumnLayout::new(&browser);
    let bound_items = Rc::new(RefCell::new(Vec::new()));
    let (headings, sorting) = list_headings(&browser, depth, columns.clone(), &bound_items);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    actions.add_css_class("icons-header-actions");
    let leftovers = super::browser::leftovers_button(&browser, depth);
    actions.append(&leftovers);
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
    let camera_photos = location
        .as_ref()
        .is_some_and(|location| location.is_camera_photo_root());
    let recent = location
        .as_ref()
        .is_some_and(|location| location.is_recent_root());
    if recent {
        actions.append(&super::browser::column_sort_direction_toggle(
            &browser, depth,
        ));
    }
    if camera_photos || recent {
        actions.append(&super::browser::column_sort_menu(&browser, depth));
    }

    let columns_popover =
        column_options::create_column_options_popover(&headings, &columns, &bound_items);
    let columns_menu = gtk::MenuButton::builder().popover(&columns_popover).build();
    super::accessibility::set_tooltip_and_label(&columns_menu, &crate::i18n::tr("Columns"));
    columns_menu.set_child(Some(&crate::assets::chrome_icon(
        crate::assets::icons::COLUMNS,
    )));
    crate::ui::controls::pane_header_action(&columns_menu);
    crate::ui::tenxer_mode::hide_while_enabled(&columns_menu);
    actions.append(&columns_menu);

    let (filter_entry, filter_revealer, filter_button) = filter_controls("Filter list (%{keys})");
    actions.append(&filter_button);
    let (
        shell,
        header,
        content,
        model,
        stack,
        status,
        administrator,
        spinner,
        truncated_hint,
        not_responding,
    ) = pane_base(
        title,
        BrowserMode::List,
        "list-pane",
        &list_loading_skeleton(&columns),
        Some(navigation.upcast()),
        Some(actions.upcast()),
    );
    if location.as_ref().is_some_and(Location::is_administrator) {
        shell.insert_child_after(
            &super::browser::administrator_notice(),
            Some(&not_responding),
        );
    }
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
    // Recursive-filter results replace a native listing; hiding its rows too
    // would drop their selection out of sight.
    let results_replace_listing = Rc::new(Cell::new(false));
    let enter_target = in_place_enter_target(&filter_entry, &browser, depth);
    super::browser::bind_listing_filter(
        &filter_entry,
        &filter,
        &filter_query,
        results_replace_listing.clone(),
        options.state.clone(),
        enter_target.clone(),
    );
    let view_model =
        gtk::SortListModel::new(Some(filtered_model.clone()), None::<gtk::CustomSorter>);
    let type_group_sorter = options
        .group_by_type
        .then(|| pane_type_group_sorter(&browser, depth));
    if let Some(sorter) = &type_group_sorter {
        view_model.set_sorter(Some(&sorter.sorter));
        view_model.set_section_sorter(Some(&sorter.sorter));
    }
    let view_model_object = view_model.clone().upcast::<gio::ListModel>();
    let view_index = SourceIndexMap::watch(&view_model_object);
    let selection = gtk::MultiSelection::new(Some(view_model.clone()));
    let syncing_selection = Rc::new(Cell::new(false));
    let sections: Rc<RefCell<Vec<PaneSection>>> = Rc::new(RefCell::new(Vec::new()));

    {
        let weak_headings = headings.downgrade();
        let weak_columns = columns.clone();
        let weak_bound_items = bound_items.clone();
        let click = gtk::GestureClick::new();
        click.set_button(3);
        click.connect_pressed(move |gesture, _, x, y| {
            let Some(headings) = weak_headings.upgrade() else {
                return;
            };
            column_options::open_column_options_popover(
                &headings,
                &headings,
                &weak_columns,
                &weak_bound_items,
                Some((x, y)),
            );
            gesture.set_state(gtk::EventSequenceState::Claimed);
        });
        headings.add_controller(click);
    }
    let scrolling = Rc::new(Cell::new(false));
    let factory = ListFactory {
        browser: Rc::downgrade(&browser),
        depth,
        positions: PanePositions {
            index: source_index.clone(),
            view: view_model_object.clone(),
        },
        selection: selection.clone(),
        syncing: syncing_selection.clone(),
        previews: click_options.previews.clone(),
        activation: click_options.activation.clone(),
        transfers: transfer_handler.clone(),
        marks: clipboard_marks.clone(),
        columns: columns.clone(),
        scrolling: scrolling.clone(),
        bound_items: bound_items.clone(),
        state: options.state.clone(),
        filter_query: filter_query.clone(),
        enter_target: enter_target.clone(),
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
    view.set_tab_behavior(gtk::ListTabBehavior::Item);
    install_tab_landing(view.upcast_ref(), options.state.clone());
    if let Some(destination) = browser.location_at(depth) {
        install_mode_directory_drop_target(&view, destination, transfer_handler.clone());
    }
    let weak_browser = Rc::downgrade(&browser);
    let weak_state_for_activate = options.state.clone();
    let source_index_for_activation = source_index.clone();
    let view_model_for_activation = view_model_object.clone();
    view.connect_activate(move |_, position| {
        if let Some(state) = weak_state_for_activate.as_ref().and_then(Weak::upgrade) {
            state.cancel_click_rename();
        }
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
    let mut section = PaneSection {
        view: view.clone().upcast(),
        view_index,
        view_model: view_model_object,
        selection,
        bound_items: bound_items.clone(),
        syncing: syncing_selection,
        visit: bound_item_visitor(bound_items),
        item_context_trigger: Rc::new(|_, _| {}),
    };
    let browser_for_filter_activate = Rc::downgrade(&browser);
    let selection_for_filter_activate = section.selection.clone();
    let view_for_filter_activate = section.view_model.clone();
    let source_index_for_filter_activate = source_index.clone();
    let filter_query_for_activate = filter_query.clone();
    let enter_for_activate = enter_target.clone();
    filter_entry.connect_activate(move |_| {
        if filter_query_for_activate.borrow().trim().is_empty() {
            return;
        }
        activate_filtered_item(
            &browser_for_filter_activate,
            filter_activation_position(enter_for_activate.as_ref(), &selection_for_filter_activate),
            &view_for_filter_activate,
            &source_index_for_filter_activate,
            depth,
        );
    });
    if let Some(enter_target) = &enter_target {
        connect_in_place_down(
            &filter_entry,
            enter_target,
            &section.view,
            options.state.clone(),
        );
    }
    connect_selection(
        &section,
        Rc::downgrade(&sections),
        &browser,
        depth,
        source_index.clone(),
        click_options.multiple_selection.clone(),
    );
    if let Some(state) = options.state.as_ref().and_then(Weak::upgrade) {
        section.item_context_trigger = install_section_context_menu(
            &state,
            &section,
            Rc::downgrade(&sections),
            &source_index,
            depth,
        );
    }
    sections.borrow_mut().push(section.clone());
    let scroll = gtk::ScrolledWindow::builder()
        .child(&view)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();
    scroll.add_css_class("browser-listing-scroll");
    scroll.add_css_class("list-listing-scroll");
    if let Some(enter_target) = &enter_target {
        attach_in_place(enter_target, &section, &scroll);
    }
    let browser_for_settle = Rc::downgrade(&browser);
    let source_index_for_settle = source_index.clone();
    let sections_for_settle = Rc::downgrade(&sections);
    let marks_for_settle = clipboard_marks.clone();
    install_scroll_refresh(&scroll, scrolling, move || {
        let Some(browser) = browser_for_settle.upgrade() else {
            return;
        };
        let Some(sections) = sections_for_settle.upgrade() else {
            return;
        };
        let marks = marks_for_settle.borrow();
        for section in sections.borrow().iter() {
            refresh_list_section(&browser, depth, &source_index_for_settle, section, &marks);
        }
    });
    let table = gtk::Box::new(gtk::Orientation::Vertical, 0);
    table.set_vexpand(true);
    table.append(&headings);
    let targets: super::marquee::MarqueeTargets = Rc::new(RefCell::new(Vec::new()));
    let (collection, marquee) = collection_with_marquee(
        view.upcast_ref(),
        scroll,
        targets.clone(),
        true,
        click_options.multiple_selection.clone(),
    );
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
    table_scroll.add_css_class("mode-scroll");
    if let Some(viewport) = table_scroll.child().and_downcast::<gtk::Viewport>() {
        // The outer viewport must not horizontally reveal oversized metadata rows; the inner
        // ListView still reveals focused rows vertically.
        viewport.set_scroll_to_focus(false);
    }
    let browser_for_search = browser.clone();
    let search = super::inline_search::wrap(
        &table_scroll,
        &filter_entry,
        move || {
            browser_for_search
                .location_at(depth)
                .and_then(|location| location.native_path().map(std::path::Path::to_path_buf))
        },
        &browser,
        search_collection_options(
            super::inline_search::SearchPresentation::Rows,
            &browser,
            click_options.previews.clone(),
            click_options.activation.clone(),
            click_options.multiple_selection.clone(),
            search_selection_handlers,
            options.search_context_state.clone(),
        ),
    );
    results_replace_listing.set(search.replaces_listing());
    content.append(&search.widget);
    let pane = Pane {
        depth,
        location,
        group_by_type: options.group_by_type,
        type_group_sorter,
        sorting: Some(sorting),
        shell,
        header,
        model,
        source_index,
        filter_model: Some(filtered_model),
        section,
        sections,
        icons: None,
        thumbnail_scale: None,
        targets,
        detached: Rc::new(Cell::new(false)),
        loading: super::loading_skeleton::DelayedLoading::new(&stack),
        stack,
        status,
        administrator,
        spinner,
        truncated_hint,
        not_responding,
        marquee,
        search,
        filter_entry: Some(filter_entry),
        filter_button: Some(filter_button),
        empty_trash_button: is_trash.then_some(empty_trash),
        leftovers_button: leftovers,
        sort_direction_button: None,
        show_hidden,
        filter: filter_for_pane,
        filter_query,
        folder_context_trigger: Rc::new(|_, _| {}),
        list_headings: Some(headings),
        list_columns: Some(columns),
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
        let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
        card.add_css_class("icons-card");
        card.set_halign(gtk::Align::Fill);
        ensure_icons_card_slot(&card, thumbnail_size);
        let slot = icons_card_icon_slot(thumbnail_size);
        let icon = block(slot, slot);
        icon.set_halign(gtk::Align::Center);
        let padding = super::icons_cell::ICONS_CARD_ICON_PADDING;
        icon.set_margin_top(padding);
        icon.set_margin_bottom(padding);
        icon.set_margin_start(padding);
        icon.set_margin_end(padding);
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
    let manager = super::preferences::PreferenceManager::shared();
    let order = manager.list_column_order();
    let hidden = manager.list_hidden_columns();

    let heading_width_for_col = |col: ListColumnId| match col {
        ListColumnId::Name => 40,
        ListColumnId::Mode => 36,
        ListColumnId::Size => 28,
        ListColumnId::Type => 30,
        ListColumnId::Modified => 58,
    };

    for &col in &order {
        let index = col.index();
        let width = heading_width_for_col(col);
        let cell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        cell.add_css_class("list-heading-cell");
        let is_visible = !hidden.contains(&col) || col == ListColumnId::Name;
        if !is_visible {
            cell.set_visible(false);
        }
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
        let row_width_for_col = |col: ListColumnId| match col {
            ListColumnId::Name => name_width(index).min(92),
            ListColumnId::Mode => 60,
            ListColumnId::Size => 38,
            ListColumnId::Type => 56,
            ListColumnId::Modified => 94,
        };
        for &col in &order {
            let column = col.index();
            let width = row_width_for_col(col);
            let cell = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            cell.add_css_class("list-metadata-cell");
            let is_visible = !hidden.contains(&col) || col == ListColumnId::Name;
            if !is_visible {
                cell.set_visible(false);
            }
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
    gtk::Button,
    gtk::Spinner,
    gtk::Image,
    gtk::Box,
) {
    let shell = super::accessibility::pane_box();
    shell.add_css_class(class);
    shell.set_hexpand(true);
    shell.set_vexpand(true);
    super::accessibility::describe_pane(&shell, title, mode);
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    header.add_css_class("mode-pane-header");
    super::tenxer_mode::hide_while_enabled(&header);
    let heading_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    heading_box.set_hexpand(true);
    heading_box.set_valign(gtk::Align::Center);
    let heading = gtk::Label::new(Some(title));
    heading.set_xalign(0.0);
    heading.set_hexpand(true);
    heading.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    heading.set_max_width_chars(1);
    crate::ui::accessibility::set_description(&heading, Some(title));
    let spinner = gtk::Spinner::new();
    spinner.set_valign(gtk::Align::Center);
    spinner.start();
    let truncated_hint = crate::assets::primary_icon(crate::assets::icons::TRIANGLE_ALERT, 16);
    crate::ui::accessibility::set_description(
        &truncated_hint,
        Some(&crate::i18n::tr(
            "This directory has more entries than could be loaded; showing a partial listing.",
        )),
    );
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
    let not_responding = super::browser::not_responding_notice();
    shell.append(&not_responding);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.set_hexpand(true);
    content.set_vexpand(true);
    let status = gtk::Label::new(Some(&crate::i18n::tr("This directory is empty")));
    status.add_css_class("status-message");
    status.set_wrap(true);
    let administrator = gtk::Button::with_label(&crate::i18n::tr("Open as Administrator"));
    administrator.add_css_class("retry-button");
    administrator.set_halign(gtk::Align::Center);
    administrator.set_visible(false);
    let status_page = gtk::Box::new(gtk::Orientation::Vertical, 8);
    status_page.set_valign(gtk::Align::Center);
    status_page.append(&status);
    status_page.append(&administrator);
    // Focusable only off the content page (see `DirectorySurface`); a group role keeps its name.
    let stack = gtk::Stack::builder()
        .hexpand(true)
        .vexpand(true)
        .accessible_role(gtk::AccessibleRole::Group)
        .build();
    stack.add_named(&content, Some(super::loading_skeleton::CONTENT_PAGE));
    stack.add_named(loading, Some(super::loading_skeleton::LOADING_PAGE));
    stack.add_named(&status_page, Some(STATUS_PAGE));
    shell.append(&stack);

    let model = gtk::StringList::new(&[]);
    (
        shell,
        header,
        content,
        model,
        stack,
        status,
        administrator,
        spinner,
        truncated_hint,
        not_responding,
    )
}

fn bound_edit(item: &gtk::ListItem) -> Option<super::collection_edit::EditWidgets> {
    super::collection_edit::item_edit(item)
}

fn install_edit_unbind(
    factory: &gtk::SignalListItemFactory,
    items: &Rc<RefCell<Vec<BoundModeItem>>>,
) {
    factory.connect_unbind(move |_, object| {
        if let Some(item) = object.downcast_ref::<gtk::ListItem>()
            && let Some(edit) = bound_edit(item)
        {
            edit.unbind();
        }
        super::thumbnail::cancel_list_item_thumbnails(object);
    });
    let teardown = items.clone();
    factory.connect_teardown(move |_, object| {
        super::collection_edit::forget_item_edit(object);
        teardown.borrow_mut().retain(|bound| {
            bound
                .item
                .upgrade()
                .is_some_and(|item| item.upcast_ref::<glib::Object>() != object)
        });
    });
}

fn register_bound_mode_item(
    items: &Rc<RefCell<Vec<BoundModeItem>>>,
    item: &gtk::ListItem,
    widget: &impl IsA<gtk::Widget>,
    rename_label: &impl IsA<gtk::Widget>,
    edit: super::collection_edit::EditWidgets,
) {
    let weak_item = glib::WeakRef::new();
    weak_item.set(Some(item));
    let weak_widget = glib::WeakRef::new();
    weak_widget.set(Some(widget.upcast_ref()));
    let weak_rename_label = glib::WeakRef::new();
    weak_rename_label.set(Some(rename_label.upcast_ref()));
    super::collection_edit::register_item_edit(item, &edit);
    items.borrow_mut().push(BoundModeItem {
        item: weak_item,
        widget: weak_widget,
        rename_label: weak_rename_label,
        edit,
    });
}

fn collection_with_marquee(
    view: &gtk::Widget,
    scroll: gtk::ScrolledWindow,
    targets: super::marquee::MarqueeTargets,
    list_rows: bool,
    multiple_selection: Rc<Cell<bool>>,
) -> (gtk::Overlay, super::marquee::Marquee) {
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&scroll));
    overlay.set_hexpand(true);
    overlay.set_vexpand(true);
    super::scrolling::install_autoscroll(&scroll, &overlay);

    let content: super::marquee::ItemPredicate = if list_rows {
        Rc::new(super::pointer::hits_list_item_content)
    } else {
        Rc::new(super::pointer::hits_item_content)
    };
    let is_item = super::marquee::item_content_predicate(targets.clone(), content);
    let marquee = super::marquee::install(super::marquee::MarqueeSetup {
        view: view.clone(),
        surface: scroll.clone().upcast(),
        scroll,
        overlay: overlay.clone(),
        targets: targets.clone(),
        is_item,
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
        allow_drag: multiple_selection,
    });
    (overlay, marquee)
}

fn set_icons_content_hover(card: &gtk::Box, x: f64, y: f64) {
    if super::pointer::hits_icon_card_content(card.upcast_ref(), x, y) {
        card.add_css_class("content-hover");
    } else {
        card.remove_css_class("content-hover");
    }
}

fn update_icons_content_hover(controller: &gtk::EventControllerMotion, x: f64, y: f64) {
    if let Some(card) = controller.widget().and_downcast::<gtk::Box>() {
        set_icons_content_hover(&card, x, y);
    }
}

fn install_icons_content_hover(card: &gtk::Box) {
    let motion = gtk::EventControllerMotion::new();
    motion.connect_enter(update_icons_content_hover);
    motion.connect_motion(update_icons_content_hover);
    motion.connect_leave(|controller| {
        if let Some(card) = controller.widget() {
            card.remove_css_class("content-hover");
        }
    });
    card.add_controller(motion);
}

fn install_folder_peek(
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
    if transfer_handler.borrow().is_none()
        || is_trash_location(&destination)
        || destination.is_recent_location()
    {
        return;
    }
    widget.add_css_class("file-drop-zone");
    let super::browser::PreparedFileDrop {
        target: drop,
        state: drop_state,
    } = super::browser::prepare_file_drop_target({
        let destination = destination.clone();
        move || Some(destination.clone())
    });
    let state_for_enter = drop_state.clone();
    drop.connect_enter(move |target, _, _| {
        super::browser::file_drop_action(target, &state_for_enter)
    });
    let state_for_motion = drop_state.clone();
    drop.connect_motion(move |target, _, _| {
        super::browser::file_drop_action(target, &state_for_motion)
    });
    drop.connect_drop(move |target, value, _, _| {
        let Some(sources) = super::browser::locations_from_file_list_value(value) else {
            return false;
        };
        let Some(handler) = transfer_handler.borrow().clone() else {
            return false;
        };
        super::browser::file_drop_commit(
            target,
            destination.clone(),
            sources,
            &drop_state,
            move |destination, sources, commit| handler(destination, sources, commit),
        );
        true
    });
    widget.add_controller(drop);
}

#[expect(
    clippy::too_many_arguments,
    reason = "drag setup wires every GTK signal it needs"
)]
fn install_list_drag_drop(
    row: &impl IsA<gtk::Widget>,
    item: &gtk::ListItem,
    browser: Weak<Browser>,
    transfer_handler: TransferHandlerSlot,
    depth: usize,
    position_map: Option<(SourceIndexMap, gio::ListModel)>,
    state: Option<Weak<super::browser::ViewState>>,
    drag_icon_and_content_click: (&gtk::GestureClick, bool, Rc<SlowClickRename>),
) {
    let (content_click, list_rows, intent) = drag_icon_and_content_click;
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
    let prepare_row = row.downgrade();
    drag.connect_prepare(move |source, x, y| {
        let prepare_row = prepare_row.upgrade()?;
        if prepare_row
            .pick(x, y, gtk::PickFlags::DEFAULT)
            .is_some_and(|target| crate::ui::focus_navigation::editable(&target))
            || (!intent.was_selected.get()
                && if list_rows {
                    !super::pointer::hits_list_item_content(&prepare_row, x, y)
                } else {
                    !super::pointer::hits_icon_card_content(&prepare_row, x, y)
                })
        {
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
        let entries = if browser.active_selection_contains(depth, position) {
            browser.selected_entries()
        } else {
            vec![entry]
        };
        if let Some((texture, hot_x, hot_y)) =
            super::browser::drag_preview_icon(&prepare_row, &entries)
        {
            source.set_icon(Some(&texture), hot_x, hot_y);
        }
        super::browser::file_drag_content(source, &entries)
    });
    let dragged_row = row.downgrade();
    let weak_state_for_begin = state.clone();
    drag.connect_drag_begin(move |_, _| {
        if let Some(row) = dragged_row.upgrade() {
            row.add_css_class("dragging");
        }
        if let Some(state) = weak_state_for_begin.as_ref().and_then(Weak::upgrade) {
            state.cancel_peek();
        }
    });
    let dragged_row = row.downgrade();
    let weak_state_for_end = state;
    drag.connect_drag_end(move |source, _, _| {
        super::browser::clear_active_drag();
        source.set_actions(gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE);
        if let Some(row) = dragged_row.upgrade() {
            row.remove_css_class("dragging");
        }
        if let Some(state) = weak_state_for_end.as_ref().and_then(Weak::upgrade) {
            state.cancel_peek();
        }
    });
    row.add_controller(drag.clone());
    // Grouping requires both gestures already attached; claiming the modifier-click
    // on content must not deny this drag before it reaches the threshold.
    drag.group_with(content_click);

    let dest_for_row = {
        let dropped_item = item.downgrade();
        let browser_for_dest = browser.clone();
        let map_for_dest = position_map.clone();
        move || {
            let browser = browser_for_dest.upgrade()?;
            let dropped_item = dropped_item.upgrade()?;
            let position = dropped_item.position();
            let position = map_for_dest.as_ref().map_or(
                (position != gtk::INVALID_LIST_POSITION).then_some(position as usize),
                |(map, view)| source_position_for_view(map, Some(view), position),
            );
            position
                .and_then(|position| browser.entry_at(depth, position))
                .filter(FileEntry::is_directory)
                .map(|entry| entry.location)
        }
    };
    let super::browser::PreparedFileDrop {
        target: drop,
        state: drop_state,
    } = super::browser::prepare_file_drop_target(dest_for_row);
    let spring_navigate: Rc<dyn Fn(Location)> = {
        let browser = browser.clone();
        Rc::new(move |location| {
            if let Some(browser) = browser.upgrade() {
                browser.navigate_location(location, false);
            }
        })
    };
    let highlighted_row = row.downgrade();
    let state_for_enter = drop_state.clone();
    let navigate_for_enter = spring_navigate.clone();
    drop.connect_enter(move |target, _, _| {
        let action = super::browser::file_drop_action(target, &state_for_enter);
        let hovered = super::browser::file_drag_hover_target(&state_for_enter, target).is_some();
        if let Some(row) = highlighted_row.upgrade() {
            if hovered {
                row.add_css_class("drop-destination");
            } else {
                row.remove_css_class("drop-destination");
            }
        }
        super::browser::arm_spring_load_navigation(&state_for_enter, target, &navigate_for_enter);
        action
    });
    let highlighted_row = row.downgrade();
    let state_for_motion = drop_state.clone();
    let navigate_for_motion = spring_navigate.clone();
    drop.connect_motion(move |target, _, _| {
        let action = super::browser::file_drop_action(target, &state_for_motion);
        let hovered = super::browser::file_drag_hover_target(&state_for_motion, target).is_some();
        if let Some(row) = highlighted_row.upgrade() {
            if hovered {
                row.add_css_class("drop-destination");
            } else {
                row.remove_css_class("drop-destination");
            }
        }
        super::browser::arm_spring_load_navigation(&state_for_motion, target, &navigate_for_motion);
        action
    });
    let highlighted_row = row.downgrade();
    let state_for_value = drop_state.clone();
    let navigate_for_value = spring_navigate.clone();
    drop.connect_value_notify(move |target| {
        if target.current_drop().is_none() {
            return;
        }
        super::browser::file_drop_action(target, &state_for_value);
        let hovered = super::browser::file_drag_hover_target(&state_for_value, target).is_some();
        if let Some(row) = highlighted_row.upgrade() {
            if hovered {
                row.add_css_class("drop-destination");
            } else {
                row.remove_css_class("drop-destination");
            }
        }
        super::browser::arm_spring_load_navigation(&state_for_value, target, &navigate_for_value);
    });
    let highlighted_row = row.downgrade();
    let state_for_leave = drop_state.clone();
    drop.connect_leave(move |_| {
        state_for_leave.cancel_spring_load_navigation();
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
    let dropped_row = row.downgrade();
    drop.connect_drop(move |target, value, _, _| {
        if let Some(row) = dropped_row.upgrade() {
            row.remove_css_class("drop-destination");
        }
        drop_state.cancel_spring_load_navigation();
        let Some(destination) = drop_state.destination() else {
            return false;
        };
        let Some(sources) = super::browser::locations_from_file_list_value(value) else {
            return false;
        };
        let Some(handler) = transfer_handler.borrow().clone() else {
            return false;
        };
        super::browser::file_drop_commit(
            target,
            destination,
            sources,
            &drop_state,
            move |destination, sources, commit| handler(destination, sources, commit),
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

fn open_entry_in_new(row: &glib::WeakRef<gtk::Widget>, location: &Location, action: &str) {
    let Some(widget) = row.upgrade() else {
        return;
    };
    let uri = crate::adapters::gio_file_for_location(location)
        .uri()
        .to_variant();
    if let Err(error) = widget.activate_action(action, Some(&uri)) {
        tracing::warn!(%error, action, "open-in-new location action unavailable");
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "mode-specific click setup keeps these inputs explicit"
)]
fn install_modified_selection_click(
    widget: &impl IsA<gtk::Widget>,
    item: &gtk::ListItem,
    selection: gtk::MultiSelection,
    syncing: Rc<Cell<bool>>,
    browser: Weak<Browser>,
    weak_state: Weak<super::browser::ViewState>,
    depth: usize,
    positions: PanePositions,
    slow_click: Rc<SlowClickRename>,
) -> gtk::GestureClick {
    let click = gtk::GestureClick::new();
    click.set_button(1);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let sequence = super::collection_interaction::PointerSequence::default();
    sequence.install(&click);
    super::collection_interaction::install_row_middle_click(widget, item, {
        let browser = browser.clone();
        let row = widget.upcast_ref::<gtk::Widget>().downgrade();
        let positions = positions.clone();
        move |view_position, modifiers| {
            let Some(browser) = browser.upgrade() else {
                return;
            };
            let Some(source_position) = positions.source_position(view_position) else {
                return;
            };
            let Some(entry) = browser.entry_at(depth, source_position) else {
                return;
            };
            let control = modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK);
            let shift = modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK);
            match super::collection_interaction::middle_click_action(
                entry.is_directory(),
                control,
                shift,
            ) {
                super::collection_interaction::MiddleClickAction::OpenTab => {
                    open_entry_in_new(&row, &entry.location, "win.open-tab-at");
                }
                super::collection_interaction::MiddleClickAction::OpenWindow => {
                    open_entry_in_new(&row, &entry.location, "win.open-window-at");
                }
                super::collection_interaction::MiddleClickAction::RevealParent => {
                    if let Some(parent) = entry.location.parent() {
                        browser.reveal_locations(parent, vec![entry.location]);
                    }
                }
            }
        }
    });
    let item = item.downgrade();
    let weak_state_for_pressed = weak_state.clone();
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
        if let Some(state) = weak_state_for_pressed.upgrade() {
            state.cancel_click_rename();
        }
        let modifiers = gesture.current_event_state();
        let control = modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK);
        let shift = modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK);
        sequence.press(modifiers);
        let selected_before = selection.is_selected(position);
        let selected_count_before = selection.selection().size();
        slow_click.was_selected.set(selected_before);
        slow_click.selected_count_before.set(selected_count_before);
        let change = super::collection_interaction::pointer_selection(
            &selection.selection(),
            position,
            browser
                .selection_anchor_position(depth)
                .and_then(|anchor| positions.view_position(anchor)),
            true,
            modifiers,
        );
        if !shift && let Some(anchor) = change.anchor {
            anchor_at(&browser, depth, &positions, anchor);
        }
        if control
            && !shift
            && let Some(row) = gesture.widget()
        {
            move_range_origin(&row, &selection, &syncing, position);
        }
        selection.set_selection(
            &change.selected,
            &gtk::Bitset::new_range(0, selection.n_items()),
        );
        if !control && !shift {
            return;
        }
        if let Some(widget) = gesture.widget()
            && super::pointer::hits_icon_card_content(&widget, x, y)
            && let Some(item_widget) = widget.parent()
        {
            item_widget.grab_focus();
        }
    });
    let weak_state_for_cancel = weak_state.clone();
    click.connect_cancel(move |_, _| {
        if let Some(state) = weak_state_for_cancel.upgrade() {
            state.cancel_click_rename();
        }
    });
    widget.add_controller(click.clone());
    click
}

/// Modified clicks are claimed before GTK's own click handling, so GTK's Shift+arrow origin
/// stays put. Only `list.select-item` moves it, and it replaces the selection, so the bits
/// are put back.
fn move_range_origin(
    row: &gtk::Widget,
    selection: &gtk::MultiSelection,
    syncing: &Cell<bool>,
    position: u32,
) {
    let selected = selection.selection().copy();
    syncing.set(true);
    if let Err(error) = row.activate_action(
        "list.select-item",
        Some(&(position, false, false).to_variant()),
    ) {
        tracing::warn!(%error, "range origin action unavailable");
    }
    selection.set_selection(&selected, &gtk::Bitset::new_range(0, selection.n_items()));
    syncing.set(false);
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
    include_icon_details: bool,
) -> Option<usize> {
    position.filter(|_| {
        super::browser::metadata_needs_fill(entry)
            || (include_mode && entry.mode == MetadataValue::Unknown)
            || entry.is_directory()
            || (include_icon_details && icon_details_need_fill(entry))
    })
}

pub(super) fn icon_details_need_fill(entry: &FileEntry) -> bool {
    let path = Path::new(&entry.native_name);
    (entry.is_directory() && entry.child_count == MetadataValue::Unknown)
        || (!entry.is_directory()
            && ((entry.image_dimensions == MetadataValue::Unknown
                && crate::services::is_image_path(path))
                || (entry.duration_seconds == MetadataValue::Unknown
                    && crate::services::is_media_path(path))))
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

/// The in-place filter's target, for a pane whose filter narrows the listing itself
/// rather than showing search results in its place. Its selection is the listing's
/// own: a new query only stops that selection from counting as the user's pick.
fn in_place_enter_target(
    entry: &gtk::Entry,
    browser: &Browser,
    depth: usize,
) -> Option<super::enter_target::EnterTarget> {
    let searches = browser
        .location_at(depth)
        .is_some_and(|location| location.native_path().is_some());
    (!searches).then(|| super::enter_target::EnterTarget::new(entry))
}

fn attach_in_place(
    enter_target: &super::enter_target::EnterTarget,
    section: &PaneSection,
    scroll: &gtk::ScrolledWindow,
) {
    let syncing = section.syncing.clone();
    enter_target.attach(
        &section.view,
        &section.selection,
        scroll,
        section.visit.clone(),
        move || syncing.get(),
    );
}

/// Down from the field lands on the top row Enter would open, not the listing's cursor.
fn connect_in_place_down(
    entry: &gtk::Entry,
    enter_target: &super::enter_target::EnterTarget,
    view: &gtk::Widget,
    state: Option<Weak<super::browser::ViewState>>,
) {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let enter_target = enter_target.clone();
    let view = view.downgrade();
    super::keys::on_press(&keys, move |_, press| {
        let Some(view) = view
            .upgrade()
            .filter(|_| press.is(super::keys::bindings::DOWN) && enter_target.offers_top())
        else {
            return glib::Propagation::Proceed;
        };
        if let Some(state) = state.as_ref().and_then(Weak::upgrade) {
            super::browser::claim_keyboard_navigation(&state);
        } else if let Some(window) = view.root().and_downcast::<gtk::Window>() {
            window.set_focus_visible(true);
        }
        view.grab_focus();
        super::browser::scroll_results_to(
            &view,
            0,
            gtk::ListScrollFlags::FOCUS | gtk::ListScrollFlags::SELECT,
        );
        glib::Propagation::Stop
    });
    entry.add_controller(keys);
}

fn filter_activation_position(
    enter_target: Option<&super::enter_target::EnterTarget>,
    selection: &gtk::MultiSelection,
) -> Option<u32> {
    if enter_target.is_some_and(super::enter_target::EnterTarget::offers_top) {
        return Some(0);
    }
    (0..selection.n_items()).find(|position| selection.is_selected(*position))
}

fn activate_filtered_item(
    browser: &Weak<Browser>,
    position: Option<u32>,
    view: &gio::ListModel,
    source_index: &SourceIndexMap,
    depth: usize,
) {
    let Some(position) = position else {
        return;
    };
    let Some(source_position) = source_index.of_view_position(view, position) else {
        return;
    };
    let Some(browser) = browser.upgrade() else {
        return;
    };
    let Some(entry) = browser.entry_at(depth, source_position) else {
        return;
    };
    let opens_file = browser.is_chooser_mode() && !entry.is_directory();
    // Trashed files do not open; the top-result fallback must not move the selection.
    if !opens_file && is_trash_location(&entry.location) && !entry.is_directory() {
        return;
    }
    browser.select(depth, source_position);
    if opens_file {
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
    rename_label: &impl IsA<gtk::Widget>,
    browser: Weak<Browser>,
    weak_state: Weak<super::browser::ViewState>,
    enabled: Rc<Cell<bool>>,
    click_activation: Rc<Cell<ClickActivation>>,
    depth: usize,
    position_map: Option<(SourceIndexMap, gio::ListModel)>,
    filter_query: Rc<RefCell<String>>,
    slow_click: Rc<SlowClickRename>,
) {
    let click = gtk::GestureClick::new();
    click.set_button(1);
    let clicked_item = item.downgrade();
    let rename_label = rename_label.as_ref().downgrade();
    super::pointer::connect_click_release(&click, item, move |gesture, press_count, x, y| {
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
            browser.select(depth, position);
            let click = if entry.is_directory() {
                FilteredFileClick::Open
            } else {
                filtered_file_click(
                    &entry,
                    browser.is_chooser_mode(),
                    enabled.get(),
                    click_activation.get().files,
                )
            };
            match click {
                FilteredFileClick::Open => browser.activate_in_place(depth, position),
                FilteredFileClick::Preview => browser.preview(depth, position),
                FilteredFileClick::Ignore => {}
            }
            return;
        }
        if should_activate_pointer_click(press_count, entry.is_directory(), click_activation.get())
        {
            if !browser.is_chooser_mode() || entry.is_directory() {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                browser.activate_in_place(depth, position);
            }
        } else if press_count == 1
            && !modifiers.intersects(
                gtk::gdk::ModifierType::ALT_MASK
                    | gtk::gdk::ModifierType::SUPER_MASK
                    | gtk::gdk::ModifierType::META_MASK,
            )
            && slow_click.was_selected.get()
            && slow_click.selected_count_before.get() == 1
            && !browser.is_chooser_mode()
            && !is_trash_location(&entry.location)
        {
            if let (Some(surface), Some(label)) = (gesture.widget(), rename_label.upgrade())
                && super::pointer::hits_name_label(&surface, &label, x, y)
                && let Some(state) = weak_state.upgrade()
            {
                state.schedule_click_rename(depth, position);
            }
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FilteredFileClick {
    Open,
    Preview,
    Ignore,
}

/// One plain click on a filtered file follows the unfiltered click-count and
/// preview rules, except that chooser and trashed files never open from it.
pub(super) fn filtered_file_click(
    entry: &FileEntry,
    chooser: bool,
    previews: bool,
    files: ClickCount,
) -> FilteredFileClick {
    if files == ClickCount::One && !chooser && !is_trash_location(&entry.location) {
        FilteredFileClick::Open
    } else if previews && super::preview::entry_supports_quick_preview(entry) {
        FilteredFileClick::Preview
    } else {
        FilteredFileClick::Ignore
    }
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
    let view_index = section.view_index.clone();
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
            let selected = selection.selection();
            let source_of = |position: u32| {
                view_index
                    .sources_of(&source_index, &view_model, std::iter::once(position))
                    .pop()
                    .flatten()
            };
            let changed = gtk::Bitset::new_range(position, count);
            changed.intersect(&selected);
            let toggled = (!changed.is_empty())
                .then(|| changed.maximum())
                .and_then(source_of);
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
                .filter(|position| selected.contains(*position))
                .and_then(source_of);
            let focused = toggled.or(native_focus).or_else(|| {
                let (mut lower, last) = gtk::BitsetIter::init_last(&selected)?;
                std::iter::once(last)
                    .chain(std::iter::from_fn(|| lower.previous()))
                    .find_map(source_of)
            });
            sync_browser_selection(&sections, &browser, depth, &source_index, focused);
        });
}

fn select_all(pane: &Pane) {
    for section in pane.item_sections() {
        section.syncing.set(true);
        section.selection.select_all();
        section.syncing.set(false);
    }
}

fn set_selections(pane: &Pane, positions: &[usize]) {
    let all_selected = positions.len() == pane.model.n_items() as usize;
    for section in pane.item_sections() {
        let total_items = section.selection.n_items();
        let selected = if all_selected && section.view_model.n_items() == pane.model.n_items() {
            gtk::Bitset::new_range(0, total_items)
        } else {
            let selected = gtk::Bitset::new_empty();
            for position in positions {
                if let Some(position) = section.source_to_view(&pane.model, *position) {
                    selected.add(position);
                }
            }
            selected
        };
        section.syncing.set(true);
        section
            .selection
            .set_selection(&selected, &gtk::Bitset::new_range(0, total_items));
        section.syncing.set(false);
    }
}

fn reset_native_range_origin(pane: &Pane, source_position: usize) {
    for section in pane.item_sections() {
        let Some(position) =
            view_position_for_source(&pane.model, Some(&section.view_model), source_position)
        else {
            continue;
        };
        // MultiSelection bits do not move GtkListView/GridView's Shift range
        // origin; list.select-item does.
        section.syncing.set(true);
        section
            .view
            .activate_action(
                "list.select-item",
                Some(&(position, false, false).to_variant()),
            )
            .expect("ListView and GridView expose list.select-item");
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
    browser: Weak<Browser>,
    expected: (usize, usize, Location),
) {
    // A later input or splice supersedes this cursor even when focus stays in the same view.
    let is_current = move || {
        browser.upgrade().is_some_and(|browser| {
            browser
                .focused_item()
                .is_some_and(|(depth, source, entry)| {
                    depth == expected.0 && source == expected.1 && entry.location == expected.2
                })
        })
    };
    glib::idle_add_local_once(move || {
        let Some(view) = view.upgrade() else {
            return;
        };
        if !is_current() || !collection_keeps_cursor(&view) {
            return;
        }
        if focus_bound_cursor(&items, position) {
            return;
        }
        let frames = Cell::new(0u8);
        let items = items.clone();
        view.add_tick_callback(move |view, _| {
            if !is_current()
                || !collection_keeps_cursor(view)
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
        focused == view
            || view.is_ancestor(focused)
            || focused.is_ancestor(view)
            || super::focus_navigation::focus_removed_from(view, focused)
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

fn set_mode_mark_style(widget: &impl IsA<gtk::Widget>, mark: ClipboardMark) {
    if mark == ClipboardMark::Cut {
        widget.add_css_class("cut");
    } else {
        widget.remove_css_class("cut");
    }
    if let Some((icon, _)) = super::icons_cell::parts(widget) {
        icon.set_mark(mark);
    } else if let Some((icon, ..)) = widget
        .upcast_ref()
        .downcast_ref::<gtk::Box>()
        .and_then(list_row_parts)
    {
        icon.set_mark(mark);
    }
}

fn refresh_mark_pane(pane: &Pane, browser: &Browser, marks: &ClipboardMarks) {
    pane.search.refresh_mark_rows();
    for section in pane.item_sections() {
        section.bound_items.borrow_mut().retain(|bound| {
            let (Some(item), Some(widget)) = (bound.item.upgrade(), bound.widget.upgrade()) else {
                return false;
            };
            let source = item
                .item()
                .and_then(|value| pane.source_index.of_item(&value));
            let mark = source
                .and_then(|position| browser.entry_at(pane.depth, position))
                .map_or(ClipboardMark::None, |entry| mark_in(marks, &entry.location));
            set_mode_mark_style(&widget, mark);
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
}

fn detach_pane_models(pane: &Pane) {
    pane.detached.set(true);
    pane.search.detach();
    for section in pane.all_sections() {
        section.syncing.set(true);
        section.selection.set_model(None::<&gio::ListModel>);
        super::browser::detach_collection_view(&section.view);
    }
    if let Some(filtered) = pane.filter_model.as_ref() {
        filtered.set_model(None::<&gio::ListModel>);
    }
}

fn deactivate_pane_models(pane: &Pane) {
    pane.detached.set(true);
    for section in pane.all_sections() {
        section.syncing.set(true);
        section.selection.set_model(None::<&gio::ListModel>);
    }
    if let Some(filtered) = pane.filter_model.as_ref() {
        filtered.set_model(None::<&gio::ListModel>);
    }
    // Keep the pane's controls, not a second directory snapshot and its identity map.
    pane.model.splice(0, pane.model.n_items(), &[]);
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
    }
}

fn show_count(pane: &Pane) {
    show_count_controls(pane);
    if pane.model.n_items() == 0 {
        pane.status.remove_css_class("error");
        pane.status
            .set_label(&crate::i18n::tr("This directory is empty"));
        pane.administrator.set_visible(false);
        pane.loading.show(STATUS_PAGE);
    } else {
        pane.loading.show(super::loading_skeleton::CONTENT_PAGE);
    }
}

fn show_count_controls(pane: &Pane) {
    let count = pane.model.n_items();
    if let Some(entry) = &pane.filter_entry {
        entry.set_placeholder_text(Some(&super::browser::filter_placeholder(count as usize)));
    }
    if let Some(button) = &pane.empty_trash_button {
        button.set_sensitive(count > 0);
    }
}

fn apply_snapshot(pane: &Pane, snapshot: &BrowserColumnSnapshot, browser: &Browser) {
    replace_entries(pane, browser, snapshot.count);
    reconnect_pane_model(pane);
    let restarts_loading =
        snapshot.loading && (snapshot.count == 0 || !snapshot.location.is_camera_photo_root());
    // A load still in progress is not an empty directory; showing that page first
    // would hand the surface focus before the grace period decides.
    if restarts_loading {
        show_count_controls(pane);
    } else {
        show_count(pane);
    }
    set_selections(pane, &snapshot.selected_positions);
    if let Some(&focused) = snapshot.selected_positions.last() {
        scroll_pane_to_source(pane, focused);
    }
    pane.truncated_hint.set_visible(snapshot.truncated);
    pane.leftovers_button
        .set_visible(!browser.column_leftovers(pane.depth).is_empty());
    pane.not_responding.set_visible(snapshot.not_responding);
    if snapshot.loading {
        pane.spinner.start();
        if restarts_loading {
            pane.loading.start();
        }
    } else {
        pane.spinner.stop();
        if let Some(message) = snapshot.error.as_deref() {
            pane.status.set_label(&rust_i18n::t!(
                "Unable to read this directory\n%{message}",
                message = message
            ));
            pane.status.add_css_class("error");
            pane.administrator
                .set_visible(snapshot.administrator.is_some());
            pane.loading.show(STATUS_PAGE);
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
    icon.add_css_class("list-file-icon");
    icon.set_valign(gtk::Align::Center);
    let name = gtk::Label::new(None);
    name.add_css_class("alternate-rename-label");
    name.set_xalign(0.0);
    name.set_hexpand(true);
    name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    // Keep the label's natural width from widening this fixed-width table cell.
    name.set_max_width_chars(1);
    let field = gtk::Entry::new();
    field.add_css_class("inline-rename");
    super::accessibility::set_label(&field, &crate::i18n::tr("Rename"));
    field.set_hexpand(true);
    field.set_visible(false);
    name_cell.append(&icon);
    name_cell.append(&name);
    name_cell.append(&field);
    let mode_label = list_metadata_label();
    mode_label.add_css_class("list-col-mode");
    let size_label = list_metadata_label();
    size_label.add_css_class("list-col-size");
    let kind_label = list_metadata_label();
    kind_label.add_css_class("list-col-type");
    let modified_label = list_metadata_label();
    modified_label.add_css_class("list-col-modified");

    let manager = super::preferences::PreferenceManager::shared();
    let order = manager.list_column_order();
    let hidden = manager.list_hidden_columns();

    for col in order {
        let is_visible = !hidden.contains(&col) || col == ListColumnId::Name;
        let widget: gtk::Widget = match col {
            ListColumnId::Name => name_cell.clone().upcast(),
            ListColumnId::Mode => mode_label.clone().upcast(),
            ListColumnId::Size => size_label.clone().upcast(),
            ListColumnId::Type => kind_label.clone().upcast(),
            ListColumnId::Modified => modified_label.clone().upcast(),
        };
        if !is_visible {
            widget.set_visible(false);
        }
        row.append(&widget);
    }
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
    let mut name_cell: Option<gtk::Box> = None;
    let mut mode: Option<gtk::Label> = None;
    let mut size: Option<gtk::Label> = None;
    let mut kind: Option<gtk::Label> = None;
    let mut modified: Option<gtk::Label> = None;

    let mut child = row.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        if current.has_css_class("list-name-cell") {
            name_cell = current.downcast::<gtk::Box>().ok();
        } else if current.has_css_class("list-col-mode") {
            mode = current.downcast::<gtk::Label>().ok();
        } else if current.has_css_class("list-col-size") {
            size = current.downcast::<gtk::Label>().ok();
        } else if current.has_css_class("list-col-type") {
            kind = current.downcast::<gtk::Label>().ok();
        } else if current.has_css_class("list-col-modified") {
            modified = current.downcast::<gtk::Label>().ok();
        }
    }

    let name_cell = name_cell?;
    let icon = name_cell
        .first_child()?
        .downcast::<super::thumbnail::ThumbnailSlot>()
        .ok()?;
    let name = icon.next_sibling()?.downcast::<gtk::Label>().ok()?;
    let field = name.next_sibling()?.downcast::<gtk::Entry>().ok()?;
    Some((icon, name, field, mode?, size?, kind?, modified?))
}

fn apply_columns_layout(
    headings: &gtk::Box,
    columns: &ListColumnLayout,
    bound_items: &Rc<RefCell<Vec<BoundModeItem>>>,
) {
    let manager = super::preferences::PreferenceManager::shared();
    let order = manager.list_column_order();
    let hidden = manager.list_hidden_columns();

    for col in ListColumnId::ALL {
        let is_visible = !hidden.contains(&col) || col == ListColumnId::Name;
        for weak in columns.cells[col.index()].borrow().iter() {
            if let Some(widget) = weak.upgrade() {
                widget.set_visible(is_visible);
            }
        }
    }

    let mut heading_children = Vec::new();
    let mut current = headings.first_child();
    while let Some(child) = current {
        current = child.next_sibling();
        heading_children.push(child);
    }
    let mut prev_heading: Option<gtk::Widget> = None;
    for col in &order {
        let class = col.css_class();
        if let Some(heading_cell) = heading_children.iter().find(|c| c.has_css_class(class)) {
            headings.reorder_child_after(heading_cell, prev_heading.as_ref());
            prev_heading = Some(heading_cell.clone());
        }
    }

    for bound in bound_items.borrow().iter() {
        let Some(widget) = bound.widget.upgrade() else {
            continue;
        };
        let Some(row) = widget.downcast::<gtk::Box>().ok() else {
            continue;
        };
        let Some(row_struct) = ListRow::from_widget(row.clone()) else {
            continue;
        };
        let mut prev_cell: Option<gtk::Widget> = None;
        for col in &order {
            let cell: gtk::Widget = match col {
                ListColumnId::Name => row_struct.name_cell.clone(),
                ListColumnId::Mode => row_struct.mode.clone().upcast(),
                ListColumnId::Size => row_struct.size.clone().upcast(),
                ListColumnId::Type => row_struct.kind.clone().upcast(),
                ListColumnId::Modified => row_struct.modified.clone().upcast(),
            };
            row.reorder_child_after(&cell, prev_cell.as_ref());
            prev_cell = Some(cell);
        }
    }
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
    marks: &ClipboardMarks,
    thumbnail_size: i32,
    scrolling: bool,
    state: Option<&super::browser::ViewState>,
) {
    let Some((icon, label)) = super::icons_cell::parts(card) else {
        return;
    };
    set_icons_entry_details(card, entry);
    let pending_name = state.and_then(|state| state.pending_rename_name(entry));
    let shown_name = pending_name.as_deref().unwrap_or(&entry.display_name);
    if label.text().as_deref() != Some(shown_name) {
        label.set_text(Some(shown_name));
    }
    super::thumbnail::set_thumbnail_or_icon(
        &icon,
        entry,
        super::browser::entry_icon(entry),
        thumbnail_size,
        thumbnail_size,
    );
    if scrolling {
        icon.set_hidden(entry.is_hidden);
        icon.set_base_opacity(if entry.is_directory() { 1.0 } else { 0.72 });
        label.set_opacity(if entry.is_hidden { 0.65 } else { 1.0 });
        if let Some(details) = super::icons_cell::details_label(card) {
            details.set_opacity(if entry.is_hidden { 0.65 } else { 1.0 });
        }
    } else {
        refresh_icons_card_chrome(item, card, &icon, &label, entry, marks);
    }
    if let Some(item) = item.filter(|_| pending_name.is_some()) {
        crate::ui::accessibility::set_description(&label, Some(shown_name));
        super::accessibility::describe_entry(item, shown_name, Some(entry));
    }
}

fn refresh_icons_card_chrome(
    item: Option<&gtk::ListItem>,
    card: &gtk::Box,
    icon: &super::thumbnail::ThumbnailSlot,
    label: &gtk::Inscription,
    entry: &FileEntry,
    marks: &ClipboardMarks,
) {
    crate::ui::accessibility::set_description(label, Some(&entry.display_name));
    if let Some(item) = item {
        super::accessibility::describe_entry(item, &entry.display_name, Some(entry));
    }
    set_mode_mark_style(card, mark_in(marks, &entry.location));
    icon.set_hidden(entry.is_hidden);
    icon.set_base_opacity(if entry.is_directory() { 1.0 } else { 0.72 });
    label.set_opacity(if entry.is_hidden { 0.65 } else { 1.0 });
    if let Some(details) = super::icons_cell::details_label(card) {
        details.set_opacity(if entry.is_hidden { 0.65 } else { 1.0 });
    }
}

fn refresh_icons_section(
    browser: &Rc<Browser>,
    depth: usize,
    source_index: &SourceIndexMap,
    section: &PaneSection,
    marks: &ClipboardMarks,
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
        if super::thumbnail::near_viewport(&card) {
            refresh_icons_card_chrome(Some(&item), &card, &icon, &label, &entry, marks);
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

pub(in crate::ui) fn entry_size(entry: &FileEntry) -> String {
    if entry.is_directory() {
        return folder_item_info(entry).unwrap_or_default();
    }
    match entry.size {
        MetadataValue::Known(bytes) => super::browser::format_file_size(bytes),
        MetadataValue::Unknown | MetadataValue::Unavailable => String::new(),
    }
}

fn entry_type(entry: &FileEntry) -> String {
    match crate::services::entry_type(entry) {
        crate::services::EntryType::Known(description) => description,
        kind => crate::i18n::tr(kind.description()),
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
            let Some((icon, name, _, mode, size, _, modified)) = list_row_parts(&row) else {
                return true;
            };
            if entry.is_locked_folder() {
                super::thumbnail::show_locked_folder(&icon);
                super::accessibility::describe_entry(&item, &name.label(), Some(entry));
            }
            mode.set_label(&entry_mode(entry));
            size.set_label(&entry_size(entry));
            crate::util::set_modified_date(&modified, Some(entry), "—");
            true
        });
    }
}

fn update_bound_icons_metadata(pane: &Pane, updates: &[(usize, FileEntry)]) {
    let updates: HashMap<usize, &FileEntry> = updates
        .iter()
        .map(|(position, entry)| (*position, entry))
        .collect();
    for section in pane.item_sections() {
        section.bound_items.borrow_mut().retain(|bound| {
            let (Some(item), Some(card)) = (bound.item.upgrade(), bound.widget.upgrade()) else {
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
            let Some(card) = card.downcast::<gtk::Box>().ok() else {
                return true;
            };
            if entry.is_locked_folder() {
                if let Some((icon, _)) = super::icons_cell::parts(&card) {
                    super::thumbnail::show_locked_folder(&icon);
                }
                super::accessibility::describe_entry(&item, &entry.display_name, Some(entry));
            }
            set_icons_entry_details(&card, entry);
            true
        });
    }
}

fn set_icons_entry_details(card: &gtk::Box, entry: &FileEntry) {
    let Some(label) = super::icons_cell::details_label(card) else {
        return;
    };
    set_label_if_changed(&label, &entry_icons_item_info(entry).unwrap_or_default());
}

fn format_duration(seconds: u64) -> String {
    let hours = seconds / 3_600;
    let minutes = (seconds % 3_600) / 60;
    let seconds = seconds % 60;
    if hours == 0 {
        format!("{minutes}:{seconds:02}")
    } else {
        format!("{hours}:{minutes:02}:{seconds:02}")
    }
}

fn folder_item_info(entry: &FileEntry) -> Option<String> {
    match entry.child_count {
        MetadataValue::Known(0) => Some(crate::i18n::tr("No items")),
        MetadataValue::Known(count) => Some(crate::i18n::count_u64("items", count)),
        MetadataValue::Unknown | MetadataValue::Unavailable => None,
    }
}

fn entry_icons_item_info(entry: &FileEntry) -> Option<String> {
    if entry.is_directory() {
        return folder_item_info(entry);
    }
    if let MetadataValue::Known(seconds) = entry.duration_seconds {
        return Some(format_duration(seconds));
    }
    if let MetadataValue::Known((width, height)) = entry.image_dimensions {
        return Some(format!("{width}×{height}"));
    }
    match entry.size {
        MetadataValue::Known(bytes) => Some(super::browser::format_file_size(bytes)),
        MetadataValue::Unknown | MetadataValue::Unavailable => None,
    }
}

fn compare_type_groups(left: &str, right: &str) -> std::cmp::Ordering {
    fn rank(label: &str) -> u8 {
        match label {
            "" => 0,
            super::browser::FOLDER_TYPE_GROUP => 1,
            super::browser::OTHER_TYPE_GROUP => 3,
            _ => 2,
        }
    }
    rank(left).cmp(&rank(right)).then_with(|| {
        left.chars()
            .flat_map(char::to_lowercase)
            .cmp(right.chars().flat_map(char::to_lowercase))
    })
}

fn model_value(item: &glib::Object) -> String {
    item.downcast_ref::<gtk::StringObject>()
        .map(|value| value.string().to_string())
        .unwrap_or_default()
}

fn model_string(item: &glib::Object) -> Option<glib::GString> {
    item.downcast_ref::<gtk::StringObject>()
        .map(|value| value.string())
}

/// Groups a pane's rows by file type. GTK's stable sort preserves the pane's
/// filename ordering within each type.
#[derive(Clone)]
struct TypeGroupSorter {
    sorter: gtk::CustomSorter,
    /// The preferences the last comparison read, so a re-sort only follows a
    /// preference change GTK has not already sorted spliced rows under.
    compared_with: Rc<Cell<Option<crate::model::ViewPreferences>>>,
}

impl TypeGroupSorter {
    fn new(preferences: impl Fn() -> Option<crate::model::ViewPreferences> + 'static) -> Self {
        let compared_with = Rc::new(Cell::new(None));
        let seen = compared_with.clone();
        let sorter = gtk::CustomSorter::new(move |left, right| {
            let preferences = preferences();
            seen.set(preferences);
            let (left, right) = (model_string(left), model_string(right));
            compare_type_groups_for_preferences(
                super::browser::model_type_group(left.as_deref().unwrap_or("")),
                super::browser::model_type_group(right.as_deref().unwrap_or("")),
                preferences.unwrap_or_default(),
            )
            .into()
        });
        Self {
            sorter,
            compared_with,
        }
    }

    /// Re-sorts after a preference change; a full re-sort is the one expensive
    /// operation on a large pane, so rows spliced under the current preferences
    /// never trigger one.
    fn refresh(&self, preferences: Option<crate::model::ViewPreferences>) {
        if self.compared_with.get() == preferences {
            return;
        }
        self.compared_with.set(preferences);
        self.sorter.changed(gtk::SorterChange::Different);
    }
}

fn pane_type_group_sorter(browser: &Rc<Browser>, depth: usize) -> TypeGroupSorter {
    let browser = Rc::downgrade(browser);
    TypeGroupSorter::new(move || {
        browser
            .upgrade()
            .and_then(|browser| browser.column_preferences(depth))
    })
}

fn compare_type_groups_for_preferences(
    left: &str,
    right: &str,
    preferences: crate::model::ViewPreferences,
) -> std::cmp::Ordering {
    if preferences.sort_key != SortKey::Type {
        return compare_type_groups(left, right);
    }
    if left.is_empty() || right.is_empty() {
        return left.cmp(right);
    }
    if preferences.folders_first {
        let folders = (right == super::browser::FOLDER_TYPE_GROUP)
            .cmp(&(left == super::browser::FOLDER_TYPE_GROUP));
        if folders != std::cmp::Ordering::Equal {
            return folders;
        }
    }
    let order = (left == super::browser::OTHER_TYPE_GROUP)
        .cmp(&(right == super::browser::OTHER_TYPE_GROUP))
        .then_with(|| crate::app::compare_display_names(left, right));
    match preferences.sort_direction {
        SortDirection::Ascending => order,
        SortDirection::Descending => order.reverse(),
    }
}

fn type_group_heading(label: &str) -> gtk::Label {
    let heading = gtk::Label::new(Some(&crate::i18n::tr(label)));
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
        let group = super::browser::model_type_group(&value);
        heading.set_label(&crate::i18n::tr(group));
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

fn selected_source_positions(source_index: &SourceIndexMap, section: &PaneSection) -> Vec<usize> {
    let selected = section.selection.selection();
    let positions = gtk::BitsetIter::init_first(&selected)
        .into_iter()
        .flat_map(|(rest, first)| std::iter::once(first).chain(rest));
    section
        .view_index
        .sources_of(source_index, &section.view_model, positions)
        .into_iter()
        .flatten()
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
            .flat_map(|section| selected_source_positions(source_index, section))
            .collect()
    };
    positions.sort_unstable();
    positions.dedup();
    let focused = focused.or_else(|| positions.last().copied());
    if super::marquee::is_updating_selection() {
        browser.commit_selection();
    }
    browser.set_selection(depth, &positions, focused);
    let model = browser.selected_positions(depth);
    if model != positions {
        for section in sections.borrow().iter() {
            let selected = gtk::Bitset::new_empty();
            let count = section.view_model.n_items();
            let sources =
                section
                    .view_index
                    .sources_of(source_index, &section.view_model, 0..count);
            for (view_position, source) in (0..count).zip(sources) {
                if source.is_some_and(|source| model.binary_search(&source).is_ok()) {
                    selected.add(view_position);
                }
            }
            section.syncing.set(true);
            section
                .selection
                .set_selection(&selected, &gtk::Bitset::new_range(0, count));
            section.syncing.set(false);
        }
    }
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
) -> Rc<dyn Fn(f64, f64)> {
    let items = section.bound_items.clone();
    let pick_position = Rc::new(move |picked: &gtk::Widget| {
        items.borrow().iter().find_map(|bound| {
            let widget = bound.widget.upgrade()?;
            (widget == *picked || picked.is_ancestor(&widget))
                .then(|| bound.item.upgrade().map(|item| item.position()))?
        })
    });
    let source_index = source_index.clone();
    let view_model = section.view_model.clone();
    let source_position = Rc::new(move |position| {
        source_position_for_view(&source_index, Some(&view_model), position)
    });
    let owner_view = section.view.downgrade();
    let clear_other_selections = Rc::new(move || {
        let Some(sections) = sections.upgrade() else {
            return;
        };
        for other in sections.borrow().iter() {
            if Some(&other.view) == owner_view.upgrade().as_ref() {
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
    )
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
