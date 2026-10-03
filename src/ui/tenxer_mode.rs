// SPDX-License-Identifier: MIT

use std::cell::Cell;

use gtk::prelude::*;

use super::preferences::PreferenceManager;

pub(crate) const MODE_DESCRIPTION: &str = "Opinionated keyboard-centric mode with Yazi-style navigation. Disables some features. Toggle with Ctrl-Shift-M.";
pub(crate) const UNUSED_SUBTITLE: &str = "Not used in 10xer mode.";
pub(crate) const TAG_TEXT: &str = "10X";
pub(crate) const TAG_NAME: &str = "10xer mode";

pub(crate) fn chrome_suppressed() -> bool {
    PreferenceManager::shared().tenxer_mode()
}

pub(crate) fn hide_while_enabled(widget: &impl IsA<gtk::Widget>) {
    widget.add_css_class("tenxer-suppressed-chrome");
    PreferenceManager::shared().bind_preference(
        widget,
        PreferenceManager::tenxer_mode,
        |widget, enabled| {
            widget.set_visible(!enabled);
            widget.set_sensitive(!enabled);
        },
    );
}

/// The sort map handler restores device-order sensitivity when shown again.
pub(crate) fn hide_sort_direction_while_enabled(widget: &impl IsA<gtk::Widget>) {
    widget.add_css_class("tenxer-suppressed-chrome");
    widget.add_css_class("tenxer-sort-direction");
    PreferenceManager::shared().bind_preference(
        widget,
        PreferenceManager::tenxer_mode,
        |widget, enabled| {
            if enabled {
                widget.set_sensitive(false);
                widget.set_visible(false);
            } else {
                widget.set_sensitive(true);
                widget.set_visible(true);
            }
        },
    );
}

pub(crate) fn hide_filter_while_enabled(button: &gtk::ToggleButton, revealer: &gtk::Revealer) {
    button.add_css_class("tenxer-suppressed-chrome");
    revealer.add_css_class("tenxer-filter-revealer");
    let revealer = revealer.downgrade();
    let primed = Cell::new(false);
    PreferenceManager::shared().bind_preference(
        button,
        PreferenceManager::tenxer_mode,
        move |widget, enabled| {
            widget.set_visible(!enabled);
            widget.set_sensitive(!enabled);
            if !primed.replace(true) {
                return;
            }
            let Some(revealer) = revealer.upgrade() else {
                return;
            };
            if enabled {
                revealer.set_reveal_child(false);
            } else {
                let button = widget
                    .downcast_ref::<gtk::ToggleButton>()
                    .expect("filter toggle binding");
                revealer.set_reveal_child(button.is_active());
            }
        },
    );
}

pub(crate) fn is_toggle_shortcut(key: gtk::gdk::Key, modifiers: gtk::gdk::ModifierType) -> bool {
    modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK)
        && modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK)
        && !modifiers
            .intersects(gtk::gdk::ModifierType::ALT_MASK | gtk::gdk::ModifierType::SUPER_MASK)
        && matches!(key, gtk::gdk::Key::m | gtk::gdk::Key::M)
}

/// A pending two-key command. While armed, the next key belongs to it alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Chord {
    /// **g** from the listing: first item or a place.
    Go,
    PreviewTop,
    Copy,
    Sort,
    Action,
}

impl Chord {
    pub(crate) fn mark(self) -> &'static str {
        match self {
            Self::Go | Self::PreviewTop => "g-",
            Self::Copy => "c-",
            Self::Sort => ",-",
            Self::Action => ";-",
        }
    }

    pub(crate) fn options(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Go => &[
                ("g", "First item"),
                ("f", "Follow search result"),
                ("h", "Home"),
                ("d", "Downloads"),
                ("c", "Config"),
                ("t", "Trash"),
                ("n", "Network"),
                ("r", "Recent"),
                ("k", "Documents"),
                ("m", "Music"),
                ("p", "Pictures"),
                ("v", "Videos"),
                ("1–9", "Pins"),
                ("+ / -", "Pin / unpin folder"),
                ("Space", "Type a path"),
            ],
            Self::PreviewTop => &[
                ("g", "Top"),
                ("f", "Follow search result"),
                ("h", "Home"),
                ("d", "Downloads"),
                ("c", "Config"),
                ("t", "Trash"),
                ("n", "Network"),
                ("r", "Recent"),
                ("k", "Documents"),
                ("p", "Pictures"),
                ("v", "Videos"),
                ("1–9", "Pins"),
                ("+ / -", "Pin / unpin folder"),
                ("Space", "Type a path"),
            ],
            Self::Copy => &[("c", "Copy path"), ("n", "Copy name")],
            Self::Sort => &[
                ("a", "Name"),
                ("m", "Modified"),
                ("s", "Size"),
                ("e", "Type"),
                ("Shift", "Reverse"),
            ],
            Self::Action => &[],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Prompt {
    Find,
    FindBackward,
    Filter,
    Search,
    Go,
    Jump,
    Recent,
    Create,
    Rename,
    MoveTo,
    CopyTo,
    ExtractTo,
}

impl Prompt {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Find => "/",
            Self::FindBackward => "?",
            Self::Filter => "filter:",
            Self::Search => "search:",
            Self::Go => "go \u{203a}",
            Self::Jump => "jump \u{203a}",
            Self::Recent => "recent \u{203a}",
            Self::Create => "create \u{203a}",
            Self::Rename => "rename \u{203a}",
            Self::MoveTo => "move to \u{203a}",
            Self::CopyTo => "copy to \u{203a}",
            Self::ExtractTo => "extract to \u{203a}",
        }
    }

    pub(crate) fn picks_history(self) -> bool {
        matches!(self, Self::Jump | Self::Recent)
    }

    /// Prompts that list folders found by fuzzy path search.
    pub(crate) fn picks_folder(self) -> bool {
        matches!(
            self,
            Self::Go | Self::MoveTo | Self::CopyTo | Self::ExtractTo
        )
    }

    pub(crate) fn holds_targets(self) -> bool {
        matches!(
            self,
            Self::Rename | Self::MoveTo | Self::CopyTo | Self::ExtractTo
        )
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Find => "Find in this listing",
            Self::FindBackward => "Find backward in this listing",
            Self::Filter => "Filter this listing",
            Self::Search => "Search this folder and its subfolders",
            Self::Go => "Go to a path or URI",
            Self::Jump => "Jump to a visited folder",
            Self::Recent => "Jump to a recently visited folder",
            Self::Create => "Create a file, or a folder ending in /",
            Self::Rename => "Rename the focused item",
            Self::MoveTo => "Move the selection to a folder",
            Self::CopyTo => "Copy the selection to a folder",
            Self::ExtractTo => "Extract the archive to a folder",
        }
    }
}
