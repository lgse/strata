// SPDX-License-Identifier: MIT

//! A session-only floating holding space. Files stay at their original locations until transferred.

use std::{
    cell::RefCell,
    rc::{Rc, Weak},
    time::Duration,
};

use gtk::{gdk, gio, glib, prelude::*};

use crate::{
    app::BrowserEvent,
    assets::{self, icons},
    model::Location,
    services::DropCommit,
    ui::browser::paths::is_trash_location,
    ui::browser::{
        BrowserView, WeakBrowserView, context_menu_option, file_drag_locations, icon_for_name,
        locations_equal, locations_from_file_list_value, show_error_dialog,
    },
    ui::thumbnail::{self, ThumbnailSlot},
    ui::{
        controls, scrolling::popover::dismiss_on_outside_scroll, shortcut_reference::ContextHint,
    },
};

mod shake;
use shake::Hyprland;

#[derive(Default)]
struct Shelf {
    locations: RefCell<Vec<Location>>,
    listeners: RefCell<Vec<Weak<dyn Fn()>>>,
}

thread_local! {
    static SHARED_SHELF: Rc<Shelf> = Rc::new(Shelf::default());
    static FLOATING: RefCell<Option<Rc<FloatingShelf>>> = const { RefCell::new(None) };
}

impl Shelf {
    fn shared() -> Rc<Self> {
        SHARED_SHELF.with(Rc::clone)
    }

    fn locations(&self) -> Vec<Location> {
        self.locations.borrow().clone()
    }

    fn add(&self, incoming: impl IntoIterator<Item = Location>) -> bool {
        let mut locations = self.locations.borrow_mut();
        let mut accepted = false;
        let mut changed = false;
        for location in incoming {
            if location.is_recent_location() {
                continue;
            }
            accepted = true;
            if !locations
                .iter()
                .any(|item| locations_equal(item, &location))
            {
                locations.push(location);
                changed = true;
            }
        }
        drop(locations);
        if changed {
            self.notify();
        }
        accepted
    }

    fn remove(&self, location: &Location) {
        self.remove_many(std::slice::from_ref(location));
    }

    fn remove_many(&self, removed: &[Location]) {
        let mut locations = self.locations.borrow_mut();
        let before = locations.len();
        locations.retain(|item| {
            !removed
                .iter()
                .any(|location| locations_equal(item, location))
        });
        let removed = locations.len() != before;
        drop(locations);
        if removed {
            self.notify();
        }
    }

    fn clear(&self) {
        if !self.locations.borrow().is_empty() {
            self.locations.borrow_mut().clear();
            self.notify();
        }
    }

    fn notify(&self) {
        self.listeners.borrow_mut().retain(|listener| {
            if let Some(listener) = listener.upgrade() {
                listener();
                true
            } else {
                false
            }
        });
    }
}

struct FloatingShelf {
    window: gtk::Window,
    active_browser: Rc<RefCell<Option<WeakBrowserView>>>,
    _view: ShelfView,
    hyprland: Option<Hyprland>,
}

pub(super) fn install(
    window: &gtk::ApplicationWindow,
    browser: &BrowserView,
    actions: &gio::SimpleActionGroup,
) {
    let Some(application) = window.application() else {
        return;
    };
    let shelf = Rc::downgrade(&Shelf::shared());
    browser.browser().observe(move |event| {
        let event = match event {
            BrowserEvent::BackgroundOperation { event, .. } => event.as_ref(),
            event => event,
        };
        if let BrowserEvent::TransferFinished { moved_locations } = event
            && !moved_locations.is_empty()
            && let Some(shelf) = shelf.upgrade()
        {
            shelf.remove_many(moved_locations);
        }
    });
    FLOATING.with(|slot| {
        if slot.borrow().is_none() {
            let active_browser = Rc::new(RefCell::new(Some(browser.downgrade())));
            let view = ShelfView::new(active_browser.clone());
            let floating = gtk::Window::builder()
                .application(&application)
                .title("Strata Shelf")
                .decorated(false)
                .resizable(false)
                .default_width(240)
                .child(&view.widget)
                .build();
            floating.set_transient_for(Some(window));
            floating.set_hide_on_close(true);
            let hyprland = Hyprland::current();
            let controller = Rc::new(FloatingShelf {
                window: floating,
                active_browser,
                _view: view,
                hyprland,
            });
            if let Some(hyprland) = controller.hyprland.as_ref() {
                let receive = hyprland.clone().listen();
                let weak = Rc::downgrade(&controller);
                glib::timeout_add_local(Duration::from_millis(55), move || {
                    let Some(controller) = weak.upgrade() else {
                        return glib::ControlFlow::Break;
                    };
                    if let Some(position) = receive.latest()
                        && crate::ui::browser::file_drag_active()
                    {
                        controller.show(Some(position));
                    }
                    glib::ControlFlow::Continue
                });
            }
            slot.replace(Some(controller));
        }
    });
    let action = gio::SimpleAction::new("show-shelf", None);
    action.connect_activate({
        let browser = browser.downgrade();
        let window = window.downgrade();
        move |_, _| {
            let shelf = FLOATING.with(|slot| slot.borrow().clone());
            if let Some(shelf) = shelf
                && let Some(window) = window.upgrade()
            {
                shelf.active_browser.replace(Some(browser.clone()));
                shelf.window.set_transient_for(Some(&window));
                shelf.show(None);
            }
        }
    });
    actions.add_action(&action);
}

pub(super) fn set_active(window: &gtk::ApplicationWindow, browser: &BrowserView) {
    let shelf = FLOATING.with(|slot| slot.borrow().clone());
    if let Some(shelf) = shelf {
        shelf.active_browser.replace(Some(browser.downgrade()));
        shelf.window.set_transient_for(Some(window));
    }
}

pub(super) fn owner_closed(window: &gtk::ApplicationWindow, application: &gtk::Application) {
    let next = application
        .windows()
        .into_iter()
        .filter_map(|candidate| candidate.downcast::<gtk::ApplicationWindow>().ok())
        .find(|candidate| candidate != window);
    let shelf = FLOATING.with(|slot| {
        if next.is_none() {
            slot.borrow_mut().take()
        } else {
            slot.borrow().clone()
        }
    });
    if let Some(shelf) = shelf {
        if let Some(next) = next {
            if shelf
                .window
                .transient_for()
                .as_ref()
                .is_none_or(|owner| owner == window.upcast_ref::<gtk::Window>())
            {
                shelf.window.set_visible(false);
                shelf.active_browser.take();
                shelf.window.set_transient_for(Some(&next));
            }
        } else {
            shelf.window.destroy();
        }
    }
}

impl FloatingShelf {
    fn show(&self, position: Option<(i32, i32)>) {
        self.window.present();
        let Some(hyprland) = self.hyprland.clone() else {
            return;
        };
        let position = position.or_else(|| hyprland.cursor());
        if let Some((x, y)) = position {
            // Hyprland positions transient windows, while Wayland prevents GTK from placing them.
            let window = self.window.downgrade();
            glib::timeout_add_local_once(Duration::from_millis(40), move || {
                if window.upgrade().is_some_and(|window| window.is_visible()) {
                    hyprland.move_shelf(x.saturating_sub(115), y.saturating_sub(105));
                }
            });
        }
    }
}

struct ShelfView {
    widget: gtk::Box,
    _refresh: Rc<dyn Fn()>,
}

impl ShelfView {
    fn new(active_browser: Rc<RefCell<Option<WeakBrowserView>>>) -> Self {
        let shelf = Shelf::shared();
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        widget.add_css_class("file-shelf");
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        bar.add_css_class("file-shelf-bar");
        let close = gtk::Button::new();
        close.set_child(Some(&assets::primary_icon(icons::X, 16)));
        close.set_tooltip_text(Some("Hide shelf"));
        crate::ui::accessibility::set_label(&close, "Hide shelf");
        controls::pane_header_action(&close);
        close.add_css_class("file-shelf-control");
        close.connect_clicked({
            let widget = widget.downgrade();
            move |_| {
                if let Some(window) = widget
                    .upgrade()
                    .and_then(|widget| widget.root())
                    .and_downcast::<gtk::Window>()
                {
                    window.set_visible(false);
                }
            }
        });
        bar.append(&close);
        let handle = gtk::WindowHandle::new();
        handle.set_hexpand(true);
        handle.set_child(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
        handle.set_cursor_from_name(Some("grab"));
        crate::ui::accessibility::set_label(&handle, "Move shelf");
        crate::ui::accessibility::set_description(&handle, Some("Drag to move the shelf"));
        bar.append(&handle);
        let menu = gtk::MenuButton::new();
        menu.set_child(Some(&assets::primary_icon(icons::ELLIPSIS, 16)));
        menu.set_direction(gtk::ArrowType::None);
        controls::pane_header_action(&menu);
        menu.add_css_class("file-shelf-control");
        menu.set_tooltip_text(Some("Shelf actions"));
        crate::ui::accessibility::set_label(&menu, "Shelf actions");
        bar.append(&menu);
        widget.append(&bar);

        let preview = gtk::Overlay::new();
        preview.add_css_class("file-shelf-preview");
        crate::ui::accessibility::set_label(&preview, "Shelf preview");
        let (back_card, back) = preview_card(96);
        back_card.add_css_class("file-shelf-preview-back");
        preview.set_child(Some(&back_card));
        let (middle_card, middle) = preview_card(96);
        middle_card.add_css_class("file-shelf-preview-middle");
        preview.add_overlay(&middle_card);
        let (front_card, front) = preview_card(96);
        front_card.add_css_class("file-shelf-preview-front");
        preview.add_overlay(&front_card);
        let drag = gtk::DragSource::builder()
            .actions(gdk::DragAction::COPY | gdk::DragAction::MOVE)
            .build();
        drag.connect_prepare({
            let shelf = shelf.clone();
            move |_, _, _| file_drag_locations(&shelf.locations())
        });
        preview.add_controller(drag);
        widget.append(&preview);

        let hint = gtk::Label::new(Some("Drop files here"));
        hint.add_css_class("file-shelf-hint");
        hint.set_wrap(true);
        hint.set_justify(gtk::Justification::Center);
        widget.append(&hint);

        let count = gtk::MenuButton::builder().label("0 items").build();
        count.set_direction(gtk::ArrowType::None);
        controls::pane_header_action(&count);
        count.add_css_class("file-shelf-count");
        count.set_halign(gtk::Align::Center);
        crate::ui::accessibility::set_description(&count, Some("Show individual shelf items"));
        widget.append(&count);
        let viewer = gtk::Popover::new();
        viewer.set_has_arrow(false);
        viewer.set_position(gtk::PositionType::Bottom);
        viewer.add_css_class("column-popover");
        viewer.add_css_class("file-shelf-viewer");
        dismiss_on_outside_scroll(&viewer);
        crate::ui::accessibility::set_label(&viewer, "Shelf items");
        count.set_popover(Some(&viewer));
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_max_content_height(300);
        scroll.set_propagate_natural_height(true);
        let list = gtk::FlowBox::new();
        list.set_selection_mode(gtk::SelectionMode::None);
        list.set_min_children_per_line(3);
        list.set_max_children_per_line(3);
        list.add_css_class("file-shelf-items");
        scroll.set_child(Some(&list));
        viewer.set_child(Some(&scroll));
        viewer.add_controller(shelf_drop_target(shelf.clone()));
        let action_popover = gtk::Popover::new();
        action_popover.set_has_arrow(false);
        action_popover.add_css_class("column-popover");
        dismiss_on_outside_scroll(&action_popover);
        menu.set_popover(Some(&action_popover));
        let actions = gtk::Box::new(gtk::Orientation::Vertical, 4);
        actions.add_css_class("column-menu");
        let clear = context_menu_option(icons::X, "Clear shelf", ContextHint::None);
        clear.connect_clicked({
            let shelf = shelf.clone();
            move |_| shelf.clear()
        });
        actions.append(&clear);
        let add = context_menu_option(icons::PLUS, "Add selection", ContextHint::None);
        add.connect_clicked({
            let shelf = shelf.clone();
            let active_browser = active_browser.clone();
            move |_| {
                if let Some(browser) = active_browser
                    .borrow()
                    .as_ref()
                    .and_then(WeakBrowserView::upgrade)
                {
                    shelf.add(
                        browser
                            .browser()
                            .selected_entries()
                            .into_iter()
                            .map(|entry| entry.location),
                    );
                }
            }
        });
        actions.append(&add);
        let copy = context_menu_option(icons::COPY, "Copy here", ContextHint::None);
        let move_button = context_menu_option(icons::FOLDER_INPUT, "Move here", ContextHint::None);
        for (button, commit) in [(&copy, DropCommit::Copy), (&move_button, DropCommit::Move)] {
            let active_browser = active_browser.clone();
            let shelf = shelf.clone();
            button.connect_clicked(move |_| {
                let Some(browser) = active_browser
                    .borrow()
                    .as_ref()
                    .and_then(WeakBrowserView::upgrade)
                else {
                    return;
                };
                let Some(destination) = browser.browser().active_location() else {
                    return;
                };
                if destination.is_recent_location() || is_trash_location(&destination) {
                    show_error_dialog(
                        &browser.widget(),
                        "Unable to transfer",
                        "Choose a regular folder as the destination.",
                    );
                    return;
                }
                browser.commit_file_drop(destination, shelf.locations(), commit);
            });
        }
        actions.append(&copy);
        actions.append(&move_button);
        action_popover.set_child(Some(&actions));
        for button in [&clear, &add, &copy, &move_button] {
            let popover = action_popover.downgrade();
            button.connect_clicked(move |_| {
                if let Some(popover) = popover.upgrade() {
                    popover.popdown();
                }
            });
        }

        widget.add_controller(shelf_drop_target(shelf.clone()));

        let refresh: Rc<dyn Fn()> = Rc::new({
            let shelf = Rc::downgrade(&shelf);
            let list = list.downgrade();
            let count = count.downgrade();
            let hint = hint.downgrade();
            let front = front.downgrade();
            let middle = middle.downgrade();
            let back = back.downgrade();
            let clear = clear.downgrade();
            let copy = copy.downgrade();
            let move_button = move_button.downgrade();
            move || {
                let (
                    Some(shelf),
                    Some(list),
                    Some(count),
                    Some(hint),
                    Some(front),
                    Some(middle),
                    Some(back),
                    Some(clear),
                    Some(copy),
                    Some(move_button),
                ) = (
                    shelf.upgrade(),
                    list.upgrade(),
                    count.upgrade(),
                    hint.upgrade(),
                    front.upgrade(),
                    middle.upgrade(),
                    back.upgrade(),
                    clear.upgrade(),
                    copy.upgrade(),
                    move_button.upgrade(),
                )
                else {
                    return;
                };
                while let Some(row) = list.first_child() {
                    thumbnail::cancel_thumbnails_in(&row);
                    list.remove(&row);
                }
                let locations = shelf.locations();
                count.set_label(&format!(
                    "{} item{}",
                    locations.len(),
                    if locations.len() == 1 { "" } else { "s" }
                ));
                for (index, image) in [&front, &middle, &back].into_iter().enumerate() {
                    if let Some(location) = locations.iter().rev().nth(index) {
                        if let Some(card) = image.parent() {
                            card.set_visible(true);
                        }
                        set_shelf_preview(image, location);
                    } else {
                        thumbnail::show_fallback_icon(image, icons::BOX, 64);
                        if let Some(card) = image.parent() {
                            card.set_visible(index == 0);
                        }
                    }
                }
                let has_items = !locations.is_empty();
                hint.set_visible(!has_items);
                count.set_sensitive(has_items);
                clear.set_sensitive(has_items);
                copy.set_sensitive(has_items);
                move_button.set_sensitive(has_items);
                for location in locations {
                    let row = gtk::Box::new(gtk::Orientation::Vertical, 6);
                    row.add_css_class("file-shelf-item");
                    let name = location
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| location.display_path());
                    crate::ui::accessibility::set_label(&row, &name);
                    let (card, image) = preview_card(72);
                    set_shelf_preview(&image, &location);
                    let thumbnail = gtk::Overlay::new();
                    thumbnail.set_child(Some(&card));
                    let remove = gtk::Button::new();
                    remove.set_child(Some(&assets::primary_icon(icons::X, 15)));
                    remove.set_tooltip_text(Some(&format!("Remove {name} from shelf")));
                    crate::ui::accessibility::set_label(
                        &remove,
                        &format!("Remove {name} from shelf"),
                    );
                    controls::pane_header_action(&remove);
                    remove.add_css_class("file-shelf-control");
                    remove.set_halign(gtk::Align::End);
                    remove.set_valign(gtk::Align::Start);
                    remove.connect_clicked({
                        let shelf = shelf.clone();
                        let location = location.clone();
                        move |_| shelf.remove(&location)
                    });
                    thumbnail.add_overlay(&remove);
                    row.append(&thumbnail);
                    let label = gtk::Label::new(Some(&name));
                    label.set_max_width_chars(15);
                    label.set_wrap(true);
                    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
                    label.set_lines(2);
                    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    row.append(&label);
                    let drag = gtk::DragSource::builder()
                        .actions(gdk::DragAction::COPY | gdk::DragAction::MOVE)
                        .build();
                    drag.connect_prepare({
                        let location = location.clone();
                        move |_, _, _| file_drag_locations(std::slice::from_ref(&location))
                    });
                    row.add_controller(drag);
                    list.insert(&row, -1);
                }
            }
        });
        shelf.listeners.borrow_mut().push(Rc::downgrade(&refresh));
        refresh();
        Self {
            widget,
            _refresh: refresh,
        }
    }
}

fn shelf_drop_target(shelf: Rc<Shelf>) -> gtk::DropTarget {
    let target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
    target.set_propagation_phase(gtk::PropagationPhase::Capture);
    target.connect_accept(|_, drop| {
        // The selected Wayland action may belong to the previous destination.
        drop.formats().contains_type(gdk::FileList::static_type())
            || drop.formats().contain_mime_type("text/uri-list")
    });
    target.connect_enter(|_, _, _| gdk::DragAction::COPY);
    target.connect_motion(|_, _, _| gdk::DragAction::COPY);
    target.connect_drop(move |_, value, _, _| {
        locations_from_file_list_value(value).is_some_and(|locations| shelf.add(locations))
    });
    target
}

fn preview_card(size: i32) -> (gtk::Box, ThumbnailSlot) {
    let image = ThumbnailSlot::new(size);
    let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.set_halign(gtk::Align::Center);
    card.set_valign(gtk::Align::Center);
    card.add_css_class("file-shelf-preview-card");
    card.append(&image);
    (card, image)
}

fn set_shelf_preview(image: &ThumbnailSlot, location: &Location) {
    let name = location
        .file_name()
        .map(|name| name.to_string_lossy().into_owned());
    let icon = if location.native_path().is_some_and(|path| path.is_dir()) {
        icons::FOLDER
    } else {
        name.as_deref().map_or(icons::DOCUMENTS, icon_for_name)
    };
    if let Some(path) = location.native_path() {
        thumbnail::set_thumbnail_or_icon_for_path(image, path, icon, 64, 96);
    } else {
        thumbnail::show_fallback_icon(image, icon, 64);
    }
}

#[cfg(test)]
mod tests;
