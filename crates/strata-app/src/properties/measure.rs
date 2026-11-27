// SPDX-License-Identifier: MIT

use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};

use rune::Tx;
use strata_core::{
    i18n::{count, file_size, tr},
    model::{FileEntry, Location, MetadataValue},
};
use strata_fs::adapters::{
    directory_summary::{DirectorySummary, summarize_directory_with_progress},
    gio_file_for_location,
};

use super::Dialog;
use crate::{App, slots::Later};

const SIZE_PROGRESS_INTERVAL: Duration = Duration::from_millis(150);

#[derive(Default)]
struct SizeProgressThrottle {
    last_update: Cell<Option<Instant>>,
}

impl SizeProgressThrottle {
    fn should_update(&self, now: Instant) -> bool {
        if self
            .last_update
            .get()
            .is_some_and(|last| now.duration_since(last) < SIZE_PROGRESS_INTERVAL)
        {
            return false;
        }
        self.last_update.set(Some(now));
        true
    }
}

pub(super) fn size_label(summary: &DirectorySummary) -> String {
    let prefix = if summary.truncated() { "≥ " } else { "" };
    format!("{prefix}{}", file_size(summary.total_size))
}

pub(super) fn directory_counts_label(summary: &DirectorySummary) -> String {
    let prefix = if summary.truncated() { "≥ " } else { "" };
    rust_i18n::t!(
        "%{files}, %{folders}",
        files = format!("{prefix}{}", count("files", summary.visible_file_count)),
        folders = format!("{prefix}{}", count("folders", summary.visible_folder_count)),
    )
    .into_owned()
}

pub(super) fn measurement_warning_text(summary: &DirectorySummary) -> Option<String> {
    let mut reasons = Vec::new();
    if summary.issues.unreadable {
        reasons.push(tr("Some folders or entries couldn't be read."));
    }
    if summary.issues.timed_out {
        reasons.push(tr("The five-minute calculation limit was reached."));
    }
    if summary.issues.depth_limited {
        reasons.push(tr("Some folders exceeded the 64-level nesting limit."));
    }
    (!reasons.is_empty()).then(|| {
        rust_i18n::t!(
            "Totals are incomplete.\n%{value1}",
            value1 = reasons.join("\n")
        )
        .into_owned()
    })
}

pub(super) fn spawn_folder(
    later: Later<App>,
    generation: u64,
    location: Location,
) -> glib::JoinHandle<()> {
    let directory = gio_file_for_location(&location);
    let throttle = Rc::new(SizeProgressThrottle::default());
    let progress_later = later.clone();
    let progress_throttle = throttle.clone();
    glib::MainContext::default().spawn_local(async move {
        let result = summarize_directory_with_progress(&directory, move |partial| {
            if partial.item_count > 0 && progress_throttle.should_update(Instant::now()) {
                progress_later.then(move |app: &mut App, _: &mut Tx| {
                    apply_summary(app, generation, partial, true)
                });
            }
        })
        .await;
        later.then(move |app: &mut App, _: &mut Tx| finish_folder(app, generation, result));
    })
}

fn apply_summary(
    app: &mut App,
    generation: u64,
    summary: DirectorySummary,
    spinning: bool,
) -> bool {
    let rendered = (
        size_label(&summary),
        directory_counts_label(&summary),
        measurement_warning_text(&summary),
    );
    match app.properties.dialog.as_mut() {
        Some(Dialog::Single(single)) if single.generation == generation => {
            single.spinning = spinning;
            single.size = rendered.0;
            single.contains = Some(rendered.1);
            single.warning = rendered.2;
            true
        }
        Some(Dialog::Selection(selection)) if selection.generation == generation => {
            selection.spinning = spinning;
            (selection.size, selection.contains, selection.warning) = rendered;
            true
        }
        _ => false,
    }
}

fn finish_folder(
    app: &mut App,
    generation: u64,
    result: Result<DirectorySummary, glib::Error>,
) -> bool {
    match result {
        Ok(summary) => apply_summary(app, generation, summary, false),
        Err(_) => {
            let Some(Dialog::Single(single)) = app.properties.dialog.as_mut() else {
                return false;
            };
            if single.generation != generation {
                return false;
            }
            single.spinning = false;
            single.size = tr("Unavailable");
            single.contains = Some(tr("Unavailable"));
            single.warning = Some(tr("Folder contents couldn't be read."));
            true
        }
    }
}

pub(super) fn spawn_selection(
    later: Later<App>,
    generation: u64,
    entries: Vec<FileEntry>,
) -> glib::JoinHandle<()> {
    let throttle = Rc::new(SizeProgressThrottle::default());
    glib::MainContext::default().spawn_local(async move {
        let mut total = initial_selection_summary(&entries);
        let initial = total;
        later.then(move |app: &mut App, _: &mut Tx| apply_summary(app, generation, initial, true));
        for entry in entries.iter().filter(|entry| entry.is_directory()) {
            let directory = gio_file_for_location(&entry.location);
            let completed = total;
            let step_later = later.clone();
            let step_throttle = throttle.clone();
            let summary = summarize_directory_with_progress(&directory, move |partial| {
                if partial.item_count > 0 && step_throttle.should_update(Instant::now()) {
                    let mut combined = completed;
                    combined.include(partial);
                    step_later.then(move |app: &mut App, _: &mut Tx| {
                        apply_summary(app, generation, combined, true)
                    });
                }
            })
            .await;
            match summary {
                Ok(summary) => total.include(summary),
                Err(_) => total.issues.unreadable = true,
            }
            let current = total;
            later.then(move |app: &mut App, _: &mut Tx| {
                apply_summary(app, generation, current, true)
            });
        }
        later.then(move |app: &mut App, _: &mut Tx| apply_summary(app, generation, total, false));
    })
}

fn initial_selection_summary(entries: &[FileEntry]) -> DirectorySummary {
    let mut total = DirectorySummary::default();
    for entry in entries.iter().filter(|entry| !entry.is_directory()) {
        total.item_count = total.item_count.saturating_add(1);
        total.visible_file_count = total.visible_file_count.saturating_add(1);
        if let MetadataValue::Known(size) = entry.size {
            total.total_size = total.total_size.saturating_add(size);
        } else {
            total.issues.unreadable = true;
        }
    }
    total
}
