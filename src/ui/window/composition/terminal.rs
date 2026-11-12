// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{gio, glib, prelude::*};
use vte4::prelude::*;

use crate::{
    assets,
    ui::{accessibility, browser::BrowserView, keys, preferences::PreferenceManager},
};

mod colors;
mod session;

pub(super) struct PaneTerminal {
    pub(super) widget: gtk::Box,
    pub(super) button: gtk::Button,
    terminal: vte4::Terminal,
    view: BrowserView,
    message: gtk::Label,
    retry: gtk::Button,
    pid: Cell<Option<glib::Pid>>,
    starting: Cell<bool>,
    cancellable: gio::Cancellable,
    directory: RefCell<Option<std::fs::File>>,
}

pub(super) fn button() -> gtk::Button {
    let button = gtk::Button::new();
    button.set_child(Some(&assets::chrome_icon(assets::icons::TERMINAL)));
    button.add_css_class("header-action");
    button.set_cursor_from_name(Some("pointer"));
    let label = crate::i18n::tr("Show or hide the terminal");
    button.set_tooltip_text(Some(&label));
    accessibility::set_label(&button, &label);
    accessibility::set_description(
        &button,
        Some(&keys::label(&keys::Keys::One(
            keys::bindings::PANE_TERMINAL.shown(),
        ))),
    );
    button
}

impl PaneTerminal {
    pub(super) fn new(view: &BrowserView) -> Rc<Self> {
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        widget.add_css_class("pane-terminal");
        widget.set_visible(false);
        let (header, close) = header();
        widget.append(&header);
        let terminal = vte4::Terminal::new();
        terminal.add_css_class("embedded-terminal");
        terminal.set_hexpand(true);
        terminal.set_vexpand(true);
        terminal.set_scrollback_lines(10_000);
        accessibility::set_label(&terminal, &crate::i18n::tr("Terminal"));
        let scroll = gtk::ScrolledWindow::builder()
            .child(&terminal)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        widget.append(&scroll);
        let message = gtk::Label::new(None);
        message.set_wrap(true);
        message.set_visible(false);
        widget.append(&message);
        let retry = gtk::Button::with_label(&crate::i18n::tr("Retry"));
        retry.set_visible(false);
        widget.append(&retry);
        let state = Rc::new(Self {
            widget,
            button: button(),
            terminal,
            view: view.clone(),
            message,
            retry,
            pid: Cell::new(None),
            starting: Cell::new(false),
            cancellable: gio::Cancellable::new(),
            directory: RefCell::new(None),
        });
        state.bind_controls(&close);
        colors::bind(&state.terminal, &PreferenceManager::shared());
        state.install_keys();
        state
    }

    fn bind_controls(self: &Rc<Self>, close: &gtk::Button) {
        for button in [&self.button, close] {
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(state) = weak.upgrade() {
                    state.toggle();
                }
            });
        }
        let weak = Rc::downgrade(self);
        self.retry.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                state.start();
            }
        });
        let weak = Rc::downgrade(self);
        self.terminal.connect_child_exited(move |_, _| {
            if let Some(state) = weak.upgrade() {
                state.pid.set(None);
                state.directory.borrow_mut().take();
                if !state.cancellable.is_cancelled() {
                    state.hide();
                }
            }
        });
    }

    pub(super) fn toggle(self: &Rc<Self>) {
        if self.widget.is_visible() {
            self.hide();
            return;
        }
        self.widget.set_visible(true);
        self.terminal.grab_focus();
        if !self.is_running() {
            self.start();
        }
    }

    pub(in crate::ui::window::composition) fn is_running(&self) -> bool {
        self.pid.get().is_some()
    }

    fn hide(&self) {
        let focused = self
            .widget
            .root()
            .and_then(|root| root.focus())
            .is_some_and(|focus| focus == self.widget || focus.is_ancestor(&self.widget));
        self.widget.set_visible(false);
        if focused {
            self.view.browser().focus_active();
        }
    }

    fn install_keys(&self) {
        let controller = gtk::EventControllerKey::new();
        controller.set_propagation_phase(gtk::PropagationPhase::Capture);
        let terminal = self.terminal.downgrade();
        keys::on_press(&controller, move |_, press| {
            let Some(terminal) = terminal.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if press.is(keys::bindings::TERMINAL_COPY) {
                terminal.copy_clipboard_format(vte4::Format::Text);
            } else if press.is(keys::bindings::TERMINAL_PASTE) {
                terminal.paste_clipboard();
            } else {
                return glib::Propagation::Proceed;
            }
            glib::Propagation::Stop
        });
        self.terminal.add_controller(controller);
    }

    pub(super) fn split(&self, listing: &gtk::Widget) -> gtk::Paned {
        let split = gtk::Paned::new(gtk::Orientation::Vertical);
        split.set_wide_handle(true);
        split.set_start_child(Some(listing));
        split.set_end_child(Some(&self.widget));
        split.set_resize_start_child(true);
        split.set_resize_end_child(false);
        split.set_shrink_start_child(false);
        split.set_shrink_end_child(false);
        split.set_vexpand(true);
        crate::ui::window::sidebar_rail::configure_paned_handle(&split, "row-resize");
        let initialized = Cell::new(false);
        let weak = split.downgrade();
        self.widget.connect_map(move |_| {
            if initialized.replace(true) {
                return;
            }
            let weak = weak.clone();
            glib::idle_add_local_once(move || {
                if let Some(split) = weak.upgrade() {
                    split.set_position(split.height() * 2 / 3);
                }
            });
        });
        split
    }
}

fn header() -> (gtk::Box, gtk::Button) {
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    header.add_css_class("pane-terminal-header");
    let title = gtk::Label::new(Some(&crate::i18n::tr("Terminal")));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    header.append(&title);
    let close = button();
    close.set_child(Some(&assets::chrome_icon(assets::icons::X)));
    header.append(&close);
    (header, close)
}

pub(in crate::ui::window) fn owns_focus(window: &gtk::Window) -> bool {
    std::iter::successors(gtk::prelude::RootExt::focus(window), gtk::Widget::parent)
        .any(|widget| widget.has_css_class("embedded-terminal"))
}

impl Drop for PaneTerminal {
    fn drop(&mut self) {
        self.shutdown();
    }
}
