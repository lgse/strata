// SPDX-License-Identifier: MIT

use std::rc::{Rc, Weak};

use gtk::prelude::*;

use super::super::PreviewState;
use super::geometry::{Geometry, separator_width, sidebar_width};
use super::{BrowserBinding, MIN_SPLIT_PREVIEW_WIDTH, RAIL_RELEASE_MARGIN};
use crate::ui::{
    browser::COLUMN_WIDTH,
    window::{
        MIN_SIDEBAR_WIDTH, SidebarFit, SidebarState, preferred_sidebar_width,
        sidebar_fits_without_preview, sidebar_rail_width,
    },
};

/// The preview's judgement of the sidebar beside its own needs.
pub(super) struct SidebarAssessment {
    /// `None` until the window has a width; a zero width is unknown, not short.
    pub(super) fit: Option<SidebarFit>,
    pub(super) saved_width: i32,
    /// A narrow window, not the user, pinned the divider where it is.
    pub(super) squeezed: bool,
    pub(super) visible: bool,
}

impl PreviewState {
    // Budget the intended sidebar width so a clamped divider can recover.
    pub(super) fn intended_sidebar_width(
        &self,
        binding: &BrowserBinding,
        content: &gtk::Paned,
    ) -> i32 {
        if binding
            .sidebar
            .as_ref()
            .and_then(Weak::upgrade)
            .is_some_and(|sidebar| sidebar.rail.get())
        {
            return sidebar_width(content);
        }
        self.full_sidebar_width(binding, content)
    }

    /// The shown sidebar at its saved width, whether or not it is railed.
    pub(super) fn full_sidebar_width(&self, binding: &BrowserBinding, content: &gtk::Paned) -> i32 {
        let current = sidebar_width(content);
        if current == 0 {
            return current;
        }
        current.max(self.saved_sidebar_width(binding) + separator_width(content))
    }

    fn saved_sidebar_width(&self, binding: &BrowserBinding) -> i32 {
        self.saved_width_of(binding.sidebar.as_ref().and_then(Weak::upgrade).as_ref())
    }

    fn saved_width_of(&self, sidebar: Option<&Rc<SidebarState>>) -> i32 {
        sidebar
            .and_then(|sidebar| sidebar.saved_width.get())
            .unwrap_or_else(|| {
                let saved = self.sizing.sidebar_saved_width.get();
                if saved > 0 {
                    saved
                } else {
                    preferred_sidebar_width()
                }
            })
            .max(MIN_SIDEBAR_WIDTH)
    }

    pub(in crate::ui::preview) fn sync_sidebar_without_preview(&self) {
        let binding = self.sizing.binding.borrow();
        let Some(binding) = binding.as_ref() else {
            return;
        };
        let (Some(content), Some(sidebar)) = (
            binding.content.upgrade(),
            binding.sidebar.as_ref().and_then(Weak::upgrade),
        ) else {
            return;
        };
        let Some(fits) = sidebar_fits_without_preview(&content) else {
            return;
        };
        if let Some(rail) = sidebar.observe_fit(SidebarFit {
            fits,
            preview: false,
        }) {
            sidebar.apply_rail(rail);
        }
    }

    pub(in crate::ui::preview) fn sidebar_fit(&self, content: &gtk::Paned) -> Option<SidebarFit> {
        if !self.preview_present() {
            return sidebar_fits_without_preview(content).map(|fits| SidebarFit {
                fits,
                preview: false,
            });
        }
        let split = self.split.borrow().clone()?;
        let binding = self.sizing.binding.borrow();
        let sidebar = binding
            .as_ref()
            .and_then(|binding| binding.sidebar.as_ref())
            .and_then(Weak::upgrade);
        self.assess_sidebar(&split, content, sidebar.as_ref()).fit
    }

    pub(super) fn assess_sidebar(
        &self,
        split: &gtk::Paned,
        content: &gtk::Paned,
        sidebar: Option<&Rc<SidebarState>>,
    ) -> SidebarAssessment {
        let geometry = self.geometry(split);
        let visible = content
            .start_child()
            .is_some_and(|sidebar| sidebar.get_visible());
        let is_railed = sidebar.is_some_and(|sidebar| sidebar.rail.get());
        let saved_width = self.saved_width_of(sidebar);
        let full = match sidebar {
            None => 0,
            Some(_) if is_railed || !visible => saved_width.max(preferred_sidebar_width()),
            // A manually narrowed sidebar must not prevent railing.
            Some(_) => content.position().max(preferred_sidebar_width()),
        };
        let content_sep = separator_width(content);
        let needs = full + content_sep + self.preview_need(geometry);
        // Hysteresis prevents toggling at the threshold.
        let margin = if is_railed { RAIL_RELEASE_MARGIN } else { 0 };
        let wants_rail = outer_width(split, geometry).map(|available| available < needs + margin);
        let squeezed = visible
            && !is_railed
            && content.position() + content_sep + COLUMN_WIDTH >= content.width();
        SidebarAssessment {
            fit: wants_rail.map(|wants_rail| SidebarFit {
                fits: !wants_rail,
                preview: true,
            }),
            saved_width,
            squeezed,
            visible,
        }
    }

    /// The browser's focused navigation, the divider, and the preview itself.
    fn preview_need(&self, geometry: Geometry) -> i32 {
        let occupied = match self.sizing.browser() {
            Some(browser) if geometry.columns => browser.preview_standard_navigation_width(),
            _ => COLUMN_WIDTH,
        };
        // A previously clamped manual width must not defeat the preview minimum.
        let preview_needed = self
            .sizing
            .manual_width
            .get()
            .unwrap_or(MIN_SPLIT_PREVIEW_WIDTH)
            .max(MIN_SPLIT_PREVIEW_WIDTH);
        occupied + geometry.separator + preview_needed
    }

    /// Restores a squeezed sidebar and rails or releases it for the preview,
    /// returning the geometry left afterwards.
    pub(super) fn sync_sidebar_rail(&self, split: &gtk::Paned, geometry: Geometry) -> Geometry {
        let binding = self.sizing.binding.borrow();
        let Some(binding) = binding.as_ref() else {
            return geometry;
        };
        let (Some(content), Some(sidebar)) = (
            binding.content.upgrade(),
            binding.sidebar.as_ref().and_then(Weak::upgrade),
        ) else {
            return geometry;
        };
        let is_railed = sidebar.rail.get();
        let resizing = self.sizing.resizing.get()
            || binding
                .browser
                .upgrade()
                .is_some_and(|browser| browser.is_resizing_columns());
        let assessment = self.assess_sidebar(split, &content, Some(&sidebar));
        let mut geometry = geometry;
        let expanded_and_idle = !is_railed && assessment.visible && !resizing;
        let fits = assessment.fit.is_some_and(|fit| fit.fits);
        if expanded_and_idle && fits && has_room_for(&content, assessment.saved_width) {
            content.set_position(assessment.saved_width);
            geometry = self.geometry(split);
        }
        if resizing {
            return geometry;
        }
        let Some(rail) = assessment.fit.and_then(|fit| sidebar.observe_fit(fit)) else {
            return geometry;
        };
        if rail {
            self.rail_sidebar(&content, &sidebar, &assessment);
        } else {
            release_sidebar(&content, &sidebar, &assessment);
        }
        self.geometry(split)
    }

    fn rail_sidebar(
        &self,
        content: &gtk::Paned,
        sidebar: &SidebarState,
        assessment: &SidebarAssessment,
    ) {
        if assessment.visible {
            let width = if assessment.squeezed {
                assessment.saved_width
            } else {
                content.position().max(MIN_SIDEBAR_WIDTH)
            };
            self.sizing.sidebar_saved_width.set(width);
            sidebar.saved_width.set(Some(width));
        }
        sidebar.set_rail(true);
        if assessment.visible {
            content.set_position(sidebar_rail_width());
        }
    }
}

fn release_sidebar(content: &gtk::Paned, sidebar: &SidebarState, assessment: &SidebarAssessment) {
    sidebar.set_rail(false);
    if assessment.visible {
        content.set_position(assessment.saved_width);
    }
}

/// A sidebar narrower than its saved width that could grow back beside a column.
fn has_room_for(content: &gtk::Paned, saved_width: i32) -> bool {
    content.position() < saved_width
        && content.width() >= saved_width + separator_width(content) + COLUMN_WIDTH
}

/// Measure outside the split so railing cannot change its own threshold.
/// The content paned is not consulted: the slot re-budgets a sidebar a
/// frame after it shows, and a one-frame shortage would latch the rail.
fn outer_width(split: &gtk::Paned, geometry: Geometry) -> Option<i32> {
    split
        .parent()
        .map(|parent| parent.width())
        .filter(|width| *width > 0)
        .or_else(|| (geometry.available > 0).then_some(geometry.available))
}
