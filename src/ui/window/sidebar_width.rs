// SPDX-License-Identifier: MIT

//! Sidebar widths, the rail fit policy and the show/hide animation.

use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};

use gtk::{glib, prelude::*};

use crate::ui::{
    motion::{animations_enabled, emphasized_deceleration},
    preferences::PreferenceManager,
};

use super::{SidebarState, SidebarView};

pub(in crate::ui) const SIDEBAR_WIDTH: i32 = 201;

pub(in crate::ui) const MIN_SIDEBAR_WIDTH: i32 = 169;

pub(in crate::ui) fn sidebar_split() -> gtk::Paned {
    let split = gtk::Paned::new(gtk::Orientation::Horizontal);
    split.add_css_class("sidebar-split");
    // Native narrow handles extend their input region into the adjacent file area.
    split.set_wide_handle(true);
    split.set_shrink_start_child(false);
    split.set_resize_start_child(false);
    split.set_position(SIDEBAR_WIDTH);
    split.set_vexpand(true);
    split
}

pub(in crate::ui) fn sidebar_rail_button_size() -> i32 {
    // Match the compact header toggle; the rail supplies its own side gutters.
    (24.0 * PreferenceManager::shared().interface_scale()).round() as i32
}

pub(in crate::ui) fn sidebar_rail_width() -> i32 {
    sidebar_rail_button_size() + 10
}

/// Whether the full sidebar fits beside the content, and whether a preview
/// took part in that judgement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::ui) struct SidebarFit {
    pub(in crate::ui) fits: bool,
    pub(in crate::ui) preview: bool,
}

/// Decides when the sidebar shows as an icon rail. The user's explicit choice
/// wins; a change in available space compacts an expanded sidebar once, and
/// space releases only what it compacted.
#[derive(Default)]
pub(in crate::ui) struct RailPolicy {
    compact: Cell<bool>,
    automatic: Cell<bool>,
    fit: Cell<Option<SidebarFit>>,
}

impl RailPolicy {
    pub(in crate::ui) fn rail(&self) -> bool {
        self.compact.get() || self.automatic.get()
    }

    /// Returns the new rail state when the choice changes what is shown.
    pub(in crate::ui) fn set_compact(&self, compact: bool) -> Option<bool> {
        let before = self.rail();
        self.compact.set(compact);
        self.automatic.set(false);
        (self.rail() != before).then(|| self.rail())
    }

    /// Returns the new rail state when a changed fit changes what is shown.
    pub(in crate::ui) fn observe(&self, fit: SidebarFit) -> Option<bool> {
        if self.fit.replace(Some(fit)) == Some(fit) {
            return None;
        }
        let before = self.rail();
        if fit.fits {
            self.automatic.set(false);
        } else if !self.compact.get() {
            self.automatic.set(true);
        }
        (self.rail() != before).then(|| self.rail())
    }
}

/// The fit rule while no preview takes part: the full sidebar beside one column.
/// `None` until the window has a width.
pub(in crate::ui) fn sidebar_fits_without_preview(content: &gtk::Paned) -> Option<bool> {
    let available = content
        .root()
        .map_or_else(|| content.width(), |root| root.width());
    (available > 0)
        .then(|| available > preferred_sidebar_width() + crate::ui::browser::COLUMN_WIDTH)
}

const SIDEBAR_TRANSITION: Duration = Duration::from_millis(300);

pub(in crate::ui) fn bind_sidebar_text_size(paned: &gtk::Paned, sidebar: &SidebarView) {
    let sidebar = Rc::downgrade(&sidebar.state);
    PreferenceManager::shared().bind_interface_scale(paned, move |widget, scale| {
        let paned = widget.downcast_ref::<gtk::Paned>().expect("sidebar split");
        if paned.position() > 0 {
            let Some(shell) = paned.start_child() else {
                return;
            };
            let target = if shell.has_css_class("sidebar-rail") {
                if let Some(sidebar) = sidebar.upgrade() {
                    sidebar.set_rail(true);
                }
                sidebar_rail_width()
            } else {
                scaled_sidebar_width(paned, scale).max(sidebar_minimum_width(&shell))
            };
            paned.set_position(target);
        }
    });
}

pub(in crate::ui) fn preferred_sidebar_width() -> i32 {
    (f64::from(SIDEBAR_WIDTH) * PreferenceManager::shared().interface_scale())
        .round()
        .max(f64::from(SIDEBAR_WIDTH)) as i32
}

pub(super) fn scaled_sidebar_width(paned: &gtk::Paned, scale: f64) -> i32 {
    let preferred = (f64::from(SIDEBAR_WIDTH) * scale).round() as i32;
    let available = if paned.width() > 0 {
        paned.width() / 2
    } else {
        preferred
    };
    preferred.min(available).max(MIN_SIDEBAR_WIDTH)
}

pub(super) fn sidebar_animation_target(
    paned: &gtk::Paned,
    sidebar: &gtk::Widget,
    state: &SidebarState,
    expanded: bool,
) -> i32 {
    if !expanded {
        0
    } else if state.rail.get() {
        sidebar_rail_width()
    } else if let Some(saved) = state.saved_width.get() {
        saved.max(sidebar_minimum_width(sidebar))
    } else {
        scaled_sidebar_width(paned, PreferenceManager::shared().interface_scale())
            .max(sidebar_minimum_width(sidebar))
    }
}

// Below this minimum, GTK can allocate more width than the divider position allows.
pub(super) fn sidebar_minimum_width(sidebar: &gtk::Widget) -> i32 {
    let (minimum, _, _, _) = sidebar.measure(gtk::Orientation::Horizontal, -1);
    minimum
}

pub(super) fn animate_sidebar(
    paned: &gtk::Paned,
    sidebar: &gtk::Widget,
    state: &Rc<SidebarState>,
    generation: &Rc<Cell<u64>>,
    animating: &Rc<Cell<bool>>,
    expanded: bool,
) {
    let animation_id = generation.get().saturating_add(1);
    generation.set(animation_id);
    animating.set(true);
    paned.set_shrink_start_child(true);
    if expanded {
        sidebar.set_visible(true);
    }
    let target = sidebar_animation_target(paned, sidebar, state, expanded);
    let start = paned.position();

    if !animations_enabled() || start == target {
        paned.set_position(target);
        sidebar.set_visible(expanded);
        paned.set_shrink_start_child(!expanded);
        animating.set(false);
        return;
    }

    let started = Instant::now();
    let sidebar = sidebar.clone();
    let generation = generation.clone();
    let animating = animating.clone();
    let state = Rc::downgrade(state);
    let _tick = paned.add_tick_callback(move |paned, _| {
        if generation.get() != animation_id {
            return glib::ControlFlow::Break;
        }
        let Some(state) = state.upgrade() else {
            return glib::ControlFlow::Break;
        };

        let target = sidebar_animation_target(paned, &sidebar, &state, expanded);
        let progress =
            (started.elapsed().as_secs_f64() / SIDEBAR_TRANSITION.as_secs_f64()).clamp(0.0, 1.0);
        let eased = emphasized_deceleration(progress);
        let position = f64::from(start) + f64::from(target - start) * eased;
        paned.set_position(position.round() as i32);

        if progress >= 1.0 {
            // Clear before the final set_position so the position clamp sees
            // the settled state, including an engaged icon rail.
            animating.set(false);
            paned.set_position(target);
            if !expanded {
                sidebar.set_visible(false);
            }
            paned.set_shrink_start_child(!expanded);
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}
