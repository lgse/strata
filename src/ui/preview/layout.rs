// SPDX-License-Identifier: MIT

use std::rc::Weak;

use super::*;
use crate::ui::{
    browser::{BrowserView, COLUMN_WIDTH, WeakBrowserView},
    browser_modes::BrowserMode,
    window::{SidebarState, SidebarView},
};

mod geometry;
mod grip;
mod resize;
mod reveal;
mod sidebar;
mod sync;

use geometry::Geometry;
use geometry::sidebar_width;
pub(in crate::ui) use geometry::{separator, separator_width};

const MIN_SPLIT_PREVIEW_WIDTH: i32 = 240;
const RAIL_RELEASE_MARGIN: i32 = 24;
/// The divider only takes its own pixel so the column beside it keeps its resize
/// edge; this strip inside the preview is where the divider is grabbed instead.
pub(super) const RESIZE_GRIP_WIDTH: i32 = 6;

#[derive(Default)]
pub(super) struct SplitSizing {
    binding: RefCell<Option<BrowserBinding>>,
    manual_width: Cell<Option<i32>>,
    resizing: Cell<bool>,
    /// A pointer, not the keyboard, is dragging the divider.
    dragging: Cell<bool>,
    /// The divider position a resize restored, which GTK reports back afterwards.
    restored_position: Cell<Option<i32>>,
    minimum_outline: RefCell<Option<gtk::Box>>,
    grip_hint: Rc<crate::ui::resize_feedback::EdgeHint>,
    suspended: Cell<bool>,
    resume_media: Cell<bool>,
    reload_on_resume: Cell<bool>,
    sidebar_saved_width: Cell<i32>,
}

impl SplitSizing {
    #[cfg(test)]
    pub(super) fn manual_width_for_test(&self) -> Option<i32> {
        self.manual_width.get()
    }

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
        self.state.refresh_panel_action();
        browser.bind_preview_scrolling(&self.state.revealer);
        follow_browser(browser, &self.state);
        split.set_end_child(Some(&self.state.slot));
        self.state.revealer.set_visible(self.state.is_enabled());
        self.state
            .slot
            .set_visible(self.state.is_enabled() || self.state.reserves_column_space());
        track_split(split, &self.state);
        if let Some(handle) = separator(split) {
            handle.set_cursor_from_name(Some("col-resize"));
        }
        resize::install_resize(split, &self.state);
        grip::install_resize_grip(split, &self.state);
    }
}

fn follow_browser(browser: &BrowserView, state: &Rc<PreviewState>) {
    let weak = Rc::downgrade(state);
    browser.connect_view_mode_changed(move |_| {
        if let Some(state) = weak.upgrade() {
            state.view_mode_changed();
        }
    });
    let weak = Rc::downgrade(state);
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
            if let Some(browser) = weak_browser.upgrade() {
                state.show_search_selection(&browser);
            }
        });
    }));
}

/// Lays the split out every frame it is active and parks the panel when it goes away.
fn track_split(split: &gtk::Paned, state: &Rc<PreviewState>) {
    let weak = Rc::downgrade(state);
    crate::ui::frame::tick_while_active(split, 60, move |split| {
        if let Some(state) = weak.upgrade() {
            state.split_frame(split);
        }
    });
    let weak = Rc::downgrade(state);
    split.connect_unmap(move |_| {
        if let Some(state) = weak.upgrade()
            && state.is_enabled()
        {
            state.suspend_panel();
        }
    });
    let weak = Rc::downgrade(state);
    split.connect_unrealize(move |_| {
        if let Some(state) = weak.upgrade() {
            state.stop();
        }
    });
}

impl PreviewState {
    fn view_mode_changed(&self) {
        self.refresh_panel_action();
        if !self.is_enabled() && !self.reserves_column_space() {
            self.hide_panel();
        }
    }

    fn show_search_selection(self: &Rc<Self>, browser: &BrowserView) {
        if browser.selected_search_results().is_none() {
            return;
        }
        let entry = if browser.results_replace_listing() {
            browser
                .browser()
                .active_depth()
                .and_then(|depth| browser.displayed_cursor_entry(depth))
        } else {
            browser.selected_search_result()
        };
        if let Some(entry) = preview_target(entry) {
            self.show_after_focus_change(entry, browser.browser().active_depth());
        } else {
            self.clear_target();
        }
    }

    fn split_frame(self: &Rc<Self>, split: &gtk::Paned) {
        self.publish_column_width_limit(split);
        if self.is_enabled() || self.reserves_column_space() {
            self.sync_split(split);
        }
        if !self.preview_present() {
            self.sync_sidebar_without_preview();
        }
    }

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

    pub(super) fn slot_is_empty(&self) -> bool {
        self.current.borrow().is_none() && !self.reserves_empty_preview()
    }

    pub(super) fn reserves_empty_preview(&self) -> bool {
        self.is_enabled()
            && !self.child_pane.get()
            && self
                .sizing
                .binding
                .borrow()
                .as_ref()
                .and_then(|binding| binding.browser.upgrade())
                .is_some_and(|browser| {
                    matches!(
                        browser.view_mode(),
                        BrowserMode::Columns | BrowserMode::Icons | BrowserMode::Compact
                    )
                })
    }

    pub(super) fn browsing_columns(&self) -> bool {
        self.sizing
            .browser()
            .is_some_and(|browser| browser.view_mode() == BrowserMode::Columns)
    }

    pub(super) fn reserves_column_space(&self) -> bool {
        self.reserve_columns.get() && self.browsing_columns()
    }

    // Columns may use the whole browser area; the slot yields down to zero
    // (docs/preview-panel-layout.md, "focused column, then preview, then peek").
    // A railed sidebar counts at full width so columns still fit once it expands.
    fn publish_column_width_limit(&self, split: &gtk::Paned) {
        let available = split.width();
        if available <= 0 {
            return;
        }
        let binding = self.sizing.binding.borrow();
        let Some(binding) = binding.as_ref() else {
            return;
        };
        let (Some(content), Some(browser)) = (binding.content.upgrade(), binding.browser.upgrade())
        else {
            return;
        };
        let sidebar = self.full_sidebar_width(binding, &content);
        browser.set_column_width_limit(available - separator_width(split) - sidebar);
    }

    fn geometry(&self, split: &gtk::Paned) -> Geometry {
        let available = split.width();
        let geometry = Geometry {
            available,
            occupied: available.saturating_sub(DEFAULT_WIDTH),
            trailing: 0,
            start_minimum: 0,
            show_minimum: 0,
            separator: separator_width(split),
            columns: false,
        };
        let binding = self.sizing.binding.borrow();
        let Some((binding, content, browser)) = bound_views(&binding) else {
            return geometry;
        };
        let sidebar = self.intended_sidebar_width(binding, &content);
        browser_geometry(geometry, sidebar, &browser)
    }

    pub(super) fn can_show_in(&self, split: &gtk::Paned) -> bool {
        split.is_mapped() && self.geometry(split).can_show_preview()
    }

    fn preserve_column_positions(&self, content_width: i32) {
        let binding = self.sizing.binding.borrow();
        if let Some((_, content, browser)) = bound_views(&binding) {
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
        self.slot.set_visible(true);
        self.revealer.set_visible(true);
        self.revealer.set_reveal_child(true);
    }

    /// A preview, a reserved placeholder, or Columns' reserved slot takes part
    /// in the sidebar fit rule; otherwise the window-only rule applies.
    pub(super) fn preview_present(&self) -> bool {
        self.current.borrow().is_some()
            || self.reserves_empty_preview()
            || self.reserves_column_space()
    }

    pub(super) fn hide_panel(&self) {
        self.finish_hide(self.hide_intent());
    }

    /// Whether the browser view needs focus restored once the panel is gone.
    pub(super) fn hide_intent(&self) -> bool {
        self.pane
            .root()
            .and_then(|root| root.focus())
            .is_some_and(|focused| {
                let in_pane = focused == self.pane || focused.is_ancestor(&self.pane);
                in_pane || self.divider_has_focus()
            })
    }

    /// GTK may focus a divider while allocating a smaller split.
    fn divider_has_focus(&self) -> bool {
        self.split
            .borrow()
            .as_ref()
            .is_some_and(|split| split.has_focus())
            || self
                .sizing
                .binding
                .borrow()
                .as_ref()
                .is_some_and(|binding| {
                    binding
                        .content
                        .upgrade()
                        .is_some_and(|content| content.has_focus())
                })
    }

    /// Pins the pane to its resting width while the divider sweeps, so the slot
    /// clips a fully laid out panel instead of reflowing it every frame.
    fn pin_pane_width(&self, split: &gtk::Paned) {
        self.pane.set_width_request(
            self.geometry(split)
                .preview_width(self.sizing.manual_width.get()),
        );
    }

    fn finish_hide(&self, restore_browser_focus: bool) {
        let keep_slot = self.reserves_column_space();
        let split = self.split.borrow().clone();
        let collapsing = split.as_ref().filter(|_| !keep_slot);
        if self.revealer.is_visible()
            && let Some(split) = collapsing
        {
            self.preserve_column_positions(split.width());
        }
        self.revealer.set_transition_duration(0);
        self.revealer.set_reveal_child(false);
        self.revealer.set_visible(false);
        if let Some(split) = collapsing {
            self.slot.set_visible(false);
            split.set_resize_start_child(true);
            split.set_resize_end_child(false);
            split.set_position(split.width());
        }
        if restore_browser_focus && let Some(split) = &split {
            self.refocus_browser(split);
        }
    }

    fn refocus_browser(&self, split: &gtk::Paned) {
        if let Some(browser) = self.sizing.browser() {
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

    pub(super) fn opening_width(&self, available: i32) -> i32 {
        self.split
            .borrow()
            .as_ref()
            .map_or(DEFAULT_WIDTH.min(available), |split| {
                self.geometry(split)
                    .preview_width(self.sizing.manual_width.get())
            })
    }
}

fn bound_views(
    binding: &Option<BrowserBinding>,
) -> Option<(&BrowserBinding, gtk::Paned, BrowserView)> {
    let binding = binding.as_ref()?;
    let content = binding.content.upgrade()?;
    let browser = binding.browser.upgrade()?;
    Some((binding, content, browser))
}

fn browser_geometry(mut geometry: Geometry, sidebar: i32, browser: &BrowserView) -> Geometry {
    let available = geometry.available;
    geometry.columns = browser.view_mode() == BrowserMode::Columns;
    geometry.occupied = sidebar + browser.preview_navigated_width((available - sidebar).max(0));
    if geometry.columns {
        geometry.trailing = browser.preview_trailing_width();
        geometry.start_minimum = sidebar.saturating_add(browser.preview_navigation_width());
        geometry.show_minimum = sidebar.saturating_add(browser.preview_standard_navigation_width());
    } else {
        geometry.start_minimum = sidebar.saturating_add(COLUMN_WIDTH);
        geometry.show_minimum = geometry.start_minimum;
    }
    geometry
}
