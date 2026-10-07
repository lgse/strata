// SPDX-License-Identifier: MIT

//! A session-only floating holding space. Files stay at their original locations until transferred.

use std::{
    cell::RefCell,
    rc::{Rc, Weak},
    time::Duration,
};

use gtk::{gdk, gio, glib, prelude::*};

use crate::{
    adapters::location_for_file,
    assets::{self, icons},
    model::Location,
    services::DropCommit,
    ui::browser::paths::is_trash_location,
    ui::browser::{
        BrowserView, WeakBrowserView, file_drag_locations, locations_from_file_list_value,
        show_error_dialog,
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

    fn add(&self, incoming: impl IntoIterator<Item = Location>) -> usize {
        let mut locations = self.locations.borrow_mut();
        let mut added = 0;
        for location in incoming {
            if !location.is_recent_location() && !locations.contains(&location) {
                locations.push(location);
                added += 1;
            }
        }
        drop(locations);
        if added != 0 {
            self.notify();
        }
        added
    }

    fn remove(&self, location: &Location) {
        let mut locations = self.locations.borrow_mut();
        let before = locations.len();
        locations.retain(|item| item != location);
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
    button: &gtk::Button,
) {
    let Some(application) = window.application() else {
        return;
    };
    FLOATING.with(|slot| {
        if slot.borrow().is_none() {
            let active_browser = Rc::new(RefCell::new(Some(browser.downgrade())));
            let view = ShelfView::new(active_browser.clone());
            let floating = gtk::Window::builder()
                .application(&application)
                .title("Strata Shelf")
                .decorated(false)
                .resizable(false)
                .default_width(420)
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
                    if let Some(position) = receive.try_iter().last() {
                        controller.show(Some(position));
                    }
                    glib::ControlFlow::Continue
                });
            }
            slot.replace(Some(controller));
        }
    });
    button.connect_clicked({
        let browser = browser.downgrade();
        move |_| {
            let shelf = FLOATING.with(|slot| slot.borrow().clone());
            if let Some(shelf) = shelf {
                shelf.active_browser.replace(Some(browser.clone()));
                shelf.show(None);
            }
        }
    });
}

pub(super) fn set_active(window: &gtk::ApplicationWindow, browser: &BrowserView) {
    let shelf = FLOATING.with(|slot| slot.borrow().clone());
    if let Some(shelf) = shelf {
        shelf.active_browser.replace(Some(browser.downgrade()));
        shelf.window.set_transient_for(Some(window));
    }
}

pub(super) fn owner_closed(window: &gtk::ApplicationWindow) {
    let Some(application) = window.application() else {
        return;
    };
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
            shelf.window.set_transient_for(Some(&next));
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
            glib::timeout_add_local_once(Duration::from_millis(40), move || {
                hyprland.move_shelf(x.saturating_sub(155), y.saturating_add(28));
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
        let title = gtk::Label::new(Some("Shelf (0)"));
        title.add_css_class("file-shelf-title");
        title.set_xalign(0.0);
        title.set_hexpand(true);
        bar.append(&title);
        let close = gtk::Button::new();
        close.set_child(Some(&assets::primary_icon(icons::X, 16)));
        close.set_tooltip_text(Some("Hide shelf"));
        crate::ui::accessibility::set_label(&close, "Hide shelf");
        close.add_css_class("shortcut-footer-button");
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
        widget.append(&bar);
        let hint = gtk::Label::new(Some(
            "Drop files here, then drag them out or transfer below",
        ));
        hint.add_css_class("file-shelf-hint");
        hint.set_xalign(0.0);
        widget.append(&hint);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_max_content_height(220);
        scroll.set_propagate_natural_height(true);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        scroll.set_child(Some(&list));
        widget.append(&scroll);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.add_css_class("file-shelf-actions");
        let clear = gtk::Button::with_label("Clear shelf");
        clear.add_css_class("shortcut-footer-button");
        clear.connect_clicked({
            let shelf = shelf.clone();
            move |_| shelf.clear()
        });
        actions.append(&clear);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        actions.append(&spacer);
        let add = gtk::Button::with_label("Add selection");
        add.add_css_class("shortcut-footer-button");
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
        let copy = gtk::Button::with_label("Copy here");
        copy.add_css_class("shortcut-footer-button");
        let move_button = gtk::Button::with_label("Move here");
        move_button.add_css_class("shortcut-footer-button");
        for (button, commit) in [(&copy, DropCommit::Copy), (&move_button, DropCommit::Move)] {
            let active_browser = active_browser.clone();
            let shelf = shelf.clone();
            button.connect_clicked(move |button| {
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
                        button,
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
        widget.append(&actions);

        let formats = gdk::ContentFormats::builder()
            .add_type(gdk::FileList::static_type())
            .add_mime_type("text/uri-list")
            .build();
        let drop_target = gtk::DropTargetAsync::new(Some(formats), gdk::DragAction::COPY);
        drop_target.set_propagation_phase(gtk::PropagationPhase::Capture);
        drop_target.connect_drag_enter(|target, drop, _, _| {
            if let Some(widget) = target.widget() {
                widget.add_css_class("drop-destination");
            }
            if drop.actions().contains(gdk::DragAction::COPY) {
                gdk::DragAction::COPY
            } else {
                gdk::DragAction::empty()
            }
        });
        drop_target.connect_drag_leave(|target, _| {
            if let Some(widget) = target.widget() {
                widget.remove_css_class("drop-destination");
            }
        });
        drop_target.connect_drop({
            let shelf = shelf.clone();
            let hint = hint.downgrade();
            move |target, drop, _, _| {
                if let Some(widget) = target.widget() {
                    widget.remove_css_class("drop-destination");
                }
                if !drop.actions().contains(gdk::DragAction::COPY) {
                    return false;
                }
                let drop = drop.clone();
                let shelf = shelf.clone();
                let hint = hint.clone();
                glib::MainContext::default().spawn_local(async move {
                    let locations = drop
                        .read_value_future(gdk::FileList::static_type(), glib::Priority::DEFAULT)
                        .await
                        .ok()
                        .and_then(|value| locations_from_file_list_value(&value));
                    let locations = match locations {
                        Some(locations) => Some(locations),
                        None => read_uri_list(&drop).await,
                    };
                    if let Some(locations) = locations.filter(|locations| !locations.is_empty()) {
                        shelf.add(locations);
                        if let Some(hint) = hint.upgrade() {
                            hint.remove_css_class("file-shelf-error");
                            hint.set_label("Drop files here, then drag them out or transfer below");
                        }
                        drop.finish(gdk::DragAction::COPY);
                    } else {
                        if let Some(hint) = hint.upgrade() {
                            hint.add_css_class("file-shelf-error");
                            hint.set_label("Could not add these files to the shelf");
                        }
                        drop.finish(gdk::DragAction::empty());
                    }
                });
                true
            }
        });
        widget.add_controller(drop_target);

        let refresh: Rc<dyn Fn()> = Rc::new({
            let shelf = Rc::downgrade(&shelf);
            let list = list.downgrade();
            let title = title.downgrade();
            let clear = clear.downgrade();
            let copy = copy.downgrade();
            let move_button = move_button.downgrade();
            move || {
                let (
                    Some(shelf),
                    Some(list),
                    Some(title),
                    Some(clear),
                    Some(copy),
                    Some(move_button),
                ) = (
                    shelf.upgrade(),
                    list.upgrade(),
                    title.upgrade(),
                    clear.upgrade(),
                    copy.upgrade(),
                    move_button.upgrade(),
                )
                else {
                    return;
                };
                while let Some(row) = list.first_child() {
                    list.remove(&row);
                }
                let locations = shelf.locations();
                title.set_label(&format!("Shelf ({})", locations.len()));
                let has_items = !locations.is_empty();
                clear.set_sensitive(has_items);
                copy.set_sensitive(has_items);
                move_button.set_sensitive(has_items);
                for location in locations {
                    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                    row.add_css_class("file-shelf-row");
                    let name = location
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| location.display_path());
                    let label = gtk::Label::new(Some(&name));
                    label.set_xalign(0.0);
                    label.set_hexpand(true);
                    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    row.append(&label);
                    let remove = gtk::Button::new();
                    remove.set_child(Some(&assets::primary_icon(icons::X, 15)));
                    remove.set_tooltip_text(Some(&format!("Remove {name} from shelf")));
                    crate::ui::accessibility::set_label(
                        &remove,
                        &format!("Remove {name} from shelf"),
                    );
                    remove.add_css_class("shortcut-footer-button");
                    remove.connect_clicked({
                        let shelf = shelf.clone();
                        let location = location.clone();
                        move |_| shelf.remove(&location)
                    });
                    row.append(&remove);
                    let drag = gtk::DragSource::builder()
                        .actions(gdk::DragAction::COPY | gdk::DragAction::MOVE)
                        .build();
                    drag.connect_prepare({
                        let location = location.clone();
                        move |_, _, _| file_drag_locations(std::slice::from_ref(&location))
                    });
                    row.add_controller(drag);
                    list.append(&row);
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

async fn read_uri_list(drop: &gdk::Drop) -> Option<Vec<Location>> {
    let (stream, _) = drop
        .read_future(&["text/uri-list"], glib::Priority::DEFAULT)
        .await
        .ok()?;
    let mut bytes = Vec::new();
    loop {
        let (data, size) = stream
            .read_future(vec![0_u8; 4096], glib::Priority::DEFAULT)
            .await
            .ok()?;
        bytes.extend_from_slice(&data[..size]);
        if bytes.len() > 1024 * 1024 {
            return None;
        }
        if size == 0 {
            break;
        }
    }
    let text = std::str::from_utf8(&bytes).ok()?;
    let locations = locations_from_uri_list(text);
    (!locations.is_empty()).then_some(locations)
}

fn locations_from_uri_list(text: &str) -> Vec<Location> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|uri| glib::Uri::parse_scheme(uri).map(|_| gio::File::for_uri(uri)))
        .filter_map(|file| location_for_file(&file))
        .collect()
}

#[cfg(test)]
mod tests;
