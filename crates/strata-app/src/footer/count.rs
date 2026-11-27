// SPDX-License-Identifier: MIT

use std::{collections::HashSet, rc::Rc};

use strata_components::footer::{
    Count, CountWords, DETAILED_SELECTION, FilterCounts, Listing, Selected, SelectionDetails,
    SingleSelection, summary,
};
use strata_core::{
    i18n::{count, file_size, integer, list, tr},
    model::{FileEntry, Location, MetadataValue},
    util::DateFormat,
};
use strata_fs::{
    app::{Browser, BrowserEvent},
    services::RequestId,
};

use crate::{App, filter, shell, slots::Listener};

/// Cheap inputs to `poll`; if unchanged and no browser event landed, `poll` is a no-op.
#[derive(Clone, Copy, PartialEq, Eq)]
struct PollKey {
    depth: usize,
    request_id: Option<RequestId>,
    selected_count: usize,
    filter: Option<FilterCounts>,
    show_modified: bool,
    date_format: DateFormat,
    date_bucket: u64,
}

/// What the footer counted at the last poll, and what it knows of the metadata it asks for.
#[derive(Default)]
pub(super) struct Counted {
    listing: Option<Listing>,
    heard: bool,
    /// The listing the files below belong to: the active column and its request.
    under: Option<(usize, Option<RequestId>)>,
    /// The selection's positions at the last poll: a new selection asks for its metadata.
    selected: Vec<usize>,
    unreadable: HashSet<Location>,
    last_poll: Option<PollKey>,
}

pub(super) fn listener() -> Listener<App> {
    Box::new(|event, _, heard| {
        let answered = match event {
            BrowserEvent::MetadataFilled { depth, updates } => Some((
                *depth,
                updates
                    .iter()
                    .filter(|(_, entry)| lacks_metadata(entry))
                    .map(|(_, entry)| entry.location.clone())
                    .collect::<Vec<_>>(),
            )),
            _ => None,
        };
        heard.then(move |app: &mut App, _| {
            let active = app.browser.active_depth();
            let counted = &mut app.footer.counted;
            counted.heard = true;
            if let Some((_, unreadable)) = answered.filter(|(depth, _)| Some(*depth) == active) {
                counted.unreadable.extend(unreadable);
            }
            false
        });
    })
}

fn poll_key(app: &App, depth: usize, filter: Option<FilterCounts>, clock: f64) -> PollKey {
    let browser = &app.browser;
    let request_id = browser.column_request_id(depth);
    let selected_count = browser.selected_count();
    let show_modified = shows_modified_date(app);
    let date_format = strata_core::util::date_format();
    let date_bucket = if show_modified && date_format == DateFormat::Relative && selected_count == 1
    {
        (clock.max(0.0) / 30.0) as u64
    } else {
        0
    };
    PollKey {
        depth,
        request_id,
        selected_count,
        filter,
        show_modified,
        date_format,
        date_bucket,
    }
}

pub(super) fn poll(app: &mut App, clock: f64) -> bool {
    let filter_counts = filter::footer_counts(app);
    let browser = Rc::clone(&app.browser);
    let Some(depth) = browser.active_depth() else {
        let counted = &mut app.footer.counted;
        let shown = counted.listing.is_some();
        *counted = Counted::default();
        return shown;
    };
    let key = poll_key(app, depth, filter_counts, clock);
    if !app.footer.counted.heard
        && app.footer.counted.listing.is_some()
        && app.footer.counted.last_poll == Some(key)
    {
        return false;
    }
    let under = Some((depth, key.request_id));
    let (selected, replaced, reselected, heard) = {
        let counted = &mut app.footer.counted;
        let replaced = counted.under != under;
        if replaced {
            counted.under = under;
            counted.unreadable.clear();
        }
        let selected = if key.selected_count <= DETAILED_SELECTION {
            browser.selected_positions(depth)
        } else {
            Vec::new()
        };
        let reselected = counted.selected != selected;
        let heard = std::mem::take(&mut counted.heard);
        counted.selected = selected.clone();
        (selected, replaced, reselected, heard)
    };
    let filtering = filter_counts.is_some_and(|f| f.displayed);
    let was_filtering = app
        .footer
        .counted
        .listing
        .as_ref()
        .and_then(|l| l.filter)
        .is_some_and(|f| f.displayed);
    if !filtering && (heard || replaced || reselected || was_filtering) {
        ask_metadata(&browser, depth, &selected, &app.footer.counted.unreadable);
    }
    let listing = Some(read(app, depth, &selected, filter_counts));
    let counted = &mut app.footer.counted;
    counted.last_poll = Some(key);
    if counted.listing == listing {
        return false;
    }
    counted.listing = listing;
    true
}

pub(super) fn shown(app: &App) -> Option<Count> {
    let listing = app.footer.counted.listing.as_ref()?;
    let live = Listing {
        filter: filter::footer_counts(app),
        ..listing.clone()
    };
    Some(summary(&live, &words()))
}

fn read(app: &App, depth: usize, selected: &[usize], filter: Option<FilterCounts>) -> Listing {
    let browser = &app.browser;
    let counts = browser.column_entry_counts(depth).unwrap_or_default();
    let count = browser.selected_count();
    let show_modified = shows_modified_date(app);
    let selected = (count > 0).then(|| {
        let detailed = count <= DETAILED_SELECTION;
        Selected {
            count,
            details: detailed.then(|| details(browser, depth, selected)),
            single: (detailed && count == 1)
                .then(|| single_entry(browser, depth, selected, show_modified))
                .flatten(),
        }
    });
    Listing {
        total: counts.total,
        files: counts.files,
        folders: counts.folders,
        selected,
        filter,
    }
}

fn shows_modified_date(app: &App) -> bool {
    shell::shown_view(app).is_some_and(|view| (view.shows)("icons") || (view.shows)("columns"))
}

fn single_entry(
    browser: &Browser,
    depth: usize,
    selected: &[usize],
    show_modified: bool,
) -> Option<SingleSelection> {
    let [position] = selected else {
        return None;
    };
    let entry = browser.entry_at(depth, *position)?;
    let is_directory = entry.is_directory();
    let size = match entry.size {
        MetadataValue::Known(bytes) => Some(bytes),
        _ => None,
    };
    let modified = show_modified.then(|| strata_core::util::modified_date(&entry));
    Some(SingleSelection {
        name: entry.display_name,
        is_directory,
        size,
        modified,
    })
}

/// The selection's kinds and the selected files' known sizes.
fn details(browser: &Browser, depth: usize, selected: &[usize]) -> SelectionDetails {
    let mut details = SelectionDetails::default();
    for entry in selected
        .iter()
        .filter_map(|&at| browser.entry_at(depth, at))
    {
        if entry.is_directory() {
            details.folders += 1;
        } else {
            details.files += 1;
            if let MetadataValue::Known(size) = entry.size {
                details.known_files += 1;
                details.known_bytes = details.known_bytes.saturating_add(size);
            }
        }
    }
    details
}

fn ask_metadata(
    browser: &Rc<Browser>,
    depth: usize,
    selected: &[usize],
    unreadable: &HashSet<Location>,
) {
    for &position in selected {
        let Some(entry) = browser.entry_at(depth, position) else {
            continue;
        };
        if lacks_metadata(&entry) && !unreadable.contains(&entry.location) {
            browser.request_metadata_fill(depth, position, entry.location, false);
        }
    }
}

fn lacks_metadata(entry: &FileEntry) -> bool {
    (!entry.is_directory() && entry.size == MetadataValue::Unknown)
        || entry.modified_unix_seconds == MetadataValue::Unknown
}

fn words() -> CountWords {
    CountWords {
        items: Rc::new(|n| count("items", n)),
        files: Rc::new(|n| count("files", n)),
        folders: Rc::new(|n| count("folders", n)),
        list: Rc::new(|parts| list(parts.iter().cloned())),
        selected: Rc::new(|items| rust_i18n::t!("%{items} selected", items = items).into_owned()),
        breakdown: Rc::new(|files, folders| {
            rust_i18n::t!("%{files}, %{folders}", files = files, folders = folders).into_owned()
        }),
        // The plural form follows the total; the selected count fills in afterwards.
        selected_of: Rc::new(|selected, total| {
            rust_i18n::replace_patterns(
                &count("selected_of_items", total),
                &["selected"],
                &[integer(selected as u64)],
            )
        }),
        selection_description: Rc::new(|selection| {
            rust_i18n::t!(
                "%{selection} Size includes selected files only; folder contents are not counted.",
                selection = selection
            )
            .into_owned()
        }),
        size: Rc::new(file_size),
        size_known: Rc::new(|size| format!(" ({size})")),
        size_incomplete: Rc::new(|size| {
            rust_i18n::t!(" (%{size} known; size incomplete)", size = size).into_owned()
        }),
        size_unavailable: tr(" (size unavailable)"),
    }
}
