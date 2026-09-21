// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    path::Path,
    rc::{Rc, Weak},
};

use gtk::{gdk, glib, prelude::*};

use crate::{app::Browser, model::FileEntry, services::PreviewContent};

use super::{PreviewDrawer, preview_target};

const SLIDESHOW_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3);
const INDEX_THUMBNAIL_SIZE: i32 = 112;

fn popup_action(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::builder().tooltip_text(tooltip).build();
    button.set_child(Some(&crate::assets::chrome_icon(icon)));
    super::super::controls::pane_header_action(&button);
    super::super::accessibility::set_label(&button, tooltip);
    button
}

fn entry_content_type(entry: &FileEntry) -> glib::GString {
    gtk::gio::content_type_guess(Some(Path::new(&entry.native_name)), None::<&[u8]>).0
}

fn entry_supports_rotation(entry: &FileEntry) -> bool {
    matches!(
        crate::services::content_family(&entry_content_type(entry)),
        PreviewContent::Image
    )
}

struct PopupInner {
    window: gtk::Window,
    parent: glib::WeakRef<gtk::ApplicationWindow>,
    drawer: PreviewDrawer,
    stage: gtk::Overlay,
    content_stack: gtk::Stack,
    title_label: gtk::Label,
    navigation: gtk::Box,
    counter_label: gtk::Label,
    index: gtk::ToggleButton,
    index_grid: gtk::FlowBox,
    index_summary: gtk::Label,
    rotate: gtk::Button,
    play: gtk::ToggleButton,
    pane_was_open: Cell<bool>,
    closing: Cell<bool>,
    browser: RefCell<Option<Weak<Browser>>>,
    current_depth: Cell<Option<usize>>,
    selected_positions: RefCell<Vec<usize>>,
    selected_index: Cell<usize>,
    slideshow: RefCell<Option<glib::SourceId>>,
}

#[derive(Clone)]
pub struct PreviewPopup {
    inner: Rc<PopupInner>,
}

impl PreviewPopup {
    pub fn new(parent: &gtk::ApplicationWindow, drawer: PreviewDrawer) -> Self {
        let window = gtk::Window::builder()
            .transient_for(parent)
            .modal(false)
            .title("Quick Look")
            .default_width(760)
            .default_height(600)
            .build();
        window.add_css_class("quick-look-window");

        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        toolbar.add_css_class("mode-pane-header");

        let close = popup_action(crate::assets::icons::X, "Close Quick Look (Space)");
        let fullscreen = popup_action(crate::assets::icons::MAXIMIZE_2, "Toggle fullscreen (F)");
        let title_label = gtk::Label::new(None);
        title_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        title_label.set_hexpand(true);
        title_label.set_max_width_chars(1);
        title_label.set_xalign(0.0);

        toolbar.append(&close);
        toolbar.append(&fullscreen);

        let navigation = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        navigation.add_css_class("list-navigation");
        navigation.set_visible(false);

        let prev = popup_action(
            crate::assets::icons::ARROW_LEFT,
            "Previous selected item (Left / Up)",
        );
        let counter_label = gtk::Label::builder().valign(gtk::Align::Center).build();
        counter_label.add_css_class("quick-look-counter");

        let next = popup_action(
            crate::assets::icons::ARROW_RIGHT,
            "Next selected item (Right / Down)",
        );
        navigation.append(&prev);
        navigation.append(&counter_label);
        navigation.append(&next);

        let index = gtk::ToggleButton::builder()
            .tooltip_text("Show selected items in a grid (Ctrl+Enter)")
            .build();
        index.set_child(Some(&crate::assets::chrome_icon(
            crate::assets::icons::ICONS,
        )));
        super::super::controls::pane_header_action(&index);
        index.set_visible(false);
        super::super::accessibility::set_label(&index, "Show selected items in a grid");

        let rotate = popup_action(
            crate::assets::icons::REFRESH,
            "Rotate left (Ctrl+R; Alt+R rotates right)",
        );
        let play = gtk::ToggleButton::builder()
            .tooltip_text("Play selected items as a slideshow")
            .build();
        play.set_child(Some(&crate::assets::chrome_icon(
            crate::assets::icons::PLAY,
        )));
        super::super::controls::pane_header_action(&play);
        super::super::accessibility::set_label(&play, "Play selected items as a slideshow");

        toolbar.append(&navigation);
        toolbar.append(&title_label);
        toolbar.append(&index);
        toolbar.append(&rotate);
        toolbar.append(&play);
        window.set_titlebar(Some(&toolbar));

        let index_grid = gtk::FlowBox::builder()
            .column_spacing(18)
            .row_spacing(18)
            .homogeneous(true)
            .max_children_per_line(5)
            .min_children_per_line(1)
            .selection_mode(gtk::SelectionMode::None)
            .valign(gtk::Align::Start)
            .build();
        index_grid.add_css_class("quick-look-index-grid");
        let index_summary = gtk::Label::new(None);
        index_summary.add_css_class("quick-look-index-summary");
        let index_scroll = gtk::ScrolledWindow::builder()
            .child(&index_grid)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .hexpand(true)
            .vexpand(true)
            .build();
        let index_page = gtk::Box::new(gtk::Orientation::Vertical, 16);
        index_page.add_css_class("quick-look-index");
        index_page.append(&index_summary);
        index_page.append(&index_scroll);

        let content_stack = gtk::Stack::new();
        content_stack.set_transition_type(gtk::StackTransitionType::None);
        content_stack.add_named(&index_page, Some("index"));

        let stage = gtk::Overlay::new();
        stage.add_css_class("quick-look-stage");
        stage.set_focusable(true);
        stage.set_can_target(true);
        stage.set_child(Some(&content_stack));
        window.set_child(Some(&stage));

        let popup = Self {
            inner: Rc::new(PopupInner {
                window: window.clone(),
                parent: parent.downgrade(),
                drawer,
                stage,
                content_stack,
                title_label,
                navigation,
                counter_label: counter_label.clone(),
                index: index.clone(),
                index_grid,
                index_summary,
                rotate: rotate.clone(),
                play: play.clone(),
                pane_was_open: Cell::new(false),
                closing: Cell::new(false),
                browser: RefCell::new(None),
                current_depth: Cell::new(None),
                selected_positions: RefCell::new(Vec::new()),
                selected_index: Cell::new(0),
                slideshow: RefCell::new(None),
            }),
        };

        let closed = popup.clone();
        window.connect_close_request(move |_| {
            closed.close();
            glib::Propagation::Stop
        });

        let popup_keys = popup.clone();
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            if popup_keys.handle_video_key(key, modifiers) {
                return glib::Propagation::Stop;
            }
            if modifiers.contains(gdk::ModifierType::ALT_MASK)
                && (key == gdk::Key::r || key == gdk::Key::R)
            {
                popup_keys.rotate_clockwise();
                return glib::Propagation::Stop;
            }
            if modifiers.intersects(gdk::ModifierType::ALT_MASK | gdk::ModifierType::SUPER_MASK) {
                return glib::Propagation::Proceed;
            }
            if modifiers.contains(gdk::ModifierType::CONTROL_MASK) {
                match key {
                    gdk::Key::r | gdk::Key::R => {
                        if modifiers.contains(gdk::ModifierType::SHIFT_MASK) {
                            popup_keys.rotate_clockwise();
                        } else {
                            popup_keys.rotate_counter_clockwise();
                        }
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::plus | gdk::Key::equal | gdk::Key::KP_Add => {
                        popup_keys.zoom_in();
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::minus | gdk::Key::underscore | gdk::Key::KP_Subtract => {
                        popup_keys.zoom_out();
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::_0 | gdk::Key::KP_0 => {
                        popup_keys.reset_zoom();
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::Up | gdk::Key::Home => {
                        popup_keys.jump_first();
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::Down | gdk::Key::End => {
                        popup_keys.jump_last();
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::y | gdk::Key::Y => {
                        popup_keys.close();
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::Return | gdk::Key::KP_Enter => {
                        popup_keys.toggle_index();
                        return glib::Propagation::Stop;
                    }
                    _ => return glib::Propagation::Proceed,
                }
            }
            if modifiers.contains(gdk::ModifierType::SHIFT_MASK)
                && matches!(
                    key,
                    gdk::Key::Left | gdk::Key::Up | gdk::Key::Right | gdk::Key::Down
                )
            {
                if let Some(browser) = popup_keys
                    .inner
                    .browser
                    .borrow()
                    .as_ref()
                    .and_then(Weak::upgrade)
                {
                    let direction = if matches!(key, gdk::Key::Left | gdk::Key::Up) {
                        -1
                    } else {
                        1
                    };
                    browser.extend_selection(direction);
                }
                return glib::Propagation::Stop;
            }
            match key {
                gdk::Key::Escape | gdk::Key::space => {
                    popup_keys.close();
                    glib::Propagation::Stop
                }
                gdk::Key::Up
                | gdk::Key::Left
                | gdk::Key::k
                | gdk::Key::K
                | gdk::Key::h
                | gdk::Key::H => {
                    popup_keys.navigate_previous();
                    glib::Propagation::Stop
                }
                gdk::Key::Down
                | gdk::Key::Right
                | gdk::Key::j
                | gdk::Key::J
                | gdk::Key::l
                | gdk::Key::L => {
                    popup_keys.navigate_next();
                    glib::Propagation::Stop
                }
                gdk::Key::Page_Up => {
                    popup_keys.page_previous();
                    glib::Propagation::Stop
                }
                gdk::Key::Page_Down => {
                    popup_keys.page_next();
                    glib::Propagation::Stop
                }
                gdk::Key::Home => {
                    popup_keys.jump_first();
                    glib::Propagation::Stop
                }
                gdk::Key::End => {
                    popup_keys.jump_last();
                    glib::Propagation::Stop
                }
                gdk::Key::f | gdk::Key::F | gdk::Key::F11 => {
                    popup_keys.toggle_fullscreen();
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        window.add_controller(keys);

        let close_popup = popup.clone();
        close.connect_clicked(move |_| close_popup.close());

        let prev_popup = popup.clone();
        prev.connect_clicked(move |_| prev_popup.navigate_previous());

        let next_popup = popup.clone();
        next.connect_clicked(move |_| next_popup.navigate_next());

        let indexed = popup.clone();
        index.connect_toggled(move |button| indexed.set_index_visible(button.is_active()));

        let rotate_popup = popup.clone();
        rotate.connect_clicked(move |_| rotate_popup.rotate_counter_clockwise());

        let toggled = popup.clone();
        fullscreen.connect_clicked(move |_| toggled.toggle_fullscreen());

        let played = popup.clone();
        play.connect_toggled(move |button| {
            if button.is_active() {
                played.start_slideshow();
            } else {
                played.stop_slideshow();
            }
        });

        let hidden = popup.clone();
        let revealer = popup
            .inner
            .drawer
            .widget()
            .downcast::<gtk::Revealer>()
            .expect("preview revealer");
        revealer.connect_reveal_child_notify(move |revealer| {
            if !revealer.reveals_child() {
                hidden.close();
            }
        });

        popup
    }

    pub fn observe_browser(&self, browser: &Rc<Browser>) {
        self.inner.browser.replace(Some(Rc::downgrade(browser)));
        let popup = self.clone();
        let weak_browser = Rc::downgrade(browser);
        browser.observe(move |event| {
            let Some(browser) = weak_browser.upgrade() else {
                return;
            };
            popup.handle_browser_event(&browser, event);
        });
    }

    fn handle_browser_event(&self, browser: &Browser, event: &crate::app::BrowserEvent) {
        match event {
            crate::app::BrowserEvent::PreviewRequested { entry } => {
                self.update_title(Some(entry));
            }
            crate::app::BrowserEvent::FocusChanged {
                depth,
                position: Some(pos),
            }
            | crate::app::BrowserEvent::SelectionSynced {
                depth,
                focused: Some(pos),
            } => {
                if self.is_open() {
                    self.stop_active_slideshow();
                    self.sync_selection_and_title(browser, *depth, *pos);
                }
            }
            crate::app::BrowserEvent::SelectionSetChanged {
                depth,
                focused: pos,
                ..
            } => {
                if self.is_open() {
                    self.stop_active_slideshow();
                    self.sync_selection_and_title(browser, *depth, *pos);
                }
            }
            crate::app::BrowserEvent::FocusChanged { position: None, .. }
            | crate::app::BrowserEvent::SelectionSynced { focused: None, .. }
                if self.is_open() =>
            {
                self.stop_active_slideshow();
                self.clear_selection_context();
            }
            crate::app::BrowserEvent::EntriesSpliced { .. } if self.is_open() => {
                self.stop_active_slideshow();
                if let Some((depth, position, _)) = browser.focused_item() {
                    self.sync_selection_and_title(browser, depth, position);
                } else {
                    self.clear_selection_context();
                }
            }
            _ => {}
        }
    }

    fn sync_selection_and_title(&self, browser: &Browser, depth: usize, pos: usize) {
        self.inner.current_depth.set(Some(depth));
        let positions = browser.selected_positions(depth);
        if positions.len() > 1 {
            let idx = positions.iter().position(|&p| p == pos).unwrap_or(0);
            self.inner.selected_index.set(idx);
            *self.inner.selected_positions.borrow_mut() = positions;
        } else {
            self.inner.selected_positions.borrow_mut().clear();
            self.inner.selected_index.set(0);
        }
        if self.index_is_visible() {
            self.rebuild_index();
        }
        if let Some(entry) = browser.entry_at(depth, pos) {
            self.update_title(Some(&entry));
        } else {
            self.update_title(None);
        }
    }

    fn clear_selection_context(&self) {
        self.inner.selected_positions.borrow_mut().clear();
        self.inner.selected_index.set(0);
        self.update_title(None);
    }

    fn update_title(&self, entry: Option<&FileEntry>) {
        let count = self.inner.selected_positions.borrow().len();
        let has_multiple = count > 1;
        self.inner.navigation.set_visible(has_multiple);
        self.inner.index.set_visible(has_multiple);
        self.inner.play.set_visible(has_multiple);
        self.inner
            .rotate
            .set_visible(entry.is_some_and(entry_supports_rotation));
        if has_multiple {
            let idx = self.inner.selected_index.get().min(count - 1) + 1;
            self.inner
                .counter_label
                .set_text(&format!("{idx} of {count}"));
        } else {
            self.set_index_visible(false);
            self.stop_active_slideshow();
        }
        if let Some(entry) = entry {
            let title = &entry.display_name;
            self.inner.title_label.set_text(title);
            self.inner.title_label.set_tooltip_text(Some(title));
            self.inner
                .window
                .set_title(Some(&format!("{title} — Quick Look")));
        } else {
            self.inner.title_label.set_text("Quick Look");
            self.inner.title_label.set_tooltip_text(None);
            self.inner.window.set_title(Some("Quick Look"));
        }
    }

    fn index_is_visible(&self) -> bool {
        self.inner.content_stack.visible_child_name().as_deref() == Some("index")
            && self.inner.index.is_active()
    }

    fn toggle_index(&self) {
        let show = !self.index_is_visible();
        if show {
            self.stop_active_slideshow();
        }
        self.set_index_visible(show);
    }

    fn set_index_visible(&self, visible: bool) {
        let visible = visible
            && self.inner.selected_positions.borrow().len() > 1
            && self.inner.content_stack.child_by_name("preview").is_some();
        if visible {
            self.rebuild_index();
            self.inner.content_stack.set_visible_child_name("index");
        } else if self.inner.content_stack.child_by_name("preview").is_some() {
            self.inner.content_stack.set_visible_child_name("preview");
        }
        if self.inner.index.is_active() != visible {
            self.inner.index.set_active(visible);
        }
    }

    fn rebuild_index(&self) {
        self.clear_index();
        let Some(browser) = self.inner.browser.borrow().as_ref().and_then(Weak::upgrade) else {
            return;
        };
        let depth = self
            .inner
            .current_depth
            .get()
            .or_else(|| browser.active_depth())
            .unwrap_or(0);
        let positions = self.inner.selected_positions.borrow().clone();
        self.inner
            .index_summary
            .set_text(&format!("{} selected items", positions.len()));
        for (index, position) in positions.into_iter().enumerate() {
            let Some(entry) = browser.entry_at(depth, position) else {
                continue;
            };
            let card = super::super::icons_cell::new_card(INDEX_THUMBNAIL_SIZE);
            if let Some((thumbnail, label)) = super::super::icons_cell::parts(&card) {
                label.set_text(Some(&entry.display_name));
                label.set_tooltip_text(Some(&entry.display_name));
                super::super::thumbnail::set_thumbnail_or_icon(
                    &thumbnail,
                    &entry,
                    super::super::browser::entry_icon(&entry),
                    48,
                    INDEX_THUMBNAIL_SIZE,
                );
            }
            let button = gtk::Button::builder().child(&card).build();
            button.add_css_class("quick-look-index-item");
            if index == self.inner.selected_index.get() {
                button.add_css_class("selected");
            }
            super::super::accessibility::set_label(
                &button,
                &format!("Preview {}", entry.display_name),
            );
            let weak = Rc::downgrade(&self.inner);
            button.connect_clicked(move |_| {
                if let Some(inner) = weak.upgrade() {
                    let popup = PreviewPopup { inner };
                    popup.stop_active_slideshow();
                    popup.show_selected_index(index, true);
                }
            });
            self.inner.index_grid.append(&button);
        }
    }

    fn clear_index(&self) {
        while let Some(child) = self.inner.index_grid.first_child() {
            super::super::thumbnail::cancel_thumbnails_in(&child);
            self.inner.index_grid.remove(&child);
        }
    }

    fn update_index_highlight(&self) {
        let selected = self.inner.selected_index.get();
        let mut child = self.inner.index_grid.first_child();
        let mut index = 0;
        while let Some(current) = child {
            child = current.next_sibling();
            if let Some(button) = current.first_child().and_downcast::<gtk::Button>() {
                if index == selected {
                    button.add_css_class("selected");
                } else {
                    button.remove_css_class("selected");
                }
            }
            index += 1;
        }
    }

    fn show_selected_index(&self, index: usize, reveal_preview: bool) -> bool {
        let Some(browser) = self.inner.browser.borrow().as_ref().and_then(Weak::upgrade) else {
            return false;
        };
        let Some(position) = self.inner.selected_positions.borrow().get(index).copied() else {
            return false;
        };
        let depth = self
            .inner
            .current_depth
            .get()
            .or_else(|| browser.active_depth())
            .unwrap_or(0);
        let Some(entry) = browser.entry_at(depth, position) else {
            return false;
        };
        self.inner.selected_index.set(index);
        self.update_index_highlight();
        self.update_title(Some(&entry));
        if let Some(entry) = preview_target(Some(entry)) {
            self.inner.drawer.show(entry, Some(depth));
        } else {
            self.inner.drawer.clear_target();
        }
        if reveal_preview {
            self.set_index_visible(false);
        }
        true
    }

    pub fn navigate_previous(&self) {
        self.stop_active_slideshow();
        let count = self.inner.selected_positions.borrow().len();
        if count > 1 {
            let current = self.inner.selected_index.get().min(count - 1);
            let previous = if current == 0 { count - 1 } else { current - 1 };
            self.show_selected_index(previous, true);
        } else if let Some(browser) = self.inner.browser.borrow().as_ref().and_then(Weak::upgrade) {
            browser.move_selection(-1);
        }
    }

    pub fn navigate_next(&self) {
        self.stop_active_slideshow();
        let count = self.inner.selected_positions.borrow().len();
        if count > 1 {
            let next = (self.inner.selected_index.get() + 1) % count;
            self.show_selected_index(next, true);
        } else if let Some(browser) = self.inner.browser.borrow().as_ref().and_then(Weak::upgrade) {
            browser.move_selection(1);
        }
    }

    pub fn page_previous(&self) {
        if let Some(browser) = self.inner.browser.borrow().as_ref().and_then(Weak::upgrade) {
            browser.page_along(-1, 10, None);
        }
    }

    pub fn page_next(&self) {
        if let Some(browser) = self.inner.browser.borrow().as_ref().and_then(Weak::upgrade) {
            browser.page_along(1, 10, None);
        }
    }

    pub fn jump_first(&self) {
        if let Some(browser) = self.inner.browser.borrow().as_ref().and_then(Weak::upgrade) {
            browser.page_along(-1, usize::MAX, None);
        }
    }

    pub fn jump_last(&self) {
        if let Some(browser) = self.inner.browser.borrow().as_ref().and_then(Weak::upgrade) {
            browser.page_along(1, usize::MAX, None);
        }
    }

    pub fn is_open(&self) -> bool {
        self.inner.drawer.is_floating() && self.inner.window.is_visible()
    }

    pub fn handle_video_key(&self, key: gdk::Key, modifiers: gdk::ModifierType) -> bool {
        self.inner.drawer.handle_video_key(key, modifiers)
    }

    #[cfg(test)]
    pub fn widget(&self) -> gtk::Widget {
        self.inner.drawer.widget()
    }

    #[cfg(test)]
    pub fn window(&self) -> gtk::Window {
        self.inner.window.clone()
    }

    pub fn toggle(&self, entry: Option<FileEntry>, depth: Option<usize>) {
        if self.is_open() {
            self.close();
        } else {
            self.open(entry, depth);
        }
    }

    pub fn open(&self, entry: Option<FileEntry>, depth: Option<usize>) {
        let Some(entry) = entry.and_then(|entry| preview_target(Some(entry))) else {
            return;
        };
        self.inner.current_depth.set(depth);
        if let Some(browser) = self.inner.browser.borrow().as_ref().and_then(Weak::upgrade) {
            let d = depth.or_else(|| browser.active_depth()).unwrap_or(0);
            let positions = browser.selected_positions(d);
            if positions.len() > 1 {
                let focused_pos = browser.focused_item().map(|(_, pos, _)| pos);
                let idx = focused_pos
                    .and_then(|p| positions.iter().position(|&x| x == p))
                    .unwrap_or(0);
                *self.inner.selected_positions.borrow_mut() = positions;
                self.inner.selected_index.set(idx);
            } else {
                self.inner.selected_positions.borrow_mut().clear();
                self.inner.selected_index.set(0);
            }
        }
        self.update_title(Some(&entry));
        if !self.inner.drawer.is_floating() {
            self.inner.pane_was_open.set(self.inner.drawer.is_enabled());
            let widget = self.inner.drawer.float_to();
            self.inner.content_stack.add_named(&widget, Some("preview"));
        }
        self.inner.drawer.show(entry, depth);
        self.set_index_visible(false);
        self.inner.window.set_visible(true);
        self.inner.window.present();
        self.inner.stage.grab_focus();
    }

    pub fn open_fullscreen(&self, entry: Option<FileEntry>, depth: Option<usize>) {
        self.open(entry, depth);
        if self.is_open() {
            self.inner.window.fullscreen();
        }
    }

    pub fn toggle_fullscreen(&self) {
        if self.inner.window.is_fullscreen() {
            self.inner.window.unfullscreen();
        } else if self.is_open() {
            self.inner.window.fullscreen();
        }
    }

    pub fn close(&self) {
        if self.inner.closing.replace(true) {
            return;
        }
        if !self.inner.drawer.is_floating() {
            self.stop_slideshow();
            self.inner.play.set_active(false);
            self.set_index_visible(false);
            if let Some(preview) = self.inner.content_stack.child_by_name("preview") {
                self.inner.content_stack.remove(&preview);
            }
            self.clear_index();
            self.inner.selected_positions.borrow_mut().clear();
            self.inner.selected_index.set(0);
            self.inner.window.set_visible(false);
            self.inner.closing.set(false);
            return;
        }
        self.stop_slideshow();
        self.inner.play.set_active(false);
        self.set_index_visible(false);
        if self.inner.window.is_fullscreen() {
            self.inner.window.unfullscreen();
        }
        if let Some(preview) = self.inner.content_stack.child_by_name("preview") {
            self.inner.content_stack.remove(&preview);
        }
        if self.inner.pane_was_open.get() {
            self.inner.drawer.dock_from_float();
        } else {
            self.inner.drawer.close();
        }
        self.clear_index();
        self.inner.selected_positions.borrow_mut().clear();
        self.inner.selected_index.set(0);
        self.inner.window.set_visible(false);
        if let Some(parent) = self.inner.parent.upgrade() {
            parent.present();
        }
        self.inner.closing.set(false);
    }

    fn start_slideshow(&self) {
        self.stop_slideshow();
        if self.inner.selected_positions.borrow().len() < 2 {
            self.inner.play.set_active(false);
            return;
        }
        self.set_index_visible(false);
        if let Some(icon) = self.inner.play.child().and_downcast::<gtk::Image>() {
            crate::assets::set_primary_icon(&icon, crate::assets::icons::PAUSE);
        }
        let count = self.inner.selected_positions.borrow().len();
        let next = (self.inner.selected_index.get() + 1) % count;
        self.show_selected_index(next, true);
        let weak = Rc::downgrade(&self.inner);
        let id = glib::timeout_add_local(SLIDESHOW_INTERVAL, move || {
            let Some(inner) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let popup = PreviewPopup { inner };
            let count = popup.inner.selected_positions.borrow().len();
            if count < 2 {
                return glib::ControlFlow::Break;
            }
            let next = (popup.inner.selected_index.get() + 1) % count;
            popup.show_selected_index(next, true);
            glib::ControlFlow::Continue
        });
        self.inner.slideshow.replace(Some(id));
    }

    fn stop_active_slideshow(&self) {
        if self.inner.play.is_active() {
            self.inner.play.set_active(false);
        } else {
            self.stop_slideshow();
        }
    }

    fn stop_slideshow(&self) {
        if let Some(id) = self.inner.slideshow.borrow_mut().take() {
            id.remove();
        }
        if let Some(icon) = self.inner.play.child().and_downcast::<gtk::Image>() {
            crate::assets::set_primary_icon(&icon, crate::assets::icons::PLAY);
        }
    }

    fn rotate_clockwise(&self) {
        self.inner.drawer.rotate_clockwise();
    }

    fn rotate_counter_clockwise(&self) {
        self.inner.drawer.rotate_counter_clockwise();
    }

    fn zoom_in(&self) {
        self.inner.drawer.zoom_in();
    }

    fn zoom_out(&self) {
        self.inner.drawer.zoom_out();
    }

    fn reset_zoom(&self) {
        self.inner.drawer.reset_zoom();
    }
}

#[cfg(test)]
mod tests;
