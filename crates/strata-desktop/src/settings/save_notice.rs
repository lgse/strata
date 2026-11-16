// SPDX-License-Identifier: MIT

//! Localized titles and details for settings and folder-view save notices, shared
//! between both interfaces (`src/ui/preferences/save_notice.rs`).

use std::path::Path;

use crate::i18n::tr;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SaveSubject {
    #[default]
    Settings,
    FolderViews,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeProblem {
    /// Writing failed; each later change tries again.
    WriteFailed,
    /// The file could not be read at startup, so saving stays off to keep it.
    UnreadableAtStartup,
}

/// Summary line shown beneath the title of a save or startup-read notice dialog.
pub fn save_notice_summary() -> String {
    tr("Changes last only until Strata closes")
}

/// Title and detail text for `subject` and `problem`; `reason` is already localized
/// for the slot after a colon.
pub fn save_notice_text(
    subject: SaveSubject,
    problem: NoticeProblem,
    path: &Path,
    reason: &str,
) -> (String, String) {
    // The error text may carry a multi-line source excerpt; the log keeps it whole.
    let reason = reason
        .lines()
        .next()
        .unwrap_or_default()
        .trim_end_matches('.');
    let path = path.display();
    let title = tr(match (subject, problem) {
        (SaveSubject::Settings, NoticeProblem::WriteFailed) => "Settings can't be saved",
        (SaveSubject::Settings, NoticeProblem::UnreadableAtStartup) => {
            "Settings file can't be read"
        }
        (SaveSubject::FolderViews, NoticeProblem::WriteFailed) => "Folder settings can't be saved",
        (SaveSubject::FolderViews, NoticeProblem::UnreadableAtStartup) => {
            "Folder settings file can't be read"
        }
    });
    match problem {
        NoticeProblem::WriteFailed => (
            title,
            rust_i18n::t!(
                "Strata couldn't write “%{path}”: %{reason}. It tries again with each change.",
                path = path,
                reason = reason
            )
            .into_owned(),
        ),
        NoticeProblem::UnreadableAtStartup => (
            title,
            rust_i18n::t!(
                "Strata couldn't read “%{path}” when it started: %{reason}. To keep the file as it is, Strata won't save over it. Fix or remove the file, then restart Strata.",
                path = path,
                reason = reason
            )
            .into_owned(),
        ),
    }
}

#[cfg(test)]
mod tests;
