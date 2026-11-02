// SPDX-License-Identifier: MIT

//! The List presentation: a table of rows under resizable, reorderable headings.

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

use gtk::{gio, prelude::*};

use super::{
    BrowserMode, column_options,
    context_menu::install_section_context_menu,
    drag_drop::install_mode_directory_drop_target,
    filter::{PaneFilter, attach_in_place, filter_controls},
    grouped_model::GroupedModel,
    list_columns::{ListColumnLayout, column_shown, register_list_column_cell},
    list_factory::{ListFactory, refresh_list_section},
    list_headings::{ListSorting, list_headings},
    pane::{
        BoundItems, CollectionActivation, Pane, PaneBuild, PaneChrome, PaneListing, PaneSection,
        bound_item_visitor, collection_with_marquee, install_tab_landing, pane_base,
        pane_directory_name, refresh_marquee_targets,
    },
    pane_header::{ContentActions, list_navigation},
    scroll_settle::install_scroll_refresh,
    selection::{SectionOwner, connect_selection},
    source_index::{PanePositions, SourceIndexMap},
    type_groups::{TypeGroupSorter, pane_type_group_sorter, type_group_header_factory},
};
use crate::ui::{
    browser,
    inline_search::SearchPresentation,
    loading_skeleton,
    preferences::{ListColumnId, PreferenceManager},
    thumbnail::ThumbnailSlot,
};

/// The headings and column layout a List pane's rows follow.
struct ListTable {
    navigation: gtk::Box,
    columns: ListColumnLayout,
    bound_items: BoundItems,
    headings: gtk::Box,
    sorting: ListSorting,
}

impl ListTable {
    fn new(inputs: &PaneBuild) -> Self {
        let navigation = list_navigation(&inputs.browser);
        let columns = ListColumnLayout::new(&inputs.browser);
        let bound_items: BoundItems = Rc::new(RefCell::new(Vec::new()));
        let (headings, sorting) =
            list_headings(&inputs.browser, inputs.depth, columns.clone(), &bound_items);
        Self {
            navigation,
            columns,
            bound_items,
            headings,
            sorting,
        }
    }

    /// A secondary click on the headings opens the column options.
    fn install_options_menu(&self) {
        let weak_headings = self.headings.downgrade();
        let columns = self.columns.clone();
        let bound_items = self.bound_items.clone();
        let click = gtk::GestureClick::new();
        click.set_button(3);
        click.connect_pressed(move |gesture, _, x, y| {
            let Some(headings) = weak_headings.upgrade() else {
                return;
            };
            column_options::open_column_options_popover(
                &headings,
                &headings,
                &columns,
                &bound_items,
                Some((x, y)),
            );
            gesture.set_state(gtk::EventSequenceState::Claimed);
        });
        self.headings.add_controller(click);
    }
}

/// The List header's action buttons.
struct ListActions {
    widget: gtk::Box,
    content: ContentActions,
    sort_menu: gtk::MenuButton,
}

impl ListActions {
    fn new(inputs: &PaneBuild, table: &ListTable) -> Self {
        let browser = &inputs.browser;
        let depth = inputs.depth;
        let widget = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        widget.add_css_class("icons-header-actions");
        let content = ContentActions::append(&widget, browser, depth);
        if let Some(state) = inputs.new_folder_state.as_ref() {
            widget.append(&browser::pane_new_folder_button(state.clone(), depth));
        }
        widget.append(&browser::pane_refresh_button(browser, depth));
        let sort_menu = browser::list_sort_menu(browser, depth);
        browser::sync_list_sort_menu(browser, depth, &sort_menu);
        let actions = Self {
            widget,
            content,
            sort_menu,
        };
        actions.append_sort_controls(inputs);
        actions.append_columns_menu(table);
        actions
    }

    fn append_sort_controls(&self, inputs: &PaneBuild) {
        if inputs
            .location()
            .is_some_and(|location| location.is_recent_root())
        {
            self.widget.append(&browser::column_sort_direction_toggle(
                &inputs.browser,
                inputs.depth,
            ));
        }
        self.widget.append(&self.sort_menu);
    }

    fn append_columns_menu(&self, table: &ListTable) {
        let columns_popover = column_options::create_column_options_popover(
            &table.headings,
            &table.columns,
            &table.bound_items,
        );
        let columns_menu = gtk::MenuButton::builder().popover(&columns_popover).build();
        crate::ui::accessibility::set_tooltip_and_label(&columns_menu, &crate::i18n::tr("Columns"));
        columns_menu.set_child(Some(&crate::assets::chrome_icon(
            crate::assets::icons::COLUMNS,
        )));
        crate::ui::controls::pane_header_action(&columns_menu);
        crate::ui::tenxer_mode::hide_while_enabled(&columns_menu);
        self.widget.append(&columns_menu);
    }
}

pub(super) fn build_list_pane(inputs: &PaneBuild, group_by_type: bool) -> Pane {
    let table = ListTable::new(inputs);
    let actions = ListActions::new(inputs, &table);
    let (filter_entry, filter_revealer, filter_button) = filter_controls("Filter list (%{keys})");
    actions.widget.append(&filter_button);
    let base = pane_base(PaneChrome {
        depth: inputs.depth,
        location: inputs.location(),
        title: &inputs.title,
        mode: BrowserMode::List,
        class: "list-pane",
        loading: list_loading_skeleton(&table.columns),
        leading: table.navigation.clone().upcast(),
        actions: actions.widget.clone().upcast(),
    });
    base.prepare_listing(inputs, &filter_revealer);
    let filter = PaneFilter::bind(filter_entry, filter_button, inputs, &base.model);
    let type_group_sorter =
        group_by_type.then(|| pane_type_group_sorter(&inputs.browser, inputs.depth));
    let listing = ListListing {
        inputs,
        table: &table,
        filter: &filter,
        source_index: &base.source_index,
        group_by_type,
    }
    .build(type_group_sorter.as_ref(), &base.header, actions.content);
    base.content.append(&listing.search.widget);
    let mut pane = Pane::assemble(base, filter, listing);
    pane.group_by_type = group_by_type;
    pane.type_group_sorter = type_group_sorter;
    pane.sorting = Some(table.sorting);
    pane.sort_menu = Some(actions.sort_menu);
    pane.list_headings = Some(table.headings);
    pane.list_columns = Some(table.columns);
    refresh_marquee_targets(&pane);
    pane
}

/// Builds a List pane's rows, their view, and the table around them.
struct ListListing<'a> {
    inputs: &'a PaneBuild,
    table: &'a ListTable,
    filter: &'a PaneFilter,
    source_index: &'a SourceIndexMap,
    group_by_type: bool,
}

impl ListListing<'_> {
    fn build(
        &self,
        sorter: Option<&TypeGroupSorter>,
        header: &gtk::Box,
        content: ContentActions,
    ) -> PaneListing {
        let view_model = GroupedModel::new(&self.filter.model, sorter.map(|sorter| &sorter.sorter));
        let view_model = view_model.upcast::<gio::ListModel>();
        let view_index = SourceIndexMap::watch(&view_model);
        let selection = gtk::MultiSelection::new(Some(view_model.clone()));
        let syncing = Rc::new(Cell::new(false));
        let sections: Rc<RefCell<Vec<PaneSection>>> = Rc::new(RefCell::new(Vec::new()));
        self.table.install_options_menu();
        let scrolling = Rc::new(Cell::new(false));
        let positions = PanePositions {
            index: self.source_index.clone(),
            view: view_model.clone(),
        };
        let factory = self.factory(positions.clone(), &selection, &syncing, &scrolling);
        let view = self.list_view(&selection, &factory, positions);
        let mut section = PaneSection {
            view: view.clone().upcast(),
            view_index,
            view_model,
            selection,
            bound_items: self.table.bound_items.clone(),
            syncing,
            visit: bound_item_visitor(self.table.bound_items.clone()),
            item_context_trigger: Rc::new(|_, _| {}),
        };
        self.filter
            .connect_entry(&section, self.inputs, self.source_index);
        let owner = SectionOwner {
            sections: Rc::downgrade(&sections),
            depth: self.inputs.depth,
            source_index: self.source_index.clone(),
        };
        connect_selection(
            &section,
            owner.clone(),
            &self.inputs.browser,
            self.inputs.click.multiple_selection.clone(),
        );
        if let Some(state) = self.inputs.state.as_ref().and_then(Weak::upgrade) {
            section.item_context_trigger = install_section_context_menu(&state, &section, &owner);
        }
        sections.borrow_mut().push(section.clone());
        let scroll = self.listing_scroll(&view, &section, scrolling, &sections);
        let (table_scroll, marquee, targets) = self.table_scroll(&view, scroll, header);
        let search = self
            .filter
            .wrap_search(&table_scroll, self.inputs, SearchPresentation::Rows);
        PaneListing {
            section,
            sections,
            targets,
            marquee,
            search,
            leftovers_button: content.leftovers,
            empty_trash_button: content.empty_trash,
        }
    }

    fn factory(
        &self,
        positions: PanePositions,
        selection: &gtk::MultiSelection,
        syncing: &Rc<Cell<bool>>,
        scrolling: &Rc<Cell<bool>>,
    ) -> gtk::SignalListItemFactory {
        ListFactory {
            browser: self.inputs.weak_browser(),
            depth: self.inputs.depth,
            positions,
            selection: selection.clone(),
            syncing: syncing.clone(),
            previews: self.inputs.click.previews.clone(),
            activation: self.inputs.click.activation.clone(),
            transfers: self.inputs.transfer.clone(),
            marks: self.inputs.marks.clone(),
            columns: self.table.columns.clone(),
            scrolling: scrolling.clone(),
            bound_items: self.table.bound_items.clone(),
            state: self.inputs.state.clone(),
            filter_query: self.filter.query.clone(),
            enter_target: self.filter.enter_target.clone(),
        }
        .build()
    }

    fn list_view(
        &self,
        selection: &gtk::MultiSelection,
        factory: &gtk::SignalListItemFactory,
        positions: PanePositions,
    ) -> gtk::ListView {
        let inputs = self.inputs;
        let view = gtk::ListView::new(Some(selection.clone()), Some(factory.clone()));
        view.add_css_class("file-list-mode");
        crate::ui::accessibility::describe_entry_container(
            &view,
            &pane_directory_name(&inputs.browser, inputs.depth),
        );
        if self.group_by_type {
            view.set_header_factory(Some(&type_group_header_factory()));
        }
        view.set_enable_rubberband(false);
        view.set_vexpand(true);
        // GTK bundles single-click activation with hover selection, which collapses
        // multi-selection. Per-row gestures honor the configured click behavior instead.
        view.set_single_click_activate(false);
        view.set_tab_behavior(gtk::ListTabBehavior::Item);
        install_tab_landing(view.upcast_ref(), inputs.state.clone());
        if let Some(destination) = inputs.location() {
            install_mode_directory_drop_target(&view, destination, inputs.transfer.clone());
        }
        let activation = CollectionActivation::new(inputs, positions);
        view.connect_activate(move |_, position| activation.activate(position));
        view
    }

    fn listing_scroll(
        &self,
        view: &gtk::ListView,
        section: &PaneSection,
        scrolling: Rc<Cell<bool>>,
        sections: &Rc<RefCell<Vec<PaneSection>>>,
    ) -> gtk::ScrolledWindow {
        let scroll = gtk::ScrolledWindow::builder()
            .child(view)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        scroll.add_css_class("browser-listing-scroll");
        scroll.add_css_class("list-listing-scroll");
        if let Some(enter_target) = &self.filter.enter_target {
            attach_in_place(enter_target, section, &scroll);
        }
        let browser = self.inputs.weak_browser();
        let depth = self.inputs.depth;
        let source_index = self.source_index.clone();
        let sections = Rc::downgrade(sections);
        let marks = self.inputs.marks.clone();
        install_scroll_refresh(&scroll, scrolling, move || {
            let Some(browser) = browser.upgrade() else {
                return;
            };
            let Some(sections) = sections.upgrade() else {
                return;
            };
            let marks = marks.borrow();
            for section in sections.borrow().iter() {
                refresh_list_section(&browser, depth, &source_index, section, &marks);
            }
        });
        scroll
    }

    /// The headings above the rows, scrolling sideways together.
    fn table_scroll(
        &self,
        view: &gtk::ListView,
        scroll: gtk::ScrolledWindow,
        header: &gtk::Box,
    ) -> (
        gtk::ScrolledWindow,
        crate::ui::marquee::Marquee,
        crate::ui::marquee::MarqueeTargets,
    ) {
        let table = gtk::Box::new(gtk::Orientation::Vertical, 0);
        table.set_vexpand(true);
        table.append(&self.table.headings);
        let (collection, marquee, targets) = collection_with_marquee(
            view.upcast_ref(),
            scroll,
            true,
            self.inputs.click.multiple_selection.clone(),
        );
        table.append(&collection);
        marquee.add_origin_surface(header);
        marquee.add_origin_surface(&self.table.headings);
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
        (table_scroll, marquee, targets)
    }
}

fn skeleton_heading_width(col: ListColumnId) -> i32 {
    match col {
        ListColumnId::Name => 40,
        ListColumnId::Mode => 36,
        ListColumnId::Size => 28,
        ListColumnId::Type => 30,
        ListColumnId::Modified => 58,
    }
}

fn skeleton_row_width(col: ListColumnId, row: u32) -> i32 {
    match col {
        ListColumnId::Name => loading_skeleton::name_width(row).min(92),
        ListColumnId::Mode => 60,
        ListColumnId::Size => 38,
        ListColumnId::Type => 56,
        ListColumnId::Modified => 94,
    }
}

/// The column order and hidden columns the skeleton lays out.
struct SkeletonColumns<'a> {
    columns: &'a ListColumnLayout,
    order: Vec<ListColumnId>,
    hidden: Vec<ListColumnId>,
}

impl SkeletonColumns<'_> {
    fn headings(&self) -> gtk::Box {
        let headings = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        headings.add_css_class("list-headings");
        for &col in &self.order {
            let cell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            cell.add_css_class("list-heading-cell");
            if !column_shown(&self.hidden, col) {
                cell.set_visible(false);
            }
            let bar = loading_skeleton::block(skeleton_heading_width(col), 8);
            bar.set_margin_start(12);
            cell.append(&bar);
            register_list_column_cell(self.columns, col.index(), &cell);
            headings.append(&cell);
        }
        headings
    }

    fn row(&self, index: u32) -> gtk::Box {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        row.add_css_class("list-row");
        for &col in &self.order {
            let column = col.index();
            let cell = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            cell.add_css_class("list-metadata-cell");
            if !column_shown(&self.hidden, col) {
                cell.set_visible(false);
            }
            if column == 0 {
                cell.append(&loading_skeleton::block(18, 18));
            }
            cell.append(&loading_skeleton::block(skeleton_row_width(col, index), 10));
            register_list_column_cell(self.columns, column, &cell);
            row.append(&cell);
        }
        row
    }
}

fn list_loading_skeleton(columns: &ListColumnLayout) -> gtk::Box {
    let skeleton = loading_skeleton::container();
    let table = gtk::Box::new(gtk::Orientation::Vertical, 0);
    table.set_valign(gtk::Align::Start);
    let manager = PreferenceManager::shared();
    let layout = SkeletonColumns {
        columns,
        order: manager.list_column_order(),
        hidden: manager.list_hidden_columns(),
    };
    table.append(&layout.headings());
    for index in 0..loading_skeleton::ROW_COUNT {
        table.append(&layout.row(index));
    }
    let scroll = loading_skeleton::scroll(&table);
    scroll.set_vexpand(true);
    skeleton.append(&scroll);
    skeleton
}

pub(super) fn assemble_list_row() -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.add_css_class("list-row");
    let name_cell = list_name_cell();
    let mode_label = list_metadata_label("list-col-mode");
    let size_label = list_metadata_label("list-col-size");
    let kind_label = list_metadata_label("list-col-type");
    let modified_label = list_metadata_label("list-col-modified");

    let manager = PreferenceManager::shared();
    let order = manager.list_column_order();
    let hidden = manager.list_hidden_columns();

    for col in order {
        let widget: gtk::Widget = match col {
            ListColumnId::Name => name_cell.clone().upcast(),
            ListColumnId::Mode => mode_label.clone().upcast(),
            ListColumnId::Size => size_label.clone().upcast(),
            ListColumnId::Type => kind_label.clone().upcast(),
            ListColumnId::Modified => modified_label.clone().upcast(),
        };
        if !column_shown(&hidden, col) {
            widget.set_visible(false);
        }
        row.append(&widget);
    }
    row
}

fn list_name_cell() -> gtk::Box {
    let name_cell = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    name_cell.add_css_class("list-name-cell");
    let icon = ThumbnailSlot::new(18);
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
    crate::ui::accessibility::set_label(&field, &crate::i18n::tr("Rename"));
    field.set_hexpand(true);
    field.set_visible(false);
    name_cell.append(&icon);
    name_cell.append(&name);
    name_cell.append(&field);
    name_cell
}

/// A row's icon, name, rename field, and its mode, size, type, and modified labels.
pub(super) type ListRowParts = (
    ThumbnailSlot,
    gtk::Label,
    gtk::Entry,
    gtk::Label,
    gtk::Label,
    gtk::Label,
    gtk::Label,
);

/// A List row's metadata labels, found by their column classes.
#[derive(Default)]
struct MetadataCells {
    name_cell: Option<gtk::Box>,
    mode: Option<gtk::Label>,
    size: Option<gtk::Label>,
    kind: Option<gtk::Label>,
    modified: Option<gtk::Label>,
}

impl MetadataCells {
    fn collect(row: &gtk::Box) -> Self {
        let mut cells = Self::default();
        let mut child = row.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            cells.take(current);
        }
        cells
    }

    fn take(&mut self, current: gtk::Widget) {
        if current.has_css_class("list-name-cell") {
            self.name_cell = current.downcast::<gtk::Box>().ok();
        } else if current.has_css_class("list-col-mode") {
            self.mode = current.downcast::<gtk::Label>().ok();
        } else if current.has_css_class("list-col-size") {
            self.size = current.downcast::<gtk::Label>().ok();
        } else if current.has_css_class("list-col-type") {
            self.kind = current.downcast::<gtk::Label>().ok();
        } else if current.has_css_class("list-col-modified") {
            self.modified = current.downcast::<gtk::Label>().ok();
        }
    }
}

pub(super) fn list_row_parts(row: &gtk::Box) -> Option<ListRowParts> {
    let cells = MetadataCells::collect(row);
    let name_cell = cells.name_cell?;
    let icon = name_cell.first_child()?.downcast::<ThumbnailSlot>().ok()?;
    let name = icon.next_sibling()?.downcast::<gtk::Label>().ok()?;
    let field = name.next_sibling()?.downcast::<gtk::Entry>().ok()?;
    Some((
        icon,
        name,
        field,
        cells.mode?,
        cells.size?,
        cells.kind?,
        cells.modified?,
    ))
}

fn list_metadata_label(class: &str) -> gtk::Label {
    let label = gtk::Label::new(None);
    label.add_css_class("list-metadata-cell");
    label.set_xalign(0.0);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    // Metadata must truncate rather than overriding a resized column's width.
    label.set_max_width_chars(1);
    label.add_css_class(class);
    label
}
