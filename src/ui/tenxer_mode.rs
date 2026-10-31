// SPDX-License-Identifier: MIT

use std::cell::Cell;

use gtk::prelude::*;

use super::preferences::PreferenceManager;

pub(crate) const MODE_DESCRIPTION: &str = "Opinionated keyboard-centric mode with Yazi-style navigation. Disables some features. Toggle with %{tenxer_mode_keys}.";
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

/// A pending two-key command. While armed, the next key belongs to it alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Chord {
    /// **g** from the listing: first item or a place.
    Go,
    PreviewTop,
    Copy,
    Sort,
    Action,
    Tabs,
}

impl Chord {
    /// The pending chord's first key, as its footer pill shows it.
    pub(crate) fn mark(self) -> String {
        use super::keys::{
            Keys,
            bindings::{ACTIONS, COPY_AS, GO, SORT_BY, TAB_CHORD},
        };
        let prefix = match self {
            Self::Go | Self::PreviewTop => GO,
            Self::Copy => COPY_AS,
            Self::Sort => SORT_BY,
            Self::Action => ACTIONS,
            Self::Tabs => TAB_CHORD,
        };
        format!("{}-", super::keys::label(&Keys::One(prefix.shown())))
    }

    /// The second keys and their untranslated actions. The `;` panel lists
    /// the matching actions itself, and a file chooser leaves out the steps
    /// it refuses.
    pub(crate) fn options(
        self,
        chooser: bool,
    ) -> Vec<(super::shortcut_reference::Mention, &'static str)> {
        use super::{
            keys::{Keys, Keys::One, bindings::*},
            shortcut_reference::Mention,
        };
        const GO_STEPS: [(Mention, &str); 14] = [
            (
                Mention::Keys(One(GO_FOLLOW.shown())),
                "Follow search result",
            ),
            (Mention::Keys(One(GO_HOME.shown())), "Home"),
            (Mention::Keys(One(GO_DOWNLOADS.shown())), "Downloads"),
            (Mention::Keys(One(GO_CONFIG.shown())), "Config"),
            (Mention::Keys(One(GO_TRASH.shown())), "Trash"),
            (Mention::Keys(One(GO_NETWORK.shown())), "Network"),
            (Mention::Keys(One(GO_RECENT.shown())), "Recent"),
            (Mention::Keys(One(GO_DOCUMENTS.shown())), "Documents"),
            (Mention::Keys(One(GO_MUSIC.shown())), "Music"),
            (Mention::Keys(One(GO_PICTURES.shown())), "Pictures"),
            (Mention::Keys(One(GO_VIDEOS.shown())), "Videos"),
            (Mention::Keys(Keys::Digits(PIN_NUMBERS.step())), "Pins"),
            (
                Mention::Keys(Keys::Any(crate::ui::keys::shown![GO_PIN, GO_UNPIN])),
                "Pin / unpin folder",
            ),
            (Mention::Keys(One(GO_PROMPT.shown())), "Type a path"),
        ];
        // Choosers open only local folders and Recent, and keep pins as they are.
        const CHOOSER_REFUSES: [Mention; 3] = [
            Mention::Keys(One(GO_TRASH.shown())),
            Mention::Keys(One(GO_NETWORK.shown())),
            Mention::Keys(Keys::Any(crate::ui::keys::shown![GO_PIN, GO_UNPIN])),
        ];
        let step = |step: super::keys::Command| Mention::Keys(One(step.shown()));
        let go = |first| -> Vec<_> {
            std::iter::once((step(GO_FIRST), first))
                .chain(GO_STEPS.iter().copied())
                .filter(|(keys, _)| !(chooser && CHOOSER_REFUSES.contains(keys)))
                .collect()
        };
        match self {
            Self::Go => go("First item"),
            Self::PreviewTop => go("Top"),
            Self::Copy => vec![
                (step(COPY_AS_PATH), "Copy path"),
                (step(COPY_AS_NAME), "Copy name"),
            ],
            Self::Sort => vec![
                (step(SORT_BY_NAME), "Name"),
                (step(SORT_BY_MODIFIED), "Modified"),
                (step(SORT_BY_SIZE), "Size"),
                (step(SORT_BY_TYPE), "Type"),
                (Mention::Held(gtk::gdk::ModifierType::SHIFT_MASK), "Reverse"),
            ],
            Self::Action => Vec::new(),
            Self::Tabs => vec![
                (step(TAB_CHORD_NEW), "New tab"),
                (step(TAB_CHORD_CLOSE), "Close tab"),
                (step(TAB_CHORD_PREVIOUS), "Previous tab"),
                (
                    Mention::Keys(Keys::Digits(TAB_CHORD_NUMBERS.step())),
                    "Select tab",
                ),
            ],
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
    /// The find labels are keycaps; the others are translated.
    pub(crate) fn label(self) -> String {
        use super::keys::{
            Keys,
            bindings::{FIND, FIND_BACKWARD},
        };
        let label = match self {
            Self::Find => return super::keys::label(&Keys::One(FIND.shown())),
            Self::FindBackward => return super::keys::label(&Keys::One(FIND_BACKWARD.shown())),
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
        };
        crate::i18n::tr(label)
    }

    pub(crate) fn picks_history(self) -> bool {
        matches!(self, Self::Jump | Self::Recent)
    }

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
