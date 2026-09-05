// SPDX-License-Identifier: GPL-3.0-or-later

//! Window-level tabs and their independent browser workspaces.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;

use super::browser::BrowserView;
use crate::app::Browser;
use crate::model::Location;

pub struct TabState {
    id: u64,
    pub view: BrowserView,
    pub label: gtk::Label,
    pub page: gtk::Widget,
    widget: gtk::Box,
    select: gtk::Button,
    close: gtk::Button,
    title: RefCell<String>,
}

pub struct TabsModel {
    stack: gtk::Stack,
    strip: gtk::Box,
    scroller: gtk::ScrolledWindow,
    tab_list: gtk::Box,
    new_tab_button: gtk::Button,
    new_tab_header: RefCell<Option<gtk::Box>>,
    new_tab_header_anchor: RefCell<Option<gtk::Widget>>,
    tabs: RefCell<Vec<Rc<TabState>>>,
    active: Cell<usize>,
    page_ids: Cell<u64>,
    on_select: RefCell<Option<SelectHandler>>,
    on_new_tab: RefCell<Option<Rc<dyn Fn()>>>,
}

type SelectHandler = Rc<dyn Fn(usize, &BrowserView)>;

const MAX_TABS: usize = 32;
const TAB_LABEL_MAX_WIDTH_CHARS: i32 = 20;

impl TabsModel {
    pub fn new(initial: BrowserView) -> Rc<Self> {
        let stack = gtk::Stack::builder()
            .hhomogeneous(false)
            .vhomogeneous(false)
            .hexpand(true)
            .vexpand(true)
            .build();
        let tab_list = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        tab_list.add_css_class("tab-list");
        let scroller = gtk::ScrolledWindow::builder()
            .child(&tab_list)
            .hscrollbar_policy(gtk::PolicyType::External)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .hexpand(true)
            .build();
        scroller.add_css_class("tab-scroller");
        let strip = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        strip.add_css_class("tab-strip");
        strip.append(&scroller);
        strip.set_visible(false);

        let new_tab_button = gtk::Button::new();
        new_tab_button.set_tooltip_text(Some("New tab (Ctrl+Shift+T)"));
        new_tab_button.set_child(Some(&crate::assets::primary_icon(
            crate::assets::icons::PLUS,
            16,
        )));
        new_tab_button.add_css_class("new-tab");
        new_tab_button.add_css_class("header-action");

        let this = Rc::new(Self {
            stack,
            strip,
            scroller,
            tab_list,
            new_tab_button,
            new_tab_header: RefCell::new(None),
            new_tab_header_anchor: RefCell::new(None),
            tabs: RefCell::new(Vec::new()),
            active: Cell::new(0),
            page_ids: Cell::new(0),
            on_select: RefCell::new(None),
            on_new_tab: RefCell::new(None),
        });
        let weak = Rc::downgrade(&this);
        this.new_tab_button.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade()
                && let Some(on_new_tab) = this.on_new_tab.borrow().clone()
            {
                on_new_tab();
            }
        });
        this.push(initial, "Home");
        this
    }

    pub fn set_on_new_tab(&self, callback: Rc<dyn Fn()>) {
        *self.on_new_tab.borrow_mut() = Some(callback);
    }

    pub fn set_on_select(&self, callback: SelectHandler) {
        *self.on_select.borrow_mut() = Some(callback);
    }

    pub fn views(&self) -> Vec<BrowserView> {
        self.tabs
            .borrow()
            .iter()
            .map(|tab| tab.view.clone())
            .collect()
    }

    pub fn clear_observers(&self) {
        for view in self.views() {
            view.browser().clear_observer();
        }
    }

    pub fn docked_widget(&self) -> gtk::Widget {
        let dock = gtk::Box::new(gtk::Orientation::Vertical, 0);
        dock.add_css_class("tab-dock");
        dock.append(&self.strip);
        dock.append(&self.stack);
        dock.upcast()
    }

    pub fn attach_new_tab_button(&self, header: &gtk::Box, after: &impl IsA<gtk::Widget>) {
        self.new_tab_header.replace(Some(header.clone()));
        self.new_tab_header_anchor
            .replace(Some(after.clone().upcast()));
        self.place_new_tab_button();
    }

    pub fn len(&self) -> usize {
        self.tabs.borrow().len()
    }

    pub fn active_index(&self) -> usize {
        self.active.get()
    }

    pub fn active_view(&self) -> BrowserView {
        self.tabs.borrow()[self.active.get()].view.clone()
    }

    pub fn active_browser(&self) -> Rc<Browser> {
        self.active_view().browser()
    }

    fn push(self: &Rc<Self>, view: BrowserView, title: &str) -> Rc<TabState> {
        let id = self.page_ids.get();
        self.page_ids.set(id + 1);

        let label = gtk::Label::builder()
            .label(title)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(TAB_LABEL_MAX_WIDTH_CHARS)
            .xalign(0.0)
            .build();
        label.add_css_class("tab-title");

        let select = gtk::Button::new();
        select.set_child(Some(&label));
        select.add_css_class("tab-select");
        select.set_hexpand(true);

        let close = gtk::Button::new();
        close.set_child(Some(&crate::assets::primary_icon(
            crate::assets::icons::X,
            14,
        )));
        close.add_css_class("close-tab");

        let widget = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        widget.add_css_class("tab");
        widget.append(&select);
        widget.append(&close);
        widget.set_size_request(152, -1);

        let page = view.widget();
        self.stack.add_named(&page, Some(&format!("tab-{id}")));
        let tab = Rc::new(TabState {
            id,
            view,
            label,
            page: page.clone(),
            widget: widget.clone(),
            select: select.clone(),
            close: close.clone(),
            title: RefCell::new(title.to_owned()),
        });
        self.tabs.borrow_mut().push(tab);

        let page_for_close = page.clone();
        let this = Rc::downgrade(self);
        close.connect_clicked(move |_| {
            if let Some(this) = this.upgrade() {
                this.close_tab_by_page(&page_for_close);
            }
        });

        let page_for_select = page.clone();
        let this = Rc::downgrade(self);
        select.connect_clicked(move |_| {
            if let Some(this) = this.upgrade() {
                this.select_by_page(&page_for_select);
            }
        });

        let middle_click = gtk::GestureClick::new();
        middle_click.set_button(gtk::gdk::BUTTON_MIDDLE);
        let page_for_middle = page.clone();
        let this = Rc::downgrade(self);
        middle_click.connect_pressed(move |gesture, _, _, _| {
            if let Some(this) = this.upgrade() {
                this.close_tab_by_page(&page_for_middle);
                gesture.set_state(gtk::EventSequenceState::Claimed);
            }
        });
        widget.add_controller(middle_click);

        let drag = gtk::DragSource::builder()
            .actions(gtk::gdk::DragAction::MOVE)
            .build();
        drag.connect_prepare(move |_, _, _| {
            Some(gtk::gdk::ContentProvider::for_value(
                &format!("strata-tab:{id}").to_value(),
            ))
        });
        let dragged_widget = widget.clone();
        drag.connect_drag_begin(move |_, _| {
            dragged_widget.add_css_class("dragging");
        });
        let dragged_widget = widget.clone();
        drag.connect_drag_end(move |_, _, _| {
            dragged_widget.remove_css_class("dragging");
        });
        select.add_controller(drag);

        let drop = gtk::DropTarget::new(String::static_type(), gtk::gdk::DragAction::MOVE);
        drop.set_preload(true);
        let this = Rc::downgrade(self);
        drop.connect_enter(move |target, _, _| {
            let accepted = this.upgrade().is_some_and(|this| {
                target
                    .value()
                    .as_ref()
                    .is_some_and(|value| this.reorder_from_value(value, id))
            });
            if accepted {
                gtk::gdk::DragAction::MOVE
            } else {
                gtk::gdk::DragAction::empty()
            }
        });
        let this = Rc::downgrade(self);
        drop.connect_motion(move |target, _, _| {
            let accepted = this.upgrade().is_some_and(|this| {
                target
                    .value()
                    .as_ref()
                    .is_some_and(|value| this.reorder_from_value(value, id))
            });
            if accepted {
                gtk::gdk::DragAction::MOVE
            } else {
                gtk::gdk::DragAction::empty()
            }
        });
        let this = Rc::downgrade(self);
        drop.connect_drop(move |_, value, _, _| {
            this.upgrade()
                .is_some_and(|this| this.reorder_from_value(value, id))
        });
        widget.add_controller(drop);

        let file_drop = gtk::DropTarget::new(
            gtk::gdk::FileList::static_type(),
            gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE,
        );
        let hover_generation = Rc::new(Cell::new(0_u64));
        let enter_generation = hover_generation.clone();
        let page_for_hover = page.clone();
        let pending_widget = widget.clone();
        let this = Rc::downgrade(self);
        file_drop.connect_enter(move |target, _, _| {
            pending_widget.add_css_class("file-drop-pending");
            let generation = enter_generation.get().saturating_add(1);
            enter_generation.set(generation);
            let current_generation = enter_generation.clone();
            let page = page_for_hover.clone();
            let this = this.clone();
            glib::timeout_add_local_once(Duration::from_millis(350), move || {
                if current_generation.get() == generation
                    && let Some(this) = this.upgrade()
                {
                    this.select_by_page(&page);
                }
            });
            super::browser::file_drop_action(target)
        });
        file_drop.connect_motion(|target, _, _| super::browser::file_drop_action(target));
        let leave_generation = hover_generation.clone();
        let pending_widget = widget.clone();
        file_drop.connect_leave(move |_| {
            leave_generation.set(leave_generation.get().saturating_add(1));
            pending_widget.remove_css_class("file-drop-pending");
        });
        let drop_generation = hover_generation;
        let drop_view = self
            .tabs
            .borrow()
            .last()
            .expect("tab just pushed")
            .view
            .clone();
        let pending_widget = widget.clone();
        file_drop.connect_drop(move |target, value, _, _| {
            drop_generation.set(drop_generation.get().saturating_add(1));
            pending_widget.remove_css_class("file-drop-pending");
            drop_view.drop_files_on_active_location(target, value)
        });
        widget.add_controller(file_drop);

        let menu = gtk::Box::new(gtk::Orientation::Vertical, 0);
        menu.add_css_class("folder-context-menu");
        let new_tab = tab_context_option("New tab");
        let close_tab = tab_context_option("Close tab");
        close_tab.add_css_class("danger");
        let close_others = tab_context_option("Close other tabs");
        let close_left = tab_context_option("Close tabs to the left");
        let close_right = tab_context_option("Close tabs to the right");
        menu.append(&new_tab);
        menu.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        menu.append(&close_tab);
        menu.append(&close_others);
        menu.append(&close_left);
        menu.append(&close_right);
        let popover = gtk::Popover::builder()
            .child(&menu)
            .autohide(true)
            .halign(gtk::Align::Start)
            .has_arrow(false)
            .build();
        popover.add_css_class("folder-context-popover");
        popover.set_parent(&widget);

        let page_for_new = page.clone();
        let new_popover = popover.downgrade();
        let this = Rc::downgrade(self);
        new_tab.connect_clicked(move |_| {
            if let Some(popover) = new_popover.upgrade() {
                popover.popdown();
            }
            if let Some(this) = this.upgrade() {
                this.select_by_page(&page_for_new);
                if let Some(on_new_tab) = this.on_new_tab.borrow().clone() {
                    on_new_tab();
                }
            }
        });
        let page_for_close = page.clone();
        let close_popover = popover.downgrade();
        let this = Rc::downgrade(self);
        close_tab.connect_clicked(move |_| {
            if let Some(popover) = close_popover.upgrade() {
                popover.popdown();
            }
            if let Some(this) = this.upgrade() {
                this.close_tab_by_page(&page_for_close);
            }
        });
        let page_for_others = page.clone();
        let others_popover = popover.downgrade();
        let this = Rc::downgrade(self);
        close_others.connect_clicked(move |_| {
            if let Some(popover) = others_popover.upgrade() {
                popover.popdown();
            }
            if let Some(this) = this.upgrade() {
                this.close_other_tabs(&page_for_others);
            }
        });
        let page_for_left = page.clone();
        let left_popover = popover.downgrade();
        let this = Rc::downgrade(self);
        close_left.connect_clicked(move |_| {
            if let Some(popover) = left_popover.upgrade() {
                popover.popdown();
            }
            if let Some(this) = this.upgrade() {
                this.close_tabs_beside(&page_for_left, false);
            }
        });
        let page_for_right = page.clone();
        let right_popover = popover.downgrade();
        let this = Rc::downgrade(self);
        close_right.connect_clicked(move |_| {
            if let Some(popover) = right_popover.upgrade() {
                popover.popdown();
            }
            if let Some(this) = this.upgrade() {
                this.close_tabs_beside(&page_for_right, true);
            }
        });
        let context = gtk::GestureClick::new();
        context.set_button(gtk::gdk::BUTTON_SECONDARY);
        let weak_popover = popover.downgrade();
        let this = Rc::downgrade(self);
        let context_page = page.clone();
        context.connect_pressed(move |gesture, _, _, y| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            let Some(this) = this.upgrade() else {
                return;
            };
            let Some(index) = this.index_of_page(&context_page) else {
                return;
            };
            close_others.set_sensitive(this.len() > 1);
            close_left.set_sensitive(index > 0);
            close_right.set_sensitive(index + 1 < this.len());
            if let Some(popover) = weak_popover.upgrade() {
                popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(0, y.round() as i32, 1, 1)));
                popover.popup();
            }
        });
        widget.add_controller(context);

        self.tab_list.append(&widget);
        self.select(self.tabs.borrow().len() - 1);
        self.update_strip();
        self.update_tab_metadata();
        self.tabs.borrow().last().cloned().expect("tab just pushed")
    }

    pub fn open_new_tab(
        self: &Rc<Self>,
        location: Location,
        new_view: impl FnOnce() -> BrowserView,
    ) -> Option<BrowserView> {
        if self.len() >= MAX_TABS {
            return None;
        }
        let title = tab_title(&location);
        let view = new_view();
        self.push(view.clone(), &title);
        view.navigate_location(location);
        Some(view)
    }

    pub fn sync_title_of(&self, browser: &Rc<Browser>) {
        let tab = self
            .tabs
            .borrow()
            .iter()
            .find(|tab| Rc::ptr_eq(&tab.view.browser(), browser))
            .cloned();
        let Some(tab) = tab else {
            return;
        };
        if let Some(location) = tab.view.browser().active_location() {
            let title = tab_title(&location);
            if *tab.title.borrow() != title {
                tab.title.replace(title.clone());
                tab.label.set_label(&title);
            }
        }
        self.update_tab_metadata();
    }

    pub fn is_active_browser(&self, browser: &Rc<Browser>) -> bool {
        self.tabs
            .borrow()
            .get(self.active.get())
            .is_some_and(|tab| Rc::ptr_eq(&tab.view.browser(), browser))
    }

    fn index_of_page(&self, page: &gtk::Widget) -> Option<usize> {
        self.tabs.borrow().iter().position(|tab| tab.page == *page)
    }

    fn close_tab_by_page(self: &Rc<Self>, page: &gtk::Widget) {
        if let Some(index) = self.index_of_page(page) {
            self.close_tab(index);
        }
    }

    fn select_by_page(&self, page: &gtk::Widget) {
        if let Some(index) = self.index_of_page(page) {
            self.select(index);
        }
    }

    fn close_other_tabs(self: &Rc<Self>, page: &gtk::Widget) {
        self.select_by_page(page);
        let pages = self
            .tabs
            .borrow()
            .iter()
            .filter(|tab| tab.page != *page)
            .map(|tab| tab.page.clone())
            .collect::<Vec<_>>();
        for page in pages {
            self.close_tab_by_page(&page);
        }
    }

    fn close_tabs_beside(self: &Rc<Self>, page: &gtk::Widget, after: bool) {
        let Some(index) = self.index_of_page(page) else {
            return;
        };
        let pages = self
            .tabs
            .borrow()
            .iter()
            .enumerate()
            .filter(|(candidate, _)| {
                if after {
                    *candidate > index
                } else {
                    *candidate < index
                }
            })
            .map(|(_, tab)| tab.page.clone())
            .collect::<Vec<_>>();
        for page in pages {
            self.close_tab_by_page(&page);
        }
    }

    pub fn select(&self, index: usize) {
        let (page, tab_view) = {
            let tabs = self.tabs.borrow();
            let Some(tab) = tabs.get(index) else {
                return;
            };
            (tab.page.clone(), tab.view.clone())
        };
        self.active.set(index);
        self.stack.set_visible_child(&page);
        self.reveal_tab(index);
        for (child_index, child) in self.tabs.borrow().iter().enumerate() {
            if child_index == index {
                child.widget.add_css_class("active");
            } else {
                child.widget.remove_css_class("active");
            }
        }
        if let Some(on_select) = self.on_select.borrow().clone() {
            on_select(index, &tab_view);
        }
    }

    fn reveal_tab(&self, index: usize) {
        let Some(tab) = self.tabs.borrow().get(index).cloned() else {
            return;
        };
        let Some(bounds) = tab.widget.compute_bounds(&self.tab_list) else {
            return;
        };
        let adjustment = self.scroller.hadjustment();
        let start = f64::from(bounds.x());
        let end = f64::from(bounds.x() + bounds.width());
        let visible_start = adjustment.value();
        let visible_end = visible_start + adjustment.page_size();
        if start < visible_start {
            adjustment.set_value(start);
        } else if end > visible_end {
            adjustment.set_value((end - adjustment.page_size()).max(adjustment.lower()));
        }
    }

    pub fn select_relative(&self, step: i32) {
        self.select(wrap_index(self.active_index(), self.len(), step));
    }

    pub fn select_numbered(&self, number: u32) {
        if let Some(index) = numbered_index(number, self.len()) {
            self.select(index);
        }
    }

    pub fn move_active(self: &Rc<Self>, step: i32) -> bool {
        let source = self.active_index();
        let target = source as i32 + step;
        if target < 0 || target >= self.len() as i32 {
            return false;
        }
        let tabs = self.tabs.borrow();
        let source_id = tabs[source].id;
        let target_id = tabs[target as usize].id;
        drop(tabs);
        self.reorder(source_id, target_id, step > 0)
    }

    pub fn close_tab(self: &Rc<Self>, index: usize) {
        let removed;
        {
            let mut tabs = self.tabs.borrow_mut();
            if tabs.len() <= 1 || index >= tabs.len() {
                return;
            }
            let active = self.active.get();
            removed = tabs.remove(index);
            self.active
                .set(clamp_active_after_close(active, index, tabs.len()));
        }
        removed.view.browser().clear_observer();
        self.tab_list.remove(&removed.widget);
        self.stack.remove(&removed.page);
        self.select(self.active.get());
        self.update_strip();
        self.update_tab_metadata();
    }

    fn reorder_from_value(self: &Rc<Self>, value: &glib::Value, target_id: u64) -> bool {
        let Ok(payload) = value.get::<String>() else {
            return false;
        };
        let Some(source_id) = payload
            .strip_prefix("strata-tab:")
            .and_then(|value| value.parse::<u64>().ok())
        else {
            return false;
        };
        if source_id == target_id {
            return true;
        }
        let tabs = self.tabs.borrow();
        let Some(source) = tabs.iter().position(|tab| tab.id == source_id) else {
            return false;
        };
        let Some(target) = tabs.iter().position(|tab| tab.id == target_id) else {
            return false;
        };
        drop(tabs);
        self.reorder(source_id, target_id, source < target)
    }

    fn reorder(self: &Rc<Self>, source_id: u64, target_id: u64, after: bool) -> bool {
        if source_id == target_id {
            return false;
        }
        let (moved, destination, active) = {
            let mut tabs = self.tabs.borrow_mut();
            let Some(source) = tabs.iter().position(|tab| tab.id == source_id) else {
                return false;
            };
            let Some(target) = tabs.iter().position(|tab| tab.id == target_id) else {
                return false;
            };
            let active_id = tabs[self.active.get()].id;
            let destination = reorder_index(source, target, after);
            let moved = tabs.remove(source);
            tabs.insert(destination, moved.clone());
            let active = tabs
                .iter()
                .position(|tab| tab.id == active_id)
                .expect("active tab remains after reorder");
            (moved, destination, active)
        };
        let previous = destination
            .checked_sub(1)
            .and_then(|index| self.tabs.borrow().get(index).map(|tab| tab.widget.clone()));
        self.tab_list
            .reorder_child_after(&moved.widget, previous.as_ref());
        self.select(active);
        self.update_tab_metadata();
        true
    }

    fn update_strip(&self) {
        let len = self.len();
        self.strip.set_visible(len > 1);
        self.place_new_tab_button();
        self.new_tab_button.set_sensitive(len < MAX_TABS);
        self.new_tab_button
            .set_tooltip_text(Some(if len < MAX_TABS {
                "New tab (Ctrl+Shift+T)"
            } else {
                "Maximum number of tabs reached"
            }));
    }

    fn place_new_tab_button(&self) {
        let Some(header) = self.new_tab_header.borrow().clone() else {
            return;
        };
        let Some(anchor) = self.new_tab_header_anchor.borrow().clone() else {
            return;
        };
        let target = if self.len() > 1 {
            self.strip.clone()
        } else {
            header.clone()
        };
        if self.new_tab_button.parent().as_ref() != Some(target.upcast_ref()) {
            if let Some(parent) = self.new_tab_button.parent().and_downcast::<gtk::Box>() {
                parent.remove(&self.new_tab_button);
            }
            if self.len() > 1 {
                self.strip.append(&self.new_tab_button);
            } else {
                header.insert_child_after(&self.new_tab_button, Some(&anchor));
            }
        }
    }

    fn update_tab_metadata(&self) {
        let tabs = self.tabs.borrow();
        let titles = tabs
            .iter()
            .map(|tab| tab.title.borrow().clone())
            .collect::<Vec<_>>();
        for (index, tab) in tabs.iter().enumerate() {
            let title = &titles[index];
            let duplicate_count = titles
                .iter()
                .filter(|candidate| *candidate == title)
                .count();
            let occurrence = titles[..=index]
                .iter()
                .filter(|candidate| *candidate == title)
                .count();
            let visible_title = if duplicate_count > 1 {
                format!("{title} {occurrence}")
            } else {
                title.clone()
            };
            tab.label.set_label(&visible_title);
            let shortcut = if index < 9 {
                format!(" (Ctrl+{})", index + 1)
            } else {
                String::new()
            };
            let location = tab
                .view
                .browser()
                .active_location()
                .map(|location| location.display_path())
                .unwrap_or_else(|| title.clone());
            tab.select
                .set_tooltip_text(Some(&format!("{location} — Tab {}{shortcut}", index + 1)));
            tab.close
                .set_tooltip_text(Some(&format!("Close {visible_title} (Ctrl+W)")));
        }
    }
}

fn tab_context_option(text: &str) -> gtk::Button {
    let label = gtk::Label::new(Some(text));
    label.set_hexpand(true);
    label.set_halign(gtk::Align::Fill);
    label.set_xalign(0.0);
    let button = gtk::Button::new();
    button.set_child(Some(&label));
    button.add_css_class("folder-context-option");
    button
}

fn tab_title(location: &Location) -> String {
    if std::env::var_os("HOME").map(Location::local).as_ref() == Some(location) {
        "Home".into()
    } else {
        location.display_name()
    }
}

fn reorder_index(source: usize, target: usize, after: bool) -> usize {
    let target_after_removal = if source < target { target - 1 } else { target };
    target_after_removal + usize::from(after)
}

fn wrap_index(active: usize, len: usize, step: i32) -> usize {
    if len == 0 {
        return 0;
    }
    (active as i32 + step).rem_euclid(len as i32) as usize
}

fn numbered_index(number: u32, len: usize) -> Option<usize> {
    (number >= 1 && number as usize <= len).then(|| number as usize - 1)
}

fn clamp_active_after_close(active: usize, closed: usize, remaining: usize) -> usize {
    if closed < active {
        active - 1
    } else {
        active.min(remaining.saturating_sub(1))
    }
}

#[cfg(test)]
mod tests;
