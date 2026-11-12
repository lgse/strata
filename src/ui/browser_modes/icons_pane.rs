// SPDX-License-Identifier: MIT

//! The Icons presentation: a grid of cards that reflows with its width, density,
//! and thumbnail size.

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

use gtk::{gio, glib, prelude::*};

use super::{
    BrowserDensity, BrowserMode, DensityScale, ICONS_CARD_SPACING, ModeViews, TransferHandlerSlot,
    filter::{PaneFilter, attach_in_place},
    icons_card::CardRefresh,
    icons_card_extent, icons_card_icon_slot,
    icons_controls::{IconsControls, ThumbnailSizeMenu, icons_controls},
    icons_view::build_icons_view,
    pane::{
        BoundItems, ModeClickOptions, Pane, PaneBuild, PaneChrome, PaneListing, PaneSection,
        collection_with_marquee, pane_base, refresh_marquee_targets,
    },
    scroll_settle::install_scroll_refresh,
    source_index::SourceIndexMap,
    type_groups::{TypeGroupSorter, pane_type_group_sorter},
};
use crate::{
    app::Browser,
    model::Location,
    ui::{
        browser::{self, ClipboardMarks, ViewState},
        enter_target::EnterTarget,
        icons_cell,
        inline_search::SearchPresentation,
        loading_skeleton,
        preferences::PreferenceManager,
    },
};

pub(super) const DEFAULT_ICONS_THUMBNAIL_SIZE: i32 = 64;
const FALLBACK_ICONS_COLUMN_WIDTH: i32 = 120;
/// Below any real Compact row, so the first pin never caps rows under what fits.
const FALLBACK_COMPACT_ROW_HEIGHT: i32 = 20;
const MAX_COMPACT_ROWS: i32 = 200;
const INITIAL_COMPACT_ROWS: u32 = 20;
const COMPACT_REPIN_FRAMES: u8 = 120;
const MIN_COMPACT_ICON_SIZE: i32 = 16;
const MAX_COMPACT_ICON_SIZE: i32 = 24;

/// How an icons pane lays out its cells.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum IconsLayout {
    /// Cards with the name under the icon, in rows that scroll down.
    Grid,
    /// One line per entry, in columns that scroll sideways.
    Compact,
}

impl IconsLayout {
    fn mode(self) -> BrowserMode {
        match self {
            Self::Grid => BrowserMode::Icons,
            Self::Compact => BrowserMode::Compact,
        }
    }

    /// The axis the grid wraps along; its viewport extent decides the cells per line.
    fn wrap_orientation(self) -> gtk::Orientation {
        match self {
            Self::Grid => gtk::Orientation::Horizontal,
            Self::Compact => gtk::Orientation::Vertical,
        }
    }
}

/// Compact icons follow the density: 16 px at Compact, growing to 24 px at Airy.
pub(super) fn compact_icon_size(density: BrowserDensity) -> i32 {
    let step = i32::from(density.px()) - i32::from(BrowserDensity::COMPACT_PX);
    (MIN_COMPACT_ICON_SIZE + step * 2).clamp(MIN_COMPACT_ICON_SIZE, MAX_COMPACT_ICON_SIZE)
}

/// Shared wiring every icons view in a pane needs, so a pane that groups entries by
/// type can build one view per group without threading a dozen arguments through.
pub(super) struct IconsContext {
    pub(super) browser: Rc<Browser>,
    pub(super) depth: usize,
    pub(super) click: ModeClickOptions,
    pub(super) transfer: TransferHandlerSlot,
    pub(super) marks: Rc<RefCell<ClipboardMarks>>,
    pub(super) state: Option<Weak<ViewState>>,
    pub(super) thumbnail_size: Rc<Cell<i32>>,
    pub(super) source_index: SourceIndexMap,
    pub(super) sections: Weak<RefCell<Vec<PaneSection>>>,
    pub(super) density: Cell<BrowserDensity>,
    pub(super) scrolling: Rc<Cell<bool>>,
    pub(super) filter_query: Rc<RefCell<String>>,
    pub(super) enter_target: Option<EnterTarget>,
    pub(super) layout: IconsLayout,
}

/// How an Icons pane sizes and groups its cards.
pub(super) struct IconsOptions {
    pub(super) thumbnail_size: Rc<Cell<i32>>,
    pub(super) group_by_type: bool,
    pub(super) density: BrowserDensity,
    pub(super) layout: IconsLayout,
}

pub(super) fn build_icons_pane(inputs: &PaneBuild, options: IconsOptions) -> Pane {
    let depth = inputs.depth;
    let layout = options.layout;
    let controls = icons_controls(&inputs.browser, depth, options.thumbnail_size.get());
    controls.thumbnail.set_visible(layout == IconsLayout::Grid);
    if let Some(state) = inputs.new_folder_state.clone() {
        controls
            .actions
            .prepend(&browser::pane_new_folder_button(state, depth));
    }
    let base = pane_base(PaneChrome {
        depth,
        location: inputs.location(),
        title: &inputs.title,
        mode: layout.mode(),
        class: match layout {
            IconsLayout::Grid => "icons-pane",
            IconsLayout::Compact => "compact-pane",
        },
        loading: match layout {
            IconsLayout::Grid => {
                icons_loading_skeleton(options.thumbnail_size.get(), options.density)
            }
            IconsLayout::Compact => compact_loading_skeleton(options.thumbnail_size.get()),
        },
        leading: controls.leading.clone().upcast(),
        actions: controls.actions.clone().upcast(),
    });
    base.prepare_listing(inputs, &controls.filter_revealer);
    let filter = PaneFilter::bind(
        controls.filter_entry.clone(),
        controls.filter_button.clone(),
        inputs,
        &base.model,
    );
    let (context, sections) = icons_context(inputs, &options, &base.source_index, &filter);
    let type_group_sorter = options
        .group_by_type
        .then(|| pane_type_group_sorter(&context.browser, depth));
    let view_model = icons_view_model(&filter.model, type_group_sorter.as_ref());
    let section = build_icons_view(&context, &view_model);
    section.view.set_vexpand(true);
    filter.connect_entry(&section, inputs, &context.source_index);
    sections.borrow_mut().push(section.clone());
    if layout == IconsLayout::Grid {
        connect_thumbnail_size(&controls.thumbnail, &context, &base.stack);
    }
    let scroll = icons_scroll(&context, &section, &controls.thumbnail.popover);
    let (collection, marquee, targets) = collection_with_marquee(
        &section.view,
        scroll,
        false,
        context.click.multiple_selection.clone(),
    );
    let search = filter.wrap_search(&collection, inputs, search_presentation(&context));
    base.content.append(&search.widget);
    marquee.add_origin_surface(&base.header);
    let IconsControls {
        sort_direction_button,
        thumbnail,
        content,
        ..
    } = controls;
    let listing = PaneListing {
        section,
        sections,
        targets,
        marquee,
        search,
        leftovers_button: content.leftovers,
        empty_trash_button: content.empty_trash,
    };
    let mut pane = Pane::assemble(base, filter, listing);
    pane.type_group_sorter = type_group_sorter;
    pane.icons = Some(context);
    pane.thumbnail_scale = (layout == IconsLayout::Grid).then_some(thumbnail.scale);
    pane.sort_direction_button = Some(sort_direction_button);
    refresh_marquee_targets(&pane);
    pane
}

type Sections = Rc<RefCell<Vec<PaneSection>>>;

fn icons_context(
    inputs: &PaneBuild,
    options: &IconsOptions,
    source_index: &SourceIndexMap,
    filter: &PaneFilter,
) -> (Rc<IconsContext>, Sections) {
    let sections: Sections = Rc::new(RefCell::new(Vec::new()));
    let context = Rc::new(IconsContext {
        browser: inputs.browser.clone(),
        depth: inputs.depth,
        click: inputs.click.clone(),
        transfer: inputs.transfer.clone(),
        marks: inputs.marks.clone(),
        state: inputs.state.clone(),
        thumbnail_size: options.thumbnail_size.clone(),
        source_index: source_index.clone(),
        sections: Rc::downgrade(&sections),
        density: Cell::new(options.density),
        scrolling: Rc::new(Cell::new(false)),
        filter_query: filter.query.clone(),
        enter_target: filter.enter_target.clone(),
        layout: options.layout,
    });
    (context, sections)
}

/// Search keeps the browsing presentation of the pane.
fn search_presentation(context: &IconsContext) -> SearchPresentation {
    match context.layout {
        IconsLayout::Grid => SearchPresentation::Icons {
            thumbnail_size: context.thumbnail_size.clone(),
            max_columns: density_icons_columns(context.density.get()),
        },
        IconsLayout::Compact => SearchPresentation::Compact {
            thumbnail_size: context.thumbnail_size.clone(),
        },
    }
}

fn icons_view_model(
    filtered: &gtk::FilterListModel,
    sorter: Option<&TypeGroupSorter>,
) -> gio::ListModel {
    let Some(sorter) = sorter else {
        return filtered.clone().upcast();
    };
    let sorted = gtk::SortListModel::new(Some(filtered.clone()), None::<gtk::CustomSorter>);
    sorted.set_sorter(Some(&sorter.sorter));
    sorted.upcast()
}

fn icons_scroll(
    context: &Rc<IconsContext>,
    section: &PaneSection,
    thumbnail_popover: &gtk::Popover,
) -> gtk::ScrolledWindow {
    let vertical_policy = match context.layout {
        IconsLayout::Grid => gtk::PolicyType::Automatic,
        IconsLayout::Compact => gtk::PolicyType::Never,
    };
    let scroll = gtk::ScrolledWindow::builder()
        .child(&section.view)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(vertical_policy)
        .vexpand(true)
        .build();
    scroll.add_css_class("browser-listing-scroll");
    if let Some(enter_target) = &context.enter_target {
        attach_in_place(enter_target, section, &scroll);
    }
    if context.layout == IconsLayout::Compact {
        wheel_scrolls_sideways(&scroll);
    }
    crate::ui::scrolling::popover::dismiss_on_outside_scroll(thumbnail_popover);
    let refresh = IconsScrollRefresh {
        browser: Rc::downgrade(&context.browser),
        source_index: context.source_index.clone(),
        sections: context.sections.clone(),
        marks: context.marks.clone(),
        depth: context.depth,
    };
    install_scroll_refresh(&scroll, context.scrolling.clone(), move || {
        refresh.refresh()
    });
    let extent_view = section.view.downgrade();
    let extent_context = Rc::downgrade(context);
    let wrap_adjustment = match context.layout.wrap_orientation() {
        gtk::Orientation::Vertical => scroll.vadjustment(),
        _ => scroll.hadjustment(),
    };
    after_viewport_extent_changes(&wrap_adjustment, move |extent| {
        let (Some(context), Some(view)) = (extent_context.upgrade(), extent_view.upgrade()) else {
            return;
        };
        pin_view_lines(&context, &view, extent);
    });
    scroll
}

/// Compact columns only scroll sideways, so a plain wheel moves them too.
pub(in crate::ui) fn wheel_scrolls_sideways(scroll: &gtk::ScrolledWindow) {
    let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
    // The grid's own scrolled window would otherwise take the vertical steps it ignores.
    wheel.set_propagation_phase(gtk::PropagationPhase::Capture);
    let adjustment = scroll.hadjustment();
    wheel.connect_scroll(move |wheel, dx, dy| sideways_wheel_step(wheel, &adjustment, dx, dy));
    scroll.add_controller(wheel);
}

fn sideways_wheel_step(
    wheel: &gtk::EventControllerScroll,
    adjustment: &gtk::Adjustment,
    dx: f64,
    dy: f64,
) -> glib::Propagation {
    // Ctrl+wheel zooms.
    if wheel
        .current_event_state()
        .contains(gtk::gdk::ModifierType::CONTROL_MASK)
    {
        return glib::Propagation::Proceed;
    }
    let delta = if dx.abs() > dy.abs() { dx } else { dy };
    if delta == 0.0 {
        return glib::Propagation::Proceed;
    }
    let step = crate::ui::scrolling::popover::scroll_delta_for_unit(
        delta,
        adjustment.page_size(),
        wheel.unit(),
    );
    crate::ui::scrolling::advance(adjustment, step);
    glib::Propagation::Stop
}

fn pin_view_lines(context: &IconsContext, view: &gtk::Widget, extent: i32) {
    let Some(sections) = context.sections.upgrade() else {
        return;
    };
    let sections = sections.borrow();
    if let Some(section) = sections.iter().find(|section| section.view == *view) {
        pin_section_lines(section, context.layout, extent, context.density.get());
    }
}

fn pin_section_lines(
    section: &PaneSection,
    layout: IconsLayout,
    extent: i32,
    density: BrowserDensity,
) {
    match layout {
        IconsLayout::Grid => pin_ungrouped_icons_columns(section, extent, density),
        IconsLayout::Compact => pin_compact_rows(section, extent),
    }
}

/// A sideways grid's `max_columns` caps its rows. Like Icons columns, keep it near what
/// fits, since GTK keeps `max_columns * 30` cells alive.
fn pin_compact_rows(section: &PaneSection, height: i32) {
    if height <= 0 {
        return;
    }
    let Ok(grid) = section.view.clone().downcast::<gtk::GridView>() else {
        return;
    };
    let measured = measured_cell_height(&section.bound_items);
    let row = measured.unwrap_or(FALLBACK_COMPACT_ROW_HEIGHT);
    pin_ungrouped_grid_columns(&grid, compact_rows(height, row));
    if measured.is_none() {
        repin_once_measured(section.bound_items.clone(), &grid);
    }
}

fn compact_rows(height: i32, row: i32) -> u32 {
    (height / row.max(1)).clamp(1, MAX_COMPACT_ROWS) as u32
}

/// The fallback row height overestimates the rows; tighten them as soon as a cell binds.
fn repin_once_measured(bound_items: BoundItems, grid: &gtk::GridView) {
    let frames = Cell::new(0u8);
    grid.add_tick_callback(move |grid, _| {
        frames.set(frames.get().saturating_add(1));
        if let Some(row) = measured_cell_height(&bound_items) {
            let height = viewport_extent(grid.upcast_ref(), gtk::Orientation::Vertical);
            if height > 0 {
                pin_ungrouped_grid_columns(grid, compact_rows(height, row));
            }
            return glib::ControlFlow::Break;
        }
        if frames.get() >= COMPACT_REPIN_FRAMES {
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}

fn measured_cell_height(bound_items: &BoundItems) -> Option<i32> {
    bound_items.borrow().iter().find_map(|bound| {
        let widget = bound.widget.upgrade()?;
        let cell = widget.parent().unwrap_or(widget);
        let (_, natural, _, _) = cell.measure(gtk::Orientation::Vertical, -1);
        (natural > 0).then_some(natural)
    })
}

/// Redraws card chrome once scrolling comes to rest.
struct IconsScrollRefresh {
    browser: Weak<Browser>,
    source_index: SourceIndexMap,
    sections: Weak<RefCell<Vec<PaneSection>>>,
    marks: Rc<RefCell<ClipboardMarks>>,
    depth: usize,
}

impl IconsScrollRefresh {
    fn refresh(&self) {
        let Some(browser) = self.browser.upgrade() else {
            return;
        };
        let Some(sections) = self.sections.upgrade() else {
            return;
        };
        let marks = self.marks.borrow();
        let refresh = CardRefresh {
            browser: &browser,
            depth: self.depth,
            source_index: &self.source_index,
            marks: &marks,
        };
        for section in sections.borrow().iter() {
            section
                .bound_items
                .borrow()
                .iter()
                .for_each(|bound| refresh.refresh(bound));
        }
    }
}

fn connect_thumbnail_size(
    menu: &ThumbnailSizeMenu,
    context: &Rc<IconsContext>,
    stack: &gtk::Stack,
) {
    let change = ThumbnailSizeChange {
        value: menu.value.clone(),
        thumbnail_size: context.thumbnail_size.clone(),
        browser: Rc::downgrade(&context.browser),
        depth: context.depth,
        stack: stack.downgrade(),
        context: Rc::downgrade(context),
        sections: context.sections.clone(),
        density: context.density.get(),
    };
    menu.scale
        .connect_value_changed(move |scale| change.apply(scale));
}

/// A thumbnail slider move: saves the size and resizes cards and the loading page.
struct ThumbnailSizeChange {
    value: gtk::Label,
    thumbnail_size: Rc<Cell<i32>>,
    browser: Weak<Browser>,
    depth: usize,
    stack: glib::WeakRef<gtk::Stack>,
    context: Weak<IconsContext>,
    sections: Weak<RefCell<Vec<PaneSection>>>,
    /// The density the pane was built with, for after its context is gone.
    density: BrowserDensity,
}

impl ThumbnailSizeChange {
    fn apply(&self, scale: &gtk::Scale) {
        let size = scale.value().round() as i32;
        self.value.set_label(&format!("{size} px"));
        // A size applied from preferences updates the cell first and is not saved back.
        if self.thumbnail_size.replace(size) != size {
            let location = self
                .browser
                .upgrade()
                .and_then(|browser| browser.location_at(self.depth));
            PreferenceManager::shared().set_folder_icons_size(location.as_ref(), size);
        }
        self.replace_loading_page(size);
        let Some(sections) = self.sections.upgrade() else {
            return;
        };
        for section in sections.borrow().iter() {
            resize_icons_thumbnail_slots(section, size);
        }
        let density = self
            .context
            .upgrade()
            .map(|context| context.density.get())
            .unwrap_or(self.density);
        for section in sections.borrow().iter() {
            pin_ungrouped_icons_columns(
                section,
                viewport_extent(&section.view, gtk::Orientation::Horizontal),
                density,
            );
        }
    }

    fn replace_loading_page(&self, size: i32) {
        let (Some(stack), Some(context)) = (self.stack.upgrade(), self.context.upgrade()) else {
            return;
        };
        let was_loading =
            stack.visible_child_name().as_deref() == Some(loading_skeleton::LOADING_PAGE);
        if let Some(old) = stack.child_by_name(loading_skeleton::LOADING_PAGE) {
            stack.remove(&old);
        }
        stack.add_named(
            &icons_loading_skeleton(size, context.density.get()),
            Some(loading_skeleton::LOADING_PAGE),
        );
        if was_loading {
            stack.set_visible_child_name(loading_skeleton::LOADING_PAGE);
        }
    }
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
/// the new extent but a grow (closing preview, hiding the sidebar, widening the
/// window) keeps the old cap. Pin on the next idle instead.
fn after_viewport_extent_changes(adjustment: &gtk::Adjustment, on_extent: impl Fn(i32) + 'static) {
    let pending = Rc::new(Cell::new(false));
    let on_extent = Rc::new(on_extent);
    adjustment.connect_page_size_notify(move |adjustment| {
        if pending.replace(true) {
            return;
        }
        let pending = pending.clone();
        let on_extent = on_extent.clone();
        let adjustment = adjustment.clone();
        glib::idle_add_local_once(move || {
            pending.set(false);
            on_extent(adjustment.page_size() as i32);
        });
    });
}

fn viewport_extent(widget: &gtk::Widget, orientation: gtk::Orientation) -> i32 {
    let mut ancestor = widget.parent();
    while let Some(widget) = ancestor {
        ancestor = widget.parent();
        if let Ok(scroll) = widget.downcast::<gtk::ScrolledWindow>() {
            let adjustment = match orientation {
                gtk::Orientation::Vertical => scroll.vadjustment(),
                _ => scroll.hadjustment(),
            };
            return adjustment.page_size() as i32;
        }
    }
    match orientation {
        gtk::Orientation::Vertical => widget.height(),
        _ => widget.width(),
    }
}

fn measured_card_width(section: &PaneSection) -> Option<i32> {
    section.bound_items.borrow().iter().find_map(|bound| {
        let widget = bound.widget.upgrade()?;
        let (_, natural, _, _) = widget.measure(gtk::Orientation::Horizontal, -1);
        (natural > 0).then_some(natural + ICONS_CARD_SPACING)
    })
}

fn resize_icons_thumbnail_slots(section: &PaneSection, size: i32) {
    let size = icons_card_icon_slot(size);
    section.bound_items.borrow().iter().for_each(|bound| {
        let Some(card) = bound.widget.upgrade().and_downcast::<gtk::Box>() else {
            return;
        };
        icons_cell::set_slot(&card, size);
    });
}

fn ensure_icons_card_slot(card: &gtk::Box, thumbnail_size: i32) {
    let (width, height) = icons_card_extent(thumbnail_size);
    if card.width_request() != width || card.height_request() != height {
        card.set_size_request(width, height);
    }
}

/// The Icons grid of the pane's loading page, when it shows one.
fn loading_grid(pane: &Pane) -> Option<gtk::GridView> {
    let loading = pane.stack.child_by_name(loading_skeleton::LOADING_PAGE)?;
    let scroll = loading
        .first_child()
        .and_downcast::<gtk::ScrolledWindow>()?;
    scroll.child().and_downcast::<gtk::GridView>()
}

pub(super) fn configure_icons_density(pane: &Pane, density: BrowserDensity) {
    if let Some(context) = pane
        .icons
        .as_ref()
        .filter(|context| context.layout == IconsLayout::Compact)
    {
        configure_compact_density(pane, context, density);
        return;
    }
    pane.search
        .set_icons_max_columns(density_icons_columns(density));
    if let Some(icons) = loading_grid(pane) {
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
        pin_ungrouped_icons_columns(
            &section,
            viewport_extent(&section.view, gtk::Orientation::Horizontal),
            density,
        );
    }
}

fn configure_compact_density(pane: &Pane, context: &IconsContext, density: BrowserDensity) {
    context.density.set(density);
    let size = compact_icon_size(density);
    context.thumbnail_size.set(size);
    for section in pane.all_sections() {
        section.bound_items.borrow().iter().for_each(|bound| {
            if let Some(card) = bound.widget.upgrade().and_downcast::<gtk::Box>() {
                icons_cell::set_compact_slot(&card, size);
            }
        });
        // Rows change height with the density; measure them once they reflowed.
        let section = section.clone();
        glib::idle_add_local_once(move || {
            pin_compact_rows(
                &section,
                viewport_extent(&section.view, gtk::Orientation::Vertical),
            );
        });
    }
}

pub(in crate::ui) fn configure_compact_view(grid: &gtk::GridView) {
    grid.set_orientation(gtk::Orientation::Horizontal);
    // Columns take their names' natural width rather than collapsing to one character.
    grid.set_hscroll_policy(gtk::ScrollablePolicy::Natural);
    grid.set_min_columns(1);
    grid.set_max_columns(INITIAL_COMPACT_ROWS);
}

pub(super) fn configure_icons_view_density(icons: &gtk::GridView, density: BrowserDensity) {
    icons.set_min_columns(1);
    icons.set_max_columns(density_icons_columns(density));
}

pub(super) fn density_icons_columns(density: BrowserDensity) -> u32 {
    if density.px() < BrowserDensity::AIRY_PX {
        20
    } else {
        16
    }
}

fn icons_loading_skeleton(thumbnail_size: i32, density: BrowserDensity) -> gtk::Box {
    let skeleton = loading_skeleton::container();
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        item.set_child(Some(&skeleton_card(thumbnail_size)));
    });
    factory.connect_bind(|_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        if let Some(label) = item.child().and_then(|card| card.last_child()) {
            label.set_width_request(loading_skeleton::name_width(item.position()));
        }
    });
    let model = gtk::StringList::new(&[""; 60]);
    let icons = gtk::GridView::new(Some(gtk::NoSelection::new(Some(model))), Some(factory));
    icons.add_css_class("file-icons");
    icons.set_valign(gtk::Align::Start);
    configure_icons_view_density(&icons, density);
    let scroll = loading_skeleton::scroll(&icons);
    scroll.set_vexpand(true);
    skeleton.append(&scroll);
    skeleton
}

fn compact_loading_skeleton(icon_size: i32) -> gtk::Box {
    let skeleton = loading_skeleton::container();
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let cell = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        cell.add_css_class("icons-card");
        cell.add_css_class("compact-card");
        cell.append(&loading_skeleton::block(icon_size, icon_size));
        let name = loading_skeleton::block(96, 10);
        name.set_valign(gtk::Align::Center);
        cell.append(&name);
        item.set_child(Some(&cell));
    });
    factory.connect_bind(|_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        if let Some(name) = item.child().and_then(|cell| cell.last_child()) {
            name.set_width_request(loading_skeleton::name_width(item.position()));
        }
    });
    let model = gtk::StringList::new(&[""; 60]);
    let grid = gtk::GridView::new(Some(gtk::NoSelection::new(Some(model))), Some(factory));
    grid.add_css_class("file-icons");
    configure_compact_view(&grid);
    let scroll = loading_skeleton::scroll(&grid);
    scroll.set_vexpand(true);
    skeleton.append(&scroll);
    skeleton
}

fn skeleton_card(thumbnail_size: i32) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.add_css_class("icons-card");
    card.set_halign(gtk::Align::Fill);
    ensure_icons_card_slot(&card, thumbnail_size);
    let slot = icons_card_icon_slot(thumbnail_size);
    let icon = loading_skeleton::block(slot, slot);
    icon.set_halign(gtk::Align::Center);
    let padding = icons_cell::ICONS_CARD_ICON_PADDING;
    icon.set_margin_top(padding);
    icon.set_margin_bottom(padding);
    icon.set_margin_start(padding);
    icon.set_margin_end(padding);
    card.append(&icon);
    let label = loading_skeleton::block(96, 10);
    label.set_halign(gtk::Align::Center);
    label.set_margin_top(3);
    card.append(&label);
    card
}

impl ModeViews {
    /// The Icons slider and its folder, while Icons shows.
    pub(in crate::ui) fn icons_thumbnail_scale(&self) -> Option<(gtk::Scale, Option<Location>)> {
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
}
