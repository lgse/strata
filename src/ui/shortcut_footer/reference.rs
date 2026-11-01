// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{glib, prelude::*};

use super::catalog::{ReferenceQuery, category_buttons, reflow_categories, render_reference};
use crate::ui::shortcut_reference::ReferenceScope;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ReferenceArea {
    Search,
    Categories,
    Results,
}

/// Space kept between the compact reference panel and the window edges, borders included.
const REFERENCE_MARGIN: i32 = 56;

#[derive(Clone)]
pub(super) struct ReferenceLayout {
    pub(super) header: glib::WeakRef<gtk::Box>,
    pub(super) body: glib::WeakRef<gtk::Box>,
    pub(super) sidebar: glib::WeakRef<gtk::ScrolledWindow>,
    pub(super) categories: glib::WeakRef<gtk::Box>,
    pub(super) scroll: glib::WeakRef<gtk::ScrolledWindow>,
    pub(super) reference: glib::WeakRef<gtk::Box>,
    pub(super) search: glib::WeakRef<gtk::Entry>,
    pub(super) footer: glib::WeakRef<gtk::Label>,
    pub(super) scope: Rc<Cell<ReferenceScope>>,
    pub(super) selected: Rc<RefCell<String>>,
}

/// The reference panel's widgets while it is laid out.
struct LayoutWidgets {
    header: gtk::Box,
    body: gtk::Box,
    sidebar: gtk::ScrolledWindow,
    categories: gtk::Box,
    scroll: gtk::ScrolledWindow,
    reference: gtk::Box,
    search: gtk::Entry,
}

impl ReferenceLayout {
    fn upgrade(&self) -> Option<LayoutWidgets> {
        Some(LayoutWidgets {
            header: self.header.upgrade()?,
            body: self.body.upgrade()?,
            sidebar: self.sidebar.upgrade()?,
            categories: self.categories.upgrade()?,
            scroll: self.scroll.upgrade()?,
            reference: self.reference.upgrade()?,
            search: self.search.upgrade()?,
        })
    }

    pub(super) fn update(&self, window: &gtk::Window) {
        let Some(widgets) = self.upgrade() else {
            return;
        };
        let footer = self.footer.upgrade();
        if let Some(footer) = &footer {
            footer.set_max_width_chars(((window.width() - 80) / 9).max(10));
        }
        let compact = window.width() < 1480;
        let changed = widgets.reference.has_css_class("compact") != compact;
        widgets.set_compact(compact);
        if changed {
            restyle_category_buttons(&widgets.categories, compact);
        }
        if compact || changed {
            reflow_categories(&widgets.categories, compact, (window.width() - 104).max(1));
        }
        widgets.size_panes(window, compact, footer.as_ref());
        if changed {
            render_reference(
                &widgets.reference,
                &ReferenceQuery {
                    scope: self.scope.get(),
                    selected: &self.selected.borrow(),
                    query: &widgets.search.text(),
                    compact,
                },
            );
        }
    }
}

impl LayoutWidgets {
    fn set_compact(&self, compact: bool) {
        let orientation = if compact {
            gtk::Orientation::Vertical
        } else {
            gtk::Orientation::Horizontal
        };
        self.header.set_orientation(orientation);
        self.body.set_orientation(orientation);
        for widget in [&self.body, &self.reference, &self.categories] {
            set_css_class(widget, "compact", compact);
        }
        self.categories.set_spacing(if compact { 8 } else { 0 });
    }

    fn size_panes(&self, window: &gtk::Window, compact: bool, footer: Option<&gtk::Label>) {
        let width = (window.width() - 80).max(1);
        self.sidebar.set_height_request(-1);
        self.sidebar.set_propagate_natural_height(compact);
        self.sidebar.set_min_content_height(-1);
        let max_sidebar_height = (window.height() / 4).clamp(60, 160);
        self.sidebar.set_max_content_height(max_sidebar_height);
        self.sidebar.set_vexpand(!compact);
        self.scroll.set_width_request(if compact {
            width
        } else {
            (window.width() - 340).clamp(1, 1200)
        });
        let mut results_height = window.height() - if compact { 300 } else { 160 };
        if compact {
            let chrome = self.compact_chrome(width, max_sidebar_height, footer);
            results_height = results_height.min(window.height() - chrome);
        }
        self.scroll.set_height_request(results_height.clamp(1, 620));
    }

    /// Wrapped hints and chip rows take their height from the results, not the chips.
    fn compact_chrome(
        &self,
        width: i32,
        max_sidebar_height: i32,
        footer: Option<&gtk::Label>,
    ) -> i32 {
        let sidebar_height = self
            .categories
            .measure(gtk::Orientation::Vertical, width)
            .1
            .min(max_sidebar_height);
        self.sidebar.set_min_content_height(sidebar_height);
        let natural_height =
            |widget: &gtk::Widget| widget.measure(gtk::Orientation::Vertical, width).1;
        let footer_height = footer
            .and_then(|footer| footer.parent())
            .map_or(0, |footer| natural_height(&footer));
        REFERENCE_MARGIN + natural_height(self.header.upcast_ref()) + sidebar_height + footer_height
    }
}

fn set_css_class(widget: &impl IsA<gtk::Widget>, class: &str, enabled: bool) {
    if enabled {
        widget.add_css_class(class);
    } else {
        widget.remove_css_class(class);
    }
}

fn restyle_category_buttons(categories: &gtk::Box, compact: bool) {
    for button in category_buttons(categories) {
        if let Some(line) = button.child().and_downcast::<gtk::Box>() {
            restyle_category_line(&line, compact);
        }
        button.set_hexpand(!compact);
        button.set_halign(if compact {
            gtk::Align::Start
        } else {
            gtk::Align::Fill
        });
        set_css_class(&button, "action-library-category", compact);
    }
}

fn restyle_category_line(line: &gtk::Box, compact: bool) {
    if let Some(label) = line.first_child().and_downcast::<gtk::Label>() {
        label.set_hexpand(!compact);
    }
    if let Some(count) = line.last_child().and_downcast::<gtk::Label>() {
        count.set_visible(!compact);
    }
}

/// The open reference is drawn in the window's overlay rather than as a
/// popover: compositors dismiss popup surfaces when the window loses focus.
#[derive(Clone)]
pub(super) struct ReferenceSession {
    pub(super) panel: glib::WeakRef<gtk::Box>,
    pub(super) more: glib::WeakRef<gtk::ToggleButton>,
    pub(super) search: glib::WeakRef<gtk::Entry>,
    pub(super) sidebar: glib::WeakRef<gtk::ScrolledWindow>,
    pub(super) scroll: glib::WeakRef<gtk::ScrolledWindow>,
    pub(super) active_area: Rc<Cell<ReferenceArea>>,
    pub(super) layout: ReferenceLayout,
    pub(super) show_hints: Rc<Cell<bool>>,
    pub(super) focus_before: Rc<RefCell<Option<glib::WeakRef<gtk::Widget>>>>,
    pub(super) attached: Rc<RefCell<Option<AttachedReference>>>,
}

pub(super) struct AttachedReference {
    window: glib::WeakRef<gtk::Window>,
    overlay: glib::WeakRef<gtk::Overlay>,
    backdrop: gtk::Box,
    focus_trap: glib::SignalHandlerId,
    resize: gtk::TickCallbackId,
}

/// Where the reference opens.
struct OpenTarget {
    panel: gtk::Box,
    search: gtk::Entry,
    window: gtk::Window,
    overlay: gtk::Overlay,
}

/// Tracks which reference area holds focus and keeps focus inside the panel.
struct FocusTrap {
    panel: glib::WeakRef<gtk::Box>,
    search: glib::WeakRef<gtk::Entry>,
    sidebar: glib::WeakRef<gtk::ScrolledWindow>,
    scroll: glib::WeakRef<gtk::ScrolledWindow>,
    area: Rc<Cell<ReferenceArea>>,
}

fn within(focus: &gtk::Widget, widget: &impl IsA<gtk::Widget>) -> bool {
    *focus == *widget.upcast_ref::<gtk::Widget>() || focus.is_ancestor(widget)
}

impl FocusTrap {
    fn follow(&self, window: &gtk::Window) {
        let (Some(panel), Some(search)) = (self.panel.upgrade(), self.search.upgrade()) else {
            return;
        };
        let Some(focus) = gtk::prelude::RootExt::focus(window) else {
            return;
        };
        if !within(&focus, &panel) {
            search.grab_focus();
            return;
        }
        if let Some(area) = self.area_of(&focus, &search) {
            self.area.set(area);
        }
    }

    fn area_of(&self, focus: &gtk::Widget, search: &gtk::Entry) -> Option<ReferenceArea> {
        if within(focus, search) {
            return Some(ReferenceArea::Search);
        }
        if self
            .sidebar
            .upgrade()
            .is_some_and(|sidebar| within(focus, &sidebar))
        {
            return Some(ReferenceArea::Categories);
        }
        self.scroll
            .upgrade()
            .is_some_and(|scroll| within(focus, &scroll))
            .then_some(ReferenceArea::Results)
    }
}

impl ReferenceSession {
    pub(super) fn is_open(&self) -> bool {
        self.attached.borrow().is_some()
    }

    fn target(&self, anchor: &impl IsA<gtk::Widget>) -> Option<OpenTarget> {
        let (Some(panel), Some(search)) = (self.panel.upgrade(), self.search.upgrade()) else {
            return None;
        };
        let window = anchor.as_ref().root().and_downcast::<gtk::Window>()?;
        let overlay = window.child().and_downcast::<gtk::Overlay>()?;
        Some(OpenTarget {
            panel,
            search,
            window,
            overlay,
        })
    }

    /// Opens over the window holding `anchor`, which must have an overlay child.
    pub(super) fn open(&self, anchor: &impl IsA<gtk::Widget>) -> bool {
        if self.is_open() {
            return true;
        }
        let Some(OpenTarget {
            panel,
            search,
            window,
            overlay,
        }) = self.target(anchor)
        else {
            return false;
        };
        if self.focus_before.borrow().is_none() {
            self.focus_before
                .replace(gtk::prelude::RootExt::focus(&window).map(|widget| widget.downgrade()));
        }
        if let Some(root) = overlay.child().and_downcast::<crate::ui::blur::BlurBin>() {
            root.set_blurred(true);
        }
        let backdrop = self.backdrop();
        overlay.add_overlay(&backdrop);
        overlay.add_overlay(&panel);
        let focus_trap = self.trap_focus(&window);
        self.active_area.set(ReferenceArea::Search);
        if let Some(scroll) = self.scroll.upgrade() {
            scroll.vadjustment().set_value(scroll.vadjustment().lower());
        }
        self.layout.update(&window);
        let resize = self.follow_resize(&panel, &window);
        self.attached.replace(Some(AttachedReference {
            window: window.downgrade(),
            overlay: overlay.downgrade(),
            backdrop,
            focus_trap,
            resize,
        }));
        if let Some(more) = self.more.upgrade() {
            more.set_active(true);
        }
        search.grab_focus();
        true
    }

    fn backdrop(&self) -> gtk::Box {
        let backdrop = gtk::Box::new(gtk::Orientation::Vertical, 0);
        backdrop.add_css_class("search-backdrop");
        backdrop.set_halign(gtk::Align::Fill);
        backdrop.set_valign(gtk::Align::Fill);
        backdrop.set_hexpand(true);
        backdrop.set_vexpand(true);
        let click = gtk::GestureClick::new();
        let session = self.clone();
        click.connect_released(move |_, _, _, _| {
            session.close();
        });
        backdrop.add_controller(click);
        backdrop
    }

    fn follow_resize(&self, panel: &gtk::Box, window: &gtk::Window) -> gtk::TickCallbackId {
        let last_size = Cell::new((window.width(), window.height()));
        let resized_window = window.downgrade();
        let layout = self.layout.clone();
        panel.add_tick_callback(move |_, _| {
            if let Some(window) = resized_window.upgrade() {
                let size = (window.width(), window.height());
                if last_size.replace(size) != size {
                    layout.update(&window);
                }
            }
            glib::ControlFlow::Continue
        })
    }

    fn trap_focus(&self, window: &gtk::Window) -> glib::SignalHandlerId {
        let trap = FocusTrap {
            panel: self.panel.clone(),
            search: self.search.clone(),
            sidebar: self.sidebar.clone(),
            scroll: self.scroll.clone(),
            area: self.active_area.clone(),
        };
        window.connect_focus_widget_notify(move |window| trap.follow(window))
    }

    pub(super) fn close(&self) -> bool {
        let Some(attached) = self.attached.borrow_mut().take() else {
            return false;
        };
        attached.resize.remove();
        if let Some(window) = attached.window.upgrade() {
            window.disconnect(attached.focus_trap);
        }
        if let Some(overlay) = attached.overlay.upgrade() {
            self.detach(&overlay, &attached.backdrop);
        }
        if let Some(more) = self.more.upgrade() {
            more.set_active(false);
            more.set_visible(self.show_hints.get());
        }
        if let Some(previous) = self
            .focus_before
            .borrow_mut()
            .take()
            .and_then(|previous| previous.upgrade())
            .filter(gtk::Widget::is_mapped)
        {
            previous.grab_focus();
        }
        true
    }

    fn detach(&self, overlay: &gtk::Overlay, backdrop: &gtk::Box) {
        if let Some(panel) = self.panel.upgrade() {
            overlay.remove_overlay(&panel);
        }
        overlay.remove_overlay(backdrop);
        if let Some(root) = overlay.child().and_downcast::<crate::ui::blur::BlurBin>() {
            root.set_blurred(false);
        }
    }
}
