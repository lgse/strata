// SPDX-License-Identifier: MIT

//! Translated [`PromptSpec`]s and [`PromptLabels`] for 10xer's twelve footer prompts
//! (`Prompt` in `src/ui/tenxer_mode.rs`, `src/ui/shortcut_footer/candidates.rs`).

use std::rc::Rc;

use strata_components::filter_bar::prompt::{Arrows, Legend, PromptLabels, PromptSpec};
use strata_core::i18n::{integer, tr};
use strata_desktop::keys::{
    self as key_catalog, Keys,
    bindings::{COMPLETE, DOWN, ENTER, ESCAPE, FIND, FIND_BACKWARD, UP},
    shown,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum PromptKind {
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

impl PromptKind {
    pub(crate) fn picks_history(self) -> bool {
        matches!(self, Self::Jump | Self::Recent)
    }

    pub(crate) fn picks_folder(self) -> bool {
        matches!(
            self,
            Self::Go | Self::MoveTo | Self::CopyTo | Self::ExtractTo
        )
    }

    pub(crate) fn label(self) -> String {
        let key = match self {
            Self::Find => return key_catalog::label(&Keys::One(FIND.shown())),
            Self::FindBackward => return key_catalog::label(&Keys::One(FIND_BACKWARD.shown())),
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
        tr(key)
    }

    pub(crate) fn name(self) -> String {
        let key = match self {
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
        };
        tr(key)
    }

    pub(crate) fn spec(self) -> PromptSpec {
        let arrows = match self {
            Self::Rename => Arrows::Swallow,
            kind if kind.picks_folder() || kind.picks_history() => Arrows::Candidates,
            _ => Arrows::Listing,
        };
        let legend = match self {
            Self::Go => Some(translated_legend("Open", true)),
            Self::MoveTo => Some(translated_legend("Move", true)),
            Self::CopyTo => Some(translated_legend("Copy", true)),
            Self::ExtractTo => Some(translated_legend("Extract", true)),
            Self::Jump | Self::Recent => Some(translated_legend("Open", false)),
            _ => None,
        };
        PromptSpec {
            label: self.label(),
            name: self.name(),
            undo: self != Self::Go,
            arrows,
            legend,
            position: self.picks_history(),
        }
    }
}

fn translated_legend(enter: &str, complete: bool) -> Legend {
    Legend {
        choose: tr("Choose"),
        complete: complete.then(|| tr("completion.complete")),
        enter: tr(enter),
        cancel: tr("Cancel"),
    }
}

pub(crate) fn prompt_labels() -> PromptLabels {
    PromptLabels {
        position: Rc::new(|pos, count| {
            rust_i18n::t!(
                "%{position} of %{count}",
                position = integer(pos as u64),
                count = integer(count as u64)
            )
            .into_owned()
        }),
        candidates: tr("Folders"),
        keys: [
            key_catalog::label(&Keys::Any(shown![UP.alternate(0), DOWN.alternate(0)])),
            key_catalog::label(&Keys::One(COMPLETE.shown())),
            key_catalog::label(&Keys::One(ENTER.shown())),
            key_catalog::label(&Keys::One(ESCAPE.shown())),
        ],
    }
}
