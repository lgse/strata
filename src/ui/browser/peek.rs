// SPDX-License-Identifier: GPL-3.0-or-later

use crate::model::{FileEntry, Location};
use crate::ui::browser::ViewState;
use crate::ui::browser::collection::cancel_source;
use crate::ui::browser::entry::{
    entry_icon, entry_model_value, icon_for_name, model_display_name, model_is_directory,
};
use crate::ui::browser::presentation::LoadPresentation;
use crate::ui::browser_modes::BrowserMode;
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

const PEEK_WIDTH: i32 = 256;

const PEEK_GAP: f32 = 8.0;

pub(super) struct PeekAnchor {
    pub(super) widget: gtk::Widget,
    pub(super) origin_depth: usize,
}

pub(super) struct PeekView {
    pub(super) revealer: gtk::Revealer,
    pub(super) anchor: gtk::Widget,
    pub(super) location: Location,
    pub(super) presentation: LoadPresentation,
    pub(super) model: gtk::StringList,
    pub(super) entries: Rc<RefCell<Vec<FileEntry>>>,
    pub(super) entry_count: Rc<Cell<usize>>,
    pub(super) spinner: gtk::Spinner,
}

#[derive(Clone, Copy)]
pub struct PeekBehavior {
    pub open_delay: Duration,
    pub close_delay: Duration,
    pub fade_duration: Duration,
    pub item_limit: usize,
}

impl Default for PeekBehavior {
    fn default() -> Self {
        Self {
            open_delay: Duration::from_millis(180),
            close_delay: Duration::from_millis(80),
            fade_duration: Duration::from_millis(150),
            item_limit: 8,
        }
    }
}

fn peek_label_factory(entries: Rc<RefCell<Vec<FileEntry>>>) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("file-row");
        let icon = gtk::Image::new();
        icon.add_css_class("file-icon");
        icon.set_pixel_size(17);
        let label = gtk::Label::builder()
            .halign(gtk::Align::Start)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        let chevron = crate::assets::primary_icon(crate::assets::icons::CHEVRON_RIGHT, 15);
        chevron.add_css_class("file-chevron");
        row.append(&icon);
        row.append(&label);
        row.append(&chevron);
        item.set_child(Some(&row));
    });
    factory.connect_bind(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(value) = item.item().and_downcast::<gtk::StringObject>() else {
            return;
        };
        let Some(row) = item.child().and_downcast::<gtk::Box>() else {
            return;
        };
        let Some(icon) = row.first_child().and_downcast::<gtk::Image>() else {
            return;
        };
        let Some(label) = icon.next_sibling().and_downcast::<gtk::Label>() else {
            return;
        };
        let Some(chevron) = label.next_sibling().and_downcast::<gtk::Image>() else {
            return;
        };
        let value = value.string();
        let name = model_display_name(&value);
        let directory = model_is_directory(&value);
        label.set_label(name);
        if let Some(entry) = entries.borrow().get(item.position() as usize) {
            crate::ui::thumbnail::set_thumbnail_or_icon(&icon, entry, entry_icon(entry), 17, 24);
        } else {
            crate::ui::thumbnail::show_fallback_icon(&icon, icon_for_name(name), 17);
        }
        icon.set_opacity(if directory { 1.0 } else { 0.82 });
        chevron.set_visible(directory);
    });
    factory.connect_unbind(|_, item| crate::ui::thumbnail::cancel_list_item_thumbnails(item));
    factory
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PeekOriginBounds {
    Anchor,
    Column,
}

fn peek_origin_bounds(mode: BrowserMode) -> PeekOriginBounds {
    match mode {
        BrowserMode::Columns => PeekOriginBounds::Column,
        BrowserMode::Icons | BrowserMode::List => PeekOriginBounds::Anchor,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PeekSide {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct PeekPlacement {
    x: f32,
    side: PeekSide,
}

fn peek_transition(side: PeekSide) -> gtk::RevealerTransitionType {
    match side {
        PeekSide::Left => gtk::RevealerTransitionType::SlideLeft,
        PeekSide::Right => gtk::RevealerTransitionType::SlideRight,
    }
}

fn peek_horizontal_layout(placement: PeekPlacement, viewport_width: f32) -> (gtk::Align, i32, i32) {
    match placement.side {
        PeekSide::Right => (gtk::Align::Start, placement.x.round() as i32, 0),
        PeekSide::Left => (
            gtk::Align::End,
            0,
            (viewport_width - placement.x - PEEK_WIDTH as f32)
                .max(0.0)
                .round() as i32,
        ),
    }
}

fn peek_horizontal_placement(
    source_x: f32,
    source_width: f32,
    viewport_width: f32,
) -> Option<PeekPlacement> {
    let right = source_x + source_width + PEEK_GAP;
    if right + PEEK_WIDTH as f32 <= viewport_width {
        return Some(PeekPlacement {
            x: right,
            side: PeekSide::Right,
        });
    }

    let left = source_x - PEEK_GAP - PEEK_WIDTH as f32;
    (left >= 0.0).then_some(PeekPlacement {
        x: left,
        side: PeekSide::Left,
    })
}

pub(super) fn append_peek_entries(peek: &PeekView, entries: Vec<FileEntry>, limit: usize) {
    let remaining = limit.max(1).saturating_sub(peek.entry_count.get());
    let entries = entries.into_iter().take(remaining).collect::<Vec<_>>();
    let mut values = Vec::with_capacity(entries.len());
    values.extend(entries.iter().map(entry_model_value));
    peek.entry_count.set(peek.entry_count.get() + entries.len());
    peek.entries.borrow_mut().extend(entries);
    let refs: Vec<_> = values.iter().map(String::as_str).collect();
    peek.model.splice(peek.model.n_items(), 0, &refs);
}

impl ViewState {
    pub(in crate::ui) fn schedule_peek(
        self: &Rc<Self>,
        origin_depth: usize,
        location: Location,
        anchor: gtk::Widget,
    ) {
        if !self.peek_enabled.get()
            || self.input_ownership.borrow().last_navigation
                == crate::ui::input_ownership::NavigationInput::Keyboard
        {
            return;
        }
        cancel_source(&self.pending_peek);
        cancel_source(&self.pending_close);
        if self.browser.is_open_child(origin_depth, &location) {
            self.peek_anchor.take();
            self.browser.close_peek();
            return;
        }
        if self
            .peek
            .borrow()
            .as_ref()
            .is_some_and(|peek| peek.location == location)
        {
            return;
        }
        self.peek_anchor.replace(Some(PeekAnchor {
            widget: anchor,
            origin_depth,
        }));

        let weak_state = Rc::downgrade(self);
        let source = glib::timeout_add_local_once(self.peek_behavior.open_delay, move || {
            if let Some(state) = weak_state.upgrade() {
                state.pending_peek.take();
                state.browser.begin_peek(origin_depth, location);
            }
        });
        self.pending_peek.replace(Some(source));
    }

    pub(in crate::ui) fn schedule_close_peek(self: &Rc<Self>) {
        cancel_source(&self.pending_peek);
        cancel_source(&self.pending_close);

        let weak_state = Rc::downgrade(self);
        let source = glib::timeout_add_local_once(self.peek_behavior.close_delay, move || {
            if let Some(state) = weak_state.upgrade() {
                state.pending_close.take();
                state.browser.close_peek();
            }
        });
        self.pending_close.replace(Some(source));
    }

    pub(super) fn append_peek(self: &Rc<Self>, location: &Location) {
        let anchor = self.peek_anchor.take();
        self.close_peek_visual();
        let Some(anchor) = anchor else {
            self.browser.close_peek();
            return;
        };
        let Some(row_bounds) = anchor.widget.compute_bounds(&self.overlay) else {
            self.browser.close_peek();
            return;
        };
        let source_bounds = match peek_origin_bounds(self.mode_views.borrow().mode()) {
            PeekOriginBounds::Anchor => row_bounds,
            PeekOriginBounds::Column => {
                let Some(bounds) = self
                    .columns
                    .borrow()
                    .get(anchor.origin_depth)
                    .and_then(|column| column.shell.compute_bounds(&self.overlay))
                else {
                    self.browser.close_peek();
                    return;
                };
                bounds
            }
        };
        let Some(placement) = peek_horizontal_placement(
            source_bounds.x(),
            source_bounds.width(),
            self.overlay.width() as f32,
        ) else {
            self.browser.close_peek();
            return;
        };

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.set_size_request(PEEK_WIDTH, -1);
        content.set_overflow(gtk::Overflow::Hidden);

        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        header.add_css_class("column-header");
        let heading = gtk::Label::new(Some(&location.display_name()));
        heading.set_xalign(0.0);
        heading.set_hexpand(true);
        let spinner = gtk::Spinner::new();
        spinner.start();
        header.append(&heading);
        header.append(&spinner);
        content.append(&header);

        let entry_count = Rc::new(Cell::new(0));
        let entries = Rc::new(RefCell::new(Vec::new()));
        let model = gtk::StringList::new(&[]);
        let selection = gtk::NoSelection::new(Some(model.clone()));
        let factory = peek_label_factory(entries.clone());
        let list = gtk::ListView::new(Some(selection), Some(factory));
        list.set_focusable(false);
        list.add_css_class("file-list");
        let weak_browser = Rc::downgrade(&self.browser);
        list.connect_activate(move |_, _| {
            if let Some(browser) = weak_browser.upgrade() {
                browser.commit_peek();
            }
        });
        let scroll = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .max_content_height(240)
            .propagate_natural_height(true)
            .build();
        let presentation = LoadPresentation::new(&scroll, None);
        presentation.stack.set_size_request(-1, 120);
        content.append(&presentation.stack);

        let motion = gtk::EventControllerMotion::new();
        let weak_state = Rc::downgrade(self);
        motion.connect_enter(move |_, _, _| {
            if let Some(state) = weak_state.upgrade() {
                cancel_source(&state.pending_close);
            }
        });
        let weak_state = Rc::downgrade(self);
        motion.connect_leave(move |_| {
            if let Some(state) = weak_state.upgrade() {
                state.schedule_close_peek();
            }
        });
        content.add_controller(motion);

        let click = gtk::GestureClick::new();
        let weak_browser = Rc::downgrade(&self.browser);
        click.connect_released(move |_, _, _, _| {
            if let Some(browser) = weak_browser.upgrade() {
                browser.commit_peek();
            }
        });
        content.add_controller(click);

        content.add_css_class("peek-popover");
        let transition_duration = self
            .peek_behavior
            .fade_duration
            .as_millis()
            .min(u128::from(u32::MAX)) as u32;
        let transition_type = peek_transition(placement.side);
        let (halign, margin_start, margin_end) =
            peek_horizontal_layout(placement, self.overlay.width() as f32);
        let revealer = gtk::Revealer::builder()
            .child(&content)
            .transition_type(transition_type)
            .transition_duration(transition_duration)
            .reveal_child(false)
            .halign(halign)
            .valign(gtk::Align::Start)
            .margin_start(margin_start)
            .margin_end(margin_end)
            .margin_top(row_bounds.y().round().max(0.0) as i32)
            .build();
        self.overlay.add_overlay(&revealer);
        self.overlay.add_css_class("peek-open");
        anchor.widget.add_css_class("peek-anchor");
        self.peek.replace(Some(PeekView {
            revealer: revealer.clone(),
            anchor: anchor.widget,
            location: location.clone(),
            presentation,
            model,
            entries,
            entry_count,
            spinner,
        }));
        glib::idle_add_local_once(move || revealer.set_reveal_child(true));
    }

    pub(super) fn close_peek_visual(&self) {
        cancel_source(&self.pending_peek);
        cancel_source(&self.pending_close);
        self.overlay.remove_css_class("peek-open");
        self.peek_anchor.take();
        if let Some(peek) = self.peek.take() {
            peek.anchor.remove_css_class("peek-anchor");
            peek.revealer.set_can_target(false);
            peek.revealer.set_reveal_child(false);
            let overlay = self.overlay.clone();
            let revealer = peek.revealer;
            let delay = Duration::from_millis(u64::from(revealer.transition_duration()));
            glib::timeout_add_local_once(delay, move || overlay.remove_overlay(&revealer));
        }
    }
}

#[cfg(test)]
mod tests;
