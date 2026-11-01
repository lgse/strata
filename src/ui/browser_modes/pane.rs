// SPDX-License-Identifier: MIT

//! The pane an Icons or List presentation shows for one directory, and the
//! chrome, models, and item bookkeeping both modes share.

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

use gtk::{gio, glib, prelude::*};

use super::{
    BrowserMode, ClickActivation, TransferHandlerSlot,
    filter::PaneFilter,
    icons_pane::IconsContext,
    list_columns::ListColumnLayout,
    list_headings::ListSorting,
    list_pane::list_row_parts,
    source_index::{PanePositions, SourceIndexMap},
    type_groups::TypeGroupSorter,
};
use crate::{
    app::{Browser, BrowserColumnSnapshot},
    model::Location,
    ui::{
        browser::{ClipboardMark, ClipboardMarks, ViewState, mark_in},
        collection_edit::EditWidgets,
        inline_search::{InlineSearch, SearchSelectionHandlers},
        loading_skeleton, marquee,
    },
};

/// The page a List or Icons pane shows for an empty or unreadable directory.
pub(super) const STATUS_PAGE: &str = "status";

pub(super) struct BoundModeItem {
    pub(super) item: glib::WeakRef<gtk::ListItem>,
    pub(super) widget: glib::WeakRef<gtk::Widget>,
    pub(super) rename_label: glib::WeakRef<gtk::Widget>,
    pub(super) edit: EditWidgets,
}

pub(super) type BoundItems = Rc<RefCell<Vec<BoundModeItem>>>;

/// One collection view inside a pane. Icons and List each keep a single section.
#[derive(Clone)]
pub(super) struct PaneSection {
    pub(super) view: gtk::Widget,
    pub(super) view_model: gio::ListModel,
    pub(super) view_index: SourceIndexMap,
    pub(super) selection: gtk::MultiSelection,
    pub(super) bound_items: BoundItems,
    pub(super) syncing: Rc<Cell<bool>>,
    pub(super) visit: marquee::ItemVisitor,
    pub(super) item_context_trigger: Rc<dyn Fn(f64, f64)>,
}

impl PaneSection {
    pub(super) fn source_to_view(&self, source: &gtk::StringList, position: usize) -> Option<u32> {
        self.view_index
            .of_item(&source.item(position as u32)?)
            .map(|position| position as u32)
    }

    /// The rendered widget of the item at `position`, when it is mapped inside the view.
    pub(super) fn mapped_item_widget(&self, position: u32) -> Option<gtk::Widget> {
        self.bound_items.borrow().iter().find_map(|bound| {
            (bound.item.upgrade()?.position() == position)
                .then(|| bound.widget.upgrade())
                .flatten()
                .filter(|row| row.is_mapped() && row.is_ancestor(&self.view))
        })
    }

    /// The rendered widget of the item at `position`, mapped or not.
    pub(super) fn item_widget(&self, position: u32) -> Option<gtk::Widget> {
        self.bound_items.borrow().iter().find_map(|bound| {
            let item = bound.item.upgrade()?;
            (item.position() == position).then(|| bound.widget.upgrade())?
        })
    }
}

#[derive(Clone)]
pub(super) struct Pane {
    pub(super) depth: usize,
    pub(super) location: Option<Location>,
    pub(super) group_by_type: bool,
    pub(super) type_group_sorter: Option<TypeGroupSorter>,
    pub(super) sorting: Option<ListSorting>,
    pub(super) sort_direction_button: Option<gtk::Button>,
    pub(super) shell: gtk::Box,
    pub(super) header: gtk::Box,
    pub(super) model: gtk::StringList,
    pub(super) source_index: SourceIndexMap,
    pub(super) filter_model: Option<gtk::FilterListModel>,
    /// The primary section that owns the pane's chrome.
    pub(super) section: PaneSection,
    pub(super) sections: Rc<RefCell<Vec<PaneSection>>>,
    pub(super) icons: Option<Rc<IconsContext>>,
    pub(super) thumbnail_scale: Option<gtk::Scale>,
    pub(super) targets: marquee::MarqueeTargets,
    /// Set while a reload has detached the pane's models from their views.
    pub(super) detached: Rc<Cell<bool>>,
    pub(super) stack: gtk::Stack,
    pub(super) loading: loading_skeleton::DelayedLoading,
    pub(super) status: gtk::Label,
    /// Shown under `status` when only permissions kept the user out of the folder.
    pub(super) administrator: gtk::Button,
    pub(super) spinner: gtk::Spinner,
    pub(super) truncated_hint: gtk::Image,
    pub(super) not_responding: gtk::Box,
    pub(super) marquee: marquee::Marquee,
    pub(super) search: InlineSearch,
    pub(super) filter_entry: Option<gtk::Entry>,
    pub(super) filter_button: Option<gtk::ToggleButton>,
    pub(super) empty_trash_button: Option<gtk::Button>,
    pub(super) leftovers_button: gtk::Button,
    pub(super) show_hidden: Rc<Cell<bool>>,
    pub(super) filter: gtk::CustomFilter,
    pub(super) filter_query: Rc<RefCell<String>>,
    pub(super) folder_context_trigger: Rc<dyn Fn(f64, f64)>,
    pub(super) list_headings: Option<gtk::Box>,
    pub(super) list_columns: Option<ListColumnLayout>,
}

impl Pane {
    /// The sections that render entries, in visual order.
    pub(super) fn item_sections(&self) -> Vec<PaneSection> {
        self.sections.borrow().clone()
    }

    /// Include the primary section even while the item sections are empty.
    pub(super) fn all_sections(&self) -> Vec<PaneSection> {
        let mut sections = self.item_sections();
        if !sections
            .iter()
            .any(|section| section.view == self.section.view)
        {
            sections.push(self.section.clone());
        }
        sections
    }

    pub(super) fn focus_view(&self) -> gtk::Widget {
        self.item_sections()
            .first()
            .map_or_else(|| self.section.view.clone(), |section| section.view.clone())
    }

    /// The view and position the browser's cursor at `source` shows in.
    pub(super) fn cursor_view(&self, source: usize) -> Option<(gtk::Widget, u32)> {
        self.item_sections().into_iter().find_map(|section| {
            let position = section.source_to_view(&self.model, source)?;
            (position < section.view_model.n_items()).then_some((section.view, position))
        })
    }

    /// Whether this pane already presents `location` at `depth`.
    pub(super) fn shows(&self, depth: usize, location: &Location) -> bool {
        self.depth == depth && self.location.as_ref() == Some(location)
    }

    /// The section view position of a directory entry, with its section.
    pub(super) fn view_position_of(&self, source_position: usize) -> Option<(PaneSection, u32)> {
        self.item_sections().into_iter().find_map(|section| {
            let position = super::source_index::view_position_for_source(
                &self.model,
                Some(&section.view_model),
                source_position,
            )?;
            Some((section, position))
        })
    }

    /// The directory entry a rendered item belongs to, when the browser still lists it.
    pub(super) fn bound_entry(
        &self,
        browser: &Browser,
        item: &gtk::ListItem,
    ) -> Option<crate::model::FileEntry> {
        let position = item
            .item()
            .and_then(|value| self.source_index.of_item(&value))?;
        browser.entry_at(self.depth, position)
    }

    pub(super) fn assemble(base: PaneBase, filter: PaneFilter, listing: PaneListing) -> Self {
        Self {
            depth: base.depth,
            location: base.location,
            group_by_type: false,
            type_group_sorter: None,
            sorting: None,
            sort_direction_button: None,
            shell: base.shell,
            header: base.header,
            model: base.model,
            source_index: base.source_index,
            filter_model: Some(filter.model),
            section: listing.section,
            sections: listing.sections,
            icons: None,
            thumbnail_scale: None,
            targets: listing.targets,
            detached: Rc::new(Cell::new(false)),
            loading: loading_skeleton::DelayedLoading::new(&base.stack),
            stack: base.stack,
            status: base.status,
            administrator: base.administrator,
            spinner: base.spinner,
            truncated_hint: base.truncated_hint,
            not_responding: base.not_responding,
            marquee: listing.marquee,
            search: listing.search,
            filter_entry: Some(filter.entry),
            filter_button: Some(filter.button),
            empty_trash_button: listing.empty_trash_button,
            leftovers_button: listing.leftovers_button,
            show_hidden: filter.show_hidden,
            filter: filter.filter,
            filter_query: filter.query,
            folder_context_trigger: Rc::new(|_, _| {}),
            list_headings: None,
            list_columns: None,
        }
    }
}

#[derive(Clone)]
pub(super) struct ModeClickOptions {
    pub(super) previews: Rc<Cell<bool>>,
    pub(super) activation: Rc<Cell<ClickActivation>>,
    pub(super) multiple_selection: Rc<Cell<bool>>,
}

/// What every mode pane is built from.
pub(super) struct PaneBuild {
    pub(super) browser: Rc<Browser>,
    pub(super) depth: usize,
    pub(super) title: String,
    pub(super) click: ModeClickOptions,
    pub(super) transfer: TransferHandlerSlot,
    pub(super) marks: Rc<RefCell<ClipboardMarks>>,
    pub(super) state: Option<Weak<ViewState>>,
    pub(super) new_folder_state: Option<Weak<ViewState>>,
    pub(super) search_selection_handlers: SearchSelectionHandlers,
    pub(super) search_context_state: Rc<RefCell<Option<Weak<ViewState>>>>,
}

impl PaneBuild {
    pub(super) fn location(&self) -> Option<Location> {
        self.browser.location_at(self.depth)
    }

    pub(super) fn weak_browser(&self) -> Weak<Browser> {
        Rc::downgrade(&self.browser)
    }
}

/// The parts of a mode pane's shell that differ between Icons and List.
pub(super) struct PaneChrome<'a> {
    pub(super) depth: usize,
    pub(super) location: Option<Location>,
    pub(super) title: &'a str,
    pub(super) mode: BrowserMode,
    pub(super) class: &'static str,
    pub(super) loading: gtk::Box,
    pub(super) leading: gtk::Widget,
    pub(super) actions: gtk::Widget,
}

/// The shell widgets and directory model a mode pane is assembled around.
pub(super) struct PaneBase {
    pub(super) depth: usize,
    pub(super) location: Option<Location>,
    pub(super) shell: gtk::Box,
    pub(super) header: gtk::Box,
    pub(super) content: gtk::Box,
    pub(super) model: gtk::StringList,
    pub(super) source_index: SourceIndexMap,
    pub(super) stack: gtk::Stack,
    pub(super) status: gtk::Label,
    pub(super) administrator: gtk::Button,
    pub(super) spinner: gtk::Spinner,
    pub(super) truncated_hint: gtk::Image,
    pub(super) not_responding: gtk::Box,
}

impl PaneBase {
    /// Adds the chrome around the listing and wires the pane surface as a drop zone.
    pub(super) fn prepare_listing(&self, inputs: &PaneBuild, filter_revealer: &gtk::Revealer) {
        if self.location.as_ref().is_some_and(Location::is_administrator) {
            self.shell.insert_child_after(
                &crate::ui::browser::administrator_notice(),
                Some(&self.not_responding),
            );
        }
        if let Some(destination) = inputs.location() {
            super::drag_drop::install_mode_directory_drop_target(
                &self.stack,
                destination,
                inputs.transfer.clone(),
            );
        }
        self.content.append(filter_revealer);
    }
}

/// The listing a pane shows and the chrome actions tied to its contents.
pub(super) struct PaneListing {
    pub(super) section: PaneSection,
    pub(super) sections: Rc<RefCell<Vec<PaneSection>>>,
    pub(super) targets: marquee::MarqueeTargets,
    pub(super) marquee: marquee::Marquee,
    pub(super) search: InlineSearch,
    pub(super) leftovers_button: gtk::Button,
    pub(super) empty_trash_button: Option<gtk::Button>,
}

pub(super) fn pane_base(chrome: PaneChrome) -> PaneBase {
    let shell = crate::ui::accessibility::pane_box();
    shell.add_css_class(chrome.class);
    shell.set_hexpand(true);
    shell.set_vexpand(true);
    crate::ui::accessibility::describe_pane(&shell, chrome.title, chrome.mode);
    let (header, spinner, truncated_hint) = pane_header(&chrome);
    shell.append(&header);
    let not_responding = crate::ui::browser::not_responding_notice();
    shell.append(&not_responding);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.set_hexpand(true);
    content.set_vexpand(true);
    let (status_page, status, administrator) = pane_status_page();
    // Focusable only off the content page (see `DirectorySurface`); a group role keeps its name.
    let stack = gtk::Stack::builder()
        .hexpand(true)
        .vexpand(true)
        .accessible_role(gtk::AccessibleRole::Group)
        .build();
    stack.add_named(&content, Some(loading_skeleton::CONTENT_PAGE));
    stack.add_named(&chrome.loading, Some(loading_skeleton::LOADING_PAGE));
    stack.add_named(&status_page, Some(STATUS_PAGE));
    shell.append(&stack);

    let model = gtk::StringList::new(&[]);
    let source_index = SourceIndexMap::watch(&model);
    PaneBase {
        depth: chrome.depth,
        location: chrome.location,
        shell,
        header,
        content,
        model,
        source_index,
        stack,
        status,
        administrator,
        spinner,
        truncated_hint,
        not_responding,
    }
}

fn pane_header(chrome: &PaneChrome) -> (gtk::Box, gtk::Spinner, gtk::Image) {
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    header.add_css_class("mode-pane-header");
    crate::ui::tenxer_mode::hide_while_enabled(&header);
    let heading_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    heading_box.set_hexpand(true);
    heading_box.set_valign(gtk::Align::Center);
    let heading = gtk::Label::new(Some(chrome.title));
    heading.set_xalign(0.0);
    heading.set_hexpand(true);
    heading.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    heading.set_max_width_chars(1);
    crate::ui::accessibility::set_description(&heading, Some(chrome.title));
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
    chrome.leading.set_valign(gtk::Align::Center);
    header.append(&chrome.leading);
    header.append(&heading_box);
    header.append(&spinner);
    chrome.actions.set_valign(gtk::Align::Center);
    header.append(&chrome.actions);
    (header, spinner, truncated_hint)
}

fn pane_status_page() -> (gtk::Box, gtk::Label, gtk::Button) {
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
    (status_page, status, administrator)
}

pub(super) fn pane_directory_name(browser: &Rc<Browser>, depth: usize) -> String {
    browser
        .location_at(depth)
        .map(|location| location.display_name())
        .unwrap_or_default()
}

pub(super) fn install_pane_surface(pane: &Pane, browser: &Rc<Browser>, directory: &str) {
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
    loading_skeleton::DirectorySurface::install(
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
pub(super) fn focus_pane_surface(pane: &Pane) -> bool {
    loading_skeleton::focus_surface_or(&pane.stack, &pane.focus_view())
}

pub(super) fn install_tab_landing(view: &gtk::Widget, state: Option<Weak<ViewState>>) {
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

/// Opens the entry behind an activated view position in place.
pub(super) struct CollectionActivation {
    pub(super) browser: Weak<Browser>,
    pub(super) state: Option<Weak<ViewState>>,
    pub(super) positions: PanePositions,
    pub(super) depth: usize,
}

impl CollectionActivation {
    pub(super) fn new(inputs: &PaneBuild, positions: PanePositions) -> Self {
        Self {
            browser: inputs.weak_browser(),
            state: inputs.state.clone(),
            positions,
            depth: inputs.depth,
        }
    }

    pub(super) fn activate(&self, position: u32) {
        if let Some(state) = self.state.as_ref().and_then(Weak::upgrade) {
            state.cancel_click_rename();
        }
        if let Some(browser) = self.browser.upgrade()
            && let Some(position) = self.positions.source_position(position)
        {
            browser.activate_in_place(self.depth, position);
        }
    }
}

pub(super) fn bound_edit(item: &gtk::ListItem) -> Option<EditWidgets> {
    crate::ui::collection_edit::item_edit(item)
}

pub(super) fn install_edit_unbind(factory: &gtk::SignalListItemFactory, items: &BoundItems) {
    factory.connect_unbind(move |_, object| {
        if let Some(item) = object.downcast_ref::<gtk::ListItem>()
            && let Some(edit) = bound_edit(item)
        {
            edit.unbind();
        }
        crate::ui::thumbnail::cancel_list_item_thumbnails(object);
    });
    let teardown = items.clone();
    factory.connect_teardown(move |_, object| {
        crate::ui::collection_edit::forget_item_edit(object);
        teardown.borrow_mut().retain(|bound| {
            bound
                .item
                .upgrade()
                .is_some_and(|item| item.upcast_ref::<glib::Object>() != object)
        });
    });
}

pub(super) fn register_bound_mode_item(
    items: &BoundItems,
    item: &gtk::ListItem,
    widget: &impl IsA<gtk::Widget>,
    rename_label: &impl IsA<gtk::Widget>,
    edit: EditWidgets,
) {
    let weak_item = glib::WeakRef::new();
    weak_item.set(Some(item));
    let weak_widget = glib::WeakRef::new();
    weak_widget.set(Some(widget.upcast_ref()));
    let weak_rename_label = glib::WeakRef::new();
    weak_rename_label.set(Some(rename_label.upcast_ref()));
    crate::ui::collection_edit::register_item_edit(item, &edit);
    items.borrow_mut().push(BoundModeItem {
        item: weak_item,
        widget: weak_widget,
        rename_label: weak_rename_label,
        edit,
    });
}

pub(super) fn bound_item_visitor(bound_items: BoundItems) -> marquee::ItemVisitor {
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

pub(super) fn refresh_marquee_targets(pane: &Pane) {
    *pane.targets.borrow_mut() = pane
        .sections
        .borrow()
        .iter()
        .map(|section| marquee::MarqueeTarget {
            selection: section.selection.clone(),
            visit_items: section.visit.clone(),
        })
        .collect();
}

/// Wraps a collection view's scroller in the overlay a marquee drag draws on.
pub(super) fn collection_with_marquee(
    view: &gtk::Widget,
    scroll: gtk::ScrolledWindow,
    list_rows: bool,
    multiple_selection: Rc<Cell<bool>>,
) -> (gtk::Overlay, marquee::Marquee, marquee::MarqueeTargets) {
    let targets: marquee::MarqueeTargets = Rc::new(RefCell::new(Vec::new()));
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&scroll));
    overlay.set_hexpand(true);
    overlay.set_vexpand(true);
    crate::ui::scrolling::install_autoscroll(&scroll, &overlay);

    let content: marquee::ItemPredicate = if list_rows {
        Rc::new(crate::ui::pointer::hits_list_item_content)
    } else {
        Rc::new(crate::ui::pointer::hits_item_content)
    };
    let is_item = marquee::item_content_predicate(targets.clone(), content);
    let clear_targets = targets.clone();
    let marquee = marquee::install(marquee::MarqueeSetup {
        view: view.clone(),
        surface: scroll.clone().upcast(),
        scroll,
        overlay: overlay.clone(),
        targets: targets.clone(),
        is_item,
        clear_selection: Rc::new(move || {
            let selections: Vec<_> = clear_targets
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
    (overlay, marquee, targets)
}

pub(super) fn set_mode_mark_style(widget: &impl IsA<gtk::Widget>, mark: ClipboardMark) {
    if mark == ClipboardMark::Cut {
        widget.add_css_class("cut");
    } else {
        widget.remove_css_class("cut");
    }
    if let Some((icon, _)) = crate::ui::icons_cell::parts(widget) {
        icon.set_mark(mark);
    } else if let Some((icon, ..)) = widget
        .upcast_ref()
        .downcast_ref::<gtk::Box>()
        .and_then(list_row_parts)
    {
        icon.set_mark(mark);
    }
}

pub(super) fn refresh_mark_pane(pane: &Pane, browser: &Browser, marks: &ClipboardMarks) {
    pane.search.refresh_mark_rows();
    for section in pane.item_sections() {
        section.bound_items.borrow_mut().retain(|bound| {
            let (Some(item), Some(widget)) = (bound.item.upgrade(), bound.widget.upgrade()) else {
                return false;
            };
            let mark = pane
                .bound_entry(browser, &item)
                .map_or(ClipboardMark::None, |entry| mark_in(marks, &entry.location));
            set_mode_mark_style(&widget, mark);
            true
        });
    }
}

pub(super) fn replace_entries(pane: &Pane, browser: &Browser, count: usize) {
    let values = browser
        .with_entries(pane.depth, 0..count, |entries| {
            entries
                .iter()
                .map(crate::ui::browser::entry_model_value)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    pane.splice_values(0, pane.model.n_items(), &values);
}

pub(super) fn detach_pane_models(pane: &Pane) {
    pane.detached.set(true);
    pane.search.detach();
    for section in pane.all_sections() {
        section.syncing.set(true);
        section.selection.set_model(None::<&gio::ListModel>);
        crate::ui::browser::detach_collection_view(&section.view);
    }
    if let Some(filtered) = pane.filter_model.as_ref() {
        filtered.set_model(None::<&gio::ListModel>);
    }
}

pub(super) fn deactivate_pane_models(pane: &Pane) {
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

pub(super) fn reconnect_pane_model(pane: &Pane) {
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

pub(super) fn show_count(pane: &Pane) {
    show_count_controls(pane);
    if pane.model.n_items() == 0 {
        pane.status.remove_css_class("error");
        pane.status
            .set_label(&crate::i18n::tr("This directory is empty"));
        pane.administrator.set_visible(false);
        pane.loading.show(STATUS_PAGE);
    } else {
        pane.loading.show(loading_skeleton::CONTENT_PAGE);
    }
}

pub(super) fn show_count_controls(pane: &Pane) {
    let count = pane.model.n_items();
    if let Some(entry) = &pane.filter_entry {
        entry.set_placeholder_text(Some(&crate::ui::browser::filter_placeholder(
            count as usize,
        )));
    }
    if let Some(button) = &pane.empty_trash_button {
        button.set_sensitive(count > 0);
    }
}

pub(super) fn apply_snapshot(pane: &Pane, snapshot: &BrowserColumnSnapshot, browser: &Browser) {
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
    super::selection::set_selections(pane, &snapshot.selected_positions);
    if let Some(&focused) = snapshot.selected_positions.last() {
        super::selection::scroll_pane_to_source(pane, focused);
    }
    pane.truncated_hint.set_visible(snapshot.truncated);
    pane.leftovers_button
        .set_visible(!browser.column_leftovers(pane.depth).is_empty());
    pane.not_responding.set_visible(snapshot.not_responding);
    show_snapshot_progress(pane, snapshot, restarts_loading);
}

fn show_snapshot_progress(pane: &Pane, snapshot: &BrowserColumnSnapshot, restarts_loading: bool) {
    if snapshot.loading {
        pane.spinner.start();
        if restarts_loading {
            pane.loading.start();
        }
        return;
    }
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

pub(super) fn clear_box(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}
