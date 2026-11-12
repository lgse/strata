// SPDX-License-Identifier: MIT

//! One Icons grid: its cards, their gestures, and its selection.

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

use gtk::{gio, glib, prelude::*};

use super::{
    ClickActivation, TransferHandlerSlot,
    clicks::{PreviewClick, SelectionClick, SlowClickRename},
    context_menu::install_section_context_menu,
    drag_drop::{DragStart, FolderPeek, ItemDragDrop},
    icons_card::{CardLook, IconsCard},
    icons_pane::{IconsContext, IconsLayout, configure_compact_view, configure_icons_view_density},
    metadata::metadata_fill_position,
    pane::{
        BoundItems, CollectionActivation, PaneSection, bound_edit, bound_item_visitor,
        install_edit_unbind, install_tab_landing, pane_directory_name, register_bound_mode_item,
    },
    selection::{SectionOwner, connect_selection},
    source_index::{PanePositions, SourceIndexMap},
};
use crate::{
    app::Browser,
    model::FileEntry,
    ui::{
        browser::{ClipboardMarks, ViewState},
        collection_edit::EditWidgets,
        enter_target::EnterTarget,
        icons_cell, pointer, thumbnail,
    },
};

pub(super) fn build_icons_view(
    context: &Rc<IconsContext>,
    model: &impl IsA<gio::ListModel>,
) -> PaneSection {
    let view_model = model.clone().upcast::<gio::ListModel>();
    let view_index = SourceIndexMap::watch(&view_model);
    let selection = gtk::MultiSelection::new(Some(view_model.clone()));
    let syncing = Rc::new(Cell::new(false));
    let bound_items: BoundItems = Rc::new(RefCell::new(Vec::new()));
    let positions = PanePositions {
        index: context.source_index.clone(),
        view: view_model.clone(),
    };
    let factory = IconsCardFactory {
        browser: Rc::downgrade(&context.browser),
        depth: context.depth,
        state: context.state.clone(),
        positions: positions.clone(),
        selection: selection.clone(),
        syncing: syncing.clone(),
        previews: context.click.previews.clone(),
        activation: context.click.activation.clone(),
        transfers: context.transfer.clone(),
        marks: context.marks.clone(),
        thumbnail_size: context.thumbnail_size.clone(),
        scrolling: context.scrolling.clone(),
        filter_query: context.filter_query.clone(),
        enter_target: context.enter_target.clone(),
        bound_items: bound_items.clone(),
        layout: context.layout,
    }
    .build();
    let view = icons_grid(context, &selection, factory, positions);
    let mut section = PaneSection {
        view: view.upcast(),
        view_index,
        view_model,
        selection,
        bound_items: bound_items.clone(),
        syncing,
        visit: bound_item_visitor(bound_items),
        item_context_trigger: Rc::new(|_, _| {}),
    };
    let owner = SectionOwner {
        sections: context.sections.clone(),
        depth: context.depth,
        source_index: context.source_index.clone(),
    };
    connect_selection(
        &section,
        owner.clone(),
        &context.browser,
        context.click.multiple_selection.clone(),
    );
    if let Some(state) = context.state.as_ref().and_then(Weak::upgrade) {
        section.item_context_trigger = install_section_context_menu(&state, &section, &owner);
    }
    section
}

fn icons_grid(
    context: &IconsContext,
    selection: &gtk::MultiSelection,
    factory: gtk::SignalListItemFactory,
    positions: PanePositions,
) -> gtk::GridView {
    let view = gtk::GridView::new(Some(selection.clone()), Some(factory));
    view.add_css_class("file-icons");
    view.set_vexpand(false);
    view.set_enable_rubberband(false);
    view.set_single_click_activate(false);
    view.set_tab_behavior(gtk::ListTabBehavior::Item);
    install_tab_landing(view.upcast_ref(), context.state.clone());
    match context.layout {
        IconsLayout::Grid => configure_icons_view_density(&view, context.density.get()),
        IconsLayout::Compact => configure_compact_view(&view),
    }
    crate::ui::accessibility::describe_entry_container(
        &view,
        &pane_directory_name(&context.browser, context.depth),
    );
    let activation = CollectionActivation {
        browser: Rc::downgrade(&context.browser),
        state: context.state.clone(),
        positions,
        depth: context.depth,
    };
    view.connect_activate(move |_, position| activation.activate(position));
    view
}

/// Builds and binds the cards of one Icons grid.
struct IconsCardFactory {
    browser: Weak<Browser>,
    depth: usize,
    state: Option<Weak<ViewState>>,
    positions: PanePositions,
    selection: gtk::MultiSelection,
    syncing: Rc<Cell<bool>>,
    previews: Rc<Cell<bool>>,
    activation: Rc<Cell<ClickActivation>>,
    transfers: TransferHandlerSlot,
    marks: Rc<RefCell<ClipboardMarks>>,
    thumbnail_size: Rc<Cell<i32>>,
    scrolling: Rc<Cell<bool>>,
    filter_query: Rc<RefCell<String>>,
    enter_target: Option<EnterTarget>,
    bound_items: BoundItems,
    layout: IconsLayout,
}

/// The entry a card binds to, looked up once per bind.
struct CardBinding {
    browser: Option<Rc<Browser>>,
    state: Option<Rc<ViewState>>,
    source_position: Option<usize>,
    thumbnail_size: i32,
}

impl IconsCardFactory {
    fn build(self) -> gtk::SignalListItemFactory {
        let bound_items = self.bound_items.clone();
        let context = Rc::new(self);
        let factory = gtk::SignalListItemFactory::new();
        let setup = context.clone();
        factory.connect_setup(move |_, item| setup.setup(item));
        factory.connect_bind(move |_, item| context.bind(item));
        install_edit_unbind(&factory, &bound_items);
        factory
    }

    fn setup(&self, object: &glib::Object) {
        let Some(item) = object.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let slot = self.icon_slot();
        let card = match self.layout {
            IconsLayout::Grid => icons_cell::new_card(slot),
            IconsLayout::Compact => icons_cell::new_compact_card(slot),
        };
        let Some((_, rename_label)) = icons_cell::parts(&card) else {
            return;
        };
        install_icons_content_hover(&card);
        self.install_interactions(item, &card, &rename_label);
        item.set_child(Some(&card));
        if let Some(parent) = card.parent()
            && self.layout == IconsLayout::Grid
        {
            parent.set_halign(gtk::Align::Center);
            parent.set_valign(gtk::Align::Start);
        }
        let field = icons_cell::ensure_rename_field(&card).expect("card editor");
        if self.layout == IconsLayout::Compact {
            gtk::prelude::EntryExt::set_alignment(&field, 0.0);
        }
        let edit = EditWidgets::new(&field, &rename_label);
        register_bound_mode_item(&self.bound_items, item, &card, &rename_label, edit);
    }

    fn icon_slot(&self) -> i32 {
        match self.layout {
            IconsLayout::Grid => super::icons_card_icon_slot(self.thumbnail_size.get()),
            IconsLayout::Compact => self.thumbnail_size.get(),
        }
    }

    fn install_interactions(
        &self,
        item: &gtk::ListItem,
        card: &gtk::Box,
        rename_label: &gtk::Inscription,
    ) {
        let slow_click = Rc::new(SlowClickRename::default());
        let weak_state = self.state.clone().unwrap_or_default();
        let activation_click = PreviewClick {
            browser: self.browser.clone(),
            state: weak_state.clone(),
            previews: self.previews.clone(),
            activation: self.activation.clone(),
            depth: self.depth,
            positions: Some(self.positions.clone()),
            filter_query: self.filter_query.clone(),
            slow_click: slow_click.clone(),
        }
        .install(card, item, rename_label);
        let content_click = SelectionClick {
            selection: self.selection.clone(),
            syncing: self.syncing.clone(),
            browser: self.browser.clone(),
            state: weak_state,
            depth: self.depth,
            positions: self.positions.clone(),
            slow_click: slow_click.clone(),
        }
        .install(card, item);
        activation_click.group_with(&content_click);
        FolderPeek {
            state: self.state.clone(),
            browser: self.browser.clone(),
            positions: self.positions.clone(),
            depth: self.depth,
        }
        .install(card, item);
        ItemDragDrop {
            browser: self.browser.clone(),
            transfer: self.transfers.clone(),
            depth: self.depth,
            positions: Some(self.positions.clone()),
            state: self.state.clone(),
        }
        .install(
            card,
            item,
            DragStart {
                content_click: &content_click,
                list_rows: false,
                intent: slow_click,
            },
        );
    }

    fn bind(&self, object: &glib::Object) {
        let Some(item) = object.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(card) = item.child().and_downcast::<gtk::Box>() else {
            return;
        };
        if let Some(enter_target) = &self.enter_target {
            enter_target.decorate(&card, item);
        }
        let source_position = item
            .item()
            .and_then(|value| self.positions.index.of_item(&value));
        let binding = CardBinding {
            browser: self.browser.upgrade(),
            state: self.state.as_ref().and_then(Weak::upgrade),
            source_position,
            thumbnail_size: self.icon_slot(),
        };
        let entry = binding.browser.as_ref().and_then(|browser| {
            source_position.and_then(|position| browser.entry_at(self.depth, position))
        });
        match self.layout {
            IconsLayout::Grid => icons_cell::set_slot(&card, binding.thumbnail_size),
            IconsLayout::Compact => icons_cell::set_compact_slot(&card, binding.thumbnail_size),
        }
        if let Some(entry) = entry {
            self.bind_entry(item, &card, &entry, &binding);
        }
    }

    fn bind_entry(
        &self,
        item: &gtk::ListItem,
        card: &gtk::Box,
        entry: &FileEntry,
        binding: &CardBinding,
    ) {
        let edit = bound_edit(item);
        if let Some(edit) = &edit {
            edit.bind(&entry.location);
        }
        if let Some(parts) = IconsCard::of(card) {
            parts.show_entry(
                item,
                entry,
                &CardLook {
                    marks: &self.marks.borrow(),
                    thumbnail_size: binding.thumbnail_size,
                    scrolling: self.scrolling.get(),
                    state: binding.state.as_deref(),
                },
            );
            parts.highlight(binding.state.as_deref(), &self.filter_query.borrow());
            if self.layout == IconsLayout::Compact {
                parts.fit_name();
            }
        }
        if let Some(edit) = &edit {
            edit.display.set_visible(!edit.is_editing());
            edit.field.set_visible(edit.is_editing());
        }
        self.request_metadata(card, entry, binding);
    }

    fn request_metadata(&self, card: &gtk::Box, entry: &FileEntry, binding: &CardBinding) {
        let Some(position) = metadata_fill_position(binding.source_position, entry, false, true)
        else {
            return;
        };
        let Some(browser) = binding.browser.as_ref() else {
            return;
        };
        let Some((icon, _)) = icons_cell::parts(card) else {
            return;
        };
        thumbnail::request_metadata(
            &icon,
            card,
            browser,
            self.depth,
            position,
            entry.location.clone(),
            true,
        );
    }
}

fn set_icons_content_hover(card: &gtk::Box, x: f64, y: f64) {
    if pointer::hits_icon_card_content(card.upcast_ref(), x, y) {
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
