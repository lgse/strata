// SPDX-License-Identifier: MIT

//! What an Icons card shows for its entry: name, thumbnail, details, and marks.

use gtk::prelude::*;

use super::{
    metadata::set_icons_entry_details,
    pane::{BoundModeItem, set_mode_mark_style},
    source_index::SourceIndexMap,
};
use crate::{
    app::Browser,
    model::FileEntry,
    ui::{
        accessibility,
        browser::{self, ClipboardMarks, ViewState, mark_in},
        icons_cell, thumbnail,
    },
};

const HIDDEN_OPACITY: f64 = 0.65;
const FILE_ICON_OPACITY: f64 = 0.72;

/// An Icons card and the icon and name it shows.
pub(super) struct IconsCard {
    card: gtk::Box,
    icon: thumbnail::ThumbnailSlot,
    label: gtk::Inscription,
}

/// How a card is drawn while it binds.
pub(super) struct CardLook<'a> {
    pub(super) marks: &'a ClipboardMarks,
    pub(super) thumbnail_size: i32,
    /// Chrome waits for scrolling to settle.
    pub(super) scrolling: bool,
    pub(super) state: Option<&'a ViewState>,
}

impl IconsCard {
    pub(super) fn of(card: &gtk::Box) -> Option<Self> {
        let (icon, label) = icons_cell::parts(card)?;
        Some(Self {
            card: card.clone(),
            icon,
            label,
        })
    }

    pub(super) fn show_entry(&self, item: &gtk::ListItem, entry: &FileEntry, look: &CardLook) {
        set_icons_entry_details(&self.card, entry);
        let pending_name = look
            .state
            .and_then(|state| state.pending_rename_name(entry));
        let shown_name = pending_name.as_deref().unwrap_or(&entry.display_name);
        if self.label.text().as_deref() != Some(shown_name) {
            self.label.set_text(Some(shown_name));
        }
        thumbnail::set_thumbnail_or_icon(
            &self.icon,
            entry,
            browser::entry_icon(entry),
            look.thumbnail_size,
            look.thumbnail_size,
        );
        if look.scrolling {
            self.dim(entry);
        } else {
            self.refresh_chrome(item, entry, look.marks);
        }
        if pending_name.is_some() {
            accessibility::set_description(&self.label, Some(shown_name));
            accessibility::describe_entry(item, shown_name, Some(entry));
        }
    }

    pub(super) fn fit_name(&self) {
        icons_cell::fit_compact_name(
            &self.label,
            self.label.text().as_deref().unwrap_or_default(),
        );
    }

    pub(super) fn highlight(&self, state: Option<&ViewState>, filter_query: &str) {
        browser::find::highlight_listing_name(
            self.label.upcast_ref(),
            state.and_then(|state| state.find_highlight()).as_deref(),
            filter_query,
        );
    }

    pub(super) fn refresh_chrome(
        &self,
        item: &gtk::ListItem,
        entry: &FileEntry,
        marks: &ClipboardMarks,
    ) {
        accessibility::set_description(&self.label, Some(&entry.display_name));
        accessibility::describe_entry(item, &entry.display_name, Some(entry));
        set_mode_mark_style(&self.card, mark_in(marks, &entry.location));
        self.dim(entry);
    }

    /// Hidden entries and files draw fainter than folders.
    fn dim(&self, entry: &FileEntry) {
        let opacity = if entry.is_hidden { HIDDEN_OPACITY } else { 1.0 };
        self.icon.set_hidden(entry.is_hidden);
        self.icon.set_base_opacity(if entry.is_directory() {
            1.0
        } else {
            FILE_ICON_OPACITY
        });
        self.label.set_opacity(opacity);
        if let Some(details) = icons_cell::details_label(&self.card) {
            details.set_opacity(opacity);
        }
    }
}

/// Redraws the chrome of the cards scrolling skipped.
pub(super) struct CardRefresh<'a> {
    pub(super) browser: &'a Browser,
    pub(super) depth: usize,
    pub(super) source_index: &'a SourceIndexMap,
    pub(super) marks: &'a ClipboardMarks,
}

impl CardRefresh<'_> {
    pub(super) fn refresh(&self, bound: &BoundModeItem) {
        let Some(card) = bound.widget.upgrade().and_downcast::<gtk::Box>() else {
            return;
        };
        let Some(parts) = IconsCard::of(&card) else {
            return;
        };
        let Some(item) = bound.item.upgrade() else {
            return;
        };
        let Some(position) = item
            .item()
            .and_then(|value| self.source_index.of_item(&value))
        else {
            return;
        };
        let Some(entry) = self.browser.entry_at(self.depth, position) else {
            return;
        };
        if thumbnail::near_viewport(&card) {
            parts.refresh_chrome(&item, &entry, self.marks);
        }
    }
}
