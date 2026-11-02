// SPDX-License-Identifier: MIT

//! Building one column in stages: pane and header, models and query, listing, surface, shell.

use super::{
    ColumnView, arm_column_spinner, lifecycle::animate_column_entry, rows, set_column_busy,
};
use crate::model::Location;
use crate::ui::browser::ViewState;
use crate::ui::browser::collection::FilterQueryBinding;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

mod header;
mod keys;
mod models;
mod query;
mod shell;
mod surface;

use header::ColumnHeader;
use models::ColumnModels;
use shell::ColumnShell;
use surface::{ColumnSurface, ContextTriggers};

fn column_list(
    location: &Location,
    selection: &gtk::MultiSelection,
    factory: gtk::SignalListItemFactory,
) -> gtk::ListView {
    let list = gtk::ListView::new(Some(selection.clone()), Some(factory));
    list.add_css_class("file-list");
    list.set_enable_rubberband(false);
    list.set_single_click_activate(false);
    list.set_tab_behavior(gtk::ListTabBehavior::Item);
    list.set_vexpand(true);
    crate::ui::accessibility::describe_entry_container(&list, &location.display_name());
    list
}

/// Everything a finished column is assembled from.
struct ColumnParts {
    header: ColumnHeader,
    not_responding: gtk::Box,
    models: ColumnModels,
    query_binding: FilterQueryBinding,
    list: gtk::ListView,
    scroll: gtk::ScrolledWindow,
    presentation: crate::ui::browser::presentation::LoadPresentation,
    bound_rows: Rc<RefCell<Vec<super::BoundRow>>>,
    marquee: crate::ui::marquee::Marquee,
    triggers: ContextTriggers,
    shell: ColumnShell,
}

impl ColumnParts {
    fn into_view(self, model_generation: Rc<Cell<u64>>) -> ColumnView {
        let Self {
            header,
            not_responding,
            models,
            query_binding,
            list,
            scroll,
            presentation,
            bound_rows,
            marquee,
            triggers,
            shell,
        } = self;
        ColumnView {
            shell: shell.shell,
            resize_handle: shell.resize_handle,
            reveal_button: shell.reveal_button,
            animation_generation: shell.animation_generation,
            unscaled_width: shell.unscaled_width,
            presentation,
            model: models.model,
            filtered_model: models.filtered_model,
            map: models.map,
            model_generation,
            header_actions: header.header_actions,
            sort_direction_button: header.sort_direction_button,
            header_actions_stack: header.header_actions_stack,
            filter_entry: header.filter_entry,
            filter_button: header.filter_button,
            selection: models.selection,
            syncing_selection: models.syncing_selection,
            list,
            listing_scroll: scroll,
            marquee,
            bound_rows,
            cursor_restore_generation: Rc::new(Cell::new(0)),
            folder_context_trigger: triggers.folder,
            item_context_trigger: triggers.item,
            entry_count: models.entry_count,
            spinner: header.spinner,
            spinner_delay: Rc::new(RefCell::new(None)),
            truncated_hint: header.truncated_hint,
            not_responding,
            empty_trash_button: header.is_trash.then_some(header.empty_trash),
            leftovers_button: header.leftovers,
            show_hidden: models.show_hidden,
            filter: models.filter,
            search_results: models.search_results,
            search_session: models.search_session,
            query_binding: Rc::new(RefCell::new(Some(query_binding))),
            search_model: models.search_model,
            recursive_search_active: models.recursive_search_active,
            renamed_result: models.renamed_result,
            hits: models.hits,
        }
    }
}

impl ViewState {
    fn column_pane(self: &Rc<Self>, depth: usize, location: &Location) -> gtk::Box {
        let column = crate::ui::accessibility::pane_box();
        column.add_css_class("directory-column");
        crate::ui::accessibility::describe_pane(
            &column,
            &location.display_name(),
            crate::ui::browser_modes::BrowserMode::Columns,
        );
        column.set_hexpand(true);
        column.set_vexpand(true);
        self.install_column_hover(&column, depth);
        column
    }

    /// The header, its notices and the models and query behind the filter field.
    fn column_controls(
        self: &Rc<Self>,
        depth: usize,
        location: &Location,
        column: &gtk::Box,
    ) -> (ColumnHeader, gtk::Box, ColumnModels, FilterQueryBinding) {
        let header = self.column_header(depth, location);
        column.append(&header.header);
        column.append(&header.filter_revealer);
        let not_responding = crate::ui::browser::not_responding_notice();
        column.append(&not_responding);
        if depth == 0 && location.is_administrator() {
            column.append(&crate::ui::browser::administrator_notice());
        }
        let models = self.column_models(depth);
        let query_binding = self.bind_column_query(depth, &header, &models);
        (header, not_responding, models, query_binding)
    }

    pub(in crate::ui::browser) fn append_column(
        self: &Rc<Self>,
        depth: usize,
        location: &Location,
    ) {
        let column = self.column_pane(depth, location);
        let (header, not_responding, models, query_binding) =
            self.column_controls(depth, location, &column);
        let rows::ColumnRows {
            factory,
            bound_rows,
            scrolling: column_scrolling,
        } = rows::column_rows(
            self,
            &rows::RowSources {
                depth,
                map: &models.map,
                selection: &models.selection,
                recursive_search_active: &models.recursive_search_active,
                search_results: &models.search_results,
                hits: &models.hits,
                enter_target: &header.enter_target,
            },
        );
        let list = column_list(location, &models.selection, factory);
        self.install_filter_navigation(depth, &header, &models, &list);
        self.install_return_to_filter(&header, &models, &list);
        self.connect_list_activation(depth, &models, &list);
        let scroll = self.listing_scroll(&list);
        let visit_rows = surface::visit_bound_rows(&bound_rows);
        let syncing_for_enter = models.syncing_selection.clone();
        header.enter_target.attach(
            &list,
            &models.selection,
            &scroll,
            visit_rows.clone(),
            move || syncing_for_enter.get(),
        );
        surface::track_listing_scrolling(&scroll, column_scrolling);
        let presentation = self.column_presentation(depth, location, &scroll, &list);
        let column_surface = ColumnSurface {
            depth,
            location,
            header: &header.header,
            list: &list,
            scroll: &scroll,
            presentation: &presentation,
            models: &models,
            bound_rows: &bound_rows,
        };
        let marquee = self.install_column_marquee(&column_surface, visit_rows);
        self.install_column_focus(&column, depth);
        self.install_column_drop_targets(
            location,
            &[column.upcast_ref(), presentation.stack.upcast_ref()],
        );
        let triggers = self.install_column_context_menus(&column_surface);
        column.append(&presentation.stack);
        let shell = self.column_shell(depth, location, &column, &header.filter_button);
        self.insert_column(
            depth,
            &column,
            ColumnParts {
                header,
                not_responding,
                models,
                query_binding,
                list,
                scroll,
                presentation,
                bound_rows,
                marquee,
                triggers,
                shell,
            },
        );
    }

    fn insert_column(self: &Rc<Self>, depth: usize, column: &gtk::Box, parts: ColumnParts) {
        let shell = parts.shell.shell.clone();
        let previous = depth
            .checked_sub(1)
            .and_then(|previous| self.columns.borrow().get(previous).cloned())
            .map(|column| column.shell);
        self.columns_widget
            .insert_child_after(&shell, previous.as_ref());
        let view = parts.into_view(self.source_generation.clone());
        self.columns.borrow_mut().push(view);

        if let Some(column) = self.columns.borrow().last() {
            set_column_busy(column, true);
            arm_column_spinner(column);
        }
        self.refresh_active_path_rows();
        let swapped = self.swap_slot.take() == Some(depth);
        animate_column_entry(column, swapped);
        self.reveal_column(shell);
    }
}
