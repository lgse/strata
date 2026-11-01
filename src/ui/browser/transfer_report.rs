// SPDX-License-Identifier: MIT

use std::{ffi::OsStr, time::Duration};

use crate::model::Location;
use crate::services::{
    Holder, KeptCause, KeptItem, RenameCause, RenamedItem, SkippedItem, SpecialKind, TransferReport,
};
use crate::ui::browser::ViewState;

#[cfg(test)]
mod tests;

const NOTICE_DURATION: Duration = Duration::from_secs(6);
const LISTED_ITEMS: usize = 8;
const NOTICE_CHARS: usize = 120;

#[derive(Debug, Eq, PartialEq)]
enum Presentation {
    Notice {
        heading: String,
        text: String,
    },
    Information {
        message: String,
        summary: String,
        detail: String,
    },
    Warning {
        detail: String,
    },
}

fn display_name(location: &Location) -> String {
    shown_name(&location.file_name().unwrap_or_default())
}

/// `name` with a mark for each control character, which would otherwise
/// break the line or not show.
pub(super) fn shown_name(name: &OsStr) -> String {
    name.to_string_lossy()
        .chars()
        .map(|character| {
            if character.is_control() {
                char::REPLACEMENT_CHARACTER
            } else {
                character
            }
        })
        .collect()
}

fn heading(cause: RenameCause, count: usize) -> String {
    crate::i18n::tr(match (cause, count == 1) {
        (RenameCause::Alias, true) => {
            "The destination can\u{2019}t tell this name apart from another"
        }
        (RenameCause::Alias, false) => "The destination can\u{2019}t tell these names apart",
        (RenameCause::Taken, true) => "This name was already taken",
        (RenameCause::Taken, false) => "These names were already taken",
        (RenameCause::Invalid, true) => "The destination doesn\u{2019}t allow this name",
        (RenameCause::Invalid, false) => "The destination doesn\u{2019}t allow these names",
        (RenameCause::TooLong, true) => "This name was too long for the destination",
        (RenameCause::TooLong, false) => "These names were too long for the destination",
    })
}

fn renamed_line(item: &RenamedItem) -> String {
    rust_i18n::t!(
        "“%{name}” was saved as “%{new_name}”",
        name = display_name(&item.requested),
        new_name = display_name(&item.saved_as)
    )
    .into_owned()
}

fn special_line(name: &str, kind: SpecialKind) -> String {
    match kind {
        SpecialKind::Pipe => rust_i18n::t!("“%{name}” (named pipe)", name = name),
        SpecialKind::Socket => rust_i18n::t!("“%{name}” (socket)", name = name),
        SpecialKind::Device => rust_i18n::t!("“%{name}” (device file)", name = name),
        SpecialKind::Symlink => rust_i18n::t!("“%{name}” (symbolic link)", name = name),
    }
    .into_owned()
}

fn skipped_line(item: &SkippedItem) -> String {
    special_line(&display_name(&item.location), item.kind)
}

fn kept_line(item: &KeptItem, in_trash: bool) -> String {
    let name = display_name(&item.location);
    let folder = || {
        item.location
            .parent()
            .map(|folder| display_name(&folder))
            .unwrap_or_default()
    };
    match &item.cause {
        KeptCause::Changed if in_trash => rust_i18n::t!(
            "“%{name}” changed during the restore and was left in the Trash",
            name = name
        )
        .into_owned(),
        KeptCause::Changed => rust_i18n::t!(
            "“%{name}” changed during the move and was left in “%{folder}”",
            name = name,
            folder = folder()
        )
        .into_owned(),
        KeptCause::ReplacementChanged => rust_i18n::t!(
            "“%{name}” was left in the Trash, as what replaced it may have changed since",
            name = name
        )
        .into_owned(),
        KeptCause::CopyGone => rust_i18n::t!(
            "“%{name}” was left in “%{folder}”, as its copy was no longer where the move put it",
            name = name,
            folder = folder()
        )
        .into_owned(),
        KeptCause::StillOpen(Some(Holder::Program(program))) => rust_i18n::t!(
            "“%{name}” was left in “%{folder}”, as a file in it is still open in %{program}",
            name = name,
            folder = folder(),
            program = program
        )
        .into_owned(),
        KeptCause::StillOpen(Some(Holder::Strata)) => rust_i18n::t!(
            "“%{name}” was left in “%{folder}”, as a file in it is still in use by Strata",
            name = name,
            folder = folder()
        )
        .into_owned(),
        KeptCause::StillOpen(None) => rust_i18n::t!(
            "“%{name}” was left in “%{folder}”, as a file in it is still open in another program",
            name = name,
            folder = folder()
        )
        .into_owned(),
        KeptCause::Undeletable(error) => rust_i18n::t!(
            "Could not delete “%{name}”: %{error}",
            name = name,
            error = crate::services::io_error_detail(&std::io::Error::from_raw_os_error(*error))
        )
        .into_owned(),
        KeptCause::Skipped(kind) => special_line(&name, *kind),
        KeptCause::OriginalNotRemoved(error) => rust_i18n::t!(
            "“%{name}” was replaced, but its original could not be removed from “%{kept}”: %{error}",
            name = name,
            kept = folder(),
            error = error
        )
        .into_owned(),
        KeptCause::NameNotRestored { saved_as, error } => rust_i18n::t!(
            "“%{new_name}” could not be renamed back to “%{name}”: %{error}",
            new_name = display_name(saved_as),
            name = name,
            error = error
        )
        .into_owned(),
    }
}

struct Lines {
    listed: Vec<String>,
    count: usize,
    note: Option<String>,
}

impl Lines {
    fn new() -> Self {
        Self {
            listed: Vec::new(),
            count: 0,
            note: None,
        }
    }

    fn of<T>(items: impl IntoIterator<Item = T>, line: impl Fn(T) -> String) -> Self {
        let mut lines = Self::new();
        for item in items {
            lines.push(|| line(item));
        }
        lines
    }

    fn push(&mut self, line: impl FnOnce() -> String) {
        if self.count < LISTED_ITEMS {
            self.listed.push(line());
        }
        self.count += 1;
    }
}

fn bullets(lines: &Lines) -> String {
    let mut text = lines
        .listed
        .iter()
        .map(|line| format!("• {line}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    if lines.count > LISTED_ITEMS {
        text.push_str("\n\n");
        text.push_str(&crate::i18n::count(
            "more_items",
            lines.count - LISTED_ITEMS,
        ));
    }
    if let Some(note) = &lines.note {
        text.push_str("\n\n");
        text.push_str(note);
    }
    text
}

fn renamed_sections(renamed: &[RenamedItem]) -> Vec<(String, Lines)> {
    let mut sections = Vec::<(RenameCause, Lines)>::new();
    for item in renamed {
        let index = match sections.iter().position(|(cause, _)| *cause == item.cause) {
            Some(index) => index,
            None => {
                sections.push((item.cause, Lines::new()));
                sections.len() - 1
            }
        };
        sections[index].1.push(|| renamed_line(item));
    }
    sections
        .into_iter()
        .map(|(cause, lines)| (heading(cause, lines.count), lines))
        .collect()
}

fn sections(sections: Vec<(String, Lines)>) -> String {
    sections
        .into_iter()
        .map(|(heading, lines)| format!("{heading}\n\n{}", bullets(&lines)))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn left_sections(report: &TransferReport) -> Vec<(String, Lines)> {
    let mut left = Vec::new();
    let (links, special): (Vec<_>, Vec<_>) = report
        .skipped
        .iter()
        .partition(|item| item.kind == SpecialKind::Symlink);
    for (skipped, one, several) in [
        (
            special,
            "This special file can\u{2019}t be copied and was left out",
            "These special files can\u{2019}t be copied and were left out",
        ),
        (
            links,
            "This symbolic link can\u{2019}t be stored at the destination and was left out",
            "These symbolic links can\u{2019}t be stored at the destination and were left out",
        ),
    ] {
        if !skipped.is_empty() {
            let heading = crate::i18n::tr(if skipped.len() == 1 { one } else { several });
            left.push((heading, Lines::of(skipped, skipped_line)));
        }
    }
    for (kept, heading, in_trash) in [
        (&report.kept, "items_kept", false),
        (&report.kept_in_trash, "items_kept_in_trash", true),
    ] {
        if !kept.is_empty() {
            let mut lines = Lines::of(kept, |item| kept_line(item, in_trash));
            if !in_trash && !report.kept_out_of_undo.is_empty() {
                let left_out = Lines::of(&report.kept_out_of_undo, |item| {
                    crate::i18n::quoted(&display_name(item))
                });
                lines.note = Some(format!(
                    "{}\n\n{}",
                    crate::i18n::tr(if report.kept_out_of_undo.len() == 1 {
                        "Undo can\u{2019}t move this item back, as part of it stayed in its original folder."
                    } else {
                        "Undo can\u{2019}t move these items back, as part of each stayed in its original folder."
                    }),
                    bullets(&left_out)
                ));
            }
            left.push((crate::i18n::count(heading, lines.count), lines));
        }
    }
    left
}

pub(super) fn outcome_detail(outcome: String, report: &TransferReport) -> String {
    let mut reported = renamed_sections(&report.renamed);
    reported.extend(left_sections(report));
    if reported.is_empty() {
        return outcome;
    }
    format!("{outcome}\n\n{}", sections(reported))
}

fn presentation(report: &TransferReport) -> Option<Presentation> {
    let mut renamed = renamed_sections(&report.renamed);
    let left = left_sections(report);
    if !left.is_empty() {
        renamed.extend(left);
        return Some(Presentation::Warning {
            detail: sections(renamed),
        });
    }
    let message = crate::i18n::count("items_renamed", report.renamed.len());
    if renamed.len() > 1 {
        return Some(Presentation::Information {
            message,
            summary: crate::i18n::tr("Names changed for different reasons"),
            detail: sections(renamed),
        });
    }
    let (heading, lines) = renamed.pop()?;
    if let [line] = lines.listed.as_slice()
        && line.chars().count() <= NOTICE_CHARS
    {
        return Some(Presentation::Notice {
            heading,
            text: line.clone(),
        });
    }
    Some(Presentation::Information {
        message,
        summary: heading,
        detail: bullets(&lines),
    })
}

impl ViewState {
    pub(super) fn show_transfer_report(&self, report: &TransferReport) {
        match presentation(report) {
            Some(Presentation::Notice { heading, text }) => {
                crate::ui::notice::add_notice(&self.overlay, Some(&heading), &text, NOTICE_DURATION)
            }
            Some(Presentation::Information {
                message,
                summary,
                detail,
            }) => crate::ui::modal::show_information_dialog_with_summary(
                &self.overlay,
                &message,
                &summary,
                &detail,
            ),
            Some(Presentation::Warning { detail }) => {
                crate::ui::modal::show_partial_failure_dialog(&self.overlay, &detail);
            }
            None => {}
        }
    }
}
