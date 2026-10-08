// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use gtk::{gdk, glib, prelude::*};

use crate::ui::{
    accessibility,
    browser::{
        BrowserView, PreparedFileDrop, file_drop_action, file_drop_commit,
        locations_from_file_list_value, prepare_file_drop_target,
    },
    motion,
};

use super::{TabWindow, layout};

struct Header {
    id: u64,
    widget: gtk::Box,
    select: gtk::Button,
    label: gtk::Label,
    hint: gtk::Label,
}

#[derive(Default)]
struct Indicator {
    position: Cell<(f64, f64)>,
    from: Cell<(f64, f64)>,
    target: Cell<(f64, f64)>,
    started: Cell<i64>,
    active: Cell<u64>,
}

pub(super) struct TabStrip {
    widget: gtk::Box,
    row: gtk::Box,
    scroller: gtk::ScrolledWindow,
    headers: Rc<RefCell<Vec<Header>>>,
    indicator: Rc<Indicator>,
    pub(super) actions: gtk::Box,
    pub(super) end: gtk::Box,
}

impl TabStrip {
    pub(super) fn new() -> Self {
        let widget = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        widget.add_css_class("tab-strip");
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        row.set_accessible_role(gtk::AccessibleRole::TabList);
        accessibility::set_label(&row, &crate::i18n::tr("Tabs"));
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        actions.set_valign(gtk::Align::Center);
        let end = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        end.add_css_class("header-actions");
        end.set_valign(gtk::Align::Center);
        let flow = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        flow.append(&row);
        flow.append(&actions);
        let handle = gtk::WindowHandle::new();
        handle.set_hexpand(true);
        handle.set_child(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
        flow.append(&handle);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&flow));
        let line = gtk::DrawingArea::new();
        line.set_can_target(false);
        line.add_css_class("tab-indicator");
        overlay.add_overlay(&line);
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::External)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .hexpand(true)
            .child(&overlay)
            .build();
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        content.set_hexpand(true);
        content.append(&scroller);
        content.append(&end);
        widget.append(&content);
        let headers = Rc::new(RefCell::new(Vec::<Header>::new()));
        let indicator = Rc::new(Indicator::default());
        let drawing = indicator.clone();
        line.set_draw_func(move |area, context, _, height| {
            let (x, width) = drawing.position.get();
            let color = area.color();
            context.set_source_rgba(
                color.red() as f64,
                color.green() as f64,
                color.blue() as f64,
                color.alpha() as f64,
            );
            context.rectangle(x, height as f64 - 3.0, width, 3.0);
            let _ = context.fill();
        });
        let ticking_headers = headers.clone();
        let ticking_indicator = indicator.clone();
        let ticking_line = line.downgrade();
        overlay.add_tick_callback(move |overlay, clock| {
            let Some(line) = ticking_line.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let headers = ticking_headers.borrow();
            let Some(header) = headers
                .iter()
                .find(|header| header.id == ticking_indicator.active.get())
            else {
                return glib::ControlFlow::Continue;
            };
            let Some(bounds) = header.widget.compute_bounds(overlay) else {
                return glib::ControlFlow::Continue;
            };
            let target = (bounds.x() as f64, bounds.width() as f64);
            if ticking_indicator.target.get() != target {
                ticking_indicator.from.set(ticking_indicator.position.get());
                ticking_indicator.target.set(target);
                ticking_indicator.started.set(clock.frame_time());
            }
            let progress = if !motion::animations_enabled() || ticking_indicator.from.get().1 == 0.0
            {
                1.0
            } else {
                ((clock.frame_time() - ticking_indicator.started.get()) as f64 / 220_000.0)
                    .clamp(0.0, 1.0)
            };
            let eased = motion::emphasized_deceleration(progress);
            let from = ticking_indicator.from.get();
            let position = (
                from.0 + (target.0 - from.0) * eased,
                from.1 + (target.1 - from.1) * eased,
            );
            if ticking_indicator.position.replace(position) != position {
                line.queue_draw();
            }
            glib::ControlFlow::Continue
        });
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        let adjustment = scroller.hadjustment();
        scroll.connect_scroll(move |controller, dx, dy| {
            let delta = if dx.abs() > dy.abs() { dx } else { dy };
            let step = if controller.unit() == gdk::ScrollUnit::Wheel {
                36.0
            } else {
                1.0
            };
            let end = (adjustment.upper() - adjustment.page_size()).max(0.0);
            adjustment.set_value((adjustment.value() + delta * step).clamp(0.0, end));
            glib::Propagation::Stop
        });
        scroller.add_controller(scroll);
        Self {
            widget,
            row,
            scroller,
            headers,
            indicator,
            actions,
            end,
        }
    }

    pub(super) fn widget(&self) -> &gtk::Box {
        &self.widget
    }

    pub(super) fn add(&self, state: &Rc<TabWindow>, id: u64, browser: &BrowserView) {
        let widget = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        widget.add_css_class("browser-tab");
        let select = gtk::Button::new();
        select.set_accessible_role(gtk::AccessibleRole::Tab);
        select.add_css_class("tab-select");
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let label = gtk::Label::new(None);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.set_max_width_chars(22);
        let hint = gtk::Label::new(None);
        hint.add_css_class("tab-key-hint");
        hint.set_valign(gtk::Align::Center);
        hint.set_visible(false);
        content.append(&label);
        content.append(&hint);
        select.set_child(Some(&content));
        let close = layout::header_action(
            crate::assets::icons::X,
            &crate::i18n::tr("Close tab (Ctrl+W)"),
        );
        close.remove_css_class("header-action");
        let close_icons = gtk::Stack::new();
        close_icons.set_halign(gtk::Align::Center);
        close_icons.set_valign(gtk::Align::Center);
        for (name, icon) in [
            (
                "normal",
                crate::assets::text_icon(crate::assets::icons::X, 10),
            ),
            (
                "hovered",
                crate::assets::primary_icon(crate::assets::icons::X, 10),
            ),
        ] {
            icon.set_halign(gtk::Align::Center);
            icon.set_valign(gtk::Align::Center);
            close_icons.add_named(&icon, Some(name));
        }
        close.set_child(Some(&close_icons));
        let close_hover = gtk::EventControllerMotion::new();
        let hovered_icons = close_icons.downgrade();
        close_hover.connect_contains_pointer_notify(move |hover| {
            if let Some(icons) = hovered_icons.upgrade() {
                icons.set_visible_child_name(if hover.contains_pointer() {
                    "hovered"
                } else {
                    "normal"
                });
            }
        });
        close.add_controller(close_hover);
        accessibility::set_label(&close, &crate::i18n::tr("Close tab"));
        close.add_css_class("tab-close");
        widget.append(&select);
        widget.append(&close);
        let hover = gtk::EventControllerMotion::new();
        hover.connect_contains_pointer_notify(move |hover| {
            let Some(widget) = hover.widget() else {
                return;
            };
            if hover.contains_pointer() {
                widget.add_css_class("hovered");
            } else {
                widget.remove_css_class("hovered");
            }
        });
        widget.add_controller(hover);
        self.row.append(&widget);
        let weak = Rc::downgrade(state);
        select.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                state.select(id);
            }
        });
        let weak = Rc::downgrade(state);
        close.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                state.close(id);
            }
        });
        let middle = gtk::GestureClick::new();
        middle.set_button(2);
        let weak = Rc::downgrade(state);
        middle.connect_released(move |_, _, _, _| {
            if let Some(state) = weak.upgrade() {
                state.close(id);
            }
        });
        widget.add_controller(middle);
        install_reordering(state, id, &select, &label);
        install_file_drop(state, id, &widget, browser);
        self.headers.borrow_mut().push(Header {
            id,
            widget,
            select,
            label,
            hint,
        });
        self.label(
            id,
            &super::tab_label(browser.browser().active_location().as_ref()),
        );
    }

    pub(super) fn label(&self, id: u64, name: &str) {
        if let Some(header) = self.headers.borrow().iter().find(|header| header.id == id)
            && header.label.text() != name
        {
            header.label.set_ellipsize(if name.chars().count() > 22 {
                gtk::pango::EllipsizeMode::End
            } else {
                gtk::pango::EllipsizeMode::None
            });
            header.label.set_text(name);
            accessibility::set_label(&header.select, name);
        }
    }

    pub(super) fn select(&self, id: u64) {
        self.indicator.active.set(id);
        for header in self.headers.borrow().iter() {
            let active = header.id == id;
            if active {
                header.widget.add_css_class("active");
            } else {
                header.widget.remove_css_class("active");
            }
        }
        let selected_headers = Rc::downgrade(&self.headers);
        let selected_indicator = self.indicator.clone();
        let row = self.row.downgrade();
        let scroller = self.scroller.downgrade();
        let actions = self.actions.downgrade();
        let publish = move || {
            if selected_indicator.active.get() == id
                && let Some(headers) = selected_headers.upgrade()
            {
                for header in headers.borrow().iter() {
                    header
                        .select
                        .update_state(&[gtk::accessible::State::Selected(Some(header.id == id))]);
                }
                if let (Some(row), Some(scroller)) = (row.upgrade(), scroller.upgrade())
                    && let Some(header) = headers.borrow().iter().find(|header| header.id == id)
                    && let Some(bounds) = header.widget.compute_bounds(&row)
                {
                    let adjustment = scroller.hadjustment();
                    let left = bounds.x() as f64;
                    let trailing = if headers
                        .borrow()
                        .last()
                        .is_some_and(|header| header.id == id)
                    {
                        actions.upgrade().map_or(0, |actions| actions.width())
                    } else {
                        0
                    };
                    let right = left + bounds.width() as f64 + trailing as f64;
                    if left < adjustment.value() {
                        adjustment.set_value(left);
                    } else if right > adjustment.value() + adjustment.page_size() {
                        adjustment.set_value(right - adjustment.page_size());
                    }
                }
            }
        };
        // Reveal new tabs only after GTK has allocated their labels.
        if let Some(clock) = self.widget.frame_clock() {
            let handler = Rc::new(RefCell::new(None));
            let finished = handler.clone();
            let id = clock.connect_after_paint(move |clock| {
                if let Some(id) = finished.take() {
                    clock.disconnect(id);
                }
                publish();
            });
            handler.replace(Some(id));
        } else {
            publish();
        }
    }

    pub(super) fn hints(&self, show: bool) {
        let mut changed = false;
        for (index, header) in self.headers.borrow().iter().enumerate() {
            let visible = show && index < 10;
            changed |= header.hint.is_visible() != visible;
            header.hint.set_text(&((index + 1) % 10).to_string());
            header.hint.set_visible(visible);
        }
        if changed {
            self.select(self.indicator.active.get());
        }
    }

    pub(super) fn remove(&self, id: u64) {
        self.headers.borrow_mut().retain(|header| {
            if header.id == id {
                self.row.remove(&header.widget);
                false
            } else {
                true
            }
        });
    }

    pub(super) fn reorder(&self, ids: &[u64]) {
        let mut headers = self.headers.borrow_mut();
        headers.sort_by_key(|header| ids.iter().position(|id| *id == header.id));
        let mut previous: Option<gtk::Widget> = None;
        for header in headers.iter() {
            self.row
                .reorder_child_after(&header.widget, previous.as_ref());
            previous = Some(header.widget.clone().upcast());
        }
        drop(headers);
        self.select(self.indicator.active.get());
    }
}

fn install_reordering(state: &Rc<TabWindow>, id: u64, widget: &gtk::Button, label: &gtk::Label) {
    let prefix = format!("strata-tab-{}:", state.drag_token);
    let value = format!("{prefix}{id}");
    let source = gtk::DragSource::new();
    source.set_actions(gdk::DragAction::MOVE);
    let drag_label = label.downgrade();
    source.connect_prepare(move |source, _, _| {
        let label = drag_label.upgrade()?;
        source.set_icon(Some(&gtk::WidgetPaintable::new(Some(&label))), 0, 0);
        Some(gdk::ContentProvider::for_value(&value.to_value()))
    });
    widget.add_controller(source);
    let target = gtk::DropTarget::new(String::static_type(), gdk::DragAction::MOVE);
    target.connect_accept(|_, drop| {
        let formats = drop.formats();
        formats.contains_type(String::static_type())
            && !formats.contains_type(gdk::FileList::static_type())
            && !formats.contain_mime_type("text/uri-list")
    });
    let weak = Rc::downgrade(state);
    target.connect_drop(move |_, value, _, _| {
        let Some(state) = weak.upgrade() else {
            return false;
        };
        let Ok(value) = value.get::<String>() else {
            return false;
        };
        let Some(source) = value
            .strip_prefix(&prefix)
            .and_then(|id| id.parse::<u64>().ok())
        else {
            return false;
        };
        state.reorder(source, id);
        true
    });
    widget.add_controller(target);
}

fn install_file_drop(state: &Rc<TabWindow>, id: u64, widget: &gtk::Box, browser: &BrowserView) {
    let destination_browser = browser.downgrade();
    let PreparedFileDrop {
        target,
        state: drop_state,
    } = prepare_file_drop_target(move || {
        destination_browser.upgrade()?.browser().active_location()
    });
    let hover = Rc::new(Cell::new(0_u64));
    let enter_hover = hover.clone();
    let weak = Rc::downgrade(state);
    let enter_drop = drop_state.clone();
    target.connect_enter(move |target, _, _| {
        if let Some(widget) = target.widget() {
            widget.add_css_class("drag-hovered");
        }
        let generation = enter_hover.get().wrapping_add(1);
        enter_hover.set(generation);
        let token = enter_hover.clone();
        let weak = weak.clone();
        glib::timeout_add_local_once(Duration::from_millis(450), move || {
            if token.get() == generation
                && let Some(state) = weak.upgrade()
            {
                state.select(id);
            }
        });
        file_drop_action(target, &enter_drop)
    });
    target.connect_leave(move |target| {
        hover.set(hover.get().wrapping_add(1));
        if let Some(widget) = target.widget() {
            widget.remove_css_class("drag-hovered");
        }
    });
    let motion_drop = drop_state.clone();
    target.connect_motion(move |target, _, _| file_drop_action(target, &motion_drop));
    let weak_browser = browser.downgrade();
    let weak = Rc::downgrade(state);
    target.connect_drop(move |target, value, _, _| {
        let (Some(state), Some(browser)) = (weak.upgrade(), weak_browser.upgrade()) else {
            return false;
        };
        if state.blocked() {
            return false;
        }
        let Some(destination) = browser.browser().active_location() else {
            return false;
        };
        let Some(sources) =
            locations_from_file_list_value(value).filter(|sources| !sources.is_empty())
        else {
            return false;
        };
        if file_drop_action(target, &drop_state).is_empty() {
            return false;
        }
        let commit = file_drop_commit(target, &destination, &sources, &drop_state);
        state.select(id);
        browser.commit_file_drop(destination, sources, commit);
        true
    });
    widget.add_controller(target);
}
