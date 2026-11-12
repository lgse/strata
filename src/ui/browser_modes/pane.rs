// SPDX-License-Identifier: MIT

//! The pane an Icons or List presentation shows for one directory, its sections,
//! and the bookkeeping of their rendered items.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{gio, glib, prelude::*};

use super::{
    filter::PaneFilter, icons_pane::IconsContext, list_columns::ListColumnLayout,
    list_headings::ListSorting, source_index::SourceIndexMap, type_groups::TypeGroupSorter,
};
use crate::{
    app::Browser,
    model::Location,
    ui::{collection_edit::EditWidgets, inline_search::InlineSearch, loading_skeleton, marquee},
};

pub(super) use super::pane_shell::{
    CollectionActivation, ModeClickOptions, PaneBase, PaneBuild, PaneChrome, PaneListing,
    focus_pane_surface, install_pane_surface, install_tab_landing, pane_base, pane_directory_name,
};
pub(super) use super::pane_state::{
    apply_snapshot, clear_box, deactivate_pane_models, detach_pane_models, reconnect_pane_model,
    refresh_mark_pane, replace_entries, set_mode_mark_style, show_count,
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
    pub(super) sort_menu: Option<gtk::MenuButton>,
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
    pub(super) retry: gtk::Button,
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
            sort_menu: None,
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
            retry: base.retry,
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
    let is_item = marquee::item_content_predicate(targets.clone(), content, !list_rows);
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
