// SPDX-License-Identifier: MIT

use std::rc::Weak;

use super::*;
use crate::ui::{
    browser::{BrowserView, COLUMN_WIDTH, WeakBrowserView},
    browser_modes::BrowserMode,
    window::{
        MIN_SIDEBAR_WIDTH, SidebarState, SidebarView, preferred_sidebar_width, sidebar_rail_width,
    },
};

const MIN_COLUMN_MULTIPLIER: i32 = 2;
const MIN_SPLIT_PREVIEW_WIDTH: i32 = 240;

#[derive(Default)]
pub(super) struct SplitSizing {
    binding: RefCell<Option<BrowserBinding>>,
    manual_width: Cell<Option<i32>>,
    resizing: Cell<bool>,
    suspended: Cell<bool>,
    resume_media: Cell<bool>,
    reload_on_resume: Cell<bool>,
    sidebar_railed: Cell<bool>,
    sidebar_saved_width: Cell<i32>,
}

impl SplitSizing {
    pub(super) fn close(&self) {
        self.resizing.set(false);
        self.suspended.set(false);
        self.resume_media.set(false);
        self.reload_on_resume.set(false);
    }

    pub(super) fn defer_load(&self) {
        self.reload_on_resume.set(true);
        self.resume_media.set(false);
    }

    pub(super) fn play_or_defer(&self, media: &gtk::MediaStream) {
        if self.suspended.get() {
            self.resume_media.set(true);
        } else {
            media.play();
        }
    }

    pub(super) fn browser(&self) -> Option<BrowserView> {
        self.binding
            .borrow()
            .as_ref()
            .and_then(|binding| binding.browser.upgrade())
    }

    pub(super) fn is_suspended(&self) -> bool {
        self.suspended.get()
    }
}

struct BrowserBinding {
    content: glib::WeakRef<gtk::Paned>,
    browser: WeakBrowserView,
    sidebar: Option<Weak<SidebarState>>,
}

#[derive(Clone, Copy)]
struct Geometry {
    available: i32,
    occupied: i32,
    start_minimum: i32,
    separator: i32,
    columns: bool,
}

impl Geometry {
    fn can_show_preview(self) -> bool {
        self.available - self.separator - self.start_minimum >= MIN_SPLIT_PREVIEW_WIDTH
    }

    fn maximum_width(self) -> i32 {
        (self.available - self.separator - self.start_minimum).max(1)
    }

    fn minimum_width(self, manual: bool) -> i32 {
        let minimum = if manual {
            COLUMN_WIDTH
        } else if self.columns {
            COLUMN_WIDTH * MIN_COLUMN_MULTIPLIER
        } else {
            MIN_WIDTH
        };
        minimum.min(self.maximum_width())
    }

    fn preview_width(self, manual: Option<i32>) -> i32 {
        let free = (self.available - self.separator - self.occupied).max(0);
        let desired = manual.unwrap_or_else(|| {
            if self.columns {
                free
            } else {
                free.saturating_mul(9).saturating_div(10).min(MAX_WIDTH)
            }
        });
        desired.clamp(self.minimum_width(manual.is_some()), self.maximum_width())
    }

    fn position(self, manual: Option<i32>) -> i32 {
        self.available - self.separator - self.preview_width(manual)
    }
}

fn separator(split: &gtk::Paned) -> Option<gtk::Widget> {
    let mut child = split.first_child();
    while let Some(widget) = child {
        if widget.css_name() == "separator" {
            return Some(widget);
        }
        child = widget.next_sibling();
    }
    None
}

fn separator_width(split: &gtk::Paned) -> i32 {
    separator(split).map_or(0, |handle| {
        handle.measure(gtk::Orientation::Horizontal, -1).0
    })
}

fn sidebar_width(content: &gtk::Paned) -> i32 {
    if content
        .start_child()
        .is_some_and(|child| child.get_visible())
    {
        content.position() + separator_width(content)
    } else {
        0
    }
}

impl PreviewDrawer {
    pub(in crate::ui) fn attach_split(
        &self,
        split: &gtk::Paned,
        content: &gtk::Paned,
        browser: &BrowserView,
        sidebar: Option<&SidebarView>,
    ) {
        self.state.split.replace(Some(split.clone()));
        self.state.sizing.binding.replace(Some(BrowserBinding {
            content: content.downgrade(),
            browser: browser.downgrade(),
            sidebar: sidebar.map(|sidebar| Rc::downgrade(&sidebar.state)),
        }));
        browser.bind_preview_scrolling(&self.state.revealer);
        let weak = Rc::downgrade(&self.state);
        let weak_browser = browser.downgrade();
        browser.connect_search_selection_changed(Rc::new(move || {
            let request_at_selection = weak.upgrade().and_then(|state| state.current_request.get());
            let weak = weak.clone();
            let weak_browser = weak_browser.clone();
            glib::idle_add_local_once(move || {
                let Some(state) = weak.upgrade().filter(|state| {
                    state.is_enabled() && state.current_request.get() == request_at_selection
                }) else {
                    return;
                };
                let Some(browser) = weak_browser.upgrade() else {
                    return;
                };
                if browser.selected_search_results().is_none() {
                    return;
                }
                if let Some(entry) = preview_target(browser.selected_search_result()) {
                    state.show_after_focus_change(entry, browser.browser().active_depth());
                } else {
                    state.clear_target();
                }
            });
        }));
        split.set_end_child(Some(&self.state.revealer));
        self.state.revealer.set_visible(self.state.is_enabled());
        let weak = Rc::downgrade(&self.state);
        split.add_tick_callback(move |split, _| {
            let Some(state) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if state.is_enabled() {
                state.sync_split(split);
            }
            glib::ControlFlow::Continue
        });
        let weak = Rc::downgrade(&self.state);
        split.connect_unmap(move |_| {
            if let Some(state) = weak.upgrade()
                && state.is_enabled()
            {
                state.suspend_panel();
            }
        });
        let weak = Rc::downgrade(&self.state);
        split.connect_unrealize(move |_| {
            if let Some(state) = weak.upgrade() {
                state.stop();
            }
        });
        if let Some(handle) = separator(split) {
            handle.set_cursor_from_name(Some("col-resize"));
        }
        install_resize(split, &self.state);
    }
}

impl PreviewState {
    pub(super) fn selected_entry(&self) -> (Option<FileEntry>, Option<usize>) {
        if let Some(binding) = self.sizing.binding.borrow().as_ref()
            && let Some(browser) = binding.browser.upgrade()
        {
            return (
                preview_target(
                    browser
                        .selected_search_result()
                        .or_else(|| browser.browser().focused_entry()),
                ),
                browser.browser().active_depth(),
            );
        }
        (
            preview_target(self.current.borrow().clone()),
            self.current_depth.get(),
        )
    }

    pub(super) fn reserves_empty_preview(&self) -> bool {
        self.is_enabled()
            && self
                .sizing
                .binding
                .borrow()
                .as_ref()
                .and_then(|binding| binding.browser.upgrade())
                .is_some_and(|browser| browser.view_mode() == BrowserMode::Icons)
    }

    fn geometry(&self, split: &gtk::Paned) -> Geometry {
        let available = split.width();
        let mut geometry = Geometry {
            available,
            occupied: available.saturating_sub(DEFAULT_WIDTH),
            start_minimum: 0,
            separator: separator_width(split),
            columns: false,
        };
        if let Some(binding) = self.sizing.binding.borrow().as_ref()
            && let Some(content) = binding.content.upgrade()
            && let Some(browser) = binding.browser.upgrade()
        {
            let sidebar = sidebar_width(&content);
            geometry.columns = browser.view_mode() == BrowserMode::Columns;
            geometry.occupied =
                sidebar + browser.preview_occupied_width((available - sidebar).max(0));
            if geometry.columns {
                geometry.start_minimum = sidebar.saturating_add(browser.preview_navigation_width(
                    (available - sidebar - geometry.separator - MIN_SPLIT_PREVIEW_WIDTH).max(0),
                ));
            } else {
                geometry.start_minimum = sidebar.saturating_add(COLUMN_WIDTH);
            }
        }
        geometry
    }

    pub(super) fn can_show_in(&self, split: &gtk::Paned) -> bool {
        split.is_mapped() && self.geometry(split).can_show_preview()
    }

    fn preserve_column_positions(&self, content_width: i32) {
        if let Some(binding) = self.sizing.binding.borrow().as_ref()
            && let Some(content) = binding.content.upgrade()
            && let Some(browser) = binding.browser.upgrade()
        {
            browser.preserve_columns_for_viewport((content_width - sidebar_width(&content)).max(0));
        }
    }

    pub(super) fn show_panel(&self) {
        if let Some(binding) = self.sizing.binding.borrow().as_ref()
            && let Some(browser) = binding.browser.upgrade()
        {
            browser.clear_preview_scroll_space();
        }
        self.revealer.set_transition_duration(0);
        self.pane.set_width_request(0);
        self.revealer.set_visible(true);
        self.revealer.set_reveal_child(true);
    }

    fn release_sidebar_rail(&self) {
        if !self.sizing.sidebar_railed.replace(false) {
            return;
        }
        if let Some(binding) = self.sizing.binding.borrow().as_ref()
            && let Some(content) = binding.content.upgrade()
        {
            let available = content
                .root()
                .map_or_else(|| content.width(), |r| r.width());
            let needs_full = preferred_sidebar_width() + COLUMN_WIDTH + 1;
            let keep_railed = available > 0 && available < needs_full;
            let sidebar = binding.sidebar.as_ref().and_then(Weak::upgrade);
            if let Some(sidebar) = sidebar.as_ref() {
                sidebar.set_rail(keep_railed);
            }
            if content
                .start_child()
                .is_some_and(|sidebar| sidebar.get_visible())
            {
                if keep_railed {
                    content.set_position(sidebar_rail_width());
                } else {
                    let restore = sidebar
                        .as_ref()
                        .and_then(|s| s.saved_width.get())
                        .unwrap_or_else(|| {
                            let saved = self.sizing.sidebar_saved_width.get();
                            if saved > 0 {
                                saved
                            } else {
                                preferred_sidebar_width()
                            }
                        })
                        .max(MIN_SIDEBAR_WIDTH);
                    content.set_position(restore);
                }
            }
        }
    }

    pub(super) fn hide_panel(&self) {
        self.release_sidebar_rail();
        let restore_browser_focus =
            self.pane
                .root()
                .and_then(|root| root.focus())
                .is_some_and(|focused| {
                    focused == self.pane
                    || focused.is_ancestor(&self.pane)
                    // GTK may focus a divider while allocating a smaller split.
                    || self.split.borrow().as_ref().is_some_and(|split| split.has_focus())
                    || self.sizing.binding.borrow().as_ref().is_some_and(|binding| {
                        binding.content.upgrade().is_some_and(|content| content.has_focus())
                    })
                });
        if let Some(split) = self.split.borrow().as_ref()
            && self.revealer.is_visible()
        {
            self.preserve_column_positions(split.width());
        }
        self.revealer.set_transition_duration(0);
        self.revealer.set_reveal_child(false);
        self.revealer.set_visible(false);
        if let Some(split) = self.split.borrow().as_ref() {
            split.set_resize_start_child(true);
            split.set_resize_end_child(false);
            split.set_position(split.width());
        }
        if restore_browser_focus
            && let Some(split) = self.split.borrow().as_ref()
            && let Some(browser) = self
                .sizing
                .binding
                .borrow()
                .as_ref()
                .and_then(|binding| binding.browser.upgrade())
        {
            split.add_tick_callback(move |_, _| {
                browser.focus_file_view();
                glib::ControlFlow::Break
            });
        }
    }

    fn suspend_panel(&self) {
        if self.sizing.suspended.replace(true) {
            return;
        }
        self.animation_generation
            .set(self.animation_generation.get().saturating_add(1));
        self.animating.set(false);
        self.sizing.resizing.set(false);
        let media = self.media.borrow().clone();
        self.sizing
            .resume_media
            .set(media.as_ref().is_some_and(|media| media.is_playing()));
        if let Some(media) = media {
            media.pause();
        }
        self.hide_panel();
    }

    pub(super) fn sync_split(self: &Rc<Self>, split: &gtk::Paned) {
        let mut geometry = self.geometry(split);
        let preview_present = self.current.borrow().is_some() || self.reserves_empty_preview();
        if preview_present
            && let Some(binding) = self.sizing.binding.borrow().as_ref()
            && let Some(content) = binding.content.upgrade()
        {
            let sidebar = binding.sidebar.as_ref().and_then(Weak::upgrade);
            let visible = content
                .start_child()
                .is_some_and(|sidebar| sidebar.get_visible());
            let is_railed = sidebar.as_ref().is_some_and(|s| s.rail.get());
            let saved_width = sidebar
                .as_ref()
                .and_then(|s| s.saved_width.get())
                .unwrap_or_else(|| {
                    let saved = self.sizing.sidebar_saved_width.get();
                    if saved > 0 {
                        saved
                    } else {
                        preferred_sidebar_width()
                    }
                })
                .max(MIN_SIDEBAR_WIDTH);
            let full = if sidebar.is_none() {
                0
            } else if is_railed || !visible {
                saved_width
            } else {
                content.position().max(MIN_SIDEBAR_WIDTH)
            };
            let content_sep = separator_width(&content);
            let occupied = if let Some(browser) = binding.browser.upgrade() {
                if geometry.columns {
                    browser.preview_navigation_width(
                        (geometry.available
                            - full
                            - content_sep
                            - geometry.separator
                            - MIN_SPLIT_PREVIEW_WIDTH)
                            .max(0),
                    )
                } else {
                    COLUMN_WIDTH
                }
            } else {
                COLUMN_WIDTH
            };
            let preview_needed = self
                .sizing
                .manual_width
                .get()
                .unwrap_or(MIN_SPLIT_PREVIEW_WIDTH);
            let needs = full + content_sep + occupied + geometry.separator + preview_needed;
            let content_has_room = content.width() <= 0 || content.width() >= full + COLUMN_WIDTH;
            if is_railed && geometry.available >= needs && content_has_room {
                if let Some(sidebar) = sidebar.as_ref() {
                    sidebar.set_rail(false);
                }
                if visible {
                    content.set_position(saved_width);
                }
                self.sizing.sidebar_railed.set(false);
                geometry = self.geometry(split);
            } else if !is_railed && sidebar.is_some() && geometry.available < needs {
                if visible {
                    let width = content.position().max(MIN_SIDEBAR_WIDTH);
                    self.sizing.sidebar_saved_width.set(width);
                    if let Some(sidebar) = sidebar.as_ref() {
                        sidebar.saved_width.set(Some(width));
                    }
                }
                if let Some(sidebar) = sidebar.as_ref() {
                    sidebar.set_rail(true);
                }
                if visible {
                    content.set_position(sidebar_rail_width());
                }
                self.sizing.sidebar_railed.set(true);
                geometry = self.geometry(split);
            }
        }
        if self.current.borrow().is_none() {
            if !self.reserves_empty_preview() || !geometry.can_show_preview() {
                if self.revealer.reveals_child() {
                    self.hide_panel();
                }
                return;
            }
            self.show_placeholder();
        }
        if !split.is_mapped() || !geometry.can_show_preview() {
            self.suspend_panel();
            return;
        }
        if self.animating.get() || self.sizing.resizing.get() {
            return;
        }
        split.set_resize_start_child(true);
        split.set_resize_end_child(false);
        let restored = self.sizing.suspended.replace(false);
        if restored || !self.revealer.reveals_child() {
            self.show_panel();
        }
        let manual = self.sizing.manual_width.get();
        let minimum = geometry.minimum_width(manual.is_some());
        let position = geometry.position(manual);
        if self.pane.width_request() != minimum || split.position() != position {
            self.pane.set_width_request(minimum);
            split.set_position(position);
        }
        if let Some(binding) = self.sizing.binding.borrow().as_ref()
            && let Some(browser) = binding.browser.upgrade()
        {
            browser.clear_preview_scroll_space();
        }
        if restored {
            if self.sizing.reload_on_resume.replace(false) {
                let entry = self.current.borrow().clone();
                if let Some(entry) = entry {
                    self.load(entry, 0);
                }
            } else if self.sizing.resume_media.replace(false) {
                let media = self.media.borrow().clone();
                if let Some(media) = media {
                    media.play();
                }
            }
        }
    }

    pub(super) fn opening_width(&self, available: i32) -> i32 {
        self.split
            .borrow()
            .as_ref()
            .map_or(DEFAULT_WIDTH.min(available), |split| {
                self.geometry(split)
                    .preview_width(self.sizing.manual_width.get())
            })
    }

    pub(super) fn animate_open(self: &Rc<Self>, split: &gtk::Paned) {
        self.sync_split(split);
        let geometry = self.geometry(split);
        if !geometry.can_show_preview() {
            return;
        }
        let target = geometry.position(self.sizing.manual_width.get());
        let start = split.width();
        // The animation owns the divider, including after a resize while the pane was hidden.
        split.set_resize_start_child(false);
        split.set_resize_end_child(true);
        self.preserve_column_positions(start);
        split.set_position(start);
        let animation_id = self.animation_generation.get().saturating_add(1);
        self.animation_generation.set(animation_id);
        self.animating.set(true);

        if !super::super::motion::animations_enabled() || start <= 0 {
            self.animating.set(false);
            self.sync_split(split);
            return;
        }

        let started = Instant::now();
        let weak = Rc::downgrade(self);
        split.add_tick_callback(move |split, _| {
            let Some(state) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if state.animation_generation.get() != animation_id {
                return glib::ControlFlow::Break;
            }
            let progress =
                (started.elapsed().as_secs_f64() / TRANSITION.as_secs_f64()).clamp(0.0, 1.0);
            let eased = super::super::motion::emphasized_deceleration(progress);
            let position = f64::from(start) + f64::from(target - start) * eased;
            let position = position.round() as i32;
            // Shrink the scroll range with the viewport, without clamping its retained offset.
            state.preserve_column_positions(position);
            split.set_position(position);
            if progress >= 1.0 {
                state.animating.set(false);
                state.sync_split(split);
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    pub(super) fn resize_preview(self: &Rc<Self>, split: &gtk::Paned, position: i32) {
        let geometry = self.geometry(split);
        if !geometry.can_show_preview() {
            return;
        }
        let width = (geometry.available - geometry.separator - position)
            .clamp(geometry.minimum_width(true), geometry.maximum_width());
        self.sizing.manual_width.set(Some(width));
        self.sync_split(split);
    }
}

fn install_resize(split: &gtk::Paned, state: &Rc<PreviewState>) {
    // Observe input without competing with GtkPaned's own drag gesture.
    let pointer = gtk::EventControllerLegacy::new();
    pointer.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(state);
    pointer.connect_event(move |controller, event| {
        let Some(state) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        let Some(split) = controller.widget().and_downcast::<gtk::Paned>() else {
            return glib::Propagation::Proceed;
        };
        match event.event_type() {
            gtk::gdk::EventType::ButtonPress | gtk::gdk::EventType::TouchBegin
                if (event.event_type() == gtk::gdk::EventType::TouchBegin
                    || event
                        .downcast_ref::<gtk::gdk::ButtonEvent>()
                        .is_some_and(|e| e.button() == 1))
                    && state.is_enabled()
                    && !state.sizing.is_suspended()
                    && on_separator(&split, event) =>
            {
                state
                    .animation_generation
                    .set(state.animation_generation.get().saturating_add(1));
                state.animating.set(false);
                state.sizing.resizing.set(true);
                state
                    .pane
                    .set_width_request(state.geometry(&split).minimum_width(true));
            }
            gtk::gdk::EventType::ButtonRelease
            | gtk::gdk::EventType::TouchEnd
            | gtk::gdk::EventType::TouchCancel
            | gtk::gdk::EventType::GrabBroken => {
                state.sizing.resizing.set(false);
            }
            _ => {}
        }
        glib::Propagation::Proceed
    });
    split.add_controller(pointer);
    let weak = Rc::downgrade(state);
    split.connect_position_notify(move |split| {
        if let Some(state) = weak.upgrade()
            && state.is_enabled()
            && state.sizing.resizing.replace(false)
        {
            state.resize_preview(split, split.position());
            state.sizing.resizing.set(true);
        }
    });

    // Unhandled browser keys also reach GtkPaned; only handle-focused actions are resizes.
    let weak = Rc::downgrade(state);
    split.connect_move_handle(move |split, _| {
        if split.has_focus() {
            if let Some(state) = weak.upgrade()
                && state.is_enabled()
                && !state.sizing.is_suspended()
            {
                state
                    .pane
                    .set_width_request(state.geometry(split).minimum_width(true));
            }
            remember_keyboard_width(weak.clone());
        }
        false
    });
    let weak = Rc::downgrade(state);
    split.connect_cancel_position(move |split| {
        if split.has_focus() {
            remember_keyboard_width(weak.clone());
        }
        false
    });
}

fn on_separator(split: &gtk::Paned, event: &gtk::gdk::Event) -> bool {
    let Some((x, y)) = event.position() else {
        return false;
    };
    let Some(native) = split.native() else {
        return false;
    };
    let (dx, dy) = native.surface_transform();
    let native: gtk::Widget = native.upcast();
    let Some(point) = native.compute_point(
        split,
        &gtk::graphene::Point::new((x + dx) as f32, (y + dy) as f32),
    ) else {
        return false;
    };
    split.pick(
        f64::from(point.x()),
        f64::from(point.y()),
        gtk::PickFlags::DEFAULT,
    ) == separator(split)
}

fn remember_keyboard_width(weak: std::rc::Weak<PreviewState>) {
    let Some(state) = weak
        .upgrade()
        .filter(|state| state.is_enabled() && !state.sizing.is_suspended())
    else {
        return;
    };
    let Some(split) = state.split.borrow().clone() else {
        return;
    };
    let before = split.position();
    state.sizing.resizing.set(true);
    // Wait for GTK's default action handler before resuming automatic layout.
    glib::idle_add_local_once(move || {
        if let Some(state) = weak.upgrade() {
            state.sizing.resizing.set(false);
            if state.is_enabled() && split.position() != before {
                state.resize_preview(&split, split.position());
            }
        }
    });
}
