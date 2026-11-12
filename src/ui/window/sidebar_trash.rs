// SPDX-License-Identifier: MIT

//! The Trash place: its contents probe, context menu rows and drop target.

use std::rc::Rc;

use gtk::prelude::*;

use crate::{
    app::BrowserEvent,
    model::Location,
    ui::browser::{BrowserView, locations_from_file_list_value},
};

use super::{
    SidebarState, sidebar,
    sidebar::PlaceNavigation,
    sidebar_context_menu::sidebar_context_option,
    sidebar_drop_feedback,
    sidebar_popover::{context_popover, open_on_secondary_click, popdown},
    sidebar_rows::sidebar_button,
};

struct TrashMenuItems {
    properties: gtk::Button,
    unpin: gtk::Button,
    empty: gtk::Button,
}

/// Closes the menu, then runs `action` on the view while it is still alive.
fn connect_view_item(
    button: &gtk::Button,
    popover: &gtk::Popover,
    view: &BrowserView,
    action: impl Fn(&BrowserView) + 'static,
) {
    let popover = popover.downgrade();
    let view = view.downgrade();
    button.connect_clicked(move |_| {
        popdown(&popover);
        if let Some(view) = view.upgrade() {
            action(&view);
        }
    });
}

/// Rows of the Trash sidebar context menu that only make sense while Trash holds items.
pub(super) struct TrashMenuRows {
    pub(super) separator: gtk::Separator,
    pub(super) empty: gtk::Button,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TrashContents {
    /// Not probed yet, or the probe failed.
    Unknown,
    Empty,
    NonEmpty,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TrashMenuVisibility {
    pub(super) separator: bool,
    pub(super) empty: bool,
}

/// Destructive actions stay hidden until Trash is confirmed to hold something, and the
/// separator goes with them so Properties is not left above an empty gap.
pub(super) fn trash_menu_visibility(contents: TrashContents) -> TrashMenuVisibility {
    let visible = contents == TrashContents::NonEmpty;
    TrashMenuVisibility {
        separator: visible,
        empty: visible,
    }
}

pub(super) fn sync_trash_menu_rows(rows: &TrashMenuRows, contents: TrashContents) {
    let visibility = trash_menu_visibility(contents);
    rows.separator.set_visible(visibility.separator);
    rows.empty.set_visible(visibility.empty);
}

pub(super) fn trash_contents_from_probe(probe: Result<bool, glib::Error>) -> TrashContents {
    match probe {
        Ok(false) => TrashContents::Empty,
        Ok(true) => TrashContents::NonEmpty,
        Err(_) => TrashContents::Unknown,
    }
}

pub(super) fn event_changes_trash_contents(event: &BrowserEvent) -> bool {
    matches!(
        event,
        BrowserEvent::DeletionFinished { .. }
            | BrowserEvent::RestorationFinished { .. }
            | BrowserEvent::TransferFinished { .. }
            | BrowserEvent::OperationCompletedWithErrors { .. }
            | BrowserEvent::OperationCancelled { .. }
    )
}

pub(super) async fn trash_has_items(root: &gio::File) -> Result<bool, glib::Error> {
    let enumerator = root
        .enumerate_children_future(
            gio::FILE_ATTRIBUTE_STANDARD_NAME,
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
        )
        .await?;
    Ok(!enumerator
        .next_files_future(1, glib::Priority::DEFAULT)
        .await?
        .is_empty())
}

pub(super) fn trash_file_drop_action(target: &gtk::DropTarget) -> gtk::gdk::DragAction {
    if target.value().as_ref().is_some_and(|value| {
        locations_from_file_list_value(value)
            .is_none_or(|sources| !BrowserView::can_trash_file_drop(&sources))
    }) {
        gtk::gdk::DragAction::empty()
    } else {
        gtk::gdk::DragAction::MOVE
    }
}

pub(super) fn install_sidebar_trash_drop(state: &SidebarState, row: &impl IsA<gtk::Widget>) {
    row.add_css_class("file-drop-zone");
    let drop = gtk::DropTarget::new(
        gtk::gdk::FileList::static_type(),
        gtk::gdk::DragAction::MOVE,
    );
    drop.set_preload(true);
    drop.set_propagation_phase(gtk::PropagationPhase::Capture);
    let enter_feedback = state.drop_feedback.clone();
    drop.connect_enter(move |target, _, _| {
        if let Some(row) = target.widget() {
            enter_feedback.enter(&row);
        }
        sidebar_trash_feedback(target, &enter_feedback)
    });
    let motion_feedback = state.drop_feedback.clone();
    drop.connect_motion(move |target, _, _| sidebar_trash_feedback(target, &motion_feedback));
    let value_feedback = state.drop_feedback.clone();
    drop.connect_value_notify(move |target| {
        if let Some(offered) = target.current_drop()
            && target
                .widget()
                .is_some_and(|row| value_feedback.is_current(&row))
        {
            offered.status(
                target.actions(),
                sidebar_trash_feedback(target, &value_feedback),
            );
        }
    });
    let leave_feedback = state.drop_feedback.clone();
    drop.connect_leave(move |target| {
        if let Some(row) = target.widget() {
            leave_feedback.leave(&row);
        }
    });
    let view = state.view().downgrade();
    let drop_feedback = state.drop_feedback.clone();
    drop.connect_drop(move |target, value, _, _| {
        if let Some(row) = target.widget() {
            drop_feedback.leave(&row);
        }
        view.upgrade().is_some_and(|view| {
            locations_from_file_list_value(value)
                .is_some_and(|sources| view.trash_file_drop(sources))
        })
    });
    row.add_controller(drop);
}

pub(super) fn sidebar_trash_feedback(
    target: &gtk::DropTarget,
    feedback: &sidebar_drop_feedback::SidebarDropFeedback,
) -> gtk::gdk::DragAction {
    let Some(row) = target.widget().filter(|row| feedback.is_current(row)) else {
        return gtk::gdk::DragAction::empty();
    };
    let action = trash_file_drop_action(target);
    feedback.folder(&row, !action.is_empty());
    action
}

impl SidebarState {
    pub(super) fn apply_trash_menu_visibility(&self) {
        if let Some(rows) = self.trash_menu_rows.borrow().as_ref() {
            sync_trash_menu_rows(rows, self.trash_contents.get());
        }
    }

    pub(super) fn set_trash_contents(&self, contents: TrashContents) {
        if self.trash_contents.get() == contents {
            return;
        }
        self.trash_contents.set(contents);
        self.apply_trash_menu_visibility();
    }

    pub(super) fn refresh_trash_contents(self: &Rc<Self>) {
        if self.local_only {
            return;
        }
        // A probe already in flight may have read Trash before this change landed, so queue
        // another pass instead of trusting the result it is about to return.
        if self.trash_probe_running.get() {
            self.trash_probe_pending.set(true);
            return;
        }
        self.trash_probe_running.set(true);
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let probe = trash_has_items(&gio::File::for_uri("trash:///")).await;
            if let Err(error) = &probe {
                tracing::warn!(
                    error_domain = ?error.domain(),
                    error_code = error.code(),
                    "unable to read trash contents"
                );
            }
            if let Some(state) = weak.upgrade() {
                state.trash_probe_running.set(false);
                state.set_trash_contents(trash_contents_from_probe(probe));
                if state.trash_probe_pending.replace(false) {
                    state.refresh_trash_contents();
                }
            }
        });
    }

    /// Keeps the context menu current when another application changes `trash:///`.
    pub(super) fn watch_trash(self: &Rc<Self>) {
        if self.trash_monitor.borrow().is_some() {
            return;
        }
        let (changed, live) = (Rc::downgrade(self), Rc::downgrade(self));
        let monitor = crate::adapters::watch(
            &Location::uri("trash:///"),
            crate::adapters::Watched::Folder,
            move |_, _| {
                if let Some(state) = changed.upgrade() {
                    state.refresh_trash_contents();
                }
            },
            move || {
                if let Some(state) = live.upgrade() {
                    state.refresh_trash_contents();
                }
            },
        );
        *self.trash_monitor.borrow_mut() = Some(monitor);
    }

    pub(super) fn append_trash_place(self: &Rc<Self>) {
        let location = Location::uri("trash:///");
        let row = sidebar_button(crate::assets::icons::TRASH, &crate::i18n::tr("Trash"));
        crate::ui::accessibility::set_description(&row, Some("trash:///"));
        self.add_place_keycap(&row, crate::ui::keys::bindings::GO_TRASH);
        self.bind_place_row(&row, location, PlaceNavigation::Direct);

        let (menu, items) = self.build_trash_menu();
        self.apply_trash_menu_visibility();
        self.watch_trash();
        self.refresh_trash_contents();
        for view in self.target.views() {
            view.set_trash_button(row.clone());
        }
        let popover = context_popover(&row, &menu);
        sidebar::append_open_in_tab(
            &menu,
            &popover,
            &row,
            &self.view(),
            &Location::uri("trash:///"),
        );
        self.connect_trash_items(&popover, &items);
        let weak_state = Rc::downgrade(self);
        popover.connect_show(move |_| {
            if let Some(state) = weak_state.upgrade() {
                state.refresh_trash_contents();
            }
        });
        open_on_secondary_click(&row, &popover);
        self.append_sidebar_row(&row);
        self.make_place_reorderable(&row, "trash");
    }

    fn build_trash_menu(&self) -> (gtk::Box, TrashMenuItems) {
        let menu = crate::ui::accessibility::menu_box();
        menu.add_css_class("folder-context-menu");
        let properties = sidebar_context_option(crate::assets::icons::INFO, "Properties", false);
        let unpin = sidebar_context_option(crate::assets::icons::PIN, "Unpin", false);
        menu.append(&unpin);
        let empty = sidebar_context_option(crate::assets::icons::TRASH, "Empty Trash…", true);
        empty.add_css_class("danger");
        let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
        menu.append(&properties);
        menu.append(&separator);
        menu.append(&empty);
        *self.trash_menu_rows.borrow_mut() = Some(TrashMenuRows {
            separator,
            empty: empty.clone(),
        });
        let items = TrashMenuItems {
            properties,
            unpin,
            empty,
        };
        (menu, items)
    }

    fn connect_trash_items(self: &Rc<Self>, popover: &gtk::Popover, items: &TrashMenuItems) {
        connect_view_item(&items.properties, popover, &self.view(), |view| {
            view.show_location_properties(&Location::uri("trash:///"));
        });
        self.connect_state_item(&items.unpin, popover, |state| {
            state.preference_manager.set_sidebar_show_trash(false);
        });
        connect_view_item(
            &items.empty,
            popover,
            &self.view(),
            BrowserView::confirm_empty_trash,
        );
    }
}
